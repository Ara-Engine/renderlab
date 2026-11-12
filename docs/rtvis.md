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
adjacency, over a few epsilons): minor, left as it is. Splitting opaque and single-sided
instances onto TLAS mask bits would only pay once single-sided casters are allowed on shadowed
frames; step 2 is dropped (root, 2026-11-12).

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

## Step 5: the held budget on moving frames (`HELD_STOP`)

Moving frames trace `active_rays` (32) per receiver; on the forest that is 9.6 ms, almost all
of it in fully lit or fully shadowed pixels. The held budget lets those pixels stop early,
only where the picture already knows the answer:

- A receiver stops after `HELD_PROBES` (8) directions when they agree with its history, and its
  history is saturated: every texel of the reprojected 3x3 in the previous frame's held history
  has been unanimously lit, or unanimously shadowed, for at least `HELD_STREAK` (4) frames. The
  stop stores 0 or 256 exactly as a full unanimous estimate would.
- Never on: frames that restart history (cut, first frame, reset, any sun turn: the cold rule
  covers cuts), restart-zone receivers (`tr1_rays` above the frame's budget), receivers whose
  placement moved or is skinned (their reprojection would read another surface's history),
  receivers in the swept sun shadow of any placement that moved or is skinned (the planner's
  `moving_bounds`, both transforms, tested as `tr1_caster_shadow` tests SK1's bounds), the
  image's border pixels, P-RTS listing frames, and the repair and retry entries (they keep
  the full budget).
- History word per pixel, written by both trace entries every reconstructed frame (stop on or
  off): `class << 8 | streak`, class 1 all blocked, 2 all clear, 0 otherwise (fractional,
  miss, skip, uncovered tile); the streak continues when the reprojected centre texel had the
  same class.

Why a rim pixel does not lock in: a receiver with true visibility 0.97 needs 32-direction
estimates that all come back clear, in itself and its eight neighbours, four frames running,
before it may stop; its inner neighbours (lower visibility) almost never do, so penumbrae keep
the full budget. A moving caster's shadow cannot leave a stale patch on saturated ground: the
ground in its swept shadow traces the full budget on every frame that it moves.
`RENDERLAB_HELD_PROBES=0` disables the stop (history is still written): main's picture.

Results (RTX 4070 SUPER, 1080p RT, release game_scene; same binary, the stop on against off,
interleaved twice; the lease's reference run 26.58 ms against 26.7 expected):

| moving frames, medians | visibility on | off | frame on | off |
|---|---|---|---|---|
| forest walk, sun 4 deg | 2.72 / 2.73 | 9.57 / 9.60 | 18.96 / 18.99 | 25.92 / 25.97 |
| game flight with the Fox | 4.60 / 4.63 | 5.39 / 5.41 | 30.22 / 30.38 | 30.99 / 31.07 |

The forest saves nearly all it could (an 8-direction budget everywhere costs 2.62). The game
scene saves less: the walking Fox and the spinning crate move every frame, and the ground in
their swept sun shadow keeps the full budget.

Picture (`heldpics` plan, every frame of 60; tools/rtvis/compare.py `held`, tools/blobs.py):
- Non-RT: byte-identical, all 60 frames of the game flight.
- RT, stop on against off: frames 0-3 identical (the streak is not yet reached), then at most
  7-8 codes on at most 9 pixels a frame; no pixel more than 8 codes in any clip (forest walk,
  game flight). Whole-frame difference maps x16 are black.
- Moving casters over saturated ground (`--diag hold=1`: the camera held at its first pose while
  the Fox walks and the crate spins): at most 2 codes anywhere, and the Fox's shadow region is
  byte-identical at frames 10, 30 and 50 (crops-held/fox-shadow-hold-on-off-diff16.png).
- blobs.py: the same blob counts frame for frame, on and off, in all three clips.
- Against the path tracer (reflection_matched `--matched` against `--reference 4096`, frames 16,
  32, 47 of its moving flight with the spinning crate and bobbing hero): mean error 0.799 /
  0.943 / 1.108 codes over the image and 6.967 / 7.685 / 8.452 on edges, identical on and off;
  on and off differ by at most 1 code.

## Ara (Mac port) flags

- U-RTVIS step 5 (held budget), wgpu renderer only. New per-view resources:
  two `R32Uint` textures at the trace (internal) resolution, ping-ponged by the
  reconstruction's parity (8 MB each at 1080p), and a 128-byte uniform (`RtvisHeld`:
  previous unjittered matrix, words, moving bounds, sun direction and disk). New group-0
  bindings 50 (uniform), 51 (`texture_2d<u32>`, previous history) and 52
  (`texture_storage_2d<r32uint, write>`, this frame's) on a new layout used by the
  reconstruction's two trace entries only. Shared planner: `PlanInputs::moving_bounds` and
  `Planned::held`. The port keeps `trace_source` (no held seams) and passes
  `moving_bounds: None`; it needs the textures, bindings and `trace_source_held` to adopt it.

- Shared planner: `Planner::prepare` (src/temporal_reconstruction.rs) sets `probe_rays` on cold
  frames; both backends plan through it.
- Shared trace module text: `trace_source` adds `COLD_PROBE_STOP`; src/backend_ara builds its
  trace kernel from the same function.
- No new buffer, texture, binding or TLAS mask; `PrimaryFrame`'s layout is unchanged
  (`sampling.y`, the probe count, now also means the cold probes when listing is off).

## Results (RTX 4070 SUPER, 1080p RT, release game_scene, 2026-11-12)

The forest's terrain was one 320k-triangle mesh for these runs; since the GI study it is the same
vertices in 16 tiles (the non-RT Virtual sun map's 32-bit packing).

Every lease opened with the reference run (default game scene, RT, 1080p): frame median 26.67,
26.54 and 26.84 ms. Medians over moving frames (8-22, 40-119), ms; "cut" is frame 24's
`visibility` (`--diag cut=1`). Repro runs set `RENDERLAB_COLD_PROBES=0` (main's cold budget).

| run (tools/rtvis/timing.py) | frame | visibility | cut |
|---|---|---|---|
| forest, sun 4 deg (`--hour 17.651`), two visits | 26.0 | 9.56 / 9.59 | 74.1 / 74.3 |
| forest, moving budget 8 (`shadow-budget=256,64,8,8,16`), two visits | 19.1 / 19.0 | 2.62 / 2.62 | 74.2 |
| forest, no sun (`sun=0`) | 14.8 | 0.41 | 0.4 |
| forest, noon | 21.7 | 5.64 | 42.3 |
| forest, 720p | 15.2 | 4.29 | 33.1 |
| field walk, sun 4 deg | 16.6 | 3.94 | 29.2 |
| field walk, `RENDERLAB_DIAG_ALL_OPAQUE=1` | 16.6 | 3.93 | 29.3 |

- The pass is almost all sun-shadow rays: 0.29 ms per direction per pixel on the forest
  (9.56 at 32, 2.62 at 8, 74 at 256), and 0.3-0.4 ms for camera rays, receivers and repair
  (no sun: 0.41). Rasterising primary visibility on RT (step 4) would save at most that.
- It scales with pixels (720p 4.29 against 1080p 9.56, 2.23x for 2.25x the pixels), and a
  low sun costs 1.7x noon (longer, grazing rays through foliage and over terrain).
- `ALL_OPAQUE` changes nothing (field walk 3.94 against 3.93), as expected on a shadowed frame.

**Step 3, cold probes** (same binary, `RENDERLAB_COLD_PROBES` 1 against 0, interleaved twice):
forest cut frame `visibility` 74.1 -> 10.3 ms and frame span 88.2 -> 24.5 ms (a moving frame
is 9.6 / 24.3); moving frames unchanged (9.57 / 9.56). Picture (`pictures` plan, every frame
written; tools/rtvis/compare.py, tools/blobs.py):
- Non-RT: byte-identical, all 40 frames of the game scene's cut clip.
- RT: frame 0 (also cold) and the cut frame differ from main by a mean of 0.001-0.002 codes;
  at most 138-450 pixels move more than 8 codes on frame 0 and on the forest's cut frame,
  fading within about 15 frames. Against the settled image of the same pose (the forest held
  still, frame 59), the cut frame's mean error is 0.732 with the stop and 0.731 without, and
  5.695 against 5.680 on edge pixels; frames 25-30 agree as closely. tools/blobs.py counts
  the same blobs per frame on and off (frame 0: 10714 px on, 11144 off).
- Native-resolution crops of the most-changed tiles (on | off | settled) show no visible
  difference: D:/Coding/renderlab-work/unit-rtvis/crops.

## Non-RT `atmosphere` on the fixtures (2026-11-12)
Outpost spends 6.8 ms of a 100 ms non-RT frame in `atmosphere` and Showcase 2.5 ms. The
fixtures spend 0.3 ms. With the stage now split under `RENDERLAB_PASS_LOG_DETAIL` (sky LUTs and
environment stay in `atmosphere`; `.aerial`, `.inject`, `.temporal`, `.integrate`; one pass a
part, byte-identical), medians in ms:

| non-RT, sun 4 deg, fog on | sky | aerial | inject | temporal | integrate | cpu.wait |
|---|---|---|---|---|---|---|
| forest walk, frames pipelined | 0.09 | 0.04 | 0.15 | 0.03 | 0.03 | 0.04 |
| forest walk, synchronous (`--every 1`) | 0.16 | 0.07 | 0.39 | 0.08 | 0.13 | 0.25 |
| game flight, pipelined | 0.09 | 0.04 | 0.13 | 0.03 | 0.02 | 0.02 |

Not reproduced, and the obvious scene-size causes are ruled out on the fixtures. The procedural
environment's key holds the camera's altitude, so a camera that changes height rewrites the cube
chain every frame, but the game flight's descent does exactly that at 0.3 ms. The froxel grid
follows the resolution, not the scene. The new `cpu.wait` part marks the end of the early
command buffer: before it, the GPU's wait for the CPU's GI and post preparation between the two
submits was counted in `atmosphere` on non-RT, and in `visibility` on RT. game_scene keeps frames
in flight, so it never waits there, but an embedder that captures or paces each frame can. What
to look at next is one Outpost capture with `RENDERLAB_PASS_LOG_DETAIL=1`: `cpu.wait` large means
the CPU's GI preparation (cheaper per-frame instance records), sky large means LUTs or
environment rebuilt each frame (quantise the key), inject large means the fog's sun lookups.

## Shading ablations on the fixtures (`RENDERLAB_ABLATE`, 2026-11-12)
Shading stage medians, ms, frames 8-39, one visit each (frame span in brackets):

| | base | reflections off | local_shadows off | gi off |
|---|---|---|---|---|
| game flight, RT | 9.2 (21.6) | 4.1 (16.4) | 5.2 (17.5) | 7.1 (13.2) |
| game flight, non-RT | 18.6 (34.5) | 6.9 (22.9) | 7.8 (23.7) | 18.0 (22.6) |
| forest walk, RT | 3.7 (21.1) | 3.7 (21.2) | 3.4 (20.8) | 1.9 (11.0) |
| forest walk, non-RT | 4.2 (38.5) | 4.2 (38.6) | 2.7 (36.7) | 3.4 (11.4; 5 timed frames) |

The game flight stands in for a reflective, lamp-lit scene: on non-RT, reflections and local
shadows each take 11-12 ms of its 18.6 ms of shading. The two do not add up, so they share
work (reflection hits shade with local shadows). The forest walk has no mirrors or lamps in
view and is not a shading stand-in.
