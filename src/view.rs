//! Views: one camera's targets, histories and camera-keyed caches (docs/design-views.md).
//!
//! A [`Renderer`] holds one selected view in `Renderer::view`, which every frame renders
//! and every pass reads, and parks the others. Selecting a view exchanges it with the
//! selected one, so a frame's code never looks up which view it serves. Everything else in
//! the renderer is shared by its views: meshes, textures, programs, materials, the
//! acceleration structures, the GI world cache, lights, the environment and pipelines.
//!
//! Bind groups the renderer shares key on generation counters of a view's resources. Each
//! view counts in its own range (`ViewId::generations`), so selecting another view rebinds
//! them as a resize would, and view 0 counts exactly as a single-view renderer did.
use super::{
    Depth, PostGpu, ReconstructionState, Renderer, ShadowState, atmosphere_gpu::AtmosphereView,
    fresh_shadow_gpu::FreshState, light_cluster_gpu::LightClusters, lighting_gpu::Hdr,
    primary_ray_gpu::PrimaryState, temporal_reconstruction::Upscale, transparency_gpu,
    virtual_shadow_gpu::VirtualState, visibility_buffer::VisibilityBuffer,
};
use crate::Error;

/// One view of a renderer or engine: a handle that is never reused. A renderer starts with
/// view 0 selected; an engine creates its views with `Engine::create_view`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(pub(crate) u64);
impl ViewId {
    /// The first generation of this view's counters: views never share a value.
    pub(super) fn generations(self) -> u64 {
        self.0 << 40
    }
}

/// Everything one view owns. Created empty: each part is allocated by the view's first
/// frame that needs it, sized to that view's output.
pub(super) struct ViewState {
    pub depth: Option<Depth>,
    pub msaa: Option<Depth>,
    /// The lit frame's exposed HDR target and its presentation group.
    pub hdr: Option<Hdr>,
    /// Bumped when `hdr` is replaced: bindings over the HDR target key on it.
    pub target_generation: u64,
    /// Post's targets and histories (exposure, motion).
    pub post: Option<PostGpu>,
    // ---- The view's look (`engine::ViewLook`): set while the view is selected ----
    /// PP1 presentation and its colour LUT.
    pub post_settings: crate::PostSettings,
    pub post_lut: Option<crate::ColourLut>,
    /// The exposure scale (2^stops) this view's frames light and present at; synced into the
    /// shared lighting uniform when it renders.
    pub exposure: f32,
    /// TR1 settings (its render scale among them); M7-R1/R1n reflection controls.
    pub reconstruction: crate::Reconstruction,
    pub reflections: crate::ReflectionSettings,
    /// The sun visibility estimator, and Virtual's tuning (the quality preset).
    pub soft_mode: crate::SoftShadows,
    pub virtual_settings: crate::VirtualShadowSettings,
    /// The sun shadow plan, its cascade bases and records (the raster maps' placement).
    pub shadows: ShadowState,
    pub fresh: Option<FreshState>,
    /// Traced or visibility-buffer primaries: hit records, shading targets, film history.
    /// Exists on renderers that can trace primary visibility.
    pub primary: Option<PrimaryState>,
    pub visibility_buffer: Option<VisibilityBuffer>,
    /// `SoftShadows::Virtual` state, created by the view's first frame (N8): its clipmap
    /// placement, page requests and pages.
    pub virtual_shadows: Option<VirtualState>,
    /// LC1: the light cluster grid (screen space), the local-shadow selection with its
    /// transitions, and the atlas holding the selected lights' maps.
    pub clusters: LightClusters,
    /// The GI cache's view keys: whether this view's camera and scene are unchanged.
    pub gi: super::gi_cache::ViewPlan,
    /// The camera volume and fog histories (`atmosphere_gpu.rs`).
    pub atmosphere: AtmosphereView,
    /// TR1 history (created by the first reconstructed frame), the next frame's sample
    /// index, and the index of the frame being prepared while that frame reconstructs.
    pub reconstruction_state: Option<ReconstructionState>,
    pub reconstruction_index: u64,
    pub reconstruction_frame: Option<u64>,
    /// TR1b upscaling: the scaled frame's internal colour target and output HDR.
    pub upscale: Option<Upscale>,
    /// TP1's colour, depth and AA targets, and TP2's glass reflection targets.
    pub transparency_targets: Option<transparency_gpu::Targets>,
    pub transparency_reflections: Option<transparency_gpu::reflection::GlassReflections>,
    /// The renderer frame this view last submitted (`FrameId`): object motion vectors
    /// reproject from that frame's placements (`InstanceState::previous_models`).
    pub last_frame: u64,
    /// What this view's primary preparation uploaded (its geometry and instance tables).
    pub primary_uploads: crate::UploadBytes,
}
impl ViewState {
    pub fn new(
        key: ViewId,
        shadows: ShadowState,
        clusters: LightClusters,
        atmosphere: AtmosphereView,
        fresh: Option<FreshState>,
        primary: Option<PrimaryState>,
    ) -> Self {
        Self {
            depth: None,
            msaa: None,
            hdr: None,
            target_generation: key.generations(),
            post: None,
            post_settings: Default::default(),
            post_lut: None,
            exposure: 1.0,
            reconstruction: Default::default(),
            reflections: Default::default(),
            soft_mode: crate::SoftShadows::Exact,
            virtual_settings: Default::default(),
            shadows,
            clusters,
            gi: Default::default(),
            fresh,
            primary,
            visibility_buffer: None,
            virtual_shadows: None,
            atmosphere,
            reconstruction_state: None,
            reconstruction_index: 0,
            reconstruction_frame: None,
            upscale: None,
            transparency_targets: None,
            transparency_reflections: None,
            last_frame: 0,
            primary_uploads: Default::default(),
        }
    }
}

impl Renderer {
    /// Adds an empty view sharing this renderer's scene. It allocates nothing until it
    /// renders; its shadow, primary and fresh state start from the selected view's settings
    /// and layouts.
    pub fn create_view(&mut self) -> ViewId {
        let key = ViewId(self.next_view);
        self.next_view += 1;
        let mut view = ViewState::new(
            key,
            self.view
                .shadows
                .fork(&self.device, &self.ledger, key.generations()),
            self.view
                .clusters
                .fork(&self.device, &self.ledger, key.generations()),
            AtmosphereView::new(&self.device, &self.ledger, key.generations()),
            self.view.fresh.as_ref().map(FreshState::fork),
            self.view.primary.as_ref().map(PrimaryState::fork),
        );
        // The selected view's look, until the caller sets this view's.
        let v = &self.view;
        view.post_settings = v.post_settings;
        view.post_lut.clone_from(&v.post_lut);
        view.exposure = v.exposure;
        view.reconstruction = v.reconstruction;
        view.reflections = v.reflections;
        view.soft_mode = v.soft_mode;
        view.virtual_settings = v.virtual_settings;
        self.parked.push((key, view));
        key
    }
    /// Drops a view's targets and histories once its submitted frames retire (wgpu keeps
    /// referenced resources alive). Removing the selected view selects a new empty one.
    pub fn remove_view(&mut self, key: ViewId) -> Result<(), Error> {
        if key == self.view_key {
            let spare = self.create_view();
            self.select_view(spare)?;
        }
        let at = self
            .parked
            .iter()
            .position(|(k, _)| *k == key)
            .ok_or(Error::InvalidInput("unknown view"))?;
        self.parked.swap_remove(at);
        Ok(())
    }
    /// Makes `key` the view that frames render and view calls (cuts, resets) change.
    pub fn select_view(&mut self, key: ViewId) -> Result<(), Error> {
        if key == self.view_key {
            return Ok(());
        }
        let at = self
            .parked
            .iter()
            .position(|(k, _)| *k == key)
            .ok_or(Error::InvalidInput("unknown view"))?;
        let (parked_key, parked) = &mut self.parked[at];
        std::mem::swap(&mut self.view, parked);
        std::mem::swap(&mut self.view_key, parked_key);
        Ok(())
    }
    pub fn selected_view(&self) -> ViewId {
        self.view_key
    }
    /// Runs `f` once with each view selected, the selected view first, and leaves the
    /// selection as it was: for settings a view keeps its own copy of (shadows, lighting).
    /// Stops at the first error; views before it keep the change.
    pub fn each_view(
        &mut self,
        mut f: impl FnMut(&mut Self) -> Result<(), Error>,
    ) -> Result<(), Error> {
        f(self)?;
        for at in 0..self.parked.len() {
            let (parked_key, parked) = &mut self.parked[at];
            std::mem::swap(&mut self.view, parked);
            std::mem::swap(&mut self.view_key, parked_key);
            let result = f(self);
            let (parked_key, parked) = &mut self.parked[at];
            std::mem::swap(&mut self.view, parked);
            std::mem::swap(&mut self.view_key, parked_key);
            result?;
        }
        Ok(())
    }
    /// Advances the engine clock one frame: the next frame any view renders runs the shared
    /// per-frame work (skinning, the particle step), and later frames in the same tick do
    /// not. Until the first call every frame is its own tick, as for a single view.
    pub fn begin_tick(&mut self) {
        self.explicit_ticks = true;
        self.tick += 1;
    }
    /// Whether an admitted frame runs the current tick's shared work, claiming it if so.
    pub(super) fn take_tick(&mut self) -> bool {
        if !self.explicit_ticks {
            return true;
        }
        let first = self.worked_tick != self.tick;
        self.worked_tick = self.tick;
        first
    }
    /// The selected view, then the parked ones.
    pub(super) fn views_mut(&mut self) -> impl Iterator<Item = &mut ViewState> {
        std::iter::once(&mut self.view).chain(self.parked.iter_mut().map(|(_, v)| v))
    }
    /// Retires every parked view's replaced resources, as `collect` does the selected one's.
    pub(super) fn collect_parked(&mut self, completed: u64) {
        for (_, view) in &mut self.parked {
            view.collect(completed);
        }
    }
}
impl ViewState {
    pub(super) fn collect(&mut self, completed: u64) {
        self.shadows.collect(completed);
        self.clusters.collect(completed);
        if let Some(fresh) = &mut self.fresh {
            fresh.collect(completed);
        }
        if let Some(state) = &mut self.virtual_shadows {
            state.collect(completed);
        }
        if let Some(primary) = &mut self.primary {
            primary.collect(completed);
        }
    }
}
