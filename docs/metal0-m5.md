# S4a metal=0 on the M5 (Showcase, 2026-11-13)

Build: Ara main b50edafbd (cap 4) with Renderlab t3/hit-reuse-trace2 7a082d1, port RT path, 1280x720,
`--ev100 10` (exposure locked). These are inline rows: the compacted pass does not start on Metal.
`RENDERLAB_RL_DEFERRED=1` faults with "ARA_GPU_CONTRACT: Readback still in flight" (RL#79's port
readback of 62/63), and `RL_PASS=1` alone runs the inline path.

## Share, ms and pictures

- **Share:** the reused share of the hits' luminance, from mask/all/none at locked exposure. Band =
  receivers 0.35-0.5, taken as the full run minus a `RENDERLAB_REFLECTION_ROUGH=0.35` run (the band
  holds 59% of the reflected luminance).
- **Shading ms:** three interleaved rounds. Reuse off is 12.38; glossy reflections off
  (REFLECTION_ROUGH=0) is 10.08, so glossy reflections cost 2.30 here.
- **Crops:** moving camera (`--video 33 --video-yaw 3`), frames 30-32, at 1:1. "vs off" is the mean
  and max difference against reuse off at the same frame. Frame-to-frame is the mean change from
  frame 30 to 31 and 31 to 32; reuse off's own is chrome 5.87, copper 5.82, gold 6.61,
  patterned 7.47.

| Tune | Share all | Share <0.35 | Share 0.35-0.5 | Shading ms | Chrome vs off | Copper vs off | Gold vs off | Patterned vs off | Frame-to-frame (chrome / copper / gold) |
|---|---|---|---|---|---|---|---|---|---|
| reuse off | - | - | - | 12.38 | - | - | - | - | 5.87 / 5.82 / 6.61 |
| REUSE=1 (default) | 0.104 | 0.082 | 0.119 | 12.53 (+0.15) | 0.07 / 17 | 0.05 / 9 | 0.05 / 8 | 0.01 / 9 | 5.86 / 5.81 / 6.61 |
| footprint=2 | 0.123 | 0.096 | 0.142 | 12.48 (+0.10) | 0.11 / 17 | 0.10 / 9 | 0.07 / 8 | 0.02 / 9 | 5.86 / 5.81 / 6.61 |
| metal=0 | 0.204 | 0.171 | 0.227 | 12.39 (+0.01) | 0.17 / 19 | 0.13 / 14 | 0.19 / 19 | 0.02 / 9 | 5.86 / 5.81 / 6.61 |
| footprint=2,metal=0 | 0.659 | 0.718 | 0.619 | 12.13 (-0.25) | 1.36 / 59 | 1.94 / 45 | 2.21 / 68 | 0.10 / 9 | 5.92 / 5.80 / 6.60 |
| footprint=4,metal=0 | 0.666 | 0.731 | 0.622 | 12.07 (-0.31) | 1.36 / 59 | 1.94 / 45 | 2.21 / 68 | 0.10 / 9 | 5.92 / 5.80 / 6.60 |

## What can be seen

- **Up to metal=0:** nothing I can see. The max is at most 19 codes and the mean at most 0.19.
- **footprint=2,metal=0 (and 4):** the first row with a visible change, on all three metal balls.
  - The lower halves, which reflect the floor and the stage, look flatter: less of the floor's
    mottling.
  - Mean brightness there barely moves (chrome -3%, copper -1%, gold +3%).
  - The tops and highlights are unchanged (ratio 1.000).
  - The difference heat map also shows thin rim arcs at the chrome and gold silhouettes.
  - No lag or smear: frame-to-frame change equals reuse off's to within 0.05.
  - No doubled edge, and no tinted or lagging highlight, that I can see.
  - The patterned (dielectric) ball is untouched.
- **What it is:** a steady change in the reflected floor, not a temporal artefact. It is either the
  reused floor carrying the camera's resolved view (its own second bounce, its GI occlusion, and the
  camera's texture filtering instead of the hit's cone mip), or a real error. Telling them apart
  needs the still reference (today | reuse | path-traced). These numbers cannot decide it.
- **Cost:** inline on the port, the reuse that pays (-0.25 / -0.31 ms of 2.30) needs metals in.
  Divergence holds the inline gain to about 11% of the reflection cost even at a 66% share. The
  compacted path removes reused hits before the hit kernel, which is where the share turns into ms.
