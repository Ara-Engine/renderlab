//! Shadow coverage is explicit and independent of a light's photometric range.
use crate::{Error, Light};

/// Entries correspond exactly to the current `Lighting::lights` vector.
#[derive(Clone, Debug, PartialEq)]
pub enum ShadowRequest {
    None,
    /// Positive camera-forward distances. The final split is the coverage limit.
    Directional {
        split_ends: Vec<f32>,
    },
    /// Radial metres. Submitted caster bounds must not intersect the near sphere.
    Point {
        near: f32,
        far: f32,
    },
    Spot {
        near: f32,
        far: f32,
    },
}

/// Hard, nearest-depth shadows. No finite-source softness or cascade blending.
/// An empty request vector explicitly disables shadows for the entire lighting set.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadows {
    pub requests: Vec<ShadowRequest>,
    /// Common square resolution for the owned depth array.
    pub resolution: u32,
    /// Receiver depth offset in normalized map depth, in [0, 1).
    pub depth_bias: f32,
    /// Receiver offset along its geometric surface normal, in metres.
    pub normal_bias: f32,
    /// Optional shadow record/map/uniform byte policy per resource generation.
    /// Hardware shadows charge records/fallback here and AS estimates separately.
    /// Retained in-flight generations are charged separately in `Stats::ownership`.
    /// Device limits always apply.
    pub max_bytes: Option<u64>,
}
impl Default for Shadows {
    fn default() -> Self {
        Self {
            requests: vec![],
            resolution: 1024,
            depth_bias: 0.00002,
            normal_bias: 0.001,
            max_bytes: Some(256 * 1024 * 1024),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowLimits {
    pub max_resolution: u32,
    pub max_views: u32,
    pub max_buffer_bytes: u64,
    pub uniform_alignment: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ShadowStats {
    /// Complete map/matrix publication, incremented only by an admitted shadowed frame.
    pub generation: u64,
    pub resource_generation: u64,
    pub views: u32,
    pub resolution: u32,
    pub bytes: u64,
    pub retained_generations: usize,
    pub caster_draws: u64,
    pub caster_triangles: u64,
}

pub(crate) const VIEW_BYTES: u64 = 112;
impl Shadows {
    pub(crate) fn enabled(&self) -> bool {
        self.requests
            .iter()
            .any(|r| !matches!(r, ShadowRequest::None))
    }
    pub(crate) fn validate(&self, lights: &[Light], limits: ShadowLimits) -> Result<u32, Error> {
        if !self.requests.is_empty() && self.requests.len() != lights.len() {
            return Err(Error::InvalidInput(
                "shadow requests must match the current light vector; replace lighting and shadows together",
            ));
        }
        if !self.depth_bias.is_finite()
            || !(0.0..1.0).contains(&self.depth_bias)
            || !self.normal_bias.is_finite()
            || self.normal_bias < 0.0
        {
            return Err(Error::InvalidInput(
                "shadow depth bias must be in [0,1); normal bias must be finite and nonnegative",
            ));
        }
        let mut count = 0u32;
        for (request, light) in self.requests.iter().zip(lights) {
            let views = match (request, light) {
                (ShadowRequest::None, _) => 0,
                (ShadowRequest::Directional { split_ends }, Light::Directional { .. }) => {
                    let mut previous = 0.0;
                    if split_ends.is_empty() {
                        return Err(Error::InvalidInput(
                            "directional shadow splits cannot be empty",
                        ));
                    }
                    for &split in split_ends {
                        if !split.is_finite() || split <= previous {
                            return Err(Error::InvalidInput(
                                "shadow splits must be finite, positive and increasing",
                            ));
                        }
                        previous = split;
                    }
                    u32::try_from(split_ends.len())
                        .map_err(|_| Error::InvalidInput("shadow view count overflow"))?
                }
                (ShadowRequest::Point { near, far }, Light::Point { .. }) => {
                    validate_range(*near, *far)?;
                    6
                }
                (ShadowRequest::Spot { near, far }, Light::Spot { outer, .. }) => {
                    validate_range(*near, *far)?;
                    if *outer >= std::f32::consts::FRAC_PI_2 {
                        return Err(Error::InvalidInput(
                            "shadowed spots require outer < pi/2; unshadowed pi/2 lights remain supported",
                        ));
                    }
                    1
                }
                _ => {
                    return Err(Error::InvalidInput(
                        "shadow request kind does not match its current light",
                    ));
                }
            };
            count = count
                .checked_add(views)
                .ok_or(Error::InvalidInput("shadow view count overflow"))?;
        }
        if count == 0 {
            return Ok(0);
        }
        if self.resolution < 2
            || self.resolution > limits.max_resolution
            || count > limits.max_views
        {
            return Err(Error::InvalidInput(
                "shadow resolution/view count exceeds device limits (resolution must be >=2)",
            ));
        }
        let stride = VIEW_BYTES.div_ceil(u64::from(limits.uniform_alignment))
            * u64::from(limits.uniform_alignment);
        if u64::from(count) * stride > limits.max_buffer_bytes {
            return Err(Error::InvalidInput(
                "shadow matrix buffer exceeds device limit",
            ));
        }
        let bytes = self.bytes(count, limits)?;
        if self.max_bytes.is_some_and(|max| bytes > max) {
            return Err(Error::InvalidInput(
                "shadow request exceeds max_bytes; lower resolution/coverage or raise the explicit budget",
            ));
        }
        Ok(count)
    }
    pub(crate) fn bytes(&self, views: u32, limits: ShadowLimits) -> Result<u64, Error> {
        let stride = VIEW_BYTES.div_ceil(u64::from(limits.uniform_alignment))
            * u64::from(limits.uniform_alignment);
        u64::from(self.resolution)
            .checked_mul(u64::from(self.resolution))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(stride + VIEW_BYTES))
            .and_then(|n| n.checked_mul(u64::from(views)))
            .ok_or(Error::InvalidInput("shadow byte count overflow"))
    }
}
fn validate_range(near: f32, far: f32) -> Result<(), Error> {
    if !near.is_normal() || !far.is_normal() || near <= 0.0 || far <= near {
        return Err(Error::InvalidInput(
            "shadow radial range must have finite normal 0 < near < far",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;
    fn limits() -> ShadowLimits {
        ShadowLimits {
            max_resolution: 8192,
            max_views: 256,
            max_buffer_bytes: 1 << 28,
            uniform_alignment: 256,
        }
    }
    #[test]
    fn shadow_policy_rejects_lost_coverage_and_no_fit() {
        let light = Light::Directional {
            travel: -Vec3::Y,
            lux: [1.; 3],
        };
        let mut s = Shadows {
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 30.],
            }],
            ..Default::default()
        };
        assert_eq!(s.validate(&[light], limits()).unwrap(), 2);
        assert!(s.validate(&[], limits()).is_err());
        assert!(
            s.validate(
                &[Light::Point {
                    position: Vec3::ZERO,
                    candela: [1.; 3],
                    range: None
                }],
                limits()
            )
            .is_err()
        );
        s.max_bytes = Some(1);
        assert!(s.validate(&[light], limits()).is_err());
        s.max_bytes = None;
        assert!(s.validate(&[light], limits()).is_ok());
        s.resolution = 8193;
        assert!(s.validate(&[light], limits()).is_err());
    }
    #[test]
    fn shadow_spot_boundary_does_not_narrow_m5_cone() {
        let light = Light::Spot {
            position: Vec3::ZERO,
            travel: -Vec3::Z,
            candela: [1.; 3],
            range: None,
            inner: 0.1,
            outer: std::f32::consts::FRAC_PI_2,
        };
        assert!(Shadows::default().validate(&[light], limits()).is_ok());
        let s = Shadows {
            requests: vec![ShadowRequest::Spot {
                near: 0.001,
                far: 30.,
            }],
            ..Default::default()
        };
        assert!(s.validate(&[light], limits()).is_err());
    }
}
