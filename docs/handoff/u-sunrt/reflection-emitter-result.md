# Desktop emitter reflection route: measured result

The existing own-program route restores a large parameter-only emitter on both RT and non-RT. No renderer implementation, build, PR or default changed. Own-program remains off: small bright emitters have a separate speck defect under investigation, including in today's inline default. The pass's S1 flip does not include own-program.

## Picture and route

Today and pass-on fallback reflect the panel at mean RGB about **42/40/42**. Pass + own-program gives **251/133/116**; the direct panel stays **252/136/120** in all six held rows. The sampled cut stays bright on both paths. GI is ablated, bloom is off, EV10 fixed; this proves the missing program/parameter evaluation, not an absolute path-traced brightness match.

```powershell
$env:RENDERLAB_RL_PASS='1'
$env:RENDERLAB_RL_DEFERRED='1'
$env:RENDERLAB_RL_HIT_OWN_PROGRAM='1'
```

The receiver must enter the deferred composite and the hit must reach its per-program hit shader. The fallback uses receiver code/parameters; own-program loads the hit's code/parameters. Merely setting OWN_PROGRAM while the pass is off does nothing.

Gaps: custom shaders without the deferred reporting hook remain inline (the default game/toon flight is such a control); mirrors/floored/overflow receivers that remain inline keep that route's fallback. Texture-streaming frames skip hit shading. Cache-terminated hits bypass full hit shading. The Apple port does not dispatch this pass and is unchanged; its compose_list fallback needs a separate item. Bindings 64/65, H layout and non-list compose_hits are unchanged.

## Cost over pass-on fallback

RTX 4070 SUPER / NVIDIA 617.14, release binaries, 1280x720. Each value is the mean of two fully reversed-round visit medians, 72 warm tail frames per visit. Timings use exclusive n8 --timing leases; both report no games, foreign GPU processes, builds or contended screen. This measures own-program over fallback, not enabling the pass over today.

| Clip / path | Full GPU frame fallback → own (ms) | Change (ms) | Hit shader fallback → own (ms) |
| --- | ---: | ---: | ---: |
| emitter / rt | 1.4909 → 1.4920 | +0.0011 | 0.0492 → 0.0512 |
| emitter / nonrt | 1.7283 → 1.7308 | +0.0025 | 0.0604 → 0.0666 |
| flight / rt | 10.5300 → 10.5531 | +0.0230 | 0.0635 → 0.0942 |
| flight / nonrt | 18.0659 → 18.1199 | +0.0540 | 0.2652 → 0.2975 |

| Clip / path | Trace fallback → own (ms) | Composite fallback → own (ms) | Reflection total fallback → own (ms) |
| --- | ---: | ---: | ---: |
| emitter / rt | 0.1664 → 0.1664 | 0.2550 → 0.2552 | 0.4706 → 0.4741 |
| emitter / nonrt | 0.6756 → 0.6771 | 0.0911 → 0.0917 | 0.8271 → 0.8350 |
| flight / rt | 0.0492 → 0.0492 | 0.0092 → 0.0092 | 0.1214 → 0.1513 |
| flight / nonrt | 0.3064 → 0.3110 | 0.0092 → 0.0092 | 0.5790 → 0.6175 |

Full wall medians (ms), fallback → own: emitter RT 2.0933 → 2.0948, non-RT 2.9118 → 2.8970 (Ara wall includes bridge/handoff); flight RT 11.2289 → 11.2992, non-RT 19.6982 → 19.9125 (drained render + sun update). Do not compare these two wall definitions across clips.

Fixture: P201, Ara 2f9eff09 / Renderlab faccf9b6, GI ablated. Flight: retained 97700fee binary, SHA256 500ee3c90f1bc3ece7dad619c662ec50059747eaa9356667ec884a8187d7072d, GI on, --diag program=standard so its materials report to the composite, plus the toon hero. Reflection production files equal faccf9b6; the sole program.rs change is inactive LVIS record indexing. This standard-route diagnostic is not the authored custom flight picture. The first eight custom-program flight visits were inactive fallback controls and are excluded from this table.

Leases: u-sunrt-emitter-timing-1114-033037, 62.94 s (valid fixture rows); u-sunrt-emitter-flight-active-1114-033621, 36.24 s (corrected active flight). Both exit 0. Initial fixture lease clock 210/405 MHz P8 → 1005/5001 P3; active flight 255/810 P5 → 2820/10501 P0. Paired ratios use the same load. Raw rounds/stages: [cost-summary.json](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/cost-summary.json).

## Moving comparison and crop files

**bin-P201, pre-#124; #124 affects only grazing horizon receivers, not the reflected panel** (U-RTVIS source assessment). No retake on a #124 binary was made; these artifacts establish emitter gain only. Every labelled pair and movie carries that caveat.

- [rt held crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-routes-1114-032121/crop-pairs/rt-held-today-vs-own-pre124-labelled.png)
- [rt cut crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-routes-1114-032121/crop-pairs/rt-cut-today-vs-own-pre124-labelled.png)
- [rt moving full](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-moving-1114-034128/comparisons/rt-full-off-vs-own-pre124.mp4)
- [rt moving emitter-crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-moving-1114-034128/comparisons/rt-emitter-crop-off-vs-own-pre124.mp4)
- [nonrt held crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-routes-1114-032121/crop-pairs/nonrt-held-today-vs-own-pre124-labelled.png)
- [nonrt cut crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-routes-1114-032121/crop-pairs/nonrt-cut-today-vs-own-pre124-labelled.png)
- [nonrt moving full](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-moving-1114-034128/comparisons/nonrt-full-off-vs-own-pre124.mp4)
- [nonrt moving emitter-crop](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/runs/u-sunrt-emitter-moving-1114-034128/comparisons/nonrt-emitter-crop-off-vs-own-pre124.mp4)

Movies contain 40 rendered frames at 60 Hz, presented at 12 fps (5x slow), with no interpolation. At frame 39, both paths have **30,271 pixels over 8 codes**, maximum 235, and **zero over-8 pixels in the direct-panel interior**. Reflected panel means off → on are 41.6/39.0/40.9 → 251.0/133.3/116.8. This is the intended restored reflection, not a flicker count. Ordinary moving lease 25.08 s, 4/4 rows exit 0.

Scene: [emitter-reflection.scene](D:/Coding/renderlab-work/u-sunrt/emitter-reflections/project/content/levels/emitter-reflection.scene), SHA256 3eba913a0e4265bd5149a53e1fd50b61ff8f4c18c812ebfaa145e60c5ea73ad3. P201 player SHA256 5abfa54ff3e50c7b1a68f57784aa8414c4635f5433cc047f57254a9cf2a33547. Owned minimal project, no scripts/assets needed. CPU Ara check: 0 errors / 0 warnings. No path-traced reference was built, per root's direction.
