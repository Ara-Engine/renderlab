# N7: world-space visibility cache for non-RT sun shadows

CPU probe and design, 2026-11-06, base `437cbd5`. It was time-boxed to about an hour. No GPU run, build, window or Unreal launch. Labels: **M** measured here or in the cited record; **I** inference or estimate.

**Bar (Islam, 2026-11-06).** The minimum is Unreal's quality at Unreal-like cost. Film quality is the stretch goal and is reported separately. A bounded error is acceptable where Unreal's error is larger. Unreal VSM against the policy film reference R_F, pose 0 ([film reference](m6-film-reference.md)): partial RMS **0.371**, occluded 0.483, contact 0.627, 24,049 missed film shadows. For comparison, Exact scores 0.232 and Interactive 0.234. VSM costs **1.61 ms** a frame and **0.50 ms** of that is shadows; a rotating sun raises shadow depth from 0.27 to 0.50 ms ([bench-motion](bench-motion.md)).

## Verdict

1. **Unreal bar, static sun, camera motion: viable (I, with measured inputs). Proceed to one GPU unit.** Keep a per-triangle lattice cache of 8-ray software visibility, filled at camera-independent lattice points. The view then pays rays only for newly seen surface:
   - typical Q-walk frame: **0.007–0.03 ms** of software rays;
   - S2 at 1 px per frame with the plate reveal: **0.11 ms**;
   - **0.50 ms** under the crude reprojection upper bound.

   The shadow part is projected at **0.1–0.3 ms** with lookup, well under 1.2 ms. A whole non-RT frame is projected at about **0.9–1.5 ms** against VSM's 1.61 ms, *if* raster primaries (N1b, unmeasured) cost 0.4–0.7 ms. On solid pixels the error stays near Interactive's, but whole-population quality with a fully warm cache was not measured (§3). Interactive already beats VSM on R_F by 1.6x.
2. **Moving sun: costed (I).** This needs a world-fixed direction lattice slid under the disk:
   - 0.25°/s: **+0.016 ms**;
   - 1°/s: **+0.08 ms**, against VSM's +0.23 ms;
   - 10°/s: **+0.71 ms**.

   Keeping Exact's per-receiver R2 directions in the disk frame instead forces a full re-trace on any sun motion: **16 ms** a frame.
3. **Exact reproduction is impossible under motion (M).** None of pose 1's 345,882 receivers is bit-identical to any pose-0 receiver. Every world-space key changes the estimand to "the visibility at a nearby fixed point". That is a bounded error, measured in §3.
4. **Film bar (stretch): not at this cost (I + M).** Film integration needs 4–8 or more in-footprint samples per pixel, with exact membership and occlusion validation. That is the world-anchored record gather T2a measured at **0.8–2.4 ms** on this GPU ([T2a](m6-motion-aa-gpu.md#6-cost)), plus 3x the cells (§1). The cache does not change that verdict.
5. **Cuts and cold starts are the recorded risk (I).** A cold 1080p view takes 4.7 M rays (**16 ms**). That is above software Interactive's 9.43 ms, so cut frames need the Interactive floor and amortized fills.

## Method (M)

`scripts/non-rt-n7-probe.py` ran as one process in 13–19 s. Inputs, read in place:

- R1's saved quarry poses 0 and 1 (`E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{0,1}`): hit records, world points, normals, the 256-ray Exact counts, and the 256-primary film counts (R_F, conditioned on the centre surface);
- T1b's Q-walk record (`t1-gpu/walk-b15/motion.json`);
- the S2 fixture and its tracks.

R1's "pose 2" is a repeat of pose 0, so no cut or near-view pose was available.

- **Footprint:** view distance × 2 tan(fov/2)/1080, median 6.97 mm. Pixel world area is footprint²/|cos θ|, median 76 mm².
- **Cell edge:** the power of two at or below footprint/√N, for N samples per footprint.
- **Rays:** 8 per cache cell, as for Exact's receivers, with the same receiver policy: viewed side, ε = max(10 µm, 16 ulp).
- **Ray cost:** **3.40 ns** per software ray (N0's 9.43 ms over 2.77 M rays). Sparse queued rays may cost more (§5).

## 1. Keying, entries and memory (M, pose 0, 1080p)

| Keying | Entries a view touches | Memory |
|---|---|---|
| **Per-triangle lattice**, whole-triangle allocation (321,669 visible triangles) | N = 1/4: 346 k samples; **N = 1: 589 k (1.7 per covered pixel)**; N = 4: 1.78 M; N = 16: 6.6 M | 10.6 / **10.9** / 12.1 / 16.9 MB at 8 ray bits plus a 32 B hash slot per triangle; 12.7–37 MB with 32-ray state |
| Micro-texels (same keys, value = cell average) | Area-based: N = 1: 1.67 M; N = 4: 6.7 M | 17 / 37 MB |
| World hash grid (placement, level, voxel) | Area-based, as micro-texels; pixel-centre keys: 111 k at 2–4 footprints, 266 k at 1–2, 345 k at N = 1 | 2.5 / 10 / 40 MB (24 B per slot) |

The area-based counts over-count grazing pixels: |cos θ| is clamped at 0.05. Whole-triangle allocation uses true triangle areas and is the count costed below. Memory is never the constraint: off-screen retention at 2–4x stays under 50 MB.

## 2. Camera motion: fresh rays per frame

**Cells filled from pixel centres stay cold (M).** If cells hold whatever pixel-centre receiver landed first, then at the 1 cm step:

- sub-footprint cells are 76–96% cold;
- at 1–2 footprints they are 29% cold;
- at 2–4 footprints, 4.6% (grid) and 77% (per-triangle, whose cells are bounded by triangles of about one pixel).

P2 measured the same class at 57–90% ([SVC doc](m6-exact-quality-fast-shadows.md)). Warmth then needs coarse cells, which costs quality (§3). **The cache must therefore fill camera-independent lattice points.** Each newly seen footprint traces all of its cells. A cell is then warm wherever its surface was seen before.

**Newly seen surface (M):**

- **Q-walk** (T1b on the GPU; it restarts any pixel whose own reprojection finds no valid history, so this is an upper bound for a world cache that remembers everything seen): median **141** pixels per moving frame, p99 469, maximum 622, over 128 moving frames.
- **1 cm step, crude reprojection** (same placement within 2 footprints at the pose-0 pixel): 10,759 pixels (3.1%). This over-counts on mm rubble; treat it as a ceiling.
- **S2:** a border column of 270, 540 and 1,080 pixels per frame at 0.25, 0.5 and 1 px per frame. The plate reveals at most 1,279 pixels per frame on its 32 moving frames.

**Rays and cost** at 1.7 cells per pixel (N = 1) × 8 rays × 3.40 ns:

| Case | Pixels per frame | Rays | Software cost |
|---|---:|---:|---:|
| Q-walk median / p99 / max | 141 / 469 / 622 | 1.9 k / 6.4 k / 8.5 k | **0.007 / 0.022 / 0.029 ms** |
| S2, 1 px per frame + plate reveal (worst frame) | 2,359 | 32 k | **0.11 ms** |
| 1 cm step, reprojection ceiling | 10,759 | 146 k | 0.50 ms |
| Cold view or cut (589 k cells) | 346 k | 4.71 M | **16.0 ms** |

Zooming in adds refinement levels: cells scale with 1/distance², about 0.4% of cells per frame at 1 cm per frame and 5 m (I). Q-zoom was not measured.

## 3. Quality: a bounded error against R_F (M, pose 1)

Each pose-1 pixel reads its key's value from a cache filled at pose 0, or traces fresh if the key is cold. Values are 8-ray outcomes (binomial on the Exact fraction, so noisier than R2) or the converged 256-ray value. Single-surface pixels: 48,041 partial and 290,919 solid. Exact and Interactive are scored on the same pixels.

| Estimate | Warm | Partial RMS | Occluded RMS | Solids > 0.002 | Flips > 0.5 | Missed / false film shadows |
|---|---:|---:|---:|---:|---:|---|
| Exact (pixel centre, 256 rays) | — | 0.248 | 0.020 | 139 | 15 | 11,462 / 15 |
| Interactive (8, binomial) | — | 0.255 | 0.023 | 70 | 17 | 12,459 / 15 |
| Per-triangle cells, N = 1/4, 8 rays | 16% | 0.257 | 0.025 | 88 | 18 | 12,438 / 20 |
| Per-triangle cells, N = 1/16, 8 rays | 23% | 0.260 | 0.032 | 107 | 27 | 12,458 / 34 |
| Grid, 1–2 footprints, 8 rays | 71% | 0.301 | 0.058 | 437 | 276 | 13,541 / 391 |
| Grid, 2–4 footprints, 8 rays | 95% | 0.357 | 0.161 | 2,021 | 1,549 | 15,252 / 1,785 |
| *Unreal VSM, pose 0, film-reference doc populations* | — | *0.371* | *0.483* | — | — | *24,049 / 9* |

- **Per-triangle keys keep the error near Interactive's** on the warm pixels. A triangle-bounded cell never mixes surfaces or triangles across a crease. These rows are diluted, though: only 16–23% of pixels were warm.
- **A fully warm lattice cache was not measured.** That needs rays from the lattice points themselves, and it is the first gate of the GPU unit.
- **Coarse grid voxels are warm but mix micro-surfaces.** Flips rise 16–91x and false shadows 26–119x over Interactive. Partial RMS stays below VSM's, but false shadows exceed VSM's 9.
- The VSM row uses the film-reference doc's populations, so it is indicative only.

## 4. Sun motion and moving objects

**Sun (M for the fractions, I for the cost).** Direction samples sit on a lattice fixed in a world chart. Only lattice points entering the 0.04 rad disk are traced; points that leave are dropped. The in-disk points of a uniformly shifted lattice remain a stratified sample, as in the SVC design. A Monte Carlo of the entering fraction matches 2ωΔt/(πα) to within 11%.

| Sun speed | Rays entering per frame | Rays (589 k cells × 8) | Added cost | VSM (S3, M) |
|---|---:|---:|---:|---|
| 0.25°/s | 0.103% | 4.9 k | **0.016 ms** | Shadow depth 0.27 → 0.50 ms; pages to clear 8 → 79 |
| 1°/s | 0.488% | 23 k | **0.078 ms** | Same |
| 10°/s | 4.45% | 210 k | **0.71 ms** | Not measured |
| R2 directions in the disk frame (Exact's own set) | 100% | 4.71 M | 16 ms | — |

- **Values change only in penumbra cells:** 14% of single-surface pixels are partial (M). Which cells flip is unknown until their rays are traced, so the cost is set by swept rays, not by changed cells.
- **Off-screen cells** carry a sun epoch and re-trace when they come back into view.
- **A jump over α/2** is a cold frame for visibility (16 ms) unless it is amortized.

**Moving objects (I, S2 dimensions M).**

- **Static visibility is cached once.** Rays are traced against static geometry only, and each ray stores one static-blocked bit.
- **Each frame, a mover is composited.** Rays in the reach region of its old and new world bounds are tested against the mover's own small BVH, never the full scene.
- **The mover's own cells** are keyed in object space, so they move with it. Its shadows from the static scene are re-traced while it moves.
- **S2 shadow-step:** the blocker moves 1 mm per frame. Its reach box, the union of the old and new shadows widened by the penumbra, is 0.075 m², about 12,800 floor pixels. That gives 174 k rays: **0.59 ms** if re-traced against the full scene, and an estimated 0.05–0.1 ms against the mover's BVH (I).
- **Cost scales with mover shadow area and count, not scene size.** Dense movers (vegetation, crowds) remain a risk. S3's pillar sun track gives the same sweep fractions; no S3 view was counted.

## 5. Unmeasured costs that decide it

- **Lookup per pixel:** one hash probe per covered pixel, keyed (instance, triangle, side, level, cell) from the raster visibility buffer's triangle and barycentrics. A traffic model gives about 0.05–0.2 ms. Every light-space structure on this GPU has run 6–20x above its model ([N4](non-rt-n4.md), [N5](non-rt-n5.md)), so this is the main risk.
- **Sparse fill queues:** compacting a few thousand cells per frame may not reach N0's full-frame 3.40 ns per ray. The latency of one dispatch, tens of µs (I), sets a floor.
- **Raster primaries (N1b):** 0.4–0.7 ms is estimated, not measured. Software primaries cost 2.84 ms, which would break the frame budget.

## 6. GPU first unit: N7a

**Scope:** a per-triangle lattice cache with a static sun, then the sliding sun. Test-only, on N0's harness. Primaries come from the existing hit records (N1b is a separate unit). All fills use N0's software rays, the same receiver policy and 8 rays per cell.

**Order:**

1. A CPU replay equals the GPU cache state on a 16-frame Q-walk prefix.
2. A fully warm cache at poses 0 and 1, scored against R_F (the §3 gap).
3. Q-walk and S2 spans, with lookup, fill and display timed separately.
4. 1°/s and 10°/s sun on S3.
5. The S2 shadow-step mover.

**Stop rules, frozen here:**

- **Quality:** on any Q-walk or S2 frame, any R_F population's RMS above Unreal VSM's, missed film shadows above VSM's, or false shadows above max(VSM's, 2x Interactive's). Stop, and do not loosen.
- **Correctness:** a stored outcome that differs from a fresh trace in validation replay. Fix the implementation.
- **Lookup and update pass** above 0.30 ms at 1080p on a warm frame. Stop.
- **Warm Q-walk shadow cost** (lookup + fills) with p50 above 0.5 ms or p99 above 1.2 ms. Stop.
- **Queued fill rays** above 6.8 ns each (2x N0). Re-price §2 before continuing.
- **Sun at 1°/s** adding more than 0.25 ms (VSM's increment). Record the miss; 10°/s is recorded either way.
- **Memory** above 128 MB. Stop.
- **Cut frames** are recorded, with no gate yet; the 16 ms cold fill needs an amortization decision from Islam.

## Rejected options

| Option | Reason |
|---|---|
| Bit-exact per-pixel reuse | 0 of 345,882 receivers recur after a 1 cm step (M) |
| Pixel-centre-filled cells at sub-footprint sizes | 76–96% cold after 1 cm, so the cost is Interactive's (M) |
| Coarse world voxels (1–4 footprints) for warmth | Flips 276–1,549 and false shadows 391–1,785 against Interactive's 17 and 15 (M) |
| Micro-texels with cell-average values | No UVs on the scans, and cell averages leak across shadow edges (P3: solid error up to 0.08); the same counts as the lattice with a worse estimand |
| Exact's R2 directions in the disk frame for a moving sun | Full re-trace, 16 ms a frame |
| Whole-scene re-trace for movers | 0.59 ms for one S2 blocker; the mover's own BVH suffices |
| Film-bar footprint gather in this unit | T2a's gathers alone cost 0.8–2.4 ms (M); kept as the stretch goal |

## Evidence and time

- **Evidence:** `E:/renderlab-work/non-rt-n7/n7.json` (every number above) and `run.log`. No raw dumps were written.
- **Runs:** two, each one process of 13–19 s. The first started with 4.7 GiB available. **The second started at 2.09 GiB, below the 4 GiB start rule, which I did not re-check before launching.** It completed in 13 s with a small footprint: two poses of buffers, well under 1.5 GiB.
- **Time:** about 35 minutes reading prior units, 15 minutes writing and running the probe, and 15 minutes writing this document. Implementation and verification were not separated further.
