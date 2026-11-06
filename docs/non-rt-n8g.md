# N8g: the non-RT shadow pass under camera motion, restructured and profiled again

Follow-up unit to [N8f](non-rt-n8f.md), 2026-11-06 UTC, base `79e8ba2`, branch `t3/n8g-shadow-camera-motion`. Brief (Islam, 2026-11-06): make camera-moving frames of the non-RT virtual-shadow pass cheap with a structural change that takes the per-pixel work off dependent geometry fetches. Quarry camera-moving frames cost 2.53-2.58 ms against Unreal VSM's estimated 0.5 ms and a 1.61 ms no-RT frame target. Quality must stay at or above VSM on every measure. Labels: **M** measured here, **I** inference.

## Result

**One structural change was kept: per-triangle walk records. It leaves every output byte-identical. Camera-moving frames fall 12% (no-RT quarry 2.47-2.51 → 2.17-2.21 ms). Static and sun-turning frames fall 18% (0.76-0.81 → 0.63-0.65 ms). Camera motion still misses VSM's estimated 0.5 ms by about 4x, and the no-RT frame with the camera moving is 2.98-3.05 ms against 1.61.**

- **Walk records (direction (a)).** Each addressed triangle gets one 48-byte record: its object-space positions with the edge neighbours inline, built once per draw list. The lookup's ring fans, walks, ring-membership tests and receiver setup then read one record per triangle, instead of index, then vertex, then adjacency. Positions are copied bits, so outputs are byte-identical. Records are camera-, sun- and instance-independent and never rebuilt, so they are cached across all motion.
- **The profile was redone.** N8f's occupancy series came from a trace that started after its camera loop had ended (below). Measured inside camera motion, the dense lookup has no long tail, and the records cut long-scoreboard stalls from 38.6% to 29.0%. The dense lookup still runs about 9 of 32 lanes per instruction, at about 7.3 resident warps per SM and 24-27% SM throughput. **The dense lookup is bound by per-visit work.** That work is the exact 8-direction tests, transforms and walk logic, run in the heaviest lane of each wave. With the records, memory latency is no longer the main bound.
- **Two direction-(b) levers were built and rejected:**
  - an exact temporal blocker cache: slower on camera frames;
  - a plane-side cull of ring triangles: byte-identical, no gain.

**No-RT quarry, shadow pass, ms** (M). Base and final binaries are interleaved in one lease, two visits each, poses 0 and 1, clean visits. A visit is clean when its primaries read at most 0.50 ms; two contended camera-moving visits are excluded.

| Pass | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / turning / camera / caster) |
|---|---:|---:|---:|---:|---|
| N8f (base: HEAD's shader, same binary) | 0.76-0.81 | 0.77-0.80 | 2.47-2.51 | 2.19-2.25 | 1.51-1.57 / 1.51-1.54 / 3.28-3.35 / 3.00-3.04 |
| **N8g** | **0.63-0.65** | **0.63-0.65** | **2.17-2.21** | **2.01-2.16** | **1.38-1.42 / 1.39-1.42 / 2.98-3.05 / 2.80-3.00** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — / — |

**Lookup dispatch, camera-moving frames** (M; Nsight GPU Trace, four consecutive frames inside the motion): 2.30-2.48 → 2.10-2.15 ms.

**Traced quarry screen** (M; pose 0, clean visits of two interleaved pairs). Static falls from 0.71-0.72 to 0.60-0.61 ms and turning from 0.70-0.73 to 0.58-0.61 ms. Camera-moving (16 frames, 1 mm steps) falls from 2.59-2.66 to 2.23-2.35 ms.

The screen's dense-static variant read 2.48-2.58 ms on base against 2.62-2.86 ms final. That is a single 24-frame median per visit, the screen column most exposed to wallpaper contention. Both no-RT camera timing and the trace show a gain.

**S2 and S3** (M, validation on, as scored by N8f): shadow p50 is unchanged within noise.

| Bench / phase | N8f shadow p50 (ms) | N8g shadow p50 (ms) | VSM (ms) |
|---|---:|---:|---:|
| S2 camera, out / hold / return | 0.63 / 0.20 / 0.62 | 0.62 / 0.20 / 0.62 | 0.35 |
| S2 shadow step, out / hold / return | 0.44 / 0.15 / 0.44 | 0.44 / 0.15 / 0.43 | 0.35-0.36 |
| S3, six phases | 0.24-0.25 | 0.24-0.25 | 0.37 static, 0.59 rotating |

## Which N8f measurements were camera motion (M)

N8f traced with `move_frames` 4,000 (the camera stepping 1 mm a frame, then one placement), starting each trace 47 s into the process. Its two traces landed differently (`E:/renderlab-work/n8f/runs/p4d.log`):
- **Hotspot profile (p4-rt): camera motion.** It started during the camera loop, and its frames hold no `vsm_raster` work.
- **Multi-pass trace (p4-mp): not camera motion.** It started after the "camera motion tail" drain, when the camera loop had ended and caster motion followed. It is the source of N8f's resident-warp series ("about 8.4 warps in the first 55%, then a tail of 45% at 2.5 or fewer") and of its second trace's throughput figures.

This unit's first traces (p2) hit the same case. They started at submit 7,112, after the drain, and `vsm_raster` ran in every traced frame at about 1.05 ms per frame. That raster only runs when a caster moves (a still camera inside the level windows finds no stale page). Casters-only frames look up only the pixels a mover reaches, which makes a sparse dispatch with long tails.

Inside camera motion, the dense lookup has no such tail. It holds about 9 warps per SM for 85% of its dispatch (below). This unit adds `move_wrap` to the no-RT harness, which walks the camera back and forth over 48 mm (every frame still moves). A long run then stays near the pose and a late trace still falls inside camera motion. The traces below use it, with no `vsm_raster` work in the traced frames (13-14 µs of empty indirect dispatches).

## Profile, camera-moving frames, before and after (M)

Nsight Graphics 2026.4.0 GPU Trace, Top-Level Triage, real-time shader profiler. No-RT quarry at 0.35 px, pose 0, camera moving, four submits 45 s into the process. The final binary ran both shaders: base was HEAD's shader with only the record-slot mask added (below). Clocks unaltered, no screenshot, no window. Exports are in `E:/renderlab-work/n8g/final-base-rt/analysis` and `final-fin-rt/analysis`.

| Over the longest lookup dispatch | Base (N8f's lookup) | N8g (records) |
|---|---:|---:|
| Lookup per frame | 2.30-2.48 ms | 2.10-2.15 ms |
| SM / L2 / VRAM throughput | 23.6% / 14.4% / 11.2% | 26.9% / 12.1% / 10.1% |
| Active threads per warp | 9.1 of 32 | 9.2 of 32 |
| Resident compute warps per SM (average) | 7.3 | 7.4 |
| Stalls: long scoreboard / wait / selected | 38.6% / 26.9% / 13.3% | 29.0% / 29.8% / 15.0% |
| Short scoreboard / branch resolving / no instruction | 6.0% / 5.5% / 3.2% | 7.2% / 6.8% / 4.7% |
| Geometry fetch samples (triangle, neighbour, first position use) | 24.6% | 19.3% |
| Watertight ray tests (`vsm_hits_sheared`, `vsm_difference`, `vsm_hits_mask`, `vsm_test`) | 19.4% | 22.1% |

A multi-pass trace of the records build (`p3-rec-mp`, 2.05 ms clean dispatch) shows a steady phase, not a tail. It held about 9 warps per SM and 34% SM throughput for the first 85% of the dispatch, then fell to under 1 warp. The register allocation was 168 per thread (12 warps possible). So the steady state is latency- and dependency-bound per warp, with two thirds of issue slots idle and 70% of lanes idle per instruction.

The records removed most of the chain stalls, and the cost moved to the arithmetic that consumes the loads. Every visited ring or walk triangle costs a model transform, eight watertight tests and the walk's point location. These run in the wave's heaviest lane: a ring-walking lit receiver, or a shadowed receiver's walks.

## What was built (M)

`shaders/virtual_shadow_records.wgsl` (new), `shaders/virtual_shadow.wgsl`, `src/virtual_shadow_gpu.rs`, `src/virtual_shadow_tests.rs`, `src/visibility_buffer_tests.rs`.

- **Walk records.** `vsm_record` writes, per addressed triangle (source and appended LOD triangles), three `vec4<u32>`: each corner's position bits, plus the adjacency word of that edge (0xffffffff without adjacency).
  - **Segments.** Records go in segments of 2^21 triangles (96 MiB). Each segment is one binding of a 64-slot binding array in the lookup's group 2. A batch's first slot sits in bits 25-31 of its horizon offset in `VsmFrame::horizon`, and offsets keep bits 0-24. A draw list's horizons must address at most 2^25 triangles, now an explicit error and already the 128 MiB binding limit before.
  - **Fallback.** A batch whose segments do not fit the 64 slots, or a device whose binding limit is below a segment, reads the chains as before.
  - **Build.** Built with the ring horizons, once per draw list.
  - **Lookup.** `vsm_tri` and `vsm_nb` replace every lookup-side `vsm_triangle` and `vsm_neighbour` call. That covers receiver setup, ring fans, ring membership, walks, located-triangle neighbours and per-triangle masks. The horizon pass keeps the chains. `VSM_RECORDS` (screen constant) turns them off.
- **Harness.** `move_wrap` in `gpu_visibility_quarry` (above). `VSM_RECORDS` was added to the screen constants.
- **Base for comparisons.** HEAD's shader reads the horizon offsets unmasked, so it is not a valid base on a binary that packs slots into them. Its ring-horizon skip then reads past its batch, and the traced hash changes. `variants/basefix.wgsl` is HEAD's shader with only that mask added. On the final binary it reproduces N8f's hashes, and every base number above uses it.
- **Programmability.** Records are geometry only (positions and adjacency). No lighting-model data entered hit records, clusters, maps or the lookup, and the pass still outputs the visibility counts that user shading programs consume.
- **Memory.** 48 B per addressed triangle: about 0.62 GB on the traced quarry (13 M triangles) and about 1.25 GB on the no-RT quarry (13 M source plus 13.0 M appended LOD triangles). That is five times the clipmap's 235 MB. It is VRAM, not system memory. **It is the cost of this unit's gain, and the merger should weigh it.**

## Tried and rejected (M)

- **Exact temporal blocker cache** (direction (b); `variants/cache.wgsl`). Per pixel and direction, an R32Uint 8-layer storage texture (66 MB at 1080p) kept the map id of the triangle that last proved the direction blocked. The next lookup tested that triangle, and its edge neighbours if needed, against every open direction first. A hit is an exact proof, so the cache could only find true blocks sooner; a fully proven receiver skipped its ring and march. No-RT camera-moving frames read 2.25-2.28 ms against 2.17-2.21 without it, and static 0.58-0.60 against 0.62-0.67. Shadowed receivers' walks are not the dense lookup's long pole, and the reads and notes cost more than they saved. It also made outputs depend on history. Removed.
- **Plane-side cull of ring triangles** (`variants/plane.wgsl`). Before the eight watertight tests, a ring triangle was skipped when the shared origin lay, by a wide margin, on the side of its plane that every open direction moves away from. The output was byte-identical (traced hashes equal), and the cost was unchanged: camera moving 2.23 against 2.17-2.21 ms, static 0.62-0.65 against 0.62-0.65. Removed.
- Not retried (brief): reprojected counts, deferred heavy receivers, launch order, coarser direction counts, hard caps.

## Prior art and the next lever (I)

No paper was fetched in this unit. The descriptions below are from prior knowledge, in my own words.

- **Irregular z-buffer** (Johnson et al. 2005) and **frustum-traced IZB** (Wyman et al. 2015-16; NVIDIA's HFTS blends it with PCSS) are exact. They insert this frame's receivers into a light-space grid of lists, then rasterize every caster triangle conservatively over the lists and test each receiver it covers.
- **Sintorn et al.'s per-triangle shadow volumes** test each caster triangle's volume against a hierarchy of view-sample clusters.
- **Alias-free / conservative-raster triangle-ID maps** store the triangles covering each texel.
- **Why they don't fit camera motion here.** IZB and per-triangle volumes rebuild per frame and scale with caster triangles times covered receivers. The receivers change every frame under camera motion, and the quarry has 13 M caster triangles. FTIZB's published cost is 20-32 ms on 12.3 M triangles (GTX 980). Triangle-ID lists would replace walks with list reads, but the profile above shows the cost is the exact tests per visited triangle, not the walk chains.
- **The next lever: an edge certificate.** It is the form of (b) that fits the profile:
  - rasterize, per stale page, the light-space extent of every boundary edge and every silhouette edge (an edge whose two faces differ in facing for some direction of the disk), with its height, into an edge max-mip;
  - over a region free of such edges, the top surface is a continuous graph of faces front-facing for every disk direction;
  - so a ray from a receiver on it can never meet it again, and a ray starting below it must cross it;
  - a receiver whose cone footprint, up to the clear height, meets no edge above it therefore gets the same answer on all eight directions;
  - one exact ray then decides it, which cuts the per-visit test work eightfold for those receivers; penumbra receivers keep the full test.

  Its gain is bounded by the waves that still hold a penumbra lane (a third of waves hold a shadowed lane, N8d). That is about 20-35% at the quarry's lane coherence (I). Not built here.

## Quality (M)

Every retained change is byte-identical to N8f's output on every frame each bench scores. Base (N8f's lookup) and final ran on the same binary and lease.

- **Traced quarry:** static and turned hashes `2d6e4fa6ca32…` and `88888216c1d9…` (N8f's and N8e's), and the dense turned hash `222153a86f18…`. Agreement against Interactive is unchanged: 2,157 flips of at least ½, 1 lit receiver shadowed, 1,438 umbra receivers lit.
- **No-RT quarry at 0.35 px:** counts SHA-256 `05e50a1d8ad17426…` (pose 0) and `bed48bf7099343bb…` (pose 1), equal for base and final in all four visits.
- **Caster-moving frames:** the N8f identity gate (rolled counts against a dense lookup of the final placement) reports 0 differing pixels at both poses in every visit.
- **S1, S2, S3:** rescored on the final binary with N8f's scorers. Every non-timing field equals N8f's: 792 S1 score fields, and 72 each in S2's and S3's `comparison.json`. Those include partial RMS, HDR RMS, lost contacts, leaks and flips.

So the full quality table is N8f's, against Unreal VSM:

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

- **Camera motion.** No-RT quarry 2.17-2.21 ms against VSM's estimated 0.5 ms, and the no-RT frame 2.98-3.05 ms against 1.61. The brief's time-box triggered: the records prototype gave 12%, under its 30% bar. Both direction-(b) levers built after it gave nothing. The structural change that could reach VSM's cost is per-receiver decisions without per-visit 8-direction tests. The edge certificate above is that change; it was not built (I).
- **Static** 0.63-0.65 ms against VSM's 0.50. **Turning** is met (0.63-0.65 ms; VSM about 0.95). The no-RT static frame is now 1.38-1.42 ms, under the 1.61 target in every clean visit (N8f: 1.55-1.64).
- **Caster motion on the quarry:** 2.01-2.16 ms, with a 1 M-triangle chunk as the caster; VSM has no recorded row. **S2's motion phases** are unchanged: 0.62 ms camera, 0.43-0.44 ms caster, against VSM's 0.35-0.36.
- **Memory:** about 1.25 GB of records on the no-RT quarry.
- **Quality misses carried from N8e and N8f:** S1 pose 1 has 2 umbra leaks against VSM's 0, and the quarry's lit-darkened margin stays thin (55-60 against 87-88).

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8g/`):
  - `final-screen-*`: traced screens with hashes.
  - `final-t-*`: no-RT timing, `timing.json` and `changed-frames.json`, summarized by `scripts/timing.py`.
  - `final-base-rt`, `final-fin-rt`: before/after GPU Trace exports, with `scripts/analyze.py`, `hot.py`, `byfn.py` and `stalls.py`.
  - `p3-rec-mp`: multi-pass metrics and `series-short.json`, the occupancy series.
  - `final`: S1 `scores.json`/`comparison.csv`, S2/S3 `comparison.json`, `n8-score.log`, `S1-score.log`, fixture.
  - `p4-t-*`, `p5-*`: the cache and plane-cull screens.
  - `variants/`: base, basefix, cache, plane and nocache shaders.
  - `edits/`: the scripted edits.
  - `runs/`: every lease's log, preflight and memory samples.

  Raw dumps over 5 MB were deleted after summarizing; they are listed in `deleted-raw.txt`. GPU Trace files were deleted after export.
- **Invalid early baselines.** In leases p2b-p5 the "base" ran HEAD's unmasked shader on binaries that pack record slots into the horizon offsets. Those base timings and hashes are invalid (its ring-horizon skip read past its batch) and are not used above. The final lease's `basefix` base reproduces N8f's hashes. p2's first records run fell back to chains, because one 4 M-triangle batch exceeded a single binding; that motivated the segments. As an unpatched control it read 2.52-2.54 ms on camera frames, matching N8f.
- **Build and environment.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan; validation off for timing and traces, on for S1-S3. The final binary and shader hashes are in `final-binaries.sha256`. The Cargo target and staged binaries were removed after the gate.
- **Gate.** `cargo test -p renderlab --lib -j 2` exits 0 on the committed source: 106 passed, 0 failed, 107 ignored. It also exited 0 inside the final lease, before formatting. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
- **Contention.** `mypaper-wallpaper` ran intermittently. Contended visits (primaries 0.70-0.86 ms) read up to 2x and are excluded. One p3 trace range held a 0.5 ms idle gap, the GPU time-sliced to another process; it is not used. No game ran (Riot client only), and no window, focus change or capture was used.
- **Leases.** 7 under the shared lock with the 4/2 GiB guard, 1,463 s in total: s1, p2, p2b, p3, p4, p5 and final. Minimum available memory at start was 8.4 GiB. p2's base and trace runs failed at pipeline creation: the records fallback pushed an override the base shader lacked. The fallback was redesigned before p2b.
- **Time (UTC).**
  - 18:00-18:25: reading.
  - 18:25-18:31: records implementation and build.
  - 18:31-18:52: screens, the segment fix, traces and the profile correction (GPU 12 minutes).
  - 18:52-19:01: blocker cache.
  - 19:01-19:13: cache and plane-cull screens (GPU 6 minutes).
  - 19:13-19:22: final batch (GPU 6 minutes).
  - 19:22-19:40: scoring, gates, analysis and this record.

  Wall time is about 100 minutes: GPU leases 24, CPU scoring about 6, implementation and analysis the rest.
