# M7-G1d: where the quarry's cache cost went, and cutting it

Unit G1d continues [M7-G1c](m7-g1c.md) on branch `t3/m7-g1d` (from main `41b01d1`; main merged at `0362cc8` after MAT1 and R0a landed). Contract: [milestone-7.md](milestone-7.md). Claim constraints: [m7-p0-claims.md](m7-p0-claims.md) W1-W3, W8, C1-C5 and C8. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

The cache stays **off by default** (`IndirectSettings::enabled`), as at G1-G1c.

## Status

QUIET_STATUS_TABLE

**What changed (all exact or quality-identical; section 3):**
1. **Pixel-driven parents.** The probe requested four parent cells around every found orphan. On the quarry's scans that listed about as many parent cells as pixel cells (123k against 133k a frame), filled 512-entry tile tables (about 2,200 dropped inserts a frame) and made inserts walk 2.3 probe steps each. Pixels now walk what their own lookup will read: the coarser level only where their level has no value but an orphan, and the finer level under split cells once. Parents in tile sets: **123k → 3.7k a frame**; overflows **2,243 → 0**; bucket-full claims **706 → 87**.
2. **8x8 probe tiles** (256 entries). The probe is latency- and barrier-bound on the quarry (Nsight, section 4); tiles of two warps instead of eight read it about 8% faster there, unchanged on SV.
3. **A merged world BVH for the non-RT near field.** While every placement of a mask-free cooked near field is drawn with its cooked transform, software rays trace one world-space BVH over their cuts instead of descending each of the twelve overlapping scans' BVHs in turn: setup and rays about 20% faster in motion, 2-3 ms off each quarry cut frame, +1.8 MB.

**Quality (M, final code `g1d-h`, both paths): unchanged.** SV populations equal G1c's to 0.01 points and the overview images are bit-identical; the quarry's constant-sky, map and full-image scores equal G1c's (section 6).

**Gates (M):** functional, determinism, non-caller, caster roll, staleness, environment steps, the SV pan and the quarry gates pass on both paths; `cargo test -p renderlab --lib` passes (138) inside a lease; fmt and clippy are clean in G1d's files (section 7).

## 1. Per-stage cost on the quarry, before (M)

**Harness.** `gpu_gi_quarry` gained `RENDERLAB_M7_G1_QUARRY_BREAKDOWN=n` and the SV test a `breakdown` section (`motion_breakdown` in `src/gi_cache_tests.rs`): from a settled cache, n camera-moving frames at 1 cm a frame along x, four passes over the same frames:
- **counters**, read back after each frame (the kernel counters plus new probe statistics, below);
- **profiled**, per-pass times from the timestamp ring;
- **two unprofiled visits**, the frame span and the shading pass. The increment is cache-on minus cache-off span p50; the lookup is the same for the shading pass.

**Probe statistics** (`IndirectCounters::probe`, `GI_W_P_*`, counted on profiled frames only): tiles, covered pixels, tile-table inserts and their probe steps, unique cells from pixels / added for parents / added by refinement, finds and how many found a cell, refinement walks, the most unique cells in a tile, and tiles by unique cells.

**G1c's code on the quarry, gate program under the RLE2 map, pose 0 moving (functional lease b1, not a quiet window):**

| | RT | Non-RT (cooked cut) |
|---|---:|---:|
| Camera-moving increment | +1.21-1.27 ms | +1.27-2.19 ms (noisy) |
| Lookup (shading with the cache, less without) | 0.08 ms | 0.09 ms |
| Probe | **991 µs** | **968 µs** |
| Claims and resolves | 26 µs | 26 µs |
| Setup / rays / finish | 33 / 23 / 50 µs | **496 / 267** / 51 µs |
| New cells a frame (mean of 32; last half) | 2,181 (1,089) | 2,104 (1,067) |
| ...of which orphans (last half) | 751 | 757 |
| Unique cells in tile sets: pixels / parents / refined | 133k / **123k** / 20k | 133k / 124k / 17k |
| Finds | 277k | 274k |
| Probe steps per tile-table insert | **2.26** | 2.06 |
| Refinement walks | 510k | 441k |
| Tile-table overflows (dropped inserts) | **2,243** | 1,734 |
| Bucket-full claims | 706 | 596 |

The same run on SV: RT +0.48 ms, non-RT +0.52-0.54 ms; probe 227-231 µs, lookup 0.20 ms, 440 new cells a frame, 18k parents, no overflow, 1.03 steps per insert.

**Where the quarry's cost goes:** the probe is about 80% of RT's increment and half of non-RT's; non-RT adds software setup and rays for new cells. The lookup is small on the quarry (it covers about a third of the screen). The whole-image runs (standard program, sun shadows) read the same probe and cell counts (b1 `qf-*`).

### 1.1 How many cells become new per moving frame

New cells per frame decay from the first moving frame: 11.4k, 7.8k, 5.7k ... 0.8k by frame 23 (RT), against SV's roughly constant 300-450. A camera translation shifts every pixel's world point by about a centimetre; where cells are smaller than a pixel on screen (grazing scan faces: the cell edge is 4 px of footprint at the receiver's distance, foreshortened on the surface), pixel samples visit a sparse subset of cells, and moving keeps sampling new ones until the visited set fills in. The variants confirm it: levels that track the footprint on the bin's tangent plane halve the creation rate, and 8 px cells cut it to a third (section 3). About 68% of the new cells are orphans: bilinear corners in the point's layer where a sloped scan surface has already left that layer.

**Footprint-tracking levels (with or without hysteresis) are not kept:** they cost quality (section 3). Coarsening at grazing angles must coarsen whole cubes (C3), so the unforeshortened direction loses resolution too: quarry map RMS 30.85% → 34.07%.

## 2. Rays per cell, probe work per new cell, and the lookup

- **Rays per cell:** 64 fixed directions, unchanged. On RT the rays pass is 13-23 µs a moving frame; on non-RT 190-270 µs, bounded by the slowest software rays rather than their number (it barely moved when new cells halved; section 4.2).
- **Probe work per new cell (setup):** RT 18-33 µs a moving frame; non-RT 230-500 µs, also a latency tail (the five probe lines and their inside tests run in parallel lanes; the critical path is a few unbounded closest-hit software traversals). Lazy inside tests (resolve in line order) made it slower (section 3).
- **The lookup:** 0.07-0.08 ms on the quarry, 0.16-0.20 ms on SV. Not the quarry's problem; unchanged.

## 3. Variants (M, functional leases b2-b14; RT unless noted)

Probe µs is the profiled p50 (statistics on); increments are unprofiled visits. Quality is G1's estimand on the quarry against the 2,048-spp RLE2-map reference (poses 0 / 1; base 30.85 / 30.83%).

| Variant | Probe | Increment | New cells (last half) | Quality | Verdict |
|---|---:|---:|---:|---|---|
| G1c code | 967-991 | +1.14-1.29 | 769-1,089 | 30.85 / 30.83 | — |
| r1: refinement walks in round 0 only (later rounds re-insert the same cells) | 838 | +0.93-1.01 | same | identical | Exact; subsumed by p1w |
| **p1: pixel-driven parents** | **456-471** | **+0.61-0.64** | 666 | **30.85 / 30.82**; sky 18.44 / 17.61; SV identical | **Kept** |
| **p1w: p1 with walks resumed where they stopped** | 471 | +0.61-0.66 | 666 | identical to p1 | **Kept** (walks 560k → 337k; same time) |
| fp50 / fp25: levels track the footprint on the bin's tangent plane (cosine clamp 0.5 / 0.25) | 491 / 472 | +0.61-0.69 | 176-363 | **34.07 / 34.62** (fp50) | Rejected: quality |
| p1 + fp50 | 414-436 | +0.51-0.54 | 183 | 34.07 / 34.62 | Rejected: quality |
| 8 px cells (diagnostic) | 460 | +0.55-0.62 | 121 | image 70% RMS from base | Not a candidate (G1: fails SV gaps) |
| p1sfw: bilinear corners follow the surface slope (layer from the pixel's normal) | 454 | +0.66-0.68 | 985 (orphans 678) | **31.24 / 31.18**; sky 18.29 / **16.27**; SV contact 1.78 (base 1.86) | Rejected: map regression and 40% more cells. Recorded: better under a constant sky |
| Lazy setup: inside tests in line order for queues of 4,096 or more | — | — | — | identical | Rejected: non-RT cut setup 6.8 → 9.3 ms; RT motion setup 24 → 59 µs (occupancy) |
| nl: no append list (rounds scan the tile table) | 473 | — | — | — | Rejected: no gain |
| dd: skip inserts a left or upper neighbour makes | 445 | — | — | — | Rejected: no clear gain |
| **t8-256: 8x8 tiles, 256 entries** | **433-437** (base 468-477) | +0.57-0.65 (base +0.61-0.67) | same | RT image bit-identical to a base run | **Kept**: quarry about −8% probe; SV probe 204 against 192-208 |
| t8-128 | 491 | — | — | overflows (image 4% RMS from base) | Rejected |
| **world: merged world BVH (non-RT)** | — | — | — | non-RT map 31.21 / 30.91, sky 18.93 / 17.70: identical | **Kept**: setup 279 → 227, rays 193 → 150 µs in motion |

**Attribution diagnostics** (images change; b11, interleaved, same lease): without seen stores the probe reads 451 µs (−20), without walks 439 (−32), with one corner per pixel instead of four 433 (−38), against 468-473. No single phase dominates the remaining probe.

**Interleaved screen (b11):** base 468-473 / t8 433-437 µs on RT across three alternations; non-RT 473-477 / 433-434.

## 4. Profiles (M, Nsight GPU Trace, quarry in camera motion)

Headless GPU Trace (real-time shader profiler, 100-200 ms windows, `RENDERLAB_M7_G1_QUARRY_TRACE`), unprofiled frames, final probe (p1w, 16x16 at the time). Traces in `runs/b7` (24 cm wrapped motion: every cell exists, so the probe and lookup alone) and `runs/b9` (300-frame sweeps: cells are created all the time).

### 4.1 RT

Per frame, wrapped motion: probe **478 µs**, shading 379 µs, primary trace 240 µs, claims 17, finish 13, setup 9.5, rays 6 µs. Forward sweeps: probe 391, finish 44, setup 18, claims 16, rays 13 µs.

**Probe metrics:** SM throughput 16.6%, L2 18.5%, VRAM 29%; warp occupancy 79% (launch register-limited 23% of the time); 17.8 of 32 lanes active. It is neither compute- nor bandwidth-bound.

**Stalls:** barrier 33%, MIO throttle (shared-memory atomics) 22%, short scoreboard 15%, long scoreboard 15%. By phase: barrier waits about 32% (lanes idle while others do a round's finds or walks), receiver decode about 16% (three vertices' positions and normals per pixel from the 13M-triangle scans), tile-table inserts about 13% (compare-exchange, priority minimum, one shared append counter), the table finds about 3%. Smaller tiles (fewer warps per barrier) were the measurable lever; the append counter (nl) and fewer inserts (dd) were not.

### 4.2 Non-RT

Forward sweeps, per frame: probe 440 µs, setup **266**, rays **247**, finish 47 µs. Setup samples: barrier 13.5%, software BVH traversal about 25% (triangle tests, node slabs, stack), the queue-length read 14%. Every software ray loops over all twelve scan placements and descends each overlapping one's BVH; that is what the merged world BVH removes.

## 5. Cuts and the 8-frame settle rule (item 5 of the brief)

QUIET_CUT_TABLE

**Amortisation.** The non-RT refill spreads rays over four direction groups; setup (query points for about 90k cells, 42% orphans) and finish (the SH of every partial cell each refill frame) are not spread. Fewer parent cells (p1w) and the merged world shorten every cut frame. An eight-group refill (the most the settle rule allows) is measured in section 5.1.

SPREAD8_SECTION

**Not tried:** spreading new cells' setup over key-hash phases, as G1c proposed. A cut makes every visible cell new; cells without a query point have no value, and their parents are new too, so the first frames would read the open sky where G1c reads 6.82% / 9.98% on SV. That is below the floor.

## 6. Quality (M, final code `g1d-h`; G1c's tables are the floor)

**SV** (`runs/g2/gates-a`, both paths): every population of the default (rotated) equals G1c's to 0.01 points: frame 1.95 / 2.75, open 1.77 / 2.02, crease-90 2.71 / 2.74, underside 2.62 / 3.43, gaps 3.73 / 4.37, contact 1.86-1.87, edges 15.26 / 17.76. The overview images are bit-identical to G1c's (RT `db3f7832535ddb9f`, non-RT `ba208a2b204c6a04` for the unrotated configuration; the rotated and refill images too). The gaps pose differs in pixels whose parent fallback changed, with identical scores. Motion: pan residual 6.14 / 6.45 (refill cold start 8.40 / 15.85), equal to G1c.

**Quarry, G1's estimand (poses 0 / 1):**

| | Constant sky (1,024 spp) | Map sky (2,048 spp) | G1c |
|---|---:|---:|---:|
| RT | 18.43 / 17.61% | 30.85 / 30.82% | 18.44 / 17.61; 30.85 / 30.82 |
| Non-RT, cooked cut (merged world) | 18.93 / 17.71% | 31.21 / 30.91% | 18.93 / 17.73; 31.21 / 30.91 |
| Non-RT, full-detail software BLAS | 18.74 / 17.62% | — | 18.74 / 17.61 |

**Quarry, whole image against Lumen's floors** (`analysis/g5/full-score.txt`, raw RMS in F0 codes at −14 / −10 stops, pose 0): all-covered RT on 14.226 / 22.032 (G1c 14.224 / 22.030), non-RT on 13.354 / 21.081 (G1c 13.349 / 21.072). Every population is within 0.03 codes of G1c's, the size of the quarry's run-to-run spread (two RT processes of the same code differ by 0.24% RMS in a few pixels; section 9). Still a miss against Lumen HW Epic (8.41 / 11.65), upstream of G1 (MAT1).

**Settle.** After a cut on the quarry, new cells a frame read 20.4k, 875, 0, 0 ... (G1c: 56k, 17k, 18, 5 ...), and the table holds 111.5k live cells (G1b: about 145k): parents are only those a pixel reads.

## 7. Gates (M, final code)

| Gate | Result |
|---|---|
| Functional (`gates-a`: quality, gates, caster; 10 configurations) | **Pass**, both paths (`runs/g2`) |
| Determinism (`gates-b`: repeat, serial and table hashes) | **Pass**: image and table repeats equal, serial equal, both paths |
| Non-caller | **Pass**: no cache frames added, toon image equal |
| Caster roll and staleness | Rolled equals fresh; staleness 0 pixels at age 9, both paths |
| Environment steps (six 15° yaws) | Every step equals a fresh cache (0 pixels); still to fresh in 7 frames, both paths (`runs/g3`) |
| Motion (9-frame SV pan) | 6.14 / 6.45% mean / max, both paths |
| Quarry, constant sky / map / full image, both paths | Section 6 (`runs/g3`, `g4`, `g5`) |
| `cargo test -p renderlab --lib` (inside a lease) | **138 passed, 0 failed**, exit 0 (`runs/g5/lib`) |
| fmt, clippy | FMT_CLIPPY |
| New unit test | `world_merges_placements_and_keeps_winding` (placements in world space; a mirrored placement keeps the source-winding normal) |

## 8. Memory (M)

| Item | G1c | G1d |
|---|---:|---:|
| Table | 92,274,944 B | 92,274,944 B |
| Work buffers | 35,913,792 B | 35,913,856 B (16 more counter words) |
| **Cache on SV** | 122.3 MiB | **122.3 MiB** |
| Non-RT quarry geometry | 38,802,704 B (cut BVHs per mesh) | **40,608,864 B** (one merged world BVH; per-mesh BVHs are packed only when an instance leaves its cooked transform) |
| **Non-RT quarry total** | 159.3 MiB | **161.0 MiB** |

Against Unreal, as G1c: RT 2,641 + 122.3 = 2,763 MiB against Lumen HW Epic's 6,444; non-RT 3,503.7 + 122.3 + 38.7 = 3,665 MiB against Lumen SW Epic's 5,142. Both stay under. The merged world adds a one-time 1.4-1.7 s CPU build at `set_indirect_near_field` on the quarry.

## 9. Claim-avoidance notes (engineering reading, not clearance)

- **W2 (US 11,790,594).** Pixel-driven parents: a pixel requests a coarser level because the cells its own lookup reads have no value, never because of a cell's age, distance or visibility; requested cells are created and traced the frame they are requested. No cell is ranked against another. The query point is still fixed by the key alone.
- **W3 (US 12,423,782).** Lookups are unchanged: fixed bilinear weights, the deterministic coarser-level fallback (P0 names it fine).
- **W1, C8.** No change to values: replaced, never counted or averaged.
- **W8 (US 10,380,785).** The merged world BVH is one global structure traversed from the top per ray; no hits are registered per cell or brick and no rays are launched per node.
- **C4.** Tile tables and claims keep compare-exchange and atomicMax with winner writes. 8x8 tiles change only how pixels are grouped; the requested cells and minimum priorities are the same.
- **C1-C3, C5.** No resampling; positions unprojected; cubic cells; no per-pixel source switching.

## 10. Misses and open items

MISSES

## 11. Evidence and process

**Retained under `E:/renderlab-work/m7-g1d`.**
- `runs/b1`-`b14`: per-stage breakdowns, variant screens, cut screens and traces (`b7`, `b9`: `.ngfx-gputrace`); `runs/g2`-`g5`: final gates; `runs/QUIET_LABEL`: the quiet window, with `samples.json` and `classification.json`.
- `analysis/`: trace analyses (`b7-rt-probe`, `b7-nonrt-setup`, `b9-*`), full-image scores (`g5`).
- `variants/`: every shader variant above; `scripts/`: `mk_p1.py`, `mk_w.py`, `mk_fp.py`, `mk_sf.py`, `mk_lazy.py`, `mk_probe.py`, `mk_diag.py` (variant generators), `port_p1w.py`, `port_world.py`, `table.py`, `quality.py`, `cmp.py`, `classify.py`, `quiet-poll.py` (now samples every foreign compiler and RAM), `ram-lease.py`, `metrics.py`, `fullscore.py`.
- Deleted at close (hashes in `deleted-raw.txt`): the Cargo target, staged binaries, raw image dumps, exact images and cached references.

**Code.** `shaders/gi_cache.wgsl` (probe statistics, pixel walks, 8x8 tiles), `shaders/gi_cells.wgsl` (`GI_F_STATS`), `shaders/gi_rays_software.wgsl` (comment), `src/gi_cache.rs` (`ProbeCounters`, 32 counter words, `World`, probe tile), `src/gi_cache_tests.rs` (breakdowns, trace modes, `spread8-4px`, cut settings), `src/lib.rs` and `src/renderer.rs` (exports), `scripts/m7-g1d-run.py`.

**Reproduce.** `py -3 -B scripts/m7-g1d-run.py build --as NAME`; plans as in G1c (`m7-g1d-run.py plan <label> plan.json`). `gpu_gi_quarry` adds `RENDERLAB_M7_G1_QUARRY_{BREAKDOWN,TRACE,SWEEP,TRACE_CUTS,POSES,NOREF}`; the SV test adds the `breakdown` section; test builds read `RENDERLAB_GI_PROBE_TILE` beside `RENDERLAB_GI_SHADERS`.

**Process.**
- Every GPU executable ran inside a g1d lease holding `gpu-slot.lock`; CPU-only tools (naga checks, unit tests, trace analysis) were announced in `gpu-schedule.md`. No game hold was in force.
- Five functional leases were stopped by the shared guard and rerun: available RAM fell below 2 GiB under other owners' release builds and processes (b4, b6, b8, g2, g4), and m7-es1's CPU-only renderlab test binaries counted as foreign renderers (b12, b13, g3; a note asks them to run outside leases).
- Functional leases overlapped foreign compilers and GPU work: cut frames there varied up to 13x between processes of the same code, and non-RT visits by 0.25 ms. Every cost claim comes from the quiet window, classified step by step.
