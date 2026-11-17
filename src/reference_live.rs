//! Live reference: path tracing the engine's own scene, so every material program runs as the
//! engine runs it (docs/design-reference-live.md). Measurement only; never in the frame loop.
//!
//! Each bounce's hits are queued, every program present stores the material records of its own
//! draws' hits through `Stage::Reference` (shaders/reference_live_entry.wgsl), and one
//! program-free kernel shades the records with src/reference.rs's estimator.

/// One queued path hit, as shaders/reference_live_entry.wgsl `RfHit`: the placement and triangle
/// the ray met, its barycentrics, the ray's origin and flags (`RF_HIT`).
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RfHit {
    pub placement: u32,
    pub primitive: u32,
    pub bary: [f32; 2],
    pub origin: [f32; 3],
    pub flags: u32,
}

/// A hit's material record, as shaders/reference_live_entry.wgsl `RfRecord`: shading normal and
/// coverage, base colour and metallic, emission and roughness, world point and lighting model,
/// geometric normal (toward the ray) and the receiver's offset epsilon.
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RfRecord {
    pub normal_alpha: [f32; 4],
    pub base_metal: [f32; 4],
    pub emission_rough: [f32; 4],
    pub world_model: [f32; 4],
    pub geometric: [f32; 4],
}

/// Bytes per queued hit, material record and path (shaders/reference_live*.wgsl).
pub(crate) const RF_HIT_BYTES: u64 = std::mem::size_of::<RfHit>() as u64;
pub(crate) const RF_RECORD_BYTES: u64 = std::mem::size_of::<RfRecord>() as u64;
pub(crate) const RF_PATH_BYTES: u64 = 80;
/// shaders/reference_live.wgsl `Frame`: eight camera vec4s, four lighting vec4s, two uvec4s.
pub(crate) const FRAME_BYTES: usize = 14 * 16;

/// Whether a live reference is requested (`RENDERLAB_LIVE_REFERENCE`, src/engine/live_reference.rs):
/// the renderer then keeps each prepared frame's draw programs.
pub(crate) fn requested() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RENDERLAB_LIVE_REFERENCE").is_some())
}

/// The program-free kernels with the engine's shadow caster mask.
pub(crate) fn kernel_source() -> String {
    crate::shadows::with_shadow_caster_mask(include_str!("../shaders/reference_live.wgsl"))
}

/// What the engine knows of the frame to trace and the renderer does not: the camera, the sun
/// (dimmed by the atmosphere's transmittance), the sky and the local lights, as
/// src/engine/scene_dump.rs records them for the dump tracer.
pub(crate) struct LiveInputs {
    pub eye: crate::Vec3,
    pub forward: crate::Vec3,
    pub right: crate::Vec3,
    pub up: crate::Vec3,
    pub tan_half_fov: f32,
    pub size: [u32; 2],
    pub sun: Option<crate::reference::Sun>,
    /// Constant sky radiance when `environment` is none.
    pub sky: [f32; 3],
    pub environment: Option<crate::reference::Environment>,
    pub lights: Vec<crate::reference::LocalLight>,
    pub samples: u32,
    /// Path segments (src/reference.rs Settings::max_depth: 1 sees emitters only, 2 is direct).
    pub max_depth: u32,
    pub seed: u32,
    /// Print progress at power-of-two sample counts.
    pub progress: bool,
}

/// shaders/reference_live.wgsl `Frame` for `sample`. The sun is packed as src/reference.rs packs
/// it: a disk of radiance E / (pi h (2 - h)) with h = 1 - cos(r) computed stably, or a delta sun of
/// illuminance E.
pub(crate) fn frame_bytes(inputs: &LiveInputs, sample: u32) -> Vec<u8> {
    let camera = [
        inputs.eye.extend(inputs.tan_half_fov).to_array(),
        inputs.forward.extend(0.0).to_array(),
        inputs.right.extend(0.0).to_array(),
        inputs.up.extend(0.0).to_array(),
    ];
    let mut floats: Vec<[f32; 4]> = Vec::with_capacity(12);
    floats.extend(camera);
    floats.extend(camera);
    let (sun, value) = match inputs.sun {
        Some(s) => {
            let d = s.to_light.normalize();
            let h = (2.0 * (0.5 * f64::from(s.angular_radius)).sin().powi(2)) as f32;
            let scale = if h == 0.0 { 1.0 } else { (1.0 / (std::f64::consts::PI * f64::from(h) * (2.0 - f64::from(h)))) as f32 };
            let [r, g, b] = s.illuminance;
            ([d.x, d.y, d.z, 1.0], [r * scale, g * scale, b * scale, h])
        }
        None => ([0.0, 1.0, 0.0, 0.0], [0.0; 4]),
    };
    floats.push(sun);
    floats.push(value);
    let size = inputs.environment.as_ref().map_or(0.0, |e| e.radiance.size() as f32);
    floats.push([inputs.sky[0], inputs.sky[1], inputs.sky[2], size]);
    let (sin, cos, intensity) = inputs.environment.as_ref().map_or((0.0, 1.0, 0.0), |e| {
        let (sin, cos) = e.yaw.sin_cos();
        (sin, cos, e.intensity)
    });
    floats.push([cos, sin, intensity, 0.0]);
    let mut bytes: Vec<u8> = floats.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    let words = [inputs.size[0], inputs.size[1], sample, inputs.lights.len() as u32, inputs.max_depth, inputs.seed, 0, 0];
    bytes.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    debug_assert_eq!(bytes.len(), FRAME_BYTES);
    bytes
}

/// The environment's texels with their sampling CDF (src/reference.rs), or one empty texel.
pub(crate) fn environment_bytes(inputs: &LiveInputs) -> Vec<u8> {
    let texels = inputs.environment.as_ref().map_or_else(|| vec![[0.0; 4]], crate::reference::inputs::environment_data);
    texels.iter().flatten().flat_map(|f| f.to_le_bytes()).collect()
}

/// The local lights as shaders/reference_live.wgsl `LocalLight` (kind 1 point, 2 spot), or one
/// empty record.
pub(crate) fn light_bytes(inputs: &LiveInputs) -> Vec<u8> {
    let mut floats: Vec<[f32; 4]> = Vec::new();
    for l in &inputs.lights {
        let (travel, inner, outer, kind) = match l.spot {
            Some((travel, inner, outer)) => (travel.normalize(), inner, outer, 2.0),
            None => (crate::Vec3::ZERO, 1.0, -1.0, 1.0),
        };
        floats.push(l.position.extend(l.range).to_array());
        floats.push([l.candela[0], l.candela[1], l.candela[2], kind]);
        floats.push(travel.extend(outer).to_array());
        floats.push([inner, 0.0, 0.0, 0.0]);
    }
    if floats.is_empty() {
        floats.extend([[0.0; 4]; 4]);
    }
    floats.iter().flatten().flat_map(|f| f.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{Stage, compose, declares_parameters};

    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");

    /// Ara's terrain shape: the standard program with its `surface` replaced, reading parameters
    /// and sampling its atlas with explicit gradients (ara-renderlab src/splat.rs).
    fn splat_like() -> String {
        let standard = crate::STANDARD_PROGRAM;
        let start = standard.find("fn surface(input:SurfaceInput) -> Surface {").unwrap();
        let end = start + standard[start..].find("\n}\n").unwrap() + 3;
        format!(
            "struct Parameters {{ uv:array<vec4<f32>,4>, tint:array<vec4<f32>,4> }};\n{}fn surface(input:SurfaceInput) -> Surface {{\n    let c=sample_base_color_grad(input,input.uv*params.uv[0].xy,input.uv_dx,input.uv_dy);\n    let w=sample_metallic_roughness(input);\n    return Surface(material_normal(input),c.rgb*params.tint[0].rgb*w.r,0.0,0.5,1.0);\n}}\n{}",
            &standard[..start],
            &standard[end..]
        )
    }

    /// The reference stage composes and validates for every kind of program on the ray-hardware
    /// shading head: the standard program and Ara's terrain shape store `material_out`, a
    /// program without the record mapping (toon) and one with parameters but no mapping store the
    /// engine's factors. Its entry reaches no group-0 binding but its own (90-93): the reference
    /// binds the engine's arrays, materials and lighting at groups 1-3, and only its queue at 0.
    #[test]
    fn reference_stage_composes_for_every_program_kind() {
        let heads = crate::renderer::primary_ray_gpu::shading_heads(PRELUDE).unwrap();
        let emissive = "struct Parameters { glow:vec3<f32> };\nstruct Surface { n:vec3<f32>, albedo:vec3<f32> };\nfn surface(input:SurfaceInput)->Surface { return Surface(input.geometric,sample_base_color(input).rgb); }\nfn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return expose(s.albedo+params.glow); }\nfn emission(s:Surface,input:SurfaceInput)->vec3<f32> { return params.glow; }\n";
        let splat = splat_like();
        let programs = [
            ("standard", crate::STANDARD_PROGRAM, true),
            ("toon", include_str!("../shaders/programs/toon.wgsl"), false),
            ("emissive", emissive, false),
            ("splat", splat.as_str(), true),
        ];
        for (name, program, mapped) in programs {
            let composed = compose(&heads[1], program, Stage::Reference, declares_parameters(program));
            let source = &composed.source;
            assert!(source.contains("fn rf_materials("), "{name}");
            assert_eq!(source.contains("return material_out(s,input);"), mapped, "{name}");
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            let entry = module.entry_points.iter().position(|e| e.name == "rf_materials").unwrap();
            let uses = info.get_entry_point(entry);
            for (handle, global) in module.global_variables.iter() {
                if let Some(b) = global.binding.as_ref().filter(|_| !uses[handle].is_empty()) {
                    assert!(
                        b.group != 0 || (90..=93).contains(&b.binding),
                        "{name}: the reference entry reaches group 0 binding {} ({:?})",
                        b.binding,
                        global.name
                    );
                }
            }
        }
        // The Rust records match the WGSL's sizes: 32 and 80 bytes.
        assert_eq!(std::mem::size_of::<RfHit>(), 32);
        assert_eq!(std::mem::size_of::<RfRecord>(), 80);
    }

    /// The program-free kernels validate with the shadow caster mask, declare the four entry
    /// points, and their `Path` and `Frame` sizes are what the host allocates and writes.
    #[test]
    fn reference_kernels_validate() {
        let source = kernel_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        for name in ["rf_begin", "rf_trace", "rf_shade", "rf_end"] {
            assert!(module.entry_points.iter().any(|e| e.name == name), "{name}");
        }
        let size = |name: &str| {
            let (handle, _) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap();
            module.types[handle].inner.size(module.to_ctx()) as usize
        };
        assert_eq!(size("Path") as u64, RF_PATH_BYTES);
        assert_eq!(size("Frame"), FRAME_BYTES);
        let inputs = LiveInputs {
            eye: crate::Vec3::ZERO,
            forward: crate::Vec3::Z,
            right: crate::Vec3::X,
            up: crate::Vec3::Y,
            tan_half_fov: 0.5,
            size: [4, 2],
            sun: None,
            sky: [1.0; 3],
            environment: None,
            lights: vec![],
            samples: 2,
            max_depth: 4,
            seed: 1,
            progress: false,
        };
        assert_eq!(frame_bytes(&inputs, 0).len(), FRAME_BYTES);
        assert_eq!(light_bytes(&inputs).len(), 64);
        assert_eq!(environment_bytes(&inputs).len(), 16);
    }
}
