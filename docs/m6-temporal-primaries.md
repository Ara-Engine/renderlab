# M6 T1: temporally jittered primaries with hit-record history

**Status:** the GPU references and T1a (static accumulation, corner-free lattice) are implemented and measured in [m6-temporal-gpu.md](m6-temporal-gpu.md). T1a fails only at three pixels where the production receiver origin is blocked at concave triangle edges, a known production defect being fixed separately. The T1b candidate (commit `b8fa1f3`) failed its Q-walk gate on silhouette ghost trails, shadow-edge blur and the frozen R_D F4 stop, and was reverted. T1c and T1d have not started. The second-sample criterion is N = 2 / N = 1 ≤ 1.0; the 0.944 in §2 was the earlier ordering's measured value, not a limit.

Design decision, 2026-11-05, base `fe196fd`. This is the CPU phase of the temporal unit that [film gap §7](m6-film-gap.md#7-next-unit-and-stop-rules) froze as F1-AA. It replays R1's 16 saved per-sample frames and film references, plus the accept-b7 Interactive captures. There was no production change, GPU run, build, window or push. Labels: **M** is measured here from saved evidence; **I** is a model or inference.

Binding inputs:

- the frozen F1-AA rules in the film gap;
- [R1](m6-receiver-policy.md) and its saved samples;
- Islam's adoption of the viewed-side receiver rule with a per-material thin flag, which lands after programmable-shading phase 1;
- Islam's 8-frame settle limit after cuts and disocclusion;
- AGENTS' quality rules;
- [programmable shading](programmable-shading.md) §2.4.

## Decisions

1. **R1's 5-sample crossing was an ordering artefact; T1 meets the 8-frame limit with at least 1.8x margin, about 3x on most populations (M).** The saved samples are a rank-1 lattice stored row by row, so R1's prefix N covered only the top N/16 of the footprint. Replayed in a progressive order with modelled 8-ray visibility, every R_L, R_D and R_F population reaches the Exact bar by frame 2–4. At frame 8 each population sits at 0.18–0.41x the bar on R_L and R_D, and 0.25–0.54x on R_F. In R1's own linear-HDR metric the progressive sequence crosses at N = 2, against R1's 5 ([§2](#2-sample-pattern-and-static-convergence)).
2. **Sample pattern: centre-first progressive lattice, scrambled per pixel by the lattice symmetries that fix the centre.** Sample 0 is the pixel centre, so a reset pixel's first frame is Interactive by construction. Samples 1–3 are the quarter-point pair and the corner, and later samples follow the bit-reversed lattice. A random first sample is 1.28x Interactive's error and would fail F2 at age 0 (M).
3. **Reprojection.** The current jittered hit is taken backward through the previous camera and the instance's previous transform. Each tap is validated: same instance with depth within 4 pixel footprints, or the tap's own surface still visible within 1.5 px. Catmull-Rom is used when all 16 taps validate; otherwise bilinear renormalised over the valid inner taps; otherwise a reset. There are no micro-normal or triangle-identity tests: at the 1 cm step they reject 4.5–70% of pixels whose history is 2–3x better than Exact (M, [§3](#3-reprojection-at-the-registered-1-cm-step)).
4. **History weighting.**
   - **Static:** cumulative mean while the pixel's fetch is exact (static camera and surface, or whole-pixel motion), up to W = 256.
   - **Sub-pixel motion:** a resampling-blur budget sets W per pixel, from 3–4 frames at large fractional offsets to about 9 at 0.15 px/frame.
   - **Light and mover motion:** lag-bounded windows computed from engine-known angular rates.

   Long windows fail under motion. A 16-frame bilinear history exceeds the bar on all-covered, R_D and silhouettes (I, [§4](#4-history-weighting-under-motion), [§5](#5-lights-movers-and-other-changes)).
5. **The history accumulates program output, as the engine-owned film reconstruction.** This is a bounded exception to "filter incident quantities, never program output": the pixel's film value is the footprint integral of the program's output, and no incident-only history can represent it. The weights read no colour, so every style converges to the integral of its own output. Validity rests on explicit rules for declared inputs ([§6](#6-programmable-shading)).
6. **Level F gate for T1 ([§7](#7-level-f-gate-for-t1))**:
   - nine quarry sequences plus fixtures;
   - the bar is the re-baselined, viewed-policy Exact on the same frame;
   - ≤ Interactive at every age;
   - ≤ bar by the 8th frame after any event;
   - the frozen F3–F10 checks, plus blur, ghost and lag diagnostics with model-consistency stops.
7. **Cost (I): 0.85–0.92 ms per frame**, against 0.8156–0.8161 ms for Interactive ([§8](#8-cost)). It straddles the matched HRT 2x floor (0.87–0.89 ms), likely missing it by up to 0.03 ms. It misses the 0.80 ms VSM floor, which Interactive already misses. Memory is about 108 MB at 1080p.
8. **GPU work waits for programmable-shading phase 1**, which owns the traced shading entry and the primary-ray files that T1 hooks ([§9](#9-gpu-implementation-plan-and-stop-rules)). Expected misses are recorded now: a 10°/s sun, and the moving blocker's shadow region while it moves, both fall to Interactive-equivalent quality.

## 1. Inputs and method

- **Samples.** R1 `quarry-viewed-p{0,1}` holds the 16 saved strata with full HDR, sun-only HDR and Exact (256-ray) visibility per sample. Saved sample i is stratum row i, column 7i mod 16: a rank-1 lattice over the 16x16 footprint. Sample 8 sits at (0.53, 0.53), the footprint centre.
- **References.** Each saved sample is one of the 256 reference strata, so each reference is replaced by its 240-stratum complement: (256·R − Σ saved)/240. This makes it independent of every replayed sample. The visibility complement uses the accepted counts. Checks: no saved count exceeds the reference, the complement minimum is ≥ 0, and pixels outside the scored crop carry ≤ 4.5e-7 codes. Reference noise adds equally to candidate and bar, so "first N ≤ bar" does not depend on it.
- **8-ray visibility (M model).** The noise is the empirical distribution of Interactive minus Exact at pixel centres: accept-b7, poses 0–1, seeds 0–3, 68,680 partial residuals in 34 bins of Exact count. Partial RMS is 0.0816. Lit and umbra centres disagree at rates of 4.4e-6 and 3.3e-5. Each partial sample's radiance takes L + U(V8 − V), with U = D/V from its own sun-only HDR. That decomposition is exact for the standard program (film gap §2: residual 3.9e-4).
  - Rejected samples (0.42% of covered-centre samples, plus foreground samples in background-centre pixels) have no saved visibility and get no added noise.
- **Bars.** The frozen Exact (E) uses the production 1 mm policy, which is not the viewed estimand: on lit footprints its R_D +4 error is 2.8x the viewed proxy's (5.11 against 1.83 codes), mostly from the brighter lower ground. Saved sample 8, a viewed-policy point 0.03 px from the centre with Exact rays, is the proxy for the re-baselined Exact. Every ratio uses bar = min(E frozen, proxy). The gate itself uses the re-baselined Exact ([§7](#7-level-f-gate-for-t1)).
- **Units.** Display codes after F0's transform at the registered exposure and +4 stops: R_L for full radiance, R_D for sun only. R_F is film visibility. Populations are R1's, defined from the viewed R_F. RMS over `image` and `background-centre` uses the full-image pixel count.
- **Motion.** Two parts:
  - measured transfer at the registered 1 cm step: pose-1 centre hits fetch the converged pose-0 film and are scored against the pose-1 film;
  - a Fourier steady-state model of exponential history under constant sub-pixel motion, calibrated against that transfer.

## 2. Sample pattern and static convergence

**Ordering (M, pose 0, 8-ray visibility, N = 8; pose 1 within 1%):**

| Order | R_L image | R_L +4 image | R_D +4 image | R_F partial | First N ≤ bar (R_L, R_D +4) |
|---|---:|---:|---:|---:|---|
| R1 saved (rows 0..N−1) | 1.638 | 2.402 | 1.064 | 0.128 | 3, 5 |
| Bit-reversed lattice | 0.788 | 1.260 | 0.465 | 0.073 | 2, 2 |
| Bit-reversed, per-pixel toroidal shift | 0.781 | 1.179 | 0.462 | 0.075 | 2, 2 |
| Random per-pixel permutation | 0.837 | 1.247 | 0.503 | 0.077 | 2, 2 |
| **Centre-first lattice (chosen)** | **0.788** | **1.260** | **0.465** | **0.073** | **2, 2** |
| Exact frozen / viewed proxy | 2.581 / 2.607 | 4.392 / 4.422 | 2.312 / 1.428 | 0.249 / 0.205 | — |

The saved prefix covers only the top of the footprint. Progressive prefixes are stratified at every power of two. R1's metric (linear HDR, saved order) reproduces exactly here: partial N = 4, all-covered and image N = 5. With the centre-first sequence the linear crossing is N = 2 (`prefix-check.json`). R1's β fits describe the ordering, not the estimator.

**First sample (M).** One jittered sample's image error depends on its position: 2.61 codes at the centre and 4.20 at the corner, with a lattice mean of about 3.3. Interactive scores 2.58. A random first position therefore fails "≤ Interactive at every age". With the centre first, T1's age-0 frame is the Interactive estimator. On the GPU it must be byte-identical, given the same policy and ray set.

**Within-prefix order (M, `prefix-check.json`).** The prefix sets are fixed at N = 4, 8 and 16; only the order inside them is free.

| Order | Image codes, N = 2 / 3 / 4 | R_D +4 covered, N = 2 / 3 |
|---|---|---|
| 8, 0, 12, 4, … (centre, then corner) | 2.08 / 1.50 / 1.09 | 2.89 / 1.97 |
| **8, 12, 4, 0, … (centre, then quarter pair; chosen)** | **1.94 / 1.26 / 1.09** | **2.51 / 1.62** |

The order also decides whether the second frame can be worse than the first: F2 requires ≤ Interactive at every age, and age 0 is the centre. Per population, at both poses, the ratio of N = 2 to N = 1 error (Exact rays, M, `prefix-check.json`) is:

- **8, 0, … (corner second):** up to 1.068–1.086 on R_D (contact, narrow) and 1.012–1.015 on R_D +4 (lit, narrow). It breaches F2.
- **8, 12, … (chosen):** at most 0.944 across all populations of R_L, R_L +4, R_D and R_D +4.
- **8, 4, …:** at most 0.952.

**Chosen sequence.**

- Index 0 is the pixel centre; indices 1–3 are rows 12, 4 and 0; the rest follow the bit-reversed lattice.
- Per pixel, one of the four lattice symmetries that fix the centre is chosen by hash: identity, point reflection, transpose, or both. That decorrelates neighbouring pixels without changing any 2^k prefix set.
- Production extends the lattice to a 256-point centre-first (0,2)-sequence. The 8 shadow directions rotate with the same index, so index 0 uses Interactive's set.

**Convergence of the chosen sequence (M, 8-ray visibility; worst of poses 0 and 1).** N = 2 values come from the 8, 0, 12, 4 order and are upper bounds for the chosen order.

| Population | R_L bar | R_L N=8/bar | R_L +4 N=8/bar | R_D N=8/bar | R_D +4 bar | R_D +4 N=8/bar | R_F bar | R_F N=8/bar | First N ≤ bar (R_L, R_L+4, R_D, R_D+4, R_F) | N=16/bar, worst |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|
| partial | 9.35 | 0.31 | 0.27 | 0.36 | 7.28 | 0.32 | 0.205 | 0.36 | 2, 2, 2, 2, 2 | 0.16 |
| faint | 8.52 | 0.31 | 0.25 | 0.39 | 5.17 | 0.33 | 0.086 | 0.54 | 2, 2, 3, 2, 4 | 0.30 |
| narrow | 10.77 | 0.30 | 0.26 | 0.37 | 7.88 | 0.37 | 0.225 | 0.34 | 2, 2, 3, 3, 2 | 0.15 |
| contact | 10.31 | 0.32 | 0.30 | 0.41 | 7.65 | 0.38 | 0.195 | 0.34 | 2, 2, 3, 2, 2 | 0.15 |
| isolated | 12.43 | 0.28 | 0.19 | 0.32 | 8.66 | 0.23 | 0.130 | 0.30 | 2, 2, 2, 2, 2 | 0.18 |
| lit | 4.78 | 0.29 | 0.24 | 0.37 | 1.83 | 0.35 | 0 | — | 2, 2, 3, 3, — | 0.15 |
| umbra | 7.69 | 0.31 | 0.31 | 0.34 | 2.01 | 0.36 | 0 | — | 2, 1, 2, 2, 1 | 0.17 |
| backlit | 11.94 | 0.30 | 0.29 | 0.30 | 7.88 | 0.28 | 0.237 | 0.28 | 2, 2, 2, 2, 2 | 0.12 |
| multi-surface | 13.66 | 0.30 | 0.27 | 0.34 | 8.44 | 0.32 | 0.116 | 0.42 | 2, 2, 2, 2, 2 | 0.21 |
| background-centre | 1.07 | 0.36 | 0.37 | 0.35 | 0.58 | 0.34 | — | — | 2, 1, 2, 2, — | 0.19 |
| all-covered | 5.82 | 0.30 | 0.26 | 0.37 | 3.25 | 0.33 | 0.078 | 0.36 | 2, 2, 2, 2, 2 | 0.16 |
| image | 2.58 | 0.31 | 0.29 | 0.36 | 1.43 | 0.33 | — | — | 2, 2, 2, 2, — | 0.15 |

- **8-frame limit:** met in every population and channel, with the worst at 0.54x (R_F faint).
- **Frozen F1-AA checks:**
  - R_L ≤ Exact by frame 2: yes, worst 0.84x.
  - Visibility partial ≤ 0.12 at frame 8: 0.073.
  - Visibility partial ≤ 0.058 at frame 32: modelled 0.022.

  None fires.
- **Model, N = 32 (I, log-log fit over N = 4–16):** image 0.21 codes (0.08x), R_F partial 0.022.
- **8-ray visibility** adds less than 1% at N = 8. Footprint aliasing dominates.
- **Solids.** On film-lit and film-umbra pixels the replayed R_F error rounds to 0.000 at every N. The modelled lit miss rate, 4.4e-6 per sample, comes from centre receivers and overstates misses on film-lit footprints. The 0.002 solid maximum is a GPU measurement.

Inspection (`pose-0-accumulation-sheet.png`, `pose-0-sun-accumulation-sheet.png`, `pose-0-accumulation-crops-plus4.png`):

- At N = 1 the error map looks like Exact's: speckle on rough faces, the crack network and the silhouette outline. By N = 4 it is faint, and by N = 8–16 it is near black at x8.
- Exact's R_D shows a bright lower ground, the 1 mm policy bias the proxy bar removes.
- The +4 crops show crack and contact darkening converging toward the film without halos.

This is static evidence only.

## 3. Reprojection at the registered 1 cm step

Pose 1 moves the camera 1 cm. Pose-1 centres reproject to pose 0 at dx median 1.25 px (p1–p99 0.77–1.71) and dy 0.32 (0.09–0.59). The median fractional offsets are 0.25 and 0.32 px.

| Rule (bilinear unless named) | Pixels with history | R_L error, all-covered | partial | multi-surface | R_D +4, all-covered |
|---|---:|---:|---:|---:|---:|
| No validation | 100% | 2.78 | 3.87 | 5.26 | 1.83 |
| Instance | 100% | 2.92 | 4.04 | 8.29 | 1.92 |
| Instance + depth ≤ 2% | 100% | 2.92 | 4.05 | 8.41 | 1.93 |
| Instance + depth ≤ 4 footprints | 99.94% | 3.02 | 4.24 | 8.45 | 1.99 |
| + normal agreement ≥ 0.5 | 99.84% | 3.15 | 4.58 | 8.52 | 2.12 |
| + normal ≥ 0.9 | 95.5% | 3.72 | 5.41 | 8.43 | 2.47 |
| + normal ≥ 0.999 | 44.1% | 3.33 | 4.22 | 6.71 | 2.24 |
| Same triangle | 30.2% | 3.01 | 3.66 | 5.19 | 2.13 |
| **Forward (instance, 4 footprints) or reverse** | **100%** | **2.84** | **3.92** | **6.71** | **1.88** |
| Nearest tap, instance + 2% | 99.5% | 4.24 | 5.30 | 7.75 | 2.72 |
| Catmull-Rom, 16 taps valid (93.9% subset) | — | 2.15 | 2.87 | 4.44 | 1.26 |
| Bilinear on the same subset | — | 2.67 | 3.76 | 5.90 | 1.69 |
| *Exact, pose 1* | — | *5.97* | *9.67* | *14.17* | *5.47* |

The film-to-film noise floor is 0.68 codes. History error is scored on accepted pixels; rows with low acceptance are on easier subsets, where Exact is 3.05–3.32.

- **Micro-normal and triangle tests reject good history.** On this scan only 11.5% of base bilinear taps share the current triangle, and the tap/current normal dot has p10 0.75. Pixels fully rejected by normal ≥ 0.9 (15,466) had history error 4.87 against Exact's 14.7 there. Under triangle identity (241,316) the figures are 3.26 against 6.86. A reset trades that history for a single sample. Rejected.
- **Depth tolerance.** For same-instance taps, |Δd| is a median 0.36 footprints (p90 1.05, p99 5.8, p99.9 21.6). Two percent of distance (19.5 footprints at any distance) misses same-instance folds closer than about 16 cm at 8 m. Four footprints (0.41%) catches folds of 3.3 cm at 8 m, and alone rejects 176 grazing pixels whose history was 2x better than Exact. The reverse test recovers those.
- **Reverse test.** A tap stays valid if its own anchor point, moved by its instance's motion, is still visible within 1.5 px with the same instance and depth within 2%. That keeps the other surface's share at multi-surface footprints: 8.41 becomes 6.71. A revealed surface still fails both tests. Unvalidated history scores best at this step (5.26), but it is a ghost source under real disocclusion.
- **Catmull-Rom** cuts single-step transfer error by 19–25% against bilinear on the same pixels. Nearest-tap fetch misregisters by up to half a pixel (+45%).
- **Silhouettes.** Background-centre pixels keep same-pixel history under a translation. Their single-step error is 0.915 against 0.767 for Exact on the same pixels. The foreground part of the mixed history moved with parallax: one colour per pixel cannot be reprojected correctly in two directions.

**Design.**

- **Per pixel** (written by the shading wrapper): the history colour (RGB mean in f32 plus the count, 16 B) and an anchor (8 B):
  - instance id;
  - the thin-material side bit;
  - the sequence index k (frames since the last reset);
  - the view distance of the latest sample.
- **Trace pass:** writes the ray distance t of the current jittered hit (4 B per pixel, covered tiles only) for the reverse test.
- **Motion:** X is the current jittered hit. It moves by the instance's previous transform M_{t−1}·M_t^{-1} and is projected by the previous camera. Background samples reproject by direction (rotation only).
- **Tap validation:**
  - forward: same instance and |d_tap − d(X)| ≤ 4 footprints;
  - or reverse: the tap's anchor point, at its instance's current transform, projects within 1.5 px of the pixel. The current frame's surface there has the same instance and t within 2%.
  - Thin-flagged materials also require the same side bit. Opaque materials have no sidedness or normal test, consistent with the viewed-side receiver rule.
- **Fetch:**
  - Catmull-Rom over 4x4 when all 16 taps validate, clamped at zero. That clamp is non-negativity, not a colour clamp.
  - Otherwise bilinear over the valid inner taps, renormalised. The history count is the weighted tap count.
  - With no valid tap: reset, so n = 0 and k = 0.
- **Generations.** Each placement has a generation counter, bumped by program reload, parameter writes, material texture changes, mesh replacement and streaming LOD changes. A tap whose placement changed generation since the previous frame is invalid. The previous frame's generations are kept in a small table.
- **Off-screen return** has no history and starts cold, like a disocclusion. That is the film gap's accepted failure mode: an unbiased 1x sample that settles within the 8-frame limit.

## 4. History weighting under motion

Model (I): h_t = (1−α)·fetch_f(h_{t−1}) + α·sample_t under constant motion (fx, fy) px per frame.

- **Bias:** the exact Fourier transfer α/(1 − (1−α)K(ω)), applied to the film. K is the bilinear or Catmull-Rom fetch with its shift removed.
- **Noise:** iid per-sample error variance (8-ray samples against the film) times the mean noise gain α²/(1 − (1−α)²|K|²). This assumes motion breaks the lattice stratification, so it is conservative for slow motion.
- **Calibration:** single-step bilinear at (0.25, 0.32) predicts 2.73 codes all-covered against the 1 cm measurement of 2.84 net of noise. Partial is 4.00 against 4.05, and background-centre 0.89 against 0.915. The Catmull-Rom single step is optimistic: 1.89 against about 2.05 on its subset.

| Fetch, motion (px/frame) | α | R_L all-covered (bar 5.82) | R_L partial (9.35) | multi-surface (13.66) | background-centre (1.07) | R_L +4 all-covered (9.20) | R_D +4 all-covered (3.25) | R_L image (2.58) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Catmull-Rom, (0.25, 0.32) | 1/3 | 3.35 | 4.97 | 7.18 | 0.81 | 4.53 | 2.07 | 1.56 |
| Catmull-Rom, (0.25, 0.32) | 1/4 | 3.27 | 4.82 | 6.61 | 0.84 | 4.51 | 2.06 | 1.54 |
| Catmull-Rom, (0.25, 0.32) | 1/16 | 4.58 | 6.56 | 7.84 | **1.19** | 6.29 | 3.01 | 2.16 |
| Catmull-Rom, (0.15, 0) | 1/8 | 2.72 | 4.21 | 5.32 | 0.50 | 3.90 | 1.62 | 1.20 |
| Catmull-Rom, (0.5, 0.5) | 1/3 | 3.36 | 5.01 | 7.17 | 0.81 | 4.54 | 2.10 | 1.56 |
| Bilinear, (0.25, 0.32) | 1/3 | 3.86 | 5.76 | 7.68 | 1.03 | 5.36 | 2.54 | 1.84 |
| Bilinear, (0.25, 0.32) | 1/8 | 5.30 | 7.63 | 8.67 | **1.57** | 7.44 | **3.68** | **2.60** |
| Bilinear, (0.25, 0.32) | 1/16 | **6.67** | 9.35 (= bar) | 10.25 | **2.06** | **9.21** | **4.69** | **3.31** |
| Bilinear, (0.5, 0.5) | 1/3 | 4.07 | 6.08 | 7.93 | **1.09** | 5.69 | 2.69 | 1.94 |
| Static, iid equivalent (α = 1/8) | 1/8 | 1.75 | 2.62 | — | — | 2.20 | 1.06 | — |

Bold marks a value above the bar.

- **Motion costs about half the static advantage.** Under sub-pixel motion T1 settles at 0.5–0.8x the bar, against 0.3x at frame 8 when static. The stratified static sequence beats the iid equivalent by 2.5x at N ≈ 16.
- **Silhouettes bind.** Background-centre pixels need Catmull-Rom and a short window: 0.76–0.79x the bar. Bilinear stays at 0.96–1.02x at large offsets, and every 16-frame history fails.
- **Window rule: a resampling-blur budget**, so the window follows the motion that set it:
  - s = Σ_axes f(1 − f) is the bilinear-equivalent variance added by one fetch at the exact fractional offset f;
  - W_motion = clamp(1 + v_max/s, 1, 256), with v_max = 1 px²;
  - this gives W ≈ 3 at (0.5, 0.5), about 3.5 at (0.25, 0.32), about 9 at (0.15, 0) and 256 as f → 0. That matches the model's optima within one step of the tested α grid.
  - Integer-pixel motion has s = 0.
  - A dolly adds the footprint-scale change to s; at 2% per frame that is negligible.
- **Count.** Per frame, n' = min(n_hist + 1, W) and m' = m + (x − m)/n', where W = min(W_motion, W_light, W_mover). k advances every frame and resets only with n.

## 5. Lights, movers and other changes

Sun visibility at a penumbra changes at most s_max = 4/(πα) per radian of relative light motion: 31.8 per radian for the 0.04-rad disk, the straight-edge maximum. An exponential history of mean age W − 1 therefore lags by at most s_max·ω·(W − 1). T1 caps that bias at b_V = 0.05 in visibility, 1/5 of Exact's film-partial error, so W_light = 1 + ⌊b_V/(s_max·ω)⌋. This is a bound at the steepest pixel; the F3 and F5 statistics measure the actual bias.

| Light motion at 60 Hz | ω (rad/frame) | W | Static-camera quality at that W (I, §2 curves) | Expected verdict |
|---|---:|---:|---|---|
| Static | 0 | 256 | ≤ 0.21x bar on R_L and R_D at N = 16 (R_F faint 0.30) | Pass |
| 0.25°/s (24-minute day) | 7.27e-5 | 22 | About the N = 16 level; lag bias ≤ 0.05 at max slope, changes over 8 frames ≤ 0.019 | Pass, including frozen F5 |
| 1°/s | 2.91e-4 | 6 | ≈ 0.3–0.5x bar | Pass on F1/F2; F5 lag is diagnostic |
| 10°/s | 2.91e-3 | 1 | Interactive (centre, 8 rays) | **Recorded miss** on R_D and F1 |
| Jump > α/2 in one frame (Q-sun 5°) | — | reset | Interactive at age 0; ≤ bar by frame 2–4 | Pass on F2 |

- **Movers** use the same bound with the relative angular speed of each occluder seen from the receiver: ω_eff(p) = Σ_j |v_j − v_X| / s_j.
  - The sum runs over instances whose old ∪ new AABB meets the receiver's sun cone (half-angle α/2, widened by one footprint).
  - s_j is the distance from X to that AABB, and v_X is the receiver's own velocity.
  - The cone-against-box test is exact and conservative. It touches only moving instances, or, for a moving receiver, the instances near it.
  - A mover's self-shadowing under pure translation has relative speed 0. Its rotation adds its angular speed, as a moving light would.
- **Q-blocker.** At 5 mm/frame, receivers within 3.2 m of the rock get W = 1 while it moves. Its shadow region shows the Interactive estimator at the centre during motion and settles like a disocclusion after it stops.
- **Reach-region boost.** Under the frozen "any F4 excess on R_D stops the candidate", the shadow region traces 32 rays per sample instead of 8: an exact trigger, costing about 0.0004–0.004 ms at 0.1–1% of pixels (I). Whether 32 rays at the centre hold F4 at Exact's count is a GPU question ([§10](#10-risks)).
- **Other declared inputs:**
  - Light intensity, colour and extent: reset on change. Programs need not be linear in intensity, and continuous intensity animation is not supported yet.
  - Environment and exposure edits: global reset.
  - Camera cut: global reset, with no history reads that frame.

## 6. Programmable shading

[Programmable shading §2.4](programmable-shading.md#24-realistic-path-quality-bar-and-world-space-inputs) says the engine accumulates incident quantities, never program output. The same section says "a final engine-owned image reconstruction (AA) remains style-independent". T1 is that reconstruction, and it accumulates program output deliberately:

1. **Only output accumulation represents the film value.** The film value of a pixel is the footprint integral of the program's output.
   - For a nonlinear program, band(mean V) ≠ mean(band(V)).
   - Incident-only history shaded once per pixel leaves the shading and geometry aliasing behind. Fully lit footprints carry 43% of the image's squared error, multi-surface footprints 9% and silhouettes 15%.
   - It also breaks the in-footprint covariance of direct light and visibility (film gap §2). The SVC was rejected for this.
2. **The weights are style-independent.** They come from geometry (instance, depth, reverse visibility, fractional offset) and declared-input rates. No colour clamp, luminance weight, variance test or demodulation reads output. A toon frame converges to anti-aliased toon, not to a photoreal-corrected toon.
3. **Visibility stays an engine incident input, fresh per sample.** Each sample's program sees 8-ray visibility at its own receiver. Nothing about lighting enters the hit record or the anchor.

**Recorded constraints:**

- Programs must be pure functions of `SurfaceInput`, `Parameters` and engine inputs. That holds today: they have no bindings and no time input.
- A future `time` input: programs whose IR references it get W = 1 unless they declare a temporal bandwidth.
- `shade` returns exposed HDR, so exposure changes invalidate history. Auto-exposure would need exposure moved after accumulation, an API change.
- Post programs run after accumulation and read the current jittered sample's screen inputs. Their outlines are not accumulated, which extends the existing R9.
- Per-sample texture gradients keep the pixel footprint, matching R_L's construction. Against a point-sampled path-traced reference that double-filters texture. It is an estimand question for a later reference, small on this quarry: flat lit pixels err 1.0 code (film gap §2).
- The accumulate call lives in the generated traced shading wrapper after `rl_finite`, never in program code.

**Escalation kept (not built): visibility-split history.**

- Evaluate `shade` twice per sample, with fresh V and with shadowed lights' V = 0.
- Accumulate L_0 with the long window and L − L_0 with the lag-bounded one.
- The split is exact for any program, because it is per sample. It costs one more `shade` body (I: 0.01–0.03 ms; the lit body is about 0.014 ms) and 12–16 B per pixel.
- It improves R_L under a fast sun or near movers, but not R_D. Build it if Q-blocker or Q-sun at 1°/s fails on R_L.

## 7. Level F gate for T1

**Bar.** E is Exact under the adopted viewed policy, the production default after its re-baseline, on the same frame and against the same reference. The bar is E·(1+s), where s is Exact's measured 2-s.d. spread across seeds.

**References.** Each frame gets R_L, R_D and R_F. The proposed generator uses 1,024 jittered samples with 32 rays each and split-half radiance and visibility sums, so scores can subtract reference noise. It must first agree with R1's 256x256 references at poses 0 and 1 within combined noise. Static holds share one reference. Q-static and Q-sun-slow are scored every 4th frame after frame 16.

**Sequences** (60 Hz nominal; each registered with camera, light and mover records hashed before the first candidate run):

| Id | Content | Exercises |
|---|---|---|
| Q-static | Pose 0 from cold, 128 frames | Convergence, flicker, age-0 identity |
| Q-walk | 64 frames of 1 cm steps with 0.05°/frame yaw, 32 hold, reverse path | Motion window, blur, silhouettes under parallax, return |
| Q-cut | 64 frames converged at pose 0, cut to the registered near view, 32 frames, cut back | Cold start, the 8-frame limit |
| Q-disocclusion | 48-frame path past the central slab revealing hidden ground | Revealed surfaces, ghosts |
| Q-zoom | Dolly-in at 2%/frame for 32 frames, 16 hold | Footprint change, blur |
| Q-blocker | One placed rock moving 5 mm/frame for 32 frames, 32 hold | Mover caps, reach boost, F4 on R_D, revealed ground |
| Q-sun-slow | 0.25°/s for 128 frames | Lag (frozen F5) |
| Q-sun-fast | 10°/s for 32 frames, then 32 hold | Recorded expected miss; recovery |
| Q-sun-jump | A 5° jump, then 32 frames | Reset and settle |
| Fixtures | Thin, stress and sawtooth: static, pan, cut, blocker and sun segments against analytic film oracles | Exactness on known geometry; thin-side bit |

**Events and ages.** An event is a cut, a rejected history (disocclusion), a generation bump, a light jump or a W = 1 reset. Age 0 is the event frame (N = 1). "By frame 8" means age 7.

**Populations.** R1's set from per-frame R_F, plus:

- **revealed:** age < 8 after a disocclusion;
- **changed:** |R_t − R_{t−8}| > 0.05 on R_F, or > 8 codes on R_D at +4;
- **reach:** the Q-blocker cone region;
- **edge:** within 1 px of an instance or background boundary.

| Check | Rule | Catches |
|---|---|---|
| T-0 age-0 identity | A reset pixel's first frame is byte-identical to Interactive (same policy, seed and ray set). Q-static frame 0 and all Q-cut cut frames | Sequence or ray-rotation errors |
| F1 converged | Pixels ≥ 32 frames past their last event: per-population RMS ≤ bar on R_L (registered, +4), R_D (registered, +4) and R_F. Solids: R_F maximum ≤ 0.002; on R_D a maximum ≤ Exact's maximum on the same pixels. Applies during continuous camera motion, where blur counts | Steady quality, leakage, motion blur |
| F2 transient | From every event: RMS ≤ Interactive's at every age, and ≤ bar at age 7, per population, R_L and R_D | The 8-frame limit (Islam) |
| F3 bias | \|mean signed error\| ≤ \|E mean\| + 4 SE, per population per frame | Lag, blur darkening, leakage |
| F4 missing and false shadows | Per frame, persisting ≥ 2 frames: (R_F ≤ 0.9 shown ≥ 0.99) plus (R_F ≥ 0.99 shown ≤ 0.9) ≤ E's count, using T1's test-only accumulated visibility. On R_D +4: pixels with error > 16 codes ≤ E's count. Any excess on R_D stops the candidate (frozen) | Missed shadows, mover regions |
| F5 lag and ghosts | Frozen: Q-sun-slow lag ≤ 0.5 frame on changed pixels. All sequences: ghost bias ≤ \|E's\| + 2 SE on changed pixels; zero history weight from a surface not visible in the previous frame. That is checked exactly from both frames' hit records via a test-only accepted-tap identity output. Changed-pixel RMS ≤ bar 8 frames after motion stops | Trails, stale history, disocclusion ghosts |
| F6 flicker | Static converged pixels: RMS(c_t − c_{t−1}) ≤ 0.5 code at the registered exposure and ≤ 1 code at +4. Motion: the residual (c_t − c_{t−1}) − (R_t − R_{t−1}) on unchanged surfaces ≤ E's | Shimmer, sequence imprint |
| F7 structure | Contour ρ ≤ the Level-B S4 bound computed with E. Error power at the lattice and sequence frequencies ≤ E's + 2 SE | Patterned residue |
| F8 determinism | Replaying the same input sequence is bit-identical and independent of frame timing. Static repeats are run several times (driver hazard, [climb 2](m6-interactive-climb-2.md#determinism-hazard-found-and-avoided)) | Nondeterministic history |
| F9 cost | Per-pass p50 and p99 per sequence, including cut, disocclusion and reach frames. No frame above 1.5x steady. Memory reported | Reset spikes |
| F10 inspection | Film, Exact, Interactive and T1 at frames 0, 1, 3, 7 and 31 after each event, plus steady motion. x8 error maps at both exposures on one scale; flip-books of every sequence. Frozen crops: the seven contact pixels, silhouettes, rubble, crevices, cliff-top partials, revealed ground, the mover's shadow, and the fixture strip, sliver, specks and wires | What the numbers miss; numbers do not override it |
| Diagnostics with stops | Static curve within 1.25x of §2 at N = 1, 2, 4, 8 and 16. Q-walk steady error within 1.25x of §4's model at the measured offsets. Band-pass (DoG 0.5–1 px) energy ratio of T1 to film on Q-walk and Q-zoom reported | Implementation or model divergence |

Thresholds are fixed now. Changing W rules, b_V, v_max, tolerances, the sequence or the fetch makes a new candidate.

## 8. Cost

Interactive passes are from [climb 2](m6-interactive-climb-2.md) (d17, M). T1 adds the following (I):

| Pass | Interactive (M) | T1 (I) | Basis |
|---|---:|---:|---|
| Fused trace + 8 rays | 0.4064–0.4079 | +0.000–0.010 | Jitter and ray rotation are ALU. Rays stay inside the same 8x4 tile footprints. t written in covered tiles: about 1.4 MB |
| Shading (gather-bound) | 0.3651–0.3671 | unchanged | Program work is unchanged |
| Accumulate, inside the shading wrapper | — | +0.03–0.08 | About 0.40 M pixels (covered tiles now or last frame) × 48 B (history and anchor read and write) ≈ 19 MB: about 0.04 ms if fully exposed. Catmull-Rom taps are mostly L1/L2 hits. One dependent fetch after the hit reconstruction, partly hidden behind texture work |
| Reverse test | — | +0.003–0.01 | Reads of local t and placement |
| Tone | 0.0328–0.0338 | unchanged | Reads the same RGBA16F target |
| **Span** | **0.8156–0.8161** | **0.85–0.92** | |

- **Floors.** 2x the matched fresh HRT medians (1.74–1.78 ms) is 0.87–0.89 ms. T1 straddles it and likely misses by up to 0.03 ms. 2x VSM (0.80 ms) is missed. The stored 2.15 ms HRT anchor gives 1.075 ms, which T1 meets.
- **Unreal side.** Unreal's numbers exclude TSR, which ships with its high-fidelity rendering and would add time on Unreal's side, unmeasured. The film gap's 0.89–0.94 ms estimate predates climb 2's 0.045 ms gain.
- **Event frames.** Cut frames read no history, so they cost no more than steady frames. Disocclusion has no extra cost. The reach boost is area-bound (§5).
- **Memory.** Ping-pong colour and anchor at 48 B per pixel plus 4 B of t is 108 MB at 1080p (about 430 MB at 4K). It is under the 128 MB stop at 1080p.

## 9. GPU implementation plan and stop rules

GPU work starts after programmable-shading phase 1 lands, since both own the traced shading entry, `primary_ray_gpu.rs` and the trace kernels. Islam's foreground-game rule applies to every GPU window. Implementation goes to Sol; acceptance goes to Opus.

**File ownership:**

- `src/film_accumulation.rs`: history resources, sequence, W rules, generation table, mover cones, CPU unit tests;
- `shaders/film_accumulation.wgsl`: fetch, validate and blend functions;
- `src/film_accumulation_tests.rs`: GPU tests, ignored by default;
- hooks only in `shaders/primary_trace.wgsl` / `primary_interactive.wgsl` (jitter, ray rotation, t), `src/primary_ray_gpu.rs` (resources, dispatch) and phase 1's generated traced shading wrapper (one call after `rl_finite`).

The API is opt-in, e.g. `FilmAccumulation::Temporal` on the traced renderer. Exact, Interactive and hard output stay byte-identical when it is off.

**Units:**

1. **T1a, static.** Jitter, sequence, ray rotation and cumulative accumulation, with no reprojection: a static camera only, and any change resets. Checks: T-0, Q-static, Q-cut, the replay match (diagnostic stop), F1/F2/F6/F8 and fixtures. This first validates the 1,024x32 reference generator against R1.
2. **T1b, motion.** Reprojection, validation, Catmull-Rom/bilinear fetch, the blur budget and generations. Q-walk, Q-zoom and Q-disocclusion under F1–F10.
3. **T1c, lights and movers.** Lag caps, mover cones and the reach boost. Q-blocker and the Q-sun set.
4. **T1d, timing and acceptance.** Paired triplets (3 processes × 24, the climb method) against Interactive and hard; the full F10 inspection; Opus acceptance.

**Expected verification:**

- 4–6 test builds;
- about 450 reference frames at about 3 s each (≈ 25 min of GPU);
- candidate sequences of about 10 min;
- about 5 min of timing;
- CPU scoring of about 15 min.

**Stop rules (frozen now):**

- **T-0 fails:** fix the implementation before anything else.
- **Static curve above 1.25x of §2** at any N in {1, 2, 4, 8, 16} on any population of R_L, R_D or R_F, either pose: the implementation or the replay is wrong. Diagnose; do not tune. The frozen F1-AA checks still apply: visibility partial > 0.12 at frame 8 or > 0.058 at frame 32, or R_L not ≤ Exact by frame 2.
- **F2 not met at age 7** on any population: stop. The next candidate changes design (cold-pixel extra samples on an exact trigger, then the split history), never the threshold.
- **Any F4 excess on R_D over Exact on any frame:** stop (frozen).
- **F1 failure under motion on background-centre, edge or multi-surface:** stop. The next candidates are a ±1 px reverse search, then two-layer silhouette history.
- **Q-sun-slow F5 failure:** stop (frozen). The Q-sun-fast miss is recorded with images and not traded for quality.
- **A determinism failure:** fix it. Keep frame-uniform values out of addressing.
- **Cost:** p50 above 1.075 ms, or any frame above 1.5x steady: stop (frozen). Report against 0.80 and 0.87–0.89 ms either way. Memory above 128 MB at 1080p: stop.
- **Q-walk steady error above 1.25x §4's model:** recompute v_max from the measured blur before continuing.
- **Default.** The default changes only after T1a–T1d pass and Islam accepts. Exact and Interactive stay selectable.

## 10. Risks

- **Motion results are a model (I).** It is calibrated within 4% for bilinear and 10–15% optimistic for Catmull-Rom at a single 1 cm step, and it assumes pure translation. Parallax is worse: multi-surface transfer measured 8.4 under forward-only validation against 4.8 modelled. Silhouettes under walking parallax are the most likely F1 failure. Two-layer history is the planned escalation.
- **The bar is a proxy here.** The frozen Exact uses the 1 mm policy, and the viewed proxy is one near-centre point with Exact rays. The re-baselined Exact decides. On R_D +4 the two errors differ 1.7x on all-covered.
- **N > 16 is extrapolated** from 16 samples. The 8-ray model resamples centre residuals and adds no noise to rejected samples (0.42% of covered-centre samples, plus silhouette foregrounds).
- **Mover and fast-sun regions** fall to W = 1. F4 on R_D in Q-blocker during motion is the main risk to the frozen stop. The 32-ray boost is a plausible but unmeasured mitigation.
- **Solids:** the 0.002 R_F maximum under 8 rays is unmeasured on film-lit footprints.
- **Cost is unmeasured:** history bandwidth, Catmull-Rom traffic, a dependent fetch in a latency-bound pass, and jitter's effect on traversal. The HRT floor is likely missed by up to 0.03 ms.
- **Programmable-shading constraints:** pure programs, exposure inside `shade`, post programs on jittered inputs, and time inputs. Each needs an API decision before it becomes real.
- **Texture double filtering** against a point-sampled path-traced reference: an estimand question for later references.
- **Scope:**
  - one quarry view, registered lighting (sun about 13% of radiance at cast boundaries), 1080p 1x;
  - the opaque, viewed-side estimand;
  - no GI, transparency or alpha test;
  - off-screen surfaces return cold.

## 11. Rejected options

| Option | Reason |
|---|---|
| R1's row-ordered prefix | Not progressive: 2.1x the N = 8 error (M) |
| Random first sample, or random per-frame jitter | Age 0 is 1.28x Interactive (fails F2). Random order is 6% worse at N = 8 (M) |
| Micro-normal agreement, triangle identity | Reject 4.5–70% of pixels whose history is 2–3x better than Exact (M) |
| Depth ≤ 2% alone | Misses same-instance folds under about 16 cm at 8 m |
| Unvalidated history | Best at 1 cm, but ghosts under disocclusion |
| Nearest-tap fetch | +45% transfer error (M) |
| Bilinear only | Silhouettes at 0.96–1.02x the bar under motion (I) |
| Fixed long window under motion | Bilinear 1/8 and 1/16 fail all-covered, R_D or silhouettes; Catmull-Rom 1/16 fails silhouettes (I) |
| Colour clamping (TAA/TSR style) | Style-dependent and heuristic; film gap §8 |
| Incident-only history | Cannot integrate shading aliasing, silhouettes or nonlinear programs; breaks direct-light/visibility covariance |
| Per-light output hooks required from programs | Rejected in programmable shading (`light()` callback) |
| f16 history colour | A rounding random walk of about 0.2% at W = 256 (I), comparable to the converged error |
| Spatial supersampling as the default | 1.58 ms at N = 2 (film gap) |
| A per-sample visibility cache now | Does not address footprint aliasing at W = 1; deferred behind the split history |

## Evidence, reproduction and time

Evidence root: `D:/Coding/renderlab-work/m6-exact-quality/t1/`:

- `temporal-replay.json` (all curves, baselines, motion models, transfer rules);
- `report.md` (the generated tables);
- `prefix-check.json`;
- the three inspection PNGs;
- `run-0{1..5}.log`.

Inputs, read in place and unchanged:

- `r1/quarry-viewed-p{0,1}/`;
- `m6-level-b-probe/runs/accept-b7/` (Interactive seeds 0–3);
- `m6-level-b-probe/gate-final/reference/` (point references, for population definitions).

```powershell
$env:OPENBLAS_NUM_THREADS='2'; $env:OMP_NUM_THREADS='2'
py -3 -B scripts/m6-temporal-replay.py --out D:/Coding/renderlab-work/m6-exact-quality/t1
py -3 -B scripts/m6-temporal-report.py D:/Coding/renderlab-work/m6-exact-quality/t1/temporal-replay.json
py -3 -B scripts/m6-temporal-prefix.py
```

Python needs NumPy, SciPy and Pillow; the replay imports F0's `m6-film-score.py` helpers.

**Verification time (CPU only, two math threads):**

| Run | Time | Outcome |
|---|---:|---|
| 1 | 79 s | Stopped on a fit edge case (all-zero solid curves) |
| 2 | ≈ 4 min | Stopped on JSON serialisation |
| 3 | 222 s | Complete |
| 4 | 295 s | Added the centre-first orders, the R1 reconciliation, the 2D model and the reverse test |
| 5 (retained) | 281 s | Fixed the full-image normalisation of image and background-centre in the motion models, which had overstated them 1.9x and 2.8x; added the 4-footprint reverse rule |
| Prefix check | 25 s, then 33 s | The rerun added the per-population second-sample ratios |

No retained number depends on the superseded runs. Design, reading and writing overlapped the runs and were not timed separately; the analysis ran from about 01:30 to 02:00 UTC. There was no GPU, window, build, Unreal launch or push.
