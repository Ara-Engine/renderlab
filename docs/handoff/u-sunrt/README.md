# U-SUNRT close-out

The controlling instruction is the 14 November close-out: no new investigations,
targets frozen, no comparison/identity campaigns. Run the overhaul through PC
integration's three-scene/two-path smoke; Linux suites are cleanup, not a gate.

## Reflection ray cost — WIP, not run

`t3/reflection-ray-cost` now includes a default-on `RENDERLAB_REFLECTION_BVH4`
prototype (`=0` retains the old builder, packing and WGSL for rollback).

- `src/reflection_software.rs`: uses the GI binned-SAH BVH4 builder, quantized
  64-byte nodes, eight-triangle leaves, original-ID payload permutation. Existing
  48-byte triangle records retain source corner order and edge arithmetic. The
  builder's 15-level limit implies at most 46 pending children; WGSL allocates 48.
- `shaders/reflections_software_bvh4.wgsl`: new closest/any-hit traversal,
  near-first four-child ordering, equal-t lower original primitive ID within an
  instance, existing lower-instance tie rule across instances. Old WGSL is kept.
- `src/primary_ray_gpu.rs`: appends the world-space union of every submitted
  software instance after the TLAS reserve and adjusts the refit's previous slice.
- `src/program.rs`: selects wide versus original WGSL from the same switch.
  Wide traversal clamps only to the padded scene-union box exit, never camera far
  or visible bounds.

**Exact next step:** build/run this prototype through the shared fast path and
PC integration smoke. It has NOT been compiled or rendered. Fix any composition
or runtime fault, then judge Glint/Outpost frame time and one frame per scene/path.
Do not present it as ready or merge solely on this handoff. Legacy CPU closest-hit
test helpers still decode BVH2 and need updating for the new default. The old
geometry-dump format label also still says BVH2; update it if diagnostics are ever
revived. Dumps/replay are abandoned under the new direction and are not gates.

## Transparency measurement — source complete, not run

`t3/transparency-measure` at debcd60e is unchanged. Linux checks and lib suites
passed (316 default / 327 backend-ara; focused counter/budget test both ways).
No PR opened and no rendered smoke. `RENDERLAB_TRANSPARENCY_LOG` is default off;
with PASS_LOG/DETAIL it records bounded parts and explicit incomplete remainder,
plus CPU draw/dispatch/area counters. See `docs/transparency-measurement.md` on
that branch. Next step is a run when the coordinator chooses to retain it.

## Preserved evidence and scripts

- `reflection-sah4-paper.md`: historical CPU estimates and tie counterexample;
  not a reason to restart a comparison campaign.
- `runtime-packing-export-proposal.md`: export #131, already merged.
- `transparency-cost.html`: source/old-row page, published version 2 at
  https://2559d3beb22a2e42.snist.dev/v/2.
- `joint-replay-plan.md`: superseded dump plan, retained for context.
- `tools/sunrt/`: archived geometry reader, list adapter, focused fixtures and
  capture script. These carry machine-specific D: paths; no waiter is archived
  as an active launcher. No raw dumps or private scene assets are committed.

Local evidence remains under `D:/Coding/renderlab-work/u-sunrt`; all PNGs,
MP4s, JSON summaries and logs stay. Root already deleted the cancelled Ara clone.
The dump waiter was stopped and never obtained a GPU lease. No Cargo process,
GPU lease or build lock belongs to this unit.
