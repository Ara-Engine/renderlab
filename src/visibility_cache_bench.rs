//! S2/S3 benches for the visibility cache (unit N7a, docs/non-rt-n7.md, docs/bench-motion.md).
//!
//! One offscreen process per bench. Per case (track and disk radius):
//! 1. Film references at the registered crop: the S1 footprint generator of
//!    motion_bench_tests.rs (`FilmCrop`, copied here unchanged in arithmetic), with its
//!    bit-identical-input reuse. Earlier references were deleted after scoring; their
//!    per-frame reference statistics in per-frame.json check the regeneration.
//! 2. The cache on N0's harness over the track, after 8 warmup frames at the first pose as
//!    the production modes had: production Interactive frames supply hit records and
//!    receivers; the cache's lookup, fill and resolve run on them with movers composited;
//!    N0's software Interactive runs on the same receivers. Crops of both are written as f32
//!    visibility for scripts/n7a-score.py, with per-frame GPU times.
use super::*;

#[derive(serde::Deserialize)]
struct Bench {
    size: [u32; 2],
    crop: [u32; 4],
    fov_y_degrees: f32,
    sun_lux: [f32; 3],
    exposure_stops: f32,
    material: SurfaceMaterial,
    meshes: Vec<Geometry>,
    tracks: Vec<Track>,
}
#[derive(serde::Deserialize)]
struct SurfaceMaterial {
    base_color: [f32; 4],
    metallic: f32,
    double_sided: bool,
}
#[derive(serde::Deserialize)]
struct Geometry {
    name: String,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    thin: bool,
    roughness: f32,
}
#[derive(serde::Deserialize)]
struct Track {
    name: String,
    frames: Vec<Pose>,
}
#[derive(serde::Deserialize)]
struct Pose {
    frame: usize,
    eye: [f32; 3],
    target: [f32; 3],
    to_light: [f32; 3],
    plate_x: f32,
    blocker_x: f32,
}
type Scene = Vec<(String, Mesh, Instances, Material)>;
const MOVERS: [&str; 2] = ["reveal-plate", "shadow-blocker"];

/// motion_bench_tests.rs read_rect: one rectangle of a texture.
fn read_rect(r: &Renderer, texture: &wgpu::Texture, rect: [u32; 4]) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect;
    let (width, height) = (x1 - x0, y1 - y0);
    let tight = width * texture.format().block_copy_size(None).unwrap();
    let row = tight.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cache bench crop"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut source = texture.as_image_copy();
    source.origin = wgpu::Origin3d { x: x0, y: y0, z: 0 };
    submit(r, |e| {
        e.copy_texture_to_buffer(
            source,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        )
    });
    let view = wait_map(r, &buffer);
    view.chunks_exact(row as usize).flat_map(|line| line[..tight as usize].to_vec()).collect()
}
/// motion_bench_tests.rs crop_hit: hit records zeroed outside the scene region and tiles.
fn crop_hit(r: &Renderer, rect: [u32; 4]) -> Vec<u8> {
    let state = r.primary.as_ref().unwrap();
    let t = state.targets.as_ref().unwrap();
    let mut hit = read_rect(r, &t.hit.texture, rect);
    let [x0, y0, x1, y1] = rect;
    let columns = (x1 - x0) / 8;
    let tiles = read_rect(r, &t.tiles.texture, [x0 / 8, y0 / 4, x1 / 8, y1 / 4]);
    for (i, h) in hit.chunks_exact_mut(16).enumerate() {
        let x = x0 + i as u32 % (x1 - x0);
        let y = y0 + i as u32 / (x1 - x0);
        let tile = ((x - x0) / 8 + (y - y0) / 4 * columns) as usize;
        if x < state.scene[0] || y < state.scene[1] || x >= state.scene[2] || y >= state.scene[3] || tiles[tile * 4..tile * 4 + 4] == [0; 4] {
            h.fill(0);
        }
    }
    hit
}
fn mover_x(name: &str, p: &Pose) -> Option<f32> {
    match name {
        "reveal-plate" => Some(p.plate_x),
        "shadow-blocker" => Some(p.blocker_x),
        _ => None,
    }
}
/// motion_bench_tests.rs configure: lighting, mover transforms and the camera of one pose.
fn configure(r: &mut Renderer, b: &Bench, p: &Pose, radius: f32, scene: &mut Scene) -> Camera {
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional { travel: -Vec3::from_array(p.to_light), lux: b.sun_lux }],
            environment: [0.; 3],
            exposure_stops: b.exposure_stops,
        },
        &Shadows {
            directional_disk_angle: radius,
            requests: vec![ShadowRequest::Directional { split_ends: vec![10., 40.] }],
            ..Default::default()
        },
    )
    .unwrap();
    for (name, _, instances, _) in scene {
        instances.set(0, Mat4::from_translation(Vec3::new(mover_x(name, p).unwrap_or(0.), 0., 0.))).unwrap();
    }
    Camera::look_at(Vec3::from_array(p.eye), Vec3::from_array(p.target), Vec3::Y, b.fov_y_degrees.to_radians(), 0.01, 100.).unwrap()
}
fn draws(scene: &mut Scene) -> Vec<Draw<'_>> {
    scene.iter_mut().map(|(_, mesh, instances, material)| Draw::new(mesh, instances).with_material(material)).collect()
}
fn frame(r: &mut Renderer, color: &wgpu::Texture, camera: &Camera, scene: &mut Scene) -> FrameReport {
    render(r, color, camera, &mut draws(scene))
}
fn upload_scene(r: &mut Renderer, b: &Bench) -> Scene {
    let mut scene = vec![];
    for g in &b.meshes {
        let vertices: Vec<_> = g
            .positions
            .iter()
            .zip(&g.normals)
            .map(|(&position, &normal)| Vertex { position, normal, uv: [0.; 2], color: [1.; 4] })
            .collect();
        let mesh = r.try_upload_mesh(&MeshData { vertices: &vertices, indices: &g.indices }).unwrap().unwrap();
        r.test_drain("cache bench mesh");
        let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("cache bench instances");
        let material = r
            .try_create_material(&MaterialDescriptor {
                base_color: b.material.base_color,
                metallic: b.material.metallic,
                roughness: g.roughness,
                double_sided: b.material.double_sided,
                thin: g.thin,
                ..Default::default()
            })
            .unwrap()
            .unwrap();
        r.test_drain("cache bench material");
        scene.push((g.name.clone(), mesh, instances, material));
    }
    scene
}

/// motion_bench_tests.rs FilmCrop: the S1 footprint film estimator on the registered crop.
struct FilmCrop {
    pipelines: [wgpu::ComputePipeline; 3],
    accumulator: wgpu::ComputePipeline,
    centre: wgpu::Buffer,
    sum: wgpu::Buffer,
    params: wgpu::Buffer,
    rect: [u32; 4],
    size: [u32; 2],
}
impl FilmCrop {
    fn new(r: &Renderer, b: &Bench) -> Self {
        let [x0, y0, x1, y1] = b.crop;
        let outside = |id: &str| format!("({id}.x<{x0}u || {id}.y<{y0}u || {id}.x>={x1}u || {id}.y>={y1}u)");
        let mut sources = super::super::super::film_reference_tests::film_sources();
        sources.kernels = edit(
            &sources.kernels,
            (
                "fn primary_scene_trace(id:vec2<u32>,size:vec2<u32>)->PrimaryTrace {",
                &format!(
                    "fn primary_scene_trace(id:vec2<u32>,size:vec2<u32>)->PrimaryTrace {{ if {} {{return PrimaryTrace(vec4<u32>(0u),vec3<f32>(0.0),array<vec3<f32>,3>(vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0)));}}",
                    outside("id")
                ),
            ),
        );
        sources.kernels = edit(&sources.kernels, ("let inside=all(id<size);", &format!("let inside=all(id<size) && !{};", outside("id"))));
        sources.shading = edit(
            &sources.shading,
            ("fn primary_shade_pixel(id:vec2<u32>,size:vec2<u32>) {", &format!("fn primary_shade_pixel(id:vec2<u32>,size:vec2<u32>) {{ if {} {{return;}}", outside("id"))),
        );
        let pipelines = r.primary.as_ref().unwrap().test_film_pipelines(&r.device, &sources.kernels, &sources.shading);
        let accumulator_source = include_str!("../support/film_accumulate.wgsl")
            .replace("if any(id.xy>=size) {return;}", &format!("if any(id.xy>=size) || {} {{return;}}", outside("id")))
            .replace("let i=id.x+id.y*size.x;", &format!("let i=(id.x-{x0}u)+(id.y-{y0}u)*{}u;", x1 - x0));
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("N7a cropped film accumulator"),
            source: wgpu::ShaderSource::Wgsl(accumulator_source.into()),
        });
        let accumulator = r.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("accumulate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let make = |bytes, usage| r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage, mapped_at_creation: false });
        let pixels = u64::from((x1 - x0) * (y1 - y0));
        Self {
            pipelines,
            accumulator,
            centre: make(pixels * 16, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
            sum: make(pixels * 48, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC),
            params: make(48, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            rect: b.crop,
            size: b.size,
        }
    }
    fn integrate(&self, r: &mut Renderer, camera: &Camera, out: &std::path::Path, pose: usize) {
        fs::create_dir_all(out).unwrap();
        let start = Instant::now();
        r.test_drain("cache bench film centre");
        let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let mut world = read_rect(r, &t.world.texture, self.rect);
        let mut normal = read_rect(r, &t.normal.texture, self.rect);
        let hit = crop_hit(r, self.rect);
        for ((w, n), h) in world.chunks_exact_mut(16).zip(normal.chunks_exact_mut(16)).zip(hit.chunks_exact(16)) {
            if h[..4] == [0; 4] {
                w.fill(0);
                n.fill(0);
            }
        }
        let centre: Vec<_> = world.chunks_exact(16).zip(hit.chunks_exact(16)).flat_map(|(w, h)| w[..12].iter().chain(h[..4].iter()).copied()).collect();
        let prefix = out.join(format!("pose-{pose}"));
        for (suffix, bytes) in [("world.f32", &world), ("normal.f32", &normal), ("hit.u32", &hit)] {
            fs::write(prefix.with_extension(suffix), bytes).unwrap();
        }
        r.queue.write_buffer(&self.centre, 0, &centre);
        let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let views = [
            &r.primary.as_ref().unwrap().targets.as_ref().unwrap().hit.texture,
            &t.world.texture,
            &t.visibility.texture,
            &r.lighting.hdr.as_ref().unwrap().resolved.texture,
        ]
        .map(|t| t.create_view(&Default::default()));
        let mut entries: Vec<_> =
            views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry { binding: i as u32, resource: wgpu::BindingResource::TextureView(v) }).collect();
        entries.extend([(4, &self.centre), (5, &self.sum), (6, &self.params)].map(|(binding, buffer)| wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }));
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.accumulator.get_bind_group_layout(0), entries: &entries });
        submit(r, |e| e.clear_buffer(&self.sum, 0, None));
        let inverse = camera.view().inverse();
        let eye = inverse.w_axis;
        let forward = -inverse.z_axis;
        let mut values: Vec<_> = eye.to_array().into_iter().chain(forward.to_array()).flat_map(f32::to_le_bytes).collect();
        values.resize(48, 0);
        for sample in 0..256u32 {
            r.queue.write_buffer(&r.uniform, 68, &(sample + 1).to_le_bytes());
            values[32..36].copy_from_slice(&sample.to_le_bytes());
            r.queue.write_buffer(&self.params, 0, &values);
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_trace(&mut encoder, &self.pipelines[0], primary_ray_gpu::TRACE_TILE, true, None);
            r.encode_primary_fresh(&mut encoder, &self.pipelines[1], None);
            r.encode_primary_shade(&mut encoder, &self.pipelines[2], primary_ray_gpu::SHADE_TILE, primary_ray_gpu::Shading::Soft, None);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.accumulator);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(4), 1);
            }
            r.queue.submit([encoder.finish()]);
            if sample % 16 == 15 {
                r.test_drain("cache bench film samples");
            }
        }
        fs::write(prefix.with_extension("film.bin"), read_buffer(r, &self.sum)).unwrap();
        r.queue.write_buffer(&r.uniform, 68, &0u32.to_le_bytes());
        fs::write(
            prefix.with_extension("json"),
            serde_json::to_vec(&serde_json::json!({"pose":pose,"size":[self.rect[2]-self.rect[0],self.rect[3]-self.rect[1]],"native_size":self.size,"crop":self.rect,"samples":256,"rays_per_sample":256,"eye":eye.to_array(),"forward":forward.to_array(),"seconds":start.elapsed().as_secs_f64(),"scope":"registered crop at native pixel coordinates; S1 production receiver policy; regenerated by N7a"})).unwrap(),
        )
        .unwrap();
    }
}

/// The software placements' model, normal and instance records for the current transforms;
/// movers are flagged in instance info.w so static fills skip them.
fn placement_bytes(scene: &Scene, bvhs: &[Bvh], single: &[bool]) -> [Vec<u8>; 3] {
    let mut out = [vec![], vec![], vec![]];
    for (slot, (name, _, instances, _)) in scene.iter().enumerate() {
        let model = instances.state.models[0];
        out[0].extend(crate::mesh::matrix_bytes(model));
        out[1].extend(crate::normal::NormalTransform::new(model).bytes());
        let mut record = bvh::instance(model, &bvhs[slot], slot as u32, single[slot]);
        if MOVERS.contains(&name.as_str()) {
            record[92..96].copy_from_slice(&1u32.to_le_bytes());
        }
        out[2].extend(record);
    }
    out
}
fn f32_crop(counts: &[u8]) -> Vec<u8> {
    u32s(counts).iter().flat_map(|&c| (c as f32 / 256.0).to_le_bytes()).collect()
}

#[test]
#[ignore = "GPU lock, 4 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST (S2/S3 scene); offscreen"]
fn gpu_visibility_cache_bench() {
    let m = manifest();
    let b: Bench = serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let started = Instant::now();
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut scene = upload_scene(&mut r, &b);
    let color = target(&r, b.size[0], b.size[1]);
    let meshes: Vec<MeshData32> = b.meshes.iter().map(|g| (g.positions.clone(), g.indices.clone())).collect();
    let (bvhs, bvh_build) = build_all(&meshes, 4, 16.0);
    let single: Vec<bool> = vec![!b.material.double_sided; b.meshes.len()];
    let sizes = Sizes {
        slots: m["slots"].as_u64().unwrap_or(1 << 22) as u32,
        queue: m["queue"].as_u64().unwrap_or(1 << 21) as u32,
        pending: b.size[0] * b.size[1],
    };
    let buffers = Cache::buffers(&r, sizes);
    let refs: Vec<&Bvh> = bvhs.iter().collect();
    let packed = bvh::pack(&refs);
    let stack = packed.stack;
    // The first pose's production frame creates the traced targets N0's bindings read.
    let first = &b.tracks[0].frames[0];
    r.set_soft_shadows(SoftShadows::Interactive);
    let camera = configure(&mut r, &b, first, 0.04, &mut scene);
    frame(&mut r, &color, &camera, &mut scene);
    r.test_drain("cache bench first frame");
    let (sw, placements) = {
        let ones: Vec<_> = buffers.iter().map(|b| [b.as_entire_buffer_binding()]).collect();
        let extra: Vec<_> = Cache::layout_entries().into_iter().zip(&ones).map(|(e, b)| (e, wgpu::BindingResource::BufferArray(b))).collect();
        Software::new(&r, &draws(&mut scene), &bvhs, &packed, &extra)
    };
    drop(refs);
    let movers = scene.iter().any(|(name, ..)| MOVERS.contains(&name.as_str()));
    let soft = primary_ray_gpu::shader_sources(SOURCE).unwrap()[1].clone();
    let constants = bvh::constants(meshes.len(), placements.len(), stack);
    let cache = Cache::new(&r, &sw, buffers, sizes, &vc::traced_source(&soft, &constants));
    let pipes = Pipelines::new(&r, &sw, meshes.len(), placements.len(), stack);
    let scene_box = placements.iter().fold(bvh::Aabb::EMPTY, |s, &(slot, model, _)| {
        let w = bvh::world_bounds(model, &bvhs[slot]);
        bvh::Aabb { lo: s.lo.min(w.lo), hi: s.hi.max(w.hi) }
    });
    // Movers sweep: widen the scene box over their whole tracks (x only).
    let sweep = b.tracks.iter().flat_map(|t| t.frames.iter()).flat_map(|p| [p.plate_x, p.blocker_x]).fold(0f32, |m, x| m.max(x.abs()));
    let scene_box = bvh::Aabb { lo: scene_box.lo - Vec3::new(sweep, 0., 0.), hi: scene_box.hi + Vec3::new(sweep, 0., 0.) };
    let sided = !b.material.double_sided;
    let budget = m["budget"].as_u64().unwrap_or(65_536) as u32;
    let prefetch = m["prefetch"].as_bool().unwrap_or(true);
    let pixel_angle = 2.0 * (b.fov_y_degrees.to_radians() * 0.5).tan() / b.size[1] as f32;
    let mut stamp = 0u32;
    let cases: Vec<(String, f32)> = m["cases"]
        .as_array()
        .map(|a| a.iter().map(|c| (c[0].as_str().unwrap().to_owned(), c[1].as_f64().unwrap() as f32)).collect())
        .unwrap_or_else(|| b.tracks.iter().map(|t| (t.name.clone(), 0.04)).collect());
    let references = m["references"].as_bool().unwrap_or(true);
    let mut summary = serde_json::Map::new();
    summary.insert("bvh_build".into(), bvh_build);
    summary.insert("memory".into(), serde_json::json!({"table_bytes": sizes.table_bytes(), "work_bytes": sizes.work_bytes(), "sizes": sizes}));
    for (track_name, radius) in &cases {
        let track = b.tracks.iter().find(|t| &t.name == track_name).unwrap();
        let case = format!("{track_name}-r{radius}");
        let directory = out.join(&case);
        fs::create_dir_all(directory.join("cache")).unwrap();
        fs::create_dir_all(directory.join("n0")).unwrap();
        let t0 = Instant::now();
        // 1. References (reused for bit-identical inputs).
        if references && !directory.join("reference-reuse.json").is_file() {
            r.set_soft_shadows(SoftShadows::Exact);
            r.shadows.test_reset_bases();
            let camera = configure(&mut r, &b, &track.frames[0], *radius, &mut scene);
            frame(&mut r, &color, &camera, &mut scene);
            let film = FilmCrop::new(&r, &b);
            let refs = directory.join("reference");
            let mut seen = std::collections::BTreeMap::<Vec<u32>, usize>::new();
            for p in &track.frames {
                let key: Vec<_> = p.eye.into_iter().chain(p.target).chain(p.to_light).chain([p.plate_x, p.blocker_x]).map(f32::to_bits).collect();
                if let Some(&prior) = seen.get(&key) {
                    for suffix in ["world.f32", "normal.f32", "hit.u32", "film.bin"] {
                        let destination = refs.join(format!("pose-{}.{}", p.frame, suffix));
                        if destination.exists() {
                            fs::remove_file(&destination).unwrap();
                        }
                        fs::hard_link(refs.join(format!("pose-{prior}.{suffix}")), destination).unwrap();
                    }
                    let mut info: serde_json::Value = serde_json::from_slice(&fs::read(refs.join(format!("pose-{prior}.json"))).unwrap()).unwrap();
                    info["pose"] = p.frame.into();
                    info["seconds"] = 0.into();
                    info["reused_from_pose"] = prior.into();
                    fs::write(refs.join(format!("pose-{}.json", p.frame)), serde_json::to_vec(&info).unwrap()).unwrap();
                    continue;
                }
                seen.insert(key, p.frame);
                let camera = configure(&mut r, &b, p, *radius, &mut scene);
                frame(&mut r, &color, &camera, &mut scene);
                film.integrate(&mut r, &camera, &refs, p.frame);
                if p.frame % 32 == 0 {
                    eprintln!("cache_bench {case} film {}", p.frame);
                }
            }
            fs::write(directory.join("reference-reuse.json"), serde_json::to_vec(&serde_json::json!({"frames": track.frames.len(), "unique_inputs": seen.len()})).unwrap()).unwrap();
        }
        let reference_s = t0.elapsed().as_secs_f64();
        // 2. The cache over the track, warm from 8 frames at its first pose.
        r.set_soft_shadows(SoftShadows::Interactive);
        r.shadows.test_reset_bases();
        let chart = Chart::new(Vec3::from_array(track.frames[0].to_light).normalize(), *radius);
        cache.clear(&r);
        let mut rows = vec![];
        let mut stats_list = vec![];
        let mut cold = None;
        let steps: Vec<(bool, &Pose)> = std::iter::repeat_n(true, 8).map(|w| (w, &track.frames[0])).chain(track.frames.iter().map(|p| (false, p))).collect();
        for (k, (warmup, p)) in steps.iter().enumerate() {
            let camera = configure(&mut r, &b, p, *radius, &mut scene);
            frame(&mut r, &color, &camera, &mut scene);
            r.test_drain("cache bench frame");
            let [models, normals, instances] = placement_bytes(&scene, &bvhs, &single);
            r.queue.write_buffer(&sw.models, 0, &models);
            r.queue.write_buffer(&sw.normals, 0, &normals);
            r.queue.write_buffer(&sw.instances, 0, &instances);
            r.queue.write_buffer(&sw.frame, 0, &frame_bytes(&r, &camera, b.size, sided));
            let receivers = &r.pipeline.primary.as_ref().unwrap().trace[1];
            submit(&r, |e| r.encode_primary_trace(e, receivers, primary_ray_gpu::TRACE_TILE, true, None));
            stamp = stamp % 0xffff + 1;
            let params = Params {
                chart,
                sun: Vec3::from_array(p.to_light).normalize(),
                scene: scene_box,
                pixel_angle,
                cell_scale: 1.0,
                stamp,
                slots: sizes.slots,
                budget: if k == 0 { sizes.queue } else { budget },
                prefetch,
                queue: sizes.queue,
                pending: sizes.pending,
                probes: 64,
                movers,
            };
            let stats = cache.frame(&r, &sw, &params);
            if k == 0 {
                cold = Some(stats.clone());
            }
            if *warmup {
                continue;
            }
            fs::write(directory.join(format!("cache/{:04}.f32", p.frame)), f32_crop(&read_rect(&r, &sw.counts, b.crop))).unwrap();
            let n0_ms = time_once(&r, |e, s| sw.pass(&r, e, &pipes.sw_shadows, [b.size[0].div_ceil(8), b.size[1].div_ceil(4)], Some(s)));
            fs::write(directory.join(format!("n0/{:04}.f32", p.frame)), f32_crop(&read_rect(&r, &sw.counts, b.crop))).unwrap();
            let hw_ms = time_once(&r, |e, s| encode(&r, &sw, &pipes, Pass::HwShadows, e, Some(s)));
            rows.push(serde_json::json!({"frame": p.frame, "stats": stats, "n0_interactive_ms": n0_ms, "hardware_interactive_ms": hw_ms}));
            stats_list.push(stats);
            if p.frame % 32 == 0 {
                eprintln!("cache_bench {case} frame {} {}", p.frame, serde_json::to_string(&rows.last().unwrap()["stats"]).unwrap());
            }
        }
        let check = cache.validate(&r, &sw, &Params {
            chart,
            sun: Vec3::from_array(track.frames.last().unwrap().to_light).normalize(),
            scene: scene_box,
            pixel_angle,
            cell_scale: 1.0,
            stamp: stamp % 0xffff + 1,
            slots: sizes.slots,
            budget,
            prefetch,
            queue: sizes.queue,
            pending: sizes.pending,
            probes: 64,
            movers,
        });
        eprintln!("cache_bench {case} validation {check}");
        let entry = serde_json::json!({"cold_first_warmup_frame": cold, "track": summarize(&stats_list), "frames": rows,
            "validation": check, "reference_s": reference_s, "case_s": t0.elapsed().as_secs_f64()});
        fs::write(directory.join("cache-stats.json"), serde_json::to_vec_pretty(&entry).unwrap()).unwrap();
        summary.insert(case.clone(), serde_json::json!({"track": entry["track"], "validation": entry["validation"], "cold": entry["cold_first_warmup_frame"], "reference_s": reference_s}));
        fs::write(out.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
        assert_eq!(check["mismatches"], 0, "{case}: stored outcomes differ from fresh traces");
    }
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    fs::write(out.join("summary.json"), serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
}
