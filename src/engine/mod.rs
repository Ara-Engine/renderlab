//! The engine-facing API: one [`Engine`] that owns a renderer, its device and the scene.
//!
//! A game creates an engine for a [`RenderPath`], creates resources (meshes, textures,
//! programs, materials, environments) and spawns objects, all named by copyable typed
//! handles ([`Id`]), and creates one or more views ([`Engine::create_view`]). Each frame it
//! moves objects, calls [`Engine::tick`] once and renders each view it shows from its
//! camera ([`Engine::render_view`] or [`Engine::render_view_to`]). Every call blocks only for GPU
//! backpressure; the engine handles admission retries, per-path configuration (traced
//! primaries or the visibility buffer, sun visibility, GI geometry) and draw batching.
//!
//! `use renderlab::engine::*` brings in everything a game needs. The crate root's
//! [`Renderer`] stays available through [`Engine::renderer_mut`] as the expert API: its
//! tuning surface changes as features are measured, while this module stays stable
//! (docs/api.md lists what is frozen).
mod device;
pub(crate) mod ids;
mod scene;
mod view;
#[cfg(test)]
mod view_tests;
// Crate-visible for a second GPU layer's engine: the same lighting and atmosphere
// translation, over the same state.
pub(crate) mod settings;
pub(crate) mod state;

pub use device::{Capabilities, ExperimentalRayQuery, RenderPath};
pub use ids::{
    EmitterId, EnvironmentId, Id, MaterialId, MeshId, ObjectId, ProgramId, TextureId, kind,
};
pub use scene::Scene;
pub use settings::{Background, Exposure, Quality, Settings, Sky, Sun};
pub use view::{Outputs, ViewDesc, ViewLook};

pub use crate::animation::{AnimationPose, ClipPlayer};
pub use crate::asset::{
    Primitive, Sampler, TangentStream, Texture as TextureData,
    environment::PreparedEnvironment,
    skin::{Influence, SkinnedAsset},
};
pub use crate::cutout::{AlphaMask, CutoutMesh, cutout};
pub use crate::{
    AtmosphereMedium, Camera, ColorKey, ColourLut, Decal, DecalBlend, DecalTexture,
    DecalTextureKind, DecalTextures, DepthOfField, EmitterDescriptor, Error, Events, Flipbook,
    Forces, FrameId, FrameReport, HeightFog, Light, Limits, LocalShadows, MAX_CURVE_KEYS,
    MAX_PARTICLES, Mat4, MeshData, PlaneCollision, PostSettings, ReflectionSettings, ReloadError,
    ReloadReport, RenderScale, Renderer, SizeKey, SpawnShape, SpriteAlignment, Tonemapper,
    Transparency, Value, Vec3, Vertex, ViewId, ViewMode, VolumetricFog,
};
pub use crate::{
    ID_NONE, ID_UNTAGGED, IdLayer, IdPixel, IdReadback, IdRect, IdRequest, IdsLost, MAX_ID_RECT,
};

use crate::{
    Emitter, EnvironmentMap, EnvironmentParameters, HardwareShadowOptions, Instances, Material,
    MaterialDescriptor, Mesh, PrimaryVisibility, Program, ProgramDescriptor, RendererOptions,
    SampleCount, SkinnedMeshInstance, Texture, TextureBinding,
};
use state::{MaterialEntry, Residency, State};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// The engine's shader prelude: bindings, the program API and the entry points that
/// programs compose with. Programs are the user-authored part (`create_program`).
const PRELUDE: &str = include_str!("../../shaders/scene.wgsl");
/// Longest the engine waits for the GPU to admit one upload or frame.
const ADMISSION: Duration = Duration::from_secs(20);
/// Events kept for `poll` between calls; older ones are counted in `Events::dropped`.
const KEPT_EVENTS: usize = 1024;

/// How to create an [`Engine`]. Construct with `..Default::default()`.
#[derive(Clone, Debug)]
pub struct EngineDescriptor {
    /// Defaults to `NonRt`, which needs no experimental features.
    pub path: RenderPath,
    /// Required for `RenderPath::HardwareRt`.
    pub experimental_ray_query: Option<ExperimentalRayQuery>,
    /// Format of every colour target the engine renders to (its own output and
    /// `render_to` targets). Use an sRGB format for display; `capture` needs RGBA8.
    pub output_format: wgpu::TextureFormat,
    /// Development policy limits per resource (instances per mesh-material batch, mesh
    /// and texture bytes). Device limits always apply as well.
    pub limits: Limits,
    pub settings: Settings,
    /// Anisotropic filtering of all-linear mipmapped textures, 1 (off) to 16. Fixed for
    /// the engine's lifetime.
    pub texture_anisotropy: u16,
    /// Directory for the persistent shared pipeline cache, so a relaunch or a reverted
    /// shader edit can reuse driver compilation. Includes first-use engine kernels.
    /// None retains only the in-memory program cache; cache I/O failures fall back to empty.
    pub pipeline_cache: Option<PathBuf>,
    /// Retain runtime bounds, loop and ray-query checks in engine-owned shaders.
    /// User programs always retain checks. Environment: RENDERLAB_ENGINE_SHADER_CHECKS.
    pub checked_engine_shaders: bool,
}
impl Default for EngineDescriptor {
    fn default() -> Self {
        Self {
            path: RenderPath::NonRt,
            experimental_ray_query: None,
            output_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            limits: Limits {
                // Large prepared scans (the quarry's 4K textures) fit the defaults.
                max_mesh_bytes: 128 << 20,
                max_texture_bytes: 128 << 20,
                ..Limits::default()
            },
            settings: Settings::default(),
            texture_anisotropy: 1,
            pipeline_cache: None,
            checked_engine_shaders: false,
        }
    }
}

/// A material's description; textures and the program are engine handles.
#[derive(Clone, Copy, Debug)]
pub struct MaterialDesc<'a> {
    /// Linear RGBA factor, multiplied by the base-colour texture and vertex colour.
    pub base_color: [f32; 4],
    /// glTF factors, multiplied by the metallic-roughness texture (B and G).
    pub metallic: f32,
    pub roughness: f32,
    pub base_color_texture: Option<TextureRef>,
    pub metallic_roughness_texture: Option<TextureRef>,
    /// Needs a mesh with tangents (`create_mesh_with_tangents`).
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    /// Both faces shade and cast. The default is true: both paths' shadowed primaries
    /// currently require double-sided materials.
    pub double_sided: bool,
    /// Shadow rays may leave either side toward the light, as for leaves and cards
    /// (cutout foliage). False keeps opaque receivers on the camera-viewed side.
    pub thin: bool,
    /// Glass and other see-through surfaces (TP1): drawn in a forward pass after the
    /// opaque frame, refracting and tinting what is behind them. Their program shades
    /// them; the standard program declares the `transmittance` hook glass needs. They
    /// cast no shadows at this revision. None is opaque.
    pub transparency: Option<Transparency>,
    /// The shading program; None is the engine's standard photoreal program.
    pub program: Option<ProgramId>,
    /// Values for the program's `struct Parameters`, checked by member name and type.
    /// Omitted members take the program's `DEFAULT_PARAMETERS`.
    pub parameters: &'a [(&'a str, Value)],
}
impl Default for MaterialDesc<'_> {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            double_sided: true,
            thin: false,
            transparency: None,
            program: None,
            parameters: &[],
        }
    }
}

/// A texture as a material binds it.
#[derive(Clone, Copy, Debug)]
pub struct TextureRef {
    pub texture: TextureId,
    pub sampler: Sampler,
}

/// The handles [`Engine::add_asset`] created, in the asset's own order.
#[derive(Clone, Debug, Default)]
pub struct AssetHandles {
    pub textures: Vec<TextureId>,
    pub materials: Vec<MaterialId>,
    /// Per asset mesh, one mesh per primitive.
    pub meshes: Vec<Vec<MeshId>>,
    /// Per placement, one object per primitive of the placed mesh.
    pub objects: Vec<Vec<ObjectId>>,
}

/// The handles [`Engine::add_skinned_asset`] created.
#[derive(Clone, Debug, Default)]
pub struct SkinnedHandles {
    pub textures: Vec<TextureId>,
    pub materials: Vec<MaterialId>,
    /// Per `SkinnedAsset::bindings` entry, its primitives as (posable mesh, material).
    /// Pose a binding with `AnimationPose::joint_palette(asset, binding)` and
    /// [`Engine::set_joint_palette`] on each of its meshes.
    pub bindings: Vec<Vec<(MeshId, MaterialId)>>,
}

/// An RGBA8 readback of the engine's output ([`Engine::capture`]).
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Tightly packed rows, top row first, in the output format's encoding.
    pub rgba8: Vec<u8>,
}

/// Static geometry, or one independently posed skinned primitive (SK1).
enum MeshEntry {
    Static(Mesh),
    Skinned(SkinnedMeshInstance),
}
impl MeshEntry {
    fn mesh(&self) -> &Mesh {
        match self {
            Self::Static(mesh) => mesh,
            Self::Skinned(skinned) => skinned.mesh(),
        }
    }
}

/// What the wgpu engine keeps for each resource: the renderer's own handles.
struct Handles;
impl Residency for Handles {
    type Mesh = MeshEntry;
    type Texture = Texture;
    type Program = Program;
    type Material = Material;
    type Environment = EnvironmentMap;
    type Emitter = Emitter;
    type Instances = Instances;
    fn set_instance(instances: &mut Instances, slot: usize, transform: Mat4) -> Result<(), Error> {
        instances.set(slot, transform)
    }
}

/// The renderer, its device, the scene's resources and objects, and the frame settings.
pub struct Engine {
    /// First, so the scene's handles drop after the renderer.
    renderer: Renderer,
    path: RenderPath,
    adapter: wgpu::AdapterInfo,
    format: wgpu::TextureFormat,
    limits: Limits,
    /// The scene: resource handles by id, objects in batches, sun, lights, sky, settings.
    state: State<Handles>,
    events: Events,
    /// The scene's object ids and the frame's id tables (src/object_ids.rs).
    ids: crate::object_ids::EngineIds,
    /// The views, and the renderer's empty view the next `create_view` takes (its first).
    views: Vec<view::ViewEntry>,
    spare: Option<ViewId>,
}

/// Instance policy for Renderlab's bounded engine-owned indirect dispatch arguments.
/// External adapters should use these flags too; explicit wgpu environment overrides apply.
///
/// Debug object names (`InstanceFlags::DEBUG`) are opt-in with `WGPU_DEBUG=1` (capture
/// tools), not a default of debug builds: with several devices alive, naming objects
/// through the Vulkan validation layer segfaulted inside Ubuntu's 1.4.341 loader
/// running the SDK's 1.4.363 layer (4 of 10 concurrent runs; 0 of 10 without names).
pub fn instance_flags() -> wgpu::InstanceFlags {
    wgpu::InstanceFlags::default()
        .difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL | wgpu::InstanceFlags::DEBUG)
        .with_env()
}

/// A Vulkan adapter for offscreen use (no surface), preferring a discrete GPU.
pub fn headless_adapter() -> Result<wgpu::Adapter, Error> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: instance_flags(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|_| Error::InvalidInput("no Vulkan adapter"))
}

/// The lib suite's one Vulkan adapter, shared by every test that builds an `Engine`. Each
/// `headless_adapter` call creates an instance and enumerates the devices; with other GPU
/// tests creating devices on the suite's second thread, an enumeration occasionally found no
/// adapter on the RTX (U-RTVIS, 2026-11-12). Enumerating once, with retries, removes that.
#[cfg(test)]
pub(crate) fn test_adapter() -> &'static wgpu::Adapter {
    static ADAPTER: std::sync::OnceLock<wgpu::Adapter> = std::sync::OnceLock::new();
    ADAPTER.get_or_init(|| {
        let mut tries = 0;
        loop {
            match headless_adapter() {
                Ok(adapter) => break adapter,
                Err(e) if tries == 9 => panic!("{e}"),
                Err(_) => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    })
}

impl Engine {
    /// Creates the device for `descriptor.path` on `adapter` (check
    /// [`Capabilities::query`] first) and configures that path's renderer.
    pub fn new(adapter: &wgpu::Adapter, descriptor: EngineDescriptor) -> Result<Self, Error> {
        let path = descriptor.path;
        if path == RenderPath::HardwareRt && descriptor.experimental_ray_query.is_none() {
            return Err(Error::InvalidInput(
                "HardwareRt needs the ExperimentalRayQuery acknowledgment",
            ));
        }
        if !Capabilities::query(adapter).supports(path) {
            return Err(Error::InvalidInput(
                "adapter lacks this render path's features or limits",
            ));
        }
        let (device, queue) = device::request_device(adapter, path)?;
        let options = RendererOptions {
            samples: SampleCount::One,
            anisotropy_clamp: descriptor.texture_anisotropy,
        };
        let format = descriptor.output_format;
        // PC1: load the shared driver cache before any renderer pipeline is built.
        let info = adapter.get_info();
        let device = crate::pipeline_compilation::PipelineDevice::with_shader_checks(
            device,
            descriptor
                .pipeline_cache
                .as_deref()
                .map(|directory| (directory, &info)),
            descriptor.checked_engine_shaders,
        );
        let mut renderer = crate::Renderer::new_configured(
            device,
            queue,
            format,
            descriptor.limits,
            options,
            (path == RenderPath::HardwareRt).then_some(HardwareShadowOptions {
                max_estimated_bytes: Some(8 << 30),
            }),
            PRELUDE,
        )?;
        if path == RenderPath::HardwareRt {
            renderer.set_primary_visibility(PrimaryVisibility::Traced)?;
        }
        let expected = match path {
            RenderPath::HardwareRt => PrimaryVisibility::Traced,
            RenderPath::NonRt => PrimaryVisibility::VisibilityBuffer,
        };
        if renderer.primary_visibility() != expected {
            return Err(Error::InvalidInput(
                "renderer did not select the path's primary visibility",
            ));
        }
        let renderer_first_view = renderer.selected_view();
        let state = State::new(
            renderer.standard_program().clone(),
            settings::ablated(descriptor.settings),
        );
        let mut engine = Self {
            renderer,
            path,
            adapter: info,
            format,
            limits: descriptor.limits,
            state,
            events: Events::default(),
            ids: Default::default(),
            views: Vec::new(),
            spare: Some(renderer_first_view),
        };
        settings::apply(&mut engine.renderer, &engine.state.settings)?;
        // Ticks are explicit from the start: the first frame runs the first tick's work.
        engine.renderer.begin_tick();
        engine.apply_lighting()?;
        Ok(engine)
    }

    pub fn path(&self) -> RenderPath {
        self.path
    }
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter
    }
    /// Drain opt-in CPU compilation timings (`RENDERLAB_STARTUP_PROFILE=1`).
    pub fn take_compilation_timings(&self) -> Vec<crate::pipeline_compilation::CompilationTiming> {
        self.renderer.take_compilation_timings()
    }

    /// The engine's device, for creating surfaces' configurations and the game's own
    /// resources. Submit nothing to its queue that the engine's frames depend on.
    pub fn device(&self) -> &wgpu::Device {
        self.renderer.device()
    }
    /// The engine's queue, for a host's own work on the engine's device after a frame,
    /// such as copying the output into a texture the host shares with another device.
    /// Submissions run in order with the engine's own; submit nothing the engine's
    /// frames depend on.
    pub fn queue(&self) -> &wgpu::Queue {
        self.renderer.queue()
    }
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.format
    }
    /// The low-level renderer, for diagnostics and tuning beyond [`Settings`]. Its API is
    /// not frozen; engine settings changes overwrite overlapping renderer tuning.
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    // ---- Resources ----

    /// Uploads immutable geometry. On the non-RT path this also builds the mesh's
    /// software-ray geometry for GI from its triangles (CPU time proportional to size).
    pub fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error> {
        let mesh = self.admit("mesh upload admission timed out", |r| {
            r.try_upload_mesh(data)
        })?;
        self.insert_mesh(mesh, data.vertices, data.indices)
    }
    /// Uploads geometry with a per-vertex tangent frame, which normal-mapped materials need.
    pub fn create_mesh_with_tangents(
        &mut self,
        data: &MeshData<'_>,
        tangents: &TangentStream,
    ) -> Result<MeshId, Error> {
        let mesh = self.admit("mesh upload admission timed out", |r| {
            r.try_upload_mesh_with_tangents(data, Some(tangents))
        })?;
        self.insert_mesh(mesh, data.vertices, data.indices)
    }
    fn insert_mesh(
        &mut self,
        mesh: Mesh,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> Result<MeshId, Error> {
        if self.path == RenderPath::NonRt {
            let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
            self.renderer
                .set_indirect_geometry(&mesh, &positions, indices)?;
        }
        Ok(self.state.meshes.insert(MeshEntry::Static(mesh)))
    }
    /// Releases the mesh once no object uses it.
    pub fn remove_mesh(&mut self, mesh: MeshId) -> Result<(), Error> {
        self.state.remove_mesh(mesh).map(drop)
    }

    /// Uploads a prepared texture (every mip level; see `renderlab_asset::Texture`).
    pub fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        let data = Arc::new(data);
        let texture = self.admit("texture upload admission timed out", |r| {
            r.try_upload_streamed_texture(data.clone())
        })?;
        Ok(self.state.textures.insert(texture))
    }
    /// Materials keep their own textures alive; this only drops the engine's handle.
    pub fn remove_texture(&mut self, texture: TextureId) -> Result<(), Error> {
        self.state.remove_texture(texture).map(drop)
    }

    /// Composes a shading program (WGSL `surface` and `shade`, optional `struct
    /// Parameters`, `scatter` and `emission`; docs/api.md) for both paths' stages now, so
    /// errors name program lines. Its pipelines compile when a frame first uses it.
    pub fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        let program = self
            .renderer
            .try_create_program(&ProgramDescriptor { label, source })?;
        Ok(self.state.programs.insert(program))
    }
    /// Replaces a program's source transactionally: on rejection its last working
    /// pipelines keep rendering and the diagnostic names program lines. Materials keep
    /// their parameter values by member name.
    pub fn reload_program(
        &mut self,
        program: ProgramId,
        source: &str,
    ) -> Result<ReloadReport, ReloadError> {
        let handle = self
            .state
            .programs
            .get(program)
            .map_err(|e| ReloadError::Rejected {
                diagnostic: e.to_string(),
                compile: Duration::ZERO,
            })?
            .clone();
        self.renderer.reload_program(&handle, source)
    }
    /// The built-in photoreal program (shaders/programs/standard.wgsl), which materials
    /// without a program use.
    pub fn standard_program(&self) -> ProgramId {
        self.state.standard
    }
    /// Materials keep their program alive; this only drops the engine's handle.
    pub fn remove_program(&mut self, program: ProgramId) -> Result<(), Error> {
        self.state.remove_program(program).map(drop)
    }

    pub fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        let program_id = desc.program.unwrap_or(self.state.standard);
        let program = self.state.programs.get(program_id)?;
        let binding = |slot: Option<TextureRef>| {
            slot.map(|t| {
                self.state
                    .textures
                    .get(t.texture)
                    .map(|texture| TextureBinding {
                        texture,
                        sampler: t.sampler,
                    })
            })
            .transpose()
        };
        let descriptor = MaterialDescriptor {
            base_color: desc.base_color,
            metallic: desc.metallic,
            roughness: desc.roughness,
            base_color_texture: binding(desc.base_color_texture)?,
            metallic_roughness_texture: binding(desc.metallic_roughness_texture)?,
            normal_texture: binding(desc.normal_texture)?,
            normal_scale: desc.normal_scale,
            double_sided: desc.double_sided,
            thin: desc.thin,
            program: Some(program),
            parameters: desc.parameters,
            transparency: desc.transparency,
        };
        let deadline = Instant::now() + ADMISSION;
        let material = loop {
            if let Some(m) = self.renderer.try_create_material(&descriptor)? {
                break m;
            }
            if Instant::now() > deadline {
                return Err(Error::InvalidInput("material admission timed out"));
            }
            stash(&mut self.events, self.renderer.poll()?);
            std::thread::yield_now();
        };
        Ok(self.state.materials.insert(MaterialEntry {
            material,
            program: program_id,
        }))
    }
    /// Sets named `Parameters` members of a material, keeping its others: a buffer write,
    /// no shader compile.
    pub fn set_material_parameters(
        &mut self,
        material: MaterialId,
        values: &[(&str, Value)],
    ) -> Result<(), Error> {
        let entry = self.state.materials.get(material)?;
        self.renderer.try_update_parameters(&entry.material, values)
    }
    pub fn material_program(&self, material: MaterialId) -> Result<ProgramId, Error> {
        Ok(self.state.materials.get(material)?.program)
    }
    /// Releases the material once no object uses it.
    pub fn remove_material(&mut self, material: MaterialId) -> Result<(), Error> {
        self.state.remove_material(material).map(drop)
    }

    /// Uploads a prepared HDR environment (RLE1/RLE2, `renderlab_asset::environment`) for
    /// [`Sky::Map`]. Raise `Renderer::set_environment_limits` first for maps above the
    /// default policy.
    pub fn create_environment(
        &mut self,
        data: &PreparedEnvironment,
    ) -> Result<EnvironmentId, Error> {
        let map = self.admit("environment upload admission timed out", |r| {
            r.try_upload_environment(data)
        })?;
        Ok(self.state.environments.insert(map))
    }
    /// The renderer keeps a selected sky map alive until the sky changes.
    pub fn remove_environment(&mut self, environment: EnvironmentId) -> Result<(), Error> {
        self.state.remove_environment(environment).map(drop)
    }

    /// Uploads a prepared asset (`renderlab_asset::decode` of a cooked `.rla`): its
    /// textures, materials and meshes (one per primitive), and one object per placed
    /// primitive. The asset's vertex data moves into the upload.
    pub fn add_asset(&mut self, mut asset: crate::asset::Asset) -> Result<AssetHandles, Error> {
        let mut handles = AssetHandles::default();
        for texture in std::mem::take(&mut asset.textures) {
            handles.textures.push(self.create_texture(texture)?);
        }
        for m in &asset.materials {
            let desc = asset_material(m, &handles.textures)?;
            handles.materials.push(self.create_material(&desc)?);
        }
        // Mesh preparation (receiver adjacency) runs for every primitive at once.
        let mut shapes = Vec::new();
        let mut primitives = Vec::new();
        for mesh in &mut asset.meshes {
            for p in &mut mesh.primitives {
                let material =
                    *handles
                        .materials
                        .get(p.material as usize)
                        .ok_or(Error::InvalidInput(
                            "asset primitive names a missing material",
                        ))?;
                let geometry = (self.path == RenderPath::NonRt).then(|| {
                    let positions: Vec<[f32; 3]> = p.vertices.iter().map(|v| v.position).collect();
                    (positions, p.indices.clone())
                });
                shapes.push((material, geometry));
                primitives.push(crate::asset::Primitive {
                    vertices: std::mem::take(&mut p.vertices),
                    tangents: p.tangents.take(),
                    indices: std::mem::take(&mut p.indices),
                    material: p.material,
                });
            }
        }
        let uploads = self.renderer.prepare_mesh_uploads(primitives)?;
        let mut flat = Vec::with_capacity(uploads.len());
        for (upload, (material, geometry)) in uploads.iter().zip(shapes) {
            let mesh = self.admit("mesh upload admission timed out", |r| {
                r.try_upload_prepared_mesh(upload)
            })?;
            if let Some((positions, indices)) = geometry {
                self.renderer
                    .set_indirect_geometry(&mesh, &positions, &indices)?;
            }
            flat.push((self.state.meshes.insert(MeshEntry::Static(mesh)), material));
        }
        let mut next = flat.iter();
        for mesh in &asset.meshes {
            handles.meshes.push(
                next.by_ref()
                    .take(mesh.primitives.len())
                    .map(|m| m.0)
                    .collect(),
            );
        }
        let first: Vec<usize> = asset
            .meshes
            .iter()
            .scan(0, |at, m| {
                let start = *at;
                *at += m.primitives.len();
                Some(start)
            })
            .collect();
        for placement in &asset.placements {
            let m = placement.mesh as usize;
            let transform = Mat4::from_cols_array_2d(&placement.transform);
            let mut objects = Vec::new();
            for &(mesh, material) in &flat[first[m]..first[m] + asset.meshes[m].primitives.len()] {
                objects.push(self.spawn(mesh, material, transform)?);
            }
            handles.objects.push(objects);
        }
        Ok(handles)
    }

    /// Uploads one skinned primitive (SK1) as an independently posed mesh. Objects spawned
    /// with it share its pose; spawn from another skinned mesh for another pose. Until
    /// [`set_joint_palette`](Self::set_joint_palette) it holds its bind pose.
    pub fn create_skinned_mesh(
        &mut self,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<MeshId, Error> {
        let mesh = self.admit("skinned mesh upload admission timed out", |r| {
            r.try_upload_skinned_mesh(primitive, influences, joint_count)
        })?;
        Ok(self.state.meshes.insert(MeshEntry::Skinned(mesh)))
    }
    /// Poses a skinned mesh with mesh-local joint matrices, uploaded with the next frame:
    /// `AnimationPose::joint_palette` gives them from a cooked clip (`ClipPlayer`).
    pub fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error> {
        match self.state.meshes.get_mut(mesh)? {
            MeshEntry::Skinned(skinned) => skinned.set_joint_palette(palette),
            MeshEntry::Static(_) => Err(Error::InvalidInput("mesh is not skinned")),
        }
    }
    /// Uploads a cooked skinned asset (`renderlab_asset::skin::decode`, an RLS file from the
    /// cooker's skin recipe): its textures, materials (double-sided, as the shadowed paths
    /// need) and one posable mesh per skinned primitive of each binding. Spawn objects from
    /// the returned meshes; its static meshes and placements are not added.
    pub fn add_skinned_asset(&mut self, asset: &SkinnedAsset) -> Result<SkinnedHandles, Error> {
        Scene::add_skinned_asset(self, asset)
    }

    // ---- Particles ----

    /// Creates a GPU particle emitter (TP1), simulated and drawn in every frame until
    /// removed. `sprite` is its optional texture; leave `desc.texture` None.
    pub fn create_emitter(
        &mut self,
        desc: &EmitterDescriptor<'_>,
        sprite: Option<TextureId>,
    ) -> Result<EmitterId, Error> {
        if desc.texture.is_some() {
            return Err(Error::InvalidInput(
                "pass an emitter's sprite as a TextureId",
            ));
        }
        let texture = sprite.map(|t| self.state.textures.get(t)).transpose()?;
        let emitter = self
            .renderer
            .try_create_emitter(&EmitterDescriptor { texture, ..*desc })?;
        Ok(self.state.emitters.insert(emitter))
    }
    /// Changes a live emitter; its particles continue. Its capacity and whether it has a
    /// sprite must stay the same.
    pub fn update_emitter(
        &mut self,
        emitter: EmitterId,
        desc: &EmitterDescriptor<'_>,
        sprite: Option<TextureId>,
    ) -> Result<(), Error> {
        if desc.texture.is_some() {
            return Err(Error::InvalidInput(
                "pass an emitter's sprite as a TextureId",
            ));
        }
        let texture = sprite.map(|t| self.state.textures.get(t)).transpose()?;
        let handle = self.state.emitters.get(emitter)?;
        self.renderer
            .try_update_emitter(handle, &EmitterDescriptor { texture, ..*desc })
    }
    pub fn remove_emitter(&mut self, emitter: EmitterId) -> Result<(), Error> {
        self.state.remove_emitter(emitter).map(drop)
    }

    // ---- Objects ----

    /// Places a mesh with a material. Objects sharing a mesh and material are drawn
    /// together; spawning or despawning re-uploads that group's transforms next frame.
    pub fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error> {
        self.state
            .spawn(mesh, material, transform, self.limits.max_instances)
    }
    /// Moves an object: a transform write on the next frame, no reallocation.
    pub fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        self.state.set_transform(object, transform)
    }
    pub fn transform(&self, object: ObjectId) -> Result<Mat4, Error> {
        self.state.transform(object)
    }
    pub fn despawn(&mut self, object: ObjectId) -> Result<(), Error> {
        self.state.despawn(object)?;
        self.ids.objects.forget(object);
        Ok(())
    }
    pub fn object_count(&self) -> usize {
        self.state.object_count()
    }

    // ---- Decals ----

    /// Replaces the scene's projected box decals atomically; empty removes them
    /// (DC1, docs/m7-dc1.md). Each `Decal` maps the local cube [-0.5, 0.5]³ to world
    /// space: local XY carries its UVs, local +Z is the receiving surface's preferred
    /// normal, and the Z scale is the projection depth. Decals edit material inputs before
    /// a program's `surface` reads them, on both paths, and GI bounce sees them. Their
    /// textures are copied at this call. The decal grid's resolution stays on
    /// `renderer_mut().set_decals_with_settings`; this call uses its default.
    pub fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        self.renderer.set_decals(decals)
    }

    // ---- Lighting, sky and settings ----

    /// Sets or removes the sun, the scene's one shadowed directional light.
    pub fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.state.sun, sun);
        self.apply_lighting()
            .inspect_err(|_| self.state.sun = previous)
    }
    pub fn sun(&self) -> Option<Sun> {
        self.state.sun
    }
    /// Replaces the local lights (point and spot). The most important lights shadow
    /// automatically; `source_radius` sets their penumbra size (zero keeps hard shadows).
    pub fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.state.lights, lights.to_vec());
        self.apply_lighting()
            .inspect_err(|_| self.state.lights = previous)
    }
    pub fn lights(&self) -> &[Light] {
        &self.state.lights
    }
    pub fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.state.sky, sky);
        self.apply_lighting()
            .inspect_err(|_| self.state.sky = previous)
    }
    pub fn sky(&self) -> Sky {
        self.state.sky
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        let settings = settings::ablated(settings);
        if settings == self.state.settings {
            return Ok(());
        }
        settings::atmosphere(self.state.sky, &settings).validate()?;
        // Validate before touching the renderer (docs/api.md): a rejected sky_light must
        // leave the stored settings unchanged, or later set_sun/set_sky/light edits fail.
        if !(settings.sky_light.is_finite() && settings.sky_light >= 0.0) {
            return Err(Error::InvalidInput(
                "sky light must be finite and nonnegative",
            ));
        }
        settings::apply(&mut self.renderer, &settings)?;
        self.state.settings = settings;
        // Views' looks depend on the settings (temporal reconstruction): reapply on render.
        for view in &mut self.views {
            view.applied = false;
        }
        // The view-path media live with the lighting and atmosphere.
        self.apply_lighting()
    }
    pub fn settings(&self) -> Settings {
        self.state.settings
    }
    /// Publishes the sun, lights, sky and exposure. Everything is validated before the
    /// renderer changes, so a rejected call leaves the previous lighting in place.
    fn apply_lighting(&mut self) -> Result<(), Error> {
        let (lighting, shadows) = settings::lighting(
            self.state.sun,
            &self.state.lights,
            self.state.sky,
            self.selected_exposure(),
        )?;
        let atmosphere = settings::atmosphere(self.state.sky, &self.state.settings);
        atmosphere.validate()?;
        let (map, parameters) = match self.state.sky {
            Sky::Atmosphere(_) => {
                let sky_light = self.state.settings.sky_light;
                if !(sky_light.is_finite() && sky_light >= 0.0) {
                    return Err(Error::InvalidInput(
                        "sky light must be finite and nonnegative",
                    ));
                }
                // The atmosphere's procedural environment lights at this intensity; its
                // drawn sky does not read it.
                let parameters = EnvironmentParameters {
                    intensity: sky_light,
                    ..EnvironmentParameters::default()
                };
                (None, parameters)
            }
            Sky::Uniform(_) => (None, EnvironmentParameters::default()),
            Sky::Map {
                map,
                rotation_y,
                intensity,
                background,
            } => {
                if !rotation_y.is_finite() || !intensity.is_finite() || intensity < 0.0 {
                    return Err(Error::InvalidInput(
                        "sky rotation must be finite and intensity finite and nonnegative",
                    ));
                }
                if let Background::ScaledSky(scale) = background
                    && !(scale.is_finite() && scale >= 0.0)
                {
                    return Err(Error::InvalidInput(
                        "sky background scale must be finite and nonnegative",
                    ));
                }
                let parameters = EnvironmentParameters {
                    rotation_y_radians: rotation_y,
                    intensity,
                };
                (Some(self.state.environments.get(map)?), parameters)
            }
        };
        self.renderer
            .set_lighting_and_shadows(&lighting, &shadows)?;
        self.renderer.set_environment(map, parameters)?;
        let background = match self.state.sky {
            Sky::Map { background, .. } => Some(background),
            _ => None,
        };
        self.renderer.set_visible_environment(matches!(
            background,
            Some(Background::Sky | Background::ScaledSky(_))
        ));
        self.renderer
            .set_visible_environment_scale(match background {
                Some(Background::ScaledSky(scale)) => Some(scale),
                _ => None,
            });
        self.renderer.set_atmosphere(atmosphere)
    }

    // ---- Frames ----

    /// Renders the renderer's selected view into `color` with the scene's batches.
    fn render_selected(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
        mode: ViewMode,
    ) -> Result<FrameReport, Error> {
        let ids: Vec<_> = self.state.batches.ids().collect();
        for id in ids {
            let batch = self.state.batches.get(id)?;
            if batch.instances.is_none() {
                let models = batch.models.clone();
                let instances = self.admit("instance upload admission timed out", |r| {
                    r.try_upload_instances(&models)
                })?;
                self.state.batches.get_mut(id)?.instances = Some(instances);
            }
        }
        let Self {
            renderer,
            state:
                State {
                    meshes,
                    materials,
                    batches,
                    ..
                },
            events,
            ..
        } = self;
        let mut draws = Vec::with_capacity(batches.len());
        for batch in batches.values_mut() {
            let mesh = meshes.get(batch.mesh)?.mesh();
            let material = &materials.get(batch.material)?.material;
            let instances = batch.instances.as_mut().expect("uploaded above");
            draws.push(crate::Draw::new(mesh, instances).with_material(material));
        }
        let deadline = Instant::now() + ADMISSION;
        loop {
            if let Some(report) = renderer.try_render_view(color, camera, &mut draws, mode)? {
                settings::log_indirect(renderer)?;
                return Ok(report);
            }
            if Instant::now() > deadline {
                return Err(Error::InvalidInput("frame admission timed out"));
            }
            stash(events, renderer.poll()?);
            std::thread::yield_now();
        }
    }
    /// Presents a surface texture rendered with `render_view_to`.
    pub fn present(&self, frame: wgpu::SurfaceTexture) -> Result<(), Error> {
        self.renderer.present(frame)
    }
    /// Nonblocking progress: completed frames and GPU pass timings since the last poll.
    pub fn poll(&mut self) -> Result<Events, Error> {
        stash(&mut self.events, self.renderer.poll()?);
        // Every view's finished and lost id readbacks.
        let selected = self.renderer.selected_view();
        for view in self.views.iter().map(|v| v.id).collect::<Vec<_>>() {
            self.renderer.select_view(view)?;
            let (ids, lost) = self.renderer.take_ids()?;
            stash_ids(&mut self.events, ids, lost);
        }
        if self.views.iter().any(|v| v.id == selected) {
            self.renderer.select_view(selected)?;
        }
        Ok(std::mem::take(&mut self.events))
    }
    // ---- Object ids (docs/design-id-output.md) ----

    /// The embedder's ids for an object's primitives: disjoint ranges `(first primitive,
    /// count, id)` in any order, so one merged object can hold many entities; one id for
    /// the whole object is `&[(0, u32::MAX, id)]`, and an empty list clears them. `ID_NONE`
    /// and `ID_UNTAGGED` are reserved; primitives outside every range read as untagged.
    /// Every view with ids reports them from its next frame.
    pub fn set_object_ids(&mut self, object: ObjectId, ranges: &[(u32, u32, u32)]) -> Result<(), Error> {
        self.state.transform(object)?;
        self.ids.objects.set(object, ranges)
    }
    /// The view's ID target of its last rendered frame: R32Uint, the view's size, two
    /// layers (0: opaque ids, 1: front ids). None while its ids are off or before a frame.
    pub fn view_ids(&mut self, view: ViewId) -> Option<&wgpu::Texture> {
        self.view_index(view).ok()?;
        self.renderer.select_view(view).ok()?;
        self.renderer.id_texture()
    }
    /// Writes one id layer of the view's last frame into `target`, for an overlay that reads
    /// ids every frame through the colour's handoff. `target` is a single-sample, single-mip
    /// 2D `Rgba8Unorm` (not sRGB) texture of the view's size whose usage includes
    /// `RENDER_ATTACHMENT | COPY_SRC` (the handoff copies from it); anything else is an
    /// error. Each 32-bit id is packed bit-exact little-endian, one texel per output pixel
    /// (R = bits 0-7, G = 8-15, B = 16-23, A = 24-31; read by exact texel fetch): `ID_NONE`
    /// (0) is nothing, `ID_UNTAGGED` (0xFFFF_FFFF) a hit without an id. Queued behind that
    /// frame, nothing waits; call it after each `render_view`/`render_view_to` the overlay
    /// follows. Needs `ViewDesc::outputs.ids`.
    pub fn render_view_ids_to(
        &mut self,
        view: ViewId,
        target: &wgpu::Texture,
        layer: IdLayer,
    ) -> Result<(), Error> {
        self.view_index(view)?;
        self.renderer.select_view(view)?;
        let layer = match layer {
            IdLayer::Opaque => 0,
            IdLayer::Front => 1,
        };
        self.renderer.write_ids_to(target, layer)
    }
    /// Asks for the ids of a rectangle (at most `MAX_ID_RECT` a side) of the view's last
    /// frame without stalling: the copy is queued after that frame, and its result arrives
    /// in `poll()` (`Events::ids`, naming the view and frame), or `Events::ids_lost` when the
    /// copy did not land within `ID_TICKS` ticks (`Engine::tick`). Refused at once while `ID_SLOTS`
    /// of the view's requests are in flight, or before the view rendered a frame with ids.
    pub fn request_ids(&mut self, view: ViewId, rect: IdRect) -> Result<IdRequest, Error> {
        self.view_index(view)?;
        self.renderer.select_view(view)?;
        self.renderer.request_ids(view, rect)
    }
    /// Turns the selected view's ID pass on or off for its next frame and, when on, gives
    /// the renderer this frame's id tables in the order the frame's draws place their
    /// instances.
    pub(super) fn prepare_view_ids(&mut self, ids: bool) -> Result<(), Error> {
        // Measurement switch: ids on for every view of an embedder that does not set them
        // (cost timing through Ara's player, docs/design-id-output.md).
        let ids = ids || std::env::var_os("RENDERLAB_IDS").is_some_and(|v| v == "1");
        if self.renderer.id_output() != ids {
            self.renderer.set_id_output(ids);
        }
        if !ids {
            return Ok(());
        }
        let Self { state, ids, .. } = self;
        let mut batches = Vec::with_capacity(state.batches.len());
        for id in state.batches.ids() {
            let batch = state.batches.get(id)?;
            let material = &state.materials.get(batch.material)?.material;
            batches.push((material.transparency.is_some(), &batch.objects[..]));
        }
        ids.objects.tables(batches.into_iter(), &mut ids.tables);
        self.renderer.set_object_tags(&self.ids.tables);
        Ok(())
    }

    /// Retries a bounded renderer admission while the GPU drains, keeping its events.
    /// `timeout` is the error after `ADMISSION`.
    fn admit<T>(
        &mut self,
        timeout: &'static str,
        mut attempt: impl FnMut(&mut Renderer) -> Result<Option<T>, Error>,
    ) -> Result<T, Error> {
        let deadline = Instant::now() + ADMISSION;
        loop {
            if let Some(value) = attempt(&mut self.renderer)? {
                return Ok(value);
            }
            if Instant::now() > deadline {
                return Err(Error::InvalidInput(timeout));
            }
            stash(&mut self.events, self.renderer.poll()?);
            std::thread::yield_now();
        }
    }
}

/// A cooked asset material as an engine material description, its textures given by the
/// asset's texture order.
fn asset_material(
    m: &crate::asset::Material,
    textures: &[TextureId],
) -> Result<MaterialDesc<'static>, Error> {
    let texture = |t: u32, sampler| -> Result<TextureRef, Error> {
        let texture = *textures.get(t as usize).ok_or(Error::InvalidInput(
            "asset material names a missing texture",
        ))?;
        Ok(TextureRef { texture, sampler })
    };
    Ok(MaterialDesc {
        base_color: m.base_color,
        metallic: m.metallic,
        roughness: m.roughness,
        base_color_texture: m
            .base_color_texture
            .map(|t| texture(t.texture, t.sampler))
            .transpose()?,
        metallic_roughness_texture: m
            .metallic_roughness_texture
            .map(|t| texture(t.texture, t.sampler))
            .transpose()?,
        normal_texture: m
            .normal_texture
            .map(|t| texture(t.texture, t.sampler))
            .transpose()?,
        normal_scale: m.normal_texture.map_or(1.0, |t| t.scale),
        double_sided: m.double_sided,
        ..Default::default()
    })
}

/// Keeps the newest `KEPT_ID_EVENTS` id events for `Engine::poll`, counting the dropped.
fn stash_ids(kept: &mut Events, ids: Vec<IdReadback>, lost: Vec<IdsLost>) {
    kept.ids.extend(ids);
    kept.ids_lost.extend(lost);
    while kept.ids.len() + kept.ids_lost.len() > crate::KEPT_ID_EVENTS {
        if kept.ids.is_empty() {
            kept.ids_lost.remove(0);
        } else {
            kept.ids.remove(0);
        }
        kept.ids_dropped += 1;
    }
}

/// Keeps the newest `KEPT_EVENTS` of each kind for `Engine::poll`.
fn stash(kept: &mut Events, new: Events) {
    fn keep<T>(kept: &mut Vec<T>, new: Vec<T>, dropped: &mut u64) {
        kept.extend(new);
        if kept.len() > KEPT_EVENTS {
            let excess = kept.len() - KEPT_EVENTS;
            kept.drain(..excess);
            *dropped += excess as u64;
        }
    }
    keep(&mut kept.completed, new.completed, &mut kept.dropped);
    keep(&mut kept.gpu, new.gpu, &mut kept.dropped);
    kept.dropped += new.dropped;
    kept.poll_cpu += new.poll_cpu;
}
