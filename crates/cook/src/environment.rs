//! Deterministic offline distant IBL preparation. No GPU, file IO, or decoder in these functions.
use crate::{Error, Result};
use renderlab_asset::environment::{
    self as asset, CubeFace, CubeLevel, Metadata, PreparedEnvironment,
};
use sha2::{Digest, Sha256};
use std::f64::consts::{PI, TAU};
mod specular;

/// Linear Rec.709, row-major equirectangular RGB32F. No exposure, transfer curve or clipping.
/// Pixel centres: phi=2*pi*((x+.5)/width-.5), theta=pi*(y+.5)/height;
/// direction=(sin(theta)*cos(phi), cos(theta), sin(theta)*sin(phi)). +Y is the north pole.
#[derive(Clone, Debug)]
pub struct LinearEnvironment {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 3]>,
}
impl LinearEnvironment {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 3]>, limits: CookLimits) -> Result<Self> {
        let count = source_texels(width, height, limits)?;
        if pixels.len() as u64 != count
            || pixels.iter().flatten().any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err(Error("invalid environment source count or radiance".into()));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.pixels
    }

    /// Unit direction in source space. Bilinear wrap in longitude, clamp in latitude.
    /// The poles average the adjacent latitude row, avoiding a longitude-dependent pole value.
    fn sample(&self, d: [f64; 3]) -> [f64; 3] {
        let u = d[2].atan2(d[0]) / TAU + 0.5;
        let v = d[1].clamp(-1.0, 1.0).acos() / PI;
        let x = u * self.width as f64 - 0.5;
        let y = v * self.height as f64 - 0.5;
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let pixel = |x: i64, y: i64| {
            self.pixels[(y.clamp(0, self.height as i64 - 1) as u64 * self.width as u64
                + x.rem_euclid(self.width as i64) as u64) as usize]
                .map(f64::from)
        };
        let tx = x - x.floor();
        let ty = y - y.floor();
        let a = pixel(x0, y0);
        let b = pixel(x0 + 1, y0);
        let c = pixel(x0, y0 + 1);
        let e = pixel(x0 + 1, y0 + 1);
        // Collapse the vanishing polar cap towards the row mean, preserving continuity at +/-Y.
        let mut rgb = std::array::from_fn(|i| {
            (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + e[i] * tx) * ty
        });
        if y < 0.0 || y > self.height as f64 - 1.0 {
            let row = if y < 0.0 { 0 } else { self.height - 1 } as usize;
            let mut mean = [0.0; 3];
            for p in &self.pixels[row * self.width as usize..(row + 1) * self.width as usize] {
                for i in 0..3 {
                    mean[i] += p[i] as f64 / self.width as f64;
                }
            }
            let polar_blend = if y < 0.0 {
                -2.0 * y
            } else {
                2.0 * (y - (self.height as f64 - 1.0))
            };
            rgb = std::array::from_fn(|i| rgb[i] * (1.0 - polar_blend) + mean[i] * polar_blend);
        }
        rgb
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CookLimits {
    pub prepared: asset::Limits,
    pub max_source_file_bytes: u64,
    pub max_source_texels: u64,
    pub max_header_bytes: usize,
    /// Conservative owned-buffer byte admission, excluding allocator/decoder bookkeeping.
    pub max_working_bytes: u64,
    pub max_sample_evaluations: u64,
}
impl CookLimits {
    pub const fn unbounded() -> Self {
        Self {
            prepared: asset::Limits::unbounded(),
            max_source_file_bytes: u64::MAX,
            max_source_texels: u64::MAX,
            max_header_bytes: usize::MAX,
            max_working_bytes: u64::MAX,
            max_sample_evaluations: u64::MAX,
        }
    }
}
impl Default for CookLimits {
    fn default() -> Self {
        Self {
            prepared: asset::Limits::default(),
            max_source_file_bytes: 256 * 1024 * 1024,
            max_source_texels: 32 * 1024 * 1024,
            max_header_bytes: 64 * 1024,
            max_working_bytes: 1024 * 1024 * 1024,
            max_sample_evaluations: 100_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PreparationSettings {
    pub diffuse_size: u32,
    pub specular_size: u32,
    pub diffuse_samples: u32,
    /// Total samples per convolved texel, split between GGX and source-radiance proposals.
    pub specular_samples: u32,
}
impl Default for PreparationSettings {
    fn default() -> Self {
        Self {
            diffuse_size: 4,
            specular_size: 16,
            diffuse_samples: 1024,
            specular_samples: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CookCost {
    pub sample_evaluations: u64,
    /// Input pixels + complete output + peak reusable quadrature/proposal buffers.
    pub owned_working_bytes: u64,
    pub prepared_pixel_bytes: u64,
}

pub(crate) fn source_texels(width: u32, height: u32, limits: CookLimits) -> Result<u64> {
    let n = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || n > limits.max_source_texels || n > isize::MAX as u64 / 12 {
        return Err(Error("environment source dimensions/texel limit".into()));
    }
    Ok(n)
}

/// Admission is checked before constructing output pixels or quadrature tables.
pub fn estimate_cost(
    input: &LinearEnvironment,
    settings: PreparationSettings,
    limits: CookLimits,
) -> Result<CookCost> {
    source_texels(input.width, input.height, limits)?;
    if settings.diffuse_samples < 4 || settings.specular_samples == 0 {
        return Err(Error(
            "environment integration needs >=4 diffuse and >=1 specular samples".into(),
        ));
    }
    let n = asset::layout_texels(
        settings.diffuse_size,
        settings.specular_size,
        limits.prepared,
    )?;
    let diffuse_n = asset::cube_texels(settings.diffuse_size, limits.prepared)?;
    let base_n = asset::cube_texels(settings.specular_size, limits.prepared)?;
    let source_count = input.pixels.len() as u64;
    let convolved = n - diffuse_n - base_n;
    let (ggx_count, hdr_count) = specular::sample_counts(settings.specular_samples);
    let proposal_work = if convolved == 0 {
        0
    } else {
        // Minima and CDF source passes, plus the shared HDR radiance sample table.
        source_count * 2 + u64::from(hdr_count)
    };
    let samples = diffuse_n
        .checked_mul(settings.diffuse_samples as u64)
        .and_then(|d| {
            convolved
                .checked_mul(settings.specular_samples as u64)
                .and_then(|s| s.checked_add(d))
        })
        .and_then(|s| s.checked_add(base_n))
        .and_then(|s| s.checked_add(settings.diffuse_samples as u64))
        .and_then(|s| s.checked_add(proposal_work))
        .ok_or_else(|| Error("environment sample count overflow".into()))?;
    let specular_bytes = if convolved == 0 {
        0
    } else {
        source_count
            .checked_mul(8)
            .and_then(|n| n.checked_add(u64::from(hdr_count) * 56))
            .and_then(|n| n.checked_add(u64::from(ggx_count) * 32))
            .ok_or_else(|| Error("environment proposal byte overflow".into()))?
    };
    let table_bytes = (u64::from(settings.diffuse_samples) * 48).max(specular_bytes);
    if table_bytes > isize::MAX as u64 {
        return Err(Error("environment sample table address-space limit".into()));
    }
    let output_bytes = n * 12;
    let bytes = (input.pixels.len() as u64 * 12)
        .checked_add(output_bytes)
        .and_then(|n| n.checked_add(table_bytes))
        .ok_or_else(|| Error("environment working byte overflow".into()))?;
    if samples > limits.max_sample_evaluations || bytes > limits.max_working_bytes {
        return Err(Error("environment sample/working byte policy limit".into()));
    }
    Ok(CookCost {
        sample_evaluations: samples,
        owned_working_bytes: bytes,
        prepared_pixel_bytes: output_bytes,
    })
}

/// Pure offline convolution. Caller owns input. Metadata scale is baked once; yaw is retained only.
/// RGB accumulation is f64; unrepresentable f32 results are rejected, never clipped/normalized away.
pub fn prepare_environment(
    input: &LinearEnvironment,
    settings: PreparationSettings,
    mut metadata: Metadata,
    limits: CookLimits,
) -> Result<PreparedEnvironment> {
    metadata.validate()?;
    estimate_cost(input, settings, limits)?;
    let mut identity = Sha256::new();
    identity
        .update(b"renderlab-environment-fibonacci-diffuse-ggx-sqrt-hdr-mis-exact-norm-control-v3");
    for v in [
        input.width,
        input.height,
        settings.diffuse_size,
        settings.specular_size,
        settings.diffuse_samples,
        settings.specular_samples,
        metadata.source_radiance_scale.to_bits(),
    ] {
        identity.update(v.to_le_bytes());
    }
    for p in &input.pixels {
        for c in p {
            identity.update(c.to_le_bytes());
        }
    }
    // Rotation/units do not alter convolution pixels; identity still covers the complete preparation.
    identity.update(metadata.rotation_y_radians.to_le_bytes());
    identity.update([match metadata.units {
        asset::RadianceUnits::CandelasPerSquareMetre => 1,
        asset::RadianceUnits::Relative => 2,
    }]);
    metadata.generator_hash = identity.finalize().into();
    let scale = metadata.source_radiance_scale as f64;
    let diffuse = convolve_diffuse_irradiance(input, settings, scale, limits)?;
    let specular = prefilter_ggx_radiance(input, settings, scale, limits)?;
    Ok(PreparedEnvironment::new(
        metadata,
        diffuse,
        specular,
        limits.prepared,
    )?)
}

/// Deterministic equal-area Fibonacci-sphere angular quadrature. Stores irradiance, including pi.
/// Normalize the cosine kernel's quadrature weight to pi (constant preservation); radiance itself
/// is never rescaled by its maximum or per-mip energy. A source sample table is shared by all texels.
pub fn convolve_diffuse_irradiance(
    input: &LinearEnvironment,
    settings: PreparationSettings,
    scale: f64,
    limits: CookLimits,
) -> Result<CubeLevel> {
    estimate_cost(input, settings, limits)?;
    check_scale(scale)?;
    let mut table = Vec::new();
    table
        .try_reserve_exact(settings.diffuse_samples as usize)
        .map_err(|_| Error("environment diffuse quadrature allocation".into()))?;
    let golden_angle = PI * (3.0 - 5.0_f64.sqrt());
    for i in 0..settings.diffuse_samples {
        let y = 1.0 - 2.0 * (i as f64 + 0.5) / settings.diffuse_samples as f64;
        let radius = (1.0 - y * y).sqrt();
        let phi = i as f64 * golden_angle;
        let d = [radius * phi.cos(), y, radius * phi.sin()];
        table.push((d, input.sample(d)));
    }
    make_cube(settings.diffuse_size, limits, |n| {
        let mut rgb = [0.0; 3];
        let mut weights = 0.0;
        for &(d, value) in &table {
            let cosine = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]).max(0.0);
            for c in 0..3 {
                rgb[c] += value[c] * cosine;
            }
            weights += cosine;
        }
        rgb.map(|v| v / weights * PI * scale)
    })
}

/// Split-sum radiance factor, N=V=R. Balanced GGX/HDR MIS with exact kernel normalization.
/// This is an approximation for nonconstant input, even with a converged matching DFG LUT.
/// Mip zero directly samples source; no PDF-based source-mip blur or half-float conversion.
pub fn prefilter_ggx_radiance(
    input: &LinearEnvironment,
    settings: PreparationSettings,
    scale: f64,
    limits: CookLimits,
) -> Result<Vec<CubeLevel>> {
    estimate_cost(input, settings, limits)?;
    check_scale(scale)?;
    let levels = settings.specular_size.ilog2() + 1;
    let mut out = Vec::new();
    // Delta-only maps need neither proposal storage nor source preprocessing.
    let quadrature = if levels > 1 {
        Some(specular::Quadrature::new(input, settings.specular_samples)?)
    } else {
        None
    };
    for i in 0..levels {
        let size = settings.specular_size >> i;
        let roughness = i as f64 / (levels - 1).max(1) as f64;
        let table = if i == 0 {
            Vec::new()
        } else {
            specular::ggx_samples(
                specular::sample_counts(settings.specular_samples).0,
                roughness,
            )?
        };
        out.push(make_cube(size, limits, |n| {
            (if i == 0 {
                input.sample(n)
            } else {
                quadrature.as_ref().unwrap().integrate(n, roughness, &table)
            })
            .map(|v| v * scale)
        })?);
    }
    Ok(out)
}

fn check_scale(scale: f64) -> Result<()> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Error("invalid environment convolution scale".into()));
    }
    Ok(())
}

fn make_cube(
    size: u32,
    limits: CookLimits,
    mut f: impl FnMut([f64; 3]) -> [f64; 3],
) -> Result<CubeLevel> {
    let count = asset::cube_texels(size, limits.prepared)?;
    let mut texels = Vec::new();
    texels
        .try_reserve_exact(count as usize)
        .map_err(|_| Error("environment cube allocation".into()))?;
    for face in CubeFace::ALL {
        for y in 0..size {
            for x in 0..size {
                let n = face.direction([
                    2.0 * (x as f64 + 0.5) / size as f64 - 1.0,
                    2.0 * (y as f64 + 0.5) / size as f64 - 1.0,
                ]);
                let value = f(n);
                if value.iter().any(|v| {
                    !v.is_finite()
                        || *v < 0.0
                        || *v > f32::MAX as f64
                        || (*v > 0.0 && *v as f32 == 0.0)
                }) {
                    return Err(Error("environment convolution outside RGB32F range".into()));
                }
                texels.push(value.map(|v| v as f32));
            }
        }
    }
    Ok(CubeLevel::new(size, texels, limits.prepared)?)
}
