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
