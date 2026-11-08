//! M7 reference path tracer: an unbiased GPU estimate of the physical image for the GI and
//! reflection gates. It handles a sun disk (or delta sun), a prepared or constant sky and any number of
//! bounces, integrated over each pixel's footprint and over a camera shutter.
//!
//! The BSDF is the standard program's model (Lambert plus GGX with Schlick Fresnel), so a
//! difference from Renderlab's image is a transport or estimator difference, never a material
//! one. The estimator is plain unidirectional path tracing: next-event estimation for the sun
//! and sky, BSDF sampling with the power heuristic, and Russian roulette. It reuses nothing
//! across pixels or frames (no reservoirs, caches or denoisers), so every pixel is an
//! independent Monte Carlo mean with its own split-half noise estimate.
//!
//! Measurement tooling: [`render`] owns its resources for one call and never runs in the frame
//! loop. The independent check against Mitsuba 3 is `scripts/m7-u0-crosscheck.py`.
use crate::{Error, Mat4, Vec3};

/// Device features [`render`] requires. Callers also acknowledge wgpu's experimental ray-query risk.
pub const FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Triangle mesh in object space. Normals are per vertex and interpolated for shading.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Empty streams use the standard defaults: UV zero, tangent +X, colour white.
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
}

/// The standard program's parameters. `specular` scales the dielectric F0 of 0.04; 0 with
/// `metallic` 0 gives pure Lambert (used to match Mitsuba's `diffuse`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub base_color: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub specular: f32,
    /// Emitted radiance, cd/m^2.
    pub emission: [f32; 3],
    pub base_color_texture: Option<renderlab_asset::BaseColorTexture>,
    pub metallic_roughness_texture: Option<renderlab_asset::MetallicRoughnessTexture>,
    pub normal_texture: Option<renderlab_asset::NormalTexture>,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: [0.5; 3],
            metallic: 0.0,
            roughness: 0.5,
            specular: 1.0,
            emission: [0.0; 3],
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pub mesh: usize,
    pub material: usize,
    pub transform: Mat4,
}

#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Direction from the scene to the sun.
    pub to_light: Vec3,
    /// Illuminance at normal incidence, lux per channel.
    pub illuminance: [f32; 3],
    /// Angular radius in radians; 0 is a delta light. A disk has uniform radiance and hides
    /// the sky behind it.
    pub angular_radius: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub instances: Vec<Instance>,
    pub sun: Option<Sun>,
    /// Constant sky radiance, cd/m^2.
    pub sky: [f32; 3],
    pub textures: Vec<renderlab_asset::Texture>,
    /// Replaces the constant sky. Uses the unblurred level zero, never a roughness convolution.
    pub environment: Option<Environment>,
    /// Optional camera-only backdrop (secondary rays still see the sky). The matched Unreal
    /// quarry uses a black backdrop with a specified lighting cubemap.
    pub camera_background: Option<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct Environment {
    pub radiance: renderlab_asset::environment::CubeLevel,
    /// Source and runtime yaw combined, in the same convention as EnvironmentParameters.
    pub yaw: f32,
    pub intensity: f32,
}

#[path = "reference_inputs.rs"]
mod inputs;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub eye: Vec3,
    pub target: Vec3,
}

/// Pinhole camera. Each sample draws a time in [0, 1) and interpolates the pose from `open`
/// to `close`; equal poses give a static frame (zero shutter).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub open: Pose,
    pub close: Pose,
    pub up: Vec3,
    pub fov_y: f32,
    pub width: u32,
    pub height: u32,
}

/// Smith masking for the GGX lobe. The standard program uses height-correlated masking;
/// separable masking exists only to match Mitsuba's `roughconductor` in the cross-check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smith {
    HeightCorrelated,
    Separable,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub samples: u32,
    /// Samples per pixel per dispatch; keep each dispatch well under the driver timeout.
    pub samples_per_dispatch: u32,
    /// Maximum path segments, as Mitsuba's `max_depth`: 1 sees emitters only, 2 is direct
    /// lighting. Long paths end by Russian roulette after three bounces.
    pub max_depth: u32,
    pub seed: u32,
    pub smith: Smith,
    /// Explicit mip level for a controlled texture-filter comparison. Zero is the physical
    /// pixel-integrated reference: pixel jitter integrates the unblurred texture itself.
    pub texture_lod: f32,
    /// Capture centre hit identities, sun-only radiance and centre-conditioned visibility.
    pub capture_geometry: bool,
    /// Independent sun visibility rays for each accepted primary sample in geometric
    /// captures. Does not change the radiance estimator; 32 matches T1b's visibility budget.
    pub visibility_samples: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            samples: 1024,
            samples_per_dispatch: 16,
            max_depth: 1024,
            seed: 0x4d37_5530,
            smith: Smith::HeightCorrelated,
            texture_lod: 0.0,
            capture_geometry: false,
            visibility_samples: 32,
        }
    }
}

/// Linear radiance per pixel (row-major from the top left), with a per-pixel standard error
/// from the difference of the even- and odd-sample means.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub mean: Vec<[f32; 3]>,
    pub standard_error: Vec<[f32; 3]>,
    /// Actual independent even/odd means, before taking the absolute split-half difference.
    pub halves: [Vec<[f32; 3]>; 2],
    /// Optional 80-byte records: world vec4, geometric normal vec4, centre IDs uvec4,
    /// direct-sun even/odd sums vec4 (w = accepted-sample visibility sum).
    pub geometry: Vec<u8>,
    /// Pixels where a sample produced a non-finite value. Any nonzero count is a tracer bug.
    pub invalid_pixels: u32,
}

/// Interleaved vertex record: position and normal x, then normal yz and two spare floats.
const VERTEX_FLOATS: usize = 16;
/// The shader's Frame: eight camera vec4s, five lighting/background vec4s and three uvec4s.
const FRAME_BYTES: usize = 16 * 16;

fn validate(scene: &Scene, view: &View, settings: &Settings) -> Result<(), Error> {
    let bad = |m| Err(Error::InvalidInput(m));
    if scene.instances.is_empty() {
        return bad("reference scene has no instances");
    }
    for mesh in &scene.meshes {
        let n = mesh.positions.len();
        if mesh.normals.len() != n
            || (!mesh.uvs.is_empty() && mesh.uvs.len() != n)
            || (!mesh.tangents.is_empty() && mesh.tangents.len() != n)
            || (!mesh.colors.is_empty() && mesh.colors.len() != n)
            || mesh.indices.is_empty()
            || mesh.indices.len() % 3 != 0
            || mesh.indices.iter().any(|&i| i as usize >= n)
        {
            return bad("reference mesh needs matching normals and in-range triangle indices");
        }
    }
    inputs::validate(scene)?;
    for i in &scene.instances {
        if i.mesh >= scene.meshes.len() || i.material >= scene.materials.len() {
            return bad("reference instance names a missing mesh or material");
        }
        if i.transform.determinant().abs() < 1e-12 || !i.transform.is_finite() {
            return bad("reference instance transform must be finite and invertible");
        }
    }
    if scene.materials.iter().any(|m| {
        !(0.0..=1.0).contains(&m.metallic)
            || !(0.0..=1.0).contains(&m.roughness)
            || m.specular < 0.0
            || m.base_color
                .iter()
                .chain(&m.emission)
                .any(|c| !c.is_finite() || *c < 0.0)
    }) {
        return bad("reference material outside its physical range");
    }
    if let Some(sun) = scene.sun
        && (sun.to_light.length_squared() < 1e-12 || !(0.0..1.5).contains(&sun.angular_radius))
    {
        return bad("reference sun needs a direction and an angular radius in [0, 1.5)");
    }
    if view.width == 0 || view.height == 0 || !(1e-4..3.1).contains(&view.fov_y) {
        return bad("reference view needs a size and a field of view in (0, pi)");
    }
    if settings.samples < 2
        || settings.samples_per_dispatch == 0
        || settings.max_depth == 0
        || settings.visibility_samples == 0
    {
        return bad("reference settings need at least two samples, a dispatch size and a depth");
    }
    Ok(())
}

/// Camera basis (eye, tan(fov/2)), forward, right, up, as the shader's `Frame` expects.
fn basis(pose: Pose, up: Vec3, tan_y: f32) -> [[f32; 4]; 4] {
    let forward = (pose.target - pose.eye).normalize();
    let right = forward.cross(up).normalize();
    let up = right.cross(forward);
    [
        pose.eye.extend(tan_y).to_array(),
        forward.extend(0.0).to_array(),
        right.extend(0.0).to_array(),
        up.extend(0.0).to_array(),
    ]
}

fn frame_bytes(scene: &Scene, view: &View, settings: &Settings, first: u32, count: u32) -> Vec<u8> {
    let tan_y = (view.fov_y * 0.5).tan();
    let mut floats: Vec<[f32; 4]> = Vec::with_capacity(13);
    floats.extend(basis(view.open, view.up, tan_y));
    floats.extend(basis(view.close, view.up, tan_y));
    // 1-cos(r) loses precision for small disks. Retain the stable cone height as
    // well as the nominal cosine used by packet inspection.
    let cone_height = scene.sun.map_or(0.0, |s| {
        (2.0 * (0.5 * f64::from(s.angular_radius)).sin().powi(2)) as f32
    });
    let (sun, value) = match scene.sun {
        Some(s) => {
            let d = s.to_light.normalize();
            let cos_r = s.angular_radius.cos();
            let [r, g, b] = s.illuminance;
            // A uniform disk of angular radius r gives illuminance E = L * pi * sin^2(r).
            let scale = if cone_height == 0.0 {
                1.0
            } else {
                (1.0 / (std::f64::consts::PI
                    * f64::from(cone_height)
                    * (2.0 - f64::from(cone_height)))) as f32
            };
            let cos_r = if s.angular_radius == 0.0 { 1.0 } else { cos_r };
            (
                [d.x, d.y, d.z, cos_r],
                [r * scale, g * scale, b * scale, 1.0],
            )
        }
        None => ([0.0, 1.0, 0.0, 1.0], [0.0; 4]),
    };
    floats.push(sun);
    floats.push(value);
    floats.push([
        scene.sky[0],
        scene.sky[1],
        scene.sky[2],
        scene
            .environment
            .as_ref()
            .map_or(0.0, |e| e.radiance.size() as f32),
    ]);
    let (sin, cos, intensity) = scene.environment.as_ref().map_or((0.0, 1.0, 0.0), |e| {
        let (sin, cos) = e.yaw.sin_cos();
        (sin, cos, e.intensity)
    });
    floats.push([
        cos,
        sin,
        intensity,
        f32::from(u8::from(settings.capture_geometry)),
    ]);
    floats.push(
        scene
            .camera_background
            .map_or([0.0, 0.0, 0.0, 0.0], |v| [v[0], v[1], v[2], 1.0]),
    );
    let mut bytes: Vec<u8> = floats
        .iter()
        .flatten()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let smith = u32::from(settings.smith == Smith::Separable);
    for word in [
        view.width,
        view.height,
        first,
        count,
        settings.max_depth,
        settings.seed,
        smith,
        settings.texture_lod.to_bits(),
        settings.visibility_samples,
        cone_height.to_bits(),
        0,
        0,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    debug_assert_eq!(bytes.len(), FRAME_BYTES);
    bytes
}

fn storage(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
    extra: wgpu::BufferUsages,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE | extra,
    })
}

fn row_transform(model: Mat4) -> [f32; 12] {
    let m = model.to_cols_array_2d();
    std::array::from_fn(|i| m[i % 4][i / 4])
}

fn floats(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_le_bytes).collect()
}

/// Trace `settings.samples` paths per pixel and read back the mean image. Blocks until done.
pub fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    view: &View,
    settings: &Settings,
) -> Result<Image, Error> {
    Ok(render_repeats(device, queue, scene, view, settings, 1, false)?.remove(0))
}

/// Keep scene resources fixed when diagnosing a replay. Every repetition clears all
/// measurement state; normal callers request one image through `render`.
fn render_repeats(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &Scene,
    view: &View,
    settings: &Settings,
    repetitions: usize,
    rebuild_pipeline: bool,
) -> Result<Vec<Image>, Error> {
    validate(scene, view, settings)?;
    if !device.features().contains(FEATURES) {
        return Err(Error::InvalidInput(
            "reference tracing needs EXPERIMENTAL_RAY_QUERY",
        ));
    }
    // One shared vertex and index buffer; each BLAS reads its mesh's slice in place.
    let mut vertex_data = Vec::new();
    let mut index_data: Vec<u32> = Vec::new();
    let mut offsets = Vec::with_capacity(scene.meshes.len());
    for mesh in &scene.meshes {
        offsets.push((
            (vertex_data.len() / VERTEX_FLOATS) as u32,
            index_data.len() as u32,
        ));
        for (k, (p, n)) in mesh.positions.iter().zip(&mesh.normals).enumerate() {
            let uv = mesh.uvs.get(k).copied().unwrap_or([0.0; 2]);
            vertex_data.extend([p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]]);
            vertex_data.extend(
                mesh.tangents
                    .get(k)
                    .copied()
                    .unwrap_or([1.0, 0.0, 0.0, 1.0]),
            );
            vertex_data.extend(mesh.colors.get(k).copied().unwrap_or([1.0; 4]));
        }
        index_data.extend(&mesh.indices);
    }
    let input = wgpu::BufferUsages::BLAS_INPUT;
    let vertices = storage(device, "reference vertices", &floats(vertex_data), input);
    let indices = storage(
        device,
        "reference indices",
        &index_data
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
        input,
    );
    let mut instance_data = Vec::new();
    for i in &scene.instances {
        let (vertex, index) = offsets[i.mesh];
        for word in [vertex, index, i.material as u32, 0] {
            instance_data.extend(word.to_le_bytes());
        }
        let normal = i.transform.inverse().transpose();
        for column in [normal.x_axis, normal.y_axis, normal.z_axis] {
            instance_data.extend(floats(column.truncate().extend(0.0).to_array()));
        }
    }
    let instances = storage(
        device,
        "reference instances",
        &instance_data,
        wgpu::BufferUsages::empty(),
    );
    let material_data = floats(
        scene
            .materials
            .iter()
            .flat_map(|m| {
                let [r, g, b] = m.base_color;
                let [er, eg, eb] = m.emission;
                [
                    r,
                    g,
                    b,
                    m.metallic,
                    er,
                    eg,
                    eb,
                    0.0,
                    m.roughness,
                    m.specular,
                    m.normal_texture.map_or(1.0, |t| t.scale),
                    0.0,
                ]
            })
            .collect::<Vec<_>>(),
    );
    let material_data = inputs::material_bytes(scene, material_data);
    let materials = storage(
        device,
        "reference materials",
        &material_data,
        wgpu::BufferUsages::empty(),
    );
    let (texels, texture_info, environment) = inputs::upload(device, scene)?;
    let pixels = u64::from(view.width) * u64::from(view.height);
    let sums_bytes = pixels * 32;
    let sums = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reference sums"),
        size: sums_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let geometry_bytes = if settings.capture_geometry {
        pixels * 80
    } else {
        80
    };
    let geometry = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reference geometric populations"),
        size: geometry_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let frame = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reference frame"),
        size: FRAME_BYTES as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let flags = wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE;
    let sizes: Vec<_> = scene
        .meshes
        .iter()
        .map(|m| wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: m.positions.len() as u32,
            index_format: Some(wgpu::IndexFormat::Uint32),
            index_count: Some(m.indices.len() as u32),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        })
        .collect();
    let blases: Vec<_> = sizes
        .iter()
        .map(|size| {
            device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("reference BLAS"),
                    flags,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            )
        })
        .collect();
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("reference TLAS"),
        max_instances: scene.instances.len() as u32,
        flags,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    for (slot, i) in scene.instances.iter().enumerate() {
        tlas[slot] = Some(wgpu::TlasInstance::new(
            &blases[i.mesh],
            row_transform(i.transform),
            slot as u32,
            0xff,
        ));
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    let entries: Vec<_> = scene
        .meshes
        .iter()
        .enumerate()
        .map(|(k, _)| wgpu::BlasBuildEntry {
            blas: &blases[k],
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &sizes[k],
                vertex_buffer: &vertices,
                first_vertex: offsets[k].0,
                vertex_stride: (VERTEX_FLOATS * 4) as u64,
                index_buffer: Some(&indices),
                first_index: Some(offsets[k].1),
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        })
        .collect();
    encoder.build_acceleration_structures(entries.iter(), [&tlas]);
    queue.submit([encoder.finish()]);

    let storage_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("reference path tracer"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(FRAME_BYTES as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            },
            storage_entry(2, true),
            storage_entry(3, true),
            storage_entry(4, true),
            storage_entry(5, true),
            storage_entry(6, false),
            storage_entry(7, true),
            storage_entry(8, true),
            storage_entry(9, true),
            storage_entry(10, false),
        ],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("reference path tracer"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: tlas.as_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: indices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: materials.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: sums.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: texels.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: texture_info.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: environment.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: geometry.as_entire_binding(),
            },
        ],
    });
    #[cfg(test)]
    let diagnostic_shader = std::env::var("RENDERLAB_M7_SHADER_OVERRIDE")
        .ok()
        .map(|path| std::fs::read_to_string(path).expect("diagnostic shader override"));
    let shader = SHADER;
    #[cfg(test)]
    let shader = diagnostic_shader.as_deref().unwrap_or(shader);
    let make_pipeline = || {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reference path tracer"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reference path tracer"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("reference path tracer"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("trace_samples"),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let pipeline = make_pipeline();
    let wait = || {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(120)),
            })
            .map_err(Error::Poll)
    };
    let mut images = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let rebuilt_pipeline = rebuild_pipeline.then(make_pipeline);
        let pipeline = rebuilt_pipeline.as_ref().unwrap_or(&pipeline);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&sums, 0, None);
        encoder.clear_buffer(&geometry, 0, None);
        queue.submit([encoder.finish()]);
        let mut first = 0;
        while first < settings.samples {
            let count = settings.samples_per_dispatch.min(settings.samples - first);
            queue.write_buffer(&frame, 0, &frame_bytes(scene, view, settings, first, count));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(view.width.div_ceil(8), view.height.div_ceil(8), 1);
            }
            queue.submit([encoder.finish()]);
            wait()?;
            first += count;
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reference readback"),
            size: sums_bytes
                + if settings.capture_geometry {
                    geometry_bytes
                } else {
                    0
                },
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&sums, 0, &readback, 0, sums_bytes);
        if settings.capture_geometry {
            encoder.copy_buffer_to_buffer(&geometry, 0, &readback, sums_bytes, geometry_bytes);
        }
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| drop(sender.send(result)));
        wait()?;
        receiver
            .recv()
            .ok()
            .and_then(Result::ok)
            .ok_or(Error::InvalidInput("reference readback failed"))?;
        let bytes = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("reference readback mapping failed"))?
            .to_vec();
        readback.unmap();
        let mut image = finish(&bytes[..sums_bytes as usize], view, settings.samples);
        if settings.capture_geometry {
            image.geometry = bytes[sums_bytes as usize..].to_vec();
        }
        images.push(image);
    }
    Ok(images)
}

/// Means and split-half standard errors from the even/odd sums (8 floats per pixel).
fn finish(bytes: &[u8], view: &View, samples: u32) -> Image {
    let value = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    let even_n = samples.div_ceil(2) as f32;
    let odd_n = (samples / 2) as f32;
    let pixels = (view.width * view.height) as usize;
    let mut mean = Vec::with_capacity(pixels);
    let mut standard_error = Vec::with_capacity(pixels);
    let mut halves = [Vec::with_capacity(pixels), Vec::with_capacity(pixels)];
    let mut invalid_pixels = 0;
    for p in 0..pixels {
        let even: [f32; 3] = std::array::from_fn(|c| value(p * 8 + c));
        let odd: [f32; 3] = std::array::from_fn(|c| value(p * 8 + 4 + c));
        if even
            .iter()
            .chain(&odd)
            .any(|v| !v.is_finite() || *v < -1e29)
        {
            invalid_pixels += 1;
        }
        mean.push(std::array::from_fn(|c| (even[c] + odd[c]) / samples as f32));
        halves[0].push(even.map(|v| v / even_n));
        halves[1].push(odd.map(|v| v / odd_n));
        standard_error.push(std::array::from_fn(|c| {
            0.5 * (even[c] / even_n - odd[c] / odd_n).abs()
        }));
    }
    Image {
        width: view.width,
        height: view.height,
        samples,
        mean,
        standard_error,
        halves,
        geometry: Vec::new(),
        invalid_pixels,
    }
}

const SHADER: &str = include_str!("../shaders/reference_path_tracer.wgsl");

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reference_quarry_tests.rs"]
mod quarry_tests;
