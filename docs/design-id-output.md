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

## The editor's needs (Ara bridge thread, read from the editor's code, 2026-11-11 evening): these set the order
- **The Scene view is one view of an editor-hosted engine**, on the editor's own device beside its UI compositor; the Game view is a separate player process. Multi-view is not on the critical path for it.
- **Ids on the GPU every frame, through the same handoff as colour.** The editor's selection, hover and review outlines, its grid (occluded by scene ids), gizmos and icons are its own overlay shaders reading an id texture each frame. Renderlab renders on its own device, so the id image rides the colour's handoff (a shared-image ring on Windows, a CPU copy on Linux; both RGBA8). So:
  - `render_view_ids_to(view, target: &wgpu::Texture, which: IdLayer)` writes the view's `opaque` (or `front`) ids into a caller's RGBA8-compatible texture (`Rgba8Unorm`, not sRGB), each 32-bit id packed bit-exact into the four channels (r = bits 0-7 ... a = bits 24-31), no filtering. Wgpu-typed, so on `Engine` only, beside `render_view_to`. `view_ids` as R32Uint stays for same-device users.
  - `opaque` is what the overlays need every frame; `front` is for the click.
  - **Depth for the overlays:** `render_view_depth_to(view, target)` writes linear view depth (metres, f32 bits packed the same way into RGBA8). It is cheap: the hit record already gives the surface point, so it is one more store in the ID pass.
- **Picking is one pixel:** `request_ids` with a 1x1 rectangle; click takes `front`, outlines `opaque`. The marquee and snapping are CPU-side in the editor; no rectangle readback is needed today (the bounded rectangle API stays, it costs nothing).
- **Tags:** a 32-bit compact index is enough; `set_object_ids` with primitive ranges stays because the bridge merges draws.
- **Capture legend:** the bridge computes `{entity, name, bbox, pixel count}` from a full-frame id image (through `capture_view`'s path, not interactive); Renderlab supplies only the image.
- **View modes:** for now `Lit` plus ids is enough. The editor's list, in the order its users would miss them: wireframe, depth, entity ids, lit wireframe, shadow cascades, GI debug, light complexity, shadow penumbra, vertex colour channels, decal coverage, and twelve debug effect switches. The data modes above (base colour, normal, roughness, metalness, emission, occlusion, lighting only) come after ids.

**U-IDS's order, from this:** (1) object ids correct on both paths with primitive ranges; (2) the id output (and depth) into a caller's RGBA8 texture every frame; (3) the one-pixel request; (4) glass `front`; (5) view modes, last.
