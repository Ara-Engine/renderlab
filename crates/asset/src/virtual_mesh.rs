//! Immutable virtual geometry; group ancestry and IO pages are separate domains.
//! Raw records must pass `Package::validate` before use. Selection revalidates its mesh.
mod codec;
mod selection;
use crate::{Asset, Error, Limits, Placement, Result, Vertex, add, cap, product, valid_transform};
pub use codec::{decode, decode_page, encode};
pub use selection::{Cut, DebtReason, Demand, QualityDebt, Selection, SelectionBudget, select};

pub const VERSION: u32 = 1;
pub const PAGE_BYTES: usize = 64 << 10;

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}
id!(GroupId);
id!(ClusterId);
id!(PageId);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub center: [f32; 3],
    pub radius: f32,
    pub error: f32,
}
impl Bounds {
    fn valid(self) -> bool {
        self.center.iter().all(|v| v.is_finite())
            && self.radius.is_finite()
            && self.radius >= 0.
            && self.error.is_finite()
            && self.error >= 0.
    }
    fn encloses(self, child: Self) -> bool {
        let distance = self
            .center
            .iter()
            .zip(child.center)
            .map(|(&a, b)| (f64::from(a) - f64::from(b)).powi(2))
            .sum::<f64>()
            .sqrt();
        let scale = self
            .center
            .iter()
            .chain(&child.center)
            .map(|v| f64::from(*v).abs())
            .fold(1., f64::max)
            .max(f64::from(self.radius))
            .max(f64::from(child.radius));
        distance + f64::from(child.radius)
            <= f64::from(self.radius) + scale * 64. * f64::from(f32::EPSILON)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub offset: u32,
    pub count: u32,
}
impl Span {
    pub fn range(self, length: usize) -> Result<std::ops::Range<usize>> {
        let end = self
            .offset
            .checked_add(self.count)
            .ok_or(Error("virtual span overflow"))? as usize;
        cap(end, length, "virtual span out of range")?;
        Ok(self.offset as usize..end)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub depth: u32,
    /// Upstream accumulated simplified bounds; terminal groups use finite f32::MAX error.
    pub simplified: Bounds,
    pub clusters: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub owner: GroupId,
    pub refined: Option<GroupId>,
    pub bounds: Bounds,
    pub material: u32,
    pub page: PageId,
    /// Index range in this page's u32 index array. Indices address the page's vertex array.
    pub indices: Span,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMesh {
    pub groups: Vec<Group>,
    pub clusters: Vec<Cluster>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub vertices: u32,
    pub indices: u32,
    pub payload_bytes: u32,
    pub sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub meshes: Vec<VirtualMesh>,
    pub pages: Vec<Page>,
    pub placements: Vec<Placement>,
    /// Only materials and textures; meshes/placements must be empty. Encoded using unchanged RLA1.
    pub resources: Asset,
}
#[derive(Clone, Copy, Debug)]
pub struct VirtualLimits {
    pub prepared: Limits,
    pub max_groups: usize,
    pub max_clusters: usize,
    pub max_pages: usize,
    pub max_metadata_bytes: usize,
    pub max_geometry_bytes: usize,
}
impl Default for VirtualLimits {
    fn default() -> Self {
        Self {
            prepared: Limits::default(),
            max_groups: 65536,
            max_clusters: 131072,
            max_pages: 8192,
            max_metadata_bytes: 8 << 20,
            max_geometry_bytes: 512 << 20,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageCounts {
    pub groups: usize,
    pub clusters: usize,
    pub pages: usize,
    pub payload_bytes: usize,
    pub disk_page_bytes: usize,
    pub metadata_host_bytes: usize,
}

impl VirtualMesh {
    /// IDs are topologically ordered: every refined group precedes its owners.
    /// All incoming shards of a nonterminal group participate in one atomic replacement.
    pub fn validate(
        &self,
        page_count: usize,
        materials: usize,
        limits: VirtualLimits,
    ) -> Result<()> {
        cap(self.groups.len(), limits.max_groups, "virtual group limit")?;
        cap(
            self.clusters.len(),
            limits.max_clusters,
            "virtual cluster limit",
        )?;
        if self.groups.is_empty() || self.clusters.is_empty() {
            return Err(Error("empty virtual mesh"));
        }
        let mut incoming = vec![0usize; self.groups.len()];
        let mut next = 0;
        for (i, g) in self.groups.iter().enumerate() {
            if !g.simplified.valid() || g.clusters.count == 0 || g.clusters.offset as usize != next
            {
                return Err(Error("invalid group bounds/range"));
            }
            let range = g.clusters.range(self.clusters.len())?;
            next = range.end;
            for c in &self.clusters[range] {
                if c.owner.index() != i
                    || !c.bounds.valid()
                    || c.page.index() >= page_count
                    || c.material as usize >= materials
                    || c.indices.count == 0
                    || !c.indices.count.is_multiple_of(3)
                {
                    return Err(Error("invalid cluster record"));
                }
                if let Some(r) = c.refined {
                    if r.index() >= i || self.groups[r.index()].depth >= g.depth {
                        return Err(Error("cyclic/unordered group dependency"));
                    }
                    let child = &self.groups[r.index()];
                    if g.simplified.error < child.simplified.error
                        || !g.simplified.encloses(child.simplified)
                    {
                        return Err(Error("nonmonotone group error/bounds"));
                    }
                    incoming[r.index()] += 1;
                }
            }
        }
        if next != self.clusters.len() {
            return Err(Error("unowned cluster"));
        }
        for (i, g) in self.groups.iter().enumerate() {
            if (incoming[i] == 0) != (g.simplified.error == f32::MAX) {
                return Err(Error("terminal group marker inconsistent"));
            }
        }
        Ok(())
    }
    pub fn terminal_groups(&self) -> impl Iterator<Item = GroupId> + '_ {
        self.groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.simplified.error == f32::MAX)
            .map(|(i, _)| GroupId(i as u32))
    }
}

impl Package {
    pub fn validate(&self, limits: VirtualLimits) -> Result<PackageCounts> {
        if !self.resources.meshes.is_empty() || !self.resources.placements.is_empty() {
            return Err(Error("virtual resources contain geometry"));
        }
        self.resources.validate(limits.prepared)?;
        cap(
            self.meshes.len(),
            limits.prepared.max_meshes,
            "virtual mesh limit",
        )?;
        cap(
            self.placements.len(),
            limits.prepared.max_placements,
            "virtual placement limit",
        )?;
        cap(self.pages.len(), limits.max_pages, "virtual page limit")?;
        let mut counts = PackageCounts {
            pages: self.pages.len(),
            ..PackageCounts::default()
        };
        counts.metadata_host_bytes =
            product(self.meshes.len(), std::mem::size_of::<VirtualMesh>())?;
        add(
            &mut counts.metadata_host_bytes,
            product(self.pages.len(), std::mem::size_of::<Page>())?,
        )?;
        add(
            &mut counts.metadata_host_bytes,
            product(self.placements.len(), std::mem::size_of::<Placement>())?,
        )?;
        for mesh in &self.meshes {
            add(&mut counts.groups, mesh.groups.len())?;
            add(&mut counts.clusters, mesh.clusters.len())?;
            cap(counts.groups, limits.max_groups, "aggregate virtual groups")?;
            cap(
                counts.clusters,
                limits.max_clusters,
                "aggregate virtual clusters",
            )?;
            add(
                &mut counts.metadata_host_bytes,
                product(mesh.groups.len(), std::mem::size_of::<Group>())?,
            )?;
            add(
                &mut counts.metadata_host_bytes,
                product(mesh.clusters.len(), std::mem::size_of::<Cluster>())?,
            )?;
        }
        cap(
            counts.metadata_host_bytes,
            limits.max_metadata_bytes,
            "virtual metadata budget",
        )?;
        // Charge validation scratch before allocating it; intervals bound page overlaps globally.
        let scratch = product(counts.clusters, std::mem::size_of::<(u32, u32, u32)>())?
            .checked_add(product(counts.groups, 8)?)
            .ok_or(Error("virtual scratch overflow"))?;
        cap(
            scratch,
            limits
                .max_metadata_bytes
                .saturating_sub(counts.metadata_host_bytes),
            "virtual validation scratch budget",
        )?;
        let mut intervals = Vec::with_capacity(counts.clusters);
        for mesh in &self.meshes {
            mesh.validate(self.pages.len(), self.resources.materials.len(), limits)?;
            for c in &mesh.clusters {
                let page = &self.pages[c.page.index()];
                let r = c.indices.range(page.indices as usize)?;
                intervals.push((c.page.0, r.start as u32, r.end as u32));
            }
        }
        intervals.sort_unstable();
        let mut cursor = 0;
        for (i, p) in self.pages.iter().enumerate() {
            let length = product(p.vertices as usize, 48)?
                .checked_add(product(p.indices as usize, 4)?)
                .ok_or(Error("page size overflow"))?;
            if p.vertices == 0
                || p.indices == 0
                || !p.indices.is_multiple_of(3)
                || length != p.payload_bytes as usize
                || length > PAGE_BYTES
            {
                return Err(Error("invalid virtual page extent"));
            }
            let mut end = 0;
            while cursor < intervals.len() && intervals[cursor].0 as usize == i {
                let (_, a, b) = intervals[cursor];
                if a != end {
                    return Err(Error("overlapping/uncovered page indices"));
                }
                end = b;
                cursor += 1;
            }
            if end != p.indices {
                return Err(Error("unreferenced page indices"));
            }
            add(&mut counts.payload_bytes, length)?;
        }
        counts.disk_page_bytes = product(self.pages.len(), PAGE_BYTES)?;
        cap(
            counts.disk_page_bytes,
            limits.max_geometry_bytes,
            "virtual geometry budget",
        )?;
        for p in &self.placements {
            if p.mesh as usize >= self.meshes.len() || !valid_transform(&p.transform) {
                return Err(Error("invalid virtual placement"));
            }
        }
        Ok(counts)
    }
}

#[derive(Clone, Debug)]
pub struct PageGeometry {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
