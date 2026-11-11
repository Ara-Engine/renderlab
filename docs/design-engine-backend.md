# An engine-level entry for the ara-gpu backend

Design note, read and answered by the build thread on 2026-11-11 (on #35, against main
`467c053`); its three changes are folded in below and marked. Nothing here is built.

## Why

Real content exists only as calls on `engine::Engine`: `examples/game_scene.rs`, the
viewer's quarry loader and every engine example build their scenes that way. The
ara-gpu backend (draft #3, `src/backend_ara/`) renders a frame from its own
`ForwardScene`, which nothing but its tests and one fixture can fill. So no real scene
has gone through it, and the two most useful defects found so far (the restart key, and
the resolve taking the upscaling path on RADV) only appeared on contact with the real
renderer. Ara drives Renderlab through the engine, so the engine is also the seam a
product would use.

The aim: the same `Engine` calls, one description of each scene, rendered by either
GPU layer. `game_scene --path ara` is then the real scene with nothing to drift.

## What the engine holds today

`Engine` is a `Renderer` plus slots of renderer handles (`Mesh`, `Texture`, `Program`,
`Material`, `EnvironmentMap`, `Instances`, `Emitter`) and plain frame state (sun,
lights, sky, settings, decals). Every `create_*` call receives CPU data, uploads it at
once and keeps only the handle. Objects are batched by (mesh, material) with their
transforms. `render_to` turns batches into `Draw`s and calls `try_render_view`.

Two kinds of state are mixed there:

- **Device-free:** ids and their slots, the batch table and object transforms, sun,
  lights, sky, settings, decals, a material's factors and flags, a program's WGSL and
  its parameter layout, emitter parameters. None of it needs a device.
- **Residency:** the renderer handles above, upload admission, the output texture,
  pipeline caches, reload.

## The split

1. `engine::state` (new, device-free): `State<R>`, generic over what a layer keeps for
   each resource. `R` names the slot payloads (`R::Mesh`, `R::Texture`, `R::Program`,
   `R::Material`, `R::Environment`, `R::Emitter`, `R::Instances`); the state owns the
   slots, the batch table with object transforms, and the frame state (sun, lights,
   sky, settings), with the bookkeeping that needs no device: stale ids, spawn and
   despawn, the still-used checks on removal. *(Build thread: one set of slots with
   generic payloads, not id-only slots beside parallel handle tables, which would cost
   two lookups and two generation checks a call.)* For wgpu the payloads are today's
   handles, so the layout compiles to today's.
2. `Engine` keeps its renderer and holds a `State` of its own payloads. Its public API,
   behaviour and errors do not change; `renderer` stays its first field, so handles
   still drop after the renderer. This step is a move, like #23, #24, #27 and #32.
3. A trait with the scene-building and frame calls an example actually uses. `Engine`
   implements it by delegating to its inherent methods, which stay the documented API
   (docs/api.md): static dispatch, no cost, and no caller needs the trait in scope. A
   call nobody generic needs yet stays off it.
4. In the draft, behind the `backend-ara` feature: `backend_ara::engine::Engine`
   implements the same trait over a `State` of its own payloads (the mesh and texture
   bytes it must keep, a program's WGSL and layout) and renders through
   `forward::Frames`.

Off the trait, on the wgpu `Engine` alone: `new`, `path`, `adapter_info`,
`take_compilation_timings`, `device`, `queue`, `output_format`, `output`, `renderer`,
`renderer_mut`, `render_to`, `present`, `reload_program`, and `poll` until the backend
has events to report. Experimental switches reached through `renderer_mut()` are not
part of this seam.

### What must survive the move exactly (build thread)

- `set_sun`, `set_lights`, `set_sky`: replace the stored value, apply to the renderer,
  restore the old value on error. The error comes from the device side.
- `set_settings`: ablation from `RENDERLAB_ABLATE`, the equality early-out, atmosphere
  and `sky_light` validation, `settings::apply`, and only then the store.
- `set_transform`: with live instances the finite check and the write happen in
  `Instances::set` before the stored transform changes; without them the engine checks.
  Same error text either way.
- `spawn` and `despawn`: clearing the batch's instances is the re-upload signal;
  `max_instances` is checked against the batch.

### Calls the wgpu side passes through and stores nothing for

`set_decals`, `set_colour_lut` and `set_joint_palette` go straight to the renderer; a
stored copy would be new work each call or each frame. `create_mesh` and its kin,
`create_texture`, `create_environment`, `create_emitter` and `update_emitter` hand
their data to an upload and keep the handle; `create_texture` and `add_asset` take
their data by value and the trait keeps those signatures. `create_program` keeps the
`Program` handle; only the backend's payload carries the WGSL. The backend keeps what
it needs of each in its own payloads.

`add_asset` prepares receiver adjacency by path, so each layer implements it.
`add_skinned_asset` is built from `create_texture`, `create_material` and
`create_skinned_mesh` alone and becomes one function generic over the trait. `cut`
holds no state.

## How programs and materials are identified without a device

- A **program** is its name, its WGSL source and the parameter layout reflected from
  that source (`program::ParameterLayout`, already device-free). `ProgramId` indexes
  the state's slot; the standard program keeps the id the engine gives it at creation.
- A **material** is its `MaterialDesc` resolved to ids: factors and flags, a
  `ProgramId`, `TextureId`s with samplers, transparency, and its parameter block
  packed against the program's layout (`ParameterLayout::pack`). `MaterialId` indexes
  that record.
- A **texture** and a **mesh** are ids over data the residency side owns. The wgpu
  engine keeps handles only, as today; the backend keeps the bytes.

The backend frame already takes programs this way (`ForwardScene::programs`: a source
and named values per object, several programs a frame).

## Refusals

The backend does not implement everything. A call it cannot honour must not fail
silently and must not stop a scene at its first gap. The backend's engine records a
`Refusal { call, what }` for each unsupported call or setting, goes on without that
piece, and exposes the list; a strict mode returns the first as an error. A run then
ends with every refusal named, which is the list that orders the port. The wgpu engine
has no refusals and gains no API for them.

`Settings` and `FrameReport` cross the trait as whole values, never mirrored field by
field *(build thread: both gain fields with the non-RT work)*. The backend compares a
`Settings` against `Settings::default()` to find what a scene asked for and refuses by
field name (`Refusal { call: "set_settings", what: "volumetric_fog" }`); a field it
does not know refuses by default. A new setting then costs the trait nothing.

## What the wgpu path must not pay

- No CPU copy of anything it does not keep today.
- No dynamic dispatch and no extra branch per call or per frame.
- No behaviour change, no new error, no change to the public API or its docs.
- No new dependency; nothing of the backend compiles without `backend-ara`.

## What the backend's first frames cost

Measured on the game scene once the backend's engine ran it (M5 Pro, 1280x720): a
frame that rebuilt everything took 196 ms, and keeping meshes and bottom-level
structures across frames saved 10 of them. The time was in parsing and validating
every program's WGSL each frame to reflect its inputs; keeping reflection per program
source brought the frame to 86 ms. A layer's engine must hold what `create_program`
makes, not only what `create_mesh` uploads.

## Order of work

1. A PR on main, now: `engine::state` and the trait, moves only, existing engine tests
   and examples unchanged in result. Flagged as a shared-planner change; the build
   thread checks the lib suite, game-scene frame identity on both paths and one quarry
   timing visit.
2. A second PR on main, right after N-VSM1 merges (it has `game_scene.rs` open): the
   example's scene construction and per-frame animation move into two functions generic
   over the trait (`build(&mut impl Scene, &Options) -> Handles`,
   `animate(&mut impl Scene, &Handles, frame)`), in a file both examples include.
   `game_scene.rs` calls them with the wgpu engine and keeps its own loop, timing,
   diagnostics and capture. *(Build thread: not the whole file generic; it is edited by
   almost every unit.)*
3. In the draft: the backend's engine, and an example that calls the same two
   functions. One description of the scene, nothing to drift.

## Decided on #35

- The API in `engine/mod.rs` is frozen at the base, additions only. `settings.rs`
  changes with the non-RT work (`Settings` fields, `settings::apply`), as may `Limits`,
  `EngineDescriptor` and `FrameReport`.
- A trait, not an enum inside `Engine`.
- The scene as two shared functions, not a generic `game_scene.rs`.
