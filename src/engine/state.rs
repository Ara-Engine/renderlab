//! The engine's scene state without a device: resource slots, the batch table with object
//! transforms, and the frame state (sun, lights, sky, settings).
//!
//! [`State`] is generic over what a GPU layer keeps for each resource ([`Residency`]): the
//! wgpu [`Engine`](super::Engine) keeps renderer handles, so its state is laid out as the
//! engine's own fields were. The bookkeeping here needs no device: stale handles, spawning
//! and despawning into mesh-material batches, and the still-used checks on removal. What
//! reaches a device (uploads, applying lighting and settings) stays with the layer.
use super::{
    EmitterId, EnvironmentId, MaterialId, MeshId, ObjectId, ProgramId, Settings, Sky, Sun,
    TextureId,
    ids::{Id, Slots, kind},
};
use crate::{Error, Light, Mat4};
use std::collections::HashMap;

/// What one GPU layer keeps for each engine resource: the payload of each slot.
pub(crate) trait Residency {
    type Mesh;
    type Texture;
    type Program;
    type Material;
    type Environment;
    type Emitter;
    /// One batch's uploaded transforms.
    type Instances;
    /// Writes one transform of a live batch, with the layer's own finite check.
    fn set_instance(
        instances: &mut Self::Instances,
        slot: usize,
        transform: Mat4,
    ) -> Result<(), Error>;
    /// Writes one placement's shadow casting into a live batch (`Batch::casts`).
    fn set_instance_casts_shadows(
        instances: &mut Self::Instances,
        slot: usize,
        casts: bool,
    ) -> Result<(), Error>;
}

/// A material and the program it names.
pub(crate) struct MaterialEntry<R: Residency> {
    pub material: R::Material,
    pub program: ProgramId,
}
struct ObjectEntry {
    batch: Id<kind::Batch>,
    slot: usize,
}
/// Objects sharing a mesh and material: one instance collection, one draw.
pub(crate) struct Batch<R: Residency> {
    pub mesh: MeshId,
    pub material: MaterialId,
    pub models: Vec<Mat4>,
    /// Per slot, whether the object blocks light (`set_casts_shadows`; spawned true).
    pub casts: Vec<bool>,
    pub objects: Vec<ObjectId>,
    /// None until the next frame uploads the current `models` (after spawn/despawn).
    pub instances: Option<R::Instances>,
}

/// The scene one engine holds. Resource slots are filled by the layer after its uploads;
/// objects and batches are kept here alone.
pub(crate) struct State<R: Residency> {
    pub meshes: Slots<kind::Mesh, R::Mesh>,
    pub textures: Slots<kind::Texture, R::Texture>,
    pub programs: Slots<kind::Program, R::Program>,
    pub materials: Slots<kind::Material, MaterialEntry<R>>,
    pub environments: Slots<kind::Environment, R::Environment>,
    objects: Slots<kind::Object, ObjectEntry>,
    pub emitters: Slots<kind::Emitter, R::Emitter>,
    pub batches: Slots<kind::Batch, Batch<R>>,
    batch_keys: HashMap<(MeshId, MaterialId), Id<kind::Batch>>,
    /// The built-in program, which materials without a program use.
    pub standard: ProgramId,
    pub sun: Option<Sun>,
    pub lights: Vec<Light>,
    pub sky: Sky,
    pub settings: Settings,
}
impl<R: Residency> State<R> {
    /// An empty scene holding the layer's standard program, with no sun, the default sky
    /// and `settings`.
    pub fn new(standard: R::Program, settings: Settings) -> Self {
        let mut programs = Slots::new("unknown or removed program");
        let standard = programs.insert(standard);
        Self {
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
            settings,
        }
    }

    /// Releases the mesh once no object uses it.
    pub fn remove_mesh(&mut self, mesh: MeshId) -> Result<R::Mesh, Error> {
        self.meshes.get(mesh)?;
        if self.batch_keys.keys().any(|(m, _)| *m == mesh) {
            return Err(Error::InvalidInput("mesh is still used by objects"));
        }
        self.meshes.remove(mesh)
    }
    pub fn remove_texture(&mut self, texture: TextureId) -> Result<R::Texture, Error> {
        self.textures.remove(texture)
    }
    /// Every program but the standard one can be removed.
    pub fn remove_program(&mut self, program: ProgramId) -> Result<R::Program, Error> {
        if program == self.standard {
            return Err(Error::InvalidInput(
                "the standard program cannot be removed",
            ));
        }
        self.programs.remove(program)
    }
    /// Releases the material once no object uses it.
    pub fn remove_material(&mut self, material: MaterialId) -> Result<MaterialEntry<R>, Error> {
        self.materials.get(material)?;
        if self.batch_keys.keys().any(|(_, m)| *m == material) {
            return Err(Error::InvalidInput("material is still used by objects"));
        }
        self.materials.remove(material)
    }
    pub fn remove_environment(
        &mut self,
        environment: EnvironmentId,
    ) -> Result<R::Environment, Error> {
        self.environments.remove(environment)
    }
    pub fn remove_emitter(&mut self, emitter: EmitterId) -> Result<R::Emitter, Error> {
        self.emitters.remove(emitter)
    }

    /// Places a mesh with a material in their batch, creating it for a new pair. At most
    /// `max_instances` objects share a batch. The batch's instances are dropped, which
    /// tells the layer to upload its transforms again.
    pub fn spawn(
        &mut self,
        mesh: MeshId,
        material: MaterialId,
        transform: Mat4,
        max_instances: u32,
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
                    casts: Vec::new(),
                    objects: Vec::new(),
                    instances: None,
                });
                self.batch_keys.insert((mesh, material), batch);
                batch
            }
        };
        let entry = self.batches.get_mut(batch)?;
        if entry.models.len() >= max_instances as usize {
            return Err(Error::InvalidInput(
                "objects per mesh-material pair exceed Limits::max_instances",
            ));
        }
        let slot = entry.models.len();
        let object = self.objects.insert(ObjectEntry { batch, slot });
        entry.models.push(transform);
        entry.casts.push(true);
        entry.objects.push(object);
        entry.instances = None;
        Ok(object)
    }
    /// Moves an object. A live batch takes the write first, with the layer's own check;
    /// one waiting for upload is checked here.
    pub fn set_transform(&mut self, object: ObjectId, transform: Mat4) -> Result<(), Error> {
        let &ObjectEntry { batch, slot } = self.objects.get(object)?;
        let entry = self.batches.get_mut(batch)?;
        if let Some(instances) = &mut entry.instances {
            R::set_instance(instances, slot, transform)?;
        } else if !transform.is_finite() {
            return Err(Error::InvalidInput("object transform must be finite"));
        }
        entry.models[slot] = transform;
        Ok(())
    }
    /// Whether an object blocks light; a live batch takes the write too.
    pub fn set_casts_shadows(&mut self, object: ObjectId, casts: bool) -> Result<(), Error> {
        let &ObjectEntry { batch, slot } = self.objects.get(object)?;
        let entry = self.batches.get_mut(batch)?;
        if let Some(instances) = &mut entry.instances {
            R::set_instance_casts_shadows(instances, slot, casts)?;
        }
        entry.casts[slot] = casts;
        Ok(())
    }
    pub fn casts_shadows(&self, object: ObjectId) -> Result<bool, Error> {
        let entry = self.objects.get(object)?;
        Ok(self.batches.get(entry.batch)?.casts[entry.slot])
    }
    pub fn transform(&self, object: ObjectId) -> Result<Mat4, Error> {
        let entry = self.objects.get(object)?;
        Ok(self.batches.get(entry.batch)?.models[entry.slot])
    }
    /// Removes an object; the batch's last object takes its slot, and an emptied batch
    /// goes with it.
    pub fn despawn(&mut self, object: ObjectId) -> Result<(), Error> {
        let ObjectEntry { batch, slot } = self.objects.remove(object)?;
        let entry = self.batches.get_mut(batch)?;
        entry.models.swap_remove(slot);
        entry.casts.swap_remove(slot);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer that keeps names, and whose live batches record their writes.
    struct Names;
    impl Residency for Names {
        type Mesh = &'static str;
        type Texture = ();
        type Program = &'static str;
        type Material = &'static str;
        type Environment = ();
        type Emitter = ();
        type Instances = Vec<(usize, Mat4)>;
        fn set_instance(
            instances: &mut Self::Instances,
            slot: usize,
            transform: Mat4,
        ) -> Result<(), Error> {
            if !transform.is_finite() {
                return Err(Error::InvalidInput("the layer's own check"));
            }
            instances.push((slot, transform));
            Ok(())
        }
        fn set_instance_casts_shadows(
            _: &mut Self::Instances,
            _: usize,
            _: bool,
        ) -> Result<(), Error> {
            Ok(())
        }
    }
    fn at(x: f32) -> Mat4 {
        Mat4::from_translation(crate::Vec3::new(x, 0.0, 0.0))
    }
    fn scene() -> (State<Names>, MeshId, MaterialId) {
        let mut state = State::<Names>::new("standard", Settings::default());
        let mesh = state.meshes.insert("cube");
        let material = state.materials.insert(MaterialEntry {
            material: "stone",
            program: state.standard,
        });
        (state, mesh, material)
    }

    #[test]
    fn objects_of_one_mesh_and_material_share_a_batch_and_keep_their_transforms() {
        let (mut state, mesh, material) = scene();
        let other = state.meshes.insert("sphere");
        let a = state.spawn(mesh, material, at(1.0), 8).unwrap();
        let b = state.spawn(mesh, material, at(2.0), 8).unwrap();
        let c = state.spawn(other, material, at(3.0), 8).unwrap();
        assert_eq!(state.batches.len(), 2);
        let d = state.spawn(mesh, material, at(4.0), 8).unwrap();
        // Despawning the first moves the batch's last object into its slot.
        state.despawn(a).unwrap();
        assert_eq!(state.transform(b).unwrap(), at(2.0));
        assert_eq!(state.transform(d).unwrap(), at(4.0));
        state.set_transform(d, at(5.0)).unwrap();
        assert_eq!(state.transform(d).unwrap(), at(5.0));
        assert_eq!(state.transform(b).unwrap(), at(2.0));
        assert!(state.transform(a).is_err(), "a despawned object is stale");
        assert_eq!(state.object_count(), 3);
        // The batch goes with its last object, and its pair can be spawned again.
        state.despawn(c).unwrap();
        assert_eq!(state.batches.len(), 1);
        assert_eq!(state.remove_mesh(other).unwrap(), "sphere");
        assert!(state.spawn(other, material, at(0.0), 8).is_err());
    }

    #[test]
    fn shadow_casting_is_per_object_and_moves_with_its_slot() {
        let (mut state, mesh, material) = scene();
        let a = state.spawn(mesh, material, at(1.0), 8).unwrap();
        let b = state.spawn(mesh, material, at(2.0), 8).unwrap();
        assert!(state.casts_shadows(a).unwrap() && state.casts_shadows(b).unwrap());
        state.set_casts_shadows(b, false).unwrap();
        // Despawning the first moves the last object, and its casting, into slot 0.
        state.despawn(a).unwrap();
        assert!(!state.casts_shadows(b).unwrap());
        let batch = state.batches.ids().next().unwrap();
        assert_eq!(state.batches.get(batch).unwrap().casts, [false]);
    }

    #[test]
    fn spawning_is_bounded_per_batch_and_checks_its_transform() {
        let (mut state, mesh, material) = scene();
        state.spawn(mesh, material, at(0.0), 2).unwrap();
        state.spawn(mesh, material, at(1.0), 2).unwrap();
        assert!(state.spawn(mesh, material, at(2.0), 2).is_err());
        assert!(
            state
                .spawn(mesh, material, Mat4::from_scale(crate::Vec3::NAN), 8)
                .is_err()
        );
        assert_eq!(state.object_count(), 2);
    }

    #[test]
    fn a_live_batch_takes_the_write_first_and_a_change_of_membership_drops_it() {
        let (mut state, mesh, material) = scene();
        let a = state.spawn(mesh, material, at(0.0), 8).unwrap();
        let b = state.spawn(mesh, material, at(1.0), 8).unwrap();
        let nan = Mat4::from_scale(crate::Vec3::NAN);
        // Waiting for upload: checked here.
        assert_eq!(
            state.set_transform(a, nan).unwrap_err().to_string(),
            Error::InvalidInput("object transform must be finite").to_string()
        );
        let batch = state.batches.ids().next().unwrap();
        state.batches.get_mut(batch).unwrap().instances = Some(Vec::new());
        // Live: the layer checks, and a rejected write leaves the stored transform.
        assert_eq!(
            state.set_transform(b, nan).unwrap_err().to_string(),
            Error::InvalidInput("the layer's own check").to_string()
        );
        assert_eq!(state.transform(b).unwrap(), at(1.0));
        state.set_transform(b, at(7.0)).unwrap();
        let live = state.batches.get(batch).unwrap();
        assert_eq!(live.instances.as_deref(), Some(&[(1, at(7.0))][..]));
        assert_eq!(live.models, [at(0.0), at(7.0)]);
        state.despawn(a).unwrap();
        assert!(state.batches.get(batch).unwrap().instances.is_none());
    }

    #[test]
    fn resources_in_use_and_the_standard_program_cannot_be_removed() {
        let (mut state, mesh, material) = scene();
        let object = state.spawn(mesh, material, at(0.0), 8).unwrap();
        assert!(state.remove_mesh(mesh).is_err());
        assert!(state.remove_material(material).is_err());
        assert!(state.remove_program(state.standard).is_err());
        let program = state.programs.insert("toon");
        assert_eq!(state.remove_program(program).unwrap(), "toon");
        state.despawn(object).unwrap();
        assert_eq!(state.remove_material(material).unwrap().material, "stone");
        assert_eq!(state.remove_mesh(mesh).unwrap(), "cube");
        assert!(state.remove_mesh(mesh).is_err(), "a removed mesh is stale");
    }
}
