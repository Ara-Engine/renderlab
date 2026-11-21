//! What fixtures share: the bench's devices and renderers, meshes from quads, a frame that
//! waits for admission, and the HDR read back as radiance.
use super::{BenchPath, RefImage};
use crate::pipeline_compilation::PipelineDevice;
use crate::renderer::Renderer;
use crate::{
    Camera, Draw, Error, HardwareShadowOptions, Instances, Light, Lighting, Limits, Material, MaterialDescriptor, Mat4,
    Mesh, MeshData, PrimaryVisibility, Program, RendererOptions, SampleCount, Value, Vec3, Vertex, ViewMode,
};
use std::time::{Duration, Instant};

/// The scene prelude every bench renderer compiles with.
pub(crate) const SOURCE: &str = include_str!("../../shaders/scene.wgsl");
/// Colour targets: the frame's display image (fixtures read the HDR, not this).
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One headless Vulkan device per path, as the engine requests it (src/engine/device.rs: the
/// path's features and limits, so the reflection pass and its composite run as in the game, with
/// timestamps and the pipeline cache): with ray queries (`Rt`, which needs
/// RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK as every RT test does) or without them. Its facade holds
/// one pipeline cache for every renderer on the path, as the engine's does, saved under
/// `LIGHTBENCH_CACHE` (default D:/Coding/renderlab-work/bench-cache): a revert, an A/B toggle or a
/// restart is a cache hit, and a rig compiles nothing another rig on its path already compiled.
pub(crate) fn device(path: BenchPath) -> Result<(PipelineDevice, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).map_err(|e| e.to_string())?;
    let render_path = match path {
        BenchPath::Rt => {
            if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref() != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK") {
                return Err("the RT path needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK".into());
            }
            crate::engine::RenderPath::HardwareRt
        }
        BenchPath::NoRt => crate::engine::RenderPath::NonRt,
    };
    let (device, queue) = crate::engine::device::request_device(&adapter, render_path).map_err(|e| e.to_string())?;
    let directory = std::path::PathBuf::from(std::env::var("LIGHTBENCH_CACHE").unwrap_or_else(|_| "D:/Coding/renderlab-work/bench-cache".into()));
    let _ = std::fs::create_dir_all(&directory);
    let info = adapter.get_info();
    Ok((PipelineDevice::new(device, Some((&directory, &info))), queue))
}

/// A renderer on `device` for `path`, single-sampled; the RT one traces its primaries. Up to
/// 8192 instances per upload (the real-size fixtures).
pub(crate) fn renderer(device: &(PipelineDevice, wgpu::Queue), path: BenchPath) -> Result<Renderer, Error> {
    let (device, queue) = device.clone();
    let options = RendererOptions { samples: SampleCount::One, ..Default::default() };
    let limits = Limits { max_instances: 8192, ..Limits::default() };
    let hardware = (path == BenchPath::Rt).then(HardwareShadowOptions::default);
    let mut r = Renderer::new_configured(device, queue, FORMAT, limits, options, hardware, SOURCE)?;
    if path == BenchPath::Rt {
        r.set_primary_visibility(PrimaryVisibility::Traced)?;
    }
    Ok(r)
}

/// Waits for the renderer's submitted work and collects it (frame slots, readbacks and the
/// pass log's lines, which only `poll` writes).
pub(crate) fn drain(r: &mut Renderer) -> Result<(), Error> {
    r.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(20)) })
        .map_err(Error::Poll)?;
    r.poll().map(drop)
}

/// Retries `attempt` (an admission that returns `None` while the renderer is busy), draining
/// between tries, for at most 20 s; past that it fails with `what`.
pub(crate) fn admitted<T>(r: &mut Renderer, what: &'static str, mut attempt: impl FnMut(&mut Renderer) -> Result<Option<T>, Error>) -> Result<T, Error> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = attempt(r)? {
            return Ok(value);
        }
        if Instant::now() > until {
            return Err(Error::InvalidInput(what));
        }
        drain(r)?;
    }
}

/// A colour target the size of the fixture's view.
pub(crate) fn color_target(r: &Renderer, size: [u32; 2]) -> wgpu::Texture {
    r.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("bench color"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// One quad: centre, normal (its front side) and a tangent half-extent `u`; the other
/// half-extent is `half_v` along n x u. Wound so the front faces along `n`.
pub(crate) fn quad(centre: Vec3, n: Vec3, u: Vec3, half_v: f32) -> ([Vec3; 4], Vec3) {
    let v = n.cross(u).normalize() * half_v;
    ([centre - u - v, centre + u - v, centre + u + v, centre - u + v], n)
}
/// The six faces of an axis-aligned box of half-extents `h`, facing out (or in).
pub(crate) fn box_faces(centre: Vec3, h: Vec3, inward: bool) -> Vec<([Vec3; 4], Vec3)> {
    let s = if inward { -1.0 } else { 1.0 };
    [
        (Vec3::X, Vec3::Z * h.z, h.y, h.x),
        (-Vec3::X, Vec3::Z * h.z, h.y, h.x),
        (Vec3::Y, Vec3::X * h.x, h.z, h.y),
        (-Vec3::Y, Vec3::X * h.x, h.z, h.y),
        (Vec3::Z, Vec3::X * h.x, h.y, h.z),
        (-Vec3::Z, Vec3::X * h.x, h.y, h.z),
    ]
    .into_iter()
    .map(|(axis, u, half_v, reach)| quad(centre + axis * reach, axis * s, u, half_v))
    .collect()
}

/// A UV sphere of `rings` x `segments` quads, facing out.
pub(crate) fn sphere_mesh(centre: Vec3, radius: f32, rings: u32, segments: u32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    for i in 0..=rings {
        let theta = std::f32::consts::PI * i as f32 / rings as f32;
        for j in 0..=segments {
            let phi = std::f32::consts::TAU * j as f32 / segments as f32;
            let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
            vertices.push(vertex(centre + n * radius, n));
        }
    }
    let row = segments + 1;
    let mut indices = vec![];
    for i in 0..rings {
        for j in 0..segments {
            let (a, b, c, d) = (i * row + j, i * row + j + 1, (i + 1) * row + j + 1, (i + 1) * row + j);
            indices.extend([a, b, c, a, c, d]);
        }
    }
    (vertices, indices)
}
/// The distance along unit `d` from `o` to the convex quad `corners`, if the ray meets it.
pub(crate) fn quad_hit(o: Vec3, d: Vec3, corners: &[Vec3; 4]) -> Option<f32> {
    let n = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
    let denom = n.dot(d);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = n.dot(corners[0] - o) / denom;
    let p = o + d * t;
    let sign = |k: usize| (corners[(k + 1) % 4] - corners[k]).cross(p - corners[k]).dot(n);
    let inside = (0..4).all(|k| sign(k) >= 0.0) || (0..4).all(|k| sign(k) <= 0.0);
    (t > 0.0 && inside).then_some(t)
}
/// The distance along unit `d` from `o` to the sphere, if the ray meets it.
pub(crate) fn sphere_hit(o: Vec3, d: Vec3, centre: Vec3, radius: f32) -> Option<f32> {
    let oc = o - centre;
    let b = oc.dot(d);
    let disc = b * b - (oc.length_squared() - radius * radius);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

/// A drawable part: its mesh (also the lighting cache's software geometry), its instances and
/// its material.
pub(crate) struct Part {
    pub mesh: Mesh,
    pub instances: Instances,
    pub material: Material,
}
/// Uploads `faces` as one mesh with a double-sided material of `program`, `albedo` and
/// the program's `parameters`, at one identity instance.
pub(crate) fn part(
    r: &mut Renderer,
    faces: &[([Vec3; 4], Vec3)],
    program: &Program,
    albedo: [f32; 3],
    parameters: &[(&str, Value)],
) -> Result<Part, Error> {
    let (vertices, indices) = faces_mesh(faces);
    mesh_part(r, &vertices, &indices, program, albedo, parameters, &[Mat4::IDENTITY])
}
/// `faces` as one triangle mesh.
pub(crate) fn faces_mesh(faces: &[([Vec3; 4], Vec3)]) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (corners, n) in faces {
        let base = vertices.len() as u32;
        vertices.extend(corners.iter().map(|p| vertex(*p, *n)));
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
    }
    (vertices, indices)
}
/// A white vertex at `position` with `normal`.
pub(crate) fn vertex(position: Vec3, normal: Vec3) -> Vertex {
    Vertex { position: position.to_array(), normal: normal.to_array(), uv: [0.0; 2], color: [1.0; 4] }
}
/// `part` for any triangle mesh, placed at each of `transforms`.
pub(crate) fn mesh_part(
    r: &mut Renderer,
    vertices: &[Vertex],
    indices: &[u32],
    program: &Program,
    albedo: [f32; 3],
    parameters: &[(&str, Value)],
    transforms: &[Mat4],
) -> Result<Part, Error> {
    let material = MaterialDescriptor {
        base_color: [albedo[0], albedo[1], albedo[2], 1.0],
        metallic: 0.0,
        roughness: 1.0,
        double_sided: true,
        program: Some(program),
        parameters,
        ..Default::default()
    };
    material_part(r, vertices, indices, &material, transforms)
}
/// A part of any material (metal, glass, films), placed at each of `transforms`.
pub(crate) fn material_part(
    r: &mut Renderer,
    vertices: &[Vertex],
    indices: &[u32],
    material: &MaterialDescriptor<'_>,
    transforms: &[Mat4],
) -> Result<Part, Error> {
    let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
    let mesh = admitted(r, "bench mesh", |r| r.try_upload_mesh(&MeshData { vertices, indices }))?;
    r.set_indirect_geometry(&mesh, &positions, indices)?;
    let instances = admitted(r, "bench instances", |r| r.try_upload_instances(transforms))?;
    let material = admitted(r, "bench material", |r| r.try_create_material(material))?;
    Ok(Part { mesh, instances, material })
}

/// A camera for a fixture's own projections and its live reference: eye, target, vertical field
/// of view (radians) and size in pixels; up is +y, as `Camera::look_at` takes it.
#[derive(Clone, Copy)]
pub(crate) struct View {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov: f32,
    pub size: [u32; 2],
}
impl View {
    pub fn camera(&self) -> Result<Camera, Error> {
        Camera::look_at(self.eye, self.target, Vec3::Y, self.fov, 0.01, 100.0)
    }
    /// The camera's forward, right and up.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.target - self.eye).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        (forward, right, right.cross(forward))
    }
    fn tangents(&self) -> (f32, f32) {
        let tan = (self.fov * 0.5).tan();
        (tan * self.size[0] as f32 / self.size[1] as f32, tan)
    }
    /// The ray through the centre of pixel `(x, y)` (row 0 at the top).
    pub fn ray(&self, x: u32, y: u32) -> Vec3 {
        let (forward, right, up) = self.basis();
        let (tx, ty) = self.tangents();
        let u = ((x as f32 + 0.5) / self.size[0] as f32 * 2.0 - 1.0) * tx;
        let v = (1.0 - (y as f32 + 0.5) / self.size[1] as f32 * 2.0) * ty;
        (forward + right * u + up * v).normalize()
    }
    /// Where world point `p` lands, in pixels (x right, y down).
    pub fn project(&self, p: Vec3) -> [f32; 2] {
        let (forward, right, up) = self.basis();
        let (tx, ty) = self.tangents();
        let d = p - self.eye;
        let (u, v) = (d.dot(right) / d.dot(forward) / tx, d.dot(up) / d.dot(forward) / ty);
        [(u + 1.0) * 0.5 * self.size[0] as f32, (1.0 - v) * 0.5 * self.size[1] as f32]
    }
}

/// The live path tracer's picture (src/reference_live.rs: the engine's own scene and material
/// programs, NEE + MIS, no denoising) of the scene `build` makes, seen from `view` under
/// `lighting` (its sun with angular radius `sun_radius`, and its uniform sky), with `samples`
/// paths per pixel. It traces on a renderer of its own with ray hardware, so fixtures on either
/// path get one; the bench caches it on disk, so it runs once per scene. Frames keep their draws'
/// programs only with RENDERLAB_LIVE_REFERENCE set before the first frame (the bench sets it).
pub(crate) fn traced_reference(
    build: &dyn Fn(&mut Renderer) -> Result<Vec<Part>, Error>,
    view: &View,
    lighting: &Lighting,
    sun_radius: f32,
    samples: u32,
) -> Result<RefImage, Error> {
    let device = device(BenchPath::Rt).map_err(|_| Error::InvalidInput("the live reference needs the ray-tracing device"))?;
    let mut r = renderer(&device, BenchPath::Rt)?;
    r.set_atmosphere(crate::Atmosphere::off())?;
    r.set_lighting(lighting)?;
    let mut parts = build(&mut r)?;
    let target = color_target(&r, view.size);
    // A frame with the see-through draws kept traced, so the traced scene includes them.
    r.set_live_opaque_frame(true);
    let framed = frame(&mut r, &target, &view.camera()?, &mut parts);
    r.set_live_opaque_frame(false);
    framed?;
    drain(&mut r)?;
    let sun = lighting.lights.iter().find_map(|l| match *l {
        Light::Directional { travel, lux } => {
            Some(crate::reference::Sun { to_light: -travel.normalize(), illuminance: lux, angular_radius: sun_radius })
        }
        _ => None,
    });
    let (forward, right, up) = view.basis();
    let inputs = crate::reference_live::LiveInputs {
        eye: view.eye,
        forward,
        right,
        up,
        tan_half_fov: (view.fov * 0.5).tan(),
        size: view.size,
        sun,
        sky: lighting.environment,
        environment: None,
        lights: vec![],
        light_radii: vec![],
        samples,
        max_depth: 12,
        seed: 0x4d37_5530,
        progress: false,
        fog: None,
    };
    let image = r.trace_live_reference(&inputs)?;
    Ok(RefImage { size: view.size, rgb: image.mean })
}

/// Measurement (`LIGHTBENCH_FIXTURE_DUMP=<dir>`): `rgb` as `<dir>/<name>.pfm` (radiance, rows
/// bottom to top as PFM stores them), for looking at a fixture's frames beside its reference.
pub(crate) fn dump(name: &str, size: [u32; 2], rgb: &[[f32; 3]]) {
    let Ok(dir) = std::env::var("LIGHTBENCH_FIXTURE_DUMP") else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mut bytes = format!("PF\n{} {}\n-1.0\n", size[0], size[1]).into_bytes();
    for row in (0..size[1] as usize).rev() {
        for p in &rgb[row * size[0] as usize..(row + 1) * size[0] as usize] {
            bytes.extend(p.iter().flat_map(|c| c.to_le_bytes()));
        }
    }
    let _ = std::fs::write(format!("{dir}/{name}.pfm"), bytes);
}

/// Mean luminance over `mask` of `image` and of `reference`, and their mean absolute difference:
/// `(energy, err)` as ratios to the reference's mean (energy 1 and err 0 match it).
pub(crate) fn against(image: &[[f32; 3]], reference: &[[f32; 3]], mask: &[bool]) -> (f64, f64) {
    let (mut a, mut b, mut d, mut n) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    for ((p, q), &m) in image.iter().zip(reference).zip(mask) {
        if m {
            let (x, y) = (f64::from(luminance(*p)), f64::from(luminance(*q)));
            a += x;
            b += y;
            d += (x - y).abs();
            n += 1;
        }
    }
    if n == 0 || b <= 0.0 {
        return (f64::NAN, f64::NAN);
    }
    (a / b, d / b)
}

/// One lit frame of `parts` from `camera` into `target`, waiting for admission.
pub(crate) fn frame(r: &mut Renderer, target: &wgpu::Texture, camera: &Camera, parts: &mut [Part]) -> Result<(), Error> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let mut draws: Vec<Draw<'_>> =
            parts.iter_mut().map(|p| Draw::new(&p.mesh, &mut p.instances).with_material(&p.material)).collect();
        if r.try_render_view(target, camera, &mut draws, ViewMode::Lit)?.is_some() {
            return Ok(());
        }
        drop(draws);
        if Instant::now() > until {
            return Err(Error::InvalidInput("bench frame: not admitted in 20 s"));
        }
        drain(r)?;
    }
}

/// The last frame's HDR, un-exposed back to radiance (cd/m^2), RGB per pixel.
pub(crate) fn capture_radiance(r: &mut Renderer, exposure: f32) -> Result<Vec<[f32; 3]>, Error> {
    drain(r)?;
    let mut capture = admitted(r, "bench HDR capture", |r| r.try_capture_hdr())?;
    drain(r)?;
    let bytes = loop {
        if let Some(bytes) = capture.try_read()? {
            break bytes;
        }
        drain(r)?;
    };
    let half: Vec<f32> = bytes
        .chunks_exact(2)
        .map(|b| crate::decode_f16(u16::from_le_bytes([b[0], b[1]])))
        .collect();
    let scale = (-exposure).exp2();
    Ok(half.chunks_exact(4).map(|p| [p[0] * scale, p[1] * scale, p[2] * scale]).collect())
}

/// Rec. 709 luminance.
pub(crate) fn luminance(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}
