# Entity-ID output and view modes (spec)

Root, 2026-11-11. API shape for the Ara bridge to write against now (2026-11-11 ~17:20 UTC); the unit builds it. It hangs off a view ([design-views.md](design-views.md)). Row 1 of [ara-render-replacement.md](ara-render-replacement.md).

## What it is for
- **Picking in the editor:** which object is under a pixel, and later a rectangle's objects.
- **Legends in agents' captures:** a capture carries, per pixel, which object it shows, so an agent can say "the lamp at (412, 300)".
- **Debug view modes** that Ara exposes (base colour, normals, roughness, metalness, lighting only, and others below), and the rendering debug switches Ara reports and Renderlab ignores today.

## What exists to build on
- Both paths already write a per-pixel hit record before shading: placement + 1 (0 is a miss), primitive, barycentrics (`primary_hit`). The non-RT visibility buffer and the RT traced primaries produce the same record.
- The engine's batch table knows, per placement, which `ObjectId` it is (`State::batches`: each batch's `objects` in instance-slot order).
- `Renderer` already has a `ViewMode` (`BaseColor`, `Normal`, `Metallic`, `Roughness`, `Lit`), used by the raster path.

## API (engine level; hangs off a view, see design-views.md)

```rust
pub struct Outputs { pub ids: bool }                 // per view, in ViewDesc; off by default
pub enum ViewMode { Lit, BaseColor, Normal, Roughness, Metallic, Emission, Occlusion, LightingOnly }
impl Engine {
    /// The embedder's ids for an object's primitives: ranges (first primitive, count, id). The
    /// bridge merges static draws into one object, so one object can hold many entities. A single
    /// id for the whole object is the one-range case. 0 and 0xFFFF_FFFF are reserved.
    pub fn set_object_ids(&mut self, object: ObjectId, ranges: &[(u32, u32, u32)]) -> Result<(), Error>;
    /// The view's ID target of its last rendered frame: R32Uint, same size as its output.
    pub fn view_ids(&self, view: ViewId) -> Option<&wgpu::Texture>;
    /// Asks for the ids of a rectangle of the view's last frame without stalling: the copy is
    /// queued after that frame and its result arrives in `poll()` as `Event::Ids`.
    pub fn request_ids(&mut self, view: ViewId, rect: IdRect) -> Result<IdRequest, Error>;
}
pub enum Event { /* existing */ Ids { request: IdRequest, rect: IdRect, ids: Vec<IdPixel> } }
pub struct IdPixel { pub opaque: u32, pub front: u32 }
```

- **Shared and typed calls:** `set_object_ids`, `request_ids` and the `Ids` event are on the shared `Scene` trait (both GPU layers); `view_ids` returns a layer-typed handle and lives on each layer's engine only (see design-views.md).
- **Two ids per pixel.** `opaque` is the nearest opaque surface. `front` is the frontmost surface of any kind that is not fully clear: glass, a cut-out leaf, a blended decal's host surface; particles are excluded (they are effects, not objects, in the first version). A click on glass therefore has both the glass and the thing behind it; the editor chooses (Unity's rule is "front", with a modifier for "through").
- **Ids do not shimmer under temporal jitter.** They come from the history's settled surface where temporal reconstruction has one (the surface covering most of the pixel), and from this frame's hit only where the history restarted. A still camera's ID image is identical across 16 jittered frames; at a moving edge a pixel names the surface covering more than half of it.
- **Both paths:** the non-RT visibility buffer and the RT traced primaries write the same hit record (placement + 1, primitive); the ID pass maps placement and instance slot to the embedder's id through a per-placement table rewritten when ids or batches change.
- **Cost:** nothing when `ids` is off; one compute pass when on (target under 0.3 ms at 1080p).

## View modes
- Each mode is a shading-stage switch on the existing paths, not a separate renderer: the program's `surface` outputs are written instead of lit colour (base colour, normal, roughness, metalness, emission, occlusion), or lighting is computed on white albedo (lighting only).
- Temporal reconstruction and post run unchanged on lit modes and are bypassed on the data modes (an exact value per pixel, no tonemap), so a debug image reads true values.
- Both paths, judged per mode against a CPU reference of what it claims to show on a fixture.

## Questions for the consumers (the answers fix the API)
1. Does picking need transparent surfaces and particles (glass, foliage cards with alpha), and if so the nearest of any kind or a list along the ray?
2. Rectangle selection: IDs read back for a region, or a GPU-side list of distinct IDs in a rectangle?
3. Is a 32-bit tag enough (entity index plus generation), or does Ara need 64?
4. Which view modes does the editor expose today, by name, and which rendering debug switches does the bridge report as ignored?
5. Do captures for agents want the legend as an ID image beside the PNG, or as a list of (tag, name, pixel count, bounding box)?
6. Should hovering show an outline drawn by Renderlab (from the ID image), or does the editor draw it?
7. The path-traced reference is in the same row: is a "reference" view mode (Renderlab's reference tracer, progressive) wanted in the editor, or only in agents' captures?

## Judged on
- Every pixel's tag matches a CPU ray cast through the same camera, on the game scene and Showcase, both paths.
- Zero GPU cost when off; under 0.3 ms at 1080p when on.
- Each view mode matches its CPU reference on a fixture; lit mode byte-identical to today with IDs off and on.
- For the ara-gpu backend: one new storage texture (R32Uint) and one buffer (tags per placement); no change to shared planners.

## Review folded in (cloud reviewer on PR #38, 2026-11-11; the only review round)

**Ids per primitive range, not per object.** The bridge merges static draws into batched objects, so an object id is not an entity id. `set_object_ids(object, &[(first_primitive, count, id)])`: the hit record already carries the primitive, and the ID pass looks the primitive up in the object's range table (sorted ranges, a binary search per pixel). A single id is the one-range case. The Scene view keeps merging and still picks entities.

**Reserved values.** `0` is nothing (sky, background). `0xFFFF_FFFF` is a hit on a surface with no id (terrain, foliage or an untagged object), so the editor can tell empty sky from untagged geometry.

**`front` on glass.** The transparency pass writes ids: a pane writes its id where its coverage (opacity after the material's alpha) is at least 0.1, and the nearest such pane wins. Partly transparent maps follow the same per-pixel threshold. Cut-out leaves are geometry in the bridge, so `opaque` already ignores their holes. Particles write no ids.

**Jitter, resolution and cuts.**
- Ids come from temporal reconstruction's anchors (the history's surface and its second slot with coverage): the surface covering more than half the pixel.
- With `render_scale` below one, an output pixel takes its nearest anchor.
- Right after a cut or a history restart, the id image is this frame's single jittered sample, so thin geometry can flicker in it until history builds (a few frames).
- Data view modes bypass temporal reconstruction, so their ids are the unjittered hit.

**Placements are positional.** Spawning or despawning renumbers placements; the history restarts on every batch-membership change (U-IDS verifies the existing restart fires on each). Ids set after a frame answer from the next frame: a request for an already rendered frame returns that frame's ids.

**Readback (what Ara's exposure meter hit on 2026-11-11):**
- 4 staging slots per engine. A request when all are busy is refused at once with an error; the caller retries next tick.
- Largest rectangle 512 x 512 pixels (2 MiB with two ids per pixel); a larger request is an error. Rectangle selection reads back a region and reduces on the CPU.
- A request names the view's last rendered frame at the time of the call; the `Ids` event carries the view and the frame it describes. A request for a view that has not rendered is an error; a view resized, cut or removed before the copy completes still gets the frame it asked for, or an `IdsLost` event if its resources were retired.
- A copy not mapped within 8 ticks is reported once as `IdsLost`; its slot is freed when the map finally arrives.
- At most 64 undelivered id events are kept; beyond that the oldest is dropped and a counter in `FrameReport` says how many.
- `capture_view` blocks and is not for picking.

**Skinned objects in the per-pixel test:** a CPU ray cast cannot see a GPU-skinned pose, so the test uses the skinned vertices read back for the fixture, or leaves skinned pixels out and says so.

**View modes, defined or dropped.** `Overdraw` is dropped: the non-RT path draws a visibility buffer, so it means nothing there. Every program provides base colour, normal, roughness, metalness and emission through its `surface` outputs, so those modes work for any program. `Occlusion` shows the program's occlusion output, 1 (white) where a program provides none; the standard program provides none today. `LightingOnly` lights white albedo with the program's normal.

**Cost** is a target until measured; ids are on permanently in the editor, so the first measurement states it (target under 0.3 ms at 1080p; the pass runs after the temporal resolve).

**Consumers' answers (from the review, Ara side):** glass and cards as `front` plus `opaque`, no list along the ray; 32 bits are enough (the bridge packs index and generation, and 0 is never a valid entity); rectangles read back and reduce on the CPU; agents get a list (id, name, pixel count, bounding box) beside the PNG; the editor draws the outline from the id image.

## As built (U-IDS, branch `t3/ids`)

Code: src/object_ids.rs (device-free: id ranges, tables, request and event types, shared by both GPU layers), src/id_output.rs and shaders/id_output.wgsl, shaders/id_glass.wgsl (the wgpu ID pass), src/view_modes.rs, shaders/program_view.wgsl and `program::view_variant` (view modes). Tests: src/id_output_tests.rs.

**API as built, and where it differs from the sketch above.**
- Per view (on main's views, #39): a view writes ids when its `ViewDesc::outputs.ids` is on and shows its `ViewDesc::view_mode`; `create_view`/`set_view` carry both. All of a view's id and view-mode state is one field of the renderer's `ViewState`, `outputs: id_output::ViewOutputs`; the scene's object ids and the frame's id tables are the engine's, shared by every view. `request_ids(view, rect)`, `view_ids(view)` and `render_view_ids_to(view, target, layer)` select the named view and read its last frame; `poll` collects every view's readbacks. (Before #39, one view stood in: `Engine::view()`, `set_outputs`, `set_view_mode`, now removed.)
- `Scene` (shared trait) carries `set_object_ids`, `request_ids` and `poll`; `view_ids` and `render_view_ids_to` are on the wgpu `Engine` only.
- `view_ids` is one R32Uint texture with two array layers: layer 0 `opaque`, layer 1 `front`.
- The editor's packed id image (on `Engine` only): `render_view_ids_to(view, target, IdLayer::Opaque | IdLayer::Front)` writes the view's last frame into a caller's single-sample `Rgba8Unorm` (not sRGB) texture of the view's size with `RENDER_ATTACHMENT | COPY_SRC` usage (the bridge's handoff copies from it; any other texture is an error), one texel per output pixel, each 32-bit id packed bit-exact little-endian (R = bits 0-7, G = 8-15, B = 16-23, A = 24-31) by a fullscreen pass (each byte k written as (k + 0.25)/255, which a rounding or a truncating unorm8 conversion both store as k); read it by exact texel fetch. It renders no new frame: call it after each `render`/`render_to` the overlays follow; it is queued behind that frame and nothing waits. No depth output: the editor's overlays need none (root, 2026-11-11); a second per-view output would sit beside the id target in `IdOutput` without changing the id path.
- The `Ids` event is `Events::ids: Vec<IdReadback { request, view, frame, rect, ids }>`; `IdsLost { request, view }` is `Events::ids_lost`. The count of dropped id events is `Events::ids_dropped` (cumulative), not a `FrameReport` field: undelivered events wait in the engine's `Events` between polls, which is where the cap applies. A tick is one `Engine::tick` (a poll-based tick reported fast pollers' requests lost before the copy could land); an embedder that never ticks never sees `IdsLost`.
- Constants: `ID_NONE` (0), `ID_UNTAGGED` (0xFFFF_FFFF), `MAX_ID_RECT` (512), `ID_SLOTS` (4), `ID_TICKS` (8), `KEPT_ID_EVENTS` (64). A busy slot, a rectangle over 512 a side or outside the view, and a view without a frame with ids are errors at the call.

**How a pixel's ids are chosen.**
- The ID pass runs after temporal reconstruction and the transparent pass, only on hit-record frames with ids on. With ids off nothing is created, bound or encoded.
- Settled placement: temporal reconstruction's two slots, the one covering at least half the pixel. The previous frame's choice is kept while it is one of the two and either nothing there moved (camera bit-identical, both placements' transforms unchanged and not skinned) or it still covers at least 0.375 (hysteresis 0.125): a still image never changes, and an edge crossing a pixel flips it once.
- The slots name placements, not primitives. The primitive is this frame's at the nearest internal sample when it shows the settled placement, else a 3x3 neighbour's that does, else the previous frame's. A still camera keeps the previous primitive of an unchanged placement, so entities merged into one object do not shimmer at their shared edges either.
- `render_scale` below one: the anchors are at the output size; the hit record is read at the nearest internal sample.
- Placement renumbering: verified that the existing restart fires. Every spawn or despawn rebuilds the batch's instances, which changes the traced `DrawKey` (instance count and buffers), bumps `geometry_generation`, and that is part of reconstruction's history key. The ID pass's own previous frame is read only when reconstruction continued (not on its reset), so a kept primitive never belongs to a renumbered placement. Tested: ids right after a despawn and a spawn in a shared batch match the CPU cast on the next frame.
- `front`: a separate raster of the visible transparent draws after the transparent pass, with the unjittered camera, its own reversed-Z depth, and the transparent pass's opaque view depth as the far limit. A pane writes where its coverage (opacity times material, texture and vertex alpha) is at least 0.1 (the test is 0.0995: opacity is stored as f16, where the default 0.1 reads 0.09998). Particles are not drawn. Raster glass knows its instance, not its primitive: a merged glass object reports its first range's id. Upscaled frames skip the transparent pass, so there `front` equals `opaque`.
- Data view modes run without reconstruction, so their ids come from the unjittered hit record.
- Right after a cut or a history restart, the id image is this frame's single jittered sample (thin geometry can flicker until history builds, a few frames).
- The measurement switch `RENDERLAB_IDS=1` turns ids on for every view, for timing through an embedder that does not set them (Ara's player for Showcase).
- A view's moved-placement test compares each transform with the one that view last submitted (`ViewState::last_frame`), so views rendered at different times do not see each other's motion. The ID and glass pipelines are per view (one small compute and one raster pipeline each).

**View modes as built.** `ViewMode { BaseColor, Normal, Metallic, Roughness, Lit, Emission, Occlusion, LightingOnly }`; `Overdraw` was dropped (review). A hit-record frame in any mode but `Lit` runs as a lit frame up to shading and shades with each program's view-mode module: the program's traced (or MP1 list) module with its final store replaced (`program::view_variant`), compiled the first time a frame needs it and keyed by program source, head, path and mode. Programs declare `fn view_data(s: Surface, input: SurfaceInput) -> ViewData` to show their own surface; without it the engine shows the material inputs, the `emission` hook and occlusion 1. The standard program declares it from its resolved surface (decals included). Data modes store misses as 0 and skip reconstruction, transparency, fog and tone mapping (a present pass copies the HDR value to the output). `LightingOnly` lights white albedo at the program's normal with the program's indirect irradiance (the open environment for programs without indirect inputs) and every light with the engine's visibility, and is reconstructed and tone-mapped. Lit frames compile, bind and encode nothing of this.

**Tests** (src/id_output_tests.rs; GPU tests run on both paths):
- Every pixel's `opaque` and `front` against a CPU ray cast through the unjittered camera, on a fixture of a ground, three cubes of one batch (one holding two entities by primitive range, one untagged), glass, a cut-out card and a particle emitter. Pixels whose 3x3 cast neighbourhood shows one id must match exactly; edge pixels must report one of their neighbourhood's ids. No skinned objects: a CPU cast cannot see a GPU-skinned pose.
- 16 jittered frames of a still camera give identical id images; a 30-frame pan (a third of a pixel a frame) matches each frame's cast, and no pixel flips more often than its centre ray's id changes (once at least allowed).
- A membership change (despawn and spawn in a shared batch) after still frames: the next frame matches the cast.
- Readback: a rectangle matches the full image; out-of-view and oversize rectangles are refused; ids off creates no target.
- Packed id image: both layers written into an Rgba8Unorm texture equal the readback bit for bit; an sRGB target is refused.
- View modes: each data mode per interior pixel against the CPU reference (the hit object's material values, the face normal) on a fixture mixing the standard program and a program that declares `view_data` (so the list path is covered), within 1.5 of the sRGB byte; a second frame is identical. `LightingOnly` has no exact reference here (it is tone-mapped, and white albedo under the sun saturates): checked as program-independent (the standard and the custom cube's sunlit top faces read alike).
- CPU: id ranges and table order, reserved ids and overlaps, rectangle bounds, naga validation of both ID shaders.

**For the ara-gpu backend.** New resources: the handoff pass (fullscreen, Rgba8Unorm, reading the 2D-array id view as a Uint texture), the ID texture (R32Uint, 2D array of 2 layers, storage write and copy source: a 2D-array storage view is new to the renderer), two settled targets (Rg32Uint storage), the glass target (R32Uint render attachment) and its Depth32Float, and buffers: per-entry ranges `[offset, count]` and ranges `[first, count, id, 0]` (storage), moved-placement bits (storage), a 32-byte frame uniform, the glass view uniform (288 bytes) and a dynamic-offset per-draw uniform (256-byte stride). View modes add one compute module per program and mode and a present pass (fullscreen triangle). No shared planner changed. `encode_program_lists` now takes the per-program pipeline from a closure (`encode_program_lists_with`); the per-frame records are unchanged. program_api.wgsl gains `struct ViewData` and the standard program a `view_data` function, so every composed module's text changes (not its entry points' code).
