//! Explicit optional wgpu ray-query shadows. Device negotiation belongs to the caller.
use crate::{Error, Light, SampleCount, ShadowRequest, Shadows};

/// Opt-in fixed at renderer creation. The caller must request
/// `Features::EXPERIMENTAL_RAY_QUERY` and explicitly acknowledge wgpu's experimental
/// safety contract in its own `DeviceDescriptor`. Renderlab never enables it implicitly.
///
/// Initial support: regular, immutable geometry; opaque double-sided materials;
/// directional hard shadows; 1x or per-sample 4x visibility. Virtual geometry is
/// rejected while shadowed. 4x requires the adapter's MULTISAMPLED_SHADING capability.
#[derive(Clone, Copy, Debug)]
pub struct HardwareShadowOptions {
    /// Admission estimate for all owned AS generations and pending build scratch,
    /// including meshes still owned by callers. None disables this policy only.
    /// Safe wgpu has no preallocation size query: this is not a driver memory cap.
    pub max_estimated_bytes: Option<u64>,
}
impl Default for HardwareShadowOptions {
    fn default() -> Self {
        Self {
            max_estimated_bytes: Some(2 * 1024 * 1024 * 1024),
        }
    }
}
impl HardwareShadowOptions {
    pub(crate) fn validate(
        self,
        features: wgpu::Features,
        limits: &wgpu::Limits,
    ) -> Result<(), Error> {
        if !features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
            || limits.max_acceleration_structures_per_shader_stage < 1
            || limits.max_blas_geometry_count < 1
            || limits.max_blas_primitive_count == 0
            || limits.max_tlas_instance_count == 0
        {
            return Err(Error::InvalidInput(
                "hardware shadows require caller-enabled EXPERIMENTAL_RAY_QUERY and nonzero AS device limits",
            ));
        }
        Ok(())
    }
    pub(crate) fn admit(self, owned: u64, additional: u64) -> Result<(), Error> {
        let total = owned.checked_add(additional).ok_or(Error::InvalidInput(
            "hardware shadow memory estimate overflow",
        ))?;
        if self.max_estimated_bytes.is_some_and(|max| total > max) {
            return Err(Error::InvalidInput(
                "hardware shadows exceed max_estimated_bytes (includes retained generations and pending scratch)",
            ));
        }
        Ok(())
    }
}

/// Counts belong to this renderer. Allocator observations are optional, backend-specific
/// attributed bytes, not physical residency. Scratch estimates retire with their flight.
#[derive(Clone, Copy, Debug, Default)]
pub struct HardwareShadowStats {
    /// Cumulative immutable mesh builds submitted by this renderer.
    pub blas_builds: u64,
    /// Cumulative placement-generation builds submitted by this renderer.
    pub tlas_builds: u64,
    /// Placements in the active scene, including camera-invisible casters.
    pub active_instances: u32,
    /// Replaced or disabled scene generations awaiting their last submission.
    pub retained_generations: usize,
    /// Mesh or scene builds whose submissions have not yet been collected.
    pub pending_builds: usize,
    /// Live owned acceleration estimates, including caller meshes and retired scenes.
    pub estimated_as_bytes: u64,
    /// Build scratch estimates awaiting submission completion.
    pub estimated_pending_scratch_bytes: u64,
    /// Named allocator attribution for this renderer's live structures. Labels are
    /// unique across renderers, but these bytes do not establish physical residency.
    pub observed_as_bytes: Option<u64>,
    /// All currently reported wgpu scratch allocations immediately after the latest
    /// build submission; may include other internal work. None means unavailable.
    pub observed_build_scratch_bytes: Option<u64>,
}

pub(crate) fn validate_requests(lights: &[Light], shadows: &Shadows) -> Result<(), Error> {
    for (light, request) in lights.iter().zip(&shadows.requests) {
        if !matches!(request, ShadowRequest::None)
            && !matches!(
                (light, request),
                (Light::Directional { .. }, ShadowRequest::Directional { .. })
            )
        {
            return Err(Error::InvalidInput(
                "hardware shadows currently support directional shadow requests only",
            ));
        }
    }
    Ok(())
}

pub(crate) const VISIBILITY_BEGIN: &str = "// renderlab: direct visibility begin";
pub(crate) const VISIBILITY_END: &str = "// renderlab: direct visibility end";
const RECEIVER_WORLD: &str = "@location(4) world:vec3<f32>,";
const SAMPLE_RECEIVER_WORLD: &str =
    "@location(4) @interpolate(perspective, sample) world:vec3<f32>,";

/// A small, explicit shader seam keeps the normal one-file reload. The optional
/// extension and AS binding exist only in the hardware pipeline's shader module.
pub(crate) fn shader_source(source: &str, samples: SampleCount) -> Result<String, Error> {
    let vertices = |text: &str| {
        text.split_once("// renderlab: affine vertices begin")
            .and_then(|(_, rest)| {
                rest.split_once("// renderlab: affine vertices end")
                    .map(|(body, _)| body.replace("\r\n", "\n"))
            })
    };
    if vertices(source).is_none()
        || vertices(source) != vertices(include_str!("../shaders/scene.wgsl"))
    {
        return Err(Error::InvalidInput(
            "hardware shadows require the unchanged affine vertex functions; displaced visibility is unsupported",
        ));
    }
    let (before, rest) = source
        .split_once(VISIBILITY_BEGIN)
        .ok_or(Error::InvalidInput(
            "hardware shadow shader requires direct visibility markers",
        ))?;
    let (_, after) = rest.split_once(VISIBILITY_END).ok_or(Error::InvalidInput(
        "hardware shadow shader requires closing direct visibility marker",
    ))?;
    if before.contains(VISIBILITY_END)
        || rest.contains(VISIBILITY_BEGIN)
        || after.contains(VISIBILITY_END)
    {
        return Err(Error::InvalidInput(
            "hardware shadow visibility markers must occur exactly once, in order",
        ));
    }
    let composed = format!(
        "enable wgpu_ray_query;\n{before}\n{}\n{after}",
        include_str!("../shaders/hardware_shadows.wgsl")
    );
    match samples {
        SampleCount::One => Ok(composed),
        SampleCount::Four => {
            if composed.matches(RECEIVER_WORLD).count() != 1 {
                return Err(Error::InvalidInput(
                    "4x hardware shadows require the unchanged receiver world declaration",
                ));
            }
            // Sample-rate execution alone would still query pixel-centre world positions.
            Ok(composed.replacen(RECEIVER_WORLD, SAMPLE_RECEIVER_WORLD, 1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_policy_requires_explicit_device_feature_and_limits() {
        let options = HardwareShadowOptions::default();
        let limits = wgpu::Limits {
            max_blas_primitive_count: 1,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 1,
            max_acceleration_structures_per_shader_stage: 1,
            ..Default::default()
        };
        assert!(options.validate(wgpu::Features::empty(), &limits).is_err());
        assert!(
            options
                .validate(
                    wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                    &wgpu::Limits::default(),
                )
                .is_err()
        );
        assert!(
            options
                .validate(wgpu::Features::EXPERIMENTAL_RAY_QUERY, &limits,)
                .is_ok()
        );
    }
    #[test]
    fn hardware_budget_counts_retained_and_pending_work() {
        let options = HardwareShadowOptions {
            max_estimated_bytes: Some(100),
        };
        assert!(options.admit(80, 20).is_ok());
        assert!(options.admit(80, 21).is_err());
        let unlimited = HardwareShadowOptions {
            max_estimated_bytes: None,
        };
        assert!(unlimited.admit(u64::MAX, 1).is_err());
        assert!(unlimited.admit(80, 21).is_ok());
    }
    #[test]
    fn hardware_reload_seam_rejects_missing_markers_and_displacement() {
        let source = include_str!("../shaders/scene.wgsl");
        let composed = shader_source(source, SampleCount::One).unwrap();
        assert!(composed.starts_with("enable wgpu_ray_query;"));
        assert_eq!(composed.matches("fn hard_shadow(").count(), 1);
        assert!(shader_source(&source.replace(VISIBILITY_END, ""), SampleCount::One).is_err());
        assert!(shader_source(&source.replace(VISIBILITY_BEGIN, ""), SampleCount::One).is_err());
        assert!(
            shader_source(
                &source.replace("vec4<f32>(position, 1.0)", "vec4<f32>(position*2.0, 1.0)"),
                SampleCount::One,
            )
            .is_err()
        );
        assert!(
            shader_source(
                &source.replace(
                    "const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);",
                    "const TINT: vec3<f32> = vec3<f32>(0.5);"
                ),
                SampleCount::One
            )
            .is_ok()
        );
    }
    #[test]
    fn hardware_reload_accepts_lf_and_crlf_but_still_rejects_displacement() {
        let lf = include_str!("../shaders/scene.wgsl").replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        for samples in [SampleCount::One, SampleCount::Four] {
            let a = shader_source(&lf, samples).unwrap();
            let b = shader_source(&crlf, samples).unwrap();
            assert_eq!(a.replace("\r\n", "\n"), b.replace("\r\n", "\n"));
            let displaced =
                crlf.replace("vec4<f32>(position, 1.0)", "vec4<f32>(position*2.0, 1.0)");
            assert!(shader_source(&displaced, samples).is_err());
        }
    }
    #[test]
    fn hardware_four_samples_interpolate_receiver_at_each_sample() {
        let source = include_str!("../shaders/scene.wgsl");
        let one = shader_source(source, SampleCount::One).unwrap();
        let four = shader_source(source, SampleCount::Four).unwrap();
        assert!(!one.contains(SAMPLE_RECEIVER_WORLD));
        assert_eq!(four.matches(SAMPLE_RECEIVER_WORLD).count(), 1);
        assert_eq!(four.replace(SAMPLE_RECEIVER_WORLD, RECEIVER_WORLD), one);
        assert!(shader_source(&source.replace(RECEIVER_WORLD, ""), SampleCount::Four).is_err());
        let duplicate = format!("{source}\n{RECEIVER_WORLD}");
        assert!(shader_source(&duplicate, SampleCount::Four).is_err());
    }
    #[test]
    fn hardware_rejects_local_shadow_requests() {
        let light = Light::Point {
            position: crate::Vec3::ZERO,
            candela: [1.; 3],
            range: None,
        };
        assert!(
            validate_requests(
                &[light],
                &Shadows {
                    requests: vec![ShadowRequest::Point {
                        near: 0.1,
                        far: 10.
                    }],
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(validate_requests(&[light], &Shadows::default()).is_ok());
    }
}
