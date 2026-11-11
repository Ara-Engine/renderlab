//! The engine's scene-building and frame calls as a trait, so that one description of a
//! scene can be rendered by more than one GPU layer.
//!
//! [`Engine`] implements [`Scene`] by delegating to its own methods, which stay the
//! documented API (docs/api.md): nothing that uses an `Engine` needs this trait in scope.
//! It holds the calls a scene shared between layers uses, and no more. Device and window
//! calls (`Engine::device`, `render_to`, `present`, `reload_program` and their kin) are
//! the wgpu engine's alone.
use super::{AssetHandles, EmitterId};
use super::{
    Engine, EnvironmentId, Image, MaterialDesc, MaterialId, MeshId, ObjectId, ProgramId, Settings,
    SkinnedHandles, Sky, Sun, TextureId, asset_material,
};
use crate::{
    Camera, Decal, EmitterDescriptor, Error, FrameReport, Light, Mat4, MeshData, Value,
    asset::{
        Asset, Primitive, Texture as TextureData,
        environment::PreparedEnvironment,
        skin::{Influence, SkinnedAsset},
    },
};

/// A scene under construction and its frames, as [`Engine`] documents each call.
pub trait Scene {
    fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error>;
    /// Takes the texture by value: its levels move into the upload.
    fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error>;
    fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error>;
    fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error>;
    fn set_material_parameters(
        &mut self,
        material: MaterialId,
        values: &[(&str, Value)],
    ) -> Result<(), Error>;
    fn create_environment(&mut self, data: &PreparedEnvironment) -> Result<EnvironmentId, Error>;
    /// Takes the asset by value: its vertex data moves into the upload. Each layer
    /// prepares meshes in its own way, so each implements this.
    fn add_asset(&mut self, asset: Asset) -> Result<AssetHandles, Error>;
    fn create_skinned_mesh(
        &mut self,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<MeshId, Error>;
    fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error>;
    /// Uploads a cooked skinned asset: its textures, its materials (double-sided, as the
    /// shadowed paths need) and one posable mesh per skinned primitive of each binding.
    /// Built from the calls above, so it is the same on every layer.
    fn add_skinned_asset(&mut self, asset: &SkinnedAsset) -> Result<SkinnedHandles, Error> {
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
    /// `desc.texture` must be None: the sprite is `sprite`.
    fn create_emitter(
        &mut self,
        desc: &EmitterDescriptor<'_>,
        sprite: Option<TextureId>,
    ) -> Result<EmitterId, Error>;
    fn set_particle_step(&mut self, seconds: f32) -> Result<(), Error>;

    fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error>;
    fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error>;
    fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error>;

    fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error>;
    fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error>;
    fn set_sky(&mut self, sky: Sky) -> Result<(), Error>;
    /// The settings as one value: a layer that lacks a feature answers for the fields it
    /// cannot honour, and new fields cost this trait nothing.
    fn set_settings(&mut self, settings: Settings) -> Result<(), Error>;
    fn settings(&self) -> Settings;

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Error>;
    fn render(&mut self, camera: &Camera) -> Result<FrameReport, Error>;
    fn cut(&mut self);
    fn capture(&mut self) -> Result<Image, Error>;
}

impl Scene for Engine {
    fn create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshId, Error> {
        Engine::create_mesh(self, data)
    }
    fn create_texture(&mut self, data: TextureData) -> Result<TextureId, Error> {
        Engine::create_texture(self, data)
    }
    fn create_program(&mut self, label: &str, source: &str) -> Result<ProgramId, Error> {
        Engine::create_program(self, label, source)
    }
    fn create_material(&mut self, desc: &MaterialDesc<'_>) -> Result<MaterialId, Error> {
        Engine::create_material(self, desc)
    }
    fn set_material_parameters(
        &mut self,
        material: MaterialId,
        values: &[(&str, Value)],
    ) -> Result<(), Error> {
        Engine::set_material_parameters(self, material, values)
    }
    fn create_environment(&mut self, data: &PreparedEnvironment) -> Result<EnvironmentId, Error> {
        Engine::create_environment(self, data)
    }
    fn add_asset(&mut self, asset: Asset) -> Result<AssetHandles, Error> {
        Engine::add_asset(self, asset)
    }
    fn create_skinned_mesh(
        &mut self,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<MeshId, Error> {
        Engine::create_skinned_mesh(self, primitive, influences, joint_count)
    }
    fn set_joint_palette(&mut self, mesh: MeshId, palette: &[Mat4]) -> Result<(), Error> {
        Engine::set_joint_palette(self, mesh, palette)
    }
    fn create_emitter(
        &mut self,
        desc: &EmitterDescriptor<'_>,
        sprite: Option<TextureId>,
    ) -> Result<EmitterId, Error> {
        Engine::create_emitter(self, desc, sprite)
    }
    fn set_particle_step(&mut self, seconds: f32) -> Result<(), Error> {
        Engine::set_particle_step(self, seconds)
    }
    fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
    ) -> Result<ObjectId, Error> {
        Engine::spawn(self, mesh, material, transform)
    }
    fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        Engine::set_transform(self, object, transform)
    }
    fn set_decals(&mut self, decals: &[Decal]) -> Result<(), Error> {
        Engine::set_decals(self, decals)
    }
    fn set_sun(&mut self, sun: Option<Sun>) -> Result<(), Error> {
        Engine::set_sun(self, sun)
    }
    fn set_lights(&mut self, lights: &[Light]) -> Result<(), Error> {
        Engine::set_lights(self, lights)
    }
    fn set_sky(&mut self, sky: Sky) -> Result<(), Error> {
        Engine::set_sky(self, sky)
    }
    fn set_settings(&mut self, settings: Settings) -> Result<(), Error> {
        Engine::set_settings(self, settings)
    }
    fn settings(&self) -> Settings {
        Engine::settings(self)
    }
    fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        Engine::resize(self, width, height)
    }
    fn render(&mut self, camera: &Camera) -> Result<FrameReport, Error> {
        Engine::render(self, camera)
    }
    fn cut(&mut self) {
        Engine::cut(self)
    }
    fn capture(&mut self) -> Result<Image, Error> {
        Engine::capture(self)
    }
}
