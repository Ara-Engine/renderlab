# M6 Level-B soft shadows

The gate and Interactive API were subsequently approved. The [implementation and verification](m6-level-b-implementation.md) passes the traced-receiver quality checks, exact mask replay and inspection. The retained 1-lane layout misses the 0.30 ms visibility and 1.10 ms frame targets; both 2x Unreal floors remain unmet. Exact remains the default.

Design decision, 2026-11-04, based on `0fd49d7`. CPU evidence only: no renderer change, GPU, Unreal or viewer run, and no build.

Inputs:

- `R = D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry/`: the immutable 16,384-ray quarry reference, production counts for seeds 0–80 and per-receiver 256-bit production hit masks for seeds 0–3.
- `U = D:/Coding/renderlab-work/ue-m5/Quarry/`: decoded native masks from the [native-mask run](unreal-native-shadow-mask.md) and the earlier SceneCapture arrays.
- The thin/contact and speck/wire GPU fixtures in `m6-adaptive/gpu-phase-6/` (5 poses × 16 seeds, analytic oracle).

Labels: **M** is measured from those records or from this unit's CPU replay; **I** is an inference.

## Decision

1. **Gate.** Level B scores visibility on light-facing receivers against the unchanged reference with two kinds of checks:
   - **U-checks.** For every population statistic and pose, a candidate is no worse than the better of Unreal's two native configurations, VSM and HRT 1-spp denoised. This is the honest "equal" bar.
   - **S-checks.** These are deliberately stricter than Unreal, with noise allowances set from production. They require zero solid flips, zero lost contacts, no systematic bias in any partial population (faint included), no coherent structure along shadow contours and deterministic static frames.

   On this quarry, Unreal's native shadows are a low bar. The U-checks alone admit a 5-ray estimator, and they admit omitting every faint shadow. The S-checks are where Level B is plainly **better than Unreal**.
2. **Method.** Trace 8 rays per receiver: the first 8 directions of the unchanged exact estimator's per-pixel sequence. There is no filter and no history.
   - Results: it passes every check on 3 poses × 4 seeds of the quarry and on 160 fixture cases. Partial RMS is .082, against .0065 for production and .370 for VSM.
   - Every edge-aware, penumbra-bounded filter tried made the result worse, even one given the reference's own penumbra width. On these scans visibility varies at pixel scale inside penumbrae.
3. **Cost.** Target 8-ray visibility at ≤0.30 ms, which gives a soft frame of about 1.08 ms against the 0.96 ms hard frame. The estimate is 0.24–0.40 ms (I).
   - At 1.08 ms, the frame is **2.0x HRT** (2.15 ms) and **1.48x VSM** (1.60 ms).
   - 2x VSM cannot come from shadows: its 0.80 ms floor is below the current *hard* frame.
4. **Unit.** Add one shader constant (256 or 8 directions) and one typed API switch. Order: cost probe first, then GPU/CPU replay exactness, this gate on traced receivers, inspection and paired timing. Stop rules are in [section 3](#3-implementation-unit).

## 1. Level-B gate

### What Unreal's error against the reference contains

Pose 0, recomputed from the native masks with the unchanged reference and gate metrics (SHA `0ffacfc6…7322`). These reproduce the published partial RMS exactly: VSM .3695, HRT 1-spp .3906, HRT 256-spp .1984.

- **Receivers with no direct light (M).** 11,779 of 26,399 reference umbra receivers face away from the light: the camera-facing geometric normal has N·L ≤ 0. These receivers hold 9,696 of VSM's 11,868 umbra-to-lit flips (82%) and 8,272 of HRT's 8,891. A shadow factor on a receiver with zero direct light is invisible, so these are policy differences, not defects. Unreal culls back-facing pixels; Renderlab flips the origin into the rock.
  - 3,404 of the 11,807 back-facing receivers face the light under Unreal's smooth shading normal. Those differences are a terminator policy, reported rather than gated.
  - A same-estimand ray tracer is exact on all but one of them, the single back-facing partial.
- **Visible receiver-policy defects (M).**
  - VSM loses 3,109 of the 9,908 light-facing contact-band receivers (31%) to its normal bias and screen-ray start; HRT loses 1,458 (15%).
  - HRT adds 717 lit-to-dark flips. Only 28 are near a depth edge and 28 lie within 3 px of a reference partial, so they are mostly self-shadowing acne. VSM has 19.
- **Sampling (M, I).** HRT 1-spp renders only 1,344 partial receivers, against 8,765 in the reference, so its edges are effectively hard. Against its own converged 256-spp output, √(.3906² − .1984²) ≈ .34 of its partial RMS is sampling error. VSM renders 5,019 partials, 72% of them on 1/8 steps.
- **Faint shadows (M).** Faint means reference V ≥ 0.9. VSM shows 1,406 of 1,494 faint receivers as fully lit, and HRT shows 1,380; production shows 249. VSM's faint RMS of .078 is *worse* than omitting every faint shadow (.050), because 51 faint receivers are shown more than 0.1 too dark. Without them, VSM's faint RMS is .0496, the same as omission.
- **Stability without AA (M).** At a static pose in the matched no-AA capture, consecutive SceneCapture frames (ticks 31 and 63) differ by partial RMS .162 for VSM: 3,272 partials and 2,033 solid receivers change by more than 0.1. HRT differs by .477. Production repeats bit for bit. Between the independently warmed native poses 0 and 1, the residual Δ(error) on partials is VSM .263, HRT .395 and production .0082.

So Unreal's measured error is an upper bound on its real quality loss on most populations, and a *lower* bound on faint loss (dropped shadows score well on RMS). The gate handles each case separately.

### Domain, reference and populations

- **Reference.** V is the mean of production seeds 1–64 (16,384 rays) on the same receivers. On traced receivers, use the reference regenerated by the [primary-ray acceptance](m6-primary-ray-production.md).
- **Domain.** Covered receivers whose camera-facing geometric normal has N·L > 0; that is 334,710 of 346,520 at pose 0. The GPU unit restates this as "the candidate's own unshadowed direct radiance is positive" and reports excluded receivers separately.
- **Existing populations, unchanged.** These are defined from the reference only: partial, narrow (lit-plus-umbra distance ≤3 px), faint, isolated, silhouette, lit and umbra.
- **New populations:**
  - **wide**: a partial with lit-plus-umbra distance >3 px.
  - **contact**: a dark receiver (V ≤ 0.5) within 3 px of a fully lit receiver, where the 5×5 depth range is ≤5%. A transition of 3 px or less means a blocker within about 0.3 m at this view. These are the receivers VSM's offsets erase. Pose 0 has 9,908.

### Checks

Every check holds on every pose (original, +1 cm, return) and every replayable seed (0–3). S1–S4 also hold on both GPU fixtures (5 poses × 16 seeds), where no Unreal data exist. Values are pose 0 / pose 1.

| ID | Check | Bound | Source | vs Unreal |
|---|---|---|---|---|
| U1 | Partial RMS | .3695 / .3688 | VSM | equal |
| U2 | Narrow RMS | .4031 / .4028 | HRT | equal |
| U3 | Wide RMS | .3237 / .3239 | VSM | equal |
| U4 | Isolated RMS | .4472 / .4506 | HRT | equal |
| U5 | Silhouette RMS | .3545 / .3324 | VSM | equal |
| U6 | Faint RMS | .0779 / .0887 | VSM | equal; **the binding Unreal bound** |
| U7 | Lit RMS | .0066 / .0074 | VSM | equal |
| U8 | Umbra RMS | .1995 / .1956 | HRT | equal |
| U9 | Contact RMS | .3323 / .3304 | HRT | equal |
| U10 | Motion: RMS of Δ(error) on partials over 0→1 and 1→return | .2626 | VSM | equal |
| S1 | Solid flips (lit with error < −0.5, umbra with error > 0.5) | 0 | Production 0; VSM 19 + 2,172; HRT 717 + 619 | **better** |
| S2 | Lost contacts (contact error > 0.5) | 0 | Production 0; VSM 3,109; HRT 1,458 | **better** |
| S3 | \|mean error\| for partial, narrow, wide, faint and isolated | ≤ \|production same-seed mean\| + 4 SE | Production. VSM faint +.026 is 13 SE; VSM wide +.207 | **better** |
| S4 | Contour structure ρ = RMS(9-px along-contour mean) / (partial RMS / 3) | √(0.9 ρ₀² + 0.9) = 1.51 / 1.50 | ρ₀ = production median over seeds 65–80 (1.239; worst 1.270). VSM 1.89 | **better** |
| S5 | Static repeat and return frame | bit-identical | Production; Unreal no-AA static frames change (above) | **better** |
| G7-B | Full-image inspection | no candidate-only defect class | below | |

Notes on the checks:

- **U-bounds** take the minimum, per statistic and per pose, over the two shipping native configurations, so a candidate is never worse than either. Diagnostic configurations are excluded: VSM with zero normal bias or zero screen ray, and HRT at 256 spp, which costs 26 ms. Neither native configuration passes the composite, so it is stricter than "as good as one of them".
- **S3** uses SE = candidate RMS/√n. Four SE across about 60 tests per candidate gives roughly a 0.4% family false-fail rate under independence. This check encodes the recorded faint-shadow and USS failures:
  - Production with every faint receiver forced lit passes U6 (.050 against .078).
  - It fails S3 faint (.040 against .005), as do the filters.
- **S4** limits the coherent share of along-contour error to 10% of the error variance. Production's worst seed reaches 1–6%, depending on the image, and VSM 27%. The [Vogel banding case](m6-reduced-ray-gate.md#level-a-gate) gives ρ ≈ 3. S4 detects bands, blotches and filter bias independently of the noise level, which U1–U9 govern.
- **Per-receiver envelopes (Level A's G1) are not used.** Level B admits noise.

**G7-B inspection** covers:

- reference, production, candidate, VSM and HRT visibility at native resolution;
- ×8 signed-error maps on one scale;
- lit images at the registered and +4-stop exposures;
- 33×33 crops at the seven named contact and self-shadow pixels, plus silhouettes, rubble, crevices and cliff-top isolated partials;
- fixture strip, sliver, specks and wires;
- a 0→1→return flip-book.

A defect class present in the candidate's error and absent from *both* production's and the better Unreal configuration's error fails. Penumbra grain is the expected class; U1–U10 quantify it. Numbers do not override inspection.

Thresholds and rules are frozen here. On traced receivers, the same rules are recomputed against the traced reference, with the Unreal masks rescored against it. A failed check stops the candidate; changing a configuration makes a new candidate.

### Calibration (CPU replay of saved GPU rays)

Maxima over 3 poses × 4 seeds; ratios are to the per-pose bound. A k-ray candidate takes the first k bits of each receiver's dumped production hit mask. That is exactly what the unchanged shader traces with k in place of 256.

| Candidate | Partial RMS | Faint RMS (× bound) | Contact RMS | Motion RMS | ρ | Flips / lost contacts | Max \|faint mean\| | Result |
|---|---:|---:|---:|---:|---:|---|---:|---|
| Production, 256 | .0066 | .0042 (0.05) | .0035 | .0082 | 1.27 | 0 / 0 | .0001 | pass |
| 64-detect adaptive (Level A reject) | .0066 | .0042 | .0035 | .0082 | 1.27 | 0 / 0 | .0002 | pass; rejected on cost (about 37% of exact rays) |
| 4 rays | .137 | .091 (1.14) | .072 | .163 | 1.26 | 0 / 0 | .0049 | **fail U6** |
| 5 rays | .108 | .076 (0.96) | .056 | .137 | 1.27 | 0 / 0 | .0046 | pass, no margin |
| 6 rays | .100 | .067 (0.86) | .052 | .118 | 1.25 | 0 / 0 | .0045 | pass (fallback) |
| **8 rays** | **.082** | **.056 (0.71)** | **.043** | **.100** | **1.27** | **0 / 0** | **.0038** | **pass** |
| 16 rays | .048 | .032 (0.41) | .025 | .066 | 1.26 | 0 / 0 | .0020 | pass |
| 8 rays + box, width from 8-ray classes | .126 | .086 (1.11) | .057 | .153 | 1.76 | 1 / 17 | .0121 | fail U6, S1–S4 |
| 8 rays + box, oracle width | .160 | .159 (1.94) | .077 | .182 | 1.85 | 2 / 52 | .0630 | fail U6, S1–S4 |
| Production with faint omitted | — | .050 | — | — | — | 0 / 0 | .040 | fail S3, S4 |
| VSM native | .370 | .089 | .516 | .263 | 1.90 | 2,191 / 3,109 | .026 | fails composite |
| HRT 1-spp denoised | .394 | .162 | .332 | .395 | 1.27 | 1,336 / 1,458 | .009 | fails composite |

Fixtures, 8 rays (80 cases each):

- **Thin/contact:** partial RMS ≤.105, ρ ≤1.157 against a 1.342 bound, faint mean z ≤2.49 and motion ≤.181.
- **Speck/wire:** ≤.066, ρ ≤1.223 against 1.430, z ≤2.28 and motion ≤.078.

Both fixtures have zero flips and zero lost contacts. Production's faint z reaches 1.81 and 2.24. The 64-detect output also passes these checks. Its handful of missed speck receivers failed Level A's much tighter production-parity checks: G1 on the quarry and G3–G5 on the stress fixture.

**Inspected (pose 0, seed 0):**

- The 8-ray error map is unstructured, two-signed grain confined to penumbra lines.
- Contacts, rubble shadows, crevices and the slab shadow are complete in the 8-ray visibility image.
- VSM and HRT show one-signed blobs of lost shadow at contacts and rubble. HRT also shows persistent dark points at (945,276) and (1388,529).

Lit and +4-stop images need the renderer's HDR and are deferred to the unit.

### Limits

The bounds are specific to this quarry view: three poses with a 1 cm X step, a static sun and blockers, 1x and no AA. A new scene needs its own native Unreal measurement. The comparison disables TSR on both sides, while Unreal's shadows are designed for temporal reconstruction. A comparison with temporal AA on both sides would set a different bar for both renderers. The contact band is a screen-space proxy. Moving lights and blockers, camera rotation and 4x are not covered.

## 2. Method: the 8-ray prefix of the exact estimator

Each covered receiver traces directions 0–7 of its existing sequence: R2 with the per-pixel Cranley–Patterson shift from the fixed screen hash. In the existing 2×2-receiver, 8-lane layout that is one ray per lane and no loop. The count is scaled by 32, so the `r32uint` 0–256 contract, `FRESH_SKIP` handling and shading are unchanged. Exact 256-ray mode stays selectable as the reference.

How it avoids each recorded failure:

- **Banding** (the Vogel disk): per-pixel randomized shifts make errors independent between pixels; ρ equals production's (M).
- **Lost contacts and solid flips** (the 8-ray box proxy): there is no spatial filter. A solid receiver is exact by construction, because all of its directions agree (M: zero on all cases).
- **Missed faint shadows** (64-detect, USS). Every receiver traces the same directions regardless of its own outcome, so the estimate is unbiased. Up to 1,086 faint receivers show fully lit on one seed; others show darker than truth, and the faint mean stays within 0.0038 of the reference (M). Faint error is noise, bounded by U6, not systematic loss. Schemes that decide a receiver's treatment from its own samples are biased by construction, which is the shared mechanism of the 64-detect and USS failures.
- **Temporal reset cost** (history): the method keeps no history. Camera, light and blocker motion cost the same as a static frame (I). Static frames are deterministic.

**Why no reconstruction filter (M).** A 3×3 same-surface box applied to the *reference itself* changes it by RMS .17 on wide penumbrae, .26 on narrow, .26 on faint and .33 on isolated receivers. That is two to four times the 8-ray noise (.082). Rough scans carry pixel-scale micro-occlusion inside penumbrae, consistent with the [traced-receiver finding](m6-primary-ray-production.md). Any kernel of radius ≥1 adds more bias than the noise it removes. A filter bounded by the oracle penumbra width (radius width/3, at most 4) still raised wide-penumbra RMS from .083 to .194 and faint RMS from .056 to .159, and lost 52 contacts.

### Cost and targets

| Pass | Hard frame (M) | Exact soft (M) | Level-B target | Basis |
|---|---:|---:|---:|---|
| Trace (soft frames also write receivers) | 0.306 | 0.315 | 0.315 | Unchanged |
| Visibility | — | 7.60–10.00 | **≤0.30** | I: 8/256 of exact is 0.24–0.31 at the measured 0.086–0.113 ns per ray; a cold first ray (the inline hard ray costs about 0.19 ms) plus 7 warm rays is about 0.40 |
| Reconstruction | — | — | none | Rejected above |
| Shading | 0.597 (includes the inline hard ray) | 0.398–0.416 | 0.40 | Unchanged soft shading |
| Tone | 0.06 | 0.06 | 0.06 | |
| **Span** | **0.964** | 8.39–10.93 | **≤1.10** | About 1.08 at target |

The fast and slow GPU states move every pass by about 25%. Targets apply to the fast state and to paired ratios.

- **Against Unreal's packaged pose-0 GPUTime** (VSM 1.60, HRT 2.15 ms, M): 1.08 ms is 1.48x VSM and 1.99x HRT, at the higher measured fidelity above.
- **2x floors:** VSM 0.80 ms, HRT 1.075 ms. The HRT floor needs visibility at ≤0.30 ms.
  - The **VSM floor is below today's hard frame**, so no shadow method reaches it. It needs trace plus shading (0.715 ms) to fall below about 0.44 ms, or a case where VSM pays more, such as a moving sun (unmeasured).
- **Memory:** no new resources.

## 3. Implementation unit

**Scope:** traced primary visibility on regular full-resident geometry, 1x, a directional light and the 0.04 rad disk, as for exact fresh visibility.

**Code:**

- `shaders/fresh_shadows.wgsl`: the direction count becomes a pipeline-overridable constant, 256 or 8.
- `fresh_shadow_gpu.rs`: builds both pipelines in the existing reload transaction.
- `Renderer::set_soft_shadows(SoftShadows::Exact | SoftShadows::Interactive)`, with the frame's choice reported in `FrameReport`. The default stays `Exact` until acceptance.
- No filter, history or new resources.

**Phases and acceptance evidence:**

1. **Cost probe.** Paired in-process visibility timing on traced receivers against exact and hard: one process × 24 pairs first, with more processes only near a threshold. Layouts:
   - 8 lanes × 1 ray (2×2 tile);
   - 4 lanes × 2 rays (2×4 tile);
   - 1 lane × 8 rays (8×4 tile).

   Retain the fastest.
2. **Exactness.** With validation active, GPU 8-ray counts equal 32 × the popcount of the first 8 bits of dumped production hit masks. This covers seeds 0–3 at the three traced-receiver poses and both fixtures at 5 poses × 16 seeds. Mask dumps use the existing diagnostic path from `e1c7822`. Reload, disabled-target and removed-blocker checks are unchanged.
3. **Gate.** Move `gate_b.py`'s rules into `support/` beside `fresh_gate.py`. Score U1–U10 and S1–S5 against the traced-receiver reference (seeds 1–64; spread seeds 65–80 from the primary-ray acceptance run). Unreal masks are rescored against the same reference.
4. **G7-B inspection,** including the renderer's lit HDR at registered and +4-stop exposures, and the flip-book.
5. **Timing acceptance.** Paired spans for interactive, hard and exact: 3 processes × 24 pairs, with fast and slow state reported, build identity and contention recorded. Targets are from the table above.
6. **Checks.** CPU tests for the enum and the count scaling; the fresh GPU suite on the traced path in both modes; Clippy and formatting on the renderer crate.

**Stop rules:**

- Best layout above **0.45 ms** (1.5x budget): stop and record. Do not trade quality for speed in this unit.
- Visibility between 0.30 and 0.45 ms: retain with the miss recorded. Then 6 rays (gated here with a 0.86 faint ratio) may run as a new candidate, repeating the gate.
- Any exactness mismatch: fix the implementation, never the replay.
- Any gate or inspection failure on any pose or seed: stop. No threshold or reference revision; filters and history are out of scope.

## 4. Rejected options

| Option | Status | Reason |
|---|---|---|
| 4–8 stratified rays plus an edge-aware, penumbra-bounded filter | Rejected (M) | Worse than unfiltered at every k, with both class-derived and oracle widths; creates flips, lost contacts, bias and structure. The reference itself varies at pixel scale (.17–.33 RMS under a 3×3 box) |
| Quad-interleaved joint stratification with 2×2 reconstruction | Rejected (I) | Not exactly replayable from per-pixel-shifted masks; any radius-1 reconstruction pays the spatial bias above |
| Ray count adapted to penumbra width, triggered by the receiver's own samples | Rejected | Biased by selection on its own samples, the failure mechanism of 64-detect and USS. Neighbour-triggered refinement is unbiased but no check needs it; deferred |
| 4 rays | Rejected (M) | U6 at 1.14x the bound |
| 5 rays | Not chosen (M) | U6 at 0.96x leaves no margin for the traced-receiver rescore |
| 6 or 7 rays | Fallback (M) | Pass at 0.86x and 0.76x of U6 |
| 16 rays | Not first (M) | Passes with wide margin at about twice the ray cost |
| 64-detect adaptive | Rejected on cost | Passes Level B, but costs about 37% of exact rays (about 3 ms) |
| Temporal accumulation or history | Rejected | Prior reset cost (72% of moving receivers, 138 ms); not needed |
| Shadow maps or a VSM hybrid | Deferred | Needs caster LOD and a cache; not needed for quality |
| Blue-noise or spatiotemporal shifts | Deferred polish | Changes the error spectrum, not RMS or bias; tiling-structure risk |
| Gate with Unreal-equal bounds only | Rejected (M) | Admits omitting faint shadows (U6 .050 against .078) and filter bias |
| Gate including back-facing receivers | Rejected (M) | 82% of VSM's umbra flips sit where direct light is zero |
| Display-code thresholds; per-receiver Level A envelope | Not used | As in the [Level-A gate](m6-reduced-ray-gate.md); Level B admits noise |

## 5. Open risks

- **The 8-ray cost is unmeasured.** First-ray cache behaviour decides where it lands in 0.24–0.40 ms. Above 0.30 ms the 2x-HRT figure misses.
- **2x VSM needs work outside shadows,** in trace and shading or a moving-light comparison.
- **Motion noise.** Screen-locked noise swims under camera motion: Δ RMS .10 per 1 cm step, against .26 for VSM. Longer paths and rotation are unmeasured, so inspection must judge their character.
- **Content dependence.** The bounds come from one view. Dense fine casters such as vegetation, cables or grates raise the faint population and its noise.
- **Receiver basis.** Raster-receiver data here; the traced rescore is required. About 0.075% of receivers differ.
- **Terminator receivers** (3,404) are excluded. The candidate is exact there, but any non-exact future method must report them.
- **Correlated error.** S3's SE assumes independent errors. That is true for this estimator but lenient for filters; S4 and inspection compensate.

## Evidence, reproduction and time

Evidence is in `D:/Coding/renderlab-work/m6-level-b/`. `identities.sha256` holds 26 hashes: scripts, outputs, images, the gate metrics and the three references.

- `scripts/levelb.py`: loading, populations and statistics.
- `ue_profile.py`: Unreal and production profiles.
- `prototype.py`: k-ray and filter replay.
- `fixture.py`: fixture replay.
- `gate_b.py`: the Level-B gate with frozen rules.
- `inspect_b.py`: images.
- `out/` holds the JSON results, cached error arrays and `images/`.

Run each script as `py -3 -I scripts/<name>.py` from that directory, with two math-library threads. The scripts import the unchanged gate metrics from `m6-primary-prod/gate/`.

CPU times:

| Run | Time |
|---|---:|
| Profile | 23.4 s |
| Quarry replay | 135.2 s |
| Fixture replay | 15.2 s |
| Gate | 69.6 s |

Structure, flicker, inspection and the omission probe each took under a minute and were not stopwatched. Design and writing time overlapped the analysis and was not isolated. No GPU, window, build, Unreal launch or push occurred.
