//! Caller-owned immutable metadata, shared page residency and per-placement legal cuts.
//! One asset/package is one explicit geometry pool. A scene submits all its instances together;
//! simultaneous independent pools must be budgeted by their caller. No hidden global cache exists.
mod io;
mod projection;
mod reference;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectorMode {
    #[default]
    Baseline,
    SimpleReference,
}
#[cfg(test)]
pub(crate) mod tests;
use crate::{Camera, Error, Instances, Mat4, Material, Renderer, Texture, asset};
use asset::virtual_mesh::{
    PAGE_BYTES, Package, Page, PageGeometry, PageId, PreparedMesh, PreparedPackage, Selection,
    SelectionBudget, VirtualLimits, VirtualMesh, decode_page,
};
use io::PageIo;
pub use projection::{project_errors, validate_transform};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub const READ_SLOTS: usize = 4;
pub const UPLOADS_PER_FRAME: usize = 2;
pub const METADATA_LIMIT: usize = 8 << 20;
pub const TEXTURE_LIMIT: usize = 64 << 20;

#[derive(Clone, Copy, Debug)]
pub struct StreamOptions {
    pub geometry_bytes: usize,
    pub max_candidates: usize,
    /// Total output across every admitted placement, including offscreen minimum covers.
    pub max_clusters: usize,
    pub error_pixels: f64,
    pub io_delay: Duration,
    pub selector: SelectorMode,
}
impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            geometry_bytes: 8 << 20,
            max_candidates: 1024,
            max_clusters: 65536,
            error_pixels: 1.,
            io_delay: Duration::ZERO,
            selector: SelectorMode::Baseline,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct VirtualPlacement {
    pub mesh: u32,
    pub transform: Mat4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageStatus {
    Missing,
    Queued,
    Ready,
    Uploading,
    Resident,
    Retiring,
    Failed,
}
/// Retained until an internal submission watermark proves its last queue use complete.
pub(crate) struct GpuPage {
    pub mesh: crate::Mesh,
    pub staging: Option<crate::observe::Owned<crate::wgpu::Buffer>>,
    pub upload: u64,
    pub last_use: u64,
}
pub(super) struct PageFailure {
    bytes: [u8; 256],
    length: usize,
}
impl PageFailure {
    fn new(message: &str) -> Self {
        let mut length = message.len().min(256);
        while !message.is_char_boundary(length) {
            length -= 1;
        }
        let mut bytes = [0; 256];
        bytes[..length].copy_from_slice(&message.as_bytes()[..length]);
        Self { bytes, length }
    }
    fn message(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.length]).unwrap()
    }
}
pub(crate) enum PageState {
    Missing,
    Queued,
    Ready(PageGeometry),
    Uploading(GpuPage),
    Resident(GpuPage),
    Retiring(GpuPage),
    Failed(PageFailure),
}
impl PageState {
    fn status(&self) -> PageStatus {
        match self {
            Self::Missing => PageStatus::Missing,
            Self::Queued => PageStatus::Queued,
            Self::Ready(_) => PageStatus::Ready,
            Self::Uploading(_) => PageStatus::Uploading,
            Self::Resident(_) => PageStatus::Resident,
            Self::Retiring(_) => PageStatus::Retiring,
            Self::Failed(_) => PageStatus::Failed,
        }
    }
    pub(crate) fn gpu(&self) -> Option<&GpuPage> {
        match self {
            Self::Uploading(p) | Self::Resident(p) | Self::Retiring(p) => Some(p),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct StreamStats {
    pub projection: Duration,
    pub selection: Duration,
    pub admission: Duration,
    pub completion: Duration,
    pub resources: Duration,
    pub protection: Duration,
    pub stale_ready: Duration,
    pub make_room: Duration,
    pub upload_admission: Duration,
    pub requests: Duration,
    pub snapshot: Duration,
    pub dropped_ready: u64,
    pub io_queue: Duration,
    pub io_read: Duration,
    pub io_verify: Duration,
    pub reads: u64,
    pub uploads: u64,
    pub uploaded_bytes: u64,
    pub retiring_peak: usize,
    pub staging_peak: usize,
    pub inflight_retirements: u64,
    pub evictions: u64,
    pub retries: u64,
    pub failures: u64,
    pub selection_runs: u64,
    pub selection_calls: u64,
    pub retained_cuts: u64,
    /// Logical reserved geometry, including 64KiB page slots and actual staging buffer bytes.
    pub geometry_bytes: usize,
    pub geometry_peak: usize,
    pub published_bytes: usize,
    pub pinned_bytes: usize,
    pub retiring_bytes: usize,
    pub staging_bytes: usize,
    pub owned_gpu_geometry_bytes: usize,
    pub payload_bytes: usize,
    pub metadata_bytes: usize,
    pub scratch_bound_bytes: usize,
    pub texture_bytes: usize,
    pub cpu_ready_bytes: usize,
    pub cpu_io_bound_bytes: usize,
    /// Missing, queued, ready, uploading, resident, retiring, failed.
    pub states: [usize; 7],
    pub cut_clusters: usize,
    pub cut_pages: usize,
    pub candidates: usize,
    pub cluster_visits: usize,
    pub unresolved_groups: usize,
    pub maximum_denied_error: f64,
    pub candidate_debt: bool,
    pub output_debt: bool,
    pub residency_debt: bool,
}
struct SceneAdmission {
    id: u64,
    scratch: usize,
    output: usize,
    output_reservation: usize,
    candidates: usize,
    protected: Vec<PageId>,
}
struct SceneRegistry {
    revision: u64,
    next: u64,
    scenes: Vec<SceneAdmission>,
}
struct SceneLease {
    id: u64,
    registry: Arc<Mutex<SceneRegistry>>,
}
impl Drop for SceneLease {
    fn drop(&mut self) {
        let mut r = self.registry.lock().unwrap();
        r.scenes.retain(|s| s.id != self.id);
        r.revision += 1;
    }
}
pub struct VirtualAsset {
    pub(crate) package: PreparedPackage,
    pub(crate) states: Vec<PageState>,
    io: PageIo,
    roots: Vec<PageId>,
    options: StreamOptions,
    pub(crate) textures: Vec<Texture>,
    pub(crate) materials: Vec<Material>,
    renderer: Option<Arc<()>>,
    pub(crate) identity: Arc<()>,
    revision: u64,
    outstanding: usize,
    stats: StreamStats,
    metadata_bytes: usize,
    registry: Arc<Mutex<SceneRegistry>>,
}
#[derive(PartialEq, Eq)]
struct SelectionKey {
    camera: [u8; 64],
    fov: u32,
    near: u32,
    height: u32,
    revision: u64,
    registry: u64,
    transforms: u64,
}
pub struct VirtualScene {
    pub(crate) asset: Arc<()>,
    lease: SceneLease,
    pub(crate) meshes: Vec<u32>,
    pending_models: Option<Vec<Mat4>>,
    pub(crate) instances: Option<Instances>,
    pub(crate) selections: Vec<Selection>,
    protected: Vec<PageId>,
    key: Option<SelectionKey>,
    continue_selection: bool,
    query_turns: Vec<QueryTurn>,
    transforms: u64,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum QueryTurn {
    #[default]
    Ordinary,
    Refine,
    Rebuild,
}
fn roots(package: &Package) -> Vec<PageId> {
    let mut out = Vec::new();
    for mesh in &package.meshes {
        for group in mesh.terminal_groups() {
            let span = mesh.groups[group.index()].clusters;
            for c in &mesh.clusters[span.offset as usize..(span.offset + span.count) as usize] {
                out.push(c.page);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
fn minimum_clusters(mesh: &VirtualMesh) -> usize {
    mesh.terminal_groups()
        .map(|g| mesh.groups[g.index()].clusters.count as usize)
        .sum()
}
fn union_into(a: &mut Vec<PageId>, b: &[PageId]) {
    a.extend(b);
    a.sort_unstable();
    a.dedup();
}
impl VirtualAsset {
    pub fn open(
        cache: &Path,
        manifest: &Path,
        options: StreamOptions,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(
            io::load(cache, manifest)?,
            cache.join("virtual-v1/pages"),
            options,
        )
        .map_err(Into::into)
    }
    /// Validates, indexes and admits metadata once, before starting any read or GPU allocation.
    pub fn new(
        package: Package,
        page_directory: PathBuf,
        options: StreamOptions,
    ) -> Result<Self, Error> {
        let package = PreparedPackage::new(package, VirtualLimits::default())
            .map_err(|e| Error::InvalidInput(e.0))?;
        if !options.error_pixels.is_finite()
            || options.error_pixels < 0.
            || options.geometry_bytes < PAGE_BYTES
            || options.io_delay > Duration::from_secs(60)
        {
            return Err(Error::InvalidInput(
                "invalid streaming limits/delay (maximum 60 seconds)",
            ));
        }
        let roots = roots(&package);
        if options.geometry_bytes < (roots.len() + 1) * PAGE_BYTES {
            return Err(Error::InvalidInput(
                "geometry capacity below pinned minimum plus one upload staging page",
            ));
        }
        let texture_bytes = package
            .resources
            .textures
            .iter()
            .flat_map(|t| &t.mips)
            .map(|m| m.rgba8.len())
            .sum::<usize>();
        if package
            .resources
            .textures
            .iter()
            .flat_map(|t| &t.mips)
            .map(|m| m.rgba8.capacity())
            .sum::<usize>()
            > TEXTURE_LIMIT
        {
            return Err(Error::InvalidInput(
                "virtual texture capacity exceeds 64MiB",
            ));
        }
        let metadata_owned = package.meshes.capacity() * std::mem::size_of::<VirtualMesh>()
            + package
                .meshes
                .iter()
                .map(|m| {
                    m.groups.capacity() * std::mem::size_of::<asset::virtual_mesh::Group>()
                        + m.clusters.capacity()
                            * std::mem::size_of::<asset::virtual_mesh::Cluster>()
                })
                .sum::<usize>()
            + package.pages.capacity() * std::mem::size_of::<Page>()
            + package.placements.capacity() * std::mem::size_of::<asset::Placement>()
            + package.resources.materials.capacity() * std::mem::size_of::<asset::Material>()
            + package.resources.textures.capacity() * std::mem::size_of::<asset::Texture>()
            + package
                .resources
                .textures
                .iter()
                .map(|t| t.mips.capacity() * std::mem::size_of::<asset::Mip>())
                .sum::<usize>();
        let metadata_bytes = metadata_owned
            + package.index_bytes()
            + package.pages.len() * std::mem::size_of::<PageState>()
            + roots.capacity() * 4
            + 4096;
        if metadata_bytes > METADATA_LIMIT {
            return Err(Error::InvalidInput("virtual retained metadata capacity"));
        }
        let states = (0..package.pages.len())
            .map(|_| PageState::Missing)
            .collect();
        let stats = StreamStats {
            metadata_bytes,
            texture_bytes,
            cpu_io_bound_bytes: (READ_SLOTS + 1) * PAGE_BYTES + 1,
            ..Default::default()
        };
        Ok(Self {
            package,
            states,
            io: PageIo::new(page_directory, options.io_delay),
            roots,
            options,
            textures: vec![],
            materials: vec![],
            renderer: None,
            identity: Arc::new(()),
            revision: 0,
            outstanding: 0,
            stats,
            metadata_bytes,
            registry: Arc::new(Mutex::new(SceneRegistry {
                revision: 0,
                next: 0,
                scenes: vec![],
            })),
        })
    }
    pub fn package(&self) -> &PreparedPackage {
        &self.package
    }
    pub fn options(&self) -> StreamOptions {
        self.options
    }
    pub fn minimum_geometry_bytes(&self) -> usize {
        (self.roots.len() + 1) * PAGE_BYTES
    }
    pub fn status(&self, page: PageId) -> Option<PageStatus> {
        self.states.get(page.index()).map(PageState::status)
    }
    pub fn failure(&self, page: PageId) -> Option<&str> {
        match self.states.get(page.index()) {
            Some(PageState::Failed(e)) => Some(e.message()),
            _ => None,
        }
    }
    /// Explicit repair retry only; failures never loop automatically. Each page has one error slot.
    pub fn retry(&mut self, page: PageId) -> Result<(), Error> {
        let state = self
            .states
            .get_mut(page.index())
            .ok_or(Error::InvalidInput("retry page outside package"))?;
        if matches!(state, PageState::Failed(_)) {
            *state = PageState::Missing;
            self.stats.retries += 1;
            self.revision += 1;
        }
        Ok(())
    }
    pub fn stats(&self) -> &StreamStats {
        &self.stats
    }
    /// Nonblocking progress. Call once per frame/wake before rendering. Roots load through the same
    /// worker and charged upload path as detail. No image is published until minimum cover is usable.
    pub fn advance(
        &mut self,
        renderer: &mut Renderer,
        scene: &mut VirtualScene,
        camera: &Camera,
        height: u32,
    ) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.identity, &scene.asset) {
            return Err(Error::InvalidInput(
                "virtual scene belongs to another asset",
            ));
        }
        renderer.virtual_admit_scene(scene.len())?;
        renderer.virtual_identity(&mut self.renderer)?;
        let section = Instant::now();
        renderer.virtual_collect()?;
        self.complete(renderer.completed_submission());
        while let Some(result) = self.io.receive() {
            self.stats.io_queue += result.queue;
            self.stats.io_read += result.read;
            self.stats.io_verify += result.verify;
            match result.geometry {
                Ok(geometry) => self.states[result.page.index()] = PageState::Ready(geometry),
                Err(e) => {
                    self.states[result.page.index()] = PageState::Failed(PageFailure::new(&e));
                    self.outstanding -= 1;
                    self.stats.failures += 1;
                }
            }
        }
        self.stats.completion += section.elapsed();
        let section = Instant::now();
        renderer.virtual_resources(self)?;
        if scene.instances.is_none()
            && let Some(instances) =
                renderer.try_upload_instances(scene.pending_models.as_ref().unwrap())?
        {
            scene.instances = Some(instances);
            scene.pending_models = None;
        }
        self.stats.resources += section.elapsed();
        let start = Instant::now();
        let available: Vec<_> = self
            .states
            .iter()
            .map(|s| matches!(s, PageState::Resident(_)))
            .collect();
        if self.roots.iter().all(|p| available[p.index()]) {
            scene.select(self, &available, camera, height)?;
        } else {
            scene.protected = self.roots.clone();
        }
        self.stats.admission += start.elapsed();
        let section = Instant::now();
        let mut protected = scene.protected.clone();
        {
            let registry = self.registry.lock().unwrap();
            for entry in &registry.scenes {
                if entry.id != scene.lease.id {
                    union_into(&mut protected, &entry.protected);
                }
            }
        }
        let protected = &protected;
        self.stats.protection += section.elapsed();
        let section = Instant::now();
        // Drop stale CPU completions; queued reads finish into the same bounded slots.
        for (id, state) in self.states.iter_mut().enumerate() {
            if matches!(state, PageState::Ready(_))
                && protected.binary_search(&PageId(id as u32)).is_err()
            {
                *state = PageState::Missing;
                self.outstanding -= 1;
                self.stats.dropped_ready += 1;
            }
        }
        self.stats.stale_ready += section.elapsed();
        let start = Instant::now();
        let mut admitted = 0;
        for &page in protected {
            if admitted == UPLOADS_PER_FRAME {
                break;
            }
            if !matches!(self.states[page.index()], PageState::Ready(_)) {
                continue;
            }
            let charge = PAGE_BYTES + self.package.pages[page.index()].payload_bytes as usize;
            let section = Instant::now();
            self.make_room(charge, protected, renderer.completed_submission());
            self.stats.make_room += section.elapsed();
            if self.geometry_charge() + charge > self.options.geometry_bytes {
                continue;
            }
            let section = Instant::now();
            if let PageState::Ready(data) = &self.states[page.index()]
                && let Some(gpu) = renderer.try_upload_virtual_page(data)?
            {
                // All memory was reserved before renderer allocation. Staging is explicit COPY_SRC.
                self.states[page.index()] = PageState::Uploading(gpu);
                self.outstanding -= 1;
                self.stats.uploads += 1;
                self.stats.uploaded_bytes +=
                    u64::from(self.package.pages[page.index()].payload_bytes);
                admitted += 1;
                self.snapshot();
            }
            self.stats.upload_admission += section.elapsed();
        }
        self.stats.admission += start.elapsed();
        let section = Instant::now();
        for &page in protected {
            if self.outstanding == READ_SLOTS {
                break;
            }
            if matches!(self.states[page.index()], PageState::Missing)
                && self.io.request(page, &self.package.pages[page.index()])
            {
                self.states[page.index()] = PageState::Queued;
                self.outstanding += 1;
                self.stats.reads += 1;
            }
        }
        self.stats.requests += section.elapsed();
        self.snapshot();
        Ok(())
    }
    fn complete(&mut self, completed: u64) {
        for state in &mut self.states {
            match state {
                PageState::Uploading(p) if p.upload <= completed => {
                    let PageState::Uploading(mut p) = std::mem::replace(state, PageState::Missing)
                    else {
                        unreachable!()
                    };
                    p.staging = None;
                    *state = PageState::Resident(p);
                    self.revision += 1;
                }
                PageState::Retiring(p) if p.last_use <= completed => {
                    *state = PageState::Missing;
                }
                _ => {}
            }
        }
    }
    fn geometry_charge(&self) -> usize {
        self.states
            .iter()
            .filter_map(PageState::gpu)
            .map(|p| PAGE_BYTES + p.staging.as_ref().map_or(0, |s| s.size() as usize))
            .sum()
    }
    fn make_room(&mut self, charge: usize, protected: &[PageId], completed: u64) {
        self.snapshot();
        let mut used = self.stats.geometry_bytes;
        let mut retiring = self.stats.retiring_bytes;
        let mut completed_once = false;
        for id in 0..self.states.len() {
            if used + charge <= self.options.geometry_bytes {
                break;
            }
            if protected.binary_search(&PageId(id as u32)).is_ok() {
                continue;
            }
            if matches!(self.states[id], PageState::Resident(_)) {
                let PageState::Resident(p) =
                    std::mem::replace(&mut self.states[id], PageState::Missing)
                else {
                    unreachable!()
                };
                let releasable = p.last_use <= completed;
                let allocation = PAGE_BYTES + p.staging.as_ref().map_or(0, |s| s.size() as usize);
                self.stats.inflight_retirements += u64::from(p.last_use > completed);
                self.states[id] = PageState::Retiring(p);
                self.stats.evictions += 1;
                self.revision += 1;
                // Observe the transient retirement even when its last use is already complete.
                retiring += PAGE_BYTES;
                self.stats.retiring_peak = self.stats.retiring_peak.max(retiring);
                if completed_once && releasable {
                    self.states[id] = PageState::Missing;
                    used -= allocation;
                    retiring -= PAGE_BYTES;
                }
            }
            if !completed_once {
                // Keep the original first-eligible completion point. At this fixed watermark,
                // only the new retirements above can become releasable after this one scan.
                self.complete(completed);
                self.snapshot();
                used = self.stats.geometry_bytes;
                retiring = self.stats.retiring_bytes;
                completed_once = true;
            }
        }
        if completed_once {
            self.snapshot();
        }
    }
    fn snapshot(&mut self) {
        let section = Instant::now();
        self.stats.states = [0; 7];
        self.stats.published_bytes = 0;
        self.stats.retiring_bytes = 0;
        self.stats.staging_bytes = 0;
        self.stats.owned_gpu_geometry_bytes = 0;
        self.stats.cpu_ready_bytes = 0;
        self.stats.payload_bytes = 0;
        for (i, s) in self.states.iter().enumerate() {
            self.stats.states[s.status() as usize] += 1;
            if matches!(s, PageState::Resident(_)) {
                self.stats.published_bytes += PAGE_BYTES;
            }
            if matches!(s, PageState::Retiring(_)) {
                self.stats.retiring_bytes += PAGE_BYTES;
            }
            if let Some(p) = s.gpu() {
                self.stats.owned_gpu_geometry_bytes +=
                    p.mesh.vertex.size() as usize + p.mesh.index.size() as usize;
                self.stats.staging_bytes += p.staging.as_ref().map_or(0, |s| s.size() as usize);
                self.stats.payload_bytes += self.package.pages[i].payload_bytes as usize;
            }
            if let PageState::Ready(g) = s {
                self.stats.cpu_ready_bytes += g.vertices.capacity() * 48 + g.indices.capacity() * 4;
            }
        }
        self.stats.retiring_peak = self.stats.retiring_peak.max(self.stats.retiring_bytes);
        self.stats.staging_peak = self.stats.staging_peak.max(self.stats.staging_bytes);
        self.stats.geometry_bytes = (self.stats.states[PageStatus::Uploading as usize]
            + self.stats.states[PageStatus::Resident as usize]
            + self.stats.states[PageStatus::Retiring as usize])
            * PAGE_BYTES
            + self.stats.staging_bytes;
        self.stats.geometry_peak = self.stats.geometry_peak.max(self.stats.geometry_bytes);
        self.stats.pinned_bytes = self
            .roots
            .iter()
            .filter(|p| self.states[p.index()].gpu().is_some())
            .count()
            * PAGE_BYTES;
        debug_assert!(self.stats.geometry_bytes <= self.options.geometry_bytes);
        debug_assert!(self.outstanding <= READ_SLOTS);
        self.stats.snapshot += section.elapsed();
    }
}
// A previous cut is already a proved complete cover. Keep it while IO is pending only when
// it has less projected debt AND its pages plus next demand fit the same transition budget.
// The metadata selector remains pure and independently comparable; this is publication policy.
struct RetentionBudget<'a> {
    roots: &'a [PageId],
    geometry_bytes: usize,
    max_clusters: usize,
}
fn retain_better_cut(
    previous: Option<&Selection>,
    next: &mut Selection,
    errors: &[f64],
    threshold: f64,
    available: &[bool],
    protected: &[PageId],
    budget: RetentionBudget<'_>,
) -> bool {
    let RetentionBudget {
        roots,
        geometry_bytes,
        max_clusters,
    } = budget;
    let Some(previous) = previous else {
        return false;
    };
    if previous.cut.clusters.len() > max_clusters
        || previous.cut.pages.iter().any(|p| !available[p.index()])
    {
        return false;
    }
    let mut active = vec![false; errors.len()];
    for g in &previous.cut.active_groups {
        active[g.index()] = true;
    }
    let mut denied = 0;
    let mut maximum = 0_f64;
    for (g, &error) in errors.iter().enumerate() {
        if !active[g] && error > threshold {
            denied += 1;
            maximum = maximum.max(error);
        }
    }
    if maximum > next.debt.maximum_denied_error
        || (maximum == next.debt.maximum_denied_error && denied >= next.debt.unresolved_groups)
    {
        return false;
    }
    let mut required = if denied == 0 {
        roots.to_vec()
    } else {
        next.demand.required_pages.clone()
    };
    union_into(&mut required, &previous.cut.pages);
    union_into(&mut required, protected);
    if required.len() * PAGE_BYTES > geometry_bytes {
        return false;
    }
    next.cut = previous.cut.clone();
    next.debt.maximum_denied_error = maximum;
    next.debt.unresolved_groups = denied;
    if denied == 0 {
        next.debt.reasons.clear();
        next.demand.required_pages = roots.to_vec();
        union_into(&mut next.demand.required_pages, &next.cut.pages);
        next.demand.requested_pages.clear();
    }
    true
}

// Keep an already admitted final plan across a bounded query turn.
// Recheck the union against its actual publication: independent refinement can add pages.
fn retain_missing_demand(
    demand: &asset::virtual_mesh::Demand,
    next: &mut Selection,
    available: &[bool],
    protected: &[PageId],
    geometry_bytes: usize,
) -> bool {
    if next.debt.unresolved_groups == 0
        || demand.required_pages.iter().all(|p| available[p.index()])
    {
        return false;
    }
    let mut union = demand.required_pages.clone();
    union_into(&mut union, &next.cut.pages);
    union_into(&mut union, protected);
    if union.len() * PAGE_BYTES > geometry_bytes {
        return false;
    }
    next.demand.required_pages = demand.required_pages.clone();
    next.demand.requested_pages = demand
        .required_pages
        .iter()
        .copied()
        .filter(|p| !available[p.index()])
        .collect();
    if !next
        .debt
        .reasons
        .contains(&asset::virtual_mesh::DebtReason::MissingPages)
    {
        next.debt
            .reasons
            .push(asset::virtual_mesh::DebtReason::MissingPages);
    }
    true
}
#[allow(clippy::too_many_arguments)]
fn run_selector(
    selector: SelectorMode,
    mesh: PreparedMesh<'_>,
    errors: &[f64],
    threshold: f64,
    available: &[bool],
    budget: SelectionBudget,
    protected: &[PageId],
    initial: Option<&asset::virtual_mesh::Cut>,
) -> Result<Selection, Error> {
    let result = match selector {
        SelectorMode::Baseline => mesh
            .select(errors, threshold, available, budget, protected, initial)
            .map_err(|e| Error::InvalidInput(e.0)),
        SelectorMode::SimpleReference => reference::select(
            mesh.mesh(),
            errors,
            threshold,
            available,
            budget,
            protected,
            initial,
        ),
    };
    #[cfg(feature = "selection-shadow")]
    {
        let old = reference::select(
            mesh.mesh(),
            errors,
            threshold,
            available,
            budget,
            protected,
            initial,
        );
        assert_eq!(result.is_ok(), old.is_ok(), "shadow error mismatch");
        if let (Ok(new), Ok(old)) = (&result, old) {
            assert_eq!(new.cut, old.cut, "shadow cut");
            assert_eq!(new.demand, old.demand, "shadow demand");
            assert_eq!(new.debt, old.debt, "shadow debt");
            assert_eq!(new.candidates, old.candidates, "shadow candidates");
            eprintln!(
                "selection_shadow policy=two-track initial={} candidates={} reference_scan_touches={} new_cluster_touches={}",
                initial.is_some(),
                new.candidates,
                old.cluster_visits,
                new.cluster_visits
            );
        }
    }
    result
}

impl VirtualScene {
    /// Admission counts offscreen instances too. Rejected scenes allocate no GPU objects or reads.
    pub fn new(asset: &VirtualAsset, placements: &[VirtualPlacement]) -> Result<Self, Error> {
        if placements.is_empty() {
            return Err(Error::InvalidInput("virtual scene needs placements"));
        }
        let mut minimum = 0usize;
        for p in placements {
            validate_transform(p.transform)?;
            let mesh = asset
                .package
                .meshes
                .get(p.mesh as usize)
                .ok_or(Error::InvalidInput(
                    "virtual placement mesh outside package",
                ))?;
            minimum = minimum
                .checked_add(minimum_clusters(mesh))
                .ok_or(Error::InvalidInput("virtual output overflow"))?;
        }
        if minimum > asset.options.max_clusters {
            return Err(Error::InvalidInput(
                "total output capacity below all-instance minimum cover",
            ));
        }
        // Includes retained results, projection, availability, prepared ranges, and one selector's
        // peak scratch. Capacities clamp to metadata; no allocation from arbitrary CLI limits.
        let largest = asset
            .package
            .meshes
            .iter()
            .map(|m| {
                m.groups.len() * 40 + m.clusters.len() * 48 + asset.package.pages.len() * 16 + 8
            })
            .max()
            .unwrap_or(0);
        let per_instance = asset
            .package
            .meshes
            .iter()
            .map(|m| m.groups.len() * 16)
            .max()
            .unwrap_or(0)
            + asset.package.pages.len() * 32
            + 256
            + std::mem::size_of::<QueryTurn>();
        let output = asset.options.max_clusters.min(
            placements
                .iter()
                .map(|p| asset.package.meshes[p.mesh as usize].clusters.len())
                .sum(),
        );
        let scratch_bound = largest
            .checked_add(
                placements
                    .len()
                    .checked_mul(per_instance)
                    .ok_or(Error::InvalidInput("scene metadata overflow"))?,
            )
            .and_then(|n| n.checked_add(output * 192))
            .and_then(|n| n.checked_add(asset.package.pages.len() * 16))
            .ok_or(Error::InvalidInput("scene scratch overflow"))?;
        let mut registry = asset.registry.lock().unwrap();
        let retained_scratch: usize = registry.scenes.iter().map(|s| s.scratch).sum();
        let retained_output: usize = registry.scenes.iter().map(|s| s.output_reservation).sum();
        if retained_output + minimum > asset.options.max_clusters {
            return Err(Error::InvalidInput(
                "total output below registered scene minimum covers",
            ));
        }
        if asset.metadata_bytes + retained_scratch + scratch_bound > METADATA_LIMIT {
            return Err(Error::InvalidInput(
                "metadata plus retained selection/projection/encoding scratch exceeds 8MiB; reduce output/instances",
            ));
        }
        let id = registry.next;
        registry.next += 1;
        registry.revision += 1;
        registry.scenes.push(SceneAdmission {
            id,
            scratch: scratch_bound,
            output: minimum,
            output_reservation: minimum,
            candidates: 0,
            protected: asset.roots.clone(),
        });
        drop(registry);
        Ok(Self {
            asset: asset.identity.clone(),
            lease: SceneLease {
                id,
                registry: asset.registry.clone(),
            },
            meshes: placements.iter().map(|p| p.mesh).collect(),
            pending_models: Some(placements.iter().map(|p| p.transform).collect()),
            instances: None,
            selections: vec![],
            protected: asset.roots.clone(),
            key: None,
            continue_selection: false,
            query_turns: vec![QueryTurn::Ordinary; placements.len()],
            transforms: 0,
        })
    }
    pub fn from_package(asset: &VirtualAsset) -> Result<Self, Error> {
        Self::new(
            asset,
            &asset
                .package
                .placements
                .iter()
                .map(|p| VirtualPlacement {
                    mesh: p.mesh,
                    transform: Mat4::from_cols_array_2d(&p.transform),
                })
                .collect::<Vec<_>>(),
        )
    }
    pub fn len(&self) -> usize {
        self.meshes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
    pub fn set(&mut self, slot: usize, transform: Mat4) -> Result<(), Error> {
        validate_transform(transform)?;
        if slot >= self.len() {
            return Err(Error::InvalidInput("virtual instance outside scene"));
        }
        let old = self.models()[slot];
        if old.to_cols_array().map(f32::to_bits) == transform.to_cols_array().map(f32::to_bits) {
            return Ok(());
        }
        if let Some(instances) = &mut self.instances {
            instances.set(slot, transform)?;
        } else {
            self.pending_models.as_mut().unwrap()[slot] = transform;
        }
        self.transforms += 1;
        Ok(())
    }
    fn models(&self) -> &[Mat4] {
        self.instances.as_ref().map_or_else(
            || self.pending_models.as_ref().unwrap().as_slice(),
            |i| i.state.models.as_slice(),
        )
    }
    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }
    pub fn ready(&self, asset: &VirtualAsset) -> bool {
        self.instances.is_some()
            && self.selections.len() == self.len()
            && asset.materials.len() == asset.package.resources.materials.len()
    }
    fn select(
        &mut self,
        asset: &mut VirtualAsset,
        available: &[bool],
        camera: &Camera,
        height: u32,
    ) -> Result<(), Error> {
        let key = SelectionKey {
            camera: crate::mesh::matrix_bytes(camera.view()),
            fov: camera.fov_y().to_bits(),
            near: camera.near().to_bits(),
            height,
            revision: asset.revision,
            registry: asset.registry.lock().unwrap().revision,
            transforms: self.transforms,
        };
        if self.key.as_ref() == Some(&key) && !self.continue_selection {
            return Ok(());
        }
        let same_view = self.key.as_ref().is_some_and(|old| {
            old.camera == key.camera
                && old.fov == key.fov
                && old.near == key.near
                && old.height == key.height
                && old.transforms == key.transforms
        });
        let mut reserved_left: usize = self
            .meshes
            .iter()
            .enumerate()
            .map(|(slot, &id)| {
                if same_view {
                    self.selections.get(slot).map(|s| s.cut.clusters.len())
                } else {
                    None
                }
                .unwrap_or(0)
                .max(minimum_clusters(&asset.package.meshes[id as usize]))
            })
            .sum();
        let mut output = 0;
        let mut reserved_output = 0;
        let mut candidates = 0;
        let mut scheduled_query = false;
        let mut protected = asset.roots.clone();
        // Charge the last published cover while planning its replacement. Otherwise a
        // root-rebuilt proposal can evict the better cover we are trying to retain and
        // repeatedly reread representations at a stationary, capacity-limited view.
        for previous in &self.selections {
            union_into(&mut protected, &previous.cut.pages);
        }
        let registry = asset.registry.lock().unwrap();
        let mut other_output = 0;
        let mut other_candidates = 0;
        for entry in &registry.scenes {
            if entry.id != self.lease.id {
                union_into(&mut protected, &entry.protected);
                other_output += entry.output_reservation;
                other_candidates += entry.candidates;
            }
        }
        drop(registry);
        let mut selections = Vec::with_capacity(self.len());
        for (slot, &id) in self.meshes.iter().enumerate() {
            let prepared = asset.package.mesh(id as usize).ok_or(Error::InvalidInput(
                "virtual placement mesh outside package",
            ))?;
            let mesh = prepared.mesh();
            // Same-view queries only refine. Preserve the actual output already owned by later slots.
            reserved_left -= if same_view {
                self.selections
                    .get(slot)
                    .map_or(0, |s| s.cut.clusters.len())
                    .max(minimum_clusters(mesh))
            } else {
                minimum_clusters(mesh)
            };
            let output_budget = asset
                .options
                .max_clusters
                .checked_sub(reserved_output + reserved_left + other_output)
                .ok_or(Error::InvalidInput(
                    "aggregate output reservation exceeds capacity",
                ))?;
            let start = Instant::now();
            let errors = project_errors(mesh, camera, self.models()[slot], height)?;
            asset.stats.projection += start.elapsed();
            let start = Instant::now();
            let budget = SelectionBudget {
                // Reserve staging before planning any resident transition.
                geometry_bytes: asset.options.geometry_bytes - PAGE_BYTES,
                max_candidates: asset
                    .options
                    .max_candidates
                    .saturating_sub(candidates + other_candidates),
                max_clusters: output_budget,
                max_scratch_bytes: METADATA_LIMIT - asset.metadata_bytes,
            };
            let rebuild_turn = self.query_turns[slot] == QueryTurn::Rebuild;
            let refine_turn = self.query_turns[slot] == QueryTurn::Refine
                && budget.max_candidates > 0
                && self.selections.get(slot).is_some_and(|s| {
                    s.cut.clusters.len() <= output_budget
                        && s.cut.pages.iter().all(|p| available[p.index()])
                });
            let initial = self
                .selections
                .get(slot)
                .filter(|_| {
                    refine_turn || (same_view && self.query_turns[slot] != QueryTurn::Rebuild)
                })
                .map(|s| &s.cut);
            // A retained turn owes one root revisit even if motion stops immediately.
            // Otherwise a now-unnecessary fine cover could remain cached forever.
            self.query_turns[slot] = if refine_turn && !same_view {
                scheduled_query = true;
                QueryTurn::Rebuild
            } else {
                QueryTurn::Ordinary
            };
            let mut selection = run_selector(
                asset.options.selector,
                prepared,
                &errors,
                asset.options.error_pixels,
                available,
                budget,
                &protected,
                initial,
            )?;
            asset.stats.selection_calls += 1;
            let coarsen = initial.is_none()
                && budget.max_candidates > 0
                && selection.candidates == budget.max_candidates
                && selection
                    .debt
                    .reasons
                    .contains(&asset::virtual_mesh::DebtReason::CandidateCapacity)
                && self.selections.get(slot).is_some_and(|s| {
                    s.cut
                        .active_groups
                        .iter()
                        .any(|g| errors[g.index()] <= asset.options.error_pixels)
                });
            if initial.is_none()
                // A capped root prefix can have more debt than redundant old detail.
                // Publish that complete fallback once, then refine it on the owed turn.
                && !coarsen
                && retain_better_cut(
                    self.selections.get(slot),
                    &mut selection,
                    &errors,
                    asset.options.error_pixels,
                    available,
                    &protected,
                    RetentionBudget {
                        roots: &asset.roots,
                        geometry_bytes: asset.options.geometry_bytes - PAGE_BYTES,
                        max_clusters: output_budget,
                    },
                )
            {
                asset.stats.retained_cuts += 1;
                if selection.debt.unresolved_groups > 0 {
                    let spent = selection.candidates;
                    let visits = selection.cluster_visits;
                    let budget = SelectionBudget {
                        geometry_bytes: asset.options.geometry_bytes - PAGE_BYTES,
                        max_candidates: asset
                            .options
                            .max_candidates
                            .saturating_sub(candidates + other_candidates + spent),
                        max_clusters: output_budget,
                        max_scratch_bytes: METADATA_LIMIT - asset.metadata_bytes,
                    };
                    if budget.max_candidates > 0 {
                        let mut incremental = run_selector(
                            asset.options.selector,
                            prepared,
                            &errors,
                            asset.options.error_pixels,
                            available,
                            budget,
                            &protected,
                            Some(&selection.cut),
                        )?;
                        asset.stats.selection_calls += 1;
                        if incremental.demand.requested_pages.is_empty() {
                            retain_missing_demand(
                                &selection.demand,
                                &mut incremental,
                                available,
                                &protected,
                                budget.geometry_bytes,
                            );
                        }
                        incremental.candidates += spent;
                        incremental.cluster_visits += visits;
                        selection = incremental;
                    }
                    // A root revisit must not cancel the preceding refinement's IO.
                    // Carry one admitted final plan, never a union of competing plans.
                    if let Some(previous) = self
                        .selections
                        .get(slot)
                        .filter(|_| rebuild_turn || selection.demand.requested_pages.is_empty())
                    {
                        retain_missing_demand(
                            &previous.demand,
                            &mut selection,
                            available,
                            &protected,
                            budget.geometry_bytes,
                        );
                    }
                }
            }
            if initial.is_none()
                && budget.max_candidates > 0
                && selection.candidates == budget.max_candidates
                && selection
                    .debt
                    .reasons
                    .contains(&asset::virtual_mesh::DebtReason::CandidateCapacity)
            {
                // Even a newly published root prefix needs a refinement turn. Repeating
                // that same capped root query on every changed view cannot advance it.
                self.query_turns[slot] = QueryTurn::Refine;
                scheduled_query |= rebuild_turn;
            }
            asset.stats.selection += start.elapsed();
            output += selection.cut.clusters.len();
            reserved_output += selection.cut.clusters.len().max(minimum_clusters(mesh));
            candidates += selection.candidates;
            union_into(&mut protected, &selection.cut.pages);
            union_into(&mut protected, &selection.demand.required_pages);
            selections.push(selection);
        }
        // Resume capped work only after a useful publication or a new view. A stalled
        // missing/capacity-denied proposal stays cached until availability changes.
        self.continue_selection = scheduled_query
            || (selections.iter().any(|s| {
                s.debt
                    .reasons
                    .contains(&asset::virtual_mesh::DebtReason::CandidateCapacity)
            }) && (!same_view
                || self
                    .selections
                    .iter()
                    .map(|s| &s.cut)
                    .ne(selections.iter().map(|s| &s.cut))));
        self.selections = selections;
        // Store this scene's own pages; other scenes' demands are unioned at admission time.
        let mut own = asset.roots.clone();
        for selection in &self.selections {
            union_into(&mut own, &selection.cut.pages);
            union_into(&mut own, &selection.demand.required_pages);
        }
        self.protected = own;
        self.key = Some(key);
        let mut registry = asset.registry.lock().unwrap();
        let entry = registry
            .scenes
            .iter_mut()
            .find(|s| s.id == self.lease.id)
            .unwrap();
        let changed = entry.output != output
            || entry.output_reservation != reserved_output
            || entry.candidates != candidates
            || entry.protected != self.protected;
        entry.output = output;
        entry.output_reservation = reserved_output;
        entry.candidates = candidates;
        entry.protected = self.protected.clone();
        if changed {
            registry.revision += 1;
        }
        self.key.as_mut().unwrap().registry = registry.revision;
        let aggregate_output = registry.scenes.iter().map(|s| s.output).sum();
        let aggregate_candidates = registry.scenes.iter().map(|s| s.candidates).sum();
        let aggregate_scratch = registry.scenes.iter().map(|s| s.scratch).sum();
        drop(registry);
        let stats = &mut asset.stats;
        stats.selection_runs += 1;
        stats.cut_clusters = aggregate_output;
        stats.candidates = aggregate_candidates;
        stats.cut_pages = self.protected.len();
        stats.scratch_bound_bytes = aggregate_scratch;
        stats.cluster_visits = self.selections.iter().map(|s| s.cluster_visits).sum();
        stats.unresolved_groups = self
            .selections
            .iter()
            .map(|s| s.debt.unresolved_groups)
            .sum();
        stats.maximum_denied_error = self
            .selections
            .iter()
            .map(|s| s.debt.maximum_denied_error)
            .fold(0., f64::max);
        use asset::virtual_mesh::DebtReason;
        stats.candidate_debt = self
            .selections
            .iter()
            .any(|s| s.debt.reasons.contains(&DebtReason::CandidateCapacity));
        stats.output_debt = self
            .selections
            .iter()
            .any(|s| s.debt.reasons.contains(&DebtReason::OutputCapacity));
        stats.residency_debt = self.selections.iter().any(|s| {
            s.debt.reasons.contains(&DebtReason::GeometryCapacity)
                || s.debt.reasons.contains(&DebtReason::MissingPages)
        });
        Ok(())
    }
}
