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

## Reflection hits: where their shading goes, and the stratum split (2026-11-12)
A reflection ray that hits runs the hit's program (`surface`, `shade`) in full. Glossy pixels
trace 4-16 strata, mirrors one. In `shade`'s light loop (`light_visibility`, shaders/program_api.wgsl)
shadowed local lights go to `rl_local_shadow` exactly as at a camera receiver: on RT at least 4
disk-sampled rays a light, on non-RT the cached atlas map's PCSS (12 blocker taps, then 24
bilinear PCF taps, about 108 texel reads). The sun gets one hard ray (`rl_hit_shadow`, software
`rs_blocked` on non-RT). GI at hits reads the open environment, with no cache lookup.

Split with `RENDERLAB_RL_HIT` (measurement only, reflection hits only; src/program.rs). Game
flight with the Fox, 1080p, frames 8-59, `shading` medians in ms, non-RT two visits, RT one:

| switch at hits | non-RT shading | RT shading |
|---|---|---|
| none (main) | 21.7-21.8 | 10.95 |
| `local`: shadowed local lights unshadowed | 13.5 | 7.75 |
| `sun`: no sun shadow ray | 19.2 | 9.50 |
| `nolights`: every light zero incidence | 11.0 | 6.60 |
| `black`: no `surface`/`shade` | 10.3 | 5.10 |
| reflections off (`RENDERLAB_ABLATE`) | 7.1 | 4.86 |

On non-RT, reflections cost 14.7 ms: 3.2 tracing and hit setup, 11.5 hit shading. Of the hit
shading, 10.8 is the lights, almost all their visibility: 8.3 local-light shadow maps and 2.6
the sun's ray. RT has the same shape at half the size.

The change, `RENDERLAB_RL_SPLIT` (on by default; `=0` restores every sample at every hit):
a glossy pixel's strata are averaged into one value. So stratum k of n evaluates every n-th
local-light disk ray (RT) or PCF tap (non-RT PCSS), from an offset that turns with the frame.
Together the strata cover each sample about once instead of each repeating all of them. One
stratum (mirrors) and camera receivers take every sample, exactly as before. The blocker
search, which sets the penumbra's width, stays whole.

Other options and their picture risk against the path tracer:
- Skipping local shadows on rough lobes leaks lamp light into reflected shadows.
- Shading reflection hits once per tile blurs reflections across pixels.
- Caching hit shading in the GI cells loses reflected detail.
- Sharing the primary hit's light cluster leaves visibility, the cost, untouched.

Cost (same binary, interleaved twice, reference 25.80 ms against 26.7): game flight `shading`,
non-RT 21.86 / 21.99 off against 15.67 / 15.85 on (-6.2 ms, frame 41.8 to 35.6); RT 10.87 /
10.78 against 9.89 / 10.41.

Picture:
- reflection_matched with its arch lamp (`--lamp 1`), both paths, moving (frames 16, 32, 47)
  and still (`--freeze 32`, frame 47): split on against off differs by at most 1-4 codes on 0-2
  pixels a frame. The error against the path tracer is the same on and off: RT moving 0.837 /
  1.001 / 1.183, still 0.976; non-RT 0.848 / 1.006 / 1.189, still 1.078.
- Game flight and game scene with the camera held (`hold=1`), every frame of 48, both paths:
  at most 24 (non-RT) and 62 (RT) pixels a frame differ by more than 2 codes. Over 48 frames
  1-20 pixels differ by more than 8. The largest difference is 15.
- Crops (D:/Coding/renderlab-work/unit-rtvis/crops-splitgame, on | off | difference x16): the
  chrome mirror sphere and the lamps differ by at most 1 code; the gold sphere by at most 9 codes
  on 16-17 pixels near its highlight, not visible side by side.
- blobs.py: the same counts on and off on every clip.

## Non-RT local soft shadows: the filter taps (2026-11-12)
`rl_local_shadow_map`'s soft shadow (shaders/light_cluster_api.wgsl) runs a 12-tap blocker search,
then 24 bilinear PCF taps of 4 texels each.

Footprints: the search radius is `source x (1/near - 1/d)` and the filter's `source x (1/blocker
- 1/d)` with blocker >= near, both capped at 96 texels. So the filter always lies inside the search
footprint. The search is 12 point samples, though, not a cover. Its existing fully-lit exit
(`blockers==0`) can miss a thin blocker between samples, and an umbra exit from the same samples
would be no more exact. An exact early-out needs a conservative depth bound over the filter's
whole footprint: a min/max mip chain of the shadow atlas, rebuilt when the cached maps update. A
filter whose texels all have stored depth above its largest reference is umbra; one whose texels
all lie below its smallest reference is lit. Cube-face edges would keep the full path. Not built.

Built instead (exact): a filter tap whose 2x2 texels lie inside its face compares them inline,
with the same receiver-plane reference and the same accumulation order, skipping
`rl_local_tap`'s per-call matrix, tile and cube-edge work. A first version read the four texels
with one `textureGather`; it differed by 1 code on 3-20 pixels from frame 25 of the game flight
(its corner coordinate can select the neighbouring block), so the shipped version loads each
texel at its exact coordinate.
`RENDERLAB_LOCAL_GATHER=0` restores the four calls; `RENDERLAB_LOCAL_PCF=skip` measures the
filter's share.

Non-RT, 60 frames, shading medians, ms (same binary, interleaved twice; reference run 28.05):

| | inline (new) | four calls (main) | filter skipped |
|---|---|---|---|
| game flight with the Fox | 16.6 / 17.3 | 17.6 / 20.6 | 13.6 |
| night lanterns (`--night-lanterns`) | 36.5 / 37.8 | 45.9 / 43.5 | 30.0 |

Every frame of both clips (40) is byte-identical with the inline compare on and off.

## The forest cut with the instance top level, cut_refill off and on (2026-11-12)
Non-RT, `--diag forest=1 --diag cut=turn`, the cut at frame 24, two visits each, ms (frame 25 was
untimed in three runs):

| | 23 | cut | +1 | +2 | +3 | +4 | +5 |
|---|---|---|---|---|---|---|---|
| off: frame | 33.6 / 37.2 | 94.9 / 99.6 | - | 63.8 / 62.8 | 66.2 / 68.8 | 65.9 / 63.6 | 67.3 / 62.6 |
| off: gi_update | 3.5 / 3.5 | 43.5 / 45.9 | - | 11.1 / 11.1 | 10.3 / 11.0 | 10.3 / 10.1 | 9.5 / 9.5 |
| on: frame | 33.8 / 35.1 | 55.4 / 53.2 | 62.8 / - | 65.5 / 61.9 | 65.8 / 60.4 | 127.8 / 114.6 | 68.8 / 73.7 |
| on: gi_update | 3.7 / 3.9 | 4.1 / 4.4 | 5.0 / - | 6.2 / 6.1 | 7.5 / 8.1 | 61.4 / 57.4 | 13.0 / 13.4 |

The turned view is heavier (about 63 ms against 34 before the cut, with or without the refill).
- **Off:** the cut costs about 30 ms over the view's own frame (43-46 ms of `gi_update`), down
  from about 1,300 ms before the top level.
- **On:** the cut frame is quiet, but cut+4 is larger than today's cut frame (57-61 ms of
  `gi_update`) because the cap ends after four frames.

The rework (cap until drained) would spread about 45 ms of cell creation over several frames. It
is still worth doing, at a smaller stake than before.

## Reflections: cost by band, and halving the glossy strata (2026-11-12, after Islam's priority change)
Game flight with the Fox and night lanterns (`--night-lanterns`), 1080p, frames 8-59, `shading`
medians in ms. One visit, reference run 28.6 (7 % high: compare within the table only):

| | base | strata <= 8 | strata <= 4 | traced to 0.3 | traced to 0.1 | reflections off |
|---|---|---|---|---|---|---|
| non-RT game | 25.3 | 16.3 | 11.2 | 15.7 | 6.6 | 5.6 |
| non-RT night | 54.1 | 42.6 | 22.5 | 25.8 | 13.6 | 12.5 |
| RT game | 10.5 | 9.3 | 8.2 | 8.6 | 5.9 | 5.2 |
| RT night | 18.6 | 16.5 | 13.9 | 15.3 | 11.9 | 11.9 |

Switches: `RENDERLAB_RL_HIT=strata=N` caps a glossy pixel's strata (RL_GLOSSY_MAX, 16);
`RENDERLAB_REFLECTION_ROUGH` replaces `max_roughness`.

- **Non-RT is where reflections cost:** about 20 ms of the game's shading and about 42 ms of the
  night lanterns', against 5-7 ms on RT. A non-RT stratum costs about four times an RT one: a
  software ray, the hit's software sun ray and its light loop.
- **The cost follows the strata count:** RQ1 raised the count to 12-16 because four strata
  smeared moving glossy metal.

Tried: `RENDERLAB_RL_HALF=1` (reflections.wgsl RL_HALF_STRATA, off). With reconstruction's
rotation, a glossy receiver of 8 or more strata traces every other stratum of its fixed set a
frame, so two frames cover the whole set with main's directions. Same binary, interleaved twice
(reference 28.03 ms):

| | half off | half on |
|---|---|---|
| non-RT game | 27.0 / 27.2 | 23.5 / 22.8 |
| non-RT night | 50.7 / 52.4 | 47.0 / 47.0 |
| RT game | 11.1 / 11.5 | 10.8 / 11.0 |
| RT night | 23.0 / 23.3 | 22.2 / 22.6 |

Against the path tracer (reflection_matched with its arch lamp, both paths), the whole-image
error barely moves (moving 0.840 / 1.003 / 1.188 on against 0.837 / 1.001 / 1.183 off). In the
most-changed tile, the gold sphere, it is 5-7 % higher while moving (frame 47: 10.52 against
9.79) and 4 % higher still. The reflected crate on the gold sphere is slightly smudgier in
motion (crops: D:/Coding/renderlab-work/unit-rtvis/crops-half, on | off | path tracer). Kept
off: about 4 ms for a visible softening on moving rough metal.

Next candidates, by the measured cost:
1. **Shared strata across a 2x2 quad**, where neighbours are the same surface and the lobe is
   wide. Each pixel traces a quarter and the quad averages. This needs the traced shading entry
   to keep uniform control flow up to a workgroup barrier, or a separate reflection pass, so it
   touches the shading entry's structure.
2. **Cheaper non-RT hits.** A non-RT stratum is about four times an RT one; its hit runs the
   software sun ray and the full light loop.
3. **Untraced estimate above a roughness** (traced to 0.3 saves about 9.6 ms of non-RT game
   shading). It needs a picture check against the path tracer for light leaking on rough metal.

## Reflections with history (2026-11-12, not merged)
A glossy receiver whose reprojected history is trusted traces one stratum of its fixed set a
frame, rotating from a per-pixel offset, and folds it into a per-pixel ping-pong record. The
record holds the mean, the mean-square luminance and the count, up to the set's count. It traces
the full set, as today, on:
- a cut, reset or moved/skinned placement;
- a restart sphere (`rl_restarts`);
- no matching record (placement and clip depth within 2%);
- a stratum whose luminance is more than 4 sigma plus 10% off the stored mean.

Mirrors are unchanged. The history is in shaders/reflections_history.wgsl, bound only in the wgpu
renderer's primary shading modules (group 0, bindings 53/54); film and Ara keep the stubs.
It is off by default; `RENDERLAB_RL_HISTORY=1` enables it, and `RENDERLAB_RL_HISTORY_TUNE`
(strata, cap, moving, sigma) sets the measurement constants.

**Cost** (same binary, interleaved twice; `shading` medians in ms):

| | off | history | never reject |
|---|---|---|---|
| game flight, non-RT | 14.7 | 13.0 | 12.6 |
| game held still, non-RT | 11.7 | 10.1 | 9.8 |
| game flight, RT | 9.8 | 9.1 | 8.9-9.7 |
| game held still, RT | 7.9 | 7.2 | 7.05 |
| Showcase-like band, non-RT, 720p moving | 13.4 | 5.8 (4 strata: 7.7) | |
| Showcase-like band, RT, 720p moving | 3.57 | 3.05 (4 strata: 3.4) | |

The Showcase-like band is reflection_matched `--stage --metal-balls`. The game glossy band is
small: about 10k pixels change, around the gold sphere.

**Picture, the gate not met:** moving glossy surfaces show grain against both the path tracer and
the history off. One stratum a frame folded over 8-16 frames is far fewer effective samples than
16 strata a frame under TR1.
- Error against the path tracer in the most-changed 64x64 tile (aluminium ball and turntable,
  frame 16), both paths: 3.1 off, 7.9 with one stratum a frame, 5.75 with four.
- Still frames are about equal.
- Crops: D:/Coding/renderlab-work/unit-rtvis/crops-hist2 (off | variants | path tracer) and
  crops-histpics (the game's gold sphere, on | off).

Getting the moving picture back needs spatial reuse across neighbours, which is
docs/design-reflection-pass.md's resolve.

### History while still, and the spatial resolve (2026-11-12, both off)
Moving frames with the history read only under a still camera are byte-identical to main:
- game flight (48 frames) and night lanterns (30 frames), both paths;
- reflection_matched `--stage --metal-balls` frames 16/32/47, both paths.

The lib suite passed with it on: 247 passed.

Still frames are slightly further from the path tracer than main. The three most-changed 64x64
tiles at the held pose (frame 47 of `--freeze 32`), RT:

| | main | history | history, cap 16 | resolve | resolve, cap 16 |
|---|---|---|---|---|---|
| still | 7.71 / 11.72 / 2.17 | 8.11 / 12.11 / 2.90 | 8.11 / 11.99 / 2.83 | 7.79 / 12.07 / 2.53 | 7.73 / 11.95 / 2.36 |
| moving, frame 16 | 3.12 / 4.37 / 6.41 | = main | = main | 6.64 / 7.16 / 9.11 | 8.19 / 7.76 / 9.93 |

Non-RT is the same within 0.15. The resolve (`RENDERLAB_RL_RESOLVE=1`) also reads the history
under motion. It averages each trusted pixel's new mean with its neighbours' previous means,
edge-aware and clamped.

Inline in the shading entry, a pixel can filter only the previous frame's neighbours, not this
frame's samples. With one stratum a frame, that is far from main's 16 strata under TR1 while
moving. Both are off by default. Moving glossy quality at a reduced ray count needs:
- this frame's samples filtered after they exist, which is the reflection pass's resolve
  (docs/design-reflection-pass.md);
- reprojection by the reflected point.

Crops: D:/Coding/renderlab-work/unit-rtvis/crops-gate2 (main | on | cap16 | res | res16 | path
tracer).

### rl_resolve as a pass, first screen (2026-11-12 22:00 UTC, off)
`RENDERLAB_RL_HISTORY=1 RENDERLAB_RL_RESOLVE_PASS=1`, b91d0a44 + ae471b4e + d940a6d7:
- A trusted glossy receiver shows the previous frame's resolved value, reprojected by the
  reflected point below roughness 0.3, and traces one rotated stratum inline.
- rl_resolve then filters this frame's samples (3x3/5x5 binomial, same placement and depth),
  clamps the history to 1.5 sigma and blends (cap 16).

It fails the gate, worse than today both still and moving (reflection_matched `--stage
--metal-balls`, three most-changed tiles against the path tracer):

| | today | resolve pass |
|---|---|---|
| RT moving 16 | 3.12 / 6.41 / 4.37 | 8.38 / 10.28 / 7.74 |
| RT moving 47 | 3.20 / 9.75 / 2.10 | 10.07 / 14.00 / 5.75 |
| RT still 47 | 7.71 / 3.13 / 2.80 | 9.22 / 4.31 / 3.89 |

Non-RT is the same within 0.1. The crops (D:/Coding/renderlab-work/unit-rtvis/crops-rp3,
today | resolve | path tracer) show two faults:
- **Gold sphere (0.25):** blotchy, mottled reflected patches. One sample per pixel a frame,
  filtered over 3x3 and clamped loosely, leaves stale or misplaced history where the
  reflected-point reprojection lands on the curved surface.
- **Turntable (0.35) and aluminium ball (0.42):** grain.

Cost (Showcase-like scene, 1080p moving, `shading` medians; that lease's reference run read
39.6 ms against 26.7, so these are relative only):

| | today | resolve pass |
|---|---|---|
| non-RT shading | 13.5 | 5.45 |
| RT shading | 3.85 | 2.82 |

Needed before it can hold:
- more samples for rough lobes (4 strata a frame or more);
- a wider, variance-guided a-trous filter (2-3 iterations, radius from the lobe's screen
  footprint);
- a tighter clamp, with the per-pixel temporal variance kept;
- the displayed value taken after this frame's resolve, not the previous one: shading reads the
  resolved specular in a second pass, or the lobe pre-pass.
