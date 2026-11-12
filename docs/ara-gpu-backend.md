# The ara-gpu backend: status

Renderlab's frame on Ara's GPU layer (`ara-gpu`) instead of wgpu, so that Ara can draw
with Renderlab on its own device: Metal on the Mac, Vulkan elsewhere. It is the Mac's
only route to Renderlab (`docs/ara-render-replacement.md`). The wgpu renderer is
unchanged and stays the default; the backend is the `backend-ara` feature, in
`src/backend_ara/`.

The working notes this page replaces (every port step, its tests and its measurements,
2026-11-10 to 11-11) are in git history: `git show 1a2c424:docs/ara-gpu-backend.md`.

## How it works

- **Shaders.** The renderer's own WGSL, unchanged, goes through naga to MSL (Metal) or
  SPIR-V (Vulkan) at run time (`shader/`). Bindings become ara-gpu heap indices
  (`bindings.rs`). Compiled code is cached under the caller's cache root, beside each
  pass's reflected inputs (`shader/reflection.rs`), so a warm process parses no WGSL.
  On Metal the MSL is compiled with Apple's Metal toolchain
  (`xcodebuild -downloadComponent MetalToolchain`).
- **Runtime checks follow the wgpu renderer (P-SPLIT).** Engine kernels are written
  without naga's loop bounds, ray-query tracking or integer-division checks. Every module
  composed from program text keeps them: user programs, the bundled standard and toon
  programs, glass and cell shading (`shader::Checks`, `Program::checked`). Image-load
  size tests and workgroup zeroing are dropped where the device does them
  (`DeviceGuards`, from ara-gpu's `DeviceInfo`). Both choices are part of the pipeline
  key. Float results keep `NoContraction`: without it the frame changes.
- **A frame** (`forward::frame`) is a graph of passes (`graph.rs`): every resource's
  state is tracked, and a read recorded before a write is refused. `forward::Frames`
  is a sequence of frames and keeps on the GPU:
  - the radiance cache and the reconstruction's history;
  - meshes and their bottom-level structures, by identity;
  - textures, by identity and slot;
  - the sky map;
  - the atmosphere's tables;
  - the Interactive trace's repair queue;
  - the two displayed images;
  - a pool of 2D targets (`image::pool`).

  Records and small buffers are still made per frame.
- **The engine entry** (`engine::Engine`) takes the same scene calls as the wgpu
  `engine::Engine` and records them without a device. `render_view(gpu, view, camera)`
  draws on the caller's device. `view_output(view)` is the view's displayed image:
  sRGB-encoded RGBA8, left in `CopySource` for a presenter. `release(gpu)` and `retire(gpu)` free what it keeps
  there. `on(gpu)` gives the engine and the device together as an `engine::Scene`.
  What it cannot honour it records once as a `Refusal { call, what }` and draws the
  rest.

## What draws

On the traced frame (`Quality::Realtime` is the Interactive trace, `Reference` the
exact one):
- Opaque meshes, batched by mesh and material and instanced.
- Skinned meshes: posed on the CPU when a palette is set, with SK1's operations
  (`skinned.rs`). Each pose's geometry is built for that frame; its previous pose feeds
  reconstruction's motion; its revision feeds the cache's and the history's pose checks.
- The standard program and user programs, with their parameters; several programs in
  one frame (tile bins).
- Base-colour, metallic-roughness and normal maps (with the mesh's tangent stream).
- The sun with traced soft shadows; clustered point and spot lights with traced
  shadows.
- Uniform, atmosphere and map skies: `Sky::Map` in both of the renderer's layouts,
  with intensity, rotation and visible or scaled backgrounds.
- Aerial perspective, height fog and volumetric fog: the fog's cells see the sun by
  one ray a sample against the frame's scene, as the wgpu renderer's hardware path
  does, and keep a two-frame history that the same changes invalidate.
- The radiance cache (GI) and traced reflections.
- Temporal reconstruction at every render scale.
- Transparent panes with their coloured sun shadows.
- Post: bloom, motion blur, depth of field, tone mapping and grading, with the colour
  LUT.
- A scene without a sun.

Refused by the engine entry, and reported:

| Call | What |
|---|---|
| `set_settings` | `sky_light` other than 1; `post.enabled` false; any field added to `Settings` later, while it differs from its default |
| `add_asset` | cooked assets |
| `create_emitter`, `update_emitter` | particle emitters |
| `set_decals` | decals |
| `render` | textures on a transparent material |

Errors from `render`:
- more than `TRACED_DRAWS` (256) draws;
- a normal-mapped material on a mesh without tangents (the wgpu renderer refuses that
  draw too).

Not on this backend: the raster frame beyond the diagnostic mesh pass, and the non-RT
path's software rays (the port is the traced frame only).

## Where it has run

- **Metal, M5 Pro (macOS 27):** the whole suite,
  `cargo test -p renderlab --features backend-ara --test ara_backend -- --test-threads=1`,
  91 tests, under `MTL_DEBUG_LAYER=1` with `MTL_SHADER_VALIDATION=1` and `=0`. Ara's
  player draws Showcase with it through the presenter's own Metal device (Ara #71).
- **Vulkan, Radeon 760M (RADV) and RTX 4070 SUPER:** the suite and the timings below,
  on the gate machine. The heads from the engine entry (0a4307c) on have run on Metal
  only; the gate's next pass runs them on Vulkan.

## Measurements (current)

On the game scene (`ara_frame --scene game --supported --timing`, 1280x720, interleaved
runs, frames compared byte for byte):

| Device | wgpu | port | Notes |
|---|---|---|---|
| Radeon 760M | 86.4 ms | 95.6 ms (1.106x) | at 7aaffdd. The camera rays are 12.2 against 11.4, the shade list 36.1 against 34.2, the resolve 4.7 against 3.8. The device guards (Ara #65) since then are worth about 2.1 ms there; not yet remeasured |
| RTX 4070 SUPER | 10.5 ms | 14.0 ms | after the binning arguments moved to device memory (`ProgramBins` 6.9 to 0.15 ms). Without the cache the port is still 1.36x. The rest has no single owner: cache +1.3, shade list +0.7, resolve +0.36, recording +0.25 |
| M5 Pro | — | 66.7 ms GPU | Metal System Trace with encoder labels: shade list 25.3, camera 10.2, cache 22.8, resolve 3.2. wgpu does not run on the Mac |

A warm process's first frame on the Mac is about 185 ms (it was 380 ms before the
reflection was kept on disk).

## Open

- **Showcase's frame on the Mac.** At 1280x720 a frame spent about 213 ms recording
  before submission and 116 ms on the GPU. Every texture was uploaded again each
  frame; textures are now kept by identity. The remeasure is next.
- **The Radeon's one pixel** at the arch lamp (588,178) under reconstruction: the port
  on RADV gives 0.5566 where Metal, wgpu and the RTX give 0.6279. The trace is built:
  `RENDERLAB_ARA_PROBE_RESOLVE=X,Y` (`resolve_probe.rs`, measurement only) prints the
  resolve's taps for one pixel. It waits for its Radeon run.
- **NVIDIA's remaining 3.5 ms:** parked; see the table.
- **Drawn by the wgpu renderer and not yet by the port, in this order:**
  1. decals;
  2. particles;
  3. textures on transparent materials;
  4. reflections inside panes.

  Then skinning on the GPU (SK1's ported kernel, with structure refits) in place of the
  CPU pose and per-pose build, for frame cost.
- **Views** (docs/design-views.md): the entry has the views API. `tick`,
  `create_view`, `set_view`, `remove_view`, `render_view(gpu, view, camera)`,
  `cut_view`, `capture_view` and `view_output`; a view's look is its `ViewDesc`'s.
  Each view draws through its own frame sequence (histories, cache, resident meshes
  and textures); the views share the scene and one pool of frame targets. Two views of
  the game scene are each the bytes of one view alone. Not yet shared between views:
  the resident meshes, textures and sky map, and the radiance cache (the wgpu engine
  shares the cache table and keeps a `ViewPlan` per view). View modes other than lit
  are refused.
- **Ids** (docs/design-id-output.md): a view with `outputs.ids` gets #41's id pass
  (`shaders/id_output.wgsl` up to the handoff) after reconstruction, with the same tag
  tables, settled ping-pong and two-layer R32Uint id texture. The calls are
  `set_object_ids`, `request_ids`, `poll`, `view_ids` and `view_ids_packed`. Where it
  differs from wgpu:
  - the packed RGBA8 image is a compute pass (`shaders/id_pack.wgsl`) into a storage
    image, not a render pass, with the same `(k + 0.25) / 255` bytes;
  - `request_ids` reads back at once, and its event arrives at the next `poll`;
  - the front layer is the opaque one: glass front ids are not ported.

  The test draws a tagged floor, a tagged raised square, an untagged square and sky, on
  the exact and the reconstructed frame; each reads its id, and the packed image is the
  readback's bytes at every pixel.

## Fixture

`support/ara_game_scene.rs` describes the example game scene (`examples/game_scene.rs`)
through `engine::Scene` calls, for the engine-level test and `ara_frame --scene game`.
It is a second copy of the example's construction. It is kept as a test fixture until
the example's construction is a function both layers call; then both include that.

## Testing

- `cargo test -p renderlab --features backend-ara --test ara_backend -- --test-threads=1`.
  On the Mac add `MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1`. The GPU tests need the
  Metal toolchain (Mac) or `VULKAN_SDK` (elsewhere). Tests write only under `target/`.
- `examples/ara_frame.rs` (its module docs list the options) renders the fixture
  levels or the game scene on either layer, compares them (`--compare`) and times them
  (`--timing`, `--without NAME`).
- Rust 1.99.1; `backend-ara` also enables `wgpu/vulkan-portability` so the crate builds
  on macOS. It creates no wgpu device.
