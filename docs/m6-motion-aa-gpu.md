# M6 T2a on the GPU: world-anchored sample records under camera motion

2026-11-05, branch `t3/world-anchored-motion-aa`: `origin/t3/temporal-primary-rays` (`113b31e`, T1a with T1b reverted) with main `8f45f36` merged. This implements and measures unit T2a of [the motion-AA design](m6-motion-aa.md#7-next-unit-and-stop-rules), Q-walk first, under its frozen stop rules.

- **Setup:** RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1. Release library test binaries were built with two Cargo jobs. The scene is the registered full-resident quarry (12 draws, 13,034,903 triangles) at 1920×1080 1x, with 16x anisotropy and a 0.04-rad disk.
- **Runs:** every GPU run was offscreen under the shared lease. There was no window, viewer, desktop input, game capture or Unreal launch.
- **Labels:** **M** is measured here; **I** is inference.

**Result: T2a stops under three frozen rules, for both K. The defaults are unchanged.**

- **What passes:** in motion, T2 visibly antialiases silhouettes. Every radiance population in F1 is within the bar, at worst 0.84× (K = 4) and 0.85× (K = 8). Missed film shadows are 13x fewer than Exact's at K = 8 and 3.6x fewer at K = 4 (F4). The GPU quality matches the CPU model.
- **What stops it:**
  - **Cost:** moving frames take 3.55 ms (K = 4) and 5.76 ms (K = 8) against the 1.075 ms stop.
  - **F1 solids:** there are replica-confirmed solids over 0.002 outside the receiver-defect band.
  - **F2:** revealed background-centre and edge pixels are above Interactive.
- **Also passing:** T-0, the static byte identity against a warm T1a, F8, F6, the CPU-model check and memory.
- **Also failing:** F3 (edge bias) and F5 (ghost bias and hidden records).
- **Status of the records modes:** `Records4` and `Records8` remain registered candidates, off by default. They are never presented as meeting the bar.

**Follow-up (2026-11-06, [§9](#9-follow-up-the-outliers-and-the-scatter-price)).**

- **The outliers:** no T2 defect was found.
  - The umbra outlier is a correct sample of a second surface in an edge pixel.
  - The lit-side solids are receiver-defect samples just outside the 1e-4 band test, plus two single-ray blocks of unknown cause from the production trace.
- **The scatter** reproduces the gathers bit for bit and does not depend on arrival order. It saves only 0.39 ms (K = 4) and 1.25 ms (K = 8): K = 8 moving frames take 4.25 ms.
- **Verdict:** K = 8 cannot plausibly reach 1.075 ms. The unit stopped there: nothing was built and defaults are unchanged.

## 1. What T2a implements

API: `Renderer::set_film_motion(FilmMotion)` takes one of:

- `Restart`: T1a, the default;
- `Records4` and `Records8`: the registered candidates.

Both K come from one binary, with K as a pipeline constant. `cut_film()` marks a camera cut. Defaults are unchanged.

**Records.** Each pixel holds up to K samples of 16 bytes (rgba32uint):

- colour as three f16 (the program output after `rl_finite`);
- placement + 1 in 16 bits (zero is background; background records take the frame's background colour);
- the hit's depth, stored as near / view depth under the view-projection with the far plane at infinity (`Camera::infinite_matrix`), so a background record is a direction at depth 0;
- its position in the pixel holding it, 12 bits per axis (1/4096 px), age (4 bits, saturating), side, valid, and the tile and cold flags.

Slot 0 is the pixel's fresh sample of the last moving frame, one Rgba32Uint texture per side. Slots 1..K-1 are moved records, a K-1-layer array per side. Both are ping-ponged by moving frame.

**Moving frame** (the camera changed, nothing else did), five passes:

1. `film_gap`: each pixel gathers, in a fixed order, the previous records that now land in its footprint. It picks the position farthest from them among the next 16 lattice indices, or the centre (index 0) when none land.
2. The motion trace: the fused Interactive trace with the T1 ray and rotation seams. It traces that sample and publishes each pixel's fresh plane (depth, octahedral normal, placement, side, index).
3. `film_validate`: each pixel gathers the same records again and keeps up to K-1 of them, youngest first:
   - geometry records on its own fresh plane or a same-placement 4-neighbour's, within four footprints along this frame's camera ray;
   - background records when the 3x3 fresh samples are all background;
   - in discontinuity pixels, every record also needs an unoccluded exact camera any-hit ray from the near plane to its point (the far plane for background). A discontinuity pixel's 3x3 fresh samples mix geometry and background, or span more than four footprints.
4. The centre retrace: the same trace pipeline (mode 1) retraces, at the centre with index 0, every pixel that kept no record and whose sample was jittered.
5. The program's motion shading: the program runs once per pixel on the fresh sample, as in Interactive. Its output becomes slot 0, and the pixel stores the mean of the sample and its kept records.

A pixel without a kept record therefore stores its centre sample's program output unchanged. Records move as points through `current x previous^-1`, composed in f64, with explicit multiply-adds; nothing is interpolated.

**Static frame** (nothing changed): T1a's trace and shading pipelines, unchanged. The records stay as the last moving frame left them.

- The first static frame after motion seeds T1a's history from them (`film_seed`: the mean of the records, weight = their count).
- A reset frame is T1a's (the production Interactive frame and `film_init`), followed by `film_records_init`, which starts the records from that image.

**Implementation choices against [section 4](m6-motion-aa.md#4-t2-design)**, all fixed before any gate run:

| Design | Implemented | Why |
|---|---|---|
| Validate in the shading wrapper after `rl_finite` | A separate pass before shading; the wrapper only accumulates | A pixel whose records all fail validation can then be retraced at the centre, so its image is Interactive's (T-0) |
| Gather and gap choice in the trace | `film_gap`, its own pass | Same arithmetic; the trace kernel keeps its registers |
| Static frames replace the oldest record in place | Records are frozen in static frames; T1a's modules are untouched | Static output is T1a's by construction. After a reset, static frames, then motion, the first moving frame starts with one record |
| Records in every pixel | Some tiles hold *implied* background records instead (lattice positions of a frame index), moving with their pixel's direction motion. These are tiles where no pixel has fresh geometry around it, a kept geometry record or a cold flag | Saves storage and gathers outside the geometry; background is uniform |
| Position u8 | 12 bits per axis | 8-bit re-quantization moves records up to 2e-3 px per frame; the real step's failures scale as the square of misplacement |
| Hit distance | Infinite-far reversed depth | Exact for geometry; background moves as a direction |
| 3x3 window around the reprojected centre | 3x3 when the 3x3 previous fresh motions agree within 0.5 px, else their bounding box, at most 5x5 | Silhouettes have two motions |
| — | Exact rays stop 2^-12 short of the record's point | Avoids self-hits; catches blockers more than 2 mm in front at 8 m |
| View-angle cap (open decision) | None; ages saturate at 15 | Left open |
| Pooled storage | Full allocation | See [memory](#5-memory) |

Moving-frame motion shading is compiled per program on its program's first moving frame, from the program's source. It sits outside the program system's variant tables, keyed by revision, program and K. Integrating it as a program variant would be follow-up work.

Nothing lighting-specific enters the hit record, the records' validation or dispatch:

- validation reads geometry only;
- records store program output, as T1a's history does ([T1 §6](m6-temporal-primaries.md#6-programmable-shading)).

## 2. Gates (Q-walk, build b10, M)

Q-walk is 161 frames: walk 1–64, hold 65–96, return 97–160. 60 frames are scored against Level F film references at the 26 walk cameras. The return reuses the walk cameras.

- **Runs:** 31 runs per K of the full program, plus one each of the sun-only and visibility programs. The scored captures come from run 0, which has test diagnostics on.
- **Scoring:** by `scripts/m6-motion-aa-score.py`; K = 8 took 443 s and K = 4 376 s, on the CPU.

| Check (frozen action) | K = 4 | K = 8 | Verdict |
|---|---|---|---|
| T-0 (fix first) | Reset frames HDR-identical to Interactive in all 31 runs. Pixels without a kept record identical by HDR, hit record and counts at 20 checked moving frames | Same | Pass |
| Static vs T1a (fix first) | Warm T1a, pose 1: byte-identical in all 128 frames and the final history, in all three programs | Warm T1a, pose 2: the same | Pass ([§3](#3-identity-and-determinism)) |
| F8 replay (fix) | 31 runs give one image sequence in all 60 frames. Final records are identical within each setting: 2 runs with test diagnostics, 29 without | Same | Pass. The scorer's F8 (`replay_gate`) checks images across all runs and records within each setting; the two settings differ only by the diagnostic band marker |
| F1 radiance: converged L0, L4, D0, D4 against the bar | Worst 0.84× (D0 background-centre, frame 112); R_F on partial populations at most 0.94× | Worst 0.85× (the same); R_F at most 0.58× | Pass |
| F1 solids: replica-confirmed R_F over 0.002 outside the receiver band (stop) | 267 unverified exceedances, max 0.33. At poses 64 and 60 (164 of them): 11 confirmed, 3 pixels, including ±0.25 in motion | 326 unverified, max 0.17. At poses 64 and 60 (144): 14 confirmed, 9 pixels, including 0.125 in motion | **Stop** ([§4](#4-solids)) |
| F1 solid D maximum above Exact's | 13 frames | Frames 140, 156, 160 | Fail (same pixels as the solids, I) |
| F2: transient vs Interactive and at age 7 (stop) | 6 pools above Interactive. Revealed background-centre D0 at ages 1–6 (age 1: 0.907 vs 0.754). Revealed occluded D0 at age 6 (0.422 vs 0.322) | 9 pools. Revealed background-centre D0 at ages 1–6 (age 1: 0.881 vs 0.616), and D4 at age 1. Revealed edge D0 at age 1 (1.100 vs 0.972). Revealed image D0 at age 1 | **Stop** |
| F3 bias | 40 failures, mostly faint D4 (−0.07 to −0.09 codes) and edge D0 | 35, mostly edge D0. In the return: +0.07 to +0.08 codes against Exact's −0.01 to −0.05, se 0.006. Faint D4 at −0.07 to −0.09 | Fail |
| F4: R_D excess over Exact (stop) | None. Missed film shadows 2,193 vs Exact 7,984 at frame 128 | None. 609 vs 7,984 | Pass |
| F5: changed pixels 8 frames after stop | Pass (frame 72) | Pass (frame 72, every channel ≤ 0.28× bar) | Pass |
| F5 ghost bias | R_F +6.4e-4 at frame 60 (se 2.5e-4) | R_F +5.0e-4 at frame 124 (se 1.6e-4) | Fail |
| F5 hidden-record weight (test-only count) | 907 kept records hidden over the run | 3,127, about 24 per moving frame (up to 38) | Fail |
| F6 flicker | Pass | Pass. Moving-pair L0 RMS 2.2–2.3 codes vs Exact's 7.0–7.2 | Pass |
| CPU-model check: all-covered L0 | 0.50–0.53× bar (limit 0.58) | 0.33–0.35× (limit 0.44) | Pass |
| Cost: moving p50 over 1.075 ms, or a frame over 1.5x steady (stop) | 3.545 ms; 2 frames over 1.5x, max 23.9 ms | 5.756 ms | **Stop** ([§6](#6-cost)) |
| Memory: Islam's budget of up to 0.6 GB | 0.27 GB records | 0.53 GB | Pass ([§5](#5-memory)) |

**F1's R_F channel.** On the solid populations, Exact's R_F RMS is zero or nearly zero. T2's RMS there is therefore set by the solid exceedances below. K = 4 frame 128's lit R_F at 1.02× Exact is the same pixels.

### Images inspected (M)

These are the scorer's crop sheets and overviews at frames 0, 1, 3, 7, 31, 48, 64, 72, 96, 99, 104, 128 and 160, plus 4x nearest-neighbour silhouette crops. The crops compare the film reference, T2, Interactive and Exact 1x at matching cameras: `peek/k8/zoom-f128.png`, `peek/k8/zoom-f099-right.png` and `peek/k4/zoom-f128.png`.

- **Silhouettes in motion.** At frame 128 (return, moving), K = 8's cliff-against-sky edge is a smooth antialiased ramp, matching the film reference. Interactive and Exact 1x show the hard staircase. K = 4 is visibly rougher than K = 8, with some steps left, and still far better than 1x. At frame 99, three frames after motion resumes, K = 8's right silhouette is mostly smooth, with a few steps left.
- **Error maps.** T2's L4 error ×8 is much lower than Exact's and Interactive's on the interior. The silhouette keeps a thin continuous error line, of one sign over long runs. It is fainter than 1x but systematic, matching F3's edge D0 bias.
- **No trails, ghosts or smears** at any inspected frame, including the hold-to-return transition (frames 96, 99 and 104).
- **R_F error maps** show isolated one-sample dots at shadow edges. Most are reference misclassification ([§4](#4-solids)).

## 3. Identity and determinism

**T-0 on pixels without records (M).** After every checked moving frame, the test rendered a production Interactive frame at the same camera. It compared every pixel left without a kept record, which is traced at the centre with index 0, by HDR, hit record and counts. Results:

- b4–b6 (K = 8, frames 1–23): all matched, 512–2,725 such pixels per frame.
- b10: all matched, at all 20 checked frames per K.
- Pixels whose 3x3 lies outside the scene rectangle report their implied background records instead.

**Replays (M).**

- **b3 did not replay bit-identically:** frame 23 and the final records differed.
  - Cause: the centre retrace can mark a tile covered that the motion trace left uncovered. The motion trace writes no hit records in uncovered tiles, so the shading read other pixels' stale records there.
  - In the first run, the interleaved T-0 Interactive frames had refreshed those records; in the replay they had not.
  - Fixed in b4: retraced tiles' other missed pixels store their miss.
- **b4 onwards** replay bit-identically. b10: 31 runs per K, as in the gate table.

**Static (M).** Static frames with records run exactly T1a's pipelines, and no record pass runs in them. The first static runs (b7, K = 8, Q-static, 128 frames × 3 poses × 3 programs) were byte-identical to T1a in seven of nine cases. In the other two, one frame each differed late: pose 1 full at frame 125, pose 2 sun-only at frame 39.

A diagnostic (b9; poses 2 and 1; full and sun-only; four runs each: T1a, T1a, records, records) compared each run with the first:

| Case | T1a run 2 vs run 1 | Records runs vs run 1 | Differences |
|---|---|---|---|
| pose 2, full | differs at frames 34, 51, 123 | differs at the same frames | 1 pixel per frame, at most 6.1e-5 in f16 HDR; hit records and counts identical |
| pose 2, sun only | differs at frames 18, 19, 38, 49, 68 | differs at the same frames | 1 pixel per frame, at most 9.5e-7 |
| pose 1, full and sun only | identical | identical | none |

**The first T1a run is the outlier.** T1a's temporal shading pipeline rounds one pixel differently on first use than afterwards. This is the driver arithmetic hazard ([determinism](determinism.md)) in T1a itself. T1a's own F8 compared repeats after the pipelines had already run, so it could not see this. This is a pre-existing T1a determinism defect, recorded here and not fixed in this unit.

The final static gate therefore compares records against a second, warm T1a run in the same process:

- K = 4, pose 1;
- K = 8, pose 2, where the drift shows.

Both are byte-identical in all 128 frames and the final history, in all three programs. The first T1a run differed from the second again: at frame 68 (pose 1, full), and at frames 60 and 54 (pose 2, full and sun-only).

## 4. Solids

The scorer lists every converged single-surface pixel whose R_F differs from the reference's 0 or 1 by more than 0.002, with its record flags. Band suspects are records whose fresh sample had all 8 rays blocked within 1e-4 barycentric of a triangle edge. They are 9 (K = 8) and 4 (K = 4).

The poses with the most other exceedances were verified with 16 further independent 1,024 × 32 visibility replicas (`gpu_film_t1_reference`, `verify.json`, 103 s):

- pose 64, which covers the hold and 124 (K = 8) and 156 (K = 4) exceedances;
- pose 60.

| | K = 4 | K = 8 |
|---|---|---|
| Exceedances at poses 64 and 60 | 164 | 144 |
| Refuted: the replicas see a partial pixel, within 1.5e-5 to 0.0035 of solid | 153 | 130 |
| Confirmed, hold phase (T1a's static mean seeded from records) | 9, one pixel, max 0.125 | 11, seven pixels, max 0.036 |
| Confirmed, moving frames | (890, 370) solid-lit −0.25; (780, 555) **umbra +0.25**, both at frame 60, discontinuity pixels, 3 kept records, not band suspects, not hidden | (991, 782) solid-lit −0.125 at frame 60; (1412, 689) −0.0156 at frames 60 and 100; no kept record found hidden by the test ray |

- **Most exceedances are the finite reference.** As in [T1a](m6-temporal-gpu.md#verified-solids-m), the 1,024-sample reference calls partial pixels solid. T2 weights a sample at 1/K, so one opposite sample in such a pixel reads as 0.125–0.25.
- **Hold phase.** Errors decay as 1/(records + static frames), for example 1/28 at frame 84 and 1/40 at frame 96. So one fully opposite sample entered the mean, in the T1a static path after the seed.
  - This is the signature T1a itself shows at 1/128 ([T1a §0.3](m6-temporal-gpu.md#03-solids-verified-locally-and-the-remaining-cause)): the receiver-origin band at concave edges.
  - Here it weighs more, because the mean has only 28–40 samples. The 1e-4 band test covers moving-frame records only.
- **Moving frames, solid-lit.** One sample with all 8 rays blocked on a verified lit footprint is the receiver-band signature too. These samples were outside the 1e-4 barycentric test, so they count under the frozen rule. The 1e-4 width comes from T1a's three measured cases. Whether wider bands exist on smaller triangles is not established (I).
- **Moving frame, umbra, K = 4 (780, 555): a fully lit sample in a verified umbra pixel.** The receiver band can only block rays, so this is not the known defect.
  - The pixel is a discontinuity pixel, so its kept records passed exact camera rays, and the test count finds none hidden.
  - Its cause is not established. Candidates, all unverified:
    - a record from a different surface landing in a footprint the reference sees as single-surface;
    - a background record kept through a mesh crack;
    - a record whose landing position differs from its world point.
  - A per-record dump at these pixels is the next diagnostic.
- **The remaining exceedances are unverified:** 103 (K = 4) and 182 (K = 8) at other poses. Confirmed cases at the two verified poses already trigger the stop. At the verified poses, 90–93% of exceedances were refuted.

## 5. Memory

Full allocation at 1920×1080 (2,073,600 pixels), on top of T1a's history:

| | Per pixel | K = 4 | K = 8 |
|---|---:|---:|---:|
| Records: two sides of fresh (16 B) and moved ((K-1) x 16 B) | 32 K B | 265.4 MB | 530.8 MB |
| Fresh planes (this frame) | 16 B | 33.2 MB | 33.2 MB |
| T1a history (colour, anchor), unchanged | 48 B | 99.5 MB | 99.5 MB |
| Temporal film total | | 398.1 MB | 663.5 MB |

- **Against the budget.** Islam's budget is up to 0.6 GB for sample records at 1080p. The records are 0.27 GB (K = 4) and 0.53 GB (K = 8), or 0.30 and 0.56 GB with the planes. Both fit.
- **At 4K** the records are 1.06 and 2.12 GB.
- **Pooling** to tiles with explicit records was not built.
  - On Q-walk those tiles hold about 357 K pixels (17%; M, record diagnostics), so pooled records would be about 46 MB (K = 4) and 92 MB (K = 8). Both are under the design's 128 MB pooled stop.
  - A fully covered view needs the full allocation.

## 6. Cost

Paired Interactive and Temporal frames along Q-walk: 8 warm-up and 56 measured pairs per run, validation off, one process each (`gpu_film_t1_timing`, b10). The variants are the same binary with test-only kernel edits:

- *no rays*: exact rays accept;
- *no gap*: the gap pass writes the next lattice index without gathering;
- *neither*: both edits.

These variants exist only for attribution. Each removes a quality-critical part.

**All numbers are contended.** Other workers compiled throughout: up to 4 cargo and 7 rustc processes at preflight, and repeated memory-floor aborts. No quiet window was available. Interactive measured 0.86–0.90 ms in most runs, against 0.866 ms quiet.

Moving-frame spans, p50 in ms; the Interactive column is from the same runs:

| K | Variant | Interactive | Moving | Trace incl. gap | Validation and retrace | Shading | Paired ratio |
|---|---|---:|---:|---:|---:|---:|---:|
| 4 | full | 0.865 | **3.545** | 1.868 | 1.213 | 0.413 | 4.11 |
| 4 | no rays (slow GPU state) | 1.302 | 3.521 | 2.255 | 0.856 | 0.645 | 3.70 |
| 4 | no gap | 0.863 | 2.419 | 0.601 | 1.198 | 0.427 | 2.61 |
| 4 | neither | 0.886 | 1.638 | 0.534 | 0.609 | 0.445 | 1.84 |
| 8 | full | 0.902 | **5.756** | 2.957 | 2.166 | 0.480 | 6.28 |
| 8 | no rays | 0.903 | 4.662 | 3.101 | 1.017 | 0.483 | 5.15 |
| 8 | no gap | 0.860 | 3.037 | 0.590 | 1.932 | 0.470 | 3.52 |
| 8 | neither | 0.901 | 2.165 | 0.621 | 0.997 | 0.493 | 2.39 |

- **Attribution (by difference, M):**

  | Part | K = 4 (ms) | K = 8 (ms) |
  |---|---:|---:|
  | Gap gather | 1.3 | 2.4 |
  | Exact discontinuity rays | 0.6 | 1.15 |
  | Validation gather with plane tests and retrace | 0.6 | 1.0 |
  | Trace and shading over Interactive's | +0.1 to +0.2 | +0.1 to +0.2 |

  Earlier screens (b5, b6) agree: K = 4 at 3.45 ms and K = 8 at 5.63 ms. Moving the gap gather out of the trace kernel (b6) left the paired ratio unchanged.
- **Against the design estimate.** §5 estimated 0.99–1.32 ms. The gathers and rays cost 10–25x their estimate. The estimate priced the bytes, not the per-record work:
  - Every pixel reprojects and tests each record of its 3x3–5x5 source window, twice: 9–25 sources × up to K-1 records.
  - Exact rays are traced per record in discontinuity pixels.
- **Spikes.** K = 4 full had 2 frames over 1.5x its median, the largest 23.9 ms.
- **Static frames.** Paired against Interactive: +0.045 ms (K = 4, in a slow GPU state with Interactive at 1.20 ms) and +0.026 ms (K = 8). That is T1a's static cost; no record pass runs.
- **Floors.** Moving frames miss the 2x HRT floor (0.87–0.89 ms) and the VSM floor (0.80 ms) by 2.7–4.9 ms. Even *neither*, which removes the gap choice and exact rays, is 1.64 and 2.17 ms.

**Lever (I, not built).** Under the frozen stop, a next candidate must first show that the cost scales down. The obvious lever is to replace both gathers with one scatter: each record is reprojected once and appended to the pixel it lands in, and validation reads only those. Exact rays would still be paid per discontinuity record, as F2's remedy requires. That should first be priced with a short reproducer, before any quality work.

## 7. Risks

1. **Cost (M).** T2's reuse as built is dominated by per-pixel gathers of other pixels' records and by exact rays. F2's frozen remedy, a wider exact-ray band, adds more rays. No variant measured here reaches 1.075 ms.
2. **Confirmed solids in motion (M).** A record weighs 1/K, so any single wrong sample costs 0.125–0.25 in a solid pixel.
   - The lit-side cases match the receiver-band signature outside the 1e-4 test.
   - The umbra case at K = 4 is unexplained.
   - Fixing the receiver origin (the separate unit) removes the first kind; the second needs diagnosis.
3. **Revealed pixels and edges (M).**
   - Revealed background-centre and edge pixels are worse than Interactive for six frames (F2).
   - In the return, edges carry a +0.07–0.08 code D0 bias (F3). The kept set at silhouettes is not area-uniform: records survive or fail validation by surface.
   - The frozen remedies are a wider exact-ray band and the cold-pixel boost. Both cost more.
4. **Hidden records (M).** About 24 kept records per moving frame (K = 8; 7 at K = 4) are hidden from the camera. They are in pixels the plane test accepted, outside the discontinuity band. Their colour weight is small but nonzero.
5. **T1a first-use determinism (M, pre-existing).** One pixel per affected frame differs between the first and later T1a runs in a process (≤ 6.1e-5 HDR). This needs its own fix in T1a's shading pipeline.
6. **Other limits:**
   - **Programmability:** motion shading is compiled outside the program variant tables.
   - **View-dependent terms** have no view-angle cap; records keep their frame's output for up to 15 frames.
   - **Pooling** was not built; at 4K the records are 1.06 and 2.12 GB.
   - **Coverage:** only Q-walk ran. Q-disocclusion and Q-zoom were not reached.

## 8. Evidence and reproduction

Evidence lives in `E:/renderlab-work/t2a/`. Scores and summaries are kept: `scores-walk-k{4,8}/score.json` and their images, `timing-summary.json`, `verify-summary.json` (each verified exceedance with its replica value), the `static-*` JSON files, `runs/`, `gpu-windows.json` and `peek/`. The reference compacts (`walk-ref/ref-p*/compact.npz`, 309 MB) are kept, so the runs can be rescored without GPU time. The measured test binary is kept in `builds/b10`. Raw captures, centre frames and replica dumps over 5 MB were deleted after scoring, about 8.6 GB. The scripts are:

- `scripts/m6-motion-aa-gpu-run.py`: the guarded lease runner, built on `scripts/m6-film-run.py`;
- `scripts/m6-motion-aa-score.py`;
- `scripts/m6-motion-aa-timing.py`.

```text
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir <own target>
# References: gpu_film_t1_reference with walk-ref.json (26 walk cameras, Exact and Interactive centre seed 0)
# Gates: gpu_film_t2_motion with walk-k4.json / walk-k8.json (31 runs, diagnostics in runs 0-1)
# Static: gpu_film_t2_static with static-k4-p1.json / static-k8-p2.json (restart, restart, records; baseline 1)
# Solids: gpu_film_t1_reference with verify.json (poses 64 and 60, 16 replicas, visibility only)
# Cost: gpu_film_t1_timing with timing-walk-k{4,8}-{1,norays,nogap,bare}.json and timing-static-k{4,8}-1.json
py -3 -B scripts/m6-motion-aa-score.py --ref E:/renderlab-work/t2a/walk-ref --run E:/renderlab-work/t2a/walk-k8 --out E:/renderlab-work/t2a/scores-walk-k8
py -3 -B scripts/m6-motion-aa-timing.py E:/renderlab-work/t2a/timing-walk-k8-1 ...
```

**Time (M).**

- **Implementation:** the first gate-ready build, b3, came at 18:27 UTC, 58 minutes after the 17:29 start. Builds b4–b10 (the replay fix, cold border pixels, infinite-far depth, `film_gap` as its own pass, diagnostics and split pass timing) were interleaved with verification until 20:46.
- **Verification:** 18:27–21:47 UTC.
  - **GPU:** 53 leases held 2,992 s, all under 45 minutes, between 18:27:37 and 21:43:46.
    - The longest real lease was 415 s (`static-diag-k8-r2`).
    - One lease, `timing-walk-k4-nogap`, stayed stale for 433 s. Its run was a 5-s preflight deferral, but its release hit a Windows share violation on `owner.json`. It was removed by hand, the ledger and schedule record it, and the release now retries.
  - **CPU scoring:** 13.6 minutes.
  - **Coordination:** most of the wall time went to the receiver-origin worker's reserved windows, memory floors while other workers compiled, and one CPU job at a time.

## 9. Follow-up: the outliers and the scatter price

2026-11-06, on this branch at `ff7b949` with main `400baaf` merged.

- **Run:** one batched GPU process, `gpu_film_t2_batch` (build c2, release, two Cargo jobs). It loads the quarry once and runs the per-record probes, the scatter identity check and the paired timing.
- **Lease:** 89 s (22:26:38–22:28:07 UTC) under the fast-check guard, 4 GiB to start and a 2 GiB in-run abort.
- **Contended:** available memory fell to 3.9 GiB, and up to 5 rustc and 7 build processes ran. Validation was off. No window or capture.

**Result: no T2 defect was found, so nothing was fixed in T2.**

- **Cost:** the scatter variant is exact and deterministic, but K = 8 moving frames still take 4.25 ms. K = 8 cannot plausibly reach 1.075 ms, so the brief's stop applies.
- **Not done:** no production build and no Q-walk gate rerun. `Records4` and `Records8` and the defaults are unchanged.

### 9.1 The outliers, record by record (M)

`support/film_t2_probe.wgsl` is test-only. It is applied to the production record kernels through exact-once seams (`PROBE_EDITS`).

- **What it writes:** at up to three probe pixels per frame, every landed record in gather order:
  - its source pixel, slot and age;
  - its landing point and world point;
  - both sides of the five plane tests, and the exact-ray result;
  - the closest hits of camera rays through the landing point, from the near plane and around the record's own point.
- **Window dumps** around each case give the hit record, visible rays, plane, records and T1a history per frame. `scripts/m6-motion-aa-probe.py` traces each kept record back to its birth sample.
- **Reproduction:** the probe run reproduces the gate's values: 0.25, 0.75, 0.875 and 0.984375 at the four moving-frame pixels.

| Case | The opposite sample | Its origin | Verdict |
|---|---|---|---|
| K = 4, frame 60, (780, 555), umbra, +0.25 | Kept record, placement 7, V = 1, landed at (780.997, 555.012) | Fresh sample of (781, 555) at frame 59: 0.034 barycentric from an edge, 8 of 8 rays visible | **Correct sample of a second surface** (below) |
| K = 4, frame 60, (890, 370), lit, −0.25 | Kept record, V = 0 | Frame 57, triangle 832578, barycentrics (1.28e-4, 0.084), 0 of 8 rays | Receiver-band signature, 1.28e-4 from an edge |
| K = 8, frame 60, (991, 782), lit, −0.125 | Kept record, V = 0 | Frame 55, triangle 910138, barycentrics (1.94e-4, 0.342), 0 of 8 rays | Receiver-band signature, 1.94e-4 from an edge |
| Hold: K = 4 (978, 589) frame 68; K = 8 (978, 589) frame 94, (933, 641) frame 84, (1323, 442) frame 95 | A T1a static sample after the seed. Every seed mean was exactly 1.0 | 0 of 8 rays, at 7.9e-4, 7.9e-4, 4.5e-5 and 2.6e-5 barycentric from an edge | Receiver-band signature, in T1a's static path |
| K = 8, frames 60 and 100, (1412, 689), lit, −1/64 | Frame 60's own fresh sample, kept as a record to frame 100 | Triangle 440454, barycentrics (0.510, 0.087), 7 of 8 rays | Not the band; unexplained |
| Hold: K = 8, (1148, 711), frame 91 | A T1a static sample | 0.041 from an edge, 7 of 8 rays | As (1412, 689) |

**The umbra outlier is not a T2 error.** Every test the record passed is correct.

- **Placement:** it carries placement 7, the placement of the pixel's up and right neighbours' fresh planes. It passed the plane test against both.
- **Visibility:** the exact camera ray to its point was unoccluded.
- **Position:** the closest-hit camera ray through its landing point, at frame 60's camera (pose 60's), hits its own triangle (592087) at t/L = 1.000000. So placement 7 is visible inside this pixel, 0.003 px from its right edge.
- **Why the reference calls it solid:**
  - All 1,024 reference samples landed on placement 1, so the pixel counts as single-surface.
  - Its own centre placements put placement 7 at the right and top-right neighbours, so it is in the edge population.
  - The 16 replicas condition on the centre surface. They confirm that surface's visibility (0), not that the pixel has a single surface.
- **What it is:** a two-surface edge pixel. The lit placement-7 sliver is too thin for 1,024 samples. T2 holds one of its four samples there, at weight 1/4.
- **First wrong decision:** classifying the pixel as a film-solid, not a record decision.
- **Crop:** `outlier-k4-f60-780-555.png` shows the instance boundary along the pixel's top-right, with the lit record on its right edge.

**The lit-side solids are the receiver defect class, outside the band test (I for concavity).**

- **The signature:** all 8 rays blocked, on a hit 2.6e-5 to 7.9e-4 barycentric from a triangle edge, in a replica-verified lit footprint. This is T1a §0.3's signature.
- **Not checked here:** concavity. T1a's micro-grids established it for its three cases.
- **Why they count:** the diagnostic's 1e-4 band is narrower than these cases. The moving-frame cases are records born at those samples. The hold cases are T1a's own static samples.
- **Status:** they stay counted under the frozen rule until the receiver-origin fix lands. T2 cannot fix them: the samples are the production trace's.

**Two single-ray blocks are unexplained.** Each is a production-trace sample with 1 of 8 rays blocked, 0.04–0.09 barycentric from any edge. Each lies in a footprint whose replicas (524,288 rays) are all lit.

- **Possible causes:** a partial region below the replicas' resolution, or another receiver-origin case.
- **Not a T2 decision:** the motion trace's fresh sample is Interactive's ray at a jittered position.
- **Handed over:** to the receiver-origin worker, with the frame, pixel, triangle and barycentrics.

**Recommendation for the next gate.**

- Verify solids for single-surface coverage as well as visibility, through replica acceptance or a placement-coverage check, so a two-surface edge pixel is not scored as a solid.
- Classify the receiver signature by geometry, not the 1e-4 band.

Neither changes a threshold. The rule itself is unchanged here.

### 9.2 The scatter price (M)

**Design.** `support/film_t2_scatter.wgsl` and `Renderer::test_record_scatter` are test-only.

- **`film_scatter`:** reprojects each previous record once, fresh and moved, explicit or implied. It appends the record's gather-order key (source pixel index × K + slot) to the pixel it lands in, with one atomic per record. Capacity is 2K keys per pixel.
- **Gap choice and validation:** these read only those keys and keep a key only if its source lies in the production source window. They recompute the landing with the production code.
- **Order:** validation sorts the keys into gather order, then keeps records exactly as `film_validate_pixel` does. The output therefore cannot depend on arrival order.
- **Fallbacks:** a pixel with more than 2K arrivals gathers instead. Validation zeroes each pixel's count for the next frame.

**Identity (M).** For K = 4 and K = 8, 64 Q-walk frames from cold, run with the gathers, the scatter, the scatter and the gathers:

- every frame's HDR image and the final records (fresh, moved and planes) were identical by hash in all four runs;
- so over these frames the scatter's images are T2a's. Static frames run no record pass, and the scatter's two runs agree.

The first identity attempt, before counts were zeroed, read stale keys. It was deterministic, and differed from the gathers from frame 2.

**Paired timing (M, contended).** Q-walk frames 8–55, all moving. Gather and scatter Temporal frames alternate, each paired with an Interactive frame, in the same process. Times are p50 in ms:

| K | Variant | n | Interactive | Moving | Trace incl. scatter and gap | Validation and retrace | Shading | Paired ratio |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 4 | gathers | 24 | 0.907 | **3.889** | 1.984 | 1.332 | 0.423 | 4.16 |
| 4 | scatter | 24 | 0.907 | **3.497** | 1.832 | 0.877 | 0.420 | 3.51 |
| 8 | gathers | 24 | 0.863 | **5.504** | 2.841 | 2.017 | 0.474 | 6.18 |
| 8 | scatter | 24 | 0.863 | **4.251** | 2.458 | 1.262 | 0.470 | 4.88 |
| 8 | scatter without exact rays (attribution only) | 48 | 0.876 | 4.754 | 3.269 | 0.824 | 0.473 | 4.73 |

- **Savings:** the scatter saves 0.39 ms (K = 4) and 1.25 ms (K = 8).
- **Exact rays:** in scatter validation they cost about 0.44 ms at K = 8 (1.262 − 0.824). Only the validation span is compared: the no-ray run's trace span was slower in its own GPU state.
- **Why K = 8 cannot plausibly reach 1.075 ms:**
  - The budget over Interactive is 0.21 ms.
  - Parts no scatter removes take more than that on their own, all measured. T2a's no-gap, no-ray variant has its trace at +0.13 ms and its motion shading at +0.13 ms over Interactive's, about 0.26 ms together.
  - The scatter's validation without rays is 0.82 ms, and the exact rays add 0.44.
  - Even a free scatter and gap choice leaves about 0.86 + 0.26 + 0.82 + 0.44 ≈ 2.4 ms.
  - Halving validation and tracing the rays densely (about 0.13 ms, I) would still leave about 1.7 ms. Reading and writing each K = 8 record once is about 90 MB, about 0.2 ms (I).
- **Prototype limits:** the prototype scatters every source pixel, implied background tiles included: 16.6 M atomics at K = 8. Scatter and gap choice take about 2.0 ms of the trace span. Skipping implied sources would cut that, but not below the floor above.

### 9.3 Evidence, reproduction and time

**Evidence** is in `E:/renderlab-work/t2b/`:

- `batch.json` (the manifest) and `batch/batch.json` (identity rows and timing samples);
- `timing-summary.json`;
- `batch/probe-k{4,8}/probe.json` and `probe-f*.bin`, with `batch/probe-k{4,8}-report.json` (every record and birth above);
- the inspected crops `batch/outlier-*.png`;
- `runs/` (lease records with memory and process context);
- `builds/c2`.

The 140 MB of window dumps were deleted after the reports.

```text
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir <own target>
py -3 -B scripts/m6-motion-aa-gpu-run.py --fast-check --label t2b-batch --manifest E:/renderlab-work/t2b/batch.json -- <test binary> gpu_film_t2_batch --ignored --nocapture --test-threads=1
#   (RENDERLAB_T2_OUT=E:/renderlab-work/t2b, RENDERLAB_T2_OWNER=t2-scatter-pricing)
py -3 -B scripts/m6-motion-aa-probe.py E:/renderlab-work/t2b/batch/probe-k4 --out <report.json>
py -3 -B scripts/m6-motion-aa-timing.py E:/renderlab-work/t2b/batch/batch.json
```

**Time (UTC).**

- **Implementation:** 21:52–22:26: the probe, the scatter prototype and the batch test. Release builds took 51–135 s.
- **Verification:**
  - Per-job attempts before the batch, 22:17–22:21: three aborted on the memory floor within 8 s, two deferred at preflight, and one was stopped. One scatter identity run completed (47 s) and found the stale-count bug.
  - The batch lease took 89 s.
  - CPU analysis took about 10 minutes.
