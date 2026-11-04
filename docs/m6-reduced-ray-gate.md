# M6 reduced-ray soft-shadow gate

**Decision.** A reduced-ray method may replace the 256-ray [fresh compute estimator](m6-fresh-compute-cost.md) only if it passes the Level-A gate below. Level A requires statistical parity with the accepted estimator. It is measured against a 16,384-ray reference that carries the measured per-receiver noise of the 256-ray estimate.

Try **exact-in-penumbra sequential tracing** first. Every receiver traces 64 detection rays; receivers near detected shadow then trace the remaining 192 production directions. CPU replay of saved evidence predicts 36.8% of production rays on the quarry. That is about 3.0 ms of compute (2.3–4.0 ms with unmeasured overheads), against 8.08 ms, and a complete span of about 9.6 ms, against 14.65 ms. That is roughly the hard-4x anchor, not 2x Unreal.

Detecting faint thin-caster shadows on every receiver sets a floor near 25% of production rays for any Level-A method. Larger savings need a Level-B gate defined from measured Unreal error, not a looser Level A. This unit made no renderer change, no GPU run and no build.

## The 256-ray reference is a noisy estimate

`fresh_estimate` integrates binary visibility over the uniform slope disk of radius tan(0.04), using the R2 sequence with a per-receiver Cranley-Patterson shift. This is randomized quasi-Monte Carlo. The shift hashes the pixel index and is fixed across frames. The estimate is therefore unbiased, with per-pixel noise locked to the screen. Static frames repeat exactly, while camera motion changes which content each pixel's sample set meets.

A CPU replica (`scripts/fixture_model.py`) reproduces 49,141–49,149 of the 49,152 GPU counts on each thin/contact fixture pose. The maximum difference is one count, from f32 edge rounding. Its exact continuous-disk oracle (Gauss–Legendre in disk u, exact interval union in v) reproduces the GPU partial populations exactly (897 / 752). Against that oracle, over the five registered poses:

| Quantity | Measured |
|---|---|
| Production error over partial receivers: RMS / p95 / p99 / max | 0.0067–0.0084 / 0.014–0.017 / 0.017–0.023 / 0.020–0.030 |
| Mean signed error; solid lit and umbra error | within ±0.00035; at most 1e-8 |
| Per-receiver σ256 from 64 seeds, RMS over partials | 0.0064–0.0082 (max 0.014), 0.30–0.35 of the binomial value |
| z = error / σ̂256 over 4,195 receivers | variance 1.06, kurtosis 2.8, maximum 3.9 |

The estimate's noise falls with ray count roughly as N^-3/4. Measured σ is 0.094 at 8 rays, 0.063 at 16, 0.043 at 32, 0.021 at 64, 0.007 at 256 and 0.0024 at 1024. Averaging A independently seeded k-ray neighbours reaches σ256 only at k·A ≈ 600–1,400 rays. Spatial reuse of independent low-count estimates is therefore less ray-efficient than per-pixel QMC unless the neighbours' sets are jointly stratified.

Each direction prefix also has a blind spot: the largest disk fraction it can miss entirely (p95 over 200 shifts):

| Prefix | 16 | 32 | 64 | 128 | 256 |
|---|---:|---:|---:|---:|---:|
| Straight edge | 0.158 | 0.069 | 0.040 | 0.019 | 0.0098 |
| Thin strip | 0.308 | 0.213 | 0.120 | 0.060 | 0.033 |

The accepted renderer itself shows "fully lit" for an aligned thin occluder covering up to about 3% of the disk.

On the saved quarry pose 0, 346,520 receivers are covered: 89.9% lit, 7.6% umbra and 8,547 partial (2.47%). Penumbrae are narrow. Lit-plus-umbra distance has median 3 px, p90 8 px and p99 12.6 px. Partial receivers form 1,438 components with median size one. 831 are isolated; 699 of those adjoin umbra, and 106 with occlusion of at least 0.1 lie away from umbra. The sampled locations of those 106 are in cliff-top crevices.

The binomial σ256 RMS over quarry partials is 0.024 (maximum 0.031). The randomized-QMC value is lower but not yet measured: only 63 broad-penumbra receivers exist, too few for a spatial-residual estimate.

At the registered exposure, lit and umbra display medians are 72.9 and 61.3 codes. That is roughly 0.12 display codes per 0.01 visibility, so visibility defects are nearly invisible in the lit image. One 256-ray draw is too noisy to serve as a per-pixel reference: a 5σ envelope against it would be about 0.1 wide.

## Level-A gate

### Reference and noise

- **Analytic fixtures:** the exact oracle V. σ̂256 comes from 64 independent seeds of the validated CPU replica.
- **Quarry:** V_R is the mean of seeds 1–64 of the unchanged production estimator, 16,384 rays per receiver, and σ̂256 is their standard deviation.
  - Seeds 65–80 are independent production-equivalent draws that measure seed spread.
  - Production, seed 0 (the shipping hash), is independent of all of them.
  - Seeds only salt the existing per-pixel hash. Geometry, bias, directions, receivers and depth are unchanged, so the reference keeps the renderer's estimand.
  - Residual reference noise, at most 0.0039 in the binomial worst case, adds 1/64 to error variance (≤0.8% RMS inflation). That is below the measured 4.3% seed spread.
  - GPU cost is about 80 compute dispatches of 8–12 ms per pose.

Populations come from the reference only:

- **partial:** 0 < V < 1
- **lit** and **umbra**
- **narrow:** lit-plus-umbra distance at most 3 px
- **faint:** V at least 0.9
- **isolated:** no partial 8-neighbour
- **silhouette-adjacent:** relative depth jump above 5% within 3×3

### Checks

Every check must hold for every pose of every workload and for every evaluated seed: 16 seeds on CPU-replayable fixtures and the 4 dumped quarry seeds. Motion changes how sample sets meet content, so passing on one seed is not robustness. Production passes all 16 seeds on both CPU fixtures.

| Check | Threshold | Justification |
|---|---|---|
| G1 per-receiver envelope, partial | \|c−V\| ≤ max(5·σ̂256(x), q, 1/256); zero violations | Production's maximum \|z\| is 3.9 over 4,195 receivers. A Gaussian tail gives about a 1.5% false-fail at the quarry's ~26k receiver-poses. q is the median, over seed-spread draws, of production's p99 \|error\| on that image (thin/contact 0.0225, stress 0.0210): errors the accepted estimator already makes at 1% of partial receivers. p99 is used rather than the maximum because the maximum varies 20% between seeds while p99 varies 7%. 1/256 is production's quantum. |
| G2 solid | ≤ 0.002 on lit and umbra | Unchanged prior softness gate. Production is exact (≤1e-8). |
| G3 aggregate parity, partial and every sub-population with n ≥ 100 | RMS and p99 ≤ production median × (1+s); \|mean\| ≤ \|production mean\| + 2 SE | s is the measured 2-s.d. relative spread of that statistic across independent production seeds on the same image: thin/contact RMS 4.3%, p99 7.2%, faint RMS 7.8%. |
| G4 structure | Mean signed error along a 9-px segment of the reference iso-contour, same surface: RMS ≤ median × (1+s) (14%); maximum ≤ maximum over draws (0.0147) | White noise shrinks threefold under the segment mean; bands, blotches, bias and seams that follow shadow edges do not. |
| G5 temporal | Repeat render and return pose bit-identical; motion residual Δ(c−V) over consecutive registered poses: RMS ≤ median × (1+s) (7.3%), p99 ≤ maximum over draws (0.0379) | Production is deterministic and screen-locked; the residual is measured on the existing motion poses. |
| G6 enumeration | List every receiver shown fully lit or fully shadowed where V is partial, with V, σ̂, error and crop | Missing partial shadows are the recorded [reuse failure](m6-solid-reuse-rejection.md). Those beyond the envelope fail G1; those within it are reported, not hidden. |
| G7 full-image inspection | No defect visible in candidate−reference that is absent from production−reference | See the inspection requirements below. |

G7 inspection covers:

- complete reference, production and candidate visibility images
- ×8 signed-error maps on one scale
- lit images at the registered exposure and at a high-contrast exposure
- crops of silhouettes, slab and wedge contacts, rubble, crevices, cliff-top isolated partials, the thin strip, the contact sliver, specks and wires
- motion flip-books

Numbers do not override inspection.

Calibration with the exact replica on the thin/contact fixture, over 16 seeds:

| Estimator | Partial RMS | Along-contour RMS / max | Motion RMS | Solid max | G1 violations per pose | Verdict |
|---|---:|---:|---:|---:|---|---|
| Production | 0.0082 median | 0.0027 / 0.0147 | 0.0122 | 1e-8 | 0 | pass |
| Fixed 32-direction Vogel disk ([rejected screen](m6-softness-screen.md)) | 0.034 | 0.034 / 0.070 | 0.061 | 1e-8 | 1–150 | fails G1, G3, G4, G5 |
| First 32 production rays, unfiltered | 0.042 | 0.015 / 0.061 | 0.076 | 1e-8 | 250–284 | fails |
| 8 rays plus 5×5 same-surface box (spatial proxy) | 0.072 | 0.069 / 0.110 | 0.081 | 0.85 | 501–803 | fails G1–G5 |
| Method 1, 64 detection rays | 0.0085 (production maximum) | 0.0147 | 0.0132 | 1e-8 | 0 on all 16 seeds | pass |

The inspected sheet `images/fixture-thin-contact-sheet.png` agrees with these numbers:

- **Vogel:** bands along both strip edges.
- **Spatial proxy:** a blotchy strip, with the 1-px contact sliver smeared into a block.
- **Method 1:** error indistinguishable from production.

These reproduce the recorded banding and contact-loss failures.

Thresholds not adopted:

- **The original softness thresholds** (penumbra RMS 0.02, p95 0.04, maximum 0.075, motion 0.02 / 0.04): 2.4–3 times looser than the measured accepted estimator, so they would admit a visible regression.
- **A per-pixel comparison against one 256-ray draw.**
- **Display-code thresholds:** at this exposure they would accept large visibility errors, and shadows must hold for any light intensity or exposure.

Level A certifies parity with the accepted estimator at 1x, 0.04 rad, on the registered workloads and poses. It does not certify continuous ground truth beyond the analytic fixtures, 4x, other disk angles, moving blockers, virtual geometry or Unreal parity. It may be stricter than Unreal-equivalent fidelity, which only measured Unreal error can decide.

A failed check stops the candidate. Thresholds and references are not revised afterwards. Changing a configuration creates a new candidate, which repeats the whole gate.

## Method 1: exact-in-penumbra sequential tracing (first)

1. **Stage 1:** every covered receiver traces the first 64 directions of its unchanged production sequence (8 per lane in the existing 2×2-receiver, 8-lane layout). It is classed lit, umbra or mixed.
2. **Region:** mixed receivers dilated by one pixel, plus every lit receiver adjacent to an umbra receiver and vice versa (the adjacent pair only).
3. **Stage 2:** region receivers are compacted into a list and trace the remaining 192 production directions through an indirect dispatch. Their counts equal production exactly.
4. **Output:** all other receivers write 0 or 256 from stage 1. The output texture, the `FRESH_SKIP` handling and shading are unchanged.

The image differs from production only where stage 1 reports uniform and production would be partial; G6 enumerates every such receiver.

It avoids the recorded failures:

- **Banding:** no low-count pattern reaches the output.
- **Contact and solid loss:** there is no filter.
- **Resets and history:** the method is stateless and deterministic, with no reprojection or disocclusion.
- **USS misses:** the four agreeing deterministic rays are replaced by a 64-direction prefix per receiver, with neighbour and umbra-adjacency triggers. The stress fixture below shows that smaller prefixes still miss.

The thin/contact fixture lacks the quarry's risk class, isolated faint partials away from umbra. A CPU-only **speck/wire stress fixture** adds it (`scripts/speck_fixture.py`):

- 72 one-centimetre squares at 0.1 / 0.2 / 0.4 m
- 12 one-millimetre wires along both axes at 0.1 / 0.3 m
- the same receiver, camera, light, estimator and five poses as the thin/contact fixture
- 854–903 partial receivers per pose, 95% of them faint

Production passes all 16 seeds there, while itself dropping 12–25 very faint receivers per five-pose run, all within the envelope.

| Configuration | Thin/contact, 16 seeds | Stress, 16 seeds | Quarry rays (model) | Quarry modelled misses ≥0.05 / ≥0.1 per pose |
|---|---|---|---:|---|
| 16 rays, region to 128 | pass | 16 fail (807 violations) | — | — |
| 32 rays, region to 128 | pass | 14 fail (80; errors 0.026–0.081) | 19.6% | 1.75 / 0.13–0.56 |
| 32 rays, region to 256 (edges dilated) | — | 12 fail | 33.4% | 1.6–2.1 / 0.13 |
| 48 rays, region to 256 | pass | 2 fail (excess ≤0.019) | 31.5% | 0.3–0.4 / ≤0.06 |
| **64 rays, region to 256** | **pass** | **pass** | **36.8%** | **0.06–0.19 / ≤0.06** |
| 64 rays, region to 256, edges dilated | pass | pass | 42.9% | 0.06 / 0 |

Notes on the stress results:

- At 32 rays, 60% of the violations come from wires and 40% from specks, and all are missed occlusion.
- Their counts are 80 / 40 / 31 / 10 at floors q = 0.021 / 0.029 / 0.036 / 0.05, so that failure does not depend on the floor choice.
- The two 48-ray violations would pass a floor near 0.04; the 48-versus-64 boundary does depend on q.
- Randomly rotating the stage-1 pattern did not help (32 rays: 14 of 16 fail).

The quarry columns come from a model: hypergeometric prefix outcomes given each receiver's saved count. On the thin/contact fixture that model predicts 1.5–2 times more misses than the exact replica, but thin casters may differ. The quarry decision therefore comes from the GPU hit-mask replay below, not from this model.

**Cost.** The 36.8% splits into 25.0% for stage 1 and 11.8% for the 54.3k region receivers. If cost is linear in rays (measured 0.451 at 128 rays), that is 2.97 ms of the 8.08 ms process-2 median.

Several costs are unmeasured, so the estimate is 2.3–4.0 ms:

- the classification and compaction pass
- the indirect dispatch
- stage 1's 8-ray lane loops instead of 32
- per-ray cost near shadow edges

Adding the 6.57 ms of non-compute passes from the same process gives a complete span of 8.9–10.6 ms, central about 9.6 ms, against 14.65 ms. Absolute levels vary about 20% between processes.

Memory adds one 4-byte list entry per receiver (8.3 MB worst case at 1080p) and indirect arguments; stage-1 class bits fit in the existing `r32uint` output.

**Pricing:**

- **Against hard-4x:** the 9.5–9.8 ms [hard-4x](m6-history-quarry-cost.md) figure is a mesh interval from another session. The central estimate is at that level, not below it. The timing screen must measure hard-4x in the same session and binary.
- **Floor:** stage 1 alone is about 2.0 ms, so this route cannot bring the span below about 8.6 ms. The two 13 M-triangle raster passes (about 6.5 ms) then dominate.
- **Against Unreal:** there is no Unreal quarry denominator. At a 9.6 ms span, 2x requires a matched Unreal frame of at least 19 ms. Below about 13 ms, 2x is unreachable even with free shadows unless rasterization shrinks, which lies outside this unit.

## Method 2 (conditional): pooled certification with penumbra-bounded reconstruction

This method applies only inside Method 1's region:

- Uniform region receivers stop at a neighbour-pooled certification set: per-receiver interleaved subsets of one jointly stratified 256-direction set per quad.
- Mixed receivers whose stage-1 hit distances imply a penumbra of at least 6 px trace 256/A interleaved directions. A depth- and normal-guided box, bounded to a third of the penumbra width, reconstructs them.
- Narrow penumbrae keep all 256 directions.

On the quarry its ceiling is small. Region tracing is 11.8% of production rays (1.85% of it mixed), and only about 10–25% of partials are 6 px or wider. That caps realistic savings at about 8% of production rays (about 0.65 ms), while reintroducing the spatial-filter contact risk and a certification risk.

It pays where the full-trace population is large: wider disks, closer cameras, dense fine casters, and especially a future 4x per-sample path. There, four samples sharing one pixel's 256-direction set average in the HDR resolve to a 256-direction pixel estimate at soft-1x ray cost. That path is not authorized here and would face the same gate on resolved visibility.

Not selected:

- **Uniformly fewer rays with any filter:** parity at 32 rays needs about 40 independent same-surface neighbours, which narrow penumbrae lack.
- **Temporal history:** rejected.
- **Analytic edge integration:** blocker discovery is unresolved.
- **Importance sampling:** a binary integrand has no density to sample without occluder knowledge, and stage-1 hits are the only such knowledge available.
- **Conservative light-space bounds:** the receiver's own surface defeats certification without bias.

## Fixtures and evidence the gate needs

1. **Thin/contact fixture:** the existing 256×192 five-pose GPU test, with the CPU replica and oracle.
2. **Speck/wire GPU fixture:** the CPU layout above with casters visible to the camera, plus wires at 12 angles to sweep the blind-spot orientation. The oracle extends to rotated rectangles. Five X-offset poses.
3. **Quarry dump:** one diagnostic GPU process with validation active, on the registered original, +1 cm and return poses. It saves per-receiver 256-bit production hit masks for seeds 0–3 (11 MB per seed and pose) and counts for seeds 1–80, beside the existing depth. One light-direction step is optional. CPU replay then evaluates any prefix configuration exactly on four seeds under every check, with no further candidate GPU runs.
4. **Implementation exactness:** the production shader must reproduce the CPU-replayed counts bit-exactly at seed 0 on every pose of all three workloads, with active validation. Reload, disabled-target and removed-blocker checks stay as they are.
5. **Timing:** the existing paired screen, four processes × 24 alternating pairs against frozen production. Stage-1, compaction and stage-2 timestamps are reported separately, alongside same-session hard-4x. Ray and region counts come only from diagnostic builds.

Stop rules:

- Stop if 64 rays fails the quarry dump or the GPU stress fixture.
- Retain the method only if every process's paired compute ratio falls below 0.85, outside the 0.85–1.11 identical-source control. The prediction is 0.28–0.50.

## What remains for the Unreal comparison

- **Scene:** a private bulk import of the 12 placed quarry scans into the existing UE 5.7.4 CL 51494982 project, with matched transforms and materials and the environment black, as in the [directional-black subset](unreal-comparison-setup.md). The fixture JSON-buffer path does not scale to 13 M triangles. Scans stay local and ignored.
- **Light:** the same direction, with Source Angle equal to the full diameter, 2 × 0.04 rad = 4.584°. Record the O(α²), roughly 0.2%, difference between a solid-angle cone and the uniform slope disk.
- **Shadow configurations:** virtual shadow maps with shadow-map ray tracing, and hardware ray-traced shadows with their denoiser, both with Nanite on. Record the effective console values and AA/TSR state. Unreal's soft shadows depend on temporal accumulation, so measure both the first frame after a cut and the steady state.
- **Visibility:** per pixel, shadowed direct radiance divided by unshadowed direct radiance, from paired captures. Score it with G1–G7 against the same R. That measured Unreal error profile is the only valid basis for a Level-B gate.
- **Performance:** complete GPU frames and shadow passes at 1920×1080 on the same poses, steady and moving, with build identity and contention recorded. The open M5 direct-specular disagreement remains separate.

## Open risks

- **The quarry miss model is approximate.** The dump decides.
- **The stress fixture is CPU-only and stylized.** The 48-versus-64 boundary depends on q. Denser fine casters, such as vegetation, cables or grates, may need more detection rays; this floor scales with caster statistics, not penumbra area.
- **Cost linearity is unproven.** Short stage-1 loops, compaction and per-ray cost near shadow edges are unmeasured, hence the 2.3–4.0 ms range.
- **The quarry's σ̂256 is unknown** (between 0.3 and 1.0 of binomial). Only the dump fixes the envelope widths.
- **Level A may reject methods that Unreal-equivalent fidelity would accept.**
- **The gate scores visibility only.** Exposure compression hides visibility defects in the lit image, so inspection must view visibility directly.
- **Rasterization dominates afterwards.** After Method 1, the double raster is about 6.5 of the 9.6 ms span. The 2x Unreal floor likely needs work outside the shadow unit.

## Evidence, reproduction and time

Evidence lives in `D:/Coding/renderlab-work/m6-reduced-ray-gate/`:

- `scripts/`: fixture model and oracle, noise, calibration, stress fixture, quarry structure, sequential model, blind spot, gate metrics, sheets
- `out/`: JSON, NPZ and logs
- `images/`
- `identities.sha256`: 45 hashes, including the input counts and depth from `m6-compute-cost/runs/fixture-c3` and `quarry-c3r`

Each script runs as `py -3 -I scripts/<name>.py` with the arguments in its header. The work was CPU-only: no GPU, window, Rust build or asset read beyond saved counts and depth.

Measured analysis times:

- fixture noise: 43.9 s
- thin/contact calibration: 16.9–34.5 s per run, four runs
- stress fixture: 37.2 s for the main pass of one run, five runs; later runs with rotation and attribution were not stopwatched
- quarry structure: 2.9 s
- tile pricing: 24.5 s

The seven sequential-model runs and the blind-spot run were not stopwatched; each took under two minutes.

Corrections:

- The oracle was generalized to unions of more than two occluders; it reproduces the earlier values exactly on 120 sampled receivers.
- One attribution indexing error was fixed before its result was used.

Design and editing time overlapped analysis and was not isolated.

## First GPU result, 2026-11-04

The 64-detect adaptive candidate (branch `t3/adaptive-fresh-shadows`, `e1c7822`) is rejected on the GPU.

- Thin/contact fixture: G1-G5 pass on all 5 poses and 16 seeds, and the output is bit-identical to production.
- Speck/wire stress fixture: G3/G4/G5 fail on several seeds. The candidate misses more faint partial shadows than production on almost every seed (for example 14 vs 6), because no detection ray hits some faint speck shadows.
- Quarry: 3 of 12 pose/seed runs have one G1 envelope violation where production has none, from the same failure mode. Only 15.7% of covered receivers (54.4k of 346.5k) need the remaining 192 rays, about 37% of production rays, which matches the forecast. Timing was not run because the gate failed first.

Gate implementation correction: G3, G4 and G5 now accept a statistic within either the seed-spread limit or production's value on the same seed. Applied unpaired, the seed-spread limits rejected production's own noisier seeds; candidates identical to production failed 24 thin checks. Both the unpaired results and production's own outcomes are recorded per seed. The thresholds are unchanged.

The quarry 16,384-ray reference (seeds 1-64), the per-pixel 256-ray sigma and the spread draws (seeds 65-80) are in `D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry/` for Level B scoring.
