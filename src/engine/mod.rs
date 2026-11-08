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
pub use ids::{EnvironmentId, Id, MaterialId, MeshId, ObjectId, ProgramId, TextureId, kind};
pub use settings::{Exposure, Quality, Settings, Sky, Sun};

pub use crate::asset::{
    Sampler, TangentStream, Texture as TextureData, environment::PreparedEnvironment,
};
pub use crate::{
    Camera, Error, Events, FrameId, FrameReport, Light, Limits, Mat4, MeshData, ReloadError,
    ReloadReport, Renderer, Value, Vec3, Vertex,
};

use crate::{
    EnvironmentMap, EnvironmentParameters, HardwareShadowOptions, Instances, Material,
    MaterialDescriptor, Mesh, PrimaryVisibility, Program, ProgramDescriptor, RendererOptions,
    SampleCount, Texture, TextureBinding, ViewMode,
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
    /// Directory for the persistent program pipeline cache, so a relaunch or a reverted
    /// shader edit is a cache hit. None keeps the cache in memory only.
    pub pipeline_cache: Option<PathBuf>,
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

/// An RGBA8 readback of the engine's output ([`Engine::capture`]).
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Tightly packed rows, top row first, in the output format's encoding.
    pub rgba8: Vec<u8>,
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
    meshes: Slots<kind::Mesh, Mesh>,
    textures: Slots<kind::Texture, Texture>,
    programs: Slots<kind::Program, Program>,
    materials: Slots<kind::Material, MaterialEntry>,
    environments: Slots<kind::Environment, EnvironmentMap>,
    objects: Slots<kind::Object, ObjectEntry>,
    batches: Slots<kind::Batch, Batch>,
    batch_keys: HashMap<(MeshId, MaterialId), Id<kind::Batch>>,
    standard: ProgramId,
    sun: Option<Sun>,
    lights: Vec<Light>,
    sky: Sky,
    settings: Settings,
    events: Events,
}

/// A Vulkan adapter for offscreen use (no surface), preferring a discrete GPU.
pub fn headless_adapter() -> Result<wgpu::Adapter, Error> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
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
        let mut renderer = match path {
            RenderPath::HardwareRt => {
                let mut r = Renderer::new_with_hardware_shadows(
                    device,
                    queue,
                    format,
                    descriptor.limits,
                    options,
                    HardwareShadowOptions {
                        max_estimated_bytes: Some(8 << 30),
                    },
                    PRELUDE,
                )?;
                r.set_primary_visibility(PrimaryVisibility::Traced)?;
                r
            }
            // A device with the visibility-buffer and Virtual-shadow features selects
            // both at creation.
            RenderPath::NonRt => Renderer::new_with_options(
                device,
                queue,
                format,
                descriptor.limits,
                options,
                PRELUDE,
            )?,
        };
        let expected = match path {
            RenderPath::HardwareRt => PrimaryVisibility::Traced,
            RenderPath::NonRt => PrimaryVisibility::VisibilityBuffer,
        };
        if renderer.primary_visibility() != expected {
            return Err(Error::InvalidInput(
                "renderer did not select the path's primary visibility",
            ));
        }
        let info = adapter.get_info();
        if let Some(directory) = &descriptor.pipeline_cache {
            renderer
                .set_program_cache_directory(directory, &info)
                .map_err(|_| Error::InvalidInput("pipeline cache directory is unreadable"))?;
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
    /// The engine's device, for creating surfaces' configurations and the game's own
    /// resources. Submit nothing to its queue that the engine's frames depend on.
    pub fn device(&self) -> &wgpu::Device {
        self.renderer.device()
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
        Ok(self.meshes.insert(mesh))
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
            thin: false,
            program: Some(program),
            parameters: desc.parameters,
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
        let texture = |t: u32, sampler| -> Result<TextureRef, Error> {
            let texture = *handles.textures.get(t as usize).ok_or(Error::InvalidInput(
                "asset material names a missing texture",
            ))?;
            Ok(TextureRef { texture, sampler })
        };
        let mut materials = Vec::with_capacity(asset.materials.len());
        for m in &asset.materials {
            materials.push(MaterialDesc {
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
            });
        }
        for desc in &materials {
            handles.materials.push(self.create_material(desc)?);
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
            flat.push((self.meshes.insert(mesh), material));
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

    // ---- Lighting, sky and settings ----

    /// Sets or removes the sun, the scene's one shadowed directional light.
    pub fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        let previous = std::mem::replace(&mut self.sun, sun);
        self.apply_lighting().inspect_err(|_| self.sun = previous)
    }
    pub fn sun(&self) -> Option<Sun> {
        self.sun
    }
    /// Replaces the local lights (point and spot; unshadowed at this revision).
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
        settings::apply(&mut self.renderer, self.path, &settings)?;
        let exposure_changed = settings.exposure != self.settings.exposure;
        self.settings = settings;
        if exposure_changed {
            self.apply_lighting()?;
        }
        Ok(())
    }
    pub fn settings(&self) -> Settings {
        self.settings
    }
    fn apply_lighting(&mut self) -> Result<(), Error> {
        let (lighting, shadows) =
            settings::lighting(self.sun, &self.lights, self.sky, self.settings.exposure)?;
        let (map, parameters) = match self.sky {
            Sky::Uniform(_) => (None, EnvironmentParameters::default()),
            Sky::Map {
                map,
                rotation_y,
                intensity,
                ..
            } => (
                Some(self.environments.get(map)?),
                EnvironmentParameters {
                    rotation_y_radians: rotation_y,
                    intensity,
                },
            ),
        };
        self.renderer.set_environment(map, parameters)?;
        self.renderer.set_lighting_and_shadows(&lighting, &shadows)
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
            let mesh = meshes.get(batch.mesh)?;
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
    /// reconstruction restarts and streamed textures verify the new view; the GI cache
    /// refills its newly seen cells over a few frames on its own.
    pub fn cut(&mut self) {
        self.renderer.cut_reconstruction();
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
