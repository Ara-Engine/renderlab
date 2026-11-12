# U-RTVIS: the RT path's visibility pass on large scenes

Outpost (Ara, RTX 4070 SUPER, 1080p) spends 12.5 ms of a 35 ms frame in `visibility` while
moving and 62 ms on the frame after a camera cut, against 4-5 ms on Showcase (docs/m7-gaps.md,
2026-11-12). This unit finds what that pass pays for and cuts it.

## What `visibility` is on RT

With temporal reconstruction on (the default), `visibility` is the reconstruction's fused
Interactive trace (`encode_reconstruction_interactive`, src/primary_ray_gpu.rs): one dispatch
per 8x4 tile that traces each pixel's camera ray, builds its receiver, and traces the sun's
shadow directions for it, followed by the near-edge repair passes. The shadow budget per
receiver comes from `ReconstructionShadowBudget` (src/temporal_reconstruction.rs):

| frame | directions per receiver |
|---|---|
| cold: first frame, `cut_view` | `cold_rays` 256 |
| reset: key change, sun turn | `reset_rays` 64 |
| moving or young history | `active_rays` 32 |
| still for `settle_frames` (16) | `held_rays` 8 |

So a moving camera traces 32 sun rays per pixel every frame, and a cut 256. Fitting root's
cut1 capture (13.6 ms at 32, 62.3 ms at 256) to `a + b x rays` gives b = 0.22 ms per ray per
pixel and a = 6.6 ms that the budget does not touch (camera rays, receivers, repair).

`RENDERLAB_PASS_LOG_DETAIL=1` now splits the pass: `visibility` is the fused trace;
`visibility.repair`, `.check`, `.retry` and `.penumbra` are the passes after it.

## Suspect 1 (single-sided draws) does not apply to shadowed frames

`shaders/primary_trace.wgsl` traces camera rays non-opaque only when `primary_frame.sided` is
set, that is when some draw is single-sided. A shadowed frame never has one:
`hardware_shadow_gpu.rs` (`placements`, `casters`) refuses single-sided materials whenever the
frame casts hardware shadows, and Ara's Renderlab bridge sets `double_sided: true` on every
material for that reason (ara-renderlab `scene.rs`, "Renderlab's shadowed paths need
double-sided materials today"). Outpost's camera rays are already `RAY_FLAG_FORCE_OPAQUE`, so
`RENDERLAB_DIAG_ALL_OPAQUE=1` cannot change its timing; the game scene's unchanged 5.40 ms
agrees.

The other traces: sun shadow rays (`fresh_shadows.wgsl`) are `FORCE_OPAQUE |
TERMINATE_ON_FIRST_HIT`; GI, reflection, atmosphere and reference rays are `FORCE_OPAQUE`. The
one non-opaque query left on opaque scenes is the receiver repair's edge probe
(`primary_trace.wgsl` `primary_repair`, only for receivers near a triangle edge of a mesh without
adjacency, over a few epsilons). Splitting opaque and single-sided instances onto TLAS mask
bits would only pay once single-sided casters are allowed on shadowed frames; it is not built.

## Step 3: cold frames probe first (`COLD_PROBE_STOP`)

On a cold frame every receiver first traces the moving budget's directions (`active_rays`,
32: the same first 32 of the cold sequence, in the same order). A receiver whose 32 probes
all agree stops there and stores 0 or 256; the rest continue through all 256 directions, so
their estimate is main's exactly. Planner: `probe_rays = active_rays` on cold frames without
P-RTS listing (`sampling.y`); the trace module stops at `j == sampling.y` when
`sampling.y < sampling.x` and the count so far is 0 or j. `RENDERLAB_COLD_PROBES=0` restores
the full budget everywhere (measurement switch).

Error: a receiver with true visibility v is called unanimous with probability about v^32
(or (1-v)^32), and is then off by 1-v (or v). The expected error (1-v) v^32 peaks at v =
32/33 at 1.1 % of full visibility; the largest single-pixel error is under 3 % with about 37 %
odds, at the outer rim of penumbrae. The 256-ray estimate's own noise there is about 1.1 % (one
standard deviation), so the cut frame's error stays at the size of main's grain. Low-
discrepancy probes (R2 over the disk) find any occluder that covers more than a few percent of
the disk. Following frames are unbiased (32 rays each, no stop) and average the cut frame's
error away as its history weight falls.

Not chosen: spreading the cold budget over frames (each frame weighs 1 in the resolve whatever
its rays, so a 64-ray cut frame has twice main's noise and the next frames are no better);
answering the cut from a sun map (the RT path keeps none); P-RTS's 8+8 probes with a listing
pass (missed thin rims, docs/p-rts.md).

## Ara (Mac port) flags

- Shared planner: `Planner::prepare` (src/temporal_reconstruction.rs) sets `probe_rays` on cold
  frames; both backends plan through it.
- Shared trace module text: `trace_source` adds `COLD_PROBE_STOP`; src/backend_ara builds its
  trace kernel from the same function.
- No new buffer, texture, binding or TLAS mask; `PrimaryFrame`'s layout is unchanged
  (`sampling.y`, the probe count, now also means the cold probes when listing is off).

## Results

Pending the first lease.
