# P-NRT3b: retain the two-submit split, reject batched gather

**DON'T MERGE. Identity passes; the early-still RT median service gate is not cleared.** The branch retains the two-submit candidate for follow-up and removes batched gather. Moving/recovered service and isolated idle gaps improve on both paths, but the eight-pair RT early-still median is not better. No optimization is promoted to main, and no quality setting is reduced.

## Source

Created `t3/p-nrt3b` from `t3/p-nrt3` (`9e4d474`), merged current main `9fd0a9efe5229b393d92bef2a578fc65d38b4e85` without conflicts (`9a4c3e2`). The WGSL blocker was fixed by renaming the gather scalar to `gi_drain_base`, leaving the setup array unchanged. CPU Naga validation then passed: 19 validation tests and all 7 GI tests, including hardware/software kernels at 64/128 directions and GI shading/program composition. No other duplicate declarations were reported.

After q1, `shaders/gi_cache.wgsl` was restored byte-for-byte from frozen main. Thus the final intended source retains only P-NRT3's renderer split and the existing plan environment support. The scalar and batched algorithm are absent. Main `9d38de7` was also merged cleanly (`a4ef92a`); its additions are docs and unrelated measurement tools, not renderer/build inputs. The freshly built `9fd0a9e` baseline remains runtime-equivalent.

The first submit starts skinning, clusters, TLAS/shadows, atmosphere, primary/visibility and sun visibility. GI/post preparation and remaining encoding follow, with the second submit completing the frame. Logical frame/retirement accounting stays on the late encoder. `CpuFrame.submit` includes both calls; encode excludes their time. One queue preserves execution order. A first-half pass must not read a resource written by the delayed GI/post/transparency preparation.

## Identity

Original fixed two-change candidate: ARA1 164/164 PNGs, Q-walk 186/186 frame hashes, RF1-RF10 26/26 float buffers and MP1 14/14 PNGs identical on both paths. Lib: 200 passed, zero failed, 164 ignored GPU tests. q1 screening s0/s1 each passes 12/12 timing PNGs per path; s2 passes 88/88 quality/timing PNGs per path.

Split-only candidate: **PASS**. ARA1 quality 152/152 PNGs, interleaved timing 24/24 PNGs, extra cold-RT timing 24/24 PNGs, Q-walk 186/186 hashes, RF1-RF10 26/26 float buffers, MP1 14/14 PNGs: every compared output is identical (426 comparisons). Lib: 200 passed, zero failed, 164 ignored. CPU module validation: 19 passed. Main's valid fixture baselines were reused. `kept-1` hit its bounded deadline during MP1 RT; `finish-1` completed MP1 RT/lib, all eight cold pairs and target cleanup in 235 s, zero failures.

## Fresh split-only service

Two interleaved runs per binary/path/case; CPU service medians, ms. This is the final `kept.exe`, not the combined screening variant.

| Scene / phase | Non-RT main | Non-RT split | RT main | RT split |
|---|---:|---:|---:|---:|
| Game still | 8.44 | 8.19 | 3.30 | 3.04 |
| Game moving | 11.46 | 11.27 | 5.13 | 4.90 |
| Game cut | 33.32 | 33.26 | 20.49 | 20.11 |
| Game recovered | 10.16 | 9.90 | 4.90 | 4.63 |
| Quarry early still | 11.88 | 11.51 | 8.24 | 8.80 |
| Quarry moving | 11.87 | 11.45 | 3.92 | 3.63 |
| Quarry cut | 25.34 | 24.45 | 15.06 | 14.77 |
| Quarry recovered | 13.54 | 12.85 | 4.57 | 4.18 |

The RT quarry early-still miss triggered eight additional interleaved pairs, retaining the registered phase and median statistic:

- Pooled median: **7.376650 to 7.422600 ms** (not improved).
- Paired run-median wins: **4/8**; median paired change **+0.029850 ms**.
- Phase mean: **7.828029 to 7.525877 ms**, a lower total phase cost. Six of eight paired means improve.
- Run medians span 6.3395-8.3149 ms on main and 6.2762-9.1621 ms on split. This is an inconclusive median win, not proof of a large systematic regression.

The mean and moving/cut/recovered results do not substitute for the planned median gate. The conservative verdict is DON'T MERGE. Next work should explain early RT phase/clock/pipeline-preparation behavior before changing scheduling; no cause is established here. Exact per-run values are in `final-summary.json`.

## Quiet screening: split isolation

Nsight game-still traces, 1920x1080. s0 and s2 hold the same batched gather constant, isolating the scheduling change. Values are medians over complete frame intervals: seven for one-submit s0 and three for two-submit s2 (eight submissions capture eight/four frames). Small sample, not an exact-repeat certification.

| ms | Non-RT s0 | Non-RT split s2 | RT s0 | RT split s2 |
|---|---:|---:|---:|---:|
| Inter-frame CPU/GPU idle gap | 1.852 | 1.253 | 0.868 | 0.447 |
| GPU frame span | 6.172 | 6.234 | 2.588 | 2.612 |
| Internal GPU gaps | 0.270 | 0.295 | 0.148 | 0.169 |

The main-vs-s2 gap figures are 1.895 to 1.253 ms non-RT and 0.860 to 0.447 ms RT. The split adds internal gaps and CPU work, while reducing the inter-frame gap. Do not report the entire inter-frame saving as shader acceleration. The fresh split-only service figures above confirm the steady/moving gains while recording the RT early-still miss. These idle-gap figures isolate the split with gather held fixed; they are screening-binary traces, not a new trace of `kept.exe`.

## Rejected gather

Gather-only service and idle-gap screen (two interleaved runs, ms):

| Measure | Non-RT main | Non-RT gather | RT main | RT gather |
|---|---:|---:|---:|---:|
| Game moving service | 11.40 | 11.18 | 5.14 | 5.12 |
| Quarry moving service | 11.85 | 11.34 | 3.89 | 3.97 |
| Game inter-frame gap | 1.895 | 1.852 | 0.860 | 0.868 |

The gather has no established RT service/idle win in this cohort; small deltas are not a broad regression certificate.

Matched quarry windows after 110/150 submissions, one-submit baseline and gather-only s0, seven complete intervals per trace. No split confound.

| Gather kernel, ms | Main | Batched | Result |
|---|---:|---:|---|
| Non-RT window 110 | 5.085024 | 4.657152 | lower |
| Non-RT window 150 | 0.433312 | 0.413920 | lower |
| RT window 110 | 0.037888 | 0.039488 | no RT win |
| RT window 150 | 0.036064 | 0.038048 | no RT win |

Total GI cache work is 6.787 to 6.294 ms and 1.720 to 1.648 ms non-RT; RT 0.695 to 0.698 ms and 0.689 to 0.678 ms. RT has largely drained by these late windows, so this is not proof of a large RT cold-fill regression or improvement. Its early quarry still service is also too variable to certify the algorithm: q1 per-run medians range 6.66-9.86 ms with ordering reversals, and the earlier l2 pair reversed the apparent direction. The conservative three-hour decision is to remove the gather, preserving the split that wins on both paths.

## Measurement contract and latency

RTX 4070 SUPER (driver 617.14), Ryzen 7 7800X3D, Windows/Vulkan/wgpu 30, native 1920x1080, release builds. ARA1 game-base/quarry and registered quality/fixture scenes; Realtime, GI/reflections/TR1/post enabled, 128-shadow budget preserved, no quality/ray/budget change. CPU service is synchronous render plus explicit GPU drain, excluding image encoding/readback. The adapter's inherited static scope string says GPU span excludes uploads/readbacks. Delayed queue writes can now place late uploads inside the marker span, so it is not a shader-only comparison. Nsight instrumentation is separate from ARA1 service timing.

CPU submission work increases: the initial screen's still submit median is about +0.03-0.04 ms, with additional encoding/preparation overhead. A CPU-bound game can pay this cost. `render()` returns after the second submit; the late encoder owns completion. Code audit preserves the same frame/present order, but these tests are offscreen: no measured surface present-order, scanout or input-to-photon latency claim follows. The remaining inter-frame gaps are not solved.

Quiet q1 leases: `q1-nonrt-1`, `q1-next-1`, `q1-rest-1`: 176/120/96 runtime samples, zero compiler samples throughout. Final retained leases `kept-1`/`finish-1` recorded 192/55 samples, zero compiler samples. All 1,061 samples across this unit were compiler-free; correctness captures stayed above the runtime floor. No broad Ara/Unreal performance superiority follows; M8's overall Ara target remains open.

## Reproduction, time and housekeeping

Evidence/plans/tools/new binaries: `D:/Coding/renderlab-work/p-nrt3b/`. `trace-summary.json` and each trace's `attribution.json` retain the inspected pass timings; summarized raw GPU traces and RF float dumps over 5 MB are deleted. `raw-hashes.json` preserves the individual SHA256s of all 78 RF buffers (about 1.94 GB removed) after both candidate comparisons completed. Non-RT initial q1 uses inherited `runs/l1`; RT uses `runs/q1-rt`. Retained service uses `runs/kept-timing`. No private assets/images are committed.

One target `D:/Coding/pnrt3b-target`, serial release builds at `-j 2`. Builds hold atomic `build-slot.lock` with owner JSON and the normal stillkit mutex acquire. Each build/lease mutex is released immediately afterward by its owning process. Admission requires no cargo/rustc, >=8 GiB available and >=5 GB D free. Every GPU/test-binary execution uses `scripts/n8-run.py` through the P-NRT wrapper, <=15 minutes, 2 GiB runtime floor. Latest root orders are reread; alternating gates match anchored owner releases. Schedule reading is streamed. PID-tree cleanup replaces the inherited trace runner's image-name kill in the local evidence runner.

Implementation was a conflict-free merge, a three-use shader rename, then removal of the rejected gather; renderer edit time was small compared with verification. Initial compiler times: candidate capture 152 s, lib 87 s; main capture 40 s, lib 89 s; screening capture 40 s. Wall build times include substantial mutex waiting and are not compiler latency. Bounded leases resumed rather than restarting valid captures. Retained compiler times: capture 49 s, lib 110 s (wall times 769/113 s include mutex waiting). Total compiler time is about 9.5 min; owned validation leases total 75.3 min, including asset loading, pipeline compilation, deadline retries and cleanup. Wall task time is about 2 h 50 min including queue/coordination. Implementation edit time was under ten minutes (estimate), separately from those verification costs. `D:/Coding/pnrt3b-target` was deleted after the final captures; no owned GPU/build process remains.

| Initial binary | SHA256 prefix |
|---|---|
| base.exe (current main runtime) | cdbc726546b7b43a |
| base-lib.exe | a62dd4edf0629d0e |
| final.exe (rejected combined candidate) | 3d5136609bbffcfe |
| final-lib.exe | a386a0a6ca017908 |
| cand1.exe (screening modes 0/1/2) | f07d83ee0066ab09 |


| Retained binary | Full SHA256 |
|---|---|
| kept.exe | 2da34626d72938be5f6214bd590905ecde5a387a89d2aff9e05b4219b3b0e38c |
| kept-lib.exe | 19c46857bdb902d102e613e34c9a7a899f0de8a42c5f21fe05973c7cd6aea6b9 |

`final-summary.json` is the canonical retained identity/quiet/cold-phase record. `runs/kept-timing/{nonrt,rt}/summary.json` contains every phase's service and GPU medians; `trace-summary.json` contains all 14 trace summaries. Old `final.exe` is the rejected combined candidate, not the retained binary. No force-push, foreground window, desktop capture or viewer launch was used.
