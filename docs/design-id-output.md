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
pub enum ViewMode { Lit, BaseColor, Normal, Roughness, Metallic, Emission, Occlusion, LightingOnly, Overdraw }
impl Engine {
    /// The embedder's own 32-bit id for an object (Ara's entity); 0 means none.
    pub fn set_object_id(&mut self, object: ObjectId, id: u32) -> Result<(), Error>;
    /// The view's ID target of its last rendered frame: R32Uint, same size as its output.
    pub fn view_ids(&self, view: ViewId) -> Option<&wgpu::Texture>;
    /// Asks for the ids of a rectangle of the view's last frame without stalling: the copy is
    /// queued after that frame and its result arrives in `poll()` as `Event::Ids`.
    pub fn request_ids(&mut self, view: ViewId, rect: IdRect) -> Result<IdRequest, Error>;
}
pub enum Event { /* existing */ Ids { request: IdRequest, rect: IdRect, ids: Vec<IdPixel> } }
pub struct IdPixel { pub opaque: u32, pub front: u32 }
```

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
