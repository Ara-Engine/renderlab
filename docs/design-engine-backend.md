# An engine-level entry for the ara-gpu backend

Design note for the build thread to read before any code moves. Against main
`9771303`. Nothing here is built.

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

1. `engine::state` (new, device-free): the slots, ids, batch table, object transforms
   and frame state move here unchanged, with the validation that goes with them (stale
   ids, descriptor checks). It stores what a call *names*, never what it uploads.
2. `Engine` keeps its residency and holds a `state`. Its public API, behaviour and
   errors do not change. This step is a move, like #23, #24, #27 and #32.
3. A trait with the scene-building and frame calls (`create_mesh`, `create_material`,
   `spawn`, `set_transform`, `set_lights`, `set_sun`, `set_sky`, `set_settings`,
   `create_emitter`, `set_decals`, `render`, `capture`, `cut` and their kin).
   `Engine` implements it by delegating to its inherent methods: static dispatch, no
   cost. Examples that should run on either layer become generic over it.
4. In the draft, behind the `backend-ara` feature: `backend_ara::engine::Engine`
   implements the same trait over `engine::state` plus its own residency (CPU copies
   of mesh and texture data, since the backend rebuilds its frame from them) and
   renders through `forward::Frames`.

Device and window calls stay on the wgpu `Engine` alone and out of the trait:
`new`, `device`, `queue`, `renderer`, `render_to`, `present`, `output_format`,
`reload_program`, pipeline-cache and adapter queries.

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

This is what removes the backend frame's one-program limit: today its material's
program is a handle tied to a wgpu `Renderer`, so the frame can only name the standard
program.

## Refusals

The backend does not implement everything. A call it cannot honour must not fail
silently and must not stop a scene at its first gap. Proposal: the backend's engine
records a `Refusal { call, what }` for each unsupported call or setting, goes on
without that piece, and exposes the list; a strict mode returns the first as an error.
A run then ends with every refusal named, which is the list that orders the port. The
wgpu engine has no refusals and gains no API for them.

## What the wgpu path must not pay

- No CPU copy of anything it does not keep today.
- No dynamic dispatch and no extra branch per call or per frame.
- No behaviour change, no new error, no change to the public API or its docs.
- No new dependency; nothing of the backend compiles without `backend-ara`.

## Order of work

1. This note is read; the build thread says where it collides with engine work in
   flight.
2. A PR on main: `engine::state` and the trait, moves only, existing engine tests and
   examples unchanged in result.
3. In the draft: the backend's engine, and `game_scene --path ara`.

## Questions for the build thread

- Is the engine API still frozen at the base, or is anything in `src/engine/` about to
  move for the non-RT work?
- Trait, as proposed, or an enum inside `Engine`? The enum needs no generic examples
  but puts a branch on every call and backend types into `engine/mod.rs`.
- `game_scene.rs` would become generic over the trait for its scene and frame loop.
  Is that acceptable in a file you edit often, or should a second, smaller example
  carry the backend path first?
