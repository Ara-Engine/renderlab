# M7-TR1b: moving silhouettes, the resolve pass and the missing tracks

2026-11-08, branch `t3/m7-tr1b` from main `bc6ba36` (TR1 merged opt-in), main `dc2e3c8` (FIX1) merged. Contract: [M7](milestone-7.md), [TR1](m7-tr1.md), the bar in [TR0](m7-tr0.md) (`E:/renderlab-work/m7-tr0/bar/tr1-bar.json`), [P0](m7-p0-claims.md).

**Status: TR1b stays opt-in. It is not the default.** STATUS_SUMMARY

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
| `scripts/m7-tr1-score.py`, `m7-tr1-plan.py`, `m7-tr1-wait.py`, `rt1c-witness.py` | every track; compact references; `compare --against`; schedule write locks; witness modes |

No shadow, GI/cache, reference, shader-reload, pipeline or primary-ray file is edited; `renderer.rs` equals main's (FIX1's still check).

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

### 3.2 Quiet cost against the half-TSR ceilings (M)

QUIET_COST

## 4. The missing tracks (item 3)

### 4.1 References in `E:/renderlab-work/m7-refs/` (M)

Compact format `m7-refs-compact-v1` (`scripts/m7-tr1b-refs.py`): L0/L1/D0/D1 as float16 already exposed at the track's registered exposure, V0/V1/visibility float16, the centre placement (`hit0`), every population mask unchanged. On a Q-disocclusion state it moves scored mean codes by **0.005 RMS (0.02 max)** and leaves split-half noise unchanged (0.4153 → 0.4154 codes, L0); the scorer reproduces Q-walk's rows exactly through the new code path. Geometry ages are generated from the full archives before compaction.

REFS_TABLE

### 4.2 Scores, every track, both paths

TRACK_TABLE

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

U1B

## 8. Memory, determinism, checks (M)

| Path | Unreconstructed reserved | TR1 | TR1b | Unreal comparator |
|---|---:|---:|---:|---:|
| RT (traced + Interactive) | 2,414.3 MiB | 2,542.3 MiB | 2,670.3 MiB | HRT 5,410 MiB |
| Non-RT (visibility buffer + Virtual) | 3,503.7 MiB (pre-FIX1) | 3,695.7 MiB | 3,759.7 MiB | VSM no-RT 3,912 MiB |

TR1b's targets add the two silhouette slots (63.3 MiB) to TR1's 118.1 MiB; the non-RT margin to VSM falls from 216 to 152 MiB.

DETERMINISM

## 9. Misses and open items

MISSES

## 10. Evidence, reproduction and time

EVIDENCE
