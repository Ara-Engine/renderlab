//! The engine-facing API: one [`Engine`] that owns a renderer, its device and the scene.
//!
//! A game creates an engine for a [`RenderPath`], creates resources (meshes, textures,
//! programs, materials, environments) and spawns objects, all named by copyable typed
//! handles ([`Id`]). Each frame it moves objects, sets the camera and calls
//! [`Engine::render`] or [`Engine::render_to`]. Every call blocks only for GPU
//! backpressure; the engine handles admission retries, per-path configuration (traced
//! primaries or the visibility buffer, sun visibility, GI geometry) and draw batching.
//!
//! `use renderlab::engine::*` brings in everything a game needs. The crate root's
//! [`Renderer`] stays available through [`Engine::renderer_mut`] as the expert API: its
//! tuning surface changes as features are measured, while this module stays stable
//! (docs/api.md lists what is frozen).
mod device;
mod ids;
mod settings;

pub use device::{Capabilities, ExperimentalRayQuery, RenderPath};
pub use ids::{
    EmitterId, EnvironmentId, Id, MaterialId, MeshId, ObjectId, ProgramId, TextureId, kind,
};
pub use settings::{Background, Exposure, Quality, Settings, Sky, Sun};

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
    Transparency, Value, Vec3, Vertex, VolumetricFog,
};

use crate::{
    Emitter, EnvironmentMap, EnvironmentParameters, HardwareShadowOptions, Instances, Material,
    MaterialDescriptor, Mesh, PrimaryVisibility, Program, ProgramDescriptor, RendererOptions,
    SampleCount, SkinnedMeshInstance, Texture, TextureBinding, ViewMode,
};
use ids::Slots;
use std::{
    collections::HashMap,
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

struct MaterialEntry {
    material: Material,
    program: ProgramId,
}
struct ObjectEntry {
    batch: Id<kind::Batch>,
    slot: usize,
}
/// Objects sharing a mesh and material: one instance collection, one draw.
struct Batch {
    mesh: MeshId,
    material: MaterialId,
    models: Vec<Mat4>,
    objects: Vec<ObjectId>,
    /// None until the next frame uploads the current `models` (after spawn/despawn).
    instances: Option<Instances>,
}

/// The renderer, its device, the scene's resources and objects, and the frame settings.
pub struct Engine {
    renderer: Renderer,
    path: RenderPath,
    adapter: wgpu::AdapterInfo,
    format: wgpu::TextureFormat,
    limits: Limits,
    output: Option<wgpu::Texture>,
    meshes: Slots<kind::Mesh, MeshEntry>,
    textures: Slots<kind::Texture, Texture>,
    programs: Slots<kind::Program, Program>,
    materials: Slots<kind::Material, MaterialEntry>,
    environments: Slots<kind::Environment, EnvironmentMap>,
    objects: Slots<kind::Object, ObjectEntry>,
    emitters: Slots<kind::Emitter, Emitter>,
    batches: Slots<kind::Batch, Batch>,
    batch_keys: HashMap<(MeshId, MaterialId), Id<kind::Batch>>,
    standard: ProgramId,
    sun: Option<Sun>,
    lights: Vec<Light>,
    sky: Sky,
    settings: Settings,
    events: Events,
}

/// Instance policy for Renderlab's bounded engine-owned indirect dispatch arguments.
/// External adapters should use these flags too; explicit wgpu environment overrides apply.
pub fn instance_flags() -> wgpu::InstanceFlags {
    wgpu::InstanceFlags::default()
        .difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
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
        let mut programs = Slots::new("unknown or removed program");
        let standard = programs.insert(renderer.standard_program().clone());
        let mut engine = Self {
            renderer,
            path,
            adapter: info,
            format,
            limits: descriptor.limits,
            output: None,
            meshes: Slots::new("unknown or removed mesh"),
            textures: Slots::new("unknown or removed texture"),
            programs,
            materials: Slots::new("unknown or removed material"),
            environments: Slots::new("unknown or removed environment"),
            objects: Slots::new("unknown or removed object"),
            emitters: Slots::new("unknown or removed emitter"),
            batches: Slots::new("internal: unknown batch"),
            batch_keys: HashMap::new(),
            standard,
            sun: None,
            lights: Vec::new(),
            sky: Sky::default(),
            settings: descriptor.settings,
            events: Events::default(),
        };
        settings::apply(&mut engine.renderer, path, &engine.settings)?;
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
        Ok(self.meshes.insert(MeshEntry::Static(mesh)))
    }
    /// Releases the mesh once no object uses it.
    pub fn remove_mesh(&mut self, mesh: MeshId) -> Result<(), Error> {
        self.meshes.get(mesh)?;
        if self.batch_keys.keys().any(|(m, _)| *m == mesh) {
            return Err(Error::InvalidInput("mesh is still used by objects"));
        }
        self.meshes.remove(mesh).map(drop)
    }

    /// Uploads a prepared texture (every mip level; see `renderlab_asset::Texture`).
    pub fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        let data = Arc::new(data);
        let texture = self.admit("texture upload admission timed out", |r| {
            r.try_upload_streamed_texture(data.clone())
        })?;
        Ok(self.textures.insert(texture))
    }
    /// Materials keep their own textures alive; this only drops the engine's handle.
    pub fn remove_texture(&mut self, texture: TextureId) -> Result<(), Error> {
        self.textures.remove(texture).map(drop)
    }

    /// Composes a shading program (WGSL `surface` and `shade`, optional `struct
    /// Parameters`, `scatter` and `emission`; docs/api.md) for both paths' stages now, so
    /// errors name program lines. Its pipelines compile when a frame first uses it.
    pub fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        let program = self
            .renderer
            .try_create_program(&ProgramDescriptor { label, source })?;
        Ok(self.programs.insert(program))
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
        self.standard
    }
    /// Materials keep their program alive; this only drops the engine's handle.
    pub fn remove_program(&mut self, program: ProgramId) -> Result<(), Error> {
        if program == self.standard {
            return Err(Error::InvalidInput(
                "the standard program cannot be removed",
            ));
        }
        self.programs.remove(program).map(drop)
    }

    pub fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        let program_id = desc.program.unwrap_or(self.standard);
        let program = self.programs.get(program_id)?;
        let binding = |slot: Option<TextureRef>| {
            slot.map(|t| {
                self.textures.get(t.texture).map(|texture| TextureBinding {
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
        Ok(self.materials.insert(MaterialEntry {
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
        let entry = self.materials.get(material)?;
        self.renderer.try_update_parameters(&entry.material, values)
    }
    pub fn material_program(&self, material: MaterialId) -> Result<ProgramId, Error> {
        Ok(self.materials.get(material)?.program)
    }
    /// Releases the material once no object uses it.
    pub fn remove_material(&mut self, material: MaterialId) -> Result<(), Error> {
        self.materials.get(material)?;
        if self.batch_keys.keys().any(|(_, m)| *m == material) {
            return Err(Error::InvalidInput("material is still used by objects"));
        }
        self.materials.remove(material).map(drop)
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
        Ok(self.environments.insert(map))
    }
    /// The renderer keeps a selected sky map alive until the sky changes.
    pub fn remove_environment(&mut self, environment: EnvironmentId) -> Result<(), Error> {
        self.environments.remove(environment).map(drop)
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
            flat.push((self.meshes.insert(MeshEntry::Static(mesh)), material));
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
        Ok(self.meshes.insert(MeshEntry::Skinned(mesh)))
    }
    /// Poses a skinned mesh with mesh-local joint matrices, uploaded with the next frame:
    /// `AnimationPose::joint_palette` gives them from a cooked clip (`ClipPlayer`).
    pub fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error> {
        match self.meshes.get_mut(mesh)? {
            MeshEntry::Skinned(skinned) => skinned.set_joint_palette(palette),
            MeshEntry::Static(_) => Err(Error::InvalidInput("mesh is not skinned")),
        }
    }
    /// Uploads a cooked skinned asset (`renderlab_asset::skin::decode`, an RLS file from the
    /// cooker's skin recipe): its textures, materials (double-sided, as the shadowed paths
    /// need) and one posable mesh per skinned primitive of each binding. Spawn objects from
    /// the returned meshes; its static meshes and placements are not added.
    pub fn add_skinned_asset(&mut self, asset: &SkinnedAsset) -> Result<SkinnedHandles, Error> {
        let mut handles = SkinnedHandles::default();
        for texture in &asset.geometry.textures {
            handles.textures.push(self.create_texture(texture.clone())?);
        }
        for m in &asset.geometry.materials {
            let desc = MaterialDesc {
                double_sided: true,
                ..asset_material(m, &handles.textures)?
            };
            handles.materials.push(self.create_material(&desc)?);
        }
        for binding in &asset.bindings {
            let missing = Error::InvalidInput("skinned binding names a missing mesh or skin");
            let joints = asset
                .skins
                .get(binding.skin as usize)
                .ok_or(missing)?
                .joints
                .len();
            let mesh = asset
                .geometry
                .meshes
                .get(binding.mesh as usize)
                .ok_or(Error::InvalidInput("skinned binding names a missing mesh"))?;
            let mut parts = Vec::with_capacity(mesh.primitives.len());
            for (p, primitive) in mesh.primitives.iter().enumerate() {
                let influences = asset
                    .influences
                    .get(binding.mesh as usize)
                    .and_then(|m| m.get(p))
                    .and_then(Option::as_ref)
                    .ok_or(Error::InvalidInput("skinned primitive has no influences"))?;
                let id = self.create_skinned_mesh(primitive, influences, joints as u32)?;
                let material = *handles.materials.get(primitive.material as usize).ok_or(
                    Error::InvalidInput("skinned primitive names a missing material"),
                )?;
                parts.push((id, material));
            }
            handles.bindings.push(parts);
        }
        Ok(handles)
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
        let texture = sprite.map(|t| self.textures.get(t)).transpose()?;
        let emitter = self
            .renderer
            .try_create_emitter(&EmitterDescriptor { texture, ..*desc })?;
        Ok(self.emitters.insert(emitter))
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
        let texture = sprite.map(|t| self.textures.get(t)).transpose()?;
        let handle = self.emitters.get(emitter)?;
        self.renderer
            .try_update_emitter(handle, &EmitterDescriptor { texture, ..*desc })
    }
    pub fn remove_emitter(&mut self, emitter: EmitterId) -> Result<(), Error> {
        self.emitters.remove(emitter).map(drop)
    }
    /// Particle simulation time per rendered frame, in seconds (default 1/60). Set it to
    /// the game's frame time; zero freezes particles.
    pub fn set_particle_step(&mut self, seconds: f32) -> Result<(), Error> {
        self.renderer.set_particle_step(seconds)
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
        self.meshes.get(mesh)?;
        self.materials.get(material)?;
        if !transform.is_finite() {
            return Err(Error::InvalidInput("object transform must be finite"));
        }
        let batch = match self.batch_keys.get(&(mesh, material)) {
            Some(&batch) => batch,
            None => {
                let batch = self.batches.insert(Batch {
                    mesh,
                    material,
                    models: Vec::new(),
                    objects: Vec::new(),
                    instances: None,
                });
                self.batch_keys.insert((mesh, material), batch);
                batch
            }
        };
        let entry = self.batches.get_mut(batch)?;
        if entry.models.len() >= self.limits.max_instances as usize {
            return Err(Error::InvalidInput(
                "objects per mesh-material pair exceed Limits::max_instances",
            ));
        }
        let slot = entry.models.len();
        let object = self.objects.insert(ObjectEntry { batch, slot });
        entry.models.push(transform);
        entry.objects.push(object);
        entry.instances = None;
        Ok(object)
    }
    /// Moves an object: a transform write on the next frame, no reallocation.
    pub fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        let &ObjectEntry { batch, slot } = self.objects.get(object)?;
        let entry = self.batches.get_mut(batch)?;
        if let Some(instances) = &mut entry.instances {
            instances.set(slot, transform)?;
        } else if !transform.is_finite() {
            return Err(Error::InvalidInput("object transform must be finite"));
        }
        entry.models[slot] = transform;
        Ok(())
    }
    pub fn transform(&self, object: ObjectId) -> Result<Mat4, Error> {
        let entry = self.objects.get(object)?;
        Ok(self.batches.get(entry.batch)?.models[entry.slot])
    }
    pub fn despawn(&mut self, object: ObjectId) -> Result<(), Error> {
        let ObjectEntry { batch, slot } = self.objects.remove(object)?;
        let entry = self.batches.get_mut(batch)?;
        entry.models.swap_remove(slot);
        entry.objects.swap_remove(slot);
        entry.instances = None;
        if let Some(&moved) = entry.objects.get(slot) {
            self.objects.get_mut(moved)?.slot = slot;
        }
        if entry.objects.is_empty() {
            let Batch { mesh, material, .. } = self.batches.remove(batch)?;
            self.batch_keys.remove(&(mesh, material));
        }
        Ok(())
    }
    pub fn object_count(&self) -> usize {
        self.objects.len()
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
        let previous = std::mem::replace(&mut self.sun, sun);
        self.apply_lighting().inspect_err(|_| self.sun = previous)
    }
    pub fn sun(&self) -> Option<Sun> {
        self.sun
    }
    /// Replaces the local lights (point and spot). The most important lights shadow
    /// automatically; `source_radius` sets their penumbra size (zero keeps hard shadows).
    pub fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.lights, lights.to_vec());
        self.apply_lighting()
            .inspect_err(|_| self.lights = previous)
    }
    pub fn lights(&self) -> &[Light] {
        &self.lights
    }
    pub fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.sky, sky);
        self.apply_lighting().inspect_err(|_| self.sky = previous)
    }
    pub fn sky(&self) -> Sky {
        self.sky
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        if settings == self.settings {
            return Ok(());
        }
        settings::atmosphere(self.sky, &settings).validate()?;
        settings::apply(&mut self.renderer, self.path, &settings)?;
        self.settings = settings;
        // Exposure and the view-path media live with the lighting and atmosphere.
        self.apply_lighting()
    }
    pub fn settings(&self) -> Settings {
        self.settings
    }
    /// Sets or clears the post pass's display-linear 3D colour LUT (PP1), applied at
    /// `Settings::post.lut_strength`.
    pub fn set_colour_lut(&mut self, lut: Option<ColourLut>) -> Result<(), Error> {
        self.renderer.set_colour_lut(lut)
    }
    /// Publishes the sun, lights, sky and exposure. Everything is validated before the
    /// renderer changes, so a rejected call leaves the previous lighting in place.
    fn apply_lighting(&mut self) -> Result<(), Error> {
        let (lighting, shadows) =
            settings::lighting(self.sun, &self.lights, self.sky, self.settings.exposure)?;
        let atmosphere = settings::atmosphere(self.sky, &self.settings);
        atmosphere.validate()?;
        let (map, parameters) = match self.sky {
            Sky::Atmosphere(_) | Sky::Uniform(_) => (None, EnvironmentParameters::default()),
            Sky::Map {
                map,
                rotation_y,
                intensity,
                ..
            } => {
                if !rotation_y.is_finite() || !intensity.is_finite() || intensity < 0.0 {
                    return Err(Error::InvalidInput(
                        "sky rotation must be finite and intensity finite and nonnegative",
                    ));
                }
                let parameters = EnvironmentParameters {
                    rotation_y_radians: rotation_y,
                    intensity,
                };
                (Some(self.environments.get(map)?), parameters)
            }
        };
        self.renderer
            .set_lighting_and_shadows(&lighting, &shadows)?;
        self.renderer.set_environment(map, parameters)?;
        self.renderer.set_visible_environment(matches!(
            self.sky,
            Sky::Map {
                background: Background::Sky,
                ..
            }
        ));
        self.renderer.set_atmosphere(atmosphere)
    }

    // ---- Frames ----

    /// Creates or resizes the engine's own colour output (for offscreen rendering and
    /// [`capture`](Self::capture)). Windowed games can use `render_to` instead.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput("output extent must be nonzero"));
        }
        if self
            .output
            .as_ref()
            .is_some_and(|t| (t.width(), t.height()) == (width, height))
        {
            return Ok(());
        }
        self.output = Some(self.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("renderlab engine output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }));
        Ok(())
    }
    /// The engine's own output, after `resize`.
    pub fn output(&self) -> Option<&wgpu::Texture> {
        self.output.as_ref()
    }
    /// Renders the scene from `camera` into the engine's output.
    pub fn render(&mut self, camera: &Camera) -> Result<FrameReport, Error> {
        let output = self.output.clone().ok_or(Error::InvalidInput(
            "call Engine::resize before Engine::render",
        ))?;
        self.render_to(&output, camera)
    }
    /// Renders the scene from `camera` into `color`: a single-sample 2D render attachment
    /// in the engine's output format, such as a surface texture (then call `present`).
    /// The camera's aspect follows `color`.
    pub fn render_to(
        &mut self,
        color: &wgpu::Texture,
        camera: &Camera,
    ) -> Result<FrameReport, Error> {
        let ids: Vec<_> = self.batches.ids().collect();
        for id in ids {
            let batch = self.batches.get(id)?;
            if batch.instances.is_none() {
                let models = batch.models.clone();
                let instances = self.admit("instance upload admission timed out", |r| {
                    r.try_upload_instances(&models)
                })?;
                self.batches.get_mut(id)?.instances = Some(instances);
            }
        }
        let Self {
            renderer,
            meshes,
            materials,
            batches,
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
            if let Some(report) =
                renderer.try_render_view(color, camera, &mut draws, ViewMode::Lit)?
            {
                return Ok(report);
            }
            if Instant::now() > deadline {
                return Err(Error::InvalidInput("frame admission timed out"));
            }
            stash(events, renderer.poll()?);
            std::thread::yield_now();
        }
    }
    /// Starts the next frame without history: a camera cut or teleport. Temporal
    /// reconstruction and volumetric fog history restart, and streamed textures verify the
    /// new view; the GI cache refills its newly seen cells over a few frames on its own.
    /// (Fog resets here, not in `cut_reconstruction`, which transmission changes also call.)
    pub fn cut(&mut self) {
        self.renderer.cut_reconstruction();
        self.renderer.reset_atmosphere_history();
        self.renderer.mark_camera_cut();
    }
    /// Presents a surface texture rendered with `render_to`.
    pub fn present(&self, frame: wgpu::SurfaceTexture) -> Result<(), Error> {
        self.renderer.present(frame)
    }
    /// Nonblocking progress: completed frames and GPU pass timings since the last poll.
    pub fn poll(&mut self) -> Result<Events, Error> {
        stash(&mut self.events, self.renderer.poll()?);
        Ok(std::mem::take(&mut self.events))
    }
    /// Reads back the engine's output after the last frame (blocking). Needs an RGBA8
    /// output format.
    pub fn capture(&mut self) -> Result<Image, Error> {
        let output = self.output.clone().ok_or(Error::InvalidInput(
            "call Engine::resize before Engine::capture",
        ))?;
        let mut capture = self.admit("capture admission timed out", |r| r.try_capture(&output))?;
        let deadline = Instant::now() + ADMISSION;
        loop {
            stash(&mut self.events, self.renderer.poll()?);
            if let Some(rgba8) = capture.try_read()? {
                return Ok(Image {
                    width: output.width(),
                    height: output.height(),
                    rgba8,
                });
            }
            if Instant::now() > deadline {
                return Err(Error::InvalidInput("capture readback timed out"));
            }
            std::thread::yield_now();
        }
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
