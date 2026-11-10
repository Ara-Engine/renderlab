# P-STILL: cost that follows change

Unit P-STILL, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10, branch `t3/p-still` rebased on main v0.1.3 (`ebfc21b`). This is a design unit with a counted-work gate: count the cache work that TR1's per-frame jitter causes, then build only on a clear win. Labels: **M** measured here, **C** read from code, **A** another unit's measurement, **I** inference.

## Result

**MERGE**: GI idleness is now keyed on the unjittered camera and the target size (`src/gi_cache.rs`, one hunk).

- **Count gate (M).** On jittered still quarry frames, **all of the GI cache's work is jitter-only**: 0.57-0.66 ms of a 3.30-3.35 ms RT frame (17-20%) and 0.91-1.51 ms of a 7.96-8.59 ms non-RT frame (11-18%). With jitter off the cache does no work and idles.
- **Cause (C, M).** The GI idle key hashed the jittered projection and the jittered `scene_pixels` rectangle. The quarry has background at its edges, so the rectangle moves a pixel with the jitter. A still view therefore never idled.
- **Effect (M).** Still views idle after GI's existing drain bound (272 identical frames with the default gather cap):
  - RT: 3.30 → 2.63 ms (-20%), now **1.71x faster than HW Lumen Epic** (4.50), up from 1.36x;
  - non-RT: 7.96 → 7.03 ms (-12%), now **1.75x slower than SW Lumen Epic** (4.02), down from 1.98x.
- **Yardstick unchanged (M).** P-YARD2's 160-frame still segment never reaches the bound. All 125 captured yardstick frames are **byte-identical** before and after, on both paths, so P-YARD2's still, moving, cut and flicker numbers stand as measured.
- **Flicker (M, P-YARD2's scorer on the same manifest):** RT 4.392 before, 4.392 after; non-RT 4.438 before, 4.438 after. Unreal is at 3.66-3.79. This unit does not move it: flicker is scored on moving frames, which nothing here touches.
- **Quality (M, inspected):**
  - Idle still frames match the running cache within 1.1 codes, with no pixel over 2 codes.
  - The first walk frames after idling differ on 1-3 pixels by up to 2.3 codes.
  - After a later cut, RT differs on one 3x8 px gravel-gap patch and one pixel pair (17-20 pixels over 8 codes). Both are closer to the path-traced reference with the change (patch RMS 28.3 against 30.1). Every scored frame's full-frame error is equal or lower.
- **Not taken (M):**
  - Non-RT still sun visibility (4.2 ms) is TR1's penumbra sampling, not jitter waste: about 70k uncertified pixels walk 8 rotated directions a frame. Stopping them freezes noise.
  - Virtual-shadow page work under jitter costs only about 0.05 ms.
  - The lagged "drained" readback never fires under jitter, because the cache keeps creating about 3-54 cells a frame (section 4).

| Quarry, 1080p, GPU span p50 (ms) | Before | After | vs Unreal Epic, before → after |
|---|---:|---:|---|
| RT still, yardstick (frames 96-159) | 3.35 | 3.35 | 1.34x faster → same (identical frames) |
| RT moving | 4.29 | 4.28 | 1.06x faster → same |
| RT cut | 12.67 | 12.74 | 2.80x slower → same (one frame each, identical image) |
| **RT still, held 272+ frames** | **3.30** | **2.63** | **1.36x → 1.71x faster** |
| non-RT still, yardstick | 8.64 | 8.64 | 2.15x slower → same |
| non-RT moving | 10.77 | 10.78 | 2.62x slower → same |
| non-RT cut | 44.46 | 40.61 | 10.8x slower → same work (identical image; single-frame noise) |
| **non-RT still, held 272+ frames** | **7.96** | **7.03** | **1.98x → 1.75x slower** |
| Flicker, moving (RT / non-RT) | 4.392 / 4.438 | 4.392 / 4.438 | Unreal 3.66-3.79: unchanged |

Unreal Epic numbers are from P-YARD2: HW Lumen 4.50 / 4.57 / 4.52, SW Lumen 4.02 / 4.12 / 4.11 for still / moving / cut.

## 1. Where "still" is decided (C)

TR1 jitters every reconstructed frame by one offset of a 256-point lattice, carried by the projection. P-NSQ rotates non-RT sun directions with the same index.

| Decision | Keyed on (main) | Under TR1 on a still camera |
|---|---|---|
| TR1's still test and shadow budget (`sampling_still`, `held_rays`) | the unjittered matrix, as CPU bits | Correct. RT sun rays settle at 8 a pixel; no jitter cost was measured (receiver 0.50 ms with and without jitter). |
| **GI cache idle** (`idle_key`, `view_key`) | the **jittered** matrix and jittered `scene_pixels` rectangle | **Never idle. Fixed here.** |
| GI idle bound with the gather cap | 16 + 64 x (gathers + 1) = 272 identical frames | Unchanged (section 4). |
| Virtual-shadow page work | the jittered matrix plus P-NSQ's rotation | Runs every frame, but costs about 0.05 ms (M). Left as is. |
| Surface-cache "still" (walkers keep last count) | page work off | Never still, which is correct: freezing walkers would freeze their 8-direction sample. |

## 2. Counts (M)

Quarry, 1080p, RTX 4070 SUPER, NVIDIA 617.14, release `p_yard_capture` (main v0.1.3 + branch), per-frame counters read back (`RENDERLAB_P_STILL_COUNT`). The manifest is P-YARD's pose p0 held for 448 frames, then its walk, hold, return, cut and recover (`manifest-count.json`). Medians per state. GI ms is the cache's own profiled passes.

| Run | State | span | GI ran | GI ms | cells created | GI rays | gathers | sun walks | page work |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| RT base | still < 272 | 3.35 | 100% | 0.660 | 54.5 | 1,088 | 23 | - | - |
| RT base | still ≥ 272 | 3.30 | 100% | 0.572 | 3 | 0 | 1 | - | - |
| RT no jitter | still < 272 | 3.20 | 100% | 0.503 | 0 | 0 | 0 | - | - |
| RT no jitter | still ≥ 272 | 2.61 | **0%** | 0 | 0 | 0 | 0 | - | - |
| **RT P-STILL** | still ≥ 272 | **2.63** | **0%** | 0 | 0 | 0 | 0 | - | - |
| RT base | walk | 4.31 | 100% | 0.707 | 143.5 | 5,056 | 153 | - | - |
| non-RT base | still < 272 | 8.59 | 100% | 1.510 | 67.5 | 1,472 | 35 | 70,780 | 100% |
| non-RT base | still ≥ 272 | 7.96 | 100% | 0.906 | 2 | 64 | 2 | 70,124 | 100% |
| non-RT no jitter, no rotation | still ≥ 272 | 2.75 | 0% | 0 | 0 | 0 | 0 | 196 | 0% |
| non-RT page key without jitter or rotation | still ≥ 272 | 7.98 | 100% | 0.898 | 2 | 64 | 2 | 70,120 | **0%** |
| **non-RT P-STILL** | still ≥ 272 | **7.03** | **0%** | 0 | 0 | 0 | 0 | 70,124 | 100% |

Jitter-only share on still frames:

- **GI:** 100% of its work, which is 0.57-0.66 ms RT and 0.91-1.51 ms non-RT. About 0.5 ms of that is simply running the passes; the "no jitter, still < 272" row runs them with zero work. The rest is real cell creation that the sub-pixel offsets keep causing (54 cells a frame early on, about 3 after 272 frames).
- **RT sun rays:** none.
- **Non-RT sun visibility:** 4.2 ms, but it is caused by P-NSQ's rotation, not by change. Stopping it, as in the no-jitter/no-rotation row, freezes each penumbra pixel's 8-direction sample.
- **Non-RT page work:** about 0.05 ms.

Game and Showcase were not counted. Showcase's still capture (60 frames after 16 settle) never reaches 272, so it cannot change (I). P-SHOW's 3.4 ms of "still" GI on Showcase is the cut's drain plus this jitter cost, not jitter alone.

## 3. The change and its screen (M)

`prepare_gi` now hashes `camera.unjittered()` and the target size instead of the jittered matrix and the jittered `scene_pixels` rectangle. The rectangle follows from the camera, the hashed placements and the size. Nothing else changes:
- the drain bound;
- the passes;
- what an idle frame skips;
- environment-only frames.

The GI values are deterministic per key (W1: replaced from fixed direction sets), so an idle frame shows what a running one would. Only cells that later jitter offsets would have created are missing. The screen bounds that effect.

Screen lease `pstill-s1`: 10 runs, one binary, `RENDERLAB_P_STILL=gi-jittered` restoring main's key as the base.

- **Yardstick, captured frames:** 125 of 125 byte-identical, both paths.
- **Long still** (`manifest-still-capture.json`: frames 150, 300, 447, walk 456, cut 608 and recover 609/616/638):

| Frame | RT: pixels > 8 / > 2 codes, max | RT error base → after | non-RT: > 8 / > 2, max | non-RT error base → after |
|---|---|---|---|---|
| 150 still (not idle yet) | 0 / 0, 0 | 6.635 → 6.635 | 0 / 0, 0 | 7.286 → 7.286 |
| 300 still (idle) | 0 / 0, 0.06 | 6.630 → 6.630 | 0 / 0, 0.14 | 7.263 → 7.263 |
| 447 still (idle) | 0 / 0, 0.33 | 6.628 → 6.628 | 0 / 0, 1.13 | 7.254 → 7.254 |
| 456 walk, 8 frames after idle | 0 / 3, 2.29 | no reference | 0 / 1, 2.13 | no reference |
| 608 cut | 20 / 53, 33.4 | 7.506 → 7.503 | 1 / 26, 8.2 | 8.035 → 8.034 |
| 609 / 616 / 638 recover | 17-20 / 51-56, 19-30 | 7.448 → 7.445 ... 6.802 → 6.800 | 1-2 / 37-44, 8.4 | 7.994 → 7.993 ... 7.394 → 7.393 |

The RT cut differences are one GI cell's patch in a gravel gap under the slab (3x8 px at (630-635, 570-578)) and a pixel pair at (559, 614). The cells differ because that region was not probed while idle. At 4x zoom, base and change look the same (`D:/Coding/renderlab-work/p-still/sheets/cutdiff-rt-609.png`; idle still sheets `still-{rt,nonrt}-NNNN.png` there show a black x8 difference). The changed patch is closer to the reference (patch RMS 28.3 against 30.1; pair 28.4 against 30.3; still so at recover frame 30). No stray bright pixels, grain or edge changes.

## 4. What remains, ranked

1. **Flicker on moving frames, 4.39 / 4.44 against Unreal's 3.66-3.79**, is untouched and is still the top visible gap. It needs its own unit on moving frames:
   - GI cells created under motion (143 a frame RT, 689 without jitter's pre-seeded neighbours);
   - the 32-direction moving sun budget;
   - TR1's history weights.
2. **Idle sooner than 272 frames.** The jittered cache keeps creating cells: 54 a frame for the first 272 frames, about 3 after.
   - Likely cause (I): `evict_frames` is 120, below the jitter's 256-frame period, so cells seen only at some offsets are evicted and re-created.
   - An eviction window of at least a jitter period, or no aging on still views, would let the counts reach zero. Then a lagged "drained" readback (prototyped and dropped here, because it never fired under jitter) could idle the yardstick's still frames too, worth 0.57-0.66 ms RT and 0.9-1.5 ms non-RT.
3. **Non-RT still sun visibility, 4.2 ms**, is about 70k rotated 8-direction walks a frame. A per-pixel converged penumbra (stop after a full rotation period, resume on change) would need TR1's history to hold it without freezing noise or aliasing shadow edges. That is a design of its own.
4. **Page work under jitter** (about 0.05 ms): not worth the risk of missing pages at boundaries.

## 5. Patent notes ([m7-p0-claims.md](m7-p0-claims.md); policy in AGENTS.md)

- **The change** only decides whether the GI cache runs on an unchanged view, from CPU bits. It is not an element of any flagged claim.
- **W1 holds** (no counter or average; US 10,614,613 (f)-(g)).
- **W2 holds:** nothing ranks cells or chooses rays (US 11,790,594).
- **W3 holds:** lookups stay deterministic (US 12,423,782).
- A world-space cache alone is allowed. No resampling is involved (US 11,315,310).
- **Rejected, and not needed:** accumulating jittered samples into converged cells. That is a per-cell running average: US 10,614,613 (e)-(g) plus dependent 2 (jitter before quantizing), which W1 and C8 design around.

## 6. Harness (kept)

- **Knobs:** `RENDERLAB_P_STILL` (`temporal_reconstruction::p_still`) takes `nojitter` (offset 0, the attribution baseline) and `gi-jittered` (main's GI key, for A/Bs). Both are unset in shipping runs, and unset is the screened default path.
- **Counts:** `tools/p-still/count.rs`, included by `examples/p_yard_capture.rs` and `examples/ara1_capture.rs` and switched on by `RENDERLAB_P_STILL_COUNT`. Per frame it records:
  - GI ran, counters and pass profile;
  - virtual-shadow page work;
  - surface-cache queued walks (`Renderer::p_still_surface_walks`, doc-hidden);
  - TR1's shadow budget.
- **Scripts:**
  - `tools/p-still/run.py`: steps; it stops its capture process below 5 GiB free;
  - `counts.py`;
  - `flicker.py`: P-YARD2's scorer;
  - `stillcheck.py`: long-still A/B against references;
  - `host.py`: admission before locks, 10 GiB start, free-RAM-prefixed schedule lines, TMP/TEMP on D:, unreleased requests older than an hour treated as stale.
- **Manifests** live in `D:/Coding/renderlab-work/p-still/`.

## 7. Process (M)

- **Session 1** (12:13-15:14 UTC): reading, code and harness. Two builds.
  - Lease `pstill-c1` was killed by this unit's own build waiter racing it (memory `own-build-and-lease-waiters-race`).
  - Lease `pstill-c2` died in its admission check with WinError 1455 during the C:/commit emergency, and was not retried.
- **Session 2** (16:31-18:30): rebased on v0.1.3 with no conflicts, then:
  - build 151 s, then count lease `pstill-c3` (6 runs, 91 s);
  - candidate build 40 s, then screen lease `pstill-s1` (10 runs, 152 s).
  - Free RAM stayed 13-18.6 GiB, and every build and lease was admitted before taking locks.
- **Time:** about 1.5 h of code and analysis, about 4 min of GPU, and about 3.5 h queued behind Ara work and the memory emergency.
- **Cleanup:** Cargo target, binaries, TMP and raw HDR captures (16.6 MB each) deleted. `D:/Coding/renderlab-work/p-still` keeps the logs, `engine.json` timings and counts, `counts-summary*.json`, `stillcheck.json`, `score/quality.json`, the manifests and the inspected crops.
