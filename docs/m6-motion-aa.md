# M6 motion AA: why resampled pixel history fails, and world-anchored sample reuse

Design decision, 2026-11-05, base `113b31e` (`t3/temporal-primary-rays`, T1b reverted). CPU only: saved R1 buffers, the T1b GPU evidence in `E:/renderlab-work/t1-gpu/`, and the installed UE 5.7.4 source. There was no production change, GPU run, window or build. Labels: **M** is measured here or in the cited evidence; **I** is a model, estimate or inference.

It answers [T1b's failure](m6-temporal-gpu.md#s2-t1b-camera-motion) under the bar in AGENTS.md: the final image against a film reference, static and in motion, with Unreal's high-fidelity rendering as the minimum. The frozen criteria are [T1 §7](m6-temporal-primaries.md#7-level-f-gate-for-t1)'s F1–F10, including the 0.002 R_F maximum on film-solid pixels. None is relaxed here.

**Status (2026-11-06): T2a ran on the GPU and stopped under three frozen rules for both K ([GPU result](m6-motion-aa-gpu.md)).**

- **Cost:** moving frames take 3.55 ms (K = 4) and 5.76 ms (K = 8), contended, against the 1.075 ms stop. The record gathers and exact rays cost 10–25x this design's estimate.
- **Replica-confirmed solids** fail F1.
- **Revealed pixels** fail F2.
- **What held:** silhouettes are antialiased in motion, and F1 radiance is within 0.84–0.85× of the bar. F4 and the CPU-model check pass, as do the static byte identity, T-0 and F8.
- **Defaults are unchanged.** The quality-first fallback in [§6](#6-verdict) stands.
- **Follow-up (2026-11-06, [GPU §9](m6-motion-aa-gpu.md#9-follow-up-the-outliers-and-the-scatter-price)):**
  - No T2 defect was found.
  - The umbra outlier is a correct second-surface sample in an edge pixel that the reference misclassified as solid.
  - The lit-side solids are receiver-defect samples outside the 1e-4 band test, plus two unexplained single-ray blocks from the production trace.
  - A scatter replacing both gathers is bit-identical and deterministic, but K = 8 moving frames still take 4.25 ms (K = 4: 3.50 ms). T2's per-pixel work cannot fit the 1.075 ms stop.

## Decisions

1. **Per-pixel resampled radiance history cannot meet the bar in motion on these scans (M, I).** A pixel's history is the integral over the footprint it had. A fractional-offset fetch mixes in neighbouring footprints. Next to a hard shadow edge, the leak into a film-solid pixel is 0.16–0.24 at T1b's window and stays above 0.002 out to 2–4 px. Keeping the leak under 0.002 needs a history weight of about 1%, which is no history ([§1](#1-why-resampled-pixel-history-fails)).
   - T1b measured a solid maximum of 0.40–0.61 on the GPU.
   - A calibrated CPU reproduction puts 16–17% of verified solids over 0.002.
   - On the real 1 cm step, one resampled fetch puts 16,315 verified solids over 0.002.
   - The anchor bit, two-layer history, a tighter budget near edges and Unreal TSR's 200% history all keep the leak.

   The class is rejected. A "motion-specific solid criterion" would relax the bar, and is not taken.
2. **Fix class: T2, world-anchored sample records.** Each pixel keeps its last K samples as records. A record holds the program output, the hit distance, the sub-pixel position and the placement.
   - Every frame, records move as points to wherever their surface point now projects.
   - A record counts only in the footprint it lands in, after geometric validation.
   - Nothing is interpolated, so a solid footprint can only collect solid samples.
   - Static frames keep T1a's cumulative mean, so static convergence is unchanged ([§4](#4-t2-design)).
3. **CPU evidence for T2 (M) ([§3](#3-cpu-prototype)):**
   - **2D motion simulation, K = 8 with the gap choice:** in motion, every population sits at 0.14–0.46× the bar, with verified solids exact and no trails. F4 is 774–967 against Exact's 11,629.
   - **Real 1 cm step:** 0.18× on all-covered and 0.38–0.43× on edge, multi-surface and background-centre. 57 of 290,780 verified solids exceed 0.002 (max 0.067). 33 of those are explained by the prototype's approximate sample depth; 24 are unresolved.
   - **Realizable silhouette test:** the prototype's cheap nearest-evidence test biases silhouettes and fails F2 (1.17× at age 4) and F3 on background-centre D0. Silhouettes therefore need exact camera rays.
4. **Cost (I):** in motion, 0.99–1.32 ms against Interactive's 0.866 ms ([§5](#5-cost)). That misses the 2x HRT floor (0.87–0.89 ms) and the VSM floor (0.80 ms). K = 4 is likely inside the 1.075 ms stop; K = 8 likely is not.
   - Memory is 0.32 GB (K = 4) to 0.59 GB (K = 8) at full 1080p allocation, above T1's 128 MB stop. With covered-tile pooling on the quarry it is 65–118 MB.
   - Static frames stay at T1a, about 0.89 ms.
5. **Verdict ([§6](#6-verdict)):**
   - T2 is the only candidate whose CPU evidence meets the bar's mechanisms in motion, and it misses both 2x floors while moving.
   - No approach meets the motion bar within the 2x floors. The miss is recorded, and quality is not reduced to pass.
   - Quality-first fallback: T1a static convergence plus an honestly labelled Interactive motion mode.
6. **Next unit: T2a on the GPU, Q-walk first,** with two pre-registered candidates (K = 4 and K = 8) and frozen stop rules ([§7](#7-next-unit-and-stop-rules)).

## 1. Why resampled pixel history fails

**Mechanism.** T1b's history value is the program-output integral over the footprint the pixel had, its film value then. Reprojection fetches it at a fractional offset f. Bilinear or Catmull-Rom combine the integrals of 4 or 16 old footprints, which partly overlap the new one and partly do not.

Take a 1D step edge at gap g outside a solid pixel. One bilinear fetch adds f·max(0, 1 − g − f) of the other state: 0.25 at f = 0.5, g = 0. The exponential history then carries it for W frames. Catmull-Rom reproduces quadratics, but not a step: its residue is the lobe ringing behind T1b's dark specks.

**Steady state (M, analytic: `scripts/m6-motion-aa-blur.py`).** The expected history, so sampling noise only adds to these values, under constant motion f px per frame. The table gives the worst |error| on film-solid pixels by gap between footprint and edge.

| Fetch, history | f | W | Gap 0–0.05 px | 0.25–0.5 | 0.5–1 | 1–2 | 2–4 |
|---|---:|---:|---:|---:|---:|---:|---:|
| Bilinear, 1x | 0.25 | 4.6 | 0.24 | 0.15 | 0.11 | 0.050 | 0.011 |
| Catmull-Rom, 1x (T1b) | 0.25 | 4.6 | 0.21 | 0.098 | 0.042 | 0.0065 | 0.0048 |
| Catmull-Rom, 1x | 0.10 | 4.6 | 0.16 | 0.055 | 0.031 | 0.0037 | 0.0006 |
| Catmull-Rom, 1x | 0.50 | 4.6 | 0.19 | — | 0.045 | 0.024 | 0.015 |
| Catmull-Rom, 1x | 0.25 | 2 | 0.11 | 0.047 | 0.016 | 0.0054 | 0.0015 |
| Catmull-Rom, 1x | 0.25 | 1.1 | 0.014 | 0.0082 | 0.0028 | 0.0015 | 0 |
| Catmull-Rom, 2x history (TSR at 200%) | 0.25 | 4.6 | 0.097 | 0.023 | 0.017 | 0.0085 | 0.0004 |

- **The bound fails at every useful W.** Even W = 1.1, a 9% history weight, leaks 0.003–0.014 within a pixel of the edge. Reaching 0.002 at gap 0 needs a history weight of about 1%. At that weight the pixel is a 1x Interactive pixel.
- **A 200% history halves the leak but keeps it.** At f = 0.5 a 2x history happens to move exactly one cell per frame and resamples nothing. That is a degenerate alignment, not a property.

**On these scans (M):**

- **GPU T1b on Q-walk** ([S2](m6-temporal-gpu.md#s2-t1b-camera-motion), `scores-walk/score.json`, frame 64, pixels aged 32 or more):
  - solid-lit R_F RMS 0.013 (max 0.42) and solid-occluded 0.045 (max 0.44), where Exact is 0;
  - contact R_F mean +0.095 against Exact's −0.088: light leaks into contact shadows;
  - 1,912 false shadows against Exact's 24;
  - occluded D4 3.03 against 1.93 codes;
  - 32 frames into the hold, the solid maximum is still 0.10, because blur decays only as 1/n.
- **CPU reproduction** ([§3](#3-cpu-prototype)): 47,796–49,954 of 291,403 verified solids exceed 0.002 (16–17%), max 0.40–0.45. Solid-lit RMS is 0.0133 against the GPU's 0.013.
- **One real 1 cm step:** pose 0's 16-sample mean, fetched bilinearly and blended at T1b's window, puts 16,315 verified solids over 0.002 (max 0.27). Occluded D4 is 1.03× the bar after one fetch.

Shadow structure on the quarry makes this the common case, not an edge case. A 3×3 block holds a median of 9 triangles, 74% of covered pixels exceed 20° normal spread, and visibility varies at triangle scale ([film gap §2](m6-film-gap.md#2-radiance-against-the-film-image)). Contact and crack shadows sit between solid pixels.

**Per-pixel fixes and what they leave (M from §3 unless noted):**

| Fix | What it changes | What remains |
|---|---|---|
| Mixed-history anchor bit | Trails 1.5–3 px into the sky: 17.6–19.0 → 0.03–1.6 codes. Edge L0 1.14–1.21 → 0.98–1.03× | Solids unchanged (47,799–49,753 over 0.002, max 0.40–0.45). Occluded D4 0.85–1.11×. F2 1.2–1.8× Interactive. F3 fails |
| Two-layer silhouette history (I) | Separates foreground and background motion | Each layer, and the coverage, is still resampled. Interior shadow edges are single-surface, so the table above applies unchanged. Double memory |
| Tighter budget near visibility edges | — | Needs W ≤ 1.01 at solid pixels near edges, so 1x there, plus an edge detector reading noisy current visibility. 16–17% of solids and every partial pixel would revert to 1x in motion |
| 200% history (TSR) | About 2x less leak | Above 0.002 out to 1–2 px. 4x history memory |
| Nearest-tap fetch | No interpolation | Misregisters by up to 0.5 px: +45% transfer error ([T1 §3](m6-temporal-primaries.md#3-reprojection-at-the-registered-1-cm-step)), and solid pixels take a neighbour's integral wholesale |

Only sub-pixel information re-integrates a moved footprint exactly: samples with positions.

## 2. Alternatives

| Approach | Reuses | Solid pixels in motion | Silhouettes | Cost (I unless marked) | Verdict |
|---|---|---|---|---|---|
| **T2: world-anchored sample records** | Point samples, moved exactly, kept only inside the footprint they land in | Exact unless a hidden or misplaced record is accepted | Exact with exact occlusion tests. The cheap nearest-evidence test is biased (§3) | +0.12–0.45 ms moving; memory 0.32–0.59 GB at 1080p | **Recommended** |
| Per-pixel resampled history (T1b, with bit, two-layer or 200%) | Footprint integrals, resampled | Fails structurally (§1) | Trails, or restarts with the bit | +0.205 ms measured (T1b) | Rejected |
| Radiance (program output) cached on surfaces, world or texture space | Shading per surface texel | Still needs per-frame sub-pixel visibility to integrate the footprint | Same | The scan has 13 M triangles of about 6 mm and no UV parameterisation; mm texels mean about 10⁸ texels | Rejected. It bakes view-dependent and user-authored output ([programmable shading §6](programmable-shading.md#6-rejected-options), [film gap §8](m6-film-gap.md#8-rejected-options)) and does not address the integration error, since shading is not the bottleneck |
| Visibility cached on surfaces (SVC, per-sample visibility cache) | Visibility | The visibility gap explains at most 7–9% of partial-pixel radiance error; 8-ray noise is under 1% of the N = 8 error (M, film gap, T1 replay) | No effect | 0.85–1.05 ms | Rejected for motion AA. Stays deferred for fast lights |
| Motion-adaptive sample counts | Extra fresh samples where history is missing | Does not fix resampled-history blur, which sits on interior solids | Helps edges at about 0.036·E ms for E extra samples on the ~18 k edge and near-silhouette pixels (5% of covered) | Revealed pixels: 79–622 per walk frame (M); 7 extra each costs ≤ 0.01 ms | An allocator, not a fix. Kept as T2's F2 escalation for revealed pixels |
| Stateless spatial supersampling | None | Exact | Exact | 1.58 ms at N = 2 (0.8–0.9× bar), 3.0 ms at N = 4, 11.8 ms at N = 16 ([film gap §4](m6-film-gap.md#4-fix-classes-priced)) | The offline film mode and the reference generator |

**What film renderers and Unreal do.**

- **Film renderers** (Cycles, production path tracers) integrate each frame independently at high sample counts, with motion blur over the shutter. They reuse no pixel history across final frames (I).
- **Unreal MRQ** renders each output frame from `SpatialSampleCount` × `TemporalSampleCount` full renders. Its own guidance is to override anti-aliasing to None for high-quality renders with many samples (`MoviePipelineAntiAliasingSetting.h:49–72`, M source). That is stateless supersampling, offline.
- **Unreal TSR**, its high-fidelity real-time path, is a per-pixel resampled history:
  - the history is fetched with Catmull-Rom (Stubbe's 5-tap form; `TSRUpdateHistory.usf:950–992`, `TSRKernels.ush:355`);
  - it is kept at 200% screen percentage on Epic and Cinematic, with a 16-sample cap (`TemporalSuperResolution.cpp:45–63`; `BaseScalability.ini:85–94`, `AntiAliasingQuality@3`: `r.TSR.History.ScreenPercentage=200`);
  - it has colour-based history rejection and clamping (`CONFIG_CLAMP 1`), flicker heuristics and history resurrection (`TSRUpdateHistory.usf:50–71`).
- **How Unreal's motion compares.** By §1's table, TSR's moving image has the same solid-pixel leak class, halved by its 200% history. Its ghosts are bounded by colour clamping, a style-dependent heuristic this design rejects ([film gap §8](m6-film-gap.md#8-rejected-options)). Unreal's high-fidelity motion is therefore probably also below the film bar in motion (I). It is unmeasured, and the 1.60/2.15 ms Unreal denominators exclude TSR's time.
- **Prior art (I).** T2 is the render-cache idea (Walter, Drettakis and Parker 1999: reprojected point samples with depth-based occlusion) with exact hit records and geometric validation. T1b was amortized supersampling (Yang et al. 2009), whose blur-adaptive weights and 2×2 sub-pixel history T1's budget and TSR's 200% history repeat.

## 3. CPU prototype

**2D motion simulation (`scripts/m6-motion-aa-sim.py`, M).**

- **World.** R1's pose-0 frame as a point cloud: in every pixel, its 16 saved samples (full HDR, sun-only HDR, Exact visibility) at their exact sub-pixel positions, plus the Exact centre sample. R1's jitter depends only on the sample index, so every position is known.
- **Motion.** The cliff stays fixed in pose-0 coordinates. The pixel grid moves by v_c per frame, and the uniform background moves by v_b, so silhouettes have parallax.
- **Fresh samples.** Each frame takes one true cloud point inside the current footprint. Nothing is interpolated.
- **Scoring.** At frames where t·v_c is integral, the grid coincides with pose 0's. Every pixel is then scored against R1's independent 240-stratum complement reference. The background is a constant colour, so its motion does not change the reference.
- **Bar.** Exact at the same footprint.
- **Verified solids.** The complement and all 17 cloud points agree exactly: 276,510 lit and 14,893 occluded. R1's 256-sample solids include partial footprints, as T1a's replica check found.
- **Motions.**
  - M1 (0.5, 0.5) px per frame, background relative motion 0.5 px, W = 3;
  - M2 (0.25, 0.875), W = 4.4, near the walk's measured W = 4.6;
  - M3 (0.125, 0.0625), W = 7.

  48 to 64 frames from cold.
- **Methods.**
  - `t1b`: T1b's fetch, forward and reverse tests, centre restart and window.
  - `t1b-bit`: adds the anchor bit.
  - `pK`: K records.
    - `gap` takes the fresh sample in the largest gap; `lattice` follows T1's sequence.
    - `nn` uses the realizable occlusion test for background records: the nearest evidence among geometry records and this frame's fresh samples decides. Without `nn`, the true cloud coverage decides, standing in for an exact camera ray.
    - `cells` averages 2×2 sub-cells first.
- **8-ray noise is not modelled.** The accept-b7 visibility captures were deleted. The T1 replay measured it at under 1% of the N = 8 error.

**Calibration against GPU T1b (M).**

| Quantity | GPU T1b, Q-walk | Sim T1b, M2 / M1 |
|---|---|---|
| L0 all-covered, partial (× bar) | 0.65–0.69, 0.61–0.65 | 0.65 / 0.75, 0.60 / 0.69 |
| L0 edge | 1.06–1.08 | 1.14 / 1.21 |
| Trail, background 1.5–3 px from silhouettes (L0 codes; Exact 0.001) | 9.8 | 19.0 / 17.6 |
| Solid-lit R_F RMS; solid R_F maximum | 0.013; 0.40–0.61 | 0.0133; 0.40 / 0.45 |
| Occluded D4 | 1.57–1.69 | 0.85 / 1.11 |

The sim reproduces T1b's failures. It under-predicts the occluded R_D failure: it has no parallax inside the cliff and one fractional offset per layer. T2 numbers on the same populations may be optimistic by the same mechanism, which bears on hidden records at folds (below).

**Results (M; worst over scored frames 32–64; ratios to the bar):**

| Run | L0 all-covered | L0 partial | L0 edge | L4 multi-surface | D4 occluded | D4 lit | Trail 1.5–3 px (codes) | Verified solids > 0.002 (max) | F4 missed + false (Exact 11,629) | D4 > 16 (Exact 7,471) |
|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|
| M1 t1b | 0.75 | 0.69 | **1.21** | 0.92 | **1.11** | 0.37 | **17.6** | **49,735 (0.45)** | 2,882 | 1,215 |
| M1 t1b-bit | 0.76 | 0.69 | **1.03** | **1.04** | **1.11** | 0.37 | 0.03 | **49,753 (0.45)** | 2,880 | 1,217 |
| M1 p8-lattice | 0.41 | 0.39 | 0.50 | 0.38 | 0.42 | 0.22 | 0.001 | 0 | 1,366 | 199 |
| M1 p8-gap | 0.32 | 0.28 | 0.41 | 0.28 | 0.36 | 0.18 | 0.001 | 0 | 879 | 56 |
| M1 p8-gap-nn | 0.32 | 0.29 | 0.45 | 0.28 | 0.36 | 0.19 | 0.001 | 0 | 874 | 60 |
| M2 t1b | 0.65 | 0.60 | **1.14** | 0.78 | 0.85 | 0.33 | **19.0** | **47,796 (0.40)** | 1,575 | 812 |
| M2 t1b-bit | 0.67 | 0.60 | 0.98 | 0.98 | 0.85 | 0.35 | 1.63 | **47,799 (0.40)** | 1,575 | 849 |
| M2 p4-gap | 0.46 | 0.40 | 0.54 | 0.46 | 0.46 | 0.21 | 0.001 | 0 | 1,873 | 228 |
| M2 p8-lattice | 0.37 | 0.36 | 0.44 | 0.32 | 0.41 | 0.18 | 0.001 | 0 | 1,015 | 108 |
| M2 p8-gap | 0.30 | 0.27 | 0.40 | 0.27 | 0.34 | 0.17 | 0.001 | 0 | 827 | 36 |
| M2 p8-gap-nn | 0.30 | 0.27 | 0.46 | 0.28 | 0.34 | 0.17 | 0.001 | 0 | 843 | 37 |
| M2 p8-gap-cells | 0.28 | 0.26 | 0.38 | 0.26 | 0.33 | 0.14 | 0.001 | 0 | 774 | 35 |
| M2 p16-gap | 0.15 | 0.14 | 0.35 | 0.18 | 0.18 | 0.095 | 0.001 | 0 | 40 | 7 |
| M3 t1b | 0.68 | 0.63 | 0.97 | **1.02** | 0.83 | 0.35 | **7.9** | **49,954 (0.41)** | 1,546 | 792 |
| M3 p8-gap | 0.35 | 0.34 | 0.33 | 0.30 | 0.35 | 0.15 | 0.001 | 0 | 962 | 60 |
| M3 p8-gap-nn | 0.35 | 0.34 | 0.43 | 0.32 | 0.36 | 0.16 | 0.001 | 0 | 967 | 61 |

Bold marks a failure.

- **Saturation.** Each footprint offers only its 17 cloud points, whose full mean scores 0.12× the bar, so p16 is near the sim's floor. Records held per pixel: mean 7.99 (p1 7) at K = 8.
- **F4 margin.** Exact's F4 and D4 > 16 counts are inflated by the 240-sample reference's noise, so the margin is wider than on the GPU's 1,024-sample references.

**Transient from the cold start (M1, aligned every 2 frames).** The worst population over L0, L4, D0 and D4, against the bar; in the sim, Interactive equals Exact.

- t1b: 1.34 / 1.80 / 1.52 / 1.79 at ages 2 / 4 / 6 / 8.
- t1b-bit: 1.35 / 1.77 / 1.20 / 1.52.
- p8-lattice: 0.82 / 0.78 / 0.68 / 0.71.
- p8-gap: 0.73 / 0.57 / 0.47 / 0.47.
- **p8-gap-nn: 1.02 / 1.17 / 0.76 / 0.92.** The failure is background-centre D0 only.

The nearest-evidence test over-rejects background records at silhouettes, because geometry records are dense and background evidence is one fresh sample per pixel. That biases mixed pixels toward geometry: edge D0 mean +0.15 codes against Exact's −0.07, so F3 also fires. With the true coverage, both pass.

**F3 (M).** T1b fires on 7–17 population/channel cases, including every solid R_F population. p8-gap fires on none at M1 and M3, and at M2 on D0 partial with a mean of −0.006 to −0.025 codes (2–4% of its RMS). That is consistent with selecting among a fixed 17-point set. p8-gap-cells and p16 fire on none. The `nn` variants add D0 edge.

**F6 proxy (M).** The L0 RMS change at the same footprint between aligned frames is 2.2–2.8 codes for p8-gap and 3.0–5.0 for t1b. The GPU walk measured T1b at 3.3–3.8 against Exact's 7.0–7.2. The proxy is distorted by the fixed candidate set: p4 reads 1.0 because its subsets recur.

**Inspection (M, `run-m2/M2-*-err-x8.png`).**

- **R_L +4:** T1b shows the cliff-coloured trail several pixels wide in the sky and doubled crack outlines. p8-gap leaves a 1-px silhouette line and faint cracks.
- **R_F:** T1b outlines every shadow edge with paired over and under bands, the blur signature. p8-gap shows isolated 1/8 speckle on partial pixels, plus faint smooth streaks along a few long penumbrae, attributed to the fixed candidate set and to be re-inspected on the GPU.

**Real 1 cm step (`scripts/m6-motion-aa-step.py`, M).** R1's registered pose pair has real parallax, folds, instance boundaries and silhouettes.

- **Placing samples.** Each of pose 0's 16 samples is placed on its pixel's centre tangent plane, or a covered neighbour's for silhouette samples, then projected into pose 1.
- **Accepting them.** Only by tests a frame can make from one hit per pixel:
  - geometry needs the same placement and a distance within 4 footprints of the landing pixel's hit plane (`own-plane`), or of a same-placement 4-neighbour's (`plane4`, T2's rule);
  - background needs the nearest coverage evidence to be background.
- **Estimate.** Accepted samples plus the frame's fresh sample (pose 1's centre), scored against pose 1's own film, which is independent of pose 0.

| Estimate | L0 all-covered | L0 partial | L0 edge | L0 multi-surface | L0 background-centre | D4 occluded | Verified solids > 0.002 (max) | F4 | D4 > 16 |
|---|---:|---:|---:|---:|---:|---:|---|---:|---:|
| Pose 1's own 16 samples, static (vs complement) | 0.12 | 0.11 | 0.15 | 0.13 | 0.16 | 0.16 | 0 | 35 + 2 | 1 |
| **T2 rule (`plane4`)** | **0.18** | **0.15** | **0.41** | **0.38** | **0.43** | **0.20** | 57 (0.067) | 259 + 2 | 50 |
| `own-plane` | 0.29 | 0.28 | 0.86 | 0.61 | 1.00 | 0.74 | 57 (0.067) | 259 + 2 | 290 |
| Placement only | 0.23 | 0.19 | 0.85 | 0.57 | 1.00 | 0.72 | 57 (0.067) | 259 + 2 | 122 |
| T1b step: bilinear fetch of pose 0's 16-sample mean, T1b blend | 0.41 | 0.37 | 0.90 | 0.61 | **1.10** | **1.03** | **16,315 (0.27)** | 188 + 583 | 459 |
| *Exact, pose 1* | *1* | *1* | *1* | *1* | *1* | *1* | *0* | *11,462 + 15* | *7,466* |

- **Acceptance.** `plane4` accepts 16.0 records per pixel and rejects 0.09% of geometry samples. The own plane alone rejects 1.6%, concentrated at folds and silhouettes, so silhouettes fall to 1x.
- **The 57 solid failures** are 2.0e-4 of pose 1's 290,780 verified solids; every one is a single opposite or partly opposite sample among 15–17.
  - 33 vanish when records within 0.02 px of a footprint edge are excluded: the prototype's tangent-plane depth misplaces samples by about 0.01 px. A stored hit distance does not.
  - 24 remain (max 0.068). The samples come from pose-0 shadow-edge pixels and agree with the landing pixel's own plane. Each is either a truly partial footprint that 257 verification samples missed, or a sample hidden at a fold shallower than 4 footprints.

  The CPU cannot separate the two; the GPU unit's replica check and exact hidden-record diagnostic can.

**F1–F6 estimate for T2, K = 8, gap (M from the above; GPU unmeasured):**

| Check | Estimate | Basis and open item |
|---|---|---|
| F1 RMS | Pass with 2.2–7x margin: worst 0.46× (edge, `nn`) in the sim, 0.43× on the real step | Sim and step |
| F1 solid maximum | Sim: pass (0 of 291,403). Real step: 57 over 0.002, 24 unexplained | Replica verification and the hidden-record diagnostic on the GPU |
| F2 | Pass with exact silhouette tests (0.47–0.73× from age 2). Fails with the nearest-evidence test (1.17× at age 4, background-centre D0) | Exact rays at silhouettes (§4) |
| F3 | Pass except D0 biases of 0.02 codes on partial (M2) and +0.15 codes on edge (`nn`) | Re-measure without the fixed candidate set |
| F4 | Pass by more than 10x on both counts | Exact's counts are inflated by reference noise |
| F5 | No trails (0.001 codes against T1b's 8–19). Zero weight from hidden surfaces holds by construction in 2D; unmeasured at real folds | Exact hidden-record diagnostic |
| F6 | Proxy at or below T1b, whose GPU value already passed | GPU |
| F7–F10 | Not estimated | GPU |

**Not covered by the CPU evidence:**

- occlusion at folds over many frames: the sim has none, and the step is one frame;
- view-dependent program output;
- lights and movers;
- Q-zoom;
- continuous jitter (the sim's candidates are 17 fixed points);
- 8-ray noise;
- cost.

## 4. T2 design

**Record (16 B).**

- program output as 3 × f16;
- sub-pixel position in the pixel that holds it (2 × u8);
- hit distance along its ray (f32);
- placement (u8);
- thin side bit;
- age (6 bits).

Single samples in f16 do not random-walk; T1 rejected f16 only for the running mean.

**Per pixel.** K records ×2 (read and write), a count, and T1a's static mean and anchor.

**Moving frame:**

1. **Gather (deterministic).** Each covered pixel reads the records of the 3×3 pixels around its reprojected centre, using the motion of its previous fresh hit as T1b did. It rebuilds each record's world point: the previous camera ray through its sub-pixel position, times the distance, with the instance's previous transform for movers. It projects the point with the current camera and keeps the records that land inside its own footprint, in a fixed source order. Background records move by direction.
   - Scatter with atomics would read less, but its arrival order breaks F8 bit-identity unless arrivals are sorted per pixel.
2. **Trace (fused Interactive).** The fresh position is the one of the next 16 T1 lattice positions that lies farthest from the arrived records. An empty pixel takes the centre, so a revealed or reset pixel's first frame is Interactive's and T-0 holds. The trace also writes the hit distance and geometric plane (16 B per covered pixel).
3. **Validate**, in the generated shading wrapper after `rl_finite`:
   - **Geometry record:** same placement, and within 4 footprints of the pixel's own fresh hit plane or a same-placement 4-neighbour's, along the current ray.
   - **Discontinuity pixels:** where the 3×3 fresh hits mix geometry and background, or span more than 4 footprints in depth, each arrived record also gets an exact camera any-hit ray to its point (to infinity for background).
   - **Elsewhere:** background records are kept when the 3×3 fresh hits are all background.

   The cheap nearest-evidence silhouette test is rejected (§3).
4. **Accumulate.** Insert the fresh record, evicting the oldest. Output the mean of the valid records and the fresh sample.
5. **Caps.** Record age is capped by T1 §5's W_light and W_mover and by a view cap (below). Records of a placement whose generation changed are dropped.

**Static frame (camera and scene unchanged).** There is no gather. T1a's cumulative mean updates exactly as now, so static output is T1a's byte for byte. The fresh sample also replaces the pixel's oldest record in place, so records are current when motion starts.

- When motion starts, the mean is dropped, because an integral cannot move. Output is the records' mean, the K-sample level of §3.
- When motion stops, the mean is seeded from the records (n = count) and T1a continues.

**Programmable shading ([§2.4](programmable-shading.md#24-realistic-path-quality-bar-and-world-space-inputs)).** Records store program output, as T1's history did ([T1 §6](m6-temporal-primaries.md#6-programmable-shading)): the engine-owned film reconstruction, the one recorded exception to "filter incident quantities". Validation reads only geometry, never colour, so every style converges to its own output.

- **New constraint (I).** A reused record's output was computed for the view direction of its own frame. View-dependent terms such as specular and rim therefore lag by at most the record age, which is at most K frames: 10 mrad at walking speed for K = 8. Programs declare a view bandwidth, or records get a default view-angle cap. The cap's value is an open decision; 0.5° is about 7 frames at 1.25 mrad per frame.
- Nothing enters the hit record or dispatch beyond geometry.

**Screen-space status.** Every sample's lighting is ray-traced in world space. Records are world points, validated by exact geometry or exact rays. Their failure mode is a cold, unbiased 1x sample. Surfaces returning from off screen start cold, as in T1.

## 5. Cost

**Measured anchors (M):**

- Interactive: 0.866 ms quiet.
- T1a: +0.025 ms (contended) with history in hit tiles. Its unrestricted 48 B per pixel over every pixel (about 100 MB per frame) cost +0.10–0.115 ms.
- T1b: +0.205 ms for a 16-tap fetch with forward and reverse tests.
- Hard trace, all primaries: 0.23–0.27 ms.

T2 per moving frame on the quarry, about 0.36–0.40 M covered pixels (I):

| Pass | K = 4 | K = 8 | Basis |
|---|---:|---:|---|
| Gather: 9 sources × K records from cache, K × 16 B unique read and write | +0.06–0.15 | +0.10–0.25 | T1b's 16-tap fetch (+0.205) for the access pattern; T1a's 0.1 ms per 100 MB for the traffic |
| Trace: gap choice (K positions) and plane write | +0.01 | +0.01–0.02 | T1a's trace +0.008–0.011 |
| Validate and accumulate | +0.03–0.05 | +0.04–0.08 | Register-resident records; plane reads of 5 pixels |
| Exact rays in discontinuity pixels: 29–60 k background records in the band (M, sim) plus geometry records in about 22 k edge and multi-surface pixels | +0.02–0.05 | +0.04–0.10 | Background rays mostly miss the BVH early; about 0.25 ms per 0.36 M primaries |
| **Moving span** | **0.99–1.13 ms** | **1.06–1.32 ms** | 0.866 + the passes |
| Static span | ≈ 0.89 ms | ≈ 0.89 ms | T1a, quiet extrapolation, plus a 16 B in-place write |

- **Floors.** In motion both K miss the 2x HRT floor (0.87–0.89 ms) by 0.10–0.45 ms and the 2x VSM floor (0.80 ms) by 0.19–0.52 ms.
- **Stop.** Against the frozen 1.075 ms stop, K = 4 is likely inside and K = 8 likely outside. Unreal's floors exclude TSR, whose time is unmeasured.
- **Memory (I).**
  - Full 1080p allocation: K = 4 is 2 × 64 + 4 + 24 = 156 B per pixel, 0.32 GB; K = 8 is 284 B per pixel, 0.59 GB. At 4K, 1.3–2.4 GB.
  - Pooled to covered tiles on the quarry (about 20% of tiles): 65 and 118 MB.
  - T1's 128 MB stop at 1080p is exceeded by any full allocation. Pooling meets it on the quarry but not in a fully covered view.
- **Not paid.** No extra shading: the program runs once per pixel per frame, as in Interactive.

## 6. Verdict

1. **Resampled per-pixel radiance history is fundamentally unable to meet the bar in motion on these scans.** The solid-pixel leak is structural, sits at triangle-scale shadow structure, and needs about 1% history weight to clear 0.002. It is measured on the GPU, in a reproduction calibrated to it, and on the real step. Every per-pixel fix, including Unreal's 200% history, keeps it.
2. **T2, world-anchored sample records, is the approach the CPU evidence supports**, under one condition: exact camera rays validate records in discontinuity pixels. Its expected motion quality is:
   - 0.28–0.35× the bar on covered pixels at K = 8, and 0.46× at K = 4;
   - 0.33–0.46× on edges, at the K = 8 runs of §3;
   - exact solids in the sim;
   - a real-step solid exceedance rate of 2e-4, half of it explained by the prototype.

   Whether it meets the bar is not established until the GPU unit passes, so nothing here is presented as meeting it.
3. **Cost.** In motion T2 costs 0.99–1.13 ms (K = 4) or 1.06–1.32 ms (K = 8) against Interactive's 0.866 ms, missing both 2x floors; static frames stay at T1a's ≈ 0.89 ms. **No approach found meets the motion bar within the 2x floors.** That miss is recorded, and quality is not reduced to pass it.
4. **Quality-first fallback, if T2a fails.** The current branch head:
   - **Static:** T1a converges to the bar by frame 2–4 and to 0.02–0.09× by frame 128, at ≈ 0.89 ms.
   - **Motion:** each frame is Interactive at 0.866 ms. R_L is within 0.01 codes of Exact (1.00× the bar), and 9,239 film shadows are missed against Exact's 8,566 at pose 0, so F4 fails against Exact by 8%.
   - **Labelling.** The motion mode is presented as "1x Interactive in motion, below the film bar". It is never a default, and never presented as meeting the bar. Defaults stay unchanged.

## 7. Next unit and stop rules

**T2a: records under camera motion on the GPU.** It runs after the receiver-origin fix lands, or lists that band's pixels separately, and only when Islam is not playing. GPU windows also require 6 GiB of RAM available for the GPU worker.

**Ownership.** Implementation goes to Sol; acceptance to Opus.

- `src/film_records.rs` (resources, constants K, frame state);
- `shaders/film_records.wgsl` (gather, gap choice, validation, estimate);
- tests in `src/film_accumulation_tests.rs`;
- hooks in `film_accumulation.rs`, `film_store.wgsl`, the primary trace (jitter seam, plane write) and `renderer.rs`.

`b8fa1f3` already has reusable pieces: the motion frame state, a reset key without the camera, the cold-pixel centre retrace, the sample record buffer, the F8 replay harness, the walk references and the motion scorer.

**Candidates, registered now.** One binary with K as a constant:

- **T2a-4:** K = 4, gap choice, exact discontinuity rays.
- **T2a-8:** K = 8, same rules.

**Sequences.** Q-static (regression), Q-walk, then Q-disocclusion and Q-zoom.

**Diagnostics:**

- solid verification with 16 replicas, as T1a did;
- a test-only exact hidden-record count: one camera any-hit ray per accepted record, feeding F5's zero-weight check;
- the CPU-model check: Q-walk all-covered L0 within 1.25× of §3, so at most 0.58× (K = 4) and 0.44× (K = 8).

**Stop rules (frozen now):**

| Rule | Action |
|---|---|
| T-0: a reset or empty pixel's age-0 frame differs from Interactive | Fix first |
| Static: Q-static output differs from T1a byte for byte | Fix first |
| A replica-confirmed solid over 0.002 outside the receiver-defect band | Stop. If the hidden-record count shows accepted hidden records there, the next candidate sends exact rays for every record in a widened discontinuity band, priced. Never relax |
| F1 above the bar, or F2 above Interactive or above the bar at age 7, on any population | Stop. For background-centre or edge, widen the exact-ray band. For revealed pixels, use the cold-pixel boost (§2). Never move the threshold |
| F4: any R_D excess over Exact on any frame | Stop (frozen) |
| F8: a replay is not bit-identical (records included) | Fix: keep the gather order fixed and frame-uniform values out of addressing |
| CPU-model check exceeded | Diagnose before tuning |
| Cost: moving p50 above 1.075 ms, or any frame above 1.5x steady | Stop (frozen). Report against 0.80 and 0.87–0.89 ms either way |
| Memory: pooled records above 128 MB at 1080p on the quarry | Stop. The full-allocation figure is reported |

**Decision for Islam before T2a.** T1's 128 MB memory stop holds only with pooling on scenes like the quarry. A fully covered 1080p view needs 0.32 GB (K = 4) to 0.59 GB (K = 8).

**Expected verification:** 3–5 test builds; Q-walk references reused from `walk-ref` where cameras match; about 10 min of GPU sequences; about 5 min of timing; about 10 min of CPU scoring.

## 8. Rejected options

| Option | Reason |
|---|---|
| Per-pixel resampled radiance history (T1b) | Structural solid-pixel leak (§1, M) |
| Mixed-history anchor bit | Removes trails only; solids, occluded R_D, F2 and F3 still fail (M) |
| Two-layer silhouette history | Each layer is still resampled, and interior shadow edges are single-surface (I) |
| Tighter blur budget near visibility edges | Needs a history weight of about 1%, so 1x at 16–17% of solids and every partial pixel. Needs a visibility edge detector (M, I) |
| Motion-specific solid criterion | Relaxes a frozen criterion |
| 200% (TSR-style) history | Halves the leak, keeps it, quadruples memory (M analytic) |
| Nearest-tap fetch | Half-pixel misregistration (M, T1 §3) |
| Colour clamping or rectification (TSR) | Style-dependent heuristic ([film gap §8](m6-film-gap.md#8-rejected-options)) |
| Surface radiance cache (world or texture space) | Bakes program output and leaves footprint integration unsolved (I) |
| Visibility cache for motion AA | At most 7–9% of the error; footprint aliasing dominates (M) |
| Adaptive counts as the fix | An allocator; does not fix resampled blur. Kept as T2's revealed-pixel boost |
| Stateless supersampling in real time | 1.58 ms at N = 2 (I) |
| Nearest-evidence silhouette validation (`nn`) | Biases silhouettes: F2 1.17× at age 4, F3 D0 edge (M) |
| Scatter with atomics, unsorted | Arrival order breaks F8 bit-identity |
| Lattice fresh positions without the gap choice | 0.37–0.41× against 0.30–0.32× all-covered at K = 8 (M); the gap choice costs only K position reads |

## 9. Risks

- **Folds.** Records hidden at folds shallower than 4 footprints are accepted until they leave. Per frame they sweep about 1.2 px × Δd/d (0.005 px at 4 footprints for the 1 cm step), so a pixel holds about K²·0.005 ≈ 0.3 stale records next to such folds at K = 8 (I). They are wrong only where a visibility change lies within that sliver. These may be the 24 unexplained real-step failures. The exact hidden-record diagnostic decides.
- **The 2D sim is optimistic.** It under-predicted T1b's occluded R_D by about 1.5–1.9x, and it has no folds or per-pixel offsets. Its candidate set is 17 fixed points per pixel, which caps p16 and distorts F3 and F6.
- **Cost is entirely I**, and moving frames miss both 2x floors. GPU gathers in this latency-bound pass have historically cost more than byte counts suggest: T1b's +0.205 ms.
- **Memory** exceeds T1's stop in fully covered views.
- **View-dependent program output** lags by up to K frames. The view cap is undecided.
- **Lights, movers** (T1 §5 caps apply to record age) **and Q-zoom** (record density per pixel changes with scale) are unmeasured.
- **Unreal's TSR-on motion image and time are unmeasured,** so no comparative claim is made either way.

## Evidence, reproduction and time

Evidence root `E:/renderlab-work/motion-aa/` (5.7 MB):

- `run-m13/` and `run-m2/`: `motion-aa-sim.json` (every frame, population and channel), `run.log`, and the inspected ×8 error crops `M*-*-{L4,F}-err-x8.png`;
- `step/step.json`: the real step, with the solid-failure diagnosis;
- `blur.json`: the §1 table.

Inputs were read in place and left unchanged:

- R1 `quarry-viewed-p{0,1}`;
- the frozen point references;
- `E:/renderlab-work/t1-gpu/scores-walk/` (`score.json`, `silhouette-split.json`, crops) and `sequences/`;
- UE 5.7.4 sources under `C:/Program Files/Epic Games/UE_5.7/Engine`.

The accept-b7 Interactive visibility captures that the T1 replay's 8-ray model needs no longer exist.

```powershell
$env:OPENBLAS_NUM_THREADS='2'; $env:OMP_NUM_THREADS='2'
py -3 -B scripts/m6-motion-aa-sim.py --out E:/renderlab-work/motion-aa/run-m13 --only M1,M3
py -3 -B scripts/m6-motion-aa-sim.py --out E:/renderlab-work/motion-aa/run-m2 --only M2
py -3 -B scripts/m6-motion-aa-step.py --out E:/renderlab-work/motion-aa/step
py -3 -B scripts/m6-motion-aa-blur.py --out E:/renderlab-work/motion-aa/blur.json
py -3 -B scripts/m6-motion-aa-report.py E:/renderlab-work/motion-aa
```

Run the simulations one at a time. Each takes about 1.7–2.3 GiB; start one only with 8 GiB available.

**Time (UTC).** Reading, design and implementation were interleaved from about 16:40 to 17:15. Verification:

- four smoke runs of 22–68 s;
- two full simulations of 1,134 s and 1,151 s, from 17:00 to 17:20;
- five real-step runs of 33–39 s, adding the diagnosis, the T1b baseline and the fresh sample;
- the blur analysis, about 2 s;
- the report, under 1 s.

The two full simulations ran concurrently. Their 2.3 + 1.7 GiB pushed available RAM to about 2 GiB and blocked the receiver worker's GPU runs, until root stopped further parallel starts; both finished unaided. There was no GPU, window, build or Unreal launch.
