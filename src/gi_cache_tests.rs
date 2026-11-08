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
                uvs: vec![],
                tangents: vec![],
                colors: vec![],
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
                base_color_texture: None,
                metallic_roughness_texture: None,
                normal_texture: None,
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
        textures: vec![],
        environment: None,
        camera_background: None,
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
    no_rt_device_with(false)
}
/// A device without ray queries. `large` raises the buffer and storage-binding limits to
/// the adapter's own: the quarry's full-detail software geometry is one 401 MB buffer.
fn no_rt_device_with(large: bool) -> (wgpu::Device, wgpu::Queue) {
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
        required_limits: {
            let mut limits = crate::visibility_limits(wgpu::Limits::default());
            if large {
                let a = adapter.limits();
                limits.max_buffer_size = a.max_buffer_size;
                limits.max_storage_buffer_binding_size = a.max_storage_buffer_binding_size;
            }
            limits
        },
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
/// The G2 gate program: the G1 gate plus a Lambert `scatter`, so its surfaces reflect bounce
/// light into the cache. Its reference is the path tracer with bounces and specular 0.
const LAMBERT_BOUNCE: &str = "struct Surface { base:vec3<f32>, n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(input.material.base_color.rgb*input.color.rgb,n);
}
fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return s.base*(max(dot(s.n,wi),0.0)/PI);
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
    /// Per bench material: G2's Lambert gate with `scatter` (bounce light).
    bounce: Vec<Material>,
    _programs: [Program; 3],
    path: Path_,
}
fn rig(sv: &Sv, path: Path_, device: (wgpu::Device, wgpu::Queue)) -> Rig {
    let (device, queue) = device;
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off, anisotropy 1, the shipping default.
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
    let programs = [
        ("g1-lambert", LAMBERT),
        ("toon", TOON),
        ("g2-lambert", LAMBERT_BOUNCE),
    ]
    .map(|(label, source)| {
        r.try_create_program(&ProgramDescriptor { label, source })
            .unwrap()
    });
    let [standard, gate, toon, bounce] = [
        None,
        Some(&programs[0]),
        Some(&programs[1]),
        Some(&programs[2]),
    ]
    .map(|program| {
        sv.materials
            .iter()
            .map(|m| {
                loop {
                    let descriptor = MaterialDescriptor {
                        base_color: [m[0], m[1], m[2], 1.0],
                        metallic: m[3],
                        roughness: m[4],
                        double_sided: true,
                        program,
                        ..Default::default()
                    };
                    if let Some(material) = r.try_create_material(&descriptor).unwrap() {
                        break material;
                    }
                    r.test_drain("G1 material admission");
                }
            })
            .collect::<Vec<_>>()
    });
    r.set_indirect_profiling(true);
    Rig {
        r,
        target,
        meshes,
        instances,
        materials: [standard, gate, toon],
        bounce,
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
        capture_radiance(&mut self.r, sv.exposure)
    }
    fn read_texture(&self, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
        read_texture(&self.r, texture, texel)
    }
    /// Placement + 1 per pixel from the shared hit records (0 = miss), with bit 31 set
    /// on edge pixels: a 3x3 neighbour hits another placement or a triangle whose
    /// normal differs. The reference integrates each pixel's footprint and this frame
    /// samples its centre, so edges measure anti-aliasing, not lighting; they are
    /// scored as their own population. A pixel is covered only inside the traced region,
    /// in a tile whose coverage flag is set: the trace leaves uncovered tiles' records
    /// stale, which once counted another pose's records as surface pixels (G1's overview
    /// populations included about 323,000 sky pixels; docs/m7-g1b.md).
    fn placements(&mut self, sv: &Sv) -> Vec<u32> {
        self.r.test_drain("G1 hit admission");
        let primary = self.r.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let region = primary.scene;
        let (hit, tiles) = (targets.hit.texture.clone(), targets.tiles.texture.clone());
        let size = hit.size();
        let tiles_width = tiles.size().width as usize;
        let data = self.read_texture(&hit, 16);
        let tile_flags = self.read_texture(&tiles, 4);
        let [tile_w, tile_h] = crate::renderer::primary_ray_gpu::TRACE_TILE.map(|t| t as usize);
        let covered = |x: usize, y: usize| {
            let inside = (region[0] as usize..region[2] as usize).contains(&x)
                && (region[1] as usize..region[3] as usize).contains(&y);
            let t = ((y / tile_h) * tiles_width + x / tile_w) * 4;
            inside && u32::from_le_bytes(tile_flags[t..t + 4].try_into().unwrap()) != 0
        };
        let row = (size.width * 16) as usize;
        let (w, h) = (size.width as usize, size.height as usize);
        let mut out = Vec::with_capacity(w * h);
        let mut normals = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let at = y * row + x * 16;
                let word = |k: usize| {
                    u32::from_le_bytes(data[at + 4 * k..at + 4 * k + 4].try_into().unwrap())
                };
                let p = if covered(x, y) { word(0) } else { 0 };
                out.push(p);
                normals.push(if p == 0 {
                    Vec3::ZERO
                } else {
                    let m = &sv.meshes[p as usize - 1];
                    let t = word(1) as usize * 3;
                    let v = |k: usize| Vec3::from(m.positions[m.indices[t + k] as usize]);
                    (v(1) - v(0)).cross(v(2) - v(0)).normalize_or_zero()
                });
            }
        }
        let flags: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                (y.saturating_sub(1)..(y + 2).min(h)).any(|ny| {
                    (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| {
                        let j = ny * w + nx;
                        out[j] != out[i] || normals[j].dot(normals[i]) < 0.999
                    })
                })
            })
            .collect();
        out.iter()
            .zip(flags)
            .map(|(&p, e)| p | (u32::from(e && p != 0) << 31))
            .collect()
    }
}

/// Exposed HDR back to radiance (cd/m^2), RGB per pixel.
fn capture_radiance(r: &mut Renderer, exposure: f32) -> Vec<[f32; 3]> {
    r.test_drain("G1 HDR admission");
    let mut capture = r.try_capture_hdr().unwrap().unwrap();
    r.test_drain("G1 HDR copy");
    let half: Vec<f32> = capture
        .try_read()
        .unwrap()
        .unwrap()
        .chunks_exact(2)
        .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
        .collect();
    let scale = (-exposure).exp2();
    half.chunks_exact(4)
        .map(|p| [p[0] * scale, p[1] * scale, p[2] * scale])
        .collect()
}
/// A texture's bytes, rows packed to `texel` bytes per pixel.
fn read_texture(r: &Renderer, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
    let size = texture.size();
    let row = (size.width * texel).next_multiple_of(256);
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("G1 texture readback"),
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
    r.device.poll(WAIT).unwrap();
    let data = slice.get_mapped_range().unwrap();
    let width = (size.width * texel) as usize;
    data.chunks_exact(row as usize)
        .flat_map(|line| line[..width].to_vec())
        .collect()
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
    // Populations, ground near each, edges, then the frame (edges excluded).
    let mut sets = vec![vec![]; names.len() * 2 + 2];
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
    for (i, &word) in placements.iter().enumerate() {
        let p = word & 0x7fff_ffff;
        if p == 0 {
            continue;
        }
        if word >> 31 != 0 {
            sets[names.len() * 2].push(i);
            continue;
        }
        let mesh = &sv.meshes[p as usize - 1];
        sets[names.len() * 2 + 1].push(i);
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
    n.push("edges".into());
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
    let mut noise = 0.0;
    let mut plain_sq = 0.0;
    let mut errors = vec![];
    for &i in pixels {
        let r = luminance(reference.mean[i]);
        let se = luminance(reference.standard_error[i]);
        if r <= 1e-6 || se > 0.02 * r {
            continue;
        }
        // Relative to the reference, floored at 5% of the open sky: a nearly black
        // reference pixel must not turn a small absolute error into a huge one.
        let e = (luminance(ours[i]) - r) / r.max(0.05 * open);
        let plain = (luminance(ours[i]) - r) / r;
        plain_sq += plain * plain;
        noise += (se / r) * (se / r);
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
        "reference_noise_rms": div(noise, n).as_f64().map(f64::sqrt),
        "rms_unfloored": div(plain_sq, n).as_f64().map(f64::sqrt),
        "p99": q(0.99),
    })
}
/// Frame-to-frame change of the residual against per-frame references (bench-motion's
/// residual_change_rms), relative to the reference.
fn flicker(ours: &[Vec<[f32; 3]>], refs: &[reference::Image], pixels: &[usize]) -> Json {
    let mut lum: Vec<f64> = pixels.iter().map(|&i| luminance(refs[0].mean[i])).collect();
    lum.sort_by(f64::total_cmp);
    let floor = 0.05 * lum.get(lum.len() * 9 / 10).copied().unwrap_or(1.0);
    let mut changes = vec![];
    for t in 1..ours.len() {
        let (mut sq, mut n) = (0.0, 0usize);
        for &i in pixels {
            let (r0, r1) = (luminance(refs[t - 1].mean[i]), luminance(refs[t].mean[i]));
            if r0 <= 1e-6 || r1 <= 1e-6 {
                continue;
            }
            let e0 = (luminance(ours[t - 1][i]) - r0) / r0.max(floor);
            let e1 = (luminance(ours[t][i]) - r1) / r1.max(floor);
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
               "trace": p.trace(), "setup": p.setup, "rays": p.rays, "finish": p.finish,
               "integrate": p.integrate, "total": p.total()},
        "counters": {"misses": c.misses, "traces": c.traces, "integrates": c.integrates,
                     "created": c.created, "orphans": c.orphans, "invalidated": c.invalidated,
                     "refreshed": c.refreshed, "full": c.full, "miss_overflow": c.miss_overflow, "lost": c.lost, "rays": c.rays,
                     "ray_items": c.ray_items, "local_overflow": c.local_overflow,
                     "orphans_no_surface": c.orphans_no_surface, "orphans_inside": c.orphans_inside,
                     "splits": c.splits, "probe": probe_json(&c.probe)},
    })
}
fn probe_json(p: &crate::ProbeCounters) -> Json {
    json!({"tiles": p.tiles, "pixels": p.pixels, "inserts": p.inserts, "steps": p.steps,
        "cells": p.cells, "parents": p.parents, "refined": p.refined, "finds": p.finds,
        "found": p.found, "walks": p.walks, "refine_tiles": p.refine_tiles,
        "max_cells": p.max_cells, "tiles_by_cells": p.tiles_by_cells})
}
/// G1d: the per-stage cost of `frames` camera-moving frames (`render(ctx, t)` draws frame t;
/// `renderer(ctx)` is its renderer) with the cache as currently set, in `visits + 2` passes
/// over the same frames:
/// - counters: each frame's kernel counters, read back after it (blocking, so not timed);
/// - profiled: back to back, the per-pass times from the timestamp ring (about 0.05 ms a
///   frame of timestamps);
/// - `visits` unprofiled: back to back, the frame span and the shading pass.
///
/// p50s skip each pass's first two frames (the jump back to the first frame). Leaves
/// profiling on.
fn motion_breakdown<C>(
    ctx: &mut C,
    renderer: impl Fn(&mut C) -> &mut Renderer,
    frames: usize,
    visits: usize,
    mut render: impl FnMut(&mut C, usize),
) -> Json {
    let tail = |v: Vec<f64>| quantiles(v.into_iter().skip(2).collect());
    let gpu_json = |gpu: &[[f64; 2]]| {
        json!({"span_ms": tail(gpu.iter().map(|g| g[0]).collect()),
            "shading_ms": tail(gpu.iter().map(|g| g[1]).collect())})
    };
    renderer(ctx).set_indirect_profiling(true);
    // Counters, frame by frame.
    let mut counters = vec![];
    renderer(ctx).test_drain("G1d breakdown");
    renderer(ctx).poll().unwrap();
    for t in 0..frames {
        let cache_frames = renderer(ctx).indirect_stats().frames;
        render(ctx, t);
        let r = renderer(ctx);
        let ran = r.indirect_stats().frames > cache_frames;
        let mut row = profile_json(r)["counters"].clone();
        row["ran"] = json!(ran);
        counters.push(row);
    }
    // Profiled, back to back.
    let cache_before = renderer(ctx).indirect_stats().frames;
    let gpu = breakdown_frames(ctx, &renderer, frames, &mut render);
    let r = renderer(ctx);
    let cache_frames = (r.indirect_stats().frames - cache_before) as usize;
    let profiles = r.indirect_profiles().unwrap();
    let profiles: Vec<IndirectProfile> =
        profiles[profiles.len().saturating_sub(cache_frames.min(64))..].to_vec();
    let pass = |f: fn(&IndirectProfile) -> f64| tail(profiles.iter().map(f).collect());
    let profiled = json!({"gpu": gpu_json(&gpu), "cache_frames": cache_frames, "cache_us": {
        "total": pass(|p| p.total()), "probe": pass(|p| p.probe), "claim": pass(|p| p.claim),
        "resolve": pass(|p| p.resolve), "scan": pass(|p| p.scan), "setup": pass(|p| p.setup),
        "rays": pass(|p| p.rays), "finish": pass(|p| p.finish), "integrate": pass(|p| p.integrate)}});
    // Unprofiled visits.
    renderer(ctx).set_indirect_profiling(false);
    let timed: Vec<Json> = (0..visits)
        .map(|_| gpu_json(&breakdown_frames(ctx, &renderer, frames, &mut render)))
        .collect();
    renderer(ctx).set_indirect_profiling(true);
    json!({"frames": frames, "counters": counters, "profiled": profiled, "timed": timed})
}
/// `frames` frames back to back, then each one's GPU span and shading pass, ms.
fn breakdown_frames<C>(
    ctx: &mut C,
    renderer: &impl Fn(&mut C) -> &mut Renderer,
    frames: usize,
    render: &mut impl FnMut(&mut C, usize),
) -> Vec<[f64; 2]> {
    renderer(ctx).test_drain("G1d breakdown");
    renderer(ctx).poll().unwrap();
    for t in 0..frames {
        render(ctx, t);
    }
    let r = renderer(ctx);
    r.test_drain("G1d breakdown frames");
    let gpu = r.poll().unwrap().gpu;
    gpu.iter()
        .rev()
        .take(frames)
        .rev()
        .map(|g| {
            [
                g.render_span.as_secs_f64() * 1e3,
                g.mesh_pass.as_secs_f64() * 1e3,
            ]
        })
        .collect()
}
/// The shading pass and the whole span of the most recent completed frame, ms.
fn frame_gpu(r: &mut Renderer) -> Json {
    r.test_drain("G1 timing");
    let events = r.poll().unwrap();
    events.gpu.last().map_or(Json::Null, |g| {
        json!({"shading_ms": g.mesh_pass.as_secs_f64() * 1e3, "span_ms": g.render_span.as_secs_f64() * 1e3})
    })
}

/// A reference image, rendered once and kept as f32 mean and standard error in
/// RENDERLAB_M7_G1_REFS (default E:/renderlab-work/m7-g1/refs) under a key naming the
/// scene, view, sample count and specular scale, so later short leases reuse it
/// instead of path tracing again. Raw caches are deleted once G1 is summarized.
fn cached_reference(
    device: &(wgpu::Device, wgpu::Queue),
    sv: &Sv,
    moved: &[(usize, Mat4)],
    specular: f32,
    eye: Vec3,
    target: Vec3,
    settings: &reference::Settings,
) -> reference::Image {
    use sha2::Digest;
    let dir = PathBuf::from(
        env("RENDERLAB_M7_G1_REFS").unwrap_or_else(|| "E:/renderlab-work/m7-g1/refs".into()),
    );
    let mut h = sha2::Sha256::new();
    h.update(sv.sha.as_bytes());
    for v in [eye, target] {
        h.update(v.to_array().map(f32::to_le_bytes).concat());
    }
    for (i, m) in moved {
        h.update(i.to_le_bytes());
        h.update(m.to_cols_array().map(f32::to_le_bytes).concat());
    }
    h.update(specular.to_le_bytes());
    h.update(settings.samples.to_le_bytes());
    h.update(settings.max_depth.to_le_bytes());
    h.update(settings.seed.to_le_bytes());
    let path = dir.join(format!("{:x}.f32", h.finalize()));
    let pixels = (sv.size[0] * sv.size[1]) as usize;
    let raw =
        |v: &[[f32; 3]]| -> Vec<u8> { v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect() };
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() == pixels * 24
    {
        let decode = |b: &[u8]| -> Vec<[f32; 3]> {
            b.chunks_exact(12)
                .map(|c| {
                    std::array::from_fn(|k| {
                        f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap())
                    })
                })
                .collect()
        };
        return reference::Image {
            width: sv.size[0],
            height: sv.size[1],
            samples: settings.samples,
            mean: decode(&bytes[..pixels * 12]),
            standard_error: decode(&bytes[pixels * 12..]),
            invalid_pixels: 0,
            halves: [vec![], vec![]],
            geometry: vec![],
        };
    }
    let image = reference::render(
        &device.0,
        &device.1,
        &reference_scene(sv, moved, specular),
        &reference_view(sv, eye, target),
        settings,
    )
    .unwrap();
    assert_eq!(image.invalid_pixels, 0);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &path,
        [raw(&image.mean), raw(&image.standard_error)].concat(),
    )
    .unwrap();
    image
}
fn quantiles(mut v: Vec<f64>) -> Json {
    if v.is_empty() {
        return Json::Null;
    }
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    json!({"n": v.len(), "p50": q(0.5), "p95": q(0.95), "p99": q(0.99), "max": q(1.0)})
}
/// One continuous sequence: frames back to back with no readback between them, then
/// the renderer's per-frame GPU spans and the cache's per-pass ring.
fn sequence(
    rig: &mut Rig,
    sv: &Sv,
    materials: &[Material],
    frames: usize,
    mut step: impl FnMut(&mut Rig, usize) -> (Vec3, Vec3),
) -> Json {
    let mut samples = vec![];
    let idle = rig.r.indirect_stats().idle_frames;
    let profiled = rig.r.indirect_stats().frames;
    for t in 0..frames {
        let (eye, target) = step(rig, t);
        rig.frame(sv, eye, target, materials);
        samples.extend(rig.r.poll().unwrap().gpu);
    }
    rig.r.test_drain("G1 sequence");
    samples.extend(rig.r.poll().unwrap().gpu);
    let cache_frames = (rig.r.indirect_stats().frames - profiled) as usize;
    let profiles = rig.r.indirect_profiles().unwrap();
    let profiles = &profiles[profiles.len().saturating_sub(cache_frames.min(64))..];
    let pass = |f: fn(&IndirectProfile) -> f64| quantiles(profiles.iter().map(f).collect());
    // The first frames after a setting change or reset are reported apart.
    let tail = |v: Vec<f64>| quantiles(v.into_iter().skip(2).collect());
    json!({
        "frames": frames, "cache_frames": cache_frames,
        "idle_frames": rig.r.indirect_stats().idle_frames - idle,
        "span_ms": tail(samples.iter().map(|g| g.render_span.as_secs_f64() * 1e3).collect()),
        "shading_ms": tail(samples.iter().map(|g| g.mesh_pass.as_secs_f64() * 1e3).collect()),
        "cache_us": {
            "total": pass(|p| p.total()), "probe": pass(|p| p.probe), "claim": pass(|p| p.claim),
            "resolve": pass(|p| p.resolve), "scan": pass(|p| p.scan), "trace": pass(|p| p.trace()),
            "setup": pass(|p| p.setup), "rays": pass(|p| p.rays), "finish": pass(|p| p.finish),
            "integrate": pass(|p| p.integrate),
        },
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
    // RENDERLAB_M7_G1_SECTIONS: a comma list of quality, gates, motion, caster and timing
    // (all by default), so a quiet cost window runs timing alone.
    let sections = env("RENDERLAB_M7_G1_SECTIONS");
    let section = |name: &str| {
        sections
            .as_deref()
            .is_none_or(|w| w.split(',').any(|x| x.trim() == name))
    };
    // RENDERLAB_M7_G1_VISITS: interleaved timing visits per configuration (default 1).
    let visits: usize = env("RENDERLAB_M7_G1_VISITS").map_or(1, |s| s.parse().unwrap());
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
    // Cost-only runs (timing, breakdown, profile) need no reference (G1d).
    let needs_refs = ["quality", "gates", "motion", "caster"]
        .iter()
        .any(|s| section(s));
    let mut references = vec![];
    for (name, eye, target) in poses.iter().filter(|_| needs_refs) {
        let image = cached_reference(&rt, &sv, &[], 0.0, *eye, *target, &settings(spp));
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
        .filter(|_| needs_refs)
        .map(|(_, eye, target)| cached_reference(&rt, &sv, &[], 1.0, *eye, *target, &settings(spp)))
        .collect();
    // Motion: a 2 cm-per-frame pan along x at the gaps pose, 9 frames.
    let (motion_eye, motion_target) = (sv.poses.last().unwrap().1, sv.poses.last().unwrap().2);
    let pan = |t: usize| Vec3::new(0.02 * t as f32, 0.0, 0.0);
    let motion_frames = if quick { 3 } else { 9 };
    let motion_refs: Vec<_> = (0..motion_frames)
        .filter(|_| needs_refs)
        .map(|t| {
            cached_reference(
                &rt,
                &sv,
                &[],
                0.0,
                motion_eye + pan(t),
                motion_target + pan(t),
                &settings(spp),
            )
        })
        .collect();
    // Caster: the contact sphere moves +1 cm per frame in x for 8 frames.
    let sphere = sv
        .meshes
        .iter()
        .position(|m| m.name == "contact-sphere")
        .unwrap();
    let caster = |t: usize| Mat4::from_translation(Vec3::new(0.01 * t.min(8) as f32, 0.0, 0.0));
    let caster_ref = needs_refs.then(|| {
        cached_reference(
            &rt,
            &sv,
            &[(sphere, caster(8))],
            0.0,
            poses[0].1,
            poses[0].2,
            &settings(spp),
        )
    });
    // G2: references with bounce light (the constant sky's light after any number of
    // bounces off the 0.5 Lambert surfaces, specular 0), per pose and per frame of a 5-frame
    // pan at the gaps pose.
    let bounce_settings = reference::Settings {
        samples: spp,
        max_depth: 10,
        ..Default::default()
    };
    let bounce_refs: Vec<_> = poses
        .iter()
        .filter(|_| section("bounce"))
        .map(|(_, eye, target)| {
            cached_reference(&rt, &sv, &[], 0.0, *eye, *target, &bounce_settings)
        })
        .collect();
    let bounce_motion: Vec<_> = (0..5)
        .filter(|_| section("bounce"))
        .map(|t| {
            cached_reference(
                &rt,
                &sv,
                &[],
                0.0,
                motion_eye + pan(t),
                motion_target + pan(t),
                &bounce_settings,
            )
        })
        .collect();
    let reference_seconds = start.elapsed().as_secs_f64();
    eprintln!("G1 references in {reference_seconds:.1} s");

    let mut results = json!({
        "bench": "SV", "scene_sha256": sv.sha, "reference_spp": spp, "reference": "renderlab::reference, max_depth 2 (sky only)",
        "reference_seconds": reference_seconds, "populations": names, "paths": {},
    });
    // RENDERLAB_M7_G1_PATHS: a comma list of rt, rtsoftware and nonrt (short leases
    // run one path each).
    let paths: Vec<Path_> = env("RENDERLAB_M7_G1_PATHS")
        .unwrap_or_else(|| "rt,rtsoftware,nonrt".into())
        .split(',')
        .map(|p| match p.trim() {
            "rt" => Path_::Rt,
            "rtsoftware" => Path_::RtSoftware,
            "nonrt" => Path_::NonRt,
            other => panic!("unknown path {other}"),
        })
        .filter(|p| !quick || *p != Path_::RtSoftware)
        .collect();
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
        // RENDERLAB_M7_G1_PROFILING=0: no per-pass timestamps (frame spans only).
        let profiling = env("RENDERLAB_M7_G1_PROFILING").as_deref() != Some("0");
        rig.r.set_indirect_profiling(profiling);
        let label = format!("{path:?}").to_lowercase();
        let [standard, materials, toon] = std::mem::take(&mut rig.materials);
        let mut path_json = json!({});
        // Profile: RENDERLAB_M7_G1_PROFILE_FRAMES frames of the default settings with the
        // camera wrapping back and forth over 48 cm at 1 cm a frame, for an external GPU
        // trace (Nsight) to land in camera motion.
        // RENDERLAB_M7_G1_PROFILE_MODE: "camera" (default; after the first sweep every
        // cell exists, so it profiles the per-pixel passes), "rotation" (a fixed rotation
        // of period 16 re-traces a sixteenth of the cells every frame, so tracing runs) or
        // "standard" (camera, with the standard program instead of the Lambert gate).
        if section("profile") {
            let frames: usize =
                env("RENDERLAB_M7_G1_PROFILE_FRAMES").map_or(0, |s| s.parse().unwrap());
            let mode = env("RENDERLAB_M7_G1_PROFILE_MODE").unwrap_or_else(|| "camera".into());
            let (_, eye, target) = poses[0].clone();
            rig.settings(IndirectSettings {
                enabled: true,
                rotation_period: if mode == "rotation" { 16 } else { 0 },
                ..Default::default()
            });
            rig.r.reset_indirect();
            let program = if mode == "standard" {
                &standard
            } else {
                &materials
            };
            if frames > 0 {
                path_json["profile"] = sequence(&mut rig, &sv, program, frames, |_, t| {
                    let k = t % 96;
                    let d = Vec3::new(0.01 * k.min(96 - k) as f32, 0.0, 0.0);
                    (eye + d, target + d)
                });
            }
        }
        // Rotating-map diagnosis: shading and frame spans with and without a selected map,
        // with the map still or yawing 1 degree a frame, with the cache on and off (camera
        // still). Separates the map's own shading cost, rotation and the cache.
        if section("mapdiag") {
            let (_, eye, target) = poses[0].clone();
            let on = IndirectSettings {
                enabled: true,
                cut_phases: 1,
                ..Default::default()
            };
            let mut rows = json!({});
            // Before any map exists: the cache's ordinary cost on a still camera.
            for (label, cache) in [("before-upload", true), ("before-upload-cache-off", false)] {
                rig.settings(if cache {
                    on
                } else {
                    IndirectSettings::off()
                });
                rig.r.reset_indirect();
                rows[label] = sequence(&mut rig, &sv, &materials, 32, |_, _| (eye, target));
            }
            let prepared = directional_environment(sv.sky);
            let map = loop {
                if let Some(map) = rig.r.try_upload_environment(&prepared).unwrap() {
                    break map;
                }
                rig.r.test_drain("G1 environment upload");
            };
            for (label, cache, selected, rotate) in [
                ("no-map", true, false, false),
                ("map-still", true, true, false),
                ("map-rotating", true, true, true),
                ("map-rotating-cache-off", false, true, true),
                ("map-still-cache-off", false, true, false),
                ("no-map-cache-off", false, false, false),
            ] {
                rig.settings(if cache {
                    on
                } else {
                    IndirectSettings::off()
                });
                rig.r.reset_indirect();
                rig.r
                    .set_environment(selected.then_some(&map), Default::default())
                    .unwrap();
                rows[label] = sequence(&mut rig, &sv, &materials, 32, |rig, t| {
                    if rotate {
                        rig.r
                            .set_environment(
                                Some(&map),
                                EnvironmentParameters {
                                    rotation_y_radians: (t as f32).to_radians(),
                                    ..Default::default()
                                },
                            )
                            .unwrap();
                    }
                    (eye, target)
                });
            }
            rig.r.set_environment(None, Default::default()).unwrap();
            path_json["mapdiag"] = rows;
        }
        // The measured configurations name rotation explicitly: "trilinear-4px" and "on"
        // are unrotated, "rotated-4px" and the "rotated" timing rows are the default.
        // Every configuration names its cut refill: 1 (none) here, 4 in "spread". The
        // shipping default (0) picks 4 on software rays and 1 on hardware rays.
        let base = IndirectSettings {
            enabled: true,
            rotate_directions: false,
            cut_phases: 1,
            ..Default::default()
        };
        // Configurations: GI off (the pre-M7 image), then the W3 lookups and cell sizes.
        let mut configs = vec![
            ("off", IndirectSettings::off()),
            ("trilinear-4px", base),
        ];
        if !quick {
            configs.extend([
                (
                    "blend-4px",
                    IndirectSettings {
                        level_blend: 0.125,
                        ..base
                    },
                ),
                (
                    "k128-rotated-blend-4px",
                    IndirectSettings {
                        level_blend: 0.125,
                        directions: 128,
                        rotate_directions: true,
                        ..base
                    },
                ),
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
                    "k128-rotated-4px",
                    IndirectSettings {
                        directions: 128,
                        rotate_directions: true,
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
                // The cut refill: the default (rotated) with four direction groups over the
                // frames after a reset.
                (
                    "spread-4px",
                    IndirectSettings {
                        rotate_directions: true,
                        cut_phases: 4,
                        ..base
                    },
                ),
                // G1d: the refill over eight groups, the most the 8-frame settle limit allows.
                (
                    "spread8-4px",
                    IndirectSettings {
                        rotate_directions: true,
                        cut_phases: 8,
                        ..base
                    },
                ),
            ]);
        }
        // RENDERLAB_M7_G1_CONFIGS: a comma list of the configurations to score (all by default).
        let wanted = env("RENDERLAB_M7_G1_CONFIGS");
        configs.retain(|(c, _)| {
            section("quality")
                && wanted
                    .as_deref()
                    .is_none_or(|w| w.split(',').any(|x| x.trim() == *c))
        });
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
                let second = rig.radiance(&sv);
                // Settled: three more frames (W1 form 2 needs its phases; an eight-group refill
                // needs eight in all); then a still frame, which runs no cache pass once inputs
                // repeat.
                let settle = if s.cut_phases > 4 {
                    s.cut_phases - 2
                } else {
                    3
                };
                for _ in 0..settle {
                    rig.frame(&sv, *eye, *target, &materials);
                }
                let image = rig.radiance(&sv);
                let idle_before = rig.r.indirect_stats().idle_frames;
                rig.frame(&sv, *eye, *target, &materials);
                let idle_gpu = frame_gpu(&mut rig.r);
                let idle = json!({"ran_no_cache_pass": rig.r.indirect_stats().idle_frames > idle_before,
                    "gpu": idle_gpu, "image_equal": hash_image(&rig.radiance(&sv)) == hash_image(&image)});
                let placements = rig.placements(&sv);
                let sets = labels(&sv, &placements, *eye, *target);
                let open = open_luminance(&sv, &sets, reference);
                let scores: Json = names
                    .iter()
                    .zip(&sets)
                    .map(|(n, px)| (n.clone(), score(&image, reference, open, px)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                let first_scores = score(&first, reference, open, sets.last().unwrap());
                let second_scores = score(&second, reference, open, sets.last().unwrap());
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
                    "scores": scores, "first_frame": first_scores, "second_frame": second_scores,
                    "first_equals_settled": hash_image(&first) == hash_image(&image),
                    "image_hash": hash_image(&image), "cut": cut, "cut_gpu": cut_gpu, "static": still, "static_gpu": still_gpu, "idle": idle,
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
        // G2: G2's Lambert gate (with `scatter`) against the bounce references, with the cache
        // off, sky only (bounce off), and with bounce: every population after twelve frames,
        // the settle (frames 1, 2, 4, 8 and 12 after a reset), and the 5-frame pan's flicker.
        if section("bounce") {
            let bounce = std::mem::take(&mut rig.bounce);
            let mut rows = json!({});
            // RENDERLAB_G2B_SV_BOUNCE (G2b): the bounce configurations, G2 quarry variant names
            // (default "bounce", the default settings); RENDERLAB_G2B_SV_FRAMES the frame the
            // scored image is taken at (default 12).
            let mut configs: Vec<(String, IndirectSettings)> = vec![
                ("off".into(), IndirectSettings::off()),
                (
                    "sky".into(),
                    IndirectSettings {
                        enabled: true,
                        bounce: false,
                        ..Default::default()
                    },
                ),
            ];
            for name in env("RENDERLAB_G2B_SV_BOUNCE")
                .unwrap_or_else(|| "bounce".into())
                .split(',')
            {
                let s = if name == "bounce" {
                    IndirectSettings::default()
                } else {
                    g2_variant(name)
                };
                configs.push((name.to_string(), s));
            }
            let last: usize = env("RENDERLAB_G2B_SV_FRAMES").map_or(12, |f| f.parse().unwrap());
            for (config, s) in configs {
                let config = config.as_str();
                let mut config_json = json!({});
                for ((pose, eye, target), reference) in poses.iter().zip(&bounce_refs) {
                    rig.settings(s);
                    rig.r.reset_indirect();
                    let mut frames = vec![];
                    for f in 1..=last {
                        rig.frame(&sv, *eye, *target, &bounce);
                        if [1, 2, 4, 8, 12, 24, 48, 96].contains(&f) || f == last {
                            frames.push((f, rig.radiance(&sv)));
                        }
                    }
                    let image = frames.last().unwrap().1.clone();
                    let placements = rig.placements(&sv);
                    let sets = labels(&sv, &placements, *eye, *target);
                    let open = open_luminance(&sv, &sets, reference);
                    let scores: Json = names
                        .iter()
                        .zip(&sets)
                        .map(|(n, px)| (n.clone(), score(&image, reference, open, px)))
                        .collect::<serde_json::Map<_, _>>()
                        .into();
                    let settle: Vec<Json> = frames
                        .iter()
                        .map(|(f, im)| json!({"frame": f, "frame_score": score(im, reference, open, sets.last().unwrap())}))
                        .collect();
                    png(
                        &out.join(format!("{label}-bounce-{config}-{pose}.png")),
                        sv.size,
                        image.iter().copied(),
                        (sv.exposure).exp2(),
                    );
                    config_json[pose] = json!({"scores": scores, "settle": settle, "image_hash": hash_image(&image)});
                    eprintln!(
                        "G2 SV {label} {config} {pose}: frame rms {}",
                        scores["frame"]["rms"]
                    );
                }
                if !bounce_motion.is_empty() {
                    rig.settings(s);
                    rig.r.reset_indirect();
                    for _ in 0..12 {
                        rig.frame(&sv, motion_eye, motion_target, &bounce);
                    }
                    let mut images = vec![];
                    for t in 0..bounce_motion.len() {
                        rig.frame(&sv, motion_eye + pan(t), motion_target + pan(t), &bounce);
                        images.push(rig.radiance(&sv));
                    }
                    let placements = rig.placements(&sv);
                    let all: Vec<usize> = placements
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| **p != 0 && **p >> 31 == 0)
                        .map(|(i, _)| i)
                        .collect();
                    config_json["motion"] = flicker(&images, &bounce_motion, &all);
                }
                rows[config] = config_json;
            }
            path_json["bounce"] = rows;
            rig.bounce = bounce;
        }
        // A program that never calls the cache runs no cache pass and draws the same bytes.
        if section("gates") {
            let (_, eye, target) = &poses[0];
            rig.settings(IndirectSettings::off());
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
        // (cross-process equality compares these hashes between runs). Table hashes skip
        // the header, which holds this frame's settings.
        if section("gates") {
            let (_, eye, target) = &poses[0];
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let table = rig.r.indirect_table().unwrap().unwrap();
            let a = (
                hash_words(&table[HEADER_WORDS as usize..]),
                hash_image(&rig.radiance(&sv)),
            );
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let table = rig.r.indirect_table().unwrap().unwrap();
            let b = (
                hash_words(&table[HEADER_WORDS as usize..]),
                hash_image(&rig.radiance(&sv)),
            );
            // Serial tracing (one thread per cell) traces the same rays into the same bits.
            rig.settings(IndirectSettings {
                parallel_rays: false,
                ..base
            });
            rig.r.reset_indirect();
            rig.frame(&sv, *eye, *target, &materials);
            rig.frame(&sv, *eye, *target, &materials);
            let table = rig.r.indirect_table().unwrap().unwrap();
            let serial = (
                hash_words(&table[HEADER_WORDS as usize..]),
                hash_image(&rig.radiance(&sv)),
            );
            rig.settings(base);
            path_json["determinism"] = json!({"table": a.0, "image": a.1,
                "table_repeat_equal": a.0 == b.0, "image_repeat_equal": a.1 == b.1,
                "serial_table_equal": a.0 == serial.0, "serial_image_equal": a.1 == serial.1});
        }
        // Motion: per-frame cost and flicker against per-frame references. Each pan starts
        // from an emptied cache (cold); "-warm" rows first settle five frames at the first
        // pan camera, as Unreal's comparator pans from warm histories (M7-U0b).
        let rotated = IndirectSettings {
            rotate_directions: true,
            ..base
        };
        let spread = IndirectSettings {
            cut_phases: 4,
            ..rotated
        };
        for (config, s) in [
            ("spread-4px", spread),
            ("rotated-4px-warm", rotated),
            ("spread-4px-warm", spread),
            ("off", IndirectSettings::off()),
            ("trilinear-4px", base),
            (
                "rotated-4px",
                IndirectSettings {
                    rotate_directions: true,
                    ..base
                },
            ),
            (
                "blend-4px",
                IndirectSettings {
                    level_blend: 0.125,
                    ..base
                },
            ),
            (
                "k128-rotated-blend-4px",
                IndirectSettings {
                    level_blend: 0.125,
                    directions: 128,
                    rotate_directions: true,
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
        ] {
            if (quick && config == "nearest-4px") || !section("motion") {
                continue;
            }
            rig.settings(s);
            rig.r.reset_indirect();
            if config.ends_with("-warm") {
                for _ in 0..5 {
                    rig.frame(&sv, motion_eye, motion_target, &materials);
                }
            }
            let mut frames = vec![];
            let mut costs = vec![];
            for t in 0..motion_frames {
                rig.frame(&sv, motion_eye + pan(t), motion_target + pan(t), &materials);
                costs
                    .push(json!({"cache": profile_json(&mut rig.r), "gpu": frame_gpu(&mut rig.r)}));
                frames.push(rig.radiance(&sv));
            }
            let placements = rig.placements(&sv);
            let all: Vec<usize> = placements
                .iter()
                .enumerate()
                .filter(|(_, p)| **p != 0 && **p >> 31 == 0)
                .map(|(i, _)| i)
                .collect();
            path_json[format!("motion-{config}")] =
                json!({"flicker": flicker(&frames, &motion_refs, &all), "frames": costs});
        }
        // Caster: invalidation spikes, then the settled image against a fresh cache.
        if section("caster") {
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
            // A fresh cache settles in three frames: an orphan's parent is requested the
            // frame after the orphan is traced, and the parent's own parent a frame later
            // (gi_probe's fallback rounds), so frame 3 is the first a reset can match.
            rig.r.reset_indirect();
            for _ in 0..3 {
                rig.frame(&sv, *eye, *target, &materials);
            }
            let fresh = rig.radiance(&sv);
            let max_diff = rolled
                .iter()
                .zip(&fresh)
                .map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0f32, f32::max))
                .fold(0.0f32, f32::max);
            png(
                &out.join(format!("{label}-caster-rolled-vs-fresh-x64.png")),
                sv.size,
                rolled
                    .iter()
                    .zip(&fresh)
                    .map(|(a, b)| std::array::from_fn(|k| (a[k] - b[k]).abs() * 64.0)),
                (sv.exposure).exp2(),
            );
            let placements = rig.placements(&sv);
            let sets = labels(&sv, &placements, *eye, *target);
            let open = open_luminance(&sv, &sets, caster_ref.as_ref().unwrap());
            let all = sets.last().unwrap().clone();
            path_json["caster"] = json!({
                "frames": costs, "rolled_equals_fresh": hash_image(&rolled) == hash_image(&fresh),
                "rolled_max_abs_difference": max_diff,
                "rolled_pixels_differing": rolled.iter().zip(&fresh).filter(|(a, b)| a != b).count(),
                "rolled_score": score(&rolled, caster_ref.as_ref().unwrap(), open, &all),
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
                    let c = rig.r.indirect_counters().unwrap().unwrap_or_default();
                    stale.push(json!({"age": age, "pixels_differing_from_fresh": d,
                        "traces": c.traces, "refreshed": c.refreshed, "invalidated": c.invalidated,
                        "score": score(&image, caster_ref.as_ref().unwrap(), open, &all)}));
                }
                path_json["rotation-staleness"] = json!(stale);
                rig.settings(base);
            }
            rig.instances[sphere].set(0, Mat4::IDENTITY).unwrap();
        }
        // Rotating environment: a directional prepared map yawing 15 degrees a frame. A yaw
        // re-integrates every cell from its stored bits (no rays), so each stepped frame must
        // equal a fresh cache at the same yaw (0 frames lag). Then a continuous rotating
        // sequence times it. The reference tracer has only a constant sky, so this checks
        // lag and cost, not error against a path-traced map.
        if section("environment") {
            let (_, eye, target) = poses[0].clone();
            rig.settings(IndirectSettings {
                enabled: true,
                cut_phases: 1,
                ..Default::default()
            });
            let prepared = directional_environment(sv.sky);
            let map = loop {
                if let Some(map) = rig.r.try_upload_environment(&prepared).unwrap() {
                    break map;
                }
                rig.r.test_drain("G1 environment upload");
            };
            let yaw = |rig: &mut Rig, degrees: f32| {
                rig.r
                    .set_environment(
                        Some(&map),
                        EnvironmentParameters {
                            rotation_y_radians: degrees.to_radians(),
                            ..Default::default()
                        },
                    )
                    .unwrap();
            };
            yaw(&mut rig, 0.0);
            rig.r.reset_indirect();
            for _ in 0..3 {
                rig.frame(&sv, eye, target, &materials);
            }
            let mut steps = vec![];
            for t in 1..=6 {
                let degrees = 15.0 * t as f32;
                // The rolled cache: the previous frame's cells, re-integrated at this yaw.
                yaw(&mut rig, degrees);
                rig.frame(&sv, eye, target, &materials);
                let counters = rig.r.indirect_counters().unwrap().unwrap_or_default();
                let rolled = rig.radiance(&sv);
                // A fresh cache at the same yaw.
                rig.r.reset_indirect();
                for _ in 0..3 {
                    rig.frame(&sv, eye, target, &materials);
                }
                let fresh = rig.radiance(&sv);
                let open = {
                    rig.settings(IndirectSettings::off());
                    rig.frame(&sv, eye, target, &materials);
                    let open = rig.radiance(&sv);
                    rig.settings(IndirectSettings {
                        enabled: true,
                        cut_phases: 1,
                        ..Default::default()
                    });
                    open
                };
                if t == 3 {
                    png(
                        &out.join(format!("{label}-environment-yaw45.png")),
                        sv.size,
                        rolled.iter().copied(),
                        (sv.exposure).exp2(),
                    );
                }
                // How much the yaw changes the occluded sky, so equality is not vacuous.
                let changed = rolled.iter().zip(&open).filter(|(a, b)| a != b).count();
                steps.push(json!({"yaw_degrees": degrees,
                    "rolled_equals_fresh": hash_image(&rolled) == hash_image(&fresh),
                    "pixels_differing": rolled.iter().zip(&fresh).filter(|(a, b)| a != b).count(),
                    "pixels_differing_from_open_sky": changed,
                    "integrates": counters.integrates, "traces": counters.traces, "rays": counters.rays}));
                // Back to the rolled state for the next step: settled at this yaw.
                rig.r.reset_indirect();
                for _ in 0..3 {
                    rig.frame(&sv, eye, target, &materials);
                }
            }
            // A continuous rotation, 1 degree a frame, camera still.
            rig.r.reset_indirect();
            let rotating = sequence(&mut rig, &sv, &materials, 48, |rig, t| {
                yaw(rig, t as f32);
                (eye, target)
            });
            // The last rotated frame against a fresh cache at its yaw (a changing map
            // re-integrates one fixed slot group a frame, so cells lag by up to seven
            // frames), then the still frames until the rolled cache equals a fresh one.
            let rolled = rig.radiance(&sv);
            rig.r.reset_indirect();
            for _ in 0..3 {
                rig.frame(&sv, eye, target, &materials);
            }
            let fresh = rig.radiance(&sv);
            let compare = |a: &[[f32; 3]], b: &[[f32; 3]]| {
                let rel: Vec<f64> = a
                    .iter()
                    .zip(b)
                    .filter(|(_, b)| luminance(**b) > 0.0)
                    .map(|(a, b)| (luminance(*a) - luminance(*b)) / luminance(*b))
                    .collect();
                let n = rel.len().max(1) as f64;
                json!({"pixels_differing": a.iter().zip(b).filter(|(a, b)| a != b).count(),
                    "relative_rms": (rel.iter().map(|x| x * x).sum::<f64>() / n).sqrt(),
                    "relative_max": rel.iter().fold(0.0f64, |m, x| m.max(x.abs()))})
            };
            let lag = compare(&rolled, &fresh);
            rig.r.reset_indirect();
            for t in 0..48 {
                yaw(&mut rig, t as f32);
                rig.frame(&sv, eye, target, &materials);
            }
            let mut settle = vec![];
            for _ in 0..10 {
                rig.frame(&sv, eye, target, &materials);
                settle.push(compare(&rig.radiance(&sv), &fresh));
            }
            let settled_after = settle
                .iter()
                .position(|s| s["pixels_differing"] == 0)
                .map(|k| k + 1);
            path_json["environment"] = json!({"steps": steps, "rotating": rotating,
                "rotating_lag": lag, "still_frames_to_fresh": settled_after, "still_after_rotation": settle});
            rig.r.set_environment(None, Default::default()).unwrap();
            rig.settings(base);
        }
        // G1d breakdown: the camera-moving cost per stage (`motion_breakdown`), 48 frames at
        // 1 cm a frame along x from the overview pose, cache off then the shipping default
        // (rotated, the path's default cut refill), each settled still first.
        if section("breakdown") {
            let (_, eye, target) = poses[0].clone();
            let mut breakdown = json!({"step_m": 0.01});
            for (state, s) in [
                ("off", IndirectSettings::off()),
                (
                    "on",
                    IndirectSettings {
                        enabled: true,
                        ..Default::default()
                    },
                ),
            ] {
                rig.settings(s);
                rig.r.reset_indirect();
                for _ in 0..12 {
                    rig.frame(&sv, eye, target, &materials);
                }
                breakdown[state] = motion_breakdown(
                    &mut rig,
                    |rig| &mut rig.r,
                    48,
                    2,
                    |rig, t| {
                        let d = Vec3::new(0.01 * t as f32, 0.0, 0.0);
                        rig.frame(&sv, eye + d, target + d, &materials);
                    },
                );
            }
            path_json["breakdown"] = breakdown;
        }
        // Timing: continuous sequences for the cache off (the pre-M7 frame), unrotated,
        // rotated (the default), serial tracing and the best-quality variant, visited in
        // turn `visits` times (timing[label] lists the visits).
        if section("timing") {
            let (_, eye, target) = poses[0].clone();
            // RENDERLAB_M7_G1_TIMING_PROGRAM=standard times the standard program (the
            // default material) instead of the Lambert gate.
            let timed = if env("RENDERLAB_M7_G1_TIMING_PROGRAM").as_deref() == Some("standard") {
                &standard
            } else {
                &materials
            };
            let mut timing =
                json!({"off": [], "on": [], "rotated": [], "serial": [], "best": [], "spread": []});
            let best = IndirectSettings {
                level_blend: 0.125,
                directions: 128,
                rotate_directions: true,
                ..base
            };
            let order = [
                ("off", IndirectSettings::off()),
                ("on", base),
                (
                    "rotated",
                    IndirectSettings {
                        rotate_directions: true,
                        ..base
                    },
                ),
                (
                    "serial",
                    IndirectSettings {
                        parallel_rays: false,
                        ..base
                    },
                ),
                ("best", best),
                (
                    "spread",
                    IndirectSettings {
                        rotate_directions: true,
                        cut_phases: 4,
                        ..base
                    },
                ),
            ];
            // RENDERLAB_M7_G1_TIMING: a comma list of the labels to visit (all by default).
            let wanted = env("RENDERLAB_M7_G1_TIMING");
            let order: Vec<_> = order
                .into_iter()
                .filter(|(l, _)| {
                    wanted
                        .as_deref()
                        .is_none_or(|w| w.split(',').any(|x| x.trim() == *l))
                })
                .collect();
            // Warm-up, discarded: clocks and caches settle before the first visit.
            rig.settings(base);
            rig.r.reset_indirect();
            sequence(&mut rig, &sv, timed, 64, |_, t| {
                let d = Vec3::new(0.01 * t as f32, 0.0, 0.0);
                (eye + d, target + d)
            });
            for &(label, s) in (0..visits).flat_map(|_| &order) {
                rig.settings(s);
                rig.r.reset_indirect();
                let still = sequence(&mut rig, &sv, timed, 24, |_, _| (eye, target));
                let camera = sequence(&mut rig, &sv, timed, 48, |_, t| {
                    let d = Vec3::new(0.01 * t as f32, 0.0, 0.0);
                    (eye + d, target + d)
                });
                let caster = sequence(&mut rig, &sv, timed, 48, |rig, t| {
                    rig.instances[sphere]
                        .set(
                            0,
                            Mat4::from_translation(Vec3::new(0.002 * t as f32, 0.0, 0.0)),
                        )
                        .unwrap();
                    (eye, target)
                });
                rig.instances[sphere].set(0, Mat4::IDENTITY).unwrap();
                let cut = sequence(&mut rig, &sv, timed, 16, |rig, t| {
                    if t % 2 == 0 {
                        rig.r.reset_indirect();
                    }
                    (eye, target)
                });
                // Environment steps: a new sky radiance every frame (no rays: the
                // cells re-integrate their stored bits).
                let environment = sequence(&mut rig, &sv, timed, 16, |rig, t| {
                    let k = 1.0 + 0.05 * (t % 2) as f32;
                    rig.r
                        .set_lighting(&Lighting {
                            lights: vec![],
                            environment: sv.sky.map(|v| v * k),
                            exposure_stops: sv.exposure,
                        })
                        .unwrap();
                    (eye, target)
                });
                timing[label]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"static": still,
                    "camera": camera, "caster": caster, "cut": cut, "environment": environment}));
            }
            // The environment step's lag: the stepped frame against a fresh cache.
            rig.settings(base);
            rig.r.reset_indirect();
            let sky = |k: f32| Lighting {
                lights: vec![],
                environment: sv.sky.map(|v| v * k),
                exposure_stops: sv.exposure,
            };
            rig.r.set_lighting(&sky(1.0)).unwrap();
            rig.frame(&sv, eye, target, &materials);
            rig.frame(&sv, eye, target, &materials);
            rig.r.set_lighting(&sky(1.5)).unwrap();
            rig.frame(&sv, eye, target, &materials);
            let stepped = rig.radiance(&sv);
            rig.r.reset_indirect();
            for _ in 0..3 {
                rig.frame(&sv, eye, target, &materials);
            }
            let fresh = rig.radiance(&sv);
            let differing = stepped.iter().zip(&fresh).filter(|(a, b)| a != b).count();
            let max_relative = stepped
                .iter()
                .zip(&fresh)
                .map(|(a, b)| (luminance(*a) - luminance(*b)).abs() / luminance(*b).max(1e-6))
                .fold(0.0f64, f64::max);
            rig.r.set_lighting(&sky(1.0)).unwrap();
            timing["environment_step_next_frame"] = json!({
                "equal_to_fresh": hash_image(&stepped) == hash_image(&fresh),
                "max_relative_difference": max_relative, "pixels_differing": differing,
            });
            path_json["timing"] = timing;
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
/// A directional sky for the rotating-environment test: SV's constant sky on average,
/// brighter toward one azimuth and elevation, dim below the horizon, so a yaw changes the
/// occluded irradiance. A legacy mip chain (16 to 1) of the unfiltered radiance, and a
/// brute-force cosine-convolved irradiance cube; exact filtering does not matter here.
fn directional_environment(sky: [f32; 3]) -> crate::asset::environment::PreparedEnvironment {
    use crate::asset::environment::{
        self as env, CubeFace, CubeLevel, Metadata, PreparedEnvironment, RadianceUnits,
    };
    let sun = Vec3::new(0.8, 0.5, 0.33).normalize();
    let radiance = |d: [f64; 3]| -> [f32; 3] {
        let d = Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32);
        let lobe = d.dot(sun).max(0.0).powi(6);
        let k = if d.y < 0.0 { 0.1 } else { 0.35 + 3.0 * lobe };
        sky.map(|v| v * k)
    };
    let level = |size: u32, value: &dyn Fn([f64; 3]) -> [f32; 3]| {
        let mut pixels = vec![];
        for face in CubeFace::ALL {
            for y in 0..size {
                for x in 0..size {
                    pixels.push(value(face.direction([
                        2. * (x as f64 + 0.5) / size as f64 - 1.,
                        2. * (y as f64 + 0.5) / size as f64 - 1.,
                    ])));
                }
            }
        }
        CubeLevel::new(size, pixels, env::Limits::default()).unwrap()
    };
    // Irradiance around n: midpoint quadrature over the sphere.
    let irradiance = |n: [f64; 3]| -> [f32; 3] {
        let (nz, np) = (32, 64);
        let mut e = [0.0f64; 3];
        for i in 0..nz {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / nz as f64;
            let r = (1.0 - z * z).sqrt();
            for j in 0..np {
                let phi = (j as f64 + 0.5) / np as f64 * std::f64::consts::TAU;
                let d = [r * phi.cos(), z, r * phi.sin()];
                let c = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
                if c > 0.0 {
                    let l = radiance(d);
                    for k in 0..3 {
                        e[k] += c * f64::from(l[k]);
                    }
                }
            }
        }
        let w = 4.0 * std::f64::consts::PI / (nz * np) as f64;
        e.map(|v| (v * w) as f32)
    };
    PreparedEnvironment::new(
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            source_radiance_scale: 1.0,
            rotation_y_radians: 0.0,
            generator_hash: [0x91; 32],
        },
        level(8, &irradiance),
        [16, 8, 4, 2, 1]
            .into_iter()
            .map(|size| level(size, &radiance))
            .collect(),
        env::Limits::default(),
    )
    .unwrap()
}
fn hash_words(words: &[u32]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for w in words {
        h.update(w.to_le_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

// ---- M7-G1b: the registered quarry (docs/m7-g1b.md section 6) ----
// The quarry loaders other test modules also include; theirs are private to those modules.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod quarry_cameras;
#[allow(dead_code)]
#[path = "../support/environment_controls.rs"]
mod quarry_environment;
#[path = "../support/limits.rs"]
mod quarry_policy;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod quarry_prepared;

/// The quarry gate program: a constant albedo of 0.5 under the engine's sky irradiance and
/// nothing else, so the image is `indirect_irradiance` itself (textures play no part).
const QUARRY_GATE: &str = "struct Surface { n:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 { n=input.geometric; }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(n);
}
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return expose(indirect_irradiance(input,s.n)*(0.5/PI));
}
";

/// Placement + 1 per covered pixel (0 = miss or uncovered tile), with bit 31 on pixels
/// whose 3x3 neighbourhood meets another placement (the quarry's silhouettes between scans).
fn quarry_hits(r: &Renderer) -> Vec<u32> {
    let primary = r.primary.as_ref().unwrap();
    let targets = primary.targets.as_ref().unwrap();
    let region = primary.scene;
    let size = targets.hit.texture.size();
    let data = read_texture(r, &targets.hit.texture, 16);
    let flags = read_texture(r, &targets.tiles.texture, 4);
    let tiles_width = targets.tiles.texture.size().width as usize;
    let [tile_w, tile_h] = crate::renderer::primary_ray_gpu::TRACE_TILE.map(|t| t as usize);
    let (w, h) = (size.width as usize, size.height as usize);
    let hits: Vec<u32> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let inside = (region[0] as usize..region[2] as usize).contains(&x)
                && (region[1] as usize..region[3] as usize).contains(&y);
            let t = ((y / tile_h) * tiles_width + x / tile_w) * 4;
            let flag = u32::from_le_bytes(flags[t..t + 4].try_into().unwrap());
            if !inside || flag == 0 {
                return 0;
            }
            u32::from_le_bytes(data[i * 16..i * 16 + 4].try_into().unwrap())
        })
        .collect();
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let p = hits[i];
            let edge = p != 0
                && (y.saturating_sub(1)..(y + 2).min(h)).any(|ny| {
                    (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| hits[ny * w + nx] != p)
                });
            p | (u32::from(edge) << 31)
        })
        .collect()
}

/// `ours` against `exact` over covered nonedge pixels: relative luminance difference floored
/// at 5% of the exact image's p90 (G1's floor), RMS, bias, p99, and the share darker or
/// brighter than exact by more than 5%. `dark` restricts to pixels below 70% of that p90.
fn quarry_score(ours: &[[f32; 3]], exact: &[[f32; 3]], hits: &[u32]) -> Json {
    let pixels: Vec<usize> = (0..hits.len())
        .filter(|&i| hits[i] != 0 && hits[i] >> 31 == 0)
        .collect();
    let mut lum: Vec<f64> = pixels.iter().map(|&i| luminance(exact[i])).collect();
    lum.sort_by(f64::total_cmp);
    let open = lum.get(lum.len() * 9 / 10).copied().unwrap_or(1.0);
    let stats = |set: &[usize]| {
        let mut e: Vec<f64> = set
            .iter()
            .map(|&i| {
                (luminance(ours[i]) - luminance(exact[i])) / luminance(exact[i]).max(0.05 * open)
            })
            .collect();
        let n = e.len().max(1) as f64;
        let rms = (e.iter().map(|x| x * x).sum::<f64>() / n).sqrt();
        let bias = e.iter().sum::<f64>() / n;
        let darker = e.iter().filter(|x| **x < -0.05).count();
        let brighter = e.iter().filter(|x| **x > 0.05).count();
        e.iter_mut().for_each(|x| *x = x.abs());
        e.sort_by(f64::total_cmp);
        json!({"pixels": set.len(), "rms": rms, "bias": bias, "p99": e.get(((e.len() as f64 - 1.0) * 0.99) as usize),
            "darker_5pct": darker, "brighter_5pct": brighter})
    };
    let dark: Vec<usize> = pixels
        .iter()
        .copied()
        .filter(|&i| luminance(exact[i]) < 0.7 * open)
        .collect();
    // Luminance ratios ours/exact, for the shape of the tails: quantiles and the pixels
    // below 5% or half of exact, or above 1.5x.
    let mut ratio: Vec<f64> = pixels
        .iter()
        .map(|&i| luminance(ours[i]) / luminance(exact[i]).max(1e-6 * open))
        .collect();
    ratio.sort_by(f64::total_cmp);
    let at = |q: f64| {
        ratio
            .get(((ratio.len() as f64 - 1.0) * q) as usize)
            .copied()
    };
    let quantiles = [0.01, 0.05, 0.25, 0.5, 0.75, 0.95, 0.99].map(at);
    json!({"frame": stats(&pixels), "dark": stats(&dark), "open_p90": open,
        "ratio": {"q": quantiles,
            "below_5pct": ratio.iter().filter(|r| **r < 0.05).count(),
            "below_half": ratio.iter().filter(|r| **r < 0.5).count(),
            "above_1_5": ratio.iter().filter(|r| **r > 1.5).count()}})
}

/// The sky a quarry reference sees: a constant radiance, or the registered RLE2 map
/// (identified by its file's digest) as the reference tracer samples it.
enum ReferenceSky {
    Constant(f32),
    Map([u8; 32], reference::Environment),
}

/// What a quarry reference renders, at `max_depth` 2 (camera, surface, light: no bounce).
enum ReferenceScene<'a> {
    /// G1's estimand: the gate's albedo 0.5 Lambert on every placement, under the sky alone.
    Gate(&'a crate::asset::Asset),
    /// The quarry's own materials and textures, its sun and the map, on a black camera
    /// backdrop: U0b's quarry protocol without bounce. The asset decodes from `asset` (with
    /// `limits`) only on a cache miss; `key` names its inputs.
    Full {
        asset: PathBuf,
        limits: crate::asset::Limits,
        sun: reference::Sun,
        key: Vec<u8>,
    },
}

/// The reference tracer's depth-2 image of the quarry pose at RENDERLAB_M7_G1_SPP samples
/// (default 1024). Cached by pose, scene, sky and samples under RENDERLAB_M7_G1_REFS, so the
/// non-RT process (no ray queries) reads what the RT process rendered.
fn quarry_reference(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    content: &ReferenceScene<'_>,
    pose: &quarry_cameras::Pose,
    sky: &ReferenceSky,
) -> reference::Image {
    use sha2::Digest;
    let settings = reference::Settings {
        samples: env("RENDERLAB_M7_G1_SPP").map_or(1024, |s| s.parse().unwrap()),
        max_depth: 2,
        ..Default::default()
    };
    let dir = PathBuf::from(
        env("RENDERLAB_M7_G1_REFS").unwrap_or_else(|| "E:/renderlab-work/m7-g1b/refs".into()),
    );
    let mut h = sha2::Sha256::new();
    // The constant-sky gate keeps its G1b key, so its cached images stay valid.
    match (content, sky) {
        (ReferenceScene::Gate(_), ReferenceSky::Constant(_)) => h.update(b"quarry-v1"),
        (ReferenceScene::Gate(_), ReferenceSky::Map(..)) => h.update(b"quarry-map-v1"),
        (ReferenceScene::Full { key, .. }, _) => {
            h.update(b"quarry-full-v1");
            h.update(key);
        }
    }
    for v in [pose.eye, pose.target, pose.up] {
        h.update(v.map(f32::to_le_bytes).concat());
    }
    h.update(pose.fov_y_radians.to_le_bytes());
    match sky {
        ReferenceSky::Constant(l) => h.update(l.to_le_bytes()),
        ReferenceSky::Map(digest, _) => h.update(digest),
    }
    h.update(settings.samples.to_le_bytes());
    h.update(settings.seed.to_le_bytes());
    let path = dir.join(format!("quarry-{:x}.f32", h.finalize()));
    let pixels = (pose.width * pose.height) as usize;
    let decode = |b: &[u8]| -> Vec<[f32; 3]> {
        b.chunks_exact(12)
            .map(|c| {
                std::array::from_fn(|k| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap()))
            })
            .collect()
    };
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() == pixels * 24
    {
        return reference::Image {
            width: pose.width,
            height: pose.height,
            samples: settings.samples,
            mean: decode(&bytes[..pixels * 12]),
            standard_error: decode(&bytes[pixels * 12..]),
            invalid_pixels: 0,
            halves: [vec![], vec![]],
            geometry: vec![],
        };
    }
    let (constant, environment) = match sky {
        ReferenceSky::Constant(l) => ([*l; 3], None),
        ReferenceSky::Map(_, map) => ([0.0; 3], Some(map.clone())),
    };
    let scene = match content {
        ReferenceScene::Gate(source) => {
            // One reference mesh per source primitive; one instance per placement and
            // primitive.
            let mut meshes = vec![];
            let mut first = vec![];
            for m in &source.meshes {
                first.push(meshes.len());
                for p in &m.primitives {
                    meshes.push(reference::Mesh {
                        positions: p.vertices.iter().map(|v| v.position).collect(),
                        normals: p.vertices.iter().map(|v| v.normal).collect(),
                        indices: p.indices.clone(),
                        uvs: vec![],
                        tangents: vec![],
                        colors: vec![],
                    });
                }
            }
            let instances = source
                .placements
                .iter()
                .flat_map(|p| {
                    let m = p.mesh as usize;
                    let transform = Mat4::from_cols_array_2d(&p.transform);
                    (0..source.meshes[m].primitives.len()).map(move |k| (m, k, transform))
                })
                .map(|(m, k, transform)| reference::Instance {
                    mesh: first[m] + k,
                    material: 0,
                    transform,
                })
                .collect();
            reference::Scene {
                meshes,
                materials: vec![reference::Material {
                    base_color: [0.5; 3],
                    metallic: 0.0,
                    roughness: 1.0,
                    specular: 0.0,
                    emission: [0.0; 3],
                    base_color_texture: None,
                    metallic_roughness_texture: None,
                    normal_texture: None,
                }],
                instances,
                sun: None,
                sky: constant,
                textures: vec![],
                environment,
                camera_background: None,
            }
        }
        ReferenceScene::Full {
            asset, limits, sun, ..
        } => {
            let bytes = std::fs::read(asset).unwrap();
            let mut scene =
                reference::Scene::from_prepared(crate::asset::decode(&bytes, *limits).unwrap());
            scene.sun = Some(*sun);
            scene.sky = constant;
            scene.environment = environment;
            scene.camera_background = Some([0.0; 3]);
            scene
        }
    };
    let at = |p: [f32; 3]| Vec3::from_array(p);
    let pose_ = reference::Pose {
        eye: at(pose.eye),
        target: at(pose.target),
    };
    let view = reference::View {
        open: pose_,
        close: pose_,
        up: at(pose.up),
        fov_y: pose.fov_y_radians,
        width: pose.width,
        height: pose.height,
    };
    let image = reference::render(device, queue, &scene, &view, &settings).unwrap();
    assert_eq!(image.invalid_pixels, 0);
    std::fs::create_dir_all(&dir).unwrap();
    let raw =
        |v: &[[f32; 3]]| -> Vec<u8> { v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect() };
    std::fs::write(
        &path,
        [raw(&image.mean), raw(&image.standard_error)].concat(),
    )
    .unwrap();
    image
}

/// G1b on the registered quarry, one path per process (RENDERLAB_M7_G1_PATHS rt or nonrt):
/// poses 0 and 1, its RLE2 sky only (no sun), the constant-albedo gate program. RT (hardware
/// rays: the cache's exact visibility for its estimator) writes its images to
/// RENDERLAB_M7_G1_QUARRY_EXACT; non-RT scores each software-geometry variant against them
/// (RENDERLAB_M7_G1_QUARRY_VARIANTS: "full" builds a full-detail BVH per mesh; "lodN" uses
/// R0c's cooked cluster BVH at error index N; "near" a cooked near field). With
/// RENDERLAB_M7_G1_QUARRY_SKY (a constant sky in place of the map, or "map"), each variant is
/// also scored against the reference tracer's depth-2 image (`quarry_reference`).
/// RENDERLAB_M7_G1_QUARRY_MODE=full renders U0b's quarry protocol instead (the quarry's own
/// materials, sun and shadows, black backdrop) for U0b's F0 scorer. Correctness only: no
/// timing claim.
#[test]
#[ignore = "GPU; optimized build; RENDERLAB_M7_G1_QUARRY, RENDERLAB_M7_G1_OUT, RENDERLAB_M7_G1_QUARRY_EXACT"]
fn gpu_gi_quarry() {
    let start = Instant::now();
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(env("RENDERLAB_M7_G1_QUARRY").unwrap()).unwrap())
            .unwrap();
    let out = PathBuf::from(env("RENDERLAB_M7_G1_OUT").unwrap());
    let exact_dir = PathBuf::from(env("RENDERLAB_M7_G1_QUARRY_EXACT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&exact_dir).unwrap();
    let path_of = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let (policy, _) = quarry_policy::Policy::args(vec![
        "--limits".into(),
        path_of("limits").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let limits = policy.prepared();
    let bytes = std::fs::read(path_of("asset")).unwrap();
    let source = crate::asset::decode(&bytes, limits).unwrap();
    drop(bytes);
    // Object-space geometry per uploaded mesh (source mesh/primitive order).
    // Only full-detail software variants need a CPU copy of every triangle (about 1.5 GB on
    // the quarry); hardware and cooked near-field runs skip it.
    let needs_geometry = env("RENDERLAB_M7_G1_QUARRY_VARIANTS").map_or(
        env("RENDERLAB_M7_G1_PATHS").as_deref() == Some("nonrt"),
        |v| {
            v.split(',')
                .any(|x| matches!(x.trim(), "full" | "software-full"))
        },
    );
    let geometry: Vec<(Vec<[f32; 3]>, Vec<u32>)> = source
        .meshes
        .iter()
        .filter(|_| needs_geometry)
        .flat_map(|m| {
            m.primitives.iter().map(|p| {
                (
                    p.vertices.iter().map(|v| v.position).collect(),
                    p.indices.clone(),
                )
            })
        })
        .collect();
    let triangles: usize = source
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .map(|p| p.indices.len() / 3)
        .sum();
    let placements: Vec<Json> = source
        .placements
        .iter()
        .map(|p| json!({"mesh": p.mesh, "transform": p.transform}))
        .collect();
    let cameras = quarry_cameras::Cameras::load(&path_of("cameras")).unwrap();
    let path = env("RENDERLAB_M7_G1_PATHS").unwrap_or_else(|| "rt".into());
    let nonrt = path == "nonrt";
    let (device, queue) = if nonrt {
        no_rt_device_with(true)
    } else {
        rt_device()
    };
    // RENDERLAB_M7_G1_QUARRY_MODE=full: the quarry's own materials and textures, its sun
    // and shadows (each path's shipping configuration), the RLE2 sky and a black camera
    // backdrop (U0b's quarry protocol), cache off and on. Its dumps feed U0b's F0 scorer
    // (docs/m7-g1c.md). Otherwise G1's estimand under the albedo-0.5 gate program.
    let full = env("RENDERLAB_M7_G1_QUARRY_MODE").as_deref() == Some("full");
    // RENDERLAB_M7_G1_QUARRY_SKY: a constant sky radiance in place of the RLE2 map, or "map".
    // Either way (and always in full mode) every variant is also scored against the
    // reference tracer's depth-2 image (`quarry_reference`), which the RT process renders
    // before the renderer takes the device.
    let sky_mode = env("RENDERLAB_M7_G1_QUARRY_SKY");
    let constant_sky: Option<f32> = sky_mode
        .as_deref()
        .filter(|s| *s != "map" && !full)
        .map(|s| s.parse().unwrap());
    let map = quarry_environment::load_environment(
        &path_of("environment"),
        crate::asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let lighting: Json = if full {
        serde_json::from_slice(&std::fs::read(path_of("lighting")).unwrap()).unwrap()
    } else {
        Json::Null
    };
    let v3 = |v: &Json| -> [f32; 3] { std::array::from_fn(|k| v[k].as_f64().unwrap() as f32) };
    let sun_travel = full.then(|| Vec3::from_array(v3(&lighting["lights"][0]["travel"])));
    let sun_lux = full.then(|| v3(&lighting["lights"][0]["lux"]));
    let reference_sky = match constant_sky {
        Some(l) => ReferenceSky::Constant(l),
        None => {
            use sha2::Digest;
            let digest = sha2::Sha256::digest(std::fs::read(path_of("environment")).unwrap());
            ReferenceSky::Map(
                digest.into(),
                reference::Environment {
                    radiance: map.specular_radiance()[0].clone(),
                    yaw: map.metadata().rotation_y_radians,
                    intensity: 1.0,
                },
            )
        }
    };
    let content = if full {
        let key = json!({"asset": path_of("asset"), "asset_bytes": std::fs::metadata(path_of("asset")).unwrap().len(),
            "lighting": lighting, "angular_radius": 0.04});
        ReferenceScene::Full {
            asset: path_of("asset"),
            limits,
            sun: reference::Sun {
                to_light: -sun_travel.unwrap(),
                illuminance: sun_lux.unwrap(),
                angular_radius: 0.04,
            },
            key: key.to_string().into_bytes(),
        }
    } else {
        ReferenceScene::Gate(&source)
    };
    // RENDERLAB_M7_G1_QUARRY_NOREF=1: no reference (cost-only runs, G1d).
    let no_reference = env("RENDERLAB_M7_G1_QUARRY_NOREF").as_deref() == Some("1");
    let references: Vec<Option<reference::Image>> = (0..2)
        .map(|pose| {
            ((sky_mode.is_some() || full) && !no_reference).then(|| {
                quarry_reference(
                    &device,
                    &queue,
                    &content,
                    &cameras.frames[pose],
                    &reference_sky,
                )
            })
        })
        .collect();
    drop(content);
    // AA state (docs/iq1-quarry-image.md): 1x, TR1 off, anisotropy 1, the shipping default.
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    // The quarry's 4K textures and scans: the limits its shadow and frame-loop tests use.
    let renderer_limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let mut r = if nonrt {
        Renderer::new_with_options(device, queue, FORMAT, renderer_limits, options, SOURCE).unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
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
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    };
    let exposure = manifest["exposure_stops"].as_f64().unwrap_or(-12.0) as f32;
    let lights = match (sun_travel, sun_lux) {
        (Some(travel), Some(lux)) => vec![Light::Directional { travel, lux }],
        _ => vec![],
    };
    // Full mode: the shipping sun shadows, a 0.04 rad disk on the 4/12/40 m splits;
    // traced Interactive on RT, and Virtual (selected at creation) on non-RT.
    let shadows = if full {
        Shadows {
            directional_disk_angle: 0.04,
            requests: vec![ShadowRequest::Directional {
                split_ends: vec![4., 12., 40.],
            }],
            ..Default::default()
        }
    } else {
        Shadows::default()
    };
    r.set_lighting_and_shadows(
        &Lighting {
            lights,
            // With the map bound the constant only clears primary misses: black, as U0b.
            environment: constant_sky.map_or([0.0; 3], |l| [l; 3]),
            exposure_stops: exposure,
        },
        &shadows,
    )
    .unwrap();
    if full && !nonrt {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    if constant_sky.is_none() {
        let sky = loop {
            if let Some(map) = r.try_upload_environment(&map).unwrap() {
                break map;
            }
            r.test_drain("G1 quarry environment");
        };
        r.set_environment(Some(&sky), Default::default()).unwrap();
    }
    drop(map);
    let mut asset = quarry_prepared::Prepared::new_with_limits(source, limits, 1_000_000).unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("G1 quarry upload");
    let program = r
        .try_create_program(&ProgramDescriptor {
            label: "g1-quarry",
            source: QUARRY_GATE,
        })
        .unwrap();
    let gate = loop {
        let descriptor = MaterialDescriptor {
            base_color: [0.5, 0.5, 0.5, 1.0],
            double_sided: true,
            program: Some(&program),
            ..Default::default()
        };
        if let Some(material) = r.try_create_material(&descriptor).unwrap() {
            break material;
        }
        r.test_drain("G1 quarry material");
    };
    r.set_indirect_profiling(true);
    let load_seconds = start.elapsed().as_secs_f64();
    // RT: "hardware" (writes the exact images) and "software-full" (software rays on the RT
    // device: traced primaries, the non-RT ray source). Non-RT: "full" and "lodN".
    let variants: Vec<String> = env("RENDERLAB_M7_G1_QUARRY_VARIANTS")
        .unwrap_or_else(|| if nonrt { "full" } else { "hardware" }.into())
        .split(',')
        .map(|v| v.trim().to_string())
        .collect();
    // RENDERLAB_M7_G1_QUARRY_SETTINGS: JSON overrides of the cache settings, `directions`
    // and `cell_pixels` (defaults otherwise).
    let tuned: Json = env("RENDERLAB_M7_G1_QUARRY_SETTINGS")
        .map_or(json!({}), |s| serde_json::from_str(&s).unwrap());
    let base = IndirectSettings::default();
    let directions = tuned["directions"]
        .as_u64()
        .map_or(base.directions, |d| d as u32);
    let cell_pixels = tuned["cell_pixels"]
        .as_f64()
        .map_or(base.cell_pixels, |p| p as f32);
    // The cut refill groups; 1 (none) unless overridden, so the first frame is settled.
    let cut_phases = tuned["cut_phases"].as_u64().map_or(1, |p| p as u32);
    let settle_frames: usize =
        env("RENDERLAB_M7_G1_QUARRY_FRAMES").map_or(4, |s| s.parse().unwrap());
    let mut results = json!({"path": path, "triangles": triangles, "load_seconds": load_seconds,
        "constant_sky": constant_sky, "settings": {"directions": directions, "cell_pixels": cell_pixels, "cut_phases": cut_phases},
        "shaders": env("RENDERLAB_GI_SHADERS"), "placements": placements, "variants": {}});
    for variant in &variants {
        let geometry_start = Instant::now();
        if variant == "full" || variant == "software-full" {
            for (mesh, (positions, indices)) in asset.meshes_mut().iter().zip(&geometry) {
                r.set_indirect_geometry(mesh, positions, indices).unwrap();
            }
        } else if variant == "near" {
            // A cooked near field (renderlab-cook m7-near, docs/m7-g1c.md): its package file,
            // whose meshes are the asset's in order.
            let field = crate::asset::near_field::decode(
                &std::fs::read(env("RENDERLAB_M7_G1_NEAR").unwrap()).unwrap(),
                crate::asset::near_field::Limits::default(),
            )
            .unwrap();
            assert_eq!(field.meshes.len(), asset.meshes_mut().len());
            for (m, mesh) in asset.meshes_mut().iter().enumerate() {
                r.set_indirect_near_field(mesh, &field, m).unwrap();
            }
        } else if let Some(n) = variant.strip_prefix("lod") {
            let n: usize = n.parse().unwrap();
            let bvh = crate::asset::cluster_bvh::decode(
                &std::fs::read(path_of("cluster_bvh")).unwrap(),
                crate::asset::cluster_bvh::Limits::default(),
            )
            .unwrap();
            for (m, mesh) in asset.meshes_mut().iter().enumerate() {
                let level = bvh
                    .levels
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.mesh as usize == m)
                    .nth(n)
                    .map(|(i, _)| i)
                    .unwrap();
                r.set_indirect_bvh(mesh, bvh.clone(), level).unwrap();
            }
        }
        let geometry_seconds = geometry_start.elapsed().as_secs_f64();
        let mut variant_json = json!({"geometry_seconds": geometry_seconds});
        // RENDERLAB_M7_G1_QUARRY_POSES: the scored poses, a comma list (default "0,1"; "none"
        // skips scoring, for cost-only and trace runs).
        let poses: Vec<usize> = env("RENDERLAB_M7_G1_QUARRY_POSES")
            .unwrap_or_else(|| "0,1".into())
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        for pose in poses {
            let frame = &cameras.frames[pose];
            let camera = frame.camera().unwrap();
            let size = [frame.width, frame.height];
            let target = color_target(&r, size);
            let mut render = |r: &mut Renderer| {
                // Full mode keeps the quarry's own materials.
                let mut draws: Vec<Draw<'_>> = asset
                    .draws(Visibility::Frustum)
                    .into_iter()
                    .map(|d| if full { d } else { d.with_material(&gate) })
                    .collect();
                let until = Instant::now() + Duration::from_secs(60);
                while r
                    .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                    .unwrap()
                    .is_none()
                {
                    assert!(Instant::now() < until, "G1 quarry admission");
                    r.test_drain("G1 quarry admission");
                }
            };
            // Cache off: the unoccluded sky, for scale.
            r.set_indirect(IndirectSettings::off()).unwrap();
            render(&mut r);
            let open = capture_radiance(&mut r, exposure);
            r.set_indirect(IndirectSettings {
                enabled: true,
                cut_phases,
                force_software: variant == "software-full",
                directions,
                cell_pixels,
                ..Default::default()
            })
            .unwrap();
            r.reset_indirect();
            render(&mut r);
            let cut = profile_json(&mut r);
            // RENDERLAB_M7_G1_QUARRY_FRAMES still frames after the cut (default 4), with the
            // counters the renderer last read back after each (they lag the GPU by frames).
            let mut trail = vec![];
            for _ in 0..settle_frames {
                render(&mut r);
                let mut row = profile_json(&mut r)["counters"].clone();
                // With RENDERLAB_M7_G1_QUARRY_TABLE=1, the table's live and distinct keys.
                if env("RENDERLAB_M7_G1_QUARRY_TABLE").as_deref() == Some("1") {
                    let table = r.indirect_table().unwrap().unwrap();
                    let slots = 1usize << IndirectSettings::default().slots_log2;
                    let keys: Vec<u32> = (0..slots)
                        .map(|s| table[HEADER_WORDS as usize + s * 16] & 0x7fff_ffff)
                        .filter(|&k| k != 0)
                        .collect();
                    let distinct: std::collections::HashSet<u32> = keys.iter().copied().collect();
                    row["live_cells"] = json!(keys.len());
                    row["distinct_keys"] = json!(distinct.len());
                }
                trail.push(row);
            }
            let settled = profile_json(&mut r);
            let image = capture_radiance(&mut r, exposure);
            let hits = quarry_hits(&r);
            let stats = r.indirect_stats();
            let sky_tag = if full {
                "-full".to_string()
            } else {
                constant_sky.map_or(String::new(), |l| format!("-sky{l}"))
            };
            let exact_path = exact_dir.join(format!("rt-pose{pose}{sky_tag}.f32"));
            let exact: Vec<[f32; 3]> = if variant != "hardware" {
                std::fs::read(&exact_path)
                    .unwrap()
                    .chunks_exact(12)
                    .map(|c| {
                        std::array::from_fn(|k| {
                            f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap())
                        })
                    })
                    .collect()
            } else {
                let raw: Vec<u8> = image
                    .iter()
                    .flatten()
                    .flat_map(|x| x.to_le_bytes())
                    .collect();
                std::fs::write(&exact_path, raw).unwrap();
                image.clone()
            };
            // RENDERLAB_M7_G1_QUARRY_DUMP: the radiance image and the scored-pixel mask as raw
            // little-endian f32 and u32, for per-pixel analysis against the reference.
            if let Some(dir) = env("RENDERLAB_M7_G1_QUARRY_DUMP") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let raw: Vec<u8> = image
                    .iter()
                    .flatten()
                    .flat_map(|x| x.to_le_bytes())
                    .collect();
                std::fs::write(dir.join(format!("{path}-{variant}-pose{pose}.f32")), raw).unwrap();
                // And the cache-off image (the unoccluded sky), for the full-mode attribution.
                let raw: Vec<u8> = open
                    .iter()
                    .flatten()
                    .flat_map(|x| x.to_le_bytes())
                    .collect();
                std::fs::write(
                    dir.join(format!("{path}-{variant}-pose{pose}-open.f32")),
                    raw,
                )
                .unwrap();
                let mask: Vec<u8> = hits.iter().flat_map(|h| h.to_le_bytes()).collect();
                std::fs::write(
                    dir.join(format!("{path}-{variant}-pose{pose}-hits.u32")),
                    mask,
                )
                .unwrap();
            }
            let scale = exposure.exp2();
            png(
                &out.join(format!("{path}-{variant}-pose{pose}.png")),
                size,
                image.iter().copied(),
                scale,
            );
            png(
                &out.join(format!("{path}-{variant}-pose{pose}-open.png")),
                size,
                open.iter().copied(),
                scale,
            );
            if nonrt {
                png(
                    &out.join(format!("{path}-{variant}-pose{pose}-vs-rt-x8.png")),
                    size,
                    image
                        .iter()
                        .zip(&exact)
                        .map(|(a, b)| std::array::from_fn(|k| (a[k] - b[k]).abs() * 8.0)),
                    scale,
                );
            }
            variant_json[format!("pose{pose}")] = json!({
                "against_rt": quarry_score(&image, &exact, &hits),
                "open_against_rt": quarry_score(&open, &exact, &hits),
                "cut": cut, "settled": settled, "trail": trail, "image_hash": hash_image(&image),
                "memory": {"table_bytes": stats.table_bytes, "work_bytes": stats.work_bytes, "geometry_bytes": stats.geometry_bytes,
                    "near_bytes": stats.near_bytes, "near_instances": stats.near_instances},
            });
            if let Some(reference) = &references[pose] {
                let pose_json = &mut variant_json[format!("pose{pose}")];
                pose_json["against_reference"] = quarry_score(&image, &reference.mean, &hits);
                pose_json["open_against_reference"] = quarry_score(&open, &reference.mean, &hits);
                if let Some(dir) = env("RENDERLAB_M7_G1_QUARRY_DUMP") {
                    // The reference's mean then standard error, raw little-endian f32.
                    let raw: Vec<u8> = reference
                        .mean
                        .iter()
                        .chain(&reference.standard_error)
                        .flatten()
                        .flat_map(|x| x.to_le_bytes())
                        .collect();
                    std::fs::write(
                        PathBuf::from(dir).join(format!("reference{sky_tag}-pose{pose}.f32")),
                        raw,
                    )
                    .unwrap();
                }
                if variant == "hardware" {
                    png(
                        &out.join(format!("reference-pose{pose}.png")),
                        size,
                        reference.mean.iter().copied(),
                        scale,
                    );
                }
                png(
                    &out.join(format!("{path}-{variant}-pose{pose}-vs-reference-x8.png")),
                    size,
                    image
                        .iter()
                        .zip(&reference.mean)
                        .map(|(a, b)| std::array::from_fn(|k| (a[k] - b[k]).abs() * 8.0)),
                    scale,
                );
                eprintln!(
                    "G1 quarry {path} {variant} pose{pose}: vs reference rms {} dark rms {} bias {}",
                    pose_json["against_reference"]["frame"]["rms"],
                    pose_json["against_reference"]["dark"]["rms"],
                    pose_json["against_reference"]["frame"]["bias"]
                );
            }
            eprintln!(
                "G1 quarry {path} {variant} pose{pose}: vs RT rms {} dark rms {}",
                variant_json[format!("pose{pose}")]["against_rt"]["frame"]["rms"],
                variant_json[format!("pose{pose}")]["against_rt"]["dark"]["rms"]
            );
        }
        // RENDERLAB_M7_G1_QUARRY_CUTS=n: n warm cuts at pose 0 with the default settings
        // (`cut_phases` 0: the four-frame refill on software rays), each followed by still
        // frames. Per frame after the cut: the GPU span and the cache's pass times, median
        // over the cuts. Functional leases show the shape; a cost claim needs a quiet window.
        if let Some(cuts) = env("RENDERLAB_M7_G1_QUARRY_CUTS").map(|n| n.parse::<usize>().unwrap())
        {
            const FRAMES: usize = 10;
            let frame = &cameras.frames[0];
            let camera = frame.camera().unwrap();
            let target = color_target(&r, [frame.width, frame.height]);
            r.set_indirect(IndirectSettings {
                enabled: true,
                force_software: variant == "software-full",
                directions,
                cell_pixels,
                // The shipping refill unless RENDERLAB_M7_G1_QUARRY_SETTINGS names one (G1d).
                cut_phases: tuned["cut_phases"].as_u64().map_or(0, |p| p as u32),
                ..Default::default()
            })
            .unwrap();
            let mut span = vec![vec![]; FRAMES];
            let mut passes: Vec<Vec<IndirectProfile>> = vec![vec![]; FRAMES];
            for _ in 0..cuts {
                r.test_drain("G1 quarry cut");
                r.poll().unwrap();
                r.reset_indirect();
                let before = r.indirect_stats().frames;
                for _ in 0..FRAMES {
                    let mut draws: Vec<Draw<'_>> = asset
                        .draws(Visibility::Frustum)
                        .into_iter()
                        .map(|d| if full { d } else { d.with_material(&gate) })
                        .collect();
                    while r
                        .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                        .unwrap()
                        .is_none()
                    {
                        r.test_drain("G1 quarry cut admission");
                    }
                }
                r.test_drain("G1 quarry cut frames");
                let gpu = r.poll().unwrap().gpu;
                for (k, g) in gpu.iter().rev().take(FRAMES).rev().enumerate() {
                    span[k].push(g.render_span.as_secs_f64() * 1e3);
                }
                // Cache frames are the first ones after the cut; idle frames come last.
                let cache_frames = (r.indirect_stats().frames - before) as usize;
                let profiles = r.indirect_profiles().unwrap();
                let profiles = &profiles[profiles.len().saturating_sub(cache_frames)..];
                for (k, p) in profiles.iter().enumerate() {
                    passes[k].push(*p);
                }
            }
            let median = |mut v: Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v.get(v.len() / 2).copied()
            };
            let pass = |k: usize, f: fn(&IndirectProfile) -> f64| {
                median(passes[k].iter().map(f).collect())
            };
            variant_json["cuts"] = json!({
                "cuts": cuts, "pose": 0,
                "span_ms": span.iter().map(|v| median(v.clone())).collect::<Vec<_>>(),
                "cache_us": (0..FRAMES).map(|k| json!({"total": pass(k, |p| p.total()),
                    "probe": pass(k, |p| p.probe), "setup": pass(k, |p| p.setup),
                    "rays": pass(k, |p| p.rays), "finish": pass(k, |p| p.finish)})).collect::<Vec<_>>(),
            });
            eprintln!("G1 quarry {path} {variant} cuts: {}", variant_json["cuts"]);
        }
        // RENDERLAB_M7_G1_QUARRY_MOTION=n: n camera-moving frames from pose 0 (1 cm a frame
        // along x, as pose 1 steps), cache off then on (settled first), two visits each: the
        // frame span p50 over the last n-2 frames of each visit. Functional leases only.
        if let Some(n) = env("RENDERLAB_M7_G1_QUARRY_MOTION").map(|n| n.parse::<usize>().unwrap()) {
            let target = color_target(&r, [cameras.frames[0].width, cameras.frames[0].height]);
            let mut run = |r: &mut Renderer, step: f32, frames: usize| -> Vec<f64> {
                r.test_drain("G1 quarry motion");
                r.poll().unwrap();
                for t in 0..frames {
                    let mut pose = cameras.frames[0].clone();
                    pose.eye[0] += step * t as f32;
                    pose.target[0] += step * t as f32;
                    let camera = pose.camera().unwrap();
                    let mut draws: Vec<Draw<'_>> = asset
                        .draws(Visibility::Frustum)
                        .into_iter()
                        .map(|d| if full { d } else { d.with_material(&gate) })
                        .collect();
                    while r
                        .try_render_view(&target, &camera, &mut draws, ViewMode::Lit)
                        .unwrap()
                        .is_none()
                    {
                        r.test_drain("G1 quarry motion admission");
                    }
                }
                r.test_drain("G1 quarry motion frames");
                let mut spans: Vec<f64> = r
                    .poll()
                    .unwrap()
                    .gpu
                    .iter()
                    .rev()
                    .take(frames)
                    .rev()
                    .skip(2)
                    .map(|g| g.render_span.as_secs_f64() * 1e3)
                    .collect();
                spans.sort_by(f64::total_cmp);
                spans
            };
            let p50 = |v: &[f64]| v.get(v.len() / 2).copied();
            let mut motion = json!({"frames": n, "step_m": 0.01});
            for (state, enabled) in [("off", false), ("on", true)] {
                r.set_indirect(IndirectSettings {
                    enabled,
                    force_software: variant == "software-full",
                    directions,
                    cell_pixels,
                    ..Default::default()
                })
                .unwrap();
                r.set_indirect_profiling(false);
                r.reset_indirect();
                run(&mut r, 0.0, 10);
                let visits: Vec<Option<f64>> = (0..2).map(|_| p50(&run(&mut r, 0.01, n))).collect();
                motion[state] = json!(visits);
            }
            r.set_indirect_profiling(true);
            eprintln!("G1 quarry {path} {variant} motion: {motion}");
            variant_json["motion"] = motion;
        }
        // One frame from pose 0 moved `dx` metres along x (as pose 1 steps).
        let moving_target = color_target(&r, [cameras.frames[0].width, cameras.frames[0].height]);
        let mut draw_at = |r: &mut Renderer, dx: f32| {
            let mut pose = cameras.frames[0].clone();
            pose.eye[0] += dx;
            pose.target[0] += dx;
            let camera = pose.camera().unwrap();
            let target = &moving_target;
            let mut draws: Vec<Draw<'_>> = asset
                .draws(Visibility::Frustum)
                .into_iter()
                .map(|d| if full { d } else { d.with_material(&gate) })
                .collect();
            while r
                .try_render_view(target, &camera, &mut draws, ViewMode::Lit)
                .unwrap()
                .is_none()
            {
                r.test_drain("G1d quarry admission");
            }
        };
        let moving = IndirectSettings {
            enabled: true,
            force_software: variant == "software-full",
            directions,
            cell_pixels,
            ..tuned_settings(&tuned)
        };
        // RENDERLAB_M7_G1_QUARRY_BREAKDOWN=n (G1d): the camera-moving cost per stage
        // (`motion_breakdown`): n frames from pose 0 at 1 cm a frame along x, cache off, then
        // on with the default settings and RENDERLAB_M7_G1_QUARRY_SETTINGS, each settled
        // still at pose 0 first.
        if let Some(n) =
            env("RENDERLAB_M7_G1_QUARRY_BREAKDOWN").map(|n| n.parse::<usize>().unwrap())
        {
            let mut breakdown = json!({"step_m": 0.01});
            for (state, enabled) in [("off", false), ("on", true)] {
                r.set_indirect(IndirectSettings { enabled, ..moving })
                    .unwrap();
                r.reset_indirect();
                for _ in 0..10 {
                    draw_at(&mut r, 0.0);
                }
                breakdown[state] =
                    motion_breakdown(&mut r, |r| r, n, 2, |r, t| draw_at(r, 0.01 * t as f32));
            }
            eprintln!(
                "G1d quarry {path} {variant} breakdown: on {} off {}",
                breakdown["on"]["timed"], breakdown["off"]["timed"]
            );
            variant_json["breakdown"] = breakdown;
        }
        // RENDERLAB_M7_G1_QUARRY_TRACE=n (G1d): n camera frames moving back and forth from
        // pose 0 at 1 cm a frame, cache on and unprofiled, for an Nsight GPU trace window inside
        // camera motion (the motion's start is printed in process seconds). Each sweep is
        // RENDERLAB_M7_G1_QUARRY_SWEEP frames (default 24): short sweeps revisit existing cells
        // (the probe and lookup alone); sweeps longer than the eviction age keep creating cells.
        // RENDERLAB_M7_G1_QUARRY_TRACE_CUTS=k resets the cache every k frames (cut frames).
        if let Some(n) = env("RENDERLAB_M7_G1_QUARRY_TRACE").map(|n| n.parse::<usize>().unwrap()) {
            r.set_indirect(moving).unwrap();
            r.set_indirect_profiling(false);
            r.reset_indirect();
            for _ in 0..10 {
                draw_at(&mut r, 0.0);
            }
            eprintln!(
                "G1d trace motion starts at {:.1} s",
                start.elapsed().as_secs_f64()
            );
            let sweep: usize =
                env("RENDERLAB_M7_G1_QUARRY_SWEEP").map_or(24, |s| s.parse().unwrap());
            let cuts: Option<usize> =
                env("RENDERLAB_M7_G1_QUARRY_TRACE_CUTS").map(|s| s.parse().unwrap());
            for t in 0..n {
                if cuts.is_some_and(|k| t % k == 0) {
                    r.reset_indirect();
                }
                let k = t % (2 * sweep);
                draw_at(&mut r, 0.01 * k.min(2 * sweep - k) as f32);
                if t % 256 == 0 {
                    r.collect().unwrap();
                    r.poll().unwrap();
                }
            }
            eprintln!(
                "G1d trace motion ends at {:.1} s",
                start.elapsed().as_secs_f64()
            );
            r.set_indirect_profiling(true);
        }
        results["variants"][variant] = variant_json;
        std::fs::write(
            out.join("g1-quarry.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
    results["seconds"] = json!(start.elapsed().as_secs_f64());
    std::fs::write(
        out.join("g1-quarry.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}

/// Cache settings from RENDERLAB_M7_G1_QUARRY_SETTINGS beyond `directions`, `cut_phases`
/// and `cell_pixels`: `slots_log2`, `level_blend` and `lookup` ("nearest"), over the defaults.
fn tuned_settings(tuned: &Json) -> IndirectSettings {
    let base = IndirectSettings::default();
    IndirectSettings {
        slots_log2: tuned["slots_log2"]
            .as_u64()
            .map_or(base.slots_log2, |v| v as u32),
        level_blend: tuned["level_blend"]
            .as_f64()
            .map_or(base.level_blend, |v| v as f32),
        lookup: if tuned["lookup"].as_str() == Some("nearest") {
            IndirectLookup::Nearest
        } else {
            base.lookup
        },
        ..base
    }
}

/// Software rays report world-space hit normals through the instance's inverse transpose,
/// as hardware ray queries do. SV's placements are translations only, so this rotated,
/// non-uniformly scaled instance is what checks the transform.
#[test]
#[ignore = "GPU; shared GPU lease"]
fn gpu_software_hit_normals_follow_instance_transforms() {
    let (device, queue) = no_rt_device();
    // One triangle in the object's xz plane; its source-winding normal is -y.
    let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Z);
    let blas = gi_bvh::Blas::new(
        gi_bvh::build(&[a.into(), b.into(), c.into()], &[0, 1, 2]).unwrap(),
        0,
    )
    .unwrap();
    let model = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 0.5, 3.0),
        glam::Quat::from_euler(glam::EulerRot::XYZ, 0.7, 0.3, -0.4),
        Vec3::new(1.0, 2.0, -1.0),
    );
    let (offsets, near, total) = gi_bvh::layout(&[&blas], 1);
    let mut words = gi_bvh::mesh_words(&[&blas], &offsets, &near, total);
    let head = gi_bvh::instance_words(&[(model, &blas, offsets[0], 0)]).unwrap();
    words[..head.len()].copy_from_slice(&head);
    let normal = model
        .inverse()
        .transpose()
        .transform_vector3((b - a).cross(c - a))
        .normalize();
    let target = model.transform_point3((a + b + c) / 3.0);
    let origin = target + 2.0 * normal + 0.3 * normal.any_orthonormal_vector();
    let direction = (target - origin).normalize();
    let entry = format!(
        "@group(0) @binding(0) var<storage,read_write> gi_out:array<f32,5>;
@compute @workgroup_size(1) fn main() {{
    let h=gi_closest(vec3<f32>({},{},{}),vec3<f32>({},{},{}),0.0,1e30);
    gi_out[0]=select(0.0,1.0,h.hit);gi_out[1]=h.t;
    gi_out[2]=h.normal.x;gi_out[3]=h.normal.y;gi_out[4]=h.normal.z;
}}",
        origin.x, origin.y, origin.z, direction.x, direction.y, direction.z
    );
    let source = [
        "fn unit_normal(n:vec3<f32>) -> vec3<f32> { return normalize(n); }",
        include_str!("../shaders/gi_rays_software.wgsl"),
        &entry,
    ]
    .join("\n");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("G1b software hit normal"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("G1b software hit normal"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let storage = |size: u64, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let geometry = storage(
        (words.len() * 4) as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    queue.write_buffer(&geometry, 0, &raw);
    let out = storage(
        20,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let read = storage(
        20,
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    );
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 35,
                resource: geometry.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &read, 0, 20);
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(WAIT).unwrap();
    let got: Vec<f32> = slice
        .get_mapped_range()
        .unwrap()
        .chunks_exact(4)
        .map(|w| f32::from_le_bytes(w.try_into().unwrap()))
        .collect();
    assert_eq!(got[0], 1.0, "the ray must hit the instance");
    assert!(
        (got[1] - (target - origin).length()).abs() < 1e-4,
        "t {}",
        got[1]
    );
    let n = Vec3::new(got[2], got[3], got[4]);
    assert!(n.dot(normal) > 0.9999, "normal {n} against {normal}");
}

// ---- M7 G2: bounce light on the registered quarry (docs/m7-g2.md) ----
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../examples/m4/cameras.rs"]
mod g2_cameras;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../viewer/src/prepared.rs"]
mod g2_prepared;

/// RENDERLAB_G2_PATH: "rt" (hardware rays, traced primaries) or "nonrt" (a device without ray
/// queries, visibility-buffer primaries).
fn g2_nonrt() -> bool {
    env("RENDERLAB_G2_PATH").as_deref() == Some("nonrt")
}
/// The variants a G2 quarry process renders, in RENDERLAB_G2_VARIANTS order (default
/// "off,g2,es1"): the cache off (the default image), the cache with bounce (the default
/// candidate), and the cache without bounce (ES1's image on this code; "es1open" is the same
/// settings in a process whose shaders hold ES1's unoccluded diagnostic). The names are the
/// capture prefixes scripts/m7-g2-score.py reads.
fn g2_variant(name: &str) -> IndirectSettings {
    // G2's own settings (4 px cells, 2^19 slots, every blocked direction, three gathers, no
    // gather cap): its default candidate and ES1's comparators keep their G2 meaning now that
    // the defaults changed (G2b).
    let g2 = IndirectSettings {
        enabled: true,
        cell_pixels: 4.0,
        slots_log2: 19,
        gathers: 3,
        gather_stride: 1,
        gather_cells: u32::MAX,
        claim_cells: u32::MAX,
        ..Default::default()
    };
    match name {
        "off" => IndirectSettings::off(),
        "g2" => g2,
        "es1" | "es1open" => IndirectSettings { bounce: false, ..g2 },
        // G2b's default: the cache as it now ships.
        "g2b" => IndirectSettings::default(),
        // G2b screens: the cache with bounce, changed by '+'-joined tokens: pxF (cell edge in
        // pixels of footprint), sN (log2 slots), capN (cells gathered a frame), gN (gathers an
        // event schedules). "c2" and "c3" are 2 and 3 px cells in a table of 2^21 slots, to see
        // how much of the crevice-mouth darkness cell size alone explains (docs/m7-g2b.md).
        "c2" => g2_variant("g2+px2+s21"),
        "c3" => g2_variant("g2+px3+s21"),
        other if other.starts_with("g2+") || other.starts_with("g2b+") => {
            let mut s = g2_variant(other.split('+').next().unwrap());
            for token in other.split('+').skip(1) {
                if token == "nocap" {
                    s.gather_cells = u32::MAX;
                    s.claim_cells = u32::MAX;
                    continue;
                }
                let (tag, value) = token.split_at(token.find(|c: char| c.is_ascii_digit()).unwrap_or(0));
                match tag {
                    "px" => s.cell_pixels = value.parse().unwrap(),
                    "s" => s.slots_log2 = value.parse().unwrap(),
                    "cap" => s.gather_cells = value.parse().unwrap(),
                    "g" => s.gathers = value.parse().unwrap(),
                    "gs" => s.gather_stride = value.parse().unwrap(),
                    "cc" => s.claim_cells = value.parse().unwrap(),
                    _ => panic!("unknown G2b variant token {token}"),
                }
            }
            s
        }
        other => panic!("unknown G2 variant {other}"),
    }
}
/// The registered quarry under U0b's full-radiance protocol, as ES1 measured it
/// (docs/m7-es1.md 4.1): the quarry's own materials, the RLE2 sky, the sun on the 4/12/40 m
/// splits, a black backdrop, exposure -14; Interactive shadows and traced primaries on RT,
/// Virtual shadows and the visibility buffer with G1c's cooked near field on non-RT. One
/// path per process. RENDERLAB_G2_QUARRY: ES1's manifest (asset, cameras, environment,
/// lighting, near). RENDERLAB_G2_SECTIONS, in order (default static,pan):
/// - static: poses 0 and 1 per variant, a reset and twelve frames, then TR1 over 8 and 64
///   still frames (RENDERLAB_G2_TR1);
/// - pan: per variant, settled at pose 0, then U0b's pan at 2 cm a frame, captured at its
///   witness frames 97, 112, 128 and 159; pan-tr1 the same with TR1 on from the cut;
/// - determinism: the first variant's reset sequence twice, images and tables by process id;
/// - settle: the first variant after a reset at pose 0, captured at frames 1-8 and 12;
/// - sun: settled at pose 0, the sun turned 10 degrees, captured at frames 1, 2, 4 and 8 after
///   the step, then a reset under the new sun settled (the fresh image);
/// - memory: per variant after a settled pose 0, the allocator, HAL and ownership totals;
/// - cost: every variant in each of five visits (the last profiled): a cut of twelve frames,
///   then 24 frames moving 1 cm a frame; frame and shading spans, the profiled visit's
///   passes and the kernels' counters.
///
/// Captures are raw little-endian f32 radiance (VARIANT-pP, VARIANT-pan-FFF, ...) for the CPU
/// scorer; nothing is scored here. Timing here is functional unless the lease is quiet.
#[test]
#[ignore = "GPU lease; RENDERLAB_G2_PATH, RENDERLAB_G2_QUARRY, RENDERLAB_G2_OUT"]
fn gpu_g2_quarry() {
    let start = Instant::now();
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(env("RENDERLAB_G2_QUARRY").unwrap()).unwrap())
            .unwrap();
    let file = |k: &str| PathBuf::from(manifest[k].as_str().unwrap());
    let path = if g2_nonrt() { "nonrt" } else { "rt" };
    let out = PathBuf::from(env("RENDERLAB_G2_OUT").unwrap())
        .join(env("RENDERLAB_G2_TAG").map_or(path.to_string(), |t| format!("{path}-{t}")));
    std::fs::create_dir_all(&out).unwrap();
    let sections: Vec<String> = env("RENDERLAB_G2_SECTIONS")
        .unwrap_or_else(|| "static,pan".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let variants: Vec<String> = env("RENDERLAB_G2_VARIANTS")
        .unwrap_or_else(|| "off,g2,es1".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let source = crate::asset::decode(
        &std::fs::read(file("asset")).unwrap(),
        crate::asset::Limits::unlimited(),
    )
    .unwrap();
    let cameras = g2_cameras::Cameras::load(&file("cameras")).unwrap();
    let map = crate::asset::environment::decode(
        &std::fs::read(file("environment")).unwrap(),
        crate::asset::environment::Limits::unbounded(),
    )
    .unwrap();
    let lighting: Json = serde_json::from_slice(&std::fs::read(file("lighting")).unwrap()).unwrap();
    let v3 = |v: &Json| Vec3::from_array(std::array::from_fn(|k| v[k].as_f64().unwrap() as f32));
    let (device, queue) = if g2_nonrt() {
        no_rt_device_with(true)
    } else {
        rt_device()
    };
    let options = RendererOptions {
        samples: SampleCount::One,
        ..Default::default()
    };
    let limits = Limits {
        max_mesh_bytes: 134217728,
        max_texture_bytes: 134217728,
        ..Default::default()
    };
    let mut r = if g2_nonrt() {
        Renderer::new_with_options(device, queue, FORMAT, limits, options, SOURCE).unwrap()
    } else {
        let mut r = Renderer::new_with_hardware_shadows(
            device,
            queue,
            FORMAT,
            limits,
            options,
            HardwareShadowOptions {
                max_estimated_bytes: Some(8 * 1024 * 1024 * 1024),
            },
            SOURCE,
        )
        .unwrap();
        r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        r
    };
    let exposure = -14.0;
    let travel = v3(&lighting["lights"][0]["travel"]);
    let lux = v3(&lighting["lights"][0]["lux"]).to_array();
    let set_sun = |r: &mut Renderer, travel: Vec3| {
        r.set_lighting_and_shadows(
            &Lighting {
                lights: vec![Light::Directional { travel, lux }],
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
    };
    set_sun(&mut r, travel);
    if !g2_nonrt() {
        r.set_soft_shadows(SoftShadows::Interactive);
    }
    let sky = loop {
        if let Some(sky) = r.try_upload_environment(&map).unwrap() {
            break sky;
        }
        r.test_drain("G2 quarry environment");
    };
    r.set_environment(Some(&sky), Default::default()).unwrap();
    drop(map);
    let mut asset = g2_prepared::Prepared::new_with_limits(
        source,
        crate::asset::Limits::unlimited(),
        1_000_000,
    )
    .unwrap();
    while !asset.ready() {
        asset.advance(&mut r).unwrap();
        r.collect().unwrap();
    }
    r.test_drain("G2 quarry upload");
    if g2_nonrt() {
        let field = crate::asset::near_field::decode(
            &std::fs::read(file("near")).unwrap(),
            crate::asset::near_field::Limits::default(),
        )
        .unwrap();
        for (m, mesh) in asset.meshes_mut().iter().enumerate() {
            r.set_indirect_near_field(mesh, &field, m).unwrap();
        }
    }
    // RENDERLAB_G2_PROGRAM: a diagnostic program in place of the standard one.
    if let Some(program) = env("RENDERLAB_G2_PROGRAM") {
        r.test_reload_standard(&std::fs::read_to_string(program).unwrap())
            .unwrap();
    }
    let frame0 = &cameras.frames[0];
    let target = color_target(&r, [frame0.width, frame0.height]);
    let camera_at = |pose: usize, dx: f32| {
        let mut f = cameras.frames[pose].clone();
        f.eye[0] += dx;
        f.target[0] += dx;
        f.camera().unwrap()
    };
    let write = |name: &str, image: &[[f32; 3]]| {
        let raw: Vec<u8> = image
            .iter()
            .flatten()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        std::fs::write(out.join(format!("{name}.f32")), raw).unwrap();
    };
    let mut draw = |r: &mut Renderer, camera: &Camera| {
        let mut draws = asset.draws(Visibility::Frustum);
        let until = Instant::now() + Duration::from_secs(60);
        while r
            .try_render_view(&target, camera, &mut draws, ViewMode::Lit)
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < until, "G2 frame admission");
            r.test_drain("G2 admission");
        }
    };
    let counters = |r: &mut Renderer| {
        r.indirect_counters().unwrap().map_or(Json::Null, |c| {
            json!({"created": c.created, "traces": c.traces, "rays": c.rays, "shades": c.shades,
                "gathers": c.gathers, "gather_rays": c.gather_rays, "gather_misses": c.gather_misses,
                "gather_requests": c.gather_requests, "light_rays": c.light_rays,
                "bounce_queued": c.bounce_queued, "orphans": c.orphans, "full": c.full,
                "miss_overflow": c.miss_overflow, "lost": c.lost, "gather_absent": c.gather_absent,
                "gather_unshaded": c.gather_unshaded, "gather_orphans": c.gather_orphans,
                "gather_deferred": c.gather_deferred, "claim_deferred": c.claim_deferred,
                "misses": c.misses, "splits": c.splits,
                "local_overflow": c.local_overflow, "items": c.ray_items})
        })
    };
    let tr1: Vec<u32> = env("RENDERLAB_G2_TR1")
        .unwrap_or_else(|| "8,64".into())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    let mut results = json!({"path": path, "variants": variants, "sections": sections, "tr1": tr1});
    // One file per process, named by its sections, so the steps of one lease that share a
    // path's directory keep their results apart.
    let json_out = out.join(format!("quarry-{}.json", sections.join("_")));
    for section in &sections {
        match section.as_str() {
            "static" => {
                for v in &variants {
                    r.set_indirect(g2_variant(v)).unwrap();
                    for pose in 0..2 {
                        let camera = camera_at(pose, 0.0);
                        r.reset_indirect();
                        for _ in 0..12 {
                            draw(&mut r, &camera);
                        }
                        write(&format!("{v}-p{pose}"), &capture_radiance(&mut r, exposure));
                        results["counters"][format!("{v}-p{pose}")] = counters(&mut r);
                        // RENDERLAB_G2_DUMP=1: the table and outgoing radiance after pose 0
                        // (diagnostics; tens of MB, deleted once summarized).
                        if pose == 0 && env("RENDERLAB_G2_DUMP").as_deref() == Some("1") {
                            if let Some(words) = r.indirect_table().unwrap() {
                                let raw: Vec<u8> =
                                    words.iter().flat_map(|w| w.to_le_bytes()).collect();
                                std::fs::write(out.join(format!("{v}-table.u32")), raw).unwrap();
                            }
                            if let Some(o) = r.indirect_outgoing().unwrap() {
                                let raw: Vec<u8> =
                                    o.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
                                std::fs::write(out.join(format!("{v}-outgoing.f32")), raw).unwrap();
                            }
                        }
                        // The shipping image (TR1 the default anti-aliasing, milestone-7.md):
                        // TR1's measured settings over N still frames of the settled cache,
                        // as IQ1's tr1xN (RENDERLAB_G2_TR1, default 8 and 64; empty: none).
                        for n in &tr1 {
                            r.set_reconstruction(Reconstruction::identity()).unwrap();
                            for _ in 0..*n {
                                draw(&mut r, &camera);
                            }
                            write(
                                &format!("{v}-tr1x{n}-p{pose}"),
                                &capture_radiance(&mut r, exposure),
                            );
                            r.set_reconstruction(Reconstruction::default()).unwrap();
                        }
                    }
                }
            }
            "pan" | "pan-tr1" => {
                // U0b's pan: frame f >= 96 is pose 0 moved 2 cm a frame in x. pan-tr1 runs it
                // with TR1 on throughout, from the cut.
                let tr1_on = section == "pan-tr1";
                let tag = if tr1_on { "pantr1" } else { "pan" };
                for v in &variants {
                    r.set_indirect(g2_variant(v)).unwrap();
                    if tr1_on {
                        r.set_reconstruction(Reconstruction::identity()).unwrap();
                    }
                    r.reset_indirect();
                    for _ in 0..12 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    for f in 97..=159 {
                        draw(&mut r, &camera_at(0, 0.02 * (f - 96) as f32));
                        if [97, 112, 128, 159].contains(&f) {
                            write(
                                &format!("{v}-{tag}-{f:03}"),
                                &capture_radiance(&mut r, exposure),
                            );
                        }
                    }
                    r.set_reconstruction(Reconstruction::default()).unwrap();
                }
            }
            "determinism" => {
                // The same reset sequence twice in this process: image and table hashes (the
                // table without its per-frame header), and both tables tagged by process id, so
                // two processes' tables compare word by word (G1d's cross-process difference).
                let v = &variants[0];
                let mut rows = vec![];
                for rep in 0..2 {
                    r.set_indirect(g2_variant(v)).unwrap();
                    r.reset_indirect();
                    for _ in 0..12 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    let image = capture_radiance(&mut r, exposure);
                    let words = r.indirect_table().unwrap().unwrap_or_default();
                    let body = &words[words.len().min(64)..];
                    let pid = std::process::id();
                    let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                    std::fs::write(out.join(format!("{v}-det{rep}-{pid}-table.u32")), raw).unwrap();
                    write(&format!("{v}-det{rep}-{pid}"), &image);
                    rows.push(json!({"rep": rep, "pid": pid, "image": hash_image(&image), "table": hash_words(body)}));
                }
                results["determinism"] = json!(rows);
            }
            "settle" => {
                let v = &variants[0];
                r.set_indirect(g2_variant(v)).unwrap();
                r.reset_indirect();
                let mut rows = vec![];
                for f in 1..=12 {
                    draw(&mut r, &camera_at(0, 0.0));
                    if f <= 8 || f == 12 {
                        write(
                            &format!("{v}-settle-{f:02}"),
                            &capture_radiance(&mut r, exposure),
                        );
                        rows.push(json!({"frame": f, "counters": counters(&mut r)}));
                    }
                }
                results["settle"] = json!(rows);
            }
            "sun" => {
                // A 10 degree turn of the sun about the vertical axis at a settled view.
                let v = &variants[0];
                r.set_indirect(g2_variant(v)).unwrap();
                r.reset_indirect();
                for _ in 0..12 {
                    draw(&mut r, &camera_at(0, 0.0));
                }
                let turned = Mat4::from_rotation_y(10f32.to_radians()).transform_vector3(travel);
                set_sun(&mut r, turned);
                for f in 1..=8 {
                    draw(&mut r, &camera_at(0, 0.0));
                    if [1, 2, 4, 8].contains(&f) {
                        write(
                            &format!("{v}-sun-{f:02}"),
                            &capture_radiance(&mut r, exposure),
                        );
                    }
                }
                r.reset_indirect();
                for _ in 0..12 {
                    draw(&mut r, &camera_at(0, 0.0));
                }
                write(
                    &format!("{v}-sun-fresh"),
                    &capture_radiance(&mut r, exposure),
                );
                set_sun(&mut r, travel);
            }
            "memory" => {
                // Per variant after a settled pose 0: the allocator's allocated and reserved
                // bytes, the HAL's buffer and texture bytes, and the ownership ledger.
                let mut rows = json!({});
                for v in &variants {
                    r.set_indirect(g2_variant(v)).unwrap();
                    r.reset_indirect();
                    for _ in 0..12 {
                        draw(&mut r, &camera_at(0, 0.0));
                    }
                    r.test_drain("G2 memory");
                    let s = r.stats();
                    rows[v.as_str()] = json!({
                        "allocated": s.allocator.as_ref().map(|a| a.allocated_bytes),
                        "reserved": s.allocator.as_ref().map(|a| a.reserved_bytes),
                        "hal_buffer_bytes": s.hal.buffer_bytes, "hal_texture_bytes": s.hal.texture_bytes,
                        "ownership": format!("{:?}", s.ownership)});
                }
                results["memory"] = rows;
            }
            "cost" => {
                let mut timing = json!({});
                for visit in 0..5 {
                    // The last visit is profiled: per-pass times and counters.
                    let profiled = visit == 4;
                    r.set_indirect_profiling(profiled);
                    for v in &variants {
                        r.set_indirect(g2_variant(v)).unwrap();
                        r.test_drain("G2 cost");
                        r.poll().unwrap();
                        r.reset_indirect();
                        for _ in 0..12 {
                            draw(&mut r, &camera_at(0, 0.0));
                        }
                        r.test_drain("G2 cut frames");
                        let cut = g2_frame_times(&mut r, 12, 0);
                        // RENDERLAB_G2_COST_SETTLE (G2b): still frames before the motion, so a
                        // gather cap's spread cut work has drained out of the moving frames.
                        let settle: usize =
                            env("RENDERLAB_G2_COST_SETTLE").map_or(0, |n| n.parse().unwrap());
                        for _ in 0..settle {
                            draw(&mut r, &camera_at(0, 0.0));
                        }
                        for t in 0..24 {
                            draw(&mut r, &camera_at(0, 0.01 * t as f32));
                        }
                        r.test_drain("G2 motion frames");
                        let motion = g2_frame_times(&mut r, 24, 2);
                        let slot = &mut timing[v.as_str()];
                        if slot.is_null() {
                            *slot = json!({"cuts": [], "motion": []});
                        }
                        if profiled {
                            let profiles = r.indirect_profiles().unwrap();
                            let tail = &profiles[profiles.len().saturating_sub(22)..];
                            let pass = |f: fn(&IndirectProfile) -> f64| {
                                quantiles(tail.iter().map(f).collect())
                            };
                            slot["passes_us"] = json!({
                                "total": pass(|p| p.total()), "probe": pass(|p| p.probe),
                                "claim": pass(|p| p.claim + p.resolve), "scan": pass(|p| p.scan),
                                "setup": pass(|p| p.setup), "rays": pass(|p| p.rays),
                                "finish": pass(|p| p.finish), "gather": pass(|p| p.gather),
                                "light": pass(|p| p.light), "shade": pass(|p| p.shade),
                            });
                            slot["counters"] = counters(&mut r);
                        } else {
                            slot["cuts"].as_array_mut().unwrap().push(cut);
                            slot["motion"].as_array_mut().unwrap().push(motion);
                        }
                    }
                }
                r.set_indirect_profiling(false);
                results["cost"] = timing;
            }
            "trace" => {
                // G2b: an Nsight GPU Trace window inside camera motion. The first variant settles
                // at pose 0, then RENDERLAB_G2_TRACE frames (default 30,000) move back and forth
                // 1 cm a frame over sweeps of RENDERLAB_G2_SWEEP frames (default 24: existing
                // cells revisited, so the probe, gathers of new cells and the lookup), unprofiled.
                // The motion's start is printed in process seconds, for ngfx --start-after-ms.
                let frames: usize = env("RENDERLAB_G2_TRACE").map_or(30_000, |f| f.parse().unwrap());
                let sweep: usize = env("RENDERLAB_G2_SWEEP").map_or(24, |f| f.parse().unwrap());
                r.set_indirect(g2_variant(&variants[0])).unwrap();
                r.set_indirect_profiling(false);
                r.reset_indirect();
                for _ in 0..12 {
                    draw(&mut r, &camera_at(0, 0.0));
                }
                eprintln!("G2b trace motion starts at {:.1} s", start.elapsed().as_secs_f64());
                for t in 0..frames {
                    let k = t % (2 * sweep);
                    draw(&mut r, &camera_at(0, 0.01 * k.min(2 * sweep - k) as f32));
                    if t % 256 == 0 {
                        r.collect().unwrap();
                        r.poll().unwrap();
                    }
                }
                eprintln!("G2b trace motion ends at {:.1} s", start.elapsed().as_secs_f64());
            }
            "cutprofile" => {
                // G2b: each variant's cut at pose 0, profiled: per frame (1-N, N from
                // RENDERLAB_G2_CUT_FRAMES, default 24) the pass times, the frame span and the
                // kernels' counters. The counter readback after each frame waits for the GPU, so
                // spans here are per-frame GPU spans, not a pipelined frame rate.
                let frames: usize = env("RENDERLAB_G2_CUT_FRAMES").map_or(24, |f| f.parse().unwrap());
                let mut rows = json!({});
                r.set_indirect_profiling(true);
                for v in &variants {
                    r.set_indirect(g2_variant(v)).unwrap();
                    // A settled cache first, so the cut's frame 1 runs warm (G1d section 5).
                    r.reset_indirect();
                    for _ in 0..12 {
                        draw(&mut r, &camera_at(1, 0.0));
                    }
                    r.test_drain("G2b cut warm-up");
                    // A fresh profile ring, so its entries are the cut's frames (idle frames add
                    // none: the passes stop 16 identical frames after the cut).
                    r.set_indirect_profiling(false);
                    r.set_indirect_profiling(true);
                    r.reset_indirect();
                    let mut counts = vec![];
                    let mut span_ms = vec![];
                    let mut shading_ms = vec![];
                    for _ in 0..frames {
                        draw(&mut r, &camera_at(0, 0.0));
                        counts.push(counters(&mut r));
                        // The counter readback waited for this frame: its GPU sample is the
                        // newest (the renderer keeps only the last few).
                        let gpu = r.poll().unwrap().gpu;
                        span_ms.push(gpu.last().map_or(f64::NAN, |g| g.render_span.as_secs_f64() * 1e3));
                        shading_ms.push(gpu.last().map_or(f64::NAN, |g| g.mesh_pass.as_secs_f64() * 1e3));
                    }
                    let spans = json!({"span_ms": span_ms, "shading_ms": shading_ms});
                    let profiles = r.indirect_profiles().unwrap();
                    let passes: Vec<_> = profiles
                        .iter()
                        .map(|p| json!({"total": p.total(), "probe": p.probe, "claim": p.claim + p.resolve,
                            "scan": p.scan, "setup": p.setup, "rays": p.rays, "finish": p.finish,
                            "gather": p.gather, "light": p.light, "shade": p.shade}))
                        .collect();
                    rows[v.as_str()] = json!({"spans": spans, "passes_us": passes, "counters": counts});
                }
                r.set_indirect_profiling(false);
                results["cutprofile"] = rows;
            }
            other => panic!("unknown G2 section {other}"),
        }
        results["stats"] = json!(format!("{:?}", r.indirect_stats()));
        std::fs::write(&json_out, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    results["seconds"] = json!(start.elapsed().as_secs_f64());
    std::fs::write(&json_out, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
}
/// The last `frames` GPU samples after skipping `skip`: per frame, the render span and the
/// shading pass in milliseconds.
fn g2_frame_times(r: &mut Renderer, frames: usize, skip: usize) -> Json {
    let gpu = r.poll().unwrap().gpu;
    let rows: Vec<_> = gpu.iter().rev().take(frames).rev().skip(skip).collect();
    json!({
        "span_ms": rows.iter().map(|g| g.render_span.as_secs_f64() * 1e3).collect::<Vec<_>>(),
        "shading_ms": rows.iter().map(|g| g.mesh_pass.as_secs_f64() * 1e3).collect::<Vec<_>>(),
    })
}
