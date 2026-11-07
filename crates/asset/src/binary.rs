use crate::*;

pub const SCHEMA_VERSION: u32 = 1;
pub const NORMAL_SCHEMA_VERSION: u32 = 2;
pub const NORMAL_VARIANCE_SCHEMA_VERSION: u32 = 3;
/// R0c's existing optional acceleration trailer, unchanged.
pub const CLUSTER_BVH_SCHEMA_VERSION: u32 = 4;
/// Slope moments without acceleration. Historical U1b v4 remains read-compatible.
pub const NORMAL_MOMENTS_SCHEMA_VERSION: u32 = 5;
/// Slope moments plus the same acceleration trailer as v4.
pub const NORMAL_MOMENTS_BVH_SCHEMA_VERSION: u32 = 6;
const MAGIC: &[u8; 4] = b"RLA\0";

struct Writer(Vec<u8>);
impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn count(&mut self, v: usize) -> Result<()> {
        self.u32(v.try_into().map_err(|_| Error("u32 count overflow"))?);
        Ok(())
    }
    fn floats(&mut self, v: &[f32]) {
        for f in v {
            self.u32(f.to_bits());
        }
    }
    fn mesh(&mut self, mesh: &Mesh, version: u32) -> Result<()> {
        self.count(mesh.primitives.len())?;
        for p in &mesh.primitives {
            self.u32(p.material);
            self.count(p.vertices.len())?;
            self.count(p.indices.len())?;
            if version >= 2 {
                self.u32(match p.tangents.as_ref().map(|t| t.basis) {
                    None => 0,
                    Some(TangentBasis::Authored) => 1,
                    Some(TangentBasis::MikkTSpace) => 2,
                });
                self.count(p.tangents.as_ref().map_or(0, |t| t.values.len()))?;
            }
            for v in &p.vertices {
                self.floats(&v.position);
                self.floats(&v.normal);
                self.floats(&v.uv);
                self.floats(&v.color);
            }
            if let Some(t) = &p.tangents {
                for value in &t.values {
                    self.floats(value);
                }
            }
            for &i in &p.indices {
                self.u32(i);
            }
        }
        Ok(())
    }
    fn reference(&mut self, reference: Option<(u32, Sampler)>) {
        let Some((index, s)) = reference else {
            self.u32(0);
            return;
        };
        self.u32(1);
        self.u32(index);
        let wrap = |w| match w {
            Wrap::Clamp => 0,
            Wrap::Mirror => 1,
            Wrap::Repeat => 2,
        };
        let filter = |f| match f {
            Filter::Nearest => 0,
            Filter::Linear => 1,
        };
        self.u32(wrap(s.wrap_u));
        self.u32(wrap(s.wrap_v));
        self.u32(filter(s.mag));
        self.u32(filter(s.min));
        self.u32(match s.mipmap {
            MipmapFilter::None => 0,
            MipmapFilter::Nearest => 1,
            MipmapFilter::Linear => 2,
        });
    }
}

/// Deterministic little-endian format, independent of Rust's struct layout.
pub fn encode(asset: &Asset, limits: Limits) -> Result<Vec<u8>> {
    encode_with_bvh(asset, None, limits)
}
pub fn encode_with_bvh(
    asset: &Asset,
    bvh: Option<&cluster_bvh::Bvh>,
    limits: Limits,
) -> Result<Vec<u8>> {
    let counts = asset.validate(limits)?;
    let mut w = Writer(MAGIC.to_vec());
    let version = if asset
        .textures
        .iter()
        .any(|t| t.role == TextureRole::NormalSlopeMoments)
    {
        if bvh.is_some() {
            NORMAL_MOMENTS_BVH_SCHEMA_VERSION
        } else {
            NORMAL_MOMENTS_SCHEMA_VERSION
        }
    } else if bvh.is_some() {
        CLUSTER_BVH_SCHEMA_VERSION
    } else if asset
        .textures
        .iter()
        .any(|t| t.role == TextureRole::NormalVarianceLinear)
    {
        NORMAL_VARIANCE_SCHEMA_VERSION
    } else if asset.extended() {
        NORMAL_SCHEMA_VERSION
    } else {
        SCHEMA_VERSION
    };
    w.u32(version);
    for n in [
        asset.meshes.len(),
        asset.placements.len(),
        asset.materials.len(),
        asset.textures.len(),
    ] {
        w.count(n)?;
    }
    for mesh in &asset.meshes {
        w.mesh(mesh, version)?;
    }
    for p in &asset.placements {
        w.u32(p.mesh);
        for column in &p.transform {
            w.floats(column);
        }
    }
    for m in &asset.materials {
        w.floats(&m.base_color);
        w.floats(&[m.metallic, m.roughness]);
        w.u32(u32::from(m.double_sided));
        w.reference(m.base_color_texture.map(|t| (t.texture, t.sampler)));
        w.reference(m.metallic_roughness_texture.map(|t| (t.texture, t.sampler)));
        if version >= 2 {
            w.reference(m.normal_texture.map(|t| (t.texture, t.sampler)));
            w.floats(&[m.normal_texture.map_or(1., |t| t.scale)]);
        }
    }
    for t in &asset.textures {
        w.u32(match t.role {
            TextureRole::BaseColorSrgb => 0,
            TextureRole::MetallicRoughnessLinear => 1,
            TextureRole::NormalLinear => 2,
            TextureRole::NormalVarianceLinear => 3,
            TextureRole::NormalSlopeMoments => 4,
        });
        w.count(t.mips.len())?;
        for m in &t.mips {
            w.u32(m.width);
            w.u32(m.height);
            w.count(m.rgba8.len())?;
            w.0.extend(&m.rgba8);
        }
    }
    if let Some(bvh) = bvh {
        if bvh.leaves.iter().any(|l| {
            l.mesh as usize >= asset.meshes.len() || l.material as usize >= asset.materials.len()
        }) {
            return Err(Error("prepared BVH mesh/material identity"));
        }
        let bytes = cluster_bvh::encode(
            bvh,
            cluster_bvh::Limits {
                max_bytes: limits
                    .max_decoded_bytes
                    .saturating_sub(counts.decoded_bytes),
            },
        )?;
        w.count(bytes.len())?;
        w.0.extend(bytes);
    }
    cap(w.0.len(), limits.max_file_bytes, "prepared file byte limit")?;
    Ok(w.0)
}

/// Stable geometry sidecar identity. Placements and textures are excluded.
pub fn encode_mesh(mesh: &Mesh, limits: Limits) -> Result<Vec<u8>> {
    validate_mesh(mesh, &mut Counts::default(), limits)?;
    let mut w = Writer(b"RLG\0".to_vec());
    let version = if mesh.primitives.iter().any(|p| p.tangents.is_some()) {
        2
    } else {
        1
    };
    w.u32(version);
    w.mesh(mesh, version)?;
    cap(w.0.len(), limits.max_file_bytes, "geometry file byte limit")?;
    Ok(w.0)
}

struct Reader<'a> {
    data: &'a [u8],
    decoded: usize,
    limits: Limits,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let (head, tail) = self
            .data
            .split_at_checked(n)
            .ok_or(Error("truncated prepared asset"))?;
        self.data = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self, max: usize, min_bytes: usize, allocation: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        cap(n, max, "prepared count limit")?;
        cap(
            product(n, min_bytes)?,
            self.data.len(),
            "truncated prepared count",
        )?;
        add(&mut self.decoded, product(n, allocation)?)?;
        cap(
            self.decoded,
            self.limits.max_decoded_bytes,
            "decoded byte limit",
        )?;
        Ok(n)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut result = [0.0; N];
        for v in &mut result {
            *v = f32::from_bits(self.u32()?);
            if !v.is_finite() {
                return Err(Error("nonfinite prepared scalar"));
            }
        }
        Ok(result)
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error("invalid boolean")),
        }
    }
    fn reference(&mut self) -> Result<Option<(u32, Sampler)>> {
        if !self.boolean()? {
            return Ok(None);
        }
        let index = self.u32()?;
        fn wrap(r: &mut Reader<'_>) -> Result<Wrap> {
            match r.u32()? {
                0 => Ok(Wrap::Clamp),
                1 => Ok(Wrap::Mirror),
                2 => Ok(Wrap::Repeat),
                _ => Err(Error("invalid wrap")),
            }
        }
        fn filter(r: &mut Reader<'_>) -> Result<Filter> {
            match r.u32()? {
                0 => Ok(Filter::Nearest),
                1 => Ok(Filter::Linear),
                _ => Err(Error("invalid filter")),
            }
        }
        Ok(Some((
            index,
            Sampler {
                wrap_u: wrap(self)?,
                wrap_v: wrap(self)?,
                mag: filter(self)?,
                min: filter(self)?,
                mipmap: match self.u32()? {
                    0 => MipmapFilter::None,
                    1 => MipmapFilter::Nearest,
                    2 => MipmapFilter::Linear,
                    _ => return Err(Error("invalid mip filter")),
                },
            },
        )))
    }
}

/// Decode with checked spans and aggregate budgets before every allocation.
pub fn decode(bytes: &[u8], limits: Limits) -> Result<Asset> {
    Ok(decode_with_bvh(bytes, limits)?.0)
}
/// Returns the optional acceleration attachment; ordinary decode validates it,
/// then discards it. Consumers opt in explicitly rather than changing Asset ownership.
pub fn decode_with_bvh(bytes: &[u8], limits: Limits) -> Result<(Asset, Option<cluster_bvh::Bvh>)> {
    cap(
        bytes.len(),
        limits.max_file_bytes,
        "prepared file byte limit",
    )?;
    let mut r = Reader {
        data: bytes,
        decoded: 0,
        limits,
    };
    if r.take(4)? != MAGIC {
        return Err(Error("unknown prepared schema"));
    }
    let version = r.u32()?;
    if !matches!(version, 1..=6) {
        return Err(Error("unknown prepared schema"));
    }
    let meshes = r.count(limits.max_meshes, 4, std::mem::size_of::<Mesh>())?;
    let placements = r.count(limits.max_placements, 68, std::mem::size_of::<Placement>())?;
    let materials = r.count(limits.max_materials, 36, std::mem::size_of::<Material>())?;
    let textures = r.count(limits.max_textures, 8, std::mem::size_of::<Texture>())?;
    let mut a = Asset {
        meshes: Vec::with_capacity(meshes),
        placements: Vec::with_capacity(placements),
        materials: Vec::with_capacity(materials),
        textures: Vec::with_capacity(textures),
    };
    let (mut primitives, mut vertices, mut indices) = (0, 0, 0);
    for _ in 0..meshes {
        let count = r.count(
            limits.max_primitives.saturating_sub(primitives),
            12,
            std::mem::size_of::<Primitive>(),
        )?;
        add(&mut primitives, count)?;
        let mut mesh = Mesh {
            primitives: Vec::with_capacity(count),
        };
        for _ in 0..count {
            let material = r.u32()?;
            let nv = r.count(limits.max_vertices.saturating_sub(vertices), 48, 48)?;
            let ni = r.count(limits.max_indices.saturating_sub(indices), 4, 4)?;
            let tangents = if version >= 2 {
                let basis = match r.u32()? {
                    0 => None,
                    1 => Some(TangentBasis::Authored),
                    2 => Some(TangentBasis::MikkTSpace),
                    _ => return Err(Error("invalid tangent basis")),
                };
                let count = r.count(nv, 16, 16)?;
                if count != basis.map_or(0, |_| nv) {
                    return Err(Error("invalid tangent count"));
                }
                basis
            } else {
                None
            };
            add(&mut vertices, nv)?;
            add(&mut indices, ni)?;
            cap(
                product(nv, 48)?
                    .checked_add(product(ni, 4)?)
                    .and_then(|n| {
                        n.checked_add(if tangents.is_some() {
                            product(nv, 16).ok()?
                        } else {
                            0
                        })
                    })
                    .ok_or(Error("size overflow"))?,
                r.data.len(),
                "truncated primitive",
            )?;
            let mut p = Primitive {
                vertices: Vec::with_capacity(nv),
                tangents: None,
                indices: Vec::with_capacity(ni),
                material,
            };
            for _ in 0..nv {
                p.vertices.push(Vertex {
                    position: r.floats()?,
                    normal: r.floats()?,
                    uv: r.floats()?,
                    color: r.floats()?,
                });
            }
            if let Some(basis) = tangents {
                let mut values = Vec::with_capacity(nv);
                for _ in 0..nv {
                    values.push(r.floats()?);
                }
                p.tangents = Some(TangentStream { basis, values });
            }
            for _ in 0..ni {
                let i = r.u32()?;
                if i as usize >= nv {
                    return Err(Error("invalid vertex index"));
                }
                p.indices.push(i);
            }
            mesh.primitives.push(p);
        }
        a.meshes.push(mesh);
    }
    for _ in 0..placements {
        let mesh = r.u32()?;
        a.placements.push(Placement {
            mesh,
            transform: [r.floats()?, r.floats()?, r.floats()?, r.floats()?],
        });
    }
    for _ in 0..materials {
        let base_color = r.floats()?;
        let [metallic, roughness] = r.floats()?;
        let double_sided = r.boolean()?;
        let base_color_texture = r
            .reference()?
            .map(|(texture, sampler)| BaseColorTexture { texture, sampler });
        let metallic_roughness_texture = r
            .reference()?
            .map(|(texture, sampler)| MetallicRoughnessTexture { texture, sampler });
        let normal_texture = if version >= 2 {
            let reference = r.reference()?;
            let [scale] = r.floats()?;
            if reference.is_none() && scale != 1. {
                return Err(Error("noncanonical absent normal scale"));
            }
            reference.map(|(texture, sampler)| NormalTexture {
                texture,
                sampler,
                scale,
            })
        } else {
            None
        };
        a.materials.push(Material {
            normal_texture,
            base_color,
            metallic,
            roughness,
            double_sided,
            base_color_texture,
            metallic_roughness_texture,
        });
    }
    for _ in 0..textures {
        let role = match r.u32()? {
            0 => TextureRole::BaseColorSrgb,
            1 => TextureRole::MetallicRoughnessLinear,
            2 if version >= 2 => TextureRole::NormalLinear,
            3 if version >= 3 => TextureRole::NormalVarianceLinear,
            4 if version == 4 || version >= NORMAL_MOMENTS_SCHEMA_VERSION => {
                TextureRole::NormalSlopeMoments
            }
            _ => return Err(Error("invalid texture role")),
        };
        let n = r.count(32, 12, std::mem::size_of::<Mip>())?;
        if n == 0 {
            return Err(Error("empty mip chain"));
        }
        let mut t = Texture {
            role,
            mips: Vec::with_capacity(n),
        };
        let mut expected = None;
        for _ in 0..n {
            let (w, h) = (r.u32()?, r.u32()?);
            if w == 0 || h == 0 || w > limits.max_dimension || h > limits.max_dimension {
                return Err(Error("texture dimension limit"));
            }
            if let Some(pair) = expected
                && pair != (w, h)
            {
                return Err(Error("invalid mip dimensions"));
            }
            if w == 1 && h == 1 && t.mips.len() + 1 != n {
                return Err(Error("mips after 1x1"));
            }
            if t.mips.len() + 1 == n && (w != 1 || h != 1) {
                return Err(Error("incomplete mip chain"));
            }
            let size = product(product(w as usize, h as usize)?, role.texel_bytes())?;
            let length = r.count(size, 1, 1)?;
            if size != length {
                return Err(Error("invalid mip byte count"));
            }
            t.mips.push(Mip {
                width: w,
                height: h,
                rgba8: r.take(size)?.to_vec(),
            });
            expected = Some(((w / 2).max(1), (h / 2).max(1)));
        }
        a.textures.push(t);
    }
    let bvh =
        if version == NORMAL_MOMENTS_BVH_SCHEMA_VERSION
            || (version == CLUSTER_BVH_SCHEMA_VERSION
                && !a
                    .textures
                    .iter()
                    .any(|t| t.role == TextureRole::NormalSlopeMoments))
        {
            let length = r.u32()? as usize;
            let remaining = limits.max_decoded_bytes.saturating_sub(r.decoded);
            let bvh = cluster_bvh::decode(
                r.take(length)?,
                cluster_bvh::Limits {
                    max_bytes: remaining,
                },
            )?;
            if bvh.leaves.iter().any(|l| {
                l.mesh as usize >= a.meshes.len() || l.material as usize >= a.materials.len()
            }) {
                return Err(Error("prepared BVH mesh/material identity"));
            }
            Some(bvh)
        } else {
            None
        };
    if !r.data.is_empty() {
        return Err(Error("trailing prepared bytes"));
    }
    a.validate(limits)?;
    Ok((a, bvh))
}
