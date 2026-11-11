# Unit U-VIEWS brief (Renderlab): more than one view per `Engine`

For the unit that runs on the Linux box (m6u, Radeon 760M with hardware ray queries). Part of replacing Ara's renderer with Renderlab (docs/ara-render-replacement.md, row 2). A second unit, U-IDS, builds object IDs and view modes in parallel on the Windows PC; read "Ownership" before writing code.

## Target
Islam's target for this and U-IDS together: done by about 17:00 UTC 2026-11-12. Say early, in a one-line note to the Renderlab root (via the coordinator), if a part clearly will not fit, and why.

## Identity and branch rules
- Commits are authored and committed as `Snowy <97126807+Snowy7@users.noreply.github.com>`. Print `git config user.email` before your first commit; if it is anything else, stop and tell root.
- Never force-push, including your own branch. Merge main into your branch; never rebase.
- No compatibility work: nothing in Renderlab or Ara has users. Change any API, including `Engine::render`, `resize`, `output`, `capture` and `cut`.

## Read first
- AGENTS.md (last lines), docs/ara-render-replacement.md ("Row 2 in detail"), docs/design-views.md (your API shape; the Ara bridge is already writing against it; change it only with a note to root and keep the doc current), docs/design-id-output.md (U-IDS's, which hangs off your view).
- src/engine/mod.rs, src/engine/state.rs (PR #36: device-free scene state, `Residency`, the `Scene` trait), src/renderer.rs (`Renderer`: which fields are per view; the list in the replacement doc is a start, not complete).

## Build
1. Split `Renderer` into shared scene and cache state and a `View` that owns everything keyed on the camera or the output: targets, temporal reconstruction histories and restart zones, upscale state, post histories, atmosphere temporal volumes, reflection film accumulation, the shadow clipmap placement and page requests, the GI per-pixel probe and idle key, transparency targets, frame records. Shared: meshes, textures, programs, materials, BVHs and acceleration structures, the GI world cache, shadow page contents, lights, environment, pipelines.
2. The `Engine` API in docs/design-views.md: `create_view`, `set_view`, `remove_view`, `render_view`, `render_view_to`, `view_output`, `capture_view`, `cut_view`. Update this repository's examples and tests to it.
3. A view that is not rendered costs nothing. Two views rendered in one frame share the scene work that does not depend on the camera (skinning, particle simulation, GI cell updates) once per frame, not once per view, where that is simple; say what you did.
4. The shadow map: per-view placement and page requests over a shared page store if it fits; otherwise per-view maps, with the memory stated.

## Done
- Two views of one scene from two cameras: each view's still frame byte-identical to a single-view engine rendering that camera alone, both paths (non-RT on this machine; RT picture checks also run on the PC's RTX: ask root).
- A moving pair (one view moving, the other still, then both moving): no history bleeding between views; the still view stays byte-identical to rendering it alone.
- Frame time for two views against two engines with the same cameras, and the memory of a second view in MiB, on the game scene and Showcase.
- `cargo test --lib` and the GPU tests you touched pass on this machine.

## Ownership (agreed with U-IDS; do not edit the other's files)
- **Yours:** `Engine`'s view state, `ViewId`/`ViewDesc`, src/engine/mod.rs and src/engine/state.rs, splitting src/renderer.rs, and the per-view state in temporal_reconstruction.rs, post_gpu.rs, atmosphere_gpu.rs, the target allocation in primary_ray_gpu.rs and visibility_buffer.rs, virtual_shadow_gpu.rs's requests and placement, gi_cache.rs's probe state, transparency_gpu.rs's targets; examples and tests that call the old single-view API.
- **U-IDS':** a new id module and its shaders, the id table and `set_object_id`, the id readback and its event, view-mode shading switches. U-IDS keeps its per-view state in one struct and one `Renderer` field; when you introduce `View`, move that field into it.
- Your view handle comes first in the public API; U-IDS's outputs hang off a view. Merge main often.

## Machine rules (m6u)
- The Linux gate and the Ara bridge threads share this machine: at most two Cargo build jobs (`-j 2`), and take the GPU lock for every GPU run: `flock /tmp/ara-m6u-gpu.lock <command>`.
- Use your own Cargo target directory; delete it at the end. Check `df -h` before large builds and stop under 30 GB free.
- **Timing:** every timing run opens with a reference run (a fixed binary and scene whose time you know on this machine); more than 10% off means discard and rerun. Record the GPU clock state at start and end if the driver exposes it.
- Do the work yourself: start no subagents or delegates.
- Commit and push after each working step; keep docs/design-views.md current. If your working room runs low, stop at a committed state and report what is left.
- Shading stays programmable. Per-draw facts come from `primary_batch_info`, never `arrayLength` of a binding-array element. Never decide a code path by float equality on a computed quotient.
- For the ara-gpu backend: per-frame records become per view; flag every shared planner whose signature changes.

## Deliver
Branch `t3/views` (from origin/main) pushed after each step; one MERGE line per landed step; a final report with the identity results, the two-view timing and memory, and what the bridge needs next.
