# M6 T1 on the GPU: references, static film accumulation (T1a) and the motion candidate (T1b)

## Main integration: opt-in T1a only (2026-11-06)

`SoftShadows::Temporal` is an opt-in static-converging mode. **Still frames
converge beyond Exact by frame 2–4** against the registered T1 film references.
**In motion, every camera change resets the entire history, so every moving
frame equals Interactive 1x, which is below the film-quality bar.** The known
receiver concave-edge defect is being fixed separately; its three recorded
solid-pixel failures remain counted. This integration makes no motion-quality
or Unreal performance claim.

The source is `origin/t3/temporal-primary-rays` at `113b31e` (the stage 1
`ad80825` code after the T1b revert), merged onto main `9828f51`. Only T1a is
retained: the corner-free lattice, tile-limited history and full resets on
camera changes. T1b and T2 remain historical experiments. Exact stays the
default, and Interactive, Exact and hard shading retain their own pipelines.
Main's programmable shading, receiver policy, demand-driven/atomic fast
reload, program defaults/reflection and environment lookup deduplication are
preserved. No receiver-origin shader is edited.

The final integration batch uses one renderer and loads the quarry once,
compares SHA-256 manifests for the frozen 285 primary and 36 Interactive
artifacts, checks standard/toon reset identity and eight repeated sequences
including history, then captures the T1 checkpoints. It runs offscreen under
the shared GPU lease and `gpu-schedule.md`, with game preflight, 4 GiB available
memory required at launch and a 2 GiB runtime abort. Evidence is kept at
`E:/renderlab-work/integrate-t1a/`; the private two-job Cargo target is removed
at completion.

### Integration results

The merge applied cleanly with no textual conflicts. The overlapping program
and renderer changes preserve main's typed parameter defaults/reflection and
shared material helpers, add the single-pipeline Temporal variant beside the
paired traced variants, compile it on demand, and include active temporal
kernels/programs in atomic reload transactions. Temporal resets continue to
use the ordinary Interactive trace and the selected program's ordinary Soft
shading pipelines. Main's receiver policy and environment deduplication code
are unchanged.

One offscreen process, one quarry upload, active Vulkan validation,
RTX 4070 SUPER / NVIDIA 617.14, 1920×1080 1x:

| Check | Result |
|---|---|
| Main primary acceptance (Exact, hard, raster controls and seeded counts) | 285/285 SHA-256 identities match the immutable main manifests |
| Main Interactive acceptance (four seeds, three poses, hard/Exact controls) | 36/36 SHA-256 identities match |
| Standard/toon camera resets | Both pass four successive camera changes; reset HDR, hit records and counts equal Interactive byte for byte |
| Static repeats | Eight 128-frame sequences per program, checkpoint HDR and history/anchor/tile-state bytes identical; the original T1a harness also passes eight standard repeats |
| Reload | Both programs: rejected edits retain live temporal history/pipelines; accepted program and prelude reloads reset to the matching Interactive image |
| Tile-limited history | 128/128 images equal fully materialized history; zero differing retained-history pixels (355,520 pixels, 11,110/64,800 tiles) |
| Frozen T1 checkpoints | 54/54 HDR hashes match stage 1: full, sun-only and visibility programs, three poses, frames 1/2/3/4/8/128 |
| CPU / Clippy / format | 93 CPU tests pass; affected crate's release all-targets Clippy with warnings denied and workspace fmt check pass |

The old raw film reference buffers were deleted under the retention policy,
so a fresh CPU rescore is unavailable. The **byte-identical checkpoint images**
establish exact reuse of their retained radiance scores from
`E:/renderlab-work/t1-gpu/scores-t1k/score.json`. Worst radiance RMS divided by
its frozen Exact-seed bar across L0/L4/D0/D4 populations:

| Pose | Frame 8 | Frame 128 |
|---|---:|---:|
| 0 | 0.377841 | 0.037859 |
| 1 | 0.392435 | 0.046226 |
| 2 | 0.420322 | 0.037086 |

These are carried scores with new image-identity evidence, not regenerated
references. The separate receiver concave-edge solid failures remain open.
No motion-film, final film-quality, non-RT parity or comparative performance
gate is claimed.

Verified visually: new full-scene frame-1/8/128 sheets against the retained
film reference at registered exposure, and native-resolution silhouette,
shadow and surface crops at +4 stops:
`E:/renderlab-work/integrate-t1a/images/overview-L0.png` and
`images/crops-L4.png`. Images preserve the static result and convergence;
moving-frame identity is checked numerically against Interactive.

The GPU batch took **108.0 s**, with no guard abort, game or validation error.
Available memory was 6,958 MiB at preflight and at least 3,255 MiB during the
run. Builds from the private cold target took 141.6 s; CPU verification took
3.3 s and Clippy 27.3 s including guard overhead. Implementation was ready at
22:41 UTC; final build/check/GPU verification ran 22:41–22:47 UTC on
2026-11-05 (2026-11-06 local). These are verification wall times, not renderer
performance measurements. The GPU slot was released to receiver-origin
after the single batch. JSON manifests, logs, scores and inspected PNGs are
retained on E:; raw captures larger than 5 MiB and the private Cargo target
are removed after summarizing.

Reproduction (with the saved integration manifest and its immutable hash
manifest paths):

```powershell
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir E:/renderlab-work/integrate-t1a/target
py -3 -B scripts/m6-temporal-integration-run.py --fast-check --label gpu-batch --manifest E:/renderlab-work/integrate-t1a/batch.json --minutes 15 -- <test-binary> gpu_t1a_integration_batch --ignored --nocapture --test-threads=1
py -3 -B scripts/m6-temporal-integration-score.py --batch E:/renderlab-work/integrate-t1a/batch --baseline E:/renderlab-work/t1-gpu/t1k/temporal.json --scores E:/renderlab-work/t1-gpu/scores-t1k/score.json --out E:/renderlab-work/integrate-t1a/film-scores.json
```

Use a fresh lease label and output directory when repeating; the runner
refuses log overwrites.

2026-11-05, base main `43d9184` (viewed-side receiver policy, programmable shading phase 1, the determinism FMA fix and fast reload), branch `t3/temporal-primary-rays`. This implements and measures the first unit of [T1](m6-temporal-primaries.md#9-gpu-implementation-plan-and-stop-rules). RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1, release library test binaries built with two Cargo jobs, the registered full-resident quarry (12 draws, 13,034,903 triangles) at 1920×1080 1x, 16x anisotropy, a 0.04-rad disk. Every GPU run was offscreen under the shared lease; no window, viewer, desktop input, game capture or Unreal launch.

**Result (T1b, builds b15 and b16).** The camera-motion candidate **failed its gate on Q-walk and is not kept** ([§S2](#s2-t1b-camera-motion)). Commit `b8fa1f3` holds it; the next commit reverts it, so the opt-in temporal mode still restarts on any camera change, as T1a does. Under motion it beats Exact on lit, partial, occluded and all-covered radiance: 0.6–0.7× Exact's error, where section 4's model predicted about 0.5–0.6×. It fails wherever history crosses a silhouette or a shadow edge. Silhouettes leave visible ghost trails 5–8 px into the sky. Shadow edges blur, so the R_F solid maximum reaches 0.10–0.61 against 0.002. On the return path, from frame 108 onwards, R_D +4 has more pixels over 16 codes than Exact on every scored frame, a frozen stop. Cost screened at +0.205 ms per frame over Interactive, above the 1.075 ms stop.

**Result (second round, build b12).** Root's decisions are applied: the corner-free lattice is the production sequence, and history is kept only in trace tiles a ray has hit, byte-identical to keeping it everywhere. T1a now passes every check except the F1 solid maximum at **three pixels**, one per pose. There a single sample of 128 lies within about 10⁻⁶ px of a concave triangle edge, where the **production receiver origin** is blocked by the adjacent triangle ([§0.3](#03-solids-verified-locally-and-the-remaining-cause)). The fixtures (thin, stress, all 12 acne cases) pass. Cost is +0.025 ms per frame over Interactive on a contended screen. Root then recorded the band as a known production receiver defect and started stage 2 (T1b). Defaults are unchanged.

The first round's text from §1 onwards is kept as measured; where §0 differs, §0 supersedes it.

## 0. Second round: decisions applied

Root's decisions (2026-11-05):

- **Solid bound.** The 0.002 solid maximum stays. A sample on the pixel corner is a sampling defect: the box filter gives the corner zero measure, but that lattice point carried 1/128 of the weight.
- **Sequence.** The corner-free lattice is adopted as that defect's fix.
- **Second-sample rule.** Frame 2 must be no worse than frame 1, so the N = 2 / N = 1 ratio must be ≤ 1.0. The **0.944 quoted in the design was the earlier ordering's measured value, not a limit**. T1a′'s worst ratio, 0.946, passes.
- **Remaining solid failures.** Each is verified with a high-sample local footprint estimate, as G2 was.
- **Cost.** History is restricted to tiles rays have hit, with byte-identical output.

### 0.1 What changed

- **Sequence (production).** k = 0 is the exact pixel centre. Every other index is the rank-1 lattice node moved by (8.5, 8.5)/256 px, so no sample lies on a pixel edge; the former corner sample sits 0.033 px inside. Every power-of-two prefix is the moved lattice with its centre point replaced by the exact centre. CPU tests check the order, the prefixes, the symmetries and that no coordinate is 0 or 1.
- **History only where rays have hit.** A per-tile flag, set once a camera ray in the 8×4 trace tile hits after a reset and ping-ponged with the history, decides where history is stored. Every pixel of an unflagged tile has seen only background, so its history is implied: mean = the background sample, weight min(frames, 256), index frames − 1. Background samples therefore contribute the clear colour as the f16 image stores it (`film_background`), the value Interactive writes and the reference integrates. Unflagged tiles and the clear blocks outside the scene rectangle store the background directly, as the Interactive and hard paths do.
- **Byte identity (M, in process, same pipelines).** With a test switch that keeps history in every tile, the 128-frame pose-0 sequence gives identical HDR images in all 128 frames, and identical history at all 355,520 pixels of the 11,110 flagged tiles (of 64,800).
- **Reset frames** still run the production Interactive pipelines; `film_init` now writes history and flags for covered tiles only.

### 0.2 Gate on the production sequence (build b12)

| Check | Result |
|---|---|
| T-0 | Pass, 9/9 |
| F8 | Pass: 31/31 static repeats including the history in each of two processes; 54/54 checkpoint captures, repeat and cut digests identical across them |
| 8-frame limit, F1-AA, F2, F4, F6, Q-cut | Pass at all three poses. Worst population at frame 8: 0.38 / 0.39 / 0.42 × bar (radiance); at frame 128 every channel ≤ 0.12 × bar |
| Second-sample rule (≤ 1.0) | Pass: worst N = 2 / N = 1 ratio 0.900 / 0.902 / 0.946 |
| F1 solid maximum on replica-confirmed solids | 6 / 10 / 11 lit pixels over 0.002 (umbra 0 / 0 / 0), maximum 0.0078 / 0.0169 / 0.0078; resolved below |
| Fixtures | Pass ([§0.4](#04-fixtures)) |

The regenerated references reproduce the first round's: replica-confirmed solids are again 276,825 / 276,239 / 1,047,987 lit pixels.

### 0.3 Solids, verified locally, and the remaining cause

`gpu_film_t1_footprint` re-estimates each of the 27 pixels over a 1024 × 1024 stratified grid covering the whole footprint, corners included, with one jittered position per cell. Each position gets 32 independently salted production shadow rays: 33.5 M rays per pixel. Acceptance is the reference's (centre placement and depth within 2%).

- **24 of 27 are partial footprints** and are reclassified. Each has 1–166 opposite samples, local visibility 0.99985–0.999999. The nearest opposite sample to a corner is 0.04–0.48 px away; only one lies within 0.05 px of a corner. Per-pixel counts are in `footprint/footprint.json`.
- **3 are truly solid**: pose 0 (1347, 380), pose 1 (1119, 713) and pose 2 (1325, 276), with zero opposite samples among 1,048,576. T1 errs by 0.0078 at each.

Cause (M). A temporal run that recorded these pixels every frame (`t1-watch`) shows one sample each, at frames 121, 115 and 45, with all 8 rays blocked; every other frame is fully lit. Each of those samples hit its triangle on an edge: barycentrics (4.2×10⁻⁶, 0.4989), (0.93793, 2.0×10⁻⁵) and (0.61097, 0.38903), where u + v = 1 − 10⁻⁵. Micro-grids of 1,048,576 samples around each exact position give:

| Window around the sample | Opposite samples, poses 0 / 1 / 2 | Visibility |
|---|---|---|
| ±0.5 px (clipped to the pixel) | 0 / 2 / 0 | 1.0 / 0.999997 / 1.0 |
| ±0.005 px | 73 / 474 / 76 | 0.99993 / 0.99955 / 0.99993 |
| ±0.00005 px | 624,988 / 625,605 / 312,801 | 0.40 / 0.40 / 0.70 |

So a band roughly 10⁻⁶ px wide (nanometres at the quarry) along each edge is fully shadowed, while the surface on both sides is lit. That matches the receiver policy at a concave edge. The origin is offset 10 µm along the hit triangle's own geometric normal; for hits within about that distance of a concave fold, the offset point lies behind the adjacent triangle, which blocks every ray.

This is the production receiver policy, shared by Exact, Interactive and the references; each hits the band with probability proportional to its area. It is not T1's accumulation. T1 exposes it because each pixel samples 256 fixed positions and weights each at 1/256 indefinitely. The band's true area is far below 0.002, so these pixels' film value is solid lit. Fixing it means changing the receiver origin near edges, for example pulling the hit point into its triangle before offsetting, which changes Exact and Interactive. That needs its own unit and acceptance, and is not made here.

### 0.4 Fixtures

`gpu_film_t1_fixtures` runs 256 temporal frames on each R1 fixture. It writes T1's own samples' centre-surface visibility in R1's film layout, split into halves by frame parity. `scripts/m6-temporal-fixtures.py` evaluates R1's continuous analytic oracles at exactly T1's sample positions: the frame's lattice index and the pixel's symmetry.

| Fixture | Result |
|---|---|
| Acne: flat and convex, 1 km and 10 km, 80°, 85° and 89° (12 cases) | Pass: 0 false-shadow pixels of 35,328 covered in every case |
| Thin | Pass: all-pixel RMS 0.00108 against noise 0.00656; partial (933 px) 0.00781 against 0.0476; solid maximum error 0; 2 accepted-count mismatches |
| Stress | Pass: all-pixel RMS 0.00094 against noise 0.00327; partial (2,297 px) 0.00433 against 0.0151; solid maximum error 0; 0 mismatches |

### 0.5 Cost after the tile restriction (M, contended screen)

Three processes of 48 paired frames, with the method of §5. Two had no foreign compiler and the third one; at least 8.1 GiB stayed available.

| Process | Interactive span | Temporal span | Paired ratio | Paired delta | Trace, I / T | Shading, I / T |
|---|---:|---:|---:|---:|---|---|
| 1 | 0.9066 | 0.9330 | 1.0290 | +0.0262 | 0.4932 / 0.4804 | 0.3695 / 0.4071 |
| 2 | 0.9074 | 0.9331 | 1.0271 | +0.0246 | 0.4925 / 0.4800 | 0.3697 / 0.4076 |
| 3 (slow GPU state) | 1.2129 | 1.2546 | 1.0318 | +0.0388 | 0.6645 / 0.6520 | 0.4703 / 0.5275 |

- The delta falls from +0.125 to +0.025 ms; history traffic now covers about 17% of tiles.
- On the quiet 0.865 ms Interactive base, T1a would be near 0.89 ms, at the top of the 0.87–0.89 ms HRT floor.
- Temporal maxima reached 1.9–2.2 ms in these processes. The quiet confirmation and the spike check are deferred, as root asked.

### 0.6 Next

- **Decided (root, 2026-11-05):** the concave-edge band is a **known production receiver defect**. A separate worker fixes it at the root in the receiver-origin code (the primary trace and shadow shaders), which this unit does not touch. Stage 1's three failing pixels are rescored once that fix lands; every other stage 1 check passed. T1b started under this decision ([§S2](#s2-t1b-camera-motion)).

## S2. T1b: camera motion

Root's decision (2026-11-05): start T1b with the concave-edge band recorded as a known production receiver defect ([§0.6](#06-next)), keeping T1b only if its gate passes. It did not pass.

### S2.1 What the candidate implements (commit `b8fa1f3`)

It follows the design's [section 3](m6-temporal-primaries.md#3-reprojection-at-the-registered-1-cm-step) and [section 4](m6-temporal-primaries.md#4-history-weighting-under-motion).

**Reprojection and validation.**

- **Fetch position.** History is fetched where the current sample's surface point was in the previous frame. Background moves by direction.
- **Tap validation.** A tap is valid by the forward test or the reverse test:
  - forward: the same instance, the same side for thin materials, and view distance within 4 footprints;
  - reverse: the tap's anchor point, rebuilt from its own sample ray, projects within 1.5 px, and the current trace there hit the same instance at a distance within 2%.
- **Fetch.** Catmull-Rom over 4×4 taps when all 16 are valid, clamped at zero. Otherwise bilinear over the valid inner taps, renormalised, with the history weight set to the bilinear-weighted tap count.
- **Window.** n' = min(n + 1, W), with W = 1 + 1/Σf(1 − f), the blur budget at v_max = 1 px².

**Restarts and frame state.**

- **Cold pixels.** In the trace, a sample whose fetch finds no inner tap by the forward test is traced again at the pixel centre with index 0. A revealed pixel's first sample is therefore Interactive's.
- **Reset key.** The key leaves out the camera, the lighting uniform's eye and the camera-fitted shadow views. Any other change still restarts every pixel. `cut_film()` is an explicit cut.
- **Never-hit tiles.** Their implied history is weighted by the frames rendered at the previous frame's camera, so static behaviour is T1a's exactly.
- **Determinism.** Explicit multiply-adds are used for every history decision.

**Resources.**

- **Sample record.** The trace writes each covered pixel's sample index and view distance into a 4-byte-per-pixel storage buffer, for the reverse test. A texture would exceed the traced device contract: it allows four storage textures per stage, and the trace already binds four. The shading then binds 8 of the 8 storage buffers it allows.
- **Memory.** 52 bytes per pixel, 108 MB at 1080p.

**Not built.** The per-placement generation table: any non-camera change still restarts globally. Moving lights and objects (T1c).

### S2.2 Q-walk gate

**Method.**

- **Sequence.** The registered 161-frame Q-walk: 64 steps of 1 cm with 0.05° of yaw per frame, a 32-frame hold, then the reverse path. It was rendered from cold with the full, sun-only and visibility programs.
- **Scoring.** 60 frames are scored against Level F references at the 26 distinct cameras. Holds and the return share the walk's references, because their cameras are identical.
- **Bars.** Exact seed 0 on the same frame and pixels, times 1 + the static 2-s.d. seed spread (at most 0.0014).
- **Ages.** A pixel's age counts frames since its history last restarted: frame 0, or no valid tap. F1 uses pixels aged 32 or more.
- **R_F.** Scored on single-surface pixels, where T1's visibility film has the reference's estimand.
- **Script.** `scripts/m6-temporal-motion-score.py` in `b8fa1f3`.

| Check | Result |
|---|---|
| T-0 | Pass, 4/4: frame 0 is byte-identical to Interactive for each program and the replay |
| F8 | Pass. An in-process replay is identical at all 60 scored frames and in the final history. The b16 refactor (below) is identical to b15 at all 60 frames of all three programs |
| F1 converged | **Fail.** See the next table |
| F2 transient | **Fail**, 509 pools: 446 above Interactive, the rest above the bar at age 7. Revealed pixels start equal to Interactive at age 0 (16.277 against 16.270 codes on all-covered L0). Then they worsen: 18.0 against 11.4 at age 3, and 13.4 against 8.3 at age 7. On background-centre pixels at age 7 it is 39.4 against 11.3. From the start event, all-covered beats Interactive (4.14 against 5.87 at age 1), but background-centre does not (1.23 against 1.04 at age 7) |
| F3 bias | **Fail**, 996 cases, mostly R_D and R_F on edges and occluded pixels |
| F4 | **Fail, frozen stop.** On the walk, R_D +4 has fewer pixels over 16 codes than Exact (2,685 against 3,233 at frame 48). On every return frame from 108 to 160 it has more (3,934 against 3,356 at frame 128). The R_F missed-plus-false count stays below Exact's. Frame 0 is Interactive and is excluded |
| F5 | The changed-pixel RMS 8 frames after motion stops passes: 2.75 against 7.97 codes on L0. Ghost bias on changed pixels fails on 30 frame/channel cases, mostly D0 and F. The exact zero-weight-from-hidden-surface output was not built |
| F6 | Pass. The motion residual is 3.1–3.8 codes against Exact's 7.0–7.2 at the registered exposure. Hold steps are 0.28 and 0.33 codes |
| F7 | Not run |
| Section 4 diagnostic | **Fail.** At the measured median offsets (0.46, 0.89) px, with W = 4.6, measured error exceeds the model on every population. Lit, partial and all-covered are 1.2–1.6× the model, above the 1.25× stop. Background-centre R_D is 3.2–3.6× and multi-surface L4 is 2.9×; the model has no parallax at silhouettes and no accumulated misregistration |
| F9 cost | Screening above the stop ([§S2.4](#s24-cost-contended-screen)) |

F1 on pixels aged 32 or more: T1 RMS as a multiple of the bar.

| Population | Walk, frames 48 and 64 | Hold, frame 96 | Return, frame 120 |
|---|---|---|---|
| all-covered, L0 / L4 / D4 | 0.65–0.69 / 0.60–0.64 / 0.74–0.79 | 0.12 / 0.13 / 0.14 | 0.62 / 0.58 / 0.71 |
| partial, L0 / D4 | 0.61–0.65 / 0.64–0.68 | 0.13 / 0.13 | 0.60 / 0.62 |
| occluded, L0 / D4 | 0.71–0.72 / **1.57–1.69** | 0.13 / 0.25 | 0.65 / **1.45** |
| multi-surface, L0 / L4 | 0.97–**1.14** / 0.93–**1.18** | 0.14 / 0.12 | 0.89 / 0.89 |
| edge, L0 / L4 | **1.06–1.08** / **1.01–1.04** | 0.20 / 0.24 | **1.10** / **1.03** |
| background-centre, L0 / L4 / D4 | **1.37–1.42** / **1.46–1.49** / **1.44–1.46** | 0.39 / 0.51 / 0.33 | **2.21** / **2.13** / **1.81** |
| R_F all-covered / partial | 0.59–0.63 / 0.56–0.59 | 0.13 / 0.13 | 0.59 / 0.55 |
| R_F solid maximum (bound 0.002) | **0.40–0.61** | **0.10–0.28** | **0.31–0.59** |

Bold marks a failure. Once the camera stops, the hold converges far below the RMS bars. The solid maximum still fails, because the motion's blur decays only as 1/n.

### S2.3 Where the error is

Frame 64 (end of the walk), pixels aged 32 or more, by distance to the nearest centre-placement boundary:

| Population, channel | 0–1.5 px | 1.5–3 px | 3–6 px | ≥ 6 px |
|---|---|---|---|---|
| background-centre L0, T1 / Exact | 20.9 / 18.2 | **9.80 / 0.00** | **7.09 / 0.00** | **0.46 / 0.00** (1.7 M px) |
| all-covered L0, T1 / Exact | 9.40 / 8.96 | 3.85 / 6.40 | 3.90 / 6.73 | 3.42 / 5.53 |
| occluded D4, T1 / Exact | 4.94 / 4.41 | 2.22 / 1.57 | 2.12 / 0.86 | **2.52 / 0.32** |
| multi-surface L4, T1 / Exact | 28.1 / 22.2 | 12.1 / 23.9 | 9.94 / 30.5 | 9.18 / 19.1 |

Three mechanisms, confirmed in the crops `scores-walk/silhouette-crops-f064.png` and `overview-f064-f128.png`:

1. **Silhouette ghost trails.**
   - Mechanism: an edge pixel's history mixes foreground and background, while its anchor records only the latest sample. A background pixel accepts that tap by the forward test (both background) and carries the foreground share with the background's motion. The foreground itself moves with parallax.
   - Extent: the trail decays at 1/W per frame, so it reaches about W × the motion, 5–8 px. It is visible as a halo of cliff colour in the sky on the trailing side. It switches sides on the return.
   - Tile edges: never-hit 8×4 sky tiles stay exactly background, so on the return the halo shows tile-shaped steps.
   - The design's single-step test (section 3) measured one step, so it could not see this accumulation.
2. **Catmull-Rom undershoot.** Dark specks line the bright silhouettes. The negative lobes clamp at zero against the sky.
3. **Shadow-edge blur.** Within the window, repeated resampling spreads visibility by about the budget's 1 px² across shadow boundaries.
   - It is not a silhouette effect: occluded pixels 6 px or more from any silhouette show D4 2.52 against Exact's 0.32.
   - Solid pixels within a pixel of a hard shadow edge cannot meet the 0.002 R_F maximum under any resampled history. Exact meets it because it does not resample.

### S2.4 Cost (contended screen)

Paired frames along Q-walk (8 warm-up and 56 measured pairs per process; moving frames). Another worker's Cargo build was running, but there was no other GPU process and at least 9.5 GiB was available. Builds b14–b16 were unoptimised (debug) test binaries. GPU timestamps are unaffected: Interactive matched the stage 1 contended 0.906 ms.

| Build, process | Interactive span | Temporal span | Ratio | Delta | Shading, I / T | Trace, T |
|---|---:|---:|---:|---:|---|---:|
| b15, 1 | 0.905 | 1.108 | 1.225 | +0.203 | 0.369 / 0.581 | 0.481 |
| b15, 2 | 0.904 | 1.108 | 1.225 | +0.205 | 0.368 / 0.581 | 0.481 |
| b15, 3 | 0.903 | 1.110 | 1.230 | +0.207 | 0.367 / 0.583 | 0.482 |
| b16, 3 (1 and 2 ran in a slow GPU state) | 0.906 | 1.128 | 1.247 | +0.223 | 0.365 / 0.598 | 0.485 |

- **Where the cost is.** All of it is in the shading's fetch and validation. The cold-pixel test and sample record leave the trace unchanged.
- **The b16 refactor did not help.** It fetches history before the program runs, makes one flag, anchor and colour load per tap, and reuses the inner taps. It was byte-identical but no faster.
- **Spikes.** Temporal maxima reached 1.3–11 ms; spikes were not investigated.
- **Against the stop.** On the quiet 0.865 ms Interactive base, T1b would be about 1.07 ms, against the 1.075 ms stop.

### S2.5 Next (root decision)

The frozen stop rules name the next candidates: a ±1 px reverse search, then two-layer silhouette history. On this evidence:

- **Trails.** A ±1 px reverse search adds history for the current surface, but it does not stop background pixels accepting mixed edge history, which is the trails' source. Two-layer silhouette history addresses that directly. So would a cheaper restriction to test first: an anchor bit marking history that mixes surfaces, which only that history's own surface may accept. Edge pixels would then restart, like Interactive, instead of trailing.
- **Shadow-edge blur.** This is the v_max = 1 px² budget at work. Within a pixel of a hard shadow edge, no resampled history keeps solid visibility within 0.002 under continuous motion. A decision is needed between two options:
  - a tighter budget near visibility edges, which costs noise there;
  - a motion-specific solid criterion.
- **Cost.** About 0.2 ms of fetch and validation must be recovered as well.
- **Not run:**
  - Q-zoom;
  - Q-disocclusion, registered as a 2 cm/frame pure translation from pose 0 (`sequences/registered.json`, SHA-256 `cbb40f2b…`);
  - F7;
  - quiet timing.

### S2.6 Evidence, reproduction and time

Evidence root `E:/renderlab-work/t1-gpu/`:

- `builds/b14`–`b16`: each binary with its SHA-256, HEAD and `source.diff`. b14 exceeded the storage-texture limit at bind group creation.
- `walk-b15/motion.json` and `walk-b16/motion.json`: per-frame motion records, T-0 and the replay digests.
- `walk-ref/ref-p*/compact.json`: reference summaries and the section 4 model at the measured offsets.
- `scores-walk/`: `score.json` with every frame, population and pool, `silhouette-split.json`, the inspected crop sheet and overview, and the per-frame images.
- `timing-walk-{1,2,3}/` and `timing-walk-b16-{1,2,3}/`: `paired-timing.json` for each process.
- `sequences/registered.json`.

Captures, ages and compact references were deleted after scoring.

```powershell
git checkout b8fa1f3
cargo test -p renderlab --lib --no-run -j 2 --target-dir E:/renderlab-work/t1-gpu/target
# gpu_film_t1_motion with walk-b15.json; gpu_film_t1_reference with walk-ref.json; gpu_film_t1_timing with timing-walk-{1,2,3}.json
$env:OPENBLAS_NUM_THREADS='2'; py -3 -B scripts/m6-temporal-motion-score.py --ref E:/renderlab-work/t1-gpu/walk-ref --t1 E:/renderlab-work/t1-gpu/walk-b15 --out E:/renderlab-work/t1-gpu/scores-walk
```

**Time (M, UTC).**

- **Implementation:** shaders, the Rust frame state, tests and the scorer, about 15:40–16:01. The scorer and the b16 refactor were also written while GPU runs and scoring proceeded, until about 16:17.
- **Verification:**
  - three debug test builds of 25 s–2 min with two jobs, plus CPU tests and Clippy;
  - GPU: 10 leases of 596.4 s, all released, from 16:01:22 to 16:20:04. The longest was the 26 references at 272.1 s;
  - CPU: reference compaction of about 150 s, scoring of 349 s and the silhouette split of about 60 s, with two math threads.
- **Not run:** no window, viewer, desktop input, game capture or Unreal launch.

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
- **Replay identity includes light transport.** Independent mode replays must start with the same transported light basis. The disk sampler uses shadow-view axes, so equal sun vectors alone do not establish equal inputs. The [S3 follow-up](temporal-reset-identity.md) records the failing GPU ray witness and the corrected 258-reset audit.
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
