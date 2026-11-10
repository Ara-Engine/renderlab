//! IQ1: per-pixel diagnostics of the registered quarry image (docs/iq1-quarry-image.md).
//! One process per path loads the quarry once and renders U0b's full-radiance protocol
//! (own materials and textures, the sun with its 0.04 rad disk, the RLE2 sky, a black
//! backdrop, cache off). Diagnostic programs are swapped into the standard program in
//! place, so every variant shades the same materials, textures and hits. Raw outputs go to
//! IQ1_OUT for CPU analysis (scripts/iq1-analyse.py). Correctness only: no timing claim.
use super::*;
use std::path::PathBuf;

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod quarry_cameras;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../support/environment_controls.rs"]
mod quarry_environment;
#[allow(clippy::duplicate_mod)]
#[path = "../support/limits.rs"]
mod quarry_policy;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod quarry_prepared;

const SOURCE: &str = include_str!("../shaders/scene.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Per-pixel shading inputs of the first light, as raw program output (not radiance):
/// R the engine's visibility, G and B the shading and viewed-side geometric N.L mapped
/// from [-1, 1] to [0, 1].
const SHADOW: &str = "struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal(input)); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let light=light_sample(0u,input);
    let g=select(-input.geometric,input.geometric,dot(input.geometric,input.eye-input.world)>=0.0);
    return vec3<f32>(light_visibility(input,light),dot(s.n,light.direction)*0.5+0.5,dot(g,light.direction)*0.5+0.5);
}
";
/// R: the shading normal against the view, G: the viewed-side vertex normal against the
/// light (both mapped to [0, 1]); B: flags (1 source-front, 2 normal-mapped, 4 no vertex
/// normal, 8 shading normal faces away from the view).
const FRAME: &str = "struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal(input)); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let light=light_sample(0u,input);
    let v=unit_normal(input.eye-input.world);
    var nv=unit_normal(input.normal);
    if input.material.double_sided!=0u && !input.source_front { nv=-nv; }
    let flags=f32(input.source_front)+2.0*f32(input.material.normal_mapped!=0u)
        +4.0*f32(dot(input.normal,input.normal)<=1e-12)+8.0*f32(dot(s.n,v)<0.0);
    return vec3<f32>(dot(s.n,v)*0.5+0.5,dot(nv,light.direction)*0.5+0.5,flags);
}
";
/// One pixel's footprint: R, G the uv gradients' lengths in 4096-texel units (the quarry's
/// textures are 4096 square), B the world-space footprint in millimetres.
const FOOTPRINT: &str = "struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface { return Surface(input.geometric); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return vec3<f32>(length(input.uv_dx)*4096.0,length(input.uv_dy)*4096.0,
        max(length(input.world_dx),length(input.world_dy))*1000.0);
}
";
/// Unlit albedo: the base factor, vertex colour and sampled base colour, as raw output.
const ALBEDO: &str = "struct Surface { base:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    let n=material_normal(input);
    let s=sample_base_color(input);
    return Surface(input.material.base_color.rgb*input.color.rgb*s.rgb+0.0*n);
}
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> { return s.base; }
";

/// The standard program with one edit, checked to apply.
fn standard_with(from: &str, to: &str) -> String {
    assert!(crate::STANDARD_PROGRAM.contains(from), "{from}");
    crate::STANDARD_PROGRAM.replace(from, to)
}

/// The variants a run renders, by name: (program source, true when its output is radiance).
fn variant(name: &str) -> (String, bool) {
    let base = "let base=clamp(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,vec3<f32>(0.0),vec3<f32>(1.0));";
    let normal = "let n=material_normal(input);";
    match name {
        "standard" => (crate::STANDARD_PROGRAM.to_string(), true),
        "shadow" => (SHADOW.to_string(), false),
        "frame" => (FRAME.to_string(), false),
        "footprint" => (FOOTPRINT.to_string(), false),
        "albedo" => (ALBEDO.to_string(), false),
        // Lighting only: every texture and vertex colour replaced by albedo 0.5.
        "light05" => (
            standard_with(base, "let base=vec3<f32>(0.5)+0.0*sampled.rgb;"),
            true,
        ),
        // No normal map: the interpolated vertex normal, reversed on source-back faces.
        "nomap" => (
            standard_with(
                normal,
                "var n=unit_normal(input.normal); if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; } \
                 if material.double_sided!=0u && !input.source_front { n=-n; } let unused=material_normal(input);",
            ),
            true,
        ),
        // Material textures one level finer (half the pixel's uv footprint): for images
        // that integrate jittered samples, whose per-sample filtering doubles the blur.
        "lodm1" => (
            standard_with(
                "let sampled=sample_base_color(input);\n    let mr=sample_metallic_roughness(input);",
                "let sampled=sample_base_color_grad(input,input.uv,input.uv_dx*0.5,input.uv_dy*0.5);\n    \
                 let mr=sample_metallic_roughness_grad(input,input.uv,input.uv_dx*0.5,input.uv_dy*0.5);",
            ),
            true,
        ),
        // Half a level finer (the footprint scaled by 2^-0.5).
        "lodm05" => (
            standard_with(
                "let sampled=sample_base_color(input);\n    let mr=sample_metallic_roughness(input);",
                "let sampled=sample_base_color_grad(input,input.uv,input.uv_dx*0.70710678,input.uv_dy*0.70710678);\n    \
                 let mr=sample_metallic_roughness_grad(input,input.uv,input.uv_dx*0.70710678,input.uv_dy*0.70710678);",
            ),
            true,
        ),
        // U1b's footprint-integrated direct BRDF (merged, off by default).
        "specaa" => (
            standard_with(
                "const SPECULAR_AA:bool=false;",
                "const SPECULAR_AA:bool=true;",
            ),
            true,
        ),
        // Faceted: the viewed-side geometric normal.
        "flat" => (
            standard_with(
                normal,
                "let unused=material_normal(input); let n=select(-input.geometric,input.geometric,dot(input.geometric,input.eye-input.world)>=0.0);",
            ),
            true,
        ),
        _ => panic!("unknown IQ1 variant {name}"),
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn rt_device() -> (wgpu::Device, wgpu::Queue) {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"),
        "GPU tests require explicit caller acknowledgment"
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let limits = crate::traced_limits(wgpu::Limits {
        max_blas_primitive_count: a.max_blas_primitive_count,
        max_blas_geometry_count: 1,
        max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
        max_acceleration_structures_per_shader_stage: 1,
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_buffer_size: a.max_buffer_size,
        ..Default::default()
    });
    // SAFETY: the test caller explicitly accepts wgpu's experimental implementation risk.
    let experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | crate::TRACED_FEATURES,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

/// A device without ray queries that selects the visibility buffer and Virtual shadows.
fn no_rt_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let a = adapter.limits();
    let mut limits = crate::visibility_limits(wgpu::Limits::default());
    limits.max_buffer_size = a.max_buffer_size;
    limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES,
        required_limits: limits,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}

/// The exposed HDR target as raw f32 RGB (no exposure undone).
fn capture(r: &mut Renderer) -> Vec<f32> {
    r.test_drain("IQ1 HDR admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("IQ1 HDR copy");
    capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(8)
        .flat_map(|p| {
            (0..3).map(move |k| crate::decode_f16(u16::from_le_bytes([p[2 * k], p[2 * k + 1]])))
        })
        .collect()
}

/// A texture's bytes, rows packed to `texel` bytes per pixel.
fn read_texture(r: &Renderer, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
    let size = texture.size();
    let row = (size.width * texel).next_multiple_of(256);
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("IQ1 readback"),
        size: u64::from(row * size.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    r.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    let data = slice.get_mapped_range().unwrap();
    let width = (size.width * texel) as usize;
    data.chunks_exact(row as usize)
        .flat_map(|line| line[..width].to_vec())
        .collect()
}

/// The deferred paths' hit records (placement + 1, primitive, barycentrics), zero outside
/// the shaded region and in uncovered tiles, whose records are stale.
fn hit_records(r: &mut Renderer) -> Vec<u8> {
    r.test_drain("IQ1 hit admission");
    let primary = r.primary.as_ref().unwrap();
    let targets = primary.targets.as_ref().unwrap();
    let region = primary.scene;
    let (hit, tiles) = (targets.hit.texture.clone(), targets.tiles.texture.clone());
    let mut data = read_texture(r, &hit, 16);
    let flags = read_texture(r, &tiles, 4);
    let tiles_width = tiles.size().width as usize;
    let [tile_w, tile_h] = primary_ray_gpu::TRACE_TILE.map(|t| t as usize);
    let w = hit.size().width as usize;
    for (i, record) in data.chunks_exact_mut(16).enumerate() {
        let (x, y) = (i % w, i / w);
        let inside = (region[0] as usize..region[2] as usize).contains(&x)
            && (region[1] as usize..region[3] as usize).contains(&y);
        let t = ((y / tile_h) * tiles_width + x / tile_w) * 4;
        if !inside || u32::from_le_bytes(flags[t..t + 4].try_into().unwrap()) == 0 {
            record.fill(0);
        }
    }
    data
}

fn write_f32(path: &std::path::Path, values: &[f32]) {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(path, bytes).unwrap();
}

/// IQ1's quarry diagnostics, one path per process (IQ1_PATH: `traced` = the RT device with
/// traced primaries and Interactive shadows, G1c's gate; `raster` = the RT device's code
/// default, raster primaries and Exact; `nonrt` = the visibility buffer and Virtual).
/// IQ1_VARIANTS names the programs (see `variant`); IQ1_JITTER=n also averages n x n
/// stratified sub-pixel offsets of the standard program (the pixel-integrated image
/// without any reconstruction); IQ1_ANISO sets the anisotropy clamp (default 1, the
/// renderer default); IQ1_TR1=n renders TR1's measured settings over n still frames;
/// IQ1_CACHE=1 adds the radiance cache's image (and its jittered average); IQ1_INTEGRATE
/// names the variant those integrated images shade with (default standard). Writes
/// `<path>-<variant>-pose<k>.f32` (raw RGB; radiance variants are exposed HDR, multiply
/// by 2^-exposure) and `<path>-pose<k>-hits.u32x4`.
#[test]
#[ignore = "GPU; optimized build; IQ1_QUARRY (G1c's manifest), IQ1_OUT, IQ1_PATH"]
fn gpu_iq1_quarry() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("IQ1_QUARRY").unwrap()).unwrap()).unwrap();
    // IQ1_OUT, or the step directory scripts/iq1-run.py plans set.
    let out = PathBuf::from(
        env("IQ1_OUT")
            .or_else(|| env("RENDERLAB_M7_G1_OUT"))
            .unwrap(),
    );
    std::fs::create_dir_all(&out).unwrap();
    let path_of = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let (policy, _) = quarry_policy::Policy::args(vec![
        "--limits".into(),
        path_of("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let limits = policy.prepared();
    let source = crate::asset::decode(&std::fs::read(path_of("asset")).unwrap(), limits).unwrap();
    let cameras = quarry_cameras::Cameras::load(&path_of("cameras")).unwrap();
    let path = env("IQ1_PATH").unwrap_or_else(|| "traced".into());
    let aniso: u16 = env("IQ1_ANISO").map_or(1, |s| s.parse().unwrap());
    // IQ1_SAMPLES=4: 4x MSAA, the raster path's only shipping anti-aliasing option.
    let samples = match env("IQ1_SAMPLES").as_deref() {
        Some("4") => SampleCount::Four,
        _ => SampleCount::One,
    };
    let options = RendererOptions {
        samples,
        anisotropy_clamp: aniso,
    };
    let renderer_limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let mut r = if path == "nonrt" {
        let (device, queue) = no_rt_device();
        Renderer::new_with_options(device, queue, FORMAT, renderer_limits, options, SOURCE).unwrap()
    } else {
        let (device, queue) = rt_device();
        Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            renderer_limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap()
    };
    if path != "nonrt" {
        // Registered IQ1 rows keep their named primary/Exact baseline after RTDEF2.
        r.set_primary_visibility(if path == "traced" {
            PrimaryVisibility::Traced
        } else {
            PrimaryVisibility::Raster
        })
        .unwrap();
        r.set_soft_shadows(SoftShadows::Exact);
    }
    // IQ1's 1x rows stay unreconstructed now that TR1 is the default (TR1b); its TR1 rows
    // enable it explicitly.
    // The GI cache is on by default since G2b; IQ1's 1x rows keep it off.
    r.set_indirect(IndirectSettings::off()).unwrap();
    r.set_reconstruction(Reconstruction::off()).unwrap();
    r.set_reconstruction_shadow_budget(crate::ReconstructionShadowBudget {
        cold_rays: 8,
        reset_rays: 8,
        active_rays: 8,
        held_rays: 8,
        settle_frames: 16,
        cut_rays: 8,
    })
    .unwrap();
    let lighting: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path_of("lighting")).unwrap()).unwrap();
    let v3 = |v: &serde_json::Value| -> [f32; 3] {
        std::array::from_fn(|k| v[k].as_f64().unwrap() as f32)
    };
    let exposure = manifest["exposure_stops"].as_f64().unwrap() as f32;
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: Vec3::from_array(v3(&lighting["lights"][0]["travel"])),
                lux: v3(&lighting["lights"][0]["lux"]),
            }],
            environment: [0.0; 3],
            exposure_stops: exposure,
        },
        &Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![4., 12., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    if path == "traced" {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    let map = quarry_environment::load_environment(
        &path_of("environment"),
        crate::asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&map).unwrap() {
            break sky;
        }
        r.test_drain("IQ1 environment");
    };
    r.set_environment(Some(&sky), Default::default()).unwrap();
    drop(map);
    let mut asset = quarry_prepared::Prepared::new_with_limits(source, limits, 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("IQ1 upload");
    let standard = r.standard_program().clone();
    let variants: Vec<String> = env("IQ1_VARIANTS")
        .unwrap_or_else(|| "standard".into())
        .split(',')
        .map(|v| v.trim().to_string())
        .collect();
    let jitter: u32 = env("IQ1_JITTER").map_or(0, |s| s.parse().unwrap());
    let integrate = env("IQ1_INTEGRATE").unwrap_or_else(|| "standard".into());
    let poses: Vec<usize> = env("IQ1_POSES")
        .unwrap_or_else(|| "0,1".into())
        .split(',')
        .map(|p| p.trim().parse().unwrap())
        .collect();
    let mut log = serde_json::json!({"path": path, "anisotropy_clamp": aniso, "samples": samples as u32, "variants": variants,
        "jitter": jitter, "exposure_stops": exposure, "primary": format!("{:?}", r.primary_mode)});
    for pose in poses {
        let frame = &cameras.frames[pose];
        let camera = frame.camera().unwrap();
        let target = r.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("IQ1 color"),
            size: wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut render = |r: &mut Renderer, camera: &Camera| {
            let mut draws = asset.draws(Visibility::Frustum);
            let until = Instant::now() + Duration::from_secs(60);
            while r
                .try_render_view(&target, camera, &mut draws, ViewMode::Lit)
                .unwrap()
                .is_none()
            {
                assert!(Instant::now() < until, "IQ1 admission");
                r.test_drain("IQ1 admission");
            }
        };
        for name in &variants {
            let (program, _) = variant(name);
            r.reload_program(&standard, &program).unwrap();
            render(&mut r, &camera);
            // Two frames, so per-frame state (Interactive's counts) is settled.
            render(&mut r, &camera);
            let image = capture(&mut r);
            write_f32(&out.join(format!("{path}-{name}-pose{pose}.f32")), &image);
            if name == "standard" && path != "raster" {
                let hits = hit_records(&mut r);
                std::fs::write(out.join(format!("{path}-pose{pose}-hits.u32x4")), hits).unwrap();
            }
        }
        // Integrated images shade with IQ1_INTEGRATE's program (default standard); its
        // name tags their files unless it is the standard program.
        let tag = if integrate == "standard" {
            String::new()
        } else {
            format!("-{integrate}")
        };
        r.reload_program(&standard, &variant(&integrate).0).unwrap();
        // The mean of `jitter` x `jitter` stratified sub-pixel offsets: the pixel-integrated
        // image without reconstruction.
        let jittered =
            |r: &mut Renderer, render: &mut dyn FnMut(&mut Renderer, &Camera), name: &str| {
                let mut sum = vec![0.0f64; (frame.width * frame.height * 3) as usize];
                for j in 0..jitter * jitter {
                    let offset =
                        [j % jitter, j / jitter].map(|k| (k as f32 + 0.5) / jitter as f32 - 0.5);
                    render(r, &camera.jittered(offset));
                    for (s, v) in sum.iter_mut().zip(capture(r)) {
                        *s += f64::from(v);
                    }
                }
                let mean: Vec<f32> = sum
                    .iter()
                    .map(|s| (*s / f64::from(jitter * jitter)) as f32)
                    .collect();
                write_f32(
                    &out.join(format!("{path}-{name}{tag}-pose{pose}.f32")),
                    &mean,
                );
            };
        if jitter > 0 {
            jittered(&mut r, &mut render, &format!("jitter{jitter}"));
        }
        // IQ1_TR1=n: TR1's measured settings over n still frames (its converged still image).
        if let Some(n) = env("IQ1_TR1").map(|s| s.parse::<u32>().unwrap()) {
            r.set_reconstruction(Reconstruction::identity()).unwrap();
            for _ in 0..n {
                render(&mut r, &camera);
            }
            write_f32(
                &out.join(format!("{path}-tr1x{n}{tag}-pose{pose}.f32")),
                &capture(&mut r),
            );
            r.set_reconstruction(Reconstruction::off()).unwrap();
        }
        // IQ1_CACHE=1: the radiance cache on (G1c's default settings, 8 settle frames),
        // then, with IQ1_JITTER, its own jittered average.
        if env("IQ1_CACHE").as_deref() == Some("1") {
            r.set_indirect(IndirectSettings {
                enabled: true,
                ..Default::default()
            })
            .unwrap();
            r.reset_indirect();
            for _ in 0..9 {
                render(&mut r, &camera);
            }
            write_f32(
                &out.join(format!("{path}-cache{tag}-pose{pose}.f32")),
                &capture(&mut r),
            );
            if jitter > 0 {
                jittered(&mut r, &mut render, &format!("cache-jitter{jitter}"));
            }
            r.set_indirect(IndirectSettings::off()).unwrap();
        }
        r.reload_program(&standard, crate::STANDARD_PROGRAM)
            .unwrap();
        log[format!("pose{pose}")] = serde_json::json!({"size": [frame.width, frame.height]});
    }
    std::fs::write(
        out.join(format!("{path}-iq1.json")),
        serde_json::to_string_pretty(&log).unwrap(),
    )
    .unwrap();
}

#[test]
fn iq1_variants_compose() {
    for name in [
        "standard",
        "shadow",
        "frame",
        "footprint",
        "albedo",
        "light05",
        "nomap",
        "lodm1",
        "lodm05",
        "specaa",
        "flat",
    ] {
        let (program, _) = variant(name);
        let composed =
            crate::program::compose(SOURCE, &program, crate::program::Stage::Raster, false);
        crate::program::analyse(name, &program, &composed).unwrap();
        for source in primary_ray_gpu::shading_sources(SOURCE, &program).unwrap() {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
