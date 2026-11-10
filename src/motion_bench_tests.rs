//! S2/S3: native-size production tracks and the S1 film estimator on registered crops.
use super::*;

#[path = "shadow_filter_gpu_tests.rs"]
mod shadow_filter_gpu_tests;

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
    radii: Vec<f32>,
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

fn crop(bytes: &[u8], size: [u32; 2], rect: [u32; 4], stride: usize) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect.map(|v| v as usize);
    let width = size[0] as usize;
    let mut result = Vec::with_capacity((x1 - x0) * (y1 - y0) * stride);
    for y in y0..y1 {
        result.extend_from_slice(&bytes[(y * width + x0) * stride..(y * width + x1) * stride]);
    }
    result
}

/// Read only the scored native pixels. F16 finiteness needs exponent bits, not
/// a floating-point conversion/power for every channel of a full image.
fn read_rect(r: &Renderer, texture: &wgpu::Texture, rect: [u32; 4]) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect;
    let width = x1 - x0;
    let height = y1 - y0;
    let tight = width * texture.format().block_copy_size(None).unwrap();
    let row = tight.div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("motion crop readback"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = r.device.create_command_encoder(&Default::default());
    let mut source = texture.as_image_copy();
    source.origin = wgpu::Origin3d { x: x0, y: y0, z: 0 };
    e.copy_texture_to_buffer(
        source,
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let done = Arc::new(OnceLock::new());
    let callback = done.clone();
    e.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |v| {
        callback.set(v).unwrap();
    });
    r.queue.submit([e.finish()]);
    r.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    done.get().unwrap().as_ref().unwrap();
    let view = buffer.get_mapped_range(..).unwrap();
    let mut bytes = Vec::with_capacity((tight * height) as usize);
    for row in view.chunks_exact(row as usize) {
        bytes.extend_from_slice(&row[..tight as usize]);
    }
    drop(view);
    buffer.unmap();
    if texture.format() == wgpu::TextureFormat::Rgba16Float {
        assert!(
            bytes
                .chunks_exact(2)
                .all(|v| u16::from_le_bytes(v.try_into().unwrap()) & 0x7c00 != 0x7c00),
            "nonfinite HDR"
        );
    }
    bytes
}

fn hdr_rect(r: &Renderer, rect: [u32; 4]) -> Vec<u8> {
    read_rect(r, &r.lighting.hdr.as_ref().unwrap().resolved.texture, rect)
}

fn frame_inputs(r: &Renderer, p: &Pose) -> serde_json::Value {
    serde_json::json!({
        "camera": r.camera_shadow.unwrap().to_vec(),
        "light": r.lighting.pending.records,
        "lighting": r.lighting.pending.uniform(Vec3::from_array(p.eye)),
        "shadow_views_sha256": r.shadows.published_hash,
        "sun": p.to_light,
    })
}

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
        if x < state.scene[0]
            || y < state.scene[1]
            || x >= state.scene[2]
            || y >= state.scene[3]
            || tiles[tile * 4..tile * 4 + 4] == [0; 4]
        {
            h.fill(0);
        }
    }
    hit
}

fn configure(r: &mut Renderer, b: &Bench, p: &Pose, radius: f32, scene: &mut Scene) -> Camera {
    r.set_lighting_and_shadows(
        &Lighting {
            lights: vec![Light::Directional {
                travel: -Vec3::from_array(p.to_light),
                lux: b.sun_lux,
            }],
            environment: [0.; 3],
            exposure_stops: b.exposure_stops,
        },
        &Shadows {
            directional_disk_angle: radius,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![10., 40.],
            }],
            ..Default::default()
        },
    )
    .unwrap();
    for (name, _, instances, _) in scene {
        let x = match name.as_str() {
            "reveal-plate" => p.plate_x,
            "shadow-blocker" => p.blocker_x,
            _ => 0.,
        };
        instances
            .set(0, Mat4::from_translation(Vec3::new(x, 0., 0.)))
            .unwrap();
    }
    Camera::look_at(
        Vec3::from_array(p.eye),
        Vec3::from_array(p.target),
        Vec3::Y,
        b.fov_y_degrees.to_radians(),
        0.01,
        100.,
    )
    .unwrap()
}

fn frame(
    r: &mut Renderer,
    color: &wgpu::Texture,
    camera: &Camera,
    scene: &mut Scene,
) -> FrameReport {
    let mut draws: Vec<_> = scene
        .iter_mut()
        .map(|(_, mesh, instances, material)| Draw::new(mesh, instances).with_material(material))
        .collect();
    render(r, color, camera, &mut draws)
}

/// Only reference rays are cropped. Production dispatches, pixel coordinates, derivatives,
/// stratification and the 256-ray estimator retain their native 1920x1080 meanings.
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
    fn new(r: &Renderer, b: &Bench, radius: f32) -> Self {
        let [x0, y0, x1, y1] = b.crop;
        let outside =
            |id: &str| format!("({id}.x<{x0}u || {id}.y<{y0}u || {id}.x>={x1}u || {id}.y>={y1}u)");
        let mut sources = film_sources();
        // The point-source reference keeps the reference-only fresh resources
        // alive but traces exactly the central direction for every source sample.
        if radius == 0. {
            sources.kernels = sources.kernels.replace("slope*sqrt(r.x)", "0.0*sqrt(r.x)");
            sources.kernels = edit(
                &sources.kernels,
                (
                    "override FRESH_DIRECTIONS:u32=256u;",
                    "override FRESH_DIRECTIONS:u32=1u;",
                ),
            );
        }
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
        sources.kernels = edit(
            &sources.kernels,
            (
                "let inside=all(id<size);",
                &format!("let inside=all(id<size) && !{};", outside("id")),
            ),
        );
        sources.shading = edit(
            &sources.shading,
            (
                "fn primary_shade_pixel(id:vec2<u32>,size:vec2<u32>) {",
                &format!(
                    "fn primary_shade_pixel(id:vec2<u32>,size:vec2<u32>) {{ if {} {{return;}}",
                    outside("id")
                ),
            ),
        );
        let pipelines = r.primary.as_ref().unwrap().test_film_pipelines(
            &r.device,
            &sources.kernels,
            &sources.shading,
        );
        let accumulator_source = include_str!("../support/film_accumulate.wgsl")
            .replace(
                "if any(id.xy>=size) {return;}",
                &format!("if any(id.xy>=size) || {} {{return;}}", outside("id")),
            )
            .replace(
                "let i=id.x+id.y*size.x;",
                &format!("let i=(id.x-{x0}u)+(id.y-{y0}u)*{}u;", x1 - x0),
            );
        let module = r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("S1 cropped film accumulator"),
            source: wgpu::ShaderSource::Wgsl(accumulator_source.into()),
        });
        let accumulator = r
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("accumulate"),
                compilation_options: Default::default(),
                cache: None,
            });
        let make = |bytes, usage| {
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage,
                mapped_at_creation: false,
            })
        };
        let pixels = u64::from((x1 - x0) * (y1 - y0));
        Self {
            pipelines,
            accumulator,
            centre: make(
                pixels * 16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            sum: make(
                pixels * 48,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ),
            params: make(
                48,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            rect: b.crop,
            size: b.size,
        }
    }
    fn integrate(&self, r: &mut Renderer, camera: &Camera, out: &std::path::Path, pose: usize) {
        fs::create_dir_all(out).unwrap();
        let start = Instant::now();
        r.test_drain("motion film centre");
        let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let mut world = read_rect(r, &t.world.texture, self.rect);
        let mut normal = read_rect(r, &t.normal.texture, self.rect);
        let hit = crop_hit(r, self.rect);
        for ((w, n), h) in world
            .chunks_exact_mut(16)
            .zip(normal.chunks_exact_mut(16))
            .zip(hit.chunks_exact(16))
        {
            if h[..4] == [0; 4] {
                w.fill(0);
                n.fill(0);
            }
        }
        let centre: Vec<_> = world
            .chunks_exact(16)
            .zip(hit.chunks_exact(16))
            .flat_map(|(w, h)| w[..12].iter().chain(h[..4].iter()).copied())
            .collect();
        let prefix = out.join(format!("pose-{pose}"));
        for (suffix, bytes) in [
            ("world.f32", &world),
            ("normal.f32", &normal),
            ("hit.u32", &hit),
        ] {
            fs::write(prefix.with_extension(suffix), bytes).unwrap();
        }
        r.queue.write_buffer(&self.centre, 0, &centre);
        let t = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let views = [
            &r.primary
                .as_ref()
                .unwrap()
                .targets
                .as_ref()
                .unwrap()
                .hit
                .texture,
            &t.world.texture,
            &t.visibility.texture,
            &r.lighting.hdr.as_ref().unwrap().resolved.texture,
        ]
        .map(|t| t.create_view(&Default::default()));
        let mut entries: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        entries.extend([(4, &self.centre), (5, &self.sum), (6, &self.params)].map(
            |(binding, buffer)| wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            },
        ));
        let bind = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.accumulator.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut clear = r.device.create_command_encoder(&Default::default());
        clear.clear_buffer(&self.sum, 0, None);
        r.queue.submit([clear.finish()]);
        let inverse = camera.view().inverse();
        let eye = inverse.w_axis;
        let forward = -inverse.z_axis;
        let mut values: Vec<_> = eye
            .to_array()
            .into_iter()
            .chain(forward.to_array())
            .flat_map(f32::to_le_bytes)
            .collect();
        values.resize(48, 0);
        for sample in 0..256u32 {
            r.queue
                .write_buffer(&r.uniform, 68, &(sample + 1).to_le_bytes());
            values[32..36].copy_from_slice(&sample.to_le_bytes());
            r.queue.write_buffer(&self.params, 0, &values);
            let mut encoder = r.device.create_command_encoder(&Default::default());
            r.encode_primary_trace(
                &mut encoder,
                &self.pipelines[0],
                primary_ray_gpu::TRACE_TILE,
                true,
                None,
            );
            r.encode_primary_fresh(&mut encoder, &self.pipelines[1], None);
            r.encode_primary_shade(
                &mut encoder,
                &self.pipelines[2],
                primary_ray_gpu::SHADE_TILE,
                primary_ray_gpu::Shading::Soft,
                None,
            );
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.accumulator);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(4), 1);
            }
            r.queue.submit([encoder.finish()]);
            if sample % 16 == 15 {
                r.test_drain("motion film samples");
            }
        }
        fs::write(
            prefix.with_extension("film.bin"),
            read_buffer(r, &self.sum, self.sum.size()),
        )
        .unwrap();
        r.queue.write_buffer(&r.uniform, 68, &0u32.to_le_bytes());
        fs::write(prefix.with_extension("json"),serde_json::to_vec(&serde_json::json!({"pose":pose,"size":[self.rect[2]-self.rect[0],self.rect[3]-self.rect[1]],"native_size":self.size,"crop":self.rect,"samples":256,"rays_per_sample":256,"eye":eye.to_array(),"forward":forward.to_array(),"seconds":start.elapsed().as_secs_f64(),"scope":"registered crop at native pixel coordinates; S1 production receiver policy"})).unwrap()).unwrap();
    }
}

fn upload_scene(r: &mut Renderer, b: &Bench) -> Scene {
    let mut scene = vec![];
    for g in &b.meshes {
        let vertices: Vec<_> = g
            .positions
            .iter()
            .zip(&g.normals)
            .map(|(&position, &normal)| Vertex {
                position,
                normal,
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect();
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &g.indices,
            })
            .unwrap()
            .unwrap();
        r.test_drain("motion upload mesh");
        let instances = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
        r.test_drain("motion instances");
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
        r.test_drain("motion material");
        scene.push((g.name.clone(), mesh, instances, material));
    }
    scene
}

/// Compare complete, independently seeded replays. Switching modes per frame
/// would discard the history that precedes light-change resets.
#[test]
#[ignore = "owned GPU lease; S3 manifest; 4 GiB preflight / 2 GiB abort"]
fn gpu_motion_reset_identity() {
    let m = manifest();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(false);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let mut scene = upload_scene(&mut r, &b);
    let color = target(&r, b.size[0], b.size[1]);
    let track = &b.tracks[0];
    let frames = m["frames"].as_u64().unwrap_or(track.frames.len() as u64) as usize;
    let radius = *b.radii.iter().find(|&&v| v > 0.).unwrap();
    let full_hdr = m["full_hdr"].as_bool().unwrap_or(false);
    let seed_replays = m["seed_replays"].as_bool().unwrap_or(true);
    let mut rows = vec![];
    for (name, mode) in [
        ("Interactive", SoftShadows::Interactive),
        ("Temporal", SoftShadows::Temporal),
    ] {
        r.set_soft_shadows(mode);
        for _ in 0..8 {
            if seed_replays {
                r.shadows.test_reset_bases();
            }
            for p in &track.frames {
                let camera = configure(&mut r, &b, p, radius, &mut scene);
                frame(&mut r, &color, &camera, &mut scene);
            }
        }
        // Replay setup belongs before the first frame, never inside the track:
        // transported shadow bases are part of its light sampling inputs.
        if seed_replays {
            r.shadows.test_reset_bases();
        }
        r.set_soft_shadows(SoftShadows::Interactive);
        let camera = configure(&mut r, &b, &track.frames[0], radius, &mut scene);
        frame(&mut r, &color, &camera, &mut scene);
        r.set_soft_shadows(mode);
        for p in track.frames.iter().take(frames) {
            let camera = configure(&mut r, &b, p, radius, &mut scene);
            frame(&mut r, &color, &camera, &mut scene);
            let reset = mode == SoftShadows::Interactive
                || r.primary.as_ref().unwrap().film.as_ref().unwrap().reset;
            if !reset {
                continue;
            }
            let t = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
            let f = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
            let outputs = [
                hdr_rect(&r, b.crop),
                crop_hit(&r, b.crop),
                read_rect(&r, &f.visibility.texture, b.crop),
                read_rect(
                    &r,
                    &t.tiles.texture,
                    [b.crop[0] / 8, b.crop[1] / 4, b.crop[2] / 8, b.crop[3] / 4],
                ),
            ];
            let inputs = frame_inputs(&r, p);
            let full_hash = full_hdr.then(|| digest(&hdr_rect(&r, [0, 0, b.size[0], b.size[1]])));
            if mode == SoftShadows::Interactive {
                for (kind, bytes) in ["hdr", "hit", "counts", "tiles"].into_iter().zip(&outputs) {
                    fs::write(out.join(format!("{name}-{}-{kind}.bin", p.frame)), bytes).unwrap();
                }
                fs::write(
                    out.join(format!("{name}-{}-inputs.json", p.frame)),
                    serde_json::to_vec(
                        &serde_json::json!({"inputs":inputs,"full_hdr_sha256":full_hash}),
                    )
                    .unwrap(),
                )
                .unwrap();
            } else {
                let mut same = vec![];
                let mut differences = vec![];
                for ((kind, bytes), stride) in ["hdr", "hit", "counts", "tiles"]
                    .into_iter()
                    .zip(&outputs)
                    .zip([8, 16, 4, 4])
                {
                    let expected =
                        fs::read(out.join(format!("Interactive-{}-{kind}.bin", p.frame))).unwrap();
                    same.push(expected == *bytes);
                    for (i, (a, z)) in expected
                        .chunks_exact(stride)
                        .zip(bytes.chunks_exact(stride))
                        .enumerate()
                        .filter(|(_, (a, z))| a != z)
                        .take(32)
                    {
                        differences.push(serde_json::json!({"kind":kind,"pixel":[b.crop[0]+i as u32%(b.crop[2]-b.crop[0]),b.crop[1]+i as u32/(b.crop[2]-b.crop[0])],"expected":a,"actual":z}));
                    }
                    if expected != *bytes {
                        fs::write(out.join(format!("Temporal-{}-{kind}.bin", p.frame)), bytes)
                            .unwrap();
                    }
                }
                let expected: serde_json::Value = serde_json::from_slice(
                    &fs::read(out.join(format!("Interactive-{}-inputs.json", p.frame))).unwrap(),
                )
                .unwrap();
                rows.push(serde_json::json!({"frame":p.frame,"identical":same,"inputs_equal":inputs==expected["inputs"],"interactive_inputs":expected["inputs"],"temporal_inputs":inputs,"full_hdr_equal":serde_json::json!(full_hash)==expected["full_hdr_sha256"],"full_hdr_sha256":full_hash,"crop_sha256":outputs.each_ref().map(|b|digest(b)),"differences":differences}));
            }
        }
    }
    fs::write(
        out.join("reset-identity.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    if let Some(expected) = m["expected_resets"].as_u64() {
        assert_eq!(rows.len() as u64, expected);
    }
    assert!(
        rows.iter().all(|v| v["inputs_equal"] == true),
        "replays use different frame inputs; see reset-identity.json"
    );
    assert!(
        rows.iter().all(|v| v["full_hdr_equal"] == true
            && v["identical"].as_array().unwrap().iter().all(|x| x == true)),
        "reset differs; see reset-identity.json"
    );
}

/// N8b decision trace (virtual_shadow_tests::diagnose) through one case's track, in
/// order so the page cache sees the production sequence; `frames` selects the traced ones.
#[test]
#[ignore = "N8b GPU lease, 4 GiB start/2 GiB abort; offscreen decision trace"]
fn gpu_motion_virtual_diagnose() {
    let m = manifest();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let case = m["case"].as_str().unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap()).join(case);
    fs::create_dir_all(&out).unwrap();
    let frames: Vec<u64> = m["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_u64().unwrap())
        .collect();
    let mut r = super::virtual_shadow_tests::diagnosis_renderer(&m);
    let mut scene = upload_scene(&mut r, &b);
    let color = target(&r, b.size[0], b.size[1]);
    let track = b
        .tracks
        .iter()
        .find(|t| case.starts_with(&format!("{}-r", t.name)))
        .unwrap();
    let radius: f32 = case.rsplit("-r").next().unwrap().parse().unwrap();
    r.shadows.test_reset_bases();
    let last = *frames.iter().max().unwrap();
    for p in track.frames.iter().take_while(|p| p.frame as u64 <= last) {
        let camera = configure(&mut r, &b, p, radius, &mut scene);
        if frames.contains(&(p.frame as u64)) {
            let label = format!("{:04}", p.frame);
            super::virtual_shadow_tests::diagnose(&mut r, &out, &label, Some(b.crop), &mut |r| {
                frame(r, &color, &camera, &mut scene);
            });
        } else {
            r.set_soft_shadows(SoftShadows::Virtual);
            frame(&mut r, &color, &camera, &mut scene);
        }
    }
}

/// N8b variant screen on one pose of a case: per `variants` entry ({name: {"constants":
/// {...}, "virtual": {...}}}), N8 counts against Interactive's on the same directions, and
/// N8's shadow GPU time static and with the sun turning by `turn_degrees` per frame.
#[test]
#[ignore = "N8b GPU lease, 4 GiB start/2 GiB abort; offscreen variant screen"]
fn gpu_motion_virtual_screen() {
    use super::virtual_shadow_tests as vs;
    let m = manifest();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let case = m["case"].as_str().unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = vs::diagnosis_renderer(&m);
    let mut scene = upload_scene(&mut r, &b);
    let color = target(&r, b.size[0], b.size[1]);
    let track = b
        .tracks
        .iter()
        .find(|t| case.starts_with(&format!("{}-r", t.name)))
        .unwrap();
    let radius: f32 = case.rsplit("-r").next().unwrap().parse().unwrap();
    let p = &track.frames[m["frame"].as_u64().unwrap_or(0) as usize];
    let turn = m["turn_degrees"].as_f64().unwrap_or(0.05) as f32;
    r.shadows.test_reset_bases();
    let camera = configure(&mut r, &b, p, radius, &mut scene);
    r.set_soft_shadows(SoftShadows::Interactive);
    frame(&mut r, &color, &camera, &mut scene);
    let [hit, rt] = interactive_outputs(&r);
    // Bring the GPU to its working clocks before the first variant's timing.
    for _ in 0..60 {
        frame(&mut r, &color, &camera, &mut scene);
    }
    let ms = |r: &mut Renderer| -> Vec<f64> {
        r.test_drain("N8b screen");
        r.poll()
            .unwrap()
            .gpu
            .iter()
            .filter_map(|s| s.visibility_compute.map(|d| d.as_secs_f64() * 1e3))
            .collect()
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied()
    };
    let mut rows = vec![];
    for (name, v) in m["variants"].as_object().unwrap() {
        let settings = exact_virtual_settings(&serde_json::json!({ "virtual": v["virtual"] }));
        r.set_virtual_shadow_settings(settings).unwrap();
        r.set_soft_shadows(SoftShadows::Virtual);
        frame(&mut r, &color, &camera, &mut scene);
        vs::set_screen_constants(&mut r, vs::screen_constants(v));
        r.shadows.test_reset_bases();
        let camera = configure(&mut r, &b, p, radius, &mut scene);
        for _ in 0..4 {
            frame(&mut r, &color, &camera, &mut scene);
        }
        let [_, n8] = interactive_outputs(&r);
        let mut row = vs::agreement(&vs::counts(&n8), &vs::counts(&rt), &hit);
        row["hash"] = serde_json::json!(vs::sha256(&n8));
        let _ = ms(&mut r);
        for _ in 0..24 {
            frame(&mut r, &color, &camera, &mut scene);
        }
        let fixed = ms(&mut r);
        let to_light = Vec3::from_array(p.to_light);
        for i in 1..=16 {
            let turned =
                Mat4::from_rotation_y((turn * i as f32).to_radians()).transform_vector3(to_light);
            let (lighting, shadows) = (
                Lighting {
                    lights: vec![Light::Directional {
                        travel: -turned,
                        lux: b.sun_lux,
                    }],
                    environment: [0.; 3],
                    exposure_stops: b.exposure_stops,
                },
                r.shadows.settings.clone(),
            );
            r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
            frame(&mut r, &color, &camera, &mut scene);
        }
        let turning = ms(&mut r);
        let [_, turned] = interactive_outputs(&r);
        row["turned_hash"] = serde_json::json!(vs::sha256(&turned));
        row["variant"] = serde_json::json!(name);
        row["static_shadow_ms_p50"] = serde_json::json!(median(fixed));
        row["turning_shadow_ms_p50"] = serde_json::json!(median(turning));
        eprintln!("N8b screen {case} {row}");
        rows.push(row);
    }
    fs::write(
        out.join(format!("screen-{case}.json")),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "S2/S3 owned GPU lease; 4 GiB preflight/2 GiB abort; one native-size process per bench"]
fn gpu_motion_bench_batch() {
    let m = manifest();
    let b: Bench =
        serde_json::from_slice(&fs::read(m["scene"].as_str().unwrap()).unwrap()).unwrap();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer_with_features(false, crate::VIRTUAL_FEATURES);
    if m["registered_direct"].as_bool().unwrap_or(false) {
        // RTDEF2's ray-grain gate reuses the registered direct-light films. The
        // complete quarry comparison separately retains current GI/atmosphere defaults.
        r.set_indirect(crate::IndirectSettings {
            enabled: false,
            ..Default::default()
        })
        .unwrap();
        r.set_atmosphere(crate::Atmosphere::off()).unwrap();
    }
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_interactive_shadow_filter(m["shadow_filter"].as_bool().unwrap_or(false))
        .unwrap();
    r.set_virtual_shadow_settings(exact_virtual_settings(&m))
        .unwrap();
    let selected = |key: &str, name: &str| {
        m[key]
            .as_array()
            .is_none_or(|names| names.iter().any(|n| n.as_str() == Some(name)))
    };
    let repeats = m["repeats"].as_u64().unwrap_or(8) as usize;
    let mut scene = upload_scene(&mut r, &b);
    let color = target(&r, b.size[0], b.size[1]);
    let mut timings: Vec<serde_json::Value> = fs::read(out.join("timings.json"))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_default();
    for track in &b.tracks {
        for &radius in &b.radii {
            let case = format!("{}-r{radius}", track.name);
            if !selected("cases", &case) {
                continue;
            }
            let directory = out.join(&case);
            if directory.join("scores-renderlab.json").is_file()
                || directory.join("reference-reuse.json").is_file()
            {
                eprintln!("MOTION {case} GPU evidence complete; reusing it");
                continue;
            }
            fs::create_dir_all(&directory).unwrap();
            let p = &track.frames[0];
            r.shadows.test_reset_bases();
            let camera = configure(&mut r, &b, p, radius, &mut scene);
            r.set_soft_shadows(SoftShadows::Exact);
            frame(&mut r, &color, &camera, &mut scene);
            let visibility =
                centre_shading(&r, &crate::program::standard_returning(VISIBILITY), true);
            let forward = (Vec3::from_array(p.target) - Vec3::from_array(p.eye)).normalize();
            let sample_program = crate::program::standard_returning(&format!(
                "rl_motion_sample({VISIBILITY}.x,dot(input.receiver.world-input.eye,vec3<f32>({:?},{:?},{:?})))",
                forward.x, forward.y, forward.z
            )) + "\nfn rl_motion_sample(v:f32,d:f32)->vec3<f32>{let h=quantizeToF16(d);return vec3<f32>(v,h,d-h+h/1024.0);}";
            let temporal_visibility = per_sample_shading(&r, &sample_program);
            for (name, mode) in [
                ("hard", SoftShadows::Exact),
                ("Exact", SoftShadows::Exact),
                // RTDEF's historical RT-device default, measured in the same scene load.
                ("Raster-Exact", SoftShadows::Exact),
                ("Interactive", SoftShadows::Interactive),
                ("Temporal", SoftShadows::Temporal),
                ("Virtual", SoftShadows::Virtual),
                // Temporal reconstruction over Interactive (docs/m7-tr1b.md): from cold, and
                // after `TR1_WARM` still frames at the track's first pose.
                ("TR1", SoftShadows::Interactive),
                ("TR1-16", SoftShadows::Interactive),
                ("TR1-32", SoftShadows::Interactive),
                ("TR1-64", SoftShadows::Interactive),
                ("TR1-128", SoftShadows::Interactive),
                ("TR1-256", SoftShadows::Interactive),
                ("TR1-Auto16", SoftShadows::Interactive),
                ("TR1-Auto32", SoftShadows::Interactive),
                ("TR1-warm", SoftShadows::Interactive),
                // P-NSQ (docs/p-nsq.md): Virtual under TR1 with main's sampling (eight fixed
                // directions a frame) and with RT's (rotated, 256/64/32/8 in batches of eight).
                ("TR1-Virtual-fixed8", SoftShadows::Virtual),
                ("TR1-Virtual-Auto32", SoftShadows::Virtual),
            ] {
                // N8 is a disk-light mode; a zero radius takes the hard path.
                if !selected("modes", name) || (mode == SoftShadows::Virtual && radius == 0.) {
                    continue;
                }
                const TR1_WARM: usize = 256;
                let reconstruction = crate::Reconstruction {
                    enabled: name.starts_with("TR1"),
                    rotate_directions: name != "TR1-Virtual-fixed8",
                    ..Default::default()
                };
                r.set_reconstruction(reconstruction).unwrap();
                let auto = name.starts_with("TR1-Auto") || name == "TR1-Virtual-Auto32";
                let active_rays = match name {
                    "TR1-Virtual-fixed8" => 8,
                    "TR1-Virtual-Auto32" => 32,
                    _ => name
                        .strip_prefix("TR1-")
                        .and_then(|s| s.strip_prefix("Auto").unwrap_or(s).parse().ok())
                        .unwrap_or(8),
                };
                r.set_reconstruction_shadow_budget(crate::ReconstructionShadowBudget {
                    cold_rays: if auto { 256 } else { active_rays },
                    reset_rays: if auto { 64 } else { active_rays },
                    active_rays,
                    held_rays: 8,
                    settle_frames: 16,
                    cut_rays: if auto {
                        crate::ReconstructionShadowBudget::default().cut_rays
                    } else {
                        active_rays
                    },
                })
                .unwrap();
                r.set_primary_visibility(if name == "Raster-Exact" {
                    PrimaryVisibility::Raster
                } else {
                    PrimaryVisibility::Traced
                })
                .unwrap();
                let path = directory.join(name);
                fs::create_dir_all(&path).unwrap();
                r.set_soft_shadows(mode);
                // P-NSQ: the fixed-eight mode keeps main's N8e walk cap; the other takes the
                // manifest's settings (the current defaults).
                if name.starts_with("TR1-Virtual") {
                    let base = exact_virtual_settings(&m);
                    r.set_virtual_shadow_settings(if name == "TR1-Virtual-fixed8" {
                        crate::VirtualShadowSettings {
                            walk_cap: 16,
                            truncated_walks_block: false,
                            ..base
                        }
                    } else {
                        base
                    })
                    .unwrap();
                }
                // Separate uninterrupted production replays from diagnostic readbacks.
                let mut records = vec![];
                for repeat in 0..repeats {
                    r.shadows.test_reset_bases();
                    let warm = configure(
                        &mut r,
                        &b,
                        p,
                        if name == "hard" { 0. } else { radius },
                        &mut scene,
                    );
                    for _ in 0..8 {
                        frame(&mut r, &color, &warm, &mut scene);
                    }
                    r.test_drain("motion warmup");
                    let _ = r.poll().unwrap();
                    let mut ids = vec![];
                    for p in &track.frames {
                        let start = Instant::now();
                        let camera = configure(
                            &mut r,
                            &b,
                            p,
                            if name == "hard" { 0. } else { radius },
                            &mut scene,
                        );
                        let f = frame(&mut r, &color, &camera, &mut scene);
                        ids.push((f.frame, p.frame, start.elapsed().as_secs_f64() * 1000.));
                        for s in r.poll().unwrap().gpu {
                            records.push((repeat, s));
                        }
                    }
                    r.test_drain("motion timing");
                    for s in r.poll().unwrap().gpu {
                        records.push((repeat, s));
                    }
                    for (id, index, cpu) in ids {
                        if let Some((_, s)) = records
                            .iter()
                            .find(|(rep, s)| *rep == repeat && s.frame == id)
                        {
                            timings.push(serde_json::json!({"case":case,"mode":name,"repeat":repeat,"frame":index,"gpu_ms":s.render_span.as_secs_f64()*1000.,"cpu_service_ms":cpu,"visibility_ms":s.visibility_compute.map(|v|v.as_secs_f64()*1000.),"trace_ms":s.receiver_pass.map(|v|v.as_secs_f64()*1000.),"virtual":(mode == SoftShadows::Virtual).then(|| format!("{:?}", r.virtual_shadow_stats()))}));
                        } else {
                            panic!(
                                "missing production timing {case} {name} repeat {repeat} frame {index}"
                            );
                        }
                    }
                }
                r.shadows.test_reset_bases();
                let _ = configure(
                    &mut r,
                    &b,
                    p,
                    if name == "hard" { 0. } else { radius },
                    &mut scene,
                );
                // Reset history by a mode round trip before the captured contiguous replay.
                r.set_soft_shadows(SoftShadows::Interactive);
                frame(&mut r, &color, &camera, &mut scene);
                r.set_soft_shadows(mode);
                if reconstruction.enabled {
                    // The round-trip frame started a history: restart it.
                    r.set_reconstruction(crate::Reconstruction::off()).unwrap();
                    r.set_reconstruction(reconstruction).unwrap();
                    if name == "TR1-warm" {
                        let warm = configure(&mut r, &b, p, radius, &mut scene);
                        for _ in 0..TR1_WARM {
                            frame(&mut r, &color, &warm, &mut scene);
                        }
                    }
                }
                for p in &track.frames {
                    let camera = configure(
                        &mut r,
                        &b,
                        p,
                        if name == "hard" { 0. } else { radius },
                        &mut scene,
                    );
                    frame(&mut r, &color, &camera, &mut scene);
                    if m["capture_frames"].as_array().is_some_and(|frames| {
                        !frames.iter().any(|f| f.as_u64() == Some(p.frame as u64))
                    }) {
                        continue;
                    }
                    let rgb = hdr_rect(&r, b.crop);
                    fs::write(path.join(format!("{:04}.rgba16f", p.frame)), &rgb).unwrap();
                    let reset = mode == SoftShadows::Temporal
                        && (radius == 0.
                            || r.primary.as_ref().unwrap().film.as_ref().unwrap().reset);
                    let inputs = frame_inputs(&r, p);
                    if reset && radius > 0. {
                        let expected: serde_json::Value = serde_json::from_slice(
                            &fs::read(directory.join(format!("Interactive/{:04}.json", p.frame)))
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            inputs, expected["inputs"],
                            "reset audit needs identical frame inputs"
                        );
                    }
                    fs::write(
                        path.join(format!("{:04}.json", p.frame)),
                        serde_json::to_vec(&serde_json::json!({"reset":reset,"inputs":inputs,
                            "shadow_rays":r.reconstruction_shadow_rays()}))
                        .unwrap(),
                    )
                    .unwrap();
                    if !m["no_full_captures"].as_bool().unwrap_or(false)
                        && selected("full_cases", &case)
                        && m["full_frames"].as_array().is_none_or(|frames| {
                            frames.iter().any(|f| f.as_u64() == Some(p.frame as u64))
                        })
                        && [
                            0, 15, 16, 17, 19, 23, 31, 32, 63, 64, 95, 96, 127, 159, 191, 319, 351,
                            352, 353, 355, 359, 367, 384,
                        ]
                        .contains(&p.frame)
                    {
                        fs::write(
                            path.join(format!("{:04}-full.rgba16f", p.frame)),
                            hdr_rect(&r, [0, 0, b.size[0], b.size[1]]),
                        )
                        .unwrap();
                    }
                    if mode == SoftShadows::Temporal && radius > 0. {
                        let film = r.primary.as_ref().unwrap().film.as_ref().unwrap();
                        let mut e = r.device.create_command_encoder(&Default::default());
                        r.encode_shade_group(
                            &mut e,
                            &temporal_visibility,
                            film.shade_group(),
                            primary_ray_gpu::SHADE_TILE,
                            None,
                        );
                        r.queue.submit([e.finish()]);
                        fs::write(
                            path.join(format!("{:04}-sample.rgba16f", p.frame)),
                            hdr_rect(&r, b.crop),
                        )
                        .unwrap();
                        fs::write(
                            path.join(format!("{:04}-hit.u32", p.frame)),
                            crop_hit(&r, b.crop),
                        )
                        .unwrap();
                    } else if name != "Raster-Exact" {
                        let mut e = r.device.create_command_encoder(&Default::default());
                        r.encode_primary_shade(
                            &mut e,
                            &visibility[usize::from(name != "hard" && radius > 0.)],
                            primary_ray_gpu::SHADE_TILE,
                            if name == "hard" || radius == 0. {
                                primary_ray_gpu::Shading::Hard
                            } else {
                                primary_ray_gpu::Shading::Soft
                            },
                            None,
                        );
                        r.queue.submit([e.finish()]);
                        fs::write(
                            path.join(format!("{:04}-visibility.rgba16f", p.frame)),
                            hdr_rect(&r, b.crop),
                        )
                        .unwrap();
                    }
                }
                let pending = out.join("timings.tmp");
                fs::write(&pending, serde_json::to_vec(&timings).unwrap()).unwrap();
                fs::rename(pending, out.join("timings.json")).unwrap();
                eprintln!("MOTION {case} {name} complete");
            }
            r.set_reconstruction(crate::Reconstruction::off()).unwrap();
            // Candidate-only reruns score against an already generated reference (N8b).
            if m["skip_reference"].as_bool() == Some(true) {
                continue;
            }
            r.set_soft_shadows(SoftShadows::Exact);
            let reference_radius = if radius == 0. { 0.04 } else { radius };
            r.shadows.test_reset_bases();
            let camera = configure(&mut r, &b, p, reference_radius, &mut scene);
            frame(&mut r, &color, &camera, &mut scene);
            let reference = FilmCrop::new(&r, &b, radius);
            let mut reference_inputs = std::collections::BTreeMap::<Vec<u32>, usize>::new();
            for p in &track.frames {
                let key: Vec<_> = p
                    .eye
                    .into_iter()
                    .chain(p.target)
                    .chain(p.to_light)
                    .chain([p.plate_x, p.blocker_x])
                    .map(f32::to_bits)
                    .collect();
                if let Some(&prior) = reference_inputs.get(&key) {
                    let refs = directory.join("reference");
                    for suffix in ["world.f32", "normal.f32", "hit.u32", "film.bin"] {
                        let destination = refs.join(format!("pose-{}.{}", p.frame, suffix));
                        if destination.exists() {
                            fs::remove_file(&destination).unwrap();
                        }
                        fs::hard_link(refs.join(format!("pose-{prior}.{suffix}")), destination)
                            .unwrap();
                    }
                    let mut info: serde_json::Value = serde_json::from_slice(
                        &fs::read(refs.join(format!("pose-{prior}.json"))).unwrap(),
                    )
                    .unwrap();
                    info["pose"] = serde_json::json!(p.frame);
                    info["seconds"] = serde_json::json!(0);
                    info["reused_from_pose"] = serde_json::json!(prior);
                    fs::write(
                        refs.join(format!("pose-{}.json", p.frame)),
                        serde_json::to_vec(&info).unwrap(),
                    )
                    .unwrap();
                    continue;
                }
                reference_inputs.insert(key, p.frame);
                let camera = configure(&mut r, &b, p, reference_radius, &mut scene);
                frame(&mut r, &color, &camera, &mut scene);
                reference.integrate(&mut r, &camera, &directory.join("reference"), p.frame);
                if p.frame == 0 && radius > 0. {
                    // Retain one independent full-dispatch equivalence check of
                    // the crop optimization against the unchanged S1 generator.
                    frame(&mut r, &color, &camera, &mut scene);
                    let full = directory.join("crop-control");
                    integrate_with(&mut r, &camera, &full, 0, true, None);
                    let full_bytes = fs::read(full.join("pose-0.film.bin")).unwrap();
                    let expected = crop(&full_bytes, b.size, b.crop, 48);
                    let mut full_rgb = Vec::with_capacity((b.size[0] * b.size[1] * 16) as usize);
                    for pixel in full_bytes.chunks_exact(48) {
                        for value in pixel[32..48].chunks_exact(4) {
                            full_rgb.extend_from_slice(
                                &(f32::from_le_bytes(value.try_into().unwrap()) / 256.)
                                    .to_le_bytes(),
                            );
                        }
                    }
                    fs::write(directory.join("film-full.rgba32f"), full_rgb).unwrap();
                    let actual = fs::read(directory.join("reference/pose-0.film.bin")).unwrap();
                    assert_eq!(
                        actual, expected,
                        "cropped film differs from S1 full-dispatch reference"
                    );
                    fs::write(directory.join("crop-equivalence.json"),serde_json::to_vec(&serde_json::json!({"byte_equal":true,"sha256":digest(&actual),"native_size":b.size,"crop":b.crop})).unwrap()).unwrap();
                    for entry in fs::read_dir(&full).unwrap() {
                        let path = entry.unwrap().path();
                        if path.extension().is_some_and(|s| s != "json") {
                            fs::remove_file(path).unwrap();
                        }
                    }
                }
                if p.frame % 16 == 0 {
                    eprintln!("MOTION {case} film {}", p.frame);
                }
            }
            fs::write(directory.join("reference-reuse.json"),serde_json::to_vec(&serde_json::json!({"frames":track.frames.len(),"unique_inputs":reference_inputs.len(),"criterion":"bit-identical camera, sun and object transforms; same deterministic S1 footprint/light samples"})).unwrap()).unwrap();
            if let Some(script) = m["score_script"].as_str() {
                let status = std::process::Command::new("py")
                    .args([
                        "-3",
                        "-B",
                        script,
                        "--root",
                        m["root"].as_str().unwrap(),
                        "--case",
                        &case,
                        "--renderlab-only",
                    ])
                    .status()
                    .unwrap();
                assert!(status.success(), "per-case scoring/compaction failed");
            }
        }
    }
    fs::write(
        out.join("complete.json"),
        b"{\"complete\":true,\"software_backend\":\"missing on db3bb2d\"}",
    )
    .unwrap();
}
