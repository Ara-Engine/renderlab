# Software screen reflections: round three

Rejected. The prototype was built, pushed, and run on all three scenes and both paths under an exclusive GPU lease. It missed the non-RT performance gate on every scene. Runtime changes were reverted; the branch retains this report only over main 9b41cedd. The measured prototype was built from c9292150; the later sun-shadow merge was brought into the reverted branch after the run. The existing separate reflection pass remains the default without ray hardware.

## 1. Branch and reproduction

- Branch: `t3/swrefl-3`.
- Measured prototype: `0d8fce793a86047b641f2c99bd951ff8a1032026` (pushed).
- Runtime revert: `8a2f0a37fb6f6091a7b9af4ada2b90f31c193be2` (pushed).
- Prototype off switch: `RENDERLAB_SW_REFLECTION_SCREEN=0`. It is absent from the reverted runtime.
- Default yardstick selected by smoke: `bin-smoke-6a9b3d1a`. No `--baseline` override.
- RTX 4070 SUPER, 1920x1080, Ara smoke defaults, foreground game closed, `--exclusive`, all six rows rendered with zero errors. This is a rejected, counter-instrumented experiment, not a shipping speed claim.

The revision was built with `smoke.py ... --owner swrefl --rebuild --min-mb 8000 --exclusive`. After lease starvation, `fast-queue.py` ran the already-built immutable binary through the same smoke harness, with a 50 ms lock retry. Locks, memory floors, exclusive build exclusion, scene runs, and timing calculations were unchanged. The harness released the GPU after 338 seconds; that invocation took 390 seconds including its wait. Earlier ordinary polling waited fifteen minutes while newer requests acquired the lease.

## 2. Measurements

Main -> prototype. Frame figures are smoke's first-view window for Showcase/Outpost and course median for Glint. Reflection totals are medians of the summed trace, hit-shading and composite GPU intervals over matching steady windows.

| Scene / path | Frame ms | FPS | Reflection GPU ms |
|---|---:|---:|---:|
| Showcase RT | 5.94 -> 5.86 | 168 -> 171 | 0.205 -> 0.196 |
| Outpost RT | 10.77 -> 13.64 | 93 -> 73 | 0.132 -> 0.131 |
| Glint RT | 7.40 -> 7.50 | 135 -> 133 | 0.117 -> 0.117 |
| Showcase non-RT | 7.11 -> 7.51 | 141 -> 133 | 1.141 -> 1.608 |
| Outpost non-RT | 12.91 -> 13.53 | 77 -> 74 | 0.881 -> 1.380 |
| Glint non-RT | 9.41 -> 9.83 | 106 -> 102 | 0.741 -> 1.057 |

Glint goal, non-RT: 12.33 -> 13.17 ms (81 -> 76 FPS). All three non-RT reflection targets (0.6 / 0.5 / 0.5 ms) were missed.

The candidate had `RENDERLAB_RL_ROOM_LOG` enabled to collect acceptance counters. This also runs the existing CPU allocator-report logging, so these frame figures include instrumentation overhead. Outpost RT has an unrelated `after_fog` interval outlier, 1.110 -> 3.429 ms, while its reflection GPU time is flat. SSR source and resources are disabled on RT. That full-frame RT loss is not attributed to SSR and was not investigated after rejecting the experiment. No clean, uninstrumented speed claim is made.

| Non-RT scene | Trace + depth + march ms | Hit shading ms | Composite + history ms |
|---|---:|---:|---:|
| Showcase | 0.4879 -> 0.8438 | 0.6226 -> 0.6318 | 0.0307 -> 0.1403 |
| Outpost | 0.2913 -> 0.6241 | 0.5760 -> 0.5862 | 0.0092 -> 0.1679 |
| Glint | 0.4030 -> 0.6088 | 0.3036 -> 0.3036 | 0.0082 -> 0.1126 |

The march ran in its own dispatch so its registers did not burden fallback BVH traversal. It used an alternating depth/surface hierarchy built from exact primary records, a maximum of 40 hierarchical cell visits, exact triangle refinement, and previous-frame HDR colour with mips. Uncertain, off-screen and invalid-history rays retained the full original tree trace. The colour snapshot preceded TR1; reprojection checked surface identity and depth, actual hit motion, relighting/shadow zones, view dependence and fog, with exposure conversion. No ray-count reduction was used.

Screen history was active, but accepted almost no hits:

| Non-RT scene | Rays attempted, median | Screen intersections, median | History accepted, median | Accepted fraction |
|---|---:|---:|---:|---:|
| Showcase | 248963.0 | 20026.5 | 12.5 | 0.0045% |
| Outpost | 27477.5 | 1484.5 | 0.0 | 0.0000% |
| Glint | 37799.5 | 1983.0 | 0.0 | 0.0000% |

This explains the unchanged hit-shading cost. The march and history preparation added work while almost every ray still took the tree. The individual history-rejection reasons were not instrumented; these numbers establish a miss for this implementation, not that screen-space reuse cannot win. Early versions were corrected for blanket invalidation by per-frame skin uploads and for loss of object identity at internal triangle boundaries; the final figures above include both corrections.

No shading-kernel edit was made. The prototype's only `src/renderer.rs` edit passed the camera into the reflection-pass frame writer. All runtime edits, including that wiring and the new shader/module, are reverted.

## 3. Inspected pictures

Run directory:
`D:/Coding/renderlab-work/ara-integration/smoke/20261115-040742-bin-smoke-0d8fce79/`

Inspected base and candidate on both paths: Showcase and Outpost `*-frame40.png`, Glint `goal.live.png`; additionally Glint non-RT `sliders.live.png` and Showcase non-RT `.live.png`. No obvious new missing or dark reflections were visible in these frames. This is weak quality evidence because history reuse was negligible.

Native crop pairs and overview pairs, preserved under `D:/Coding/renderlab-work/main-thread/swrefl/third/`:

- `showcase-balls-still-pair.png` and `showcase-balls-still-pair-stack.png`: polished balls, still frame.
- `showcase-balls-frame40-pair.png` and `showcase-balls-frame40-pair-stack.png`: the balls during camera movement.
- `glint-rings-still-pair.png`: chrome-ball ring reflection and lit rings/platform.
- `rt-pairs.png`, `nort-pairs.png`: all three scenes, base beside candidate.

Two consecutive moving frames and the frame after a cut were not captured or certified. The experiment was stopped on performance as instructed, so the inspection above does not certify motion, edge switching or disocclusion quality.

Raw logs and summaries: `final-fast.log`, `final-passes.json`, `final-rows.json`, `final-acceptance.json`, `performance.html`; original smoke summary: `D:/Coding/renderlab-work/ara-integration/smoke/smoke-20261115-040742-bin-smoke-0d8fce79.txt`.

Implementation took approximately 25 active minutes; verification and reporting occupied about an hour of wall time, mostly lease waits. Warm release builds took 43-57 seconds. No standalone Cargo target was created; the shared smoke target was left intact. PNG evidence was preserved.

## 4. Outstanding / next step

No SSR default or speed improvement is retained; consecutive-motion and cut quality remain unverified.
Before another march optimization, isolate why valid screen intersections fail history acceptance, and reuse the visibility pass's depth rather than reconstructing another full-frame hierarchy.
