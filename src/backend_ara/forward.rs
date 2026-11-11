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
    temporal::{History, Reconstruct},
    toon::{MeshAttachments, ToonDraw, ToonPass},
    transfer,
    transmission_shadow::Transmission,
    transparency_glass::{self, Glass},
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
    commands::Command,
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

/// One texture slot: the asset the wgpu renderer would take through
/// `try_upload_texture`, with every mip level, and its `TextureBinding` sampler.
#[derive(Clone, Copy)]
pub struct ObjectTexture<'a> {
    pub data: &'a crate::asset::Texture,
    pub sampler: crate::asset::Sampler,
}
/// A tangent-space normal map with the tangent frames it is authored against: what the
/// wgpu renderer takes as a material's `normal_texture` and the mesh's tangent stream
/// (`try_upload_mesh_with_tangents`), which it refuses to draw one without the other.
#[derive(Clone, Copy)]
pub struct NormalMap<'a> {
    /// `TextureRole::NormalLinear`. The variance and slope-moment roles are not
    /// assembled.
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
    /// From submission until the GPU has finished the frame.
    pub gpu: std::time::Duration,
    /// Reading the images back and releasing the frame's resources.
    pub finish: std::time::Duration,
    /// On a sequence `without_images`: the GPU's own time in each labelled pass, in
    /// milliseconds from the device's timestamps, summed by label in first-run order.
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
}

/// The post `Settings` record at this output height for the post frame with this
/// serial (one for the first), with the identity LUT the renderer starts with.
pub fn post_record(settings: &crate::PostSettings, height: u32, serial: u32) -> [f32; 20] {
    settings.record(serial, height, LUT_SIDE, false)
}
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
impl ResidentMesh {
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
}
/// What a traced frame's transparent panes add: `encode_transparency`'s targets,
/// records and pipelines.
struct Panes {
    hit_depth: Kernel,
    seed: Seed,
    /// Each distinct pane program composed for the glass pass, with its pipeline.
    programs: Vec<(Program, Glass)>,
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
impl Panes {
    fn retire(self, gpu: &mut GpuDevice, graph: &mut Graph) -> Result<()> {
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
    traced: Option<&'a Traced>,
    /// The radiance cache's table where the cache runs; otherwise a zero header binds.
    gi_table: Option<&'a StorageBuffer>,
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
                _ => &self.buffers[&spec.key],
            }),
            (InputKind::BufferArray { .. }, group, binding) => {
                Input::Buffers(match (group, binding) {
                    (1, 2) => &self.arrays.vertices,
                    (1, 3) => &self.arrays.indices,
                    (1, 4) => &self.arrays.tangents,
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
                    _ => provided(self.sky, self.traced, self.deferred, spec.key)
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
/// The atmosphere's and the shadow passes' images where they ran.
fn provided<'a>(
    sky: Option<&'a Sky<'a>>,
    traced: Option<&'a Traced>,
    deferred: Option<&'a Deferred>,
    key: InputKey,
) -> Option<&'a Texture> {
    match (key.group, key.binding) {
        (0, 13) | (0, 7) => deferred.map(|d| &d.hit),
        (0, 20) | (0, 21) => deferred.map(|d| &d.tiles),
        (0, 23) => deferred.map(|d| &d.epsilon),
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
    Program::new(
        Pass::MeshToon,
        crate::program::compose(
            &head,
            source,
            crate::program::Stage::Raster,
            crate::program::declares_parameters(source),
        )
        .source,
    )
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
    /// The Interactive trace's repair queue: a header, then room for every pixel.
    repairs: Option<StorageBuffer>,
    /// The 2D targets the last frame made and released, for the next frame to take.
    targets: super::image::pool::Pool,
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
    pub fn without_images() -> Self {
        Self {
            blind: true,
            ..Self::default()
        }
    }
    /// Forgets history: the next frame is a first frame after a cut. The cache keeps
    /// its buffers and starts from an empty table, as `Renderer::reset_indirect`. The
    /// meshes and the atmosphere's tables stay as they are.
    pub fn cut(&mut self) {
        if let Some(cache) = &mut self.kept.gi {
            cache.reset();
        }
        if let Some(history) = &mut self.kept.tr1 {
            history.cut();
        }
        self.carried = Carried::default();
    }
    /// Releases what the sequence kept on the GPU.
    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
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
        if let Some(repairs) = self.kept.repairs {
            repairs.retire(gpu)?;
        }
        self.kept.targets.retire(gpu)
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
        let result = super::image::pool::active(&mut targets, gpu, |gpu| {
            frame(&self.carried, &mut self.kept, self.blind, gpu, cache, scene)
        });
        self.kept.targets = targets;
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
        repairs: kept_repairs,
        // Taken out and made active for this frame by `Frames::render`.
        targets: _,
    } = kept;
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
    let mut left = Carried {
        count: history.count + 1,
        post_serial: history.post_serial,
        post_camera: None,
        shadow_bases: Vec::new(),
        // Per placement, in draw then instance order, as every table below.
        transforms: (0..scene.objects.len())
            .flat_map(|i| placed(i).iter().copied())
            .collect(),
    };
    ensure!(!scene.objects.is_empty(), "forward frame needs an object");
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
    // `prepare_cache_and_post`: only traced frames feed the cache.
    let indirect = scene.indirect.filter(|i| i.settings.enabled);
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
    let program = if deferred_path {
        primary_shade::hardware_soft_program(material_source)?
    } else {
        standard_program(scene.sun_shadows.is_some())?
    };
    // A traced frame of several programs: each one's list entry, in slot order.
    let list_programs = if deferred_path && sources.len() > 1 {
        sources
            .iter()
            .map(|source| primary_shade::hardware_soft_list_program(source))
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
    let mut bounds = Vec::new();
    for (index, object) in scene.objects.iter().enumerate() {
        let local =
            crate::visibility::Aabb::from_positions(object.vertices.iter().map(|v| v.position))
                .context("empty mesh")?;
        for transform in placed(index) {
            bounds.push(shadow_gpu::world_bounds(local, *transform)?);
        }
    }
    let clusters = LightClusters::new(gpu, cache)?;
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let mut staging = Vec::new();

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
            settings.volumetric_fog.is_none(),
            "forward frame does not assemble volumetric fog yet"
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
        h.finalize().into()
    };
    // `prepare_light_clusters` on a frame whose shading traces local shadows: every
    // point and spot light's record names itself, and needs no map slot.
    let local_shadows = scene.local_shadows.filter(|s| s.count > 0);
    ensure!(
        local_shadows.is_none() || deferred_path,
        "local-light shadows are assembled on traced frames only"
    );
    if local_shadows.is_some() {
        for (index, (record, light)) in records
            .as_chunks_mut::<{ crate::lighting::LIGHT_BYTES }>()
            .0
            .iter_mut()
            .zip(&scene.lighting.lights)
            .enumerate()
        {
            if lc::sphere(light).is_some() {
                record[60..64].copy_from_slice(&((index as u32 & 0xffff) + 1).to_ne_bytes());
            }
        }
    }
    let mut frame = lc::header(&view, size, packed.count, true, &[]);
    // LC2's source-sampling word turns only with reconstruction: its sample plus one.
    let turn = sample.map_or(0u32, |k| k.wrapping_add(1) as u32);
    frame[88..92].copy_from_slice(&turn.to_ne_bytes());
    let near = scene.local_shadows.unwrap_or_default().near;
    frame[100..104].copy_from_slice(&near.to_ne_bytes());
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
    let atmosphere = AtmosphereParameters::for_camera_with_sun(
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
    // nearest sheet passes and that sheet's depth. Its record is `TpTransmission`: the
    // map's matrix, the direction toward the sun with the layer count, twice the fit's
    // bias. Without panes or a sun the record stays zero and shading reads full light.
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
                            .extend(scene.transparent.len() as f32)
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
            let usage =
                ara_gpu::vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
                    | ara_gpu::vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
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
                let key = scene.meshes.map(|m| m[index]);
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
                        let vertex_bytes: Vec<u8> = object
                            .vertices
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
                        let vertices = StorageBuffer::with_usage(gpu, &vertex_bytes, usage)?;
                        let indices = StorageBuffer::with_usage(
                            gpu,
                            bytemuck::cast_slice(object.indices),
                            usage,
                        )?;
                        let blas =
                            gpu.create_acceleration_structure(&AccelerationDesc::Triangles {
                                vertices: vertices.buffer,
                                vertex_count: object.vertices.len() as u32,
                                vertex_stride: 48,
                                indices: indices.buffer,
                                index_count: object.indices.len() as u32,
                            })?;
                        for buffer in [&vertices, &indices] {
                            graph.import(Resource::Buffer(buffer.buffer), Access::HostWrite);
                        }
                        graph.node(
                            &mut rec,
                            &[
                                (Resource::Buffer(vertices.buffer), Access::AsInput),
                                (Resource::Buffer(indices.buffer), Access::AsInput),
                                (Resource::AccelerationStructure(blas), Access::AsBuild),
                            ],
                        )?;
                        rec.push(Command::BeginPass("ara::BuildBlas".into()));
                        rec.push(Command::BuildAccelerationStructure(blas));
                        rec.push(Command::EndPass);
                        let mesh = ResidentMesh {
                            vertices,
                            indices,
                            blas,
                            counts,
                        };
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
                for transform in placed(index) {
                    let m = transform.to_cols_array_2d();
                    instances.push(AccelerationInstance {
                        geometry: place.of(resident, &transient).blas,
                        transform: std::array::from_fn(|i| m[i % 4][i / 4]),
                        custom_index: instances.len() as u32,
                        mask: 255,
                    });
                }
                geometry.push(place);
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
                rendered_frames: history.count,
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
                visible_sky: settings.sky,
                pixel_angle: 2.0 * (0.5 * scene.camera.fov_y()).tan() / target[1] as f32,
                scene_bounds,
                skin_bounds: None,
                skin_revisions: vec![],
                rigid_motion,
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
                        field,
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
        // The placement words, then each draw's batch word (triangle count; no adjacency
        // suffix and no skinned stream here), as `prepare_primary` uploads them.
        let batches: Vec<u32> = scene
            .objects
            .iter()
            .map(|o| crate::primary_ray::batch_word(o.indices.len() as u64, false))
            .collect();
        let placement_buffer = StorageBuffer::new(
            gpu,
            bytemuck::cast_slice(&crate::primary_ray::placement_words(&placements, &batches)),
        )?;
        let models = StorageBuffer::new(gpu, &[matrices.as_slice(), &previous].concat())?;
        let normals = StorageBuffer::new(gpu, &normal_bytes)?;
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
    for list in &list_programs {
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
    // The transmission record and map are what every program reads at (3, 26) and
    // (3, 27); each pane's geometry and material are drawn into its layer below.
    const TRANSMISSION_SIZE: [u32; 2] = [1024; 2];
    let mut transmission = None;
    if let Some(record) = &transmission_record {
        let record = StorageBuffer::new(gpu, record)?;
        graph.import(Resource::Buffer(record.buffer), Access::HostWrite);
        buffers.insert(key(3, 26), record);
        inputs.insert(
            key(3, 27),
            Texture::array(
                gpu,
                TRANSMISSION_SIZE,
                scene.transparent.len() as u32,
                Format::Rgba16Float,
                false,
                true,
            )?,
        );
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
                    || provided(sky.as_ref(), traced.as_ref(), deferred.as_ref(), spec.key)
                        .is_some()
                    || inputs.contains_key(&spec.key)
                {
                    continue;
                }
                let (format, texel) = match sample {
                    SampleType::Float => (Format::Rgba16Float, 8),
                    SampleType::Uint => (Format::R32Uint, 4),
                    SampleType::Depth => (Format::Depth32Float, 4),
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
        // `MaterialDescriptor::bytes` reads `normal_mapped` off its own texture binding,
        // which names a wgpu texture and stays empty here.
        let mut record = object.material.bytes();
        record[36..40].copy_from_slice(&u32::from(normal_map(index).is_some()).to_ne_bytes());
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
    if let Some((pass, panes, base, _, depth_views)) = &transmission {
        let map = &inputs[&key(3, 27)];
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
    }
    // `try_upload_texture` and `sampler_descriptor` for each object's texture slots.
    let mut object_textures: Vec<[Option<(Texture, SamplerDesc)>; 3]> = Vec::new();
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
                let first = texture
                    .data
                    .mips
                    .first()
                    .context("texture without a level")?;
                ensure!(
                    texture.data.role == role,
                    "texture slot holds a {:?} texture",
                    texture.data.role
                );
                let image = Texture::sampled_mips(
                    gpu,
                    [first.width, first.height],
                    texture.data.mips.len() as u32,
                    format,
                    dimension,
                )?;
                for (level, mip) in texture.data.mips.iter().enumerate() {
                    ensure!(
                        mip.rgba8.len() == mip.width as usize * mip.height as usize * 4,
                        "texture level {level} is not tightly packed RGBA8"
                    );
                    staging.push(transfer::upload_mip(
                        gpu,
                        &mut graph,
                        &mut rec,
                        image.image,
                        level as u32,
                        [mip.width, mip.height],
                        &mip.rgba8,
                    )?);
                }
                let sampler = sampler_desc(
                    texture.sampler,
                    textures.anisotropy_clamp,
                    texture.data.mips.len(),
                )?;
                slots[slot] = Some((image, sampler));
            }
            object_textures.push(slots);
        }
    }
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
        traced: traced.as_ref(),
        gi_table: gi.map(|(cache, _)| cache.table()),
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
            let arguments = StorageBuffer::with_usage(
                gpu,
                &vec![0u8; count as usize * 16],
                ara_gpu::vk::BufferUsageFlags::INDIRECT_BUFFER,
            )?;
            graph.import(Resource::Buffer(arguments.buffer), Access::HostWrite);
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
            // The repair runs over the queue the trace filled, from its own counts.
            // `encode_program_lists`: binning, then each program's list from the counts
            // the binning pass left.
            if let (Some(lists), true) = (&program_lists, std::ptr::eq(kernel, &deferred.shade)) {
                lists
                    .bins
                    .record(&mut graph, &mut rec, &list_packets[0], &lists.dispatch)?;
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
                None => kernel.record(
                    &mut graph,
                    &mut rec,
                    packet,
                    [groups[0] * threads[0], groups[1] * threads[1], 1],
                )?,
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
                });
            }
            let mut programs = Vec::new();
            for source in sources {
                let program = transparency_glass::traced_program(source)?;
                let glass = Glass::new(gpu, cache, &program)?;
                programs.push((program, glass));
            }
            Some(Panes {
                hit_depth: Kernel::new(gpu, cache, Pass::TransparencyHitDepth)?,
                seed: Seed::new(gpu, cache)?,
                programs,
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
        for pane in &t.panes {
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
                .map(|spec| {
                    Ok((
                        spec.key,
                        match (spec.kind, spec.key.group, spec.key.binding) {
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
                        },
                    ))
                })
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
    // The post tail, in `PostGpu::encode` order. Each stage reads the one before it.
    let mut post_images = Vec::new();
    let mut post_buffers = Vec::new();
    let mut reads = [None, None, None];
    let mut tail = None;
    if let Some(settings) = scene.post.filter(|s| s.enabled) {
        settings.validate()?;
        let passes = PostPasses::new(gpu, cache, Format::Rgba32Float)?;
        left.post_serial = history.post_serial.wrapping_add(1);
        let record =
            SettingsRecord::new(gpu, &post_record(&settings, target[1], left.post_serial))?;
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
        let lut = Texture::volume(gpu, LUT_SIDE, Format::Rgba16Float)?;
        let identity: Vec<u8> = (0..LUT_SIDE.pow(3))
            .flat_map(|i| {
                [i & 1, (i >> 1) & 1, i >> 2, 1].map(|v| half::f16::from_f32(v as f32).to_bits())
            })
            .flat_map(u16::to_le_bytes)
            .collect();
        post_buffers.push(transfer::upload(
            gpu, &mut graph, &mut rec, lut.image, &identity,
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
                                // Rigid placements: last frame's vertices are this frame's.
                                (1, 6) => Input::Buffers(&arrays.vertices),
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
        let display = Texture::new(gpu, target, Format::Rgba8, false, true)?;
        passes.display(&mut graph, &mut rec, &linear, &display)?;
        if !blind {
            reads[1] = Some(transfer::readback(
                gpu,
                &mut graph,
                &mut rec,
                display.image,
                u64::from(target[0]) * u64::from(target[1]) * 4,
            )?);
        }
        post_images.extend([lut, linear, display]);
        post_buffers.push(record.buffer);
        tail = Some((passes, bloom, surface));
    }

    times.record = started.elapsed();
    let submitted = std::time::Instant::now();
    if blind {
        // A timed frame: one completion scope around its submission returns every
        // pass's device timestamps when it is done.
        gpu.begin_deferred_completion_on_current_queue()?;
        let submission = gpu.submit(rec.finish(), &[]);
        let mut completion = gpu.end_deferred_completion()?;
        submission?;
        for pass in gpu.wait_completion(&mut completion)? {
            match times
                .passes
                .iter_mut()
                .find(|(label, _)| *label == pass.pass)
            {
                Some((_, total)) => *total += pass.milliseconds,
                None => times.passes.push((pass.pass, pass.milliseconds)),
            }
        }
    } else {
        let point = gpu.submit(rec.finish(), &[])?;
        gpu.wait(point)?;
    }
    times.gpu = submitted.elapsed();
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
    if let Some((_, panes, base, depth, _)) = transmission {
        for (geometry, material) in panes {
            geometry.retire(gpu, &mut graph)?;
            material.retire(gpu)?;
        }
        for image in [base, depth] {
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
        .chain(parameter_blocks)
        .chain(buffers.into_values())
        .chain([camera, lighting])
    {
        buffer.retire(gpu)?;
    }

    atmosphere.retire(gpu, &mut graph)?;
    cluster_inputs.retire(gpu)?;
    depth.retire(gpu, &mut graph)?;
    for image in inputs
        .into_values()
        .chain(post_images)
        .chain(object_textures.into_iter().flatten().flatten().map(|t| t.0))
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
