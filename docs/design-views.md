# More than one view per `Engine` (spec)

Root, 2026-11-11. API shape published first so the Ara bridge can write against it while the unit builds it. Row 2 of [ara-render-replacement.md](ara-render-replacement.md). Nothing has users: the existing single-view calls change freely.

## API

```rust
/// A view: one camera's targets, histories and camera-keyed caches. Scene resources are shared.
/// As built: `renderlab::ViewId`, a handle never reused, shared by `Renderer` and `Engine`
/// (re-exported by `renderlab::engine`), so frame reports and events name the view directly.
pub struct ViewId(/* u64 */);

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

## As built (U-VIEWS, `t3/views`)

Where the code and the sections above differ, this section is current. Code: `src/view.rs` (renderer views), `src/engine/view.rs` (engine views), `game_scene --views a|b|ab [--b-orbit]` (the identity and motion checks).

**How the split works.** `Renderer` holds the selected view in its field `view: ViewState` and parks the others. `select_view` swaps them, so the code that encodes a frame reads `self.view.*` and never looks up which view it serves. `create_view` forks layouts from the selected view, and a new view allocates nothing until its first frame. Bind groups the renderer shares (the lighting group) key on generation counters of a view's resources. Each view counts in its own range (`id << 40`), so selecting another view rebinds them as a resize would, and view 0 counts exactly as a single-view renderer did.

**What `ViewState` owns:** depth, MSAA and HDR targets; post (targets and histories); TR1's history, sample index and upscale state; the visibility buffer; the primary state (hit records, shading targets, geometry tables, film); fresh targets; the Virtual shadow state (clipmap placement, page requests and pages: the per-view-map fallback); the sun shadow plan (cascade bases and records); LC1's screen-space cluster grid and its frame uniform; the GI `ViewPlan`; the atmosphere's camera volume and fog histories; transparency and glass-reflection targets; the frame it last submitted; its own primary-table uploads; and the look (post settings and LUT, exposure, `Reconstruction`, reflections, soft-shadow mode, Virtual tuning).

**Shared, with why:**
- The sky-view LUT: one texture, recomputed from the eye in every view's frame. Same result as one per view, no extra memory.
- The atmosphere's procedural sky, which lights the world (image-based light, the GI cache): placed once per tick at the altitude of the tick's first view. Later views keep it. Placed per view, two views at different heights rebuilt the sky on every switch, and the GI cache re-integrated and relit every tick (20 ms a tick on the 760M at 1080p). Cost: a later view at another height differs from an engine of its own by at most one level of sky light (game scene, 0.6% of pixels).
- LC1's local shadows (`LocalMaps`): one atlas (384 MiB at the defaults) and one selection, made in the tick's first lit frame from the union of the views' ranked demands (this view's, and the others' from their last frames). One view selects exactly as before.
- The atmosphere uniform, the camera uniform, the lighting uniform and the light clusters' kernel inputs: rewritten in each frame before use (queue order makes that safe).
- Sun shadow settings: canonical on the renderer, copied into a view when it renders. Exposure: the selected view's, copied into the lighting uniform when it renders.
- Scene-wide restarts (fog media, transmission changes) cut every view's TR1 history.

**Ticks.** `Engine::tick(seconds)` sets the particle step and calls `Renderer::begin_tick`. The first frame of a tick runs skinning and the particle simulation. A later view of the same tick spawns nothing and moves nothing: its particle update runs with step 0, which only relights the particles for its own sun-map fit (`tp_incident`). The GI cache's per-tick work (scene, environment and light events, scan and eviction, integrate, gathers, cell shading, the clock) runs in the tick's first frame whose plan is not idle. Later views of the tick probe, claim and trace only their new cells, into the tick's next gather queue. A renderer that never calls `begin_tick` treats every frame as a tick, as before.

**Continuity per view.** TR1's plan and post's motion history continue from the view's own last frame, not the renderer's. TR1's history key hashes the scene's uploads and the view's own primary uploads, not other views'.

**Object motion: per view (decided over the tick-restart rule above, 2026-11-11).** Each instance collection keeps, per view, the placement that view last submitted (`InstanceState::motion`, 64 B per instance per view). A view that skipped ticks reprojects rigid motion over the whole gap instead of restarting history. Skinned previous positions are still per engine, so a view that skipped ticks gets a skinned object's motion from the last tick only.

**Looks.** `ViewLook` (exposure, quality, post with its tone mapper, bloom and depth of field, colour LUT, reflections) and `render_scale` are applied when a view is created and when it renders after `set_view` or `set_settings` changed them. The renderer keeps them in the view, so views with different looks alternate without restarting each other's histories. A look change restarts nothing except a render-scale change (TR1 restarts at a new internal size). `renderer_mut` tuning applies to the selected view (the last created or rendered).

**Measured (m6u, Radeon 760M RADV, release `game_scene --still`, 60 drained ticks, reference run before and after within 0.2%):**

| | one view (a / b) | two engines (a + b) | two views in one engine (ab) | second view's memory (allocator) |
|---|---|---|---|---|
| HardwareRt 1920x1080 | 98.6 / 78.7 ms | 177.3 ms | 170.5 ms (-3.8%) | 710 MiB: 605 MiB targets, the rest buffers |
| NonRt 1280x720 | 129.8 / 94.7 ms | 224.4 ms | 222.1 ms (-1.0%) | 773 MiB: 213 MiB targets, 503 MiB shadow (Virtual page store 357, Surface cache 64, its casters), the rest buffers |

NonRt is timed at 720p because the moving and still NonRt game scene at 1080p hangs RADV's gfx ring on this GPU on main too (docs/m7-gaps.md). The 250 MiB budget for a 1080p second view is below one view's own targets on HardwareRt (633 MiB for the first view as well). A Scene view meets it only at a lower `render_scale`. On NonRt the per-view Virtual store (the per-view-map fallback) and Surface cache are the rest, until the world-keyed shared page store exists. Showcase renders only through the Ara bridge and is not measured here.

**For the ara-gpu backend: shared planners whose signatures changed.**
- `gi_cache::Planner::begin(&config, run, bounce, tick)` and `finish(&config, &begun, bounce, tick, &mut ViewPlan, inputs)`: `tick` is true for the tick's first frame whose plan runs (a renderer without explicit ticks passes true every frame). `ViewPlan` (idle and view keys) moved out of `Planner` and is owned per view. Call `advance` only when `tick` is true.
- `temporal_reconstruction::PlanInputs`: `rendered_frames` became `previous_frame` (the frame this view rendered last) and `frame` (this frame's id). History continues only when the plan's previous frame is the view's own.
- `InstanceState::previous_models(view, previous_frame)` and `motion_submitted(view, frame)` take the view.
- Per-frame records the port keeps once per renderer that are per view here: the TR1 plan, post's previous matrix, the atmosphere's reprojection, the instance motion record, the light cluster selection.

**Cuts.** `Engine::cut_view` restarts that view's TR1 history, its fog history and the camera texture verification. Other views and the GI cache are untouched.

Checked on m6u (`game_scene --views ab --cut-b 12` against `a` and `b`, 1280x720, GI off, view a flying):
- With `--sky uniform`, both views are byte-identical on all 20 frames, NonRt and HardwareRt, to single-view engines. For view b that is an engine cut at the same frame, so the frames right after the cut carry no stale history, and view a is unaffected.
- Under the atmosphere sky, view b differs by at most 1-2 levels on every frame, before and after the cut: the sky light placed at view a's altitude.

**An idle view costs nothing measurable.** A second view created and never rendered (`--views ai` against `a`, release, still, 60 ticks, reference drift 0.5% and 0.1%):
- HardwareRt 1920x1080: 99.05 against 98.42 ms a tick, 14 MiB.
- NonRt 1280x720: 83.61 against 83.79 ms a tick, 9 MiB.
Its targets are created by its first frame.

**The ara-gpu backend (`src/backend_ara`, feature `backend-ara`) renders one view.** Views are not ported there. Its `Scene` implementation maps the view calls onto its single view: `create_view` once (a second is refused), and `set_view`, `remove_view`, `render_view`, `cut_view` and `capture_view` call its `resize`, `render`, `cut` and `capture`. `tick` is its particle step. The look comes from that view's `ViewDesc`. Its GI plans with one `ViewPlan` and every frame a tick.

**Not built yet:** ids (`Outputs::ids` is rejected until U-IDS lands); the GI touch pass for views that skip ticks; texture streaming feedback is shared without a per-view union rule (each streaming frame's requests add to one residency).
