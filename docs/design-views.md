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
    /// How this view looks: exposure, tone mapper, colour table and its strength, bloom, depth of
    /// field, quality and reflection settings. Everything that lights the world stays engine-wide.
    pub look: ViewLook,
}

impl Engine {
    /// Advances the engine one frame: the shared per-frame work runs here, once, never per view
    /// (skinning, the particle step, joint palette upload, the TLAS refit, the GI cache's cell work
    /// and its clock). `render_view` renders a view against the current tick; rendering a view
    /// twice in one tick renders twice without stepping anything.
    pub fn tick(&mut self, seconds: f32) -> Result<(), Error>;
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

## Which calls are shared by both GPU layers (decided 2026-11-11, with the ara-gpu port's author)
The Mac gets Renderlab only through the ara-gpu port, so the bridge must be able to drive views and ids without wgpu types.
- **On the shared `Scene` trait** (wgpu `Engine` and the port both implement it; no GPU-layer types): `create_view`, `set_view`, `remove_view`, `render_view` (into the view's own output), `cut_view`, `capture_view` (CPU `Image`), and for ids `set_object_id`, `request_ids` and the `poll` event that carries them.
- **On each layer's engine only** (typed by that layer): `view_output` and `view_ids` (a `wgpu::Texture` on the wgpu engine; the port's own image or export-ring handle on the port), and `render_view_to` (renders into a caller's `wgpu::Texture`). The bridge's image handoff already differs per layer, so it calls these on the concrete engine, not through the trait.

## The GI planner: per engine and per view
`gi_cache::Planner` holds both scene state and camera state today, and both layers hold it (the port bundles it with the table in its `gi_cache::Cache`). It splits in two:
- **`gi_cache::Planner` (per engine, with the table and cells):** placement bounds and poses; the environment digest and its streak, catch-up and phase; the light digest, light streak, relight catch-up and phase; the bounce digest and its busy count; the frame stamp cells are marked seen with; the pending clear and frames since it; the update parity. Cell updates, relighting and clearing run once per engine frame, whichever view triggers them.
- **`gi_cache::ViewPlan` (per view, owned by the view):** the idle key and idle frames, the view key and view frames, and the claimed pixel region (`scene_pixels`). Probing, claims and the gather run per view.
- **Planning a frame:** `planner.begin(&mut view_plan, &inputs)` and `finish` as today, with the view's state passed in. A view is idle when its own keys and the engine's scene and lighting keys are unchanged; the engine's per-frame cell work runs if any view rendered this frame is not idle.
- Both layers hold one `Planner` per engine and one `ViewPlan` per view, the same way.

## What a view owns, and what stays shared
| Owned by a view | Shared by the engine |
|---|---|
| Output, depth, MSAA, visibility-buffer and hit-record targets, transparency targets | Meshes, textures, programs, materials, BVHs and acceleration structures |
| Temporal reconstruction's histories, restart zones and upscale state | The GI world radiance cache (table, cells and `gi_cache::Planner`); `gi_cache::ViewPlan`, the per-pixel probe and the claimed region are per view |
| Post histories (exposure, motion) | The shadow map's page contents; page requests and clipmap placement are per view |
| Atmosphere temporal volumes (aerial, fog) | Environment maps, LUTs, the sky |
| Reflection film accumulation | Lights, sun, decals, emitters and particle state |
| The ID target and view mode | Pipelines and caches |

The shadow clipmap's levels follow the camera; two cameras far apart would each want their own placement. First version: each view has its own clipmap placement and page requests over one shared page store (the dense levels today; the page pool later), so a second view adds its requested pages, not a second map. If that does not fit in the time, each view gets its own map and the memory is stated.

## Judged on
- Two views of one scene from two cameras: each view's still frame byte-identical to a single-view engine rendering that camera alone once the shared GI cache is quiescent, both paths.
- A moving pair (one view's camera moving, the other still, then both): no history bleeding between views; the still view stays byte-identical to rendering it alone.
- Frame time for two views against two engines with the same cameras; memory of the second view in MiB.
- For the ara-gpu backend: the per-frame records become per view; flag every shared planner whose signature changes.

## Review folded in (cloud reviewer on PR #38, 2026-11-11; the only review round)

**Settings: what moves into a view.** `ViewLook` carries exposure (manual or automatic, metered per view), the tone mapper, the colour table and its strength, bloom, depth of field, quality and reflection settings. Engine-wide `Settings` keeps what lights the world: sun, sky and sky light, local lights, GI on or off, atmosphere and fog media. Otherwise the Game view takes the Scene view's exposure and look.

**One clock: `Engine::tick`.** Shared per-frame work runs once per tick. The GI cache's frame stamp, update parity and relight and environment phases advance per tick, never per `render_view`. Each view keeps its own **sample counter** (jitter, sun direction rotation, film accumulation index), so a view's noise sequence does not depend on how often another view renders, and its frames match a single-view engine.

**GI eviction with a view that renders rarely.** Eviction counts engine ticks. A view that skips ticks keeps its cells alive by a touch: each view records the cell slots its last render claimed (bounded by its probe tiles), and on every tick in which the view does not render, one small pass re-stamps those slots as seen. A redraw after a long pause therefore does not refill like a cut. The gather cap is per tick and shared; a cut in one view may slow the other's refill for a few ticks, which is acceptable. Two views at different distances key the same surface as different cells; the table's capacity is stated with two views in the measurements.

**Previous transforms for a view that skipped ticks.** Motion reads the previous submitted placement, which is per engine. Rule: each placement carries the tick it last moved; a view that did not render the previous tick restarts history on pixels whose placement moved since that view's last render. Per-view previous transforms come later only if this looks bad.

**Identity gate.** Each view's still frame is byte-identical to a single-view engine rendering that camera alone **once the shared GI cache is quiescent** (a view that finds cells filled by the other differs during fill, for the better).

**Per view or shared, one line each:**
- `cut_view`: per view. Temporal reconstruction, the atmosphere's histories and the camera texture verification (`mark_camera_cut`) restart for that view. The GI cache is not reset.
- Texture streaming: shared, demand is the union of the views' feedback under one budget, so a texture seen only by the Scene view still streams in.
- Local-light shadow selection: shared atlas, importance is the union of the views' demands under one budget. Light clusters: per view (they are screen-space).
- Shadow pages: shared store keyed by world position (level and world page coordinates), so one view's window move never invalidates the other's pages. Invalidation by moved casters and sun turns is engine-wide and applies to every view's pages. If the pages turn out to be addressed relative to the window, U-VIEWS uses the fallback (one map per view, memory stated). The `Surface` lookup's receiver cache is keyed by receiver cell: shared.
- Atmosphere: transmittance and multiscatter LUTs shared; the sky-view LUT depends on the camera's height, so it is per view, as are the aerial and fog volumes.
- `set_view`: a resize or a render-scale change restarts that view's histories; a mode or look change restarts nothing. `remove_view` retires the view's resources after its frames in flight complete.
- `FrameReport` and events carry the view they describe.
- Memory of a second view at 1080p: measured and stated; budget 250 MiB.
