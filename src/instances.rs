use crate::{
    Error, Mat4,
    mesh::matrix_bytes,
    observe::Owned,
    visibility::{Aabb, ClipVolume, HomogeneousWorldBounds},
};
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    /// Requires the shader's camera * model * original mesh position contract.
    #[default]
    Frustum,
    Unculled,
}

/// Fixed, nonempty transform slots. Owns its buffers, without retaining any mesh borrow.
/// Pending edits survive backpressure. Dropping releases ownership; wgpu retains submitted uses.
pub struct Instances {
    pub(crate) renderer: Arc<()>,
    pub(crate) state: InstanceState,
    pub(crate) normals: Owned<wgpu::Buffer>,
    pub(crate) transforms: Owned<wgpu::Buffer>,
    pub(crate) id_buffer: Owned<wgpu::Buffer>,
    pub(crate) bind: Owned<wgpu::BindGroup>,
}
impl Instances {
    pub fn len(&self) -> usize {
        self.state.models.len()
    }
    pub fn is_empty(&self) -> bool {
        self.state.models.is_empty()
    }
    /// No GPU work. Bit-identical edits are no-ops. A -> B -> A stays dirty until submission.
    pub fn set(&mut self, slot: usize, model: Mat4) -> Result<(), Error> {
        self.state.set(slot, model)
    }
    /// Whether the placement in `slot` blocks light (default true). A non-casting placement is
    /// drawn, reflected, bounced and shaded as before, and receives shadows, but no shadow test
    /// on either path sees it: shadow rays and any-hit walks skip it, and the sun clipmap, the
    /// cascades and the local shadow atlas leave it out. No GPU work; unchanged values are no-ops.
    pub fn set_casts_shadows(&mut self, slot: usize, casts: bool) -> Result<(), Error> {
        self.state.set_casts_shadows(slot, casts)
    }
}

#[derive(PartialEq, Eq)]
struct VisibilityKey {
    clip: [u8; 64],
    bounds: [u32; 6],
    mode: Visibility,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct VisibilityUpdate {
    pub changed_id_span: Option<Range<usize>>,
    pub full_tests: u64,
    pub partial_tests: u64,
}

/// One view's last successfully submitted placement, independent of pending/coalesced edits.
struct Motion {
    view: u64,
    previous: Vec<Mat4>,
    frame: u64,
    edits: u64,
}

pub(crate) struct InstanceState {
    /// Unique per collection for the process's life: with `edits` it names the placements, so a
    /// pass can key its cached work on both instead of hashing every matrix (`Self::revision`).
    id: u64,
    pub models: Vec<Mat4>,
    /// One bit per slot: the placement casts no shadow (`Instances::set_casts_shadows`).
    no_shadow: Vec<u64>,
    /// Per view, the placement it last submitted (motion vectors' previous frame).
    motion: Vec<Motion>,
    /// Counts accepted edits, so a view knows whether `models` changed since it submitted.
    edits: u64,
    pub normals: Vec<crate::normal::NormalTransform>,
    pub invalid_normals: usize,
    dirty: Vec<u64>,
    pub dirty_count: usize,
    visible: Vec<u64>,
    pub ids: Vec<u32>,
    key: Option<VisibilityKey>,
    transformed_bounds: Vec<HomogeneousWorldBounds>,
    /// The world bounds of every placement under one local box, kept until the placements or
    /// the box change (`Self::world_bounds`).
    world_bounds: std::sync::Mutex<Option<WorldBoundsCache>>,
}

/// `InstanceState::world_bounds`'s memory: the edit count and local box it was built for.
struct WorldBoundsCache {
    edits: u64,
    local: [u32; 6],
    bounds: Arc<Vec<crate::shadow_plan::Bounds>>,
}
impl InstanceState {
    pub fn validate(models: &[Mat4], max_instances: u32) -> Result<(), Error> {
        if models.is_empty()
            || models.len() > max_instances as usize
            || models.iter().any(|m| !m.is_finite())
        {
            return Err(Error::InvalidInput(
                "instances must be nonempty, finite and fit max_instances",
            ));
        }
        Ok(())
    }
    pub fn new(models: &[Mat4]) -> Self {
        let normals: Vec<_> = models
            .iter()
            .copied()
            .map(crate::normal::NormalTransform::new)
            .collect();
        let invalid_normals = normals.iter().filter(|n| !n.valid()).count();
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            models: models.to_vec(),
            no_shadow: vec![0; models.len().div_ceil(64)],
            motion: Vec::new(),
            edits: 0,
            normals,
            invalid_normals,
            dirty: vec![0; models.len().div_ceil(64)],
            dirty_count: 0,
            visible: vec![u64::MAX; models.len().div_ceil(64)],
            ids: (0..models.len() as u32).collect(),
            key: None,
            transformed_bounds: Vec::new(),
            world_bounds: std::sync::Mutex::new(None),
        }
    }
    /// The conservative world box of `local` under each placement, in slot order. Several
    /// passes ask every frame (the shadow plan, the GI cache, the camera rays); the boxes are
    /// computed when the placements or `local` changed, not per pass per frame.
    pub(crate) fn world_bounds(
        &self,
        local: Aabb,
    ) -> Result<Arc<Vec<crate::shadow_plan::Bounds>>, Error> {
        let key = [
            local.min[0].to_bits(),
            local.min[1].to_bits(),
            local.min[2].to_bits(),
            local.max[0].to_bits(),
            local.max[1].to_bits(),
            local.max[2].to_bits(),
        ];
        let mut cache = self
            .world_bounds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cache
            .as_ref()
            .filter(|c| c.edits == self.edits && c.local == key)
        {
            return Ok(Arc::clone(&cached.bounds));
        }
        let bounds = Arc::new(
            self.models
                .iter()
                .map(|&model| crate::renderer::shadow_gpu::world_bounds(local, model))
                .collect::<Result<Vec<_>, _>>()?,
        );
        *cache = Some(WorldBoundsCache {
            edits: self.edits,
            local: key,
            bounds: Arc::clone(&bounds),
        });
        Ok(bounds)
    }
    /// Whether the placement in `slot` blocks light (`Instances::set_casts_shadows`).
    pub fn casts_shadows(&self, slot: usize) -> bool {
        self.no_shadow[slot / 64] & (1 << (slot % 64)) == 0
    }
    /// Whether any placement casts no shadow (consumers skip the per-slot test otherwise).
    pub fn any_non_casting(&self) -> bool {
        self.no_shadow.iter().any(|&w| w != 0)
    }
    /// The non-casting bits, 64 slots a word: a cache key for shadow caster lists.
    pub fn non_casting(&self) -> &[u64] {
        &self.no_shadow
    }
    /// A change counts as an edit, so passes keyed on `revision` see it; transforms stay clean.
    fn set_casts_shadows(&mut self, slot: usize, casts: bool) -> Result<(), Error> {
        if slot >= self.models.len() {
            return Err(Error::InvalidInput("instance slot outside collection"));
        }
        if self.casts_shadows(slot) != casts {
            self.no_shadow[slot / 64] ^= 1 << (slot % 64);
            self.edits += 1;
        }
        Ok(())
    }
    fn set(&mut self, slot: usize, model: Mat4) -> Result<(), Error> {
        if slot >= self.models.len() || !model.is_finite() {
            return Err(Error::InvalidInput(
                "instance slot outside collection or nonfinite model",
            ));
        }
        if self.models[slot].to_cols_array().map(f32::to_bits)
            == model.to_cols_array().map(f32::to_bits)
        {
            return Ok(());
        }
        let normal = crate::normal::NormalTransform::new(model);
        self.invalid_normals -= usize::from(!self.normals[slot].valid());
        self.invalid_normals += usize::from(!normal.valid());
        self.normals[slot] = normal;
        self.models[slot] = model;
        self.edits += 1;
        let bit = 1 << (slot % 64);
        if self.dirty[slot / 64] & bit == 0 {
            self.dirty[slot / 64] |= bit;
            self.dirty_count += 1;
        }
        Ok(())
    }
    /// The placement `view` submitted in its frame `previous_frame`, or the current one when
    /// that frame did not draw these instances (no surface, so no motion).
    pub fn previous_models(&self, view: u64, previous_frame: u64) -> &[Mat4] {
        match self.motion.iter().find(|m| m.view == view) {
            Some(m) if m.frame == previous_frame => &m.previous,
            _ => &self.models,
        }
    }
    /// Whether the placements may differ from the ones `view` submitted in `previous_frame`:
    /// false when it submitted them at the current edit count (an accepted edit is the only way
    /// a model changes), so a caller comparing them one by one for motion can skip the walk.
    /// False too when that frame did not draw these instances (`previous_models` then is the
    /// current placement).
    pub fn edited_since_submit(&self, view: u64, previous_frame: u64) -> bool {
        match self.motion.iter().find(|m| m.view == view) {
            Some(m) if m.frame == previous_frame => m.edits != self.edits,
            _ => false,
        }
    }
    /// Names these exact placements: the collection and its accepted-edit count. Equal while
    /// no model changed, and different for any change or any other collection.
    pub fn revision(&self) -> (u64, u64) {
        (self.id, self.edits)
    }
    /// Called only after `view` successfully submitted `frame`; backpressure keeps history.
    pub fn motion_submitted(&mut self, view: u64, frame: u64) {
        let edits = self.edits;
        match self.motion.iter_mut().find(|m| m.view == view) {
            Some(m) => {
                if m.edits != edits {
                    m.previous.clone_from(&self.models);
                    m.edits = edits;
                }
                m.frame = frame;
            }
            None => self.motion.push(Motion {
                view,
                previous: self.models.clone(),
                frame,
                edits,
            }),
        }
    }
    pub fn dirty_runs(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        let mut slots = self
            .dirty
            .iter()
            .enumerate()
            .flat_map(|(word, &bits)| {
                let mut bits = bits;
                std::iter::from_fn(move || {
                    if bits == 0 {
                        return None;
                    }
                    let bit = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    Some(word * 64 + bit)
                })
            })
            .peekable();
        std::iter::from_fn(move || {
            let start = slots.next()?;
            let mut end = start + 1;
            while slots.peek() == Some(&end) {
                slots.next();
                end += 1;
            }
            Some(start..end)
        })
    }
    pub fn clear_dirty(&mut self) {
        if self.dirty_count > 0 {
            self.dirty.fill(0);
            self.dirty_count = 0;
        }
    }
    pub fn visibility_cache_bytes(&self) -> usize {
        self.transformed_bounds.capacity() * std::mem::size_of::<HomogeneousWorldBounds>()
    }
    pub fn visibility(&mut self, clip: Mat4, bounds: Aabb, mode: Visibility) -> VisibilityUpdate {
        let key = VisibilityKey {
            clip: if mode == Visibility::Unculled {
                [0; 64]
            } else {
                matrix_bytes(clip)
            },
            bounds: [
                bounds.min[0],
                bounds.min[1],
                bounds.min[2],
                bounds.max[0],
                bounds.max[1],
                bounds.max[2],
            ]
            .map(f32::to_bits),
            mode,
        };
        let full = self.key.as_ref() != Some(&key);
        if !full && (self.dirty_count == 0 || mode == Visibility::Unculled) {
            return VisibilityUpdate::default();
        }
        let rebuild_bounds = self
            .key
            .as_ref()
            .is_none_or(|old| old.bounds != key.bounds || old.mode == Visibility::Unculled);
        if mode == Visibility::Frustum {
            self.transformed_bounds
                .reserve(self.models.len() - self.transformed_bounds.len());
        }
        let volume = ClipVolume::new(&clip);
        let mut changed = false;
        let mut tests = 0;
        for word in 0..self.visible.len() {
            let mut slots = if full { u64::MAX } else { self.dirty[word] };
            while slots != 0 {
                let bit = slots.trailing_zeros() as usize;
                slots &= slots - 1;
                let slot = word * 64 + bit;
                if slot >= self.models.len() {
                    break;
                }
                let visible = mode == Visibility::Unculled || {
                    if rebuild_bounds || self.dirty[word] & (1 << bit) != 0 {
                        let transformed = HomogeneousWorldBounds::new(&self.models[slot], &bounds);
                        if slot == self.transformed_bounds.len() {
                            self.transformed_bounds.push(transformed);
                        } else {
                            self.transformed_bounds[slot] = transformed;
                        }
                    }
                    tests += 1;
                    volume.may_be_visible(&self.transformed_bounds[slot])
                };
                let mask = 1 << bit;
                changed |= (self.visible[word] & mask != 0) != visible;
                if visible {
                    self.visible[word] |= mask;
                } else {
                    self.visible[word] &= !mask;
                }
            }
        }
        self.key = Some(key);
        let span = changed.then(|| self.rebuild_ids()).flatten();
        VisibilityUpdate {
            changed_id_span: span,
            full_tests: if full { tests } else { 0 },
            partial_tests: if full { 0 } else { tests },
        }
    }
    fn rebuild_ids(&mut self) -> Option<Range<usize>> {
        let mut count = 0;
        let mut first = None;
        let mut last = 0;
        for slot in 0..self.models.len() {
            if self.visible[slot / 64] & (1 << (slot % 64)) == 0 {
                continue;
            }
            let id = slot as u32;
            if self.ids.get(count) != Some(&id) {
                first.get_or_insert(count);
                last = count + 1;
                if count < self.ids.len() {
                    self.ids[count] = id;
                } else {
                    self.ids.push(id);
                }
            }
            count += 1;
        }
        self.ids.truncate(count);
        first.map(|first| first..last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;
    const BOUNDS: Aabb = Aabb {
        min: [-0.1; 3],
        max: [0.1; 3],
    };
    #[test]
    fn world_bounds_are_computed_once_per_edit_and_per_local_box() {
        let a = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let mut s = InstanceState::new(&[a, Mat4::IDENTITY]);
        let first = s.world_bounds(BOUNDS).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first[0].min.x > 0.8 && first[1].max.x < 0.2);
        // Asked again (another pass of the frame): the same boxes, not computed again.
        assert!(Arc::ptr_eq(&first, &s.world_bounds(BOUNDS).unwrap()));
        // Another local box is another answer.
        let wide = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        assert!(s.world_bounds(wide).unwrap()[1].max.x > 0.9);
        // An accepted edit is another answer; a no-op edit is not.
        s.set(1, Mat4::IDENTITY).unwrap();
        let again = s.world_bounds(wide).unwrap();
        s.set(1, Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0)))
            .unwrap();
        let moved = s.world_bounds(wide).unwrap();
        assert!(!Arc::ptr_eq(&again, &moved));
        assert!(moved[1].min.y > 3.9);
    }
    #[test]
    fn motion_history_tracks_submissions_and_initializes_the_first_pose() {
        let mut s = InstanceState::new(&[Mat4::IDENTITY; 2]);
        let a = Mat4::from_translation(Vec3::X);
        let b = Mat4::from_rotation_y(0.7);
        // An edit before the first render has no invented initial velocity.
        s.set(0, a).unwrap();
        assert_eq!(s.previous_models(0, 0), s.models);
        s.motion_submitted(0, 1);
        s.set(0, b).unwrap();
        s.clear_dirty(); // upload is not submission
        assert_eq!(s.previous_models(0, 1), [a, Mat4::IDENTITY]);
        s.set(0, Mat4::IDENTITY).unwrap();
        assert_eq!(s.previous_models(0, 1), [a, Mat4::IDENTITY]);
        // An omitted draw has no surface in the immediately previous frame.
        assert_eq!(s.previous_models(0, 2), s.models);
        s.motion_submitted(0, 3);
        assert_eq!(s.previous_models(0, 3), [Mat4::IDENTITY; 2]);
        s.motion_submitted(0, 4);
        assert_eq!(s.previous_models(0, 4), [Mat4::IDENTITY; 2]);
    }
    #[test]
    fn edited_since_submit_follows_accepted_edits() {
        let mut s = InstanceState::new(&[Mat4::IDENTITY; 2]);
        // No submission yet, or a frame that did not draw these: nothing to compare.
        assert!(!s.edited_since_submit(0, 0));
        s.motion_submitted(0, 1);
        assert!(!s.edited_since_submit(0, 1));
        assert!(!s.edited_since_submit(0, 7));
        s.set(1, Mat4::from_translation(Vec3::Y)).unwrap();
        assert!(s.edited_since_submit(0, 1));
        // Submitting the edited placement clears it.
        s.motion_submitted(0, 2);
        assert!(!s.edited_since_submit(0, 2));
    }
    #[test]
    fn revision_changes_with_every_accepted_edit_and_differs_per_collection() {
        let mut s = InstanceState::new(&[Mat4::IDENTITY; 2]);
        let other = InstanceState::new(&[Mat4::IDENTITY; 2]);
        let first = s.revision();
        assert_ne!(first, other.revision());
        assert_eq!(first, s.revision());
        s.set(1, Mat4::from_translation(Vec3::Y)).unwrap();
        assert_ne!(first, s.revision());
        assert_eq!(s.revision().0, first.0);
    }
    #[test]
    fn each_view_keeps_its_own_previous_placement() {
        // Two views render each engine frame; the object moves between engine frames.
        let mut s = InstanceState::new(&[Mat4::IDENTITY]);
        let [a, b] = [1.0, 2.0].map(|x| Mat4::from_translation(Vec3::X * x));
        s.motion_submitted(0, 1);
        s.motion_submitted(1, 2);
        s.set(0, a).unwrap();
        // Both views see the move from their own last frame, not from each other's.
        assert_eq!(s.previous_models(0, 1), [Mat4::IDENTITY]);
        s.motion_submitted(0, 3);
        assert_eq!(s.previous_models(1, 2), [Mat4::IDENTITY]);
        s.motion_submitted(1, 4);
        s.set(0, b).unwrap();
        assert_eq!(s.previous_models(0, 3), [a]);
        s.motion_submitted(0, 5);
        assert_eq!(s.previous_models(1, 4), [a]);
        // A view that skipped a frame of these instances sees no motion.
        assert_eq!(s.previous_models(1, 3), [b]);
    }
    #[test]
    fn edits_validate_before_mutation_and_coalesce_across_words() {
        assert!(InstanceState::validate(&[], 128).is_err());
        assert!(InstanceState::validate(&[Mat4::from_cols_array(&[f32::NAN; 16])], 128).is_err());
        assert!(InstanceState::validate(&[Mat4::IDENTITY; 129], 128).is_err());
        let mut s = InstanceState::new(&[Mat4::IDENTITY; 128]);
        assert!(s.set(128, Mat4::IDENTITY).is_err());
        assert!(
            s.set(0, Mat4::from_cols_array(&[f32::INFINITY; 16]))
                .is_err()
        );
        s.set(0, Mat4::IDENTITY).unwrap();
        assert_eq!(s.dirty_count, 0);
        for slot in [1, 63, 64, 65, 127] {
            s.set(slot, Mat4::from_translation(Vec3::X)).unwrap();
        }
        s.set(1, Mat4::IDENTITY).unwrap();
        assert_eq!(s.dirty_count, 5);
        assert_eq!(s.dirty_runs().collect::<Vec<_>>(), [1..2, 63..66, 127..128]);
        s.clear_dirty();
        assert_eq!(s.dirty_runs().count(), 0);
        let mut negative_zero = Mat4::IDENTITY.to_cols_array();
        negative_zero[1] = -0.0;
        s.set(0, Mat4::from_cols_array(&negative_zero)).unwrap();
        assert_eq!(s.dirty_count, 1);
    }
    #[test]
    fn cache_partial_tests_and_id_shrink_empty_reexpansion() {
        let outside = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.5));
        let inside = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5));
        let mut s = InstanceState::new(&[inside, outside, inside]);
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum),
            VisibilityUpdate {
                changed_id_span: Some(1..2),
                full_tests: 3,
                partial_tests: 0
            }
        );
        assert_eq!(s.ids, [0, 2]);
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum),
            VisibilityUpdate::default()
        );
        s.set(2, outside).unwrap();
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum),
            VisibilityUpdate {
                partial_tests: 1,
                ..Default::default()
            }
        );
        assert_eq!(s.ids, [0]);
        s.clear_dirty();
        s.set(0, outside).unwrap();
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum),
            VisibilityUpdate {
                partial_tests: 1,
                ..Default::default()
            }
        );
        assert!(s.ids.is_empty());
        s.clear_dirty();
        s.set(1, inside).unwrap();
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum),
            VisibilityUpdate {
                changed_id_span: Some(0..1),
                full_tests: 0,
                partial_tests: 1
            }
        );
        assert_eq!(s.ids, [1]);
        s.clear_dirty();
        assert_eq!(
            s.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Unculled),
            VisibilityUpdate {
                changed_id_span: Some(0..3),
                full_tests: 0,
                partial_tests: 0
            }
        );
        assert_eq!(s.ids, [0, 1, 2]);
    }

    #[test]
    fn transformed_cache_tracks_camera_mesh_and_unculled_edits() {
        let mut state = InstanceState::new(&[Mat4::IDENTITY]);
        state.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum);
        assert_eq!(state.ids, [0]);
        let capacity = state.visibility_cache_bytes();
        let shifted = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        state.visibility(shifted, BOUNDS, Visibility::Frustum);
        assert!(state.ids.is_empty());
        assert_eq!(state.visibility_cache_bytes(), capacity);
        let larger = Aabb {
            min: [-20.0; 3],
            max: [20.0; 3],
        };
        state.visibility(shifted, larger, Visibility::Frustum);
        assert_eq!(state.ids, [0]);
        state.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Unculled);
        assert_eq!(
            state.visibility(shifted, BOUNDS, Visibility::Unculled),
            VisibilityUpdate::default()
        );
        state.set(0, shifted).unwrap();
        state.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Unculled);
        state.clear_dirty();
        state.visibility(Mat4::IDENTITY, BOUNDS, Visibility::Frustum);
        assert!(state.ids.is_empty());
    }

    #[test]
    fn changed_id_spans_publish_the_correct_prefix_across_many_edits() {
        let mut state = InstanceState::new(&[Mat4::IDENTITY; 193]);
        let mut published: Vec<u32> = (0..193).collect();
        let mut expected_visible = [true; 193];
        let mut seed = 0x7218_ace1_u32;
        for frame in 0..256 {
            for _ in 0..7 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let slot = seed as usize % expected_visible.len();
                expected_visible[slot] = !expected_visible[slot];
                state
                    .set(
                        slot,
                        Mat4::from_translation(Vec3::new(
                            if expected_visible[slot] { 0.0 } else { 10.0 },
                            0.0,
                            0.5,
                        )),
                    )
                    .unwrap();
            }
            let mode = if frame % 17 == 0 {
                Visibility::Unculled
            } else {
                Visibility::Frustum
            };
            let update = state.visibility(Mat4::IDENTITY, BOUNDS, mode);
            if let Some(span) = update.changed_id_span {
                published[span.clone()].copy_from_slice(&state.ids[span]);
            }
            let expected: Vec<_> = expected_visible
                .iter()
                .enumerate()
                .filter(|(_, visible)| **visible || mode == Visibility::Unculled)
                .map(|(slot, _)| slot as u32)
                .collect();
            assert_eq!(&published[..state.ids.len()], expected, "frame={frame}");
            state.clear_dirty();
            assert_eq!(
                state.visibility(Mat4::IDENTITY, BOUNDS, mode),
                VisibilityUpdate::default()
            );
        }
    }
}
