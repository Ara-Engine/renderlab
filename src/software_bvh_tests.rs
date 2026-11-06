//! Unit N0 of docs/non-rt-path.md: the software BVH traversal probe.
//!
//! CPU tests cover the builder, the reference traversal and the WGSL composition. One
//! batched GPU test loads the quarry once, checks a fixture ray kernel first, then compares
//! the three software kernels with the hardware ray-query path at poses 0, 1 and return:
//! hit records by f64-replay classification, 8-ray and Exact 256-ray visibility by mask,
//! and finally times every kernel in paired, alternating dispatches. With `cpu_only` in the
//! manifest it instead runs the quarry CPU checks (invariants, brute force) without a GPU.
//! Nested under primary_ray_gpu_tests for its quarry, renderer and readback helpers.
use super::*;
use crate::software_bvh::{self as bvh, Bvh, Mode, Ray, Triangles};
use glam::{DMat4, DVec2, DVec3, DVec4};
use std::collections::BTreeMap;
use wgpu::util::DeviceExt;

/// splitmix64.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 16_777_216.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn direction(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.unit(), self.unit(), self.unit()) * 2.0 - 1.0;
            let l = v.length_squared();
            if l > 1e-4 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
    }
}

type MeshData32 = (Vec<[f32; 3]>, Vec<u32>);

fn tris(mesh: &MeshData32) -> Triangles<'_> {
    Triangles {
        positions: &mesh.0,
        indices: &mesh.1,
    }
}

fn scale(bvh: &Bvh) -> f32 {
    bvh.bounds
        .lo
        .abs()
        .max(bvh.bounds.hi.abs())
        .max_element()
        .max(1.0)
}

/// Half the rays aim at random triangle points (a fifth of those exactly at a vertex or on
/// an edge); half start anywhere in the widened bounds with a random direction.
fn random_rays(tris: Triangles<'_>, bounds: bvh::Aabb, n: usize, rng: &mut Rng) -> Vec<Ray> {
    let extent = bounds.hi - bounds.lo;
    let size = extent.max_element().max(1e-3);
    (0..n)
        .map(|i| {
            let origin;
            let direction;
            if i % 2 == 0 {
                let v = tris.vertices(rng.below(tris.len()) as u32);
                let (a, b) = (rng.unit(), rng.unit());
                let w = match rng.below(10) {
                    0 => [1.0, 0.0, 0.0],
                    1 => [a, 1.0 - a, 0.0],
                    _ => {
                        let s = a.sqrt();
                        [1.0 - s, s * (1.0 - b), s * b]
                    }
                };
                let p = v[0] * w[0] + v[1] * w[1] + v[2] * w[2];
                origin = p + rng.direction() * size * (0.001 + rng.unit());
                direction = (p - origin).normalize();
            } else {
                let u = Vec3::new(rng.unit(), rng.unit(), rng.unit());
                origin = bounds.lo - extent * 0.1 + extent * 1.2 * u;
                direction = rng.direction();
            }
            Ray {
                origin,
                direction,
                tmin: 0.0,
                tmax: 1e30,
            }
        })
        .collect()
}

/// Any-hit ray of `ray` ending at a fraction of its closest hit (or unbounded on a miss).
fn shadow_of(ray: &Ray, closest: Option<bvh::Hit>, fraction: f32) -> Ray {
    Ray {
        tmax: closest.map_or(1e30, |h| h.t * fraction),
        ..*ray
    }
}

/// Rays whose BVH and brute-force results differ: closest (distance, primitive and
/// barycentrics) and any-hit with tmax at 0.5-1.5 of the closest distance, including 1.0.
fn cpu_mismatches(
    bvh: &Bvh,
    tris: Triangles<'_>,
    brute: &bvh::Brute,
    rays: &[Ray],
    threads: usize,
) -> Vec<usize> {
    let chunk = rays.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = rays
            .chunks(chunk)
            .enumerate()
            .map(|(c, rays)| {
                s.spawn(move || {
                    let mut bad = vec![];
                    for (i, ray) in rays.iter().enumerate() {
                        let fast = bvh.trace(tris, ray, Mode::Closest);
                        let slow = brute.trace(tris, ray, Mode::Closest);
                        let shadow = shadow_of(ray, fast, 0.5 + (i % 7) as f32 / 6.0);
                        let any = bvh.trace(tris, &shadow, Mode::Any).is_some();
                        if fast != slow || any != brute.trace(tris, &shadow, Mode::Any).is_some() {
                            bad.push(c * chunk + i);
                        }
                    }
                    bad
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

fn soup(rng: &mut Rng, n: usize) -> MeshData32 {
    let mut p = vec![];
    for _ in 0..n {
        let c = Vec3::new(rng.unit(), rng.unit(), rng.unit()) * 10.0;
        let s = 0.02 + rng.unit() * 0.5;
        for _ in 0..3 {
            p.push((c + rng.direction() * s).to_array());
        }
    }
    let i = (0..p.len() as u32).collect();
    (p, i)
}

/// A bumpy n x n grid of shared-vertex quads in the z = 0 neighbourhood.
fn grid(rng: &mut Rng, n: u32) -> MeshData32 {
    let mut p = vec![];
    for y in 0..=n {
        for x in 0..=n {
            p.push([x as f32 * 0.37, y as f32 * 0.29, (rng.unit() - 0.5) * 0.05]);
        }
    }
    let mut i = vec![];
    for y in 0..n {
        for x in 0..n {
            let a = y * (n + 1) + x;
            let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
            i.extend([a, b, d, a, d, c]);
        }
    }
    (p, i)
}

/// Forty copies of one triangle (equal centroids), colinear and point triangles.
fn stacked() -> MeshData32 {
    let mut p = vec![];
    for _ in 0..40 {
        p.extend([[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]]);
    }
    for k in 0..20 {
        let x = k as f32 * 0.1;
        p.extend([[x, 0.0, 0.0], [x + 0.05, 0.05, 0.0], [x + 0.1, 0.1, 0.0]]);
    }
    for _ in 0..10 {
        p.extend([[0.5, 0.5, 0.5]; 3]);
    }
    let i = (0..p.len() as u32).collect();
    (p, i)
}

#[test]
fn software_bvh_matches_brute_force_on_fixtures() {
    let mut rng = Rng(0x5eed);
    let meshes = [
        ("soup", soup(&mut rng, 3000)),
        ("grid", grid(&mut rng, 40)),
        ("stacked", stacked()),
    ];
    for (name, mesh) in &meshes {
        let t = tris(mesh);
        for (max_leaf, pad) in [(1, 16.0), (4, 16.0), (4, 4.0)] {
            let b = bvh::build(t, max_leaf, pad);
            b.check(t, max_leaf)
                .unwrap_or_else(|e| panic!("{name} leaf {max_leaf}: {e}"));
            let brute = bvh::Brute::new(t, scale(&b));
            let rays = random_rays(t, b.bounds, 3000, &mut rng);
            let bad = cpu_mismatches(&b, t, &brute, &rays, 2);
            assert!(
                bad.is_empty(),
                "{name} leaf {max_leaf}: {} of {} rays differ, first {:?}",
                bad.len(),
                rays.len(),
                rays[bad[0]]
            );
        }
    }
    // Watertight: rays through every interior grid vertex and edge midpoint hit the grid.
    let mesh = &meshes[1].1;
    let t = tris(mesh);
    let b = bvh::build(t, 4, 4.0);
    let n = 41;
    let mut targets = vec![];
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            let p = Vec3::from_array(mesh.0[y * n + x]);
            targets.push(p);
            targets.push((p + Vec3::from_array(mesh.0[y * n + x + 1])) * 0.5);
            targets.push((p + Vec3::from_array(mesh.0[(y + 1) * n + x + 1])) * 0.5);
        }
    }
    for p in targets {
        for side in [1.0, -1.0] {
            let origin = p + Vec3::new(
                rng.unit() - 0.5,
                rng.unit() - 0.5,
                side * (0.2 + rng.unit()),
            );
            let ray = Ray {
                origin,
                direction: (p - origin).normalize(),
                tmin: 0.0,
                tmax: 1e30,
            };
            assert!(b.trace(t, &ray, Mode::Closest).is_some(), "gap at {p}");
        }
    }
}

/// Group/binding pairs `entry` uses.
fn bindings(module: &naga::Module, info: &naga::valid::ModuleInfo, entry: &str) -> Vec<(u32, u32)> {
    let index = module
        .entry_points
        .iter()
        .position(|e| e.name == entry)
        .unwrap_or_else(|| panic!("no entry {entry}"));
    let function = info.get_entry_point(index);
    let mut used: Vec<_> = module
        .global_variables
        .iter()
        .filter(|(handle, _)| !function[*handle].is_empty())
        .filter_map(|(_, v)| v.binding.as_ref().map(|b| (b.group, b.binding)))
        .collect();
    used.sort();
    used
}

/// Bindings of the software probe's groups 0-2; group 3 is the renderer's lighting group.
const SOFTWARE_BINDINGS: [&[u32]; 3] = [
    &[3, 4, 5, 7, 10, 13, 20, 21, 23],
    &[0, 1, 5],
    &[40, 41, 42, 43, 44, 45, 46],
];

const VISIBLE: &str = "        visible+=select(1u,0u,blocked);";
/// The e1c7822 diagnostic hit-mask operation, at a test-only binding.
const MASK: &str = "\n        if blocked {atomicOr(&software_masks[(pixel.x+pixel.y*width)*8u+j/32u],1u<<(j%32u));}";
const MASKS: &str =
    "@group(2) @binding(45) var<storage,read_write> software_masks:array<atomic<u32>>;\n";
const OCCLUDED: &str =
    "                blocked=software_occluded(ray_origin,direction,tmin,tmax);\n";
const DUMP: &str =
    "@group(2) @binding(46) var<storage,read_write> software_dump_rays:array<vec4<f32>>;
// The mask buffer's tail holds each pixel's dump slot, keeping the probe within the
// device's eight storage buffers per stage.
override SOFTWARE_SLOTS:u32=0u;
// Each traced ray of a flagged pixel: origin and tmin, then direction and tmax.
fn software_dump(pixel:u32,j:u32,origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32) {
    let slot=atomicLoad(&software_masks[SOFTWARE_SLOTS+pixel]);
    if slot==0xffffffffu {return;}
    let i=(slot*FRESH_DIRECTIONS+j)*2u;
    software_dump_rays[i]=vec4<f32>(origin,tmin);
    software_dump_rays[i+1u]=vec4<f32>(direction,tmax);
}
";

/// The software module with hit masks and ray dumps; outputs otherwise unchanged.
fn software_diagnostic(soft: &str, constants: &str) -> String {
    let s = bvh::traced_source(soft, constants);
    let s = edit(&s, (VISIBLE, &format!("{VISIBLE}{MASK}")));
    let s = edit(
        &s,
        (
            OCCLUDED,
            &format!(
                "{OCCLUDED}                software_dump(pixel.x+pixel.y*width,j,ray_origin,direction,tmin,tmax);\n"
            ),
        ),
    );
    format!("{s}\n{MASKS}{DUMP}")
}
/// The production soft module with hit masks.
fn hardware_diagnostic(soft: &str) -> String {
    format!(
        "{}\n{MASKS}",
        edit(soft, (VISIBLE, &format!("{VISIBLE}{MASK}")))
    )
}

const RAY_KERNEL: &str = "struct SoftwareQuery {origin:vec4<f32>,direction:vec4<f32>,place:vec4<u32>};
@group(0) @binding(0) var<storage,read> software_queries:array<SoftwareQuery>;
@group(0) @binding(1) var<storage,read_write> software_results:array<vec4<u32>>;
// One object-space ray per invocation: origin.w tmin, direction.w tmax, place root, mesh, mode.
@compute @workgroup_size(64) fn software_rays(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&software_queries) {return;}
    let q=software_queries[id.x];
    var hit=SoftwareHit(q.direction.w,SOFTWARE_NONE,SOFTWARE_NONE,vec2<f32>(0.0));
    let ray=software_ray(q.origin.xyz,q.direction.xyz);
    if q.place.z==SOFTWARE_ANY {
        // A first hit's triangle, for f64 replay: the closest hit in the same range.
        if software_blas_any(ray,q.place.x,q.place.y,q.origin.w,q.direction.w) {
            software_blas(ray,q.place.x,q.place.y,0u,false,q.origin.w,&hit);
            hit.primitive=select(hit.primitive,0xfffffffeu,hit.primitive==SOFTWARE_NONE);
        }
    } else {
        software_blas(ray,q.place.x,q.place.y,0u,false,q.origin.w,&hit);
    }
    software_results[id.x]=vec4<u32>(bitcast<u32>(hit.t),hit.primitive,bitcast<vec2<u32>>(hit.bary));
}
";
fn ray_kernel(constants: &str) -> String {
    format!(
        "enable wgpu_binding_array;\n{}\n{RAY_KERNEL}",
        bvh::core_source(constants)
    )
}

#[test]
fn software_bvh_wgsl_validates_on_the_probe_layout() {
    let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
    let constants = bvh::constants(12, 12, 32);
    let traced = bvh::traced_source(soft, &constants);
    assert!(!traced.contains("ray_query"));
    let software = [
        (
            traced,
            &[
                "primary_trace",
                "fresh_compute",
                "primary_trace_interactive",
            ][..],
        ),
        (
            software_diagnostic(soft, &constants),
            &["fresh_compute", "primary_trace_interactive"][..],
        ),
    ];
    for (source, entries) in &software {
        let (module, info) = validate_wgsl(source);
        for entry in *entries {
            for (group, binding) in bindings(&module, &info, entry) {
                assert!(
                    group == 3 || SOFTWARE_BINDINGS[group as usize].contains(&binding),
                    "{entry} uses {group}/{binding}"
                );
            }
        }
    }
    let (module, info) = validate_wgsl(&ray_kernel(&constants));
    assert_eq!(
        bindings(&module, &info, "software_rays"),
        [(0, 0), (0, 1), (2, 40), (2, 41), (2, 43), (2, 44)]
    );
    validate_wgsl(&hardware_diagnostic(soft));
}

/// The decoded quarry's draw geometry, in `Prepared` batch order: one per primitive of a
/// placed mesh, with that mesh's placement models.
struct Quarry {
    /// Taken by the upload; CPU-only runs keep it.
    source: Option<asset::Asset>,
    meshes: Vec<MeshData32>,
    models: Vec<Vec<Mat4>>,
}
fn manifest_path(manifest: &serde_json::Value, key: &str) -> PathBuf {
    PathBuf::from(manifest[key].as_str().unwrap())
}
fn quarry_policy(manifest: &serde_json::Value) -> policy::Policy {
    let limits = manifest_path(manifest, "limits");
    policy::Policy::args(vec![
        "--limits".into(),
        limits.to_string_lossy().into_owned(),
    ])
    .unwrap()
    .0
}
fn decode_quarry(manifest: &serde_json::Value) -> Quarry {
    let started = Instant::now();
    let bytes = fs::read(manifest_path(manifest, "asset")).unwrap();
    let source = asset::decode(&bytes, quarry_policy(manifest).prepared()).unwrap();
    drop(bytes);
    let mut placed = vec![vec![]; source.meshes.len()];
    for p in &source.placements {
        placed[p.mesh as usize].push(Mat4::from_cols_array_2d(&p.transform));
    }
    let (mut meshes, mut models) = (vec![], vec![]);
    for (mesh, models_of) in source.meshes.iter().zip(&placed) {
        if models_of.is_empty() {
            continue;
        }
        for primitive in &mesh.primitives {
            meshes.push((
                primitive.vertices.iter().map(|v| v.position).collect(),
                primitive.indices.clone(),
            ));
            models.push(models_of.clone());
        }
    }
    eprintln!(
        "software_quarry_decode ms={:.1} meshes={}",
        started.elapsed().as_secs_f64() * 1e3,
        meshes.len()
    );
    Quarry {
        source: Some(source),
        meshes,
        models,
    }
}
/// `load_quarry` with an already decoded asset.
fn upload_quarry(
    manifest: &serde_json::Value,
    r: &mut Renderer,
    source: asset::Asset,
) -> (prepared::Prepared, cameras::Cameras) {
    let lighting =
        lighting_settings::parse(&fs::read_to_string(manifest_path(manifest, "lighting")).unwrap())
            .unwrap();
    let shadows = Shadows {
        directional_disk_angle: 0.04,
        requests: vec![ShadowRequest::Directional {
            split_ends: vec![4., 12., 40.],
        }],
        ..Default::default()
    };
    r.set_lighting_and_shadows(&lighting, &shadows).unwrap();
    let env = environment_controls::load_environment(
        &manifest_path(manifest, "environment"),
        asset::environment::Limits {
            max_dimension: 4096,
            max_texels: 64 * 1024 * 1024,
            max_payload_bytes: 128 * 1024 * 1024,
        },
    )
    .unwrap();
    let env = r.try_upload_environment(&env).unwrap().unwrap();
    r.set_environment(Some(&env), EnvironmentParameters::default())
        .unwrap();
    r.test_drain("software probe environment");
    let policy = quarry_policy(manifest);
    let mut asset =
        prepared::Prepared::new_with_limits(source, policy.prepared(), 1_000_000).unwrap();
    let started = Instant::now();
    while !asset.ready() {
        asset.advance(r).unwrap();
        r.collect().unwrap();
        assert!(started.elapsed() < Duration::from_secs(120));
    }
    r.test_drain("software probe upload");
    let cameras = cameras::Cameras::load(&manifest_path(manifest, "cameras")).unwrap();
    (asset, cameras)
}

/// Builds every mesh's BVH and reports per-mesh statistics.
fn build_all(meshes: &[MeshData32], max_leaf: u32, pad: f32) -> (Vec<Bvh>, serde_json::Value) {
    let started = Instant::now();
    let bvhs: Vec<Bvh> = meshes
        .iter()
        .map(|m| bvh::build(tris(m), max_leaf, pad))
        .collect();
    let total = started.elapsed().as_secs_f64() * 1e3;
    let stats: Vec<_> = bvhs.iter().map(|b| b.stats).collect();
    let bytes: usize = stats.iter().map(|s| s.bytes).sum();
    let triangles: usize = stats.iter().map(|s| s.triangles).sum();
    let summary = serde_json::json!({"max_leaf": max_leaf, "pad_ulps": pad, "build_ms_total": total,
        "bytes": bytes, "triangles": triangles, "stack": stats.iter().map(|s| s.stack).max(),
        "meshes": stats});
    eprintln!("software_bvh_build {summary}");
    (bvhs, summary)
}

fn u32s(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn read_buffer(r: &Renderer, buffer: &wgpu::Buffer) -> Vec<u8> {
    let staging = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("software probe readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    r.queue.submit([encoder.finish()]);
    wait_map(r, &staging)
}
fn submit(r: &Renderer, f: impl FnOnce(&mut wgpu::CommandEncoder)) {
    let mut encoder = r.device.create_command_encoder(&Default::default());
    f(&mut encoder);
    r.queue.submit([encoder.finish()]);
}

fn layout_entry(
    binding: u32,
    ty: wgpu::BindingType,
    count: Option<u32>,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: count.and_then(std::num::NonZeroU32::new),
    }
}
fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}
fn sampled(sample_type: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}
fn written(format: wgpu::TextureFormat) -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format,
        view_dimension: wgpu::TextureViewDimension::D2,
    }
}
fn group(
    r: &Renderer,
    layout: &wgpu::BindGroupLayout,
    entries: &[(u32, wgpu::BindingResource<'_>)],
) -> wgpu::BindGroup {
    let entries: Vec<_> = entries
        .iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();
    r.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("software probe"),
        layout,
        entries: &entries,
    })
}
fn group_layout(r: &Renderer, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    r.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("software probe"),
            entries,
        })
}
fn init(r: &Renderer, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    r.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("software probe"),
            contents,
            usage,
        })
}
/// The BVH group (binding 40-44) entries of `meshes` vertex and index buffers.
fn scene_entries(
    meshes: usize,
    extra: &[wgpu::BindGroupLayoutEntry],
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = vec![
        layout_entry(40, storage(true), Some(meshes as u32)),
        layout_entry(41, storage(true), Some(meshes as u32)),
        layout_entry(43, storage(true), Some(meshes as u32)),
        layout_entry(44, storage(true), Some(meshes as u32)),
    ];
    entries.extend_from_slice(extra);
    entries
}
fn compute(
    r: &Renderer,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    constants: &[(&str, f64)],
) -> wgpu::ComputePipeline {
    r.device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            cache: None,
        })
}
fn module(r: &Renderer, source: &str) -> wgpu::ShaderModule {
    r.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("software probe"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

const ARITHMETIC: &str = "@group(0) @binding(0) var<storage,read> arithmetic_in:array<vec4<f32>>;
@group(0) @binding(1) var<storage,read_write> arithmetic_out:array<vec4<f32>>;
fn software_difference(a:f32,b:f32,c:f32,d:f32)->f32 {
    let w=c*d;
    return fma(a,b,-w)+fma(-c,d,w);
}
@compute @workgroup_size(64) fn arithmetic(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&arithmetic_in) {return;}
    let v=arithmetic_in[id.x];
    arithmetic_out[id.x]=vec4<f32>(fma(v.x,v.y,v.z),software_difference(v.x,v.y,v.z,v.w),v.x*v.y-v.z*v.w,fma(-v.z,v.w,v.z*v.w));
}
";

/// GPU against CPU bits for fma, the Kahan difference, plain a*b-c*d and the product
/// error term, on inputs whose products nearly cancel.
fn arithmetic_probe(r: &Renderer, rng: &mut Rng) -> serde_json::Value {
    let n = 4096;
    let mut input = vec![];
    for _ in 0..n {
        let a = rng.unit() * 8.0 - 4.0;
        let b = rng.unit() * 8.0 - 4.0;
        let c = rng.unit() * 8.0 - 4.0;
        let d = a * b / c * (1.0 + (rng.unit() - 0.5) * 1e-5);
        input.push([a, b, c, d]);
    }
    let bytes: Vec<u8> = input
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let layout = group_layout(
        r,
        &[
            layout_entry(0, storage(true), None),
            layout_entry(1, storage(false), None),
        ],
    );
    let pipeline_layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("arithmetic probe"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let pipeline = compute(
        r,
        &pipeline_layout,
        &module(r, ARITHMETIC),
        "arithmetic",
        &[],
    );
    let input_buffer = init(r, &bytes, wgpu::BufferUsages::STORAGE);
    let output = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("arithmetic out"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let g = group(
        r,
        &layout,
        &[
            (0, input_buffer.as_entire_binding()),
            (1, output.as_entire_binding()),
        ],
    );
    submit(r, |e| {
        let mut pass = e.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &g, &[]);
        pass.dispatch_workgroups((n as u32).div_ceil(64), 1, 1);
    });
    let out = f32s(&read_buffer(r, &output));
    let mut same = [0usize; 4];
    let mut unfused = [0usize; 2];
    for (i, [a, b, c, d]) in input.iter().copied().enumerate() {
        let w = c * d;
        let cpu = [
            a.mul_add(b, c),
            a.mul_add(b, -w) + (-c).mul_add(d, w),
            a * b - c * d,
            (-c).mul_add(d, w),
        ];
        for k in 0..4 {
            same[k] += usize::from(out[i * 4 + k].to_bits() == cpu[k].to_bits());
        }
        unfused[0] += usize::from(out[i * 4].to_bits() == (a * b + c).to_bits());
        unfused[1] += usize::from(out[i * 4 + 3] == 0.0);
    }
    serde_json::json!({"inputs": n, "fma_equal_fused": same[0], "kahan_equal": same[1],
        "plain_equal_unfused": same[2], "error_term_equal": same[3],
        "fma_equal_unfused": unfused[0], "error_term_zero": unfused[1]})
}

/// An object-space ray against one mesh's tree: root node, mesh slot and WGSL mode.
struct Query {
    ray: Ray,
    mesh: usize,
    mode: Mode,
}
/// GPU fixture kernel: each query traced by `software_blas`, as (t bits, primitive, bary).
fn gpu_rays(
    r: &Renderer,
    buffers: &[(&wgpu::Buffer, &wgpu::Buffer)],
    packed: &bvh::Packed,
    queries: &[Query],
) -> Vec<[u32; 4]> {
    let scopes = OperationScopes::new(&r.device);
    let source = ray_kernel(&bvh::constants(buffers.len(), 0, packed.stack));
    let layouts = [
        group_layout(
            r,
            &[
                layout_entry(0, storage(true), None),
                layout_entry(1, storage(false), None),
            ],
        ),
        group_layout(r, &scene_entries(buffers.len(), &[])),
    ];
    let layout = r
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("software rays"),
            bind_group_layouts: &[Some(&layouts[0]), None, Some(&layouts[1])],
            immediate_size: 0,
        });
    let pipeline = compute(r, &layout, &module(r, &source), "software_rays", &[]);
    let mut bytes = vec![];
    for q in queries {
        let mode = if q.mode == Mode::Any { 2 } else { 0 };
        let words = [
            q.ray.origin.x.to_bits(),
            q.ray.origin.y.to_bits(),
            q.ray.origin.z.to_bits(),
            q.ray.tmin.to_bits(),
            q.ray.direction.x.to_bits(),
            q.ray.direction.y.to_bits(),
            q.ray.direction.z.to_bits(),
            q.ray.tmax.to_bits(),
            0,
            q.mesh as u32,
            mode,
            0,
        ];
        bytes.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    }
    let storage_usage = wgpu::BufferUsages::STORAGE;
    let queries_buffer = init(r, &bytes, storage_usage);
    let results = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("software ray results"),
        size: 16 * queries.len() as u64,
        usage: storage_usage | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let nodes: Vec<_> = packed
        .nodes
        .iter()
        .map(|b| init(r, b, storage_usage))
        .collect();
    let prims: Vec<_> = packed
        .prims
        .iter()
        .map(|b| init(r, b, storage_usage))
        .collect();
    let nodes: Vec<_> = nodes
        .iter()
        .map(wgpu::Buffer::as_entire_buffer_binding)
        .collect();
    let prims: Vec<_> = prims
        .iter()
        .map(wgpu::Buffer::as_entire_buffer_binding)
        .collect();
    let vertices: Vec<_> = buffers
        .iter()
        .map(|b| b.0.as_entire_buffer_binding())
        .collect();
    let indices: Vec<_> = buffers
        .iter()
        .map(|b| b.1.as_entire_buffer_binding())
        .collect();
    use wgpu::BindingResource as R;
    let groups = [
        group(
            r,
            &layouts[0],
            &[
                (0, queries_buffer.as_entire_binding()),
                (1, results.as_entire_binding()),
            ],
        ),
        group(
            r,
            &layouts[1],
            &[
                (40, R::BufferArray(&nodes)),
                (41, R::BufferArray(&prims)),
                (43, R::BufferArray(&vertices)),
                (44, R::BufferArray(&indices)),
            ],
        ),
    ];
    submit(r, |e| {
        let mut pass = e.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &groups[0], &[]);
        pass.set_bind_group(2, &groups[1], &[]);
        pass.dispatch_workgroups((queries.len() as u32).div_ceil(64), 1, 1);
    });
    let out = read_buffer(r, &results);
    if let Some(e) = scopes.finish() {
        panic!("software ray kernel failed: {e}");
    }
    u32s(&out)
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect()
}

/// f64 replay of one triangle against an object-space query ray whose segment ends at
/// `reach`, with the rounding scale of that decision (4 f32 ulps of the largest coordinate
/// plus 1e-6 of the distance along the ray).
fn object_replay(tris: Triangles<'_>, ray: &Ray, primitive: u32, reach: f64) -> (bvh::Replay, f64) {
    let (o, d) = (ray.origin.as_dvec3(), ray.direction.as_dvec3());
    let tmax = f64::from(ray.tmax).min(reach);
    let v = tris.vertices(primitive).map(|p| p.as_dvec3());
    let rep = bvh::replay(o, d, f64::from(ray.tmin), tmax, v);
    let m = v
        .iter()
        .fold(o.abs().max_element(), |m, p| m.max(p.abs().max_element()));
    let along = if rep.t.is_nan() {
        tmax
    } else {
        rep.t.abs().min(tmax)
    };
    let tol = 4.0 * f64::from(bvh::ulp(m as f32)) + 1e-6 * along * d.length();
    (rep, tol)
}

/// GPU kernel results against the CPU reference. GPU division is not correctly rounded, so
/// the shear differs by an ulp and results near edges may differ; every difference is
/// replayed in f64: "exact" bits, "rounded" (same triangle, distance within rounding;
/// "grazing" when within rounding over the cosine to the triangle plane),
/// "tie" (two triangles at one distance), "edge" (the result only one side has lies within
/// rounding of a triangle's boundary), "any" and "any-edge" for any-hit queries, "other".
fn compare_rays(
    meshes: &[Triangles<'_>],
    bvhs: &[&Bvh],
    queries: &[Query],
    results: &[[u32; 4]],
    examples: &mut Vec<serde_json::Value>,
) -> BTreeMap<&'static str, usize> {
    let mut classes = BTreeMap::new();
    for (q, g) in queries.iter().zip(results) {
        let (tris, tree) = (meshes[q.mesh], bvhs[q.mesh]);
        let cpu = tree.trace(tris, &q.ray, q.mode);
        let gpu = (g[1] != bvh::NONE).then(|| bvh::Hit {
            t: f32::from_bits(g[0]),
            primitive: g[1],
            bary: [f32::from_bits(g[2]), f32::from_bits(g[3])],
        });
        let reach = 2.0
            * [cpu, gpu]
                .iter()
                .flatten()
                .map(|h| f64::from(h.t))
                .fold(1.0, f64::max);
        let replay = |h: &bvh::Hit| object_replay(tris, &q.ray, h.primitive, reach);
        let near = |h: &bvh::Hit| {
            let (rep, tol) = replay(h);
            rep.margin <= tol
        };
        let class = match (q.mode, cpu, gpu) {
            (Mode::Any, a, b) if a.is_some() == b.is_some() => "any",
            (Mode::Any, a, b) => {
                if a.or(b)
                    .is_some_and(|h| (h.primitive as usize) < tris.len() && near(&h))
                {
                    "any-edge"
                } else {
                    "other"
                }
            }
            (_, None, None) => "exact",
            (_, Some(h), None) | (_, None, Some(h)) => {
                if near(&h) {
                    "edge"
                } else {
                    "other"
                }
            }
            (_, Some(a), Some(b)) => {
                let length = f64::from(q.ray.direction.length());
                let (ra, tol) = replay(&a);
                let (rb, _) = replay(&b);
                let apart = (ra.t - rb.t).abs() * length;
                if a == b {
                    "exact"
                } else if a.primitive == b.primitive {
                    // A grazing ray turns a rounding shift into a distance shift of 1/cos.
                    let v = tris.vertices(a.primitive).map(|p| p.as_dvec3());
                    let normal = (v[1] - v[0]).cross(v[2] - v[0]).normalize();
                    let cos = normal.dot(q.ray.direction.as_dvec3() / length).abs();
                    let shift = (f64::from(a.t) - f64::from(b.t)).abs() * length;
                    if shift <= tol {
                        "rounded"
                    } else if shift <= tol / cos.max(1e-9) {
                        "grazing"
                    } else {
                        "other"
                    }
                } else if apart <= tol {
                    "tie"
                } else {
                    // The nearer triangle was missed by one side: it must lie at an edge.
                    let nearer = if ra.t < rb.t { (ra, tol) } else { replay(&b) };
                    if nearer.0.margin <= nearer.1 {
                        "edge"
                    } else {
                        "other"
                    }
                }
            }
        };
        if class == "other" && examples.len() < 40 {
            let own = gpu.map(|h| bvh::intersect(tris, &q.ray, h.primitive));
            let margins: Vec<_> = [cpu, gpu].iter().flatten().map(replay).collect();
            examples.push(
                serde_json::json!({"mesh": q.mesh, "any": q.mode == Mode::Any,
                "origin": q.ray.origin.to_array(), "direction": q.ray.direction.to_array(),
                "tmin": q.ray.tmin, "tmax": q.ray.tmax,
                "cpu": cpu.map(|h| (h.t, h.primitive, h.bary)),
                "gpu": gpu.map(|h| (h.t, h.primitive, h.bary)),
                "replays": format!("{margins:?}"),
                "cpu_test_of_gpu_triangle": format!("{own:?}")}),
            );
        }
        *classes.entry(class).or_insert(0) += 1;
    }
    classes
}

/// Uploads `mesh` with the renderer's 48-byte vertex layout (position first) for the fixture.
fn fixture_buffers(r: &Renderer, mesh: &MeshData32) -> (wgpu::Buffer, wgpu::Buffer) {
    let mut v = vec![];
    for p in &mesh.0 {
        let words = [
            p[0], p[1], p[2], 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0,
        ];
        v.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    }
    let i: Vec<u8> = mesh.1.iter().flat_map(|w| w.to_le_bytes()).collect();
    (
        init(r, &v, wgpu::BufferUsages::STORAGE),
        init(r, &i, wgpu::BufferUsages::STORAGE),
    )
}

/// Closest and any-hit queries for each mesh (object space); any-hit ranges end at
/// 0.5, 0.8, 1.2 or 1.5 of the reference's closest distance, never at a tie.
fn mesh_queries(
    meshes: &[Triangles<'_>],
    bvhs: &[&Bvh],
    per_mesh: usize,
    rng: &mut Rng,
) -> Vec<Query> {
    let mut queries = vec![];
    for (mesh, (t, b)) in meshes.iter().zip(bvhs).enumerate() {
        for (i, ray) in random_rays(*t, b.bounds, per_mesh, rng)
            .into_iter()
            .enumerate()
        {
            let closest = b.trace(*t, &ray, Mode::Closest);
            let shadow = shadow_of(&ray, closest, [0.5, 0.8, 1.2, 1.5][i % 4]);
            queries.push(Query {
                ray,
                mesh,
                mode: Mode::Closest,
            });
            queries.push(Query {
                ray: shadow,
                mesh,
                mode: Mode::Any,
            });
        }
    }
    queries
}

/// Placed geometry for f64 replays, in placement order.
struct World<'a> {
    meshes: Vec<Triangles<'a>>,
    bvhs: &'a [Bvh],
    /// Mesh slot, model, inverse and padded world bounds.
    placements: Vec<(usize, DMat4, DMat4, DVec3, DVec3)>,
    /// Largest world coordinate magnitude.
    scale: f64,
}
fn segment_box(o: DVec3, d: DVec3, tmin: f64, tmax: f64, lo: DVec3, hi: DVec3) -> bool {
    let (mut near, mut far) = (tmin, tmax);
    for a in 0..3 {
        if d[a] == 0.0 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
        near = near.max(t0.min(t1));
        far = far.min(t0.max(t1));
    }
    near <= far
}
impl<'a> World<'a> {
    /// `placements`: each placement's mesh slot and model, in placement order.
    fn new(meshes: &'a [MeshData32], bvhs: &'a [Bvh], placements: &[(usize, Mat4)]) -> Self {
        let bounds: Vec<_> = placements
            .iter()
            .map(|&(slot, model)| bvh::world_bounds(model, &bvhs[slot]))
            .collect();
        Self {
            meshes: meshes.iter().map(tris).collect(),
            bvhs,
            placements: placements
                .iter()
                .zip(&bounds)
                .map(|(&(slot, model), b)| {
                    let (m, inverse) = bvh::transforms(model);
                    (slot, m, inverse, b.lo.as_dvec3(), b.hi.as_dvec3())
                })
                .collect(),
            scale: bounds
                .iter()
                .map(|b| f64::from(b.lo.abs().max(b.hi.abs()).max_element()))
                .fold(1.0, f64::max),
        }
    }
    fn triangle(&self, placement: u32, primitive: u32) -> [DVec3; 3] {
        let (slot, model, ..) = &self.placements[placement as usize];
        self.meshes[*slot]
            .vertices(primitive)
            .map(|v| model.transform_point3(v.as_dvec3()))
    }
    /// Every (placement, primitive) whose padded leaf box meets the segment widened by `pad`.
    fn candidates(&self, o: DVec3, d: DVec3, tmin: f64, tmax: f64, pad: f64) -> Vec<(u32, u32)> {
        let (mut out, mut prims) = (vec![], vec![]);
        for (p, (slot, _, inverse, lo, hi)) in self.placements.iter().enumerate() {
            if !segment_box(o, d, tmin, tmax, *lo - pad, *hi + pad) {
                continue;
            }
            let stretch = [inverse.x_axis, inverse.y_axis, inverse.z_axis]
                .iter()
                .map(|c| c.truncate().length())
                .fold(0.0, f64::max);
            prims.clear();
            let (oo, dd) = (inverse.transform_point3(o), inverse.transform_vector3(d));
            self.bvhs[*slot].candidates(oo, dd, tmin, tmax, pad * stretch, &mut prims);
            out.extend(prims.iter().map(|&t| (p as u32, t)));
        }
        out
    }
    /// Rounding scale of a ray/triangle decision: 4 f32 ulps of the largest coordinate in
    /// world or object space (both backends intersect in object space), plus 1e-6 of the
    /// distance along the ray for direction rounding.
    fn tol(&self, placement: u32, primitive: u32, o: DVec3, v: &[DVec3; 3], distance: f64) -> f64 {
        let (slot, _, inverse, ..) = &self.placements[placement as usize];
        let object = self.meshes[*slot].vertices(primitive).map(|p| p.as_dvec3());
        let m = v
            .iter()
            .chain(&object)
            .fold(o.abs().max_element(), |m, p| m.max(p.abs().max_element()))
            .max(inverse.transform_point3(o).abs().max_element());
        4.0 * f64::from(bvh::ulp(m as f32)) + 1e-6 * distance.abs()
    }
    fn pad(&self, length: f64) -> f64 {
        4.0 * f64::from(bvh::ulp(self.scale as f32)) + 1e-6 * length
    }
    /// World point of a hit record (placement + 1, primitive, f32 barycentrics).
    fn point(&self, record: [u32; 4]) -> DVec3 {
        let v = self.triangle(record[0] - 1, record[1]);
        let (b1, b2) = (
            f64::from(f32::from_bits(record[2])),
            f64::from(f32::from_bits(record[3])),
        );
        v[0] * (1.0 - b1 - b2) + v[1] * b1 + v[2] * b2
    }
    fn adjacent(&self, a: [u32; 4], b: [u32; 4]) -> bool {
        if a[0] != b[0] {
            return false;
        }
        let (slot, ..) = self.placements[a[0] as usize - 1];
        let t = self.meshes[slot];
        let (va, vb) = (t.vertices(a[1]), t.vertices(b[1]));
        va.iter().any(|p| vb.contains(p))
    }
}

/// One shadow-ray disagreement replayed in f64 from the software ray (origin, tmin,
/// direction, tmax): "edge" or "endpoint" when a triangle lies within rounding of the
/// segment's boundary (either answer is admissible), otherwise the side that contradicts a
/// clear f64 result ("other-hardware" or "other-software"). Returns the smallest margin.
fn classify_shadow(
    world: &World<'_>,
    ray: [f32; 8],
    hw_blocked: bool,
) -> (&'static str, f64, serde_json::Value) {
    let o = DVec3::new(ray[0].into(), ray[1].into(), ray[2].into());
    let d = DVec3::new(ray[4].into(), ray[5].into(), ray[6].into());
    let (tmin, tmax) = (f64::from(ray[3]), f64::from(ray[7]));
    if d == DVec3::ZERO {
        return ("other-untraced", f64::NAN, serde_json::Value::Null);
    }
    let length = d.length();
    let (mut clear, mut near) = (false, None::<(f64, bool)>);
    let mut deciding = vec![];
    for (p, t) in world.candidates(o, d, tmin, tmax, world.pad(length * tmax)) {
        let v = world.triangle(p, t);
        let rep = bvh::replay(o, d, tmin, tmax, v);
        let along = if rep.t.is_nan() {
            tmax
        } else {
            rep.t.clamp(tmin, tmax)
        };
        let tol = world.tol(p, t, o, &v, length * along);
        if rep.margin <= tol || rep.inside {
            deciding.push(
                serde_json::json!({"placement": p, "primitive": t, "replay": rep, "tol": tol}),
            );
        }
        if rep.margin <= tol {
            if near.is_none_or(|n| rep.margin < n.0) {
                near = Some((rep.margin, rep.endpoint));
            }
        } else if rep.inside {
            clear = true;
        }
    }
    let deciding = serde_json::Value::from(deciding);
    let wrong = |blocked_truth: bool| {
        if hw_blocked == blocked_truth {
            "other-software"
        } else {
            "other-hardware"
        }
    };
    match (clear, near) {
        (true, n) => (wrong(true), n.map_or(f64::NAN, |n| n.0), deciding),
        (false, Some((m, endpoint))) => (if endpoint { "endpoint" } else { "edge" }, m, deciding),
        (false, None) => (wrong(false), f64::NAN, deciding),
    }
}

/// One camera hit-record disagreement replayed in f64 from the pixel-centre ray: both
/// results must be admissible under rounding (a hit within the rounding of the clear
/// closest hit, or a miss when there is none). "silhouette" when one is a miss or they
/// lie at different depths, "edge tie" for neighbours at one depth, "depth tie" for
/// coincident surfaces, else "other".
fn classify_camera(
    world: &World<'_>,
    inverse: DMat4,
    pixel: [usize; 2],
    size: [usize; 2],
    hw: [u32; 4],
    sw: [u32; 4],
) -> (&'static str, f64) {
    let ndc = DVec2::new(
        (pixel[0] as f64 + 0.5) / size[0] as f64 * 2.0 - 1.0,
        1.0 - (pixel[1] as f64 + 0.5) / size[1] as f64 * 2.0,
    );
    let a = inverse * DVec4::new(ndc.x, ndc.y, 1.0, 1.0);
    let b = inverse * DVec4::new(ndc.x, ndc.y, 0.0, 1.0);
    let o = a.truncate() / a.w;
    let d = b.truncate() / b.w - o;
    let length = d.length();
    let reps: Vec<_> = world
        .candidates(o, d, 0.0, 1.0, world.pad(length))
        .into_iter()
        .map(|(p, t)| {
            let v = world.triangle(p, t);
            let rep = bvh::replay(o, d, 0.0, 1.0, v);
            let along = if rep.t.is_nan() {
                1.0
            } else {
                rep.t.clamp(0.0, 1.0)
            };
            ((p, t), rep, world.tol(p, t, o, &v, length * along))
        })
        .collect();
    let clear = reps
        .iter()
        .filter(|(_, rep, tol)| rep.inside && rep.margin > *tol)
        .map(|(_, rep, _)| rep.t)
        .fold(f64::INFINITY, f64::min);
    let admissible = |record: [u32; 4]| -> Option<(Option<f64>, f64)> {
        if record[0] == 0 {
            let margin = reps
                .iter()
                .filter(|(_, rep, tol)| rep.margin <= *tol)
                .map(|(_, rep, _)| rep.margin)
                .fold(f64::NAN, f64::min);
            return clear.is_infinite().then_some((None, margin));
        }
        let (_, rep, tol) = reps
            .iter()
            .find(|(k, ..)| *k == (record[0] - 1, record[1]))?;
        ((rep.inside || rep.margin <= *tol) && rep.t * length <= clear * length + tol)
            .then_some((Some(rep.t), rep.margin))
    };
    let (Some((th, mh)), Some((ts, ms))) = (admissible(hw), admissible(sw)) else {
        return ("other", f64::NAN);
    };
    let margin = mh.min(ms);
    let tol = world.pad(length);
    match (th, ts) {
        (Some(x), Some(y)) if (x - y).abs() * length <= tol => {
            if world.adjacent(hw, sw) {
                ("edge tie", margin)
            } else {
                ("depth tie", margin)
            }
        }
        _ => ("silhouette", margin),
    }
}

/// Labelled counts with the largest margin of each label and the first examples.
#[derive(Default, serde::Serialize)]
struct Classes {
    counts: BTreeMap<String, usize>,
    max_margin_m: BTreeMap<String, f64>,
    examples: Vec<serde_json::Value>,
}
impl Classes {
    fn add(&mut self, label: &str, margin: f64, example: impl FnOnce() -> serde_json::Value) {
        *self.counts.entry(label.into()).or_insert(0) += 1;
        if margin.is_finite() {
            let m = self.max_margin_m.entry(label.into()).or_insert(0.0);
            *m = m.max(margin);
        }
        if label.starts_with("other") && self.examples.len() < 32 {
            self.examples.push(example());
        }
    }
    fn others(&self) -> usize {
        self.counts
            .iter()
            .filter(|(k, _)| k.starts_with("other"))
            .map(|(_, v)| v)
            .sum()
    }
}

/// Pixels outside `scene` or in a clear tile flag: misses without a record.
fn uncovered_by(tiles: &[u8], scene: [u32; 4], [w, h]: [usize; 2]) -> Vec<bool> {
    let [tw, th] = primary_ray_gpu::TRACE_TILE.map(|v| v as usize);
    let columns = w.div_ceil(tw);
    let s = scene.map(|v| v as usize);
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let tile = x / tw + y / th * columns;
            !(s[0]..s[2]).contains(&x)
                || !(s[1]..s[3]).contains(&y)
                || tiles[tile * 4..tile * 4 + 4] == [0; 4]
        })
        .collect()
}

/// Software targets and bindings on the hardware path's frame inputs.
struct Software {
    layout: wgpu::PipelineLayout,
    groups: [wgpu::BindGroup; 3],
    frame: wgpu::Buffer,
    hit: wgpu::Texture,
    counts: wgpu::Texture,
    tiles: wgpu::Texture,
    epsilon: wgpu::Texture,
    masks: wgpu::Buffer,
    dump: wgpu::Buffer,
}
/// Dumped rays per pass B: flagged pixels times directions.
const DUMP_RAYS: u64 = 1 << 21;

impl Software {
    fn new(
        r: &Renderer,
        quarry: &mut prepared::Prepared,
        bvhs: &[Bvh],
        packed: &bvh::Packed,
    ) -> (Self, Vec<(usize, Mat4, bool)>) {
        use wgpu::{BindingResource as R, TextureFormat as F};
        let draws = quarry.draws(Visibility::Frustum);
        let mut models = vec![];
        let mut normals = vec![];
        let mut placements = vec![];
        let mut instances = vec![];
        let table = crate::primary_ray::placement_table(draws.iter().map(|d| {
            let m = d.material.unwrap();
            (
                d.instances.len(),
                d.mesh.tangent.is_some(),
                !m.double_sided,
                m.thin,
            )
        }))
        .unwrap();
        for (slot, d) in draws.iter().enumerate() {
            for (model, normal) in d
                .instances
                .state
                .models
                .iter()
                .zip(&d.instances.state.normals)
            {
                let single = table[placements.len()] & 1 << 30 != 0;
                models.extend(crate::mesh::matrix_bytes(*model));
                normals.extend(normal.bytes());
                instances.extend(bvh::instance(*model, &bvhs[slot], slot as u32, single));
                placements.push((slot, *model, single));
            }
        }
        let vertices: Vec<_> = draws
            .iter()
            .map(|d| (**d.mesh.vertex).as_entire_buffer_binding())
            .collect();
        let indices: Vec<_> = draws
            .iter()
            .map(|d| (**d.mesh.index).as_entire_buffer_binding())
            .collect();
        let meshes = draws.len();
        let read = wgpu::BufferUsages::STORAGE;
        let words: Vec<u8> = table.iter().flat_map(|w| w.to_le_bytes()).collect();
        let [models, normals, table, instances] =
            [&models, &normals, &words, &instances].map(|b| init(r, b, read));
        let nodes: Vec<_> = packed.nodes.iter().map(|b| init(r, b, read)).collect();
        let prims: Vec<_> = packed.prims.iter().map(|b| init(r, b, read)).collect();
        let nodes: Vec<_> = nodes
            .iter()
            .map(wgpu::Buffer::as_entire_buffer_binding)
            .collect();
        let prims: Vec<_> = prims
            .iter()
            .map(wgpu::Buffer::as_entire_buffer_binding)
            .collect();
        let rw = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let size = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let pixels = u64::from(size.width) * u64::from(size.height);
        let buffer = |label, size| {
            r.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: rw,
                mapped_at_creation: false,
            })
        };
        // 256 mask bits, then one dump slot per pixel.
        let masks = buffer("software masks and dump slots", pixels * 36);
        let dump = buffer("software dumped rays", DUMP_RAYS * 32);
        let frame = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("software frame"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = |label, size: wgpu::Extent3d, format| {
            r.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let tile_size = wgpu::Extent3d {
            width: size.width.div_ceil(8),
            height: size.height.div_ceil(4),
            depth_or_array_layers: 1,
        };
        let hit = texture("software hit records", size, F::Rgba32Uint);
        let counts = texture("software counts", size, F::R32Uint);
        let tiles = texture("software tiles", tile_size, F::R32Uint);
        let epsilon = texture("software epsilon", size, F::R32Float);
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let uint = wgpu::TextureSampleType::Uint;
        let layouts = [
            group_layout(
                r,
                &[
                    layout_entry(3, sampled(float), None),
                    layout_entry(4, sampled(float), None),
                    layout_entry(5, written(F::R32Uint), None),
                    layout_entry(7, sampled(uint), None),
                    layout_entry(
                        10,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        None,
                    ),
                    layout_entry(13, written(F::Rgba32Uint), None),
                    layout_entry(20, written(F::R32Uint), None),
                    layout_entry(21, sampled(uint), None),
                    layout_entry(23, written(F::R32Float), None),
                ],
            ),
            group_layout(
                r,
                &[
                    layout_entry(0, storage(true), None),
                    layout_entry(1, storage(true), None),
                    layout_entry(5, storage(true), None),
                ],
            ),
            group_layout(
                r,
                &scene_entries(
                    meshes,
                    &[
                        layout_entry(42, storage(true), None),
                        layout_entry(45, storage(false), None),
                        layout_entry(46, storage(false), None),
                    ],
                ),
            ),
        ];
        let hw = r.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let fresh = r.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (hit_view, counts_view, tiles_view, epsilon_view) =
            (view(&hit), view(&counts), view(&tiles), view(&epsilon));
        let groups = [
            group(
                r,
                &layouts[0],
                &[
                    (3, R::TextureView(&fresh.world.view)),
                    (4, R::TextureView(&fresh.normal.view)),
                    (5, R::TextureView(&counts_view)),
                    (7, R::TextureView(&hw.hit.view)),
                    (10, frame.as_entire_binding()),
                    (13, R::TextureView(&hit_view)),
                    (20, R::TextureView(&tiles_view)),
                    (21, R::TextureView(&hw.tiles.view)),
                    (23, R::TextureView(&epsilon_view)),
                ],
            ),
            group(
                r,
                &layouts[1],
                &[
                    (0, models.as_entire_binding()),
                    (1, normals.as_entire_binding()),
                    (5, table.as_entire_binding()),
                ],
            ),
            group(
                r,
                &layouts[2],
                &[
                    (40, R::BufferArray(&nodes)),
                    (41, R::BufferArray(&prims)),
                    (42, instances.as_entire_binding()),
                    (43, R::BufferArray(&vertices)),
                    (44, R::BufferArray(&indices)),
                    (45, masks.as_entire_binding()),
                    (46, dump.as_entire_binding()),
                ],
            ),
        ];
        let layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("software probe ABI"),
                bind_group_layouts: &[
                    Some(&layouts[0]),
                    Some(&layouts[1]),
                    Some(&layouts[2]),
                    Some(&r.lighting.bindings),
                ],
                immediate_size: 0,
            });
        (
            Self {
                layout,
                groups,
                frame,
                hit,
                counts,
                tiles,
                epsilon,
                masks,
                dump,
            },
            placements,
        )
    }
    fn pass(
        &self,
        r: &Renderer,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        groups: [u32; 2],
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("software probe"),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        for (i, g) in self.groups.iter().enumerate() {
            pass.set_bind_group(i as u32, g, &[]);
        }
        pass.set_bind_group(3, r.lighting.bind(), &[]);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
    }
}

/// The traced frame uniform as `prepare_primary` writes it.
fn frame_bytes(r: &Renderer, camera: &Camera, size: [u32; 2], sided: bool) -> [u8; 112] {
    let inverse = camera.matrix(size[0], size[1]).inverse();
    let mut bytes = [0; 112];
    bytes[..64].copy_from_slice(&crate::mesh::matrix_bytes(inverse));
    for (dst, value) in bytes[64..80]
        .chunks_exact_mut(4)
        .zip(r.lit_clear().into_iter().chain([1.0]))
    {
        dst.copy_from_slice(&(value as f32).to_ne_bytes());
    }
    bytes[80..84].copy_from_slice(&u32::from(sided).to_ne_bytes());
    for (dst, value) in bytes[96..]
        .chunks_exact_mut(4)
        .zip(r.primary.as_ref().unwrap().scene)
    {
        dst.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

/// Every pipeline the probe dispatches.
struct Pipelines {
    sw_camera: wgpu::ComputePipeline,
    sw_shadows: wgpu::ComputePipeline,
    sw_exact: wgpu::ComputePipeline,
    sw_fused: wgpu::ComputePipeline,
    sw_diag: [wgpu::ComputePipeline; 3],
    hw_shadows: Owned<wgpu::ComputePipeline>,
    hw_diag: [wgpu::ComputePipeline; 2],
    hw_masks: wgpu::BindGroup,
}
const EIGHT: [(&str, f64); 1] = [("FRESH_DIRECTIONS", 8.0)];

impl Pipelines {
    fn new(r: &Renderer, sw: &Software, meshes: usize, instances: usize, stack: u32) -> Self {
        let scopes = OperationScopes::new(&r.device);
        let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
        let constants = bvh::constants(meshes, instances, stack);
        let size = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let slots = f64::from(size.width * size.height * 8);
        let diag8 = [EIGHT[0], ("SOFTWARE_SLOTS", slots)];
        let diag256 = [("SOFTWARE_SLOTS", slots)];
        let traced = module(r, &bvh::traced_source(soft, &constants));
        let diagnostic = module(r, &software_diagnostic(soft, &constants));
        let primary = r.primary.as_ref().unwrap();
        let hw_module = module(r, soft);
        let hw_diag_module = module(r, &hardware_diagnostic(soft));
        let mask_layout = group_layout(r, &[layout_entry(45, storage(false), None)]);
        let (fresh_layout, _) = primary.test_fresh_bindings();
        let hw_layout = r
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("hardware diagnostic masks"),
                bind_group_layouts: &[
                    Some(fresh_layout),
                    None,
                    Some(&mask_layout),
                    Some(&r.lighting.bindings),
                ],
                immediate_size: 0,
            });
        let hw_masks = group(r, &mask_layout, &[(45, sw.masks.as_entire_binding())]);
        let p = Self {
            sw_camera: compute(r, &sw.layout, &traced, "primary_trace", &[]),
            sw_shadows: compute(r, &sw.layout, &traced, "fresh_compute", &EIGHT),
            sw_exact: compute(r, &sw.layout, &traced, "fresh_compute", &[]),
            sw_fused: compute(r, &sw.layout, &traced, "primary_trace_interactive", &EIGHT),
            sw_diag: [
                compute(r, &sw.layout, &diagnostic, "fresh_compute", &diag8),
                compute(r, &sw.layout, &diagnostic, "fresh_compute", &diag256),
                compute(
                    r,
                    &sw.layout,
                    &diagnostic,
                    "primary_trace_interactive",
                    &diag8,
                ),
            ],
            hw_shadows: primary.fresh_pipeline(
                &r.device,
                &r.ledger,
                &hw_module,
                "fresh_compute",
                &EIGHT,
            ),
            hw_diag: [
                compute(r, &hw_layout, &hw_diag_module, "fresh_compute", &EIGHT),
                compute(r, &hw_layout, &hw_diag_module, "fresh_compute", &[]),
            ],
            hw_masks,
        };
        if let Some(e) = scopes.finish() {
            panic!("probe pipelines failed to compile: {e}");
        }
        p
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pass {
    HwCamera,
    SwCamera,
    HwShadows,
    SwShadows,
    HwFused,
    SwFused,
    HwExact,
    SwExact,
}
/// Encodes one production or software kernel. Shadow passes read the hardware receivers
/// and coverage the last hardware receiver trace wrote.
fn encode(
    r: &Renderer,
    sw: &Software,
    p: &Pipelines,
    pass: Pass,
    encoder: &mut wgpu::CommandEncoder,
    stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let size = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
    let g8x4 = [size.width.div_ceil(8), size.height.div_ceil(4)];
    let g2x2 = [size.width.div_ceil(2), size.height.div_ceil(2)];
    let hw = r.pipeline.primary.as_ref().unwrap();
    let trace = primary_ray_gpu::TRACE_TILE;
    match pass {
        Pass::HwCamera => r.encode_primary_trace(encoder, &hw.trace[0], trace, false, stamps),
        Pass::SwCamera => sw.pass(r, encoder, &p.sw_camera, g8x4, stamps),
        Pass::HwShadows => r.encode_primary_fresh_tile(encoder, &p.hw_shadows, [8, 4], stamps),
        Pass::SwShadows => sw.pass(r, encoder, &p.sw_shadows, g8x4, stamps),
        Pass::HwFused => r.encode_primary_interactive(encoder, &hw.interactive, trace, stamps),
        Pass::SwFused => sw.pass(r, encoder, &p.sw_fused, g8x4, stamps),
        Pass::HwExact => r.encode_primary_fresh(encoder, &hw.fresh, stamps),
        Pass::SwExact => sw.pass(r, encoder, &p.sw_exact, g2x2, stamps),
    }
}
fn hw_diag(r: &Renderer, p: &Pipelines, exact: bool, encoder: &mut wgpu::CommandEncoder) {
    let size = r.lighting.hdr.as_ref().unwrap().resolved.texture.size();
    let tile = if exact { [2, 2] } else { [8, 4] };
    let (_, fresh) = r.primary.as_ref().unwrap().test_fresh_bindings();
    let mut pass = encoder.begin_compute_pass(&Default::default());
    pass.set_pipeline(&p.hw_diag[usize::from(exact)]);
    pass.set_bind_group(0, fresh, &[]);
    pass.set_bind_group(2, &p.hw_masks, &[]);
    pass.set_bind_group(3, r.lighting.bind(), &[]);
    pass.dispatch_workgroups(
        size.width.div_ceil(tile[0]),
        size.height.div_ceil(tile[1]),
        1,
    );
}

/// One pose's complete hardware and software outputs.
struct Outputs {
    hw_hit: Vec<u8>,
    hw_fused: Vec<u8>,
    hw_split: Vec<u8>,
    hw_epsilon: Vec<u8>,
    hw_masks: [Vec<u32>; 2],
    hw_exact: Vec<u8>,
    sw_hit: Vec<u8>,
    sw_epsilon: Vec<u8>,
    sw_shadows: Vec<u8>,
    sw_exact: Vec<u8>,
    sw_fused_hit: Vec<u8>,
    sw_fused: Vec<u8>,
    /// Software masks of the shadow, Exact and fused diagnostic passes.
    sw_masks: [Vec<u32>; 3],
    /// Counts of those diagnostic passes, which must equal production's.
    diag_counts: Vec<Vec<u8>>,
}

fn read_tex(r: &Renderer, t: &wgpu::Texture) -> Vec<u8> {
    read_texture(r, t)
}
/// Software hit records (zeroed where uncovered) and counts (256 there) of the last
/// software camera or fused pass.
fn sw_records(r: &Renderer, sw: &Software, size: [usize; 2], counts: bool) -> (Vec<u8>, Vec<u8>) {
    let scene = r.primary.as_ref().unwrap().scene;
    let miss = uncovered_by(&read_tex(r, &sw.tiles), scene, size);
    let mut hit = read_tex(r, &sw.hit);
    let mut c = if counts {
        read_tex(r, &sw.counts)
    } else {
        vec![]
    };
    for (i, m) in miss.iter().enumerate() {
        if *m {
            hit[i * 16..i * 16 + 16].fill(0);
            if counts {
                c[i * 4..i * 4 + 4].copy_from_slice(&256u32.to_le_bytes());
            }
        }
    }
    (hit, c)
}

/// The mask words of every pixel (8 per pixel), without the dump-slot tail.
fn read_masks(r: &Renderer, sw: &Software, size: [usize; 2]) -> Vec<u32> {
    let mut masks = u32s(&read_buffer(r, &sw.masks));
    masks.truncate(size[0] * size[1] * 8);
    masks
}

/// Runs software diagnostic pass `which` (8-ray, Exact or fused) with dump `slots` per pixel
/// (none by default) and returns its masks and counts.
fn sw_diag(
    r: &Renderer,
    sw: &Software,
    p: &Pipelines,
    which: usize,
    size: [usize; 2],
    slots: Option<&[u32]>,
) -> (Vec<u32>, Vec<u8>) {
    let none = vec![u32::MAX; size[0] * size[1]];
    let slots = slots.unwrap_or(&none);
    let bytes: Vec<u8> = slots.iter().flat_map(|w| w.to_le_bytes()).collect();
    let n = (size[0] * size[1]) as u64;
    r.queue.write_buffer(&sw.masks, n * 32, &bytes);
    let g = if which == 1 {
        [(size[0] as u32).div_ceil(2), (size[1] as u32).div_ceil(2)]
    } else {
        [(size[0] as u32).div_ceil(8), (size[1] as u32).div_ceil(4)]
    };
    submit(r, |e| {
        e.clear_buffer(&sw.masks, 0, Some(n * 32));
        e.clear_buffer(&sw.dump, 0, None);
        sw.pass(r, e, &p.sw_diag[which], g, None);
    });
    let masks = read_masks(r, sw, size);
    let counts = if which == 2 {
        sw_records(r, sw, size, true).1
    } else {
        read_tex(r, &sw.counts)
    };
    (masks, counts)
}

fn pose_outputs(r: &Renderer, sw: &Software, p: &Pipelines, size: [usize; 2]) -> Outputs {
    let primary = r.primary.as_ref().unwrap();
    let hw_targets = primary.targets.as_ref().unwrap();
    let fresh = &r
        .fresh
        .as_ref()
        .unwrap()
        .targets
        .as_ref()
        .unwrap()
        .visibility
        .texture;
    let [hw_hit, hw_fused] = interactive_outputs(r);
    submit(r, |e| encode(r, sw, p, Pass::HwCamera, e, None));
    let hw_epsilon = read_tex(r, &hw_targets.epsilon.texture);
    let receivers = &r.pipeline.primary.as_ref().unwrap().trace[1];
    submit(r, |e| {
        r.encode_primary_trace(e, receivers, primary_ray_gpu::TRACE_TILE, true, None);
        encode(r, sw, p, Pass::HwShadows, e, None);
    });
    assert_eq!(
        hit_record(r),
        hw_hit,
        "receiver trace hit records differ from the fused pass"
    );
    let hw_split = read_tex(r, fresh);
    let mut hw_masks = [vec![], vec![]];
    let mut diag_counts = vec![];
    for exact in [false, true] {
        submit(r, |e| {
            e.clear_buffer(&sw.masks, 0, None);
            hw_diag(r, p, exact, e);
        });
        hw_masks[usize::from(exact)] = read_masks(r, sw, size);
        diag_counts.push(read_tex(r, fresh));
    }
    submit(r, |e| encode(r, sw, p, Pass::HwExact, e, None));
    let hw_exact = read_tex(r, fresh);
    submit(r, |e| encode(r, sw, p, Pass::SwCamera, e, None));
    let (sw_hit, _) = sw_records(r, sw, size, false);
    let sw_epsilon = read_tex(r, &sw.epsilon);
    submit(r, |e| encode(r, sw, p, Pass::SwShadows, e, None));
    let sw_shadows = read_tex(r, &sw.counts);
    submit(r, |e| encode(r, sw, p, Pass::SwExact, e, None));
    let sw_exact = read_tex(r, &sw.counts);
    submit(r, |e| encode(r, sw, p, Pass::SwFused, e, None));
    let (sw_fused_hit, sw_fused) = sw_records(r, sw, size, true);
    let mut sw_masks = [vec![], vec![], vec![]];
    for (which, masks) in sw_masks.iter_mut().enumerate() {
        let (m, c) = sw_diag(r, sw, p, which, size, None);
        *masks = m;
        diag_counts.push(c);
    }
    Outputs {
        hw_hit,
        hw_fused,
        hw_split,
        hw_epsilon,
        hw_masks,
        hw_exact,
        sw_hit,
        sw_epsilon,
        sw_shadows,
        sw_exact,
        sw_fused_hit,
        sw_fused,
        sw_masks,
        diag_counts,
    }
}

/// Visible count implied by a mask: 256 - 32 * blocked for 8 rays, 256 - blocked for 256.
fn mask_count(masks: &[u32], pixel: usize, exact: bool) -> u32 {
    let w = &masks[pixel * 8..pixel * 8 + 8];
    if exact {
        256 - w.iter().map(|m| m.count_ones()).sum::<u32>()
    } else {
        256 - 32 * (w[0] & 0xff).count_ones()
    }
}

/// Pixels whose two masks differ, with `include` restricting the pixels compared.
fn differing(a: &[u32], b: &[u32], include: impl Fn(usize) -> bool) -> Vec<usize> {
    (0..a.len() / 8)
        .filter(|&i| include(i) && a[i * 8..i * 8 + 8] != b[i * 8..i * 8 + 8])
        .collect()
}

/// Dumps the software rays of `pixels` with diagnostic pass `which` and classifies every
/// ray whose hardware and software mask bits differ.
#[allow(clippy::too_many_arguments)]
fn classify_masks(
    r: &Renderer,
    sw: &Software,
    p: &Pipelines,
    world: &World<'_>,
    which: usize,
    size: [usize; 2],
    hw: &[u32],
    pixels: &[usize],
) -> Classes {
    let directions = if which == 1 { 256 } else { 8 };
    let capacity = (DUMP_RAYS / directions as u64) as usize;
    let mut classes = Classes::default();
    for chunk in pixels.chunks(capacity) {
        let mut slots = vec![u32::MAX; size[0] * size[1]];
        for (s, &pixel) in chunk.iter().enumerate() {
            slots[pixel] = s as u32;
        }
        let (masks, _) = sw_diag(r, sw, p, which, size, Some(&slots));
        let rays = f32s(&read_buffer(r, &sw.dump));
        for (s, &pixel) in chunk.iter().enumerate() {
            for j in 0..directions {
                let bit = |m: &[u32]| m[pixel * 8 + j / 32] >> (j % 32) & 1 != 0;
                let (hw_blocked, sw_blocked) = (bit(hw), bit(&masks));
                if hw_blocked == sw_blocked {
                    continue;
                }
                let i = (s * directions + j) * 8;
                let ray: [f32; 8] = rays[i..i + 8].try_into().unwrap();
                let (label, margin, deciding) = classify_shadow(world, ray, hw_blocked);
                classes.add(label, margin, || {
                    serde_json::json!({"pixel": [pixel % size[0], pixel / size[0]], "direction": j,
                        "hardware_blocked": hw_blocked, "ray": ray, "triangles": deciding})
                });
            }
        }
    }
    classes
}

fn records(bytes: &[u8]) -> Vec<[u32; 4]> {
    u32s(bytes)
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect()
}

/// Compares one pose's outputs and classifies every difference.
#[allow(clippy::too_many_arguments)]
fn pose_parity(
    r: &Renderer,
    sw: &Software,
    p: &Pipelines,
    world: &World<'_>,
    o: &Outputs,
    inverse: DMat4,
    size: [usize; 2],
    out: &std::path::Path,
    name: &str,
) -> serde_json::Value {
    let n = size[0] * size[1];
    let (hw, swr, fused) = (
        records(&o.hw_hit),
        records(&o.sw_hit),
        records(&o.sw_fused_hit),
    );
    // Kernel 1: camera hit records.
    let mut camera = Classes::default();
    let (mut identical, mut rounded, mut max_rounded) = (0, 0, 0f64);
    let (mut covered_hw, mut covered_sw) = (0, 0);
    let mut camera_pixels = vec![false; n];
    for i in 0..n {
        let (a, b) = (hw[i], swr[i]);
        covered_hw += usize::from(a[0] != 0);
        covered_sw += usize::from(b[0] != 0);
        if a == b {
            identical += 1;
            continue;
        }
        let pixel = [i % size[0], i / size[0]];
        if a[..2] == b[..2] {
            let distance = world.point(a).distance(world.point(b));
            if distance <= 1e-4 {
                rounded += 1;
                max_rounded = max_rounded.max(distance);
                continue;
            }
            camera_pixels[i] = true;
            let v = world.triangle(a[0] - 1, a[1]);
            let normal = (v[1] - v[0]).cross(v[2] - v[0]).normalize();
            let eye = world.point(a);
            let ndc = DVec2::new(
                (pixel[0] as f64 + 0.5) / size[0] as f64 * 2.0 - 1.0,
                1.0 - (pixel[1] as f64 + 0.5) / size[1] as f64 * 2.0,
            );
            let near = inverse * DVec4::new(ndc.x, ndc.y, 1.0, 1.0);
            let d = (eye - near.truncate() / near.w).normalize();
            let cos = normal.dot(d).abs();
            let tol = world.tol(
                a[0] - 1,
                a[1],
                near.truncate() / near.w,
                &v,
                (eye - near.truncate() / near.w).length(),
            );
            let label = if distance <= 4.0 * tol / cos.max(1e-12) {
                "grazing"
            } else {
                "other"
            };
            camera.add(label, distance, || serde_json::json!({"pixel": pixel, "hardware": a, "software": b, "distance_m": distance, "cos": cos}));
            continue;
        }
        camera_pixels[i] = true;
        let (label, margin) = classify_camera(world, inverse, pixel, size, a, b);
        camera.add(
            label,
            margin,
            || serde_json::json!({"pixel": pixel, "hardware": a, "software": b}),
        );
    }
    let epsilon_same = (0..n)
        .filter(|&i| {
            hw[i] == swr[i]
                && hw[i][0] != 0
                && o.hw_epsilon[i * 4..i * 4 + 4] == o.sw_epsilon[i * 4..i * 4 + 4]
        })
        .count();
    // Hardware self-consistency: split counts equal fused ones; diagnostic masks and counts
    // reproduce production counts.
    let hw_split_equal = o.hw_split == o.hw_fused;
    let hw_masks_consistent = (0..n).all(|i| {
        u32s(&o.hw_split[i * 4..i * 4 + 4])[0] == mask_count(&o.hw_masks[0], i, false)
            && u32s(&o.hw_exact[i * 4..i * 4 + 4])[0] == mask_count(&o.hw_masks[1], i, true)
    });
    let diag_equal = [
        o.diag_counts[0] == o.hw_split,
        o.diag_counts[1] == o.hw_exact,
        o.diag_counts[2] == o.sw_shadows,
        o.diag_counts[3] == o.sw_exact,
        o.diag_counts[4] == o.sw_fused,
    ];
    let sw_masks_consistent = (0..n).all(|i| {
        u32s(&o.sw_shadows[i * 4..i * 4 + 4])[0] == mask_count(&o.sw_masks[0], i, false)
            && u32s(&o.sw_exact[i * 4..i * 4 + 4])[0] == mask_count(&o.sw_masks[1], i, true)
    });
    // Kernel 2 (8 rays) and Exact (256) from the hardware receivers: by count and by ray.
    let count_diff = |a: &[u8], b: &[u8], include: &dyn Fn(usize) -> bool| {
        (0..n)
            .filter(|&i| include(i) && a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4])
            .count()
    };
    let all = |_: usize| true;
    let shadow_pixels = differing(&o.hw_masks[0], &o.sw_masks[0], all);
    let exact_pixels = differing(&o.hw_masks[1], &o.sw_masks[1], all);
    let shadows = classify_masks(r, sw, p, world, 0, size, &o.hw_masks[0], &shadow_pixels);
    let exact = classify_masks(r, sw, p, world, 1, size, &o.hw_masks[1], &exact_pixels);
    // Kernel 3 (fused): records must equal kernel 1's; counts compare where the record
    // equals the hardware one, ray by ray against the hardware 8-ray masks.
    let same = |i: usize| fused[i][..2] == hw[i][..2];
    let fused_pixels = differing(&o.hw_masks[0], &o.sw_masks[2], same);
    let fused_classes = classify_masks(r, sw, p, world, 2, size, &o.hw_masks[0], &fused_pixels);
    let rays = |m: &[u32], k: &[u32], pixels: &[usize]| -> u32 {
        pixels
            .iter()
            .map(|&i| {
                (0..8)
                    .map(|w| (m[i * 8 + w] ^ k[i * 8 + w]).count_ones())
                    .sum::<u32>()
            })
            .sum()
    };
    // Mismatch map: grey coverage, red hit records, blue 8-ray receivers, green Exact.
    let mut image = vec![0u8; n * 3];
    for i in 0..n {
        let g = if hw[i][0] != 0 { 60 } else { 0 };
        image[i * 3..i * 3 + 3].copy_from_slice(&[g, g, g]);
    }
    for &i in &exact_pixels {
        image[i * 3..i * 3 + 3].copy_from_slice(&[0, 255, 0]);
    }
    for &i in &shadow_pixels {
        image[i * 3..i * 3 + 3].copy_from_slice(&[40, 120, 255]);
    }
    for (i, &bad) in camera_pixels.iter().enumerate() {
        if bad {
            image[i * 3..i * 3 + 3].copy_from_slice(&[255, 40, 40]);
        }
    }
    let file = fs::File::create(out.join(format!("{name}-mismatch.png"))).unwrap();
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        size[0] as u32,
        size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&image)
        .unwrap();
    serde_json::json!({
        "hardware_self_consistent": {"split_equals_fused": hw_split_equal, "masks_reproduce_counts": hw_masks_consistent},
        "diagnostic_counts_equal_production": diag_equal,
        "software_masks_reproduce_counts": sw_masks_consistent,
        "camera": {"covered_hardware": covered_hw, "covered_software": covered_sw,
            "identical_records": identical, "same_triangle_within_0.1mm": rounded,
            "same_triangle_max_m": max_rounded, "mismatches": camera,
            "epsilon_identical_at_identical_records": epsilon_same},
        "shadows8": {"count_mismatch_receivers": count_diff(&o.hw_split, &o.sw_shadows, &all),
            "ray_mismatch_receivers": shadow_pixels.len(),
            "ray_mismatches": rays(&o.hw_masks[0], &o.sw_masks[0], &shadow_pixels), "classes": shadows},
        "exact256": {"count_mismatch_receivers": count_diff(&o.hw_exact, &o.sw_exact, &all),
            "ray_mismatch_receivers": exact_pixels.len(),
            "ray_mismatches": rays(&o.hw_masks[1], &o.sw_masks[1], &exact_pixels), "classes": exact},
        "fused": {"records_equal_camera_kernel": o.sw_fused_hit == o.sw_hit,
            "receivers_on_the_hardware_triangle": (0..n).filter(|&i| same(i) && hw[i][0] != 0).count(),
            "count_mismatch_receivers_on_the_hardware_triangle": count_diff(&o.hw_fused, &o.sw_fused, &same),
            "ray_mismatch_receivers": fused_pixels.len(),
            "ray_mismatches": rays(&o.hw_masks[0], &o.sw_masks[2], &fused_pixels), "classes": fused_classes},
        "others": camera.others() + shadows.others() + exact.others() + fused_classes.others(),
    })
}

/// Hashes of the software outputs of one pose, for repeat and return identity.
/// Epsilon is written at camera hits only, like the hardware target, so it is hashed there.
fn software_hashes(o: &Outputs) -> Vec<String> {
    let mut epsilon = o.sw_epsilon.clone();
    for (i, record) in o.sw_hit.chunks_exact(16).enumerate() {
        if record[..4] == [0; 4] {
            epsilon[i * 4..i * 4 + 4].fill(0);
        }
    }
    [
        &o.sw_hit,
        &epsilon,
        &o.sw_shadows,
        &o.sw_exact,
        &o.sw_fused_hit,
        &o.sw_fused,
    ]
    .iter()
    .map(|b| hash(b))
    .collect()
}

/// One timed pair: a production pass and a software pass of some variant.
struct Timed<'a> {
    name: String,
    hw: Pass,
    sw: &'a Software,
    pipeline: &'a wgpu::ComputePipeline,
    groups: [u32; 2],
}

/// Paired timing with one short submission per pair, so no long dispatch shares a
/// submission: each repetition visits every entry in rotating order, hardware first on even
/// repetitions and software first on odd ones.
fn time_pairs(
    r: &Renderer,
    sw: &Software,
    p: &Pipelines,
    entries: &[Timed<'_>],
    warmup: usize,
    reps: usize,
) -> serde_json::Value {
    let queries = r.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("software probe timing"),
        ty: wgpu::QueryType::Timestamp,
        count: 4,
    });
    let resolve = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("software probe stamps"),
        size: 32,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let period = f64::from(r.queue.get_timestamp_period()) * 1e-6;
    let mut samples = vec![[vec![], vec![]]; entries.len()];
    let started = Instant::now();
    for rep in 0..warmup + reps {
        for k in 0..entries.len() {
            let i = (k + rep) % entries.len();
            let e = &entries[i];
            let mut encoder = r.device.create_command_encoder(&Default::default());
            let order = if rep % 2 == 0 { [0, 1] } else { [1, 0] };
            for side in order {
                let stamps = wgpu::ComputePassTimestampWrites {
                    query_set: &queries,
                    beginning_of_pass_write_index: Some(2 * side),
                    end_of_pass_write_index: Some(2 * side + 1),
                };
                if side == 0 {
                    encode(r, sw, p, e.hw, &mut encoder, Some(stamps));
                } else {
                    e.sw.pass(r, &mut encoder, e.pipeline, e.groups, Some(stamps));
                }
            }
            encoder.resolve_query_set(&queries, 0..4, &resolve, 0);
            r.queue.submit([encoder.finish()]);
            let stamps: Vec<u64> = u32s(&read_buffer(r, &resolve))
                .chunks_exact(2)
                .map(|w| u64::from(w[0]) | u64::from(w[1]) << 32)
                .collect();
            if rep >= warmup {
                for side in 0..2 {
                    samples[i][side]
                        .push((stamps[2 * side + 1] - stamps[2 * side]) as f64 * period);
                }
            }
        }
    }
    let median = |v: &[f64]| {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
    };
    let spread = |v: &[f64]| {
        serde_json::json!({"median_ms": median(v), "min_ms": v.iter().copied().fold(f64::MAX, f64::min),
            "max_ms": v.iter().copied().fold(0.0, f64::max)})
    };
    let mut summary = serde_json::Map::new();
    for (e, [hw, swv]) in entries.iter().zip(&samples) {
        let ratios: Vec<f64> = swv.iter().zip(hw).map(|(s, h)| s / h).collect();
        summary.insert(
            e.name.clone(),
            serde_json::json!({"hardware": spread(hw), "software": spread(swv),
                "paired_ratio_median": median(&ratios), "samples_hardware_ms": hw, "samples_software_ms": swv}),
        );
    }
    summary.insert(
        "host_unix_seconds".into(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            .into(),
    );
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    serde_json::Value::Object(summary)
}

/// Software camera, 8-ray and fused pipelines of one screen variant: override constants
/// and exact text edits of the composed module.
fn variant_pipelines(
    r: &Renderer,
    sw: &Software,
    constants: &[(&str, f64)],
    edits: &[(String, String)],
    shape: (usize, usize, u32),
) -> [wgpu::ComputePipeline; 3] {
    let scopes = OperationScopes::new(&r.device);
    let soft = &primary_ray_gpu::shader_sources(SOURCE).unwrap()[1];
    let mut source = bvh::traced_source(soft, &bvh::constants(shape.0, shape.1, shape.2));
    for (find, replace) in edits {
        source = edit(&source, (find, replace));
    }
    let traced = module(r, &source);
    let eight: Vec<_> = constants.iter().copied().chain(EIGHT).collect();
    let p = [
        compute(r, &sw.layout, &traced, "primary_trace", constants),
        compute(r, &sw.layout, &traced, "fresh_compute", &eight),
        compute(r, &sw.layout, &traced, "primary_trace_interactive", &eight),
    ];
    if let Some(e) = scopes.finish() {
        panic!("screen variant failed to compile: {e}");
    }
    p
}

/// Hashes of a variant's software camera records, 8-ray counts and fused records and counts.
fn variant_hashes(
    r: &Renderer,
    sw: &Software,
    p: &[wgpu::ComputePipeline; 3],
    size: [usize; 2],
) -> Vec<String> {
    let g = [(size[0] as u32).div_ceil(8), (size[1] as u32).div_ceil(4)];
    submit(r, |e| sw.pass(r, e, &p[0], g, None));
    let (hit, _) = sw_records(r, sw, size, false);
    submit(r, |e| sw.pass(r, e, &p[1], g, None));
    let counts = read_tex(r, &sw.counts);
    submit(r, |e| sw.pass(r, e, &p[2], g, None));
    let (fused_hit, fused) = sw_records(r, sw, size, true);
    [hit, counts, fused_hit, fused]
        .iter()
        .map(|b| hash(b))
        .collect()
}

/// The quarry CPU checks: invariants and brute force on `rays` random rays per mesh.
fn cpu_checks(
    quarry: &Quarry,
    bvhs: &[Bvh],
    max_leaf: u32,
    rays: usize,
    threads: usize,
) -> serde_json::Value {
    let mut rng = Rng(0x0000_9e37);
    let mut meshes = vec![];
    for (mesh, b) in quarry.meshes.iter().zip(bvhs) {
        let started = Instant::now();
        let t = tris(mesh);
        b.check(t, max_leaf).unwrap();
        let checked = started.elapsed().as_secs_f64();
        let brute = bvh::Brute::new(t, scale(b));
        let r = random_rays(t, b.bounds, rays, &mut rng);
        let bad = cpu_mismatches(b, t, &brute, &r, threads);
        let hits = r
            .iter()
            .filter(|ray| b.trace(t, ray, Mode::Closest).is_some())
            .count();
        let line = serde_json::json!({"triangles": t.len(), "rays": rays, "hits": hits,
            "mismatches": bad.len(), "invariant_s": checked,
            "brute_s": started.elapsed().as_secs_f64() - checked});
        eprintln!("software_cpu_check {line}");
        assert!(
            bad.is_empty(),
            "BVH and brute force differ: {:?}",
            r[bad[0]]
        );
        meshes.push(line);
    }
    serde_json::json!({"rays_per_mesh": rays, "threads": threads, "meshes": meshes})
}

#[test]
#[ignore = "GPU lock, 4 GiB RAM, acknowledgment and RENDERLAB_PRIMARY_MANIFEST; offscreen"]
fn gpu_software_bvh_probe() {
    let manifest = manifest();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let max_leaf = manifest["max_leaf"].as_u64().unwrap_or(4) as u32;
    let pad = manifest["pad_ulps"].as_f64().unwrap_or(16.0) as f32;
    let started = Instant::now();
    let mut summary = serde_json::Map::new();
    let write = |summary: &serde_json::Map<String, serde_json::Value>, name: &str| {
        fs::write(out.join(name), serde_json::to_vec_pretty(summary).unwrap()).unwrap();
    };
    if manifest["cpu_only"].as_bool().unwrap_or(false) {
        let quarry = decode_quarry(&manifest);
        let (bvhs, build) = build_all(&quarry.meshes, max_leaf, pad);
        summary.insert("build".into(), build);
        // f64 replay of listed shadow rays: [origin, tmin, direction, tmax, hardware blocked].
        if let Some(list) = manifest["replay_rays"].as_array() {
            let placed: Vec<_> = quarry
                .models
                .iter()
                .enumerate()
                .flat_map(|(slot, models)| models.iter().map(move |&m| (slot, m)))
                .collect();
            let world = World::new(&quarry.meshes, &bvhs, &placed);
            let replays: Vec<_> = list
                .iter()
                .map(|r| {
                    let ray: [f32; 8] = std::array::from_fn(|i| r[i].as_f64().unwrap() as f32);
                    let (label, margin, deciding) = classify_shadow(&world, ray, r[8].as_bool().unwrap());
                    serde_json::json!({"ray": ray, "label": label, "margin": margin, "triangles": deciding})
                })
                .collect();
            summary.insert("replays".into(), replays.into());
        }
        let rays = manifest["cpu_rays"].as_u64().unwrap_or(10_000) as usize;
        let threads = manifest["threads"].as_u64().unwrap_or(4) as usize;
        if rays > 0 {
            summary.insert(
                "cpu".into(),
                cpu_checks(&quarry, &bvhs, max_leaf, rays, threads),
            );
        }
        summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
        write(&summary, "cpu.json");
        return;
    }
    let validate = manifest["validate"].as_bool().unwrap_or(false);
    let _validation = validation::begin(validate);
    let mut r = renderer(validate);
    summary.insert("validation".into(), validate.into());
    // Fixtures first: the GPU kernel against the CPU reference on small meshes.
    let mut rng = Rng(0xf17e);
    let arithmetic = arithmetic_probe(&r, &mut rng);
    eprintln!("software_arithmetic {arithmetic}");
    summary.insert("arithmetic".into(), arithmetic);
    let fixtures = [soup(&mut rng, 3000), grid(&mut rng, 40)];
    let fixture_bvhs: Vec<Bvh> = fixtures
        .iter()
        .map(|m| bvh::build(tris(m), max_leaf, pad))
        .collect();
    let fixture_buffers: Vec<_> = fixtures.iter().map(|m| fixture_buffers(&r, m)).collect();
    let refs: Vec<&Bvh> = fixture_bvhs.iter().collect();
    let fixture_tris: Vec<_> = fixtures.iter().map(tris).collect();
    let queries = mesh_queries(&fixture_tris, &refs, 4000, &mut rng);
    let buffers: Vec<_> = fixture_buffers.iter().map(|(v, i)| (v, i)).collect();
    let results = gpu_rays(&r, &buffers, &bvh::pack(&refs), &queries);
    let mut examples = vec![];
    let fixture = compare_rays(&fixture_tris, &refs, &queries, &results, &mut examples);
    eprintln!("software_fixture {fixture:?}");
    fs::write(
        out.join("fixture-others.json"),
        serde_json::to_vec_pretty(&examples).unwrap(),
    )
    .unwrap();
    assert!(
        !fixture.contains_key("other"),
        "fixture kernel differs from the CPU reference: {fixture:?}"
    );
    summary.insert(
        "fixture_rays".into(),
        serde_json::to_value(&fixture).unwrap(),
    );
    // The quarry, loaded once.
    let mut quarry = decode_quarry(&manifest);
    let (bvhs, build) = build_all(&quarry.meshes, max_leaf, pad);
    summary.insert("build".into(), build);
    let source = quarry.source.take().unwrap();
    let (mut asset, cameras) = upload_quarry(&manifest, &mut r, source);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(crate::SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    let size = [1920usize, 1080];
    let poses = [0usize, 1, 0];
    traced_frame(
        &mut r,
        &color,
        &cameras.frames[0].camera().unwrap(),
        &mut asset,
        true,
    );
    r.test_drain("software probe first frame");
    let refs: Vec<&Bvh> = bvhs.iter().collect();
    let packed = bvh::pack(&refs);
    let (sw, placements) = Software::new(&r, &mut asset, &bvhs, &packed);
    {
        let draws = asset.draws(Visibility::Frustum);
        assert_eq!(draws.len(), quarry.meshes.len());
        for ((d, mesh), models) in draws.iter().zip(&quarry.meshes).zip(&quarry.models) {
            assert_eq!(d.mesh.index_count as usize, mesh.1.len());
            assert_eq!(&d.instances.state.models, models);
        }
        // GPU kernel against the CPU reference on random object-space quarry rays.
        let buffers: Vec<_> = draws
            .iter()
            .map(|d| (&**d.mesh.vertex, &**d.mesh.index))
            .collect();
        let quarry_tris: Vec<_> = quarry.meshes.iter().map(tris).collect();
        let per_mesh = manifest["gpu_rays"].as_u64().unwrap_or(10_000) as usize;
        let queries = mesh_queries(&quarry_tris, &refs, per_mesh, &mut rng);
        let results = gpu_rays(&r, &buffers, &packed, &queries);
        let mut examples = vec![];
        let classes = compare_rays(&quarry_tris, &refs, &queries, &results, &mut examples);
        fs::write(
            out.join("quarry-ray-others.json"),
            serde_json::to_vec_pretty(&examples).unwrap(),
        )
        .unwrap();
        eprintln!("software_quarry_rays {classes:?}");
        summary.insert(
            "quarry_rays".into(),
            serde_json::to_value(&classes).unwrap(),
        );
    }
    let instances_bytes = placements.len() * bvh::INSTANCE_BYTES;
    let gpu_bytes = packed.bytes() + instances_bytes;
    let sum = |v: &[Vec<u8>]| v.iter().map(Vec::len).sum::<usize>();
    summary.insert(
        "memory".into(),
        serde_json::json!({"nodes": sum(&packed.nodes), "prims": sum(&packed.prims),
        "instances": instances_bytes, "total": gpu_bytes, "stack": packed.stack}),
    );
    let sided = placements.iter().any(|p| p.2);
    let p = Pipelines::new(&r, &sw, quarry.meshes.len(), placements.len(), packed.stack);
    let placed: Vec<_> = placements
        .iter()
        .map(|&(slot, model, _)| (slot, model))
        .collect();
    let world = World::new(&quarry.meshes, &bvhs, &placed);
    let mut poses_out = vec![];
    let mut first_hashes = vec![];
    let mut others = 0;
    // `timing_only` skips parity for an unvalidated timing process.
    let parity_poses = if manifest["timing_only"].as_bool().unwrap_or(false) {
        &poses[..0]
    } else {
        &poses[..]
    };
    for (index, &pose) in parity_poses.iter().enumerate() {
        let camera = cameras.frames[pose].camera().unwrap();
        traced_frame(&mut r, &color, &camera, &mut asset, true);
        r.test_drain("software probe pose");
        r.queue
            .write_buffer(&sw.frame, 0, &frame_bytes(&r, &camera, [1920, 1080], sided));
        let o = pose_outputs(&r, &sw, &p, size);
        let hashes = software_hashes(&o);
        let inverse = camera.matrix(1920, 1080).inverse().as_dmat4();
        let mut parity = pose_parity(
            &r,
            &sw,
            &p,
            &world,
            &o,
            inverse,
            size,
            &out,
            &format!("pose-{index}"),
        );
        others += parity["others"].as_u64().unwrap();
        if index == 0 {
            // Eight repeats of every software kernel must reproduce the first outputs.
            let mut repeats = 0;
            for _ in 0..8 {
                assert_eq!(
                    software_hashes(&pose_outputs(&r, &sw, &p, size)),
                    hashes,
                    "software repeat differs"
                );
                repeats += 1;
            }
            parity["identical_repeats"] = repeats.into();
            first_hashes = hashes.clone();
        }
        if index == 2 {
            parity["return_identical"] = (hashes == first_hashes).into();
            assert_eq!(hashes, first_hashes, "return pose differs from pose 0");
        }
        eprintln!("software_pose {index} others={}", parity["others"]);
        parity["software_hashes"] = hashes.into();
        poses_out.push(parity);
        summary.insert("poses".into(), poses_out.clone().into());
        write(&summary, "summary.json");
    }
    // Paired timing on pose 0, after a fresh hardware receiver trace.
    let camera = cameras.frames[0].camera().unwrap();
    traced_frame(&mut r, &color, &camera, &mut asset, true);
    r.test_drain("software probe timing inputs");
    r.queue
        .write_buffer(&sw.frame, 0, &frame_bytes(&r, &camera, [1920, 1080], sided));
    let receivers = &r.pipeline.primary.as_ref().unwrap().trace[1];
    submit(&r, |e| {
        r.encode_primary_trace(e, receivers, primary_ray_gpu::TRACE_TILE, true, None)
    });
    let warmup = manifest["warmup_reps"].as_u64().unwrap_or(8) as usize;
    let reps = manifest["reps"].as_u64().unwrap_or(24) as usize;
    let g8x4 = [1920u32.div_ceil(8), 1080u32.div_ceil(4)];
    let g2x2 = [1920u32.div_ceil(2), 1080u32.div_ceil(2)];
    let entry = |name: &str, hw, pipeline, groups| Timed {
        name: name.into(),
        hw,
        sw: &sw,
        pipeline,
        groups,
    };
    let mut entries = vec![
        entry("camera", Pass::HwCamera, &p.sw_camera, g8x4),
        entry("shadows8", Pass::HwShadows, &p.sw_shadows, g8x4),
        entry("fused", Pass::HwFused, &p.sw_fused, g8x4),
    ];
    if !manifest["skip_exact_timing"].as_bool().unwrap_or(false) {
        entries.push(entry("exact256", Pass::HwExact, &p.sw_exact, g2x2));
    }
    let timing = time_pairs(&r, &sw, &p, &entries, warmup, reps);
    // Hillclimb screen: variants by box padding and override constants. Non-diagnostic
    // variants must reproduce the baseline software outputs byte for byte.
    if let Some(variants) = manifest["screen"].as_array() {
        let baseline = variant_hashes(
            &r,
            &sw,
            &[
                p.sw_camera.clone(),
                p.sw_shadows.clone(),
                p.sw_fused.clone(),
            ],
            size,
        );
        let mut sets: Vec<(f32, Vec<Bvh>, bvh::Packed, Software)> = vec![];
        for v in variants {
            let pad_v = v["pad_ulps"].as_f64().map_or(pad, |x| x as f32);
            if pad_v != pad && !sets.iter().any(|s| s.0 == pad_v) {
                let (b, build) = build_all(&quarry.meshes, max_leaf, pad_v);
                summary.insert(format!("build_pad_{pad_v}"), build);
                let packed_v = bvh::pack(&b.iter().collect::<Vec<_>>());
                let (sw_v, _) = Software::new(&r, &mut asset, &b, &packed_v);
                r.queue.write_buffer(
                    &sw_v.frame,
                    0,
                    &frame_bytes(&r, &camera, [1920, 1080], sided),
                );
                sets.push((pad_v, b, packed_v, sw_v));
            }
        }
        let mut compiled = vec![];
        let mut identity = serde_json::Map::new();
        for v in variants {
            let name = v["name"].as_str().unwrap().to_owned();
            let pad_v = v["pad_ulps"].as_f64().map_or(pad, |x| x as f32);
            let (sw_v, stack) = match sets.iter().find(|s| s.0 == pad_v) {
                Some((_, _, packed_v, sw_v)) => (sw_v, packed_v.stack),
                None => (&sw, packed.stack),
            };
            let constants: Vec<(String, f64)> = v["constants"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, x)| {
                    (
                        k.clone(),
                        x.as_f64().or(x.as_bool().map(f64::from)).unwrap(),
                    )
                })
                .collect();
            let refs: Vec<(&str, f64)> = constants.iter().map(|(k, x)| (k.as_str(), *x)).collect();
            let edits: Vec<(String, String)> = v["edits"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    (
                        e[0].as_str().unwrap().to_owned(),
                        e[1].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            let pipelines = variant_pipelines(
                &r,
                sw_v,
                &refs,
                &edits,
                (quarry.meshes.len(), placements.len(), stack),
            );
            let hashes = variant_hashes(&r, sw_v, &pipelines, size);
            let same = hashes == baseline;
            assert!(
                name.starts_with("diag") || same,
                "{name} changes the software outputs"
            );
            identity.insert(name.clone(), same.into());
            compiled.push((name, sw_v, pipelines));
        }
        let mut screen = vec![];
        for (name, sw_v, pipelines) in &compiled {
            for (k, (kernel, hw)) in [
                ("camera", Pass::HwCamera),
                ("shadows8", Pass::HwShadows),
                ("fused", Pass::HwFused),
            ]
            .into_iter()
            .enumerate()
            {
                screen.push(Timed {
                    name: format!("{name}/{kernel}"),
                    hw,
                    sw: sw_v,
                    pipeline: &pipelines[k],
                    groups: g8x4,
                });
            }
        }
        for (kernel, hw, pipeline) in [
            ("camera", Pass::HwCamera, &p.sw_camera),
            ("shadows8", Pass::HwShadows, &p.sw_shadows),
            ("fused", Pass::HwFused, &p.sw_fused),
        ] {
            screen.push(entry(&format!("base/{kernel}"), hw, pipeline, g8x4));
        }
        let screen_reps = manifest["screen_reps"].as_u64().unwrap_or(8) as usize;
        let screened = time_pairs(&r, &sw, &p, &screen, 2, screen_reps);
        summary.insert(
            "screen".into(),
            serde_json::json!({"identical_outputs": identity, "timing": screened}),
        );
        write(&summary, "summary.json");
    }
    let fused = timing["fused"]["software"]["median_ms"].as_f64().unwrap();
    let shadows = timing["shadows8"]["software"]["median_ms"]
        .as_f64()
        .unwrap();
    let camera_ms = timing["camera"]["software"]["median_ms"].as_f64().unwrap();
    let projected = fused + 0.45 + 0.03;
    summary.insert("timing".into(), timing);
    summary.insert("stop_rules".into(), serde_json::json!({
        "unexplained_mismatches": others,
        "software_shadows8_ms": shadows, "shadows_above_1ms": shadows > 1.0,
        "software_primary_ms": camera_ms, "software_primaries_within_0.40ms": camera_ms <= 0.40,
        "projected_interactive_frame_ms": projected,
        "verdict": if projected <= 1.20 { "proceed to N1" } else if projected <= 1.61 { "proceed; beats VSM by less than 1.33x and misses 2x" } else { "quality-first mode only; record the cost miss" },
        "bvh_bytes": gpu_bytes, "bvh_above_0.5GB": gpu_bytes as f64 > 0.5e9,
    }));
    summary.insert("wall_s".into(), started.elapsed().as_secs_f64().into());
    write(&summary, "summary.json");
    eprintln!("software_probe_done others={others} projected_ms={projected:.4}");
    assert_eq!(others, 0, "unexplained mismatches; see summary.json");
}
