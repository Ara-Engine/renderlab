# N8m: the exact lookup's occupancy limiter, found and removed

**Follow-up: [N8n](non-rt-n8n.md)** keeps the exact walk's counts per receiver surface cell across frames (`VirtualLookup::Surface`, opt-in then; default since [N8p](non-rt-n8p.md)). Camera-moving frames fall 31% (1.49-1.53 → 1.03-1.04 ms) and static frames to nothing, above Unreal VSM on every quality measure and on flicker in motion; camera motion still misses VSM's estimated ~0.5 ms.

Follow-up unit to [N8l](non-rt-n8l.md), 2026-11-07 UTC, base main `a6d773e`, merged with main `311075c` ([N8k](non-rt-n8k.md)'s map and hybrid lookups), branch `t3/n8m-occupancy`. Brief (Islam, 2026-11-07): keep cutting the non-RT virtual-shadow exact lookup on camera-moving frames (no-RT quarry 1.96-1.98 ms against Unreal VSM's estimated ~0.5 ms). N8l found it memory-latency bound, at about 7.5 resident warps per SM where registers allow 12, limiter unknown. Find the limiter with Nsight counters, raise occupancy, measure every step, output byte-identical to main. Addendum (root, mid-unit): N8k merged; time its Hybrid (`march_samples` 6, walk cap 16) beside Exact, and report its queued exact pass if it moves. Labels: **M** measured here, **I** inference.

## Result

**Kept: a persistent, heaviest-first lookup schedule.**
- **Camera-moving frames fall 22-23%** on the no-RT quarry: 1.93-1.96 → 1.50-1.51 ms. On the RT device's traced quarry they fall 20%: 1.97-1.99 → 1.58-1.59 ms.
- **Caster-moving frames fall 9-12%.** Static and turning frames are unchanged.
- **Output is main's, byte for byte**, on every hash and score this unit checks.
- **Exact now costs what N8k's Hybrid costs on camera frames** (1.50-1.53 ms), at Exact's quality.
- **Hybrid's static frames fall about a quarter** (0.81-0.85 → 0.60-0.67 ms) and its turning frames 23% (0.81-0.82 → 0.62-0.64 ms), through a second, adaptive change to its queued pass. Its camera frames are unchanged.
- **Camera motion still misses VSM's estimated ~0.5 ms by about 3x.**

- **The limiter is the launch, not a resource.**
  - *Static limit: 12 warps per SM.* 168 registers per thread allow 3 warps per SM sub-partition (16,384 registers each), so 12 per SM. The compiler spills the rest of the live state into shared memory up to the same limit: 8,448-8,460 B per warp, 12 × that within the 100 KB carve-out. 6,144 B of it are N8l's tables.
  - *Achieved: 8.5-9.3 warps, then a tail.* The lookup launched one 32-thread workgroup per 8x4 tile. A finished warp's slot stays empty until the next workgroup launches, so the SMs never reach 12. In the last 15% of the dispatch the slowest tiles, which happen to start late in launch order, run nearly alone.
- **The fix: persistent workgroups, a tile queue, and the heaviest tiles first.**
  - The encoder launches 4,096 workgroups. Each loops, taking the next tile from an atomic queue once its last one is done.
  - Each tile's heaviest lane records its work (ring tests plus walk visits). Before the next lookup, two small passes (2.8 + 4.3 µs, aggregated per workgroup) bucket the tiles by log2 of that work. The queue then hands them out heaviest first.
  - The SMs now hold exactly 12.00 warps for 95% of the dispatch, at 43% SM throughput against 32%. The lookup dispatch is 24% shorter in the same lease: 1,834-1,893 → 1,394-1,419 µs.
  - Output is unchanged by construction: every pixel's count is the same whichever warp computes it. Measured below.
- **The compiler chooses an occupancy target, and small edits move it.** These were byte-identical but slower:
  - claiming the next tile before running one: 10,240 B of shared memory, 10 warps;
  - broadcasting the tile index through a table slot: 224 registers, 8 warps;
  - ordering inside the grid launch: 216 registers, 8 warps;
  - `vsm_own` in workgroup slots: 8,704 B, 11 warps.

  The kept form claims a tile only after finishing one, and keeps 168 registers and 8,448 B: 12 warps.
- **Register cuts do not pay.**
  - Moving `vsm_last` or `vsm_own` into explicit workgroup slots leaves registers at 168: the compiler holds the 12-warp budget and spills less.
  - Going beyond 12 would need at most 128 registers and 6.4 KB of shared memory per warp, against 168 and 8.4 KB now.
- **Hybrid** (N8k, `march_samples` 6, cap 16):
  - The new schedule does not reach it: its exact work runs in its own queued pass. On camera frames that pass is one partial wave (9 warps per SM decaying to 0, 4.4 of 32 lanes active), set by the heaviest receivers' dependent chains: 527-566 µs on both binaries.
  - On rolling frames the queue is about 1/16 as long, too few warps for the SMs, each carrying 32 divergent walks. The queue-argument pass now spreads such a small queue over up to 8 lanes per receiver, and keeps one lane each above 8,192 receivers.
- **Not kept, measured:**
  - 48-byte records with inline neighbours: −2.5% for +149 MiB;
  - the heaviest tiles split into four row warps: no gain, and again a shifted compile (224 registers, 8 warps);
  - heaviness thresholds of 64 and 256: slower;
  - the static-stride and unordered queues (step table below).
- **Memory:** the scratch buffer grows from 16 B to 0.5 MiB; allocator reserve is unchanged. Programmability is unchanged.

## Timing (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off, 1920x1080, poses 0 and 1.
- **Binaries.** Main is `311075c`, built from a detached worktree with its own Cargo target (`main.exe`). Final is this branch's committed source (`fin.exe`). They run as separate processes interleaved in one lease (final, main, final, main), each running all its variants in one process.
- **Clean visits.** A visit is clean when its primaries read at most 0.45 ms (no-RT); ranges are clean visits' medians over poses and visits. One main visit (pose 1, second Exact) read 0.72-0.74 ms static and 2.10 ms camera with clean primaries; it is excluded as contended.

**No-RT quarry, LOD 0.35 px, shadow pass, ms (final lease)**

| Configuration | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| Main, Exact (N8l's lookup) | 0.564-0.581 | 0.574-0.591 | 1.927-1.959 | 1.871-1.962 | 1.28-1.32 / 2.65-2.72 / 2.60-2.70 |
| **N8m, Exact (shipped)** | **0.561-0.586** | **0.569-0.604** | **1.495-1.512** | **1.700-1.729** | **1.30-1.34 / 2.21-2.28 / 2.42-2.49** |
| Main, Hybrid (N8k) | 0.806-0.846 | 0.808-0.819 | 1.503-1.532 | 2.371-2.406 | 1.53-1.60 / 2.24-2.35 / 3.09-3.25 |
| **N8m, Hybrid (adaptive spread)** | **0.599-0.673** | **0.616-0.636** | **1.501-1.526** | **2.364-2.415** | **1.33-1.42 / 2.25-2.32 / 3.09-3.22** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

The same lease ran two intermediate steps in the final process:
- the grid launch on this binary: 0.567-0.608 / 0.576-0.591 / 1.948-1.962 / 1.875-1.947;
- the unordered queue: 0.552-0.568 / 0.563-0.583 / 1.707-1.726 / 1.696-1.750.

Hybrid with N8k's one lane per receiver read 0.801-0.824 / 0.809-0.831 / 1.503-1.533 / 2.381-2.412.

**Traced quarry on the RT device** (validation off, final lease, ms, static / turning / camera / caster):

| Primaries | Main | N8m |
|---|---|---|
| Traced | 0.520-0.544 / 0.535-0.552 / 1.967-1.991 / 1.923-1.965 | **0.523-0.530 / 0.538-0.548 / 1.582-1.592 / 1.723-1.743** |
| LOD 0.35 px (RT device) | 0.559-0.594 / 0.572-0.583 / 1.965-1.969 / 1.917-1.964 | 0.568-0.585 / 0.575-0.584 / 1.520-1.539 / 1.731-1.752 |

The validation-on traced screens (static 0.72 / 0.55 ms, camera 1.71 / 1.97 ms, final / main) are used for hashes and agreement only, as in N8i and N8l.

## The occupancy limiter (M)

Nsight Graphics 2026.4.0 GPU Trace, Top-Level Triage, N8g's recipe: no-RT quarry pose 0, `move_wrap` 48 mm, four submits from 45 s. Clocks unaltered, no screenshot, no window.
- **Derived metrics.** Registers per thread are CS register allocation ÷ (resident warps × 32). Shared memory per warp is shared memory allocated ÷ resident warps.
- **Camera motion only.** With 8,000 wrapped camera frames, several traces of leases P5-P8 and the final lease started after the camera loop had ended, in caster motion (`vsm_raster` 0.3-0.55 ms per pass). They are not used for camera-frame claims. P9 traced with 30,000 frames; every trace below has `vsm_raster` under 11 µs.

**Lookup dispatch, camera-moving frames, before and after (P9, one lease)**

| | Main (grid launch) | **N8m (persistent, ordered)** |
|---|---:|---:|
| Duration (multi-pass / real-time) | 1,834-1,893 / 1,830-1,862 µs | **1,394-1,419 / 1,401-1,420 µs** |
| Tile order passes | — | 2.8 + 4.3 µs |
| Resident warps per SM, average | 7.52-7.74 | **11.47-11.70** |
| Resident warps over the dispatch (20 buckets) | 8.5-9.3 for 85%, then 2.3, 0.5, 0.1 | **12.00 for 95%**, then 5.9 |
| Registers per thread / shared per warp | 168 / 8,453 B | 168 / 8,441-8,448 B |
| SM / L2 / VRAM throughput | 31.4-32.2 / 9.0-9.3 / 12.6-13.3% | **42.5-43.3 / 18.1-18.4 / 18.0-18.9%** |
| Active threads per warp / L1TEX hit rate | 8.5 / 71% | 8.9 / 66% |
| Stalls: wait / long scoreboard / selected | 33.3 / 23.0 / 17.4% | 29.3 / 28.9 / 14.8% |
| Branch / short scoreboard / no instruction | 7.3 / 7.0 / 4.7% | 6.5 / 6.3 / 4.9% |
| Hottest functions (self samples) | `vsm_tri_in` 6.5, `vsm_hits_mask` 6.3, `vsm_hits_sheared` 5.3, `vsm_nbs` 4.9, `vsm_corner` 4.8% | `vsm_nbs` 9.0, `vsm_tri_in` 7.3, `vsm_march` 6.6, `vsm_hits_mask` 5.3, `vsm_hits_sheared` 4.6% |

- With 12 warps resident, the same latency-bound code keeps a third more loads in flight. Long-scoreboard waits are a larger share of samples, and L2 and VRAM throughput double, but the dispatch is a quarter shorter.
- The order costs some locality: L1 hits fall from 71% to 66%, since heavy tiles from the whole image run together.
- The hottest code is now the walk's record and neighbour loads (`vsm_nbs`, `vsm_tri_in`): the dependent chains N8l named.

**What sets residency, variant by variant** (multi-pass traces inside camera motion; every row byte-identical to main; camera times from the no-RT harness, clean visits):

| Variant | Registers | Shared per warp | Resident warps per SM | Camera ms |
|---|---:|---:|---:|---:|
| Grid launch (main) | 168 | 8,453 B | 8.5-9.3, then the tail | 1.93-1.99 |
| Queue, next tile claimed before running one (P1) | 168 | **10,240 B** | 10.00 (shared-limited); 35% tail | 2.08-2.11 |
| Queue, tile index through a table slot (P2) | **224** | 6,144 B | 8.00 (register-limited) | contended |
| Grid launch, workgroup k takes the k-th heaviest tile (P4) | **216** | 6,400 B | 8.0 falling to 4 | 1.87 (one clean visit) |
| **Queue, claimed after each tile, ordered (shipped)** | **168** | **8,448 B** | **12.00** | **1.50-1.51** |
| Shipped + `vsm_last` in workgroup slots (P5) | 168 | 8,448 B | 12.00 | 1.54 (contended) |
| Shipped + `vsm_own` in workgroup slots (P5) | 168 | **8,704 B** | 11.00 (shared-limited) | 1.62-1.68 |
| Shipped + both (P5) | 168 | 8,703 B | 11.00 | 1.60 |

- **The static limit is 12 in every variant that keeps it.**
  - Registers: 168 per thread is the most that fits 3 warps in a sub-partition's 16,384, so 12 per SM.
  - Shared memory: 8,448 B × 12 = 101,376 B, inside the 100 KB carve-out.
  - The compiler spills what does not fit in registers into shared memory, about 2.3 KB per warp beyond the 6 KB of tables (main has the same 8,453 B). Explicit workgroup slots just replace spill.
  - Variants that grow either resource lose warps: 10,240 B allows 10, and 224 registers allow 2 per sub-partition (8).
- **The grid launch never fills it.**
  - N8g's records trace showed the first samples at 11.3 warps, then about 9: one-warp workgroups finish at different times, and their slots wait for the next launch.
  - The queue removes the wait, because a warp takes its next tile itself.
- **The tail is the order of the work.**
  - In launch order, the slowest tiles can start last. The first queue (unordered, claiming early, P1) kept a tail of 35% of its dispatch at 2 warps or fewer.
  - Heaviest first, the SMs stay full until the last 5%.

## Every step measured (M)

No-RT quarry, clean visits' medians over both poses, shadow pass ms. Steps P1-P6 compare with main's own shader on the same binary (`RENDERLAB_VSM_SHADER`), P7 on with `main.exe`. Every row is byte-identical to main at both poses, with 0 differing pixels in the caster identity gate, unless noted.

| Step | Change | Static | Camera | Caster | Verdict |
|---|---|---:|---:|---:|---|
| Base (P6) | main's lookup | 0.570-0.586 | 1.968-1.990 | 1.866-1.938 | base |
| Grid (P6) | this binary, one workgroup per tile | 0.569-0.604 | 1.979-1.998 | 1.881-1.975 | = base |
| P1 queue, early claim, 1,344 groups | persistent; next tile claimed before running one | 0.618-0.715 | 2.075-2.114 | 1.753-1.798 | rejected (10 warps, longer tail) |
| P1 static stride, 1,344 groups | every 1,344th tile, no queue | 0.581-0.585 | contended | 1.955-2.040 | rejected |
| P2 table-slot broadcast | P1 without the tile variable | — | contended | 1.744-1.762 | rejected (224 registers) |
| P4 ordered grid | workgroup k takes the k-th heaviest tile | 0.562-0.565 | 1.874 (one clean) | 1.97-2.05 | rejected (216 registers) |
| P4 ordered queue, early claim | — | — | — | — | **wrong output**: a `select` claimed two tiles per loop, and the identity gate caught it (671-674 pixels). Early claim dropped |
| P6 queue, unordered | claimed after each tile | 0.554-0.571 | 1.751-1.761 | 1.707-1.712 | superseded (−11%) |
| **P6 ordered queue** | heaviest first, aggregated order passes | **0.565-0.589** | **1.515-1.547** | 1.763-1.793 | **kept (−22%)** |
| P5 1,344 groups | instead of 4,096 | 0.557-0.582 | contended | 1.750-1.787 | equal; 4,096 kept for larger GPUs |
| P5 L / O / LO | register-pressure variants (table above) | 0.56-0.60 | 1.54* / 1.62-1.68 / 1.60 | 1.77-1.80 | rejected |
| P6 heaviness 4 / 64 / 256 | `VSM_HEAVY` (16 kept) | 0.56-0.58 | 1.503-1.529 / 1.770-1.775 / 1.892-1.900 | 1.70-1.79 | 4 within noise; 64 and 256 slower |
| P6 48-byte records | three 16-byte words per source triangle, neighbours inline; 36 bytes interleaved | 0.553-0.586 | 1.483-1.499 against 1.527-1.536 | 1.75-1.78 | not kept (−2.5% for +149 MiB) |
| **P7 casters-only frames unordered** | no order where only casters move | 0.569-0.604 | 1.489-1.541 | **1.696-1.745** | **kept** (caster −3% against ordered) |
| P8 heavy-tile split | tiles in bucket 7 or 8 and up as four row warps | 0.589-0.610 | 1.491-1.509 | 1.704-1.814 | rejected (no gain, static +5%; it compiled to 224 registers, 8 warps) |

\* contended visits only.

The 48-byte records stay an open trade, like N8l's packed vertices in the other direction. With them the no-RT total would be about 3,589 MiB reserved, still under Unreal VSM's 3,912. The code is kept as `variants/rec48.diff`.

## Hybrid (M)

N8k's Hybrid calls this module's `vsm_prepare`, `vsm_ring` and `vsm_march` from its own queued pass (`vsm_lookup_queued`), so the new schedule does not reach it. Its exact work is one partial wave over the receivers the map queued.

| Hybrid's queued pass, camera-moving | Main (P9) | N8m (P9) | N8m, 4 lanes per receiver (P7) |
|---|---:|---:|---:|
| Duration | 535-566 µs | 527-544 µs | 532-567 µs* |
| Resident warps per SM: start → average | 8.98 → 4.81-5.08 | 9.14 → 5.04-5.26 | 12.0 → 6.8-7.2 |
| Active threads per warp | 4.4 | 4.4 | 2.2 |
| Registers / shared per warp | 168 / 8,448 B | 168 / 8,448 B | 168 / 8,448 B |

\* P7's traces landed in caster motion; it is shown for its residency and lanes only.

- **Camera frames hit a latency floor, as N8k found.**
  - About 17k receivers fill 530 warps. That wave decays from its start.
  - Its length is the heaviest receivers' dependent walk chains. More lanes per receiver fill the SMs but run no faster: two and four lanes read 1.59-1.68 ms in P7's clean camera visits, against 1.50-1.53.
- **Rolling frames have too few warps.**
  - The refresh queues about 1/16 as many receivers: roughly 33 warps over 56 SMs, each carrying 32 divergent walks.
  - Spreading them over 2, 4 and 8 lanes each cut Hybrid's static pass to 0.72-0.75, 0.66-0.67 and 0.59-0.63 ms (P7), against 0.80-0.83.
- **Kept: the spread chosen per frame.**
  - `vmap_queue_args` sets lanes per receiver to `clamp(8192 / queued receivers, 1, 8)`.
  - It stores the value in the queue's word 1, where `vsm_lookup_queued` reads it, and sizes the dispatch.
  - Camera-moving queues (over 8,192 receivers) keep one lane each.

| Hybrid, P8 screen | Static | Sun turning | Camera moving | Caster moving |
|---|---:|---:|---:|---:|
| One lane per receiver (N8k) | 0.790-0.881 | 0.804-0.832 | 1.504-1.516 | 2.365-2.454 |
| Adaptive, lane budget 4,096 | 0.674-0.703 | 0.664-0.688 | 1.503-1.507 | 2.270-2.464 |
| **Adaptive, lane budget 8,192 (kept)** | **0.595-0.628** | **0.614-0.631** | contended in P8; 1.501-1.526 in the final lease | 2.30-2.49 |
| Adaptive, lane budget 16,384 | 0.587-0.631 | 0.607-0.623 | 1.512 (one clean visit) | 2.19-2.49 |

## What was built (M)

- **`shaders/virtual_shadow.wgsl`.**
  - The lookup body is now `vsm_lookup_tile`. `vsm_lookup` either runs it for its workgroup's tile (`VSM_PERSIST` 0), or loops over a tile queue (`VSM_PERSIST` 4,096, the default).
  - With `VSM_ORDER` (default) the queue indexes an order that two new passes build first: `vsm_order_count` and `vsm_order_place`, 256 threads a workgroup, aggregated in workgroup memory.
  - Each tile records a work word, its heaviest lane's ring tests plus walk visits, when that reaches `VSM_HEAVY` (16). Casters-only frames keep the grid's order.
  - The scratch binding (20) is now one `VsmScratch` layout:
    - a queue word and three reserved words;
    - 32 words of bucket counts and cursors;
    - per trace tile an order entry and a work word;
    - then the `VSM_DEBUG` trace.

    N8k's hybrid queue uses the same declaration: count, spread, then two words per receiver.
- **`shaders/virtual_shadow_hybrid.wgsl` and `shaders/virtual_shadow_lookup.wgsl`.**
  - The queued pass reads the queue through `VsmScratch` and runs one receiver per `spread` lanes.
  - `vmap_queue_args` chooses the spread (`VMAP_QUEUE_SPREAD` 8, `VMAP_QUEUE_LANES` 8,192) and sizes the dispatch.
- **`src/virtual_shadow_gpu.rs`.**
  - The scratch buffer holds 144 bytes plus 8 per trace tile, plus the trace in debug runs.
  - The encoder zeroes the header before a persistent lookup and dispatches the order passes, which are built on first use.
  - It takes the schedule's defaults from the module's `override` declarations (`schedule_defaults`), so a variant file of main's lookup runs as the grid launch.
  - The hybrid's queue-argument pass is rebuilt with the `VMAP_` screen constants.
- **`src/virtual_shadow_tests.rs`.** The diagnosis trace is read after the scratch's per-tile words. New screen constants: `VSM_PERSIST`, `VSM_ORDER`, `VMAP_QUEUE_SPREAD`, `VMAP_QUEUE_LANES`.
- **Programmability is unchanged.** The schedule and the work words are visibility-only bookkeeping. No lighting-model data entered hit records, clusters, maps or the lookups, which still output the visibility counts that user shading programs consume.

## Quality and identity (M)

The shipped lookups are byte-identical to main everywhere this unit checked, in the final lease unless named:
- **No-RT quarry at LOD 0.35 px.** Exact counts `05e50a1d8ad17426…` (pose 0) and `bed48bf7099343bb…` (pose 1), hits `01dff017…` / `c0de3961…`. Hybrid counts `b26bdbd11b6b18d2…` / `c5cc972dd9394732…`.
  - These are equal on both binaries in every visit, and for every intermediate schedule (grid, unordered queue, one-lane Hybrid).
  - The caster identity gate reports 0 differing pixels in all 72 checks.
  - Every screened variant of P1-P8 matched them too, except the defective P4 form above.
- **RT device.** Counts are equal to main's for traced primaries (`95ea47e97211bc02…` / `5b9d5d6b136b233b…`) and for LOD primaries, at both poses, in both visits.
- **Traced quarry screen.**
  - Exact: static `2d6e4fa6ca32…`, turned `88888216c1d9…`; agreement against Interactive unchanged (2,157 flips of at least ½, 1 lit receiver shadowed, 1,438 umbra receivers lit).
  - Hybrid: `d3b38edfcf19…` / `a0328b90f84b…`; agreement 7,329 / 25 / 5,123.
  - Both binaries alike.
- **Fixture.** Counts `5917aada…` (15°) and `c5004c93…` (40°) on both binaries, as N8l.
- **S1, S2, S3.** Rescored on the final binary with the unchanged scorers. 0 non-timing differences:
  - Exact against N8l's final scores on main: 904 S1 score fields and 72 each in S2's and S3's `comparison.json`;
  - Hybrid (`march_samples` 6) against N8k's `final-hybrid-samples6` scores, the same fields.
- **Pre-existing, unchanged.** The validation-on fixture logs 10 SPIR-V layout errors (VUID-StandaloneSpirv-None-10684, a Function-storage struct with Offset decorations) on both binaries alike.

So the quality tables are N8l's for Exact and N8k's for Hybrid. Exact is above Unreal VSM on every measure except N8h's carried misses. Hybrid is above VSM on every measure of every bench (N8k). Neither is film quality, and neither is presented as meeting the film gate.

## Memory against Unreal (M)

- The scratch buffer grows from 16 B to 518,560 B at 1080p (144 B plus 8 B per trace tile).
- Allocator reserved is unchanged: 3,439.7 MiB for Exact and 3,599.7 MiB for Hybrid, on both binaries, both poses. Allocated memory grows by 0.5 MiB.
- That stays under Unreal VSM's 3,912 MiB (no RT, N8h's measurement).

## Misses, honestly

- **Camera motion.** Exact reads 1.50-1.51 ms against VSM's estimated ~0.5 ms, about 3x. The no-RT frame with the camera moving is 2.21-2.28 ms against VSM's 1.61.
- **Occupancy is now at its static limit** of 12 warps for 95% of the dispatch.
  - Going further needs at most 128 registers and 6.4 KB of shared memory per warp, against 168 and 8.4 KB. The march and walk's live state would have to shrink by about a third.
  - No register or table cut tried here moved the compiler's 168 (I: it holds the 12-warp budget and spills the rest).
- **Hybrid's queued pass** keeps N8k's latency floor on camera frames: 527-544 µs in the trace. More warps do not shorten the heaviest receivers' dependent chains.
- **Static.** Exact reads 0.56-0.59 ms against VSM's 0.50, and turning is met. Hybrid's static frames, 0.60-0.67 ms, are now near Exact's.
- **Caster motion.** 1.70-1.73 ms; VSM has no recorded row.
- **Carried from N8e-N8l:**
  - S1 pose 1 has 2 umbra leaks against VSM's 0.
  - The quarry's lit-darkened margin stays thin (55-60 against 87-88).
  - S2's camera phases sit above VSM's 0.35 ms.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8m/`):
  - `p1-t-*` … `p8-t-*`, `final-t-*`, `final-tt-*`: timing, hashes and memory reports per lease. Summarize them with `scripts/vt.py` (per pose) or `scripts/summ.py` (clean ranges).
  - `*-mp`, `*-rt`: trace exports (`occ.json`, `occ-queued.json`, `analysis/`), from `scripts/occ.py` and `scripts/analyze.py`. Check a trace's `vsm_raster` with `scripts/ranges.py` before reading it as camera motion. Trace files were deleted after export.
  - `final-fix*`: fixture counts. `final-screen-*`: traced screens. `final-scores`, `final-scores-hybrid`: S1-S3 runs and scores; compare them with `scripts/scorediff.py`.
  - `variants/`: every screened shader, and `rec48.diff` (the 48-byte records).
  - `edits/`: the generator scripts. `p*.sh`, `final.sh`, `trace.sh`, `lease.sh`: the lease scripts.
  - `final-binaries.sha256`, `runs/`: every lease's log and preflight.

  Raw dumps over 5 MB were deleted after hashing, scoring and export, and are listed in `deleted-raw.txt`: trace files, and this run's scored S1-S3 frames. So were the staged binaries, both Cargo targets and the detached main worktree.
- **Build and environment.**
  - Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan.
  - Validation was off for timing and traces, and on for the traced screens, the fixture and S1-S3.
- **Gate.**
  - `cargo test -p renderlab --lib -j 2` exited 0 inside the final lease, on the committed source: 106 passed, 0 failed, 110 ignored.
  - Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab (all targets) and renderlab-viewer, and fmt passes.
- **Contention.** `mypaper-wallpaper` contended P1-P5 heavily and the others intermittently; contended visits are excluded. M3's and N8k's leases interleaved. No game ran, and no window, focus change or capture was used.
- **Leases.** Nine under the shared lock with the 4/2 GiB guard, each taken after the other workers' release lines, 3,303 s in all: P1 154 s, P2 367, P4 172, P5 577, P6 430, P7 403, P8 193, final 692, P9 315. At least 17.4 GiB was available at every start.
  - P3's script was cancelled before its lease began: its binary lacked a screen constant.
  - P5's base runs failed at pipeline creation, a shipped-default problem fixed by `schedule_defaults`; P6 reran them.
- **Time (UTC).**
  - 04:11-04:22: reading.
  - 04:22-04:30: the persistent queue and P1.
  - 04:30-04:50: the P2 traces (the limiter), the ordered queue and P4.
  - 04:50-05:20: the shipped schedule, base, register, threshold and record screens (P5, P6).
  - 05:20-05:40: the N8k merge, the shared scratch layout and Hybrid (P7).
  - 05:40-05:50: the adaptive spread and the split screen (P8).
  - 05:55-06:07: the final lease.
  - 06:07-06:30: scoring, the camera-motion traces (P9) and this record.

  GPU leases total 55 minutes; implementation and analysis took the rest.
