//! Phase 2 offscreen gate. Program/material authoring uses the public API; private
//! access is limited to engine visibility readback, seeded test kernels and pipeline
//! identities. No primary-ray production source is changed by this test.
#![allow(clippy::duplicate_mod)]
use super::*;
use std::{fs, path::Path};

#[allow(dead_code)]
#[path = "../examples/m4/cameras.rs"]
mod cameras;
#[allow(dead_code)]
#[path = "../support/environment_controls.rs"]
mod environment_controls;
#[allow(dead_code)]
#[path = "../support/lighting_fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../support/lighting_settings.rs"]
mod lighting_settings;
#[path = "../support/limits.rs"]
mod policy;
#[path = "../support/validation.rs"]
mod validation;

const TOON: &str = include_str!("../shaders/programs/toon.wgsl");
const PRELUDE: &str = include_str!("../shaders/scene.wgsl");
const PARAMETERS: [(&str, Value); 7] = [
    ("shadow_tint", Value::Vec3([0.12, 0.08, 0.24])),
    ("bands", Value::F32(4.)),
    ("rim_color", Value::Vec3([0.12, 0.2, 0.3])),
    ("rim_power", Value::F32(4.)),
    ("thresholds", Value::Vec2([0.12, 0.9])),
    ("fill", Value::F32(0.25)),
    ("flat_shaded", Value::U32(0)),
];

#[test]
fn toon_composes_on_every_stage_and_reflects_parameters() {
    let hardware = crate::hardware_shadows::shader_source(PRELUDE, SampleCount::One).unwrap();
    let fresh = fresh_shadow_gpu::shader_source(PRELUDE).unwrap();
    let traced = primary_ray_gpu::shading_heads(PRELUDE).unwrap();
    for (head, stage) in [
        (PRELUDE, Stage::Raster),
        (hardware.as_str(), Stage::Raster),
        (fresh.as_str(), Stage::Raster),
        (traced[0].as_str(), Stage::Traced),
        (traced[1].as_str(), Stage::Traced),
    ] {
        let composed = program::compose(head, TOON, stage, true);
        let layout = program::analyse("toon", TOON, &composed)
            .unwrap()
            .parameters
            .unwrap();
        ParameterLayout::check(Some(&layout), PARAMETERS).unwrap();
        assert_eq!(layout.fields.len(), 7);
        let module = naga::front::wgsl::parse_str(&composed.source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let entry = if stage == Stage::Raster {
            "fs_lit"
        } else {
            "primary_shade_tiles"
        };
        naga::back::spv::write_vec(
            &module,
            &info,
            &Default::default(),
            Some(&naga::back::spv::PipelineOptions {
                shader_stage: if stage == Stage::Raster {
                    naga::ShaderStage::Fragment
                } else {
                    naga::ShaderStage::Compute
                },
                entry_point: entry.into(),
            }),
        )
        .unwrap();
    }
}

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

/// Upload retries are admission retries, with a bounded completion wait between them.
fn admitted<T>(
    r: &mut Renderer,
    mut upload: impl FnMut(&mut Renderer) -> Result<Option<T>, Error>,
) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = upload(r).unwrap() {
            return value;
        }
        assert!(start.elapsed() < Duration::from_secs(120));
        r.test_drain("toon upload admission");
    }
}

struct Scene {
    meshes: Vec<Mesh>,
    instances: Vec<Instances>,
    material_ids: Vec<usize>,
    standard: Vec<Material>,
    toon: Vec<Material>,
    cameras: [Camera; 2],
}
impl Scene {
    fn upload(
        r: &mut Renderer,
        asset: &asset::Asset,
        program: &Program,
        cameras: [Camera; 2],
    ) -> Self {
        let textures: Vec<_> = asset
            .textures
            .iter()
            .map(|t| admitted(r, |r| r.try_upload_texture(t)))
            .collect();
        let materials = |r: &mut Renderer, toon: bool| {
            asset
                .materials
                .iter()
                .map(|m| {
                    let binding = |id: u32, sampler| TextureBinding {
                        texture: &textures[id as usize],
                        sampler,
                    };
                    admitted(r, |r| {
                        r.try_create_material(&MaterialDescriptor {
                            base_color: m.base_color,
                            metallic: m.metallic,
                            roughness: m.roughness,
                            double_sided: m.double_sided,
                            base_color_texture: m
                                .base_color_texture
                                .map(|t| binding(t.texture, t.sampler)),
                            metallic_roughness_texture: m
                                .metallic_roughness_texture
                                .map(|t| binding(t.texture, t.sampler)),
                            normal_texture: m.normal_texture.map(|t| binding(t.texture, t.sampler)),
                            normal_scale: m.normal_texture.map_or(1., |t| t.scale),
                            program: toon.then_some(program),
                            parameters: if toon { &PARAMETERS } else { &[] },
                            ..Default::default()
                        })
                    })
                })
                .collect()
        };
        let standard = materials(r, false);
        let toon = materials(r, true);
        let (mut meshes, mut instances, mut material_ids) = (vec![], vec![], vec![]);
        for (id, mesh) in asset.meshes.iter().enumerate() {
            let models: Vec<_> = asset
                .placements
                .iter()
                .filter(|p| p.mesh as usize == id)
                .map(|p| Mat4::from_cols_array_2d(&p.transform))
                .collect();
            if models.is_empty() {
                continue;
            }
            for p in &mesh.primitives {
                meshes.push(admitted(r, |r| {
                    r.try_upload_mesh_with_tangents(
                        &MeshData {
                            vertices: &p.vertices,
                            indices: &p.indices,
                        },
                        p.tangents.as_ref(),
                    )
                }));
                instances.push(admitted(r, |r| r.try_upload_instances(&models)));
                material_ids.push(p.material as usize);
            }
        }
        r.test_drain("toon scene uploaded");
        Self {
            meshes,
            instances,
            material_ids,
            standard,
            toon,
            cameras,
        }
    }
    fn frame(&mut self, r: &mut Renderer, color: &wgpu::Texture, pose: usize, toon: bool) {
        let materials = if toon { &self.toon } else { &self.standard };
        let mut draws: Vec<_> = self
            .meshes
            .iter()
            .zip(&mut self.instances)
            .zip(&self.material_ids)
            .map(|((mesh, instances), &m)| Draw::new(mesh, instances).with_material(&materials[m]))
            .collect();
        let start = Instant::now();
        while r
            .try_render_view(color, &self.cameras[pose], &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            assert!(start.elapsed() < Duration::from_secs(20));
            r.collect().unwrap();
        }
        r.test_drain("toon frame complete");
    }
}

fn read_texture(r: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
    let tight = texture.width() * texture.format().block_copy_size(None).unwrap();
    let row = tight.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("toon gate readback"),
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
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
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |v| {
        callback.set(v).unwrap();
    });
    r.queue.submit([encoder.finish()]);
    r.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    done.get().unwrap().as_ref().unwrap();
    buffer
        .get_mapped_range(..)
        .unwrap()
        .chunks_exact(row as usize)
        .flat_map(|r| r[..tight as usize].iter().copied())
        .collect()
}

fn hdr(r: &mut Renderer) -> Vec<u8> {
    let mut c = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("toon HDR");
    c.try_read().unwrap().unwrap()
}

/// Complete count image, with unused traced scratch pixels canonicalised just as in
/// the registered primary/Interactive acceptance harnesses. Covered bytes are untouched.
fn counts(r: &Renderer) -> Vec<u8> {
    let mut counts = read_texture(
        r,
        &r.fresh
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .visibility
            .texture,
    );
    if r.primary_mode == PrimaryVisibility::Traced {
        let primary = r.primary.as_ref().unwrap();
        let t = primary.targets.as_ref().unwrap();
        let tiles = read_texture(r, &t.tiles.texture);
        let s = primary.scene.map(|v| v as usize);
        let [tw, th] = primary_ray_gpu::TRACE_TILE.map(|v| v as usize);
        let width = t.hit.texture.width() as usize;
        let columns = t.tiles.texture.width() as usize;
        for (i, count) in counts.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i % width, i / width);
            let tile = x / tw + y / th * columns;
            if !(s[0]..s[2]).contains(&x)
                || !(s[1]..s[3]).contains(&y)
                || tiles[tile * 4..tile * 4 + 4] == [0; 4]
            {
                count.copy_from_slice(&256u32.to_le_bytes());
            }
        }
    }
    counts
}

fn save_png(r: &Renderer, color: &wgpu::Texture, path: &Path) {
    let mut encoder = png::Encoder::new(
        fs::File::create(path).unwrap(),
        color.width(),
        color.height(),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&read_texture(r, color))
        .unwrap();
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn save_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn load_scene(r: &mut Renderer, m: &serde_json::Value, toon: &Program, quarry: bool) -> Scene {
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    if quarry {
        let path = |key: &str| Path::new(m[key].as_str().unwrap());
        let lighting =
            lighting_settings::parse(&fs::read_to_string(path("lighting")).unwrap()).unwrap();
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
        let env = environment_controls::load_environment(
            path("environment"),
            asset::environment::Limits {
                max_dimension: 4096,
                max_texels: 64 * 1024 * 1024,
                max_payload_bytes: 128 * 1024 * 1024,
            },
        )
        .unwrap();
        let env = admitted(r, |r| r.try_upload_environment(&env));
        r.set_environment(Some(&env), EnvironmentParameters::default())
            .unwrap();
        let (policy, _) = policy::Policy::args(vec![
            "--limits".into(),
            path("limits").to_string_lossy().into_owned(),
        ])
        .unwrap();
        let asset = asset::decode(&fs::read(path("asset")).unwrap(), policy.prepared()).unwrap();
        let cameras = cameras::Cameras::load(path("cameras")).unwrap();
        Scene::upload(
            r,
            &asset,
            toon,
            [0, 1].map(|i| cameras.frames[i].camera().unwrap()),
        )
    } else {
        let f = fixture::Fixture::new();
        let mut lighting = f.lighting.clone();
        // Fresh soft visibility currently accepts one directional light only.
        lighting.lights.truncate(1);
        r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
        let mut asset = asset::Asset::default();
        for (i, o) in f.objects.iter().enumerate() {
            asset.materials.push(asset::Material {
                base_color: o.base,
                metallic: o.metal,
                roughness: o.rough,
                double_sided: true,
                ..Default::default()
            });
            asset.meshes.push(asset::Mesh {
                primitives: vec![asset::Primitive {
                    vertices: o.vertices.clone(),
                    indices: o.indices.clone(),
                    tangents: None,
                    material: i as u32,
                }],
            });
            asset.placements.push(asset::Placement {
                mesh: i as u32,
                transform: o.transform.to_cols_array_2d(),
            });
        }
        Scene::upload(r, &asset, toon, [f.camera(0), f.camera(1)])
    }
}

/// Seed variants affect the engine's estimator only, identically for both programs.
fn seed_pipelines(r: &Renderer) -> Vec<Owned<wgpu::ComputePipeline>> {
    let soft = primary_ray_gpu::shader_sources(PRELUDE).unwrap()[1].clone();
    let find = "    let seed=fresh_hash(pixel.x+pixel.y*width);";
    assert_eq!(soft.matches(find).count(), 1);
    let source = format!("{}\noverride FRESH_SEED:u32=0u;\n", soft.replace(find,
        "    var seed=fresh_hash(pixel.x+pixel.y*width);\n    if FRESH_SEED!=0u {seed=fresh_hash(seed^(FRESH_SEED*0x9e3779b9u));}"));
    let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("toon gate seeded visibility"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    (0..4)
        .map(|seed| {
            r.primary.as_ref().unwrap().interactive_pipeline(
                &r.device,
                &r.ledger,
                &module,
                &[("FRESH_SEED", f64::from(seed)), ("FRESH_DIRECTIONS", 8.)],
            )
        })
        .collect()
}

fn reload_gate(
    r: &mut Renderer,
    scene: &mut Scene,
    color: &wgpu::Texture,
    toon: &Program,
    out: &Path,
) {
    let mut rows = vec![];
    for mode in [PrimaryVisibility::Raster, PrimaryVisibility::Traced] {
        r.set_primary_visibility(mode).unwrap();
        scene.frame(r, color, 0, false);
        let standard = hdr(r);
        let standard_pairs = r.programs[0]
            .pipelines
            .fresh
            .as_ref()
            .map(|p| p.each_ref().map(|p| (**p).clone()));
        let standard_traced = r.programs[0]
            .pipelines
            .traced
            .as_ref()
            .map(|p| p.each_ref().map(|p| (**p).clone()));
        scene.frame(r, color, 0, true);
        let accepted = hdr(r);
        let visibility = counts(r);
        let created = r.stats().pipelines.created;
        for material in &scene.toon {
            r.try_update_parameters(material, &[("bands", Value::F32(2.))])
                .unwrap();
        }
        scene.frame(r, color, 0, true);
        assert!(
            hdr(r) != accepted,
            "band update must change visible shading"
        );
        assert!(counts(r) == visibility);
        assert_eq!(
            r.stats().pipelines.created,
            created,
            "parameters compile nothing"
        );
        for material in &scene.toon {
            r.try_update_parameters(material, &PARAMETERS).unwrap();
        }
        scene.frame(r, color, 0, true);
        assert!(hdr(r) == accepted, "parameter restore");

        // File save -> reload call -> GPU-completed image. No polling or presentation
        // latency is included. A process-unique literal prevents a disk-cache hit.
        let salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let gain = 0.55 + f64::from(salt) / 1e9 * 0.1;
        let edited = TOON.replace("return exposed+", &format!("return exposed*{gain:.9}+"));
        let path = out.join("toon-edit.wgsl");
        let start = Instant::now();
        fs::write(&path, edited).unwrap();
        let report = r
            .reload_program(toon, &fs::read_to_string(&path).unwrap())
            .unwrap();
        let compiled = r.stats().pipelines.created - created;
        assert_eq!(compiled, 2, "only the toon frame's active pipeline pair");
        scene.frame(r, color, 0, true);
        let edit_to_image_ms = start.elapsed().as_secs_f64() * 1e3;
        let changed = hdr(r);
        assert!(changed != accepted);
        assert!(counts(r) == visibility);
        save_png(r, color, &out.join(format!("{mode:?}-edited.png")));
        assert_eq!(
            standard_pairs,
            r.programs[0]
                .pipelines
                .fresh
                .as_ref()
                .map(|p| p.each_ref().map(|p| (**p).clone()))
        );
        assert_eq!(
            standard_traced,
            r.programs[0]
                .pipelines
                .traced
                .as_ref()
                .map(|p| p.each_ref().map(|p| (**p).clone()))
        );
        scene.frame(r, color, 0, false);
        let before_standard = r.stats().pipelines.created;
        assert!(hdr(r) == standard, "toon edit changed standard image");
        scene.frame(r, color, 0, false);
        assert_eq!(before_standard, r.stats().pipelines.created);
        scene.frame(r, color, 0, true);
        let invalid = TOON.replace(
            "let sampled=sample_base_color(input);",
            "let sampled=sample_base_color(input;",
        );
        let line = TOON
            .lines()
            .position(|l| l.contains("let sampled="))
            .unwrap()
            + 1;
        let before_invalid = r.stats().pipelines.created;
        let diagnostic = match r.reload_program(toon, &invalid) {
            Err(ReloadError::Rejected { diagnostic, .. }) => diagnostic,
            other => panic!("expected toon rejection: {other:?}"),
        };
        assert!(
            diagnostic.contains(&format!("program `toon` line {line}:")),
            "{diagnostic}"
        );
        scene.frame(r, color, 0, true);
        assert!(
            hdr(r) == changed,
            "invalid edit lost last working toon image"
        );
        scene.frame(r, color, 0, false);
        assert!(hdr(r) == standard, "invalid toon edit changed standard");
        assert_eq!(before_invalid, r.stats().pipelines.created);
        scene.frame(r, color, 0, true);
        r.reload_program(toon, TOON).unwrap();
        scene.frame(r, color, 0, true);
        assert!(hdr(r) == accepted, "toon source restore");
        rows.push(serde_json::json!({"path":format!("{mode:?}"), "pipelines_compiled":compiled,
            "compile_ms":report.compile.as_secs_f64()*1e3, "save_to_completed_image_ms":edit_to_image_ms,
            "parameter_compiles":0,"standard_pipelines_preserved":true,"standard_image_preserved":true,
            "rejection":diagnostic,"last_working_preserved":true}));
    }
    save_json(&out.join("reload.json"), &serde_json::json!(rows));
}

fn gate(quarry: bool) {
    let _validation = validation::begin(true);
    let m: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let out = Path::new(m["output"].as_str().unwrap());
    fs::create_dir_all(out).unwrap();
    let mut r = renderer();
    let toon = r
        .try_create_program(&ProgramDescriptor {
            label: "toon",
            source: TOON,
        })
        .unwrap();
    let mut scene = load_scene(&mut r, &m, &toon, quarry);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut seeded = seed_pipelines(&r);
    let (width, height) = if quarry { (1920, 1080) } else { (960, 640) };
    let color = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("toon offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let repeats = m["static_repeats"].as_u64().unwrap_or(4);
    let mut rows = vec![];
    for (index, pose) in [0, 1, 0].into_iter().enumerate() {
        for mode in 0..6 {
            let name = [
                "raster",
                "exact",
                "interactive-0",
                "interactive-1",
                "interactive-2",
                "interactive-3",
            ][mode];
            r.set_primary_visibility(if mode == 0 {
                PrimaryVisibility::Raster
            } else {
                PrimaryVisibility::Traced
            })
            .unwrap();
            r.set_soft_shadows(if mode < 2 {
                SoftShadows::Exact
            } else {
                SoftShadows::Interactive
            });
            if mode >= 2 {
                std::mem::swap(
                    &mut r.pipeline.primary.as_mut().unwrap().interactive,
                    &mut seeded[mode - 2],
                );
            }
            let mut pair = vec![];
            for toon in [false, true] {
                let style = if toon { "toon" } else { "standard" };
                let prefix = format!("pose-{index}-{name}-{style}");
                scene.frame(&mut r, &color, pose, toon);
                let image = hdr(&mut r);
                let visibility = counts(&r);
                if toon {
                    for repeat in 0..repeats {
                        scene.frame(&mut r, &color, pose, true);
                        assert!(
                            hdr(&mut r) == image,
                            "static image {prefix} repeat {repeat}"
                        );
                        assert!(
                            counts(&r) == visibility,
                            "static visibility {prefix} repeat {repeat}"
                        );
                    }
                }
                for (kind, bytes) in [("rgba16f", &image), ("u32", &visibility)] {
                    let path = out.join(format!("{prefix}.{kind}"));
                    fs::write(&path, bytes).unwrap();
                    if index == 2 {
                        assert!(
                            *bytes
                                == fs::read(out.join(format!("pose-0-{name}-{style}.{kind}")))
                                    .unwrap(),
                            "return {prefix}.{kind}"
                        );
                    }
                }
                save_png(&r, &color, &out.join(format!("{prefix}.png")));
                rows.push(
                    serde_json::json!({"pose":pose,"index":index,"mode":name,"style":style,
                    "hdr_sha256":hash(&image),"visibility_sha256":hash(&visibility)}),
                );
                pair.push((image, visibility));
            }
            assert!(
                pair[0].1 == pair[1].1,
                "program changed visibility {name} pose {pose}"
            );
            assert!(pair[0].0 != pair[1].0, "toon did not change shading");
            if mode >= 2 {
                std::mem::swap(
                    &mut r.pipeline.primary.as_mut().unwrap().interactive,
                    &mut seeded[mode - 2],
                );
            }
        }
    }
    assert!(
        fs::read(out.join("pose-0-exact-toon.rgba16f")).unwrap()
            != fs::read(out.join("pose-1-exact-toon.rgba16f")).unwrap(),
        "camera motion did not change toon image"
    );
    save_json(
        &out.join("captures.json"),
        &serde_json::json!({"width":width,"height":height,"static_repeats":repeats,"captures":rows}),
    );
    reload_gate(&mut r, &mut scene, &color, &toon, out);
}

#[test]
#[ignore = "offscreen; GPU slot, >=6 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_toon_quarry() {
    gate(true);
}

#[test]
#[ignore = "offscreen; GPU slot, >=6 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST"]
fn gpu_toon_material_fixture() {
    gate(false);
}
