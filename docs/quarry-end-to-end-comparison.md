# Quarry: matched end-to-end frame-loop comparison

2026-11-05 local / 2026-11-04 UTC; benchmark branch based on `7de1ede522e359e87dd14e6e67aeff7222522c6c`. **Measured offscreen throughput exceeds 2x both Unreal modes in every retained block. VSM misses 2x GPU-only in the static and moving-camera workloads. HRT clears 2x GPU-only by a small margin. Combined fidelity/2x acceptance remains unestablished.** These are measurements of the direct-light/material/environment/shadow subset on this quarry, under recorded desktop/CPU contention; they are not an engine-wide or display-presentation claim.

## Ratios and verdicts

Every ratio is **Unreal time / Renderlab time**, so larger favors Renderlab. Each cell is the median of three same-block ratios **[minimum, maximum]**. Ranges are observed spread, not confidence intervals. Ratios pair process measurements within a block; individual frames are not simultaneous pairs. End-to-end throughput uses measured block means, including Renderlab's final completion drain, rather than `max(CPU, GPU)` or a sum of passes.

| Workload / Unreal mode | GPU-only | CPU critical-field proxy | Frame-to-frame median | End-to-end throughput |
|---|---:|---:|---:|---:|
| Static / VSM | 1.866 [1.817, 2.078] | 3.587 [3.207, 3.624] | 3.604 [3.205, 3.612] | **3.514 [3.165, 3.532]** |
| Static / HRT | 2.040 [2.029, 2.108] | 3.307 [2.725, 4.807] | 3.270 [2.777, 4.833] | **3.141 [2.778, 5.083]** |
| Camera / VSM | 1.855 [1.836, 1.901] | 3.061 [3.028, 3.975] | 3.077 [3.031, 4.013] | **2.996 [2.980, 4.313]** |
| Camera / HRT | 2.060 [2.057, 2.076] | 2.771 [2.438, 2.968] | 2.797 [2.493, 2.959] | **2.734 [2.498, 2.867]** |
| Sun / VSM | 2.382 [2.287, 2.465] | 3.160 [3.023, 3.292] | 3.207 [3.077, 3.308] | **3.051 [2.639, 3.101]** |
| Sun / HRT | 2.008 [2.005, 2.030] | 3.337 [2.700, 3.403] | 3.312 [2.701, 3.313] | **2.925 [2.633, 3.112]** |

- **VSM:** observed 2x frame-throughput floor passes all three workloads, including every block; GPU-only fails static/camera and passes moving sun. It does not pass all GPU workloads.
- **HRT, 1-spp denoised:** observed 2x frame-throughput and GPU-median floors pass all workloads/blocks. The moving-sun GPU margin is only 0.27% in its lowest block. This is not a robust hardware-independent 2x guarantee, or a per-frame/tail guarantee.
- **CPU:** the reported critical-field proxy exceeds 2x in every block for both modes. A dependency-aware, pure CPU-work 2x verdict remains **unestablished**: native critical fields include dependent waits, and Renderlab's admission wall time includes GPU backpressure. Neither is a CPU instruction/cycle measurement. The proxy's construction is explicit below.
- **Combined quality and speed:** **not accepted** for either mode. Radiance parity, receiver/representation differences and moving-sun image-quality acceptance remain open. See the subset limits below.

## Measured times

Milliseconds, median of three process values **[minimum, maximum]**. GPU, CPU proxy and frame-median columns summarize each process's sample median. Completed-frame columns summarize each process's measured mean. A ratio of these table medians need not equal the median of paired block ratios above.

| Workload / renderer | GPU | CPU critical-field proxy | Frame median | Completed-frame mean |
|---|---:|---:|---:|---:|
| Static / Renderlab | .8599 [.8302, .8622] | .8822 [.8545, .8859] | .8826 [.8547, .8863] | .9063 [.8902, .9307] |
| Static / VSM | 1.6086 [1.5621, 1.7248] | 3.0650 [2.8407, 3.1974] | 3.0804 [2.8404, 3.1875] | 3.1277 [2.9454, 3.2007] |
| Static / HRT | 1.7497 [1.7490, 1.7546] | 2.8255 [2.4137, 4.2407] | 2.7950 [2.4608, 4.2654] | 2.7959 [2.5851, 4.6068] |
| Camera / Renderlab | .8624 [.8611, .8643] | .8931 [.8914, .8957] | .8934 [.8917, .8960] | .9362 [.9185, .9390] |
| Camera / VSM | 1.6035 [1.5835, 1.6371] | 2.7417 [2.7039, 3.5429] | 2.7572 [2.7076, 3.5784] | 2.8047 [2.7987, 3.9615] |
| Camera / HRT | 1.7782 [1.7767, 1.7879] | 2.4702 [2.1832, 2.6509] | 2.4938 [2.2339, 2.6433] | 2.5111 [2.3460, 2.6840] |
| Sun / Renderlab | .8656 [.8632, .8668] | .8891 [.8828, .8950] | .8894 [.8831, .8955] | .9721 [.9351, 1.0561] |
| Sun / VSM | 2.0649 [1.9744, 2.1338] | 2.8096 [2.6683, 2.9461] | 2.8526 [2.7173, 2.9624] | 2.8535 [2.7869, 3.0148] |
| Sun / HRT | 1.7405 [1.7312, 1.7574] | 2.9460 [2.4007, 3.0460] | 2.9260 [2.4022, 2.9656] | 3.0255 [2.4626, 3.0891] |

All frame samples are retained. To expose tails rather than only process-median spread, these are pooled **GPU p95 / frame-interval p95**, ms:

| Workload | Renderlab | VSM | HRT |
|---|---:|---:|---:|
| Static | 1.2203 / 1.2377 | 2.1121 / 3.8096 | 2.1411 / 5.4651 |
| Camera | 1.2382 / 1.2750 | 2.2297 / 4.4158 | 2.5753 / 2.9174 |
| Sun | 1.3090 / 2.0537 | 2.5538 / 3.4523 | 2.2974 / 3.6262 |

Frame-interval maxima are 3.6775 / 3.7736 / 9.1995 ms for Renderlab static/camera/sun, 4.5697 / 12.7233 / 3.7921 ms for VSM, and 10.7875 / 5.5524 / 7.3310 ms for HRT. No outlier, slow block or CPU-build sample was discarded. These numbers do not establish a 2x tail-latency guarantee.

## Workload, builds and sequencing

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D (8 cores / 16 logical processors), Windows. Renderlab uses Rust 1.96.0, wgpu 30.0.1/Vulkan, optimized Cargo release, validation off for performance. Unreal is the unchanged packaged **Development** UE **5.7.4 CL51494982**, D3D12 SM6; no editor, engine rebuild, asset import or recook was used.

Both use the registered twelve scans/placements, 1920x1080 output and internal resolution, 1x, 16x anisotropy, the registered sun/disk, environment and manual exposure -14. Renderlab uses **public `Renderer::try_render_view`, `PrimaryVisibility::Traced`, `SoftShadows::Interactive`**, with all 13,034,903 source triangles resident. There are no replayed receivers, shader replacements, diagnostic kernels or hard/Exact frames in its timed loop. Every frame asserts the real traced/Interactive mode and twelve draws/source triangle count. All nine HRT native memory dumps confirm **13,034,903** noncompacted native Nanite RT triangles; VSM keeps its native view-selected Nanite representation.

- **Static:** independently loaded poses 0, 1 and 0 in blocks 0/1/2. The third process reloads pose 0; it is not a claim about temporal return history.
- **Camera:** exact registered 0/1 poses alternate every engine/frame-loop tick, a 1 cm source-X translation with unchanged orientation and no cuts. All 64 measured Unreal rows per process verify alternating native Y=520/521 cm, X=-620, Z=340.
- **Sun:** static pose 0; native sun yaw alternates -53.130102/-52.130102 degrees each tick, pitch -57.994617, roll 0. Renderlab rotates its source travel by -1 degree about source Y for the second state. Native setter calls update the actual transform; runtime component readbacks at CSV frames 92/93 verify both rotations.

Each workload has **three blocks**, ordered **Renderlab -> VSM -> HRT**, **HRT -> VSM -> Renderlab**, then **Renderlab -> VSM -> HRT**. Processes run sequentially. The final five sun processes were resumed later in the same session after RAM deferrals; their individual pressure status is below. This is 27 retained processes: nine Renderlab, nine VSM, nine HRT.

Renderlab warms 512 frames and measures 2,048 per process: **6,144 GPU/CPU frames per workload**, 18,432 total. The first interval after the warmup drain has no previous measured frame and is excluded only from frame-interval statistics, leaving **6,141 intervals per workload**. All CPU/GPU frames remain in their other statistics and in whole-block throughput. Unreal captures 160 native CSV frames/process, with the fixed **96-159 block, n=64**: **192 frames per mode/workload**, 1,152 total. ProfileGPU at 64 and the native geometry dump at 90 are outside that block. There is no screenshot, DumpGPU or profiler UI in the performance schedule. Native GPU trees contain 96-109 parsed rows per process; their inclusive/async nodes are not summed into a frame time.

Build/evidence identities:

- Renderlab measured executable `build-b3/e2e-tests.exe`: SHA256 **`0f0f4425cd1ad6532e700f7e5b3b61df0b8876319f589025ae67ece152a655cf`**. `build-b3/source-hashes.json`, `source.diff`, `build.json` and the base commit bind the measured source. The final Rust-only adjustment after measurement is a Clippy allowance on the ignored benchmark's release guard; renderer, shader and measured-loop behavior are unchanged.
- Unreal executable: **`69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5`**. Existing cooked content identities remain in `D:/Coding/renderlab-work/ue-m5/Quarry/full-detail-identities.json` (main container `9b37d5cf78567a813216980270d6d94ea540b4f35d4716d37624546b9b153008`). Session identities hash the actual executable again.
- `identity.json` and `resume-*.json` record source RLA, limits, cameras, lighting, environment and script hashes. The scene remains the full RLA `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`; no private content enters Git.

The old stored 1.597/2.150 ms Unreal anchors are **not** used in these ratios. The fresh HRT medians are lower. No isolated cause is established; this run uses the packaged motion path and omits the old pre-measurement screenshot, and clocks/contention/history differ.

## CPU, GPU and preparation boundaries

Renderlab's CPU frame wall includes lighting changes, draw construction, validation/preparation, visibility/bounds/packing, shadow planning, resource/queue writes, encoding, submission, every busy admission attempt, polling and yield gaps. `collection_ms` separately measures event delivery; the CPU proxy adds it to that serial wall time. Frame-to-frame timestamps also include the harness bookkeeping between iterations. The renderer's **three in-flight slots** supply bounded backpressure. The only full GPU drains are first-image verification, the warmup boundary and the measured block's final completion boundary; there is no per-frame drain. GPU samples are associated by frame ID, and no measured event or sample is lost.

Renderlab GPU time is its measured first-rendered-pass to final-tone-pass span: fused camera/8-ray visibility, material/environment shading and tone mapping. Upload commands, timestamp resolve/readback and display acquire/present are outside that span. They remain in the actual frame-loop throughput. Unreal GPU time is its native **GPUTime**, with native game/render/RHI fields preserved independently. Query boundaries/backends differ, so the GPU ratios are explicitly GPU-span/frame-field comparisons, not certified equal whole-device busy intervals.

Unreal CPU proxy is the per-frame **maximum of `GameThreadTime_CriticalPath`, `RenderThreadTime_CriticalPath`, `RHIThreadTime`**, then the process median. Installed-engine inspection confirms the native critical fields retain dependent waits while subtracting noncritical idle. Renderlab is a single caller thread with GPU admission waits. These observations are measured; interpreting their maximum as a complete cross-thread dependency path is a **proxy**, not a measured trace. Do not sum these fields, add GPU time, or describe them as pure CPU work.

Pooled native thread medians, **GT-critical / RT-critical / RHI**, ms:

| Workload | VSM | HRT |
|---|---:|---:|
| Static | .5238 / 3.0277 / 1.7371 | .5134 / 2.9136 / 1.9077 |
| Camera | 1.1901 / 2.8937 / 1.6889 | .9493 / 2.4676 / 1.6927 |
| Sun | 1.0683 / 2.8111 / 1.7112 | 1.0703 / 2.8253 / 1.8819 |

**The render thread supplies the longest native field here; the game-thread column alone does not certify a CPU advantage.**

Renderlab pooled per-frame median details, ms; columns are independently measured and must not be summed as a frame percentile:

| Workload | Calls excluding yield gaps | Sun setter | Draw preparation | Shadow preparation | Upload packing/writes | Encode | Submit | Event delivery |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Static | .6058 | .0000 | .0002 | .0067 | .0073 | .0667 | .0321 | .0010 |
| Camera | .6278 | .0000 | .0002 | .0070 | .0100 | .0679 | .0327 | .0010 |
| Sun | .6055 | .0010 | .0002 | .0089 | .0115 | .0816 | .0408 | .0012 |

The calls column includes repeated nonblocking admission/poll calls; it is not CPU cycles or thread utilization. Static timed frames upload no camera, light or geometry bytes. Each camera run uploads **360,448 camera bytes** across 2,048 frames (176/frame); each sun run uploads **24,576 light bytes** (12/frame); all nine runs upload **zero geometry bytes** during measurement. Required camera/light work is not prepared out of the loop.

Resident preparation is reported separately, not amortized away or called frame time. Renderlab load/decode/upload/AS preparation is **2.063-18.158 s**, and startup to first GPU-completed image is **2.396-18.776 s** across the nine processes. The slow startup/load processes are retained. Native total process wall is retained in every `process.json`, including map loading, warmup, profiling and teardown; it is **not** a measured first-image latency and is not used as one. Both reuse prepared content. Offline cooker/import/cook time was not retimed; prior Unreal 252 s full-residency preparation and 128 s first SM6 rebuild are historical evidence in [the full-detail report](unreal-quarry-full-detail-comparison.md). No startup, cook, streaming, deforming-geometry or arbitrary-world 2x claim is established.

## Contention, memory exception and GPU windows

The first 22 successful launches obeyed the original >=8 GiB `FreePhysicalMemory` guard; the lowest accepted primary/native preflight was **8.362 GiB**. RAM later stayed below that guard under desktop use. No launch bypassed a failed preflight. Deferrals and the one nested Unreal preflight rejection are retained, and completed runs were preserved across resumes.

Islam explicitly authorized **only the last five moving-sun runs** to launch at **>=6 GiB** using **`\Memory\Available MBytes`**, with an abort below **3 GiB**. The runner reads that exact English PDH counter, retains quarter-second samples, and can terminate only its owned GPU child. All five are labeled **measured under memory pressure**, including the final HRT launch whose available memory had recovered above 8 GiB. They were sequential; Renderlab and Unreal never ran together.

| Run directory / mode / block | Available at primary preflight, GiB | Sampled minimum while monitored, GiB | Counter samples |
|---|---:|---:|---:|
| 22 / VSM / 1 | 6.676 | 3.794 | 64 |
| 23 / Renderlab / 1 | 6.968 | **3.104** | 28 |
| 24 / Renderlab / 2 | 6.820 | 3.616 | 21 |
| 25 / VSM / 2 | 7.476 | 4.561 | 56 |
| 26 / HRT / 2 | 8.253 | 5.385 | 46 |

No sampled value crossed 3 GiB and no memory abort occurred. The earlier HRT sun/block-1 process ran under the original guard; that block's comparison spans the documented RAM pause. The sun spread therefore includes both memory conditions and an interruption. No quiet-memory result is inferred from it.

Across the retained session, **126 full contention samples** found zero games or foreign Renderlab/Unreal renderer conflicts. **55 samples** contained build processes, at most **10 cargo/rustc/Clippy/link processes**. These were mainly other workers; the task's own Clippy check ran during a RAM-only pause. Desktop GPU activity, CPU scheduling and changing GPU clocks remained. These are **contended desktop measurements**, with asymmetric CPU outliers; the end-to-end advantage cannot all be attributed to GPU rendering efficiency.

Unreal's foreground handle/PID matched at 11/18 pre/post endpoints; the others changed between desktop applications. **Zero endpoints belonged to the owned Unreal PID.** Endpoints cannot prove uninterrupted focus stability. All launches request hidden startup and offscreen rendering; no focus/input automation, desktop/game capture or foreign-process shutdown was performed. Renderlab creates no window.

GPU leases use atomic `mkdir` plus `owner.json`; all are released. Retained-session windows (UTC):

| Acquired | Released | Duration |
|---|---|---:|
| 22:32:29.703308 | 22:38:10.453936 | 340.751 s |
| 22:39:06.894610 | 22:41:19.409627 | 132.515 s, RAM-only deferrals |
| 22:43:36.158543 | 22:45:49.213754 | 133.055 s, RAM-only deferrals |
| 22:47:32.057339 | 22:48:37.288499 | 65.231 s |

Including builds, probes and deferrals, **12 windows totaled 928.967 s**, longest **340.751 s**, well below 45 minutes. The final shared-slot release is 22:48:37.286784 UTC (the runner ledger follows by about 2 ms). Every acquisition/release is in the private lease histories and Unreal slot history; the owned lock was removed on release.

## Quality evidence and remaining subset limits

The camera and sun preflight suites ran optimized Renderlab with active Khronos validation: **errors=0, missing_layer=0, active_layer=1**. All six measured static/camera processes also compare the **complete HDR image byte for byte** with the accepted c11 Interactive checkpoint, after timing. Sun outputs are hashed; there is no frozen moving-sun quality reference or acceptance score. Prior Level-B static/1 cm motion evidence carries over for the unchanged renderer, not for an unmeasured quality domain.

Remaining limits:

1. **No local bounce GI, reflections/SSR/Lumen, atmosphere/fog, transparency/volumes, TSR or temporal AA/reconstruction** is being compared. Output is native 1080p/no AA. The environment's prefiltered specular illumination is not dynamic local reflections or GI.
2. **Radiance parity remains open:** UE's native SH/specular sky convolution versus Renderlab RLE2 filtering, native deferred BRDF/composition, generated texture mips/sampling, display/tonemap curves and sun-color quantization. Intended environment source/orientation/intensity/exposure match; the actual rendered radiance does not yet have an equality certificate. See [the full-detail comparison](unreal-quarry-full-detail-comparison.md).
3. **Receiver and representation differences remain:** native Nanite compression/LOD, source-only silhouette receivers, UE's actual 1 cm near plane versus the registered Renderlab .5 cm plane, geometric versus native shading-normal offset/culling, biases, sample placement, filtering and HRT histories. VSM's view-selected geometry is not all source triangles. See [native-mask evidence](unreal-native-shadow-mask.md).
4. Interactive is an eight-ray, history-free estimator; HRT uses one ray with native denoising/history, VSM uses native SMRT/cache behavior. Their shadow/temporal quality is not equivalent merely because one image gate passes. Moving-sun timing is now measured, but moving-sun full-image quality remains unaccepted.
5. **Offscreen frame throughput, not scanout:** neither side exercises swapchain acquisition/presentation, desktop composition or display pacing. Cross-backend, release-Rust versus packaged-Development-Unreal, native profiler/harness overhead, CPU-build contention and the five pressure runs limit generalization. Unreal motion commands use console macro file reads/native setters each tick; that harness cost is included in its CPU/frame fields and was not independently subtracted or isolated.

The frame times, counters, upload totals, readbacks, hashes and ratios above are **measured/derived from measured samples**. The longest-field CPU construction is an explicitly labeled **proxy**. No inferred `max(CPU,GPU)` frame time, additive pass total, pure CPU-work speedup, radiance-parity claim or quality-superiority claim is substituted for those measurements.

## Evidence, checks and reproduction

Private root **`D:/Coding/renderlab-work/quarry-e2e/`**:

- `matched-s3/summary.json`: per-process statistics and all 18 same-block ratios; `analysis.json`: pooled tails, grouped ranges, thread details, pressure samples and lease totals.
- `matched-s3/schedule.json`, `identity.json`, `resume-*.json`, `finished.json`, `lease-history.json`: order, immutable inputs, changes of launcher revision and windows.
- `matched-s3/00` through `26`: Rust `frames.json`, manifests/logs or `ue-summary.json`/`ue-evidence.json`/`motion-check.json`, run records, preflights and contention. `deferred-*` preserves failed attempts. Quarter-second memory samples for directories 22-26 are in `contention.json`.
- Native raw CSV, `ue.log`/ProfileGPU tree, native memory dumps and launch/process/foreground records are linked by each `ue-evidence.json` under **`D:/Coding/renderlab-work/ue-m5/Quarry/launches/e2e-matched-s3-*`**.
- `build-b1` through `build-b3` retain build logs/identities. `probe-p1` exposed startup-event bookkeeping, `probe-p3` exposed Unreal absolute-macro-path handling; neither is performance evidence. `probe-p4` verifies native camera/sun movement and active validation. Earlier lease/RAM deferrals are retained.

Checks: six focused benchmark CPU tests and eleven existing quarry CPU tests pass, including available-memory policy/owned-child abort, CSV growing-header alignment, motion scheduling and both native GPU-tree layouts. Affected release `cargo clippy -p renderlab --lib --tests -j 2 -- -D warnings`, `cargo fmt --check` and `git diff --check` pass. No renderer/shader implementation was changed. The benchmark uses the actual library inside a release test executable.

Implementation and verification overlapped; coding time was not separately stopwatched. Two successful incremental optimized builds took **28.079 + 28.764 = 56.842 s**. The initial cold build failed on a private-helper access and held its lease for 90.439 s; its compiler-only duration was not preserved. Retained process execution totals about **226.115 s** (Rust wrapper duration plus native Unreal process duration), separate from lease/RAM waits; these process totals are not frame times. All timing run counts and CPU/GPU samples are above; no fixed large repeat campaign or optimization hillclimb was added.

```powershell
# CPU-only analysis of retained evidence:
py -3 -B scripts/quarry-e2e.py summarize matched-s3
py -3 -B scripts/quarry-e2e-report.py D:/Coding/renderlab-work/quarry-e2e/matched-s3
py -3 -B scripts/quarry-e2e-test.py
py -3 -B scripts/ue-quarry-test.py

# New independent measurement: guarded own target, -j 2, no push/build/cook of UE.
py -3 -B scripts/quarry-e2e.py build new-build-label
py -3 -B scripts/quarry-e2e.py session new-session-label --binary D:/Coding/renderlab-work/quarry-e2e/new-build-label/e2e-tests.exe --repeats 3

# Only for the explicitly authorized final five sun runs of the same schedule:
# ... session <label> --binary <same-binary> --repeats 3 --resume --memory-pressure
```

Scripts/docs are committed on this test branch. No push was performed.
