# N8f: the non-RT shadow pass under camera and caster motion, profiled

Follow-up unit to [N8e](non-rt-n8e.md), 2026-11-06 UTC, base `8a45027`, branch `t3/n8f-shadow-motion`. Brief (Islam, 2026-11-06): cut the virtual-shadow pass on frames where the camera or a caster moves (quarry 2.92-3.16 ms in N8e, against Unreal VSM's estimated 0.5 ms and a 1.61 ms no-RT frame target), with every quality measure at or above VSM's. Profile first, with Nsight's GPU Trace, and find out why Unreal pays so little. Labels: **M** measured here, **I** inference.

## Result

**Two levers were kept, and both leave the output byte-identical to N8e's.**

1. **Casters-only rolling frames.** These are frames where a caster moves and the camera holds still. They now keep every count the moved casters cannot reach: no screen-space reprojection, just the moved casters' light-space and pixel bounds. Quarry caster-moving frames fall from 3.21-3.33 to 2.30-2.34 ms (−29%). S2's caster-motion phases fall from 0.68 to 0.44 ms.
2. **Page work only for stale pages.** Camera-moving frames skip the caster cull and rebuild max-mips only inside pages rasterized that frame: −0.04 to −0.09 ms (2-3%).

**Camera-moving frames still cost 2.53-2.58 ms on the no-RT quarry, against VSM's estimated 0.5 ms, and miss.** The profile explains it. The lookup is bound by memory latency, not throughput: SM throughput is 10-14% and long-scoreboard stalls are the largest share. The hot code is the exact walks' dependent geometry fetches (index, then vertex, then neighbour). Only 9.4-9.8 of 32 lanes are active per instruction. The SMs hold about 8 resident warps in the first half of the dispatch, then run a tail of 45% of its duration at 2.5 warps or fewer. Unreal's projection is cheap because per pixel it only reads depth texels; ours proves its blocks exactly against triangles.

Three restructurings aimed at this cost were built and measured. All three left camera-moving frames as slow or slower:
- deferring heavy receivers into dense waves (byte-identical, but 0.5-0.8 ms slower);
- changing the dispatch's launch order;
- N8e's lit-where-undecided cap.

So no camera-motion lever above 20% was found. That miss is recorded here.

| No-RT quarry, shadow pass, ms (M; same binaries, interleaved visits, clean ranges, poses 0 and 1) | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| N8e (base binary) | 0.82-0.86 | 0.83-0.85 | 2.61-2.71 | 3.21-3.33 | 1.55-1.64 / 3.38-3.65 / 4.01-4.09 |
| **N8f** | **0.81-0.87** | **0.82-0.83** | **2.53-2.58** | **2.30-2.34** | **1.55-1.64 / 3.33-3.42 / 3.11-3.14** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

Traced quarry screen (M, pose 0, same binaries, clean visits): static 0.70-0.74 → 0.71-0.78 ms, turning 0.70-0.77 → 0.72-0.78 ms, camera moving 2.91-2.94 → 2.64-2.87 ms; a dense lookup costs 3.10-3.50 ms in both. The camera-moving screen steps the camera 1 mm a frame (N8e's), which also re-requests pages. The caster-moving row moves one quarry placement (a 1 M-triangle terrain chunk, a large caster) 1 mm a frame at pose 0's camera. That re-rasterizes every page under its light-space bounds at all three levels, and most of the image is reached. A small caster reaches far less (S2, below).

**S2** shadow p50 ms (M, final binary vs N8e's final2 run; VSM 0.35-0.36): camera out 0.66 → 0.63, hold 0.20 → 0.20, return 0.65 → 0.62; **shadow-step out 0.68 → 0.44**, hold 0.15 → 0.15, **return 0.68 → 0.44**. **S3**: 0.24-0.25 ms in every phase, unchanged (VSM 0.37 static, 0.59 rotating).

## Why the pass costs this much (M)

**Profiler.** The run used Nsight Graphics 2026.4.0's GPU Trace with the Top-Level Triage metric set, in two traces. The first had the real-time shader profiler (210,646 samples in the lookup's longest dispatch). The second had multi-pass metrics. Both captured the no-RT quarry with the camera moving 1 mm a frame, on the final shader (its dense lookup equals N8e's). Clocks were left unaltered, there was no screenshot, and no window opened. The traced frames came about 1,000 frames into the motion, so their view had moved about 1 m and their lookups took 1.41-2.94 ms; the cost structure is the point, not those times. Exported data is in `E:/renderlab-work/n8f/p4-rt/analysis`, `p4-rt/explore`, `p4-rt/hotspots.txt`, `p4-mp/metrics.json` and `p4-mp/series.json`.

| Over the lookup dispatch | Value |
|---|---|
| SM issue throughput | 10.4% (trace 1), 13.9% (trace 2) of peak; IPC 0.42-0.56 of 4 |
| L2 / VRAM / L1TEX throughput | 8.6-13.7% / 8.9-9.2% / 6.2% |
| L1TEX hit rate / L2 hit rate for L1TEX | 44% / 88% |
| Active threads per warp | 9.4-9.8 of 32 (29-31% coherence) |
| Resident compute warps per SM | 3.6-4.9 average of 48; about 8.4 in the dispatch's first 55%, then 2.5 falling to 0 |
| Register allocation | about 168 per thread (26,280 per SM at 4.9 warps), which allows up to 12 warps per SM |
| Stall reasons (sampled) | long scoreboard 41.7%, wait 25.3%, selected 12.6%, short scoreboard 5.7%, branch resolving 5.2%, no instruction 3.0% |

The front end waits for the dispatch to drain 99.9% of the time (it launches every workgroup early). Over the dispatch, SM throughput is about 26% for the first 55%, then falls from 8.6% to 0.1% in the tail.

**Hot source lines** (share of the lookup's samples; WGSL lines of `shaders/virtual_shadow.wgsl`):

| Share | Code | Main stall |
|---:|---|---|
| 17.3% | `vsm_triangle`: three index loads, then three vertex loads (L426-428) | long scoreboard 65% |
| about 11% | the watertight test's arithmetic (`vsm_hits_sheared`, `vsm_difference`), waiting on loaded vertices | long scoreboard, wait |
| 8.0% | adjacency loads: ring fans (L652) 4.4%, walk steps (L932) 3.6%; `vsm_neighbour` itself another 3% | long scoreboard 93-95% |
| 4.8% | `vsm_same` position compares on loaded vertices (ring corners, walk ownership) | long scoreboard 88% |
| 4.7% | the eight-direction loop in `vsm_hits_mask` (L855) | wait, branch resolving, no instruction |
| 4.6% | model transforms of loaded triangles (L908, L862) | long scoreboard 85% |
| about 5% | march slot loop, cap check, last-candidate select (L1187, L1214, L1208) | wait, branch resolving |

About 1.8% is attributed to `vsm_reached`'s level loop, which camera frames never run, so source-line attribution is approximate at that level.

**Decomposition** (M, timestamps on the traced quarry, dense lookups, the cleanest of a contended screen): dispatch with immediate writes 0.11 ms; plus receiver setup 0.34-0.47; plus the own triangle, ring horizon and ring walk 0.80; plus the far march and its walks 2.48. The walks are about 1.2 ms of the march (N8d's split, with N8e's cap).

**Work distribution** (M, `VSM_DEBUG` trace at the defaults, pose 0, 346,517 receivers; `p3/waves-pose0.json`, `scripts/waves.py`). Work is ring triangles plus walk visits. Lit receivers (310 k) do 60% of it, mostly the 16% of them whose ring walk visits about 14 triangles. Umbra receivers (30 k) do 34%: p50 16, p90 53, p99 114. Partial receivers (6 k) do 5%. Summed over 8x4 waves, 32 times the slowest lane is 3.8 times the lane total. Ring-only work accounts for 0.67 of that wave cost, walks for 0.46 (they overlap).

**Why Unreal pays almost nothing (I, from the measurements above and N8's account).**
- **Per pixel, SMRT reads at most about 8 rays × 8 depth texels and stops early when a wave agrees.** Those are independent texture reads with no pointer chasing, and they do not depend on the geometry near the receiver. Unreal's quarry projection costs 0.14-0.16 ms (recorded).
- **Ours does exact geometry instead.** Every candidate texel and every ring triangle is fetched as index, then vertex, then neighbour, three dependent memory round trips. With 3.6-8.4 resident warps per SM, nothing hides that latency: 42% of samples wait on it, and the SMs run at 10-14% of peak.
- **Our fixed costs alone exceed Unreal's whole projection.** Setup, the ring and the march's texel reads come to about 0.8-1.3 ms. Setup alone is twice Unreal's projection.
- **Divergence and the tail roughly double the effective cost of the walks.** About 70% of lanes idle per instruction, and the tail of slow warps sets the dispatch's length.
- **Under camera motion both engines recompute every pixel every frame.** Unreal's recompute is about 16x cheaper per pixel. It is cheap because it is texture-only, which is also why VSM loses contacts, leaks, flips and darkens lit receivers by normal offset (12,390 flips and 14,935 leaks on the quarry, against our 2,434 and 3,288).

## What was built (M)

`shaders/virtual_shadow.wgsl`, `src/virtual_shadow_gpu.rs`, the no-RT quarry harness (`src/visibility_buffer_tests.rs`, `scripts/n1c-batch.py`), `viewer/src/prepared.rs` and the test instance flags (`src/fresh_shadow_tests.rs`).

- **Casters-only rolling frames.** A frame whose only change is moved casters still rolls: the same camera, sun basis, receivers and level windows, with no new resources, settings change or rebuild. The lookup then visits every tile. Each tile looks up its pixels if it is this frame's tile of its refresh block, or if `vsm_reached` holds for a pixel; every other pixel keeps its count. `vsm_reached` holds when the pixel lies inside a moved caster's old or new bounds projected to pixels, padded by 2 px. Inside them its receiver may have changed; with a still camera, nothing outside them can. It also holds when, at any level the receiver's march visits, its march neighbourhood (the radius `vsm_mark` requests pages for) overlaps a moved caster's light-space rectangle (old and new bounds). Nothing else a count depends on changes, so the kept counts equal a dense lookup's. The harness gates this exactly: after 48 caster-moving frames, the rolled counts equal a dense lookup of the final placement, 0 differing pixels at both quarry poses. `VsmFrame` gains the pixel rectangles (`FRAME_BYTES` 912 → 1168), and `n8e.w` bit 0 marks such a frame.
- **Page work only for stale pages.** `vsm_compact` sets the caster cull's indirect dispatch when it flags a page (zeroed each page-work frame), so a frame with no stale page dispatches no cull group. `vsm_mip` rewrites a mip cell inside one page only when that page was rasterized this frame. Coarser mips are always rebuilt, as they are cheap. The other pages' top words are unchanged, so their mips are too. The outputs stay byte-identical.
- **Harness.** `move_frames` / `move_batch` (`--move-frames`, `--move-batch`) time camera-moving, then caster-moving, frames after each no-RT configuration, plus the identity gate above. `Prepared::model` / `set_model` move one placement. `WGPU_DEBUG=1` (`InstanceFlags::with_env` in the test devices) adds WGSL line info for profilers.
- **Programmability.** Kept counts are visibility only. No lighting-model data entered hit records, clusters, maps or the motion test, and the pass still outputs the visibility counts that user shading programs consume.
- **Memory.** 16 B for the cull arguments. The pixel rectangles are part of the uniform.

## Tried and rejected (M)

- **Deferred lookups** (`E:/renderlab-work/n8f/variants/c4-deferred/`). The first pass stopped a receiver once its ring triangles plus walk visits passed a budget (2, 4, 8 or 16) and queued its pixel. A second, indirect pass looked the queued pixels up from the start, 32 to a wave. The trace model promised 34-43% less ring and walk wave cost at budgets 2-4. Hashes equalled the one-pass output at every budget, but camera-moving frames read 3.25-3.53 ms against the controls' 2.65-3.05. The repeated setup and march work for queued receivers, the second pass and the budget flags in the hot loops (cf. N8d's codegen hazard) cost more than packing saved. It also needed an eleventh storage buffer per stage. N8c's compaction failed the same way. Removed.
- **Launch order** (`VSM_ROW_SHIFT`, screen only). Rotating the dispatch's tile rows so the heavy band (tile rows 108-189 of 270) launches first, last or in the middle gave 2.50-3.22 ms on camera-moving frames against the controls' 2.60-2.64, within noise. The tail is not a launch-order effect. Removed.
- **Lit-where-undecided cap** (N8e's `VSM_CAP_RULE` 2, at caps 16, 32 and 64). Camera-moving frames read 2.45-2.73 ms against 2.60-2.64, with umbra receivers lit against Interactive rising from 1,438 to 4,830-5,169. There was no gain resolvable above contention. The heaviest receivers include lit ones (up to 249 walk visits), which this rule cannot bound before its cap. Not kept.
- **Prompt cap** (modelled only). Checking the walk cap between refinement walks, not only between march texels, bounds umbra receivers' tails (35.6% exceed 16 visits). On the trace it cuts ring and walk wave cost by 12%, about 8% of the lookup. Not built: too small for its quality cost.
- Not retried: reprojection of kept counts under camera motion (N8e reverted it; the brief excludes it).

## Quality (M)

Every retained change is byte-identical to N8e's output on the frames each bench scores.
- **Quarry:** the traced static and turned hashes (`2d6e4fa6ca32…`, `88888216c1d9…`) equal N8e's final2 default, and the no-RT 0.35 px counts equal the base binary's at poses 0 and 1. So the quarry scores are N8e's and were not recomputed.
- **Caster-moving frames:** equal to a dense lookup by the identity gate.
- **S1, S2, S3:** rescored on the final binary with the same scorers. They equal N8e's to every printed digit, including S2's shadow-step phases, which now run as casters-only rolling frames.

| Bench / mode | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges / contacts | Unreal VSM |
|---|---:|---:|---:|---:|---:|---|
| Quarry pose 0, no-RT 0.35 px | .3529 | 2,434 | 3,288 | 60 | 2,376 | .483 / 12,390 / 14,935 / 87 / 11,884 |
| Quarry pose 1, no-RT 0.35 px | .3476 | 2,474 | 3,366 | 55 | 2,424 | lit darkened 88 |
| Quarry pose 0 / 1, traced | .2863 / .2896 | 340 / 356 | 1,201 / 1,213 | 0 / 0 | 339 / 356 | as above |
| S1 pose 0 / 1 | .1085 / .1065 | 0 / 0 | 0 / **2** | — | contacts lost 0/419, 0/335 | .1642 / .1628; flips 0; leaks 0 / 0; lost 5-6 / 2-3 |
| S2 camera, out / hold / return | .0833 / .0865 / .0809 | 0 | 0 | — | 0 | .1752 / .1758 / .1739; leaks 57-62; flips 152-167 |
| S2 shadow step, out / hold / return | .0759 / .0766 / .0759 | 0 | 0 | — | 0 | .1697 / .1675 / .1692; leaks 65-67; flips 119-120 |
| S3, all six phases | .1159-.1266 | 0 | 0-1 | — | 0 | .1811-.1878; leaks 103-114; flips 187-226 |

HDR RMS is .0099-.0115 on S2 and .0146-.0152 on S3, against VSM's .0145-.0154 and .0194-.0219. Not film quality, and never presented as meeting the film gate.

## Misses, honestly

- **Camera motion.** The quarry costs 2.53-2.58 ms (no-RT) and 2.64-2.87 ms (traced) against VSM's estimated 0.5 ms, and the no-RT frame is 3.33-3.42 ms against 1.61. The profile found no lever above 20% that keeps quality: page work is 3% of the frame, and the latency and divergence levers measured no gain or were slower. Reaching VSM's cost needs per-pixel decisions that are mostly texture reads, with exact geometry kept where quality needs it (contacts, thin margins), or world-anchored reuse of exact results. Neither was built here (I).
- **Caster motion on the quarry.** 2.30-2.34 ms, with a 1 M-triangle chunk as the caster; VSM has no recorded row for it. **S2's caster phases** are 0.44 ms against VSM's 0.36.
- **Static** 0.81-0.87 ms against VSM's 0.50, as in N8e. **Turning** met (0.82-0.83 ms no-RT; VSM about 0.95).
- **Quality misses carried from N8e:** S1 pose 1 has 2 umbra leaks against VSM's 0. The quarry's lit-darkened margin stays thin (55-60 against 87-88).
- **Profiling access.** Counters were admin-only at first (`RmProfilingAdminOnly` unset, a non-elevated session). After Islam enabled them for all users (value 0), traces still failed until two harness fixes:
  - the GPU Trace layer failed `vkCreateDevice` because the lease's empty implicit-layer path had been replaced by Nsight's whole target directory, which also loads `VKSC_LAYER_NV_GPU_Trace` (the Vulkan SC layer). A directory holding only the four registry-registered Nsight layers fixed it.
  - a 1 s trace allocated a 4,000 MB perf-monitor buffer, which tripped the 2 GiB memory guard. A 200 ms window over 4 submits needed 800 MB.

  No ERR_NVGPUCTRPERM was ever reported. `ngfx --platform "Windows (x86_64)"` crashes (0xC0000409); omit it. No driver setting or counter permission was changed by this unit.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8f/`, 1.6 GB, nearly all of it hardlinked N7a references):
  - `p4-rt`, `p4-mp`: GPU Trace exports (trace info, ranges, metrics, hotspots, series). The `.ngfx-gputrace` files (17 MB and 5 MB) were deleted after export.
  - `p3/waves-pose0.json`: per-wave work.
  - `p2-t-*`, `p4-t-c2`, `final-t-*`: no-RT timing (`timing.json`, `changed-frames.json`, summarized by `scripts/timing.py`).
  - `p3`, `p6`, `p7`, `final-screen-*`: traced screens.
  - `final`: S1 `scores.json`/`comparison.csv`, S2/S3 `comparison.json`, `n8-score.log`, `S1-score.log`.
  - `variants/`: the deferred-lookup diff and shader.
  - `scripts/`: the MCP client and analysis scripts.
  - `layers/`: the Nsight-only layer manifests.
  - `runs/`: every lease's log, preflight and memory samples.

  Raw arrays and dumps (1,539 files) were deleted after scoring; they are listed in `deleted-raw.txt`.
- **Build and environment.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Validation was off for timing and traces and on for S1-S3 scoring. Final and base binaries are in `final-binaries.sha256`; base is N8e's shader and GPU code with this unit's harness. The Cargo target and staged binaries were removed after the gate.
- **Gate.** `cargo test -p renderlab --lib -j 2` exit status 0 on the committed source: 106 passed, 0 failed, 107 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
- **Contention.** `mypaper-wallpaper` and `GrowthOS.exe` ran throughout. Other sessions' builds twice pushed available memory under the 2 GiB guard mid-lease (p3, p3b; rerun). Traced screens read up to 2x high in contended visits, which are excluded; no-RT visits count as clean where primaries read at most 0.45 ms. No game ran (Riot client only), and no window, focus change or capture was used.
- **Leases.** 18 under the shared lock with the 4/2 GiB guard, 1,191 s in total. The failed ones:
  - p1, p1b: the `--platform` crash and the test filter.
  - p1c, p1d, p2's trace, p4, p5: layer path issues.
  - p3, p3b, p4b, p4c: memory guard aborts.
  - p7: a timestamp-write validation error in the deferred pass, fixed and rerun.
- **Time (UTC).**
  - 16:40-16:50: reading.
  - 16:50-17:12: profile attempts (ngfx CLI, MCP client, layer paths), with page-work and caster levers written in between.
  - 17:12-17:23: baseline timing and the bisection screen.
  - 17:23-17:33: counter-enabled traces and the layer fix.
  - 17:33-17:50: trace analysis, the launch-order and cap screens, and deferral built, screened and removed.
  - 17:50-18:00: final batch, scoring and gates.
  - 18:00-18:20: cleanup and this record.

  Wall time is about 100 minutes: GPU leases 20 minutes, waiting on memory or builds about 8, CPU scoring and trace queries about 10, implementation and analysis the rest.
