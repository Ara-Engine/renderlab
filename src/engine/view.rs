//! Views (docs/design-views.md): an editor's Scene and Game views, or a game's split screen,
//! rendered from one engine. Each view owns its output, targets and histories and has its
//! own look; the scene, its caches and what lights it are shared.
//!
//! A frame of the embedder is one [`Engine::tick`] (the shared per-frame work: skinning, the
//! particle step) and then any number of [`Engine::render_view`] calls. A view that is not
//! rendered costs nothing.
use super::{Engine, Image, settings};
use crate::{Camera, Error, FrameReport, RenderScale, ViewId, ViewMode};
use std::time::Instant;

pub use super::settings::ViewLook;

/// Which extra per-pixel outputs a view writes beside its colour (docs/design-id-output.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    /// Object ids per pixel; off by default.
    pub ids: bool,
}

/// A view's size and how it renders. `ViewDesc::new(width, height)` for the defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewDesc {
    pub width: u32,
    pub height: u32,
    /// Internal resolution against the output: `Native`, or the `Quality` (2/3 per axis)
    /// and `Performance` (1/2) upscaling presets, which TR1 reconstructs to the output
    /// size. Below `Native` needs `Settings::temporal_reconstruction`.
    pub render_scale: RenderScale,
    pub view_mode: ViewMode,
    pub outputs: Outputs,
    pub look: ViewLook,
}
impl ViewDesc {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            render_scale: RenderScale::Native,
            view_mode: ViewMode::Lit,
            outputs: Outputs::default(),
            look: ViewLook::default(),
        }
    }
}

/// What the engine keeps for one view.
pub(super) struct ViewEntry {
    pub id: ViewId,
    pub desc: ViewDesc,
    /// The view's own colour output (`render_view`, `capture_view`).
    pub output: wgpu::Texture,
    /// The renderer holds this view's look and render scale (`apply_look`).
    pub applied: bool,
}

impl Engine {
    /// Advances the engine one frame of `seconds` (the particle step, in [0, 1]; zero
    /// freezes particles). The next view rendered runs the shared per-frame work: skinning
    /// from the joint palettes set since the last tick, the particle step and the skinned
    /// geometry's refits. Further views of the same tick reuse it, and rendering a view
    /// twice in one tick renders twice without stepping anything.
    pub fn tick(&mut self, seconds: f32) -> Result<(), Error> {
        self.renderer.set_particle_step(seconds)?;
        self.renderer.begin_tick();
        Ok(())
    }

    /// Creates a view and its output, and selects it: its look is the renderer's until
    /// another view renders, so `renderer_mut` tuning after this call applies to it. It
    /// allocates its targets on its first frame.
    pub fn create_view(&mut self, desc: &ViewDesc) -> Result<ViewId, Error> {
        validate(desc)?;
        settings::validate_look(&self.state.settings, &desc.look, desc.render_scale)?;
        let output = self.create_output(desc.width, desc.height);
        // The renderer's first view serves the engine's first view.
        let id = match self.spare.take() {
            Some(id) => id,
            None => self.renderer.create_view(),
        };
        self.views.push(ViewEntry {
            id,
            desc: desc.clone(),
            output,
            applied: false,
        });
        self.renderer.select_view(id)?;
        self.apply_look(self.views.len() - 1)?;
        Ok(id)
    }
    /// Changes a view. A new size or render scale restarts its histories (as a cut does);
    /// a new mode, look or outputs restarts nothing.
    pub fn set_view(&mut self, view: ViewId, desc: &ViewDesc) -> Result<(), Error> {
        validate(desc)?;
        settings::validate_look(&self.state.settings, &desc.look, desc.render_scale)?;
        let at = self.view_index(view)?;
        let old = &self.views[at].desc;
        let resized = (old.width, old.height) != (desc.width, desc.height);
        let rescaled = old.render_scale != desc.render_scale;
        let restyled = rescaled || old.look != desc.look;
        if resized {
            self.views[at].output = self.create_output(desc.width, desc.height);
        }
        self.views[at].desc = desc.clone();
        self.views[at].applied &= !restyled;
        if resized || rescaled {
            self.cut_view(view);
        }
        Ok(())
    }
    /// Removes a view. Its targets and histories are released once its frames in flight
    /// complete.
    pub fn remove_view(&mut self, view: ViewId) -> Result<(), Error> {
        let at = self.view_index(view)?;
        self.views.swap_remove(at);
        let was_selected = self.renderer.selected_view() == view;
        self.renderer.remove_view(view)?;
        if was_selected {
            // The renderer replaced it with an empty view; the next created view takes it.
            self.spare = Some(self.renderer.selected_view());
        }
        Ok(())
    }
    /// Renders `view` from `camera` into the view's own output.
    pub fn render_view(&mut self, view: ViewId, camera: &Camera) -> Result<FrameReport, Error> {
        let output = self.views[self.view_index(view)?].output.clone();
        self.render_view_to(view, &output, camera)
    }
    /// Renders `view` from `camera` into `color`: a single-sample 2D render attachment in
    /// the engine's output format, such as a surface texture (then call `present`). The
    /// view's targets follow `color`'s size and the camera's aspect follows it.
    pub fn render_view_to(
        &mut self,
        view: ViewId,
        color: &wgpu::Texture,
        camera: &Camera,
    ) -> Result<FrameReport, Error> {
        let at = self.view_index(view)?;
        self.renderer.select_view(view)?;
        self.apply_look(at)?;
        let mode = self.views[at].desc.view_mode;
        self.prepare_view_ids(self.views[at].desc.outputs.ids)?;
        self.render_selected(color, camera, mode)
    }
    /// The view's own output, as its last `render_view` left it.
    pub fn view_output(&self, view: ViewId) -> Option<&wgpu::Texture> {
        let at = self.view_index(view).ok()?;
        Some(&self.views[at].output)
    }
    /// Reads back the view's output after its last frame (blocking). Needs an RGBA8 output
    /// format. Not for picking: use the view's ids (docs/design-id-output.md).
    pub fn capture_view(&mut self, view: ViewId) -> Result<Image, Error> {
        let output = self.views[self.view_index(view)?].output.clone();
        let mut capture = self.admit("capture admission timed out", |r| r.try_capture(&output))?;
        let deadline = Instant::now() + super::ADMISSION;
        loop {
            super::stash(&mut self.events, self.renderer.poll()?);
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
    /// A camera cut or teleport for this view only: its temporal reconstruction, fog and
    /// post motion histories restart and streamed textures verify its new view. Other views
    /// and the GI cache are untouched (the cache refills newly seen cells on its own).
    pub fn cut_view(&mut self, view: ViewId) {
        if self.view_index(view).is_err() || self.renderer.select_view(view).is_err() {
            return;
        }
        self.renderer.cut_reconstruction();
        self.renderer.reset_atmosphere_history();
        self.renderer.mark_camera_cut();
    }
    /// The engine's views in creation order (removal reorders).
    pub fn views(&self) -> impl Iterator<Item = ViewId> + '_ {
        self.views.iter().map(|v| v.id)
    }
    pub fn view_desc(&self, view: ViewId) -> Option<&ViewDesc> {
        Some(&self.views[self.view_index(view).ok()?].desc)
    }

    pub(super) fn view_index(&self, view: ViewId) -> Result<usize, Error> {
        self.views
            .iter()
            .position(|v| v.id == view)
            .ok_or(Error::InvalidInput("unknown or removed view"))
    }
    /// Gives view `at` (selected) its look and render scale, unless it holds them already.
    /// The renderer keeps each view's look with the view, so views with different looks
    /// restart nothing when they alternate.
    fn apply_look(&mut self, at: usize) -> Result<(), Error> {
        if self.views[at].applied {
            return Ok(());
        }
        let desc = &self.views[at].desc;
        settings::apply_look(
            &mut self.renderer,
            self.path,
            &self.state.settings,
            &settings::ablated_look(desc.look.clone()),
            desc.render_scale,
        )?;
        // The exposure travels with the lighting.
        self.apply_lighting()?;
        self.views[at].applied = true;
        Ok(())
    }
    /// The selected view's exposure (the default for the renderer's spare view).
    pub(super) fn selected_exposure(&self) -> super::Exposure {
        let selected = self.renderer.selected_view();
        self.views
            .iter()
            .find(|v| v.id == selected)
            .map_or(super::Exposure::default(), |v| v.desc.look.exposure)
    }
    fn create_output(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("renderlab view output"),
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
        })
    }
}

fn validate(desc: &ViewDesc) -> Result<(), Error> {
    if desc.width == 0 || desc.height == 0 {
        return Err(Error::InvalidInput("view extent must be nonzero"));
    }
    Ok(())
}
