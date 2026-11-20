//! What fixtures share: the bench's devices and renderers, meshes from quads, a frame that
//! waits for admission, and the HDR read back as radiance.
use super::BenchPath;
use crate::renderer::Renderer;
use crate::{
    Camera, Draw, Error, HardwareShadowOptions, Instances, Limits, Material, MaterialDescriptor, Mat4, Mesh,
    MeshData, PrimaryVisibility, Program, RendererOptions, SampleCount, Value, Vec3, Vertex, ViewMode,
};
use std::time::{Duration, Instant};

/// The scene prelude every bench renderer compiles with.
pub(crate) const SOURCE: &str = include_str!("../../shaders/scene.wgsl");
/// Colour targets: the frame's display image (fixtures read the HDR, not this).
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One headless Vulkan device per path: with ray queries (`Rt`, which needs
/// RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK as every RT test does) or without them.
pub(crate) fn device(path: BenchPath) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).map_err(|e| e.to_string())?;
    let descriptor = match path {
        BenchPath::Rt => {
            if std::env::var("RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK").as_deref() != Ok("I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK") {
                return Err("the RT path needs RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK".into());
            }
            let a = adapter.limits();
            wgpu::DeviceDescriptor {
                required_features: crate::RECEIVER_FEATURES
                    | wgpu::Features::EXPERIMENTAL_RAY_QUERY
                    | crate::TRACED_FEATURES
                    | wgpu::Features::TIMESTAMP_QUERY,
                required_limits: crate::traced_limits(wgpu::Limits {
                    max_blas_primitive_count: a.max_blas_primitive_count,
                    max_blas_geometry_count: 1,
                    max_tlas_instance_count: 4096.min(a.max_tlas_instance_count),
                    max_acceleration_structures_per_shader_stage: 1,
                    max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
                    max_buffer_size: a.max_buffer_size,
                    ..Default::default()
                }),
                // SAFETY: the caller accepted wgpu's experimental ray-query risk (checked above).
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                memory_hints: crate::MEMORY_HINTS,
                ..Default::default()
            }
        }
        BenchPath::NoRt => wgpu::DeviceDescriptor {
            required_features: crate::RECEIVER_FEATURES
                | crate::VISIBILITY_FEATURES
                | crate::VISIBILITY_RASTER_FEATURES
                | crate::VIRTUAL_FEATURES
                | wgpu::Features::TIMESTAMP_QUERY,
            required_limits: crate::visibility_limits(wgpu::Limits::default()),
            memory_hints: crate::MEMORY_HINTS,
            ..Default::default()
        },
    };
    pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| e.to_string())
}

/// A renderer on `device` for `path`, single-sampled; the RT one traces its primaries.
pub(crate) fn renderer(device: &(wgpu::Device, wgpu::Queue), path: BenchPath) -> Result<Renderer, Error> {
    let (device, queue) = device.clone();
    let options = RendererOptions { samples: SampleCount::One, ..Default::default() };
    Ok(match path {
        BenchPath::Rt => {
            let mut r = Renderer::new_with_hardware_shadows(
                device, queue, FORMAT, Limits::default(), options, HardwareShadowOptions::default(), SOURCE,
            )?;
            r.set_primary_visibility(PrimaryVisibility::Traced)?;
            r
        }
        BenchPath::NoRt => Renderer::new_with_options(device, queue, FORMAT, Limits::default(), options, SOURCE)?,
    })
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

/// A drawable part: its mesh (also the lighting cache's software geometry), one identity
/// instance and its material.
pub(crate) struct Part {
    pub mesh: Mesh,
    pub instances: Instances,
    pub material: Material,
}
/// Uploads `faces` as one mesh with a double-sided material of `program`, `albedo` and
/// the program's `parameters`.
pub(crate) fn part(
    r: &mut Renderer,
    faces: &[([Vec3; 4], Vec3)],
    program: &Program,
    albedo: [f32; 3],
    parameters: &[(&str, Value)],
) -> Result<Part, Error> {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (corners, n) in faces {
        let base = vertices.len() as u32;
        vertices.extend(corners.iter().map(|p| Vertex {
            position: p.to_array(),
            normal: n.to_array(),
            uv: [0.0; 2],
            color: [1.0; 4],
        }));
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
    }
    let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
    let mesh = admitted(r, "bench mesh", |r| r.try_upload_mesh(&MeshData { vertices: &vertices, indices: &indices }))?;
    r.set_indirect_geometry(&mesh, &positions, &indices)?;
    let instances = admitted(r, "bench instances", |r| r.try_upload_instances(&[Mat4::IDENTITY]))?;
    let material = admitted(r, "bench material", |r| {
        r.try_create_material(&MaterialDescriptor {
            base_color: [albedo[0], albedo[1], albedo[2], 1.0],
            metallic: 0.0,
            roughness: 1.0,
            double_sided: true,
            program: Some(program),
            parameters,
            ..Default::default()
        })
    })?;
    Ok(Part { mesh, instances, material })
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
