# N4: light-space caster grid for non-RT shadows

2026-11-06 UTC. Unit N4 of [non-rt-path-2.md](non-rt-path-2.md) (§5), on N0's harness ([non-rt-n0.md](non-rt-n0.md)). Branch `t3/n4-light-space-grid`: `origin/t3/software-bvh-traversal-probe` (ffba3d2) with `origin/main` merged in, first adabea9 and then b6deb85 (parallel adjacency preparation). Test-only; no production file changed. Labels: **M** measured here, **I** inference.

## Result

- **Exactness: pass (M).** On the GPU, the grid's 8-ray and Exact counts equal N0's software counts byte for byte at poses 0, 1 and return, with 8 identical repeats and identical rebuilds. The CPU replay matched N0's any-hit on all 3.8 M rays it traced, at every cell size.
- **Cost: stop (M).** After one retained hillclimb step, Interactive costs **8.08 ms**. The step caps each ray at 32 entries and falls back to N0's BVH after that; it was byte-identical to N0 at pose 0. That is 19.5x hardware (0.42 ms) and 6.7x the frozen 1.2 ms line. Before the step it cost 13.58 ms, slower than N0's 10.89 ms in the same run. Per the §5 rule, N1 does not proceed on LSG.
- **Exact: 198 ms** (331-341 ms before the step), against hardware's 9.2 ms. The grid does not beat hardware Exact (8.49 ms in N0's record), so it is not proposed as the Exact or reference backend.
- **Moving sun (M).**
  - A rebuild costs 305 ms, 293 ms of it in the per-cell sort. Count and scatter alone take 11.5 ms, still above the 2 ms rule.
  - Reusing the grid inside a margin cone costs 28.1 ms (α = 0.05 rad) and 47.7 ms (α = 0.1) for Interactive, before the hillclimb step.
- **Memory (M).** 669 MB at 4 mm cells: 536 MB of 12 B entries, plus the cell ends, the pyramid and 104 MB of build ranges. It comes on top of N0's 412.5 MB BVH, which the grid still needs as its fallback.
- **Why the design's estimate missed (M plus I).**
  - Per ray, the grid does what the CPU model predicted: about 4 walk steps, 13 entries and 4 triangle tests.
  - The estimate of 0.3-1.0 ms rested on modelled bytes. Bytes did not predict time on either processor: the CPU replay ran the grid only 1.4x faster than N0's BVH.
  - On the GPU, a grid ray costs about as much time as an N0 ray while doing far less work. That points to latency: dependent, divergent loads through binding-array heaps and long-list tails holding back warps (I; no hardware counters were captured). The entry-cap step, which sends long-list rays to the BVH, is consistent with the tail part.
- **Projected non-RT Interactive frame (I): about 9-11 ms.** That is 8.08 ms of shadows, 0.45 ms of shading and tone, 0.03 ms of accumulation, and primaries of 0.4-0.7 ms (N1b, estimated) or 2.84 ms (software, M). That is 6-7x Unreal VSM's 1.61 ms, so non-RT stays a quality-first mode at its measured cost.

## What was built

- **N0 on main.** Main had merged and then reverted N0, so merging main dropped N0's code. The merge restores `src/software_bvh*.rs` and `shaders/software_bvh.wgsl` from ffba3d2, with their two module hooks.
  - Main's edge repair (d72b02c) split the hardware `primary_closest` into `primary_raw_closest`, a wrapper, and `primary_repair`, whose edge probe uses ray queries. N0's camera seam, `CLOSEST`, now removes all three. `software_bvh.wgsl` supplies software versions: N0's old closest hit (renamed), the same wrapper, and a copy of `primary_repair`. The copy's probe candidates come from `software_candidates`, and its vertices come from N0's bindings.
  - The shadow-ray seams were unchanged. No production text gained a seam. The composed module validates (naga) and binds only the probe layout. N0's camera kernel was not re-run for parity in this unit.
- **`src/light_grid.rs`**: the frame (a sun-aligned basis and 4 mm cells), per-placement rounding margins, and the CPU reference build and per-ray walk. It also holds the GPU layout and parameter block, and the WGSL composition.
- **`shaders/light_grid.wgsl`**: the query, and a build module.
  - The query is appended to N0's traced module and replaces `software_occluded` at N0's existing shadow-ray seam. The grid therefore traces exactly N0's rays: the same receivers, ε, R2 directions and light-box tmax.
  - The build runs clear, count (which stores each triangle's cell range), a three-pass scan, scatter (which reuses the stored range), a per-cell sort and the pyramid.
- **`src/light_grid_tests.rs`**:
  - CPU tests: fixtures against brute force on five placements, one with object coordinates 300-410 m from its origin as in quarry placement 11, three cell sizes and tilts; and WGSL validation.
  - The quarry CPU replay.
  - The batched GPU probe, nested under N0's tests for their loaders, bindings and paired timing.

**The grid.**

- **Binning.** Each triangle's light-space box is widened by its placement's margin: 8 ulps of the largest object-space magnitude (scaled to world units), 16 ulps of the world magnitude, and 1e-6 of the scene diagonal. The box goes into every 4 mm cell it overlaps.
- **Entries.** An entry holds the id, the widened top, and the box rounded outward to 8 bits within the cell enlarged by 1/16 on each side. Entries are 12 B, and each list is sorted by top descending.
- **Pyramid.** A max-top pyramid sits above the cells.
- **Query.** A shadow ray is a rising line in light space:
  - The walk starts at the finest level and skips any pyramid cell whose top lies below the ray where it enters.
  - In a fine cell it scans entries until the tops fall below the ray, and tests only triangles whose box the ray crosses below their top.
  - A test is N0's: the placement's world box, then the object-space ray and watertight test, with the same expressions as `software_trace`.
- **Exactness and fallback.** Any-hit is order-independent, so counts equal N0's whenever the widening covers the rounding. Rays that do not rise, that exceed 4096 steps, or that pass an optional entry cap fall back to N0's BVH. None exceeded the step limit on the quarry (M).
- **Placement in the bind group.** The grid's buffers are binding arrays of 64 MiB chunks in N0's group 2. Two wgpu rules ruled out plain bindings (§ GPU probe, item 1).

## CPU replay (M)

The replay is Rust (`cpu_replay` in `light_grid_tests.rs`), not the planned `scripts/n4-lsg-replay.py`. That way it calls N0's own f32 watertight test with `fma`, which NumPy cannot reproduce.

The inputs were the R1 viewed pose-0 receivers (346,517 covered pixels, saved before the edge repair) and the RLA1 quarry, loaded once. Rays came from `fresh_estimate`'s formula on CPU: the same hash, R2 sequence, ε and thin origins, around the registered sun, with a CPU basis and tmax at the scene box. The CPU rays are not bit-identical to the GPU's; both CPU sides trace the same rays. Each ray was compared with N0's any-hit (placement boxes, then the BVH8, on the same f32 object rays), and per-receiver work was counted.

- **Identity:** 0 differences in every run. That covers 2,772,136 8-ray shadow rays (225,223 blocked) at every cell size and start level, and 1,048,576 Exact rays (296,473 blocked) on the 64x64 crop at (819, 532). No ray fell back.
- **Runs:** one process each, 43-63 s including the 7 s load and the 18 s BVH build. Available memory never fell below 4.5 GB.

**Design iteration.** All rows are 8 rays per receiver. Work is the per-receiver mean, and p99 is in parentheses.

| Grid | Entries read | Triangle tests | Pyramid reads | Modelled bytes | Grid memory |
|---|---:|---:|---:|---:|---:|
| N0 BVH8 (reference) | 566 children | 36.9 (184) | — | 20,312 (49,376) | 412.5 MB |
| First design: 8 mm, ids and tops only, 54-triangle always list, walk from level 4 | 177 (2,063) | 169 (2,057) | 56 | 13,417 (142,668) | 217 MB |
| + 8-bit box per entry, walk from level 0 | 178 | 23.8 | 24 | 5,274 | 306 MB |
| + big triangles binned like the rest (always list removed) | 178 (2,101) | 23.8 (311) | 24 | 3,733 (42,120) | 306 MB |
| Same at 3 mm | 77 (1,004) | 26.6 (400) | 34 | 2,722 (36,528) | 736 MB |
| **Same at 4 mm (retained)** | **93 (1,166)** | **25.5 (366)** | **31** | **2,839 (35,956)** | **539 MB** |

- The per-entry box removes 86% of triangle tests. The grid then tests 31% fewer triangles than N0's BVH, with no node traversal.
- The always list cost 64 reads per receiver. Binning its 54 triangles added 5,000 entries (0.01%).
- Coarser cells cost more traffic: at 12 mm and 16 mm, the first design read 292 and 434 entries.
- Modelled bytes count 4 per pyramid read, 8 per list, 12 per entry and 60 per triangle test (three indices and three vertex reads).
- Exact crop at 3 mm: 5.1 triangle tests per ray. Modelled traffic is 562 MB, against N0's 3,372 MB.
- CPU build in f64, single-threaded: 2.3 s at 8 mm and 4.2 s at 4 mm, for 3.4 entries per triangle at 4 mm.

**Step-1 stop rule.** Mean entries read is 93, under the 400 line. The traffic model scales N0's measured 9.43 ms by modelled bytes. It projects 1.32 ms at 4 mm (1.26 ms at 3 mm), above the 1.0 ms line, so the frozen §5 rule says stop before the WGSL probe.

That projection assumes cost is proportional to bytes, which N0's limiter (divergent, stack-bound traversal) does not support. The brief for this unit asked for the WGSL probe and its timing as the deciding measurement, so the probe ran. **This is a recorded deviation from the frozen step-1 rule, for Islam's review.** The measured GPU numbers below are what the §5 step-3 rules apply to.

## GPU probe: what failed on the way (M)

Each item was found in a short lease and fixed before the batched check.

1. **Bind-group rules** (gpu-1 and gpu-2, 33-43 s each):
   - A uniform buffer may not share a bind group with binding arrays.
   - N0's layout already fills the stage's eight storage buffers, counting the lighting group's two.
   - The parameter block is therefore a one-element binding array, and the heaps are binding arrays of 64 MiB chunks.
2. **Memory** (gpu-3 and gpu-4).
   - **gpu-3.** It held four grids, each with its own N0 binding set: a 412 MB BVH copy, plus masks and dumps. In that state a readback missed N0's 2 s wait, and available memory fell from 8.6 to 1.2 GB in under a second, so the guard aborted the run.
   - **gpu-4.** It reached the 2 GiB floor during the asset upload, with BVHs alive and other workers compiling.
   - **The fix.** One binding set and one grid heap, sized for the largest grid, with each grid rebuilt into it in turn. That saved about 3 GB of GPU memory. BVHs are now built after the upload, and leases start only with 8.5 GiB available.
   - **Contention.** gpu-6's first attempt was aborted when another worker started an 8-process build.
3. **A bounded first look** (gpu-5).
   - This look runs before any full-frame grid dispatch: the 8-ray kernel with at most 64 steps and 512 entries per ray, with atomic counters.
   - A kernel runs at full frame only if under 1% of rays hit a bound. Exact runs only if 32x the 8-ray time fits a 1 s dispatch budget, which stays under Windows' 2 s GPU timeout.
   - It found 16% of rays at the step limit, so every grid kernel was skipped. The rebuild timing showed the sort taking 0.64 s at 4 mm and 1.36 s at 8 mm, and ten rebuilds changed the heap.
4. **Count and scatter disagreed** (gpu-6, a stage-by-stage heap check).
   - The two passes recomputed each triangle's cell range in separate entry points. In 704 cells the scatter inserted one entry more than the count, so lists overlapped and the sort raced: 595 unsorted lists and 570 wrong level-0 tops.
   - The count pass now stores the ranges (2 words per triangle, 104 MB) and the scatter reuses them. Entries are written without atomics, since each slot has one writer.
   - After the fix (gpu-7): 0 bad list ends, 0 unsorted lists, 0 top or pyramid mismatches, and rebuilds are identical.
5. **The walk position never advanced** (gpu-7 to gpu-9).
   - gpu-7 dumped 4,096 GPU rays that hit the step limit. Walked on the CPU, they took 5.5 steps on average (at most 15) and agreed with N0 on all 4,096.
   - gpu-8 ran the same CPU walk on the GPU-built grid read back: also 5.5 steps. The grid data was right, so the WGSL walk was wrong.
   - gpu-9 traced one ray step by step on both sides. On the GPU, `at` never changed after the first exit while t advanced, and the walk cycled between levels 5 and 6 on one cell.
   - Runtime-indexed component stores into a `vec2` local (`at[axis] = …`, `next[axis] += …`) did not take effect with naga's SPIR-V on NVIDIA 617.14. The step now uses whole-vector `select`s, and the WGSL records why.

## Batched check (M)

**Setup.** gpu-10 was one offscreen process, with the asset loaded once.

- Hardware: Windows/Vulkan, RTX 4070 SUPER with NVIDIA 617.14, Ryzen 7 7800X3D; 1920x1080, 1x, Interactive soft shadows, 0.04 rad disk.
- Build: release test binary b14, on main b6deb85 plus N4.
- Validation: no Khronos validation layer, so the timing is valid; wgpu's own validation stays active.
- Memory: it started 05:06:04 UTC with 10.7 GB available, under the standing 4 GiB start and 2 GiB in-run abort guard. Available memory never fell below 7.7 GB; at most 2 rustc processes from other workers ran.
- Duration: 112 s.

**Count identity against N0.** LSG counts equal N0's software counts byte for byte at poses 0, 1 and return. That holds for every grid's 8-ray kernel and for the untilted grids' Exact kernel, and 8 repeats at pose 0 were identical. Against hardware, LSG differs exactly where N0 does: 24 and 534 receivers at pose 0, and 29 and 542 at pose 1, all of N0's "edge" class. Ten rebuilds left the heap bit-identical.

| Grid | 8-ray counts vs N0, poses 0 / 1 / return | Exact vs N0 | Grid memory |
|---|---|---|---:|
| 4 mm, sun-aligned (base) | 0 / 0 / 0 | 0 / 0 / 0 | 669 MB |
| 8 mm | 0 / 0 / 0 | 0 / 0 / 0 | 426 MB |
| 4 mm, tilted 0.05 rad (moving-sun margin) | 0 / 0 / 0 | skipped (projected over 1 s per dispatch) | 668 MB |
| 4 mm, tilted 0.1 rad | 0 / 0 / 0 | skipped (projected over 1 s per dispatch) | 667 MB |

Grid memory at 4 mm is the front heap (cell ends, the 12-level pyramid, scan scratch and 104 MB of per-triangle cell ranges), 134 MB, plus entries, 536 MB.

**Paired timing at pose 0.** 24 pairs after 8 warmup pairs, each pair its own submission with alternating order. Times are medians [min, max] in ms.

| Kernel | Hardware | Software | Paired ratio |
|---|---:|---:|---:|
| N0 BVH, 8 rays | 0.420 [0.411, 0.850] | 10.89 [9.14, 16.59] | 23.7 |
| **LSG 4 mm, 8 rays** | 0.414 [0.407, 1.002] | **13.58** [11.60, 32.01] | 32.5 |
| LSG 8 mm, 8 rays | 0.419 | 17.67 | 41.1 |
| LSG 4 mm tilted 0.05 rad, 8 rays | 0.500 | 28.13 | 65.4 |
| LSG 4 mm tilted 0.1 rad, 8 rays | 0.573 | 47.74 | 96.6 |
| N0 BVH, Exact 256 | 8.898 | 159.6 [118.0, 253.2] | 18.2 |
| **LSG 4 mm, Exact 256** | 9.181 | **340.5** [135.7, 447.8] | 36.1 |
| LSG 8 mm, Exact 256 | 8.893 | 552.1 | 60.9 |

**Rebuild (the moving-sun cost).** Medians of 8 after 2 warmup builds, in ms:

| Grid | Clear | Count | Scan | Scatter | Sort | Pyramid | Total |
|---|---:|---:|---:|---:|---:|---:|---:|
| 4 mm | 0.01 | 2.58 | 0.08 | 8.88 | 292.97 | 0.06 | **304.7** |
| 8 mm | 0.00 | 3.45 | 0.03 | 4.38 | 472.87 | 0.03 | 481.9 |

The per-cell insertion sort dominates. It costs O(n²) for the longest lists (992 entries at 4 mm), on one thread per cell. Without it, count and scatter alone take 11.5 ms, still above the 2 ms rule.

**First look (walk counters, pose 0, 4 mm).** Per 8-ray shadow ray:

- about 4.0 walk steps (29 at most), 13.4 entries read and 4.0 triangle tests;
- 0.09% of rays read more than 512 entries.

The CPU model's per-ray work was close: 3.9 steps, 11.6 entries and 3.2 tests. **Work per ray was not the problem; GPU cost per unit of work was.** The CPU replay itself ran LSG only 1.4x faster than N0's BVH (0.5 s against 0.7 s for 2.77 M rays). The bytes model, which projected 7x less traffic, did not predict time on either processor.

## Hillclimb (M)

**The screen.** gpu-11 ran one process on the 4 mm grid at pose 0. Each variant first had to reproduce N0's 8-ray and Exact counts byte for byte, and all did (0 differences). Each was then timed against hardware in 12 pairs after 4 warmup pairs, beside the base kernels.

- **The mechanism:** an entry cap. A ray that has read more than K entries stops walking and falls back to N0's exact BVH. The first look suggested it: its bounded kernel, capped at 512 entries, ran faster than the unbounded one, so a few long-list rays were holding their warps back.
- **Conditions:** it started at 05:30 UTC with 10.6 GB available; the minimum was 7.4 GB, with no other worker compiling. Duration 80 s.
- **Selection:** the cap is the pipeline override `LIGHT_GRID_ENTRY_LIMIT`. Its default stays unbounded, so the batched check's kernels are what the code runs by default.

| Variant | 8 rays (ms) | Exact 256 (ms) | Verdict |
|---|---:|---:|---|
| Base (no cap) | 13.52 | 331.3 | — |
| Cap 16 entries | 9.21 | 214.2 | |
| **Cap 32 entries** | **8.08** | **198.4** | **Retained: 1.67x on both kernels** |
| Cap 64 | 9.06 | 289.2 | |
| Cap 128 | 9.41 | 313.9 | |
| Cap 256 | 12.74 | 295.9 | |
| Cap 64, 16 steps | 9.05 | 289.9 | No gain from the step cap |

Hardware in the same pairs: 0.41-0.43 ms for 8 rays and 8.5-9.4 ms for Exact.

**Steps two and three were not attempted.** The frozen rule allows up to three retained steps. Stopping after one is a time-box call, recorded here for Islam to overrule. The gap to the line is 6.7x. The remaining levers identified are:

- sharing one walk across a receiver's 8 near-parallel rays: an estimated 2-3x fewer entry reads, but no fewer triangle tests (I);
- loading the parameter block into registers;
- a shorter sort for faster rebuilds, which does not touch the query.

None is estimated to come near 6.7x. The frozen rule's next action, the 8-ray packet BVH lever, is outside N4's files and is proposed as the follow-up.

## Stop-rule verdicts

| Rule (§5) | Value | Verdict |
|---|---|---|
| CPU replay: counts equal N0's | 0 differences, 3.8 M rays | Pass |
| CPU replay: mean entries read above 400 | 93 | Pass |
| CPU replay: traffic model projects above 1.0 ms | 1.32 ms (bytes-scaled) | **Triggered.** Proceeded to the WGSL probe as the brief directed; recorded deviation |
| GPU exactness: byte-identical to N0, poses 0, 1 and return | 0 differences | Pass |
| Interactive at or below 0.6 ms | 8.08 ms | No |
| Interactive 0.6-1.2 ms | 8.08 ms | No |
| **Interactive above 1.2 ms after the hillclimb** | 8.08 ms after one retained step (13.58 before) | **Stop.** Non-RT stays quality-first at its measured cost; the packet-BVH lever is the follow-up |
| Rebuild above 2 ms, so the margin cone applies | 305 ms rebuild; cone at 28.1 / 47.7 ms | Recorded: a moving sun costs 2-3.5x the static query |
| Exact at or below 4 ms (LSG for Exact and reference) | 198 ms, against hardware 9.2 ms | No; not proposed |

## Evidence and time

- **Evidence** is in `E:/renderlab-work/n4/`:
  - `runs/replay-1` to `replay-3`: CPU replay JSON.
  - `runs/gpu-1` to `gpu-11`: summaries, logs, preflights and memory samples.
  - `builds/`: test binaries. gpu-10 ran `n4-tests-b14.exe` and gpu-11 ran `n4-tests-b15.exe`.
  - `gpu-windows.json`: the lease ledger.
  - `lease.py`, `run.py` and `ue_gate.py`: the lock, the memory guard and the UE-reservation gate.
  - No raw dumps were kept; the heap readbacks were checked in process.
- **Verification time:**
  - GPU: 12 leases took 665 s in total, the longest 112 s (gpu-10). Every lease was released. That includes two bind-group failures, two memory-floor aborts, gpu-5 (whose first look withheld every grid kernel), and four diagnosis leases.
  - CPU: three quarry replays took 43-63 s each. Release builds took 1-2.5 min each (about 15). Debug lib tests passed by exit status on the final tree: 103 passed.
  - Waiting: about 2 h for the GPU lock and the s2-s3 UE reservation (03:08-05:06 UTC). During it, n4 ran no GPU job or build.
- **Implementation** ran from about 00:50 to 05:45 UTC, interleaved with the checks. Editing time was not isolated.
- There was no window, viewer, game capture or Unreal launch.
