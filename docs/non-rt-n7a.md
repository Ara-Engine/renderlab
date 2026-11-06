# N7a: world-space visibility cache on the GPU (non-RT)

2026-11-06 UTC. Unit N7a of [non-rt-n7.md](non-rt-n7.md) §6, on N0's software tracer ([non-rt-n0.md](non-rt-n0.md)). Branch `t3/n7a-visibility-cache`: `origin/t3/n4-light-space-grid` (a0cb9cb) with `origin/main` (7979345) merged. Test-only; no production file changed. Labels: **M** measured here, **I** inference.

## Result

**Stop on the frozen rules (M).** Values are measured, and the cache is exact against its own fresh traces.

- **Cost.** The warm lookup costs 0.40–0.42 ms, against the 0.30 ms line. Warm Q-walk shadow work costs 4.1 ms p50 and 13.4 ms p99, against 0.5 and 1.2 ms.
- **Quality.** Below Unreal VSM's error on the main populations everywhere: partial, contact, occluded, missed shadows and flips. Above VSM on lit, faint and isolated pixels on the quarry, so the frozen quality rule stops it there. Exact and Interactive also exceed VSM on faint and isolated there. On S2, the rule breaks in 11–22 of 160 frames per track, against Interactive's 1–11.

| Frozen rule ([non-rt-n7.md §6](non-rt-n7.md#6-gpu-first-unit-n7a)) | Measured | Verdict |
|---|---|---|
| Quality: any R_F population RMS above VSM's, missed above VSM's, or false above max(VSM's, 2x Interactive's), on any Q-walk or S2 frame | Quarry poses 0 and 1: lit 0.023 against 0.0027, faint 0.155 against 0.046, isolated 0.187 against 0.049. Every other population and the missed count are below VSM. False shadows: 595, under 2x this run's Interactive (2 x 393) but above 2x the film doc's (2 x 9). S2: 15 / 22 / 17 / 11 frames break it, mostly faint, then lit and one false; Interactive breaks it in 5 / 11 / 4 / 1 | **Stop** |
| Correctness: a stored outcome differs from a fresh trace in validation replay | 0 of 111 M stored rays replayed, over the quarry, Q-walk, sun, S2 and S3 | Pass |
| Lookup and update pass above 0.30 ms, warm 1080p frame | Quarry 0.404–0.424 ms (Q-walk 0.56 ms p50); S2 0.27–0.28 ms; S3 0.15 ms | **Stop** (quarry) |
| Warm Q-walk shadow cost p50 above 0.5 ms or p99 above 1.2 ms | 4.12 ms p50, 13.4 ms p99 over 128 moving frames. The return path alone, all on surface seen before, is 0.99 / 3.5 ms | **Stop** |
| Queued fill rays above 6.8 ns each | Cold views 5.4–6.6 ns. Sparse fills 64 ns (Q-walk, 52 k rays in 3.3 ms) to 136 ns (1°/s sun, 6.6 k rays in 0.89 ms) | **Triggered** for sparse fills; §2 re-priced below |
| Sun at 1°/s adds more than 0.25 ms | Quarry +0.90 ms (1.31 against 0.41 ms p50); S3 +0.31 ms (0.46 against 0.15) | **Miss** recorded; 10°/s: quarry +2.40 ms |
| Memory above 128 MB | 100.5 MB allocated. The Q-walk's 3.72 M cells overfill the 4 M-slot table: 156 k probe-limit lookups fall back. Holding them at load 0.5 needs 2^23 slots, a 134 MB table, 168 MB in all | Pass as allocated; **above the cap** at the size the walk needs |
| Cut frames: recorded, no gate | Cold quarry view 47–63 ms (1.05 M cells, 8.4 M rays) | Amortization measured below; decision for Islam |

**Why the N7 estimates missed (M).** Three causes:

- **Triangle density.** The quarry has about 8 triangles under each pixel footprint, while pixel centres hit about one. The N7 probe counted 589 k cells, the triangles that pixel centres hit, plus subdivision. The full set under the view is about 2.8 M cells: 2.77 M keys after 16 Q-walk frames. After any camera step, most pixel centres land on triangles never filled.
  - A 1-ring of adjacent triangles per new cell, the doc's "each newly seen footprint traces all of its cells", reduced the misses but did not remove them. The outward walk still claims 14 k new cells per frame p50, and moving frames fill 15.6 k cells (52 k rays) p50, against the 240 cells the doc estimated.
- **Sparse fills are latency-bound.** A dispatch of a few thousand cells costs about 0.8 ms however few rays it holds: one deep software traversal sets the floor. Full-frame fills reach 5.4–6.6 ns per ray.
- **The lookup costs 0.40 ms, against the 0.05–0.2 ms model.** Per pixel it fetches the triangle, computes the footprint level, probes the hash and tests 16 lattice directions against the cone. This is the same model-versus-GPU gap as N4 and N5.

**Quality defect found (M).** A cell's centroid can lie inside other geometry. On S3, slab triangles partly under a pillar and pillar triangles below the ridged slab return "fully shadowed" for lit pixels: 114 flips per frame, in arcs at the pillar feet ([image](#evidence-and-time)). On the quarry the 12 scans interpenetrate. Within 2 px of another placement (4.5% of pixels), lit RMS is 0.106 for the cache against 0.014 for Interactive. Elsewhere it is 0.0128 against 0.0109. Interactive samples the pixel's visible point, which is never buried. A burial guard at fill time is the proposed fix (§6). It is not built here, because the cost stops already decide the unit.

## 1. What was built

Files (all new, test-only unless noted):

- `shaders/visibility_cache.wgsl`: table, lookup, plan, fill, resolve and validation kernels, appended to N0's traced module. Rays use N0's `software_occluded`, with the same receivers' ε and side policy.
- `src/visibility_cache.rs`: the CPU mirror of keys, cells, lattice, window and shift; the parameter block; the composition. `#[cfg(test)]` module in `lib.rs`.
- `src/visibility_cache_tests.rs`: CPU tests and the quarry probe `gpu_visibility_cache_probe`.
- `src/visibility_cache_bench.rs`: S2/S3 `gpu_visibility_cache_bench`. It copies motion_bench_tests.rs's crop film generator, whose file N8 owns.
- `scripts/n7a-score.py`: R_F scoring and the frozen rule.
- Small edits:
  - N0's `Software::new` takes a draw list, so S2/S3 scenes bind too, and keeps its placement buffers for movers.
  - `film_sources` is `pub(super)`.

**Cells.** A receiver triangle at level k splits into the 4^k sub-triangles of its 2^k barycentric subdivision.

- A cell is keyed by (placement, primitive, level, i, j, upper, side) and sampled at its centroid.
- A pixel picks k = round(log2(sqrt(area) / footprint)), so a cell has about one footprint of area, and reads the cell under its hit barycentrics.
- Triangles smaller than the footprint are one cell. That is 98.6% of quarry cells (2.74 M of 2.77 M).

**Directions.** Each cell owns a jittered, rotated square lattice in a gnomonic chart fixed in the world at the anchor sun.

- The spacing puts 8 points on average in the sun cone (0.04 rad). The window test measured 7.6–8.4 near the anchor and up to 9.6 at 20°.
- A cell stores a 4x4 window: 16 blocked bits, 16 valid bits and the window origin.
- Disk membership is the exact cone around the current sun, which is Interactive's estimand. When the sun moves, the window slides and only points entering the cone are traced.

**Table and passes.** An open-addressed table of 2^22 slots, 16 B each (64 MB):

- Key claims use a compare-exchange on the first key word. Readers that see a claim in flight retry in the resolve.
- The lookup writes usable cells' counts. It queues cold cells once per frame (an enqueue stamp), prefetches their neighbours, and defers the pixel.
- The plan takes visible requests first, then prefetch, up to the fill budget (65,536 cells).
- The fill traces the missing rays against static placements.
- The resolve reads the now-filled cell. If the budget left it empty, it uses a filled whole-triangle neighbour, else N0's Interactive estimate at the pixel's own receiver.
- Queue and pending appends are aggregated per workgroup, so fills run in tile order.

**Movers.**

- Static visibility only is cached.
- Each frame, the lookup tests a cell's unblocked rays against the movers' own BVHs (S2 plate and blocker).
- Receivers on movers are not cached; they take the Interactive fallback (about 15 k S2 pixels a frame).

**Changes made during the unit, each measured first:**

1. **Footprint prefetch.** The first build filled only the pixel-centre cells. 60% of pixels were cold after a 1 cm step (210 k new cells), against the doc's 0.04%. Unusable cells now prefetch their in-triangle sub-triangle neighbours, and for whole-triangle cells the triangles across their edges, from the runtime adjacency.
2. **Claims in flight retried in resolve.** 5% of cold-view pixels lost a claim race and fell back to Interactive.
3. **Partial windows.** A cell missing at most one ray in the cone stays usable while that ray waits at secondary priority. This changes the estimand during sun motion by at most one of about 8 rays.
4. **Workgroup-aggregated queues.** The cold-view fill went from 8.8 to 5.4 ns per ray.

## 2. Correctness (M)

**Validation replay.** Every valid stored ray of every claimed slot is traced again with N0's query, in its stored window, and compared with its stored bit: **0 mismatches in 111 M rays**. That covers quarry poses (8.4 M), the Q-walk (27.2 M), the rotating sun (9.7 M), the earlier b3 run (45 M), S2 (17.7 M) and S3 (3.2 M).

**CPU replay, first 16 Q-walk frames, unbounded fills.** The CPU recomputes every pixel's key from the GPU hit records in f32, with the same prefetch rules and the CPU-built runtime adjacency.

| Check | CPU | GPU | Difference and class |
|---|---:|---:|---|
| Keys | 2,775,096 | 2,774,676 | 911 CPU-only: 894 prefetch-only, 13 centres, 4 same triangle with another key. 491 GPU-only: 489 prefetch-only, 2 same triangle. The prefetch-only keys match the claims the GPU dropped while a slot was in flight (counter: 138 per frame p50, 44 k in the cold frame): a claim race, not a value error |
| Windows | — | — | 323 of 2.77 M differ; 2 lie within 1e-4 of a lattice line, the rest are not classified further. Separately, 1,196 cells were claimed in the last frame's resolve and are not filled yet |
| Stored rays, 20,000 sampled cells | 159,856 | — | 2 differ, at 3.5 and 6.5 µm from a triangle boundary or the ε segment end (ε = 10 µm): f32 receiver rounding |

The replay is not bit-equal, so the doc's order item 1 is met only up to these classes. Stored values are exact by the validation replay.

**Determinism.** Warm frames at a camera are identical to each other. Q-walk frame 160, which returns to pose 0's camera, equals pose 0's warm frame. Pose 0's cold frame differs from its warm frame in about 550 pixels that the cold frame resolved by neighbour substitution.

## 3. Cost (M)

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan; release test binaries `n7a-b3.exe` and `n7a-b4.exe`; 1920x1080; one offscreen process per scene; no validation layer. Each pass is timed with begin/end timestamps, one submission per frame. N0's Interactive and the hardware Interactive were timed in the same processes.

**Quarry (13.0 M triangles):**

| Case | Lookup | Fill | Resolve | Shadow total | Rays | Notes |
|---|---:|---:|---:|---:|---:|---|
| Warm, static camera | 0.404–0.424 | — | — | **0.41–0.43** p50 | 0 | Rule line 0.30 |
| Cold view, unbounded fill (2 runs x 2 reps) | 1.3–1.7 | 45–62 | 0.53–0.61 | **47–63** | 8.4 M | 1.05 M cells: 0.33 M centre, 0.71 M ring; 5.4–6.6 ns/ray |
| Pose 1 after pose 0 (1 cm) | 0.99 | 23.3 | 0.33 | 24.6 | 3.6 M | 454 k new cells |
| Q-walk moving frames (128), p50 / p99 | 0.56 / 1.12 | 3.28 / — | — | **4.12 / 13.4** | 52 k p50 | 15.6 k cells filled p50 |
| — walk out (64) | 0.60 | 4.56 | 0.32 | 5.37 / 13.4 | 280 k | |
| — hold (32) | 0.53 | 0.01 | 0.44 | 0.98 / 1.78 | 0 | 99 px/frame fall back: the table is overfull |
| — return (64) | 0.48 | 0.21 | 0.19 | 0.99 / 3.5 | 17 | Surface seen on the way out |
| Sun 1°/s (64 frames) | 0.41 | 0.89 | 0.01 | 1.31 p50 | 6.6 k | +0.90 ms; 136 ns/ray |
| Sun 10°/s (64 frames) | 0.46 | 2.33 | 0.02 | 2.81 p50 | 101 k | Budget-capped at 65,536 cells |
| Sun jump 5° | 1.24 | 2.60 | 11.8 | 15.6, then 14.7 | 378 k | 279 k pixels fall back to Interactive |

Same processes: N0 software Interactive 9.5–11.4 ms; hardware Interactive 0.39–0.41 ms; display (the production soft shading pass, whose cost does not depend on the visibility values) 0.37–0.39 ms. Unreal VSM: 1.61 ms per frame, 0.50 ms of it shadows ([bench-motion](bench-motion.md), [non-rt-n7.md](non-rt-n7.md)).

**S2 (32 triangles) and S3 (525 k triangles), per frame:**

| Bench / track | Cache shadow p50 / p99 | Lookup p50 | N0 Interactive | Hardware Interactive | Cold view |
|---|---:|---:|---:|---:|---:|
| S2 camera 0.25 px | 0.312 / 0.407 | 0.271 | 0.230 | 0.122 | 1.25 ms, 530 k cells |
| S2 camera 0.5 px | 0.315 / 0.405 | 0.273 | 0.230 | 0.122 | 1.27 ms |
| S2 camera 1 px | 0.324 / 0.413 | 0.281 | 0.231 | 0.123 | 1.26 ms |
| S2 shadow-step | 0.317 / 0.353 | 0.280 | 0.229 | 0.123 | 1.26 ms |
| S3 static and holds | 0.15 | 0.147 | 1.62 | 0.172 | 5.0 ms, 383 k cells |
| S3 0.25°/s | 0.434 / 0.829 | — | 1.61 | — | |
| S3 1°/s | 0.462 / 0.827 | — | 1.61 | — | |
| S3 jump recovery | 0.149 / 1.869 | — | 1.64 | — | |

On S2, rays are so cheap that the cache costs more than N0's Interactive: 0.31 against 0.23 ms. The lookup there includes the mover composite: about 2 M lattice rays a frame tested against the two movers. On S3 it is 10x cheaper than N0's Interactive when the sun holds, and adds 0.28–0.31 ms while it rotates. Unreal VSM's rotation increment on S3 is +0.23 ms of shadow depth.

**§2 re-priced (the 6.8 ns rule).**

- Cold views run at 5.4–6.6 ns per ray.
- Sparse fills cost about 0.8 ms a dispatch plus about 11 ns per ray: 6.6 k rays take 0.89 ms, and 101 k rays take 2.33 ms.
- N7's per-frame estimates (0.007–0.11 ms) assumed 3.4 ns per ray. A dispatch of a few thousand rays cannot go below the floor.

## 4. Quality against film references (M)

**Quarry, pose 0, policy R_F** (populations of [m6-film-reference.md](m6-film-reference.md), scored in one process). Pose 1 rows differ by at most 0.011 per population (isolated is the largest), with the same rule results; they are in `scores-probe-4/quarry.json`.

| Method | Partial | Lit | Occluded | Faint | Isolated | Contact | Contact lost | Leaks | Solid flips | Missed | False |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Exact (saved, f0) | 0.2316 | 0.0019 | 0.0000 | 0.1093 | 0.1823 | 0.2084 | 327 | 0 | 1 | 9,199 | 9 |
| N0 Interactive, this run | 0.2483 | 0.0110 | 0.0080 | 0.1382 | 0.1765 | 0.2026 | 103 | 15 | 21 | 7,200 | 393 |
| **N7a cache, warm** | **0.2569** | **0.0228** | **0.0095** | **0.1554** | **0.1873** | **0.2119** | **229** | **14** | **139** | **7,576** | **595** |
| N7a cache, cold view | 0.2569 | 0.0233 | 0.0095 | 0.1557 | 0.1873 | 0.2120 | 230 | 14 | 145 | 7,580 | 601 |
| N7a cache, pose 1 after pose 0 | 0.2579 | 0.0225 | 0.0116 | 0.1582 | 0.1976 | 0.2103 | 228 | 14 | 136 | 7,583 | 582 |
| Unreal VSM | 0.3706 | 0.0027 | 0.4826 | 0.0456 | 0.0486 | 0.6267 | 7,638 | 4,390 | 3,373 | 24,049 | 9 |

- This run's Interactive has 393 false shadows, where the film doc's Interactive had 9. Current production receivers differ from the saved ones near edges.
- The rule's "2x Interactive" therefore has two readings, 786 or 18. The cache's 595 passes the first and breaks the second.

**S2, all four disk tracks (pixel-frames pooled, worst-frame leaks and flips).**

- The regenerated references reproduce the archived per-frame reference statistics on all 640 frames.
- N0's Interactive equals the archived hardware Interactive to every digit, so the archived Unreal rows compare directly.

| Track | Method | Partial RMS | Lost contact / contact | Max leaks | Max flips | Missed | False | Error change RMS | Shadow ms p50 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| camera 0.25 px | **N7a cache** | **0.0776** | **237 / 48,650** | **1** | **6** | 24,860 | 5,922 | 0.0140 | 0.31 |
| | Interactive | 0.0749 | 0 | 0 | 0 | 23,450 | 4,319 | 0.0104 | 0.23 (N0) / 0.12 (HW) |
| | Unreal VSM | 0.1748 | 5,112 | 62 | 167 | 231,165 | 10,744 | 0.0598 | 2.13 (whole GPU) |
| camera 0.5 px | N7a cache | 0.0781 | 306 / 48,904 | 4 | 4 | 25,224 | 5,780 | 0.0177 | 0.32 |
| | Interactive | 0.0747 | 0 | 0 | 0 | 23,659 | 4,386 | 0.0129 | |
| | Unreal VSM | 0.1751 | 5,052 | 70 | 183 | 230,447 | 10,678 | 0.0594 | |
| camera 1 px | N7a cache | 0.0785 | 239 / 47,928 | 1 | 5 | 25,671 | 5,774 | 0.0214 | 0.32 |
| | Interactive | 0.0753 | 0 | 0 | 0 | 24,197 | 4,051 | 0.0150 | |
| | Unreal VSM | 0.1750 | 5,005 | 71 | 168 | 228,765 | 9,801 | 0.0595 | |
| shadow-step | N7a cache | 0.0764 | 480 / 42,206 | 0 | 8 | 24,542 | 6,026 | 0.0034 | 0.32 |
| | Interactive | 0.0742 | 0 | 0 | 0 | 24,054 | 3,732 | 0.0032 | |
| | Unreal VSM | 0.1691 | 5,186 | 67 | 120 | 221,148 | 7,313 | 0.0525 | |

- **Motion stability.** The cache's frame-to-frame error change is 1.3–1.4x Interactive's under camera motion, and equal for the moving blocker. That is 2.8–15x lower than VSM's.
- **Static error.** The cache also loses contacts on hold frames (32–96 per phase), so part of its error is static. Cells span up to about 1.4 px on S2's large triangles.
- **Movers.** Mover receivers take Interactive. Contacts under the moving blocker are lost in 192 pixel-frames per moving phase.

**S3, 1°/s sun, per phase.** The references were regenerated with reset shadow bases (the [reset-identity fix](temporal-reset-identity.md)), so their sample realizations differ from the archive: partial counts match in 133 of 385 frames. N0's Interactive (0.0779) matches the archived hardware value (0.07790), so the Unreal rows compare at reference-noise level.

| Phase | Method | Partial RMS | Lost contact / contact | Max leaks | Max flips | Error change RMS | Shadow ms p50 / p99 |
|---|---|---:|---:|---:|---:|---:|---:|
| static (32) | N7a cache | 0.1155 | 448 / 48,512 | 3 | 114 | 0.0000 | 0.154 / 0.161 |
| | N0 Interactive | 0.0775 | 0 | 0 | 0 | 0.0000 | 1.62 |
| | Unreal VSM | 0.1878 | 2,929 | 111 | 187 | — | |
| rotate 0.25°/s (128) | N7a cache | 0.1156 | 2,036 / 194,291 | 4 | 114 | 0.0023 | 0.434 / 0.829 |
| | N0 Interactive | 0.0780 | 0 | 0 | 0 | 0.0025 | 1.61 |
| | Unreal VSM | 0.1875 | 11,630 | 111 | 190 | — | |
| rotate 1°/s (128) | N7a cache | 0.1134 | 1,723 / 196,290 | 6 | 116 | 0.0047 | 0.462 / 0.827 |
| | N0 Interactive | 0.0779 | 0 | 0 | 0 | 0.0050 | 1.61 |
| | Unreal VSM | 0.1863 | 12,289 | 106 | 192 | — | 2.44 (whole GPU) |
| 5° jump recovery (33) | N7a cache | 0.1114 | 416 / 51,777 | 0 | 109 | 0.0036 | 0.149 / 1.869 |
| | N0 Interactive | 0.0761 | 0 | 0 | 0 | 0.0011 | 1.64 |
| | Unreal VSM | 0.1811 | 3,153 | 114 | 226 | — | |

- On S3 the cache stays below VSM on partial RMS, contact, leaks and flips.
- It is clearly worse than Interactive: 1.46x partial RMS and 114 flips per frame. Its frozen rule breaks on all 385 frames (faint, lit, false, solid), against 318 for Interactive (faint, isolated).
- The flips are the buried-centroid defect at the pillar feet.
- Sun motion adds no visible lag: error-change RMS equals Interactive's while rotating.

## 5. Cold views and their amortization (M, proposal I)

- **A cold quarry view costs 47–63 ms**: 1.05 M cells, 8.4 M rays. That is 5–6x N0's full-frame Interactive (9.5–11.4 ms), because it fills about 3 cells per pixel.
- **Measured progressive fill** at pose 0, with each unfilled pixel taking Interactive at its own receiver:
  - 131,072 cells per frame: 5 frames, 16.4, 14.6, 8.5, 1.5 and 0.4 ms.
  - 32,768 cells per frame: 12 frames, 16.4 ms falling to 0.4 ms.
- Quality stays at Interactive's or better throughout. Cost does not: sparse Interactive fallbacks and fills both pay the latency floor.

**Proposal for Islam (I).** On a cut or a jump over half the disk:

- show N0's Interactive full-frame for one frame (9.5–11.4 ms), as the bar requires;
- fill the cache at 131 k cells per frame behind it, warm in about 4 frames;
- accept 2–3 frames of 9–16 ms on the software path.

No cheaper fallback meets the bar. A hard 1-ray shadow (about 1.2 ms) would be a below-bar mode on those frames and cannot be a default. Both options are open; the frozen rules gave no gate here.

## 6. Where next (I)

- **Burial guard.** At fill time, mark a cell unusable when a short probe from its centroid along the keyed side is blocked within a few triangle sizes. Its pixels take Interactive, or a finer level. Expected to remove the S3 foot flips and most of the quarry's 7.7x lit excess near scan overlaps, at some fallback cost. It does not change the cost stops.
- **Cells over triangle clusters, not triangles.** The 8-triangles-per-footprint factor sets both the cell count (2.8 M) and the Q-walk misses.
  - This needs a 2D chart per cluster, such as the meshlets N1b would cull, and must not mix surfaces.
  - N7's world-voxel rows showed what mixing costs: 276–1,549 flips.
- **Lookup.** The per-pixel key from N1b's visibility buffer, plus a sun epoch per cell that skips the 16-direction cone test when the sun holds. Unmeasured. Every structure on this GPU has run above its model.
- **Sparse fills.** Batch partial-window fills every K frames: about 0.11 ms average at 1°/s with K = 8 (I), with p99 frames near 0.9 ms; or overlap them with other work.
- **Not proposed.** Loosening any frozen rule.

## Evidence and time

Evidence is in `E:/renderlab-work/n7a/`:

- `runs/probe-4/summary.json`: final quarry run, b4. probe-3 is b3; probe-1 and probe-2 are earlier builds.
- `runs/s2-1/`, `runs/s3-1/`: per-case `cache-stats.json`, crops, regenerated references (npz), `summary.json`.
- `scores-probe-4/`, `scores-s2-1/`, `scores-s3-1/`: scorer output with per-frame rule results.
- `runs/s3-1/cache-error-f0000.png`: the S3 flip map (yellow) inspected above.
- `runs/*.log`, `*-preflight.json`, `gpu-windows.json`: lease logs, the memory guard and preflight records.
- `builds/`: test binaries with SHA-256.
- `edits/`: the analysis scripts used here.
- Raw per-pose count dumps above 5 MB were deleted after scoring.

**Verification time (M).** Every lease ran under the 4 GiB start and 2 GiB abort guard, with game and foreign-renderer checks; no lease was deferred or aborted.

| Lease | Wall time | Note |
|---|---:|---|
| probe-1 | 31 s | Failed on a usage conflict, fixed in b2 |
| probe-2 | 37 s | |
| probe-3 | 47 s | |
| probe-4 | 43 s | |
| s2-1 | 224 s | Includes 213 s of references |
| s3-1 | 178 s | Includes 168 s of references |

- Release test builds took 1–1.5 min each, four in all.
- CPU scoring took 11–150 s per run.
- `cargo test -p renderlab --lib` passed by exit status: 106 passed, 0 failed, 101 ignored.

**Implementation.** About 07:35 to 08:45 UTC, interleaved with the checks above; editing time was not isolated. There was no window, viewer, game capture or Unreal launch.
