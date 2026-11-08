# M7-TR1b: moving silhouettes, the resolve pass and the missing tracks

2026-11-08, branch `t3/m7-tr1b` from main `bc6ba36` (TR1 merged opt-in), main `dc2e3c8` (FIX1) merged. Contract: [M7](milestone-7.md), [TR1](m7-tr1.md), the bar in [TR0](m7-tr0.md) (`E:/renderlab-work/m7-tr0/bar/tr1-bar.json`), [P0](m7-p0-claims.md).

**Status (overnight push, 2026-11-08 evening): TR1 is the default on both paths, with `Reconstruction::off()` as the named switch and quality (2/3) and performance (1/2) upscaling presets (§4.6).** Islam's overnight rules replaced the zero-loss gate below; TR1's remaining losses to no-AA and its gaps to TSR are listed in [m7-gaps.md](m7-gaps.md) under TR1, and the default is never presented as meeting the Unreal bar.

Earlier the same day TR1b stayed opt-in: the default-AA ruling's condition 3 (TR1 no worse than no-AA on every population of the visible-sky quarry walk, both paths) **fails** (§4.5), so the default was not flipped and the decision is reported back. Against the path-traced reference TR1b is worse than no-AA in 149 of 256 pooled rows on both paths, almost all from a lighting error both images share in this backlit view. Against the renderer's own pixel integral (a lighting-matched control) it is better in 219 of 256, but still worse in 31: its cold first frame, small revealed-pixel pockets, single-pixel maxima, 0.02 codes of sky and the still-hold convergence step. Against TSR it beats Q-disocclusion's revealed pixels, Q-sun-fast's flicker and most S2G rows (§4), still misses moving edges and background-near-edge pixels on Q-walk, and fails U1b's gate (§7.2).

Labels: **M** measured here, **R** retained evidence read here. All GPU runs are headless, under the shared lock and m7-tr1's own leases; RTX 4070 SUPER, NVIDIA 617.14, 1920×1080, 1x.

## 1. What changed from TR1

`Reconstruction` keeps TR1's settings and adds none; the TR1b behaviour below is unconditional when reconstruction is on. TR1's `silhouette_window`, `silhouette_start_radius`, `silhouette_mix`, `silhouette_spatial` and `box_clamp` are gone with the mixed silhouette history they tuned; `coverage_window` (16) replaces `silhouette_window`.

1. **Every colour history is one surface's.** A silhouette pixel (its jittered 3×3 shows another surface) keeps a second slot (`Rgba32Uint`, 16 B/px, ping-ponged): the other surface's placement and side, its exposed mean and weight (f16), and the occluder's coverage of the pixel footprint with its weight (f16). The film is `mix(back, front, coverage)`. The occluder is never the background; between two hits it is the nearer sample.
2. **Coverage moves with the occluder.** It is fetched where the occluder's sample point was in the previous frame and updated with this frame's sample (1 when it shows the occluder). Under motion, partly covered taps carry it as a straight edge: the edge normal is the coverage gradient over the 2×2 taps, each tap's coverage becomes a signed edge distance through the exact box-area inverse, the distances are moved to the fetch point, averaged bilinearly and turned back into box area. A moving straight edge is therefore not resampled, and the coverage window under motion is `coverage_window` at any sub-pixel offset. (Box-area function and inverse checked against Monte Carlo: max error 1.4e-4; inverse round trip 1e-16.)
3. **Still silhouettes keep their two surfaces.** On a still view (CPU-decided, FIX1's check) a pixel with a stored pair keeps it whatever this frame's jittered 3×3 shows; a sample of a third surface carries the pixel's state unchanged (the two-layer limit, §2.2).
4. **A hidden surface carries at most the start window.** Under motion the slot of the surface a pixel no longer samples is only resampled, never sampled; when it is revealed it restarts within 4 samples' weight.
5. **The 3×3 identities are loaded once per workgroup** (a 10×10 tile in workgroup memory) instead of nine hit-record loads per pixel, and Catmull-Rom reuses the four bilinear taps instead of testing them again.

Interiors are unchanged from TR1: one history per surface, continued from either slot of the previous frame, Catmull-Rom with history-only deringing, T1b's blur budget, the tangent-plane test, spatial starts for revealed pixels, engine events, rotated Interactive directions (RT).

### 1.1 Files

| File | Change |
|---|---|
| `shaders/temporal_resolve.wgsl` | second slot, coverage, straight-edge transport, still pairs, hidden cap, workgroup tile |
| `src/temporal_reconstruction.rs` | layer targets, settings (`coverage_window`), bind group 47/48 |
| `src/temporal_reconstruction_tests.rs` | quarry and bench (`scene`: S2G) content, per-frame sun (`to_light`), FIX1's devices |
| `src/specular_aa_tests.rs` | U1b screen: `*-tr1` mode (hand-off b) |
| `src/motion_bench_tests.rs` | motion bench: `TR1`, `TR1-warm` modes (hand-off a) |
| `scripts/m7-tr1b-refs.py` | the m7-refs set: Q-disocclusion ages and compaction, S2G and Q-sun regeneration, TSR captures |
| `scripts/m7-tr1-score.py`, `m7-tr1-plan.py`, `m7-tr1-wait.py`, `rt1c-witness.py` | every track; compact references; `compare --against`; `pooled` (the sky gate); schedule write locks; witness modes |
| `shaders/program_traced_entry.wgsl`, `src/environment.rs`, `src/environment_gpu.rs`, `src/primary_ray_gpu.rs`, `src/film_accumulation.rs` | opt-in visible sky (§4.5): `Renderer::set_visible_environment`, traced and visibility-buffer misses sample the bound RLE2 map; default off, T1a unchanged (its seam texts follow the new miss store) |
| `src/reference_quarry_tests.rs` | an explicit `camera_background: null` leaves primary misses on the environment; every other manifest unchanged |

No shadow, GI/cache, shader-reload or pipeline source is edited. `renderer.rs` has TR1b's marked overnight edits only (§4.6): the Temporal-shadow jitter condition, the upscale field, the internal-target substitution and the present redirect. Test harnesses of other units changed only to switch TR1 off explicitly or to restart it with `off()` (§4.6). The primary-miss branches were IQ1's and passed to TR1b after IQ1 merged; the visible-sky flag moved to `PrimaryFrame.sided.w` when RT1d took `sided.z`.

## 2. Moving silhouettes (item 1, M)

### 2.1 Screens

Q-walk, T1b's 60 scored checkpoints, TSR Epic rows of the same frames (22,684 rows; 6,458 rows of other frames absent on every line). RT: traced + Interactive against TSR Epic HRT (`w-e-r-c1`); non-RT: visibility buffer + Virtual against TSR Epic no-RT (`w-e-n-c2`).

| Variant (RT) | Wins | Losses | Decision |
|---|---:|---:|---|
| TR1 (mixed silhouette history, `e-mix2s`) | 8,266 | 7,233 | baseline |
| Two-layer history, coverage window 16 | 8,387 | 7,112 | built |
| … coverage window 8 / 32 | 8,403 / 8,386 | 7,096 / 7,113 | 16 kept (§2.3) |
| … sticky still pairs (`a-sticky`) | 8,362 | 7,137 | hold step 0.375 (regression, §2.2) |
| … ±1 px reverse search | 8,251 | 7,248 | rejected |
| … coverage observed as the 3×3 share | 8,244 | 7,255 | rejected |
| … stable still pairs | 8,371 | 7,128 | kept: hold step 0.221 |
| … + straight-edge coverage transport | **8,613** | **6,886** | kept |
| … + coverage window 32 | 8,433 | 7,066 | rejected |
| … + own-sample coverage start | 8,611 | 6,888 | rejected (no gain) |
| … + hidden-surface cap | **8,632** | **6,867** | **kept (TR1b)** |
| … + 5×5 spatial start / start window 8 | 8,608 / 8,634 | 6,891 / 6,865 | rejected (start-age-0 loss / +2 rows) |
| … + Catmull-Rom outer taps by identity only | — | — | rejected: same cost (§3) |

| Variant (non-RT) | Wins | Losses |
|---|---:|---:|
| Two-layer, sticky (`a-sticky`) | 8,163 | 7,333 |
| **TR1b** | **8,441** | **7,055** |

The consolidated shader reproduces the screened `k-hidden` variant byte for byte (60/60 frames). Replays are byte-identical.

### 2.2 The hold-step regression and its fix

The first two-layer history stepped 0.375/0.594 codes (L0/L4) between held frames 80→81, against TR1's 0.244/0.288. The step map put 69 % (L0) and 83 % (L4) of the energy on the 11,421 edge pixels, and **89 % on the 210 pixels whose 3×3 shows three or more surfaces** (step RMS 13.6–39.7 codes; two-surface edges 0.60). On a held view the jitter changes which other surface the 3×3 shows; replacing the second slot discarded that surface's history and the coverage, every frame. Keeping a still pixel's stored pair (§1 item 3) brings the step to **0.221/0.262**, below TR1 and TSR (0.432/0.478). The cost is a bias at three-surface corners on long holds: their third surface's samples are not integrated.

### 2.3 Headline rows (RT, Q-walk checkpoints)

| Row | TSR Epic HRT | T1b | TR1 | TR1b |
|---|---:|---:|---:|---:|
| Edge, frame 64, L0 / L4 | 17.81 / 31.88 | 23.57 / 50.5 | 19.92 / 40.83 | **17.98** / **31.77** |
| Background-centre, frame 64, L0 / L4 | 0.371 / 1.168 | 1.584 / 5.456 | 0.901 / 2.699 | 0.419 / 1.195 |
| Edge, frame 160, L0 / L4 | 16.75 / 28.94 | 22.59 / 47.98 | 19.83 / 41.15 | 16.89 / 29.97 |
| Hold 80→81 step, L0 / L4 | 0.432 / 0.478 | 0.277 / 0.331 | 0.244 / 0.288 | **0.221 / 0.262** |
| Revealed age 3, all-covered, L0 / L4 | 7.70 / 12.06 | 22.24 / 55.54 | 10.77 / 17.66 | 9.64 / 16.63 |
| Revealed age 7, all-covered, L0 | 7.76 | 15.97 | 9.71 | 9.66 |
| Revealed age 7, background-centre, L0 / L4 | 4.75 / 15.93 | 43.51 / 121.5 | 17.09 / 51.68 | 5.55 / 18.35 |
| All-covered, frame 64, L0 / L4 | 11.24 / 16.67 | 10.91 / 17.59 | 10.89 / 17.13 | **10.89** / 17.13 |
| Partial, frame 64, L0 / L4 | 13.50 / 21.00 | 13.94 / 22.55 | 13.91 / 21.94 | 13.91 / 21.94 |
| Return 127→128 flicker, L0 | 3.425 | 4.492 | 4.42 | 4.36 |

Edge rows reach parity at frame 64 (L4 wins, L0 within 1 %); background-centre falls from 2.4× to 1.13× TSR; holds beat TSR. **Revealed pixels remain the miss.**

### 2.4 Why revealed pixels still lose

Q-walk's revealed pixels are few (about 110–165 per checkpoint) and almost all lie within 1.5 px of a reference edge. Frames 64–96 share one camera, so frame 96 shows the same pixels converged: frame 64's age-3 revealed pixels have RMS 12.0 in motion and 9.2, 8.3, 7.5 at frames 72, 80, 96 (signed −8.5 → −5.6). The decay follows 1/√n towards a ~7-code floor: the reconstruction part is sampling noise in young history (Interactive's eight rays per frame), not leakage or bias; the floor and most of the signed offset are the converged image's own error there (converged edge-band pixels: −0.4 signed, RMS 18). TSR's inputs are denoised before accumulation. A wider spatial start (5×5, start window 8) did not move the rows (9.62) and blurred track starts. Lowering this noise needs denoised or more-sampled young history; colour-statistic weights or SVGF-style filters are excluded by P0 (§5).

## 3. The resolve pass (item 2)

### 3.1 Optimisation (M, paired processes)

The pass is copy + resolve. Workgroup-tile identities and Catmull-Rom inner-tap reuse (byte-identical output) against the screened shader, alternating processes A/B/A/B, the unreconstructed span equal in all four (0.86 ms p50), 459 frames each:

| RT pass, ms | mean | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Before (`k-hidden`, per-pixel 3×3 loads) | 0.527 / 0.541 | 0.461 / 0.463 | 0.902 / 0.946 | 0.992 / 1.078 |
| **TR1b** | **0.478 / 0.476** | **0.405 / 0.404** | 0.862 / 0.868 | 0.943 / 0.980 |
| + outer Catmull-Rom taps without the plane test | 0.476 / 0.468 | 0.403 / 0.402 | 0.867 / 0.861 | 0.983 / 0.994 |

The tile cuts the pass by 12 % at the median. Skipping the outer plane test saved nothing and changed 59 frames, so it is not kept. These screen tails are contended: individual visits of single frames double in both the pass and the shading pass at once (the intermittent wallpaper load), so p95/p99 are taken only in the quiet window (§3.2). Not done, and why: shading straight into the sample texture (drops the 16.6 MB copy) needs the primary-ray shading pass to write a second target, a primary-ray file outside TR1b's files; a ping-pong of the HDR target would move present and tonemap bindings in `renderer.rs`, more than a minimal shared edit. Reading receivers' world/normal instead of rebuilding the hit point is impossible on the Interactive path, which writes hit records only.

### 3.2 Quiet cost against the half-TSR ceilings

**Not yet re-timed with TR1b's final binary.** TR1's quiet measurement ([m7-tr1.md](m7-tr1.md) §8.1, **R**) stands as the recorded cost: p50 increments pass (RT 0.377 ms against 0.727, non-RT 0.458 against 0.781), and every p95/p99 increment and total misses (RT increment p95 0.815 / p99 1.009 against 0.769 / 0.816). TR1b's pass is 12 % cheaper at p50 in the paired screens (§3.1). The quiet re-time of both paths, including the non-RT still frames after FIX1, is the next GPU step; until it passes, the tails stay recorded as a miss (the ruling's condition 4).

## 4. The missing tracks (item 3)

### 4.1 References in `E:/renderlab-work/m7-refs/` (M)

Compact format `m7-refs-compact-v1` (`scripts/m7-tr1b-refs.py`): L0/L1/D0/D1 as float16 already exposed at the track's registered exposure, V0/V1/visibility float16, the centre placement (`hit0`), every population mask unchanged. On a Q-disocclusion state it moves scored mean codes by **0.005 RMS (0.02 max)** and leaves split-half noise unchanged (0.4153 → 0.4154 codes, L0); the scorer reproduces Q-walk's rows exactly through the new code path. Geometry ages are generated from the full archives before compaction.

| Set | States | Source and identity | Size |
|---|---:|---|---:|
| Q-disocclusion | 48 | U0b's accepted `quarry-final` archives (lossless, provenance hashes checked), compacted; TR0's fixed geometry ages (`m7-tr0-s2g.py advance_age`: nearest previous centre, same instance, 2 % depth) generated first | 365 MB |
| S2G (four tracks, 640 frames) | 288 | U0b's qualified D8 tracer, each state's scene built exactly as TR0's `reference` built it (scene SHA-256 equal to TR0's record). A state is accepted when every retained array is **byte-identical to TR0's qualified corpus** (recorded per-array SHA-256), or, traced alone, when its own independent-rebuild replay is bit-identical (TR0's per-state criterion): D8 differs across AS rebuilds by ≤2 ULP on some runs (bounded-AS). TR0's ages (`m7-tr0/s2g/ages`) are reused. All 288 states ended byte-identical to TR0's corpus (solo retries were needed only to get there) | 2.9 GB |
| Q-sun-fast | 32 | quarry-final's tracer (`997151eb`) and replay policy (`bounded-as-v1`), one lighting file per sun state; static-camera ages (TR0's identical-view rule) | 240 MB |
| TSR Epic captures | 4 | TR0's `run` with U0's original package (`69d6f88a…`, the one TR0's Q-walk captures used): Q-disocclusion and Q-sun-fast, HRT and VSM no-RT. Kept as lossless native frames (R11G11B10 values, exact in float16, checked) plus TR0's per-frame exposure factor; raw dumps deleted | 319 MB |
| U1b screen and references | 25 + 25 frames ×2 | §7.2 | 36 MB |

Q-sun-jump is not regenerated: TR0's registered input holds one sun direction for all 33 frames (the 5° step is missing), so it is a static hold already covered by Q-walk frame 0's reference; recorded as an input defect for TR0's owner. Q-sun-slow had no TSR capture in TR0 and is not part of this item. The S2G compaction moves a few top-5 % highlight-mask pixels at the threshold (24 of 40,868 S2G population rows per track and path have pixel counts one or two off TSR's); every other population count matches.

### 4.2 Scores, every track, both paths (M)

Every row of every registered frame, lower is better, a missing row never passes. Q-walk and S2G compare with TR0's bar (TSR Epic rows scored by TR0 from f32 references); Q-disocclusion and Q-sun-fast compare with the new TSR captures scored by this scorer from the same compact references and ages (`compare --against`). Population pixel counts equal TSR's on every shared row except the 24 S2G highlight rows per track and path noted in §4.1.

| Track (frames) | Path | Rows | TR1b wins | Losses | Ties | TR1 wins / losses |
|---|---|---:|---:|---:|---:|---|
| Q-walk (93 references) | RT | 22,684 | **12,716** | 8,811 | 1,157 | 12,320 / 9,209 |
| | non-RT | 22,684 | **12,468** | 9,056 | 1,160 | 12,078 / 9,448 (pre-FIX1) |
| Q-disocclusion (48) | RT | 11,628 | **5,835** | 4,913 | 880 | not run |
| | non-RT | 11,628 | **5,791** | 4,957 | 880 | |
| Q-sun-fast (64) | RT | 15,402 | **8,428** | 5,630 | 1,344 | not run (TR1's 0.004 turn: 7,371 / 6,687) |
| | non-RT | 15,402 | **8,264** | 5,794 | 1,344 | |
| S2G camera 0.25 px (160) | RT | 46,292 | **37,235** | 6,471 | 2,586 | not run |
| | non-RT | 46,292 | **31,234** | 12,376 | 2,682 | |
| S2G camera 0.5 px (160) | RT | 46,536 | **38,592** | 6,151 | 1,793 | |
| | non-RT | 46,536 | **32,703** | 11,949 | 1,884 | |
| S2G camera 1 px (160) | RT | 46,540 | **38,584** | 6,200 | 1,756 | |
| | non-RT | 46,540 | **32,951** | 11,757 | 1,832 | |
| S2G sun 0.25° (160) | RT | 45,904 | **27,195** | 15,059 | 3,650 | |
| | non-RT | 45,904 | 15,476 | **26,772** | 3,656 | |

### 4.3 What the tracks show

- **Q-disocclusion** (2 cm/frame sideways): revealed pixels **beat TSR at every pooled age** on both paths (all-covered, L0: age 1 24.70 vs 25.13, age 3 25.04 vs 26.25, age 7 25.03 vs 26.70; L4 age 3 46.9 vs 48.8). Edge rows are level (frame 24: 17.35 vs 17.26 L0, 31.12 vs 31.18 L4). Misses: revealed background-centre pixels (1.86 vs 1.41) and the converged L4 interior (frame 47: 18.30 vs 17.78), and the largest net-loss groups are shading populations (multi-surface, backlit, isolated, partial). The end-of-track step is lower than TSR's (11.8 vs 14.2).
- **Q-sun-fast** (10°/s for 32 frames, then a hold): TR1's 0.004-rad turn cap restarted every pixel each moving frame (Q-sun turns 0.0029 rad/frame, a window of 1.4): moving-sun flicker 3.36 against TSR's 0.42. A screen of the cap (0.016, 0.032, 0.064, 0.256 rad; RT) peaked at 0.064 and stayed flat beyond it; **TR1b's default is 0.064** (about 22 frames at this speed). Flicker now beats TSR (0.367 vs 0.416 at frame 16; 0.248 vs 0.376 at 24), the hold step too (0.147 vs 0.347), and moving penumbrae at L0 (11.69 vs 12.03). Q-walk, Q-disocclusion and the S2G camera tracks have no sun turn: their frames are byte-identical under the new default (Q-walk checked, 93/93 frames, both paths). Remaining losses are L4 rows whose converged hold loses too (frame 63, lit: 10.96 vs 9.86; lit signed mean L0 4.63 vs 3.16) — the lighting gap (§4.4).
- **S2G** (glossy metal r0.1 and dielectric r0.2 reliefs, camera at 0.25/0.5/1 px per frame): TR1b wins about 80 % of rows on RT and 70 % on non-RT. On the 1 px track (RT; non-RT in brackets) median highlight-energy error 0.23 vs TSR's 0.52 (0.24 vs 0.90) on metal and 0.11 vs 0.29 (0.11 vs 0.24) on dielectric; metal centroid drift 6.4 vs 8.9 px (6.4 vs 17.4); same-instance flicker on metal 1.79 vs 4.38 (1.78 vs 1.91). Dielectric centroid drift on RT is the exception (2.11 vs 1.47). The largest net losses are L4 signed means and backlit pixels (RT) and occluded/faint populations (non-RT, Virtual shadows): converged-image offsets, not history.
- **S2G sun 0.25°/frame:** RT wins 59 % of rows. **Non-RT loses** (15,476 wins, 26,772 losses): its net-loss groups are occluded, solid-occluded and faint populations, lost on every frame including converged ones (occluded L4 24.8 vs 22.8 at frame 59; occluded signed mean L0 9.5 vs 7.1 at frame 125). That is the Virtual path's shadowed shading under a turning sun, not history; RT's net losses are L4 signed means, as on the camera tracks.

### 4.4 The +4-stop gap is lighting

At Q-walk frame 96, 32 still frames after the walk stops, history has converged (hold step 0.22 codes), and TR1b still loses lit L4 (11.97 vs 11.21) and partial L4 (21.26 vs 20.68) while winning the same populations at L0 (7.78 vs 8.54; 13.03 vs 13.32). Lit surfaces carry a 4.4-code signed offset at L0 (TSR 3.3) that +4 stops magnify. Q-sun's hold (frame 63) and Q-disocclusion's last frame show the same. These rows are recorded as **lighting**, the converged still image's own error, not reconstruction; they account for most remaining losses on every track (L4 `rms_noise_subtracted` and `absolute_signed_mean` groups).

### 4.5 The visible-sky default gate (M)

The ruling ([milestone-7.md](milestone-7.md), default anti-aliasing, condition 3): before TR1 becomes the default, it must be no worse than no-AA on every population of a quarry walk with the real sky visible behind silhouettes, on both paths.

**Visible sky (opt-in).** `Renderer::set_visible_environment(true)` makes traced and visibility-buffer primary misses sample the selected RLE2 map along the pixel's ray at level 0, exposed as the lighting it provides (`primary_miss` in `program_traced_entry.wgsl`; flag in `PrimaryFrame.sided.w`). Default off: every existing image is unchanged. TR1b reconstructs sky samples by direction when it is on (`tr1.background.w = 0`). T1a's film path still stores the constant clear. **Open question for Islam:** whether a visible sky should become the shipping default (now the backdrop is the lit clear colour).

**Walk.** `D:/Coding/renderlab-work/m7-refs-sky/sky-walk.json`: 40 frames on the quarry, frames 0–7 still, 8–31 moving 1 cm/frame sideways, 32–39 still (25 distinct states). The camera looks 4.69° from the sky's sun axis (measured by UE-SKY on the saved walk; an earlier estimate here said 8°): about 215,000 sky pixels are above 1.0 exposed (the RLE2 map's sun disk), and silhouettes cross it. The view is backlit: 935,142 of 938,456 covered pixels are shadowed.

**References.** quarry-final's protocol (quarry tracer, 1024 spp per pixel-integrated frame, zero shutter, the quarry's sun and RLE2 environment, `bounded-as-v1`) with `camera_background: null`, traced by this branch's test binary (`tr1b-k`, SHA-256 `cfa3b51a…`, pinned in every reference): 19 states accepted by strict replay, 6 by bounded-AS. TR0's geometry ages. Compact v1, 530 MB on `D:/Coding/renderlab-work/m7-refs-sky`.

**Identity of an existing manifest (q-disocclusion-000, black backdrop).** quarry-final's binary reproduces the retained archive byte for byte on 3 of 3 runs (strict replays). This branch's binary does not: geometry, visibility, D0/D1 and every population mask are byte-identical, but L0 differs in 43–50 of 2.07 M pixels by at most 2 ULP (RMS 6.6e-10 relative) on all 6 runs, every run accepted only as bounded-AS, and its runs differ from each other by 1–56 values. **The request to show bit-identity is not met**; the difference is within the accepted bounded-AS envelope, and its source (code merged from main since `997151eb`, or the null-backdrop edit) is not isolated.

**Result against the path-traced reference** (`m7-tr1-score.py pooled`, every row pooled per segment; RT and non-RT agree within one row):

| Segment | Rows | TR1b better | TR1b worse |
|---|---:|---:|---:|
| all 40 frames | 256 | 101 | **149** |
| first still (0–7) | 166 | 74 | 86 |
| moving (8–31) | 172 | 85 | 81 |
| last still (32–39) | 256 | 91 | 159 |
| aged revealed/start pools | 626 | 342 | 284 |

**Why, measured.** Both images carry a lighting error the reference does not: on shadowed pixels 50 px or more from the sky, the reference reads 2.6e-6 exposed luma and both renderer images 9.5e-3 (frame 20; TR1b 9.47e-3, no-AA 9.50e-3, equal in linear light). That is the unshadowed environment term IQ1 measured as the quarry's 28-code occluded population, here covering 99.6 % of covered pixels. Against a reference that dark:
- integrating a pixel raises its mean code under the concave display curve, so TR1b's covered RMS at +4 stops and its signed means count as worse (all-covered L4: 85.7 vs 83.9, first still);
- 99.9 % of TR1b's sky error is in sky pixels touching an edge, where it integrates the over-bright rock the footprint partly covers (TR1b ≈ 0.05 exposed, reference and no-AA ≈ 1e-6: the reference's rock is as dark as its sky);
- the 741 covered pixels next to the sun disk read 3.37 in the reference, 4.25 in TR1b and 0.028 in no-AA, whose centre sample misses the sun.

**Lighting-matched control.** Each path's own pixel integral (`gpu_tr1_sequence` with `integrate: 8`: 64 stratified jittered no-AA frames per state, same binary source) replaces L0/L1 in the references, populations and ages unchanged:

| Segment | Rows | TR1b better | TR1b worse (RT / non-RT) |
|---|---:|---:|---:|
| all 40 frames | 256 | 219 | 31 / 31 |
| first still | 166 | 137 / 133 | 22 / 26 |
| moving | 172 | 143 | 23 |
| last still | 256 | 228 / 229 | 22 / 21 |
| aged pools | 626 | 553 / 549 | 73 / 77 |

TR1b's covered-population RMS is lower in every segment except the first still frames' solid-occluded and single-surface at L4 (11.15 vs 10.91). Moving edges: 11.3 vs 35.4 codes (L0), 16.5 vs 39.9 (L4); revealed all-covered at age 0: 12.1 vs 29.0. **What still loses, and is TR1b's:**
1. **The cold first frame**, a spatial start for every pixel: all-covered 15.3 vs 12.0 at L4 (5.65 vs 5.13 at L0). At frame 1 all-covered already wins at L0 (4.77 vs 5.13), but 10 start rows of age 1 still lose.
2. **Small revealed pockets** at ages 2–4: lit 50.0 vs 23.7 and isolated 28.9 vs 13.7 at L4, age 2.
3. **Single-pixel maxima** while moving (lit L4 210 vs 80): a restarted pixel whose jittered sample fell in a dark crease (frame 10, pixel 579,745).
4. **The sky**, 0.020 vs 0.004 codes RMS far from edges.
5. **The still-hold step** (0.80 codes at L0) against no-AA's exact zero while converging; signed means of the first still frames.

**Decision: the default is not flipped** (condition 3 fails on both references). TR1b's revealed-pixel and cold-start rows are the work that continues. The TSR Epic capture on this walk was not made: U0's package has a black backdrop and a specified lighting cube, so a visible sky needs a new quarry level with an unlit sky sphere sampling the RLE2 cube, cooked as a separate package as U0b did for SV (`m7-u0b-sv-prepare.py`). UE-SKY has since captured TSR on this walk ([ue-sky.md](ue-sky.md); rows in [m7-gaps.md](m7-gaps.md)).

### 4.6 Overnight pass: the default, cheap fixes and upscaling presets (M)

Islam's overnight ruling (AGENTS.md, 2026-11-08): TR1 becomes the default on both paths now, with a named off-switch; fixing the losing rows continues only where it is cheap; upscaling ships as a quality preset.

**Default.** `Reconstruction::default()` is enabled; `Reconstruction::off()` restores unjittered single frames with no history; `identity()` equals the default. Temporal soft-shadow frames are no longer jittered (they keep their own film history and never reconstruct; one marked condition in `renderer.rs`). The exact-repeat certifications switch TR1 off explicitly: the shared RT and no-RT test renderers (`fresh_shadow_tests.rs renderer_with_features`, which M6 film, RT1, N8, the quarry frame loop and primary-ray identity use; `visibility_buffer_tests.rs no_rt_renderer`), the traced suite renderer (`tests.rs`, U1b's hardware mode), R0b's program-primary test and IQ1's renderer (its 1x rows stay 1x). Harness restarts that used `default()` to mean off now use `off()` (IQ1, motion bench, U1b's `*-tr1` mode, TR1's harness, G2's `tr1xN` and `pan-tr1` rows, which also force a cold start before each TR1 run). G2's other gate rows now render the TR1 default; its SV rig's exact-repeat booleans (`image_repeat_equal` and the staleness checks) will see the jitter unless that rig switches TR1 off.

**Cheap fixes, against each path's own 64-spp integral on the visible-sky walk** (worse rows of 256 pooled, RT; segment counts still start / moving / still end / aged pools):

| Change | All | Segments |
|---|---:|---|
| TR1b as gated (`tr1b-k`) | 31 | 22 / 23 / 22 / 73 |
| Point-symmetric jitter pairs (lattice y shift 91, pairs in T1's order, centre at k = 0 and 255) | 32 | 20 / 23 / 22 / 75 |
| + spatial start at tent radius 1 (bilinear interpolation to the centre; radius-2 fallback when the tent misses the other surface) | 32 | 4 / 23 / 22 / 44 |
| + exact single layer at whole coverage, + sub-1e-3-px fetch snap (the sky under translation was resampled with Catmull-Rom's 16-sample window) | **26** | **4 / 18 / 21 / 37** |
| + pending pairs (hold a pair's first sample while still) | 26 | 2 / 18 / 21 / 43: rejected |

The default keeps the fourth row (non-RT: 26; 8 / 16 / 23 / 37). The cold first frame now equals no-AA bit for bit on both paths (k = 0 is unjittered; radius 1 starts from the own sample; whole-coverage pixels take one layer exactly, where `mix` was off by up to 48 f16 ULP at bright-sky silhouettes). What remains, by kind, is in m7-gaps.md. Radius 1 without the fallback left a revealed surface black when it appeared only at the 3×3's far side (revealed sky, age 0: 37 vs 8.5 codes); the fallback fixes it. Q-walk, Q-disocclusion, Q-sun and S2G were not re-scored with these defaults.

**Upscaling presets.** `RenderScale::{Native, Quality, Performance}` on `Reconstruction::scale` (`Reconstruction::quality()`, `performance()`): a reconstructed frame renders every pass at 2/3 or 1/2 of the target per axis into an internal colour target (`renderer.rs`: one marked substitution after the jitter, and present reads the reconstruction's output); TR1's histories and resolve run at the target size. Each target pixel takes the internal sample nearest its centre, weighted by a one-pixel tent on its offset (zero: the pixel keeps its history this frame); the spatial start, coverage share and anchor rebuild use each sample's own offset. At native scale every mapping is the identity and every weight exactly 1, so native frames are unchanged. Raster, diagnostic and unreconstructed frames always render at the target size.

Inspected (`D:/Coding/renderlab-work/m7-tr1b/inspect/presets`, both paths): Q-walk frames 8, 40, 64, 96 and S2G (0.5 px/frame) frames 0, 16, 80, 159; off, native, quality, performance. All four register identically (no scale or offset error); quality and performance are slightly softer than native on the quarry and show no trails or holes in motion; on S2G's moving glossy reliefs both presets show stepped aliasing on the finest specular stripes that native does not.

One timing run (functional lease, not QUIET; Q-walk, 1920×1080 target, 2 visits, 322 frames each; GPU frame span, ms):

| Path | No AA p50 / p95 | TR1 native | Quality | Performance |
|---|---:|---:|---:|---:|
| RT (traced + Interactive) | 1.03 / 1.54 | 1.47 / 2.20 | **0.93 / 1.41** | **0.73 / 1.01** |
| Non-RT (visibility buffer + Virtual) | 1.90 / 2.66 | 2.37 / 3.27 | **1.71 / 2.44** | **1.45 / 2.02** |

The resolve pass itself (p50): RT 0.405 native, 0.289 quality, 0.249 performance. Lib suite (release test binary `tr1b-q`, inside a lease): **148 passed, 0 failed, 144 ignored**.

## 5. Claim-avoidance notes (engineering reading, not clearance)

TR1's reading ([m7-tr1.md](m7-tr1.md) §2) holds; what TR1b adds:

| Claim | TR1b has | TR1b lacks, and why |
|---|---|---|
| US 10,116,916 / 10,362,289 | — | no neighbourhood colour box or statistic: coverage is an observed hit fraction of the pixel's own samples, the edge normal a gradient of stored coverages, never of colour |
| History-clamp family (fast/short histories) | a second slot | it is another surface's single history; one value per surface per pixel, no second list of the same surface, no clamp of one by the other |
| US 11,847,737 | per-slot windows | windows from resampling geometry and events only; the coverage window is fixed, not derived from pixel values |
| US 10,600,167 (W5) | slots, coverages | read only by `tr1_resolve`, next frame; no other stage binds them |
| US 12,437,371 / 2026/0162231 | — | no lighting difference, own-pixel moments or confidence |

Not used: variance clipping, SVGF, temporal gradients, own-pixel moments, colour or luminance rejection.

## 6. Programmability

Unchanged from TR1: programs shade the jittered hit; acceptance, coverage and weights never read colour, so any program converges to the footprint integral of its own output, edges included (the coverage is the fraction of the program's own samples that hit each surface). Constraint added: at a corner where three or more surfaces meet, a still pixel integrates only its two stored surfaces (§2.2).

## 7. Hand-offs

### 7.1 (a) rt1c's S2 witness at +4 stops (M)

`motion_bench_tests::gpu_motion_bench_batch` on rt1c's witness scene (S2 `camera-0.25px-r0.04`, frame 0 of the outward phase, crop [704,480,1216,768]), rt1c's own scorer (`scripts/rt1c-witness.py`, unchanged metric). The regenerated film reference has **zero differing channels** from rt1c's; the Interactive image reproduces rt1c's numbers exactly.

| Crop display RMS, codes | +0 stops | +4 stops |
|---|---:|---:|
| Recorded Unreal HRT1 | 7.347 | 13.665 |
| Interactive (rt1c) | 5.518 | 14.083 (miss) |
| **TR1b, cold** (frame 0 is a restart) | **5.169** | **12.788** (pass) |
| **TR1b, after 256 still frames** at the pose | **0.354** | **0.834** (pass) |

The witness passes from cold: the spatial start's 3×3 tent integrates the boundary pixels where rt1c's loss lay. This is one witness frame, not the full S2 motion audit.

### 7.2 (b) U1b's 776-comparison gate (M)

Regenerated into `E:/renderlab-work/m7-refs/u1b` with U1b's own screen (`gpu_specular_aa_motion_screen`, 320×180 fixture, its unchanged scorer `scripts/m7-u1b-score.py`, both references: 256 strata and 4096), Vulkan validation active with **zero errors** in all six processes. A new `*-tr1` mode renders the unfiltered "before" program with TR1b on traced (RT suite) or visibility-buffer (no-RT device) primaries: each track from cold, 256 frames of its first index, then its indices consecutively. **Reproduction:** all 400 regenerated "before" rows equal U1b's recorded rows exactly against both references. (This machine's registry lists a missing PRISM capture-hook layer whose loader errors fail validated runs; the runs set `VK_IMPLICIT_LAYER_PATH` to an empty directory, which skips implicit overlay layers and keeps the validation layer.)

| TR1b against "before", failed comparisons of 776 | Non-RT | RT |
|---|---:|---:|
| 4096-stratum reference | **458** | **460** |
| 256-stratum reference | 461 | 462 |

**TR1b fails U1b's gate.** By track (4096 reference, non-RT): static 5, camera 220, light 233. Where it wins, it wins by large factors: static geometric r0.08 RMS 0.30 against 28.9, full frame 0.36 against 4.62; camera geometric r0.08 14.5 against 34.4. It fails for two structural reasons:

1. **View-dependent shading lags under camera motion.** The flat-control patch is metal at roughness 0.08: its reference radiance falls 25 % over the camera track (0.0171 → 0.0129) as the view direction changes, and TR1b's history, accepted by surface identity and warmed to 256 frames, follows slowly (the error at the patch's far column grows from 0.0019 at frame 1 to 0.0116 at frame 11; before matches to 1e-5). At whole-pixel offsets the window returns to the static 256. TR1b has no signal for view dependence that does not read colour.
2. **The light track turns 0.005 rad per index**, beyond TR1's 0.004-rad restart angle, so every frame restarts from the 3×3 spatial start, which blurs the sharp r0.08 highlights (geometric r0.08 17.8 against 8.96).

Both need a decision outside this unit: a view-change window cap tied to material roughness (a program declaring its view sensitivity), or a colour test, which P0 restricts to a plain min/max box (§5).

## 8. Memory, determinism, checks (M)

| Path | Unreconstructed reserved | TR1 | TR1b | Unreal comparator |
|---|---:|---:|---:|---:|
| RT (traced + Interactive) | 2,414.3 MiB | 2,542.3 MiB | 2,670.3 MiB | HRT 5,410 MiB |
| Non-RT (visibility buffer + Virtual) | 3,503.7 MiB (pre-FIX1) | 3,695.7 MiB | 3,759.7 MiB | VSM no-RT 3,912 MiB |

TR1b's targets add the two silhouette slots (63.3 MiB) to TR1's 118.1 MiB; the non-RT margin to VSM falls from 216 to 152 MiB.

| Check (final binary `tr1b-i`, Q-walk's 93 reference frames) | Result |
|---|---|
| In-process replay of the whole track, RT and non-RT, every step | byte-identical |
| Cross-process replay, RT and non-RT | identical |
| Non-RT path on the RT device against the no-RT device | **identical on all 93 frames** |
| Consolidated shader against the screened variant | identical, 60/60 checkpoints |
| Library suite (`--lib`, release test binary, inside a lease) | **139 passed, 0 failed, 130 ignored; exit 0** |
| `cargo fmt`; Clippy `--lib --tests` | clean in TR1b's files; pre-existing warnings remain in `virtual_shadow_gpu.rs`, `reference_inputs.rs`, `reference_tests.rs`, `visibility_buffer.rs` |
| CPU tests | lattice, jittered projection, naga validation of the resolve and rotated trace modules, settings; box-area pair against Monte Carlo |

`tr1b-g` (Q-disocclusion runs) and `tr1b-i` differ only in test harness modes (U1b, motion bench); renderer and shaders are the same source.

## 9. Misses and open items

1. **The default gate fails** (§4.5): TR1 is not the default. TR1b's own losses against the lighting-matched control are its cold first frame, small revealed pockets at ages 2–4, single-pixel maxima, the sky's 0.02 codes and the still-hold step; the rest of the gap to the path-traced reference is the shared lighting error.
2. **Below TSR Epic** (never presented as meeting the Unreal bar): revealed pixels on Q-walk (age 3, all-covered 9.64 vs 7.70), background-centre next to edges (0.419 vs 0.371 at frame 64), edges at L0 by under 1 % (§2.3), and +4-stop populations where the converged still image is itself above TSR's (§4.4).
3. **U1b's gate fails** (§7.2): view-dependent lag and light-track restarts.
4. **S2G sun on non-RT loses** to TSR no-RT (§4.3): Virtual's shadowed shading under a turning sun.
5. **Cost tails** (§3.2): p95/p99 over the half-TSR ceilings as recorded by TR1; the quiet re-time with TR1b's binary is outstanding.
6. **Existing-manifest bit-identity not shown** for the new reference binary (§4.5): L0 within ≤2 ULP on 43–50 pixels, accepted as bounded-AS, not strict.
7. **No TSR capture on the visible-sky walk** (§4.5): it needs a new Unreal quarry level with a visible sky.
8. **Q-sun-jump** is not regenerated: TR0's input lacks its sun step (§4.1), an input defect for TR0's owner.
9. **RT users get TR1 only on traced primaries**; the RT device's code default is still raster (IQ1's item 2).
10. **IQ1's finer texture LOD under TR1** (up to +0.09 codes on backlit at −10) is not evaluated yet.
11. **Visible sky as a default** is a separate open decision; T1a's film path does not show it.

## 10. Evidence, reproduction and time

Bulk evidence since the 2026-11-08 disk ruling is on D:; earlier evidence stays on E:.
- `E:/renderlab-work/m7-tr1/scores`: Q-walk, Q-disocclusion, Q-sun-fast and S2G scores and comparisons (`*-compare.json`); `E:/renderlab-work/m7-refs`: the regenerated references (§4.1), U1b and the rt1c witness.
- `D:/Coding/renderlab-work/m7-refs-sky`: the visible-sky walk, its 25 compact references, ages and the identity runs (`identity/`, `identity.json`, `u0b-997151eb/repeats.json`).
- `D:/Coding/renderlab-work/m7-tr1b/scores`: `sky-{rt,nonrt}-{tr1b,off}.json` and `sky-*-gate.json` (path-traced reference); `ctrl-*` (lighting-matched control); `inspect/`: the diagnostic scripts and inspected crops.
- Runs and plans: `D:/Coding/renderlab-work/m7-tr1/runs/sky-walk`, `runs/sky-int`, `plans/`.

Reproduce the gate: `m7-tr1b-refs.py sky --binary tr1b-k`; `m7-tr1-plan.py sky-walk --sequence …/sky-walk.json --sky --variants '{"tr1b":{},"off":{"enabled":false}}' --frames all`; `m7-tr1-score.py score … --track sky-walk --sequence … --references D:/Coding/renderlab-work/m7-refs-sky --ages …/ages --sky`, then `pooled <tr1b score> <off score> --sequence …`. The control adds `integrate: 8` to the plan's manifests and builds its references with `D:/Coding/renderlab-work/m7-tr1b/inspect/int_refs.py`.
