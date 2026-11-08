# M7-TR1: identity-validated temporal reconstruction (T1b revived)

2026-11-08, branch `t3/m7-tr1` from main `ca80f85`, main `7971991` (M7-U0b) merged. Contract: [M7](milestone-7.md), [design §6–9](m7-design.md), [P0](m7-p0-claims.md), the bar in [TR0](m7-tr0.md) (`E:/renderlab-work/m7-tr0/bar/tr1-bar.json`).

**Status: TR1 is built on both paths, measured against every row of TR0's bar, and stays opt-in. It is not the default.** On the registered Q-walk it wins 12,320 of TSR Epic HRT's 22,684 rows on RT and 12,078 of TSR Epic no-RT's on non-RT, and loses 9,209 and 9,448. It beats TSR on converged and moving interiors at registered exposure, occluded pixels, hold flicker (0.244 vs 0.432 codes) and trails (no background error beyond 1.5 px at any exposure; TSR's reach 41 px at +4 stops). It loses on moving silhouettes (edge 19.9 vs 17.8 codes), revealed pixels (10.8 vs 7.7 at age 3), background-centre pixels next to edges (0.90 vs 0.37), and +4-stop populations where Renderlab's converged still image is itself above TSR's. Quiet cost: p50 increments pass the half-TSR ceilings on both paths (RT 0.377 ms vs 0.727); the p95/p99 increments and totals miss, and the non-RT total frame misses before TR1 adds anything. Byte-identical replays in and across processes, and the non-RT path on the RT device equals the no-RT device. P0 constraints hold: no variance clip, own-pixel moments, SVGF or gradients, W5 kept.

Measurement and verification times are in §11.

## 1. What TR1 is

`Renderer::set_reconstruction(Reconstruction)` (default off). On traced and visibility-buffer lit frames at 1x, on RT and non-RT devices:

1. **Jitter through the projection.** Frame *k* samples every pixel at one offset of T1's corner-free 256-point rank-1 lattice (one symmetry for the whole image), carried by `Camera::jittered` in the view-projection. Traced rays, the N1c visibility raster and resolve, the receivers and every program's shading take it from the same matrix, so no primary, shading or shadow shader is edited. Frame 0 of a sequence is the pixel centre. A jitter of zero keeps every existing matrix bit-identical.
2. **Interactive's eight disk directions rotate** by 8·(*k* mod 32) through each pixel's own 256-direction sequence (RT), through the unused `PrimaryFrame.sided.y` word. Only TR1's own compiled copy of the Interactive kernels reads it (`ROTATION_SEAM`); the near-edge check, which traces no direction, stays production's.
3. **The program shades normally.** Its output is copied, and one compute pass (`shaders/temporal_resolve.wgsl`, compiled from the no-ray-query head, so it runs on no-RT devices) writes the HDR target and the next history.
4. **Reprojection by surface identity.** Each pixel's current sample point comes from its hit record (placement, primitive, barycentrics, the same receiver point R1 uses); its history is fetched where that point was in the previous frame. A history tap is continued only when its anchor (the latest sample's placement, viewed side and view distance) shows the same surface: the tap's own sample point, rebuilt from its previous-frame ray, must lie within four pixel footprints of the current point's tangent plane. No colour, luminance or neighbourhood statistic decides acceptance or weight.
5. **Silhouette bit.** A pixel whose 3×3 samples show more than one surface writes a "mixed" anchor. Interior and open-background pixels never continue mixed history, so nothing trails into interiors or sky. Open background (every 3×3 sample a miss) is the background exactly.
6. **Still frames** (bit-equal unjittered view-projection, decided on the CPU): each pixel continues its own history while the tap's surface is still among its 3×3 samples, so edges converge to the footprint integral as T1a does.
7. **Windows.** Static 256; under bilinear motion T1b's blur budget W = 1 + v_max / Σf(1−f); Catmull-Rom over 4×4 when all 16 taps are accepted (window 16), limited to its own inner taps' range (the undershoot fix: history only, not a current-frame box); moving silhouettes cap the window.
8. **Revealed and restarted pixels start from this frame's samples only:** a tent over the 3×3 samples of the same surface (radius 2 inside a surface, effective count ≤ 4).
9. **Engine events.** A key over programs, prelude, geometry, bindings, materials, textures, transforms, light colours and kinds, environment and every shadow/visibility setting restarts every pixel. Exposure changes rescale the exposed history exactly. A sun turn caps the window at `turn_angle / turn` (0.004 rad; larger turns restart). `cut_reconstruction()` restarts on a camera cut the engine cannot infer.

### Settings (`Reconstruction`, default off; `Reconstruction::identity()` enables the measured values)

| Field | Value | Meaning |
|---|---:|---|
| `window` | 256 | static window |
| `blur_budget` | 1 px² | T1b's bilinear resampling budget |
| `sharp_window` | 16 | moving window when all 16 Catmull-Rom taps are accepted |
| `silhouette_window` | 2 | moving silhouettes |
| `start_window`, `start_radius` | 4, 2 px | revealed pixels' spatial start |
| `silhouette_start_radius` | 1 px | silhouettes' spatial estimate (bilinear at the pixel centre) |
| `turn_angle` | 0.004 rad | sun-turn window cap; larger turns restart |
| `catmull_rom`, `spatial_start`, `rotate_directions` | on | |
| `silhouette_mix`, `silhouette_spatial` | on | moving silhouettes keep coverage and blend this frame's bilinear 3×3 estimate |
| `box_clamp` | off | plain min/max box over all 3×3 samples (last option, measured, no effect) |

### 1.1 Files

| File | Content |
|---|---|
| `src/temporal_reconstruction.rs` | `Reconstruction`, history state, frame words, pipelines, prepare/encode, CPU tests |
| `shaders/temporal_resolve.wgsl` | the resolve pass |
| `src/temporal_reconstruction_tests.rs` | `gpu_tr1_sequence` (both paths, variants, frames, visits, replay) |
| `src/mesh.rs` | `Camera::jittered` (projection plumbing) |
| `src/renderer.rs`, `src/primary_ray_gpu.rs`, `src/observe.rs`, `src/lib.rs` | settings, jittered camera of reconstructed frames, rotation word, rotated Interactive set on production passes, timestamps 4/5 (`GpuSample::reconstruction_pass`), Virtual still-key fix (§7.2) |
| `scripts/m7-tr1-*.py` | runner/waiter/quiet poller, plan, scorer, compare, table, settle, cost, inspect |

No shadow, GI/cache, reference, shader-reload or pipeline file and no shader of primary rays, shadows or programs is edited. `film_accumulation.rs` (T1a) is unchanged and stays a separate opt-in mode.

## 2. Claim-avoidance notes (engineering reading, not clearance)

Against P0's element reading ([m7-p0-claims.md](m7-p0-claims.md) §3.C–D, §4.3–4.6):

| Claim | TR1 has | TR1 lacks, and why |
|---|---|---|
| US 10,116,916 cl. 1, 11, 18 (variance clip) | (a) current frame, (b) previous colour, (d) blend | (c) no neighbourhood mean, variance or σ box: acceptance and weights come from surface identity, geometry and engine state. The optional min/max box (`box_clamp`, off) has no statistic |
| US 10,362,289 (box excluding a sample) | (a), (b), (e) | (c)–(d): no box by default; the optional box spans **all** 3×3 samples, so no neighbourhood sample lies outside it |
| History-clamp family (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) | a full history | no fast or short history and no clamp of one history by another: one value per pixel |
| US 11,847,737 cl. 1 | (a), (b) | (c) windows come from resampling geometry and engine events (sun turn, exposure, key), never from comparing pixel values; (d) one blended value per pixel, no retained list. P0's ambiguity on W-capped blends stands |
| US 12,536,735 | one list-like value | a second list and second algorithm |
| US 12,437,371 / pending US 2026/0162231 | (a) lighting data of two frames exist | (c)–(e): no lighting difference, gradient or confidence is computed; own-pixel moments (`Reconstruction::OwnMoments`) are not built |
| US 10,600,167 cl. 1 | (a1) back-projection through the hit record, (a2) the history sample, (a3) placement/side/plane check, (a4) colour plus the anchor and silhouette bit | (b)–(c): **W5** — the weights, anchors, silhouette bits and history are read only by `tr1_resolve` itself, next frame. No later stage (present, GI, reflections, any filter) binds them; the bind group exists only in `reconstruction_groups` |
| US 11,113,792 cl. 1, 11, 15 | the revealed-pixel and silhouette spatial estimates are spatial filters | cl. 1/11 (a)–(c): fixed 3×3 taps and fixed tent radii, not derived from render data or jittered; cl. 15 (c): the spatial estimate filters **this frame's samples**, never the temporally filtered values |
| US 12,423,782 | the camera sample jitters on a lattice | (a) filter taps are fixed: the 3×3 tent, bilinear and Catmull-Rom taps never move with the jitter; P0 already reads camera jitter as outside (a) |
| US 11,688,042 | — | no separable multi-pass kernel |

Not used: variance clipping, SVGF, temporal gradients, own-pixel moments, fast/slow histories, colour or luminance rejection. The plain min/max box was built as the design's last option and measured (§5); it changed nothing measurable and stays off.

## 3. Programmability

- Shading is untouched: programs shade the jittered hit as any traced frame; TR1 reads only their output colour, through an engine copy.
- Acceptance and weights never read colour, so toon, flat and custom programs converge to the footprint integral of their own output. Two places read output colour: the spatial estimates (a weighted mean of the program's own samples, not a decision), and the optional `box_clamp` (off).
- Constraint recorded: a program whose output is not a function of the surface point (screen-space patterns, dithering keyed to the pixel, output that depends on the frame) is integrated over the jitter like any other signal; screen-locked patterns blur by the jitter footprint. A program that wants per-frame stochastic effects should key them on the surface, not the screen.
- Constraint recorded: TR1 needs each pixel's hit record and placement, so it applies to traced and visibility-buffer frames only. Raster frames render unjittered without history.

## 4. How TR1 is measured against the bar

- **Same inputs as TR0's T1b replay:** the registered Q-walk (`E:/renderlab-work/t1-gpu/sequences/q-walk.json`, SHA-256 `db018bb0…`), quarry asset, limits, `sun128.rle2` bound for lighting, black primary background (`t1b-black-b/lighting-black-background.json`: sun travel (−0.4, −0.8, −0.3), 1,000/950/850 lux, exposure −14), 0.04-rad disk, 1920×1080, 1x. **RT** is traced primaries with Interactive (the fast RT setup) on the RT device; **non-RT** is the no-RT device's visibility buffer (LOD 0.35 px, extent 32, lift 1) with Virtual Surface shadows, M6-close's non-RT default.
- **Same target and arithmetic:** `scripts/m7-tr1-score.py` is TR0's `score` with the native capture replaced by Renderlab's exposed HDR. References are U0b's `quarry-final` lossless zero-shutter archives; populations, split-half noise subtraction, the F0 transform, distance bands, error extents, the +8 changed mask and flicker residuals are TR0's code. Age pools use **the same fixed T1b masks** TR0 used for TSR (`t1b-black-b/data/age-fNNN.u8`), so every pool is the same pixel set for TSR and TR1. A self-check compares every shared row's pixel count with the bar: **all match** (e.g. 19,744 / 19,744 on the first run).
- **Comparison:** `compare` matches each of TR0's TSR Epic rows (and the recorded T1b rows) by frame, channel, population and measure. Lower is better; a missing TR1 row is a loss. TSR Epic HRT (`w-e-r-c1`) is RT's comparator and TSR Epic globally-no-RT VSM (`w-e-n-c2`) is non-RT's.
- **Screens** used T1b's 60 scored checkpoints (1 GB per variant); final runs write all 93 reference frames.
- **Images** were inspected at frame 64, L0 and L4, as full frames and error maps, and per pixel where a pool regressed (`E:/renderlab-work/m7-tr1/inspect/`).

## 5. Results on Q-walk, both paths (M)

Final build `tr1final` (renderer code identical to `tr1final2`, which adds two test-only devices), all 93 frames with physical references, every row of TR0's bar. Frame 0 of each run starts cold.

| Comparison, all bar rows (22,684 per path) | TR1 wins | TR1 losses | ties |
|---|---:|---:|---:|
| RT (traced + Interactive) vs TSR Epic HRT | **12,320** | **9,209** | 1,155 |
| Non-RT (visibility buffer + Virtual Surface) vs TSR Epic no-RT | **12,078** | **9,448** | 1,158 |
| RT vs the recorded T1b black replay (13,826 rows) | 9,746 | 3,799 | 281 |
| Non-RT unreconstructed baseline vs TSR Epic no-RT | 4,155 | 17,371 | 1,158 |

**TR1 does not beat TSR Epic on every measure on either path, so it stays opt-in and is not the default.**

### 5.1 Headline rows, RT against TSR Epic HRT (F0 codes; bold: worse than TSR)

Rows are TR0's: noise-subtracted RMS on T1b age ≥ 32 pixels unless named; "Off" is the unreconstructed RT fast setup (Interactive 1x), byte-identical to its scored s1 frames on all 93 frames.

| Measure | Ch | TSR Epic | T1b | Off | TR1 |
|---|---|---:|---:|---:|---:|
| All-covered, frame 64 | L0 | 11.24 | 10.91 | **12.05** | 10.88 |
| All-covered, frame 64 | L4 | 16.67 | 17.59 | **19.67** | **17.14** |
| Partial, frame 64 | L0 | 13.5 | 13.94 | **16.43** | **13.91** |
| Partial, frame 64 | L4 | 21 | 22.55 | **28.09** | **21.99** |
| Occluded, frame 64 | L0 | 31.29 | 28.99 | 29.93 | 29.07 |
| Occluded, frame 64 | L4 | 51.95 | 49.9 | 51.63 | 49.98 |
| Edge, frame 64 | L0 | 17.81 | 23.57 | **23.38** | **19.92** |
| Edge, frame 64 | L4 | 31.88 | 50.5 | **50.33** | **40.83** |
| Background-centre, frame 64 | L0 | 0.3714 | 1.584 | **1.19** | **0.9005** |
| Background-centre, frame 64 | L4 | 1.168 | 5.456 | **3.49** | **2.699** |
| Hold frame 96, all-covered | L0 | 11.19 | 10.4 | **12.04** | 10.4 |
| Hold frame 96, all-covered | L4 | 16.56 | 16.91 | **19.66** | **16.85** |
| Return 120, all-covered | L0 | 10.97 | 10.59 | **11.85** | 10.67 |
| Return 120, all-covered | L4 | 16.04 | 16.85 | **19.16** | **16.66** |
| Return 128, all-covered | L0 | 10.9 | 10.55 | **11.82** | 10.58 |
| Return 128, all-covered | L4 | 15.88 | 16.64 | **19.03** | **16.45** |
| Frame 160, edge | L0 | 16.75 | 22.59 | **22.3** | **19.83** |
| Frame 160, edge | L4 | 28.94 | 47.98 | **48.87** | **41.15** |
| Hold 80->81 step | L0 | 0.432 | 0.2774 | 0 | 0.2439 |
| Hold 80->81 step | L4 | 0.4776 | 0.3314 | 0 | 0.2881 |
| Return 127->128 flicker | L0 | 3.425 | 4.492 | **7.763** | **4.42** |
| Return 127->128 flicker | L4 | 4.177 | 6.01 | **11.72** | **5.071** |
| Frame 64 background 1.5-3 px | L0 | 0.004062 | 11.29 | 0 | 0 |
| Frame 64 background 1.5-3 px | L4 | 0.06495 | 44.19 | 0 | 0 |
| Frame 64 background 3-6 px | L0 | 0.006408 | 8.188 | 0 | 0 |
| Frame 64 background 3-6 px | L4 | 0.1021 | 33.69 | 0 | 0 |
| Revealed age 3, all-covered | L0 | 7.695 | 22.24 | **12.05** | **10.77** |
| Revealed age 3, all-covered | L4 | 12.06 | 55.54 | **17.99** | **17.66** |
| Revealed age 7, all-covered | L0 | 7.76 | 15.97 | **10.73** | **9.708** |
| Revealed age 7, all-covered | L4 | 12.4 | 32.48 | **17.21** | **16.35** |
| Revealed age 7, background-centre | L0 | 4.754 | 43.51 | **13.88** | **17.09** |
| Revealed age 7, background-centre | L4 | 15.93 | 121.5 | **43.66** | **51.68** |
| Start age 0, all-covered | L0 | 10.61 | 11.56 | **11.56** | **10.61** |
| Start age 0, all-covered | L4 | 15.2 | 18.48 | **18.48** | **16.18** |
| Start age 3, all-covered | L0 | 10.62 | 10.56 | **11.6** | 10.35 |
| Start age 3, all-covered | L4 | 15.28 | 17.08 | **18.65** | **15.94** |
| Start age 7, all-covered | L0 | 10.67 | 10.42 | **11.61** | 10.32 |
| Start age 7, all-covered | L4 | 15.38 | 16.5 | **18.62** | **16** |
| +8 changed mask, frame 72 | L0 | 11.66 | 11.18 | **13.56** | 11.27 |
| +8 changed mask, frame 72 | L4 | 17.34 | 17.59 | **22.27** | **17.55** |
| +8 changed |mean|, frame 72 | L0 | 3.751 | 3.165 | **4.256** | 2.961 |
| +8 changed |mean|, frame 72 | L4 | 5.056 | 3.864 | **6.502** | 3.541 |

### 5.2 Headline rows, non-RT against TSR Epic no-RT

"Off" is the unreconstructed non-RT default (visibility buffer + Virtual Surface).

| Measure | Ch | TSR Epic | T1b | Off | TR1 |
|---|---|---:|---:|---:|---:|
| All-covered, frame 64 | L0 | 11.25 | 10.91 | **12.1** | 10.92 |
| All-covered, frame 64 | L4 | 16.67 | 17.59 | **19.76** | **17.2** |
| Partial, frame 64 | L0 | 13.51 | 13.94 | **16.48** | **13.94** |
| Partial, frame 64 | L4 | 21 | 22.55 | **28.17** | **22.04** |
| Occluded, frame 64 | L0 | 31.29 | 28.99 | 29.95 | 29.05 |
| Occluded, frame 64 | L4 | 51.95 | 49.9 | 51.81 | 50.03 |
| Edge, frame 64 | L0 | 17.82 | 23.57 | **23.42** | **19.87** |
| Edge, frame 64 | L4 | 31.88 | 50.5 | **50.53** | **40.71** |
| Background-centre, frame 64 | L0 | 0.3711 | 1.584 | **1.185** | **0.8986** |
| Background-centre, frame 64 | L4 | 1.167 | 5.456 | **3.48** | **2.692** |
| Hold frame 96, all-covered | L0 | 11.19 | 10.4 | **12.1** | 10.45 |
| Hold frame 96, all-covered | L4 | 16.56 | 16.91 | **19.76** | **16.92** |
| Return 120, all-covered | L0 | 10.97 | 10.59 | **11.92** | 10.72 |
| Return 120, all-covered | L4 | 16.04 | 16.85 | **19.31** | **16.72** |
| Return 128, all-covered | L0 | 10.9 | 10.55 | **11.89** | 10.62 |
| Return 128, all-covered | L4 | 15.88 | 16.64 | **19.18** | **16.51** |
| Frame 160, edge | L0 | 16.75 | 22.59 | **22.38** | **19.8** |
| Frame 160, edge | L4 | 28.94 | 47.98 | **49.03** | **41.12** |
| Hold 80->81 step | L0 | 0.4325 | 0.2774 | 0 | 0.2434 |
| Hold 80->81 step | L4 | 0.4782 | 0.3314 | 0 | 0.2876 |
| Return 127->128 flicker | L0 | 3.425 | 4.492 | **7.826** | **4.422** |
| Return 127->128 flicker | L4 | 4.176 | 6.01 | **11.89** | **5.069** |
| Frame 64 background 1.5-3 px | L0 | 0.004068 | 11.29 | 0 | 0 |
| Frame 64 background 1.5-3 px | L4 | 0.06504 | 44.19 | 0 | 0 |
| Frame 64 background 3-6 px | L0 | 0.006413 | 8.188 | 0 | 0 |
| Frame 64 background 3-6 px | L4 | 0.1022 | 33.69 | 0 | 0 |
| Revealed age 3, all-covered | L0 | 7.691 | 22.24 | **13.49** | **10.82** |
| Revealed age 3, all-covered | L4 | 12.05 | 55.54 | **25.91** | **17.8** |
| Revealed age 7, all-covered | L0 | 7.76 | 15.97 | **12.37** | **9.785** |
| Revealed age 7, all-covered | L4 | 12.4 | 32.48 | **24.32** | **16.46** |
| Revealed age 7, background-centre | L0 | 4.753 | 43.51 | **13.83** | **16.98** |
| Revealed age 7, background-centre | L4 | 15.93 | 121.5 | **43.52** | **51.32** |
| Start age 0, all-covered | L0 | 10.61 | 11.56 | **11.64** | **10.66** |
| Start age 0, all-covered | L4 | 15.2 | 18.48 | **18.64** | **16.26** |
| Start age 3, all-covered | L0 | 10.62 | 10.56 | **11.67** | 10.39 |
| Start age 3, all-covered | L4 | 15.28 | 17.08 | **18.79** | **16** |
| Start age 7, all-covered | L0 | 10.67 | 10.42 | **11.69** | 10.36 |
| Start age 7, all-covered | L4 | 15.38 | 16.5 | **18.78** | **16.06** |
| +8 changed mask, frame 72 | L0 | 11.66 | 11.18 | **13.61** | 11.31 |
| +8 changed mask, frame 72 | L4 | 17.35 | 17.59 | **22.39** | **17.6** |
| +8 changed |mean|, frame 72 | L0 | 3.759 | 3.165 | **4.35** | 3.049 |
| +8 changed |mean|, frame 72 | L4 | 5.065 | 3.864 | **6.62** | 3.642 |

### 5.3 What TR1 wins and loses

- **Wins on both paths:** converged and moving interiors at registered exposure (all-covered 10.88 / 10.92 vs 11.24 at frame 64; hold 10.40 / 10.45 vs 11.19; return 120/128), occluded at both exposures, the +8 changed mask at L0 and its bias at both exposures, start-age pools at L0, **hold flicker** (80→81 step 0.244 vs 0.432 L0, 0.288 vs 0.478 L4), and **trails**: no background pixel 1.5 px or more from the silhouette exceeds 1 code at any frame or exposure, where TSR has up to 110 such pixels reaching 41 px at L4 (8 over 8 codes at 19 px).
- **Losses on both paths, by mechanism:**
  1. **Silhouettes in motion.** Edge 19.9 vs 17.8 (L0), 40.8 vs 31.9 (L4) at frame 64; background-centre 0.90 vs 0.37. A single history value per pixel cannot hold partial coverage as an edge slides; TSR Epic keeps its history at 200% resolution. Moving flicker on the return (4.42 vs 3.43) is the same band.
  2. **Revealed pixels.** All-covered revealed at age 3/7: 10.8 / 9.7 vs 7.7 / 7.8 (better than T1b's 22.2 / 16.0 and than no reconstruction's 12.1 / 10.7); revealed background-centre at age 7 17.1 vs 4.75. T1b's revealed pools are mostly moving-silhouette pixels (§6.1), so they share mechanism 1.
  3. **+4-stop populations of converged pixels** (all-covered 17.14 vs 16.67; partial 21.99 vs 21.0; hold frame 96 16.85 vs 16.56). The converged still image itself is above TSR at L4 (hold 96: 16.85 vs 16.56) while below it at L0 (10.40 vs 11.19): this is Renderlab's lighting/shading against the physical target in dark regions, not reconstruction error. TR1 cannot close it.
- **Settle equivalent (the frozen film bar is deleted):** after the stop at frame 64, at +8 (frame 72) TR1's unaged errors are all-covered 10.51 vs 11.17, partial 13.17 vs 13.33, edge 16.80 vs 16.95, occluded 28.87 vs 31.21 (L0), and 16.85 vs 16.58, 21.30 vs 20.78, 30.88 vs 29.62, background-centre 0.41 vs 0.28 / 1.31 vs 0.90 at L4/L0 where TSR leads. TR1 keeps converging through the hold (edge L0 16.30 vs TSR 17.14 at frame 96); TSR is flat from the stop.

### 5.4 Equivalents for the certifications TR0 could not produce

| TR0 gap | Equivalent used, and how | TSR | TR1 |
|---|---|---|---|
| Post-TSR R_F / R_D (visibility, direct) | Not separable from TSR's final colour without new Unreal sun-only and visibility captures (§10). Equivalent: final-colour errors in the shadow-boundary populations TR0 already scores (partial, solid-occluded, contact, solid-lit), same pixels, both engines | rows in the bar | rows above; partial wins at L0, loses at L4 |
| Frozen-film settle | Error at +8 after the stop, and frames to stay within 2 % of each engine's own frame-96 value (`m7-tr1-settle.py`), on unaged populations (every hold frame has a reference) | flat from the stop | §5.3 |
| Connected trail width | TR0's thresholded nearest-edge extent of background-centre error over 1/4/8 codes, every frame; zero pixels bounds any connected trail to the 0–1.5 px band | L4: up to 110 px over 1 code, 41 px away | **0 pixels** at every threshold, frame and exposure |

## 6. Screens: what each mechanism did (M, RT unless noted)

Each screen ran the registered track from cold in one process per path and was scored on T1b's 60 checkpoints (s1–s2 on all 93) against the bar. "Off" is the unreconstructed frame. Wins/losses count bar rows on the scored frames (60-frame screens have 6,458 rows without frames).

| Screen | Change | Edge f64 L0 | Revealed age 3 L0 | Revealed bg age 7 L0 | Hold step L0 | Rows won / lost |
|---|---|---:|---:|---:|---:|---:|
| Off | no reconstruction | 23.38 | 12.05 | 13.88 | 0 | — |
| s1 | T1b-style rules + identity, silhouette bit, spatial start | 25.26 | 13.52 | 23.49 | 1.607 | 10,226 / 11,303 (93 f) |
| s2 | sun-turn angle fix (acos of a rounded unit dot product capped every window near 12), radius-2 start tent | 25.26 | 13.55 | 23.52 | 1.008 | — |
| s3 | other surfaces' silhouette history refused | 24.78 | 23.50 | 13.64 | 1.252 | — |
| s3 | + plain min/max box (all 3×3 samples) | 24.76 | 23.51 | 13.55 | 1.258 | no measurable change |
| s6 | still silhouettes keep own history; radius-1 silhouette start | 24.62 | 23.99 | 12.62 | 1.785 | — |
| s7 | tangent-plane test replaces the 1/cos distance tolerance | 24.62 | 23.99 | 12.62 | 1.778 | no change |
| s8 | **still decided on the CPU** (bit-equal matrices) | 24.62 | 23.98 | 12.50 | **0.243** | 7,099 / 8,400 |
| s8 | + silhouette mixing, window 2 | 24.37 | 14.14 | 16.91 | 0.245 | 7,502 / 7,997 |
| s9 | **+ moving silhouettes blend this frame's bilinear 3×3 estimate** | **19.92** | **10.77** | 17.09 | 0.244 | **8,266 / 7,233** |
| s10 | silhouette window 1 | 19.35 | 13.29 | 10.26 | 0.246 | 8,251 / 7,248 |
| s10 | silhouette start radius 0.75 | 20.22 | 11.95 | 16.35 | 0.245 | 8,053 / 7,446 |
| s10 | Catmull-Rom window 32 | 19.92 | 10.77 | 17.09 | 0.232 | 8,317 / 7,182 (return flicker 4.52 vs 4.42) |
| s10 | blur budget 2 | 19.92 | 10.77 | 17.09 | 0.233 | 8,117 / 7,382 |

s9 is the final setting. Non-RT tracked RT within a few hundredths throughout (s6, s7 and the final run).

### 6.1 Diagnoses that changed the design

- **Still frames.** On the hold, 1 % of pixels carried 90 % of the frame-to-frame step energy, all on placement outlines. Hold cameras are bit-identical, but the GPU's two inlined projections of one point did not round alike, so motion was never exactly zero and silhouettes never converged. The still decision moved to the CPU (`flags.z`).
- **Revealed pixels are moving silhouettes.** T1b's age-3 revealed pool is a few dozen pixels per frame spread over the whole image. Without mixing, rows along the cliff's top edge whose reference is ~65 codes came out exactly 0: a jittered sample just above the edge makes the pixel "background", which continued only background history. Mixing restored ~45–57 codes; the current-frame bilinear estimate (s9) brought the pool from 14.1 to 10.8.
- **Virtual kept counts on jittered holds** (§7.2), which also broke in-process replay.

### 6.2 Box clamp (the design's last option, B)

Measured in s3 on RT: every headline row within 0.02 of the same build without it, and no change in rows won. Identity rejection already removes what the box would; at silhouettes, where the leak is, the 3×3 contains both surfaces and the box spans them. It stays off. With that record, `Reconstruction::VarianceClip`'s trigger (a population still losing after the named fix and the box) is met on edges, revealed and background-centre pixels; per the design that is **trigger input**, not adoption: the replay of saved TR1 buffers on the standard, toon and flat programs that the fallback requires was not run, and US 10,116,916 stays unbuilt.

## 7. Defects found outside TR1

### 7.1 The non-RT Virtual pass exceeds `visibility_limits` (main, since G1 `714a5a7`)

G1 added the radiance-cache table as storage binding 20 of the lighting group. The `N8 visibility ABI` layout now needs **11** storage buffers per compute stage, while `visibility_limits` requests 10, so a no-RT device created at `visibility_limits(Limits::default())` (as `visibility_buffer_tests::no_rt_renderer` does) fails `create_pipeline_layout` on the first sun-shadowed Virtual frame. G1b's non-RT quarry runs were sky-only and never reached that pass. `primary_ray.rs` is outside TR1's files, so TR1's harness asks for 11 on its own no-RT device (`no_rt_renderer_11`). **Owner fix:** raise `visibility_limits` to 11 (or move binding 20 out of the lighting group).

### 7.2 Virtual's still-frame key ignores the projection (fixed for reconstructed frames)

Virtual Surface keeps a pixel's visibility counts on a frame whose key (camera view and field of view, sun, casters, settings) is unchanged. A jittered still frame has the same view but receivers moved by the jitter, so it kept counts computed at an earlier sub-pixel receiver, and non-RT shadow edges stopped integrating over the footprint on holds. The kept state also made in-process replays diverge on the hold. TR1 now clears Virtual's `rolling` flag on reconstructed frames, the existing mechanism other passes use (one condition in `renderer.rs`; no shadow file is edited). Cost: a still non-RT TR1 frame looks up every receiver, as a moving frame does (§6). **Owner option:** key the still test on the full view-projection.

## 8. Cost, memory and determinism

### 8.1 Quiet cost against the half-TSR ceilings (M)

TR0's method: the whole registered Q-walk replayed from a reset per visit, TR1 and the unreconstructed frame alternating (order reversed on odd visits), the first 8 track frames of each visit excluded, increments paired by input frame; 8 pairs = **1,224 frames per method**, plus one unpaired warm-up visit. The span is Renderlab's GPU render span (first pass to present end); the reconstruction pass (sample copy and resolve) has its own timestamps. RTX 4070 SUPER, NVIDIA 617.14, 1920×1080, final build `tr1final`.

**Window `tq2-1`** (06:04:25–06:04:53Z): exclusive lease announced as QUIET, 120 s with no stillkit compiler before it and none at any 3 s sample inside, live wallpaper paused through its supported setting (GPU 2 % at start) and restored, no game. Evidence: `runs/tq2-1/quiet-check.json`, `quiet/tq2-1/`. An earlier window `tq-1` without the wallpaper pause is kept separately (RT increment p50 0.401 ms) and not used.

| Path, ms | p50 | p95 | p99 | Ceiling p50 / p95 / p99 | Verdict |
|---|---:|---:|---:|---|---|
| RT increment over traced + Interactive | **0.377** | 0.815 | 1.009 | 0.727 / 0.769 / 0.816 | p50 passes; **p95, p99 miss** |
| RT total frame with TR1 | **1.253** | 1.709 | 1.878 | 1.618 / 1.651 / 1.709 | p50 passes; **p95, p99 miss** |
| RT reconstruction pass alone | 0.340 | 0.386 | 0.649 | — | — |
| Non-RT increment over visibility buffer + Virtual Surface | **0.458** | 1.474 | 1.699 | 0.781 / 0.819 / 0.853 | p50 passes; **p95, p99 miss** |
| Non-RT total frame with TR1 | 2.280 | 2.668 | 2.782 | 1.616 / 1.646 / 1.679 | **miss** (unreconstructed already 1.849 / 2.047 / 2.183) |
| Non-RT reconstruction pass alone | 0.338 | 0.578 | 0.583 | — | — |

Where the cost is (paired frames): on RT the mean increment, 0.424 ms, is almost all the pass (0.354); in the top 5 % of increments the jittered frame's trace and shading also cost more (+0.19 and +0.26 ms), as jitter and rotated directions change the work per frame. On non-RT the top 5 % are the hold frames 65–84: TR1 makes Virtual look up every jittered receiver (§7.2) while the unreconstructed still frame looks up nothing, about +1.0 ms per still frame. The non-RT total-frame miss is the shadow hillclimb's camera-moving cost before TR1 adds anything. No optimisation pass was run on the resolve: its p50 already meets the ceiling, and the default is blocked on quality (§5). Candidates for one: shade straight into the sample texture (drops the 16.6 MB copy), share the 3×3 identities through workgroup memory, and skip the plane test on Catmull-Rom's outer taps.

### 8.2 Memory (M, allocator report after each run)

| Path | Unreconstructed reserved | TR1 reserved | TR1's own targets | Unreal comparator |
|---|---:|---:|---:|---:|
| RT (traced + Interactive) | 2,414.3 MiB | 2,542.3 MiB | 118.1 MiB | HRT 5,410 MiB (DXGI) |
| Non-RT (visibility buffer + Virtual Surface) | 3,503.7 MiB | 3,695.7 MiB | 118.1 MiB | VSM no-RT 3,912 MiB (DXGI) |

TR1's own targets at 1920×1080: two Rgba32Float colour histories (67.5 MiB), two Rg32Uint anchor sets (33.75 MiB) and the Rgba16Float sample copy (16.9 MiB); the frame state is 304 bytes. Both totals stay below Unreal's, which also excludes TSR's own 200 % history. The 3,504 MiB baseline matches the recorded non-RT figure; the non-RT margin to VSM falls from 408 to 216 MiB.

### 8.3 Determinism and paired identity (M, final build)

| Check | Result |
|---|---|
| In-process replay of the whole track, RT and non-RT | byte-identical, all 93 written frames |
| Cross-process replay (second process, same binary), RT | identical, TR1 and unreconstructed |
| Cross-process replay, non-RT, and across the two final binaries | identical |
| Non-RT path on the RT device (`nonrt-on-rt`) against the no-RT device | **identical on all 93 frames**, TR1 and unreconstructed |
| Unreconstructed RT frames across every build of this unit | identical (s1, s2 and final hashes) |
| Library suite (`--lib`, the final release test binary) | **138 passed, 0 failed, 129 ignored; process exit 0** |
| `cargo fmt --all --check`; Clippy (`--lib --tests`) | clean; no Clippy warning in TR1's files. Pre-existing warnings on main remain in `virtual_shadow_gpu.rs`, `reference_inputs.rs`, `reference_tests.rs` and `visibility_buffer.rs` (outside TR1's ownership) |
| CPU tests of this unit | lattice prefix and symmetry, jittered-projection pixel shift and unjittered bit identity, naga validation of the resolve module (no ray queries) and the rotated trace module, settings validation |

## 9. Hand-offs

- **(a) RT fast setup's eight-ray grain under +4 stops (rt1c).** On the quarry, TR1's rotated directions and history integrate it: partial (penumbra) L4 at frame 64 falls from 28.09 (Interactive 1x) to **21.99**, against TSR Epic HRT's 21.00; the converged hold reaches 21.28 (unaged, frame 96) against TSR's 20.68. That is a large reduction but **not yet a win at +4 stops**, and part of the remaining gap is the converged image's own L4 offset (§5.3). The rt1c S2 witness (frame 0 of the outward phase, a cold start for TR1) was not rerun. So TR1 does not by itself make traced + Interactive the RT default.
- **(b) U1b's specular footprint (776 comparisons).** **Not run.** TR1 integrates the program's output over the jittered footprint, which is the "1-sample footprint" path, but U1b's gate could not be reproduced here: its 4096-stratum reference and per-variant captures were deleted at its delivery (only manifests remain), and its non-RT ("software") suite renders raster primaries, which TR1 does not reconstruct. Next step: regenerate the reference (~104 s GPU recorded), add a TR1 mode to `gpu_specular_aa_motion_screen` that renders each track's indices consecutively after a warm-up with the unfiltered program on traced and visibility-buffer primaries, and score it with `scripts/m7-u1b-score.py` as a new variant against "before". The light track turns 0.005 rad per index, past TR1's 0.004 restart angle, so it would score cold starts.

## 10. Misses and open items

1. **Default not changed:** TR1 loses TSR Epic rows on both paths (9,209 RT, 9,448 non-RT of 22,684): moving silhouettes, revealed pixels, background-centre near edges, and +4-stop converged populations where Renderlab's still image is itself above TSR's (§5.3).
2. **Cost:** p50 increments pass on both paths (RT 0.377 ms, non-RT 0.458 ms against 0.727 / 0.781); every p95 and p99 increment and total misses, and non-RT's total frame misses before TR1 (§8.1). The resolve has not had its optimisation pass.
3. **Tracks not measured:** Q-disocclusion (references exist; TSR's raw capture was deleted, so a new Unreal capture with TR0's `m7-u0` package would be needed for a comparison), Q-sun and every S2G track (S2G references deleted after TR0; 288 states would need regenerating). TR1's sun-turn rule is therefore unmeasured against TSR; at 0.25°/frame (0.0044 rad) it restarts every pixel each frame.
4. **Post-reconstruction R_F/R_D** for TSR would need new Unreal sun-only and visibility-output captures; §5.4 gives the population equivalent instead.
5. **Moving instances restart every pixel** (the key includes transforms). Rigid reprojection through previous transforms and per-pixel caster-motion caps are not built.
6. **Non-RT Virtual directions do not rotate.** TR1 rotates only Interactive's (RT); Virtual's eight directions are fixed per pixel (shadow files). Non-RT penumbrae integrate only through the jittered receivers.
7. **Two-layer silhouette history and the ±1 px reverse search were not built.** The current-frame bilinear estimate at moving silhouettes was the named-fix slot's measured best; the two-layer history is the remaining fix for mechanism 1.
8. **Defects outside TR1** (§7.1 storage-buffer limit; §7.2 Virtual still key) need their owners.
9. Frame 0 of a sequence is no longer the Interactive image (it is the spatial start), so T1a's T-0 identity does not apply to TR1.

## 11. Evidence, reproduction and time

Evidence root `E:/renderlab-work/m7-tr1/`: `plans/` (every manifest), `runs/` (process records, `tr1.json` with hashes and spans, allocator reports), `scores/` (score JSON, per-row comparisons `*-compare.json`, headline tables, settle), `inspect/` (frame 64/128 sheets at L0/L4, edge crops and the hold-step map). Raw frames were deleted after scoring and inspection; their SHA-256 are in each `tr1.json` and score file.

```powershell
py -3 -B scripts/m7-tr1-run.py build --release --beside-leases --as tr1final
py -3 -B scripts/m7-tr1-plan.py final --paths rt,rt-b,nonrt,nonrt-b,nonrt-on-rt --binary tr1final --frames reference --variants '{"a-tr1": {}, "z-off": {"enabled": false}}'
py -3 -B scripts/m7-tr1-wait.py final E:/renderlab-work/m7-tr1/plans/final/plan.json --minutes 30
py -3 -B scripts/m7-tr1-score.py score E:/renderlab-work/m7-tr1/runs/final/rt/data/a-tr1 --case w-e-r-c1 --out rt.json
py -3 -B scripts/m7-tr1-score.py compare rt.json --out rt-compare.json
py -3 -B scripts/m7-tr1-table.py --case w-e-r-c1 TR1=rt.json
py -3 -B scripts/m7-tr1-settle.py --case w-e-r-c1 TR1=rt.json
# Cost: a QUIET window after 120 s without stillkit
py -3 -B scripts/m7-tr1-plan.py quiet2 --paths rt,nonrt --binary tr1final --frames none --visits 8 --no-replay --no-write --variants '{"a-tr1": {}, "z-off": {"enabled": false}}'
py -3 -B scripts/m7-tr1-quiet.py tq2 E:/renderlab-work/m7-tr1/plans/quiet2/plan.json --minutes 10
py -3 -B scripts/m7-tr1-cost.py E:/renderlab-work/m7-tr1/runs/quiet2/rt/data/tr1.json
```

Runs write under `runs/<plan label>/` (the plan's manifests); lease records under `runs/<lease label>/`.

**Time (UTC, 2026-11-08).** Reading and design 02:49–03:00; first implementation, harness and scorer 03:00–03:20 (first release build 2 m 30 s; later incremental release builds about 2–3 min, two jobs, beside other owners' functional leases, never inside a QUIET window). Screens s1–s10 03:23–05:25, final runs 05:20–05:40, quiet windows 05:59 and 06:04, document 06:05–06:20. GPU: 15 leases, **1,789.7 s** in total, all headless and offscreen, released, none inside another owner's QUIET window or a game hold; the two cost windows lasted 29 s and 24 s. CPU scoring: about 3 min per 60-frame candidate and 6 min per 93-frame candidate, two math threads, never inside the quiet windows. No window, viewer, focus change, input automation or desktop capture. No Unreal launch.

**Cleanup.** All 339 raw frame dumps (5.62 GB) were hash-recorded (`deleted-raw.json`) and deleted after scoring and inspection. This unit's Cargo target (`E:/renderlab-work/m7-tr1/target`) is deleted at delivery; staged test binaries are kept with their SHA-256 (`tr1final` `af8bfe12…`).

AA state (IQ1, 2026-11-08; [iq1-quarry-image.md](iq1-quarry-image.md)): this unit's sequences score TR1 variants (and TR1 off) at 16x anisotropy; the shipping default is TR1 off at anisotropy 1. IQ1 measured TR1 against the shipping 1x image on the quarry: better on every population, static and on the Q-walk, on both paths (IQ1 §5).
