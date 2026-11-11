# More than one view per `Engine` (spec)

Root, 2026-11-11. API shape published first so the Ara bridge can write against it while the unit builds it. Row 2 of [ara-render-replacement.md](ara-render-replacement.md). Nothing has users: the existing single-view calls change freely.

## API

```rust
/// A view: one camera's targets, histories and camera-keyed caches. Scene resources are shared.
pub type ViewId = Id<kind::View>;

pub struct ViewDesc {
    pub width: u32,
    pub height: u32,
    /// Internal resolution against the output (Native, Quality, Performance, ...), per view.
    pub render_scale: RenderScale,
    pub view_mode: ViewMode,        // Lit by default; see design-id-output.md
    pub outputs: Outputs,           // ids on or off; see design-id-output.md
}

impl Engine {
    pub fn create_view(&mut self, desc: &ViewDesc) -> Result<ViewId, Error>;
    pub fn set_view(&mut self, view: ViewId, desc: &ViewDesc) -> Result<(), Error>; // resize, scale, mode, outputs
    pub fn remove_view(&mut self, view: ViewId) -> Result<(), Error>;
    /// Renders `view` from `camera` into the view's own output texture.
    pub fn render_view(&mut self, view: ViewId, camera: &Camera) -> Result<FrameReport, Error>;
    /// Renders into a caller's texture (a surface texture), as render_to does today.
    pub fn render_view_to(&mut self, view: ViewId, color: &wgpu::Texture, camera: &Camera) -> Result<FrameReport, Error>;
    pub fn view_output(&self, view: ViewId) -> Option<&wgpu::Texture>;
    pub fn capture_view(&mut self, view: ViewId) -> Result<Image, Error>;
    /// A camera cut for this view only: its histories restart; other views are untouched.
    pub fn cut_view(&mut self, view: ViewId);
}
```

- `Engine::new` creates no view. The current `resize`, `render`, `render_to`, `output`, `capture` and `cut` are removed (or become thin calls on a first view; the unit's choice, nothing depends on them but this repository's examples and the bridge).
- Scene calls (`create_mesh`, `spawn`, `set_sun`, `set_settings`, ...) stay on `Engine` and apply to every view.
- A view that is not rendered costs nothing per frame. Rendering only visible views, and the Scene view at a lower `render_scale` or only on change, is the embedder's choice through these calls.

## What a view owns, and what stays shared
| Owned by a view | Shared by the engine |
|---|---|
| Output, depth, MSAA, visibility-buffer and hit-record targets, transparency targets | Meshes, textures, programs, materials, BVHs and acceleration structures |
| Temporal reconstruction's histories, restart zones and upscale state | The GI world radiance cache (cells); its per-pixel probe and idle key are per view |
| Post histories (exposure, motion) | The shadow map's page contents; page requests and clipmap placement are per view |
| Atmosphere temporal volumes (aerial, fog) | Environment maps, LUTs, the sky |
| Reflection film accumulation | Lights, sun, decals, emitters and particle state |
| The ID target and view mode | Pipelines and caches |

The shadow clipmap's levels follow the camera; two cameras far apart would each want their own placement. First version: each view has its own clipmap placement and page requests over one shared page store (the dense levels today; the page pool later), so a second view adds its requested pages, not a second map. If that does not fit in the time, each view gets its own map and the memory is stated.

## Judged on
- Two views of one scene from two cameras: each view's still frame byte-identical to a single-view engine rendering that camera alone, both paths.
- A moving pair (one view's camera moving, the other still, then both): no history bleeding between views; the still view stays byte-identical to rendering it alone.
- Frame time for two views against two engines with the same cameras; memory of the second view in MiB.
- For the ara-gpu backend: the per-frame records become per view; flag every shared planner whose signature changes.
