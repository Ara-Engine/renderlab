//! M7 G1 GPU gates on U0's SV bench (scripts/m7-u0-scenes.py): Renderlab's sky term
//! against the reference path tracer's sky-only image (`max_depth` 2: camera, surface,
//! sky), on the RT path, the non-RT path (a device without ray queries) and software
//! rays on the RT device. One process loads the bench once and runs every
//! configuration; it writes JSON summaries and inspected PNGs, never raw HDR.
//!
//! RENDERLAB_M7_G1_BENCH: SV's scene.json. RENDERLAB_M7_G1_OUT: output directory.
//! RENDERLAB_M7_G1_SPP: reference samples (default 4096). RENDERLAB_M7_G1_QUICK=1 runs
//! the overview pose and default settings only.
use super::*;
use crate::reference;
use glam::Vec2;
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

struct BenchMesh {
    name: String,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    material: usize,
}
struct Sv {
    meshes: Vec<BenchMesh>,
    /// base rgb, metallic, roughness.
    materials: Vec<[f32; 5]>,
    poses: Vec<(String, Vec3, Vec3)>,
    sky: [f32; 3],
    exposure: f32,
    size: [u32; 2],
    fov: f32,
    populations: Vec<(String, Vec<String>)>,
    sha: String,
}
fn load(path: &Path) -> Sv {
    use sha2::Digest;
    let bytes = std::fs::read(path).unwrap();
    let v: Json = serde_json::from_slice(&bytes).unwrap();
    let f = |x: &Json| x.as_f64().unwrap() as f32;
    let v3 = |x: &Json| [f(&x[0]), f(&x[1]), f(&x[2])];
    let materials = v["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let b = v3(&m["base_color"]);
            assert_eq!(f(&m["specular"]), 1.0, "Renderlab's F0 is 0.04");
            [b[0], b[1], b[2], f(&m["metallic"]), f(&m["roughness"])]
        })
        .collect();
    let meshes = v["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| BenchMesh {
            name: m["name"].as_str().unwrap().into(),
            positions: m["positions"].as_array().unwrap().iter().map(v3).collect(),
            normals: m["normals"].as_array().unwrap().iter().map(v3).collect(),
            indices: m["indices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i.as_u64().unwrap() as u32)
                .collect(),
            material: m["material"].as_u64().unwrap() as usize,
        })
        .collect();
    let poses = v["poses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_string(),
                Vec3::from(v3(&p["eye"])),
                Vec3::from(v3(&p["target"])),
            )
        })
        .collect();
    let populations = v["populations"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, list)| {
            (
                k.clone(),
                list.as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect();
    Sv {
        meshes,
        materials,
        poses,
        sky: v3(&v["sky"]["constant"]),
        exposure: f(&v["exposure_stops"]),
        size: [0, 1].map(|k| v["size"][k].as_u64().unwrap() as u32),
        fov: f(&v["fov_y_degrees"]).to_radians(),
        populations,
        sha: format!("{:x}", sha2::Sha256::digest(&bytes)),
    }
}
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn reference_scene(sv: &Sv, moved: &[(usize, Mat4)], specular: f32) -> reference::Scene {
    reference::Scene {
        meshes: sv
            .meshes
            .iter()
            .map(|m| reference::Mesh {
                positions: m.positions.clone(),
                normals: m.normals.clone(),
                indices: m.indices.clone(),
            })
            .collect(),
        materials: sv
            .materials
            .iter()
            .map(|m| reference::Material {
                base_color: [m[0], m[1], m[2]],
                metallic: m[3],
                roughness: m[4],
                specular,
                emission: [0.0; 3],
            })
            .collect(),
        instances: sv
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| reference::Instance {
                mesh: i,
                material: m.material,
                transform: moved
                    .iter()
                    .find(|(k, _)| *k == i)
                    .map_or(Mat4::IDENTITY, |(_, t)| *t),
            })
            .collect(),
        sun: None,
        sky: sv.sky,
    }
}
fn reference_view(sv: &Sv, eye: Vec3, target: Vec3) -> reference::View {
    let pose = reference::Pose { eye, target };
    reference::View {
        open: pose,
        close: pose,
        up: Vec3::Y,
        fov_y: sv.fov,
        width: sv.size[0],
        height: sv.size[1],
    }
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
            | crate::TRACED_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: limits,
        experimental_features,
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap()
}
fn no_rt_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::RECEIVER_FEATURES
            | crate::VISIBILITY_FEATURES
            | crate::VISIBILITY_RASTER_FEATURES
            | crate::VIRTUAL_FEATURES
            | wgpu::Features::TIMESTAMP_QUERY,
        required_limits: crate::visibility_limits(wgpu::Limits::default()),
        memory_hints: crate::MEMORY_HINTS,
        ..Default::default()
    }))
    .unwrap();
    assert!(!device.features().intersects(
        wgpu::Features::EXPERIMENTAL_RAY_QUERY | wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN
    ));
    (device, queue)
}
const SOURCE: &str = include_str!("../shaders/scene.wgsl");
/// The G1 gate program: Lambert under the engine's sky irradiance and nothing else, so
/// its image isolates `indirect_irradiance`. Its reference has specular 0.
const LAMBERT: &str = "struct Surface { base:vec3<f32>, n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(input.material.base_color.rgb*input.color.rgb,n);
}
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return expose(indirect_irradiance(input,s.n)*s.base/PI);
}
";
const TOON: &str = include_str!("../shaders/programs/toon.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Path_ {
    Rt,
    /// Software rays on the RT device (same images as Rt's primaries).
    RtSoftware,
    NonRt,
}
struct Rig {
    r: Renderer,
    target: wgpu::Texture,
    meshes: Vec<crate::Mesh>,
    instances: Vec<Instances>,
    /// Per bench material: the standard program, the Lambert gate and toon (a non-caller).
    materials: [Vec<Material>; 3],
    _programs: [Program; 2],
    path: Path_,
}
fn rig(sv: &Sv, path: Path_, device: (wgpu::Device, wgpu::Queue)) -> Rig {
    let (device, queue) = device;
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let mut r = if path == Path_::NonRt {
        Renderer::new_with_options(device, queue, FORMAT, Limits::default(), options, SOURCE)
            .unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            Limits::default(),
            options,
            HardwareShadowOptions::default(),
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    };
    assert_eq!(
        r.primary_visibility(),
        if path == Path_::NonRt {
            PrimaryVisibility::VisibilityBuffer
        } else {
            PrimaryVisibility::Traced
        }
    );
    r.set_lighting(&Lighting {
        lights: vec![],
        environment: sv.sky,
        exposure_stops: sv.exposure,
    })
    .unwrap();
    let target = color_target(&r, sv.size);
    let mut meshes = vec![];
    let mut instances = vec![];
    for m in &sv.meshes {
        let vertices: Vec<Vertex> = m
            .positions
            .iter()
            .zip(&m.normals)
            .map(|(&position, &normal)| Vertex {
                position,
                normal,
                uv: [0.0; 2],
                color: [1.0; 4],
            })
            .collect();
        let mesh = loop {
            if let Some(mesh) = r
                .try_upload_mesh(&MeshData {
                    vertices: &vertices,
                    indices: &m.indices,
                })
                .unwrap()
            {
                break mesh;
            }
            r.test_drain("G1 mesh upload");
        };
        r.set_indirect_geometry(&mesh, &m.positions, &m.indices)
            .unwrap();
        let inst = loop {
            if let Some(i) = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap() {
                break i;
            }
            r.test_drain("G1 instance upload");
        };
        meshes.push(mesh);
        instances.push(inst);
    }
    let programs = [("g1-lambert", LAMBERT), ("toon", TOON)].map(|(label, source)| {
        r.try_create_program(&ProgramDescriptor { label, source })
            .unwrap()
    });
    let materials = [None, Some(&programs[0]), Some(&programs[1])].map(|program| {
        sv.materials
            .iter()
            .map(|m| {
                r.try_create_material(&MaterialDescriptor {
                    base_color: [m[0], m[1], m[2], 1.0],
                    metallic: m[3],
                    roughness: m[4],
                    double_sided: true,
                    program,
                    ..Default::default()
                })
                .unwrap()
                .unwrap()
            })
            .collect()
    });
    r.set_indirect_profiling(true);
    Rig {
        r,
        target,
        meshes,
        instances,
        materials,
        _programs: programs,
        path,
    }
}
pub(super) fn color_target(r: &Renderer, size: [u32; 2]) -> wgpu::Texture {
    r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("G1 color"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
impl Rig {
    fn settings(&mut self, s: IndirectSettings) {
        self.r
            .set_indirect(IndirectSettings {
                force_software: self.path == Path_::RtSoftware,
                ..s
            })
            .unwrap();
    }
    fn frame(&mut self, sv: &Sv, eye: Vec3, target: Vec3, materials: &[Material]) {
        let camera = Camera::look_at(eye, target, Vec3::Y, sv.fov, 0.01, 100.0).unwrap();
        let sv_meshes = &sv.meshes;
        let mut draws: Vec<Draw<'_>> = self
            .meshes
            .iter()
            .zip(self.instances.iter_mut())
            .zip(sv_meshes)
            .map(|((m, i), b)| Draw::new(m, i).with_material(&materials[b.material]))
            .collect();
        let until = Instant::now() + Duration::from_secs(20);
        while self
            .r
            .try_render_view(&self.target, &camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < until, "G1 frame admission");
            self.r.test_drain("G1 admission");
        }
    }
    /// Exposed HDR back to radiance (cd/m^2), RGB per pixel.
    fn radiance(&mut self, sv: &Sv) -> Vec<[f32; 3]> {
        self.r.test_drain("G1 HDR admission");
        let mut capture = self.r.try_capture_hdr().unwrap().unwrap();
        self.r.test_drain("G1 HDR copy");
        let half: Vec<f32> = capture
            .try_read()
            .unwrap()
            .unwrap()
            .chunks_exact(2)
            .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
            .collect();
        let scale = (-sv.exposure).exp2();
        half.chunks_exact(4)
            .map(|p| [p[0] * scale, p[1] * scale, p[2] * scale])
            .collect()
    }
    /// Placement + 1 per pixel from the shared hit records (0 = miss).
    fn placements(&mut self) -> Vec<u32> {
        self.r.test_drain("G1 hit admission");
        let r = &self.r;
        let hit = &r
            .primary
            .as_ref()
            .unwrap()
            .targets
            .as_ref()
            .unwrap()
            .hit
            .texture;
        let size = hit.size();
        let row = (size.width * 16).next_multiple_of(256);
        let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("G1 hits"),
            size: u64::from(row * size.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            hit.as_image_copy(),
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
        r.device.poll(WAIT).unwrap();
        let data = slice.get_mapped_range().unwrap();
        let mut out = Vec::with_capacity((size.width * size.height) as usize);
        for y in 0..size.height {
            for x in 0..size.width {
                let at = (y * row + x * 16) as usize;
                out.push(u32::from_le_bytes(data[at..at + 4].try_into().unwrap()));
            }
        }
        out
    }
}

/// Per-pixel populations: by mesh (scene.json's lists) and ground near each feature.
/// The open-sky luminance of a frame: the 90th percentile of the reference over the
/// open ground. Crevice and over-occlusion thresholds are fractions of it.
fn open_luminance(sv: &Sv, sets: &[Vec<usize>], reference: &reference::Image) -> f64 {
    let open = sv
        .populations
        .iter()
        .position(|(n, _)| n == "open")
        .unwrap();
    let mut v: Vec<f64> = sets[open]
        .iter()
        .map(|&i| luminance(reference.mean[i]))
        .collect();
    v.sort_by(f64::total_cmp);
    v.get(v.len() * 9 / 10).copied().unwrap_or(1.0)
}
fn labels(sv: &Sv, placements: &[u32], eye: Vec3, target: Vec3) -> Vec<Vec<usize>> {
    let names: Vec<String> = sv.populations.iter().map(|(n, _)| n.clone()).collect();
    let mut sets = vec![vec![]; names.len() * 2 + 1];
    let camera = Camera::look_at(eye, target, Vec3::Y, sv.fov, 0.01, 100.0).unwrap();
    let inverse = camera.matrix(sv.size[0], sv.size[1]).inverse();
    let ground = sv.meshes.iter().position(|m| m.name == "ground");
    let bounds: Vec<Option<([f32; 3], [f32; 3])>> = sv
        .populations
        .iter()
        .map(|(_, list)| {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for m in sv.meshes.iter().filter(|m| list.contains(&m.name)) {
                for p in &m.positions {
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
            }
            lo[0].is_finite().then_some((lo, hi))
        })
        .collect();
    for (i, &p) in placements.iter().enumerate() {
        if p == 0 {
            continue;
        }
        let mesh = &sv.meshes[p as usize - 1];
        sets[names.len() * 2].push(i);
        for (k, (_, list)) in sv.populations.iter().enumerate() {
            if list.contains(&mesh.name) {
                sets[k].push(i);
            }
        }
        if Some(p as usize - 1) == ground {
            // Ground top is y = 0: intersect the pixel ray with it.
            let (x, y) = (
                (i as u32 % sv.size[0]) as f32,
                (i as u32 / sv.size[0]) as f32,
            );
            let ndc = Vec2::new(
                (x + 0.5) / sv.size[0] as f32 * 2.0 - 1.0,
                1.0 - (y + 0.5) / sv.size[1] as f32 * 2.0,
            );
            let near = inverse.project_point3(ndc.extend(1.0));
            let far = inverse.project_point3(ndc.extend(0.0));
            let d = far - near;
            if d.y.abs() < 1e-9 {
                continue;
            }
            let w = near + d * (-near.y / d.y);
            for (k, b) in bounds.iter().enumerate() {
                if names[k] == "open" {
                    continue;
                }
                if let Some((lo, hi)) = b
                    && w.x > lo[0] - 0.3
                    && w.x < hi[0] + 0.3
                    && w.z > lo[2] - 0.3
                    && w.z < hi[2] + 0.3
                {
                    sets[names.len() + k].push(i);
                }
            }
        }
    }
    sets
}
fn population_names(sv: &Sv) -> Vec<String> {
    let mut n: Vec<String> = sv.populations.iter().map(|(n, _)| n.clone()).collect();
    n.extend(
        sv.populations
            .iter()
            .map(|(n, _)| format!("ground-near-{n}")),
    );
    n.push("frame".into());
    n
}
fn luminance(c: [f32; 3]) -> f64 {
    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
}
/// Relative error statistics over `pixels` with a resolved reference.
fn score(ours: &[[f32; 3]], reference: &reference::Image, open: f64, pixels: &[usize]) -> Json {
    let (mut n, mut sq, mut bias, mut crevice, mut crevice_sq, mut crevice_bias) =
        (0usize, 0.0, 0.0, 0usize, 0.0, 0.0);
    let (mut open_n, mut over) = (0usize, 0usize);
    let mut errors = vec![];
    for &i in pixels {
        let r = luminance(reference.mean[i]);
        let se = luminance(reference.standard_error[i]);
        if r <= 1e-6 || se > 0.02 * r {
            continue;
        }
        let e = (luminance(ours[i]) - r) / r;
        n += 1;
        sq += e * e;
        bias += e;
        errors.push(e.abs());
        if r < 0.7 * open {
            crevice += 1;
            crevice_sq += e * e;
            crevice_bias += e;
        }
        if r >= 0.95 * open {
            open_n += 1;
            if e < -0.05 {
                over += 1;
            }
        }
    }
    errors.sort_by(f64::total_cmp);
    let q = |p: f64| {
        errors
            .get(((errors.len() as f64 - 1.0) * p) as usize)
            .copied()
    };
    let div = |a: f64, b: usize| {
        if b == 0 {
            Json::Null
        } else {
            json!(a / b as f64)
        }
    };
    json!({
        "pixels": n, "rms": div(sq, n).as_f64().map(f64::sqrt), "bias": div(bias, n),
        "p95": q(0.95), "max": q(1.0),
        "crevice_pixels": crevice, "crevice_rms": div(crevice_sq, crevice).as_f64().map(f64::sqrt),
        "crevice_bias": div(crevice_bias, crevice),
        "open_pixels": open_n, "over_occluded_open": over,
    })
}
/// Frame-to-frame change of the residual against per-frame references (bench-motion's
/// residual_change_rms), relative to the reference.
fn flicker(ours: &[Vec<[f32; 3]>], refs: &[reference::Image], pixels: &[usize]) -> Json {
    let mut changes = vec![];
    for t in 1..ours.len() {
        let (mut sq, mut n) = (0.0, 0usize);
        for &i in pixels {
            let (r0, r1) = (luminance(refs[t - 1].mean[i]), luminance(refs[t].mean[i]));
            if r0 <= 1e-6 || r1 <= 1e-6 {
                continue;
            }
            let e0 = (luminance(ours[t - 1][i]) - r0) / r0;
            let e1 = (luminance(ours[t][i]) - r1) / r1;
            sq += (e1 - e0) * (e1 - e0);
            n += 1;
        }
        changes.push((sq / n.max(1) as f64).sqrt());
    }
    let mean = changes.iter().sum::<f64>() / changes.len().max(1) as f64;
    json!({"mean": mean, "max": changes.iter().copied().fold(0.0, f64::max), "frames": changes})
}
fn png(path: &Path, size: [u32; 2], pixels: impl Iterator<Item = [f32; 3]>, scale: f32) {
    let bytes: Vec<u8> = pixels
        .flat_map(|c| {
            c.map(|v| {
                let x = (v * scale).max(0.0);
                ((x / (1.0 + x)).powf(1.0 / 2.2) * 255.0 + 0.5) as u8
            })
        })
        .collect();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}
fn hash_image(pixels: &[[f32; 3]]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for p in pixels {
        for v in p {
            h.update(v.to_le_bytes());
        }
    }
    format!("{:x}", h.finalize())[..16].to_string()
}
fn profile_json(r: &mut Renderer) -> Json {
    let p = r.indirect_profile().unwrap().unwrap_or_default();
    let c = r.indirect_counters().unwrap().unwrap_or_default();
    json!({
        "us": {"probe": p.probe, "claim": p.claim, "resolve": p.resolve, "scan": p.scan,
               "args": p.args, "trace": p.trace, "integrate": p.integrate, "total": p.total()},
        "counters": {"misses": c.misses, "traces": c.traces, "integrates": c.integrates,
                     "created": c.created, "orphans": c.orphans, "invalidated": c.invalidated,
                     "refreshed": c.refreshed, "full": c.full, "miss_overflow": c.miss_overflow,
                     "local_overflow": c.local_overflow},
    })
}
/// The shading pass and the whole span of the most recent completed frame, ms.
fn frame_gpu(r: &mut Renderer) -> Json {
    r.test_drain("G1 timing");
    let events = r.poll().unwrap();
    events.gpu.last().map_or(Json::Null, |g| {
        json!({"shading_ms": g.mesh_pass.as_secs_f64() * 1e3, "span_ms": g.render_span.as_secs_f64() * 1e3})
    })
}

#[test]
#[ignore = "GPU; RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK, RENDERLAB_M7_G1_BENCH, RENDERLAB_M7_G1_OUT"]
fn gpu_gi_sky_visibility_sv() {
    let bench = PathBuf::from(env("RENDERLAB_M7_G1_BENCH").expect("RENDERLAB_M7_G1_BENCH"));
    let out = PathBuf::from(env("RENDERLAB_M7_G1_OUT").expect("RENDERLAB_M7_G1_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let spp: u32 = env("RENDERLAB_M7_G1_SPP").map_or(4096, |s| s.parse().unwrap());
    let quick = env("RENDERLAB_M7_G1_QUICK").as_deref() == Some("1");
    let sv = load(&bench);
    let names = population_names(&sv);
    let start = Instant::now();
    let rt = rt_device();
    // References: sky only (no bounce), pixel-integrated, per pose; and per motion frame.
    let settings = |samples| reference::Settings {
        samples,
        max_depth: 2,
        ..Default::default()
    };
    let poses: Vec<_> = if quick {
        sv.poses[..1].to_vec()
    } else {
        sv.poses.clone()
    };
    let mut references = vec![];
    for (name, eye, target) in &poses {
        let image = reference::render(
            &rt.0,
            &rt.1,
            &reference_scene(&sv, &[], 0.0),
            &reference_view(&sv, *eye, *target),
            &settings(spp),
        )
        .unwrap();
        assert_eq!(image.invalid_pixels, 0);
        png(
            &out.join(format!("{name}-reference.png")),
            sv.size,
            image.mean.iter().copied(),
            (sv.exposure).exp2(),
        );
        references.push(image);
    }
    let standard_refs: Vec<_> = poses
        .iter()
        .map(|(_, eye, target)| {
            reference::render(
                &rt.0,
                &rt.1,
                &reference_scene(&sv, &[], 1.0),
                &reference_view(&sv, *eye, *target),
                &settings(spp),
            )
            .unwrap()
        })
        .collect();
    // Motion: a 2 cm-per-frame pan along x at the gaps pose, 9 frames.
    let (motion_eye, motion_target) = (sv.poses.last().unwrap().1, sv.poses.last().unwrap().2);
    let pan = |t: usize| Vec3::new(0.02 * t as f32, 0.0, 0.0);
    let motion_frames = if quick { 3 } else { 9 };
    let motion_refs: Vec<_> = (0..motion_frames)
        .map(|t| {
            reference::render(
                &rt.0,
                &rt.1,
                &reference_scene(&sv, &[], 0.0),
                &reference_view(&sv, motion_eye + pan(t), motion_target + pan(t)),
                &settings(spp / 4),
            )
            .unwrap()
        })
        .collect();
    // Caster: the contact sphere moves +1 cm per frame in x for 8 frames.
    let sphere = sv
        .meshes
        .iter()
        .position(|m| m.name == "contact-sphere")
        .unwrap();
    let caster = |t: usize| Mat4::from_translation(Vec3::new(0.01 * t.min(8) as f32, 0.0, 0.0));
    let caster_ref = reference::render(
        &rt.0,
        &rt.1,
        &reference_scene(&sv, &[(sphere, caster(8))], 0.0),
        &reference_view(&sv, poses[0].1, poses[0].2),
        &settings(spp),
    )
    .unwrap();
    let reference_seconds = start.elapsed().as_secs_f64();
    eprintln!("G1 references in {reference_seconds:.1} s");

    let mut results = json!({
        "bench": "SV", "scene_sha256": sv.sha, "reference_spp": spp, "reference": "renderlab::reference, max_depth 2 (sky only)",
        "reference_seconds": reference_seconds, "populations": names, "paths": {},
    });
    let paths = if quick {
        vec![Path_::Rt, Path_::NonRt]
    } else {
        vec![Path_::Rt, Path_::RtSoftware, Path_::NonRt]
    };
    let mut rt = Some(rt);
    for path in paths {
        let device = match path {
            Path_::NonRt => no_rt_device(),
            _ => {
                let d = rt.take().unwrap_or_else(rt_device);
                (d.0.clone(), d.1.clone())
            }
        };
        let mut rig = rig(&sv, path, device);
        let label = format!("{path:?}").to_lowercase();
        let [standard, materials, toon] = std::mem::take(&mut rig.materials);
        let mut path_json = json!({});
        let base = IndirectSettings {
            enabled: true,
            ..Default::default()
        };
        // Configurations: GI off (the pre-M7 image), then the W3 lookups and cell sizes.
        let mut configs = vec![
            ("off", IndirectSettings::default()),
            ("trilinear-4px", base),
        ];
        if !quick {
            configs.extend([
                (
                    "rotated-4px",
                    IndirectSettings {
                        rotate_directions: true,
                        ..base
                    },
                ),
                (
                    "k128-4px",
                    IndirectSettings {
                        directions: 128,
                        ..base
                    },
                ),
                (
                    "nearest-4px",
                    IndirectSettings {
                        lookup: IndirectLookup::Nearest,
                        ..base
                    },
                ),
                (
                    "trilinear-2px",
                    IndirectSettings {
                        cell_pixels: 2.0,
                        ..base
                    },
                ),
                (
                    "trilinear-8px",
                    IndirectSettings {
                        cell_pixels: 8.0,
                        ..base
                    },
                ),
                (
                    "slots-4",
                    IndirectSettings {
                        direction_phases: 4,
                        ..base
                    },
                ),
            ]);
        }
        for (config, s) in &configs {
            let mut config_json = json!({});
            for (((pose, eye, target), reference), standard_ref) in
                poses.iter().zip(&references).zip(&standard_refs)
            {
                rig.settings(IndirectSettings {
                    enabled: false,
                    ..*s
                });
                rig.settings(*s);
                rig.r.reset_indirect();
                // Frame 1 creates every visible cell (a cut); frame 2 is the static view.
                rig.frame(&sv, *eye, *target, &materials);
                let cut = profile_json(&mut rig.r);
                let cut_gpu = frame_gpu(&mut rig.r);
                let first = rig.radiance(&sv);
                rig.frame(&sv, *eye, *target, &materials);
                let still = profile_json(&mut rig.r);
                let still_gpu = frame_gpu(&mut rig.r);
                let image = rig.radiance(&sv);
                let placements = rig.placements();
                let sets = labels(&sv, &placements, *eye, *target);
                let open = open_luminance(&sv, &sets, reference);
                let scores: Json = names
                    .iter()
                    .zip(&sets)
                    .map(|(n, px)| (n.clone(), score(&image, reference, open, px)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                let first_scores = score(&first, reference, open, sets.last().unwrap());
                png(
                    &out.join(format!("{label}-{config}-{pose}.png")),
                    sv.size,
                    image.iter().copied(),
                    (sv.exposure).exp2(),
                );
                if *config == "trilinear-4px" || *config == "off" {
                    // |ours - reference| x 8, for inspection.
                    png(
                        &out.join(format!("{label}-{config}-{pose}-diff8.png")),
                        sv.size,
                        image
                            .iter()
                            .zip(&reference.mean)
                            .map(|(a, b)| std::array::from_fn(|k| (a[k] - b[k]).abs() * 8.0)),
                        (sv.exposure).exp2(),
                    );
                }
                config_json[pose] = json!({
                    "scores": scores, "first_frame": first_scores, "first_equals_second": hash_image(&first) == hash_image(&image),
                    "image_hash": hash_image(&image), "cut": cut, "cut_gpu": cut_gpu, "static": still, "static_gpu": still_gpu,
                });
                if *config == "trilinear-4px" || *config == "off" {
                    // The standard program's full sky image: diffuse from the cache, sky
                    // specular still unoccluded (R1 scope).
                    rig.frame(&sv, *eye, *target, &standard);
                    let full = rig.radiance(&sv);
                    let open = open_luminance(&sv, &sets, standard_ref);
                    png(
                        &out.join(format!("{label}-{config}-{pose}-standard.png")),
                        sv.size,
                        full.iter().copied(),
                        (sv.exposure).exp2(),
                    );
                    config_json[pose]["standard"] = names
                        .iter()
                        .zip(&sets)
                        .map(|(n, px)| (n.clone(), score(&full, standard_ref, open, px)))
                        .collect::<serde_json::Map<_, _>>()
                        .into();
                }
                eprintln!(
                    "G1 {label} {config} {pose}: frame rms {} crevice rms {} over {}",
                    scores["frame"]["rms"],
                    scores["frame"]["crevice_rms"],
                    scores["frame"]["over_occluded_open"]
                );
            }
            path_json[*config] = config_json;
        }
        // A program that never calls the cache runs no cache pass and draws the same bytes.
        {
            let (_, eye, target) = &poses[0];
            rig.settings(IndirectSettings::default());
            rig.frame(&sv, *eye, *target, &toon);
            let off = hash_image(&rig.radiance(&sv));
            rig.settings(base);
            let frames = rig.r.indirect_stats().frames;
            rig.frame(&sv, *eye, *target, &toon);
            let on = hash_image(&rig.radiance(&sv));
            path_json["non_caller"] = json!({"toon_image_equal": off == on,
                "cache_frames_added": rig.r.indirect_stats().frames - frames});
        }
        rig.settings(base);
        // Determinism: the table and image after the same sequence, in this process
        // (cross-process equality compares these hashes between runs).
        {
            let (_, eye, target) = &poses[0];
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let table = rig.r.indirect_table().unwrap().unwrap();
            let a = (hash_words(&table), hash_image(&rig.radiance(&sv)));
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let table = rig.r.indirect_table().unwrap().unwrap();
            let b = (hash_words(&table), hash_image(&rig.radiance(&sv)));
            path_json["determinism"] = json!({"table": a.0, "image": a.1, "repeat_equal": a == b});
        }
        // Motion: per-frame cost and flicker against per-frame references.
        for (config, s) in [
            ("trilinear-4px", base),
            (
                "nearest-4px",
                IndirectSettings {
                    lookup: IndirectLookup::Nearest,
                    ..base
                },
            ),
        ] {
            if quick && config != "trilinear-4px" {
                continue;
            }
            rig.settings(s);
            rig.r.reset_indirect();
            let mut frames = vec![];
            let mut costs = vec![];
            for t in 0..motion_frames {
                rig.frame(&sv, motion_eye + pan(t), motion_target + pan(t), &materials);
                costs
                    .push(json!({"cache": profile_json(&mut rig.r), "gpu": frame_gpu(&mut rig.r)}));
                frames.push(rig.radiance(&sv));
            }
            let placements = rig.placements();
            let all: Vec<usize> = placements
                .iter()
                .enumerate()
                .filter(|(_, p)| **p != 0)
                .map(|(i, _)| i)
                .collect();
            path_json[format!("motion-{config}")] =
                json!({"flicker": flicker(&frames, &motion_refs, &all), "frames": costs});
        }
        // Caster: invalidation spikes, then the settled image against a fresh cache.
        {
            rig.settings(base);
            let (_, eye, target) = &poses[0];
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            let mut costs = vec![];
            for t in 1..=10 {
                rig.instances[sphere].set(0, caster(t)).unwrap();
                rig.frame(&sv, *eye, *target, &materials);
                costs
                    .push(json!({"cache": profile_json(&mut rig.r), "gpu": frame_gpu(&mut rig.r)}));
            }
            let rolled = rig.radiance(&sv);
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let fresh = rig.radiance(&sv);
            let max_diff = rolled
                .iter()
                .zip(&fresh)
                .map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max))
                .fold(0.0f32, f32::max);
            let placements = rig.placements();
            let sets = labels(&sv, &placements, *eye, *target);
            let open = open_luminance(&sv, &sets, &caster_ref);
            let all = sets.last().unwrap().clone();
            path_json["caster"] = json!({
                "frames": costs, "rolled_equals_fresh": hash_image(&rolled) == hash_image(&fresh),
                "rolled_max_abs_difference": max_diff,
                "rolled_score": score(&rolled, &caster_ref, open, &all),
            });
            // Rotation alone (exact invalidation off): staleness by frames since the stop.
            if !quick {
                rig.settings(IndirectSettings {
                    rotation_period: 8,
                    invalidate_casters: false,
                    ..base
                });
                rig.r.reset_indirect();
                rig.instances[sphere].set(0, caster(0)).unwrap();
                rig.frame(&sv, *eye, *target, &materials);
                rig.instances[sphere].set(0, caster(8)).unwrap();
                let mut stale = vec![];
                for age in 0..10 {
                    rig.frame(&sv, *eye, *target, &materials);
                    let image = rig.radiance(&sv);
                    let d = image.iter().zip(&fresh).filter(|(a, b)| a != b).count();
                    stale.push(json!({"age": age, "pixels_differing_from_fresh": d,
                        "score": score(&image, &caster_ref, open, &all)}));
                }
                path_json["rotation-staleness"] = json!(stale);
                rig.settings(base);
            }
            rig.instances[sphere].set(0, Mat4::IDENTITY).unwrap();
        }
        let stats = rig.r.indirect_stats();
        path_json["memory"] = json!({"table_bytes": stats.table_bytes, "work_bytes": stats.work_bytes,
            "geometry_bytes": stats.geometry_bytes, "software": stats.software});
        results["paths"][&label] = path_json;
        std::fs::write(
            out.join("g1-sv.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
    results["seconds"] = json!(start.elapsed().as_secs_f64());
    std::fs::write(
        out.join("g1-sv.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
fn hash_words(words: &[u32]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for w in words {
        h.update(w.to_le_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}
