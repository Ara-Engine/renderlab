//! Owned prepared assets. No GPU, window, importer, codecs or math crate.
mod binary;
pub mod environment;
pub mod virtual_mesh;

pub use binary::{NORMAL_SCHEMA_VERSION, SCHEMA_VERSION, decode, encode, encode_mesh};
use std::fmt;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asset {
    pub meshes: Vec<Mesh>,
    pub placements: Vec<Placement>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub tangents: Option<TangentStream>,
    pub indices: Vec<u32>,
    pub material: u32,
}

/// One frame per base vertex, in the same indexed domain.
#[derive(Clone, Debug, PartialEq)]
pub struct TangentStream {
    pub basis: TangentBasis,
    pub values: Vec<[f32; 4]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TangentBasis {
    Authored,
    MikkTSpace,
}

/// Validate external frames without changing an authored basis or averaging a seam.
pub fn validate_tangents(
    vertices: &[Vertex],
    indices: &[u32],
    stream: &TangentStream,
) -> Result<()> {
    if stream.values.len() != vertices.len() {
        return Err(Error("tangent count must equal vertex count"));
    }
    for (v, t) in vertices.iter().zip(&stream.values) {
        let dot = |a: &[f32], b: &[f32]| {
            a.iter()
                .zip(b)
                .map(|(&a, &b)| f64::from(a) * f64::from(b))
                .sum::<f64>()
        };
        if !t.iter().chain(&v.normal).all(|f| f.is_finite())
            || (dot(&t[..3], &t[..3]) - 1.).abs() > 2e-4
            || (dot(&v.normal, &v.normal) - 1.).abs() > 2e-4
            || dot(&t[..3], &v.normal).abs() > 2e-4
            || !matches!(t[3], -1. | 1.)
        {
            return Err(Error("invalid unit tangent frame or handedness"));
        }
    }
    for tri in indices.chunks_exact(3) {
        let first = stream
            .values
            .get(tri[0] as usize)
            .ok_or(Error("invalid tangent index"))?[3];
        for &i in &tri[1..] {
            if stream
                .values
                .get(i as usize)
                .ok_or(Error("invalid tangent index"))?[3]
                != first
            {
                return Err(Error("triangle crosses tangent handedness seam"));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Linear RGBA. Opaque materials ignore alpha when shading.
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub mesh: u32,
    /// Column-major, composed hierarchy transform. Finite, nonsingular affine.
    pub transform: [[f32; 4]; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub base_color_texture: Option<BaseColorTexture>,
    pub metallic_roughness_texture: Option<MetallicRoughnessTexture>,
    pub normal_texture: Option<NormalTexture>,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseColorTexture {
    pub texture: u32,
    pub sampler: Sampler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetallicRoughnessTexture {
    pub texture: u32,
    pub sampler: Sampler,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalTexture {
    pub texture: u32,
    pub sampler: Sampler,
    /// Linear tangent-space RGB, +Y along W * cross(N, T). Scale XY before normalization.
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureRole {
    BaseColorSrgb,
    MetallicRoughnessLinear,
    NormalLinear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub role: TextureRole,
    pub mips: Vec<Mip>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mip {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Clamp,
    Mirror,
    Repeat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipmapFilter {
    None,
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub wrap_u: Wrap,
    pub wrap_v: Wrap,
    pub mag: Filter,
    pub min: Filter,
    pub mipmap: MipmapFilter,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            wrap_u: Wrap::Repeat,
            wrap_v: Wrap::Repeat,
            mag: Filter::Linear,
            min: Filter::Linear,
            mipmap: MipmapFilter::Linear,
        }
    }
}

/// Caps apply to the whole asset, including all primitives and mip levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_meshes: usize,
    pub max_primitives: usize,
    pub max_placements: usize,
    pub max_materials: usize,
    pub max_textures: usize,
    pub max_vertices: usize,
    pub max_indices: usize,
    pub max_dimension: u32,
}

impl Limits {
    /// Remove admission policy ceilings. Format widths, checked arithmetic and validity still apply.
    /// These maxima are comparisons, never requested allocation sizes.
    pub const fn unlimited() -> Self {
        Self {
            max_file_bytes: usize::MAX,
            max_decoded_bytes: usize::MAX,
            max_meshes: usize::MAX,
            max_primitives: usize::MAX,
            max_placements: usize::MAX,
            max_materials: usize::MAX,
            max_textures: usize::MAX,
            max_vertices: usize::MAX,
            max_indices: usize::MAX,
            max_dimension: u32::MAX,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 << 20,
            max_decoded_bytes: 256 << 20,
            max_meshes: 4096,
            max_primitives: 16384,
            max_placements: 16384,
            max_materials: 4096,
            max_textures: 256,
            max_vertices: 2_000_000,
            max_indices: 6_000_000,
            max_dimension: 4096,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub meshes: usize,
    pub primitives: usize,
    pub placements: usize,
    pub materials: usize,
    pub textures: usize,
    pub vertices: usize,
    pub indices: usize,
    pub tangent_vertices: usize,
    pub tangent_bytes: usize,
    pub mip_levels: usize,
    pub texture_bytes: usize,
    /// Logical element bytes, excluding Vec headers, allocator overhead and spare capacity.
    pub decoded_bytes: usize,
}

pub(crate) fn cap(value: usize, limit: usize, name: &'static str) -> Result<()> {
    if value > limit {
        return Err(Error(name));
    }
    Ok(())
}
pub(crate) fn add(total: &mut usize, amount: usize) -> Result<()> {
    *total = total.checked_add(amount).ok_or(Error("count overflow"))?;
    Ok(())
}
pub(crate) fn product(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Error("size overflow"))
}

pub fn valid_transform(m: &[[f32; 4]; 4]) -> bool {
    if !m.iter().flatten().all(|v| v.is_finite())
        || m[0][3] != 0.0
        || m[1][3] != 0.0
        || m[2][3] != 0.0
        || m[3][3] != 1.0
    {
        return false;
    }
    let a = |c: usize, r: usize| f64::from(m[c][r]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(2, 1) * a(1, 2))
        - a(1, 0) * (a(0, 1) * a(2, 2) - a(2, 1) * a(0, 2))
        + a(2, 0) * (a(0, 1) * a(1, 2) - a(1, 1) * a(0, 2));
    det.is_finite() && det != 0.0
}

impl Asset {
    pub fn extended(&self) -> bool {
        self.meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .any(|p| p.tangents.is_some())
            || self.materials.iter().any(|m| m.normal_texture.is_some())
            || self
                .textures
                .iter()
                .any(|t| t.role == TextureRole::NormalLinear)
    }
    /// Check all references and scalar domains before encoding or GPU integration.
    pub fn validate(&self, limits: Limits) -> Result<Counts> {
        for count in [
            self.meshes.len(),
            self.placements.len(),
            self.materials.len(),
            self.textures.len(),
        ] {
            cap(count, u32::MAX as usize, "prepared count width")?;
        }
        cap(self.meshes.len(), limits.max_meshes, "mesh limit")?;
        cap(
            self.placements.len(),
            limits.max_placements,
            "placement limit",
        )?;
        cap(self.materials.len(), limits.max_materials, "material limit")?;
        cap(self.textures.len(), limits.max_textures, "texture limit")?;
        let mut counts = Counts {
            meshes: self.meshes.len(),
            placements: self.placements.len(),
            materials: self.materials.len(),
            textures: self.textures.len(),
            ..Counts::default()
        };
        add(
            &mut counts.decoded_bytes,
            product(self.placements.len(), std::mem::size_of::<Placement>())?,
        )?;
        add(
            &mut counts.decoded_bytes,
            product(self.materials.len(), std::mem::size_of::<Material>())?,
        )?;
        for mesh in &self.meshes {
            validate_mesh(mesh, &mut counts, limits)?;
            for p in &mesh.primitives {
                if p.material as usize >= self.materials.len() {
                    return Err(Error("invalid material index"));
                }
                if self.materials[p.material as usize].normal_texture.is_some()
                    && p.tangents.is_none()
                {
                    return Err(Error("normal-mapped primitive requires tangents"));
                }
            }
        }
        for placement in &self.placements {
            if placement.mesh as usize >= self.meshes.len() {
                return Err(Error("invalid mesh index"));
            }
            if !valid_transform(&placement.transform) {
                return Err(Error("invalid affine placement transform"));
            }
        }
        for material in &self.materials {
            if !material
                .base_color
                .iter()
                .chain([&material.metallic, &material.roughness])
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            {
                return Err(Error("invalid material factor"));
            }
            if material
                .normal_texture
                .is_some_and(|t| !t.scale.is_finite())
            {
                return Err(Error("nonfinite normal scale"));
            }
            let refs = [
                material
                    .base_color_texture
                    .map(|t| (t.texture, TextureRole::BaseColorSrgb)),
                material
                    .metallic_roughness_texture
                    .map(|t| (t.texture, TextureRole::MetallicRoughnessLinear)),
                material
                    .normal_texture
                    .map(|t| (t.texture, TextureRole::NormalLinear)),
            ];
            for (index, role) in refs.into_iter().flatten() {
                let texture = self
                    .textures
                    .get(index as usize)
                    .ok_or(Error("invalid texture index"))?;
                if texture.role != role {
                    return Err(Error("texture role mismatch"));
                }
            }
        }
        for texture in &self.textures {
            let first = texture.mips.first().ok_or(Error("empty mip chain"))?;
            let (mut w, mut h) = (first.width, first.height);
            if w == 0 || h == 0 || w > limits.max_dimension || h > limits.max_dimension {
                return Err(Error("texture dimension limit"));
            }
            for (level, mip) in texture.mips.iter().enumerate() {
                if level > 0
                    && texture.mips[level - 1].width == 1
                    && texture.mips[level - 1].height == 1
                {
                    return Err(Error("mips after 1x1"));
                }
                if mip.width != w
                    || mip.height != h
                    || mip.rgba8.len() != product(product(w as usize, h as usize)?, 4)?
                {
                    return Err(Error("invalid mip dimensions or bytes"));
                }
                add(&mut counts.mip_levels, 1)?;
                add(&mut counts.texture_bytes, mip.rgba8.len())?;
                add(&mut counts.decoded_bytes, mip.rgba8.len())?;
                cap(
                    counts.decoded_bytes,
                    limits.max_decoded_bytes,
                    "decoded byte limit",
                )?;
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
            let last = texture.mips.last().unwrap();
            if last.width != 1 || last.height != 1 {
                return Err(Error("incomplete mip chain"));
            }
        }
        cap(
            counts.decoded_bytes,
            limits.max_decoded_bytes,
            "decoded byte limit",
        )?;
        Ok(counts)
    }
}

pub(crate) fn validate_mesh(mesh: &Mesh, counts: &mut Counts, limits: Limits) -> Result<()> {
    if mesh.primitives.is_empty() {
        return Err(Error("empty mesh"));
    }
    cap(
        mesh.primitives.len(),
        u32::MAX as usize,
        "primitive count width",
    )?;
    add(&mut counts.primitives, mesh.primitives.len())?;
    cap(counts.primitives, limits.max_primitives, "primitive limit")?;
    for p in &mesh.primitives {
        if p.vertices.is_empty() || p.indices.is_empty() || !p.indices.len().is_multiple_of(3) {
            return Err(Error("empty or nontriangle primitive"));
        }
        cap(p.vertices.len(), u32::MAX as usize, "vertex count width")?;
        cap(p.indices.len(), u32::MAX as usize, "index count width")?;
        add(&mut counts.vertices, p.vertices.len())?;
        add(&mut counts.indices, p.indices.len())?;
        cap(counts.vertices, limits.max_vertices, "vertex limit")?;
        cap(counts.indices, limits.max_indices, "index limit")?;
        add(&mut counts.decoded_bytes, product(p.vertices.len(), 48)?)?;
        add(&mut counts.decoded_bytes, product(p.indices.len(), 4)?)?;
        if let Some(t) = &p.tangents {
            validate_tangents(&p.vertices, &p.indices, t)?;
            add(&mut counts.tangent_vertices, t.values.len())?;
            let bytes = product(t.values.len(), 16)?;
            add(&mut counts.tangent_bytes, bytes)?;
            add(&mut counts.decoded_bytes, bytes)?;
        }
        cap(
            counts.decoded_bytes,
            limits.max_decoded_bytes,
            "decoded byte limit",
        )?;
        if p.indices.iter().any(|&i| i as usize >= p.vertices.len()) {
            return Err(Error("invalid vertex index"));
        }
        for v in &p.vertices {
            if !v
                .position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.color)
                .all(|v| v.is_finite())
            {
                return Err(Error("nonfinite vertex"));
            }
            if v.normal.iter().all(|v| *v == 0.0) {
                return Err(Error("zero normal"));
            }
            if !v.color.iter().all(|v| (0.0..=1.0).contains(v)) {
                return Err(Error("invalid vertex color"));
            }
        }
    }
    Ok(())
}
