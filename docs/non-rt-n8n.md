# N8n: a surface-keyed visibility cache for the non-RT virtual-shadow pass

**Follow-ups: [N8o](non-rt-n8o.md)** makes Surface deterministic (every phase hash-equal across visits and processes) and its caster invalidation exact (a rolled frame equals a fresh lookup byte for byte at a static camera; with camera history every difference is reuse), and caster-moving frames cheaper than Exact's (1.56-1.65 against 1.68-1.71 ms). It stayed opt-in in N8o: on the quarry it is below Exact on four of five film-reference measures (above VSM on all), and camera motion still costs about 1.04 ms. [N8p](non-rt-n8p.md) subsequently adopts it as the default by Islam's decision.

Follow-up unit to [N8m](non-rt-n8m.md), 2026-11-07 UTC, base main `f40cc5b`, branch `t3/n8n-surface-cache`. Brief (Islam, 2026-11-07): the camera moves, but the sun and the casters do not, so a receiver point's shadow does not change. Key the exact walk's results by the receiver's surface point instead of its pixel, walk only the misses, and invalidate on sun changes (N8e's hysteresis) and caster motion (N8f's reach test). It need not be byte-identical, but it must beat Unreal VSM on every measure, including temporal stability under camera motion, within the memory budget. Make it the default only if it beats VSM everywhere. Labels: **M** measured here, **I** inference.

## Result

**Kept at this checkpoint, opt-in: `VirtualLookup::Surface`.** It is above Unreal VSM on every quality measure of every bench, and on motion stability. It was not the default at this checkpoint: camera-moving frames miss VSM's estimated cost. [N8p](non-rt-n8p.md) subsequently adopts N8o's corrected Surface as the default by Islam's decision.

| No-RT quarry, shadow pass, ms (M) | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| Main (`main.exe`, f40cc5b) | 0.557-0.578 | 0.569-0.582 | 1.489-1.503 | 1.687-1.704 | 1.28-1.31 / 2.21-2.26 / 2.40-2.44 |
| Exact on this binary | 0.555-0.578 | 0.568-0.583 | 1.490-1.532 | 1.692-1.782 | 1.26-1.35 / 2.21-2.26 / 2.41-2.54 |
| **Surface** | **0.000** | **0.571-0.594** | **1.025-1.040** | **1.715-1.773** | **0.72-0.75 / 1.76-1.80 / 2.47-2.56** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

- **Camera-moving frames fall 31%** (1.49-1.53 → 1.03-1.04 ms), and the no-RT frame with the camera moving 20% (2.21-2.26 → 1.76-1.80). That is still about 2x VSM's estimated ~0.5 ms: **a miss.**
- **Static frames cost nothing.** A frame that changes nothing a count depends on runs no pass; the timestamps read 0.00006 ms. The no-RT static frame falls to 0.72-0.75 ms, under VSM's 1.61.
- **Turning frames** cost Exact's (0.57-0.59 ms), under VSM's about 0.95. **Caster-moving frames** cost Exact's within 1-2%.
- **Quality equals Exact's within a few percent** on the quarry, and on S1-S3 to the printed digit in most fields:
  - **Lit-darkened:** fewer than Exact on the quarry (30-40 against 55-60; VSM 87-88).
  - **S1:** no leaks on either pose, which removes Exact's carried 2-leak miss on pose 1.
  - **Flicker under motion:** equal to Exact's against the film reference, 4-25x below VSM's.
- **Memory:** 3,503.7 MiB reserved against Exact's 3,439.7, 408 MiB under Unreal VSM's 3,912.
- **The cost is the boundary pixels.**
  - While the camera moves, 86% of receivers are shown from the cache. The 14% that walk are shadow-boundary pixels, which must be evaluated at their own point for VSM-beating lit-darkened, plus cells not seen yet.
  - Their walk takes 714-737 µs (Nsight) against Exact's whole lookup at 1,408-1,422 µs. It is bound by a tail of long walk chains, and no schedule, split or ordering tried here removed it.

## How it works (M)

shaders/virtual_shadow_cache.wgsl, appended to shaders/virtual_shadow.wgsl as N8k's hybrid is; src/virtual_shadow_cache.rs owns the table, epochs and passes.

- **Cells.**
  - A cell is a receiver's placement, viewed side, and a world-space cube around its point. The cube's edge is a power-of-two multiple of the finest map texel, at most one pixel of the receiver's footprint (`VSC_CELL` 1).
  - Cells live in a hash table: 2^20 buckets of four 8-byte slots (32 MiB). Each bucket hash names a 4x4x4-cell block whose 64 cells share 64 consecutive buckets (2 KiB), and an independent 32-bit chain is the fingerprint (52 bits name a cell).
  - The value holds the visible count, the march's start level, a check nibble, the caster and sun epochs, and whether the cell may be shown.
- **Which pixels skip the walk.**
  - A pixel shows its cell's count only when the cell is solid (fully lit or fully blocked), and the pixel that computed it found every receiver of its 3x3 neighbourhood with the same count. The cell then lies inside a region that agreed.
  - Every other pixel walks: a cell beside a shadow edge or penumbra, a cell not computed yet, a cell due for refresh. It walks at its own receiver point with Exact's directions and per-pixel jitter. So the cache never darkens a lit receiver by a cell's extent, nor moves an edge.
  - A cell once seen beside a differing count is never shown again until a new epoch.
- **Per frame** (encode_virtual, one compute pass after the page work):
  1. `vsc_probe` looks up every covered pixel in 16x8 workgroups. A shown cell writes its count; any other pixel is queued in the lookup's scratch (the hybrid's queue layout). Cells known to be heavy (the walk's work, N8m's measure, kept in the unshown cell) go from the queue's top end.
  2. `vsc_args` sizes the next two dispatches.
  3. `vsc_missed` runs the exact walk over the queue (N8m's persistent workgroups, heavy walks first).
  4. `vsc_insert` stores the results with the 3x3 verdict, read from the finished counts. A shown result also fills the cells at the midpoints to its same-placement neighbours, which lie inside the region that agreed.
- **Still frames.**
  - A frame that keeps the camera, receivers, casters, map and target (N8e's rolling condition) runs no pass once 16 frames have passed since the sun last turned.
  - If only the sun turned, or only casters moved, a walking pixel keeps its last count until its cell's refresh frame, or until the moved casters reach it. Inside a moved caster's screen bounds it always walks.
- **Validity.** A cell is current while:
  - the cache epoch is unchanged. It is hashed into every key, and settings, the casters' draw list, the receiver producer, a map rebase (the sun leaving the hysteresis) or an epoch wrap bump it;
  - the sun has not turned since the cell was computed, or turned fewer than 16 frames ago and this is not its world block's refresh frame (N8e's lag bound);
  - no caster moved since, or every caster-moving frame since cleared the cell by N8f's reach test, which marks a kept hit current again. A cell nobody looked at on a caster-moving frame becomes stale.
- **Programmability is unchanged.** Cells hold visibility counts only; no lighting-model data entered hit records, clusters, maps or the lookup, which outputs the counts user shading programs consume.

## Every step measured (M)

No-RT quarry, 1920x1080, LOD 0.35 px, lift 1, clean visits' medians over both poses (shadow pass ms: static / turning / camera / caster). "Walks" is the share of receivers that walk while the camera moves (`VSC_STATS`, last 16 of 48 frames at 1 mm a frame). Quality is the quarry's no-RT film-reference score: pose 0 partial RMS / flips / leaks / lit-darkened / lost edges, Exact .353 / 2,434 / 3,289 / 60 / 2,376.

| Step | Change | Timing | Walks | Quality | Verdict |
|---|---|---|---:|---|---|
| P1 | Every cell shown, evaluated once at a canonical point (cube centre on the triangle) with the cell's jitter; cells keyed by triangle too | 0.16-0.33 / 0.51-0.58 / 1.31-1.33 / 1.88-1.90 | 31-38% | .391 / 3,090 / 3,749 / **697 ✗** / 2,677 | rejected: lit receivers beside penumbrae and contact edges take a neighbour cell's 7/8 or 0/8 |
| P2b | Persistent miss walk; counters | camera 1.22 | 31-36% | — | first moving frame 42% shown; 1 mm, 1 cm and a 2 mrad pan alike |
| P3 s1-s4 | Show only solid cells whose 3x3 (s4: 5x5) neighbourhood agreed; others walk at their own point | 1.04-1.28 / 1.13-1.36 / 1.60-1.80 / 2.18-2.35 | 42-63% | **≈ Exact** (lit-darkened 47-62) | quality solved; boundary pixels walked every frame, even static |
| P4 | Idle still frames; still frames keep walking pixels' counts; midpoint splats | **0.000** / 0.56-0.61 / 1.56-1.74 / 1.70-1.79 | 41-63% | ≈ Exact | splats changed nothing (41.7 → 41.3%) |
| P5 | Cells keyed without the triangle (1 px cells) | 0.000 / 0.62-0.65 / 1.13-1.35 / 1.79-1.82 | **14%** | ≈ Exact (lit-darkened 60 / 55) | kept: LOD triangles are about a pixel, so a sub-pixel step moved most pixels to another triangle's cells |
| P6 | Walk schedule: 2 and ~5 lanes per walk, grid launch | spread slower; grid ≈ persistent | — | — | trace: the walk spends 60% in a tail, 4.4 of 32 lanes |
| P7 | Heavy walks first; block-local buckets | probe 106 → 84 µs, walk 846-1,002 → 718-811 µs | 14% | — | kept |
| P8 | Heavy walks split over 8 / 4 / 2 lanes, one direction set each | walk 1,875 µs (split 8); every variant slower, split 1 too | — | leaks 6,369 (+3,000) | rejected: per-lane walks repeat shared steps without the cap; the code shifted the compile |
| P9 | 16x8 probe workgroups; upper bound of proving boundary umbra cheaply (`VSC_REUSE` 3, unsafe) | 1.03-1.05; bound 0.88 | 14% (bound 8.7%) | — | probe kept; blocker hints would save at most 0.16 ms |
| F1-F3 | Final candidate; fixes from the gates (below) | 0.000 / 0.58-0.67 / 1.02-1.07 / 1.70-1.77 | 14% | — | — |
| F5 | Walks with Exact's per-pixel jitter (the cell's jitter is `VSC_JITTER`) | **0.000 / 0.57-0.59 / 1.03-1.04 / 1.72-1.77** | 14% | .362-.365 / 2,638-2,685 / 3,428-3,454 / 36-40 / 2,600-2,652 | **shipped (opt-in)** |

The final gates found three defects, each fixed and re-verified:
- **Casters-only frames.** A walking pixel inside a moved caster's screen bounds kept its last count, though its receiver may have changed. It now walks (N8f's screen rectangles).
- **Sun jumps.** A sun leaving the map's hysteresis only lag-refreshed cells, and idle frames then never refreshed them. On S3, the bench's return to its first sun kept other-sun cells: static-phase partial RMS .423 and 12,096 contacts lost. A rebase now retires every cell, as Exact looks up every tile then, and frames stay non-idle for 16 frames after a turn.
- **Cell-keyed jitter.** It let S1's umbra receivers see one direction more (4 leaks a pose against VSM's 0), and raised S2's camera-phase flicker (.0198 against Exact's .0143). Walks now use Exact's jitter.

The table is also dropped when Surface is not the lookup.

## Timing (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off, 1920x1080, poses 0 and 1.
- **Binaries.** `fin4.exe` is commit `1c52f09`. `main.exe` is f40cc5b, built from a detached worktree with its own Cargo target.
- **Clean visits.** A visit is clean when its primaries read at most 0.45 ms, and the ranges are clean visits' medians.
- **Contended visits.** About half of Surface's visits ran slower in every stage, with primaries at 0.65-0.92 ms and the shadow pass at 1.5-2.4 ms; Exact's visits in the same processes were mostly clean. N8k saw the same with its lighter Map lookup: the lighter the frame, the slower the next primaries (I: GPU clocks under the harness's frame pacing, after Surface's idle static phase).
- **Cross-check.** Nsight's per-dispatch sums below agree with the clean ranges.

**Traced quarry on the RT device** (validation off, ms, static / turning / camera / caster):

| | Main | Exact on this binary | Surface |
|---|---|---|---|
| Traced primaries | 0.530-0.545 / 0.557-0.559 / contended / 1.733-1.737 | 0.532-0.540 / 0.553-0.560 / 1.569-1.570 / 1.696-1.753 | **0.000 / 0.562-0.564 / 1.031-1.036 (F1), 1.195 (F5, one clean visit) / 1.772-1.847** |

**Per stage, camera-moving frames** (Nsight Graphics 2026.4.0 GPU Trace, Top-Level Triage multi-pass, N8m's recipe: no-RT quarry pose 0, `move_wrap` 48 mm over 30,000 frames, four submits from 45 s. `vsm_raster` read 1.6-1.9 µs per pass, so the trace is inside camera motion):

| Dispatch, µs | Exact (F1) | Surface (F5) |
|---|---:|---:|
| `vsm_mark` | 36.4 | 28.0 |
| Other page passes (compact, clear, cull, finish, 2 rasters, mip) | ~11 | ~11.5 |
| Tile order (Exact) / probe (Surface) | 7.1 | 67.5 |
| Lookup (Exact) / queued walk (Surface) | 1,408-1,422 | 714-737 |
| Insert | — | 35.1-37.0 |
| **Sum** | **about 1,465** | **about 875** |

| Occupancy (multi-pass) | Exact lookup | Surface walk | Surface probe |
|---|---:|---:|---:|
| Average resident warps per SM | 11.5-11.6 | 6.8-7.1 | 44.6-44.9 |
| Warps over the dispatch (20 buckets) | 12.00 for 95% | 12.00 for 45%, then 11.2 → 0.1 | — |
| Active threads per warp | 8.9 | 4.7-4.8 | 28.3-28.4 |
| SM throughput | 42.7-43.1% | 25.5-26.7% | 16.8-17.1% |
| Registers per thread / shared per warp | 168 / 8,448 B | 168 / 8,448 B | 32 / 64 B |

- The walk processes 14% of receivers in half the lookup's time. Its waves are incoherent: boundary receivers from many tiles share a warp.
- Its second half is a tail of the longest chains. Ordering heavy walks first moved full occupancy from 40% to 45-50% of the dispatch; thresholds of 4, 5 and 7 measured within noise of each other and of no ordering.
- The probe's hottest lines wait on the table load (35% of its samples) and the coverage load (18% before 16x8 workgroups).

**Hit rates, final binary** (`VSC_STATS`, medians of the last 16 frames, both poses):

| Phase | Shown | Not shown (boundary cells) | Walked | Of which sun refreshes |
|---|---:|---:|---:|---:|
| Camera, 1 mm a frame | 86.0-86.1% | 11.0-11.1% | 13.8-14.0% | 142-284 |
| Camera, 1 cm a frame | 85.5-85.7% | 10.7-10.8% | 14.3-14.5% | 104-206 |
| Camera, 2 mrad pan a frame | 85.8-85.9% | 10.6-10.7% | 14.1-14.2% | 126-254 |
| Sun turning (still camera) | 79.3-79.8% | 13.7-13.8% | 6.4-6.9% | 21,400-21,700 |
| Caster moving (still camera) | 61.8% | 9.5-9.6% | 28.6% (about 99,000) | 0 |
| Static | — | — | 0 (idle) | — |

- **Cold start.** The first moving frame after a still view walks 33% (cells around the new sample points are not filled yet), falling to 17% by frame 10 and 14% by frame 40.
- **Per frame** (clean visits): 1.3-1.5 ms for the first four frames, 1.0-1.1 ms by frame 20.

## Quality (M)

The final binary, the unchanged scorers, recorded Unreal rows. Exact's rows are its scores on this binary, its output main's (hashes below). ✗ marks a measure below VSM; a tie at 0 is not a miss.

**Quarry, film reference** (n8e-score, 64-seed traced Exact reference built in process). Each pose is approached by 16 camera frames of 2 mm, so its frames start from what motion cached; two Surface visits:

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|
| Unreal VSM | .483 / .483 | 12,390 | 14,935 | 87 / 88 | 11,884 |
| Exact, no-RT | .353 / .348 | 2,434 / 2,474 | 3,289 / 3,366 | 60 / 55 | 2,376 / 2,424 |
| **Surface, no-RT** | **.362-.365 / .356-.358** | **2,638-2,685 / 2,681-2,727** | **3,428-3,454 / 3,510-3,535** | **36-40 / 30-32** | **2,600-2,652 / 2,655-2,703** |
| Exact, traced | .286 / .290 | 340 / 356 | 1,202 / 1,213 | 0 / 0 | 339 / 356 |
| **Surface, traced** | **.304-.308 / .305-.309** | **646-731 / 698-779** | **1,405-1,460 / 1,449-1,494** | **2-3 / 1** | **642-729 / 697-778** |

- Surface differs from Exact in 936-1,249 of 346,000 receivers (0.3%): shown cells whose pixel's own walk would differ, at sub-pixel features.
- Two Surface visits differ in 313-434 receivers (200-254 solid). Concurrent inserts and evictions decide which cells survive; within one run a cell keeps its value.

**Traced quarry screen** (against hardware Interactive on the same directions, 346,517 receivers, validation on):
- **Static:** Surface's hash equals Exact's (`2d6e4fa6ca32…`). The first frame walks every pixel at its own point with Exact's directions, and still frames run no pass.
- **Turned:** Surface has 2,083 flips of at least ½, 9 lit receivers shadowed and 1,482 umbra receivers lit, against Exact's 2,071 / 12 / 1,505.

**S1, contact fold** (environments 0 and 1 equal unless given):

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Contacts lost |
|---|---:|---:|---:|---:|
| Unreal VSM | .1642 / .1628 (env 1: .1644 / .1620) | 0 (env 1: 1) / 0 | 0 / 0 | 5-6 / 2-3 |
| Exact | .1085 / .1065 | 0 / 0 | 0 / **2 ✗** | 0 / 0 |
| **Surface** | **.1085 / .1062** | **0 / 0** | **0 / 0** | **0 / 0** |

**S2 and S3 in motion** (validation on, as scored; per-phase maxima for leaks and flips; HDR RMS, contacts lost of n, partial RMS and shadow-pass p50 ms):

| Bench, phase | Exact pRMS | **Surface pRMS** | VSM pRMS | Surface HDR (VSM) | Surface contacts lost of n (VSM) | Surface leaks / flips (VSM) | Shadow p50: Exact / **Surface** / VSM |
|---|---:|---:|---:|---|---|---|---|
| S2 camera out | .0833 | **.0832** | .1752 | .0107 (.0151) | 0 of 19,396 (2,101) | 0 / 0 (62 / 167) | 0.50 / **0.40** / 0.35 ✗ |
| S2 camera hold | .0865 | **.0863** | .1758 | .0115 (.0154) | 0 of 11,264 (1,039) | 0 / 0 (57 / 152) | 0.19 / **0.0001** / 0.35 |
| S2 camera return | .0809 | **.0808** | .1739 | .0103 (.0145) | 0 of 17,990 (1,972) | 0 / 0 (62 / 155) | 0.49 / **0.37** / 0.35 ✗ |
| S2 shadow out | .0759 | **.0759** | .1697 | .0099 (.0152) | 0 of 16,687 (2,070) | 0 / 0 (65 / 120) | 0.37 / **0.38** / 0.36 ✗ |
| S2 shadow hold | .0766 | **.0766** | .1675 | .0100 (.0150) | 0 of 8,832 (1,059) | 0 / 0 (66 / 119) | 0.14 / **0.0001** / 0.35 |
| S2 shadow return | .0759 | **.0759** | .1692 | .0099 (.0152) | 0 of 16,687 (2,057) | 0 / 0 (67 / 120) | 0.37 / **0.39** / 0.35 ✗ |
| S3 static | .1186 | **.1186** | .1878 | .0150 (.0194) | 0 of 48,512 (2,929) | 1 / 0 (111 / 187) | 0.24 / **0.0001** / 0.37 |
| S3 rotate .25 | .1217 | **.1217** | .1875 | .0151 (.0194) | 0 of 194,291 (11,630) | 1 / 0 (111 / 190) | 0.24 / **0.27** / 0.59 |
| S3 hold slow | .1266 | **.1265** | .1873 | .0152 (.0194) | 0 of 48,640 (3,006) | 1 / 0 (109 / 191) | 0.25 / **0.0001** / 0.36 |
| S3 rotate 1 | .1218 | **.1216** | .1863 | .0150 (.0200) | 0 of 196,290 (12,289) | 2 / 0 (106 / 192) | 0.24 / **0.28** / 0.59 |
| S3 hold fast | .1210 | **.1212** | .1875 | .0150 (.0205) | 0 of 49,728 (3,378) | 0 / 0 (103 / 188) | 0.25 / **0.0001** / 0.36 |
| S3 jump recovery | .1159 | **.1159** | .1811 | .0146 (.0219) | 0 of 51,777 (3,153) | 0 / 0 (114 / 226) | 0.24 / **0.0002** / 0.37 |

Surface is above VSM on every S2 and S3 quality measure in every phase, and equals Exact's to the printed digit in most. Exact's HDR RMS and contacts equal Surface's. Its leaks are 1 in S3's first four phases and 0 elsewhere; Surface's are the same except 2 in rotate 1.

**Motion stability: frame-to-frame flicker against the film reference** (bench-motion's `residual_change_rms`: RMS over covered pixels of the frame's change in error from the previous frame, ((v − ref) − (v′ − ref′)); phase mean (max); VSM from the recorded per-frame rows):

| Bench, phase | Exact | **Surface** | Unreal VSM |
|---|---|---|---|
| S2 camera out | .01428 (.02288) | **.01423 (.02288)** | .06128 (.08098) |
| S2 camera hold | .00042 (.01340) | **.00042 (.01332)** | .06112 (.07462) |
| S2 camera return | .01358 (.02347) | **.01353 (.02348)** | .05769 (.07386) |
| S2 shadow out / return | .00488 (.00653) | **.00488 (.00653)** | .05270 (.06677) / .05244 (.06595) |
| S2 shadow hold | .00015 (.00468) | **.00015 (.00468)** | .05198 (.06533) |
| S3 static | 0 (0) | **0 (0)** | .08033 (.09871) |
| S3 rotate .25 | .00324 (.00547) | **.00325 (.00572)** | .07995 (.10006) |
| S3 hold slow | .00119 (.00449) | **.00123 (.00421)** | .08014 (.09963) |
| S3 rotate 1 | .00627 (.03613) | **.00631 (.03643)** | .08155 (.10333) |
| S3 hold fast | .00214 (.00663) | **.00212 (.00667)** | .08239 (.10216) |
| S3 jump recovery | .00161 (.05327) | **.00161 (.05327)** | .08644 (.10644) |

- Surface's flicker equals Exact's within 1% of every phase mean, and is 4-25x below VSM's.
- Before the rebase fix, S3's jump-recovery maximum was .112, above VSM's .106; that is the spike that led to the fix.
- With the cell-keyed jitter, S2's camera phases read .0198 / .0191: a pixel's noise then changed whenever its point entered a new cell.
- The quarry has no per-frame motion references. Its motion evidence is the approach-scored table above and the caster identity gate below.

**Caster identity gate** (the harness's N8f check: after 48 caster-moving frames, the rolled counts against a fresh lookup of the final placement):
- Exact differs in 0 pixels.
- Surface differs in 530-570 pixels (no-RT) and 631-682 (traced). That is a fresh lookup's every-pixel walk against the rolled state's shown cells: the same order as two Surface visits' 313-434 differing pixels.
- Before the screen-bounds fix it read about the same (533 no-RT, 651 traced): the defect that fix removes (a kept count under a changed receiver) did not show at this caster test.
- So this gate cannot separate stale caster data from that reuse difference (I).

## Memory against Unreal (M)

| No-RT quarry, MiB | Allocator reserved | Allocated | Against Unreal VSM's 3,912 |
|---|---:|---:|---|
| Exact (main and this binary) | 3,439.7 | 3,344.8 | 472 below |
| **Surface** | **3,503.7** | **3,392.6** | **408 below** |

- Surface adds the 32 MiB table and the 16 MiB queue (8 bytes per pixel, plus 16), and 96 bytes of uniform, counters and dispatch.
- The table exists only while Surface is the lookup: an Exact configuration after a Surface one reads 3,439.7.

## What was built (M)

- **`shaders/virtual_shadow_cache.wgsl`** (new): `vsc_probe`, `vsc_args`, `vsc_missed`, `vsc_insert`. Screen constants:
  - `VSC_CELL`, `VSC_REUSE`, `VSC_CONSENSUS`, `VSC_CANONICAL`, `VSC_TRIANGLE`, `VSC_JITTER`;
  - `VSC_SPLAT`, `VSC_HEAVY`, `VSC_PERSIST`, `VSC_SPREAD`, `VSC_LANES`, `VSC_PROBE_X`/`_Y`;
  - `VSC_FINER`, and `VSC_STATS` (counters).
- **`src/virtual_shadow_cache.rs`** (new): `SurfaceCache`, which owns the table, frame uniform, counters, dispatch, layouts, pipelines and epochs (`advance`, `flush`, `restart`), and the encoder. A unit test checks that every pass uses only bindings its pipeline layout provides.
- **`src/virtual_shadow_gpu.rs`** (small edits):
  - `VirtualLookup::Surface`; `map()` now excludes it, and `queues()` covers Hybrid and Surface;
  - the queue buffer for Surface, with `COPY_SRC`;
  - the surface cache's creation and drop, its epochs and still/idle flags in `prepare_virtual`, and no rolling refresh for Surface;
  - its groups, its encode branch, and `VSC_` constants kept out of the exact pipelines.
- **Harness** (tests only):
  - `lookup: "surface"` in the settings parser, and the `VSC_` screen constants;
  - per-frame surface counters for a `VSC_STATS` variant (`surface-stats.json`);
  - `move_step`, `move_yaw` (camera step and pan) and `approach_frames`/`approach_step` (scored poses approached by moving frames), also in scripts/n1c-batch.py.
- **Unchanged:** shaders/virtual_shadow.wgsl and Exact's output. Exact's counts are main's on every final screen (`05e50a1d8ad17426…` and `bed48bf7099343bb…`), and main's binary gives the same hashes.

## Misses, honestly

- **Camera motion.** 1.03-1.04 ms against VSM's estimated ~0.5, about 2x. The no-RT frame with the camera moving is 1.76-1.80 ms against VSM's 1.61.
  - The 14% of receivers that walk are shadow-boundary pixels plus unseen cells. The lit-darkened bar (VSM 87-88, Exact 55-60) requires them at their own points: P1's cell reuse put 697.
  - Their walk is incoherent (4.8 of 32 lanes) and ends in a tail of long chains.
- **Cold starts.** The first moving frames after a still view cost 1.3-1.5 ms (33% walk on the first frame).
- **S2's moving phases** read 0.37-0.40 ms against VSM's 0.35-0.36 (validation on); its holds read 0.0001.
- **Caster motion** costs Exact's (1.72-1.77 ms); VSM has no recorded row.
  - On the quarry's moved terrain chunk, the reach test retires 29% of receivers' cells per frame.
  - The rolled image differs from a fresh lookup in 530-570 pixels. That matches the reuse's run-to-run difference; with this gate's construction it cannot be shown to be caster-free (I).
- **Nondeterminism.** Two visits differ in 313-434 of 346,000 receivers. Each run is stable frame to frame.
- **Not tried to completion:**
  - blocker hints for boundary umbra, whose upper bound is −0.16 ms (P9);
  - showing lit boundary cells, a leak risk on S1, whose VSM bar is 0;
  - walking boundary pixels in N8m's ordered tile schedule instead of a queue;
  - folding `vsm_mark`'s page requests into the probe, so only walking pixels request pages (about 25 µs).
- **Carried from N8e-N8m:** none of Exact's misses got worse. S1 pose 1's 2 umbra leaks are gone in Surface (0); the quarry's lit-darkened margin widens (30-40 against 87-88).
- Not film quality in any mode, and never presented as meeting the film gate. Surface was not the default at this checkpoint; see [N8p](non-rt-n8p.md) for the subsequent default decision.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8n/`):
  - `p1`-`p9`, `p2b`: each screen's script, timing roots (`-t`), counters (`-s*`), quality roots (`-q`, scored by `n8e-scores.json`) and traces (`-mp`, `-rt`: `occ*.json`, `analysis-probe/`).
  - `final-*` (F1/F2, `fin.exe`), `final2-*` (F3, `fin2.exe`), `final3-*` (F4, `fin3.exe`) and `final4-*` (F5/F6, `fin4.exe`, the shipped source): timing, traced timing, counters, quality, screen, and S1-S3 scores (`final*-scores-*`: `S1/comparison.csv`, `S2|S3/comparison.json`, `n8-comparison.json`, per-frame `scores-renderlab.json`). Exact's S1-S3 scores are `final-scores-exact`.
  - `s23.json`, `flicker.json`: the S2/S3 and flicker tables. `scripts/`: `vt.py`, `vtt.py` and `summ.py` (timing), `cache.py` and `cache2.py` (counters), `s23.py`, `flicker.py`, N8m's trace scripts.
  - `final-binaries.sha256`, `runs/`: every lease's log and preflight, and the gate logs. `lease.sh`, `trace.sh`, `p*.sh`, `f*.sh`: the lease scripts; `edit-*.py`: the scripted edits.

  Raw dumps over 5 MB (quarry hit, counts, HDR, world and reference dumps; S1-S3 frames; trace files; staged binaries) were deleted after scoring and export, and are listed in `deleted-raw.txt`. So were both Cargo targets and the detached main worktree.
- **Build and environment.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Validation was off for timing and traces, and on for the quality runs, the traced screen and S1-S3.
- **Gate.**
  - `cargo test -p renderlab --lib -j 2` exited 0 inside lease F5, on the shipped source: 107 passed, 0 failed, 110 ignored.
  - Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab (all targets) and renderlab-viewer, and fmt passes.
  - `docs/milestones-5-6-decisions.tsv` is untouched.
- **Contention.** Contended visits are excluded by the clean rule (above). No game ran, and no window, focus change or capture was used.
- **Leases.** 16 under the shared lock with the 4/2 GiB guard, 3,306 s in all, at least 18 GiB available at every start:
  - P1 160 s, P2 153, P2b 187, P3 210, P4 168, P5 181, P6 168, P7 152, P8 205, P9 126;
  - F1 435, F2 260, F3 156, F4 315, F5 361, F6 69.

  P1's counters and P2's whole batch failed on harness and layout defects; the binding-layout unit test now catches the latter on the CPU.
- **Time (UTC).**
  - 06:29-06:50: reading and design.
  - 06:50-07:20: the canonical-point cache and its counters (P1-P2b).
  - 07:20-07:56: consensus reuse, idle frames, keys without the triangle, schedules, heavy-first, split and bound screens (P3-P9).
  - 07:58-08:45: the final leases, the three gate fixes, their re-verification and the scoring (F1-F6).
  - 08:45-08:51: this record and the cleanup.

  GPU leases total 55 minutes; implementation, CPU scoring and analysis took the rest.
