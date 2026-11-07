# N8o: the surface cache made deterministic and caster-exact; still opt-in

Follow-up unit to [N8n](non-rt-n8n.md), 2026-11-07 UTC, base main `fb169cd`, branch `t3/n8o-surface-default`.

Brief (Islam, via root, 2026-11-07): fix N8n's three blockers to making `VirtualLookup::Surface` the non-RT default, then keep cutting camera-moving cost toward VSM's ~0.5 ms:
1. Two runs differed in 313-434 of 346k receivers.
2. The caster identity gate could not separate stale cells from reuse.
3. Caster-moving frames cost more than Exact's.

The default switch was conditional on every gate. Quality had to stay above VSM everywhere and at least equal to Exact's (quarry poses 0 and 1, S1, S2 and S3 in all motion phases, and flicker in motion), with memory under Unreal's.

Labels: **M** measured here, **I** inference.

## Result

**All three blockers are fixed (M). Surface is still not the default: it does not reach Exact's quality on every measure.**

| Blocker | N8n | N8o (M) |
|---|---|---|
| Determinism | 313-434 receivers differed between visits | **0**: every phase's counts hash-equal over 2 visits × 2 processes, and over repeated in-process sequences |
| Caster staleness | 530-570 pixels differ from a fresh lookup; undecidable | Static camera: rolled frame **byte-identical** to a fresh lookup and to a fresh Surface run (both poses). With camera history: every differing pixel also differs, with the same count, in a twin run whose caster held still (**0 unexplained**) |
| Caster-moving cost | 1.715-1.773 ms (Exact 1.692-1.782) | **1.560-1.649 ms** against Exact's 1.676-1.707 on the same binary; trace spans 1,638-1,743 µs against 1,690-1,828 |

**Why Surface stays opt-in** (the brief's own fallback):
- **Quarry:** Surface is above Unreal VSM on every measure, but below Exact on four of five. With film-reference scores (pose 0):
  - partial RMS .365 against .353;
  - solid flips 2,685 against 2,434;
  - umbra leaks 3,452 against 3,289;
  - lost umbra edges 2,651 against 2,376.

  It is better than Exact on lit-darkened (37 against 60).
- **S3:** rotate 1 shows 2 umbra leaks against Exact's 1, and flicker in rotate 0.25 and rotate 1 is 0.3-1% above Exact's.
- **The cause is reuse, not staleness.** These are pixels showing a solid cell computed at another camera position (N8n's 936-1,249 receivers against Exact). The history gate shows it directly: 732-780 receivers differ from a fresh lookup, and fills (N8n's midpoint splats) double that count.
- **What is left** is a reuse rule whose shown cells never lose to the pixel's own walk on any measure, or an agreed tolerance. That is the brief's decision, not this unit's.

**Speed:**
- Camera-moving frames cost 1.043-1.046 ms against Exact's 1.493-1.503 (N8n: 1.025-1.040). This is still about 2x VSM's estimated ~0.5 ms: **a miss.**
- Static frames cost 0.000 ms. Turning frames cost 0.570-0.594 ms against Exact's 0.572-0.618.
- Every speed step tried is measured below. One kept step cut caster frames: a work estimate in every cell feeding heaviest-first order. Each camera-frame step was neutral or a loss.

## The three fixes (M)

shaders/virtual_shadow_cache.wgsl's header describes the result; src/virtual_shadow_cache.rs owns the table, claim words, epochs and passes.

**1. Order-independent writes.**
- **Two-pass insert.** `vsc_claim` picks each walked result's slot from the table as the frame's probe left it: the cell's own slot, else the first free or stale slot from one the fingerprint picks. It then raises the slot's two claim words with atomicMax: one to its priority, one to the priority's complement. The priority packs:
  - direct result above fill;
  - verdict grade (lit or blocked, certified; or none);
  - margin;
  - the pixel, which makes it unique.

  `vsc_resolve` lets the highest claim write the slot and clears the claims. The cell is shown only when the lowest claim's grade equals the highest's, so every pixel that computed it, and every fill, agreed.
- **Probe-time writes are per cell.** On a caster-moving frame the probe rewrites a kept cell's caster epoch. N8n decided this per pixel, with each pixel's own point and level, so one pixel's rewrite let another skip its test. The test now depends on the cell alone (below), so every writer writes the same value.
- **A flush clears the table**, instead of hashing a new epoch into every key. Nothing from an earlier sequence survives a reset, so in-process repeats equal fresh processes.
- **Other changes:**
  - consensus skips uncovered tiles, whose records are stale (`vsm_covered`);
  - walks call Exact's own `vsm_prepare`;
  - a new height range or level ladder clears the table too (`ladder_changed`), because every level's raster or march changes with it.

**2. Exact caster invalidation.**
- **The reach test covers a region, not a point.** `vsc_reached` runs N8f's test for every point within an extent of the cube's centre at once:
  - the radius at the region's lowest height, plus the extent;
  - the march's stop level taken at that lowest height;
  - levels from the cell's recorded start.
- **Two extents.**
  - A cell is retired when its cube (extent √3/2 of its edge) is reached.
  - A shown cell also stores a margin exponent: its certifying neighbourhood (the 3x3 receivers, plus the cube any of their counts came from) lies within edge × 2^m of its centre, m ≤ 7, else it is not shown. When the moved casters reach that margin but not the cube, the cell is kept but unshown. Its count is still exact, and its pixels walk once the camera moves.
- **Fills** store a margin covering their source's neighbourhood from the fill's own centre.
- **Two gates in the quarry harness** (`caster_gates`, src/visibility_buffer_tests.rs), per pose:
  - *Static:* reset, 48 frames at the pose, then 48 frames with placement 0 stepping 1 mm a frame. The rolled counts must equal a fresh lookup's (reset, one frame: every pixel walks) and those of a twin run with the placement final throughout.
  - *History:* reset, N8n's 16-frame approach, 8 pose frames, 48 camera frames away and back, then the 48 caster frames. A pixel may differ from a fresh lookup only where the twin with the caster held still differs too, with the same count: a cell computed at another camera position that no moved caster reached.

**3. Caster cost.**
- Caster-reached cells are retired by the cube test only, as above.
- Every cell now carries its last walk's work estimate (3 bits; the sun epoch went from 13 to 10 bits, and a wrap clears the table). Reached shown cells now walk heaviest first too: the caster-frame walk fell 13%, from 759-777 to 642-692 µs.

## Every step measured (M)

No-RT quarry, 1920x1080, LOD 0.35 px, lift 1, both poses.
- **Timing:** clean visits' medians, a visit clean when its primaries read ≤ 0.45 ms. Columns are shadow-pass ms: static / turning / camera / caster.
- **Traces:** Nsight Graphics 2026.4.0 GPU Trace, Top-Level Triage multi-pass, N8m's recipe, `vsm_raster` ≈ 2 µs in every camera trace. Caster and steady-turning traces use the new `caster_wrap` (48 mm back and forth) and `turn_wrap` (16 steps of 0.5 mrad) harness options.

| Lease | Change | Measured | Verdict |
|---|---|---|---|
| L1 | Claim/resolve inserts, per-cell reach test with margin, table cleared on flush, no fills, Exact's `vsm_prepare` | 0 hash differences (2 visits, 2 processes); static gate 0; history gate 0 unexplained (342 differ); walks while moving **18.7%** (N8n 14%); turning 0.68-0.70 | determinism and staleness solved; hit rate lost |
| L2 | Deterministic fills (claims below direct results) | walks 14.4% (85.6% shown); history differs 750/707, all explained; claim+resolve 96 µs (N8n insert 36); turning 0.77; caster 1.91-2.04 | fills kept; overhead too high |
| L3 | Fills only on camera frames, fill mask in the queue word, current-unshown slots unclaimed, caster walks only where the cube is reached | claim+resolve 55 µs; caster spans 1,743-1,771 µs (Exact 1,690-1,828) | kept; caster at parity |
| L4 | Work estimate in every cell | caster walk 642-692 µs (−13%), spans 1,622-1,648; caster timing 1.55-1.66 against Exact's 1.69-1.82 | **kept: blocker 3 met** |
| L5 | Queue sorted heaviest first into 8 / 2 / 1 buckets; phase blocks of 8³ and 16³ cells; 64k-lane spread | 8 buckets worst (camera walk 781-847 µs, turning 0.77-0.79); 2 buckets best on camera (725-766 µs) and caster (1.61-1.64); no order best on turning (0.573-0.60); phase blocks and spread no gain | 2 buckets, conditionally |
| L6 | 2-bucket order only on queues ≥ 32,768 entries (more than a wave of resident lanes), per-workgroup sort places | turning 0.584-0.592; camera 1.021 (one clean visit), trace span 897-954 µs; caster 1.578-1.649; 32k and 64k lane spreads no gain | **shipped (opt-in)** |

L5's first run failed at validation: the sort pass's bind group bound the dispatch-arguments buffer as storage while dispatching indirectly from it. It reran as L5b with the probe's group.

## Timing (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off. The binary is `fin.exe` = commit `bdb193a`, sha256 `5bfc1de7…`; rustfmt changed whitespace only afterwards (`d58011f`). Two processes ran Exact and Surface interleaved, with no approach, as N8n's table.

| No-RT quarry, shadow pass, ms | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| Exact on this binary (main's counts) | 0.559-0.614 | 0.572-0.618 | 1.493-1.503 | 1.676-1.707\* | 1.29-1.38 / 2.24-2.26 / 2.43-2.46 |
| N8n Surface (recorded, `fin4.exe`) | 0.000 | 0.571-0.594 | 1.025-1.040 | 1.715-1.773 | 0.72-0.75 / 1.76-1.80 / 2.47-2.56 |
| **N8o Surface** | **0.000** | **0.570-0.594** | **1.043-1.046** | **1.560-1.649** | **0.71-0.77 / 1.78-1.85 / 2.32-2.45** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

\* One further visit read 2.24 ms, with a 3.04 ms frame, though its primaries passed the clean rule. It is excluded as contended in the shadow pass. About half of Surface's camera visits were contended (primaries 0.66-0.83 ms), as N8n found for its light frames.

**Traced quarry on the RT device** (one visit, ms, static / turning / camera / caster):
- Exact: 0.527-0.534 / 0.545-0.553 / 1.560-1.576 / 1.691-1.750.
- Surface: **0.000 / 0.545-0.559 / 1.023-1.203 / 1.619**. The pose-0 caster visit was contended (primaries 0.48).

**Per stage, from traces of camera-moving, caster-moving and steady-turning frames** (µs per frame, four frames each):

| Dispatch | Exact, camera | Surface, camera (L6 = final code) | N8n Surface, camera | Exact, caster (L3) | Surface, caster (final) |
|---|---:|---:|---:|---:|---:|
| Page work (mark, compact, clear, cull, finish, rasters, mip) | 39.2-39.6 | 39.7-39.9 | ~39.5 | 742-752 | 738-744 |
| Probe | — | 68.6-68.9 | 67.5 | — | 86.6-87.2 |
| Order passes / sort | 4.2-7.2 | 3.5 | — | — | 5.5-5.7 |
| Lookup / queued walk | 1,395-1,434 | 677-736 | 714-737 | 878-1,018 | 649-736 |
| Insert (claim + resolve) | — | 54.1-54.6 | 35.1-37.0 | — | 90.5-92.4 |
| **Span** | **1,474-1,520** | **897-954** | **about 875** | **1,690-1,828** | **1,638-1,743** |

- **Camera walk.** As in N8n, it holds 12 warps for half its dispatch and then decays to 0 (L3 trace). Averages: 6.8-7.7 warps, 4.8 of 32 lanes active, 168 registers and 8,424-8,440 B of shared memory per warp.
- **Caster walk.** Same tail shape at 8.0 lanes; Exact's caster lookup shows the same at 10.1 lanes.
- **Steady sun turning** (wrapped traces, not the harness's 16-frame phase): Surface's probe (81 µs), walk (539-656 µs) and insert (45-56 µs) against Exact's lookup (486-636 µs) plus order (6-7 µs). On long continuous turning Surface costs more than Exact. The harness's turning phase (16 frames after a still view) has them equal.

**Hit rates** (`VSC_STATS`, medians of each phase's last 16 frames, both poses alike):

| Phase | Shown | Not shown (boundary) | Walked |
|---|---:|---:|---:|
| Camera, 1 mm a frame | 85.2% | 12.0% | 14.8% (50,800) |
| Camera, 1 cm a frame | 84.4% | 11.9% | 15.6% |
| Camera, 2 mrad pan a frame | 84.7% | 11.9% | 15.3% |
| Sun turning (still camera) | 78.8-78.9% | 14.9% | 6.3% (21,800, almost all sun refreshes) |
| Caster moving (still camera) | 60.2% | 11.0-11.1% | 29.0% (100,400) |
| Static | — | — | 0 (idle) |

Without fills (L3 counters), camera frames walk 18.6% with 81.4% shown.

## Determinism and caster gates (M)

Hashes of the counts in every phase are equal across visits and processes. Final leases: 2 timing processes with 2 Surface visits each, plus the gate process.

| Pose 0 / pose 1 counts (sha256 prefix) | Exact | Surface |
|---|---|---|
| After 16 turning frames (snapshot) | `05e50a1d8ad17426` / `bed48bf7099343bb` (main's) | `c3d5a6946c4d274f` / `e029d2ef86448e21` |
| After 48 camera frames | `c58c0822a7bc83d4` / `3db7b54ddac3f706` | `3e0786a539a9343e` / `03fe73d4bafaf2d6` |
| After 48 caster frames | `371171617a237358` / `0222cbae8e8b72a2` | `95d7350cc915693e` / `4c9b1ac32bd669af` |
| Static caster gate, rolled | `371171617a237358` / `0222cbae8e8b72a2` | **`371171617a237358` / `0222cbae8e8b72a2`** |

| Caster gates, pose 0 / pose 1 (pixels) | Exact | Surface |
|---|---:|---:|
| Static: rolled against a fresh lookup / against the final-placement twin | 0 / 0 | **0 / 0** |
| History: rolled against a fresh lookup | 2 / 2 | 780 / 732 |
| History: the still-caster twin against its fresh lookup | 2 / 2 | 948 / 917 |
| History: differing pixels the twin does not explain | 0 | **0** |
| History repeated in process | 0 differ | **0 differ** |
| N8f's identity check after the harness's full history | 2 / 2 | 516 / 500 |

- **The static gate makes Surface's rolled frame Exact's fresh frame**, byte for byte. With a static camera every shown cell was computed by pixels that all agreed and walk to it, so no reuse difference exists. No stale cell is shown, or a pixel would differ.
- **The history gate separates reuse from staleness.** A pixel showing a cell kept through the caster motion shows the same value as in the twin. Its own walk cannot have changed, because the cell's cube contains its point. Reached cells are recomputed by the pose's pixels and equal a fresh lookup.
- **Exact's own 2 pixels** come from its camera history (the level windows moved during motion; I). They are unchanged from main's Exact.
- **The N8f check (516 / 500)** is the reuse difference after the harness's camera phase: N8n's 530-570 measured the same thing.
- The table is cleared on reset, so the first visit of a process and later visits run identical sequences.

## Quality (M)

The final binary with the unchanged scorers. Exact's rows are its scores on this binary; its counts are main's.
- Quarry: `n8e-score`, 64-seed traced Exact reference built in process.
- S1: `bench-contact-fold-score`.
- S2/S3: `n8-score`, 8 timing replays, validation on.
- Exact's S1-S3 rows are N8n's `final-scores-exact`; Exact is unchanged.

✗ marks a measure below VSM; ≺ one below Exact.

**Quarry, film reference** (each pose approached by 16 camera frames of 2 mm). Two Surface visits gave identical scores:

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|
| Unreal VSM | .483 / .483 | 12,390 | 14,935 | 87 / 88 | 11,884 |
| Exact, no-RT | .353 / .348 | 2,434 / 2,474 | 3,289 / 3,366 | 60 / 55 | 2,376 / 2,424 |
| N8n Surface, no-RT (two visits) | .362-.365 / .356-.358 | 2,638-2,685 / 2,681-2,727 | 3,428-3,454 / 3,510-3,535 | 36-40 / 30-32 | 2,600-2,652 / 2,655-2,703 |
| **N8o Surface, no-RT** | **.365 ≺ / .358 ≺** | **2,685 ≺ / 2,724 ≺** | **3,452 ≺ / 3,525 ≺** | **37 / 30** | **2,651 ≺ / 2,699 ≺** |
| Exact, traced | .286 / .290 | 340 / 356 | 1,202 / 1,213 | 0 / 0 | 339 / 356 |
| **N8o Surface, traced** | **.308 ≺ / .309 ≺** | **714 ≺ / 756 ≺** | **1,447 ≺ / 1,478 ≺** | **1 ≺ / 1 ≺** | **713 ≺ / 755 ≺** |

A no-fill variant scored identically. The harness applies a variant's constants only from the pose's first frame, after the approach, so the comparison is void. Measuring fills' quality cost needs the constants applied before the approach (not done).

**S1, contact fold** (environments 0 and 1 equal):

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Contacts lost |
|---|---:|---:|---:|---:|
| Unreal VSM | .1642 / .1628 | 0 (env 1: 1) / 0 | 0 / 0 | 5-6 / 2-3 |
| Exact | .1085 / .1065 | 0 / 0 | 0 / **2 ✗** | 0 / 0 |
| **N8o Surface** | **.1085 / .1063** | **0 / 0** | **0 / 0** | **0 / 0** |

**S2 and S3 in motion** (per-phase maxima for leaks and flips; shadow-pass p50 ms with validation on):

| Bench, phase | Exact pRMS | **Surface pRMS** | VSM pRMS | Surface HDR (VSM) | Surface contacts lost (VSM) | Surface leaks / flips (VSM) | Shadow p50: Exact / **Surface** / VSM |
|---|---:|---:|---:|---|---|---|---|
| S2 camera out | .0833 | **.0832** | .1752 | .0107 (.0151) | 0 of 19,396 (2,101) | 0 / 0 (62 / 167) | 0.50 / **0.36** / 0.35 ✗ |
| S2 camera hold | .0865 | **.0863** | .1758 | .0115 (.0154) | 0 of 11,264 (1,039) | 0 / 0 (57 / 152) | 0.19 / **0.0001** / 0.35 |
| S2 camera return | .0809 | **.0808** | .1739 | .0103 (.0145) | 0 of 17,990 (1,972) | 0 / 0 (62 / 155) | 0.49 / **0.35** / 0.35 |
| S2 shadow out | .0759 | **.0759** | .1697 | .0099 (.0152) | 0 of 16,687 (2,070) | 0 / 0 (65 / 120) | 0.37 / **0.38** / 0.36 ✗ |
| S2 shadow hold | .0766 | **.0766** | .1675 | .0100 (.0150) | 0 of 8,832 (1,059) | 0 / 0 (66 / 119) | 0.14 / **0.0001** / 0.35 |
| S2 shadow return | .0759 | **.0759** | .1692 | .0099 (.0152) | 0 of 16,687 (2,057) | 0 / 0 (67 / 120) | 0.37 / **0.39** / 0.35 ✗ |
| S3 static | .1186 | **.1186** | .1878 | .0150 (.0194) | 0 of 48,512 (2,929) | 1 / 0 (111 / 187) | 0.24 / **0.0001** / 0.37 |
| S3 rotate .25 | .1217 | **.1216** | .1875 | .0151 (.0194) | 0 of 194,291 (11,630) | 1 / 0 (111 / 190) | 0.24 / **0.27** / 0.59 |
| S3 hold slow | .1266 | **.1266** | .1873 | .0152 (.0194) | 0 of 48,640 (3,006) | 1 / 0 (109 / 191) | 0.25 / **0.0001** / 0.36 |
| S3 rotate 1 | .1218 | **.1215** | .1863 | .0150 (.0200) | 0 of 196,290 (12,289) | **2 ≺** / 0 (106 / 192) | 0.24 / **0.29** / 0.59 |
| S3 hold fast | .1210 | **.1210** | .1875 | .0150 (.0205) | 0 of 49,728 (3,378) | 0 / 0 (103 / 188) | 0.25 / **0.0001** / 0.36 |
| S3 jump recovery | .1159 | **.1159** | .1811 | .0146 (.0219) | 0 of 51,777 (3,153) | 0 / 0 (114 / 226) | 0.24 / **0.0002** / 0.37 |

Surface is above VSM on every S2/S3 quality measure in every phase. Its partial RMS equals or beats Exact's in every phase. Exact's HDR RMS and contacts equal Surface's. Exact's leaks are 1 in rotate 1 against Surface's 2.

**Motion stability: frame-to-frame flicker against the film reference** (bench-motion's `residual_change_rms`, phase mean (max); VSM from the recorded per-frame rows):

| Bench, phase | Exact | N8n Surface | **N8o Surface** | Unreal VSM |
|---|---|---|---|---|
| S2 camera out | .01428 (.02288) | .01423 (.02288) | **.01423 (.02288)** | .06128 (.08098) |
| S2 camera hold | .00042 (.01340) | .00042 (.01332) | **.00042 (.01333)** | .06112 (.07462) |
| S2 camera return | .01358 (.02347) | .01353 (.02348) | **.01354 (.02348)** | .05769 (.07386) |
| S2 shadow out / return | .00488 (.00653) | .00488 (.00653) | **.00488 (.00653)** | .05270 / .05244 |
| S2 shadow hold | .00015 (.00468) | .00015 (.00468) | **.00015 (.00468)** | .05198 (.06533) |
| S3 static | 0 (0) | 0 (0) | **0 (0)** | .08033 (.09871) |
| S3 rotate .25 | .00324 (.00547) | .00325 (.00572) | **.00325 ≺ (.00559)** | .07995 (.10006) |
| S3 hold slow | .00119 (.00449) | .00123 (.00421) | **.00109 (.00453)** | .08014 (.09963) |
| S3 rotate 1 | .00627 (.03613) | .00631 (.03643) | **.00633 ≺ (.03633)** | .08155 (.10333) |
| S3 hold fast | .00214 (.00663) | .00212 (.00667) | **.00203 (.00703)** | .08239 (.10216) |
| S3 jump recovery | .00161 (.05327) | .00161 (.05327) | **.00161 (.05327)** | .08644 (.10644) |

Surface's flicker is at least 4x below VSM's in every moving phase (4-350x). It is equal to or below Exact's in 9 of 11 phase means, and 0.3% (rotate .25) and 1.0% (rotate 1) above it in two.

## Memory against Unreal (M)

| No-RT quarry, MiB | Allocator reserved | Allocated | Against Unreal VSM's 3,912 |
|---|---:|---:|---|
| Exact | 3,439.7 | 3,344.8 | 472 below |
| N8n Surface | 3,503.7 | 3,392.6 | 408 below |
| **N8o Surface** | **3,503.7** | **3,424.6** | **408 below** |

- The claim words add 32 MiB, which fits in the allocator's existing block slack (reserve unchanged); the queue's tail grows from 16 to 128 bytes.
- The table and claims exist only while Surface is the lookup.
- The RT device reads 4,137.6-4,201.6 MiB reserved for both lookups; there, Unreal's VSM-RT-on total is 3,946 and HRT's 5,410 (N8h).

## What was built (M)

- **`shaders/virtual_shadow_cache.wgsl`.**
  - New: `vsc_claim`, `vsc_resolve`, `vsc_sort`, the region reach test, margins, fills with masks, a work estimate in every cell, and the bucketed queue. `vsc_walk` now uses Exact's `vsm_prepare`.
  - Removed: `vsc_insert`, the rejected N8n screens `VSC_REUSE` 0/1/3, `VSC_CANONICAL`, `VSC_JITTER`, `VSC_TRIANGLE` and `VSC_FINER`, and the two-ended heavy queue.
  - New screen constants: `VSC_SORT`, `VSC_SORT_MIN`, `VSC_UNKNOWN`, `VSC_PHASE_BLOCK`.
- **`src/virtual_shadow_cache.rs`.**
  - The claims buffer and the table clear on flush.
  - The new passes' pipelines and groups; the sort pass is bound with the probe's group.
  - `QUEUE_TAIL`, the sun-epoch width, and counters for margin overflow and fills.
  - The binding unit test covers the new passes.
- **`src/virtual_shadow_gpu.rs`.** A new height range or level ladder clears the surface table; the queue tail is sized by `QUEUE_TAIL`.
- **Harness** (tests only):
  - `caster_gates`;
  - counts hashes after the camera and caster phases;
  - `caster_frames`/`caster_wrap` and `turn_wrap`;
  - `--caster-gates` and `--move-wrap` in scripts/n1c-batch.py.
- **Unchanged:** shaders/virtual_shadow.wgsl and Exact's output (main's hashes above), `VirtualShadowSettings::default()` (Exact), and docs/milestones-5-6-decisions.tsv.
- **Programmability.** Cells hold visibility counts and bookkeeping only. No lighting-model data entered hit records, clusters, maps or the lookup, which still outputs the counts user shading programs consume.

## Misses, honestly

- **Not the default.** Surface does not reach Exact's quality on every measure:
  - the quarry's partial RMS, flips, leaks and lost edges (+2.9-12%), traced and no-RT;
  - S3 rotate 1's leaks (2 against 1);
  - flicker in two S3 phases (+0.3% and +1.0%).

  It is above Unreal VSM everywhere. The cause is N8n's reuse of cells computed at other camera positions, which no gate here can make zero while cells are shown away from the points that computed them. Fills double that difference. Measuring their quality cost needs the harness to apply variant constants before the approach.
- **Camera motion.** 1.043-1.046 ms against VSM's estimated ~0.5, about 2x; N8n's 1.025-1.040. The walk of the ~15% boundary and new pixels is tail-bound (12 warps for half the dispatch, 4.8 of 32 lanes). Every order tried left the tail:
  - heaviest first in 2 or 8 buckets;
  - wider spreads;
  - coarser refresh blocks.

  The two-pass insert costs 54 µs against N8n's 36.
- **Ideas not tried, with reasons:**
  - *Penumbra cells with per-direction results.* Every pixel's eight directions are jittered per pixel. Another pixel's per-direction result is a different sample, and N8n measured cell-keyed sampling raising S2's flicker 39%: the flicker gate would fail.
  - *Walks seeded from a neighbour's blocking triangle.* A hit on the seed proves a block that Exact's march might miss. That changes walking pixels away from Exact's counts, and with them both identity gates.
  - *Prefetching along the motion.* It would address only the ~3% of new cells.
  - *Folding `vsm_mark` into the probe.* About 25 µs.
- **Steady sun turning.** Over long continuous turning (wrapped traces) Surface's turning frames cost more than Exact's (about 0.70-0.77 against 0.50-0.64 ms traced). The harness's 16-frame turning phase has them equal. Not diagnosed.
- **S2's moving phases** read 0.35-0.39 ms against VSM's 0.35-0.36 (validation on).
- **Carried:** Exact's own misses are unchanged (S1 pose 1's 2 leaks, which Surface does not have). Not film quality in any mode, and never presented as meeting the film gate.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8o/`):
  - `l1-*` … `l6-*`, `final-t`, `final-tb` (timing), `final-g` (gates), `final-s1mm`/`-s1cm`/`-span` (counters), `final-tt` (traced timing), `final-q` (quality, `n8e-scores.json`) and `final-scores-surface` (S1-S3).
  - Trace exports: `*-mp`, `*-cmp`, `*-tmp`, `occ.json`.
  - `scripts/`:
    - `gates.py` (hashes and gates);
    - `frames.py` (per-frame dispatch sums);
    - `summ.py`, `cache2.py`, `s23.py`, `flicker.py`, `occ.py`, `ranges.py` (N8n's).
  - `l*.sh`, `final.sh`, `score.sh`, `lease.sh`, `trace.sh`: the lease scripts. `edit-*.py`: the scripted edits. `binaries.sha256` and `runs/`: every lease's log and preflight.
  - Raw dumps over 5 MB (trace files, staged binaries, quarry dumps, S1-S3 frames) were deleted after export and scoring, and are listed in `deleted-raw.txt`. So was the Cargo target.
- **Gate.**
  - `cargo test -p renderlab --lib -j 2` exited 0 inside lease final-b, on `bdb193a`: 113 passed, 0 failed, 112 ignored.
  - Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab (all targets) and renderlab-viewer, and fmt passes, on `d58011f`.
- **Leases.** Nine under the shared lock with the 4/2 GiB guard, at least 19.8 GiB available at every start, 2,485 s in all:
  - L1 167 s, L2 266, L3 357, L4 243;
  - L5 306 (failed validation), L5b 336, L6 271;
  - final-a 381, final-b 158.

  A first final lease was refused (the runner allows at most 45 minutes). A second started with a stale script and was stopped before any GPU job; its lock was released with a schedule line. No game ran, and no window, focus change or capture was used.
- **Time (UTC).**
  - 09:15-09:32: reading and design.
  - 09:32-10:00: the deterministic inserts, caster gates, fills and work estimates (L1-L4).
  - 10:00-10:28: walk-order screens (L5-L6).
  - 10:28-10:37: the final leases.
  - 10:37-10:50: scoring, clippy and fmt, cleanup and this record.

  GPU leases total about 41 minutes; implementation, CPU scoring and analysis took the rest.
