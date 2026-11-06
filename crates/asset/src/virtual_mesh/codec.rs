use super::*;

fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}
fn count(out: &mut Vec<u8>, value: usize) -> Result<()> {
    u32v(
        out,
        value
            .try_into()
            .map_err(|_| Error("virtual u32 count overflow"))?,
    );
    Ok(())
}
fn bounds(out: &mut Vec<u8>, b: Bounds) {
    for f in b.center.into_iter().chain([b.radius, b.error]) {
        u32v(out, f.to_bits());
    }
}

/// Metadata only. Page files are immutable siblings addressed by SHA-256, never embedded geometry.
pub fn encode(p: &Package, limits: VirtualLimits) -> Result<Vec<u8>> {
    p.validate(limits)?;
    let resources = crate::encode(&p.resources, limits.prepared)?;
    let mut out = b"RLV\0".to_vec();
    let version = p.version();
    u32v(&mut out, version);
    for n in [
        p.meshes.len(),
        p.placements.len(),
        p.pages.len(),
        resources.len(),
    ] {
        count(&mut out, n)?;
    }
    out.extend(resources);
    for m in &p.meshes {
        count(&mut out, m.groups.len())?;
        count(&mut out, m.clusters.len())?;
        for g in &m.groups {
            u32v(&mut out, g.depth);
            bounds(&mut out, g.simplified);
            u32v(&mut out, g.clusters.offset);
            u32v(&mut out, g.clusters.count);
        }
        for c in &m.clusters {
            for v in [c.owner.0, c.refined.map_or(u32::MAX, |v| v.0)] {
                u32v(&mut out, v);
            }
            bounds(&mut out, c.bounds);
            for v in [c.material, c.page.0, c.indices.offset, c.indices.count] {
                u32v(&mut out, v);
            }
        }
    }
    for p in &p.placements {
        u32v(&mut out, p.mesh);
        for f in p.transform.iter().flatten() {
            u32v(&mut out, f.to_bits());
        }
    }
    for p in &p.pages {
        for v in [p.vertices, p.indices, p.payload_bytes] {
            u32v(&mut out, v);
        }
        if version == 2 {
            u32v(
                &mut out,
                match p.tangents {
                    None => 0,
                    Some(crate::TangentBasis::Authored) => 1,
                    Some(crate::TangentBasis::MikkTSpace) => 2,
                },
            );
        }
        out.extend(p.sha256);
    }
    cap(
        out.len(),
        limits.prepared.max_file_bytes,
        "virtual metadata file limit",
    )?;
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    allocated: usize,
    limit: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let (a, b) = self
            .data
            .split_at_checked(n)
            .ok_or(Error("truncated virtual package"))?;
        self.data = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn n(&mut self, max: usize, min: usize, size: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        cap(n, max, "virtual count limit")?;
        cap(product(n, min)?, self.data.len(), "truncated virtual count")?;
        add(&mut self.allocated, product(n, size)?)?;
        cap(
            self.allocated,
            self.limit,
            "virtual decode allocation limit",
        )?;
        Ok(n)
    }
    fn float(&mut self) -> Result<f32> {
        let f = f32::from_bits(self.u32()?);
        if !f.is_finite() {
            return Err(Error("nonfinite virtual scalar"));
        }
        Ok(f)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.; N];
        for v in &mut out {
            *v = self.float()?;
        }
        Ok(out)
    }
    fn bounds(&mut self) -> Result<Bounds> {
        Ok(Bounds {
            center: self.floats()?,
            radius: self.float()?,
            error: self.float()?,
        })
    }
    fn span(&mut self) -> Result<Span> {
        Ok(Span {
            offset: self.u32()?,
            count: self.u32()?,
        })
    }
}

/// Counts and byte budgets are charged before allocating. The full DAG is validated before return.
pub fn decode(bytes: &[u8], limits: VirtualLimits) -> Result<Package> {
    cap(
        bytes.len(),
        limits.prepared.max_file_bytes,
        "virtual metadata file limit",
    )?;
    let mut r = Reader {
        data: bytes,
        allocated: 0,
        limit: limits.max_metadata_bytes,
    };
    if r.take(4)? != b"RLV\0" {
        return Err(Error("unknown virtual schema"));
    }
    let version = r.u32()?;
    if !matches!(version, 1 | 2) {
        return Err(Error("unknown virtual schema"));
    }
    let nm = r.n(
        limits.prepared.max_meshes,
        8,
        std::mem::size_of::<VirtualMesh>(),
    )?;
    let nt = r.n(
        limits.prepared.max_placements,
        68,
        std::mem::size_of::<Placement>(),
    )?;
    let np = r.n(limits.max_pages, 44, std::mem::size_of::<Page>())?;
    let nr = r.u32()? as usize;
    cap(
        nr,
        limits.prepared.max_file_bytes,
        "virtual resource file limit",
    )?;
    let resource_bytes = r.take(nr)?;
    if resource_bytes.len() < 24 || resource_bytes[8..16] != [0; 8] {
        return Err(Error("virtual resources contain geometry or are truncated"));
    }
    if version == 1 && resource_bytes[4..8] != 1u32.to_le_bytes() {
        return Err(Error("extended resources in legacy virtual schema"));
    }
    let resources = crate::decode(resource_bytes, limits.prepared)?;
    if !resources.meshes.is_empty() || !resources.placements.is_empty() {
        return Err(Error("virtual resources contain geometry"));
    }
    let resource_metadata = Package {
        meshes: vec![],
        pages: vec![],
        placements: vec![],
        resources,
    };
    add(
        &mut r.allocated,
        resource_metadata.resource_metadata_bytes()?,
    )?;
    cap(
        r.allocated,
        r.limit,
        "virtual decode resource metadata limit",
    )?;
    let resources = resource_metadata.resources;
    let mut meshes = Vec::with_capacity(nm);
    let mut ng = 0;
    let mut nc = 0;
    for _ in 0..nm {
        let groups = r.n(
            limits.max_groups.saturating_sub(ng),
            32,
            std::mem::size_of::<Group>(),
        )?;
        let clusters = r.n(
            limits.max_clusters.saturating_sub(nc),
            44,
            std::mem::size_of::<Cluster>(),
        )?;
        add(&mut ng, groups)?;
        add(&mut nc, clusters)?;
        let mut m = VirtualMesh {
            groups: Vec::with_capacity(groups),
            clusters: Vec::with_capacity(clusters),
        };
        for _ in 0..groups {
            m.groups.push(Group {
                depth: r.u32()?,
                simplified: r.bounds()?,
                clusters: r.span()?,
            });
        }
        for _ in 0..clusters {
            let owner = GroupId(r.u32()?);
            let raw = r.u32()?;
            m.clusters.push(Cluster {
                owner,
                refined: (raw != u32::MAX).then_some(GroupId(raw)),
                bounds: r.bounds()?,
                material: r.u32()?,
                page: PageId(r.u32()?),
                indices: r.span()?,
            });
        }
        meshes.push(m);
    }
    let mut placements = Vec::with_capacity(nt);
    for _ in 0..nt {
        placements.push(Placement {
            mesh: r.u32()?,
            transform: [r.floats()?, r.floats()?, r.floats()?, r.floats()?],
        });
    }
    let mut pages = Vec::with_capacity(np);
    for _ in 0..np {
        let vertices = r.u32()?;
        let indices = r.u32()?;
        let payload_bytes = r.u32()?;
        let tangents = if version == 2 {
            match r.u32()? {
                0 => None,
                1 => Some(crate::TangentBasis::Authored),
                2 => Some(crate::TangentBasis::MikkTSpace),
                _ => return Err(Error("invalid page tangent basis")),
            }
        } else {
            None
        };
        pages.push(Page {
            tangents,
            vertices,
            indices,
            payload_bytes,
            sha256: r.take(32)?.try_into().unwrap(),
        });
    }
    if !r.data.is_empty() {
        return Err(Error("trailing virtual bytes"));
    }
    let p = Package {
        meshes,
        pages,
        placements,
        resources,
    };
    p.validate(limits)?;
    Ok(p)
}

/// Caller supplies SHA-256 of the entire 64KiB file; the dependency-free codec checks it.
/// Runtime IO must hash the actual bytes, then call this before publishing availability.
pub fn decode_page(p: &Page, bytes: &[u8], actual_sha256: [u8; 32]) -> Result<PageGeometry> {
    let size = product(
        p.vertices as usize,
        if p.tangents.is_some() { 64 } else { 48 },
    )?
    .checked_add(product(p.indices as usize, 4)?)
    .ok_or(Error("page byte overflow"))?;
    if bytes.len() != PAGE_BYTES
        || size != p.payload_bytes as usize
        || size > PAGE_BYTES
        || p.vertices == 0
        || p.indices == 0
        || !p.indices.is_multiple_of(3)
        || p.sha256 != actual_sha256
    {
        return Err(Error("page extent/digest mismatch"));
    }
    if bytes[size..].iter().any(|&b| b != 0) {
        return Err(Error("nonzero page padding"));
    }
    let mut r = Reader {
        data: &bytes[..size],
        allocated: 0,
        limit: PAGE_BYTES,
    };
    let mut vertices = Vec::with_capacity(p.vertices as usize);
    let mut indices = Vec::with_capacity(p.indices as usize);
    for _ in 0..p.vertices {
        vertices.push(Vertex {
            position: r.floats()?,
            normal: r.floats()?,
            uv: r.floats()?,
            color: r.floats()?,
        });
    }
    let tangents = if let Some(basis) = p.tangents {
        let mut values = Vec::with_capacity(p.vertices as usize);
        for _ in 0..p.vertices {
            values.push(r.floats()?);
        }
        Some(crate::TangentStream { basis, values })
    } else {
        None
    };
    for _ in 0..p.indices {
        indices.push(r.u32()?);
    }
    let mut mesh = crate::Mesh {
        primitives: vec![crate::Primitive {
            adjacency: None,
            tangents,
            vertices,
            indices,
            material: 0,
        }],
    };
    crate::validate_mesh(
        &mesh,
        &mut crate::Counts::default(),
        Limits {
            max_decoded_bytes: PAGE_BYTES,
            ..Limits::default()
        },
    )?;
    let p = mesh.primitives.pop().unwrap();
    Ok(PageGeometry {
        tangents: p.tangents,
        vertices: p.vertices,
        indices: p.indices,
    })
}
