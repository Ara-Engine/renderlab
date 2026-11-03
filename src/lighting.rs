use crate::{Error, Vec3};

/// Metres and linear Rec.709 RGB. Y=.2126R+.7152G+.0722B carries the named
/// photometric unit. `travel` points away from a source and is normalized on set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Light {
    Directional {
        travel: Vec3,
        lux: [f32; 3],
    },
    /// Inverse square is regularized at 1 cm. None is unlimited range.
    Point {
        position: Vec3,
        candela: [f32; 3],
        range: Option<f32>,
    },
    /// Cone half angles in radians; squared interpolation between their cosines.
    Spot {
        position: Vec3,
        travel: Vec3,
        candela: [f32; 3],
        range: Option<f32>,
        inner: f32,
        outer: f32,
    },
}

/// Whole replacement, validated atomically by Renderer::set_lighting. The constant
/// environment is cd/m², with no occlusion or bounce transport. Exposure is 2^stops.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lighting {
    pub lights: Vec<Light>,
    pub environment: [f32; 3],
    pub exposure_stops: f32,
}

pub(crate) struct PackedLighting {
    pub records: Vec<u8>,
    pub count: u32,
    pub environment: [f32; 3],
    pub exposure: f32,
}
impl PackedLighting {
    pub fn settings(value: &Lighting) -> Result<f32, Error> {
        let exposure = value.exposure_stops.exp2();
        if !value.exposure_stops.is_finite() || !exposure.is_normal() || exposure <= 0.0 {
            return Err(Error::InvalidInput(
                "2^exposure_stops must be a positive finite normal f32",
            ));
        }
        energy(value.environment)?;
        Ok(exposure)
    }
    pub fn new(value: &Lighting, max: u32) -> Result<Self, Error> {
        let count = u32::try_from(value.lights.len())
            .ok()
            .filter(|n| *n <= max)
            .ok_or(Error::InvalidInput("lighting exceeds device max_lights"))?;
        let exposure = Self::settings(value)?;
        let mut records = Vec::with_capacity(value.lights.len() * 64);
        for light in &value.lights {
            let (position, range, travel, rgb, inner, outer, kind) = match *light {
                Light::Directional { travel, lux } => {
                    (Vec3::ZERO, None, direction(travel)?, lux, 0.0, 0.0, 0u32)
                }
                Light::Point {
                    position,
                    candela,
                    range,
                } => (position, range, Vec3::ZERO, candela, 0.0, 0.0, 1),
                Light::Spot {
                    position,
                    travel,
                    candela,
                    range,
                    inner,
                    outer,
                } => {
                    if !inner.is_finite()
                        || !outer.is_finite()
                        || inner < 0.0
                        || inner >= outer
                        || outer > std::f32::consts::FRAC_PI_2
                    {
                        return Err(Error::InvalidInput(
                            "spot requires 0 <= inner < outer <= pi/2",
                        ));
                    }
                    let (ci, co) = (inner.cos(), outer.cos());
                    if ci <= co {
                        return Err(Error::InvalidInput("spot cone cosines collapse in f32"));
                    }
                    (position, range, direction(travel)?, candela, ci, co, 2)
                }
            };
            energy(rgb)?;
            if !position.is_finite() || range.is_some_and(|r| !r.is_finite() || r <= 0.0) {
                return Err(Error::InvalidInput(
                    "light position must be finite; range must be a positive finite f32",
                ));
            }
            for v in position
                .to_array()
                .into_iter()
                .chain([range.unwrap_or(0.0)])
                .chain(travel.to_array())
                .chain([outer])
                .chain(rgb)
                .chain([inner])
            {
                records.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [kind, 0, 0, 0] {
                records.extend_from_slice(&v.to_ne_bytes());
            }
        }
        Ok(Self {
            records,
            count,
            environment: value.environment,
            exposure,
        })
    }
    pub fn uniform(&self, eye: Vec3) -> [u8; 32] {
        let mut bytes = [0; 32];
        for (dst, v) in bytes[..12].chunks_exact_mut(4).zip(eye.to_array()) {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        bytes[12..16].copy_from_slice(&self.count.to_ne_bytes());
        for (dst, v) in bytes[16..]
            .chunks_exact_mut(4)
            .zip(self.environment.into_iter().chain([self.exposure]))
        {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        bytes
    }
}
fn energy(rgb: [f32; 3]) -> Result<(), Error> {
    if rgb.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(Error::InvalidInput(
            "light/environment RGB must be finite and nonnegative",
        ));
    }
    Ok(())
}
fn direction(v: Vec3) -> Result<Vec3, Error> {
    if !v.is_finite() || v == Vec3::ZERO {
        return Err(Error::InvalidInput(
            "light travel must be finite and nonzero",
        ));
    }
    Ok(v.as_dvec3().normalize().as_vec3())
}
/// First/last differing words, including appended records; COPY_BUFFER_ALIGNMENT on both ends.
pub(crate) fn dirty_span(old: &[u8], new: &[u8]) -> Option<std::ops::Range<usize>> {
    let start = new
        .chunks_exact(4)
        .enumerate()
        .find(|(i, b)| old.get(i * 4..i * 4 + 4) != Some(*b))?
        .0
        * 4;
    let end = new
        .chunks_exact(4)
        .enumerate()
        .rev()
        .find(|(i, b)| old.get(i * 4..i * 4 + 4) != Some(*b))
        .unwrap()
        .0
        * 4
        + 4;
    Some(start..end)
}

/// Decode stored IEEE binary16 capture channels without a runtime dependency.
pub fn decode_f16(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 31;
    let mantissa = bits & 1023;
    sign * match exponent {
        0 => f32::from(mantissa) * 2.0_f32.powi(-24),
        31 => {
            if mantissa == 0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => (1.0 + f32::from(mantissa) / 1024.0) * 2.0_f32.powi(i32::from(exponent) - 15),
    }
}
pub(crate) const DFG_SIZE: u32 = 128;
pub(crate) const DFG: &[u8] = include_bytes!("dfg.rg16f");
