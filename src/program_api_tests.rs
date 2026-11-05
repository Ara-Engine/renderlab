//! Public boundary checks for caller files, defaults and fixed arrays on both paths.
#![allow(clippy::duplicate_mod)]
use super::*;
use crate::program_watch::{Kind, ShaderWatch};
use std::{fs, path::PathBuf};
#[path = "../support/validation.rs"]
mod validation;
const PRELUDE: &str = include_str!("../shaders/scene.wgsl");
const SOURCE: &str = r#"struct Parameters { tint:vec3<f32>, gain:f32, weights:array<vec3<f32>,2> };
const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.25,0.5,0.75),0.5,array<vec3<f32>,2>(vec3<f32>(0.0),vec3<f32>(0.125)));
struct Surface { color:vec3<f32> };
fn surface(input:SurfaceInput)->Surface { return Surface(params.tint*params.gain+params.weights[1]); }
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return s.color; }
"#;
fn renderer() -> Renderer {
    assert_eq!(
        std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref(),
        Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::debugging(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("toon_adapter={:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::TRACED_FEATURES
            | wgpu::Features::EXPERIMENTAL_RAY_QUERY
            | wgpu::Features::PIPELINE_CACHE,
        required_limits: crate::traced_limits(wgpu::Limits {
            max_blas_primitive_count: 2_000_000,
            max_blas_geometry_count: 1,
            max_tlas_instance_count: 4096,
            max_acceleration_structures_per_shader_stage: 1,
            ..Default::default()
        }),
        // This ignored test requires the explicit acknowledgment checked above.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .unwrap();
    Renderer::new_with_hardware_shadows(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Limits {
            max_mesh_bytes: 134217728,
            max_texture_bytes: 134217728,
            ..Default::default()
        },
        RendererOptions {
            samples: SampleCount::One,
            anisotropy_clamp: 16,
        },
        HardwareShadowOptions {
            max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
        },
        PRELUDE,
    )
    .unwrap()
}

fn admitted<T>(
    r: &mut Renderer,
    mut upload: impl FnMut(&mut Renderer) -> Result<Option<T>, Error>,
) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = upload(r).unwrap() {
            return value;
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        r.test_drain("program API upload");
    }
}

#[test]
#[ignore = "offscreen; shared GPU lease, >=6 GiB RAM, experimental acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_registered_program_files_defaults_arrays() {
    let _validation = validation::begin(true);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    let mut r = renderer();
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "caller-owned",
            source: SOURCE,
        })
        .unwrap();
    let other = r
        .try_create_program(&ProgramDescriptor {
            label: "other",
            source: SOURCE,
        })
        .unwrap();
    let mesh = admitted(&mut r, |r| {
        r.try_upload_mesh(&MeshData {
            vertices: &[[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]].map(|[x, y]| Vertex {
                position: [x, y, 0.],
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            }),
            indices: &[0, 1, 2, 0, 2, 3],
        })
    });
    let mut instances = admitted(&mut r, |r| r.try_upload_instances(&[Mat4::IDENTITY]));
    let make = |r: &mut Renderer, parameters: &[(&str, Value)]| {
        admitted(r, |r| {
            r.try_create_material(&MaterialDescriptor {
                program: Some(&program),
                parameters,
                double_sided: true,
                ..Default::default()
            })
        })
    };
    let defaults = make(&mut r, &[]);
    let explicit = make(&mut r, &[("gain", Value::F32(1.))]);
    assert!(
        r.try_create_material(&MaterialDescriptor {
            program: Some(&program),
            parameters: &[("weights", Value::Array(vec![Value::Vec3([0.; 3])]))],
            ..Default::default()
        })
        .is_err()
    );
    let color = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("program API offscreen"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let camera =
        Camera::look_at(Vec3::new(0., 0., 3.), Vec3::ZERO, Vec3::Y, 0.8, 0.1, 10.).unwrap();
    let mut frame = |r: &mut Renderer, material: &Material| {
        let start = Instant::now();
        while r
            .try_render_view(
                &color,
                &camera,
                &mut [Draw::new(&mesh, &mut instances).with_material(material)],
                ViewMode::Lit,
            )
            .unwrap()
            .is_none()
        {
            assert!(start.elapsed() < Duration::from_secs(10));
            r.test_drain("program API frame admission");
        }
        r.test_drain("program API completed image");
        let mut c = r.try_capture_hdr().unwrap().unwrap();
        r.test_drain("program API HDR");
        c.try_read().unwrap().unwrap()
    };
    let centre = |image: &[u8]| {
        let p = (32 * 64 + 32) * 8;
        [0, 1, 2].map(|c| {
            decode_f16(u16::from_le_bytes(
                image[p + c * 2..p + c * 2 + 2].try_into().unwrap(),
            ))
        })
    };
    let path = out.join("user-registered.wgsl");
    let prelude = out.join("user-prelude.wgsl");
    let other_path = out.join("other-registered.wgsl");
    fs::write(&path, SOURCE).unwrap();
    fs::write(&prelude, PRELUDE).unwrap();
    fs::write(&other_path, SOURCE).unwrap();
    let mut watch = ShaderWatch::new(
        prelude.clone(),
        PRELUDE.into(),
        out.join("absent-standard.wgsl"),
    );
    watch.watch_program(&program, &path, SOURCE);
    watch.watch_program(&other, &other_path, SOURCE);
    // Re-registering a handle replaces its watch, rather than reloading twice.
    watch.watch_program(&program, &path, SOURCE);
    assert!(watch.poll(&mut r).is_empty());
    let mut rows = vec![];
    for mode in [PrimaryVisibility::Raster, PrimaryVisibility::Traced] {
        r.set_primary_visibility(mode).unwrap();
        let accepted = frame(&mut r, &defaults);
        assert_eq!(centre(&accepted), [0.25, 0.375, 0.5]);
        let overridden = frame(&mut r, &explicit);
        assert_eq!(centre(&overridden), [0.375, 0.625, 0.875]);
        let created = r.stats().pipelines.created;
        let writes = r.stats().uploads.write_calls;
        assert!(
            r.try_update_parameters(
                &defaults,
                &[("gain", Value::F32(0.)), ("weights", Value::Array(vec![]))]
            )
            .is_err()
        );
        assert_eq!(
            r.stats().uploads.write_calls,
            writes,
            "invalid update is atomic"
        );
        assert_eq!(frame(&mut r, &defaults), accepted);
        r.try_update_parameters(
            &defaults,
            &[(
                "weights",
                Value::Array(vec![Value::Vec3([0.; 3]), Value::Vec3([0.25; 3])]),
            )],
        )
        .unwrap();
        assert_eq!(centre(&frame(&mut r, &defaults)), [0.375, 0.5, 0.625]);
        assert_eq!(r.stats().pipelines.created, created);
        r.try_update_parameters(
            &defaults,
            &[(
                "weights",
                Value::Array(vec![Value::Vec3([0.; 3]), Value::Vec3([0.125; 3])]),
            )],
        )
        .unwrap();
        assert_eq!(frame(&mut r, &defaults), accepted);
        // Unique live literal forces an uncached edit; include the viewer's 50 ms poll.
        let salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let gain = 0.55 + f64::from(salt) * 1e-10;
        let edited = SOURCE.replace("return s.color;", &format!("return s.color*{gain:.9};"));
        let saved = Instant::now();
        fs::write(&path, &edited).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        let mut reloads = watch.poll(&mut r);
        assert_eq!(reloads.len(), 1);
        let reload = reloads.remove(0);
        assert_eq!(reload.kind, Kind::Program(program.clone()));
        let report = reload.result.unwrap();
        let compile = report.compile;
        assert!(report.zeroed_parameters.is_empty());
        let image = frame(&mut r, &defaults);
        let elapsed = saved.elapsed();
        assert_ne!(image, accepted, "live shading edit changes the image");
        assert_ne!(
            frame(&mut r, &explicit),
            overridden,
            "the edited shade function also applies to explicit materials"
        );
        assert_eq!(r.stats().pipelines.created - created, 2);
        let invalid = edited.replace("s.color", "s.missing");
        fs::write(&path, &invalid).unwrap();
        let rejection = watch.poll(&mut r).remove(0).result.unwrap_err().to_string();
        assert!(
            rejection.contains("program `caller-owned` line 5:"),
            "{rejection}"
        );
        assert!(watch.rejected());
        assert!(
            watch.poll(&mut r).is_empty(),
            "unchanged rejection is not recompiled"
        );
        assert_eq!(frame(&mut r, &defaults), image);
        fs::write(&path, SOURCE).unwrap();
        watch.poll(&mut r).remove(0).result.unwrap();
        assert!(!watch.rejected());
        assert_eq!(frame(&mut r, &defaults), accepted);
        assert_eq!(frame(&mut r, &explicit), overridden);
        rows.push(serde_json::json!({"path":format!("{mode:?}"),"scene":"64x64 indexed quad","compile_ms":compile.as_secs_f64()*1e3,
            "save_to_detect_ms":(reload.detected-saved).as_secs_f64()*1e3,"save_to_image_ms":elapsed.as_secs_f64()*1e3,
            "pipelines":2,"rejection":rejection,"revert_byte_identical":true}));
    }
    // A program depending on a new prelude helper is retried after that prelude lands.
    fs::write(
        &path,
        SOURCE.replace("return s.color;", "return s.color*api_gain();"),
    )
    .unwrap();
    assert!(watch.poll(&mut r).remove(0).result.is_err());
    fs::write(
        &prelude,
        format!("{PRELUDE}\nfn api_gain()->f32 {{ return 0.5; }}"),
    )
    .unwrap();
    let reloads = watch.poll(&mut r);
    assert_eq!(reloads.len(), 2);
    assert_eq!(reloads[0].kind, Kind::Prelude);
    assert!(reloads[1].retry);
    assert!(reloads.iter().all(|r| r.result.is_ok()));
    assert!(!watch.rejected());
    assert_eq!(centre(&frame(&mut r, &defaults)), [0.125, 0.1875, 0.25]);
    // A prelude constant used by DEFAULT_PARAMETERS must repack live materials too.
    fs::write(&path, SOURCE.replace(",0.5,array", ",0.5*TINT.x,array")).unwrap();
    watch.poll(&mut r).remove(0).result.unwrap();
    fs::write(
        &prelude,
        PRELUDE.replace("vec3<f32>(1.0, 1.0, 1.0);", "vec3<f32>(0.5, 0.5, 0.5);"),
    )
    .unwrap();
    watch.poll(&mut r).remove(0).result.unwrap();
    assert_eq!(centre(&frame(&mut r, &defaults)), [0.1875, 0.25, 0.3125]);
    assert_eq!(centre(&frame(&mut r, &explicit)), [0.375, 0.625, 0.875]);
    // Retyping drops incompatible overrides and uses the program's new defaults.
    let retyped = SOURCE
        .replace("gain:f32", "gain:u32")
        .replace(",0.5,array", ",2u,array")
        .replace("params.tint*params.gain", "params.tint*f32(params.gain)");
    fs::write(&path, &retyped).unwrap();
    assert!(
        watch
            .poll(&mut r)
            .remove(0)
            .result
            .unwrap()
            .zeroed_parameters
            .is_empty()
    );
    for material in [&defaults, &explicit] {
        assert_eq!(centre(&frame(&mut r, material)), [0.625, 1.125, 1.625]);
    }
    fs::write(&path, SOURCE).unwrap();
    watch.poll(&mut r).remove(0).result.unwrap();
    assert_eq!(
        centre(&frame(&mut r, &explicit)),
        [0.25, 0.375, 0.5],
        "old f32 override must not resurrect"
    );
    fs::write(
        out.join("registered-reload.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    eprintln!(
        "registered_reload={}",
        serde_json::to_string(&rows).unwrap()
    );
    assert!(r.fault.get().is_none());
}
