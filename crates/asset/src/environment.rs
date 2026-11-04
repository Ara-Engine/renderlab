//! Owned linear Rec.709 environments. RLE1 mip chains and explicit RLE2 roughness fields.
use crate::{Error, Result};

pub const VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 96;
pub const GGX_ALPHA_FLOOR: f64 = 1e-4;
pub const FIELD_LEVELS: usize = 65;
pub const FIELD_FACE_SIZE: u32 = 128;
pub const RLE2_HEADER_BYTES: usize = 128 + FIELD_LEVELS * 4;
pub const MAX_HEADER_BYTES: usize = RLE2_HEADER_BYTES;

/// Generator arithmetic is provenance, independent of the radiance storage format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldGenerator {
    /// Accepted f64 proposal, basis and exact Z, rounded into the compensated f32 GPU kernel v1.
    F64HostF32GpuV1,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpecularLayout {
    LegacyMipChain,
    IndependentRoughness {
        knots: Box<[f32; FIELD_LEVELS]>,
        generator: FieldGenerator,
    },
}

pub fn roughness_knots() -> [f32; FIELD_LEVELS] {
    std::array::from_fn(|i| (i as f32 / 64.0).powi(2))
}

/// Exact quadratic knot bracket, blended in r^4. Reject invalid caller input at this boundary.
pub fn roughness_bracket(r: f32) -> Result<(usize, usize, f32)> {
    if !r.is_finite() || !(0.0..=1.0).contains(&r) {
        return Err(Error("environment roughness domain"));
    }
    if r == 1.0 {
        return Ok((64, 64, 0.0));
    }
    let lo = ((64.0 * r.sqrt()).floor() as usize).min(63);
    let knots = roughness_knots();
    let a = knots[lo].powi(4);
    let b = knots[lo + 1].powi(4);
    Ok((lo, lo + 1, ((r.powi(4) - a) / (b - a)).clamp(0.0, 1.0)))
}

/// Cube layers match WebGPU: +X, -X, +Y, -Y, +Z, -Z. Rows run top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    /// `uv` spans [-1,1], with v increasing downwards. Returns a unit source-space direction.
    pub fn direction(self, uv: [f64; 2]) -> [f64; 3] {
        let [u, v] = uv;
        let p = match self {
            Self::PositiveX => [1.0, -v, -u],
            Self::NegativeX => [-1.0, -v, u],
            Self::PositiveY => [u, 1.0, v],
            Self::NegativeY => [u, -1.0, -v],
            Self::PositiveZ => [u, -v, 1.0],
            Self::NegativeZ => [-u, -v, -1.0],
        };
        let length = p.iter().map(|x| x * x).sum::<f64>().sqrt();
        p.map(|x| x / length)
    }
}

/// Policy, not format ceilings. `unbounded()` retains checked sizes and structural validation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_dimension: u32,
    pub max_texels: u64,
    pub max_payload_bytes: u64,
}
impl Limits {
    pub const fn unbounded() -> Self {
        Self {
            max_dimension: u32::MAX,
            max_texels: u64::MAX,
            max_payload_bytes: u64::MAX,
        }
    }
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 768 * 1024 * 1024,
        }
    }
}

/// An RGB triple has photometric luminance .2126 R + .7152 G + .0722 B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadianceUnits {
    CandelasPerSquareMetre,
    Relative,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Metadata {
    pub units: RadianceUnits,
    /// Source-to-world right-handed rotation about +Y, radians. NOT baked into cube pixels.
    /// World-space lookups must use the inverse rotation before sampling either cube.
    pub rotation_y_radians: f32,
    /// Multiplier already baked during preparation; provenance only, do not apply again.
    pub source_radiance_scale: f32,
    /// Cooker algorithm, source pixels, dimensions and settings identity, not a payload checksum.
    pub generator_hash: [u8; 32],
}
impl Metadata {
    pub fn validate(&self) -> Result<()> {
        if !self.rotation_y_radians.is_finite()
            || !self.source_radiance_scale.is_finite()
            || self.source_radiance_scale <= 0.0
        {
            return Err(Error("invalid environment rotation or radiance scale"));
        }
        Ok(())
    }
}

/// Face-major RGB32F, then row-major texels. Finite nonnegative components; no 16f clipping.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeLevel {
    size: u32,
    texels: Vec<[f32; 3]>,
}
impl CubeLevel {
    pub fn new(size: u32, texels: Vec<[f32; 3]>, limits: Limits) -> Result<Self> {
        let count = cube_texels(size, limits)?;
        check_totals(count, limits)?;
        if texels.len() as u64 != count {
            return Err(Error("environment cube texel count"));
        }
        if texels.iter().flatten().any(|x| !x.is_finite() || *x < 0.0) {
            return Err(Error("invalid environment radiance/irradiance"));
        }
        Ok(Self { size, texels })
    }
    pub fn size(&self) -> u32 {
        self.size
    }
    pub fn texels(&self) -> &[[f32; 3]] {
        &self.texels
    }
    pub fn face(&self, face: CubeFace) -> &[[f32; 3]] {
        let n = self.texels.len() / 6;
        &self.texels[face as usize * n..(face as usize + 1) * n]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedEnvironment {
    metadata: Metadata,
    diffuse_irradiance: CubeLevel,
    specular_radiance: Vec<CubeLevel>,
    layout: SpecularLayout,
}
impl PreparedEnvironment {
    /// Diffuse stores E=integral(L cos(theta) dOmega), in lux for absolute photometric input.
    /// Specular is normalized GGX prefiltered radiance, NOT a complete reflected BRDF integral.
    /// Mip i uses perceptual r=i/(mip_count-1), alpha=max(r*r,1e-4), N=V=R.
    /// Mip zero is the unblurred directional source lookup (delta convention).
    pub fn new(
        metadata: Metadata,
        diffuse_irradiance: CubeLevel,
        specular_radiance: Vec<CubeLevel>,
        limits: Limits,
    ) -> Result<Self> {
        metadata.validate()?;
        let first = specular_radiance
            .first()
            .ok_or(Error("empty environment specular chain"))?;
        let expected = first.size.ilog2() + 1;
        if !first.size.is_power_of_two() || specular_radiance.len() != expected as usize {
            return Err(Error(
                "environment needs complete power-of-two specular chain",
            ));
        }
        let count = layout_texels(diffuse_irradiance.size, first.size, limits)?;
        for (i, level) in specular_radiance.iter().enumerate() {
            if level.size != first.size >> i {
                return Err(Error("environment mip dimensions"));
            }
        }
        check_totals(count, limits)?;
        Ok(Self {
            metadata,
            diffuse_irradiance,
            specular_radiance,
            layout: SpecularLayout::LegacyMipChain,
        })
    }
    /// Equal spatial dimensions at every roughness. Production candidate A uses 128-square faces.
    /// Smaller power-of-two faces support bounded fixtures without changing the knot convention.
    pub fn new_independent(
        metadata: Metadata,
        diffuse_irradiance: CubeLevel,
        specular_radiance: Vec<CubeLevel>,
        knots: [f32; FIELD_LEVELS],
        generator: FieldGenerator,
        limits: Limits,
    ) -> Result<Self> {
        metadata.validate()?;
        validate_knots(&knots)?;
        let first = specular_radiance
            .first()
            .ok_or(Error("empty environment field"))?;
        field_texels(diffuse_irradiance.size, first.size, limits)?;
        if specular_radiance.len() != FIELD_LEVELS
            || specular_radiance
                .iter()
                .any(|level| level.size != first.size)
        {
            return Err(Error("environment independent dimensions/count"));
        }
        Ok(Self {
            metadata,
            diffuse_irradiance,
            specular_radiance,
            layout: SpecularLayout::IndependentRoughness {
                knots: Box::new(knots),
                generator,
            },
        })
    }
    pub fn specular_layout(&self) -> &SpecularLayout {
        &self.layout
    }
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub fn diffuse_irradiance(&self) -> &CubeLevel {
        &self.diffuse_irradiance
    }
    pub fn specular_radiance(&self) -> &[CubeLevel] {
        &self.specular_radiance
    }
    pub fn texel_count(&self) -> u64 {
        self.diffuse_irradiance.texels.len() as u64
            + self
                .specular_radiance
                .iter()
                .map(|m| m.texels.len() as u64)
                .sum::<u64>()
    }
    pub fn pixel_bytes(&self) -> u64 {
        self.texel_count() * 12
    }
}

fn validate_knots(knots: &[f32; FIELD_LEVELS]) -> Result<()> {
    if knots
        .iter()
        .zip(roughness_knots())
        .any(|(a, b)| a.to_bits() != b.to_bits())
    {
        return Err(Error("unsupported environment roughness knots"));
    }
    Ok(())
}

/// Checked admission for independent spatial levels, before allocating any field pixels.
pub fn field_texels(diffuse_size: u32, face_size: u32, limits: Limits) -> Result<u64> {
    if !face_size.is_power_of_two() {
        return Err(Error("environment field size must be power of two"));
    }
    let n = cube_texels(face_size, limits)?
        .checked_mul(FIELD_LEVELS as u64)
        .and_then(|n| n.checked_add(cube_texels(diffuse_size, limits).ok()?))
        .ok_or(Error("environment field size overflow/dimensions"))?;
    check_totals(n, limits)?;
    Ok(n)
}

pub fn cube_texels(size: u32, limits: Limits) -> Result<u64> {
    if size == 0 || size > limits.max_dimension {
        return Err(Error("environment dimension limit"));
    }
    u64::from(size)
        .checked_mul(u64::from(size))
        .and_then(|n| n.checked_mul(6))
        .ok_or(Error("environment cube size overflow"))
}

/// Preflight an entire prepared layout before allocation or convolution.
pub fn layout_texels(diffuse_size: u32, specular_size: u32, limits: Limits) -> Result<u64> {
    if !specular_size.is_power_of_two() {
        return Err(Error("environment specular size must be power of two"));
    }
    let mut n = cube_texels(diffuse_size, limits)?;
    let mut size = specular_size;
    loop {
        n = n
            .checked_add(cube_texels(size, limits)?)
            .ok_or(Error("environment texel count overflow"))?;
        if size == 1 {
            break;
        }
        size /= 2;
    }
    check_totals(n, limits)?;
    Ok(n)
}

fn check_totals(n: u64, limits: Limits) -> Result<u64> {
    let bytes = n
        .checked_mul(12)
        .ok_or(Error("environment byte count overflow"))?;
    if n > limits.max_texels || bytes > limits.max_payload_bytes || bytes > isize::MAX as u64 {
        return Err(Error("environment texel/byte/address-space limit"));
    }
    Ok(bytes)
}

fn encode_field(environment: &PreparedEnvironment, limits: Limits) -> Result<Vec<u8>> {
    let SpecularLayout::IndependentRoughness {
        knots,
        generator: FieldGenerator::F64HostF32GpuV1,
    } = &environment.layout
    else {
        unreachable!()
    };
    let n = field_texels(
        environment.diffuse_irradiance.size,
        environment.specular_radiance[0].size,
        limits,
    )?;
    let payload = check_totals(n, limits)?;
    let len = payload
        .checked_add(RLE2_HEADER_BYTES as u64)
        .filter(|n| *n <= isize::MAX as u64)
        .ok_or(Error("environment file length overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len as usize)
        .map_err(|_| Error("environment file allocation"))?;
    out.extend_from_slice(b"RLE2");
    for v in [
        2,
        1,
        1,
        match environment.metadata.units {
            RadianceUnits::CandelasPerSquareMetre => 1,
            RadianceUnits::Relative => 2,
        },
        2,
        environment.metadata.rotation_y_radians.to_bits(),
        environment.metadata.source_radiance_scale.to_bits(),
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&environment.metadata.generator_hash);
    for v in [
        environment.diffuse_irradiance.size,
        environment.specular_radiance[0].size,
        FIELD_LEVELS as u32,
        0,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&payload.to_le_bytes());
    // Independent layout, normalized N=V GGX + delta, r^4 interpolant, arithmetic v1, knots, reserved.
    for v in [1_u32, 1, 1, 1, FIELD_LEVELS as u32, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&GGX_ALPHA_FLOOR.to_le_bytes());
    for knot in knots.iter() {
        out.extend_from_slice(&knot.to_le_bytes());
    }
    for level in
        std::iter::once(&environment.diffuse_irradiance).chain(&environment.specular_radiance)
    {
        for rgb in &level.texels {
            for c in rgb {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    Ok(out)
}

fn decode_field(bytes: &[u8], limits: Limits) -> Result<PreparedEnvironment> {
    if bytes.len() < RLE2_HEADER_BYTES {
        return Err(Error("environment RLE2 header"));
    }
    let u32_at = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    if u32_at(4) != 2
        || u32_at(8) != 1
        || u32_at(12) != 1
        || u32_at(20) != 2
        || u32_at(76) != 0
        || u32_at(96) != 1
        || u32_at(100) != 1
        || u32_at(104) != 1
        || u32_at(108) != 1
        || u32_at(112) != FIELD_LEVELS as u32
        || u32_at(116) != 0
        || u64_at(120) != GGX_ALPHA_FLOOR.to_bits()
    {
        return Err(Error("unsupported environment RLE2 conventions/generator"));
    }
    let metadata = Metadata {
        units: match u32_at(16) {
            1 => RadianceUnits::CandelasPerSquareMetre,
            2 => RadianceUnits::Relative,
            _ => return Err(Error("environment radiance units")),
        },
        rotation_y_radians: f32::from_bits(u32_at(24)),
        source_radiance_scale: f32::from_bits(u32_at(28)),
        generator_hash: bytes[32..64].try_into().unwrap(),
    };
    metadata.validate()?;
    let knots = std::array::from_fn(|i| f32::from_bits(u32_at(128 + i * 4)));
    validate_knots(&knots)?;
    let (diffuse_size, size) = (u32_at(64), u32_at(68));
    let n = field_texels(diffuse_size, size, limits)?;
    let payload = check_totals(n, limits)?;
    if u32_at(72) != FIELD_LEVELS as u32
        || u64_at(80) != n
        || u64_at(88) != payload
        || payload.checked_add(RLE2_HEADER_BYTES as u64) != Some(bytes.len() as u64)
    {
        return Err(Error("environment RLE2 counts/file length"));
    }
    // Reject the complete external payload before allocating any decoded cube.
    if bytes[RLE2_HEADER_BYTES..].chunks_exact(4).any(|c| {
        let v = f32::from_le_bytes(c.try_into().unwrap());
        !v.is_finite() || v < 0.0
    }) {
        return Err(Error("invalid environment radiance/irradiance"));
    }
    let mut at = RLE2_HEADER_BYTES;
    let mut read = |size| -> Result<CubeLevel> {
        let count = cube_texels(size, limits)? as usize;
        let mut texels = Vec::new();
        texels
            .try_reserve_exact(count)
            .map_err(|_| Error("environment cube allocation"))?;
        for _ in 0..count {
            texels.push(std::array::from_fn(|_| {
                let v = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                at += 4;
                v
            }));
        }
        Ok(CubeLevel { size, texels })
    };
    let diffuse = read(diffuse_size)?;
    let mut specular = Vec::with_capacity(FIELD_LEVELS);
    for _ in 0..FIELD_LEVELS {
        specular.push(read(size)?);
    }
    PreparedEnvironment::new_independent(
        metadata,
        diffuse,
        specular,
        knots,
        FieldGenerator::F64HostF32GpuV1,
        limits,
    )
}

pub fn encode(environment: &PreparedEnvironment, limits: Limits) -> Result<Vec<u8>> {
    if matches!(
        environment.layout,
        SpecularLayout::IndependentRoughness { .. }
    ) {
        return encode_field(environment, limits);
    }
    let n = layout_texels(
        environment.diffuse_irradiance.size,
        environment.specular_radiance[0].size,
        limits,
    )?;
    let bytes = check_totals(n, limits)?;
    let len = bytes
        .checked_add(HEADER_BYTES as u64)
        .filter(|n| *n <= isize::MAX as u64)
        .ok_or(Error("environment file length overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len as usize)
        .map_err(|_| Error("environment file allocation"))?;
    out.extend_from_slice(b"RLE1");
    for value in [
        VERSION,
        1,
        1,
        match environment.metadata.units {
            RadianceUnits::CandelasPerSquareMetre => 1,
            RadianceUnits::Relative => 2,
        },
        1,
        environment.metadata.rotation_y_radians.to_bits(),
        environment.metadata.source_radiance_scale.to_bits(),
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&environment.metadata.generator_hash);
    for value in [
        environment.diffuse_irradiance.size,
        environment.specular_radiance[0].size,
        environment.specular_radiance.len() as u32,
        0,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&bytes.to_le_bytes());
    for level in
        std::iter::once(&environment.diffuse_irradiance).chain(&environment.specular_radiance)
    {
        for rgb in &level.texels {
            for channel in rgb {
                out.extend_from_slice(&channel.to_le_bytes());
            }
        }
    }
    Ok(out)
}

/// Requires exact length and validates the complete header/budget before any pixel allocation.
pub fn decode(bytes: &[u8], limits: Limits) -> Result<PreparedEnvironment> {
    if bytes.starts_with(b"RLE2") {
        return decode_field(bytes, limits);
    }
    if bytes.len() < HEADER_BYTES || &bytes[..4] != b"RLE1" {
        return Err(Error("environment magic/header"));
    }
    let u32_at = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    if u32_at(4) != VERSION
        || u32_at(8) != 1
        || u32_at(12) != 1
        || u32_at(20) != 1
        || u32_at(76) != 0
    {
        return Err(Error("unsupported environment version/conventions"));
    }
    let units = match u32_at(16) {
        1 => RadianceUnits::CandelasPerSquareMetre,
        2 => RadianceUnits::Relative,
        _ => return Err(Error("environment radiance units")),
    };
    let metadata = Metadata {
        units,
        rotation_y_radians: f32::from_bits(u32_at(24)),
        source_radiance_scale: f32::from_bits(u32_at(28)),
        generator_hash: bytes[32..64].try_into().unwrap(),
    };
    metadata.validate()?;
    let (diffuse_size, specular_size) = (u32_at(64), u32_at(68));
    let n = layout_texels(diffuse_size, specular_size, limits)?;
    let pixel_bytes = check_totals(n, limits)?;
    if u32_at(72) != specular_size.ilog2() + 1
        || u64_at(80) != n
        || u64_at(88) != pixel_bytes
        || pixel_bytes.checked_add(HEADER_BYTES as u64) != Some(bytes.len() as u64)
    {
        return Err(Error("environment counts/file length"));
    }
    let mut at = HEADER_BYTES;
    let mut read_level = |size| -> Result<CubeLevel> {
        let n = cube_texels(size, limits)? as usize;
        let mut texels = Vec::new();
        texels
            .try_reserve_exact(n)
            .map_err(|_| Error("environment cube allocation"))?;
        for _ in 0..n {
            let rgb = std::array::from_fn(|_| {
                let x = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                at += 4;
                x
            });
            texels.push(rgb);
        }
        CubeLevel::new(size, texels, limits)
    };
    let diffuse = read_level(diffuse_size)?;
    let mut specular = Vec::new();
    for i in 0..=specular_size.ilog2() {
        specular.push(read_level(specular_size >> i)?);
    }
    PreparedEnvironment::new(metadata, diffuse, specular, limits)
}
