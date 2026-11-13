//! The engine-level entry: the same scene-building and frame calls as
//! [`crate::engine::Engine`], recorded into the engine's device-free state and rendered
//! through [`forward::Frames`].
//!
//! A scene written against [`crate::engine::Scene`] runs on either layer. This one does
//! not implement everything: a call or setting it cannot honour is recorded as a
//! [`Refusal`] and the scene goes on without that piece, so a run ends with every gap
//! named. What the frame itself rejects (more than `TRACED_DRAWS` objects) is an
//! error from [`Scene::render`].
use super::{
    cache::ShaderCache,
    forward::{
        self, BlockTexture, ForwardImages, ForwardObject, ForwardScene, NormalMap, ObjectProgram,
        ObjectTexture, ObjectTextures, Primary, ScenePrograms, SceneTextures, Skin, SunShadows,
        TextureSource,
    },
    gi_cache::Indirect,
    skinned::Skinned,
    sky_map::SkyMap,
    temporal::Reconstruct,
};
use crate::{
    Camera, Decal, EmitterDescriptor, Error, FrameId, FrameReport, Light, Mat4, MaterialDescriptor,
    MeshData, PrimaryVisibility, SoftShadows, Value, Vertex, ViewId, ViewMode,
    asset::{
        Asset, Primitive, Sampler, TangentStream, Texture as TextureData,
        environment::PreparedEnvironment, skin::Influence,
    },
    engine::{
        AssetHandles, Background, EmitterId, EnvironmentId, Exposure, Image, MaterialDesc,
        MaterialId, MeshId, ObjectId, ProgramId, Quality, Scene, Settings, Sky, Sun, TextureId,
        ViewDesc,
        settings::{atmosphere, lighting, validate_look},
        state::{MaterialEntry, Residency, State},
    },
    program::{self, ParameterLayout},
};
use ara_gpu::GpuDevice;
use std::sync::Arc;

/// A call or setting this layer did not honour. `call` is the engine call; `what` names
/// the piece, a `Settings` field by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub call: &'static str,
    pub what: &'static str,
}

/// A program as the backend keeps one: its source and the layout of its `Parameters`.
struct Program {
    source: String,
    layout: Option<ParameterLayout>,
}
impl Program {
    /// Composes and validates `source` as `Renderer::try_create_program` does at the
    /// raster stage, so errors name program lines.
    fn new(label: &str, source: &str) -> Result<Self, Error> {
        let composed = program::compose(
            include_str!("../../shaders/scene.wgsl"),
            source,
            program::Stage::Raster,
            program::declares_parameters(source),
        );
        Ok(Self {
            source: source.to_owned(),
            layout: program::analyse(label, source, &composed)?.parameters,
        })
    }
}
/// Geometry the frame is rebuilt from.
enum Mesh {
    Static {
        /// Its identity to the frames, which keep its buffers and bottom-level
        /// structure on the GPU while a frame names it. Never reused.
        key: u64,
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
        /// Its tangent frame, which a normal-mapped material needs.
        tangents: Option<TangentStream>,
    },
    /// Posed on the CPU when its palette is set (`skinned.rs`).
    Skinned(Box<Skinned>),
}
/// A material's factors, with the program and textures it keeps alive.
struct Material {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    normal_scale: f32,
    double_sided: bool,
    thin: bool,
    transparency: Option<crate::Transparency>,
    /// Its identity to the frames' radiance cache, which names a draw by its mesh and
    /// material. Never reused.
    key: u64,
    program: Arc<Program>,
    base_color_texture: Option<MaterialTexture>,
    metallic_roughness_texture: Option<MaterialTexture>,
    normal_texture: Option<MaterialTexture>,
    parameters: Vec<(String, Value)>,
}

/// A texture as the host gave it: RGBA8 levels (`create_texture`), or block-compressed
/// levels as shipped (`create_compressed_texture`).
#[derive(Clone)]
enum Stored {
    Texels(Arc<TextureData>),
    Blocks(Arc<BlockTexture>),
}
impl Stored {
    fn source(&self) -> TextureSource<'_> {
        match self {
            Self::Texels(texels) => TextureSource::Texels(texels),
            Self::Blocks(blocks) => TextureSource::Blocks(blocks),
        }
    }
}

/// A material's texture slot: the texture's identity to the frames, the texture and
/// the sampler.
type MaterialTexture = (u64, Stored, Sampler);

/// What this layer keeps for each resource: the data its frames are rebuilt from.
struct Kept;
impl Residency for Kept {
    type Mesh = Mesh;
    /// Its identity to the frames, and the texture.
    type Texture = (u64, Stored);
    type Program = Arc<Program>;
    type Material = Material;
    /// Its identity to the frames, and the map.
    type Environment = (u64, Arc<PreparedEnvironment>);
    type Emitter = ();
    /// Nothing is kept resident between frames, so no batch is ever live.
    type Instances = ();
    fn set_instance(_: &mut (), _: usize, _: Mat4) -> Result<(), Error> {
        Ok(())
    }
    /// The frames read `Batch::casts` when they are rebuilt.
    fn set_instance_casts_shadows(_: &mut (), _: usize, _: bool) -> Result<(), Error> {
        Ok(())
    }
}

/// The scene and its frames on ara-gpu, on a device the caller owns (Ara's own, so a
/// frame needs no second device and no copy to be presented).
///
/// The scene calls take no device and touch none: they record what the next frame
/// draws. [`render`](Self::render) draws it on the device it is given and leaves the
/// displayed image there ([`output`](Self::output)); [`retire`](Self::retire) releases
/// what the engine keeps on that device. Use one device for an engine's whole life.
/// [`on`](Self::on) borrows the engine and the device together as a [`Scene`], for
/// code written against that trait. Read [`refusals`](Self::refusals) for what was left
/// out.
pub struct Engine {
    cache: ShaderCache,
    state: State<Kept>,
    /// The views in creation order (removal reorders), each with its frames.
    views: Vec<View>,
    /// Views created so far: the next one's identity.
    views_made: u64,
    /// The 2D targets every view's frames take and give back in turn (`image::pool`).
    targets: super::image::pool::Pool,
    /// The scene's object ids and the last frame's tag tables (docs/design-id-output.md).
    ids: crate::object_ids::EngineIds,
    /// Finished id readbacks, for `poll`.
    events: crate::Events,
    /// Id requests made so far: the next one's name.
    requests: u64,
    /// Histograms in flight (`view_histogram`), oldest first.
    histograms: Vec<super::histogram::Pending>,
    /// Meshes created so far: the next one's identity.
    meshes: u64,
    /// Materials created so far: the next one's identity.
    materials: u64,
    /// Environments created so far: the next one's identity.
    environments: u64,
    /// Textures created so far: the next one's identity.
    textures: u64,
    /// The largest block-compressed texture side the frames' device takes, once the
    /// engine has seen that device and it samples BC (`adopt_device`); None before, and
    /// on a device without BC.
    block_textures: Option<u32>,
    /// Whether frames are read back (`set_readback`).
    readback: bool,
    refusals: Vec<Refusal>,
    /// Frame sequences a view change, a removal or a settings or readback change ended,
    /// released at the next call that has the device.
    ended: Vec<forward::Frames>,
}

/// One view (docs/design-views.md): its description, and the frame sequence that draws
/// it, which holds its histories, its cache and what it keeps on the GPU. Views share
/// the scene and the engine's pool of frame targets, not their sequences: a second view
/// keeps its own copy of the scene's meshes and textures on the GPU and its own radiance
/// cache.
struct View {
    id: ViewId,
    desc: ViewDesc,
    frames: forward::Frames,
    /// The view's last frame's images (`images`, `capture_view`).
    last: Option<ForwardImages>,
    /// Frames rendered for this view.
    frame: u64,
}

/// One draw of a frame.
#[derive(Clone, Copy)]
struct Placed<'a> {
    vertices: &'a [Vertex],
    indices: &'a [u32],
    tangents: Option<&'a TangentStream>,
    material: &'a Material,
    /// Its first transform.
    transform: Mat4,
    /// The mesh's identity to the frames.
    mesh: u64,
    /// Every transform it is placed at.
    placements: &'a [Mat4],
    /// Whether each placement blocks light (`Batch::casts`, `set_casts_shadows`).
    casts: &'a [bool],
    /// A skinned mesh's pose before this one, and its pose revision.
    skin: Option<Skin<'a>>,
}

/// Records a refusal once.
fn note(refusals: &mut Vec<Refusal>, call: &'static str, what: &'static str) {
    let refusal = Refusal { call, what };
    if !refusals.contains(&refusal) {
        refusals.push(refusal);
    }
}
fn fault(error: anyhow::Error) -> Error {
    Error::Fault(crate::Fault(format!("{error:#}")))
}

impl Engine {
    /// An empty scene compiling through `cache`, with `settings`.
    /// Refuses RENDERLAB_GI_PASS=1 (`forward::GI_PASS_REFUSAL`).
    pub fn new(cache: ShaderCache, settings: Settings) -> Result<Self, Error> {
        if crate::renderer::primary_ray_gpu::gi_receiver_pass::on() {
            return Err(Error::InvalidInput(super::forward::GI_PASS_REFUSAL));
        }
        let standard = Arc::new(Program::new("standard", crate::STANDARD_PROGRAM)?);
        let mut engine = Self {
            cache,
            state: State::new(standard, settings),
            views: Vec::new(),
            views_made: 0,
            targets: Default::default(),
            ids: Default::default(),
            events: Default::default(),
            requests: 0,
            histograms: Vec::new(),
            meshes: 0,
            materials: 0,
            environments: 0,
            textures: 0,
            block_textures: None,
            readback: true,
            refusals: Vec::new(),
            ended: Vec::new(),
        };
        engine.refuse_settings(&settings);
        Ok(engine)
    }
    /// Every call and setting this layer did not honour so far, each once, in order.
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
    /// The view's last reconstructed frame's plan, for telemetry: whether it restarted
    /// every pixel, and the Interactive trace's shadow rays a pixel
    /// (`Renderer::reconstruction_shadow_rays`).
    pub fn reconstruction_plan(&self, view: ViewId) -> Option<(bool, u32)> {
        self.view(view).ok()?.frames.reconstruction_plan()
    }
    /// The view's last frame's images as the frame read them back: the exposed HDR
    /// before post, and the displayed bytes `capture_view` returns.
    pub fn images(&self, view: ViewId) -> Option<&ForwardImages> {
        self.view(view).ok()?.last.as_ref()
    }
    /// The engine's views in creation order (removal reorders).
    pub fn views(&self) -> impl Iterator<Item = ViewId> + '_ {
        self.views.iter().map(|v| v.id)
    }
    fn view(&self, view: ViewId) -> Result<&View, Error> {
        self.views
            .iter()
            .find(|v| v.id == view)
            .ok_or(Error::InvalidInput("unknown or removed view"))
    }
    fn view_index(&self, view: ViewId) -> Result<usize, Error> {
        self.views
            .iter()
            .position(|v| v.id == view)
            .ok_or(Error::InvalidInput("unknown or removed view"))
    }
    /// Whether frames come back to the CPU. On, the default, `capture` and `images`
    /// have the last frame. Off, a frame is rendered and left on the GPU, which is
    /// what a frame time should measure. Changing it starts a new frame sequence.
    pub fn set_readback(&mut self, on: bool) {
        if on != self.readback {
            self.readback = on;
            self.restart();
        }
    }
    /// A frame sequence as `set_readback` asks for.
    fn fresh(&self) -> forward::Frames {
        if self.readback {
            forward::Frames::default()
        } else {
            forward::Frames::without_images()
        }
    }
    /// Starts every view's frame sequence anew: a sequence's size and settings are fixed,
    /// so a change ends it, and its cache and history are released at the next call
    /// that has the device.
    fn restart(&mut self) {
        for at in 0..self.views.len() {
            self.restart_view(at);
        }
    }
    fn restart_view(&mut self, at: usize) {
        let fresh = self.fresh();
        self.ended
            .push(std::mem::replace(&mut self.views[at].frames, fresh));
    }
    /// Releases the ended sequences on `gpu`.
    fn release_ended(&mut self, gpu: &mut GpuDevice) -> Result<(), Error> {
        for ended in self.ended.drain(..) {
            ended.retire(gpu).map_err(fault)?;
        }
        Ok(())
    }
    /// The view's last displayed image on the device it was rendered on: sRGB-encoded
    /// RGBA8 at the view's size, kept until its frame after next and left in
    /// `CopySource`. `None` before the view's first frame.
    pub fn view_output(&self, view: ViewId) -> Option<&super::image::Texture> {
        self.view(view).ok()?.frames.output()
    }
    /// Bins `view`'s last displayed image on the GPU as `desc` says and returns without
    /// waiting; the reading comes from [`histograms`](Self::histograms) once the GPU has
    /// finished it, usually a frame later. Refused while four are in flight, or before the
    /// view has displayed a frame.
    pub fn view_histogram(
        &mut self,
        gpu: &mut GpuDevice,
        view: ViewId,
        desc: super::histogram::HistogramDesc,
    ) -> Result<u64, Error> {
        if self.histograms.len() >= 4 {
            return Err(Error::InvalidInput("four histograms are already in flight"));
        }
        let at = self.view_index(view)?;
        let frame = FrameId(self.views[at].frame);
        let image = self.views[at]
            .frames
            .output()
            .ok_or(Error::InvalidInput("the view has displayed no frame yet"))?;
        let submitted = super::histogram::submit(gpu, &self.cache, image, desc).map_err(fault)?;
        self.requests += 1;
        self.histograms.push(super::histogram::Pending::new(
            self.requests,
            view,
            frame,
            submitted,
        ));
        Ok(self.requests)
    }
    /// The histograms the GPU has finished since the last call, in request order.
    pub fn histograms(
        &mut self,
        gpu: &mut GpuDevice,
    ) -> Result<Vec<super::histogram::HistogramReading>, Error> {
        let mut finished = Vec::new();
        while let Some(pending) = self.histograms.first() {
            match pending.take(gpu).map_err(fault)? {
                Some(reading) => {
                    finished.push(reading);
                    self.histograms.remove(0);
                }
                None => break,
            }
        }
        Ok(finished)
    }
    /// Releases what the engine keeps on `gpu` (its frame sequences and pipelines) and
    /// stays usable: the scene is kept, and the next frame starts a new sequence and
    /// makes its pipelines again. What a host calls before it destroys the device.
    pub fn release(&mut self, gpu: &mut GpuDevice) -> anyhow::Result<()> {
        for pending in self.histograms.drain(..) {
            pending.drop_unread(gpu)?;
        }
        self.restart();
        self.release_ended(gpu)?;
        std::mem::take(&mut self.targets).retire(gpu)?;
        self.cache.retire(gpu)?;
        Ok(())
    }
    /// [`release`](Self::release), for an engine that is done.
    pub fn retire(mut self, gpu: &mut GpuDevice) -> anyhow::Result<()> {
        self.release(gpu)
    }
    /// The engine and `gpu` together as a [`Scene`], for code written against the trait.
    pub fn on<'a>(&'a mut self, gpu: &'a mut GpuDevice) -> Bound<'a> {
        self.adopt_device(gpu);
        Bound { engine: self, gpu }
    }
    /// Records what `gpu`, the device every frame of this engine uses, can do. The
    /// engine's state is device-free, so `compressed_textures` is false until this has
    /// run: a host that uploads textures before its first frame calls it first. `on` and
    /// `render_view` call it too.
    pub fn adopt_device(&mut self, gpu: &GpuDevice) {
        let info = gpu.info();
        self.block_textures = info
            .texture_compression_bc
            .then_some(info.max_image_dimension);
    }
    /// Whether the frames' device samples BC7 and BC5, so `create_compressed_texture`
    /// works (`adopt_device`).
    pub fn compressed_textures(&self) -> bool {
        self.block_textures.is_some()
    }
    /// A block-compressed texture as shipped (`renderlab_asset::CompressedTexture`: BC7
    /// colour or data, BC5 normals), as `crate::engine::Engine::create_compressed_texture`.
    /// A BC5 map gains its variance layer here. Errors where `compressed_textures` is
    /// false; upload RGBA8 (`create_texture`) there.
    pub fn create_compressed_texture(
        &mut self,
        data: crate::asset::CompressedTexture,
    ) -> Result<TextureId, Error> {
        let Some(max_dimension) = self.block_textures else {
            return Err(Error::InvalidInput(
                "compressed textures need a device that samples BC; upload RGBA8 instead",
            ));
        };
        data.validate(max_dimension)
            .map_err(|e| Error::InvalidInput(e.0))?;
        self.textures += 1;
        let blocks = Stored::Blocks(Arc::new(BlockTexture::new(data)));
        Ok(self.state.textures.insert((self.textures, blocks)))
    }

    fn refuse(&mut self, call: &'static str, what: &'static str) {
        note(&mut self.refusals, call, what);
    }
    /// Records the fields of `asked` this layer cannot honour. A field it does not know
    /// is refused whenever it differs from its default.
    fn refuse_settings(&mut self, asked: &Settings) {
        if asked.sky_light != 1.0 {
            self.refuse("set_settings", "sky_light");
        }
        // Every field named here is one this layer reads; a field added to `Settings`
        // later is missing from the list and so compares against its default.
        #[allow(clippy::needless_update)]
        let known = Settings {
            global_illumination: asked.global_illumination,
            temporal_reconstruction: asked.temporal_reconstruction,
            aerial_perspective: asked.aerial_perspective,
            height_fog: asked.height_fog,
            volumetric_fog: asked.volumetric_fog,
            local_shadows: asked.local_shadows,
            sky_light: asked.sky_light,
            ..Settings::default()
        };
        if known != *asked {
            self.refuse("set_settings", "a field this layer does not know");
        }
    }

    /// The frame's draws in batch order: each opaque mesh-material batch as one draw of
    /// its objects, as the wgpu engine draws it, then each transparent object on its
    /// own, as the pane pass takes them.
    fn placed(state: &State<Kept>) -> Result<[Vec<Placed<'_>>; 2], Error> {
        let (mut opaque, mut transparent) = (Vec::new(), Vec::new());
        for id in state.batches.ids() {
            let batch = state.batches.get(id)?;
            let (key, vertices, indices, tangents, skin) = match state.meshes.get(batch.mesh)? {
                Mesh::Static {
                    key,
                    vertices,
                    indices,
                    tangents,
                } => (key, vertices, indices, tangents, None),
                Mesh::Skinned(mesh) => (
                    &mesh.key,
                    &mesh.vertices,
                    &mesh.indices,
                    &mesh.tangents,
                    Some(Skin {
                        previous: &mesh.previous,
                        revision: mesh.revision,
                    }),
                ),
            };
            let material = &state.materials.get(batch.material)?.material;
            if material.normal_texture.is_some() && tangents.is_none() {
                // As the wgpu renderer refuses the draw.
                return Err(Error::InvalidInput(
                    "normal-mapped draw requires tangent stream",
                ));
            }
            let placed = |transform, placements, casts| Placed {
                vertices,
                indices,
                tangents: tangents.as_ref(),
                material,
                transform,
                mesh: *key,
                placements,
                casts,
                skin,
            };
            if material.transparency.is_some() {
                transparent.extend(
                    batch
                        .models
                        .iter()
                        .zip(&batch.casts)
                        .map(|(model, casts)| {
                            placed(*model, std::slice::from_ref(model), std::slice::from_ref(casts))
                        }),
                );
            } else if let Some(first) = batch.models.first() {
                opaque.push(placed(*first, &batch.models[..], &batch.casts[..]));
            }
        }
        Ok([opaque, transparent])
    }
}

fn texture(slot: &Option<MaterialTexture>) -> Option<ObjectTexture<'_>> {
    slot.as_ref().map(|(key, data, sampler)| ObjectTexture {
        data: data.source(),
        sampler: *sampler,
        key: Some(*key),
    })
}
fn objects<'a>(placed: &[Placed<'a>]) -> Vec<ForwardObject<'a>> {
    placed
        .iter()
        .map(
            |&Placed {
                 vertices,
                 indices,
                 material,
                 transform,
                 ..
             }| ForwardObject {
                vertices,
                indices,
                transform,
                material: MaterialDescriptor {
                    base_color: material.base_color,
                    metallic: material.metallic,
                    roughness: material.roughness,
                    normal_scale: material.normal_scale,
                    double_sided: material.double_sided,
                    thin: material.thin,
                    transparency: material.transparency,
                    ..Default::default()
                },
            },
        )
        .collect()
}

/// The scene calls, as [`Scene`] documents each; none touches the device.
impl Engine {
    /// A mesh with a per-vertex tangent frame, which normal-mapped materials need.
    /// The frame is checked as the wgpu renderer checks it.
    pub fn create_mesh_with_tangents(
        &mut self,
        data: &MeshData<'_>,
        tangents: &TangentStream,
    ) -> Result<MeshId, Error> {
        crate::asset::validate_tangents(data.vertices, data.indices, tangents)
            .map_err(|_| Error::InvalidInput("invalid unit tangent frame or handedness"))?;
        self.insert_mesh(data, Some(tangents.clone()))
    }
    /// Releases the mesh once no object uses it; its buffers leave the device with the
    /// next frame that no longer names it.
    pub fn remove_mesh(&mut self, mesh: MeshId) -> Result<(), Error> {
        self.state.remove_mesh(mesh).map(drop)
    }
    /// Releases the texture once no material uses it.
    pub fn remove_texture(&mut self, texture: TextureId) -> Result<(), Error> {
        self.state.remove_texture(texture).map(drop)
    }
    /// Releases the material once no object uses it.
    pub fn remove_material(&mut self, material: MaterialId) -> Result<(), Error> {
        self.state.remove_material(material).map(drop)
    }
    /// Releases the environment once the sky no longer uses it.
    pub fn remove_environment(&mut self, environment: EnvironmentId) -> Result<(), Error> {
        self.state.remove_environment(environment).map(drop)
    }
    /// Releases the emitter.
    pub fn remove_emitter(&mut self, emitter: EmitterId) -> Result<(), Error> {
        self.state.remove_emitter(emitter).map(drop)
    }
    /// Removes the object from the scene.
    pub fn despawn(&mut self, object: ObjectId) -> Result<(), Error> {
        self.state.despawn(object)
    }
    /// Emitters are refused on this layer (`create_emitter`), so there is nothing to
    /// change; the call is recorded once.
    pub fn update_emitter(
        &mut self,
        emitter: EmitterId,
        _desc: &EmitterDescriptor<'_>,
        _sprite: Option<TextureId>,
    ) -> Result<(), Error> {
        self.state.emitters.get(emitter)?;
        self.refuse("update_emitter", "particle emitters");
        Ok(())
    }
    pub fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error> {
        self.insert_mesh(data, None)
    }
    fn insert_mesh(
        &mut self,
        data: &MeshData<'_>,
        tangents: Option<TangentStream>,
    ) -> Result<MeshId, Error> {
        self.meshes += 1;
        Ok(self.state.meshes.insert(Mesh::Static {
            key: self.meshes,
            vertices: data.vertices.to_vec(),
            indices: data.indices.to_vec(),
            tangents,
        }))
    }
    pub fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        self.textures += 1;
        let texels = Stored::Texels(Arc::new(data));
        Ok(self.state.textures.insert((self.textures, texels)))
    }
    pub fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        let program = Program::new(label, source)?;
        Ok(self.state.programs.insert(Arc::new(program)))
    }
    pub fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        let program_id = desc.program.unwrap_or(self.state.standard);
        let program = self.state.programs.get(program_id)?.clone();
        ParameterLayout::check(
            program.layout.as_ref(),
            desc.parameters.iter().map(|(n, v)| (*n, v.clone())),
        )?;
        let texture = |slot: Option<crate::engine::TextureRef>| {
            slot.map(|t| {
                let (key, data) = self.state.textures.get(t.texture)?;
                Ok::<_, Error>((*key, data.clone(), t.sampler))
            })
            .transpose()
        };
        self.materials += 1;
        let material = Material {
            key: self.materials,
            base_color: desc.base_color,
            metallic: desc.metallic,
            roughness: desc.roughness,
            normal_scale: desc.normal_scale,
            double_sided: desc.double_sided,
            thin: desc.thin,
            transparency: desc.transparency,
            base_color_texture: texture(desc.base_color_texture)?,
            metallic_roughness_texture: texture(desc.metallic_roughness_texture)?,
            normal_texture: texture(desc.normal_texture)?,
            parameters: desc
                .parameters
                .iter()
                .map(|(n, v)| ((*n).to_owned(), v.clone()))
                .collect(),
            program,
        };
        Ok(self.state.materials.insert(MaterialEntry {
            material,
            program: program_id,
        }))
    }
    pub fn set_material_parameters(
        &mut self,
        material: MaterialId,
        values: &[(&str, Value)],
    ) -> Result<(), Error> {
        let material = &mut self.state.materials.get_mut(material)?.material;
        ParameterLayout::check(
            material.program.layout.as_ref(),
            values.iter().map(|(n, v)| (*n, v.clone())),
        )?;
        for (name, value) in values {
            match material.parameters.iter_mut().find(|(n, _)| n == name) {
                Some(kept) => kept.1 = value.clone(),
                None => material
                    .parameters
                    .push(((*name).to_owned(), value.clone())),
            }
        }
        Ok(())
    }
    /// Keeps the map for `Sky::Map`; the first frame that names it uploads it. Maps above
    /// the wgpu renderer's default policy are refused as it refuses them.
    pub fn create_environment(
        &mut self,
        data: &PreparedEnvironment,
    ) -> Result<EnvironmentId, Error> {
        crate::environment::upload_bytes(
            data,
            crate::environment::EnvironmentLimits::default(),
            u32::MAX,
        )?;
        self.environments += 1;
        Ok(self
            .state
            .environments
            .insert((self.environments, Arc::new(data.clone()))))
    }
    pub fn add_asset(&mut self, _: Asset) -> Result<AssetHandles, Error> {
        self.refuse("add_asset", "cooked assets");
        Ok(AssetHandles::default())
    }
    /// A posable mesh in its bind pose; `set_joint_palette` poses it.
    pub fn create_skinned_mesh(
        &mut self,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<MeshId, Error> {
        self.meshes += 1;
        let mesh = Skinned::new(self.meshes, primitive, influences, joint_count)?;
        Ok(self.state.meshes.insert(Mesh::Skinned(Box::new(mesh))))
    }
    /// Poses a skinned mesh with mesh-local joint matrices; the next frame draws it.
    pub fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error> {
        match self.state.meshes.get_mut(mesh)? {
            Mesh::Skinned(skinned) => skinned.set_palette(palette),
            Mesh::Static { .. } => Err(Error::InvalidInput("mesh is not skinned")),
        }
    }
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
        sprite.map(|t| self.state.textures.get(t)).transpose()?;
        self.refuse("create_emitter", "particle emitters");
        Ok(self.state.emitters.insert(()))
    }
    /// One engine frame: the shared per-frame work. Skinned poses are applied when their
    /// palettes are set and there are no particles to step, so it does nothing here.
    pub fn tick(&mut self, _seconds: f32) -> Result<(), Error> {
        Ok(())
    }

    pub fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error> {
        self.state.spawn(mesh, material, transform, u32::MAX)
    }
    pub fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        self.state.set_transform(object, transform)
    }
    /// Whether an object blocks light (default true), as the wgpu engine's `set_casts_shadows`.
    /// Stored on its batch (`Batch::casts`); this layer's shadow casters do not read it yet
    /// (instance masks, the atlas and the clipmap follow in the port).
    pub fn set_casts_shadows(&mut self, object: ObjectId, casts: bool) -> Result<(), Error> {
        self.state.set_casts_shadows(object, casts)
    }
    pub fn casts_shadows(&self, object: ObjectId) -> Result<bool, Error> {
        self.state.casts_shadows(object)
    }
    pub fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        if !decals.is_empty() {
            self.refuse("set_decals", "decals");
        }
        Ok(())
    }

    pub fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        lighting(sun, &self.state.lights, self.state.sky, Exposure::default())?;
        self.state.sun = sun;
        Ok(())
    }
    pub fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        lighting(self.state.sun, lights, self.state.sky, Exposure::default())?;
        self.state.lights = lights.to_vec();
        Ok(())
    }
    pub fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        if let Sky::Map {
            map,
            rotation_y,
            intensity,
            background,
        } = sky
        {
            self.state.environments.get(map)?;
            // As the wgpu engine checks them.
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
        }
        atmosphere(sky, &self.state.settings).validate()?;
        self.state.sky = sky;
        Ok(())
    }
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        atmosphere(self.state.sky, &settings).validate()?;
        self.refuse_settings(&settings);
        if settings != self.state.settings {
            self.restart();
        }
        self.state.settings = settings;
        Ok(())
    }
    pub fn settings(&self) -> Settings {
        self.state.settings
    }

    /// Checks a view as the wgpu engine does, and records what this layer leaves out of
    /// one: view modes other than lit, and a look without post.
    fn check_view(&mut self, desc: &ViewDesc) -> Result<(), Error> {
        if desc.width == 0 || desc.height == 0 {
            return Err(Error::InvalidInput("view extent must be nonzero"));
        }
        validate_look(&self.state.settings, &desc.look, desc.render_scale)?;
        if desc.view_mode != ViewMode::Lit {
            self.refuse("create_view", "view modes other than lit");
        }
        if !desc.look.post.enabled {
            self.refuse("create_view", "look.post.enabled");
        }
        Ok(())
    }
    /// Creates a view; its first frame allocates its targets.
    pub fn create_view(&mut self, desc: &ViewDesc) -> Result<ViewId, Error> {
        self.check_view(desc)?;
        self.views_made += 1;
        let id = ViewId(self.views_made);
        let frames = self.fresh();
        self.views.push(View {
            id,
            desc: desc.clone(),
            frames,
            last: None,
            frame: 0,
        });
        Ok(id)
    }
    /// Changes a view. A new size or render scale restarts its frame sequence (its
    /// histories and what it keeps on the GPU); a new mode, look or outputs restarts
    /// nothing.
    pub fn set_view(&mut self, view: ViewId, desc: &ViewDesc) -> Result<(), Error> {
        self.check_view(desc)?;
        let at = self.view_index(view)?;
        let old = &self.views[at].desc;
        if (old.width, old.height, old.render_scale) != (desc.width, desc.height, desc.render_scale)
        {
            self.restart_view(at);
        }
        self.views[at].desc = desc.clone();
        Ok(())
    }
    /// Removes a view; its sequence is released at the next call that has the device.
    pub fn remove_view(&mut self, view: ViewId) -> Result<(), Error> {
        let at = self.view_index(view)?;
        let removed = self.views.swap_remove(at);
        self.ended.push(removed.frames);
        Ok(())
    }
    /// A camera cut for this view only: its reconstruction, fog and post histories
    /// restart (and, unlike the wgpu engine's shared cache, its own radiance cache).
    pub fn cut_view(&mut self, view: ViewId) {
        if let Ok(at) = self.view_index(view) {
            self.views[at].frames.cut();
        }
    }
    /// The view's last displayed frame, from its read-back images.
    pub fn capture_view(&mut self, view: ViewId) -> Result<Image, Error> {
        let view = self.view(view)?;
        let display = view
            .last
            .as_ref()
            .and_then(|images| images.display.clone())
            .ok_or(Error::InvalidInput(
                "no displayed frame to capture: render the view with read-back on first",
            ))?;
        Ok(Image {
            width: view.desc.width,
            height: view.desc.height,
            rgba8: display,
        })
    }
    /// The embedder's ids for an object's primitives: disjoint ranges `(first primitive,
    /// count, id)`, as the wgpu engine takes them. Frames from the next one on report them.
    pub fn set_object_ids(
        &mut self,
        object: ObjectId,
        ranges: &[(u32, u32, u32)],
    ) -> Result<(), Error> {
        self.state.transform(object)?;
        self.ids.objects.set(object, ranges)
    }
    /// The view's id texture of its last frame with ids on (`ViewDesc::outputs.ids`):
    /// R32Uint, the view's size, layer 0 the opaque ids and layer 1 the front ids.
    pub fn view_ids(&self, view: ViewId) -> Option<&super::image::Texture> {
        self.view(view).ok()?.frames.ids()
    }
    /// One id layer of the view's last frame packed bit-exact into an RGBA8 image (R =
    /// bits 0-7 ... A = bits 24-31), left in `CopySource` for a presenter to copy: what
    /// `render_view_ids_to` writes into a caller's texture on the wgpu engine. Returns once
    /// the pack is submitted, without waiting for the GPU.
    pub fn view_ids_packed(
        &mut self,
        gpu: &mut GpuDevice,
        view: ViewId,
        layer: crate::IdLayer,
    ) -> Result<&super::image::Texture, Error> {
        let at = self.view_index(view)?;
        let layer = match layer {
            crate::IdLayer::Opaque => 0,
            crate::IdLayer::Front => 1,
        };
        self.views[at]
            .frames
            .pack_ids(gpu, &self.cache, layer)
            .map_err(fault)
    }
    /// Reads a rectangle of the view's last frame's ids. The frame is complete when
    /// `render_view` returns, so the readback finishes here; it arrives at the next
    /// `poll` (`Events::ids`), as on the wgpu engine.
    pub fn request_ids(
        &mut self,
        gpu: &mut GpuDevice,
        view: ViewId,
        rect: crate::IdRect,
    ) -> Result<crate::IdRequest, Error> {
        let at = self.view_index(view)?;
        let desc = &self.views[at].desc;
        rect.check(desc.width, desc.height)?;
        let ids = self.views[at]
            .frames
            .read_ids(gpu, [rect.x, rect.y, rect.width, rect.height])
            .map_err(fault)?;
        self.requests += 1;
        let request = crate::IdRequest(self.requests);
        self.events.ids.push(crate::IdReadback {
            request,
            view,
            frame: FrameId(self.views[at].frame),
            rect,
            ids: ids
                .into_iter()
                .map(|[opaque, front]| crate::IdPixel { opaque, front })
                .collect(),
        });
        while self.events.ids.len() > crate::KEPT_ID_EVENTS {
            self.events.ids.remove(0);
            self.events.ids_dropped += 1;
        }
        Ok(request)
    }
    /// The id readbacks finished since the last poll.
    pub fn poll(&mut self) -> Result<crate::Events, Error> {
        Ok(std::mem::take(&mut self.events))
    }
    /// Draws `view`'s next frame from `camera` on `gpu` (the device every frame of this
    /// engine uses).
    pub fn render_view(
        &mut self,
        gpu: &mut GpuDevice,
        view: ViewId,
        camera: &Camera,
    ) -> Result<FrameReport, Error> {
        self.adopt_device(gpu);
        self.release_ended(gpu)?;
        let at = self.view_index(view)?;
        let desc = self.views[at].desc.clone();
        // `RENDERLAB_ABLATE` switches features off whatever the embedder asks, as on wgpu.
        let look = crate::engine::settings::ablated_look(desc.look.clone());
        let (size, look) = ([desc.width, desc.height], &look);
        let settings = crate::engine::settings::ablated(self.state.settings);
        let [opaque, transparent] = Self::placed(&self.state)?;
        // The frame's texture table covers its opaque objects alone.
        if transparent.iter().any(|p| {
            let m = p.material;
            m.base_color_texture.is_some()
                || m.metallic_roughness_texture.is_some()
                || m.normal_texture.is_some()
        }) {
            note(
                &mut self.refusals,
                "render",
                "textures on a transparent material",
            );
        }
        // The traced frame shades with a sun; a scene without one draws with a sun that
        // adds no light (its shadow rays are the cost).
        let sun = self.state.sun.unwrap_or(Sun {
            travel: crate::Vec3::NEG_Y,
            illuminance: [0.; 3],
            // The sun's shadow planning wants a disk.
            angular_radius: 0.04,
        });
        let (lit, shadows) =
            lighting(Some(sun), &self.state.lights, self.state.sky, look.exposure)?;
        let sun_shadows = shadows.requests.first().and_then(|r| match r {
            crate::ShadowRequest::Directional { split_ends } => Some(SunShadows {
                split_ends: split_ends.clone(),
                disk_angle: shadows.directional_disk_angle,
            }),
            _ => None,
        });
        let media = atmosphere(self.state.sky, &settings);
        let sky_map = match self.state.sky {
            Sky::Map {
                map,
                rotation_y,
                intensity,
                background,
            } => {
                let (key, data) = self.state.environments.get(map)?;
                Some(SkyMap {
                    key: *key,
                    data,
                    rotation_y,
                    intensity,
                    visible: match background {
                        Background::Sky => Some(intensity),
                        Background::ScaledSky(scale) => Some(scale),
                        Background::Color(_) => None,
                    },
                })
            }
            _ => None,
        };
        let opaque_objects = objects(&opaque);
        let keys: Vec<u64> = opaque.iter().map(|p| p.mesh).collect();
        let material_keys: Vec<u64> = opaque.iter().map(|p| p.material.key).collect();
        let placements: Vec<&[Mat4]> = opaque.iter().map(|p| p.placements).collect();
        let casts: Vec<&[bool]> = opaque.iter().map(|p| p.casts).collect();
        let skins: Vec<Option<Skin<'_>>> = opaque.iter().map(|p| p.skin).collect();
        // The view's ids: the tag tables in the frame's placement order, every opaque
        // placement's entry and then every transparent one's (`prepare_view_ids`).
        let id_entries = desc.outputs.ids.then(|| {
            let mut batches = Vec::new();
            for id in self.state.batches.ids() {
                if let (Ok(batch), Ok(entry)) = (
                    self.state.batches.get(id),
                    self.state
                        .batches
                        .get(id)
                        .and_then(|b| self.state.materials.get(b.material)),
                ) {
                    batches.push((entry.material.transparency.is_some(), &batch.objects[..]));
                }
            }
            let mut tables = crate::object_ids::TagTables::default();
            self.ids.objects.tables(batches.into_iter(), &mut tables);
            let opaque_count: usize = opaque.iter().map(|p| p.placements.len()).sum();
            let mut entries = tables.opaque.clone();
            entries.resize(opaque_count, [0, 0]);
            entries.extend_from_slice(&tables.transparent);
            entries.resize(opaque_count + transparent.len(), [0, 0]);
            (entries, tables.ranges)
        });
        let transparent_objects = objects(&transparent);
        let values: Vec<Vec<(&str, Value)>> = opaque
            .iter()
            .map(|p| {
                p.material
                    .parameters
                    .iter()
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect()
            })
            .collect();
        let programs: Vec<_> = opaque
            .iter()
            .zip(&values)
            .map(|(p, values)| {
                Some(ObjectProgram {
                    source: &p.material.program.source,
                    parameters: values,
                })
            })
            .collect();
        // A pane's program by its source; its parameter block is not assembled.
        let pane_programs: Vec<_> = transparent
            .iter()
            .map(|p| {
                Some(ObjectProgram {
                    source: &p.material.program.source,
                    parameters: &[],
                })
            })
            .collect();
        if transparent
            .iter()
            .any(|p| p.material.program.layout.is_some())
        {
            return Err(Error::InvalidInput(
                "a transparent material's program with Parameters is not assembled",
            ));
        }
        let textures: Vec<_> = opaque
            .iter()
            .map(|p| ObjectTextures {
                base_color: texture(&p.material.base_color_texture),
                metallic_roughness: texture(&p.material.metallic_roughness_texture),
                // `placed` refused a map without a tangent stream.
                normal: texture(&p.material.normal_texture)
                    .zip(p.tangents)
                    .map(|(texture, tangents)| NormalMap { texture, tangents }),
            })
            .collect();
        let textured = textures.iter().any(|t| {
            t.base_color.is_some() || t.metallic_roughness.is_some() || t.normal.is_some()
        });
        let scene = ForwardScene {
            objects: &opaque_objects,
            lighting: &lit,
            camera: *camera,
            size,
            atmosphere: media,
            sun_shadows: sun_shadows.as_ref(),
            primary: match look.quality {
                Quality::Realtime => Primary::TracedInteractive,
                Quality::Reference => Primary::Traced,
            },
            post: Some(look.post),
            indirect: settings.global_illumination.then(|| Indirect {
                settings: crate::IndirectSettings::default(),
                read_table: false,
            }),
            reconstruction: settings.temporal_reconstruction.then(|| Reconstruct {
                settings: crate::Reconstruction {
                    scale: desc.render_scale,
                    ..Default::default()
                },
                read_history: false,
            }),
            reflections: Some(look.reflections),
            textures: textured.then_some(SceneTextures {
                objects: &textures,
                anisotropy_clamp: 1,
            }),
            local_shadows: (settings.local_shadows.count > 0).then_some(settings.local_shadows),
            transparent: &transparent_objects,
            programs: Some(ScenePrograms {
                objects: &programs,
                transparent: &pane_programs,
            }),
            meshes: Some(&keys),
            materials: Some(&material_keys),
            instances: Some(&placements),
            sky_map,
            skins: Some(&skins),
            casts: Some(&casts),
            colour_lut: look.colour_lut.as_ref(),
            ids: id_entries
                .as_ref()
                .map(|(entries, ranges)| forward::IdScene { entries, ranges }),
        };
        let images = self.views[at]
            .frames
            .render_with(gpu, &self.cache, &scene, &mut self.targets)
            .map_err(fault)?;
        // The renderer's counts are of its opaque draws; a pane is not among them.
        let triangles: usize = opaque
            .iter()
            .map(|p| p.indices.len() / 3 * p.placements.len())
            .sum();
        let instances: usize = opaque.iter().map(|p| p.placements.len()).sum();
        let view_state = &mut self.views[at];
        view_state.last = Some(images);
        view_state.frame += 1;
        Ok(FrameReport {
            view,
            frame: FrameId(view_state.frame),
            revision: crate::Revision(0),
            cpu: Default::default(),
            uploads: Default::default(),
            timed: false,
            scene: crate::SceneCounts {
                collections: opaque.len() as u64,
                total_instances: instances as u64,
                visible_instances: instances as u64,
                submitted_instances: instances as u64,
                draws: opaque.len() as u64,
                triangles: triangles as u64,
                ..Default::default()
            },
            primary: PrimaryVisibility::Traced,
            soft_shadows: match look.quality {
                Quality::Realtime => SoftShadows::Interactive,
                Quality::Reference => SoftShadows::Exact,
            },
        })
    }
}

/// An [`Engine`] and the device its frames are drawn on, as a [`Scene`]: what
/// [`Engine::on`] gives.
pub struct Bound<'a> {
    engine: &'a mut Engine,
    gpu: &'a mut GpuDevice,
}
/// The engine's own accessors (`images`, `refusals`, `output`) through the borrow.
impl std::ops::Deref for Bound<'_> {
    type Target = Engine;
    fn deref(&self) -> &Engine {
        self.engine
    }
}
impl Bound<'_> {
    /// `Engine::create_compressed_texture`, on the bound device.
    pub fn create_compressed_texture(
        &mut self,
        data: crate::asset::CompressedTexture,
    ) -> Result<TextureId, Error> {
        self.engine.create_compressed_texture(data)
    }
}
impl Scene for Bound<'_> {
    fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error> {
        self.engine.create_mesh(data)
    }
    fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        self.engine.create_texture(data)
    }
    fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        self.engine.create_program(label, source)
    }
    fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        self.engine.create_material(desc)
    }
    fn set_material_parameters(
        &mut self,
        material: MaterialId,
        values: &[(&str, Value)],
    ) -> Result<(), Error> {
        self.engine.set_material_parameters(material, values)
    }
    fn create_environment(&mut self, data: &PreparedEnvironment) -> Result<EnvironmentId, Error> {
        self.engine.create_environment(data)
    }
    fn add_asset(&mut self, asset: Asset) -> Result<AssetHandles, Error> {
        self.engine.add_asset(asset)
    }
    fn create_skinned_mesh(
        &mut self,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<MeshId, Error> {
        self.engine
            .create_skinned_mesh(primitive, influences, joint_count)
    }
    fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error> {
        self.engine.set_joint_palette(mesh, palette)
    }
    fn create_emitter(
        &mut self,
        desc: &EmitterDescriptor<'_>,
        sprite: Option<TextureId>,
    ) -> Result<EmitterId, Error> {
        self.engine.create_emitter(desc, sprite)
    }
    fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error> {
        self.engine.spawn(mesh, material, transform)
    }
    fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        self.engine.set_transform(object, transform)
    }
    fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        self.engine.set_decals(decals)
    }
    fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        self.engine.set_sun(sun)
    }
    fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        self.engine.set_lights(lights)
    }
    fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        self.engine.set_sky(sky)
    }
    fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        self.engine.set_settings(settings)
    }
    fn settings(&self) -> Settings {
        self.engine.settings()
    }
    fn tick(&mut self, seconds: f32) -> Result<(), Error> {
        self.engine.tick(seconds)
    }
    fn set_object_ids(
        &mut self,
        object: ObjectId,
        ranges: &[(u32, u32, u32)],
    ) -> Result<(), Error> {
        self.engine.set_object_ids(object, ranges)
    }
    fn request_ids(
        &mut self,
        view: ViewId,
        rect: crate::IdRect,
    ) -> Result<crate::IdRequest, Error> {
        self.engine.request_ids(self.gpu, view, rect)
    }
    fn poll(&mut self) -> Result<crate::Events, Error> {
        self.engine.poll()
    }
    fn create_view(&mut self, desc: &ViewDesc) -> Result<ViewId, Error> {
        self.engine.create_view(desc)
    }
    fn set_view(&mut self, view: ViewId, desc: &ViewDesc) -> Result<(), Error> {
        self.engine.set_view(view, desc)
    }
    fn remove_view(&mut self, view: ViewId) -> Result<(), Error> {
        self.engine.remove_view(view)
    }
    fn render_view(&mut self, view: ViewId, camera: &Camera) -> Result<FrameReport, Error> {
        self.engine.render_view(self.gpu, view, camera)
    }
    fn cut_view(&mut self, view: ViewId) {
        self.engine.cut_view(view)
    }
    fn capture_view(&mut self, view: ViewId) -> Result<Image, Error> {
        self.engine.capture_view(view)
    }
}
