# M6 soft shadows at Exact quality or better, fast

Design decision, 2026-11-05, based on `48eabf5` (AGENTS.md: final visible image against a film-quality reference; temporal accumulation, caching, denoising and approximations allowed; no screen-space core shadows). CPU evidence only: no renderer change, GPU run, build, window or Unreal launch. Labels: **M** is measured from saved GPU evidence or this unit's CPU replay; **I** is an inference.

## Decision

1. **Strict Level A at about 1 ms is not reachable on this hardware.** The per-frame estimator is already rate-optimal, so matching σ256 needs about 256 rays of information per receiver per frame. No certification removes them: hardware ray queries cannot answer a cone query, and the near field holds 92% of the cost. The best output-preserving speed for a *moving* view remains Exact itself, at 8.4–10.9 ms. A static view can reach the bit-exact Exact image in 32 frames at Interactive cost ([section 1](#1-strict-level-a-at-1-ms)).
2. **Exact is not the visible-image bar on this quarry.** Point-sampled visibility varies strongly inside one pixel footprint (M): partial receivers differ from a compatible neighbour 0.42 footprints away by RMS 0.30. Against a pixel-integrated (film) reference, that puts the Exact path's static error in penumbrae near 0.2 RMS, about 30 times its 0.0065 ray noise (I). Against the same reference, Interactive is only 7–22% worse than Exact (I, from σ_fp and from the proxy below). A GPU film reference must confirm this before anything else is built ([section 2](#2-what-the-film-bar-measures-on-this-quarry)).
3. **Recommended mechanism: a surface visibility cache (SVC).** It is world-space and keyed to instance and object-space position. Receivers are spread over the footprint rather than fixed at pixel centres. Direction samples sit on per-cell lattices in a fixed world direction chart, so a moving sun keeps every traced ray still inside the disk. Movers get exact conservative invalidation, there is a fresh unbiased floor, and the ray budget per frame is hard-capped. Estimated cost is 0.85–0.93 ms per frame with a static or slow sun, and about 1.03–1.11 ms with the sun moving at 10°/s (I). That beats the 2x HRT floor (1.075 ms) except under a fast sun. The 2x VSM floor (0.80 ms) still misses.

   On an exact planar fixture (P3, M), the cache beats Exact against the film reference in every motion and light-change segment: partial RMS .008–.013 against .015–.029. It shows zero missed film-visible shadows, against Exact's 171, and no measurable lag. It is **not yet** Exact-or-better everywhere: static convergence, faint and newly revealed shadows, the moving blocker's shadow, solid leakage and convergence flicker all still fail. [Section 4](#4-recommended-surface-visibility-cache) has the design, [section 5](#5-moving-sun-and-moving-blockers) the moving-sun pricing and the fixture evidence.
4. **The pixel-centre variant of the same cache is rejected (M).** After the 1 cm move, 57–90% of its cells are cold, and stale micro-values leave 153–1,005 persistent dark-ghost pixels. Against the film proxy it gains nothing. Reuse on rough scans works only for an area estimand.
5. **The gate becomes Level F.** It uses a film reference per frame and scores motion, cuts, disocclusion, zoom, moving blockers and moving suns. It adds time-to-quality, lag, ghost, flicker and missing-shadow checks against the reference ([section 6](#6-level-f-gate)). Level A stays as the tool for output-preserving performance work.
6. **First unit (F0):** build the GPU film reference and rescore Exact, Interactive, VSM and HRT against it. F1 (the static-light SVC) and F2 (sliding sun) follow only if F0 confirms the estimand finding ([section 7](#7-first-unit-and-stop-rules)). If F0 does not confirm it, the honest result stands: Exact-quality soft shadows cost 8.4–10.9 ms per moving frame on this hardware. Static views can still reach the bit-exact Exact image in 32 frames at 0.86 ms per frame.

## 1. Strict Level A at 1 ms

Level A means per-receiver parity with the 256-ray point estimator in every population, motion included.

| Route | Level A? | Cost | Failure mode |
|---|---|---|---|
| Fewer rays, better points | No | — | Half-plane discrepancy has an N^-3/4 lower bound (Beck; Alexander), and the measured estimator already falls as N^-0.75 (8 → 256 rays: σ .094 → .007). No point set improves the constant enough: from .082 to .0065 needs about 29x the rays at that rate. Reaching σ256 per frame needs roughly 150–256 rays, so 2.2–4.5 ms of visibility at the measured 0.015 (fused) to 0.030 (separate) ms per ray per pixel |
| Unbiased adaptive allocation with a guaranteed detection floor | No | ≥ 37% of Exact rays, ≈ 3 ms (M, 64-detect) | A floor guaranteed for every occluder needs a cone query. Ray prefixes cannot provide it: the measured blind spot is 12% of the disk at 64 rays for a thin strip, and 64-detect missed faint speck shadows on the GPU. wgpu ray queries only trace rays |
| Cone certification on a software BVH | Unknown, impractical | ~1.3 GB extra memory (second BVH plus triangles for 13 M triangles); cone traversal unmeasured (I: 30–100 ray-equivalents per lit receiver) | The receiver's own micro-geometry intersects the cone near its apex, so most traversal happens there |
| Shadow-map bulk umbra plus exact rays where uncertain | No saving | Saves ≤ 8% | A conservative light-space min/max map certifies only beyond the receiver's near field. At mm texels its inconclusive height is about 5–15 cm, and rays capped at 25 cm already cost 0.917 of the full dispatch (M). Umbra is already cheap through first-hit termination |
| Analytic disk-against-triangle visibility near the receiver | No | Blocker discovery equals the cone query, plus a union of hundreds of projected micro-triangles | Scans are not watertight; overlapping occluders make edge-integral methods inexact; numerically fragile |
| Importance sampling | No | — | A binary integrand has no density to sample without occluder knowledge; history-derived strata are caching |
| Static progressive accumulation (frame k traces directions 8k..8k+7 of the same sequence) | **Static only: bit-exact** | Interactive cost | At frame 32 the counts equal Exact by construction, and later frames improve on it. Any camera, light or geometry change returns to 8-ray quality |

An oracle classifier that knew which receivers Exact counts as partial would make Level A cheap. Tracing 256 rays only on those 2.47% of receivers, after compaction, is about 0.2–0.3 ms (I). The whole difficulty is the oracle, and nothing available supplies one exactly.

## 2. What the film bar measures on this quarry

AGENTS.md now judges the final image against a high-sample path-traced reference. Such a reference integrates over the pixel footprint. The Exact estimator samples one point, the pixel centre.

**In-footprint variation (P1, M).** Inputs are the traced 16,384-ray reference and the traced receivers. Pose 1 is the registered 1 cm step, so its pixel centres land between pose-0 centres on the same surfaces. For each pose-1 receiver, take the nearest pose-0 receiver with normal agreement above 0.5 and compare their reference values. Footprint is view distance × 0.001028 rad (the measured median). Reference noise is 0.0008, which is negligible.

| Population (pose 1) | n | Pair RMS at <0.125 fp | 0.25–0.5 fp | 0.5–0.75 fp | All pairs < 1 fp (mean 0.42 fp) | Estimated in-footprint σ | Pairs differing > 0.1 (0.25–0.75 fp) |
|---|---:|---:|---:|---:|---:|---:|---:|
| partial | 8,757 | .148 | .319 | .322 | .304 | .215 | 49–52% |
| faint (V ≥ 0.9) | 1,524 | .144 | .397 | .394 | .378 | .267 | 34–35% |
| narrow | 4,653 | .149 | .389 | .409 | .365 | .258 | 67–69% |
| wide | 4,104 | .146 | .196 | .242 | .216 | .153 | 27–40% |
| transition (≤ 2 px from a class change) | 107,314 | .131 | .198 | .265 | .210 | .149 | 7–11% |
| lit | 310,205 | .073 | .070 | .082 | .072 | .051 | 0.7% |

The estimated in-footprint σ is pair RMS/√2: the RMS difference between one point value and the footprint mean. For comparison, same-pose 1-pixel neighbours differ by .420 on partials and .298 on transitions. This agrees with the earlier findings: a 3×3 box changes the reference by .17–.33, and 72% of nearest history anchors fail a .999 normal test under the 1 cm move because source microfacets turn by 5–144°.

**Two-lattice footprint proxy (P2, M).** As a crude stand-in for the film value, average the point reference over the pose-0 and pose-1 receivers within half a footprint (median 2 samples). The proxy averages two points, so it understates the true film difference.

| Pose 0, against the proxy | Partial RMS | Faint | Narrow | Isolated | Transition | Film-visible shadow (proxy ≤ 0.9) shown ≥ 0.99 |
|---|---:|---:|---:|---:|---:|---:|
| Point reference itself | .116 | .119 | .139 | .182 | .077 | 1,414 |
| Exact, 256 rays | .116 | .119 | .139 | .182 | .077 | 1,413 |
| Interactive, 8 rays | .142 | .131 | .161 | .200 | .080 | 1,593 |

Against the point reference, Exact misses no shadow and Interactive misses 122. Against the proxy, both miss about 1,400 film-visible shadows per pose, where the pixel centre is lit but the footprint is partly shadowed. Exact's 256-ray precision is spent on a quantity the film reference does not show.

**Consequences:**

- **The point gate cannot certify the visible image.** Level A and B measure parity with an aliased estimand. On these scans the aliasing error (≈ .15–.27, I) exceeds the 8-ray noise (.082), so "Interactive versus Exact" is mostly not the visible difference. The film reference must decide, and it can move the bar either way.
- **Temporal reuse on rough micro-geometry works only for an area estimand.** A new pixel centre is a new point with new micro-occlusion. Screen-space history failed for exactly this reason (72% of anchors reset), and P2 shows the same for a world cache fed by pixel centres.
- **There is headroom above Exact.** A footprint-integrated estimate with N samples spread over position and direction has error ≤ 0.5/√N, and QMC does better. It matches Exact against the film reference at about N ≈ 6–8 per footprint and is about 3x better at N = 64 (I, if σ_fp ≈ 0.2 holds).
- **The receiver policy may contribute.** Each microfacet's origin moves 1 mm along its own geometric normal, comparable to the micro-geometry. Pairs 1.25 mm apart already differ by .148. A physically based reference uses a minimal epsilon, so F0 must produce both a policy reference and a minimal-epsilon reference. If they differ by more than Exact's error, the receiver policy is itself a visible defect, and fixing it costs nothing.

## 3. Options priced against the film bar

| Option | Film bar reachable? | Cost (I unless marked) | Failure modes | Status |
|---|---|---|---|---|
| **SVC, area-distributed receivers, sliding direction window** | Plausible if F0 confirms σ_fp. Fixture: better than Exact under motion, not yet when static, faint, revealed or near a moving blocker | 0.85–0.93 ms static or slow sun; +0.18 ms at 10°/s | Time-to-quality on smooth content; solid leakage at cell edges; LOD transitions; thin two-sided features; dense movers | **Recommended, conditional on F0** ([§4](#4-recommended-surface-visibility-cache)) |
| World cache, pixel-centre receivers (C0) | No gain | ≈ Interactive | P2: 57–90% cold after the 1 cm move (c = 1.0 / 0.5 footprint cells); stale micro-values persist as 153 (c = 0.5) to 1,005 (c = 1.0) dark-ghost pixels against the point reference for ≥ 64 frames. Against the proxy .108–.124 versus Exact's .115 | Rejected (M) |
| Screen-space reprojected accumulation (TAA-style shadow history) | Out of policy | Measured 138 ms with 256-ray resets | 72% of anchors reset under 1 cm motion (M); off-screen and disocclusion loss; AGENTS.md avoids screen-space core shadows | Rejected |
| Per-pixel progressive accumulation, exact-state cache | Static frames only | Interactive cost | Bit-exact Exact at frame 32, better later; any motion drops to Interactive | Optional static path inside SVC; not standalone |
| Cross-pixel spatial denoiser (edge-aware, à-trous, penumbra-bounded) | No | 0.05–0.2 ms | Every tested kernel ≥ radius 1 was worse than unfiltered (M, Level B). Against the film reference, filtering *within* the footprint is integration; filtering *across* pixels is still bias | Rejected; the SVC integrates only within the footprint |
| Lag-bounded exponential history for a moving sun | No under steady motion | Interactive or less | Bias ≈ (2/(πα))·ω·W. At 0.25°/s and 60 Hz, staying under Exact's noise needs W ≤ 5.6 frames, about 45 effective rays (σ ≈ .026). Worse at higher speeds | Rejected; the sliding window has no lag |
| Reset on light change | No under steady sun motion | Interactive | Continuous sun motion means permanent 8-ray grain | Fallback only for cuts and jumps |
| Shadow-map or VSM hybrid | Not needed | Page rendering for 13 M triangles; cache invalidation | Receiver bias erased 31% of contacts in UE VSM (M); light-space texel aliasing on rough scans | Rejected for core shadows |
| Analytic disk visibility, importance sampling, cone certification | — | See [§1](#1-strict-level-a-at-1-ms) | Same blockers as for Level A | Rejected |
| 16 fresh rays (no reuse) | Below film target under the σ_fp hypothesis; better than Interactive | +0.12 ms | Grain .048 versus point; aliasing unchanged | Fallback only |

## 4. Recommended: surface visibility cache

**Estimand.** For each light and pixel, the SVC estimates the mean binary disk visibility over the pixel footprint on the pixel-centre surface. It keeps the existing slope-disk measure and the receiver policy F0 selects. Shading still reads one visibility scalar per light per pixel, so user-authored shading stages are unaffected. **Recorded constraint:** the value is box-filtered over the footprint and integrated over the disk, so a shader cannot request per-direction or per-sub-pixel visibility from the cache.

**Cells.** A sparse hash table of world-space cells:

- **Key:** instance, LOD level, object-space cell coordinate, and a light-facing side bit. The cell edge is the power of two nearest half the footprint at the receiver.
- **Object-space keys** carry a rigid mover's cells with it. Its visibility is still invalidated when it moves.
- **Entry: 32 B.** 8 B key, 4 B count and value, 4 B lattice and sequence state, and 16 B of outcome bits for about 64 in-disk directions per cell (about 256 per footprint).
- **Display table:** a separate 4 B entry per cell (key tag and u16 value), so display reads touch 4 B per cell.
- **Capacity:** 2^21 cells with least-recently-seen eviction, about 72 MB in all. The visible quarry needs about 1.4 M cells; the rest retains off-screen surfaces.

**Receivers spread over the footprint.**

- Each frame, one jittered closest-hit camera ray per 2×2 pixel block lands at a stratified sub-pixel position over the block's four footprints.
- Each such receiver traces 8 shadow directions into its cell: 2 rays per pixel per frame on average.
- The displayed primary ray stays at the pixel centre, so geometry, hit records and shading are unchanged.
- Jittered hits that land on another surface (at silhouettes) feed that surface's cells, which is correct for those cells.

P2 is why pixel-centre receivers are not used.

**Directions on a world direction chart.**

- Each cell holds layers of strip lattices, with 8 in-disk points per layer and a per-cell random shift.
- The chart is aligned with the light's direction of motion; a static sun centres it on L.
- Layers fill at the base rate. When the light moves, points of full layers that enter the disk are traced first, and points that leave are dropped.
- The estimate uses full layers only. The in-disk points of a uniform lattice are a stratified sample of the current disk, so the estimate is unbiased with no lag ([§5](#5-moving-sun-and-moving-blockers)).

**Display.** The displayed value is the box filter of the cell field over the pixel footprint: up to 3×3 cells on the tangent plane, weighted by overlap. Cells without a full layer are skipped. If less than half the footprint is covered, the pixel uses the fresh floor below.

**Fresh floor.** Pixels without enough cache coverage trace the existing fused Interactive rays (8 at the centre). The transient image is therefore never worse than today's Interactive image, which F2 of the gate checks.

**Budget cap.** The per-frame average is capped at B = 8 rays per pixel, Interactive's measured cost. The cap covers the fresh floor, maintenance, re-traced invalidated rays and new layers. Priority order:

1. Fresh floor.
2. Re-traced invalidated rays.
3. Maintenance of full layers.
4. New layers, youngest cells first.

Over budget, the highest layers lapse first. No path traces 256 rays for any receiver, so the 138 ms reset failure cannot recur.

**Invalidation, exact and conservative:**

- **Light direction:** handled by the sliding window. A jump outside the chart (a cut, or more than α/2 in one frame) resets.
- **Light extent:** reset. **Light intensity or colour:** nothing changes.
- **Moving or deforming instance:**
  - For cells inside the reach box of the instance's old or new bounds, drop only the stored rays that meet those bounds, then re-trace them.
  - The instance's own cells reset.
  - The candidate test is the receiver box from which any disk ray can reach the bounds, the fixture's `reach_box` generalised to world AABBs.
- **Reload, geometry edits, receiver-policy changes:** reset all. That frame is a cold frame within the same cap.

**How it avoids the recorded failures:**

| Recorded failure | Cause | SVC answer |
|---|---|---|
| Vogel banding | One fixed pattern shared everywhere | Per-cell randomized lattice shifts, so errors are independent between cells; Level F checks structure |
| Filters losing contacts and solids | Cross-pixel kernels | No cross-pixel kernel; footprint box only |
| 64-detect and USS missing faint shadows | Treatment decided from a receiver's own samples | No classification anywhere. Every displayed value is an unbiased mean, and the floor never certifies |
| Screen-space history: 72% resets, 138 ms frames | Point estimand plus 256-ray resets | Area estimand: only real disocclusion is cold. Cold work is capped at ≤ 8 rays per pixel |
| Pixel-centre world cache (P2) | Point samples reused at new points | Receivers spread over the footprint |
| Moving-sun reset cost | Scalar history | Sliding direction window: only rays entering the disk are traced |

**Expected cold fraction (M, from the P1 pairs).** A pose-1 receiver can be uncovered only if no compatible pose-0 receiver lies within about one footprint. At ≥ 1 footprint the fractions are 2.8% of partials, 2.5% of transitions, 2.6% of lit and 6.1% of umbra receivers. At ≥ 1.5 footprints they are 0.2%, 0.4%, 0.35% and 1.0%. Another 0.2% have no compatible neighbour at all. By comparison, the pixel-centre cache is 57–90% cold and screen-space history resets 71.7%. Over a 1 cm step, an area cache should pay the fresh floor on roughly 0.4–3% of pixels (I).

**Cost (I).** Measured Interactive values are from the [hillclimb](m6-interactive-climb.md). The split of the fused pass (fixed, primary, shadow) is the pre-fusion diagnostic.

| Pass | Interactive (M) | SVC, static or slow sun (I) | Basis |
|---|---:|---:|---|
| Fused fixed work | 0.07 | 0.07 | Unchanged |
| Primary traversal | 0.23 | 0.23 | Unchanged |
| Pixel-centre shadow rays | 0.12 (8 rays) | ≈ 0.00–0.01 | Fresh floor only on the 0.4–3% uncovered pixels |
| Jittered receivers, one per 2×2 | — | 0.05–0.07 | A quarter of the primary traversal |
| Shadow rays, 8 per jittered receiver | — | 0.03 | 2 rays per pixel at the measured 0.015 ms per ray per pixel |
| Cache update (~86 k atomics) and display reads (≤ 9 × 4 B per pixel) | — | 0.03–0.08 | 12.5 MB of reads per frame; the working set is near the 48 MB L2 |
| Shading | 0.396 | 0.396 | Unchanged |
| Tone | 0.049 | 0.049 | Unchanged |
| **Frame** | **0.861** | **0.85–0.93** | 2.3–2.5x HRT, 1.7–1.9x VSM |

Cut frames run the fresh floor everywhere plus jittered receivers, about 0.95–1.0 ms. The 2x VSM floor (0.80 ms) is still out of reach through shadows alone.

## 5. Moving sun and moving blockers

**Price of a moving sun.** For a sun at angular speed ω per frame, keeping N_fp in-disk samples per footprint needs about (2ω/(πα))·N_fp new rays per footprint per frame. That is the leading crescent the disk sweeps (I, verified by P3 below). At 60 Hz, α = 0.04 and N_fp = 256 (Exact's count):

| Sun speed | Rays per pixel per frame | Added cost (I) | Frame (I) |
|---|---:|---:|---:|
| Static | 0 (after convergence) | 0 | 0.85–0.93 |
| 0.25°/s (24 min game day) | 0.30 | +0.005 ms | 0.86–0.94 |
| 1°/s | 1.2 | +0.02 ms | 0.87–0.95 |
| 10°/s (time-lapse) | 11.9 | +0.18 ms | 1.03–1.11 |
| Jump of more than α/2 per frame | Reset | Cold frame within the cap | ≈ 0.95–1.0, then converges |

The chart follows the sun's path, a great circle locally straight over 2α. An arbitrary user-driven light whose motion direction keeps changing re-bases the chart and pays one cold transient each time. That is recorded as a risk.

The alternatives price worse:

- **Exact:** stateless at 8.4–10.9 ms at any speed.
- **Interactive:** 0.86 ms with permanent grain.
- **Reset on change:** permanent Interactive grain while the sun moves.
- **Lag-bounded decay:** about 45 effective rays (σ ≈ .026) at 0.25°/s for bias within Exact's noise, and worse when faster.

**Price of a moving blocker.** Re-traced rays equal the stored rays that meet the mover's old or new bounds, within its reach box. That number grows with the mover's shadow-region area and its speed, not with the scene. Whole-cell invalidation costs the fresh floor over the entire reach box every frame it moves.

### Fixture prototype (P3, M)

P3 is an exact planar fixture authored for this unit, CPU only:

- **Scene:** 128×96 orthographic pixels of 1 cm, with the same disk (tan 0.04, 1 mm bias) lit from 45°. Occluders: a 2 cm strip at 0.6 m, a contact plate 2 cm up, six 1 cm specks at 0.1, 0.2 and 0.4 m, and four 1 mm wires. A 10 cm plate at 0.25 m moves 6 mm per frame and uncovers a wire shadow no frame has seen.
- **Film reference:** the exact disk oracle at 4×4 sub-pixel points, validated against 65,536 brute-force rays to within 2e-4.
- **Sequence, 136 frames:**
  - static, frames 0–39;
  - camera pan at 0.37 and 0.11 px per frame;
  - moving plate, 64–79;
  - slow sun at 7.3e-5 rad per frame, 88–119;
  - fast sun at 3e-3 rad per frame, 120–127;
  - holds between segments.
- **C1 as simulated:** 0.5 px texture-space cells with jittered in-cell receivers and 8-point layers (16 at most). About 4 rays per pixel per frame warm, 32 per pixel cold. Full layers are maintained first, the moving plate uses per-ray invalidation, and the display is an exact footprint box.

| Partial RMS against film | Static 32–39 | Pan | Moving plate | Slow sun | Fast sun | Final hold |
|---|---:|---:|---:|---:|---:|---:|
| Exact (256, point) | .0158 | .0293 | .0255 | .0249 | .0189 | .0150 |
| Interactive (8) | .0850 | .0879 | .0915 | .0894 | .0853 | .0823 |
| C0, pixel-centre cache with decay | .0178 | .0559 | .0271 | .0349 | .0572 | .0368 |
| **C1, SVC** | **.0172** | **.0133** | **.0108** | **.0090** | **.0084** | **.0087** |
| Faint: Exact / C1 | .0094 / .0161 | .0093 / .0121 | .0074 / .0100 | .0077 / .0086 | .0078 / .0074 | .0081 / .0071 |
| Sub-pixel edges: Exact / C1 | .047 / .010 | .061 / .016 | .090 / .008 | .081 / .007 | .052 / .008 | .035 / .010 |
| Film-visible shadows shown lit (sum over frames): Exact / Interactive / C1 | 0 / 440 / 0 | 107 / 1,123 / 0 | 16 / 1,137 / 0 | 32 / 1,747 / 0 | 8 / 546 / 0 | 8 / 568 / 0 |
| C1 rays per pixel per frame | 4.0 | 4.2 | 5.5 | 4.5 | 25.1 | 0.5 |

**Where C1 is better than Exact against film:** every segment with camera, blocker or light motion, all sub-pixel edges, and missing shadows (zero, against Exact's 171). Measured lag is +0.03 frame with the plate and +0.04 with the fast sun. At the fast sun, C1 traces 25.1 rays per pixel against the formula's 24.4 for this fixture's 512 samples per footprint, confirming the moving-sun price. Exact's error also rises during the pan (.0158 to .0293) as the pixel centres change sub-pixel phase: this is the point-sampling aliasing measured from the other side.

**Where C1 would fail Level F as proposed.** Each item is a design requirement for F1, not a footnote:

1. **Static convergence.** RMS falls from .054 when cold to .028 by frame 12 and .017 by frame 39, but never below Exact's .0158 within 40 static frames at 4 rays per pixel.
2. **Faint shadows** stay worse than Exact until about frame 119.
3. **Revealed faint wire shadow:** .038 at age 0, .030 at ages 1–4 and .018–.020 at ages 8–16, against Exact's .0078 (Interactive .067).
4. **Moving-plate changed region:** .0126 against Exact's .0094. Whole-cell invalidation gives .0154. Per-ray invalidation still re-traces about 110 k stored rays per moving frame, because most rays inside the mover's shadow meet it.
5. **Solid leakage.** On lit pixels the maximum error is .059 (4,825 of 1.27 M pixel-frames above .002); on umbra .081 (440 of 4,200 above .002). Cells straddling the footprint edge cause it, and Exact is exact on solids.
6. **Flicker** while converging is .0025 RMS (frames 33–39), against 0 for Exact; it is .0003 once converged.

C0 equals Exact statically on this planar scene, which has no micro-structure, and degrades once anything moves. The fixture is the unfavourable case for an area cache: Exact's film error here is only .016. On the quarry the expected gap is about .2 (I). The F1 design must therefore add:

- a budget set by the T_q Islam chooses (I: 8 rays per pixel for young or faint cells halves convergence time);
- 2×2 sub-cell display values, or position-aware reads of partial cells, against leakage;
- the static exact-state path as a fallback if T_q cannot otherwise be met.

The image sheet (`images/fixture-sheet.png`) was inspected: rows are frames 39, 55, 66, 79, 119, 127 and 135; columns are film, then ×4 error for Exact, Interactive, C0 and C1. C1 shows fine two-signed grain on the strip penumbra and a faint ring around the moving shadow; no bands or blotches. Exact shows sub-pixel outline aliasing at the contact sliver, wires and moving shadow. Interactive shows heavy grain.

## 6. Level F gate

Level F scores the visible estimand against a film reference, frame by frame, through motion and change. Level A remains the gate for output-preserving performance work: bit-exact, or parity with Exact. Level B is retired as an acceptance gate, because it certified Interactive against a point reference.

### References

- **R_F, film visibility.** Render 256 sub-pixel-jittered frames (16×16 stratified offsets, box filter) of the existing traced pipeline with Exact visibility and independent seeds, accumulating per pixel only the samples that land on the pixel-centre surface (same instance, depth within 2%). That is 65,536 rays per pixel, about 2.0–2.8 s per frame at the measured Exact rates (7.6–10.9 ms per 256 rays).
  - Two receiver policies: the renderer's, and a minimal-epsilon policy (1e-5 m offset and t_min).
  - Split-half seeds give the reference's own per-pixel noise σ_F. Scores report raw error and error with σ_F² subtracted.
- **R_L, film image.** The same jittered frames, shaded and averaged in HDR. This is a 256-spp box-filtered image of the renderer's own shading, used for the final-image inspection at the registered and +4-stop exposures. Geometric anti-aliasing remains a separate open item at 1x. R_L shows it and does not hide it.
- **Fixtures.** The analytic disk oracle integrated over each footprint, as in P3. The GPU R_F must match it within σ_F on both fixtures before any quarry score counts.

### Sequences

Each frame has its own reference. Frame counts are at a nominal 60 Hz.

| Id | Content | Exercises |
|---|---|---|
| Q-static | Pose 0, 128 frames from cold | Convergence, flicker, time-to-quality |
| Q-walk | 64 frames of 1 cm steps with 0.05°/frame yaw, 32 hold, then the reverse path | Motion, return without bit-identity |
| Q-cut | Cut between registered poses, then 32 frames | Cold start |
| Q-disocclusion | 48-frame path past the central slab, revealing hidden ground | First-seen surfaces |
| Q-zoom | Dolly-in at 2%/frame for 32 frames | LOD transitions |
| Q-blocker | One placed rock moving 5 mm/frame for 32 frames, then 32 hold | Moving shadow, ground revealed under the mover |
| Q-sun | 0.25°/s for 128 frames, 10°/s for 32, a 5° jump, then 32 | Moving light, lag, reset |

Both GPU fixtures run the static, pan, cut, blocker and sun structure against the analytic film oracle. Unreal motion comparisons need new native captures of these sequences. Until then, U-checks use the rescored static native masks only.

### Populations and checks

Populations are defined from R_F per frame:

- the existing partial, faint, narrow, wide, isolated, silhouette, contact, lit and umbra;
- **point-aliased:** the pixel centre is solid but R_F is partial;
- **micro:** in-footprint standard deviation above 0.1;
- **revealed:** frames since a pixel's surface was first seen;
- **changed:** |R_F,t − R_F,t−8| > 0.05.

E denotes Exact on the same frame and seed against the same R_F. s is Exact's measured 2-s.d. seed spread for that statistic.

| Check | Rule | Catches |
|---|---|---|
| F1 converged quality | On pixels ≥ 32 frames past their last event, per-population RMS ≤ E·(1+s). Lit and umbra, where E is exactly 0, use Level A's absolute solid bound: maximum error ≤ 0.002. On static poses, also ≤ the better Unreal native configuration | Below-Exact steady images, leakage onto solids |
| F2 transient floor | From age 0 (first seen, cut or invalidated), RMS ≤ Interactive's at every age, and ≤ E·(1+s) by age T_q. Proposed T_q: 8 frames for partial, narrow and wide; 16 for faint and revealed. **Islam decides T_q**, since it is a judgment on visible transients | Slow recovery after cuts and disocclusion |
| F3 bias | \|mean\| ≤ \|E mean\| + 4 SE on every population, every frame | Lag bias, systematic loss, cell leakage |
| F4 missing and false shadows | Per frame, pixels with R_F ≤ 0.9 shown ≥ 0.99, and R_F ≥ 0.99 shown ≤ 0.9, persisting ≥ 2 frames: ≤ E's count. Enumerated with crops | The USS and 64-detect failure class |
| F5 lag and ghosting | On changed pixels: lag (−8β, β from regressing e_t on R_F,t − R_F,t−8) ≤ 0.5 frame. \|ghost bias\| ≤ \|E's\| + 2 SE, where ghost bias = mean(e·sign(R_F,t−8 − R_F,t)) over pixels R_F moved by > 0.25; point aliasing gives E its own signed value (fixture: −.0036). Changed-pixel RMS ≤ E·(1+s) by T_q after motion stops. On a slow sun the lag regression is noise-dominated for every estimator (fixture), so use the sun-speed sweep and inspection there | Trailing shadows, stale history |
| F6 stability | Still camera with unchanged R_F, converged pixels: RMS(c_t − c_t−1) ≤ max(E's, 1/256). Co-located motion residual ≤ E's | Flicker, swimming |
| F7 structure | Contour ρ against R_F ≤ the Level-B S4 bound computed with E. Error power at the cell-grid frequency ≤ E's + 2 SE | Bands, blocks, grid imprint |
| F8 determinism | Replaying the same input sequence (camera, light, movers, order) is bit-identical; independent of frame timing | Replaces S5's return identity, which no history-carrying method can meet |
| F9 cost | p50 and p99 per sequence, including cut and invalidation frames. No frame above 1.5x the steady budget. Memory reported | The 138 ms reset class |
| F10 inspection | R_F (both policies), Exact, Interactive and candidate visibility; lit at registered and +4 stops; ×8 error maps against R_F on one scale; flip-books of every sequence. Frozen crops: the seven contact and self-shadow pixels, silhouettes, rubble, crevices, cliff-top partials, revealed regions, the mover's shadow, and the fixture strip, sliver, specks and wires | Defect classes the numbers miss; numbers do not override it |

Thresholds are frozen at the end of F0 from measured Exact spreads. A failed check stops the candidate. Changing a configuration makes a new candidate.

## 7. First unit and stop rules

**F0: film reference and rescoring.** GPU, test-only, no production change; Islam's foreground-game rule applies.

- Add a test-only jittered-primary accumulation mode with both receiver policies. Run it on quarry poses 0, 1 and 2 and on both fixtures, first validating against the analytic film oracle.
- Rescore Exact, Interactive, VSM and HRT (native masks) on R_F, with per-population tables, an enumeration of missing film-visible shadows, and inspection.
- Expected verification cost: one test build plus about 3 poses × 2 policies × 2–2.8 s of reference rendering, and the fixtures.
- **Decision rules, frozen here:**
  - Exact's partial RMS against R_F (policy) ≥ 0.05, about 7x its point noise: the estimand finding holds; proceed to F1.
  - Below 0.02: the finding is wrong. Exact stays the default and the visible bar. Record that soft shadows at Exact quality cost 8.4–10.9 ms on this hardware, and stop.
  - Between 0.02 and 0.05: recompute F1's convergence target and ray budget from the measured gap before building.
  - If the two policy references differ by more than Exact's error on any population, the 1 mm receiver policy is itself a visible defect. Open a receiver-policy unit first, because it changes every mode's estimand.

**F1: SVC with a static light**, opt-in `SoftShadows::Cached`, default unchanged.

- **File ownership:** new `src/visibility_cache.rs` and `shaders/visibility_cache.wgsl` (hash, lattice, display table). Fused-pass changes are confined to `shaders/primary_trace.wgsl` and `src/primary_ray_gpu.rs`. Exact and Interactive output must stay byte-identical.
- **Order:**
  1. A CPU model replay equals the GPU cache state for a short sequence.
  2. Q-static, Q-walk, Q-cut, Q-disocclusion and Q-blocker, plus fixtures, under F1–F10.
  3. Paired timing: 3 processes × 24 pairs against Interactive and Exact.
- **Stop rules:**
  - Any F4 excess over Exact, on any frame of any sequence.
  - Any F1 population failing on the quarry.
  - F2 not crossing Exact by T_q.
  - A stored ray outcome differing from a fresh trace in validation replay (fix the implementation, never the replay).
  - p50 above 1.075 ms or any frame above 1.6 ms (cut frames included): record, with no quality trade.
  - Memory above 128 MB.

**F2: sliding sun** on Q-sun. Stop if the 10°/s p50 exceeds 1.2 ms, or any F5 lag exceeds 0.5 frame. The default changes only after F1 and F2 pass and Islam accepts. Exact stays selectable.

## 8. Rejected options

| Option | Reason |
|---|---|
| Strict Level A per frame with fewer rays | Rate bound and no exact certifier ([§1](#1-strict-level-a-at-1-ms)) |
| Adaptive allocation from prefixes or own samples | Biased; missed faint shadows on the GPU |
| Software cone-BVH certification | About 1.3 GB extra memory; near-field cost dominates; unmeasured |
| Shadow-map bulk umbra with exact rays | Saves ≤ 8%; near field unresolved |
| Analytic disk-against-triangle | Blocker discovery and micro-triangle unions |
| Importance or better stratified sampling alone | Rate-optimal already; history strata are caching |
| Pixel-centre world cache (C0) | No reuse on rough scans; ghosts against the point reference (P2, M) |
| Screen-space reprojected history | 72% resets; policy |
| Cross-pixel denoisers and filters | Measured worse; bias across pixels |
| Lag-bounded decay or reset for a moving sun | Lag or permanent grain ([§5](#5-moving-sun-and-moving-blockers)) |
| Whole-cell invalidation for movers | Fresh-floor quality over the whole reach box (P3) |
| Interactive as default | Visible grain; below Exact on the point and proxy measures |

## 9. Open risks

- **The estimand finding rests on CPU proxies.** It comes from two pixel lattices and a nearest-neighbour proxy. F0 decides, and the receiver policy's share is unknown.
- **Smooth content converges slowly.** On the planar fixture at 4 rays per pixel per frame, C1 has not crossed Exact after 40 static frames, and its faint population takes about 120 frames. The proposed quarry base rate is 2 rays per pixel, so smooth surfaces would take longer. Fallbacks: raise the budget for young and faint cells (I: +0.03–0.06 ms), or layer the static exact-state path (bit-exact Exact by frame 32).
- **Cell leakage** onto solids reaches a maximum error of .059 (lit) and .081 (umbra) in P3. Sub-cell display values or position-aware partial-cell reads are needed to meet the 0.002 solid bound.
- **Cache structure:** hash collisions and eviction for large views; thin two-sided features inside one cell; cold transients at LOD changes.
- **Dense movers** (vegetation, characters). Per-ray invalidation scales with their shadow area. Many movers push the fresh floor up toward Interactive quality there.
- **Unmeasured costs:** jittered primary cost, atomics, and display reads near L2 capacity.
- **Multiple lights** need per-light caches. The cache's visibility scalar is per light per pixel ([§4](#4-recommended-surface-visibility-cache) constraint).
- **Scope:** one quarry view; Unreal motion comparisons need new captures; 4x, transparency and virtual geometry are out of scope.
- **Geometric anti-aliasing.** The film image also differs from any 1x renderer at silhouettes. That is a separate work item; R_L makes it visible.

## Evidence, reproduction and time

Evidence root: `D:/Coding/renderlab-work/m6-exact-quality/`. `identities.sha256` hashes the scripts, outputs and images. Inputs are read in place, unchanged:

- `m6-level-b-probe/gate-final/reference/pose-{0,1,2}-reference.npz`: the traced 16,384-ray reference;
- `m6-level-b-probe/runs/quarry-b6/`: traced receivers and real GPU 256-bit hit masks, seeds 0–3.

| Script | Purpose | CPU time |
|---|---|---:|
| `scripts/quarry_estimand.py` | P1: in-footprint pair differences | 1.4 s |
| `scripts/quarry_cache.py` | P2: footprint proxy, Exact and Interactive baselines, pixel-centre world cache at 4 settings | 20.5 s |
| `scripts/quarry_shimmer.py` | Screen-locked change at the 1 cm step; dominated by parallax, so not used for conclusions | 8.1 s |
| `scripts/fixture_temporal.py` | P3: exact planar fixture, film and point references, 136 frames, five estimators | 232.3 s (oracle 61.5 s); whole-cell variant 275.9 s |
| `scripts/fixture_metrics.py`, `fixture_followup.py` | Proposed Level-F metrics, the image sheet, convergence, flicker and solid maxima | 0.8 s, 0.3 s |

Run each script as `py -3 -I scripts/<name>.py` from the evidence root with two math-library threads. Pass `--whole-cell` to `fixture_temporal.py`, and `-wholecell` to the metrics script, for the whole-cell invalidation variant.

Corrections made during the analysis, all before results were used:

- the first P3 run crashed on an array-shape error;
- the second had a C1 maintenance bug (partial layers were topped up as if full) and a grid-aligned display;
- its lag and ghost metrics were noise-dominated: even the stateless Exact path showed "lag".

The superseded outputs are retained as `out/fixture_metrics-v2-wholecell-nolayerflag.log`.

There was no GPU run, window, build or Unreal launch, and no asset was read beyond the saved arrays. Design and writing overlapped the analysis and were not timed separately.
