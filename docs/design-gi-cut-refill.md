# GI cache: refilling after a camera cut

Root, 2026-11-12. The third item of the frame-cost backlog (docs/m7-gaps.md, "One camera cut on Outpost").

## What a cut costs today
Ara's single-cut Outpost capture (`cut1`, RTX 4070 SUPER, 1080p, Renderlab 5955d4a) on the non-RT path, ms per frame:

| | before | cut | +1 | +2 | +5 |
|---|---|---|---|---|---|
| frame | 90.2 | 265.3 | 230.8 | 175.0 | 128.4 |
| gi_update | 3.2 | 144.5 | 64.0 | 27.4 | 9.4 |
| gi_gather | 35 | 35 | 76 | 59 | 38 |

193k cells are created on the cut frame, at about 0.75 µs each in `gi_update` (each new cell's first sky term, on software rays). The gathers for those cells follow over the next frames. The picture is clean, with a little extra fine grain for about five frames.

## What already exists, and why it does not run
G2b (docs/m7-g2b.md) built the machinery for this, for a cleared table:
- `claim_cells`: a per-frame budget of new cells (`CLAIM_CELLS_SOFTWARE` 20,480; `CLAIM_CELLS_HARDWARE` 49,152). A frame creates one fixed key phase of the missing cells, and the rest wait.
- `COARSE_CUT_FRAMES` (4): during those frames pixels read cells one level coarser, so a quarter of the cells covers the view within the budget. The finer cells that follow fall back to the coarse ones until they exist.
- `cut_phases` (4 on software rays): a new cell traces one group of its fixed directions a frame, so its first sky term costs a quarter of its rays per frame and completes by the fourth.

Two things keep it off on a camera cut:
1. The window is keyed to `since_clear`, frames since the table was emptied. A camera cut through `Engine::cut_view` keeps the table since RL#39, so no window opens.
2. `claim_cells` defaults to u32::MAX (uncapped): on the SV test scene the coarse-to-fine handover settled more slowly than it saved.

## The change (main, behind `IndirectSettings::cut_refill`, off)
- `Planner::camera_cut`, called from `Engine::cut_view` through `Renderer::mark_indirect_cut`, starts a `since_cut` count.
- With `cut_refill` on, the refill window runs on `min(since_clear, since_cut)`. For COARSE_CUT_FRAMES frames after a cut, pixels read one level coarser, and claims are capped at the per-path budget even when `claim_cells` is uncapped. For the cut-phase window, new cells trace one direction group a frame, and partially traced cells re-queue.
- Off, a cut changes no header word (unit test `a_camera_cut_opens_the_refill_window_only_with_cut_refill`).
- `RENDERLAB_GI="cut_refill=1"` turns it on through any embedder.

Expected on the Outpost cut, from the costs above: gi_update on the cut frame bounded by the claim budget and a quarter of the directions, roughly 145 ms down to about 5-10 ms, with the remaining cells over the next frames. The coarse cells answer the pixels meanwhile.

## What has to be shown before it becomes the default
- **Picture:** the cut frame and the next ten against today, at native resolution, both paths, on Outpost through Ara (`RENDERLAB_GI="cut_refill=1"`) and on the game scene's field walk with `--diag cut=every`. The bar is no more grain than today's five frames, no blocks, and no visible coarse-to-fine step. G2b saw the handover settle more slowly on SV, so judge the settle by frame as well as the first frame.
- **Cost:** the frame and `gi_update`/`gi_gather` per frame for the ten frames after the cut, the switch off against on, same binary.
- **Cuts in Ara:** this only helps if Ara calls `cut_view` when its camera cuts. Confirm that it does. If it does not, add a camera-jump detector (the eye moved more than a few metres, or the view turned more than about 30 degrees, between frames) that calls the same `camera_cut`.

## Not in this change
- Seeding new cells from the surface cache: kept as the fallback if the coarse level shows.
- The RT cut frame (visibility 13.6 -> 62.3 ms): that is the sun-shadow restart, U-RTVIS's step 3.

## Measured (U-RTVIS, 2026-11-12): not ready as a default
RTX 4070 SUPER, 1080p, release game_scene at main 776dcd0f, `RENDERLAB_GI="cut_refill=0|1"`, same
binary, two interleaved visits (one on RT). Cuts: the game flight's orbit jump (`--diag cut=1`)
and the forest walk turned 150 degrees in place (`--diag forest=1 --diag cut=turn`, new), both
through `Engine::cut_view` at frame 24 (game_scene does call it). Tools: tools/rtvis/timing.py
`refill`/`refillpics`/`gi`, tools/rtvis/compare.py `refill`, tools/blobs.py.

`gi_update`, ms, from the frame before the cut (23) to cut+5; both visits agreed within 10 %:

| | 23 | cut | +1 | +2 | +3 | +4 | +5 |
|---|---|---|---|---|---|---|---|
| non-RT forest, off | 54 | **1016** | 301 | 210 | 187 | 190 | 184 |
| non-RT forest, on | 55 | 72 | 74 | 93 | 114 | **1307-1471** | 236 |
| non-RT game, off | 4.1 | **8.4** | 4.8 | 4.7 | 4.7 | 4.9 | 4.8 |
| non-RT game, on | 4.1 | 4.2 | 4.0 | 4.0 | 4.0 | **8.3** | 4.9 |
| RT game, off | 2.5 | 3.7 | 2.9 | 2.9 | 2.9 | 2.9 | 2.9 |
| RT game, on | 2.5 | 2.7 | 2.6 | 2.5 | 2.5 | 3.8 | 3.0 |

- **Cost: the spike moves, it does not shrink.** The claim cap and the cut phases apply only
  while `since_event < COARSE_CUT_FRAMES` (gi_cache.rs `cut_capped`, `spread`); at cut+4 the cap
  returns to uncapped and every missing fine cell is created and fully traced in one frame. On
  the forest that frame is larger than today's cut frame (1307-1471 against 1016 ms).
- **Picture: blocks.** During the coarse frames the coarse cells show as rectangles on lit flat
  faces: the game scene's arch lintel at cut+1, the forest's wall at cut+2, a blockier mottle on
  the forest ground (crops: D:/Coding/renderlab-work/unit-rtvis/crops-refill, left on, right off).
  blobs.py on the non-RT forest: cut frame 17,107 blob px on against 12,808 off; frame 0, where
  the first frame's refill window also applies, 51,071 against 11,820. The game scene is even.
  After the handover (cut+4 on) the frames match off closely.
- **Suggested next step:** keep the claim cap and the per-cell direction spread until the
  backlog after the cut drains (a frame whose claims stayed under the cap), not for a fixed four
  frames, and answer pixels whose fine cell is missing from something finer than one coarser
  level (the surface-cache seed listed under "Not in this change", or the cut frame's own
  neighbouring cells), since the coarse level is what shows as blocks.

## Rework (U-RTVIS, 2026-11-12): cap until drained, no coarse read. Not ready as a default.
Changed behind `cut_refill` (still off):
- **Claim cap:** published for up to `CUT_REFILL_FRAMES` (32) frames after a cut or clear instead
  of 4. `gi_claim_period` already stops deferring once a frame's misses fit under the cap, so the
  cap ends when the backlog drains; the 32 frames bound a scene that never drains.
- **Cut phases:** the direction spread carries `SPREAD_WHILE_DEFERRING` (0x100 in `GI_H_SPREAD`).
  `gi_setup_directions` spreads a new cell's directions only on frames whose claims were split,
  and partial cells re-queue through the window and one frame past it.
- **No coarse read:** pixels read their own level; a missing cell takes the lookup's fallbacks.
  An explicit `claim_cells` keeps G2b's coarse start.
- **Off is main:** with `cut_refill` off, every header word is main's.

**Cost** (main + top-level BVH, RTX 4070 SUPER, 1080p, same binary, two interleaved visits): the
forest `cut=turn` non-RT cut frame goes from 94-106 ms to 52.6 ms. Its gi_update goes from
44.7 to 3.4 ms, then stays at 6-10 ms over +2..+8 (off: 9-11), with no later spike. The game
jump non-RT cut-frame gi_update goes from 8.8 to 4.4-4.6 ms, at most 7.5 on a later frame. RT
changes little (forest cut gi_update 5.0 to 2.6). One visit's first refill frame showed 381 ms
of `cpu.wait`: a first-use shader compile, gone on the second visit.

**Picture, the bar not met on non-RT:** still clips (camera frozen, the cut turns or jumps the
view, frame 59 of the off run as the settled image). Mean error against it, on / off (whole
image; edge pixels):

| | cut | +2 | +4 | +8 |
|---|---|---|---|---|
| non-RT forest | 2.86 / 3.68; 9.24 / 9.03 | 2.65 / 2.94; 7.19 / 6.51 | 2.65 / 2.47; 6.44 / 5.43 | 2.37 / 1.78; 4.86 / 3.62 |
| non-RT game | 1.71 / 1.71 | 0.67 / 0.45 | 0.59 / 0.33 | 0.39 / 0.20 |
| RT forest | 5.07 / 6.22 | 4.40 / 5.00 | 3.49 / 4.18 | 2.52 / 2.96 |
| RT game | 1.70 / 1.70 | 0.46 / 0.48 | 0.35 / 0.36 | 0.23 / 0.23 |

- **RT:** as good or better everywhere.
- **Non-RT:** converges more slowly. The forest wall shows dark cell-sized blotches with the
  refill on that persist past +8 (crops: D:/Coding/renderlab-work/unit-rtvis/crops-rf4,
  nonrt-forest-still-centre.png, on | off | settled). blobs.py runs a few percent higher with it
  on over +1..+8.
- **Cause:** new cells arrive over several frames with partial direction sets. A cell's first
  sky term is then noisier, and its bounce gathers start later than when every cell was created
  and fully traced on the cut frame.

**Next options:**
1. RT only: the refill passes there.
2. Non-RT without the phase spread: claims capped, but each new cell traces all its directions at
   once, since the top level made a full trace cheap.
3. A larger non-RT cap, so the backlog drains in about two frames.
