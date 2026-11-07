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
    let descriptor = wgpu::DeviceDescriptor {
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
    };
    let (device, queue) = if std::env::var_os("RENDERLAB_VULKAN_INTEROP").is_some() {
        crate::shader_compiler::request_device(&adapter, &descriptor)
            .unwrap()
            .unwrap()
    } else {
        pollster::block_on(adapter.request_device(&descriptor)).unwrap()
    };
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

/// Short driver screen: fresh unique sources per policy, no asset/window/readback.
#[test]
#[ignore = "shared GPU lease, >=4 GiB RAM, experimental acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_vulkan_compile_screen() {
    use crate::shader_compiler::{self, CompilePolicy};
    let _validation = validation::begin(true);
    assert!(std::env::var_os("RENDERLAB_VULKAN_INTEROP").is_some());
    let mut r = renderer();
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    eprintln!(
        "vulkan_capabilities={:?}",
        shader_compiler::capabilities(&r.device)
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let mut rows = vec![];
    let mut serial = 0u32;
    for variant in [Variant::Lit, Variant::Traced] {
        let vertex_identity = Sha256::digest(PRELUDE.as_bytes()).into();
        let abi = r.program_abi();
        let scopes = OperationScopes::new(&r.device);
        shader_compiler::with_program_policy(CompilePolicy::WarmLibraries, vertex_identity, || {
            r.program_slot(r.standard.id())
                .modules
                .pipeline(&abi, variant, 1)
        });
        assert!(scopes.finish().is_none());
        eprintln!(
            "vulkan_warm variant={variant:?} timings={:?}",
            shader_compiler::take_timings(&r.device)
        );
        for repeat in 0..2 {
            for policy in [
                CompilePolicy::Optimized,
                CompilePolicy::Preview,
                CompilePolicy::PreviewNoInline,
                CompilePolicy::PreviewHelpers,
                CompilePolicy::LibraryFast,
                CompilePolicy::LibraryLto,
            ] {
                if variant == Variant::Traced
                    && matches!(
                        policy,
                        CompilePolicy::LibraryFast | CompilePolicy::LibraryLto
                    )
                {
                    continue;
                }
                serial += 1;
                let tint = f32::from_bits(
                    0x3f000000 | ((salt.wrapping_add(serial * 100003)) & 0x003fffff),
                );
                let source = STANDARD_PROGRAM.replace(
                    "return exposed;",
                    &format!("return exposed*vec3<f32>({tint:.9},0.9,1.0);"),
                );
                let mut abi = r.program_abi();
                abi.cache = None;
                let compiled =
                    compile_program(&abi, &r.preludes(), "driver-screen", &source).unwrap();
                shader_compiler::take_timings(&r.device);
                let scopes = OperationScopes::new(&r.device);
                let start = Instant::now();
                let pipeline =
                    shader_compiler::with_program_policy(policy, vertex_identity, || {
                        compiled.modules.pipeline(&abi, variant, 1)
                    });
                let elapsed = start.elapsed();
                assert!(scopes.finish().is_none());
                drop(pipeline);
                let timings = shader_compiler::take_timings(&r.device);
                assert!(!timings.is_empty());
                let driver = timings.last().unwrap();
                let row = serde_json::json!({"variant":format!("{variant:?}"),"policy":format!("{policy:?}"),"repeat":repeat,
                    "tint":tint,"source_sha256":format!("{:x}",Sha256::digest(source.as_bytes())),
                    "api_ms":elapsed.as_secs_f64()*1e3,"driver_ms":driver.duration.as_secs_f64()*1e3,"cache_hit":driver.application_cache_hit,
                    "shader_bytes":driver.shader_bytes,
                    "parts":timings.iter().map(|t|serde_json::json!({"kind":t.kind,"ms":t.duration.as_secs_f64()*1e3,"cache_hit":t.application_cache_hit,"shader_bytes":t.shader_bytes})).collect::<Vec<_>>()});
                eprintln!("vulkan_screen {row}");
                rows.push(row);
            }
        }
    }
    fs::write(
        out.join("compile-screen.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

/// Synthetic call-graph experiment; engine helper sources stay untouched.
#[test]
#[ignore = "shared GPU lease, >=4 GiB RAM, experimental acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_vulkan_helper_size() {
    use crate::shader_compiler::{self, CompilePolicy};
    let _validation = validation::begin(true);
    let mut r = renderer();
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let mut rows = vec![];
    for variant in [Variant::Lit, Variant::Traced] {
        for repeat in 0..2 {
            for calls in [0, 1, 2, 4] {
                let tint =
                    0.5 + f64::from(salt.wrapping_add(repeat * 100003 + calls * 99391)) * 1e-11;
                let mut body = format!("var c=input.color.xyz*{tint:.10};");
                for i in 0..calls {
                    body += &format!(
                        "c+=environment_reflection(normalize(input.normal+vec3<f32>({:.3},0.2,0.3)),clamp(input.uv.x*{:.3}+0.1,0.0,1.0));",
                        0.1 + f64::from(i) * 0.17,
                        0.5 + f64::from(i) * 0.07
                    );
                }
                let source = format!(
                    "struct Surface {{ color:vec3<f32> }}; fn surface(input:SurfaceInput)->Surface {{ return Surface(input.color.xyz); }} fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {{ {body} return expose(c); }}"
                );
                let mut abi = r.program_abi();
                abi.cache = None;
                let compiled =
                    compile_program(&abi, &r.preludes(), "helper-size", &source).unwrap();
                for policy in [CompilePolicy::Optimized, CompilePolicy::PreviewNoInline] {
                    shader_compiler::take_timings(&r.device);
                    let scopes = OperationScopes::new(&r.device);
                    let start = Instant::now();
                    let pipeline =
                        shader_compiler::with_program_policy(policy, abi.vertex_identity, || {
                            compiled.modules.pipeline(&abi, variant, 1)
                        });
                    let api_ms = start.elapsed().as_secs_f64() * 1e3;
                    assert!(scopes.finish().is_none());
                    drop(pipeline);
                    let timings = shader_compiler::take_timings(&r.device);
                    let driver = timings.last().unwrap();
                    rows.push(serde_json::json!({"variant":format!("{variant:?}"),"policy":format!("{policy:?}"),"calls":calls,"repeat":repeat,"api_ms":api_ms,"driver_ms":driver.duration.as_secs_f64()*1e3,"shader_bytes":driver.shader_bytes,"source_sha256":format!("{:x}",Sha256::digest(source.as_bytes()))}));
                }
            }
        }
    }
    fs::write(
        out.join("helper-size.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
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
        // Compile every active ABI on owned worker recipes, while the old image is
        // still submitted. The replacement must be byte-identical to sync creation.
        let sets = [
            Variant::Lit,
            Variant::Hardware,
            Variant::Fresh,
            Variant::Traced,
            Variant::Temporal,
        ]
        .into_iter()
        .filter(|v| r.program_slot(program.id()).modules.has(*v))
        .map(|v| (program.id(), v))
        .collect();
        r.ensure_pipelines(sets).unwrap();
        let job = r.start_program_reload(&program, SOURCE).unwrap();
        assert_eq!(frame(&mut r, &defaults), accepted);
        while !job.is_finished() {
            std::thread::yield_now();
        }
        assert!(job.current(&r));
        r.finish_program_reload(job).unwrap();
        assert_eq!(frame(&mut r, &defaults), accepted);
        assert_eq!(frame(&mut r, &explicit), overridden);

        // A completed stale snapshot cannot replace a more recent transaction.
        let stale = r
            .start_program_reload(
                &program,
                &SOURCE.replace("return s.color;", "return s.color*0.25;"),
            )
            .unwrap();
        r.reload_program(&program, SOURCE).unwrap();
        while !stale.is_finished() {
            std::thread::yield_now();
        }
        assert!(!stale.current(&r));
        stale.discard();
        assert_eq!(frame(&mut r, &defaults), accepted);

        // Rapid saves coalesce while a worker runs; syntax errors preserve the image,
        // and correction restores it. The first queued result must never install.
        let mut asynchronous = ShaderWatch::programs_only();
        asynchronous.watch_program(&program, &path, SOURCE);
        fs::write(
            &path,
            SOURCE.replace("return s.color;", "return s.color*0.25;"),
        )
        .unwrap();
        assert!(asynchronous.poll_async(&mut r).is_empty());
        fs::write(&path, "invalid WGSL").unwrap();
        let start = Instant::now();
        let rejected = loop {
            let mut events = asynchronous.poll_async(&mut r);
            if !events.is_empty() {
                break events.remove(0);
            }
            assert!(start.elapsed() < Duration::from_secs(20));
            assert_eq!(frame(&mut r, &defaults), accepted);
        };
        assert!(matches!(rejected.result, Err(ReloadError::Rejected { .. })));
        assert!(asynchronous.rejected());
        assert_eq!(frame(&mut r, &defaults), accepted);
        fs::write(&path, SOURCE).unwrap();
        let start = Instant::now();
        loop {
            let events = asynchronous.poll_async(&mut r);
            if let Some(event) = events.into_iter().next() {
                event.result.unwrap();
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(20));
            assert_eq!(frame(&mut r, &defaults), accepted);
        }
        assert!(!asynchronous.rejected());
        assert_eq!(frame(&mut r, &defaults), accepted);
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
    if std::env::var_os("RENDERLAB_VULKAN_INTEROP").is_some() {
        use crate::program_watch::ReloadStage;
        use crate::shader_compiler::{self, CompilePolicy};
        let mut comparisons = vec![];
        let standard = r.standard_program().clone();
        let material = admitted(&mut r, |r| {
            r.try_create_material(&MaterialDescriptor::default())
        });
        for mode in [PrimaryVisibility::Raster, PrimaryVisibility::Traced] {
            r.set_primary_visibility(mode).unwrap();
            r.reload_program(&standard, STANDARD_PROGRAM).unwrap();
            let baseline = frame(&mut r, &material);
            for policy in [
                CompilePolicy::Preview,
                CompilePolicy::PreviewNoInline,
                CompilePolicy::PreviewHelpers,
                CompilePolicy::LibraryFast,
            ] {
                let edited = STANDARD_PROGRAM.replace(
                    "return exposed;",
                    "return exposed*vec3<f32>(0.713257,0.9,1.0);",
                );
                let mut staged = ShaderWatch::programs_only();
                staged.watch_program(&standard, &path, STANDARD_PROGRAM);
                staged.set_preview_policy(Some(policy)).unwrap();
                fs::write(&path, &edited).unwrap();
                let started = Instant::now();
                let preview = loop {
                    let mut results = staged.poll_async(&mut r);
                    if !results.is_empty() {
                        break results.remove(0);
                    }
                    assert_eq!(frame(&mut r, &material), baseline);
                    assert!(started.elapsed() < Duration::from_secs(60));
                };
                assert_eq!(preview.stage, ReloadStage::Preview);
                preview.result.unwrap();
                assert!(
                    staged.set_preview_policy(None).is_err(),
                    "promotion remains owned until a preview frame"
                );
                let preview_image = frame(&mut r, &material);
                let mut rejected = false;
                if policy == CompilePolicy::Preview {
                    fs::write(&path, "invalid WGSL").unwrap();
                }
                loop {
                    let results = staged.poll_async(&mut r);
                    let mut promoted = false;
                    for result in results {
                        match result.result {
                            Ok(_) => {
                                assert_eq!(result.stage, ReloadStage::Optimized);
                                promoted = true;
                            }
                            Err(ReloadError::Rejected { .. }) => rejected = true,
                            Err(error) => panic!("{error}"),
                        }
                    }
                    if promoted {
                        break;
                    }
                    assert_eq!(frame(&mut r, &material), preview_image);
                    assert!(started.elapsed() < Duration::from_secs(60));
                }
                assert_eq!(rejected, policy == CompilePolicy::Preview);
                assert_eq!(
                    staged.rejected(),
                    rejected,
                    "optimizing the last accepted source must retain the current invalid-file diagnostic"
                );
                let optimized = frame(&mut r, &material);
                r.reload_program(&standard, &edited).unwrap();
                let native = frame(&mut r, &material);
                let different = |a: &[u8], b: &[u8]| {
                    a.chunks_exact(8)
                        .zip(b.chunks_exact(8))
                        .filter(|(a, b)| a != b)
                        .count()
                };
                let row = serde_json::json!({"mode":format!("{mode:?}"),"policy":format!("{policy:?}"),"preview_different_pixels":different(&preview_image,&native),"optimized_different_pixels":different(&optimized,&native),"pixels":64*64,
                    "preview_sha256":format!("{:x}",Sha256::digest(&preview_image)),"optimized_sha256":format!("{:x}",Sha256::digest(&optimized)),"native_sha256":format!("{:x}",Sha256::digest(&native)),"timings":format!("{:?}",shader_compiler::take_timings(&r.device))});
                assert_eq!(
                    optimized, native,
                    "optimized pipeline must equal native output: {row}"
                );
                comparisons.push(row);
                drop(staged);
                r.reload_program(&standard, STANDARD_PROGRAM).unwrap();
                assert_eq!(frame(&mut r, &material), baseline);
            }
        }
        fs::write(
            out.join("preview-images.json"),
            serde_json::to_vec_pretty(&comparisons).unwrap(),
        )
        .unwrap();
    }
}
