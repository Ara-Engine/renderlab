# M6 T1 on the GPU: references and static film accumulation (T1a)

2026-11-05, base main `43d9184` (viewed-side receiver policy, programmable shading phase 1, the determinism FMA fix and fast reload), branch `t3/b4519ec1`. This implements and measures the first unit of [T1](m6-temporal-primaries.md#9-gpu-implementation-plan-and-stop-rules). RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1, release library test binaries built with two Cargo jobs, the registered full-resident quarry (12 draws, 13,034,903 triangles) at 1920×1080 1x, 16x anisotropy, a 0.04-rad disk. Every GPU run was offscreen under the shared lease; no window, viewer, desktop input, game capture or Unreal launch.

**Result.** The film references were regenerated under the adopted receiver policy and agree with R1. T1a (static accumulation) is implemented as the opt-in `SoftShadows::Temporal`, with the Exact, Interactive and hard modules untouched. It passes T-0, the frozen F1-AA checks, the 8-frame limit, F1 on radiance and on partial-type visibility, F2, F4, F6, F8 across processes and Q-cut's cold identity.

**It has one recorded miss: the F1 solid bound on R_F (maximum ≤ 0.002).** On references verified with 17,408 samples, 27–64 pixels per pose out of 0.29–1.1 M exceed it, by one sample in 128. Most come from the lattice's exact pixel-corner sample ([§4](#solids-the-recorded-miss)). A corner-free candidate cuts them by 4–6x but does not clear the bound either. Fixtures were not run.

Cost on a contended screen is +0.124–0.128 ms per frame over Interactive, so T1a would land near 0.99 ms against the 0.87–0.89 ms HRT floor: a likely miss, unconfirmed on a quiet machine. T1b–T1d (motion, lights and movers, timing acceptance) were not started; the gate's miss needs a decision first, since each stage is kept only if its gate passes. Defaults are unchanged.

## 1. Film references under the adopted policy

**Generator** (`src/film_accumulation_tests.rs` `gpu_film_t1_reference`, `support/film_reference_t1.wgsl`). As proposed in [T1 §7](m6-temporal-primaries.md#7-level-f-gate-for-t1):

- 1,024 jittered primary samples per pixel in two independent halves of 512. Each half is a 32×16 stratification of the footprint with one uniformly random point per stratum, so each half alone is an unbiased footprint estimate and their difference measures reference noise.
- Each sample has 32 independently salted shadow rays from the production estimator (`FRESH_DIRECTIONS` = 32) at the production receiver (viewed side, `max(10 µm, 16 ulp)`, ε t_min).
- Each sample is shaded with the standard program (R_L) and with its sun-only variant (R_D, environment reset before the light loop, black background). Visibility (R_F) is conditioned on the centre surface as in F0/R1: same placement and depth within 2%. Radiance includes every sample.
- The same run saves the production Exact frame (seeds 0–7) and Interactive frame (seeds 0–3) at each pose: full and sun-only HDR and counts.
- Poses: 0 and 1 (the registered pair) and 2, a near view registered for Q-cut (pose 0's eye halfway to its target; `E:/renderlab-work/t1-gpu/cameras.json`, SHA-256 `28db006e…`).

A reference takes 5.3–5.9 s of GPU time with validation (the frame's own wall time in its JSON), against 10.9 s for a 256×256 R1 reference. Partial-population R_F noise (split-half σ RMS) is 0.0020, against R1's 0.0138.

**Agreement with R1's 256×256 viewed references** (M). The new reference minus R1, per population, against their combined noise: the new reference's split-half noise plus R1's per-sample variance from its 16 saved samples divided by 256. R1's term is conservative, since stratification lowers its true noise.

| Pose | Channel | partial | lit | occluded | multi-surface | background-centre | image |
|---|---|---:|---:|---:|---:|---:|---:|
| 0 | R_L, codes (RMS / noise) | 0.167 / 0.654 | 0.095 / 0.375 | 0.155 / 0.559 | 0.319 / 1.021 | 0.027 / 0.079 | 0.052 / 0.192 |
| 0 | R_D +4, codes | 0.125 / 0.546 | 0.041 / 0.171 | 0.043 / 0.156 | 0.184 / 0.669 | 0.014 / 0.044 | 0.028 / 0.115 |
| 0 | R_F | 0.005 / 0.013 | 0.000 / 0.000 | 0.000 / 0.000 | — | — | — |
| 1 | R_L, codes | 0.168 / 0.652 | 0.095 / 0.375 | 0.153 / 0.562 | 0.316 / 1.029 | 0.030 / 0.079 | 0.053 / 0.192 |
| 1 | R_D +4, codes | 0.127 / 0.548 | 0.042 / 0.171 | 0.052 / 0.153 | 0.189 / 0.669 | 0.015 / 0.044 | 0.029 / 0.115 |

Differences are 0.23–0.43 of the combined noise at both exposures, with mean signed differences of at most 0.016 codes. R_F partial and faint are 0.4–0.5 of their combined noise. No pixel changes between solid and partial by more than 0.1 in R_F. The generator is accepted.

## 2. Exact and Interactive re-measured: the bars

Against the new references, with production Exact (seed 0) and its spread across seeds 0–7. The bar is E·(1+s) with s the 2-s.d. relative seed spread; s is below 0.001 on every radiance population, so the bar equals Exact here. Pose 0 (pose 1 agrees within 3%; pose 2 is the near view):

| Population | R_L | R_L +4 | R_D | R_D +4 | R_F | Doc's bar, R_L / R_D +4 / R_F |
|---|---:|---:|---:|---:|---:|---|
| partial | 9.383 | 16.536 | 1.067 | 7.175 | 0.200 | 9.35 / 7.28 / 0.205 |
| faint | 8.667 | 13.164 | 1.061 | 5.051 | 0.077 | 8.52 / 5.17 / 0.086 |
| contact | 10.429 | 20.941 | 0.761 | 7.768 | 0.192 | 10.31 / 7.65 / 0.195 |
| lit | 4.719 | 6.509 | 0.447 | 1.739 | 0 | 4.78 / 1.83 / 0 |
| occluded | 7.822 | 14.679 | 0.190 | 2.052 | 0 | 7.69 / 2.01 / 0 |
| multi-surface | 13.529 | 21.726 | 1.540 | 8.273 | 0.118 | 13.66 / 8.44 / 0.116 |
| background-centre | 1.090 | 2.481 | 0.063 | 0.582 | — | 1.07 / 0.58 / — |
| all-covered | 5.813 | 9.194 | 0.575 | 3.203 | 0.077 | 5.82 / 3.25 / 0.078 |
| image | 2.576 | 4.388 | 0.242 | 1.413 | — | 2.58 / 1.43 / — |

The doc's bars were min(1 mm Exact, viewed proxy) on R1's complement references. The re-baselined bars agree within 5% and are stricter where the 1 mm policy had biased R_D (lit: 1.739 against 1.83). Production Interactive (seed 0) matches Exact within 0.01 codes on R_L and within 1.3% on R_D; it misses 9,239 film shadows against Exact's 8,566 (pose 0).

Bars, populations and all per-seed scores are in `scores-ref/score.json` and `scores-t1g/score.json`.

## 3. What T1a implements

- **API.** `SoftShadows::Temporal`, opt-in on traced soft frames. Leaving it releases the history. Hard, raster and diagnostic frames are unaffected, as for Interactive.
- **Sample.** Each frame traces one primary per pixel at sequence index k (frames since the pixel's last reset): the 256-point rank-1 lattice (j, 71 j mod 256)/256 in bit-reversed order, with the first four reordered to centre, quarter pair, corner. Every power-of-two prefix from 4 to 256 is a lattice, and the 16-point prefix is the design's (71 ≡ 7 mod 16). Per pixel, a hash picks one of the four centre-fixing symmetries. Reflection preserves every prefix; transposition preserves prefixes up to 16, since 71² ≢ 1 (mod 32). The generator was chosen to maximise the worst prefix's toroidal minimum distance (0.745 of hexagonal packing); CPU tests check the order, the prefixes and the symmetries.
- **Rays.** Sample k uses directions 8 (k mod 32) … 8 (k mod 32) + 7 of the pixel's 256-direction Exact sequence. Index 0 is Interactive's set; 32 consecutive frames cover Exact's 256 directions.
- **Accumulation.** The program's output, after `rl_finite`, updates a per-pixel f32 mean and weight: m′ = m + (x − m)/n′ with n′ = min(n + 1, 256), the multiply-add pinned with `fma`. The weight reads no colour. History is ping-ponged: Rgba32Float mean and weight plus an Rg32Uint anchor (placement + 1, k, view distance), 48 B per pixel (99.5 MB at 1080p).
- **Resets.** T1a holds a static view. A frame resets every pixel and reads no history whenever any of these changed: the camera, the prelude or program revision, the frame's program, the draw tables or bindings, the TLAS, any GPU write of geometry, transforms, materials, parameters, textures, lights, lighting, exposure or environment, the shadow settings, or the shadow views.
- **Reset frames are Interactive frames.** A reset frame dispatches the production Interactive kernel and the program's own Soft shading pipeline, then `film_init` (`shaders/film_init.wgsl`) starts every history from that image: mean = the stored HDR, weight 1, k = 0, and the anchor from the hit record. T-0 therefore holds by construction. The first implementation shaded reset frames with the temporal module at k = 0, which relied on the driver compiling the program identically in two modules; build b9 added only engine anchor arithmetic to that module and broke byte identity at pose 0. The reset pass adds one light dispatch to cut frames, not a second shading pass.
- **Anchor arithmetic.** The anchor's view distance is computed from the hit triangle's receiver point with every multiply-add explicit (`film_view_distance`), not from the program-facing interpolated position.
- **Composition.** The temporal kernel is the production Interactive kernel module with three seam edits; each program's temporal shading module is its Soft composition with the ray seam, plus every store routed through `film_store`. Seams must match exactly once or the composition is rejected. The Exact, Interactive and hard modules never see them, so their sources, and therefore their outputs, are unchanged. The temporal kernel compiles on the first temporal frame and recompiles inside a prelude reload once used. Each program's temporal pipeline compiles on first use, as `Variant::Temporal`.
- **Programmable shading.** The accumulate call is engine code after the program in the generated entry, as [T1 §6](m6-temporal-primaries.md#6-programmable-shading) specifies. The recorded constraints apply unchanged: programs are pure functions of their inputs; `shade` returns exposed HDR, so exposure edits reset; post programs would read the current jittered sample; a future `time` input needs W = 1 or a declared bandwidth. No lighting-model data enters the hit record, the anchor or dispatch.
- **Files.** `src/film_accumulation.rs`, `shaders/film_accumulation.wgsl`, `shaders/film_store.wgsl`, `src/film_accumulation_tests.rs`, `support/film_reference_t1.wgsl`, `support/film_t1_visibility.wgsl`, `scripts/m6-temporal-gpu-{run,score}.py`. Hooks: `primary_ray_gpu.rs` (state, layouts, pipelines, generations), `program_gpu.rs` (`Variant::Temporal`), `renderer.rs` (frame path, key, reload) and one-line additions in `shadows.rs` and `shadow_gpu.rs`. `shaders/scene.wgsl` and `shaders/programs/standard.wgsl` were not edited.

## 4. T1a gate

Final build b10 (`builds/b10`, runs `t1g` and `t1h`). Static Q-static runs of 128 frames from cold at poses 0, 1 and 2 with the standard, sun-only and visibility programs, checkpoints at N = 1–8, 12, 16, 24, 32, 48, 64, 96 and 126–128. T1's R_F is its own samples' visibility on the centre surface (`support/film_t1_visibility.wgsl`), the reference's estimand; T1's plain visibility-program mean also counts other surfaces and background and is not comparable.

| Check | Result |
|---|---|
| T-0 age-0 identity | **Pass.** 9/9: three poses × three programs, HDR, hit records and counts byte-identical to production Interactive |
| F1-AA frozen | **Pass.** Visibility partial 0.074 at frame 8 (≤ 0.12) and 0.019 at frame 32 (≤ 0.058); R_L image 1.95 at frame 2 against Exact 2.58 (pose 0; poses 1 and 2 agree) |
| 8-frame limit | **Pass.** Every population and channel reaches the bar by frame 2–4 at all three poses |
| F1 converged (N ≥ 48) | **Pass on R_L, R_D and partial-type R_F**: 0.02–0.09 × bar at frame 128. **Miss on the solid R_F maximum** ([below](#solids-the-recorded-miss)) |
| F2 transient | **Pass.** ≤ production Interactive at every age (age 0 is identical) and ≤ bar at age 7, every population, R_L and R_D, both exposures |
| F3 bias | Pass except on solid R_F populations, where Exact's mean and spread are exactly zero (same root cause as the miss) |
| F4 missed / false shadows | **Pass** from frame 8: frame 128 has 0 / 0 at every pose against Exact's 8,566 / 13, 8,464 / 16 and 16,312 / 24. R_D +4 pixels over 16 codes: 0 against 3,270 (pose 0) |
| F6 flicker | **Pass.** Consecutive converged frames differ by 0.025–0.042 codes RMS (registered) and 0.033–0.060 (+4), ≤ 0.075 on covered pixels, against limits of 0.5 and 1 |
| F8 determinism | **Pass** on the final build (b10): 32 in-process static repeats of the 128-frame sequence, 31/31 identical to the first including the f32 history, in each of two processes; 54/54 checkpoint captures, all repeat and all cut digests identical across them. See [determinism](#determinism) for the failure found on the way |
| Q-cut | **Pass.** Every frame after a cut (to the near view and back) equals the cold start of its pose byte for byte |
| Static diagnostic (1.25x of §2) | Deviations diagnosed, not tuned: see below |
| Fixtures | **Not run** |

**Convergence** (M; ratio to the bar at N = 1/2/4/8/32/128, pose 0):

| Population | R_L | R_D +4 | R_F (N = 8/32/128) |
|---|---|---|---|
| partial | 1.00/0.76/0.43/0.33/0.07/0.03 | 1.01/0.80/0.48/0.35/0.07/0.03 | 0.37/0.09/0.04 |
| faint | 1.00/0.73/0.43/0.34/0.07/0.03 | 1.00/0.75/0.49/0.35/0.08/0.03 | 0.56/0.19/0.08 |
| contact | 1.00/0.77/0.45/0.34/0.07/0.03 | 1.01/0.84/0.56/0.41/0.08/0.04 | 0.36/0.09/0.04 |
| isolated | 1.00/0.64/0.33/0.26/0.06/0.02 | 1.00/0.58/0.31/0.23/0.05/0.02 | 0.25/0.09/0.04 |
| lit | 1.00/0.73/0.45/0.31/0.08/0.03 | 1.00/0.87/0.58/0.40/0.09/0.04 | (bar 0) |
| backlit | 1.00/0.75/0.39/0.30/0.06/0.02 | 1.00/0.69/0.39/0.28/0.06/0.03 | 0.26/0.07/0.03 |
| multi-surface | 1.00/0.75/0.46/0.32/0.08/0.03 | 1.01/0.80/0.49/0.34/0.08/0.04 | 0.38/0.11/0.05 |
| background-centre | 1.00/0.83/0.52/0.37/0.10/0.04 | 1.00/0.82/0.52/0.35/0.10/0.04 | — |
| image | 1.00/0.76/0.46/0.33/0.08/0.03 | 1.01/0.82/0.51/0.36/0.08/0.03 | — |

The near view (pose 2) is within 0.1 of these at every N; its worst at frame 8 is background-centre at 0.48 (R_L) and 0.46 (R_D +4). These match or beat the CPU replay's 0.28–0.41 at frame 8 except where noted next.

**Static diagnostic against the replay.** Ratios above 1.25x of [§2](m6-temporal-primaries.md#2-sample-pattern-and-static-convergence) occur in three places, each diagnosed:

1. N = 2 on backlit (R_L +4, 1.26–1.30) and isolated (R_F, 1.25–1.33). The replay's stored curve uses the corner-second order (8, 0, 12, 4), not the chosen quarter-second order (8, 12, 4, 0). For the chosen order the doc's own criterion is the second-sample ratio N = 2 / N = 1 ≤ 0.944; on the GPU it is at most 0.903, 0.906 and 0.943 at poses 0, 1 and 2.
2. Solid R_F populations at N = 2–16 (13–28x of replay values near 1e-4) and R_D on the occluded population at N = 8 (1.26–1.43x of 0.05 codes). These are the solid-region samples described below; the replay drew its samples from R1's random in-stratum positions and its 8-ray noise from centre receivers, so it could not see them.
3. Nothing else exceeds 1.25x: every R_L, R_D and partial-type R_F population at N = 1, 4, 8 and 16 is within it.

### Solids: the recorded miss

F1 requires the R_F maximum on film-solid pixels (R_F exactly 0 or 1) to stay at most 0.002. T1 measures 0.025 (pose 0), 0.125 (pose 1) and 0.25–0.29 (pose 2) at frames 48–128, on 106–155 lit and 18–64 umbra pixels per pose out of about 0.3–1.1 M.

Diagnosis (M):

- On single-surface outliers, exactly one of the 128 samples disagrees with all 8 of its rays (an error of 8/1024). That is not 8-ray noise at a solid receiver: that sample's receiver is in the other state.
- The disagreeing sample is most often k = 3, the corner lattice point (0, 0). Moving every non-centre sample by (0.37, 0.61)/256 px leaves the outliers in place (107 against 106 lit pixels at frame 128; the same pixels flip at the same index). So it is not a numerical degeneracy at the exact pixel boundary.
- A straight shadow edge near a pixel corner cuts off a tiny region containing that corner point. The reference's random samples rarely land in it. Any equal-weight 256-point accumulation then weights such a region at 1/256 = 0.0039 against its true area, so a fixed sequence exceeds 0.002 there by construction. A random sequence does on average too, as noise.

**Verified solids (M).** Following the [G2 resolution](m6-receiver-followup.md#verified-solid-classifications), every reference-solid pixel was checked with 16 further independent 1,024 × 32 visibility replicas (16,384 samples, 524,288 rays per pixel, 46–111 s per pose). Selection uses only the references, never T1's error.

| Pose | Reference solids lit / umbra | Reclassified partial | T1 frame 128 on reclassified, RMS vs replicas | Confirmed solids over 0.002, lit / umbra | Maximum |
|---|---|---:|---:|---|---:|
| 0 | 279,329 / 15,850 | 2,504 / 262 | 0.0013 / 0.0017 | 22 / 5 | 0.0078 |
| 1 | 278,836 / 15,810 | 2,597 / 275 | 0.0012 / 0.0016 | 27 / 9 | 0.0078 |
| 2 | 1,052,183 / 96,397 | 4,196 / 945 | 0.0011 / 0.0014 | 44 / 20 | 0.0132 |

- About 0.9% of the 1,024-sample reference's solids are partial at 16,384 samples. T1 tracks those within 0.0011–0.0017 RMS: the finite reference, not T1, was wrong there.
- On confirmed solids, every outlier at frame 128 is one sample of 128 with all 8 rays opposite (0.0078; 0.0132 is one of 76 accepted). 22 of 27, 26 of 36 and 52 of 64 first deviate at frame 4, which is k = 3, the lattice point at the exact pixel corner (0, 0).
- A straight shadow edge near a corner can cut off an arbitrarily small region containing the corner point, which 17,408 random samples miss. T1 weights it at 1/128 now and 1/256 once the window fills, so its static limit exceeds 0.002 there.
- An earlier diagnostic stored each sample's depth in the f16 target, which flipped the 2% centre-surface acceptance near the threshold and produced spurious 0.125–0.29 maxima on multi-surface pixels. The final runs carry depth at f32 precision; acceptance rates then agree with the replicas to 0.0002 on average.

**Candidate T1a′, measured, not adopted.** Under the frozen rules a sequence change makes a new candidate, so the production sequence stays the design's. The candidate keeps k = 0 at the exact centre and moves every other lattice point by (8.5, 8.5)/256 px, so no sample lies on a pixel edge and the former corner point sits 0.033 px inside. Every power-of-two prefix stays a shifted lattice. It was run with the same binary through the test's diagnostic sequence edit:

| | T1a (design lattice) | T1a′ (corner-free) |
|---|---|---|
| Confirmed solids over 0.002, poses 0/1/2 | 27 / 36 / 64 | 6 / 10 / 11 (umbra 0) |
| Maximum | 0.0078 / 0.0078 / 0.0132 | 0.0078 / 0.0169 / 0.0078 |
| Worst population at frame 8, R_L / R_D +4 (× bar) | 0.37–0.48 / 0.41–0.46 | 0.32–0.42 / 0.36–0.39 |
| Worst at frame 128, R_L / R_D +4 | 0.042–0.044 / 0.040–0.042 | 0.032–0.040 / 0.032–0.038 |
| Second-sample ratio N = 2 / N = 1, worst (doc bound 0.944) | 0.903 / 0.906 / 0.943 | 0.900 / 0.902 / **0.946** |

T1a′ converges faster and removes most solid outliers, but it still misses 0.002 and marginally misses the second-sample bound at the near view. Neither sequence meets the solid maximum. Any equal-weight 256-sample accumulation can put one sample in a region far smaller than 1/256 of the footprint.

## 5. Cost

**Paired screen (M, contended).** `gpu_film_t1_timing` alternates production Interactive and steady static Temporal frames on one renderer: 24 warmup and 48 measured pairs per process, three processes, validation off, pose 0. Other workers were compiling: 1–2 `rustc` processes and at least 6.5 GiB available during sampling, recorded per sample. Medians, ms:

| Process | Interactive span | Temporal span | Paired ratio | Paired delta | Trace, I / T | Shading, I / T | Temporal maximum |
|---|---:|---:|---:|---:|---|---|---:|
| 1 | 0.9110 | 1.0360 | 1.1365 | +0.1245 | 0.4938 / 0.5022 | 0.3718 / 0.4865 | 1.774 |
| 2 | 0.9090 | 1.0370 | 1.1404 | +0.1275 | 0.4944 / 0.5047 | 0.3699 / 0.4747 | 1.556 |
| 3 | 0.9111 | 1.0357 | 1.1365 | +0.1243 | 0.4951 / 0.5064 | 0.3714 / 0.4718 | 1.297 |

- **Trace:** jitter, ray rotation and the anchor read add 0.008–0.011 ms, inside the design's +0.000–0.010 estimate.
- **Shading:** +0.10–0.115 ms, above the design's +0.03–0.08. The store reads and writes 48 B of history for every image pixel, including the background clear blocks. The design assumed only the roughly 0.4 M pixels of covered tiles. Skipping history for pixels that can only see background is the first lever.
- **Totals.** Interactive here is 0.91 ms against its quiet 0.865 ms (receiver follow-up), so absolute spans are inflated. On the quiet base, T1a would be about 0.99 ms. That is under the 1.075 ms stop and above the 0.87–0.89 ms HRT and 0.80 ms VSM floors: a recorded likely miss, unconfirmed quietly.
- **Spikes.** Maxima up to 1.77 ms co-occur with the foreign compilers, so the "no frame above 1.5x steady" check needs a quiet run.
- **Reset frames** cost one Interactive frame plus one history-start pass; they were not timed separately.
- **Memory:** 48 B per pixel, 99.5 MB at 1080p, under the 128 MB stop.

Before this screen, two harness defects were fixed: temporal frames were flagged as having a separate visibility pass and as not timing their trace, which disabled frame timing (`renderer.rs` flight flags).

### Determinism

Build b8 failed one static repeat: repeat 14 matched every HDR checkpoint (frames 1–128) but not the final f32 history. Two changes followed, and each was verified:

1. **Engine arithmetic pinned.** The anchor's distance used the program-facing interpolated position with implicit multiply-adds; it now uses `film_view_distance` with explicit FMA.
2. **Reset frames are the production Interactive frame.** Adding that helper changed the temporal module's code generation, and T-0 then failed at pose 0 (b9). T-0 had held in b3 and b4 only because the driver compiled the program's shared code identically in two modules. Reset frames now run the production pipelines, so T-0 holds by construction.

With both, b10 passed 31/31 repeats including the history in two processes, and the cross-process comparison above. The program's own arithmetic in the temporal module is still subject to the driver's [contraction hazard](determinism.md). The history exposes it at f32 precision before the f16 image would, so the F8 check should keep comparing histories.

## 6. Not done, and why

- **T1b–T1d** (reprojection with identity/depth/visible-surface acceptance, Catmull-Rom with bilinear fallback and the motion window; moving lights and movers with the reach boost; timing triplets and full Level F on nine sequences) were not started. T1a's gate has a recorded miss that needs a decision, and stages are kept only after their gate passes. A T1b shader draft (reprojection, forward and reverse tests, cold-pixel centre retrace) is parked, uncommitted, at `E:/renderlab-work/t1-gpu/wip/`.
- **Fixtures** (thin, stress and sawtooth against analytic oracles) were not run for T1. The R1 fixture oracles score film sums from the reference generator; T1 needs a fixture harness for its own accumulated output.
- **Quiet timing** and the reset-frame spike check were not run; the screen above is contended.
- No sequence beyond static holds and cuts was rendered, so motion, blur, ghosts and lag are unmeasured.

## 7. Decisions needed

1. **The solid bound for footprint estimators.** The measured options are:
   - (a) keep 0.002 as a maximum, which no equal-weight 256-sample accumulation meets on these scans;
   - (b) bound verified solids by a count or RMS, with the measured 22–64 pixels per pose at 0.0078–0.0132;
   - (c) adopt T1a′ under a new candidate number, with 6–11 pixels per pose, faster convergence and a 0.946 second-sample ratio against 0.944.
2. **Whether T1b may start** on T1a or T1a′ as measured. A T1b draft exists (below), but nothing of it was built or run.

## Evidence, reproduction and time

Evidence root `E:/renderlab-work/t1-gpu/`:

- `cameras.json`, the registered poses including the near view;
- `builds/b1`–`b10`: each test binary with its SHA-256, HEAD, `source.diff` and status (b10 also archives the new files);
- `runs/`: every lease's log, preflight and resource samples; `gpu-windows.json`, the lease ledger;
- `scores-ref/` (references and bars), `scores-t1g/` (final T1a with verified solids), `scores-shift/` (T1a′), and the superseded `scores-t1a/`, `scores-t1c/` and `scores-verify/`: `score.json` with every population, channel, seed and checkpoint, plus inspection PNGs (film, Exact, Interactive and T1 at both exposures with ×8 error maps);
- `inspect/`: crop sheets and overviews that were inspected;
- `archive/`, `t1g/`, `t1h/`, `t1-shift/`: `temporal.json` digests, identity and repeat results of each run; `timing-c{1,2,3}/paired-timing.json` with per-sample resource context.

Raw references and checkpoint dumps were deleted after scoring, as the disk rules require; the summaries and inspected PNGs remain.

```powershell
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir E:/renderlab-work/t1-gpu/target
py -3 -B scripts/m6-temporal-gpu-run.py --label <name> --manifest E:/renderlab-work/t1-gpu/ref.json --runtime-memory-mbytes 3072 -- <test-binary> renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::film_accumulation_tests::gpu_film_t1_reference --ignored --exact --nocapture --test-threads=1
# gpu_film_t1_temporal with t1g.json and t1h.json (T1a′: t1-shift.json); gpu_film_t1_reference with verify.json (16 replicas, visibility only); gpu_film_t1_timing with timing-c{1,2,3}.json
$env:OPENBLAS_NUM_THREADS='2'; py -3 -B scripts/m6-temporal-gpu-score.py --ref E:/renderlab-work/t1-gpu/ref --t1 E:/renderlab-work/t1-gpu/t1g --verify E:/renderlab-work/t1-gpu/verify --out E:/renderlab-work/t1-gpu/scores-t1g
```

Manifests are kept beside the evidence. Runs use the 6 GiB preflight and abort below 3 GiB at runtime: the run's own quarry load consumed the strict guard's margin while other workers were compiling, and two strict-guard attempts aborted. Contention (other workers' compilers) is recorded per sample in each run's JSON.

**Time (M, UTC).** Reading, design, implementation and verification were interleaved from about 12:55 to 15:05; editing time was not instrumented separately.

- **Verification:** 10 incremental release test builds of 37–55 s each with two jobs; CPU tests and Clippy runs of 6–36 s.
- **GPU:** 34 leases of 771.7 s in total, the longest 212.2 s (the replica verification), all released, from 13:40:58 to 14:50:35. 26 child runs took 747.7 s; the rest is deferral handshakes. The ledger includes failed and memory-aborted attempts: four aborted below the runtime floor while other workers compiled, and none overlapped another GPU job.
- **CPU scoring:** six scorer runs of 81–240 s each, with two math threads.
- **Not run:** no window, viewer, desktop input, game capture or Unreal launch.
