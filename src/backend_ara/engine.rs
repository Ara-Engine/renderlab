//! The engine-level entry: the same scene-building and frame calls as
//! [`crate::engine::Engine`], recorded into the engine's device-free state and rendered
//! through [`forward::Frames`].
//!
//! A scene written against [`crate::engine::Scene`] runs on either layer. This one does
//! not implement everything: a call or setting it cannot honour is recorded as a
//! [`Refusal`] and the scene goes on without that piece, so a run ends with every gap
//! named. What the frame itself rejects (no sun, more than `TRACED_DRAWS` objects) is an
//! error from [`Scene::render`].
use super::{
    cache::ShaderCache,
    forward::{
        self, ForwardImages, ForwardObject, ForwardScene, ObjectProgram, ObjectTexture,
        ObjectTextures, Primary, ScenePrograms, SceneTextures, SunShadows,
    },
    gi_cache::Indirect,
    temporal::Reconstruct,
};
use crate::{
    Camera, Decal, EmitterDescriptor, Error, FrameId, FrameReport, Light, Mat4, MaterialDescriptor,
    MeshData, PrimaryVisibility, SoftShadows, Value, Vertex,
    asset::{
        Asset, Primitive, Sampler, Texture as TextureData, environment::PreparedEnvironment,
        skin::Influence,
    },
    engine::{
        AssetHandles, EmitterId, EnvironmentId, Image, MaterialDesc, MaterialId, MeshId, ObjectId,
        ProgramId, Quality, Scene, Settings, Sky, Sun, TextureId,
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
/// Geometry the frame is rebuilt from, or a mesh this layer refused.
enum Mesh {
    Static {
        /// Its identity to the frames, which keep its buffers and bottom-level
        /// structure on the GPU while a frame names it. Never reused.
        key: u64,
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
    },
    Refused,
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
    base_color_texture: Option<(Arc<TextureData>, Sampler)>,
    metallic_roughness_texture: Option<(Arc<TextureData>, Sampler)>,
    parameters: Vec<(String, Value)>,
}

/// What this layer keeps for each resource: the data its frames are rebuilt from.
struct Kept;
impl Residency for Kept {
    type Mesh = Mesh;
    type Texture = Arc<TextureData>;
    type Program = Arc<Program>;
    type Material = Material;
    type Environment = ();
    type Emitter = ();
    /// Nothing is kept resident between frames, so no batch is ever live.
    type Instances = ();
    fn set_instance(_: &mut (), _: usize, _: Mat4) -> Result<(), Error> {
        Ok(())
    }
}

/// The scene and its frames on ara-gpu. Build it with the [`Scene`] calls, read
/// [`refusals`](Self::refusals) for what was left out, and [`retire`](Self::retire) it
/// to get the device back.
pub struct Engine {
    gpu: GpuDevice,
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
    /// Whether frames are read back (`set_readback`).
    readback: bool,
    refusals: Vec<Refusal>,
}

/// One draw of a frame: its geometry, material, first transform, mesh identity and
/// every transform it is placed at.
type Placed<'a> = (&'a [Vertex], &'a [u32], &'a Material, Mat4, u64, &'a [Mat4]);

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
    /// An empty scene on `gpu`, compiling through `cache`, with `settings`.
    pub fn new(gpu: GpuDevice, cache: ShaderCache, settings: Settings) -> Result<Self, Error> {
        let standard = Arc::new(Program::new("standard", crate::STANDARD_PROGRAM)?);
        let mut engine = Self {
            gpu,
            cache,
            state: State::new(standard, settings),
            frames: forward::Frames::default(),
            size: None,
            last: None,
            frame: 0,
            meshes: 0,
            materials: 0,
            readback: true,
            refusals: Vec::new(),
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
    pub fn set_readback(&mut self, on: bool) -> Result<(), Error> {
        if on != self.readback {
            self.readback = on;
            self.restart()?;
        }
        Ok(())
    }
    /// Starts a new frame sequence: the history's size and settings are fixed within
    /// one, so a change of either releases the old sequence's cache and history.
    fn restart(&mut self) -> Result<(), Error> {
        let fresh = if self.readback {
            forward::Frames::default()
        } else {
            forward::Frames::without_images()
        };
        std::mem::replace(&mut self.frames, fresh)
            .retire(&mut self.gpu)
            .map_err(fault)
    }
    /// Releases the frame history and the pipelines and returns the device.
    pub fn retire(mut self) -> anyhow::Result<GpuDevice> {
        self.frames.retire(&mut self.gpu)?;
        self.cache.retire(&mut self.gpu)?;
        Ok(self.gpu)
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
        if !asked.post.enabled {
            self.refuse("set_settings", "post.enabled");
        }
        // Every field named here is one this layer reads; a field added to `Settings`
        // later is missing from the list and so compares against its default.
        #[allow(clippy::needless_update)]
        let known = Settings {
            quality: asked.quality,
            global_illumination: asked.global_illumination,
            temporal_reconstruction: asked.temporal_reconstruction,
            render_scale: asked.render_scale,
            exposure: asked.exposure,
            aerial_perspective: asked.aerial_perspective,
            height_fog: asked.height_fog,
            volumetric_fog: asked.volumetric_fog,
            local_shadows: asked.local_shadows,
            post: asked.post,
            reflections: asked.reflections,
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
            let Mesh::Static {
                key,
                vertices,
                indices,
            } = state.meshes.get(batch.mesh)?
            else {
                continue;
            };
            let material = &state.materials.get(batch.material)?.material;
            if material.transparency.is_some() {
                transparent.extend(batch.models.iter().map(|model| {
                    (
                        &vertices[..],
                        &indices[..],
                        material,
                        *model,
                        *key,
                        std::slice::from_ref(model),
                    )
                }));
            } else if let Some(first) = batch.models.first() {
                opaque.push((
                    &vertices[..],
                    &indices[..],
                    material,
                    *first,
                    *key,
                    &batch.models[..],
                ));
            }
        }
        Ok([opaque, transparent])
    }
}

fn texture(slot: &Option<(Arc<TextureData>, Sampler)>) -> Option<ObjectTexture<'_>> {
    slot.as_ref().map(|(data, sampler)| ObjectTexture {
        data,
        sampler: *sampler,
    })
}
fn objects<'a>(placed: &[Placed<'a>]) -> Vec<ForwardObject<'a>> {
    placed
        .iter()
        .map(
            |&(vertices, indices, material, transform, _, _)| ForwardObject {
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

impl Scene for Engine {
    fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error> {
        self.meshes += 1;
        Ok(self.state.meshes.insert(Mesh::Static {
            key: self.meshes,
            vertices: data.vertices.to_vec(),
            indices: data.indices.to_vec(),
        }))
    }
    fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        Ok(self.state.textures.insert(Arc::new(data)))
    }
    fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        let program = Program::new(label, source)?;
        Ok(self.state.programs.insert(Arc::new(program)))
    }
    fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        let program_id = desc.program.unwrap_or(self.state.standard);
        let program = self.state.programs.get(program_id)?.clone();
        ParameterLayout::check(
            program.layout.as_ref(),
            desc.parameters.iter().map(|(n, v)| (*n, v.clone())),
        )?;
        let texture = |slot: Option<crate::engine::TextureRef>| {
            slot.map(|t| Ok::<_, Error>((self.state.textures.get(t.texture)?.clone(), t.sampler)))
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
            parameters: desc
                .parameters
                .iter()
                .map(|(n, v)| ((*n).to_owned(), v.clone()))
                .collect(),
            program,
        };
        if let Some(normal) = desc.normal_texture {
            // Checked as the others are; the map needs the mesh's tangent stream, which
            // this layer's meshes do not keep yet.
            self.state.textures.get(normal.texture)?;
            self.refuse("create_material", "normal_texture");
        }
        Ok(self.state.materials.insert(MaterialEntry {
            material,
            program: program_id,
        }))
    }
    fn set_material_parameters(
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
    fn create_environment(&mut self, _: &PreparedEnvironment) -> Result<EnvironmentId, Error> {
        self.refuse("create_environment", "environment maps");
        Ok(self.state.environments.insert(()))
    }
    fn add_asset(&mut self, _: Asset) -> Result<AssetHandles, Error> {
        self.refuse("add_asset", "cooked assets");
        Ok(AssetHandles::default())
    }
    fn create_skinned_mesh(
        &mut self,
        _: &Primitive,
        _: &[Influence],
        _: u32,
    ) -> Result<MeshId, Error> {
        self.refuse("create_skinned_mesh", "skinned meshes");
        Ok(self.state.meshes.insert(Mesh::Refused))
    }
    fn set_joint_palette(&mut self, mesh: MeshId, _: &[Mat4]) -> Result<(), Error> {
        match self.state.meshes.get(mesh)? {
            Mesh::Refused => Ok(()),
            Mesh::Static { .. } => Err(Error::InvalidInput("mesh is not skinned")),
        }
    }
    fn create_emitter(
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
    fn set_particle_step(&mut self, _: f32) -> Result<(), Error> {
        Ok(())
    }

    fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error> {
        self.state.spawn(mesh, material, transform, u32::MAX)
    }
    fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        self.state.set_transform(object, transform)
    }
    fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        if !decals.is_empty() {
            self.refuse("set_decals", "decals");
        }
        Ok(())
    }

    fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        lighting(
            sun,
            &self.state.lights,
            self.state.sky,
            self.state.settings.exposure,
        )?;
        self.state.sun = sun;
        Ok(())
    }
    fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        lighting(
            self.state.sun,
            lights,
            self.state.sky,
            self.state.settings.exposure,
        )?;
        self.state.lights = lights.to_vec();
        Ok(())
    }
    fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        if let Sky::Map { map, .. } = sky {
            // The previous sky stays.
            self.state.environments.get(map)?;
            self.refuse("set_sky", "Sky::Map");
            return Ok(());
        }
        atmosphere(sky, &self.state.settings).validate()?;
        self.state.sky = sky;
        Ok(())
    }
    fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        atmosphere(self.state.sky, &settings).validate()?;
        settings.post.validate()?;
        self.refuse_settings(&settings);
        if settings != self.state.settings {
            self.restart()?;
        }
        self.state.settings = settings;
        Ok(())
    }
    fn settings(&self) -> Settings {
        self.state.settings
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput("output extent must be nonzero"));
        }
        if self.size.is_some_and(|size| size != [width, height]) {
            self.restart()?;
        }
        self.size = Some([width, height]);
        Ok(())
    }
    fn render(&mut self, camera: &Camera) -> Result<FrameReport, Error> {
        let size = self.size.ok_or(Error::InvalidInput(
            "call Engine::resize before Engine::render",
        ))?;
        let settings = self.state.settings;
        let [opaque, transparent] = Self::placed(&self.state)?;
        // The frame's texture table covers its opaque objects alone.
        if transparent
            .iter()
            .any(|p| p.2.base_color_texture.is_some() || p.2.metallic_roughness_texture.is_some())
        {
            note(
                &mut self.refusals,
                "render",
                "textures on a transparent material",
            );
        }
        let (lit, shadows) = lighting(
            self.state.sun,
            &self.state.lights,
            self.state.sky,
            settings.exposure,
        )?;
        let sun_shadows = shadows.requests.first().and_then(|r| match r {
            crate::ShadowRequest::Directional { split_ends } => Some(SunShadows {
                split_ends: split_ends.clone(),
                disk_angle: shadows.directional_disk_angle,
            }),
            _ => None,
        });
        let mut media = atmosphere(self.state.sky, &settings);
        media.volumetric_fog = None;
        let opaque_objects = objects(&opaque);
        let keys: Vec<u64> = opaque.iter().map(|p| p.4).collect();
        let material_keys: Vec<u64> = opaque.iter().map(|p| p.2.key).collect();
        let placements: Vec<&[Mat4]> = opaque.iter().map(|p| p.5).collect();
        let transparent_objects = objects(&transparent);
        let values: Vec<Vec<(&str, Value)>> = opaque
            .iter()
            .map(|p| {
                p.2.parameters
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
                    source: &p.2.program.source,
                    parameters: values,
                })
            })
            .collect();
        // A pane's program by its source; its parameter block is not assembled.
        let pane_programs: Vec<_> = transparent
            .iter()
            .map(|p| {
                Some(ObjectProgram {
                    source: &p.2.program.source,
                    parameters: &[],
                })
            })
            .collect();
        if transparent.iter().any(|p| p.2.program.layout.is_some()) {
            return Err(Error::InvalidInput(
                "a transparent material's program with Parameters is not assembled",
            ));
        }
        let textures: Vec<_> = opaque
            .iter()
            .map(|p| ObjectTextures {
                base_color: texture(&p.2.base_color_texture),
                metallic_roughness: texture(&p.2.metallic_roughness_texture),
                normal: None,
            })
            .collect();
        let textured = textures
            .iter()
            .any(|t| t.base_color.is_some() || t.metallic_roughness.is_some());
        let scene = ForwardScene {
            objects: &opaque_objects,
            lighting: &lit,
            camera: *camera,
            size,
            atmosphere: media,
            sun_shadows: sun_shadows.as_ref(),
            primary: match settings.quality {
                Quality::Realtime => Primary::TracedInteractive,
                Quality::Reference => Primary::Traced,
            },
            post: Some(settings.post),
            indirect: settings.global_illumination.then(|| Indirect {
                settings: crate::IndirectSettings::default(),
                read_table: false,
            }),
            reconstruction: settings.temporal_reconstruction.then(|| Reconstruct {
                settings: crate::Reconstruction {
                    scale: settings.render_scale,
                    ..Default::default()
                },
                read_history: false,
            }),
            reflections: Some(settings.reflections),
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
        };
        let images = self
            .frames
            .render(&mut self.gpu, &self.cache, &scene)
            .map_err(fault)?;
        // The renderer's counts are of its opaque draws; a pane is not among them.
        let triangles: usize = opaque.iter().map(|p| p.1.len() / 3 * p.5.len()).sum();
        let instances: usize = opaque.iter().map(|p| p.5.len()).sum();
        self.last = Some(images);
        self.frame += 1;
        Ok(FrameReport {
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
            soft_shadows: match settings.quality {
                Quality::Realtime => SoftShadows::Interactive,
                Quality::Reference => SoftShadows::Exact,
            },
        })
    }
    fn cut(&mut self) {
        self.frames.cut();
    }
    fn capture(&mut self) -> Result<Image, Error> {
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
}
