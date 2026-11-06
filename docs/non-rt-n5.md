# N5: near/far split for non-RT sun shadows

CPU probe, 2026-11-06 UTC, base `5a0438e`. It follows N0's and N4's cost stops ([non-rt-n0.md](non-rt-n0.md), [non-rt-n4.md](non-rt-n4.md)) under [non-rt-path-2.md](non-rt-path-2.md). It is CPU only: no GPU run, build, window or Unreal launch. Labels: **M** measured here or in the cited record; **I** inference or estimate.

## Result

**Verdict: no.** The split can be made exact, but it does not project to 1.2 ms or less on this GPU. No GPU probe plan is written. Non-RT film-quality shadows stay a quality-first mode at their measured cost: 8.08 ms (LSG) to 9.43 ms (N0) for the 8 Interactive rays.

- **Exactness: achievable (M).**
  - **Near segment.** A spatial list (every triangle whose box lies within r of the receiver's triangle) captures every near blocker by construction. A vertex 1-ring (14.2 triangles) has 0 lost shadows at r = 2 mm on this sample, but no ring is lossless at r ≥ 5 mm.
  - **Far segment.** A conservative map with two proofs (certainly lit, certainly blocked) had 0 lost far shadows and 0 false proofs, in all 20 (r, texel) settings × 573,120 rays. Rays it cannot decide go to an exact ray.
- **Zero far errors from a map alone: none at any texel or bias (M).** At r = 25 mm and 0.5 mm texels, the best setting, the undecided rays are 2.6% of all rays. Scoring them as shadowed gives 1.2% false shadows overall (25% of penumbra rays); scoring them as lit loses 1.4%.
- **The split moves the cost instead of removing it (M).**
  - Small r keeps the near list small: the 1-ring at 2 mm. But the far map then leaves 12-20% of rays undecided, mostly penumbra and grazing receivers.
  - Large r cuts that to 2.5-4%. But the spatial list grows from 21 triangles (2 mm) through 41 (5 mm) and 93 (10 mm) to 377 (25 mm). Rings stop capturing, because overlapping scan placements put near blockers in other meshes: 9 hits at 2 mm, 320 at 25 mm.
- **Projected GPU cost (I).**
  - At the unit costs N0 and N4 measured on this GPU: **5.3-15 ms**, best 5.3 ms (r = 2 mm, 1-ring, 1 mm map).
  - Under a throughput model: 1.16 ms at best (r = 5 mm, spatial list, 0.5 mm map), for about 3.5 GB of extra memory: 1.3 GB of map and 2.2 GB of per-triangle lists. N4 already measured a similar light-space structure at about 6x its traffic model.
  - Exact refinement alone, at the measured cost of a software ray, takes 1.0-1.9 ms in the compact 1-ring settings (r = 2 mm).
  - All of this is against the frozen 1.2 ms Interactive stop and Unreal VSM's 0.50 ms shadow share.

## Method (M)

`scripts/non-rt-n5-probe.py` uses the same 300 random 16x16 pose-0 tiles (seed 20261106) as the path-2 probe: 71,640 R1 viewed receivers with ε = max(10 µm, 16 ulp), from the read-only RLA1 quarry (13,034,903 triangles).

- **Rays.** Each receiver traces the production's first 8 `fresh_direction` indices: R2 with the per-pixel hash shift, slope tan 0.04 and tmin = ε. The disk basis is the probe's light basis, not the shadow view's, so per-pixel directions are rotated relative to production. All comparisons are against the probe's own exact test on the same rays.
- **Exact test.** Two-sided f64 Möller-Trumbore over f32 light-space vertices. A ray is tested against every triangle whose box can meet its cone, and **every** hit is kept. That gives 24.0 M ray tests and 52,904 blocked rays (9.2%).
- **Cross-check.** 38.5% of blocked rays are blocked within 1 mm and 58.8% within 10 mm. Path-2's central rays gave 39% and 59%.
- **Near sets.**
  - Cone set: triangles whose box meets the truncated sun cone [ε, r].
  - Hit set: triangles actually hit within r.
  - Ring capture: vertex k-ring distance from the receiver's triangle to each near hit, over welded source positions within one placement (as the M6 adjacency does), or cross-placement.
  - Own cluster: the receiver triangle's 128-triangle cluster, in Morton order per placement.
  - Spatial lists, sizes measured on 16 receivers per tile (4,800): the per-triangle ball list (boxes within r of the receiver triangle's box) and the per-cluster list (whole clusters within r).
- **Far map.** It is local to each tile's reachable region and conservative:
  - **top:** per texel, the highest point of any candidate triangle over the texel. That is its plane at the texel corners, clamped to its top vertex.
  - **cover:** for the topmost triangle whose projection contains the whole texel, the plane's height range over the texel.
- **Far walk.** The segment (r, ∞) walks every texel its light-space path crosses, with an exact DDA. It is **lit** once the ray rises above every top it crosses. It is **blocked** when it enters a covered column below the cover range and leaves above it: by continuity it crosses that triangle. Otherwise it is **unsure**, and an exact ray is needed. A coarse 8x8 max level is also walked, to price pyramid skipping.
- **Map memory.** `scripts/non-rt-n5-footprint.py` measures the visible surface in the light plane from the saved pose-0 world buffer, and the 128² pages it touches, with one ring of dilation.

## 1. Near segment (M)

Per receiver; mean, with p99 in parentheses. "Near-blocked" counts rays blocked within r. "Lost" counts rays that end up wrong when the far segment is exact. A near miss can be masked by a later far hit: a two-sided ray that enters a surface also hits its far side.

| r | Cone set | Hit set | Near-blocked rays | Cross-placement near hits | 1-ring: near missed / lost | 3-ring | Own cluster | Ball list | Cluster list |
|---|---:|---:|---:|---:|---|---|---|---:|---:|
| 2 mm | 2.5 (6) | 0.06 (2) | 22,703 | 9 | **87 / 0** | 17 / 0 | 736 / 15 | 21 (45) | 3,271 (22,016) |
| 5 mm | 2.8 (8) | 0.10 (2) | 26,899 | 41 | 1,165 / 22 | 56 / 2 | 1,765 / 123 | 41 (119) | 3,492 (22,784) |
| 10 mm | 3.0 (12) | 0.15 (4) | 31,121 | 70 | 3,807 / 270 | 658 / 31 | 3,375 / 435 | 93 (313) | 3,893 (24,448) |
| 25 mm | 3.6 (25) | 0.28 (6) | 38,103 | 320 | 10,364 / 2,189 | 4,107 / 543 | 7,845 / 1,764 | 377 (1,447) | 5,306 (31,872) |

- **Ring sizes** (mean, p99): ring 1 has 14.2 (19) triangles, ring 2 has 42.9 (60) and ring 3 has 89 (125).
- **Lost rays by population.** Lost rays come only from receivers in penumbra (2,102 receivers) and umbra (5,530). Fully lit receivers lose nothing.
- **Rings beyond 3.** At r ≥ 5 mm the residual lost rays never reach zero; at 5 mm, rings 3-6 all leave 2. The cause is cross-placement blockers, which topology cannot reach. The lossless 1-ring at r = 2 mm is a sample result, not a guarantee: its 9 cross-placement hits were all masked by far hits on the same rays.
- **Cluster lists are not usable.** They are 3,000-5,000 triangles on average, up to 295,000, because large triangles inflate cluster boxes.
- **Ball lists capture by construction**, but their memory over all triangles is 1.1 GB at 2 mm and 2.2 GB at 5 mm (I: the mean on receivers × 13.03 M × 4 B). Strictly, the radius should be r + ε; ε is at most 0.49 mm, on placement 11.

## 2. Far segment (M)

Rays needing a far answer are those not blocked within r. Every row has **0 lost far shadows and 0 false blocked proofs**. That covers the coarse level too, so the conservative proofs held on every ray.

| r | Texel | Far-blocked rays | Proved blocked by cover | Undecided (refine), % of all rays | Without cover proofs | Coarse level only | Cells walked, mean (p99 when lit) | Map, MiB (top + cover) |
|---|---|---:|---:|---:|---:|---:|---|---:|
| 2 mm | 0.5 mm | 30,201 | 13,215 | **12.5%** | 14.8% | 59.1% | 12.1 (56) | 1,274 |
| 2 mm | 1 mm | 30,201 | 6,953 | 20.0% | 21.3% | 80.1% | 7.1 (28) | 354 |
| 2 mm | 2 mm | 30,201 | 2,229 | 37.0% | 37.4% | 90.3% | 4.6 (14) | 105 |
| 5 mm | 0.5 mm | 26,005 | 11,892 | **6.3%** | 8.4% | 25.9% | 11.9 (58) | 1,274 |
| 5 mm | 1 mm | 26,005 | 6,442 | 8.8% | 9.9% | 47.7% | 7.0 (30) | 354 |
| 10 mm | 0.5 mm | 21,783 | 10,207 | 4.2% | 6.0% | 11.9% | 11.5 (59) | 1,274 |
| 10 mm | 1 mm | 21,783 | 5,661 | 5.6% | 6.5% | 23.6% | 6.8 (30) | 354 |
| 25 mm | 0.5 mm | 14,801 | 6,890 | **2.6%** | 3.8% | 5.1% | 10.5 (58) | 1,274 |
| 25 mm | 1 mm | 14,801 | 3,900 | 3.2% | 3.9% | 7.8% | 6.3 (30) | 354 |
| 25 mm | 8 mm | 14,801 | 16 | 7.9% | 7.9% | 59.9% | 2.5 (19) | 13 |

- **Cover proofs** decide 44-47% of far-blocked rays at 0.5 mm and 23-27% at 1 mm. They almost vanish at 4 mm and above (about 1%), where a triangle rarely covers a whole texel.
- **The coarse level does not substitute for the fine walk.** Walked alone at 8x the texel, it leaves 5-96% of rays undecided.
- **Memory.** The visible surface covers 18.8 m² of the light plane (29.1 m² in world). Pages touched, with one ring of dilation: 6,796 at 0.5 mm, 1,888 at 1 mm, 562 at 2 mm. The top map alone is 425 / 118 / 35 MiB; with cover it is 1,274 / 354 / 105 MiB, in f32. Unreal's 1 texel per pixel sits at 5-10 mm here ([non-rt-path-2.md](non-rt-path-2.md#1-quarry-probe-m)).

**A map-only hybrid** decides the undecided rays one way, without refinement. Error rates are per ray, by receiver population:

| r, texel | Undecided as blocked: false, all / penumbra / grazing | Undecided as lit: lost, all / penumbra / umbra |
|---|---|---|
| 5 mm, 0.5 mm | 3.8% / 38.2% / 22.4% | 2.5% / 24.8% / 22.5% |
| 5 mm, 1 mm | 5.4% / 40.7% / 32.4% | 3.4% / 31.9% / 32.1% |
| 25 mm, 0.5 mm | 1.2% / 24.7% / 5.7% | 1.4% / 16.0% / 11.8% |
| 25 mm, 1 mm | 1.3% / 26.4% / 6.3% | 1.9% / 21.3% / 16.6% |

Grazing means 0 < n·l < 0.26 (6,234 receivers). The errors concentrate where the film gate is strictest: penumbrae and grazing receivers. **No texel and bias reach zero far errors.** A bias would only move undecided rays between these two columns.

## 3. GPU cost estimate (I)

The frame has 346,517 receivers and 2,772,136 rays. Each component is priced twice: at unit costs measured on this RTX 4070 SUPER, and at an optimistic throughput bound.

| Unit | Measured on this GPU | Optimistic bound |
|---|---|---|
| **Near packet:** one triangle fetched for one receiver, tested against its 8 rays | 0.387 ns. N0's triangle test with fetch: 4.95 ms over 12.79 M tests (M). The 7 extra tests on fetched data are taken as free | 0.03 ns: 48 B from L2 at about 2.5 TB/s, plus 8 tests at 30% of 35 TFLOPS |
| **Walk cell:** one texel step | 0.079 ns. N4's per step or entry cost, after its triangle tests at N0's rate (M) | 7 ps. Unreal SMRT: 0.155 ms for 8 rays × 8 samples (M), over Renderlab's 346,517 covered pixels |
| **Refine ray:** exact software ray from t = r | 3.40 ns. N0: 9.43 ms / 2.77 M rays (M) | 2.91 ns. LSG: 8.08 ms / 2.77 M (M) |

Projected milliseconds for the five best settings. Static sun only: a moving sun adds a map rebuild, which is not priced.

| Setting | Optimistic: near + walk + refine = total | Measured units: total | Extra memory |
|---|---|---:|---|
| r 5 mm, ball list (41.5), 0.5 mm | 0.43 + 0.22 + 0.51 = **1.16** | 8.63 | 1.3 GB map + 2.2 GB lists |
| r 5 mm, ball list, 1 mm | 0.43 + 0.13 + 0.71 = 1.27 | 7.85 | 0.37 GB + 2.2 GB |
| r 2 mm, 1-ring (14.2), 0.5 mm | 0.15 + 0.23 + 1.01 = 1.38 | 5.62 | 1.3 GB + ring CSR |
| r 10 mm, ball list (92.8), 0.5 mm | 0.96 + 0.21 + 0.34 = 1.52 | 15.2 | 1.3 GB + 4.8 GB |
| r 2 mm, 1-ring, 1 mm | 0.15 + 0.13 + 1.62 = 1.90 | **5.29** | 0.37 GB + ring CSR |

- **Measured units: 5.3 ms at best,** 4.4x the 1.2 ms stop and 10.6x VSM's 0.50 ms share.
- **Optimistic bound: 1.16 ms at best,** but only with about 3.5 GB of extra memory, before the moving sun, the queue compaction or N0's 412.5 MB BVH for refinement. N4's lesson applies: its CPU-side model projected 1.32 ms and it measured 8.08 ms.
- **Refinement does not depend on the model.** Undecided rays need a full software ray, and in the settings with compact near lists that is 1.0-1.9 ms on its own.

## 4. Decision and the honest non-RT position

1. **The split is not adopted, and there is no GPU probe plan.** The stop condition holds: exact is possible, but not at a projected ≤ 1.2 ms.
2. **Non-RT is a quality-first mode at its measured cost.** Exact 8-ray shadows cost 8.08 ms (LSG, N4) to 9.43 ms (N0), so the projected non-RT Interactive frame is about 9-11 ms. Unreal's non-RT VSM frame is 1.61 ms, but VSM fails the film gate on these contacts. Non-RT therefore misses the "beat Unreal's non-RT configurations" goal on cost while keeping the film-quality estimand. Record this as a miss; do not trade quality for speed.
3. **What the probe adds.** It confirms the irreducible part: 2.5-20% of sun rays sit too close to a far caster's silhouette for any conservative depth representation to decide. Each needs an exact ray, at 3 ns on this GPU without RT cores. Near-field exactness is cheap in triangles, about 3 per receiver in the cone, but no compact structure delivers exactly those. Topology rings miss cross-placement contacts on overlapping scans, and spatial lists cost gigabytes.
4. **What would reopen it.** One GPU microbenchmark under 90 s, on N0's harness with no renderer change. It would need to show all of:
   - a coherent flat near-list pass at or below 0.05 ns per (receiver, triangle);
   - a DDA texel walk at or below 15 ps per cell;
   - a queued software refine ray at or below 1 ns.

   Every light-space structure measured so far on this GPU has run 6-20x above its traffic model, so this is not proposed as the next unit.

## Evidence and time

- **Evidence** is in `E:/renderlab-work/non-rt-n5/`:
  - `full/n5.json`: every table above.
  - `full/rays.npz` (1 MB): per-ray nearest and farthest hit, near-hit rings and the cluster flag.
  - `footprint.json`: light-plane area and pages.
  - `smoke/`: the 4-tile smoke runs.
  - `full.log`: the final run's log.

  No raw dumps were written.
- **Runs:**
  - The final full run took 612 s (17 s load, 574 s rays and maps, 11 s rings). It was one process, which started with 7 GiB available and kept at least 4.4 GiB available throughout.
  - Peak private memory was 1.52 GiB. The 1.89 GiB peak working set includes the mapped asset's file pages.
  - The footprint run took about 10 s.
  - Two earlier full attempts did not finish:
    - The in-run guard stopped the first at 20 s, when system available memory fell to 1.47 GiB because of other workers.
    - I stopped the second at tile 75, when its peak private memory reached 2.08 GiB: map transients at 0.5 mm. The fixes (smaller chunks, an 8 M-texel map cap that splits tiles, sorted grid keys, single-threaded BLAS) reproduce the smoke results bit for bit.
- **Time:** about 15 minutes reading, 45 minutes writing and debugging the probe, 15 minutes of runs (the smokes, the two aborted attempts and the final run) and 20 minutes writing this document. There was no GPU, window, build or Unreal launch.
