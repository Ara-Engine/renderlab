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
        self, ForwardImages, ForwardObject, ForwardScene, NormalMap, ObjectProgram, ObjectTexture,
        ObjectTextures, Primary, ScenePrograms, SceneTextures, Skin, SunShadows,
    },
    gi_cache::Indirect,
    skinned::Skinned,
    sky_map::SkyMap,
    temporal::Reconstruct,
};
use crate::{
    Camera, Decal, EmitterDescriptor, Error, FrameId, FrameReport, Light, Mat4, MaterialDescriptor,
    MeshData, PrimaryVisibility, RenderScale, SoftShadows, Value, Vertex, ViewId,
    asset::{
        Asset, Primitive, Sampler, TangentStream, Texture as TextureData,
        environment::PreparedEnvironment, skin::Influence,
    },
    engine::{
        AssetHandles, Background, EmitterId, EnvironmentId, Image, MaterialDesc, MaterialId,
        MeshId, ObjectId, ProgramId, Quality, Scene, Settings, Sky, Sun, TextureId, ViewDesc,
        ViewLook,
        settings::{atmosphere, lighting},
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

/// A material's texture slot: the texture's identity to the frames, the texture and
/// the sampler.
type MaterialTexture = (u64, Arc<TextureData>, Sampler);

/// What this layer keeps for each resource: the data its frames are rebuilt from.
struct Kept;
impl Residency for Kept {
    type Mesh = Mesh;
    /// Its identity to the frames, and the texture.
    type Texture = (u64, Arc<TextureData>);
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
    frames: forward::Frames,
    size: Option<[u32; 2]>,
    /// The last frame's images, for `capture`.
    last: Option<ForwardImages>,
    frame: u64,
    /// Meshes created so far: the next one's identity.
    meshes: u64,
    /// Materials created so far: the next one's identity.
    materials: u64,
    /// Environments created so far: the next one's identity.
    environments: u64,
    /// Textures created so far: the next one's identity.
    textures: u64,
    /// Whether frames are read back (`set_readback`).
    readback: bool,
    refusals: Vec<Refusal>,
    /// Frame sequences a resize or a settings or readback change ended, released at the
    /// next call that has the device.
    ended: Vec<forward::Frames>,
    /// This layer renders one view (docs/design-views.md: views are not ported here): its
    /// handle once created, and its look and render scale.
    view: Option<ViewId>,
    look: ViewLook,
    render_scale: RenderScale,
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
    pub fn new(cache: ShaderCache, settings: Settings) -> Result<Self, Error> {
        let standard = Arc::new(Program::new("standard", crate::STANDARD_PROGRAM)?);
        let mut engine = Self {
            cache,
            state: State::new(standard, settings),
            frames: forward::Frames::default(),
            size: None,
            last: None,
            frame: 0,
            meshes: 0,
            materials: 0,
            environments: 0,
            textures: 0,
            readback: true,
            refusals: Vec::new(),
            ended: Vec::new(),
            view: None,
            look: ViewLook::default(),
            render_scale: RenderScale::Native,
        };
        engine.refuse_settings(&settings);
        Ok(engine)
    }
    /// Every call and setting this layer did not honour so far, each once, in order.
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
    /// The last reconstructed frame's plan, for telemetry: whether it restarted every
    /// pixel, and the Interactive trace's shadow rays a pixel
    /// (`Renderer::reconstruction_shadow_rays`).
    pub fn reconstruction_plan(&self) -> Option<(bool, u32)> {
        self.frames.reconstruction_plan()
    }
    /// The last frame's images as the frame read them back: the exposed HDR before
    /// post, and the displayed bytes `capture` returns.
    pub fn images(&self) -> Option<&ForwardImages> {
        self.last.as_ref()
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
    /// Starts a new frame sequence: the history's size and settings are fixed within
    /// one, so a change of either ends the old sequence, whose cache and history are
    /// released at the next call that has the device.
    fn restart(&mut self) {
        let fresh = if self.readback {
            forward::Frames::default()
        } else {
            forward::Frames::without_images()
        };
        self.ended.push(std::mem::replace(&mut self.frames, fresh));
    }
    /// Releases the ended sequences on `gpu`.
    fn release_ended(&mut self, gpu: &mut GpuDevice) -> Result<(), Error> {
        for ended in self.ended.drain(..) {
            ended.retire(gpu).map_err(fault)?;
        }
        Ok(())
    }
    /// The last frame's displayed image on the device it was rendered on: sRGB-encoded
    /// RGBA8 at the output's size, kept until the frame after next. `None` before the
    /// first frame with post enabled.
    pub fn output(&self) -> Option<&super::image::Texture> {
        self.frames.output()
    }
    /// Releases what the engine keeps on `gpu` (its frame sequences and pipelines) and
    /// stays usable: the scene is kept, and the next frame starts a new sequence and
    /// makes its pipelines again. What a host calls before it destroys the device.
    pub fn release(&mut self, gpu: &mut GpuDevice) -> anyhow::Result<()> {
        self.release_ended(gpu)?;
        let fresh = if self.readback {
            forward::Frames::default()
        } else {
            forward::Frames::without_images()
        };
        std::mem::replace(&mut self.frames, fresh).retire(gpu)?;
        self.cache.retire(gpu)?;
        Ok(())
    }
    /// [`release`](Self::release), for an engine that is done.
    pub fn retire(mut self, gpu: &mut GpuDevice) -> anyhow::Result<()> {
        self.release(gpu)
    }
    /// The engine and `gpu` together as a [`Scene`], for code written against the trait.
    pub fn on<'a>(&'a mut self, gpu: &'a mut GpuDevice) -> Bound<'a> {
        Bound { engine: self, gpu }
    }

    fn refuse(&mut self, call: &'static str, what: &'static str) {
        note(&mut self.refusals, call, what);
    }
    /// Records the fields of `asked` this layer cannot honour. A field it does not know
    /// is refused whenever it differs from its default.
    fn refuse_settings(&mut self, asked: &Settings) {
        if asked.volumetric_fog.is_some() {
            self.refuse("set_settings", "volumetric_fog");
        }
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
            let placed = |transform, placements| Placed {
                vertices,
                indices,
                tangents: tangents.as_ref(),
                material,
                transform,
                mesh: *key,
                placements,
                skin,
            };
            if material.transparency.is_some() {
                transparent.extend(
                    batch
                        .models
                        .iter()
                        .map(|model| placed(*model, std::slice::from_ref(model))),
                );
            } else if let Some(first) = batch.models.first() {
                opaque.push(placed(*first, &batch.models[..]));
            }
        }
        Ok([opaque, transparent])
    }
}

fn texture(slot: &Option<MaterialTexture>) -> Option<ObjectTexture<'_>> {
    slot.as_ref().map(|(key, data, sampler)| ObjectTexture {
        data,
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
    /// The post pass's colour LUT is the identity on this layer; a LUT is recorded
    /// once as refused and the frame is graded without it.
    pub fn set_colour_lut(&mut self, lut: Option<crate::post::ColourLut>) -> Result<(), Error> {
        if lut.is_some() {
            self.refuse("set_colour_lut", "colour lookup tables");
        }
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
        Ok(self.state.textures.insert((self.textures, Arc::new(data))))
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
    /// Kept by nothing: there are no particles to step.
    pub fn set_particle_step(&mut self, _: f32) -> Result<(), Error> {
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
    pub fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        if !decals.is_empty() {
            self.refuse("set_decals", "decals");
        }
        Ok(())
    }

    pub fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        lighting(sun, &self.state.lights, self.state.sky, self.look.exposure)?;
        self.state.sun = sun;
        Ok(())
    }
    pub fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        lighting(self.state.sun, lights, self.state.sky, self.look.exposure)?;
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

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput("output extent must be nonzero"));
        }
        if self.size.is_some_and(|size| size != [width, height]) {
            self.restart();
        }
        self.size = Some([width, height]);
        Ok(())
    }
    /// Draws the next frame on `gpu` (the device every frame of this engine uses).
    pub fn render(&mut self, gpu: &mut GpuDevice, camera: &Camera) -> Result<FrameReport, Error> {
        self.release_ended(gpu)?;
        let size = self.size.ok_or(Error::InvalidInput(
            "call Engine::resize before Engine::render",
        ))?;
        let settings = self.state.settings;
        let look = self.look.clone();
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
        let mut media = atmosphere(self.state.sky, &settings);
        media.volumetric_fog = None;
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
        let skins: Vec<Option<Skin<'_>>> = opaque.iter().map(|p| p.skin).collect();
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
                    scale: self.render_scale,
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
        };
        let images = self
            .frames
            .render(gpu, &self.cache, &scene)
            .map_err(fault)?;
        // The renderer's counts are of its opaque draws; a pane is not among them.
        let triangles: usize = opaque
            .iter()
            .map(|p| p.indices.len() / 3 * p.placements.len())
            .sum();
        let instances: usize = opaque.iter().map(|p| p.placements.len()).sum();
        self.last = Some(images);
        self.frame += 1;
        Ok(FrameReport {
            view: self.view.unwrap_or(ViewId(0)),
            frame: FrameId(self.frame),
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
    pub fn cut(&mut self) {
        self.frames.cut();
    }
    pub fn capture(&mut self) -> Result<Image, Error> {
        let size = self.size.ok_or(Error::InvalidInput(
            "call Engine::resize before Engine::capture",
        ))?;
        let display = self
            .last
            .as_ref()
            .and_then(|images| images.display.clone())
            .ok_or(Error::InvalidInput(
                "no displayed frame to capture: render with post enabled first",
            ))?;
        Ok(Image {
            width: size[0],
            height: size[1],
            rgba8: display,
        })
    }
    // ---- One view (docs/design-views.md). This layer is not ported to views: it renders
    // a single view, and the view calls map onto `resize`, `render`, `cut` and `capture`.

    /// The view: a second is refused.
    pub fn create_view(&mut self, desc: &ViewDesc) -> Result<ViewId, Error> {
        if self.view.is_some() {
            return Err(Error::InvalidInput("the ara-gpu layer renders one view"));
        }
        self.set_look(desc)?;
        let view = ViewId(0);
        self.view = Some(view);
        Ok(view)
    }
    pub fn set_view(&mut self, view: ViewId, desc: &ViewDesc) -> Result<(), Error> {
        self.check_view(view)?;
        self.set_look(desc)
    }
    pub fn remove_view(&mut self, view: ViewId) -> Result<(), Error> {
        self.check_view(view)?;
        self.view = None;
        Ok(())
    }
    pub fn render_view(
        &mut self,
        gpu: &mut GpuDevice,
        view: ViewId,
        camera: &Camera,
    ) -> Result<FrameReport, Error> {
        self.check_view(view)?;
        self.render(gpu, camera)
    }
    pub fn cut_view(&mut self, view: ViewId) {
        if self.view == Some(view) {
            self.cut();
        }
    }
    pub fn capture_view(&mut self, view: ViewId) -> Result<Image, Error> {
        self.check_view(view)?;
        self.capture()
    }
    /// One engine frame: its particle step (this layer has no particles).
    pub fn tick(&mut self, seconds: f32) -> Result<(), Error> {
        self.set_particle_step(seconds)
    }
    fn check_view(&self, view: ViewId) -> Result<(), Error> {
        if self.view != Some(view) {
            return Err(Error::InvalidInput("unknown or removed view"));
        }
        Ok(())
    }
    /// The view's size, look and render scale. A change of look or scale starts a new
    /// frame sequence, as a settings change does.
    fn set_look(&mut self, desc: &ViewDesc) -> Result<(), Error> {
        desc.look.post.validate()?;
        if desc.outputs.ids {
            return Err(Error::InvalidInput("the ara-gpu layer has no id output"));
        }
        if desc.view_mode != crate::ViewMode::Lit {
            self.refuse("create_view", "view_mode");
        }
        if !desc.look.post.enabled {
            self.refuse("create_view", "look.post.enabled");
        }
        if desc.look.colour_lut.is_some() {
            self.refuse("create_view", "look.colour_lut");
        }
        self.resize(desc.width, desc.height)?;
        if desc.look != self.look || desc.render_scale != self.render_scale {
            self.restart();
        }
        self.look = desc.look.clone();
        self.render_scale = desc.render_scale;
        Ok(())
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
