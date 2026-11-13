//! Matched plain path-traced reference using the existing independently cross-checked tracer.
#[path = "../tools/ara1/scene.rs"]
mod scene;
use renderlab::{Mat4, Vec3, asset, engine::headless_adapter, reference};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::Instant};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn codes(pixels: &[[f32; 3]], stops: f32) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|pixel| {
            let rgb = pixel.map(|value| {
                let x = (value * stops.exp2()).max(0.0);
                let mapped =
                    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
                let srgb = if mapped <= 0.0031308 {
                    12.92 * mapped
                } else {
                    1.055 * mapped.powf(1.0 / 2.4) - 0.055
                };
                (srgb * 255.0).round() as u8
            });
            [rgb[0], rgb[1], rgb[2], 255]
        })
        .collect()
}
fn png(path: &Path, size: [u32; 2], pixels: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = png::Encoder::new(fs::File::create(path)?, size[0], size[1]);
    writer.set_color(png::ColorType::Rgba);
    writer.set_depth(png::BitDepth::Eight);
    writer.write_header()?.write_image_data(pixels)?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let begin = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 || args[1] != "reference" {
        return Err("ara1_reference SCENE.json reference OUT".into());
    }
    if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref()
        != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    {
        return Err("explicit experimental-ray-query acknowledgement required".into());
    }
    let spec = scene::load(Path::new(&args[0]))?;
    if spec.effects.atmosphere || spec.effects.fog || spec.effects.post || spec.effects.alpha_panel
    {
        return Err(
            "this reference is for the opaque clear-medium target, with matching display curve"
                .into(),
        );
    }
    if !spec.lights.is_empty() {
        return Err(
            "the existing reference API has sun/sky only; local-light references are unavailable"
                .into(),
        );
    }
    let out = Path::new(&args[2]);
    fs::create_dir_all(out.join("reference-halves"))?;
    let encoded = fs::read(&spec.asset)?;
    let a = asset::decode(&encoded, asset::Limits::unlimited())?;
    drop(encoded);
    let mut meshes = Vec::new();
    let mut mesh_map = Vec::new();
    for mesh in &a.meshes {
        let mut primitives = Vec::new();
        for p in &mesh.primitives {
            primitives.push((meshes.len(), p.material as usize));
            meshes.push(reference::Mesh {
                positions: p.vertices.iter().map(|v| v.position).collect(),
                normals: p.vertices.iter().map(|v| v.normal).collect(),
                uvs: p.vertices.iter().map(|v| v.uv).collect(),
                colors: p.vertices.iter().map(|v| v.color).collect(),
                tangents: p
                    .tangents
                    .as_ref()
                    .map_or_else(Vec::new, |t| t.values.clone()),
                indices: p.indices.clone(),
            });
        }
        mesh_map.push(primitives);
    }
    let mut instances = Vec::new();
    for p in &a.placements {
        for &(mesh, material) in &mesh_map[p.mesh as usize] {
            instances.push(reference::Instance {
                casts_shadows: true,
                mesh,
                material,
                transform: Mat4::from_cols_array_2d(&p.transform),
            });
        }
    }
    let materials = a
        .materials
        .iter()
        .map(|m| reference::Material {
            base_color: [m.base_color[0], m.base_color[1], m.base_color[2]],
            metallic: m.metallic,
            roughness: m.roughness,
            base_color_texture: m.base_color_texture,
            metallic_roughness_texture: m.metallic_roughness_texture,
            normal_texture: m.normal_texture,
            ..Default::default()
        })
        .collect();
    let source = reference::Scene {
        meshes,
        materials,
        instances,
        sun: Some(reference::Sun {
            to_light: Vec3::from_array(spec.to_light),
            illuminance: spec.sun_lux,
            angular_radius: spec.sun_radius,
        }),
        sky: spec.environment,
        textures: a.textures,
        ..Default::default()
    };
    drop(a.meshes);
    drop(a.materials);
    drop(a.placements);
    let adapter = headless_adapter()?;
    let limits = adapter.limits();
    let limits = renderlab::traced_limits(wgpu::Limits {
        max_blas_primitive_count: limits.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 4096.min(limits.max_tlas_instance_count),
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_buffer_size: limits.max_buffer_size,
        ..Default::default()
    });
    // SAFETY: the caller supplied the same explicit acknowledgement as the renderer tests.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: reference::FEATURES | renderlab::TRACED_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: renderlab::MEMORY_HINTS,
        ..Default::default()
    }))?;
    let samples = std::env::var("ARA1_REFERENCE_SAMPLES")
        .ok()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(1024);
    let selected: Option<Vec<usize>> = std::env::var("ARA1_REFERENCE_INDICES")
        .ok()
        .map(|v| v.split(',').map(str::parse).collect())
        .transpose()?;
    let scene_sha = hex(&Sha256::digest(fs::read(&args[0])?));
    let mut rows = Vec::new();
    let mut first = None;
    for (index, f) in spec
        .frames
        .iter()
        .enumerate()
        .filter(|(i, f)| f.capture && selected.as_ref().is_none_or(|s| s.contains(i)))
    {
        let pose = reference::Pose {
            eye: Vec3::from_array(f.eye),
            target: Vec3::from_array(f.target),
        };
        let view = reference::View {
            open: pose,
            close: pose,
            up: Vec3::Y,
            fov_y: f.fov_y,
            width: spec.size[0],
            height: spec.size[1],
        };
        let start = Instant::now();
        let image = reference::render(
            &device,
            &queue,
            &source,
            &view,
            &reference::Settings {
                samples,
                ..Default::default()
            },
        )?;
        let seconds = start.elapsed().as_secs_f64();
        if first.is_none() {
            first = Some(begin.elapsed().as_secs_f64() * 1e3);
            fs::write(
                out.join("first-image.ready"),
                b"path-traced output read back",
            )?;
        }
        let halves = [
            codes(&image.halves[0], spec.exposure_stops),
            codes(&image.halves[1], spec.exposure_stops),
        ];
        let noise = (halves[0]
            .iter()
            .zip(&halves[1])
            .enumerate()
            .filter(|(i, _)| i % 4 != 3)
            .map(|(_, (&a, &b))| (f64::from(a) - f64::from(b)).powi(2) / 4.0)
            .sum::<f64>()
            / (image.mean.len() * 3) as f64)
            .sqrt();
        let name = format!("frame-{index:04}.png");
        let path = out.join(&name);
        png(&path, spec.size, &codes(&image.mean, spec.exposure_stops))?;
        for (k, values) in halves.iter().enumerate() {
            png(
                &out.join("reference-halves")
                    .join(format!("frame-{index:04}-{k}.png")),
                spec.size,
                values,
            )?;
        }
        fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&json!({"scene_sha256":scene_sha,"frame_index":index,
            "image_sha256":hex(&Sha256::digest(fs::read(&path)?)),"display_transform":"AcesFit+sRGB","accepted_for_scoring":image.invalid_pixels==0,
            "method":"plain unidirectional path tracing, NEE, MIS and Russian roulette; no temporal/spatial reuse or denoising",
            "material_model":"Renderlab standard Lambert+height-correlated GGX; transport tracer cross-checked on independent Mitsuba fixtures, not an independent engine material model",
            "samples":samples,"max_depth":1024,"invalid_pixels":image.invalid_pixels,"split_half_mean_noise_rms_codes":noise,
            "reference_seconds":seconds,"quality_limit":"finite-sample reference; compare errors with split-half uncertainty; no film-exact certificate"}))?,
        )?;
        rows.push(json!({"index":index,"phase":f.phase,"reference_seconds":seconds,"samples":samples,"noise_rms_codes":noise,"invalid_pixels":image.invalid_pixels}));
        eprintln!(
            "reference frame={index} samples={samples} seconds={seconds:.3} noise={noise:.3}"
        );
    }
    fs::write(
        out.join("engine.json"),
        serde_json::to_vec_pretty(
            &json!({"engine":"renderlab-reference","path":"reference","adapter":adapter.get_info().name,
        "size":spec.size,"startup_to_readback_ms":first,"frames":rows,"samples":samples,"debug_build":cfg!(debug_assertions)}),
        )?,
    )?;
    Ok(())
}
