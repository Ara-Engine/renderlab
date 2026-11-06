# Non-RT path 2: shadows and primaries after N0

Design decision, 2026-11-06, base `6437b00`. It follows the N0 stop ([non-rt-n0.md](non-rt-n0.md)): software BVH traversal is exact but 12-22x slower than hardware. Design plus CPU evidence only: one NumPy light-space probe over the saved pose-0 receivers and the read-only quarry source, `scripts/non-rt-shadow-probe.py`. There was no GPU run, build, window or Unreal launch. Labels: **M** measured, here or in the cited record; **A** author-reported; **I** inference or estimate.

## Decisions

1. **Without RT, only exact-estimand methods can pass the film gate on this content.** The gate scores per-sample visibility from exact receivers, with a 10 µm ε and per-pixel rotated R2 directions. On the quarry, 39% of blocked sun rays are blocked within 1 mm of the receiver, and 59% within 10 mm (M, [§1](#1-quarry-probe-m)). Every depth-map method misses or invents shadows at that scale, at every texel size tested down to 0.625 mm, which is 8-16x finer than Unreal's 1 texel per pixel here. The shipped non-RT techniques surveyed are all depth-map based: VSM, cascades, PCF/PCSS, and HFTS beyond contacts. Near contacts they rely on bias, filtering or screen-space contact shadows, which AGENTS.md rules out.
2. **Non-RT shadows: a light-space caster grid (LSG).** The receiver's 8 (Interactive) or 256 (Exact) directions query, in one pass, a sun-aligned 2D grid of conservatively binned casters with a max-height pyramid. Survivors are tested per direction with N0's watertight test in placement object space. Any-hit is order-independent, so counts must be **byte-identical to N0's software counts**. This is the inverse of an irregular z-buffer and the parallel-light case of Hunt & Mark's ray-specialized grids (2008). It removes N0's limiter, near-surface traversal: the probe's per-pixel cone of the sun disk contains a median of 3 candidate triangles (mean 40). Estimate (I): 0.3-1.0 ms for Interactive, against VSM's 0.50 ms shadow share.
3. **Kalbertodt's tiled soft shadow volumes do not transfer.** Per-pixel work is the tile list: 20,859 triangles on average on the quarry against the author's ~30 (M against A), about 145 ms (I). Its light-facing cull loses 1.7% of blocked rays on the open scans (M).
4. **Primaries: N1b, the culled raster visibility buffer, using Bevy's meshlet chain.** Estimate 0.38-0.7 ms against software primaries' 2.84 ms. The RT path keeps traced primaries (0.30 ms) unless N1b measures faster.
5. **Caster LOD is rejected.** Software cost is within 25 cm of the receiver (M, N0), and there the gate tolerates less error than one source triangle.
6. **RT path:** LSG would not beat hardware Interactive (0.12-0.20 ms of rays). It may beat hardware **Exact**: an estimated 1.5-4 ms against 8.49 ms (I), because each triangle is fetched once and tested against all 256 directions. N4 measures this; if it holds, Exact and reference generation use LSG on both paths.
7. **Projected non-RT Interactive frame (I): 1.2-2.2 ms static**, against VSM 1.61 and RT 0.86 (M). The 2x floor (0.80 ms) is missed by 1.5-2.8x. A moving sun costs a rebuild whenever it leaves a margin cone ([§5](#5-first-unit-n4-light-space-caster-grid)).
8. **First unit: N4**, a CPU replay of LSG on all pose-0 receivers, then a WGSL probe on the N0 harness, with the stop rules in §5. N1b can run in parallel under separate file ownership.

## 1. Quarry probe (M)

Pose 0, registered sun (0.4, 0.8, 0.3) normalized (58° elevation), 0.04 rad disk, R1 viewed receivers with ε = 10 µm. 300 random 16x16 tiles (seed 20261106) give 71,640 receivers, 21% of the 346,517 covered pixels. A directional light makes each central ray a 2D point query in light space: a receiver is blocked if a triangle's projection contains it with light depth above the receiver by more than ε. That is a CPU irregular z-buffer, two-sided, in f64 over f32 light-space vertices.

| Measure | Value |
|---|---|
| Central ray blocked / lit | 6,642 / 64,998 |
| Agreement with the saved 256-ray Exact | 70 of 4,950 Exact umbra are centrally lit; 306 of 64,591 Exact fully-lit are centrally blocked (0.5% overall). Not classified: the central direction is not in the 256-sample set, whose spacing is about 0.0044 rad (1.8 mm at 40 cm). The cull and depth-map rows below compare against this probe's own exact test, with the same triangles and arithmetic |
| Blocker distance, blocked rays: p50 / p90 / p99 | 4.6 / 115 / 425 mm |
| Blocked within 1 / 5 / 10 / 25 mm | 39% / 51% / 59% / 72% |
| Drop light-facing triangles (Kalbertodt's closed-mesh cull) | **Loses 116 blocked rays (1.7%), 3 within 10 mm** |
| Keep only light-facing triangles (the converse) | Loses 0 here. It holds only because the scanner captured the sun-facing tops, so a renderer cannot assume it |
| Central-ray depth complexity, triangles containing the point: mean / p50 / p99 | 2.7 / 2 / 12 |
| Sun-disk cone candidates per pixel (bbox-conservative): mean / p50 / p90 / p99 | 40 / 3 / 8 / 1,127 |
| Same, per 16x16 tile (the tile's swept cone): mean / p50 / p90 / p99 | 20,859 / 4,068 / 24,686 / 292,691 |

**Depth map at Unreal's texel ladder (M).** The map holds the nearest-to-light surface at the texel centre, and the receiver is compared at its own position. "Receiver plane" extrapolates the receiver's own triangle plane to the texel centre, the strongest standard depth correction. False = truly lit, map shadowed (of 64,998); lost = truly blocked, map lit (of 6,642). Each cell is false / lost, with the lost shadows inside 10 mm in parentheses.

| Texel | Receiver plane, bias 0.1 mm | 0.5 mm | 1 mm | 5 mm | Plain depth, 1 mm |
|---|---|---|---|---|---|
| 10 mm | 21,855 / 1,225 (966) | 13,394 / 1,258 | 8,748 / 1,287 (996) | 1,950 / 1,455 (1,113) | 21,928 / 1,254 |
| **5 mm** | 14,135 / 882 (720) | 6,304 / 909 | **3,498 / 930 (745)** | 863 / 1,082 (871) | 15,106 / 918 |
| 2.5 mm | 7,083 / 643 | 2,325 / 672 | 1,266 / 692 (578) | 435 / 866 | 8,135 / 611 |
| 1.25 mm | 2,844 / 435 | 878 / 453 | 552 / 470 (391) | 220 / 700 | 3,073 / 418 |
| 0.625 mm | 1,022 / 262 | 405 / 286 | 283 / 304 (272) | 113 / 589 | 1,048 / 261 |

- **Unreal's ladder (I).** Unreal's directional clipmap texels are powers of two. At this footprint (7.6-10.4 mm, M), 1 texel per pixel lands on the 5 or 10 mm level, depending on the LOD-bias CVar, which was not recorded. There a depth map loses 14-19% of hard shadows, most of them contacts, while still inventing 5-13% false ones.
- **No row reaches zero, even at 0.625 mm.** Bias trades false shadows for lost contacts. At bias 0, lost shadows fall only about 30% per halving of the texel (1,192, 835, 598, 390, 232), because texel-centre sampling misses small casters.
- **Receiver-plane false margins** reach 12 mm at p99.9 even at 0.625 mm, on grazing receivers.
- **This is the hard central ray only.** SMRT's soft estimate marches the same single-layer map, so it cannot recover what the map lost. Unreal's native evidence agrees: VSM partial RMS .369, and .287 with NormalBias 0, against Renderlab's .0065 ([competitor weaknesses](research/competitor-weaknesses.md#what-the-existing-unreal-numbers-actually-establish)).

## 2. Shadow options priced against the film gate

Cost denominators (M): Unreal VSM frame 1.61 ms static and 2.06 ms with a moving sun. Its shadow depths take 0.349 ms and projection 0.155 ms ([full-detail comparison](unreal-quarry-full-detail-comparison.md), [end-to-end](quarry-end-to-end-comparison.md)).

| Option | Same estimand? | Quarry cost | Film gate | Verdict |
|---|---|---|---|---|
| VSM, clipmaps, page caching, ~1 texel/px | No | 0.50 ms (M, Unreal) | Fails: 14-19% lost, 5-13% false at 5-10 mm and 1 mm bias (§1) | Rejected as core |
| VSM at 0.625-1.25 mm | No | 16-256x the texels, ≥ 3.5 ms (I) | Still fails: every row keeps both. At 0.625 mm, bias 0.1-5 mm gives 0.2-1.6% false and 3.9-8.9% lost | Rejected |
| FTIZB / hard IZB per direction (Wyman 2015-16) | Only with 8 frame-global directions, which is a different, correlated estimator | 20 ms per direction for 12.3 M triangles on a GTX 980 (A); about 3-4 ms per direction here (I), so 25-30 ms for 8 | Hard shadows exact; soft only through 8 IZBs | Rejected |
| Tiled per-triangle soft shadow volumes (Kalbertodt 2023) | Yes, with Renderlab's directions instead of 208 cells | Tile lists average 20,859: 7.2 G pairs per frame, about 145 ms at the author's ~50 G pairs/s (I) | Its cull leaks 1.7% (M); without the cull it is exact | Rejected as organized |
| Conservative-raster or deep shadow maps | No: per-texel visibility is filtered or aliased at mm contacts | ~VSM | Fails like VSM | Rejected as visibility; conservative binning survives as LSG's cull layer |
| Hybrid: exact hard IZB + penumbra estimator (PCSS-like) | No | ~IZB + map | A single blocker depth cannot estimate a disk with blockers at 1-10 mm | Rejected |
| Hybrid: map bulk + exact refinement near edges | Only if the bulk test is conservative | Map + refinement | A texel-centre map declares 3.5-19% of blocked receivers certainly lit (the bias-0 lost counts). Made conservative, this hybrid is LSG | Folded into LSG |
| Software BVH, 8 rays (N0) | Yes | 9.43 ms (M); 2.5-3 ms with CWBVH and stack levers (I, N0) | Pass (M parity) | Kept as the fallback for local lights and incoherent rays |
| Same BVH, one 8-ray packet per receiver (shared near-surface nodes, one triangle fetch) | Yes, byte-identical | 1.4-3.6 ms (I) | Pass | Fallback lever if LSG misses or the sun moves fast |
| **LSG** | **Yes, byte-identical to N0** | **0.3-1.0 ms Interactive, 1.5-4 ms Exact, plus 1-3 ms per rebuild (I)** | Pass by construction | **Chosen for the first unit** |

**Why LSG is cheap where N0 was slow (I).** N0 walks the receiver's own overlapping 256 B nodes eight times per receiver, and the overlapping placement BLASes as well. In light space, the receiver's own surface mostly lies outside the 2.3° cone above it: the median cone holds 3 triangles. A max-height pyramid proves empty space in a few lookups. Neighbouring receivers query neighbouring cells, so traffic is coherent. Ideal culling would cost about 14 M (receiver, triangle) pairs per frame, 40 per receiver. The real cost is the cell overfetch and the p99 tail of 1,127 candidates. N4's CPU replay counts both exactly before any WGSL is written.

**Limits of LSG.** It is per light and sun-aligned. A moving sun is handled by tracing within a margin cone (0.04 + α rad) and rebuilding when the sun leaves it; that stays exact, at a wider cone. Local lights would need perspective grids (Hunt & Mark) or the BVH. Incoherent rays (GI, T2 validation) still need a BVH.

## 3. Primaries: N1b, priced

- **Workload (M).** Pose 0 covers 346,517 pixels and shows 321,669 distinct triangles, 0.93 per covered pixel. View distances are 7.4 / 8.3 / 10.1 m (p10 / p50 / p90), a footprint of 7.6-10.4 mm against 6.2 mm median triangles. Full detail is already about one triangle per pixel, so N1b's "finest clusters only" costs no more than Nanite's own cut here.
- **Matched yardstick (M).** On this GPU and scene, Unreal's Nanite visibility pass is 0.379 ms for 22,639 clusters and 2,849,851 rasterized triangles, with `MaxPixelsPerEdge = .25` ([full-detail comparison](unreal-quarry-full-detail-comparison.md)). That is 8.9 rasterized triangles per visible one, from cluster granularity, back faces and occlusion-culling slack. Renderlab's traced primaries cost 0.23 traversal + 0.07 fixed; software primaries cost 2.84 ms.
- **Estimate (I).** Bevy's chain (instance, BVH and cluster culling with two-pass HZB, then a software/hardware raster split into a u64 atomic visibility buffer) at the finest level should draw a similar 2-3 M triangles. Bevy's own author names culling as its bottleneck, and its raster lacks persistent threads. So: **0.38-0.7 ms**, plus about 0.03 ms for the exact barycentric recompute in shading. That is 4-7x faster than software primaries.
- **Decision.** N1b becomes the non-RT primary producer. Its gate is unchanged from [non-rt-path.md](non-rt-path.md#units-and-stop-rules): hit records equal the traced ones except classified silhouette and edge-tie pixels.
- **RT path.** N1b does not replace traced primaries unless it measures below their 0.30 ms. That is unlikely given Nanite's 0.379 ms. One primary producer for both backends would be simpler, so it is measured once on both.
- **Changes from Bevy (I):** keep `(depth, global cluster, local triangle)` in the visibility buffer and resolve it to `(placement, primitive)`; add near-plane clipping to the software raster; classify edge ties instead of adding a top-left rule; replace `clear_texture` on the R64 target with a compute clear. Copy under MIT with per-file notices.

## 4. Caster LOD for shadows

N0's 25 cm cap saved nothing (10.9 against 10.1 ms, M): almost all software shadow cost is within 25 cm of the receiver. The probe adds that 59% of blocked rays are blocked within 10 mm (M).

- **Error the gate tolerates (I).** A caster at distance t moves the shadow on the receiver by about its own geometric error δ. The sun disk's penumbra there is 0.08 t wide, and the pixel footprint is 8 mm. A δ that leaves the image unchanged must be small against both. At t = 10 cm the penumbra is 8 mm, so δ must stay well below a millimetre. At the median blocker distance, 4.6 mm, the penumbra is 0.37 mm. A cluster cut at a "sub-texel" bound of one VSM texel (5-10 mm here) is coarser than the 6.2 mm source triangles.
- **Receivers versus casters.** For receivers, LOD is already rejected: a 1 px cut moves receivers by up to 7.3 mm against the 10 µm ε ([non-rt-path.md](non-rt-path.md#decisions)). For casters alone, LOD preserves quality only where δ ≪ 0.08 t holds for the cut's δ. With a 5 mm bound that is t > 1-2 m, and N0 shows the cost is not there.
- **Verdict.** Rejected as a cost lever for in-memory content. It returns only with LOD-consistent receivers and casters for larger-than-memory scenes, as already planned.

## 5. First unit: N4, light-space caster grid

Owned files: `scripts/n4-lsg-replay.py`, `src/light_grid.rs`, `shaders/light_grid.wgsl`, `src/light_grid_tests.rs`. Branch from `origin/t3/software-bvh-traversal-probe` and reuse its harness, receivers and watertight test read-only. N1b, if run in parallel, owns `src/visibility_buffer*.rs` and `shaders/visibility_buffer*.wgsl`. Sol implements; Opus accepts.

1. **CPU replay first, no GPU.** Build the grid for the registered sun in Python:
   - 8-16 mm cells, triangles binned by light-space bbox, widened by a rounding margin;
   - a max light-depth pyramid;
   - the 65 triangles wider than 5 cm on an always-test list.

   Replay all 346,517 pose-0 receivers with 8 directions, and 256 on a 64x64 crop. Report per receiver: pyramid lookups, list entries read, triangle tests and bytes touched (mean, p99, max). Counts must match the saved hardware 8-ray and Exact counts up to N0's "edge" class, classified by f64 replay.
   - **Stop if** the mean entries read exceed 400 per receiver, or the traffic model projects more than 1.0 ms. Report the cell size that minimizes traffic.
2. **WGSL probe.** Build the grid on the GPU: count, prefix sum and scatter. The query kernel tests survivors with N0's `software_triangle` on the placement's object-space ray.
   - **Exactness gate:** 8-direction and 256-direction counts byte-identical to N0's software kernels at poses 0, 1 and return. A difference is a missed candidate, fixed in the implementation.
   - **Timing:** paired against hardware, 24 pairs, as N0. Build time is reported separately.
3. **Stop rules, frozen now:**
   - Interactive query ≤ 0.6 ms: proceed to N1 with LSG plus N1b. The frame is then at most VSM's 1.61 ms if N1b measures ≤ 0.5 ms.
   - 0.6-1.2 ms: proceed, and record that the frame is at VSM parity or worse.
   - Above 1.2 ms after at most three retained hillclimb steps: stop. Try the 8-ray packet BVH lever once; otherwise non-RT stays a quality-first mode at its measured cost.
   - Rebuild above 2 ms: a moving sun reuses the grid inside a margin cone. Report the margin's query cost at α = 0.05 and 0.1 rad.
   - Exact (256) at or below 4 ms on the full frame: propose LSG as the Exact and reference backend on both paths, as a separate decision for Islam.
4. **Expected verification:** CPU replay in under 10 minutes on one process (< 1.5 GiB), 2-3 release test builds, about 6 GPU windows of 90 s or less.

## 6. Where non-RT will lose

- **Cost against RT:** a frame of 1.2-2.2 ms (I) against 0.86 ms; the RT path stays the fast path.
- **Against VSM:** between 25% faster and 37% slower, static. VSM likely wins on a fast-moving sun if rebuilds dominate (VSM moving sun 2.06 ms, M).
- **The 2x VSM floor (0.80 ms):** missed by 1.5-2.8x. The RT path already misses it.
- **Local and many lights:** each light needs its own grid, or falls back to the BVH at 12-22x hardware.
- **Incoherent rays (GI, T2 validation, reflections):** BVH cost, 12-22x hardware (M, N0). Non-RT GI against Lumen software will likely lose on cost until a world-space cache carries it.
- **Primaries:** N1b at 0.38-0.7 ms against traced 0.30 ms.
- **Unknown:** mid-range and small-L2 GPUs (N3); 8 GB cards hold the geometry, the grid and the BVH together.

## 7. Rejected options

| Option | Reason |
|---|---|
| VSM, clipmaps and page caching as core, at any texel from 10 to 0.625 mm | §1: every row has both false and lost shadows; contacts lost at every bias |
| Kalbertodt tiled soft shadow volumes, as organized | Tile lists of 20,859 against ~30 per pixel; its light-facing cull leaks 1.7% on open scans |
| Kalbertodt's 208-cell disk grid | A different estimator from the gated 8/256 R2 directions |
| FTIZB, HFTS, per-direction IZBs | Per-pixel rotated directions; about 3-4 ms per direction (I) |
| Deep or conservative-raster shadow maps as visibility | Filtered or aliased at mm contacts; conservative binning is kept only as LSG's cull |
| Exact hard + penumbra estimator | Not the gated estimand; blockers at 1-10 mm defeat single-depth penumbra estimates |
| Non-conservative map bulk + refinement | Declares 3.5-19% of blocked receivers certainly lit |
| Caster LOD on in-memory content | Near-field error budget below the source triangle size; no far-field cost to save |
| Assuming a cull by facing on user content | One direction leaks on the quarry; the other is lossless only because of how it was scanned. Needs a per-asset check |
| Screen-space contact shadows to repair maps | AGENTS.md: no screen-space core shadows |

## Evidence and time

- **Probe:** `scripts/non-rt-shadow-probe.py`. Inputs were the R1 viewed pose-0 buffers (`E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p0`) and the RLA1 quarry source, read-only. The summary is in `E:/renderlab-work/non-rt-2/full/probe.json`, with a 4-tile smoke run in `.../smoke/`. No raw dumps were written.
- **Run:** waited about 5 minutes for 8 GiB of available memory. Smoke took 30 s; the full run took 412 s (18 s load, 13,034,903 triangles), one process, about 1.4 GB working set including the mapped asset.
- **Literature cited from prior reading, not re-read in this pass:** Hunt & Mark 2008 (ray-specialized grids); Sintorn, Eisemann & Assarsson 2008 and Johnson et al. 2009 (sample-exact soft IZB); Schwarz & Stamminger 2007 (bitmask soft shadows).
- **Time:** about 25 minutes reading, 25 minutes writing and debugging the probe, 8 minutes of runs, and 35 minutes writing this doc. There was no GPU, window, build or Unreal launch.
