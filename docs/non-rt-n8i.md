# N8i: the edge certificate and a compacted second kernel, built and rejected

Follow-up unit to [N8h](non-rt-n8h.md), 2026-11-07 UTC, base `15fd73f` merged with main `7fe0bdd` (N8j's history reset), branch `t3/n8i-edge-certificate`. Brief (Islam, 2026-11-07): cut the non-RT virtual-shadow pass on camera-moving frames (no-RT quarry 2.41-2.43 ms against Unreal VSM's estimated ~0.5 ms), starting with N8g's unbuilt light-space edge certificate, then the remaining divergence, for example a compacted second kernel. Labels: **M** measured here, **I** inference.

## Result

**Neither lever is kept. The renderer is main's, byte for byte. Camera motion still misses VSM's estimated cost by about 5x.**

- **The certificate works as a proof, but buys no time.**
  - It certifies 61% of the traced quarry's receivers, and with the ring kept on all eight directions the output stays byte-identical.
  - Camera-moving frames got slower: 2.61-2.64 ms against 2.44-2.47 with it off and 2.39-2.47 on base.
  - Even its upper bound, where certified receivers test only their own triangle and one ray, read 2.63-2.92 ms.
  - The receivers it certifies are lit ones whose work was already cheap.
- **The compacted second kernel made it no better.**
  - Uncertified receivers were deferred into dense waves: camera motion read 2.57-2.63 ms.
  - The trace shows why. The second kernel holds 39% of the receivers, takes 75% of base's lookup time, and runs 5.9 active lanes of 32, against base's 8.8.
  - The divergence is between heavy receivers, not between light and heavy ones.
- **Building the edge marks cost 3.2-3.6 ms on caster-moving frames** (5.3-5.8 against 2.1-2.2 ms).
- **Shipped:** the no-RT quarry harness takes per-variant lookup override constants (`virtual_variants.{name}.constants`), as the traced screen already did. Its output is unchanged (below).
- **Evidence:** the certificate and deferral code is kept as a diff in `E:/renderlab-work/n8i/variants/candidate.diff`.

## Timing (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off, 1920x1080, no-RT quarry at LOD 0.35 px, poses 0 and 1.
- **Candidates:** variants run in one process; N8j's reset makes in-process configurations independent.
- **Base:** main's shader on the same binary (`RENDERLAB_VSM_SHADER`, with empty stubs for the candidate's extra entry points), as a separate process in the same lease.
- **Clean visits:** a visit is clean when primaries read at most 0.45 ms. Ranges are clean visits' medians; contended visits (primaries 0.50-1.2 ms) are excluded.

**No-RT quarry, shadow pass, ms**

| Configuration | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| **Final (main's renderer), final lease** | **0.66-0.70** | **0.67-0.69** | **2.39-2.41** | **2.07-2.17** | **1.39-1.44 / 3.15-3.16 / 2.79-2.91** |
| Base: main's shader, leases p2-p4 | 0.66-0.70 | 0.70 | 2.39-2.47 | 2.09-2.20 | 1.38-1.48 / 3.11-3.20 / 2.82-2.93 |
| Candidate, certificate off (edge pass not dispatched, p4) | 0.68-0.70 | 0.70 | 2.48-2.51 | 2.14-2.21 | — |
| Certificate on, ring on all eight directions, inline (p2) | 0.68-0.69 | 0.70-0.71 | 2.61-2.64 | 5.48-5.56 | 1.39-1.40 / 3.34-3.39 / 6.20-6.31 |
| Certificate on, deferred second kernel (p3) | 0.69-0.71 | — | 2.57-2.63 | 5.31-5.37 | 1.43-1.45 / 3.29-3.39 / 6.04-6.12 |
| Upper bound: certified receivers test only their own triangle, inline (p4) | 0.68-0.71 | — | 2.63-2.92 | 5.50-6.05 | — |
| Upper bound, deferred (p4) | — | — | 2.66-2.69 | 5.30-5.41 | — |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

- **Caster column.** With the certificate on, the caster column includes the edge pass. P2's version reads before its `atomicMax`, skips repeated cells and decides each shared edge once; p1's first version cost about 5 ms.
- **Certificate-off overhead.** With the certificate off, the candidate shader is 0.02-0.10 ms slower than main's on camera frames. That is N8d's codegen hazard: the extra code in the lookup, unused.
- **Traced screens (validation on) were unusable for timing in the screening leases.** On both binaries their static frames read 0.71-2.0 ms and camera frames 7.5-9.3 ms, with two visits of one variant up to 4x apart. Their counts and agreement are used below, not their times. In the final lease the screen was clean: 0.621 static, 0.631 turning, 2.409 ms camera moving, N8h's values.

## What was built (M)

Full code in `variants/candidate.diff`: shaders/virtual_shadow.wgsl, src/virtual_shadow_gpu.rs, the harness.

- **Edge marks.**
  - A pass `vsm_edges` runs after the page raster, over the stale pages' cluster lists: one workgroup per listed cluster, one invocation per triangle.
  - It marks per 2x2-texel cell the highest normalized height of any caster edge that can change how often a ray crosses the casters as its direction moves inside the march's cone. The cone's half-angle is atan(`params.z`), which covers the disk and the sun hysteresis.
  - Marked edges:
    - boundary edges;
    - edges with a face within the cone of edge-on;
    - edges whose two faces' facing classes differ, with the neighbour's normal flipped when the shared edge runs the same way in both triangles (inconsistent winding).
  - The marks are the first mip of a second max-mip pyramid, in the max-mip buffer after the top layer's mips (16 MiB more on the quarry's three levels). They are cleared with their page and cached with it.
  - The hardware cluster list's dispatch arguments go after the raster work buffer's lists, so its layout is unchanged.
- **Certificate check (`vsm_certified`).** It certifies an opaque receiver when:
  - its eye-side plane faces the whole cone;
  - at every step of `vsm_march`'s schedule, the edge mips over the cone's footprint hold no mark above the step's lowest ray height.

  Then no ray of the family passes through a silhouette or boundary edge, so all rays from the origin cross the casters equally often: all directions are blocked or none is. One ray, the disk centre, decides it.
- **Ring handling (`VSM_CERT_RING`).** The no-RT receivers' own LOD ring is not among the certified casters.
  - 1 (default): first tests the ring on all eight directions, and keeps the eight-direction lookup unless the ring blocks none.
  - 0: tests the ring on the centre ray only.
  - 2 (upper bound): tests only the receiver's own triangle.
- **Deferral.**
  - `vsm_lookup` finishes certified receivers. It writes every uncertified pixel to a queue: workgroup-aggregated slots, one global atomic per workgroup.
  - The queue lives in the lookup's read-write scratch binding, the debug trace's, since the lookup stage is at its ten storage buffers. Debug pipelines never defer.
  - `vsm_lookup_deferred` runs the queue 32 to a workgroup on the lookup's grid with a bounds check. An indirect buffer would conflict with the read-write scratch binding.
- **Diagnostics.** `VSM_CERT_SHOW` (screens) writes 257/258 for certified receivers, to count them.

## Profile, camera-moving frames (M)

Nsight Graphics 2026.4.0 GPU Trace, Top-Level Triage, real-time shader profiler; N8g's recipe:
- no-RT quarry pose 0;
- camera wrapping over 48 mm;
- four submits starting at submit 4718, 45 s in, inside camera motion (the loop started at 31.7 s).

Clocks unaltered, no screenshot, no window. Exports are in `E:/renderlab-work/n8i/p5-base-rt/analysis` and `p5-cand-rt/analysis-a` (kernel A) / `analysis-deferred` (kernel B).

| Lookup, per camera-moving frame | Base (main's lookup) | Candidate kernel A: setup and certificate for every receiver, ring and one ray for the certified 61% | Candidate kernel B: the uncertified 39%, dense |
|---|---:|---:|---:|
| Duration | 2.26-2.32 ms | 0.77-0.78 ms | 1.69-1.75 ms |
| SM / L2 throughput | 29.6% / 13.0% | 32.0% / 13.9% | 29.8% / 9.4% |
| Active threads per warp | 8.8 of 32 (27%) | 14.1 (44%) | 5.9 (18%) |
| Resident compute warps per SM | 7.8 | 9.2 | 7.6 |
| Stalls: long scoreboard / wait / selected | 30.9% / 30.3% / 15.6% | — | 27.5% / 31.0% / 16.2% |
| Hottest lines | walk tests `vsm_hits_mask` 4.7%, `vsm_same` 4.7%, fan steps 4.3%, cap check 3.8% | — | walk tests 7.3%, `vsm_difference` 5.0%, walk transforms 4.1%, walk edge select 3.9%, records 3.8% |

**What the profile says (I, from the rows above).**
- The certified receivers' work was not what set the dense lookup's cost.
- The uncertified receivers, a minority, carry most of it, and they diverge among themselves. Their walks differ in length and path, so packing 32 of them to a wave lowers lane activity rather than raising it.
- The pass is bound by per-visit work in the heaviest lane, as N8g found.
- **The lever left for that is refill at a finer grain than a receiver.** Persistent lanes would take a new walk or pixel when theirs ends, as wavefront path tracers do, instead of a wave waiting on its longest walk. That is a restructure of the walk and march into a resumable loop, not built here.

## Quality (M)

**Shipped:** the renderer is main's, so its output is N8h's, unchanged.

- **Fresh outputs (final lease):**
  - traced static and turned hashes `2d6e4fa6ca32…` / `88888216c1d9…`;
  - no-RT counts `05e50a1d8ad17426…` (pose 0) / `bed48bf7099343bb…` (pose 1).

  These equal N8h's, and N8j's in-process ones.
- **Harness check.** A variant that sets a lookup constant to its default value (`VSM_HORIZON: 1`) rebuilds the pipeline and gives the same hashes. Its times equal the default visits'.
- **S1-S3:** not rescored, since no renderer output changed. The quality table is N8h's: above Unreal VSM on every measure except N8h's carried misses (below).

**The rejected certificate's own quality (for the record).**
- **Ring on all eight directions:**
  - Byte-identical on the traced quarry screen, static and turned.
  - Byte-identical in the RT-device quality run, traced and LOD 0.35 px, both poses. Every n8e-score field there equals the certificate-off run: pose 0 no-RT .3529 / 2,434 / 3,288 / 60 / 2,376, traced .2863 / 340 / 1,201 / 0 / 339.
  - The no-RT timing harness's snapshot differs (`8d67f9daa1284a54…` and `ed5077aea661f72e…`). That snapshot holds counts from the sun-turning frames (N8j), so with the sun inside the map hysteresis, some LOD receivers' single-ray answers differ from their eight-direction ones. Not scored, since the lever is rejected.
- **Ring on the centre ray only:** changes the traced turned output slightly. Against Interactive: flips 2,065 against 2,071, lit receivers shadowed 13 against 12, umbra receivers lit 1,512 against 1,505.
- **Certified receivers against Interactive:** they agree in every pixel on the traced screen. None is partial in Interactive, and none has the opposite solid value. 211,306 are lit and 49 blocked.

## Memory against Unreal (M)

- **Final: unchanged from N8h.** No-RT 3,511 MiB (allocator) / 3,662 (WDDM) against Unreal VSM's 3,912; RT 3,175 / 3,241 against 5,410.
- **Rejected certificate:** would have added 16 MiB of edge mips (the screen's stats: 251,573,256 against 234,796,044 bytes) and 8 MiB of queue. That leaves the no-RT total about 3,535 MiB, still under Unreal's.

## Tried and rejected (M)

- **Edge certificate with single-ray decisions** (`VSM_CERT_RING` 0, 1, 2): slower on camera motion in every form (table above), and 3.2-3.6 ms on caster frames.
- **Deferred second kernel for uncertified receivers:** slower (table above). Its waves run 5.9 of 32 lanes.
- **Edge-pass cost cuts that were kept in the candidate:** load-before-atomic, repeat-cell skip, one owner per shared edge. They took the edge pass from about 5.3 to 3.3 ms on caster frames, still 1.5x the whole base pass.
- Not retried (brief): reprojected kept counts, deferred dense waves by work budget, launch order, fewer directions, hard caps, temporal blocker cache, plane-side ring cull, paged record pools, 32-byte delta records.

## Misses, honestly

- **Camera motion:** 2.39-2.41 ms against VSM's estimated ~0.5 ms, about 5x. The no-RT frame with the camera moving is 3.15-3.16 ms against VSM's 1.61. N8g estimated the certificate at 20-35%; it measured −3 to −8%.
- **Static:** 0.66-0.70 ms against VSM's 0.50. Turning is met (0.67-0.69 against about 0.95). The no-RT static frame stays under 1.61 ms.
- **Caster motion:** 2.07-2.17 ms; VSM has no recorded row.
- **Carried from N8e-N8h:**
  - S1 pose 1 has 2 umbra leaks against VSM's 0.
  - The quarry's lit-darkened margin stays thin (55-60 against 87-88).
  - S2's camera phases (0.68 ms) stay above VSM's 0.35.
- Not film quality, and never presented as meeting the film gate.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8i/`):
  - `p1`-`p4*`, `final*`: screens, timing (`timing-summary.json`), hashes. `runs/`: every lease's log, preflight and samples.
  - `p5-*-rt/analysis*`: trace exports, with the trace files deleted after export.
  - `variants/`: `candidate.diff`, `base.wgsl` (main's shader with entry stubs), and the screened shaders `p1`-`p4.wgsl`.
  - `scripts/`: `certdiff.py`, `rows.py`, `timing2.py`, N8g's trace scripts.
  - `counts-hashes.txt`, `final-binaries.sha256`.

  Raw dumps over 5 MB (counts, hits, HDR and world arrays, the two traces) were deleted after hashing and scoring, and are listed in `deleted-raw.txt`. The Cargo target and staged binaries were removed after the gate.
- **Gate.** `cargo test -p renderlab --lib -j 2` exited 0 inside the final lease: 106 passed, 0 failed, 108 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
- **Contention.**
  - Intermittent: in p1 every candidate visit was contended, while p2-p4 were mostly clean.
  - No game ran, and no window, focus change or capture was used.
  - The first S0 lease failed in 6 s: bare `bash` inside the runner is WSL. It was rerun with Git bash.
- **Leases.** Nine, 791 s in total: s0 6, s0b 35, p1 160, p2 146, p3 133, p4 145, p5 106 (the two traces), final 60. All ran under the shared lock with the 4/2 GiB guard, each taken after N8j's release lines.
- **Time (UTC).**
  - 00:10-00:27: reading, the HEAD build and S0.
  - 00:27-00:50: certificate, deferral and the p1-p4 screening leases, built between leases.
  - 00:50-00:56: traces (p5) and their analysis.
  - 00:56-01:05: the final lease, this record and cleanup.

  GPU leases total 13 minutes; implementation and analysis took the rest.
