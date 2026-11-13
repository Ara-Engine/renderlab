//! Renderlab's forward lit frame, whole, on ara-gpu: clustered light lists, then the
//! lit mesh pass with the standard program into the exposed HDR target.
//!
//! This is the first frame assembled from the ported passes rather than one pass
//! replayed alone. It covers what `Renderer::try_render_view(.., ViewMode::Lit)`
//! encodes on an untraced frame with the atmosphere, sun and local shadows, GI,
//! decals and transmission off: every such input is bound with the same "off"
//! contents the wgpu renderer binds. With an active atmosphere it adds
//! `encode_atmosphere`: the four LUTs, the procedural environment cube, the aerial
//! volume, sun transmittance on the directional records and the sky backdrop under
//! the mesh pass (volumetric fog is not assembled). With sun shadows it is the frame
//! `Renderer::new_with_hardware_shadows` draws for a sun with a disk: the scene's
//! acceleration structures, a receiver prepass, 256-ray fresh visibility and the lit
//! pass composed on the fresh prelude. With post settings it continues through
//! `PostGpu::encode` and the tone stage: motion surface, tile maximum, motion blur,
//! bloom, tone and the display transfer. On a traced frame with [`ForwardScene::indirect`]
//! the world radiance cache's sky-visibility passes run between the estimator and
//! shading, as `GiState::encode` places them, and shading reads the cache's table.
//! [`render`] is one-shot: its frame is always the first after a cut. [`Frames`] keeps
//! what a sequence carries from frame to frame.
use super::shader::memo::Memo;
use super::{
    acceleration::Scene,
    atmosphere::{AtmosphereParameters, AtmospherePasses, AtmosphereSun, AtmosphereTargets},
    bindings::{HeapBindings, Input},
    bloom::Bloom,
    buffer::StorageBuffer,
    cache::ShaderCache,
    environment::EnvironmentPasses,
    fresh_visibility::{self, Mode, Visibility},
    gi_cache::{Cache, CacheScene, Indirect},
    graph::Graph,
    image::Texture,
    kernel::Kernel,
    light_cluster::{ClusterInputs, LightClusters},
    mesh::MeshGeometry,
    post::{FilterInputs, PostPasses, SettingsRecord, ToneInputs},
    post_surface::{PostCameraFrame, RasterDepth, RasterPostSurface},
    primary_shade, primary_trace,
    program_bins::{BinDispatch, ProgramBins},
    receiver_raster::{ReceiverDraw, ReceiverTargets, Receivers},
    shader::{self, InputKey, InputKind, Pass, Program, SampleType, ShaderInput},
    shadow_raster::ShadowDraw,
    sky::SkyPass,
    sky_map::{ResidentMap, SkyMap},
    temporal::{History, Reconstruct},
    toon::{MeshAttachments, ToonDraw, ToonPass},
    transfer,
    transmission_shadow::Transmission,
    transparency_glass::{self, Glass, PaneIds},
    transparency_seed::{MsaaColor, Seed},
    volumes::VolumePasses,
};
use crate::{
    light_cluster as lc, lighting::PackedLighting,
    renderer::primary_ray_gpu::temporal_reconstruction as tr, renderer::shadow_gpu,
    renderer::transparency_gpu as tp,
};
use anyhow::{Context, Result, bail, ensure};
use ara_gpu::{
    AccelerationStructure, Access, Device, GpuDevice, Queue, Resource,
    acceleration::{AccelerationDesc, AccelerationInstance},
    commands::{Command, Recorder},
    descriptors::Format,
    sampler::{Filter, SamplerDesc},
    texture::{SampleCount, ViewDimension},
};
use std::collections::{BTreeMap, BTreeSet};

/// One opaque object: its own mesh, one instance and untextured material factors.
pub struct ForwardObject<'a> {
    /// Renderlab's 48-byte vertices.
    pub vertices: &'a [crate::Vertex],
    /// Triangle list.
    pub indices: &'a [u32],
    /// Object-to-world transform.
    pub transform: crate::Mat4,
    /// Factors and sidedness; texture slots must be empty (the 1x1 defaults bind).
    pub material: crate::MaterialDescriptor<'a>,
}

/// A block-compressed texture as the frames upload it: what the wgpu renderer makes of a
/// `CompressedTexture` in `try_upload_compressed_texture`.
#[derive(Clone, Debug)]
pub struct BlockTexture {
    pub format: crate::asset::BlockFormat,
    /// The base level's width and height: multiples of 4.
    pub size: [u32; 2],
    /// Array layers, each with every level's blocks down to 1x1. One layer; a BC5 normal
    /// map has two, the map and its variance layer (`normal_variance_bc`), which the
    /// shading programs read as layer 1.
    pub layers: Vec<Vec<Vec<u8>>>,
}
impl BlockTexture {
    /// The upload for `data`, which the caller has validated
    /// (`CompressedTexture::validate`). A BC5 map gains its variance layer here, once.
    pub fn new(data: crate::asset::CompressedTexture) -> Self {
        let variance = (data.format == crate::asset::BlockFormat::Bc5)
            .then(|| crate::normal_variance_bc::variance_layer(&data));
        Self {
            format: data.format,
            size: [data.width, data.height],
            layers: std::iter::once(data.mips).chain(variance).collect(),
        }
    }
}
/// A texture slot's levels: RGBA8 texels, or block-compressed levels as shipped.
#[derive(Clone, Copy)]
pub enum TextureSource<'a> {
    /// The asset the wgpu renderer would take through `try_upload_texture`.
    Texels(&'a crate::asset::Texture),
    /// Needs a device that samples BC (`DeviceInfo::texture_compression_bc`).
    Blocks(&'a BlockTexture),
}
impl<'a> From<&'a crate::asset::Texture> for TextureSource<'a> {
    fn from(texels: &'a crate::asset::Texture) -> Self {
        Self::Texels(texels)
    }
}
impl<'a> From<&'a BlockTexture> for TextureSource<'a> {
    fn from(blocks: &'a BlockTexture) -> Self {
        Self::Blocks(blocks)
    }
}
/// One texture slot: every mip level of the texture (`TextureSource`) and its
/// `TextureBinding` sampler.
#[derive(Clone, Copy)]
pub struct ObjectTexture<'a> {
    pub data: TextureSource<'a>,
    pub sampler: crate::asset::Sampler,
    /// Its identity to the frames, which keep it on the GPU for as long as the frames of
    /// a sequence keep naming it (`Frames`); an identity must always name the same
    /// texture. `None` uploads it for this frame alone.
    pub key: Option<u64>,
}
/// A skinned object's pose before this frame's, for reconstruction's motion (SK1's
/// previous vertex stream), and its pose revision.
#[derive(Clone, Copy)]
pub struct Skin<'a> {
    /// The object's vertices in the previous pose, one per vertex of its mesh.
    pub previous: &'a [crate::Vertex],
    /// Changes exactly when the pose does (`SkinnedMesh::revision`).
    pub revision: u64,
}
/// A tangent-space normal map with the tangent frames it is authored against: what the
/// wgpu renderer takes as a material's `normal_texture` and the mesh's tangent stream
/// (`try_upload_mesh_with_tangents`), which it refuses to draw one without the other.
#[derive(Clone, Copy)]
pub struct NormalMap<'a> {
    /// `TextureRole::NormalLinear`, or a BC5 block texture, whose Z the shading programs
    /// rebuild and whose variance layer they read. The variance and slope-moment texel
    /// roles are not assembled.
    pub texture: ObjectTexture<'a>,
    /// One frame per vertex of the object's mesh.
    pub tangents: &'a crate::asset::TangentStream,
}

/// One object's material textures; an empty slot samples the renderer's 1x1 default.
#[derive(Clone, Copy, Default)]
pub struct ObjectTextures<'a> {
    /// `TextureRole::BaseColorSrgb`.
    pub base_color: Option<ObjectTexture<'a>>,
    /// `TextureRole::MetallicRoughnessLinear`.
    pub metallic_roughness: Option<ObjectTexture<'a>>,
    /// Traced frames only: the mesh pass's tangent vertex stream is not assembled. The
    /// material's `normal_scale` applies.
    pub normal: Option<NormalMap<'a>>,
}
/// A material's shading program as the engine names one: its WGSL (`surface`, `shade`
/// and the hooks it declares, as `shaders/programs/standard.wgsl`) and the named values
/// of its `Parameters` block; members not named keep the program's defaults.
#[derive(Clone, Copy)]
pub struct ObjectProgram<'a> {
    pub source: &'a str,
    pub parameters: &'a [(&'a str, crate::Value)],
}
/// The scene's shading programs, on traced frames.
#[derive(Clone, Copy)]
pub struct ScenePrograms<'a> {
    /// One entry per object, in `ForwardScene::objects` order; `None` is the standard
    /// program. Objects naming the same source share a program. A frame of one program
    /// shades the whole region in one dispatch; a frame of several bins its 4x4 tiles
    /// by program and each program shades its own list (MP1), up to 32 programs.
    pub objects: &'a [Option<ObjectProgram<'a>>],
    /// One entry per pane, in `ForwardScene::transparent` order, or empty for the
    /// standard program on every pane. A pane's program is composed into the glass
    /// pass as the renderer composes it; one that declares `Parameters` is refused,
    /// since a pane's parameter block is not assembled.
    pub transparent: &'a [Option<ObjectProgram<'a>>],
}

/// The scene's material textures.
#[derive(Clone, Copy)]
pub struct SceneTextures<'a> {
    /// One entry per object, in `ForwardScene::objects` order.
    pub objects: &'a [ObjectTextures<'a>],
    /// `RendererOptions::anisotropy_clamp`, 1 to 16: applied to fully linear samplers.
    pub anisotropy_clamp: u8,
}

/// Where one frame's wall time went, as the CPU saw it.
#[derive(Clone, Debug, Default)]
pub struct FrameTimes {
    /// Building the frame before it is submitted: allocation, uploads, shader
    /// composition, pipelines and recording. Includes `geometry`.
    pub record: std::time::Duration,
    /// Of `record`: the scene's mesh buffers and acceleration structures.
    pub geometry: std::time::Duration,
    /// From submission until the GPU has finished the frame. On a sequence
    /// `without_images`: the device span of the newest frame the GPU has finished since
    /// the last report (this frame when it reads anything back, else an earlier one, and
    /// zero when none finished).
    pub gpu: std::time::Duration,
    /// Reading the images back and releasing the frame's resources.
    pub finish: std::time::Duration,
    /// On a sequence `without_images`: the GPU's own time in each labelled pass of the
    /// frame `gpu` describes, in milliseconds from the device's timestamps, summed by
    /// label in first-run order.
    /// What `gpu` holds beyond their sum is unlabelled: acceleration-structure builds,
    /// uploads, copies and fills, and the gaps between passes.
    pub passes: Vec<(String, f64)>,
}

/// A scene for [`render`], in the terms the wgpu renderer takes it.
pub struct ForwardScene<'a> {
    /// Draw order; equal depths resolve to the earlier object, as in the mesh pass.
    pub objects: &'a [ForwardObject<'a>],
    /// Lights, uniform environment radiance and exposure.
    pub lighting: &'a crate::Lighting,
    /// View and projection; the aspect follows `size`.
    pub camera: crate::Camera,
    /// Target extent in pixels.
    pub size: [u32; 2],
    /// `Atmosphere::off()` leaves the uniform environment. Volumetric fog must be off.
    pub atmosphere: crate::Atmosphere,
    /// Ray-traced soft shadows for the first directional light; needs a ray-query device.
    pub sun_shadows: Option<&'a SunShadows>,
    /// How primary visibility is resolved. `Traced` needs `sun_shadows` and no post yet.
    pub primary: Primary,
    /// `None` stops at the exposed HDR target. Depth of field is not assembled yet.
    pub post: Option<crate::PostSettings>,
    /// The world radiance cache, on `Primary::Traced` frames only. `None` or disabled
    /// settings publish it as off: programs read the unoccluded environment.
    pub indirect: Option<Indirect>,
    /// Temporal reconstruction, on `Primary::Traced` frames only: the frame is rendered
    /// through a jittered camera and resolved against its history.
    pub reconstruction: Option<Reconstruct>,
    /// Traced reflections, on traced frames only: the shading program's own reflection
    /// rays (one mirror ray, or strata over its declared lobe) up to the settings'
    /// roughness. `None` is `ReflectionSettings::off()`: the untraced estimate.
    pub reflections: Option<crate::ReflectionSettings>,
    /// Base-colour and metallic-roughness textures per object; `None` binds the
    /// defaults everywhere. The texture slots of `ForwardObject::material` itself must
    /// stay empty: they name the wgpu renderer's own uploads.
    pub textures: Option<SceneTextures<'a>>,
    /// Shadows for point and spot lights, on traced frames only, where every local
    /// light is shadowed by rays from the shading program (one ray, or four over a
    /// light with a source radius). `None` is a count of zero: local lights unshadowed.
    /// The map path of untraced frames is not assembled.
    pub local_shadows: Option<crate::LocalShadows>,
    /// Transparent panes, on traced frames only: each object's material carries a
    /// `Transparency`. Drawn after opaque shading and the resolve, far to near, each
    /// over the colour behind it on four coverage samples, with its sun visibility a
    /// ray its own fragments trace (`Renderer::encode_transparency` on a ray-traced
    /// renderer). Each pane also casts its coloured sun shadow on everything opaque
    /// (`prepare_transmission`). Short of the renderer in two ways: no traced
    /// reflection in a pane, and no particles. A frame below native scale skips the
    /// panes and keeps their shadows, as the renderer does.
    pub transparent: &'a [ForwardObject<'a>],
    /// The objects' shading programs and their parameters, on traced frames only: the
    /// mesh pass composes the standard program alone. `None` is the standard program
    /// for every object. The program shades camera hits, reflection hits and, with
    /// bounce, the radiance cache's cells, as the renderer composes it at each stage.
    pub programs: Option<ScenePrograms<'a>>,
    /// One identity per object, on traced frames: objects with equal identities are
    /// one mesh, whose buffers and bottom-level structure are built once and kept on
    /// the GPU for as long as the frames of a sequence keep naming it (`Frames`). An
    /// identity must always name the same geometry. `None` uploads and builds every
    /// object's geometry each frame.
    pub meshes: Option<&'a [u64]>,
    /// One identity per object for its material, on frames with `meshes` and the
    /// radiance cache: an object's mesh and material identities together are its
    /// draw's lasting name in the cache, as a renderer draw's vertex buffer and
    /// material are. A cell remembers the draw it lies on by that name, so it keeps
    /// reading its own draw's material and program when draws enter, leave or change
    /// place. An identity names one material for the whole sequence, whatever its
    /// values become. Without it (or without `meshes`) the cache numbers the draws
    /// by their place in the frame, which holds only while the draw order does.
    pub materials: Option<&'a [u64]>,
    /// One list of transforms per object, on traced frames: the object is one draw of
    /// that many instances, placed at each in turn, in place of its own `transform` (a
    /// renderer `Draw` over its `Instances`). A frame holds `TRACED_DRAWS` draws and
    /// any number of placements. `None` draws each object once at its `transform`.
    pub instances: Option<&'a [&'a [crate::Mat4]]>,
    /// Per object, whether each of its placements (`instances`' order) blocks light
    /// (`set_casts_shadows`): a non-caster is left out of shadow rays (its acceleration
    /// instance lacks `SHADOW_CASTER_MASK`) and of the shadow plan. `None`: every placement
    /// casts.
    pub casts: Option<&'a [&'a [bool]]>,
    /// A prepared environment map that lights the scene in place of the uniform
    /// environment (`Sky::Map`), kept on the GPU while frames go on naming its key.
    /// The atmosphere's procedural environment must be off.
    pub sky_map: Option<SkyMap<'a>>,
    /// One entry per object on traced frames: a skinned object's previous pose, `None`
    /// for a rigid one. A skinned object's geometry is uploaded and built each frame
    /// (its `meshes` identity still names its draw), its batch word says it is skinned,
    /// and the reconstruction and the cache see its pose change. `None` is all rigid.
    pub skins: Option<&'a [Option<Skin<'a>>]>,
    /// The display-linear 3D colour LUT the post pass applies at `post.lut_strength`
    /// (`Renderer::set_colour_lut`); `None` is the identity, at strength zero.
    pub colour_lut: Option<&'a crate::post::ColourLut>,
    /// Object ids, on traced frames: each output pixel's settled opaque id and its front
    /// id are resolved into the sequence's id texture (`Frames::ids`). `None` writes none.
    pub ids: Option<IdScene<'a>>,
}

/// A frame's object-id tables (`object_ids::TagTables`), in the frame's placement order:
/// every opaque object's placements, then every transparent object's.
#[derive(Clone, Copy)]
pub struct IdScene<'a> {
    /// Per placement: (first range, range count).
    pub entries: &'a [[u32; 2]],
    /// Sorted, disjoint (first primitive, count, id, 0) per entry.
    pub ranges: &'a [[u32; 4]],
}

/// The id targets a sequence keeps: the ids of its last frame with ids (R32Uint, two
/// layers: opaque, front, at the output's size), the settled (placement + 1, primitive,
/// pending primitive, its streak; Rgba32Uint) of the last two frames, which ping-pong, and the camera the last ids were taken from.
struct IdTargets {
    size: [u32; 2],
    ids: Texture,
    settled: [Texture; 2],
    parity: usize,
    camera: Option<[f32; 16]>,
    /// The packed RGBA8 image `Frames::pack_ids` writes, at the same size.
    packed: Texture,
    /// Whether `pack_ids` wrote it (it is then in `CopySource`).
    packed_written: bool,
}

/// Which of the renderer's frame families draws the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// The mesh pass: what an untraced `ViewMode::Lit` frame encodes.
    Raster,
    /// Camera rays and deferred shading, as `HardwareRt` at `Quality::Reference` with
    /// GI, temporal reconstruction and reflections off: the camera trace with
    /// receivers, the gated fresh estimator, then shading on the hardware soft head.
    Traced,
    /// The same frame with `SoftShadows::Interactive`, as `HardwareRt` draws it at its
    /// realtime quality: the fused trace (camera hit and eight sun directions per pixel
    /// in one dispatch, near-edge pixels listed) and its indirectly dispatched repair
    /// replace the trace and the 256-ray estimator. With reconstruction the directions
    /// rotate with the sample and the ray count is the planner's budget.
    TracedInteractive,
}

/// The sun's shadow request, as `Shadows` carries it for a directional light.
#[derive(Clone, Debug)]
pub struct SunShadows {
    /// Positive camera-forward distances; the last one is the coverage limit.
    pub split_ends: Vec<f32>,
    /// The sun disk's half-angle in radians, in (0, 0.1].
    pub disk_angle: f32,
}

/// Readbacks of one frame, each tightly packed in row order.
pub struct ForwardImages {
    /// Where this frame's wall time went.
    pub times: FrameTimes,
    /// The exposed linear HDR image post reads, as RGBA16F at the scene's size. At
    /// native scale it is `Renderer::try_capture_hdr`'s bytes; below native scale it is
    /// the resolve's upscaled output, which the wgpu renderer does not expose.
    pub hdr: Vec<u8>,
    /// Below native scale: the frame's shaded samples before the resolve, RGBA16F at
    /// the internal size. This is what `Renderer::try_capture_hdr` returns there.
    pub samples: Option<Vec<u8>>,
    /// Output-space shutter velocity and linear depth as RGBA32F, when motion blur ran.
    pub surface: Option<Vec<u8>>,
    /// The displayed image as RGBA8 with the display transfer applied, when post ran.
    pub display: Option<Vec<u8>>,
    /// The frame as depth of field left it, RGBA16F, when it ran: what bloom and the
    /// tone stage then read.
    pub focused: Option<Vec<u8>>,
    /// The cache's whole table after this frame, when `Indirect::read_table` asked.
    pub indirect_table: Option<Vec<u32>>,
    /// With bounce and `read_table`: every slot's outgoing radiance, w 1 once written.
    pub indirect_outgoing: Option<Vec<[f32; 4]>>,
    /// With the cell filter and `read_table`: its sources, 22 words a slot (hot words 1
    /// to 22 as written before filtering).
    pub indirect_raw: Option<Vec<u32>>,
    /// With `Reconstruct::read_history`: the history colour this frame wrote, RGBA32F
    /// (exposed mean, accumulated weight), and whether the frame restarted every pixel.
    pub reconstruction_history: Option<(Vec<f32>, bool)>,
    /// MEASUREMENT ONLY: with `RENDERLAB_ARA_PROBE_RESOLVE` set, the resolve's
    /// one-pixel trace (`resolve_probe::describe` reads it).
    pub resolve_probe: Option<Vec<[f32; 4]>>,
    /// With `RENDERLAB_RL_DEFERRED=1`, until the composite reads them on the GPU: the
    /// deferred reflections' outputs as RGBA16F (`program::deferred_variant`: reflectance
    /// and traced fraction; untraced specular and the inline traced luminance).
    pub deferred_reflections: Option<[Vec<u8>; 2]>,
}

/// The post `Settings` record at this output height for the post frame with this
/// serial (one for the first), with the identity LUT the renderer starts with.
pub fn post_record(settings: &crate::PostSettings, height: u32, serial: u32) -> [f32; 20] {
    settings.record(serial, height, LUT_SIDE, false)
}
/// The identity LUT's side, as the renderer starts with it.
const LUT_SIDE: u32 = 2;
/// `atmosphere_gpu::FROXEL_TILE`: output pixels per camera-volume cell.
const FROXEL_TILE: u32 = 8;

/// A mesh on the GPU: its vertex and index buffers, which are also its build inputs,
/// and its bottom-level structure.
struct ResidentMesh {
    vertices: StorageBuffer,
    indices: StorageBuffer,
    blas: AccelerationStructure,
    /// Vertex and index counts, to hold an identity to one geometry.
    counts: [usize; 2],
}
/// A per-placement table's bytes, or zeros the size of its largest record (256 bytes)
/// when the frame places nothing: a storage buffer cannot be empty and its reflected
/// binding holds one element, and these tables are read only through a hit, which an
/// empty frame (`render_with`'s masked stand-in instance) never has.
fn per_placement(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if bytes.is_empty() {
        std::borrow::Cow::Owned(vec![0; 256])
    } else {
        std::borrow::Cow::Borrowed(bytes)
    }
}

impl ResidentMesh {
    /// Uploads a mesh's vertices (48-byte records) and indices and records its BLAS
    /// build into `rec`; the buffers enter `graph` as host-written.
    fn build(
        gpu: &mut GpuDevice,
        graph: &mut Graph,
        rec: &mut Recorder,
        vertices: &[crate::Vertex],
        indices: &[u32],
    ) -> Result<Self> {
        let usage = ara_gpu::vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
            | ara_gpu::vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
        let vertex_bytes: Vec<u8> = vertices
            .iter()
            .flat_map(|v| {
                v.position
                    .into_iter()
                    .chain(v.normal)
                    .chain(v.uv)
                    .chain(v.color)
            })
            .flat_map(f32::to_le_bytes)
            .collect();
        let vertex_buffer = StorageBuffer::with_usage(gpu, &vertex_bytes, usage)?;
        let index_buffer = StorageBuffer::with_usage(gpu, bytemuck::cast_slice(indices), usage)?;
        let blas = gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
            vertices: vertex_buffer.buffer,
            vertex_count: vertices.len() as u32,
            vertex_stride: 48,
            indices: index_buffer.buffer,
            index_count: indices.len() as u32,
        })?;
        for buffer in [&vertex_buffer, &index_buffer] {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
        }
        graph.node(
            rec,
            &[
                (Resource::Buffer(vertex_buffer.buffer), Access::AsInput),
                (Resource::Buffer(index_buffer.buffer), Access::AsInput),
                (Resource::AccelerationStructure(blas), Access::AsBuild),
            ],
        )?;
        rec.push(Command::BeginPass("ara::BuildBlas".into()));
        rec.push(Command::BuildAccelerationStructure(blas));
        rec.push(Command::EndPass);
        Ok(Self {
            vertices: vertex_buffer,
            indices: index_buffer,
            blas,
            counts: [vertices.len(), indices.len()],
        })
    }
    fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        gpu.retire(Resource::AccelerationStructure(self.blas))?;
        self.vertices.retire(gpu)?;
        self.indices.retire(gpu)
    }
}
#[derive(Clone, Copy)]
enum Geometry {
    Resident(u64),
    Transient(usize),
}
impl Geometry {
    fn of<'a>(
        self,
        resident: &'a BTreeMap<u64, ResidentMesh>,
        transient: &'a [ResidentMesh],
    ) -> &'a ResidentMesh {
        match self {
            Self::Resident(key) => &resident[&key],
            Self::Transient(index) => &transient[index],
        }
    }
}

/// What sun shadows add: the scene to query, receivers to query from and the counts.
struct Traced {
    /// Where each object's geometry is: resident in the sequence under its identity,
    /// or this frame's own at an index of `transient`.
    geometry: Vec<Geometry>,
    /// Geometry built for this frame alone.
    transient: Vec<ResidentMesh>,
    /// Each object's tangent stream, where it has a normal map.
    tangents: Vec<Option<StorageBuffer>>,
    scene: Scene,
    world: Texture,
    normal: Texture,
    /// Visible rays of 256 per pixel, `R32Uint`.
    visibility: Texture,
    /// The shadow view records the estimator and the lit pass read.
    records: StorageBuffer,
    receivers: Receivers,
    kernel: Visibility,
    program: Program,
}

/// What a traced frame adds to the scene and receivers: `Renderer::prepare_primary`'s
/// records and targets, and the three kernels.
struct Deferred {
    trace: Kernel,
    trace_program: Program,
    fresh: Kernel,
    fresh_program: Program,
    shade: Kernel,
    /// Hit records, one tile flag per trace group, and receiver precision.
    hit: Texture,
    tiles: Texture,
    /// `scene_pixels`: the rectangle camera rays can reach a placement in.
    scene: [u32; 4],
    /// That rectangle widened to whole trace tiles, as the region record holds it.
    region_rect: [u32; 4],
    /// RENDERLAB_MATERIAL_PASS: the material record (program.rs MATERIAL_RECORD_BINDINGS), a
    /// pair of rgba32uint images of the frame's size; None while the pass is off.
    material: Option<[Texture; 2]>,
    epsilon: Texture,
    frame: StorageBuffer,
    region: StorageBuffer,
    placements: StorageBuffer,
    /// Current then previous model matrices, and the normal transforms, per placement.
    models: StorageBuffer,
    normals: StorageBuffer,
    /// Pads the per-draw arrays, as the renderer's 48-byte absent buffer does.
    absent: StorageBuffer,
    /// Interactive frames: the near-edge repair queue, whose first words are the
    /// repair's dispatch arguments.
    repairs: Option<StorageBuffer>,
    /// Dispatch groups of the trace, the estimator and the shading.
    groups: [[u32; 2]; 3],
}

/// A frame of several programs (`ProgramLists`): the binning kernel, each slot's list
/// program with its kernel and `RlList` record, and the lists and dispatch arguments
/// the binning pass fills.
struct ProgramLists {
    bins: ProgramBins,
    slots: Vec<(Program, Kernel, StorageBuffer)>,
    entries: StorageBuffer,
    arguments: StorageBuffer,
    dispatch: BinDispatch,
}

/// Consecutive films (`Pane::film`) from which a run blends in one pass instead of pane by
/// pane (`encode` below; measured on the M5, Ara's Outpost and Glint).
const FILM_RUN: usize = 8;
/// One transparent pane's own resources.
struct Pane {
    /// The colour behind it: the HDR target copied before it draws.
    behind: Texture,
    geometry: MeshGeometry,
    material: StorageBuffer,
    /// `TpDraw`: the thin slab's thickness.
    slot: StorageBuffer,
    /// Its program's index in `Panes::programs`.
    program: usize,
    /// The pixels its bounds project to, [x0, y0, x1, y1): what per-pixel shading visits.
    rect: [u32; 4],
    /// A film (index of refraction one, no thickness): blended in hardware when its program
    /// declares films (`Panes::films`), in one seeded pass with the films next to it.
    film: bool,
}
/// What a traced frame's transparent panes add: `encode_transparency`'s targets,
/// records and pipelines.
struct Panes {
    hit_depth: Kernel,
    seed: Seed,
    /// Each distinct pane program composed for the glass pass, with its pipeline.
    programs: Vec<(Program, Glass)>,
    /// The same programs' film pipelines (`transparency_glass::film_program`), for programs
    /// that declare films.
    films: Vec<Option<(Program, Glass)>>,
    /// Per-pixel pane shading (`PanePixels`), in place of the glass raster unless
    /// `RENDERLAB_ARA_PANES=raster`.
    pixels: Option<PanePixels>,
    target: MsaaColor,
    /// The pass's camera block and `TpFrame`, from the unjittered camera.
    camera: StorageBuffer,
    frame: StorageBuffer,
    /// Opaque view depth from the hit records, `R32Float`.
    depth: Texture,
    /// The traced reflection no pass writes yet (the frame block says so), and the
    /// material textures' 1x1 defaults.
    reflected: Texture,
    defaults: [Texture; 3],
    /// Far to near.
    panes: Vec<Pane>,
}
/// Per-pixel pane shading's programs and target (`shaders/pane_pixels.wgsl`): for each pane
/// program its id raster and its shading kernel, and the id raster's target.
struct PanePixels {
    programs: Vec<(Program, PaneIds, Program, Kernel)>,
    /// Four-sample R32Uint at the frame's size: the triangle (plus one) under each sample.
    ids: Texture,
}
/// The pixels a pane's world bounds project to through `camera` at `size`, two pixels wider
/// on each side; the whole frame when a corner lies behind the eye.
fn pane_rect(
    bounds: (crate::Vec3, crate::Vec3),
    camera: &crate::Camera,
    size: [u32; 2],
) -> [u32; 4] {
    let m = camera.matrix(size[0], size[1]);
    let (lo, hi) = bounds;
    let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
    for corner in 0..8 {
        let p = crate::Vec3::new(
            if corner & 1 == 0 { lo.x } else { hi.x },
            if corner & 2 == 0 { lo.y } else { hi.y },
            if corner & 4 == 0 { lo.z } else { hi.z },
        );
        let clip = m * p.extend(1.);
        if clip.w <= 1e-6 {
            return [0, 0, size[0], size[1]];
        }
        let px = [
            (clip.x / clip.w * 0.5 + 0.5) * size[0] as f32,
            (0.5 - clip.y / clip.w * 0.5) * size[1] as f32,
        ];
        for k in 0..2 {
            a[k] = a[k].min(px[k]);
            b[k] = b[k].max(px[k]);
        }
    }
    let clamp = |v: f32, n: u32| v.clamp(0., n as f32) as u32;
    [
        clamp(a[0].floor() - 2., size[0]),
        clamp(a[1].floor() - 2., size[1]),
        clamp(b[0].ceil() + 2., size[0]),
        clamp(b[1].ceil() + 2., size[1]),
    ]
}
impl Panes {
    fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
        if let Some(pixels) = self.pixels {
            super::image::release(gpu, pixels.ids.image)?;
        }
        self.target.retire(gpu, graph)?;
        self.camera.retire(gpu)?;
        self.frame.retire(gpu)?;
        for pane in self.panes {
            pane.geometry.retire(gpu, graph)?;
            pane.material.retire(gpu)?;
            pane.slot.retire(gpu)?;
            super::image::release(gpu, pane.behind.image)?;
        }
        for image in [self.depth, self.reflected]
            .into_iter()
            .chain(self.defaults)
        {
            super::image::release(gpu, image.image)?;
        }
        Ok(())
    }
}

/// The per-draw arrays of a traced frame, padded to `TRACED_DRAWS`.
#[derive(Default)]
struct Arrays<'a> {
    vertices: Vec<&'a StorageBuffer>,
    indices: Vec<&'a StorageBuffer>,
    /// Each draw's tangent stream, the absent stand-in where it has none.
    tangents: Vec<&'a StorageBuffer>,
    absent: Vec<&'a StorageBuffer>,
    materials: Vec<&'a StorageBuffer>,
    /// Each draw's `Parameters` block; empty when the program declares none.
    parameters: Vec<&'a StorageBuffer>,
    /// Per sampler-array key; a key without an entry takes `default_samplers`.
    samplers: BTreeMap<InputKey, Vec<SamplerDesc>>,
    default_samplers: Vec<SamplerDesc>,
    textures: BTreeMap<InputKey, Vec<&'a Texture>>,
    /// Each draw's previous vertices: a skinned draw's own stream, a rigid draw's
    /// current one (SK1's `motion_previous_vertices`).
    previous: Vec<&'a StorageBuffer>,
}

/// Everything a packet of this frame can name, borrowed for the packets' lifetime.
struct Shared<'a> {
    output: &'a Texture,
    deferred: Option<&'a Deferred>,
    arrays: &'a Arrays<'a>,
    camera: &'a StorageBuffer,
    lighting: &'a StorageBuffer,
    clusters: &'a ClusterInputs,
    lists: &'a Texture,
    atmosphere: &'a AtmosphereParameters,
    sky: Option<&'a Sky<'a>>,
    map: Option<&'a ResidentMap>,
    traced: Option<&'a Traced>,
    /// The radiance cache's table where the cache runs; otherwise a zero header binds.
    gi_table: Option<&'a StorageBuffer>,
    /// The cache's outgoing radiance, for cache-terminated reflection hits; otherwise zeros.
    gi_outgoing: Option<&'a StorageBuffer>,
    buffers: &'a BTreeMap<InputKey, StorageBuffer>,
    images: &'a BTreeMap<InputKey, Texture>,
}
impl<'a> Shared<'a> {
    /// One reflected input of any of the frame's programs, for one object or none.
    fn input(
        &self,
        spec: &ShaderInput,
        object: Option<(&'a MeshGeometry, &'a StorageBuffer)>,
    ) -> Result<Input<'a>> {
        let per_object = || object.with_context(|| format!("{} needs an object", spec.name));
        let deferred = || self.deferred.context("not a traced frame");
        Ok(match (spec.kind, spec.key.group, spec.key.binding) {
            (InputKind::AccelerationStructure, ..) => {
                Input::Scene(&self.traced.context("no scene in this frame")?.scene)
            }
            (InputKind::Buffer { .. }, group, binding) => Input::Buffer(match (group, binding) {
                (0, 0) => self.camera,
                (0, 10) => &deferred()?.frame,
                (0, 22) => &deferred()?.region,
                (0, 24) | (0, 26) => deferred()?
                    .repairs
                    .as_ref()
                    .context("no repair queue in this frame")?,
                (1, 0) => match object {
                    Some((geometry, _)) => geometry.models(),
                    None => &deferred()?.models,
                },
                (1, 1) => match object {
                    Some((geometry, _)) => geometry.normals(),
                    None => &deferred()?.normals,
                },
                (1, 5) => &deferred()?.placements,
                (2, 0) => per_object()?.1,
                (3, 0) => self.lighting,
                (3, 1) => &self.clusters.lights,
                (3, 23) => &self.clusters.frame,
                (3, 50) => &self.atmosphere.record,
                (3, 5) if self.traced.is_some() => &self.traced.context("checked")?.records,
                (3, 20) if self.gi_table.is_some() => self.gi_table.context("checked")?,
                (3, 28) if self.gi_outgoing.is_some() => self.gi_outgoing.context("checked")?,
                _ => &self.buffers[&spec.key],
            }),
            (InputKind::BufferArray { .. }, group, binding) => {
                Input::Buffers(match (group, binding) {
                    (1, 2) => &self.arrays.vertices,
                    (1, 3) => &self.arrays.indices,
                    (1, 4) => &self.arrays.tangents,
                    (1, 6) if !self.arrays.previous.is_empty() => &self.arrays.previous,
                    (2, 10) => &self.arrays.materials,
                    (2, 17) => &self.arrays.parameters,
                    _ => &self.arrays.absent,
                })
            }
            (InputKind::ImageArray { .. }, ..) => Input::Images(
                self.arrays
                    .textures
                    .get(&spec.key)
                    .with_context(|| format!("forward frame cannot supply {}", spec.name))?,
            ),
            (InputKind::SamplerArray { .. }, ..) => Input::Samplers(
                self.arrays
                    .samplers
                    .get(&spec.key)
                    .unwrap_or(&self.arrays.default_samplers),
            ),
            (InputKind::Image { .. } | InputKind::StorageImage { .. }, group, binding) => {
                Input::Image(match (group, binding) {
                    (3, 24) => self.lists,
                    (0, 18) => self.output,
                    _ => provided(self.sky, self.map, self.traced, self.deferred, spec.key)
                        .or_else(|| self.images.get(&spec.key))
                        .with_context(|| format!("forward frame cannot supply {}", spec.name))?,
                })
            }
            // `environment_sampler`: bilinear within a level, exact integer levels.
            (InputKind::Sampler { .. }, 3, 11) => Input::Sampler(SamplerDesc {
                mip: Filter::Nearest,
                ..SamplerDesc::default()
            }),
            (InputKind::Sampler { .. }, ..) => Input::Sampler(SamplerDesc::default()),
        })
    }
}
/// A fog pass's packet: the scene its rays trace, the atmosphere record, a linear
/// sampler, and its volumes by binding.
fn fog_packet<'a>(
    pass: Pass,
    scene: &'a Scene,
    record: &'a StorageBuffer,
    images: &[(u32, &'a Texture)],
) -> Result<Vec<(InputKey, Input<'a>)>> {
    shader::inputs(pass)?
        .iter()
        .map(|spec| {
            let value = match (spec.kind, spec.key.binding) {
                (InputKind::AccelerationStructure, _) => Input::Scene(scene),
                (InputKind::Buffer { .. }, 0) => Input::Buffer(record),
                (InputKind::Sampler { .. }, _) => Input::Sampler(SamplerDesc::default()),
                (_, binding) => Input::Image(
                    images
                        .iter()
                        .find(|(b, _)| *b == binding)
                        .with_context(|| format!("fog pass cannot supply {}", spec.name))?
                        .1,
                ),
            };
            Ok((spec.key, value))
        })
        .collect()
}

/// The atmosphere's and the shadow passes' images where they ran, and the sky map's.
fn provided<'a>(
    sky: Option<&'a Sky<'a>>,
    map: Option<&'a ResidentMap>,
    traced: Option<&'a Traced>,
    deferred: Option<&'a Deferred>,
    key: InputKey,
) -> Option<&'a Texture> {
    match (key.group, key.binding) {
        (3, 6 | 7 | 9 | 10) if map.is_some() => map?.binding(key.binding),
        (0, 13) | (0, 7) => deferred.map(|d| &d.hit),
        (0, 20) | (0, 21) => deferred.map(|d| &d.tiles),
        (0, 23) => deferred.map(|d| &d.epsilon),
        (0, b) if b == crate::program::MATERIAL_RECORD_BINDINGS[0] => {
            deferred?.material.as_ref().map(|m| &m[0])
        }
        (0, b) if b == crate::program::MATERIAL_RECORD_BINDINGS[1] => {
            deferred?.material.as_ref().map(|m| &m[1])
        }
        (0, 3) | (0, 16) => traced.map(|t| &t.world),
        (0, 4) | (0, 17) => traced.map(|t| &t.normal),
        (0, 5) | (0, 6) => traced.map(|t| &t.visibility),
        (3, 51) => sky.map(|s| &s.luts.transmittance),
        (3, 52) => sky.map(|s| &s.luts.sky),
        (3, 53) => sky.map(|s| s.volume),
        (3, 6) => sky?.environment.map(|e| e[0]),
        (3, 7) => sky?.environment.map(|e| e[1]),
        _ => None,
    }
}

/// `material::sampler_descriptor` for a texture of `levels` mips.
fn sampler_desc(
    sampler: crate::asset::Sampler,
    anisotropy_clamp: u8,
    levels: usize,
) -> Result<SamplerDesc> {
    use crate::asset::{Filter as F, MipmapFilter as M, Wrap};
    use ara_gpu::sampler::AddressMode;
    // The renderer clamps the LOD to the base level for `MipmapFilter::None`; the
    // layer's samplers have no LOD clamp, so that is only the same without a chain.
    ensure!(
        sampler.mipmap != M::None || levels == 1,
        "MipmapFilter::None on a mip chain needs a sampler LOD clamp, which ara-gpu lacks"
    );
    let wrap = |w| match w {
        Wrap::Clamp => AddressMode::ClampToEdge,
        Wrap::Mirror => AddressMode::MirrorRepeat,
        Wrap::Repeat => AddressMode::Repeat,
    };
    let filter = |f| match f {
        F::Nearest => Filter::Nearest,
        F::Linear => Filter::Linear,
    };
    let linear =
        sampler.mag == F::Linear && sampler.min == F::Linear && sampler.mipmap == M::Linear;
    Ok(SamplerDesc {
        min: filter(sampler.min),
        mag: filter(sampler.mag),
        mip: if sampler.mipmap == M::Linear {
            Filter::Linear
        } else {
            Filter::Nearest
        },
        address: [
            wrap(sampler.wrap_u),
            wrap(sampler.wrap_v),
            AddressMode::ClampToEdge,
        ],
        compare: None,
        anisotropy: if linear { anisotropy_clamp } else { 1 },
    })
}

/// The atmosphere's tables a sequence keeps on the GPU, as the renderer does: the
/// transmittance and multiple-scattering tables are remade when the medium changes,
/// the procedural environment when its content would (`update_procedural_environment`'s
/// key), and the sky view every frame, into the same texture.
struct SkyTables {
    /// The medium the first two tables hold; `None` until a frame has made them.
    medium: Option<crate::AtmosphereMedium>,
    luts: AtmosphereTargets,
    environment: Option<Environment>,
    /// The camera volume and the fog's scatter volume, for the grid they were made at.
    /// Fog is not assembled: the scatter volume and the camera volume's fog slices
    /// stay the zeros they were made with, and aerial perspective rewrites its own
    /// slices every frame.
    volumes: Option<([u32; 2], Texture, Texture)>,
}
/// The procedural diffuse and specular cubes with their level views.
struct Environment {
    /// What the cubes were integrated from; `None` until a frame has made them.
    digest: Option<[u8; 32]>,
    diffuse: (Texture, Vec<Texture>),
    specular: (Texture, Vec<Texture>),
}
impl SkyTables {
    fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        let cubes = self.environment.map(|e| [e.diffuse.0, e.specular.0]);
        let volumes = self.volumes.map(|(_, volume, scatter)| [volume, scatter]);
        for image in [
            self.luts.transmittance,
            self.luts.multiscatter,
            self.luts.sky,
        ]
        .into_iter()
        .chain(cubes.into_iter().flatten())
        .chain(volumes.into_iter().flatten())
        {
            super::image::release(gpu, image.image)?;
        }
        Ok(())
    }
    /// A frame that failed may have left any table unwritten: the next remakes all.
    fn forget(&mut self) {
        self.medium = None;
        if let Some(environment) = &mut self.environment {
            environment.digest = None;
        }
    }
}

/// What an active atmosphere produces for the sky backdrop and the lit pass.
struct Sky<'a> {
    luts: &'a AtmosphereTargets,
    /// Camera volume: fog in slices 0..64 (zero here), aerial perspective in 64..96.
    volume: &'a Texture,
    /// The procedural diffuse and specular cubes, when the environment is on.
    environment: Option<[&'a Texture; 2]>,
}

fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}

/// The standard program composed exactly as the wgpu renderer's lit raster variant, or
/// its fresh variant, whose `hard_shadow` reads the receiver pixel's visible-ray count.
pub fn standard_program(fresh: bool) -> Result<Program> {
    static KEPT: Memo<bool, Program> = Memo::new();
    KEPT.get(fresh, || standard_program_uncached(fresh))
}
fn standard_program_uncached(fresh: bool) -> Result<Program> {
    let source = crate::program::STANDARD_PROGRAM;
    let scene = include_str!("../../shaders/scene.wgsl");
    let head = if fresh {
        crate::renderer::fresh_shadow_gpu::shader_source(scene)?
    } else {
        scene.to_owned()
    };
    Ok(Program::new(
        Pass::MeshToon,
        crate::program::compose(
            &head,
            source,
            crate::program::Stage::Raster,
            crate::program::declares_parameters(source),
        )
        .source,
    )?
    .checked())
}

/// Renders `scene` once, as the first frame after a cut, and reads back what it
/// produced. Use [`Frames`] for a sequence.
pub fn render(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    scene: &ForwardScene<'_>,
) -> Result<ForwardImages> {
    let mut frames = Frames::default();
    let images = frames.render(gpu, cache, scene);
    frames.retire(gpu)?;
    images
}

/// What one frame hands to the next, as the wgpu renderer keeps it: post's frame serial
/// and previous camera, the shadow planner's light orientations and the placement
/// transforms.
#[derive(Default)]
struct Carried {
    /// Frames rendered since creation.
    count: u64,
    /// Post frames prepared so far: the `Settings` record's serial.
    post_serial: u32,
    /// The unjittered view-projection post last saw, and the frame it is previous to.
    /// A frame without post in between breaks the chain, as `PostGpu::prepare` does.
    post_camera: Option<(u64, crate::Mat4)>,
    /// `ShadowState::bases`: the orientations the planner transports from.
    shadow_bases: Vec<Option<glam::DMat4>>,
    /// The last frame's object transforms, for traced frames' previous matrices.
    transforms: Vec<crate::Mat4>,
    /// Volumetric fog's history source: the last lit frame's view-projection and eye,
    /// and the key its fog was integrated under (`FogHistory`).
    fog: Option<(crate::Mat4, crate::Vec3, [u8; 32])>,
    /// Frames the atmosphere has encoded (`Atmosphere::frame`).
    fog_frame: u32,
}
/// What a sequence keeps on the GPU from frame to frame.
#[derive(Default)]
struct Kept {
    /// The radiance cache, allocated by the first frame that asks for it.
    gi: Option<Cache>,
    /// Temporal reconstruction's history, allocated the same way.
    tr1: Option<History>,
    /// The meshes the last frame named by identity (`ForwardScene::meshes`).
    meshes: BTreeMap<u64, ResidentMesh>,
    /// The atmosphere's tables, made by the first frame with an active atmosphere.
    sky: Option<SkyTables>,
    /// The sky map the last frame named (`ForwardScene::sky_map`).
    map: Option<ResidentMap>,
    /// The textures the last frame named by identity (`ObjectTexture::key`), by
    /// identity and slot: a slot fixes the format and view.
    textures: BTreeMap<(u64, usize), Texture>,
    /// Volumetric fog's cells at their grid: this frame's raw cells, and the resolved
    /// cells of the two most recent frames (each frame's history is the other).
    fog: Option<([u32; 2], [Texture; 3])>,
    /// The object ids of the last frame that resolved them.
    ids: Option<IdTargets>,
    /// The Interactive trace's repair queue: a header, then room for every pixel.
    repairs: Option<StorageBuffer>,
    /// Which point and spot lights trace shadow rays (`LocalShadows::rays`), kept from
    /// frame to frame for their fades.
    rays: lc::RaySelection,
    /// The 2D targets the last frame made and released, for the next frame to take.
    targets: super::image::pool::Pool,
    /// The displayed images, two so a frame draws into the one not on screen while a
    /// presenter may still read the other; `shown` is the last frame's.
    outputs: [Option<Texture>; 2],
    shown: Option<usize>,
    /// Frames submitted without waiting for the GPU, oldest first (`settle`).
    in_flight: std::collections::VecDeque<ara_gpu::GpuCompletion>,
    /// The newest finished frame's device span and passes, until a frame reports them.
    done: Option<(std::time::Duration, Vec<(String, f64)>)>,
}
/// A sequence of frames: what each hands to the next, and what stays on the GPU (the
/// radiance cache, the reconstruction's history, named meshes with their bottom-level
/// structures, the atmosphere's tables), and the 2D targets each frame takes and gives
/// back (`image::pool`). Records and buffers are still made per frame.
/// Call [`Frames::retire`] when the sequence ends.
#[derive(Default)]
pub struct Frames {
    carried: Carried,
    kept: Kept,
    /// Frames leave their images on the GPU: `ForwardImages::hdr` comes back empty
    /// and `samples`, `surface`, `display` and `focused` as `None` (`without_images`).
    blind: bool,
}
impl Frames {
    /// A sequence whose frames are rendered and not read back: for timing, where the
    /// copies to the CPU are not the frame's work. Tables and history asked for by
    /// `Indirect::read_table` and `Reconstruct::read_history` still come back.
    /// A frame that reads nothing back returns without waiting for the GPU: one
    /// earlier frame may still run while the next records, and its times arrive with
    /// a later frame (`FrameTimes::gpu`).
    pub fn without_images() -> Self {
        Self {
            blind: true,
            ..Self::default()
        }
    }
    /// Forgets history: the next frame is a first frame after a cut. The cache keeps
    /// its buffers and starts from an empty table, as `Renderer::reset_indirect`, and the
    /// ray-shadowed lights are chosen afresh. The meshes and the atmosphere's tables stay
    /// as they are.
    pub fn cut(&mut self) {
        if let Some(cache) = &mut self.kept.gi {
            cache.reset();
        }
        if let Some(history) = &mut self.kept.tr1 {
            history.cut();
        }
        // The cut frame's shadowed lights start at full strength.
        self.kept.rays = lc::RaySelection::default();
        self.carried = Carried::default();
    }
    /// Releases what the sequence kept on the GPU, after its frames in flight finish.
    pub fn retire(mut self, gpu: &mut GpuDevice) -> Result<()> {
        settle(gpu, &mut self.kept.in_flight, &mut self.kept.done, 0)?;
        if let Some(cache) = self.kept.gi {
            cache.retire(gpu)?;
        }
        if let Some(history) = self.kept.tr1 {
            history.retire(gpu)?;
        }
        for mesh in self.kept.meshes.into_values() {
            mesh.retire(gpu)?;
        }
        if let Some(sky) = self.kept.sky {
            sky.retire(gpu)?;
        }
        if let Some(map) = self.kept.map {
            map.retire(gpu)?;
        }
        for texture in self.kept.textures.into_values() {
            gpu.retire(Resource::Image(texture.image))?;
        }
        for texture in self
            .kept
            .ids
            .into_iter()
            .flat_map(|t| [t.ids, t.packed, t.settled[0], t.settled[1]])
        {
            gpu.retire(Resource::Image(texture.image))?;
        }
        for texture in self.kept.fog.into_iter().flat_map(|(_, cells)| cells) {
            gpu.retire(Resource::Image(texture.image))?;
        }
        if let Some(repairs) = self.kept.repairs {
            repairs.retire(gpu)?;
        }
        for output in self.kept.outputs.into_iter().flatten() {
            gpu.retire(ara_gpu::Resource::Image(output.image))?;
        }
        self.kept.targets.retire(gpu)
    }
    /// The id texture of the last frame that resolved ids (`ForwardScene::ids`): R32Uint,
    /// layer 0 the opaque ids and layer 1 the front ids, at the output's size, left read.
    pub fn ids(&self) -> Option<&Texture> {
        self.kept.ids.as_ref().map(|t| &t.ids)
    }
    /// Packs `layer` (0 opaque, 1 front) of the last ids into the sequence's RGBA8 image,
    /// bit-exact little-endian (R = bits 0-7 ... A = 24-31), and returns it, left in
    /// `CopySource` for a presenter. Returns once the pack is submitted: work that reads the
    /// image on this device's queue runs after it.
    pub fn pack_ids(
        &mut self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        layer: u32,
    ) -> Result<&Texture> {
        let targets = self.kept.ids.as_mut().context("no frame resolved ids")?;
        ensure!(layer < 2, "an id layer is 0 (opaque) or 1 (front)");
        let mut graph = Graph::default();
        let mut rec = gpu.begin(Queue::Graphics);
        graph.import(Resource::Image(targets.ids.image), Access::ShaderRead);
        if targets.camera.is_some() && targets.packed_written {
            graph.import(Resource::Image(targets.packed.image), Access::CopySource);
        }
        let uniform = StorageBuffer::new(gpu, &[layer, 0, 0, 0].map(u32::to_le_bytes).concat())?;
        graph.import(Resource::Buffer(uniform.buffer), Access::HostWrite);
        let values = shader::inputs(Pass::IdPack)?
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match spec.key.binding {
                        0 => Input::Image(&targets.ids),
                        1 => Input::Image(&targets.packed),
                        2 => Input::Buffer(&uniform),
                        _ => bail!("the id pack cannot supply {}", spec.name),
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let bindings = HeapBindings::new(gpu, Pass::IdPack, &values)?;
        bindings.import(&mut graph);
        let size = targets.size;
        Kernel::new(gpu, cache, Pass::IdPack)?.record(
            &mut graph,
            &mut rec,
            &bindings,
            [size[0], size[1], 1],
        )?;
        graph.node(
            &mut rec,
            &[(Resource::Image(targets.packed.image), Access::CopySource)],
        )?;
        let record = bindings.release();
        gpu.submit(rec.finish(), &[])?;
        // Reclaimed once the submission completes.
        record.retire(gpu)?;
        uniform.retire(gpu)?;
        targets.packed_written = true;
        Ok(&targets.packed)
    }
    /// Reads back `rect` ([x, y, width, height]) of the last ids: (opaque, front) per
    /// pixel, row by row; waits for the GPU.
    pub fn read_ids(&mut self, gpu: &mut GpuDevice, rect: [u32; 4]) -> Result<Vec<[u32; 2]>> {
        let targets = self.kept.ids.as_ref().context("no frame resolved ids")?;
        let [x, y, w, h] = rect;
        ensure!(
            w > 0 && h > 0 && x + w <= targets.size[0] && y + h <= targets.size[1],
            "id rectangle outside the last ids"
        );
        let layer_bytes = u64::from(w) * u64::from(h) * 4;
        let buffer = gpu.create_readback_buffer(layer_bytes * 2)?;
        let mut graph = Graph::default();
        let mut rec = gpu.begin(Queue::Graphics);
        graph.import(Resource::Image(targets.ids.image), Access::ShaderRead);
        graph.node(
            &mut rec,
            &[
                (Resource::Image(targets.ids.image), Access::CopySource),
                (Resource::Buffer(buffer), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::BeginPass("ara::IdReadback".into()));
        rec.push(Command::ReadImageRegion {
            source: targets.ids.image,
            destination: buffer,
            buffer_offset: 0,
            region: ara_gpu::texture::ImageRegion {
                mip: 0,
                origin: [x, y, 0],
                extent: [w, h, 1],
                base_layer: 0,
                layer_count: 2,
            },
        });
        rec.push(Command::EndPass);
        graph.node(
            &mut rec,
            &[
                (Resource::Image(targets.ids.image), Access::ShaderRead),
                (Resource::Buffer(buffer), Access::HostRead),
            ],
        )?;
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.resolve(point)?;
        let bytes = gpu.read(buffer)?;
        gpu.retire(Resource::Buffer(buffer))?;
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        Ok((0..(w * h) as usize)
            .map(|i| [word(i * 4), word(layer_bytes as usize + i * 4)])
            .collect())
    }
    /// The last frame's displayed image (sRGB-encoded RGBA8, the output's size), kept on
    /// the device until the frame after next draws over it, and left in `CopySource`:
    /// what a presenter shows. `None` before the first frame with post enabled.
    pub fn output(&self) -> Option<&Texture> {
        self.kept.shown.and_then(|s| self.kept.outputs[s].as_ref())
    }
    /// The last reconstructed frame's plan (`History::planned`), for telemetry.
    pub fn reconstruction_plan(&self) -> Option<(bool, u32)> {
        self.kept.tr1.as_ref().map(History::planned)
    }
    /// The serial the next post frame's `Settings` record carries.
    pub fn next_post_serial(&self) -> u32 {
        self.carried.post_serial.wrapping_add(1)
    }
    /// Renders the next frame of the sequence and reads back what it produced.
    pub fn render(
        &mut self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        scene: &ForwardScene<'_>,
    ) -> Result<ForwardImages> {
        let mut targets = std::mem::take(&mut self.kept.targets);
        let result = self.render_with(gpu, cache, scene, &mut targets);
        self.kept.targets = targets;
        result
    }
    /// [`render`](Self::render) taking its 2D targets from `targets`, a pool other
    /// sequences share (an engine's views): the sequence's own pool is not used.
    pub(crate) fn render_with(
        &mut self,
        gpu: &mut GpuDevice,
        cache: &ShaderCache,
        scene: &ForwardScene<'_>,
        targets: &mut super::image::pool::Pool,
    ) -> Result<ForwardImages> {
        let result = super::image::pool::active(targets, gpu, |gpu| {
            frame(&self.carried, &mut self.kept, self.blind, gpu, cache, scene)
        });
        match result {
            Ok((images, carried)) => {
                self.carried = carried;
                Ok(images)
            }
            Err(error) => {
                // What is kept outlives a failed frame. The cache's planner may already
                // have planned that frame (and consumed a pending clear), so it starts
                // from an empty table again; the history and the tables likewise.
                if let Some(cache) = &mut self.kept.gi {
                    cache.reset();
                }
                if let Some(history) = &mut self.kept.tr1 {
                    history.cut();
                }
                if let Some(sky) = &mut self.kept.sky {
                    sky.forget();
                }
                Err(error)
            }
        }
    }
}

/// Consumes the frames in `in_flight` the GPU has finished, oldest first, waiting for
/// the oldest while more than `keep` remain, and leaves the newest finished frame's
/// device span and passes (summed by label, in first-run order) in `done`.
fn settle(
    gpu: &mut GpuDevice,
    in_flight: &mut std::collections::VecDeque<ara_gpu::GpuCompletion>,
    done: &mut Option<(std::time::Duration, Vec<(String, f64)>)>,
    keep: usize,
) -> Result<()> {
    loop {
        let waiting = in_flight.len() > keep;
        let Some(front) = in_flight.front_mut() else {
            return Ok(());
        };
        let finished = if waiting {
            Some(gpu.wait_completion(front)?)
        } else {
            gpu.poll_completion(front)?
        };
        let Some(finished) = finished else {
            return Ok(());
        };
        let span = front
            .timing_window()
            .map_or(0., ara_gpu::commands::TimestampSpan::milliseconds);
        let mut passes: Vec<(String, f64)> = Vec::new();
        for pass in finished {
            match passes.iter_mut().find(|(label, _)| *label == pass.pass) {
                Some((_, total)) => *total += pass.milliseconds,
                None => passes.push((pass.pass, pass.milliseconds)),
            }
        }
        *done = Some((std::time::Duration::from_secs_f64(span / 1000.), passes));
        in_flight.pop_front();
    }
}

/// A pane's world bounds, least and greatest corner.
fn pane_bounds(object: &ForwardObject<'_>) -> Result<(crate::Vec3, crate::Vec3)> {
    let local = crate::visibility::Aabb::from_positions(object.vertices.iter().map(|v| v.position))
        .context("a transparent pane without vertices")?;
    let b = shadow_gpu::world_bounds(local, object.transform)?;
    Ok((b.min, b.max))
}

/// A program's reflected `Parameters` layout, composed and validated at the raster
/// stage as `Renderer::try_create_program` does. Kept by source for the life of the
/// process: a frame names the same programs every time, and validating one parses the
/// whole scene shader.
fn parameter_layout(source: &str) -> Result<Option<crate::program::ParameterLayout>> {
    type Kept = std::sync::Mutex<BTreeMap<[u8; 32], Option<crate::program::ParameterLayout>>>;
    static KEPT: std::sync::OnceLock<Kept> = std::sync::OnceLock::new();
    use sha2::Digest as _;
    let key: [u8; 32] = sha2::Sha256::digest(source.as_bytes()).into();
    let table = KEPT.get_or_init(Default::default);
    if let Some(kept) = table.lock().expect("layout lock").get(&key) {
        return Ok(kept.clone());
    }
    let composed = crate::program::compose(
        include_str!("../../shaders/scene.wgsl"),
        source,
        crate::program::Stage::Raster,
        crate::program::declares_parameters(source),
    );
    let layout = crate::program::analyse("scene program", source, &composed)
        .map_err(|e| anyhow::anyhow!("scene program: {e}"))?
        .parameters;
    table
        .lock()
        .expect("layout lock")
        .insert(key, layout.clone());
    Ok(layout)
}

/// One frame after `history`, over what the sequence keeps on the GPU; returns its
/// images and what it hands to the next frame.
fn frame(
    history: &Carried,
    kept: &mut Kept,
    blind: bool,
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    scene: &ForwardScene<'_>,
) -> Result<(ForwardImages, Carried)> {
    let Kept {
        gi,
        tr1,
        meshes: resident,
        sky: tables,
        map: kept_map,
        textures: kept_textures,
        fog: kept_fog,
        ids: kept_ids,
        repairs: kept_repairs,
        rays: ray_selection,
        // Taken out and made active for this frame by `Frames::render`.
        targets: _,
        outputs,
        shown: shown_output,
        in_flight,
        done,
    } = kept;
    // At most one earlier frame stays in flight while this one records.
    settle(gpu, in_flight, done, 1)?;
    let started = std::time::Instant::now();
    let mut times = FrameTimes::default();
    ensure!(
        scene.instances.is_none_or(|lists| {
            scene.primary != Primary::Raster
                && lists.len() == scene.objects.len()
                && lists.iter().all(|list| !list.is_empty())
        }),
        "instances are assembled on traced frames, one nonempty list per object"
    );
    // Each object's placements: its instance list, or its own transform.
    let placed = |index: usize| -> &[crate::Mat4] {
        match scene.instances {
            Some(lists) => lists[index],
            None => std::slice::from_ref(&scene.objects[index].transform),
        }
    };
    ensure!(
        scene.casts.is_none_or(|casts| {
            casts.len() == scene.objects.len()
                && casts
                    .iter()
                    .enumerate()
                    .all(|(index, list)| list.len() == placed(index).len())
        }),
        "shadow casting is given per placement, one list per object"
    );
    // Whether placement `k` of object `index` blocks light (`ForwardScene::casts`).
    let casts = |index: usize, k: usize| scene.casts.is_none_or(|lists| lists[index][k]);
    // Each object's previous pose, where it is skinned.
    let skin = |index: usize| scene.skins.and_then(|s| s.get(index).copied().flatten());
    ensure!(
        scene.skins.is_none_or(|s| {
            scene.primary != Primary::Raster
                && s.len() == scene.objects.len()
                && s.iter().enumerate().all(|(i, k)| {
                    k.is_none_or(|k| k.previous.len() == scene.objects[i].vertices.len())
                })
        }),
        "skins are assembled on traced frames, one entry per object, one previous vertex per vertex"
    );
    let mut left = Carried {
        count: history.count + 1,
        post_serial: history.post_serial,
        post_camera: None,
        shadow_bases: Vec::new(),
        // Per placement, in draw then instance order, as every table below.
        transforms: (0..scene.objects.len())
            .flat_map(|i| placed(i).iter().copied())
            .collect(),
        fog: None,
        fog_frame: history.fog_frame,
    };
    ensure!(
        scene.objects.iter().all(|o| {
            o.material.base_color_texture.is_none()
                && o.material.metallic_roughness_texture.is_none()
                && o.material.normal_texture.is_none()
                && o.material.program.is_none()
        }),
        "forward frame binds the default textures and the standard program only"
    );
    // The caller's size, and the size every pass before the resolve renders at: below
    // native scale a reconstructed frame is drawn at an internal size and resolved to
    // the caller's (`Renderer::upscale_target`). Equal otherwise.
    let target = scene.size;
    let deferred_path = scene.primary != Primary::Raster;
    let size = match scene
        .reconstruction
        .filter(|r| r.settings.enabled && deferred_path)
    {
        Some(r) => r.settings.scale.internal_size(target[0], target[1]),
        None => target,
    };
    ensure!(
        scene.transparent.is_empty() || deferred_path,
        "transparent panes are assembled on traced frames only"
    );
    ensure!(
        scene
            .transparent
            .iter()
            .all(|o| o.material.transparency.is_some()),
        "a transparent pane's material carries a Transparency"
    );
    // Each object's normal map, by object index.
    let normal_map = |index: usize| scene.textures.and_then(|t| t.objects.get(index)?.normal);
    for (index, object) in scene.objects.iter().enumerate() {
        if let Some(map) = normal_map(index) {
            ensure!(
                deferred_path,
                "normal maps are assembled on traced frames only"
            );
            crate::asset::validate_tangents(object.vertices, object.indices, map.tangents)
                .map_err(|e| anyhow::anyhow!("object {index}'s tangents: {e:?}"))?;
        }
    }
    let interactive = scene.primary == Primary::TracedInteractive;
    ensure!(
        !deferred_path || scene.sun_shadows.is_some(),
        "a traced frame needs sun shadows"
    );
    // `prepare_cache_and_post`: only traced frames feed the cache, and an empty frame
    // has no surface to cache (its misses read the sky directly).
    let indirect = scene
        .indirect
        .filter(|i| i.settings.enabled && !scene.objects.is_empty());
    ensure!(
        indirect.is_none() || deferred_path,
        "the radiance cache runs on traced frames"
    );
    // `render_view`: a reconstructed frame draws everything through the camera jittered
    // by its sample of the sequence; post and the cache's still key use it unjittered.
    // The traced shading program carries its reflection rays whenever its material
    // declares a lobe; the frame record's words switch them on (`primary_frame.reflection`).
    let reflections = scene
        .reflections
        .unwrap_or_else(crate::ReflectionSettings::off);
    reflections
        .validate()
        .map_err(|e| anyhow::anyhow!("reflection settings: {e}"))?;
    ensure!(
        !reflections.enabled || deferred_path,
        "traced reflections run on traced frames"
    );
    let reconstruct = scene.reconstruction.filter(|r| r.settings.enabled);
    ensure!(
        reconstruct.is_none() || deferred_path,
        "temporal reconstruction runs on traced frames"
    );
    if let Some(r) = reconstruct {
        ensure!(
            tr1.as_ref().is_none_or(|h| h.matches(r.settings, target)),
            "reconstruction settings or size changed within a sequence, which is not assembled yet"
        );
        if tr1.is_none() {
            *tr1 = Some(History::new(gpu, cache, r.settings, target)?);
        }
    }
    let sample = tr1
        .as_ref()
        .filter(|_| reconstruct.is_some())
        .map(History::index);
    let view = match sample {
        Some(k) => scene.camera.jittered(tr::sample_offset(k)),
        None => scene.camera,
    };
    // The frame's one shading program and each object's parameter values.
    let object_program = |index: usize| scene.programs.and_then(|p| *p.objects.get(index)?);
    if let Some(programs) = scene.programs {
        ensure!(
            deferred_path && programs.objects.len() == scene.objects.len(),
            "scene programs are assembled on traced frames, one entry per object"
        );
    }
    // `frame_slots`: the frame's distinct programs in first-draw order and each
    // object's slot among them. Slot 0 also writes misses.
    let mut sources: Vec<&str> = Vec::new();
    let slots: Vec<u32> = (0..scene.objects.len())
        .map(|i| {
            let source = object_program(i).map_or(crate::program::STANDARD_PROGRAM, |p| p.source);
            let slot = sources
                .iter()
                .position(|s| *s == source)
                .unwrap_or_else(|| {
                    sources.push(source);
                    sources.len() - 1
                });
            slot as u32
        })
        .collect();
    // An empty frame still shades its misses: through the standard program.
    if sources.is_empty() {
        sources.push(crate::program::STANDARD_PROGRAM);
    }
    ensure!(
        sources.len() <= crate::renderer::primary_ray_gpu::program_lists::MAX_FRAME_PROGRAMS,
        "a frame shades at most MAX_FRAME_PROGRAMS programs"
    );
    let material_source = *sources.first().context("no object")?;
    // `Engine::create_program`: each program must compose and validate, and its
    // `Parameters` block is reflected from it.
    let parameter_layouts = sources
        .iter()
        .map(|source| parameter_layout(source))
        .collect::<Result<Vec<_>>>()?;
    for index in 0..scene.objects.len() {
        if let Some(program) = object_program(index) {
            crate::program::ParameterLayout::check(
                parameter_layouts[slots[index] as usize].as_ref(),
                program.parameters.iter().map(|(n, v)| (*n, v.clone())),
            )
            .map_err(|e| anyhow::anyhow!("object {index}'s parameters: {e}"))?;
        }
    }
    // Lean shading (program::FrameInputs): the port projects no decals; media follow the
    // atmosphere's flags.
    let lean = crate::program::FrameInputs {
        decals: false,
        media: scene.atmosphere.aerial_perspective
            || scene.atmosphere.height_fog.is_some()
            || scene.atmosphere.volumetric_fog.is_some(),
    }
    .lean()
        && crate::program::lean_on();
    // RENDERLAB_MATERIAL_PASS (program.rs material_pass_on): programs with the record mapping
    // shade from the material record, which a material pass writes first.
    let material_on = deferred_path && crate::program::material_pass_on();
    let reading = |source: &str, list: bool| -> Result<Option<shader::Program>> {
        if material_on {
            primary_shade::hardware_soft_material(source, list, crate::program::MaterialRole::Read)
        } else {
            Ok(None)
        }
    };
    let program = if deferred_path {
        match reading(material_source, false)? {
            Some(program) => program,
            None => primary_shade::hardware_soft_part(material_source, primary_shade::ALL_TERMS, lean)?,
        }
    } else {
        standard_program(scene.sun_shadows.is_some())?
    };
    // A traced frame of several programs: each one's list entry, in slot order.
    let list_programs = if deferred_path && sources.len() > 1 {
        sources
            .iter()
            .map(|source| match reading(source, true)? {
                Some(program) => Ok(program),
                None => primary_shade::hardware_soft_list_part(source, primary_shade::ALL_TERMS, lean),
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    // The material pass: per slot (one for a single-program frame), the program's record
    // writer, or None where the program keeps its single kernel.
    let material_programs = if material_on {
        let list = sources.len() > 1;
        let frame_sources: &[&str] = if list { &sources } else { &sources[..1] };
        frame_sources
            .iter()
            .map(|source| {
                primary_shade::hardware_soft_material(source, list, crate::program::MaterialRole::Write)
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    // The pass log's shading parts (src/shade_detail.rs): each part's tiled program, or its
    // list programs in slot order.
    let shade_parts = if deferred_path && crate::renderer::shade_detail::wanted() {
        crate::renderer::shade_detail::PARTS
            .iter()
            .map(|&(name, terms)| {
                let programs = if sources.len() > 1 {
                    sources
                        .iter()
                        .map(|source| primary_shade::hardware_soft_list_part(source, terms, lean))
                        .collect::<Result<Vec<_>>>()?
                } else {
                    vec![primary_shade::hardware_soft_part(material_source, terms, lean)?]
                };
                Ok((name, programs))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let pass = if deferred_path {
        None
    } else {
        Some(ToonPass::program(
            gpu,
            cache,
            &program,
            Format::Rgba16Float,
        )?)
    };
    // Per placement, its world bounds and whether it blocks light (the shadow plan's casters).
    let (mut bounds, mut skin_bounds, mut casters) = (Vec::new(), Vec::new(), Vec::new());
    for (index, object) in scene.objects.iter().enumerate() {
        let local =
            crate::visibility::Aabb::from_positions(object.vertices.iter().map(|v| v.position))
                .context("empty mesh")?;
        for (k, transform) in placed(index).iter().enumerate() {
            let world = shadow_gpu::world_bounds(local, *transform)?;
            bounds.push(world);
            casters.push(casts(index, k));
            if skin(index).is_some() {
                skin_bounds.push(world);
            }
        }
    }
    // Per placement, the skinned draw's identity and pose revision (`PlacementPose`), and
    // every skinned draw's revision in draw order.
    let poses: Vec<Option<(usize, u64)>> = (0..scene.objects.len())
        .flat_map(|i| {
            let pose = skin(i).map(|k| {
                let name = scene.meshes.map_or(i as u64, |m| m[i]);
                (name as usize, k.revision)
            });
            std::iter::repeat_n(pose, placed(i).len())
        })
        .collect();
    let skin_revisions: Vec<u64> = (0..scene.objects.len())
        .filter_map(|i| skin(i).map(|k| k.revision))
        .collect();
    let clusters = LightClusters::new(gpu, cache)?;
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();

    // `Sky::Map`: the map's images, uploaded by the first frame that names it and read
    // by every frame after it. Another map, or none, releases them.
    if kept_map
        .as_ref()
        .is_some_and(|kept| scene.sky_map.is_none_or(|map| map.key != kept.key))
    {
        kept_map.take().context("checked")?.retire(gpu)?;
    }
    match (scene.sky_map, kept_map.as_ref()) {
        (Some(_), Some(kept)) => kept.import(&mut graph),
        (Some(map), None) => {
            ensure!(
                !scene.atmosphere.environment,
                "a sky map lights the scene in place of the procedural environment"
            );
            *kept_map = Some(ResidentMap::upload(
                gpu,
                &mut graph,
                &mut rec,
                &mut staging,
                &map,
            )?);
        }
        (None, _) => {}
    }
    let map = kept_map.as_ref();

    // Lighting group 3: records, the eye/count/environment/exposure uniform and the
    // cluster frame, packed by Renderlab's own writers.
    let packed = PackedLighting::new(scene.lighting, lc::MAX_CLUSTERED as u32)?;
    let eye = view.view().inverse().w_axis.truncate();
    let mut uniform = [0u8; 48];
    uniform[..32].copy_from_slice(&packed.uniform(eye));
    let mut records = packed.records.clone();
    records.resize(records.len().max(crate::lighting::LIGHT_BYTES), 0);
    // `Renderer::render_view` with hardware shadows: plan the sun's views, convert their
    // biases to ray distances and publish each light's view range in its record.
    let shadow_frame = match scene.sun_shadows {
        Some(sun_shadows) => {
            let mut first = true;
            let shadows = crate::Shadows {
                requests: scene
                    .lighting
                    .lights
                    .iter()
                    .map(|l| match l {
                        crate::Light::Directional { .. } if std::mem::take(&mut first) => {
                            crate::ShadowRequest::Directional {
                                split_ends: sun_shadows.split_ends.clone(),
                            }
                        }
                        _ => crate::ShadowRequest::None,
                    })
                    .collect(),
                directional_disk_angle: sun_shadows.disk_angle,
                ..Default::default()
            };
            ensure!(
                !first && sun_shadows.disk_angle > 0. && sun_shadows.disk_angle <= 0.1,
                "sun shadows need a directional light and a disk angle in (0, 0.1]"
            );
            // `hardware_shadow_gpu`'s admission rule for casters.
            ensure!(
                scene.objects.iter().all(|o| o.material.double_sided),
                "hardware shadows currently require opaque double-sided materials"
            );
            let mut frame = shadow_gpu::plan_frame(
                &shadows,
                &history.shadow_bases,
                &view,
                size,
                &scene.lighting.lights,
                &bounds,
                &casters,
            )?;
            frame.prepare_hardware_bias(&records)?;
            left.shadow_bases = frame.bases().to_vec();
            for (record, range) in records
                .as_chunks_mut::<{ crate::lighting::LIGHT_BYTES }>()
                .0
                .iter_mut()
                .zip(&frame.light_ranges)
            {
                record[52..56].copy_from_slice(&range[0].to_ne_bytes());
                record[56..60].copy_from_slice(&range[1].to_ne_bytes());
            }
            Some(frame)
        }
        None => None,
    };
    // `Renderer::atmosphere_sun` and `sun_scale`: the first directional light.
    let settings = scene.atmosphere;
    settings.validate()?;
    let sun = scene.lighting.lights.iter().find_map(|l| match *l {
        crate::Light::Directional { travel, lux } => Some((-travel.normalize(), lux)),
        _ => None,
    });
    if settings.active() {
        ensure!(
            settings.volumetric_fog.is_none() || deferred_path,
            "volumetric fog is assembled on traced frames only (its sun rays need the scene)"
        );
        if let (true, Some((toward, _))) = (settings.sun_transmittance, sun) {
            records = crate::atmosphere::scale_directional(
                &records,
                settings.transmittance(eye.y - settings.medium.ground_height, toward),
            );
        }
        if settings.environment {
            // `EnvironmentState::uniform` for the unrotated five-level procedural chain.
            uniform[32..].copy_from_slice(bytemuck::cast_slice(&[1f32, 0., 1., 5.]));
        }
    }
    if let (Some(sky_map), Some(kept)) = (scene.sky_map, map) {
        uniform[32..].copy_from_slice(bytemuck::cast_slice(&kept.words(&sky_map)));
    }
    // What `update_procedural_environment` keys the procedural map's content on, and
    // the environment uniform's rotation words: the cache re-integrates when they change.
    let field = settings.active() && settings.environment;
    let environment_digest: [u8; 32] = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        if field {
            let altitude = ((eye.y - settings.medium.ground_height) * 1e-3).clamp(
                1e-3,
                settings.medium.top_radius_km - settings.medium.ground_radius_km - 1e-3,
            );
            h.update(format!("{:?}", (sun, settings.medium, altitude)).as_bytes());
        }
        h.update(&uniform[32..40]);
        // A sky map by its identity: another map re-integrates the cache.
        if let Some(map) = scene.sky_map {
            h.update(map.key.to_le_bytes());
        }
        h.finalize().into()
    };
    // `prepare_light_clusters` on a frame whose shading traces local shadows: the ray
    // budget's point and spot lights name themselves (`RaySelection`), with no map slot.
    let local_shadows = scene.local_shadows.filter(|s| s.count > 0);
    ensure!(
        local_shadows.is_none() || deferred_path,
        "local-light shadows are assembled on traced frames only"
    );
    if let Some(settings) = &local_shadows {
        let vp = scene.camera.matrix(size[0], size[1]);
        let eye = scene.camera.view().inverse().w_axis.truncate();
        let words = ray_selection.words(&scene.lighting.lights, vp, eye, settings);
        for (record, word) in records
            .as_chunks_mut::<{ crate::lighting::LIGHT_BYTES }>()
            .0
            .iter_mut()
            .zip(words)
        {
            if word != 0 {
                record[60..64].copy_from_slice(&word.to_ne_bytes());
            }
        }
    }
    let mut frame = lc::header(&view, size, packed.count, true, &[]);
    // LC2's source-sampling word turns only with reconstruction: its sample plus one.
    let turn = sample.map_or(0u32, |k| k.wrapping_add(1) as u32);
    frame[88..92].copy_from_slice(&turn.to_ne_bytes());
    let near = scene.local_shadows.unwrap_or_default().near;
    frame[100..104].copy_from_slice(&near.to_ne_bytes());
    // No local shadows assembled: hits have none to drop.
    let unshadowed = scene
        .local_shadows
        .map_or(0.0, |s| lc::unshadowed_hits_word(&s));
    frame[104..108].copy_from_slice(&unshadowed.to_ne_bytes());
    let cluster_inputs = ClusterInputs {
        lights: StorageBuffer::new(gpu, &records)?,
        frame: StorageBuffer::new(gpu, &frame)?,
    };
    cluster_inputs.import(&mut graph);
    let lists = LightClusters::target(gpu)?;
    clusters.record(&mut graph, &mut rec, &cluster_inputs, &lists)?;

    let mut camera = [0u8; 80];
    camera[..64].copy_from_slice(&crate::mesh::matrix_bytes(view.matrix(size[0], size[1])));
    camera[64..68].copy_from_slice(&(crate::ViewMode::Lit as u32).to_ne_bytes());
    let camera = StorageBuffer::new(gpu, &camera)?;
    let lighting = StorageBuffer::new(gpu, &uniform)?;
    let grid = if settings.active() {
        size.map(|n| n.div_ceil(FROXEL_TILE))
    } else {
        [1, 1]
    };
    let (toward_sun, illuminance) = sun.unwrap_or((crate::Vec3::Y, [0.; 3]));
    // Volumetric fog reuses the last frame's cells while nothing they were integrated
    // under changed: the renderer drops its history on any geometry, placement or pose
    // upload, a new grid, and a change of sun, exposure or environment.
    let fogged = settings.active() && settings.volumetric_fog.is_some();
    let fog_key: [u8; 32] = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(format!("{sun:?}{grid:?}{:?}", packed.exposure).as_bytes());
        h.update(environment_digest);
        for transform in &left.transforms {
            h.update(bytemuck::cast_slice(&transform.to_cols_array()));
        }
        h.update(format!("{poses:?}").as_bytes());
        if let Some(meshes) = scene.meshes {
            h.update(bytemuck::cast_slice(meshes));
        }
        h.finalize().into()
    };
    let fog_history = crate::backend_ara::atmosphere::FogHistory {
        previous: history
            .fog
            .filter(|(_, _, key)| fogged && *key == fog_key)
            .map(|(matrix, eye, _)| (matrix, eye)),
        frame: history.fog_frame,
    };
    if settings.active() {
        left.fog_frame = history.fog_frame.wrapping_add(1);
        left.fog = fogged.then(|| (view.matrix(size[0], size[1]), eye, fog_key));
    }
    let atmosphere = AtmosphereParameters::for_frame(
        gpu,
        settings,
        AtmosphereSun {
            toward_sun,
            illuminance,
            exposure: packed.exposure,
        },
        view,
        size,
        grid,
        fog_history,
    )?;
    for buffer in [&camera, &lighting, &atmosphere.record] {
        graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
    }

    // `Renderer::encode_atmosphere`, in its order, over the tables the sequence keeps.
    let sky = if settings.active() {
        // Make what is missing, and decide what this frame remakes, before the tables
        // are borrowed for the rest of the frame.
        let made = tables.is_none();
        if made {
            *tables = Some(SkyTables {
                medium: None,
                luts: AtmosphereTargets::new(gpu)?,
                environment: None,
                volumes: None,
            });
        }
        let kept = tables.as_mut().context("made above")?;
        let cubes_made = settings.environment && kept.environment.is_none();
        if cubes_made {
            kept.environment = Some(Environment {
                digest: None,
                specular: Texture::cube_chain(gpu, 128, 5)?,
                diffuse: Texture::cube_chain(gpu, 16, 1)?,
            });
        }
        let volumes_made = kept
            .volumes
            .as_ref()
            .is_none_or(|(at, ..)| *at != [grid[0], grid[1]]);
        if volumes_made {
            if let Some((_, volume, scatter)) = kept.volumes.take() {
                for image in [volume, scatter] {
                    super::image::release(gpu, image.image)?;
                }
            }
            kept.volumes = Some((
                [grid[0], grid[1]],
                Texture::volume_shape(gpu, [grid[0], grid[1], 96], Format::Rgba16Float, true)?,
                Texture::volume_shape(gpu, [grid[0], grid[1], 64], Format::Rgba16Float, false)?,
            ));
        }
        // `a.luts != Some(medium)`, and `ProceduralKey` changing.
        let stale = kept.medium != Some(settings.medium);
        kept.medium = Some(settings.medium);
        let integrate = match kept.environment.as_mut().filter(|_| settings.environment) {
            Some(cubes) => cubes.digest.replace(environment_digest) != Some(environment_digest),
            None => false,
        };
        let kept = tables.as_ref().context("made above")?;
        let luts = &kept.luts;
        let cubes = kept.environment.as_ref().filter(|_| settings.environment);
        // What an earlier frame of the sequence left: written, then read by shading.
        if !made {
            for image in [&luts.transmittance, &luts.multiscatter, &luts.sky] {
                graph.import(Resource::Image(image.image), Access::ShaderRead);
            }
        }
        if let Some(cubes) = cubes.filter(|_| !cubes_made) {
            for image in [&cubes.diffuse.0, &cubes.specular.0] {
                graph.import(Resource::Image(image.image), Access::ShaderRead);
            }
        }
        let (_, volume, scatter) = kept.volumes.as_ref().context("made above")?;
        if !volumes_made {
            for image in [volume, scatter] {
                graph.import(Resource::Image(image.image), Access::ShaderRead);
            }
        }
        // wgpu zero-initialises these; the kernels write only part of each. Once, when
        // each is made.
        for (image, texels) in made
            .then_some((luts.transmittance.image, 256 * 65))
            .into_iter()
            .chain(
                [
                    (volume.image, grid[0] * grid[1] * 96),
                    (scatter.image, grid[0] * grid[1] * 64),
                ]
                .into_iter()
                .filter(|_| volumes_made),
            )
        {
            staging.push(transfer::upload(
                gpu,
                &mut graph,
                &mut rec,
                image,
                &vec![0; texels as usize * 8],
            )?);
        }
        let lut_kernels = AtmospherePasses::new(gpu, cache)?;
        for stage in [Pass::AtmosphereTransmittance, Pass::AtmosphereMultiscatter]
            .into_iter()
            .filter(|_| stale)
            .chain([Pass::AtmosphereSky, Pass::AtmosphereAmbient])
        {
            lut_kernels.record(&mut graph, &mut rec, stage, &atmosphere, luts)?;
        }
        let mut environment = None;
        if let Some(cubes) = cubes {
            let kernels = EnvironmentPasses::new(gpu, cache)?;
            if integrate {
                for level in &cubes.specular.1 {
                    kernels.record(
                        &mut graph,
                        &mut rec,
                        Pass::EnvironmentSpecular,
                        &atmosphere,
                        &luts.sky,
                        level,
                    )?;
                }
                kernels.record(
                    &mut graph,
                    &mut rec,
                    Pass::EnvironmentDiffuse,
                    &atmosphere,
                    &luts.sky,
                    &cubes.diffuse.1[0],
                )?;
            }
            environment = Some([&cubes.diffuse.0, &cubes.specular.0]);
        }
        let volume_kernels = VolumePasses::new(gpu, cache)?;
        volume_kernels.record(
            &mut graph,
            &mut rec,
            Pass::AtmosphereAerial,
            &atmosphere,
            luts,
            scatter,
            volume,
        )?;
        Some(Sky {
            luts,
            volume,
            environment,
        })
    } else {
        None
    };
    // TP2 (`prepare_transmission`): the panes' coloured sun shadow on everything opaque.
    // A sun map over the panes' own footprint holds, in one layer a pane, the light its
    // nearest sheet passes and that sheet's depth; shading reads it reduced to two layers
    // (shaders/transmission_reduce.wgsl). Its record is `TpTransmission`: the map's matrix,
    // the direction toward the sun with the reduced layer count, twice the fit's bias.
    // Without panes or a sun the record stays zero and shading reads full light.
    let transmission_record: Option<Vec<u8>> =
        match scene.lighting.lights.iter().find_map(|l| match l {
            crate::Light::Directional { travel, .. } => Some(travel.normalize()),
            _ => None,
        }) {
            Some(travel) if deferred_path && !scene.transparent.is_empty() => {
                let receivers = scene
                    .transparent
                    .iter()
                    .map(pane_bounds)
                    .collect::<Result<Vec<_>>>()?;
                tp::shadow_fit(travel, &receivers, &[]).map(|fit| {
                    let mut record = crate::mesh::matrix_bytes(fit.matrix).to_vec();
                    record.extend(
                        (-travel)
                            .extend(scene.transparent.len().min(2) as f32)
                            .to_array()
                            .into_iter()
                            .chain([fit.bias * 2.0, 0.0, 0.0, 0.0])
                            .flat_map(f32::to_ne_bytes),
                    );
                    record
                })
            }
            _ => None,
        };
    // What the panes' sun map is made from (`prepare_transmission`'s key): its record,
    // and each pane's geometry, material and transform.
    let transmission_key: Option<[u8; 32]> = transmission_record.as_ref().map(|record| {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(record);
        for pane in scene.transparent {
            h.update((pane.vertices.len() as u64).to_le_bytes());
            h.update((pane.indices.len() as u64).to_le_bytes());
            h.update(pane.material.bytes());
            h.update(bytemuck::cast_slice(&pane.transform.to_cols_array()));
        }
        h.finalize().into()
    });
    // The scene and the fresh visibility's targets and kernels.
    let geometry_started = std::time::Instant::now();
    let traced = match &shadow_frame {
        Some(frame) => {
            ensure!(
                scene.meshes.is_none_or(|m| m.len() == scene.objects.len())
                    && scene
                        .materials
                        .is_none_or(|m| m.len() == scene.objects.len()),
                "mesh and material identities need one entry per object"
            );
            let mut geometry = Vec::new();
            let mut transient = Vec::new();
            // The identities this frame names, each imported or built once.
            let mut named = BTreeSet::new();
            let mut tangents = Vec::new();
            let mut instances = Vec::new();
            for (index, object) in scene.objects.iter().enumerate() {
                // `Mesh::tangent`: one vec4 per vertex, read by traced shading alone.
                tangents.push(match normal_map(index) {
                    Some(map) => {
                        let buffer = StorageBuffer::new(
                            gpu,
                            bytemuck::cast_slice(map.tangents.values.as_slice()),
                        )?;
                        graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                        Some(buffer)
                    }
                    None => None,
                });
                // A skinned object's geometry is this pose's alone.
                let key = scene
                    .meshes
                    .map(|m| m[index])
                    .filter(|_| skin(index).is_none());
                let counts = [object.vertices.len(), object.indices.len()];
                let kept = key.and_then(|key| resident.get(&key));
                let place = match (key, kept) {
                    (Some(key), Some(mesh)) => {
                        ensure!(
                            mesh.counts == counts,
                            "mesh identity {key} named another geometry before"
                        );
                        if named.insert(key) {
                            // Built and read in an earlier frame of the sequence.
                            for buffer in [&mesh.vertices, &mesh.indices] {
                                graph.import(Resource::Buffer(buffer.buffer), Access::ShaderRead);
                            }
                            graph.import(
                                Resource::AccelerationStructure(mesh.blas),
                                Access::AsInput,
                            );
                        }
                        Geometry::Resident(key)
                    }
                    _ => {
                        let mesh = ResidentMesh::build(
                            gpu,
                            &mut graph,
                            &mut rec,
                            object.vertices,
                            object.indices,
                        )?;
                        match key {
                            Some(key) => {
                                named.insert(key);
                                resident.insert(key, mesh);
                                Geometry::Resident(key)
                            }
                            None => {
                                transient.push(mesh);
                                Geometry::Transient(transient.len() - 1)
                            }
                        }
                    }
                };
                // `hardware_shadow_gpu`: one instance per placement, in draw then
                // instance order, named by its place in that order.
                // Every ray sees bit 0x02; shadow rays cull by `SHADOW_CASTER_MASK`, which a
                // placement that casts no shadow lacks.
                for (k, transform) in placed(index).iter().enumerate() {
                    let m = transform.to_cols_array_2d();
                    let caster = if casts(index, k) {
                        crate::shadows::SHADOW_CASTER_MASK as u8
                    } else {
                        0
                    };
                    instances.push(AccelerationInstance {
                        geometry: place.of(resident, &transient).blas,
                        transform: std::array::from_fn(|i| m[i % 4][i / 4]),
                        custom_index: instances.len() as u32,
                        mask: 0x02 | caster,
                    });
                }
                geometry.push(place);
            }
            // An empty scene still traces: against one triangle no ray sees (mask 0),
            // since a TLAS needs an instance. Its misses draw the sky.
            if instances.is_empty() {
                let corner = |position| crate::Vertex {
                    position,
                    normal: [0., 1., 0.],
                    uv: [0., 0.],
                    color: [1.; 4],
                };
                let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 0., 1.]].map(corner);
                transient.push(ResidentMesh::build(
                    gpu,
                    &mut graph,
                    &mut rec,
                    &vertices,
                    &[0, 1, 2],
                )?);
                instances.push(AccelerationInstance {
                    geometry: transient[transient.len() - 1].blas,
                    transform: [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.],
                    custom_index: 0,
                    mask: 0,
                });
            }
            // A mesh no object names any more leaves the GPU.
            let unnamed: Vec<u64> = resident
                .keys()
                .copied()
                .filter(|key| !named.contains(key))
                .collect();
            for key in unnamed {
                if let Some(mesh) = resident.remove(&key) {
                    mesh.retire(gpu)?;
                }
            }
            let tlas = Scene::new(gpu, instances)?;
            let mut uses: Vec<_> = named
                .iter()
                .map(|key| &resident[key])
                .chain(&transient)
                .map(|m| (Resource::AccelerationStructure(m.blas), Access::AsInput))
                .collect();
            uses.push((
                Resource::AccelerationStructure(tlas.acceleration),
                Access::AsBuild,
            ));
            graph.node(&mut rec, &uses)?;
            rec.push(Command::BeginPass("ara::BuildTlas".into()));
            rec.push(Command::BuildAccelerationStructure(tlas.acceleration));
            rec.push(Command::EndPass);
            times.geometry = geometry_started.elapsed();
            let mut bytes = frame.records().to_vec();
            bytes.resize(bytes.len().max(crate::shadows::VIEW_BYTES as usize), 0);
            let records = StorageBuffer::new(gpu, &bytes)?;
            graph.import(Resource::Buffer(records.buffer), Access::HostWrite);
            Some(Traced {
                geometry,
                transient,
                tangents,
                scene: tlas,
                world: Texture::new(gpu, size, Format::Rgba32Float, deferred_path, true)?,
                normal: Texture::new(gpu, size, Format::Rgba32Float, deferred_path, true)?,
                visibility: Texture::new(gpu, size, Format::R32Uint, true, false)?,
                records,
                receivers: Receivers::new(gpu, cache)?,
                kernel: Visibility::new(gpu, cache, Mode::Exact)?,
                program: Mode::Exact.program()?,
            })
        }
        None => None,
    };

    // The fog passes' packet records, retired with the frame.
    let mut fog_records = Vec::new();
    // `encode_atmosphere`'s volumetric fog, after the scene its sun rays trace: raw cells,
    // then the resolved cells against the other frame's, then the camera volume's fog
    // slices from them.
    if let (true, Some(sky), Some(traced)) = (fogged, &sky, &traced) {
        let made = kept_fog.as_ref().is_none_or(|(at, _)| *at != grid);
        if made {
            if let Some((_, cells)) = kept_fog.take() {
                for texture in cells {
                    gpu.retire(Resource::Image(texture.image))?;
                }
            }
            let mut cell =
                || Texture::volume_shape(gpu, [grid[0], grid[1], 64], Format::Rgba16Float, true);
            let cells = [cell()?, cell()?, cell()?];
            // wgpu zero-initialises them; the first frame's history is read regardless.
            for texture in &cells {
                staging.push(transfer::upload(
                    gpu,
                    &mut graph,
                    &mut rec,
                    texture.image,
                    &vec![0; (grid[0] * grid[1] * 64 * 8) as usize],
                )?);
            }
            *kept_fog = Some((grid, cells));
        }
        let (_, cells) = kept_fog.as_ref().context("made above")?;
        if !made {
            for texture in cells {
                graph.import(Resource::Image(texture.image), Access::ShaderRead);
            }
        }
        let parity = (history.fog_frame % 2) as usize;
        let (raw, resolved, previous) = (&cells[0], &cells[1 + parity], &cells[2 - parity]);
        let extent = [grid[0], grid[1], 64];
        for (pass, images) in [
            (Pass::AtmosphereFogInject, vec![(15, raw)]),
            (
                Pass::AtmosphereFogTemporal,
                vec![(5, previous), (6, raw), (15, resolved)],
            ),
        ] {
            let kernel = Kernel::new(gpu, cache, pass)?;
            let values = fog_packet(pass, &traced.scene, &atmosphere.record, &images)?;
            let bindings = HeapBindings::new(gpu, pass, &values)?;
            bindings.import(&mut graph);
            kernel.record(&mut graph, &mut rec, &bindings, extent)?;
            fog_records.push(bindings.release());
        }
        let (_, volume, _) = tables
            .as_ref()
            .and_then(|t| t.volumes.as_ref())
            .context("the camera volume")?;
        VolumePasses::new(gpu, cache)?.record(
            &mut graph,
            &mut rec,
            Pass::AtmosphereFogIntegrate,
            &atmosphere,
            sky.luts,
            resolved,
            volume,
        )?;
    }

    // Every other reflected input, of any program this frame runs, is a feature that is
    // off. The wgpu renderer binds zero-filled buffers and zero-initialised textures there.
    // `Renderer::lit_clear`: the uniform environment, exposed, in f64. The sky
    // backdrop replaces it on every pixel when the atmosphere draws one.
    let clear = packed
        .environment
        .map(|v| (f64::from(v) * f64::from(packed.exposure)).min(65504.0) as f32);
    // `Renderer::prepare_primary`: the frame and region records, the placement table,
    // the placement transforms and the targets of a traced frame.
    let mut gi_frame = None;
    // The reconstruction's frame record and the copy of the frame's shaded samples.
    let mut tr1_records = None;
    let deferred = if deferred_path {
        // `encode_reconstruction_interactive` or `encode_primary_interactive`: the
        // rotated kernels where reconstruction rotates directions, else production's.
        let rotated = interactive && reconstruct.is_some_and(|r| r.settings.rotate_directions);
        let (trace_program, fresh_program) = if interactive {
            (
                primary_trace::Interactive::Deferred.program(rotated)?,
                primary_trace::Interactive::Repair.program(rotated)?,
            )
        } else {
            (
                primary_trace::Entry::Receivers.program()?,
                fresh_visibility::traced_program()?,
            )
        };
        let matrix = view.matrix(size[0], size[1]);
        let inverse = matrix.inverse();
        ensure!(inverse.is_finite(), "traced camera inverse is nonfinite");
        let rect = crate::renderer::primary_ray_gpu::scene_pixels(&bounds, matrix, size);
        let lo = [rect[0] / 8 * 8, rect[1] / 4 * 4];
        let region = [
            lo[0],
            lo[1],
            lo[0] + (rect[2] - lo[0]).div_ceil(8) * 8,
            lo[1] + (rect[3] - lo[1]).div_ceil(4) * 4,
        ];
        let placements =
            crate::primary_ray::placement_table(scene.objects.iter().enumerate().map(|(i, o)| {
                (
                    placed(i).len(),
                    normal_map(i).is_some(),
                    !o.material.double_sided,
                    o.material.thin,
                )
            }))?;
        // `PrimaryFrame`: 144 bytes, then TR1's restart zones (zero unless reconstructed).
        let mut frame = [0u8; 144 + tr::ZONES_BYTES];
        frame[..64].copy_from_slice(&crate::mesh::matrix_bytes(inverse));
        frame[64..80].copy_from_slice(bytemuck::cast_slice(&[clear[0], clear[1], clear[2], 1.]));
        let sided = placements.iter().any(|p| p & (1 << 30) != 0);
        frame[80..84].copy_from_slice(&u32::from(sided).to_ne_bytes());
        // Visible sky: misses read the map at the radiance scale whose bits this word
        // holds (0: off; `primary_miss`).
        let visible = scene
            .sky_map
            .and_then(|m| m.visible)
            .map_or(0, f32::to_bits);
        frame[92..96].copy_from_slice(&visible.to_ne_bytes());
        frame[96..112].copy_from_slice(bytemuck::cast_slice(&rect));
        frame[112..116].copy_from_slice(&8u32.to_ne_bytes());
        frame[116..120].copy_from_slice(&8u32.to_ne_bytes());
        frame[124..128].copy_from_slice(&1f32.to_bits().to_ne_bytes());
        for (dst, value) in frame[128..144]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(reflections.frame_words())
        {
            *dst = value.to_ne_bytes();
        }
        let (matrices, normal_bytes) = super::mesh::pack_models(&left.transforms)?;
        // `InstanceState::previous_models`: last frame's matrices when it drew the
        // same placements, otherwise the current ones.
        let previous = if history.transforms.len() == left.transforms.len() {
            super::mesh::pack_models(&history.transforms)?.0
        } else {
            matrices.clone()
        };
        // `prepare_reconstruction`, through the renderer's own planner.
        if let (Some(r), Some(k), Some(images)) = (reconstruct, sample, tr1.as_mut()) {
            staging.extend(images.begin(gpu, &mut graph, &mut rec)?);
            let bits = |m: &crate::Mat4| m.to_cols_array().map(f32::to_bits);
            let rigid_motion = history.transforms.len() == left.transforms.len()
                && history
                    .transforms
                    .iter()
                    .zip(&left.transforms)
                    .any(|(a, b)| bits(a) != bits(b));
            let scene_bounds = bounds
                .iter()
                .copied()
                .reduce(|a, b| crate::shadow_plan::Bounds {
                    min: a.min.min(b.min),
                    max: a.max.max(b.max),
                });
            // The sun's request, as the shadow frame above gives it to the planner.
            let mut first = scene.sun_shadows.is_some();
            let requests: Vec<_> = scene
                .lighting
                .lights
                .iter()
                .map(|l| match (l, scene.sun_shadows) {
                    (crate::Light::Directional { .. }, Some(sun)) if std::mem::take(&mut first) => {
                        crate::ShadowRequest::Directional {
                            split_ends: sun.split_ends.clone(),
                        }
                    }
                    _ => crate::ShadowRequest::None,
                })
                .collect();
            // `reconstruction_key`: what restarts history when it changes, and nothing
            // the planner follows instead. As there: a sun enters by its lux alone (its
            // turn caps the window), a local light without its candela (a flicker
            // continues under the box), the environment by its uniform and whether the
            // sky shows, and the scene by its draws and materials. The atmosphere's
            // settings stand for `set_atmosphere`'s cut. What the procedural sky's
            // content is keyed on (the eye's altitude among it) is not here: it relights
            // the cache and leaves history alone, and its bits change under a camera that
            // only translates. Reflection and cache settings do not restart either.
            let key: [u8; 32] = {
                use sha2::Digest;
                let lights: Vec<_> = scene
                    .lighting
                    .lights
                    .iter()
                    .map(|light| match *light {
                        crate::Light::Directional { lux, .. } => format!("directional {lux:?}"),
                        local => format!("{:?}", tr::without_candela(local)),
                    })
                    .collect();
                let mut h = sha2::Sha256::new();
                h.update(format!("{:?}{:?}{lights:?}", r.settings, scene.sun_shadows).as_bytes());
                h.update(format!("{:?}{:?}", scene.primary, settings).as_bytes());
                for object in scene.objects {
                    h.update((object.vertices.len() as u64).to_le_bytes());
                    h.update((object.indices.len() as u64).to_le_bytes());
                    h.update(format!("{:?}", object.material.base_color).as_bytes());
                    h.update(
                        [object.material.metallic, object.material.roughness]
                            .map(f32::to_bits)
                            .map(u32::to_le_bytes)
                            .concat(),
                    );
                }
                h.update(
                    scene
                        .lighting
                        .environment
                        .map(f32::to_bits)
                        .map(u32::to_le_bytes)
                        .concat(),
                );
                h.update(&uniform[32..48]);
                // The program, and each object's parameter values (`state.program` and
                // the materials' uploads in the renderer's key).
                for source in &sources {
                    h.update(source.as_bytes());
                }
                h.update(bytemuck::cast_slice(&slots));
                // `prepare_transmission` cuts when the panes' sun map would change.
                if let Some(key) = transmission_key {
                    h.update(key);
                }
                for index in 0..scene.objects.len() {
                    if let Some(program) = object_program(index) {
                        h.update(format!("{:?}", program.parameters).as_bytes());
                    }
                }
                h.finalize().into()
            };
            let planned = images.plan(tr::PlanInputs {
                settings: r.settings,
                budget: crate::ReconstructionShadowBudget::default(),
                // One sequence draws one view: its frames are consecutive.
                previous_frame: history.count,
                frame: history.count + 1,
                index: k,
                key,
                lights: &scene.lighting.lights,
                requests: &requests,
                directional_disk_angle: scene.sun_shadows.map_or(0., |s| s.disk_angle),
                matrix: scene.camera.unjittered().matrix(size[0], size[1]),
                eye,
                jitter: view.jitter(),
                exposure: packed.exposure,
                background: clear,
                // AT1's sky, or a visible sky map: misses vary per pixel.
                visible_sky: settings.sky
                    || scene
                        .sky_map
                        .and_then(|m| m.visible)
                        .is_some_and(|v| v != 0.),
                pixel_angle: 2.0 * (0.5 * scene.camera.fov_y()).tan() / target[1] as f32,
                scene_bounds,
                skin_bounds: skin_bounds.iter().copied().reduce(|a, b| {
                    crate::shadow_plan::Bounds {
                        min: a.min.min(b.min),
                        max: a.max.max(b.max),
                    }
                }),
                skin_revisions: skin_revisions.clone(),
                rigid_motion,
                moving_bounds: None,
                listing: false,
            });
            // The rotation word (zero with `rotate_directions` off), then the sampling
            // words the planner budgets.
            let rotation = if r.settings.rotate_directions {
                tr::rotation(k)
            } else {
                0
            };
            frame[84..88].copy_from_slice(&rotation.to_ne_bytes());
            frame[112..128].copy_from_slice(&planned.sampling);
            frame[144..].copy_from_slice(planned.zones());
            let record = StorageBuffer::new(gpu, &planned.frame)?;
            graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
            let samples = Texture::new(gpu, size, Format::Rgba16Float, false, false)?;
            tr1_records = Some((record, samples));
        }
        // `prepare_gi`, after `prepare_primary`: allocate on the first frame that asks.
        if let Some(indirect) = indirect {
            ensure!(
                gi.as_ref()
                    .is_none_or(|c| c.settings() == indirect.settings),
                "indirect settings changed within a sequence, which is not assembled yet"
            );
            // The draws' lasting names, where the scene gives both halves.
            let draw_names: Option<Vec<[u64; 2]>> =
                scene
                    .meshes
                    .zip(scene.materials)
                    .map(|(meshes, materials)| {
                        meshes
                            .iter()
                            .zip(materials)
                            .map(|(a, b)| [*a, *b])
                            .collect()
                    });
            // The frame's programs shade the cells; a kept cache keeps its table.
            match gi.as_mut() {
                Some(kept) => kept.shade_with(gpu, cache, &sources)?,
                None => {
                    *gi = Some(Cache::new(
                        gpu,
                        cache,
                        indirect.settings,
                        packed.count,
                        &sources,
                    )?)
                }
            }
            gi_frame = Some(
                gi.as_mut().context("allocated above")?.prepare(
                    gpu,
                    &mut graph,
                    &mut rec,
                    CacheScene {
                        camera: view,
                        size,
                        bounds: bounds
                            .iter()
                            .map(|b| [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z])
                            .collect(),
                        draws: (0..scene.objects.len())
                            .map(|i| [placed(i).len() as u32, slots[i]])
                            .collect(),
                        names: draw_names.clone(),
                        region: rect,
                        models: &matrices,
                        normals: &normal_bytes,
                        environment: environment_digest,
                        // The procedural environment or a sky map.
                        field: field || map.is_some(),
                        poses: poses.clone(),
                        skin_revisions: skin_revisions
                            .iter()
                            .flat_map(|r| r.to_le_bytes())
                            .collect(),
                        lights: packed.count,
                        // `prepare_gi`'s lighting key: the lights, the panes' sun map,
                        // the environment, the programs, and the draws by program and
                        // by name (the renderer's ids). There are no decals.
                        light_key: {
                            use sha2::Digest;
                            let mut h = sha2::Sha256::new();
                            h.update(format!("{:?}", scene.lighting.lights).as_bytes());
                            h.update(transmission_key.unwrap_or_default());
                            h.update(environment_digest);
                            for source in &sources {
                                h.update(source.as_bytes());
                            }
                            h.update(bytemuck::cast_slice(&slots));
                            if let Some(names) = &draw_names {
                                h.update(bytemuck::cast_slice(names));
                            }
                            h.finalize().into()
                        },
                    },
                )?,
            );
        }
        let frame = StorageBuffer::new(gpu, &frame)?;
        let region_buffer = StorageBuffer::new(gpu, bytemuck::cast_slice(&region))?;
        // The placement words, then each draw's batch word (triangle count, no adjacency
        // suffix, whether its previous-vertex stream is skinned), as `prepare_primary`
        // uploads them.
        let batches: Vec<u32> = scene
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| crate::primary_ray::batch_word(o.indices.len() as u64, skin(i).is_some()))
            .collect();
        let placement_buffer = StorageBuffer::new(
            gpu,
            &per_placement(bytemuck::cast_slice(&crate::primary_ray::placement_words(
                &placements,
                &batches,
            ))),
        )?;
        let models = StorageBuffer::new(
            gpu,
            &per_placement(&[matrices.as_slice(), &previous].concat()),
        )?;
        let normals = StorageBuffer::new(gpu, &per_placement(&normal_bytes))?;
        let absent = StorageBuffer::new(gpu, &[0; 48])?;
        // `repair_bytes`: the queue's header, then room for every pixel. The sequence
        // keeps it; each frame clears its header before the trace, as the renderer
        // does, and what lies past the header's count is never read.
        let repairs = if interactive {
            let bytes = 16 + u64::from(size[0]) * u64::from(size[1]) * 36;
            let queue = match kept_repairs.take() {
                Some(queue) if queue.len == bytes => {
                    graph.import(Resource::Buffer(queue.buffer), Access::Storage);
                    queue
                }
                stale => {
                    if let Some(stale) = stale {
                        stale.retire(gpu)?;
                    }
                    StorageBuffer::device_with_usage(
                        gpu,
                        bytes,
                        ara_gpu::vk::BufferUsageFlags::INDIRECT_BUFFER,
                    )?
                }
            };
            super::gi_cache::fill(&mut graph, &mut rec, &queue, 0, 16)?;
            Some(queue)
        } else {
            None
        };
        for buffer in [
            &frame,
            &region_buffer,
            &placement_buffer,
            &models,
            &normals,
            &absent,
        ] {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
        }
        let trace_groups = [size[0].div_ceil(8), size[1].div_ceil(4)];
        Some(Deferred {
            trace: Kernel::from_program(gpu, cache, &trace_program)?,
            fresh: Kernel::from_program(gpu, cache, &fresh_program)?,
            shade: Kernel::from_program(gpu, cache, &program)?,
            trace_program,
            fresh_program,
            hit: Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
            scene: rect,
            region_rect: region,
            material: if material_on {
                Some([
                    Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
                    Texture::new(gpu, size, Format::Rgba32Uint, true, false)?,
                ])
            } else {
                None
            },
            tiles: Texture::new(gpu, trace_groups, Format::R32Uint, true, false)?,
            epsilon: Texture::new(gpu, size, Format::R32Float, true, false)?,
            frame,
            region: region_buffer,
            placements: placement_buffer,
            models,
            normals,
            absent,
            repairs,
            groups: [
                trace_groups,
                [size[0].div_ceil(2), size[1].div_ceil(2)],
                [
                    (region[2] - region[0])
                        .div_ceil(8)
                        .max(size[0].div_ceil(16)),
                    (region[3] - region[1]).div_ceil(4) + size[1].div_ceil(16),
                ],
            ],
        })
    } else {
        None
    };

    // The cache as this frame's passes and shading see it.
    let gi = gi.as_ref().zip(gi_frame.as_ref());
    let mut specs = program.inputs().to_vec();
    // A list program reads what its own source reads; its record and the lists are the
    // frame's own.
    for list in list_programs
        .iter()
        .chain(shade_parts.iter().flat_map(|(_, programs)| programs))
    {
        specs.extend(
            list.inputs()
                .iter()
                .filter(|spec| !matches!((spec.key.group, spec.key.binding), (0, 38 | 39)))
                .cloned(),
        );
    }
    if let Some((cache, _)) = gi {
        for stage in cache.programs().chain(cache.shade_programs()) {
            specs.extend(stage.inputs().iter().cloned());
        }
    }
    // The history as this frame's resolve sees it; `tr1` itself advances at the end.
    let resolve = tr1.as_ref().zip(tr1_records.as_ref());
    if let Some((images, _)) = resolve {
        specs.extend(images.program().inputs().iter().cloned());
    }
    if let Some(traced) = &traced {
        specs.extend(shader::inputs(Pass::ReceiverRaster)?);
        specs.extend(traced.program.inputs().iter().cloned());
    }
    if let Some(deferred) = &deferred {
        specs.extend(deferred.trace_program.inputs().iter().cloned());
        specs.extend(deferred.fresh_program.inputs().iter().cloned());
    }
    let mut buffers = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    // The transmission record and reduced map are what every program reads at (3, 26) and
    // (3, 27); each pane's geometry and material are drawn into its own layer below, and
    // the layers are reduced to two.
    const TRANSMISSION_SIZE: [u32; 2] = [1024; 2];
    let mut transmission = None;
    if let Some(record) = &transmission_record {
        let record = StorageBuffer::new(gpu, record)?;
        graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
        buffers.insert(key(3, 26), record);
        inputs.insert(
            key(3, 27),
            Texture::array(gpu, TRANSMISSION_SIZE, 2, Format::Rgba16Float, true, false)?,
        );
        let layers = Texture::array(
            gpu,
            TRANSMISSION_SIZE,
            scene.transparent.len() as u32,
            Format::Rgba16Float,
            false,
            true,
        )?;
        let mut panes = Vec::new();
        for object in scene.transparent {
            let geometry = MeshGeometry::new(
                gpu,
                object.vertices,
                object.indices,
                &[0],
                &[object.transform],
            )?;
            geometry.import(&mut graph);
            let material = StorageBuffer::new(gpu, &object.material.bytes())?;
            graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
            panes.push((geometry, material));
        }
        // The material's base colour texture: the renderer's white default.
        let base = Texture::new(gpu, [1, 1], Format::Rgba8Srgb, false, false)?;
        staging.push(transfer::upload(
            gpu, &mut graph, &mut rec, base.image, &[255; 4],
        )?);
        let (depth, depth_views) = Texture::depth_atlas(gpu, TRANSMISSION_SIZE, 1, 1)?;
        transmission = Some((
            Transmission::new(gpu, cache)?,
            panes,
            base,
            depth,
            depth_views,
            layers,
        ));
    }
    for spec in &specs {
        match spec.kind {
            InputKind::Buffer { minimum, .. } => {
                if [
                    key(0, 0),
                    key(1, 0),
                    key(1, 1),
                    key(2, 0),
                    key(3, 0),
                    key(3, 1),
                    key(3, 23),
                    key(3, 50),
                ]
                .contains(&spec.key)
                    || (spec.key == key(3, 5) && traced.is_some())
                    || (deferred.is_some()
                        && [key(0, 10), key(0, 22), key(0, 24), key(0, 26), key(1, 5)]
                            .contains(&spec.key))
                    || (gi.is_some()
                        && (spec.key == key(3, 20)
                            || spec.key == key(3, 28)
                            || (spec.key.group == 0 && (30..=37).contains(&spec.key.binding))))
                    || (resolve.is_some() && spec.key == key(0, 40))
                    || buffers.contains_key(&spec.key)
                {
                    continue;
                }
                let buffer = StorageBuffer::new(gpu, &vec![0; (minimum as usize).max(64)])?;
                graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                buffers.insert(spec.key, buffer);
            }
            InputKind::Image { dimension, sample }
            | InputKind::ImageArray {
                dimension, sample, ..
            } => {
                if spec.key == key(3, 24)
                    || (resolve.is_some()
                        && spec.key.group == 0
                        && (41..=47).contains(&spec.key.binding))
                    || provided(
                        sky.as_ref(),
                        map,
                        traced.as_ref(),
                        deferred.as_ref(),
                        spec.key,
                    )
                    .is_some()
                    || inputs.contains_key(&spec.key)
                {
                    continue;
                }
                let (format, texel) = match sample {
                    SampleType::Float => (Format::Rgba16Float, 8),
                    SampleType::Uint => (Format::R32Uint, 4),
                    SampleType::Depth => (Format::Depth32Float, 4),
                    // Only the pane shading reads one, and it binds its own.
                    SampleType::UintMultisampled => {
                        bail!("no frame default for the multisampled image {}", spec.name)
                    }
                };
                // The defaults `Renderer::new_configured` creates, and the prepared DFG.
                let (format, extent, bytes) = match (spec.key.group, spec.key.binding) {
                    (2, 1 | 11) => (Format::Rgba8Srgb, [1, 1, 1], vec![255; 4]),
                    (2, 3 | 13) => (Format::Rgba8, [1, 1, 1], vec![255; 4]),
                    (2, 5 | 15) => (Format::Rgba8, [1, 1, 1], vec![128, 128, 255, 255]),
                    (3, 2) => (
                        Format::Rg16Float,
                        [crate::lighting::DFG_SIZE, crate::lighting::DFG_SIZE, 1],
                        crate::lighting::DFG.to_vec(),
                    ),
                    // An absent environment: one black cube, as the renderer's fallback.
                    _ if dimension == ViewDimension::CubeArray => {
                        (format, [1, 1, 6], vec![0; texel * 6])
                    }
                    _ => {
                        let layers = if dimension == ViewDimension::D2 { 1 } else { 6 };
                        (format, [1, 1, layers], vec![0; texel * layers as usize])
                    }
                };
                let flat = [extent[0], extent[1]];
                let image = match dimension {
                    ViewDimension::D2 => Texture::new(gpu, flat, format, false, false)?,
                    ViewDimension::D2Array => Texture::array(
                        gpu,
                        flat,
                        extent[2],
                        format,
                        false,
                        sample == SampleType::Depth,
                    )?,
                    ViewDimension::D3 => Texture::volume_shape(gpu, extent, format, false)?,
                    ViewDimension::CubeArray => Texture::cubes(gpu, 1, 1, format)?,
                };
                staging.push(transfer::upload(
                    gpu,
                    &mut graph,
                    &mut rec,
                    image.image,
                    &bytes,
                )?);
                inputs.insert(spec.key, image);
            }
            InputKind::Sampler { comparison } => {
                ensure!(!comparison, "unexpected comparison sampler {}", spec.name)
            }
            // Deferred reflections' inputs (`program::deferred_variant`): the shading entry
            // writes them per pixel; the composite after shading reads them.
            InputKind::StorageImage { .. }
                if traced.is_some()
                    && matches!((spec.key.group, spec.key.binding), (0, 62) | (0, 63)) =>
            {
                if let std::collections::btree_map::Entry::Vacant(slot) = inputs.entry(spec.key) {
                    slot.insert(Texture::new(gpu, size, Format::Rgba16Float, true, false)?);
                }
            }
            InputKind::AccelerationStructure | InputKind::StorageImage { .. }
                if traced.is_some() => {}
            InputKind::BufferArray { .. } | InputKind::SamplerArray { .. }
                if deferred.is_some() => {}
            _ => bail!("forward frame cannot supply {}", spec.name),
        }
    }

    let mut geometries = Vec::new();
    let mut materials = Vec::new();
    let mut parameter_blocks = Vec::new();
    for (index, object) in scene.objects.iter().enumerate() {
        if !deferred_path {
            let geometry = MeshGeometry::new(
                gpu,
                object.vertices,
                object.indices,
                &[0],
                &[object.transform],
            )?;
            geometry.import(&mut graph);
            geometries.push(geometry);
        }
        // `MaterialDescriptor::bytes` reads `normal_mapped` and the variance code off its
        // own texture binding, which names a wgpu texture and stays empty here. As there:
        // 1 is a normal map; 2 a BC5 one, whose Z the programs rebuild
        // (`rl_normal_texel`) and whose variance is its second layer (variance code 1).
        let mut record = object.material.bytes();
        let bc5 = normal_map(index).is_some_and(|map| {
            matches!(map.texture.data, TextureSource::Blocks(blocks)
                if blocks.format == crate::asset::BlockFormat::Bc5)
        });
        let normal_mapped = match normal_map(index) {
            None => 0u32,
            Some(_) if bc5 => 2,
            Some(_) => 1,
        };
        record[36..40].copy_from_slice(&normal_mapped.to_ne_bytes());
        if bc5 {
            record[40..44].copy_from_slice(&1u32.to_ne_bytes());
        }
        let material = StorageBuffer::new(gpu, &record)?;
        graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
        materials.push(material);
        // `MaterialParameters`: the program's defaults with this object's named values.
        // Packed against the draw's own program; a program without `Parameters` gets a
        // zero block when another program of the frame declares some.
        if parameter_layouts.iter().any(Option::is_some) {
            let values: Vec<(String, crate::Value)> = object_program(index)
                .map_or(&[][..], |p| p.parameters)
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect();
            let block = crate::program::ParameterLayout::pack(
                parameter_layouts[slots[index] as usize].as_ref(),
                &values,
            );
            let block = StorageBuffer::new(gpu, &block)?;
            graph.import(Resource::Buffer(block.buffer), Access::HostWrite);
            parameter_blocks.push(block);
        }
    }
    // `encode_transmission`: each pane into its own layer, cleared to full light.
    let mut transmission_packets = Vec::new();
    if let Some((pass, panes, base, _, depth_views, map)) = &transmission {
        for (layer, (geometry, material)) in panes.iter().enumerate() {
            let packet = HeapBindings::new(
                gpu,
                Pass::TransmissionShadow,
                &[
                    (key(0, 30), Input::Buffer(&buffers[&key(3, 26)])),
                    (key(1, 0), Input::Buffer(geometry.models())),
                    (key(2, 0), Input::Buffer(material)),
                    (key(2, 1), Input::Image(base)),
                    (key(2, 2), Input::Sampler(SamplerDesc::default())),
                ],
            )?;
            packet.import(&mut graph);
            pass.record_layer(
                &mut graph,
                &mut rec,
                &ShadowDraw {
                    geometry,
                    bindings: &packet,
                    first_instance: 0,
                    instance_count: 1,
                },
                &map.view_2d(gpu, 0, layer as u32)?,
                &depth_views[0],
            )?;
            transmission_packets.push(packet);
        }
        // The layers reduced to two per texel, which shading reads (audit 3.5).
        let reduce = HeapBindings::new(
            gpu,
            Pass::TransmissionReduce,
            &[
                (key(0, 0), Input::Image(map)),
                (key(0, 1), Input::Image(&inputs[&key(3, 27)])),
            ],
        )?;
        reduce.import(&mut graph);
        Kernel::new(gpu, cache, Pass::TransmissionReduce)?.record(
            &mut graph,
            &mut rec,
            &reduce,
            [TRANSMISSION_SIZE[0], TRANSMISSION_SIZE[1], 1],
        )?;
        transmission_packets.push(reduce);
    }
    // `try_upload_texture` and `sampler_descriptor` for each object's texture slots. A
    // texture with an identity is uploaded by the first frame that names it and kept;
    // one without is this frame's alone (`transient_textures`).
    let mut object_textures: Vec<[Option<(Texture, SamplerDesc)>; 3]> = Vec::new();
    let mut transient_textures = Vec::new();
    let mut named_textures = std::collections::BTreeSet::new();
    if let Some(textures) = scene.textures {
        ensure!(
            textures.objects.len() == scene.objects.len()
                && (1..=16).contains(&textures.anisotropy_clamp),
            "scene textures need one entry per object and an anisotropy clamp in 1..=16"
        );
        for object in textures.objects {
            let mut slots = [None, None, None];
            // The normal slot is sampled as a one-layer array (`normal_map`).
            for (slot, (texture, role, format, dimension)) in [
                (
                    object.base_color,
                    crate::asset::TextureRole::BaseColorSrgb,
                    Format::Rgba8Srgb,
                    ViewDimension::D2,
                ),
                (
                    object.metallic_roughness,
                    crate::asset::TextureRole::MetallicRoughnessLinear,
                    Format::Rgba8,
                    ViewDimension::D2,
                ),
                (
                    object.normal.map(|n| n.texture),
                    crate::asset::TextureRole::NormalLinear,
                    Format::Rgba8,
                    ViewDimension::D2Array,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let Some(texture) = texture else { continue };
                // What the slot uploads: base size, levels, GPU format and array layers.
                let (extent, levels, format, layers) = match texture.data {
                    TextureSource::Texels(data) => {
                        let first = data.mips.first().context("texture without a level")?;
                        ensure!(
                            data.role == role,
                            "texture slot holds a {:?} texture",
                            data.role
                        );
                        ([first.width, first.height], data.mips.len(), format, 1)
                    }
                    TextureSource::Blocks(blocks) => {
                        use crate::asset::BlockFormat as B;
                        let levels = blocks.layers.first().map_or(0, Vec::len);
                        ensure!(
                            blocks.format.role() == role,
                            "texture slot holds a {:?} texture",
                            blocks.format
                        );
                        ensure!(
                            levels > 0
                                && blocks.layers.iter().all(|layer| layer.len() == levels)
                                && (blocks.layers.len() == 1
                                    || dimension == ViewDimension::D2Array),
                            "a block texture's layers hold the same levels, and only the \
                             normal slot is an array"
                        );
                        ensure!(
                            gpu.info().texture_compression_bc,
                            "the device does not sample block-compressed textures"
                        );
                        let format = match blocks.format {
                            B::Bc7Srgb => Format::Bc7Srgb,
                            B::Bc7Linear => Format::Bc7,
                            B::Bc5 => Format::Bc5,
                        };
                        (blocks.size, levels, format, blocks.layers.len() as u32)
                    }
                };
                let named = texture.key.map(|key| (key, slot));
                let image = match named.and_then(|named| kept_textures.get(&named)) {
                    Some(kept) => {
                        ensure!(
                            kept.size == extent
                                && kept.shape.mip_levels == levels as u32
                                && kept.format == format
                                && kept.shape.layers == layers,
                            "texture identity {:?} named another texture before",
                            texture.key
                        );
                        // Written by an earlier frame, read by shading since.
                        if named_textures.insert(named) {
                            graph.import(Resource::Image(kept.image), Access::ShaderRead);
                        }
                        *kept
                    }
                    None => {
                        let image = Texture::sampled_mips(
                            gpu,
                            extent,
                            levels as u32,
                            layers,
                            format,
                            dimension,
                        )?;
                        match texture.data {
                            TextureSource::Texels(data) => {
                                for (level, mip) in data.mips.iter().enumerate() {
                                    ensure!(
                                        mip.rgba8.len()
                                            == mip.width as usize * mip.height as usize * 4,
                                        "texture level {level} is not tightly packed RGBA8"
                                    );
                                    staging.push(transfer::upload_mip(
                                        gpu,
                                        &mut graph,
                                        &mut rec,
                                        image.image,
                                        level as u32,
                                        0,
                                        [mip.width, mip.height],
                                        &mip.rgba8,
                                    )?);
                                }
                            }
                            // Blocks go up as shipped: each level's true texel extent,
                            // whole 4x4 blocks of bytes (one block under 4x4 texels).
                            TextureSource::Blocks(blocks) => {
                                for (layer, chain) in blocks.layers.iter().enumerate() {
                                    for (level, bytes) in chain.iter().enumerate() {
                                        let size = extent.map(|n| (n >> level).max(1));
                                        ensure!(
                                            bytes.len() as u64
                                                == crate::asset::BlockFormat::level_bytes(
                                                    size[0], size[1]
                                                ),
                                            "texture level {level} is not whole 4x4 blocks"
                                        );
                                        staging.push(transfer::upload_mip(
                                            gpu,
                                            &mut graph,
                                            &mut rec,
                                            image.image,
                                            level as u32,
                                            layer as u32,
                                            size,
                                            bytes,
                                        )?);
                                    }
                                }
                            }
                        }
                        match named {
                            Some(named) => {
                                named_textures.insert(Some(named));
                                kept_textures.insert(named, image);
                            }
                            None => transient_textures.push(image),
                        }
                        image
                    }
                };
                let sampler = sampler_desc(texture.sampler, textures.anisotropy_clamp, levels)?;
                slots[slot] = Some((image, sampler));
            }
            object_textures.push(slots);
        }
    }
    // A texture no object names any more leaves the GPU.
    kept_textures.retain(|named, texture| {
        let keep = named_textures.contains(&Some(*named));
        if !keep {
            transient_textures.push(*texture);
        }
        keep
    });
    let output = Texture::new(gpu, size, Format::Rgba16Float, deferred_path, true)?;
    // `Upscale::hdr`: below native scale the resolve writes a target-size image, which
    // is what post and the caller then see.
    let upscaled = if size == target {
        None
    } else {
        Some(Texture::new(gpu, target, Format::Rgba16Float, true, false)?)
    };
    let shown = upscaled.as_ref().unwrap_or(&output);
    let depth = RasterDepth::new(gpu, size, SampleCount::One)?;
    // Skinned draws' previous vertices, as the mesh's vertex records.
    let previous_buffers: Vec<Option<StorageBuffer>> = (0..scene.objects.len())
        .map(|i| {
            skin(i)
                .map(|k| {
                    let bytes: Vec<u8> = k
                        .previous
                        .iter()
                        .flat_map(|v| {
                            v.position
                                .into_iter()
                                .chain(v.normal)
                                .chain(v.uv)
                                .chain(v.color)
                        })
                        .flat_map(f32::to_le_bytes)
                        .collect();
                    let buffer = StorageBuffer::new(gpu, &bytes)?;
                    graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                    Ok::<_, anyhow::Error>(buffer)
                })
                .transpose()
        })
        .collect::<Result<_>>()?;
    let mut arrays = Arrays::default();
    if let (Some(deferred), Some(traced)) = (&deferred, &traced) {
        let draws = crate::TRACED_DRAWS as usize;
        ensure!(
            materials.len() <= draws,
            "traced frames hold TRACED_DRAWS draws"
        );
        arrays.vertices = (0..draws)
            .map(|i| {
                traced.geometry.get(i).map_or(&deferred.absent, |g| {
                    &g.of(resident, &traced.transient).vertices
                })
            })
            .collect();
        arrays.indices = (0..draws)
            .map(|i| {
                traced.geometry.get(i).map_or(&deferred.absent, |g| {
                    &g.of(resident, &traced.transient).indices
                })
            })
            .collect();
        arrays.absent = vec![&deferred.absent; draws];
        arrays.previous = (0..draws)
            .map(|i| {
                previous_buffers
                    .get(i)
                    .and_then(Option::as_ref)
                    .unwrap_or(arrays.vertices[i])
            })
            .collect();
        arrays.tangents = (0..draws)
            .map(|i| {
                traced
                    .tangents
                    .get(i)
                    .and_then(Option::as_ref)
                    .unwrap_or(&deferred.absent)
            })
            .collect();
        // The renderer pads with its default material; padded slots are never read.
        // An empty frame has no material of its own: the default one pads its slots.
        if materials.is_empty() {
            let material = StorageBuffer::new(gpu, &crate::MaterialDescriptor::default().bytes())?;
            graph.import(Resource::Buffer(material.buffer), Access::HostWrite);
            materials.push(material);
        }
        arrays.materials = (0..draws)
            .map(|i| materials.get(i).unwrap_or(&materials[0]))
            .collect();
        // Parameter blocks likewise, where the program declares `Parameters`.
        arrays.parameters = (0..draws)
            .filter_map(|i| parameter_blocks.get(i).or(parameter_blocks.first()))
            .collect();
        arrays.default_samplers = vec![SamplerDesc::default(); draws];
        // Base colour at 11 and 12, metallic-roughness at 13 and 14, the normal map at 15
        // and 16: each draw's own texture and sampler where it has one.
        for (slot, bindings) in [(0, [11, 12]), (1, [13, 14]), (2, [15, 16])] {
            arrays.samplers.insert(
                key(2, bindings[1]),
                (0..draws)
                    .map(|i| {
                        object_textures
                            .get(i)
                            .and_then(|t| t[slot].as_ref())
                            .map_or(SamplerDesc::default(), |t| t.1)
                    })
                    .collect(),
            );
        }
        for spec in &specs {
            if let InputKind::ImageArray { .. } = spec.kind {
                let slot = match (spec.key.group, spec.key.binding) {
                    (2, 11) => Some(0),
                    (2, 13) => Some(1),
                    (2, 15) => Some(2),
                    _ => None,
                };
                arrays.textures.insert(
                    spec.key,
                    (0..draws)
                        .map(|i| {
                            slot.and_then(|slot| object_textures.get(i)?[slot].as_ref())
                                .map_or(&inputs[&spec.key], |t| &t.0)
                        })
                        .collect(),
                );
            }
        }
    }
    let shared = Shared {
        output: &output,
        deferred: deferred.as_ref(),
        arrays: &arrays,
        camera: &camera,
        lighting: &lighting,
        clusters: &cluster_inputs,
        lists: &lists,
        atmosphere: &atmosphere,
        sky: sky.as_ref(),
        map,
        traced: traced.as_ref(),
        gi_table: gi.map(|(cache, _)| cache.table()),
        gi_outgoing: gi.map(|(cache, _)| cache.outgoing_radiance()),
        buffers: &buffers,
        images: &inputs,
    };
    // One lit packet per object on the raster path (none when traced: no geometries).
    let mut packets = Vec::new();
    for (index, object) in geometries.iter().zip(&materials).enumerate() {
        let values = program
            .inputs()
            .iter()
            .map(|spec| {
                // The object's own base colour (1, 2) and metallic-roughness (3, 4).
                let own = |slot: usize| object_textures.get(index).and_then(|t| t[slot].as_ref());
                Ok((
                    spec.key,
                    match (spec.key.group, spec.key.binding, own(0), own(1)) {
                        (2, 1, Some(t), _) | (2, 3, _, Some(t)) => Input::Image(&t.0),
                        (2, 2, Some(t), _) | (2, 4, _, Some(t)) => Input::Sampler(t.1),
                        _ => shared.input(spec, Some(object))?,
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::for_program(gpu, &program, &values)?;
        packet.import(&mut graph);
        packets.push(packet);
    }
    // The receiver prepass draws every object; the estimator runs once over the frame.
    let mut shadow_packets = Vec::new();
    if let Some(traced) = traced.as_ref().filter(|_| !deferred_path) {
        let layout = shader::inputs(Pass::ReceiverRaster)?;
        for object in geometries.iter().zip(&materials) {
            let values = layout
                .iter()
                .map(|spec| Ok((spec.key, shared.input(spec, Some(object))?)))
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::new(gpu, Pass::ReceiverRaster, &values)?;
            packet.import(&mut graph);
            shadow_packets.push(packet);
        }
        let values = traced
            .program
            .inputs()
            .iter()
            .map(|spec| Ok((spec.key, shared.input(spec, None)?)))
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::for_program(gpu, &traced.program, &values)?;
        packet.import(&mut graph);
        shadow_packets.push(packet);
    }

    // One packet per cache entry: the cache's own buffers, the rest from the frame.
    let mut gi_packets = Vec::new();
    if let Some((cache, records)) = gi {
        for stage in cache.programs() {
            let values = stage
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match cache.input(records, spec.key) {
                            Some(buffer) => Input::Buffer(buffer),
                            None => shared.input(spec, None)?,
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, stage, &values)?;
            packet.import(&mut graph);
            gi_packets.push(packet);
        }
    }
    // Cell shading: the cache's group 0, then the traced shading interface.
    let mut gi_shade_packets = Vec::new();
    if let Some((cache, records)) = gi {
        for stage in cache.shade_programs() {
            let values = stage
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match cache.shade_input(records, spec.key) {
                            Some(buffer) => Input::Buffer(buffer),
                            None => shared.input(spec, None)?,
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, stage, &values)?;
            packet.import(&mut graph);
            gi_shade_packets.push(packet);
        }
    }
    // The resolve: its frame record, the copied samples, the history sides the parity
    // selects and the HDR target it rewrites; the rest from the traced frame.
    // MEASUREMENT ONLY: the one-pixel trace's buffer, zeroed each frame.
    let probe = match resolve.filter(|_| super::resolve_probe::pixel().is_some()) {
        Some(_) => {
            let probe = StorageBuffer::new(gpu, &vec![0; super::resolve_probe::BYTES as usize])?;
            graph.import(Resource::Buffer(probe.buffer), Access::HostWrite);
            Some(probe)
        }
        None => None,
    };
    let resolve_packet = match resolve {
        Some((images, (record, samples))) => {
            let values = images
                .program()
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match (spec.key.group, spec.key.binding) {
                            (0, 40) => Input::Buffer(record),
                            (0, 41) => Input::Image(samples),
                            (0, 46) => Input::Image(shown),
                            (0, super::resolve_probe::BINDING) => {
                                Input::Buffer(probe.as_ref().context("the resolve's probe buffer")?)
                            }
                            _ => match images.input(spec.key) {
                                Some(image) => Input::Image(image),
                                None => shared.input(spec, None)?,
                            },
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, images.program(), &values)?;
            packet.import(&mut graph);
            Some(packet)
        }
        None => None,
    };
    // The traced frame's three packets: none of them belongs to one object.
    let mut deferred_packets = Vec::new();
    if let Some(deferred) = &deferred {
        for stage in [&deferred.trace_program, &deferred.fresh_program, &program] {
            let values = stage
                .inputs()
                .iter()
                .map(|spec| Ok((spec.key, shared.input(spec, None)?)))
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, stage, &values)?;
            packet.import(&mut graph);
            deferred_packets.push(packet);
        }
    }
    // MP1 (`prepare_program_lists`): with several programs, one list per slot.
    let program_lists = match &deferred {
        Some(deferred) if !list_programs.is_empty() => {
            let count = sources.len() as u32;
            // Words of one slot's list: its length, then every 4x4 shading tile of the
            // trace-tile-widened region and every 16x16 clear block.
            let capacity = 1
                + (size[0] + 8).div_ceil(4) * (size[1] + 4).div_ceil(4)
                + size[0].div_ceil(16) * size[1].div_ceil(16);
            let entries = StorageBuffer::device(gpu, u64::from(count) * u64::from(capacity) * 4)?;
            // Device-local, as the renderer's: binning clears it every frame before its
            // atomics. Host-visible, it lay in system memory on a discrete NVIDIA card,
            // and every 4x4 tile's appends crossed the bus.
            let arguments = StorageBuffer::device_with_usage(
                gpu,
                u64::from(count) * 16,
                ara_gpu::vk::BufferUsageFlags::INDIRECT_BUFFER,
            )?;
            let mut built = Vec::new();
            for (slot, program) in list_programs.into_iter().enumerate() {
                // `RlList`: slot, list base, capacity, flags, then each batch's slot.
                let mut words = vec![slot as u32, slot as u32 * capacity, capacity, 0];
                words.extend(
                    (0..crate::TRACED_DRAWS as usize).map(|b| slots.get(b).copied().unwrap_or(0)),
                );
                let info = StorageBuffer::new(gpu, bytemuck::cast_slice(&words))?;
                graph.import(Resource::Buffer(info.buffer), Access::HostWrite);
                let kernel = Kernel::from_program(gpu, cache, &program)?;
                built.push((program, kernel, info));
            }
            let r = deferred.region_rect;
            Some(ProgramLists {
                bins: ProgramBins::new(gpu, cache)?,
                slots: built,
                entries,
                arguments,
                dispatch: BinDispatch {
                    slots: count,
                    capacity,
                    groups: [
                        (r[2] - r[0]).div_ceil(4).max(size[0].div_ceil(16)),
                        (r[3] - r[1]).div_ceil(4) + size[1].div_ceil(16),
                        1,
                    ],
                },
            })
        }
        _ => None,
    };
    let mut list_packets = Vec::new();
    if let (Some(lists), Some(deferred)) = (&program_lists, &deferred) {
        // The binning packet, then one shading packet per slot: the shading inputs with
        // that slot's record and the lists.
        let values = shader::inputs(Pass::ProgramBins)?
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match (spec.key.group, spec.key.binding) {
                        (0, 0) => Input::Image(&deferred.hit),
                        (0, 1) => Input::Image(&deferred.tiles),
                        (0, 2) => Input::Buffer(&deferred.region),
                        (0, 3) => Input::Buffer(&deferred.placements),
                        (0, 4) => Input::Buffer(&lists.slots[0].2),
                        (0, 5) => Input::Buffer(&lists.entries),
                        (0, 6) => Input::Buffer(&lists.arguments),
                        _ => bail!("unexpected binning input {}", spec.name),
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::new(gpu, Pass::ProgramBins, &values)?;
        packet.import(&mut graph);
        list_packets.push(packet);
        for (program, _, info) in &lists.slots {
            let values = program
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match (spec.key.group, spec.key.binding) {
                            (0, 38) => Input::Buffer(info),
                            (0, 39) => Input::Buffer(&lists.entries),
                            _ => shared.input(spec, None)?,
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, program, &values)?;
            packet.import(&mut graph);
            list_packets.push(packet);
        }
    }
    // Each shading part's kernels and packets, bound as the frame's own shading binds them.
    let mut part_records = Vec::new();
    for (name, programs) in shade_parts.iter() {
        let mut slots = Vec::new();
        for (slot, program) in programs.iter().enumerate() {
            let values = program
                .inputs()
                .iter()
                .map(|spec| {
                    Ok((
                        spec.key,
                        match (&program_lists, spec.key.group, spec.key.binding) {
                            (Some(lists), 0, 38) => Input::Buffer(&lists.slots[slot].2),
                            (Some(lists), 0, 39) => Input::Buffer(&lists.entries),
                            _ => shared.input(spec, None)?,
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, program, &values)?;
            packet.import(&mut graph);
            slots.push((Kernel::from_program(gpu, cache, program)?, packet));
        }
        part_records.push((*name, slots));
    }
    // RENDERLAB_MATERIAL_PASS: per slot, the material pass's kernel and packet (bound as that
    // slot's shading is), or None where the program keeps its single kernel.
    let mut material_records = Vec::new();
    for (slot, program) in material_programs.iter().enumerate() {
        let Some(program) = program else {
            material_records.push(None);
            continue;
        };
        let values = program
            .inputs()
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match (&program_lists, spec.key.group, spec.key.binding) {
                        (Some(lists), 0, 38) => Input::Buffer(&lists.slots[slot].2),
                        (Some(lists), 0, 39) => Input::Buffer(&lists.entries),
                        _ => shared.input(spec, None)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::for_program(gpu, program, &values)?;
        packet.import(&mut graph);
        material_records.push(Some((Kernel::from_program(gpu, cache, program)?, packet)));
    }
    if let Some(deferred) = &deferred {
        // `encode_primary_visibility`, `encode_primary_fresh`, the cache's passes
        // (`GiState::encode`), then `encode_primary_shading`.
        let threads = [
            deferred.trace_program.threads()?,
            deferred.fresh_program.threads()?,
            program.threads()?,
        ];
        for (((kernel, packet), groups), threads) in
            [&deferred.trace, &deferred.fresh, &deferred.shade]
                .into_iter()
                .zip(&deferred_packets)
                .zip(deferred.groups)
                .zip(threads)
        {
            if let (Some((cache, records)), true) = (gi, std::ptr::eq(kernel, &deferred.shade)) {
                cache.record(
                    &mut graph,
                    &mut rec,
                    records,
                    &gi_packets,
                    &gi_shade_packets,
                )?;
            }
            // The pass log's shading parts replay the shading first; the frame's own
            // shading below rewrites every pixel (src/shade_detail.rs).
            if std::ptr::eq(kernel, &deferred.shade) {
                for (name, slots) in &part_records {
                    match &program_lists {
                        Some(lists) => {
                            lists.bins.record_labelled(
                                &mut graph,
                                &mut rec,
                                &list_packets[0],
                                &lists.dispatch,
                                name.to_string(),
                            )?;
                            for (slot, (part, packet)) in slots.iter().enumerate() {
                                part.record_indirect_labelled(
                                    &mut graph,
                                    &mut rec,
                                    packet,
                                    &lists.arguments,
                                    slot as u64 * 16,
                                    name.to_string(),
                                )?;
                            }
                        }
                        None => slots[0].0.record_labelled(
                            &mut graph,
                            &mut rec,
                            &slots[0].1,
                            [groups[0] * threads[0], groups[1] * threads[1], 1],
                            name.to_string(),
                        )?,
                    }
                }
            }
            // The repair runs over the queue the trace filled, from its own counts.
            // `encode_program_lists`: binning, then each program's list from the counts
            // the binning pass left.
            if let (Some(lists), true) = (&program_lists, std::ptr::eq(kernel, &deferred.shade)) {
                lists
                    .bins
                    .record(&mut graph, &mut rec, &list_packets[0], &lists.dispatch)?;
                // The material pass writes each opted-in slot's record before any slot shades.
                for (slot, record) in material_records.iter().enumerate() {
                    if let Some((kernel, packet)) = record {
                        kernel.record_indirect_labelled(
                            &mut graph,
                            &mut rec,
                            packet,
                            &lists.arguments,
                            slot as u64 * 16,
                            "ara::PrimaryMaterialList".into(),
                        )?;
                    }
                }
                for (slot, ((_, kernel, _), packet)) in
                    lists.slots.iter().zip(&list_packets[1..]).enumerate()
                {
                    kernel.record_indirect(
                        &mut graph,
                        &mut rec,
                        packet,
                        &lists.arguments,
                        slot as u64 * 16,
                    )?;
                }
                continue;
            }
            match deferred
                .repairs
                .as_ref()
                .filter(|_| std::ptr::eq(kernel, &deferred.fresh))
            {
                Some(queue) => kernel.record_indirect(&mut graph, &mut rec, packet, queue, 0)?,
                None => {
                    // The material pass writes the record before the single program shades.
                    if let (true, Some(Some((material, record)))) =
                        (std::ptr::eq(kernel, &deferred.shade), material_records.first())
                    {
                        material.record_labelled(
                            &mut graph,
                            &mut rec,
                            record,
                            [groups[0] * threads[0], groups[1] * threads[1], 1],
                            "ara::PrimaryMaterialTiles".into(),
                        )?;
                    }
                    kernel.record(
                        &mut graph,
                        &mut rec,
                        packet,
                        [groups[0] * threads[0], groups[1] * threads[1], 1],
                    )?
                }
            }
        }
    }
    // `encode_reconstruction`: keep the shaded samples, then resolve into the HDR target.
    if let (Some((images, (_, samples))), Some(packet)) = (resolve, &resolve_packet) {
        graph.node(
            &mut rec,
            &[
                (Resource::Image(output.image), Access::CopySource),
                (Resource::Image(samples.image), Access::CopyDestination),
            ],
        )?;
        rec.push(Command::BeginPass("ara::CopyImage".into()));
        rec.push(Command::CopyImage {
            source: output.image,
            destination: samples.image,
        });
        rec.push(Command::EndPass);
        images.record(&mut graph, &mut rec, packet)?;
    }
    let draws: Vec<_> = geometries
        .iter()
        .zip(&packets)
        .map(|(geometry, bindings)| ToonDraw { geometry, bindings })
        .collect();
    // `Renderer::encode_fresh`: receivers with the frame's depth, then visibility.
    if let (Some(traced), Some((estimate, receivers))) = (&traced, shadow_packets.split_last()) {
        let draws: Vec<_> = geometries
            .iter()
            .zip(receivers)
            .map(|(geometry, bindings)| ReceiverDraw { geometry, bindings })
            .collect();
        traced.receivers.record(
            &mut graph,
            &mut rec,
            &draws,
            ReceiverTargets {
                world: &traced.world,
                normal: &traced.normal,
                depth: &depth,
            },
        )?;
        traced.kernel.record(&mut graph, &mut rec, estimate)?;
    }
    let backdrop = match sky.as_ref().filter(|_| settings.sky && !deferred_path) {
        Some(sky) => {
            let backdrop = SkyPass::new(gpu, cache, Format::Rgba16Float)?;
            backdrop.record(
                &mut graph,
                &mut rec,
                &atmosphere,
                sky.luts,
                sky.volume,
                &output,
            )?;
            Some(backdrop)
        }
        None => None,
    };
    if let Some(pass) = &pass {
        pass.record(
            &mut graph,
            &mut rec,
            &draws,
            &MeshAttachments {
                color: &output,
                depth: &depth,
                clear_color: backdrop
                    .is_none()
                    .then_some([clear[0], clear[1], clear[2], 1.0]),
                clear_depth: Some(0.0),
            },
        )?;
    }
    drop(draws);
    // TP1/TP2 (`encode_transparency`): opaque depth from the hit records, then each pane
    // far to near: the HDR target copied behind it, a four-sample target seeded from
    // that copy, the pane, and the resolve back into the HDR target.
    let panes = match (deferred.as_ref(), upscaled.as_ref()) {
        (Some(deferred), None) if !scene.transparent.is_empty() => {
            let receivers = scene
                .transparent
                .iter()
                .map(pane_bounds)
                .collect::<Result<Vec<_>>>()?;
            let casters: Vec<_> = bounds.iter().map(|b| (b.min, b.max)).collect();
            let sun = scene.lighting.lights.iter().find_map(|l| match l {
                crate::Light::Directional { travel, lux } => Some((travel.normalize(), *lux)),
                _ => None,
            });
            let fit = sun.and_then(|(travel, _)| tp::shadow_fit(travel, &receivers, &casters));
            // The unjittered camera, as `Renderer::transparency.camera`.
            let block = tp::frame_block(&tp::FrameInputs {
                camera: scene.camera,
                size,
                sun,
                fit,
                step: 1.0 / 60.0,
                // The reflection lobe pass and `tp_reflect` are not assembled.
                reflect: false,
                scene: deferred.scene,
            });
            let mut camera_block = [0u8; 80];
            camera_block[..64].copy_from_slice(&crate::mesh::matrix_bytes(
                scene.camera.matrix(size[0], size[1]),
            ));
            camera_block[64..68].copy_from_slice(&(crate::ViewMode::Lit as u32).to_ne_bytes());
            let camera = StorageBuffer::new(gpu, &camera_block)?;
            let frame = StorageBuffer::new(gpu, &block)?;
            for buffer in [&camera, &frame] {
                graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
            }
            // The traced reflection's stand-in, and `Renderer::new_configured`'s material
            // defaults: white base colour and metallic-roughness, a flat normal in a
            // one-layer array.
            let reflected = Texture::new(gpu, [1, 1], Format::Rgba16Float, false, false)?;
            let defaults = [
                Texture::new(gpu, [1, 1], Format::Rgba8Srgb, false, false)?,
                Texture::new(gpu, [1, 1], Format::Rgba8, false, false)?,
                Texture::array(gpu, [1, 1], 1, Format::Rgba8, false, false)?,
            ];
            for (image, bytes) in [
                (&reflected, &[0u8; 8][..]),
                (&defaults[0], &[255; 4]),
                (&defaults[1], &[255; 4]),
                (&defaults[2], &[128, 128, 255, 255]),
            ] {
                staging.push(transfer::upload(
                    gpu,
                    &mut graph,
                    &mut rec,
                    image.image,
                    bytes,
                )?);
            }
            // Far to near along the view's forward axis, by world bounds centre.
            let inverse_view = scene.camera.view().inverse();
            let (eye, forward) = (
                inverse_view.w_axis.truncate(),
                -inverse_view.z_axis.truncate().normalize(),
            );
            let mut order: Vec<usize> = (0..scene.transparent.len()).collect();
            let distance = |i: usize| ((receivers[i].0 + receivers[i].1) * 0.5 - eye).dot(forward);
            order.sort_by(|a, b| distance(*b).total_cmp(&distance(*a)));
            // Each pane's program, composed once per distinct source.
            let pane_programs = scene.programs.map_or(&[][..], |p| p.transparent);
            ensure!(
                pane_programs.is_empty() || pane_programs.len() == scene.transparent.len(),
                "pane programs need one entry per pane"
            );
            let mut sources: Vec<&str> = Vec::new();
            let mut panes = Vec::new();
            for index in order {
                let object = &scene.transparent[index];
                let source = pane_programs
                    .get(index)
                    .copied()
                    .flatten()
                    .map_or(crate::program::STANDARD_PROGRAM, |p| p.source);
                ensure!(
                    !crate::program::declares_parameters(source),
                    "a pane's program with Parameters is not assembled"
                );
                let program = sources
                    .iter()
                    .position(|s| *s == source)
                    .unwrap_or_else(|| {
                        sources.push(source);
                        sources.len() - 1
                    });
                let geometry = MeshGeometry::new(
                    gpu,
                    object.vertices,
                    object.indices,
                    &[0],
                    &[object.transform],
                )?;
                geometry.import(&mut graph);
                let material = StorageBuffer::new(gpu, &object.material.bytes())?;
                let thickness = object.material.transparency.map_or(0.0, |t| t.thickness);
                let mut slot = [0u8; 16];
                slot[..4].copy_from_slice(&thickness.to_ne_bytes());
                let slot = StorageBuffer::new(gpu, &slot)?;
                for buffer in [&material, &slot] {
                    graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                }
                panes.push(Pane {
                    behind: Texture::new(gpu, size, Format::Rgba16Float, false, true)?,
                    geometry,
                    material,
                    slot,
                    program,
                    rect: pane_rect(pane_bounds(object)?, &scene.camera, size),
                    film: object
                        .material
                        .transparency
                        .is_some_and(|t| t.ior <= 1.0 && t.thickness == 0.0),
                });
            }
            let mut programs = Vec::new();
            let mut films = Vec::new();
            for source in &sources {
                let program = transparency_glass::traced_program(source)?;
                let glass = Glass::new(gpu, cache, &program)?;
                programs.push((program, glass));
                films.push(match transparency_glass::film_program(source)? {
                    Some(program) => {
                        let glass = Glass::film(gpu, cache, &program)?;
                        Some((program, glass))
                    }
                    None => None,
                });
            }
            let per_pixel = std::env::var("RENDERLAB_ARA_PANES").map_or(true, |v| v != "raster");
            let pixels = if per_pixel {
                let mut pixel_programs = Vec::new();
                for source in &sources {
                    let ids = transparency_glass::pane_id_program(source)?;
                    let raster = PaneIds::new(gpu, cache, &ids)?;
                    let shade = transparency_glass::pane_shade_program(source)?;
                    let kernel = Kernel::from_program(gpu, cache, &shade)?;
                    pixel_programs.push((ids, raster, shade, kernel));
                }
                Some(PanePixels {
                    programs: pixel_programs,
                    ids: Texture::shaped(
                        gpu,
                        size,
                        Format::R32Uint,
                        false,
                        true,
                        ara_gpu::texture::ImageShape {
                            samples: SampleCount::Four,
                            ..Default::default()
                        },
                    )?,
                })
            } else {
                None
            };
            Some(Panes {
                hit_depth: Kernel::new(gpu, cache, Pass::TransparencyHitDepth)?,
                seed: Seed::new(gpu, cache)?,
                programs,
                films,
                pixels,
                target: MsaaColor::new(gpu, size)?,
                camera,
                frame,
                depth: Texture::new(gpu, size, Format::R32Float, true, false)?,
                reflected,
                defaults,
                panes,
            })
        }
        _ => None,
    };
    // Per-pixel panes' records (`PanePixels` in pane_pixels.wgsl), made before the packets
    // that read them: each pane's rectangle and placement slot (0: each pane is its own
    // geometry).
    let mut pane_records: Vec<StorageBuffer> = Vec::new();
    if let Some(t) = panes.as_ref().filter(|t| t.pixels.is_some()) {
        for pane in &t.panes {
            let words: Vec<u8> = pane
                .rect
                .into_iter()
                .chain([0; 4])
                .flat_map(u32::to_le_bytes)
                .collect();
            let record = StorageBuffer::new(gpu, &words)?;
            graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
            pane_records.push(record);
        }
    }
    let mut pane_packets = Vec::new();
    if let (Some(t), Some(deferred)) = (&panes, deferred.as_ref()) {
        let values = shader::inputs(Pass::TransparencyHitDepth)?
            .iter()
            .map(|spec| {
                Ok((
                    spec.key,
                    match (spec.key.group, spec.key.binding) {
                        (0, 30) => Input::Buffer(&t.frame),
                        (0, 50) => Input::Image(&deferred.hit),
                        (0, 51) => Input::Image(&deferred.tiles),
                        (0, 52) => Input::Image(&t.depth),
                        _ => shared.input(spec, None)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let packet = HeapBindings::new(gpu, Pass::TransparencyHitDepth, &values)?;
        packet.import(&mut graph);
        t.hit_depth
            .record(&mut graph, &mut rec, &packet, [size[0], size[1], 1])?;
        pane_packets.push(packet);
        // Films whose program declares them blend in hardware: a run of consecutive films takes
        // one copy and one seed, each film blends into the seeded samples in one render pass,
        // and one resolve. Only runs of FILM_RUN or more: Metal's fragment stage shades a film
        // pixel about 15x slower than the per-pixel kernel (docs/m7-gaps.md), so the batch pays
        // only where it saves many panes' copy, id raster and dispatch (Ara's Outpost: about
        // 125 films, 22 ms to 1.5 ms on the M5; Glint's three contact-shadow films cost more
        // blended). A pane's rectangle is no estimate of its pixels: a pane with a corner
        // behind the eye takes the whole view.
        let filmish = |p: &Pane| p.film && t.films[p.program].is_some();
        let mut batched = vec![false; t.panes.len()];
        let mut start = 0;
        while start < t.panes.len() {
            let end = (start..t.panes.len())
                .find(|&i| !filmish(&t.panes[i]))
                .unwrap_or(t.panes.len());
            if end - start >= FILM_RUN {
                batched[start..end].fill(true);
            }
            start = end.max(start + 1);
        }
        let film = |at: usize| batched.get(at).copied().unwrap_or(false);
        let mut film_packets: Vec<(usize, HeapBindings)> = Vec::new();
        for (at, pane) in t.panes.iter().enumerate() {
            let is_film = film(at);
            let opens = !is_film || at == 0 || !film(at - 1);
            if opens {
                graph.node(
                    &mut rec,
                    &[
                        (Resource::Image(output.image), Access::CopySource),
                        (Resource::Image(pane.behind.image), Access::CopyDestination),
                    ],
                )?;
                rec.push(Command::BeginPass("ara::CopyImage".into()));
                rec.push(Command::CopyImage {
                    source: output.image,
                    destination: pane.behind.image,
                });
                rec.push(Command::EndPass);
            }
            // The glass entry's inputs, shared by the per-pixel passes.
            let glass_input = |spec: &shader::ShaderInput| -> Result<Input<'_>> {
                Ok(match (spec.kind, spec.key.group, spec.key.binding) {
                    (_, 0, 0) => Input::Buffer(&t.camera),
                    (_, 0, 30) => Input::Buffer(&t.frame),
                    (_, 0, 31) => Input::Image(&pane.behind),
                    (_, 0, 32) => Input::Image(&t.depth),
                    // `TP1 linear clamp`.
                    (_, 0, 33) => Input::Sampler(SamplerDesc {
                        mip: Filter::Nearest,
                        ..SamplerDesc::default()
                    }),
                    (_, 0, 37) => Input::Buffer(&pane.slot),
                    (_, 0, 38) => Input::Image(&t.reflected),
                    (_, 2, 1) => Input::Image(&t.defaults[0]),
                    (_, 2, 3) => Input::Image(&t.defaults[1]),
                    (_, 2, 5) => Input::Image(&t.defaults[2]),
                    _ => shared.input(spec, Some((&pane.geometry, &pane.material)))?,
                })
            };
            if is_film {
                if opens {
                    let seed = HeapBindings::new(
                        gpu,
                        Pass::TransparencySeed,
                        &[(key(0, 31), Input::Image(&pane.behind))],
                    )?;
                    seed.import(&mut graph);
                    t.seed.record(&mut graph, &mut rec, &seed, &t.target)?;
                    pane_packets.push(seed);
                }
                let (program, _) = t.films[pane.program].as_ref().expect("a film's pipeline");
                let values = program
                    .inputs()
                    .iter()
                    .map(|spec| Ok((spec.key, glass_input(spec)?)))
                    .collect::<Result<Vec<_>>>()?;
                let packet = HeapBindings::for_program(gpu, program, &values)?;
                packet.import(&mut graph);
                film_packets.push((at, packet));
                if !film(at + 1) {
                    // The run, far to near, in one render pass; then one resolve.
                    let films: Vec<_> = film_packets
                        .iter()
                        .map(|(at, packet)| {
                            let pane = &t.panes[*at];
                            let glass =
                                &t.films[pane.program].as_ref().expect("a film's pipeline").1;
                            (glass, packet, &pane.geometry)
                        })
                        .collect();
                    transparency_glass::record_films(&mut graph, &mut rec, &films, &t.target)?;
                    t.target.resolve_to(&mut graph, &mut rec, &output)?;
                    pane_packets.extend(film_packets.drain(..).map(|(_, packet)| packet));
                }
                continue;
            }
            if let Some(pixels) = &t.pixels {
                // Per pixel: the triangle under each coverage sample, then one shading
                // dispatch over the pane's rectangle into the output (see PanePixels).
                let [x0, y0, x1, y1] = pane.rect;
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let (id_program, raster, shade_program, kernel) = &pixels.programs[pane.program];
                let values = id_program
                    .inputs()
                    .iter()
                    .map(|spec| Ok((spec.key, glass_input(spec)?)))
                    .collect::<Result<Vec<_>>>()?;
                let packet = HeapBindings::for_program(gpu, id_program, &values)?;
                packet.import(&mut graph);
                raster.record(&mut graph, &mut rec, &packet, &pane.geometry, &pixels.ids)?;
                pane_packets.push(packet);
                let values = shade_program
                    .inputs()
                    .iter()
                    .map(|spec| {
                        Ok((
                            spec.key,
                            match (spec.key.group, spec.key.binding) {
                                (0, 70) => Input::Buffer(pane.geometry.vertices()),
                                (0, 71) => Input::Buffer(pane.geometry.indices()),
                                (0, 72) => Input::Image(&pixels.ids),
                                (0, 76) => Input::Image(&output),
                                (0, 77) => Input::Buffer(&pane_records[at]),
                                _ => glass_input(spec)?,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let packet = HeapBindings::for_program(gpu, shade_program, &values)?;
                packet.import(&mut graph);
                kernel.record(&mut graph, &mut rec, &packet, [x1 - x0, y1 - y0, 1])?;
                pane_packets.push(packet);
                continue;
            }
            let seed = HeapBindings::new(
                gpu,
                Pass::TransparencySeed,
                &[(key(0, 31), Input::Image(&pane.behind))],
            )?;
            seed.import(&mut graph);
            t.seed.record(&mut graph, &mut rec, &seed, &t.target)?;
            pane_packets.push(seed);
            let (program, glass) = &t.programs[pane.program];
            let values = program
                .inputs()
                .iter()
                .map(|spec| Ok((spec.key, glass_input(spec)?)))
                .collect::<Result<Vec<_>>>()?;
            let packet = HeapBindings::for_program(gpu, program, &values)?;
            packet.import(&mut graph);
            glass.record(&mut graph, &mut rec, &packet, &pane.geometry, &t.target)?;
            t.target.resolve_to(&mut graph, &mut rec, &output)?;
            pane_packets.push(packet);
        }
    }
    let read = if blind {
        None
    } else {
        Some(transfer::readback(
            gpu,
            &mut graph,
            &mut rec,
            shown.image,
            u64::from(target[0]) * u64::from(target[1]) * 8,
        )?)
    };
    // Below native scale the HDR target still holds the shaded samples.
    let samples_read = match upscaled.as_ref().filter(|_| !blind) {
        Some(_) => Some(transfer::readback(
            gpu,
            &mut graph,
            &mut rec,
            output.image,
            u64::from(size[0]) * u64::from(size[1]) * 8,
        )?),
        None => None,
    };
    let table_read = match gi.filter(|_| indirect.is_some_and(|i| i.read_table)) {
        Some((cache, _)) => Some(transfer::readback_buffer(
            gpu,
            &mut graph,
            &mut rec,
            cache.table().buffer,
            0,
            cache.table().len,
        )?),
        None => None,
    };
    let outgoing_read = match gi
        .filter(|_| table_read.is_some())
        .and_then(|(cache, _)| cache.outgoing())
    {
        Some(outgoing) => Some(transfer::readback_buffer(
            gpu,
            &mut graph,
            &mut rec,
            outgoing.buffer,
            0,
            outgoing.len,
        )?),
        None => None,
    };

    let raw_read = match gi
        .filter(|_| table_read.is_some())
        .and_then(|(cache, _)| cache.raw())
    {
        Some(raw) => Some(transfer::readback_buffer(
            gpu, &mut graph, &mut rec, raw.buffer, 0, raw.len,
        )?),
        None => None,
    };
    let probe_read = match &probe {
        Some(probe) => Some(transfer::readback_buffer(
            gpu,
            &mut graph,
            &mut rec,
            probe.buffer,
            0,
            super::resolve_probe::BYTES,
        )?),
        None => None,
    };
    let deferred_read = match (inputs.get(&key(0, 62)), inputs.get(&key(0, 63))) {
        (Some(reflectance), Some(untraced)) => Some([
            transfer::readback(
                gpu,
                &mut graph,
                &mut rec,
                reflectance.image,
                u64::from(reflectance.size[0]) * u64::from(reflectance.size[1]) * 8,
            )?,
            transfer::readback(
                gpu,
                &mut graph,
                &mut rec,
                untraced.image,
                u64::from(untraced.size[0]) * u64::from(untraced.size[1]) * 8,
            )?,
        ]),
        _ => None,
    };
    let history_read = match resolve.filter(|_| reconstruct.is_some_and(|r| r.read_history)) {
        Some((images, _)) => Some(transfer::readback(
            gpu,
            &mut graph,
            &mut rec,
            images.written().image,
            u64::from(target[0]) * u64::from(target[1]) * 16,
        )?),
        None => None,
    };
    // `encode_ids`: the output's settled and front object ids, after reconstruction and
    // the panes. Front ids through glass are not resolved (no glass raster yet): front is
    // the opaque id.
    let mut id_records = Vec::new();
    if let (Some(ids), Some(deferred)) = (scene.ids, &deferred) {
        let reconstructed = resolve.is_some();
        let id_size = if reconstructed { target } else { size };
        if kept_ids.as_ref().is_none_or(|t| t.size != id_size) {
            if let Some(old) = kept_ids.take() {
                for texture in [old.ids, old.packed, old.settled[0], old.settled[1]] {
                    gpu.retire(Resource::Image(texture.image))?;
                }
            }
            let settled = [
                Texture::new(gpu, id_size, Format::Rgba32Uint, true, false)?,
                Texture::new(gpu, id_size, Format::Rgba32Uint, true, false)?,
            ];
            *kept_ids = Some(IdTargets {
                size: id_size,
                ids: Texture::array(gpu, id_size, 2, Format::R32Uint, true, false)?,
                settled,
                parity: 0,
                camera: None,
                packed: Texture::new(gpu, id_size, Format::Rgba8, true, false)?,
                packed_written: false,
            });
            // The previous settled side is read before any frame wrote it.
            let side = &kept_ids.as_ref().context("made above")?.settled;
            for texture in side {
                staging.push(transfer::upload(
                    gpu,
                    &mut graph,
                    &mut rec,
                    texture.image,
                    &vec![0; (id_size[0] * id_size[1] * 16) as usize],
                )?);
            }
        } else if let Some(t) = kept_ids.as_ref() {
            for texture in [&t.ids, &t.settled[0], &t.settled[1]] {
                graph.import(Resource::Image(texture.image), Access::ShaderRead);
            }
        }
        let targets = kept_ids.as_mut().context("made above")?;
        let continued = targets.camera.is_some()
            && reconstructed
            && resolve.is_some_and(|(images, _)| !images.reset());
        let camera = scene
            .camera
            .unjittered()
            .matrix(id_size[0], id_size[1])
            .to_cols_array();
        let still = targets.camera == Some(camera);
        targets.camera = Some(camera);
        targets.parity = 1 - targets.parity;
        // Moved placements: a transform that changed since the last frame, or a skinned one.
        let placements = left.transforms.len();
        let mut moved = vec![0u32; placements.div_ceil(32).max(1)];
        let mut at = 0;
        for (index, _) in scene.objects.iter().enumerate() {
            for _ in placed(index) {
                let changed = skin(index).is_some()
                    || history.transforms.get(at).is_none_or(|p| {
                        p.to_cols_array().map(f32::to_bits)
                            != left.transforms[at].to_cols_array().map(f32::to_bits)
                    });
                if changed {
                    moved[at / 32] |= 1 << (at % 32);
                }
                at += 1;
            }
        }
        let flags =
            u32::from(reconstructed) | if still { 2 } else { 0 } | if continued { 4 } else { 0 };
        let frame_words = [id_size[0], id_size[1], flags, placements as u32, 8, 4, 0, 0];
        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let entries = if ids.entries.is_empty() {
            vec![[0u32; 2]]
        } else {
            ids.entries.to_vec()
        };
        let ranges = if ids.ranges.is_empty() {
            vec![[0u32; 4]]
        } else {
            ids.ranges.to_vec()
        };
        let buffers = [
            StorageBuffer::new(gpu, &words(&frame_words))?,
            StorageBuffer::new(gpu, &words(entries.as_flattened()))?,
            StorageBuffer::new(gpu, &words(&moved))?,
            StorageBuffer::new(gpu, &words(ranges.as_flattened()))?,
        ];
        for buffer in &buffers {
            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
        }
        let zeros = [
            Texture::new(gpu, [1, 1], Format::Rg32Uint, false, false)?,
            Texture::new(gpu, [1, 1], Format::Rgba32Uint, false, false)?,
            Texture::new(gpu, [1, 1], Format::R32Uint, false, false)?,
        ];
        for (texture, bytes) in zeros.iter().zip([8, 16, 4]) {
            staging.push(transfer::upload(
                gpu,
                &mut graph,
                &mut rec,
                texture.image,
                &vec![0; bytes],
            )?);
        }
        let (anchors, layers) = match resolve {
            Some((images, _)) => images.written_slots(),
            None => (&zeros[0], &zeros[1]),
        };
        let (previous, written) = (
            &targets.settled[1 - targets.parity],
            &targets.settled[targets.parity],
        );
        let values = shader::inputs(Pass::IdResolve)?
            .iter()
            .map(|spec| {
                let value = match spec.key.binding {
                    0 => Input::Buffer(&buffers[0]),
                    1 => Input::Image(&deferred.hit),
                    2 => Input::Image(&deferred.tiles),
                    3 => Input::Image(anchors),
                    4 => Input::Image(layers),
                    5 => Input::Buffer(&buffers[1]),
                    6 => Input::Buffer(&buffers[2]),
                    7 => Input::Image(&zeros[2]),
                    8 => Input::Image(previous),
                    9 => Input::Image(written),
                    10 => Input::Image(&targets.ids),
                    11 => Input::Buffer(&buffers[3]),
                    _ => bail!("the id pass cannot supply {}", spec.name),
                };
                Ok((spec.key, value))
            })
            .collect::<Result<Vec<_>>>()?;
        let bindings = HeapBindings::new(gpu, Pass::IdResolve, &values)?;
        bindings.import(&mut graph);
        Kernel::new(gpu, cache, Pass::IdResolve)?.record(
            &mut graph,
            &mut rec,
            &bindings,
            [id_size[0], id_size[1], 1],
        )?;
        // What the next frame and the id calls between frames find: both read.
        graph.node(
            &mut rec,
            &[
                (Resource::Image(targets.ids.image), Access::ShaderRead),
                (Resource::Image(written.image), Access::ShaderRead),
            ],
        )?;
        id_records.push(bindings.release());
        id_records.extend(buffers);
        for texture in zeros {
            super::image::release(gpu, texture.image)?;
        }
    }

    // The post tail, in `PostGpu::encode` order. Each stage reads the one before it.
    let mut post_images = Vec::new();
    let mut post_buffers = Vec::new();
    let mut reads = [None, None, None];
    let mut tail = None;
    if let Some(settings) = scene.post.filter(|s| s.enabled) {
        settings.validate()?;
        let passes = PostPasses::new(gpu, cache, Format::Rgba32Float)?;
        left.post_serial = history.post_serial.wrapping_add(1);
        let record = SettingsRecord::new(
            gpu,
            &settings.record(
                left.post_serial,
                target[1],
                scene.colour_lut.map_or(LUT_SIDE, |l| l.size),
                scene.colour_lut.is_some(),
            ),
        )?;
        // `PostGpu::prepare`: the previous camera counts only from the frame before.
        let previous_camera = history
            .post_camera
            .filter(|(frame, _)| *frame == left.count)
            .map(|(_, matrix)| matrix);
        left.post_camera = Some((
            left.count + 1,
            scene.camera.unjittered().matrix(size[0], size[1]),
        ));
        graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
        // `PostGpu::write_lut`: RGBA8 texels of the LUT, or of the identity.
        let identity;
        let colour_lut = match scene.colour_lut {
            Some(lut) => {
                lut.validate()
                    .map_err(|e| anyhow::anyhow!("colour LUT: {e}"))?;
                lut
            }
            None => {
                identity = crate::post::ColourLut::identity(LUT_SIDE)
                    .map_err(|e| anyhow::anyhow!("identity LUT: {e}"))?;
                &identity
            }
        };
        let lut = Texture::volume(gpu, colour_lut.size, Format::Rgba8)?;
        let texels: Vec<u8> = colour_lut
            .rgb
            .iter()
            .flat_map(|v| {
                v.map(|x| (x * 255.0).round() as u8)
                    .into_iter()
                    .chain([255])
            })
            .collect();
        post_buffers.push(transfer::upload(
            gpu, &mut graph, &mut rec, lut.image, &texels,
        )?);
        let mut surface = None;
        if settings.motion_blur || settings.depth_of_field.is_some() {
            // `PostGpu::prepare` takes the frame's camera, jittered on a reconstructed
            // frame: its current matrix is the unjittered one, its inverse the frame's.
            // Both at the internal size; every post target is at the caller's.
            let frame = PostCameraFrame::new(
                gpu,
                view,
                previous_camera,
                size,
                settings.shutter_angle_degrees,
                settings.max_motion_radius_px,
            )?;
            graph.import(Resource::Buffer(frame.record.buffer), Access::HostWrite);
            let auxiliary = Texture::new(gpu, target, Format::Rgba32Float, true, false)?;
            // `PostGpu::encode`'s surface pass: from the depth target on a raster frame,
            // from the hit records and the placements' motion on a traced one.
            // The hit-based surface's packet record, released with the frame.
            let surface_record = if deferred_path {
                let kernel = Kernel::new(gpu, cache, Pass::PostPrimarySurface)?;
                let values = shader::inputs(Pass::PostPrimarySurface)?
                    .iter()
                    .map(|spec| {
                        Ok((
                            spec.key,
                            match (spec.key.group, spec.key.binding) {
                                (0, 40) => Input::Buffer(&frame.record),
                                (0, 41) => Input::Image(&auxiliary),
                                // Rigid draws' last vertices are this frame's.
                                (1, 6) => Input::Buffers(&arrays.previous),
                                _ => shared.input(spec, None)?,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let packet = HeapBindings::new(gpu, Pass::PostPrimarySurface, &values)?;
                packet.import(&mut graph);
                kernel.record(&mut graph, &mut rec, &packet, [target[0], target[1], 1])?;
                // The packet borrows the frame record and the surface, which move on
                // below: keep only its own record.
                Some(packet.release())
            } else {
                RasterPostSurface::new(gpu, cache)?
                    .record(&mut graph, &mut rec, &depth, &frame, &auxiliary)?;
                None
            };
            // `PostGpu::encode`: the tile maximum and the blur with motion blur on, then
            // depth of field over what that left, each only when its setting asks.
            let mut blurred = None;
            if settings.motion_blur {
                let tiles = Texture::new(
                    gpu,
                    target.map(|n| n.div_ceil(16)),
                    Format::Rgba16Float,
                    true,
                    false,
                )?;
                passes.filter(
                    &mut graph,
                    &mut rec,
                    Pass::TileMax,
                    &FilterInputs {
                        colour: shown,
                        second: shown,
                        auxiliary: &auxiliary,
                        velocity_tiles: shown,
                        settings: &record,
                    },
                    &tiles,
                )?;
                let image = Texture::new(gpu, target, Format::Rgba16Float, true, false)?;
                passes.filter(
                    &mut graph,
                    &mut rec,
                    Pass::MotionBlur,
                    &FilterInputs {
                        colour: shown,
                        second: shown,
                        auxiliary: &auxiliary,
                        velocity_tiles: &tiles,
                        settings: &record,
                    },
                    &image,
                )?;
                post_images.push(tiles);
                blurred = Some(image);
            }
            if settings.depth_of_field.is_some() {
                let source = blurred.as_ref().unwrap_or(shown);
                let focused = Texture::new(gpu, target, Format::Rgba16Float, true, false)?;
                // The kernel reads no velocity tiles: that slot takes its own input.
                passes.filter(
                    &mut graph,
                    &mut rec,
                    Pass::DepthOfField,
                    &FilterInputs {
                        colour: source,
                        second: source,
                        auxiliary: &auxiliary,
                        velocity_tiles: source,
                        settings: &record,
                    },
                    &focused,
                )?;
                if !blind {
                    reads[2] = Some(transfer::readback(
                        gpu,
                        &mut graph,
                        &mut rec,
                        focused.image,
                        u64::from(target[0]) * u64::from(target[1]) * 8,
                    )?);
                }
                post_images.extend(blurred.replace(focused));
            }
            let blurred = blurred.context("a post surface with no pass to read it")?;
            if !blind {
                reads[0] = Some(transfer::readback(
                    gpu,
                    &mut graph,
                    &mut rec,
                    auxiliary.image,
                    u64::from(target[0]) * u64::from(target[1]) * 16,
                )?);
            }
            surface = Some((surface_record, frame, blurred));
            post_images.push(auxiliary);
        }
        let colour = surface.as_ref().map_or(shown, |(_, _, blurred)| blurred);
        let bloom = if settings.bloom > 0.0 {
            let bloom = Bloom::new(gpu, target)?;
            bloom.record(&passes, &mut graph, &mut rec, colour, &record)?;
            Some(bloom)
        } else {
            None
        };
        let linear = Texture::new(gpu, target, Format::Rgba32Float, false, true)?;
        passes.tone(
            &mut graph,
            &mut rec,
            &ToneInputs {
                colour,
                bloom: bloom.as_ref().map_or(colour, Bloom::output),
                lut: &lut,
                settings: &record,
            },
            &linear,
        )?;
        // The displayed image is kept past the frame for a presenter: the one of the
        // sequence's two that is not on screen, made again when the size changes.
        let slot = shown_output.map_or(0, |s| 1 - s);
        if outputs[slot].as_ref().is_some_and(|o| o.size != target)
            && let Some(old) = outputs[slot].take()
        {
            super::image::release(gpu, old.image)?;
        }
        if outputs[slot].is_none() {
            outputs[slot] = Some(Texture::new(gpu, target, Format::Rgba8, false, true)?);
        }
        let display = outputs[slot].as_ref().context("made above")?;
        passes.display(&mut graph, &mut rec, &linear, display)?;
        // A presenter copies it from here (`ExternalFrame::image` is in `CopySource`).
        graph.node(
            &mut rec,
            &[(Resource::Image(display.image), Access::CopySource)],
        )?;
        if !blind {
            reads[1] = Some(transfer::readback(
                gpu,
                &mut graph,
                &mut rec,
                display.image,
                u64::from(target[0]) * u64::from(target[1]) * 4,
            )?);
        }
        post_images.extend([lut, linear]);
        *shown_output = Some(slot);
        post_buffers.push(record.buffer);
        tail = Some((passes, bloom, surface));
    }

    times.record = started.elapsed();
    let submitted = std::time::Instant::now();
    if blind {
        // A timed frame: one completion scope around its submission returns every
        // pass's device timestamps when it is done. A frame that reads nothing back
        // leaves it in flight and returns: the next frame records while it runs, and
        // the GPU times reported are the newest finished frame's.
        let reads_back = read.is_some()
            || samples_read.is_some()
            || reads.iter().any(Option::is_some)
            || table_read.is_some()
            || outgoing_read.is_some()
            || raw_read.is_some()
            || history_read.is_some()
            || probe_read.is_some();
        gpu.begin_deferred_completion_on_current_queue()?;
        let submission = gpu.submit(rec.finish(), &[]);
        in_flight.push_back(gpu.end_deferred_completion()?);
        submission?;
        settle(gpu, in_flight, done, if reads_back { 0 } else { 2 })?;
        if let Some((span, passes)) = done.take() {
            times.gpu = span;
            times.passes = passes;
        }
    } else {
        settle(gpu, in_flight, done, 0)?;
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
        times.gpu = submitted.elapsed();
    }
    let finishing = std::time::Instant::now();
    let mut images = ForwardImages {
        times,
        hdr: read
            .map(|b| gpu.read(b))
            .transpose()
            .context("HDR readback")?
            .unwrap_or_default(),
        samples: samples_read.map(|b| gpu.read(b)).transpose()?,
        surface: reads[0].map(|b| gpu.read(b)).transpose()?,
        display: reads[1].map(|b| gpu.read(b)).transpose()?,
        focused: reads[2].map(|b| gpu.read(b)).transpose()?,
        indirect_table: table_read.map(|b| gpu.read(b)).transpose()?.map(|bytes| {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes(*w))
                .collect()
        }),
        indirect_outgoing: outgoing_read
            .map(|b| gpu.read(b))
            .transpose()?
            .map(|bytes| bytemuck::pod_collect_to_vec(&bytes)),
        indirect_raw: raw_read
            .map(|b| gpu.read(b))
            .transpose()?
            .map(|bytes| bytemuck::pod_collect_to_vec(&bytes)),
        deferred_reflections: match deferred_read {
            Some([a, b]) => Some([gpu.read(a)?, gpu.read(b)?]),
            None => None,
        },
        reconstruction_history: history_read
            .map(|b| gpu.read(b))
            .transpose()?
            .map(|bytes| bytemuck::pod_collect_to_vec(&bytes))
            .zip(resolve.map(|(images, _)| images.reset())),
        resolve_probe: probe_read
            .map(|b| gpu.read(b))
            .transpose()?
            .map(|bytes| bytemuck::pod_collect_to_vec(&bytes)),
    };

    if let Some((_, bloom, surface)) = tail {
        if let Some(bloom) = bloom {
            bloom.retire(gpu)?;
        }
        if let Some((record, frame, blurred)) = surface {
            if let Some(record) = record {
                record.retire(gpu)?;
            }
            frame.record.retire(gpu)?;
            post_images.push(blurred);
        }
    }
    for packet in packets
        .into_iter()
        .chain(shadow_packets)
        .chain(deferred_packets)
        .chain(gi_packets)
        .chain(gi_shade_packets)
        .chain(resolve_packet)
        .chain(pane_packets)
        .chain(transmission_packets)
        .chain(list_packets)
    {
        packet.retire(gpu, &mut graph)?;
    }
    if let Some(panes) = panes {
        panes.retire(gpu, &mut graph)?;
    }
    if let Some((_, panes, base, depth, _, layers)) = transmission {
        for (geometry, material) in panes {
            geometry.retire(gpu, &mut graph)?;
            material.retire(gpu)?;
        }
        for image in [base, depth, layers] {
            super::image::release(gpu, image.image)?;
        }
    }
    if let Some(lists) = program_lists {
        for (_, _, info) in lists.slots {
            info.retire(gpu)?;
        }
        lists.entries.retire(gpu)?;
        lists.arguments.retire(gpu)?;
    }
    if let Some((record, samples)) = tr1_records {
        record.retire(gpu)?;
        post_images.push(samples);
        // The frame is recorded and submitted: the history moves to its next sample.
        if let Some(images) = tr1.as_mut() {
            images.advance(history_read.is_some());
        }
    }
    if let Some(records) = gi_frame {
        records.retire(gpu)?;
    }
    if let Some(deferred) = deferred {
        *kept_repairs = deferred.repairs;
        for buffer in [
            deferred.frame,
            deferred.region,
            deferred.placements,
            deferred.models,
            deferred.normals,
            deferred.absent,
        ]
        .into_iter()
        {
            buffer.retire(gpu)?;
        }
        post_images.extend([deferred.hit, deferred.tiles, deferred.epsilon]);
        post_images.extend(deferred.material.into_iter().flatten());
    }
    if let Some(traced) = traced {
        traced.scene.retire(gpu)?;
        for mesh in traced.transient {
            mesh.retire(gpu)?;
        }
        for tangents in traced.tangents.into_iter().flatten() {
            tangents.retire(gpu)?;
        }
        traced.records.retire(gpu)?;
        post_images.extend([traced.world, traced.normal, traced.visibility]);
    }
    for geometry in geometries {
        geometry.retire(gpu, &mut graph)?;
    }
    for buffer in materials
        .into_iter()
        .chain(pane_records)
        .chain(parameter_blocks)
        .chain(buffers.into_values())
        .chain([camera, lighting])
    {
        buffer.retire(gpu)?;
    }

    atmosphere.retire(gpu, &mut graph)?;
    cluster_inputs.retire(gpu)?;
    for record in id_records {
        record.retire(gpu)?;
    }
    for record in fog_records {
        record.retire(gpu)?;
    }
    for buffer in previous_buffers.into_iter().flatten() {
        buffer.retire(gpu)?;
    }
    depth.retire(gpu, &mut graph)?;
    for image in inputs
        .into_values()
        .chain(post_images)
        .chain(transient_textures)
        .chain([lists, output])
        .chain(upscaled)
    {
        super::image::release(gpu, image.image)?;
    }
    for buffer in staging
        .into_iter()
        .chain(post_buffers)
        .chain(reads.into_iter().flatten())
        .chain(table_read)
        .chain(outgoing_read)
        .chain(raw_read)
        .chain(history_read)
        .chain(probe_read)
        .chain(read)
    {
        gpu.retire(Resource::Buffer(buffer))?;
    }
    if let Some(probe) = probe {
        probe.retire(gpu)?;
    }
    gpu.collect()?;
    images.times.finish = finishing.elapsed();
    Ok((images, left))
}
