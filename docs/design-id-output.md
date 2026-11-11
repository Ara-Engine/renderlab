# Entity-ID output and view modes (spec)

Root, 2026-11-11. Draft for review by the Ara bridge thread and the editor's owner before the API is fixed. Row 1 of [ara-render-replacement.md](ara-render-replacement.md).

## What it is for
- **Picking in the editor:** which object is under a pixel, and later a rectangle's objects.
- **Legends in agents' captures:** a capture carries, per pixel, which object it shows, so an agent can say "the lamp at (412, 300)".
- **Debug view modes** that Ara exposes (base colour, normals, roughness, metalness, lighting only, and others below), and the rendering debug switches Ara reports and Renderlab ignores today.

## What exists to build on
- Both paths already write a per-pixel hit record before shading: placement + 1 (0 is a miss), primitive, barycentrics (`primary_hit`). The non-RT visibility buffer and the RT traced primaries produce the same record.
- The engine's batch table knows, per placement, which `ObjectId` it is (`State::batches`: each batch's `objects` in instance-slot order).
- `Renderer` already has a `ViewMode` (`BaseColor`, `Normal`, `Metallic`, `Roughness`, `Lit`), used by the raster path.

## Proposed API (engine level)

```rust
/// What the engine writes besides the colour image.
pub struct Outputs { pub ids: bool }          // Engine::set_outputs; off by default
pub enum ViewMode { Lit, BaseColor, Normal, Roughness, Metallic, Emission, Occlusion,
                    LightingOnly, Overdraw, Wireframe /* later */, ShadowCascades /* later */ }
impl Engine {
    pub fn set_outputs(&mut self, outputs: Outputs) -> Result<(), Error>;
    pub fn set_view_mode(&mut self, mode: ViewMode) -> Result<(), Error>;
    /// The ID image of the last rendered frame: one u32 per pixel, an object's tag (below)
    /// or 0 for none. Same size as the colour output.
    pub fn id_output(&self) -> Option<&wgpu::Texture>;          // R32Uint, for GPU consumers
    pub fn pick(&mut self, x: u32, y: u32) -> Result<Option<ObjectId>, Error>;   // one-pixel readback
    pub fn capture_ids(&mut self) -> Result<IdImage, Error>;   // whole-frame readback for legends
    /// The caller's own 32-bit tag for an object (an Ara entity id); 0 is reserved.
    pub fn set_tag(&mut self, object: ObjectId, tag: u32) -> Result<(), Error>;
}
```

- **Tags, not internal ids.** The ID image holds a caller-chosen 32-bit tag per object (Ara's entity index), so Ara needs no table to translate back. Untagged objects write their engine object index + 1 with the top bit set, so they are still distinct.
- **Which surface a pixel names:** the nearest opaque surface on the primary visibility (the hit record). Transparent surfaces and particles do not write IDs in the first version; a stated rule, open to the consumers (see questions).
- **Cost:** one compute pass after primary visibility, reading the hit record and a per-placement tag buffer (one u32 per placement, rewritten when tags or batches change). Nothing runs when `ids` is off.
- **Temporal reconstruction:** IDs come from this frame's unjittered-or-jittered hit record, so an edge pixel names whichever surface this frame's sample hit. For picking that is right; for legends it is right at pixel precision.

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
