# M5-DL: development latency and optimized runtime

2026-11-07 UTC. Branch `t3/m5-dev-loop`, measured main **`0981fda8278580912aba8ddd64767b7208810558`**, including N8p. **Surface is the non-RT default measured here.** No rendering algorithm or quality setting was changed. The retained code changes are opt-in measurement instrumentation and an explicit M1 harness lookup option.

The requested development-loop and release-runtime observations are now recorded. Fresh standard/user program edits still take about **1.1 s**, missing the historical 100 ms target. Quarry process startup takes about **8.4 s**. No-RT camera/caster work remains expensive. These measurements do **not** close combined Unreal fidelity/2x acceptance; numerical, scope and unresolved timing limits are below.

## Host, profiles and workload

**RT scope:** every RT runtime timing, speed ratio and memory row in this record is
**measured on the non-default fast setup (traced + Interactive)**. The library and
viewer's RT-device default remains raster + 256-ray Exact; [RT1](rt1-interactive.md)
keeps the fast setup opt-in while its film false-shadow gate fails.

- Ryzen 7 7800X3D, 8 cores/16 logical processors; RTX 4070 SUPER, 12,282 MiB; NVIDIA 617.14; Windows 11 Pro build 26300; Vulkan/wgpu 30.0.1. Rust 1.96.0 `ac68faa20`/LLVM 22.1.2, x86_64-pc-windows-msvc. Two Cargo jobs and an isolated target on E:.
- **Development:** repository dev profile: viewer opt-0, renderlab opt-1, asset/wgpu crates opt-2, incremental, line-table debug information. The native viewer uses FIFO presentation. Its actual logs report active Khronos validation and zero errors, even without `--validate`: wgpu debug-build defaults enable it. This is the practical debug viewer, not a release throughput measurement.
- **Optimized:** `cargo test -p renderlab --lib --release --no-run`, then the actual library inside the M1 test executable. Validation disabled explicitly, no swapchain/window, 1920x1080, 1x/native resolution, 16x anisotropy, full texture residency, original 4K color mip chains, RLE2 environment, GGX/Lambert and exposed HDR/Reinhard. No GI, dynamic local reflections or temporal reconstruction.
- Quarry: twelve scans/draw placements, **13,034,903 source triangles**. No-RT uses VisibilityBuffer primaries with the existing 0.35 px LOD, software extent 32, receiver lift 1, and Virtual **Surface** shadows, on a device requesting no experimental ray features or acceleration-structure limits. RT uses full-detail **Traced + hardware Interactive** (eight-ray) shadows; its visibility/shadows are fused. These are the existing supported path settings, not identical triangle representations.
- Window development tests use the existing full-resident raster viewer, with no shadow requests (`views=0` in logs), quarry at 1920x1080/16x and the two-cube/triangle fixture at 960x640/1x. They do not measure Surface or traced shader reloads: the current viewer does not expose those paths. The release runtime tests do.
- Source RLA: `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3` (1,548,588,300 bytes); environment: `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764`. All 190 Rust/WGSL/manifest source identities, private asset/lighting/camera hashes, source diffs and commands are retained in the evidence. No private assets enter Git.

## Development loop

Every cell is **median [minimum, maximum]**, five repeats. Shader units are milliseconds; fresh edits use unique visible tint literals, avoiding previously compiled source-cache hits. Both standard and custom program files go through the viewer’s existing `ShaderWatch` and `--program` slot. The custom program has its own shade body; a separately registered nonstandard material handle is not exposed by this viewer and was not separately timed.

The endpoint is the first frame of the accepted revision **observed GPU-complete**, with its present call recorded independently. A subsequent renderer-owned HDR readback proves that the image changed. Readback/PNG encoding is outside the timed endpoint. These are completion/presentation-call proxies, **not measured display scanout**.

| Program/action | Save → completed image, ms | Save → present call, ms | Reload module/pipeline creation, ms | Save → detection, ms |
|---|---:|---:|---:|---:|
| Standard / edit | 1083.233 [970.028, 1130.180] | 1075.959 [965.176, 1116.105] | 1038.546 [928.289, 1080.869] | 34.305 [32.250, 36.455] |
| Standard / restore | 107.408 [98.381, 108.763] | 98.328 [93.211, 101.251] | 72.422 [56.608, 75.142] | 25.669 [20.077, 35.349] |
| Standard / recover | 90.329 [87.853, 102.508] | 83.834 [80.000, 91.959] | 54.212 [48.576, 60.630] | 26.934 [21.534, 32.252] |
| User / edit | 1094.718 [1063.901, 1254.439] | 1085.845 [1054.394, 1237.082] | 1051.737 [1015.879, 1193.053] | 37.120 [30.877, 40.866] |
| User / restore | 119.858 [95.736, 134.328] | 108.612 [88.466, 120.780] | 74.717 [61.088, 76.481] | 30.374 [24.802, 41.738] |
| User / recover | 114.735 [84.113, 121.100] | 98.124 [81.109, 107.499] | 61.630 [50.554, 72.121] | 33.394 [26.559, 35.061] |

| Invalid program | Save → rejection observed externally, ms | Parser rejection, ms | Last working image |
|---|---:|---:|---|
| Standard | 34.314 [24.361, 39.439] | 0.032 [0.016, 0.070] | Byte-identical, all 5 repeats; subsequent frames still presented |
| User | 40.852 [25.696, 44.065] | 0.021 [0.014, 0.023] | Byte-identical, all 5 repeats; subsequent frames still presented |

All ten fresh edits change the complete HDR hash. All ten reversions, ten invalid saves and ten corrections preserve/restore their own baseline hash exactly. Whole images were inspected for a standard tint, recovered custom shading, the fixture/library background edit and viewer camera edit. Shader compile time combines module validation and hot pipeline creation; driver-only compile and individual pipeline creation are not separately instrumented. Cached reverts must not be reported as fresh shader-edit performance.

### Incremental Rust edit to an actual running window

A minimal **library** edit changes `src/lighting_gpu.rs`’s lit clear/background value; a **viewer-only** edit changes the fixture’s initial camera yaw. Both visibly change the rendered fixture and require a fresh viewer process. Source bytes are restored exactly. These measure a small implementation edit, not an API-wide rebuild, large refactor or hot patch.

| Edit | Build including link, s | Sampled linker lifetime, s | Restart → completion observed, s | Edit → changed viewer including harness, s |
|---|---:|---:|---:|---:|---:|
| Library | 5.781 [5.692, 8.066] | 0.431 [0.428, 0.622] | 1.029 [1.006, 1.033] | 7.173 [7.065, 9.434] |
| Viewer | 1.311 [1.267, 1.330] | 0.418 [0.389, 0.431] | 1.026 [0.992, 1.045] | 2.687 [2.659, 2.698] |

The linker column is the **sampled lower-bound lifetime** of the owned `link.exe`, with a nominal 10 ms Toolhelp polling interval. It overlaps the inclusive build column. Exact linker start/exit events and isolated codegen/link CPU time were not collected; do not subtract these medians to invent compilation time. Cargo unit-timing HTML and each process sample are retained.

The measurement harness spends 0.345–0.362 s after a timed build hashing/logging/copying evidence before launch. It is included in the edit-to-viewer column. The derived sum of each measured build and its actual viewer restart, excluding only that harness handoff, is:
- Library: **6.814 [6.720, 9.072] s** (derived, not a separate wall-clock run).
- Viewer: **2.330 [2.308, 2.343] s** (derived, not a separate wall-clock run).

Rust measurements restart the small fixture. They do not establish edit-to-running-quarry latency; the separately measured quarry startup below must not be silently replaced by the fixture’s restart.

### Cold-process startup and its components

Each repeat creates and closes a real background native window. “Cold” means a **new process and scene load**; OS file caches and NVIDIA shader caches are retained. There is no cold-disk or purged-driver-cache measurement. External startup begins immediately before `Popen`; main-entry timing excludes loader/CRT startup. Its observer has a 5 ms poll interval plus scheduling delay.

| Startup component, ms | Small fixture | Quarry |
|---|---:|---:|
| Process launch → observed first completed image | 723.960 [653.229, 2157.530] | 8395.073 [8004.900, 8820.612] |
| Main entry → first completed image | 643.388 [575.380, 1723.716] | 8312.257 [7928.763, 8731.215] |
| Main entry → first present call | 642.934 [574.910, 1723.354] | 8256.972 [7907.269, 8710.689] |
| CPU asset/camera load, validation and preparation | 0.004 [0.004, 0.005] | 1370.950 [1318.124, 1444.275] |
| Asset file I/O (subset of load) | — | 583.179 [571.404, 625.099] |
| Asset decoding (subset of load) | — | 465.868 [428.001, 487.205] |
| Window/adapter/device/surface setup | 326.395 [296.230, 417.348] | 339.643 [330.923, 378.521] |
| Renderer construction / initial pipelines | 74.349 [62.622, 189.791] | 71.230 [67.566, 82.761] |
| Asset upload wall, including preparation/admission | — | 5461.400 [5119.261, 5779.141] |
| First accepted render call, including lazy pipelines | 28.285 [25.529, 848.113] | 34.047 [32.207, 36.885] |

These are independent/partly nested boundaries and their medians must not be added. Environment loading/upload, wake/presentation gaps and final GPU-completion observation remain in the total. Renderer setup excludes lazy program pipelines; the first render includes those and ordinary preparation/encoding. **A fully isolated first-frame pipeline-creation split remains unavailable.** The fixture’s first-ever sample has an 848 ms render call; later repeats use driver-cached shaders (25–30 ms). Both are retained.

The earlier [independent integration](m5-independent-integration.md) recorded one 1.985 s shader edit, 0.110 s restore, 6.311 s Rust-label build and 13.377 s including restart under contention. Those are historical observations with different source/workload boundaries, not paired baselines for a speedup claim.

## Release runtime: complete rendered-pass span and per-pass cost

RT runtime figures are **measured on the non-default fast setup (traced + Interactive)**.

`scripts/memory-m1.py --visits 8 --lookup surface --cpu-timing`, one asset load per device/path, two registered poses. Each pose/visit has 32 warm frames, 8 measured static frames, 16 turning-sun frames (0.5 mrad about Y each), 48 moving-camera frames (1 mm along source X each), and 48 moving-caster frames (placement 0, 1 mm each). Static/turn/camera/caster contribute **128/256/768/768 GPU samples per path**. Warm samples are excluded. GPU results join to CPU records by actual frame ID.

“Clean visit” is a **descriptive contamination screen**: median primary <=0.45 ms no-RT (the prior N8 rule), <=0.60 ms RT, and shade <=0.50 ms. The RT threshold was selected during analysis, not preregistered. A qualifying primary does not prove an uncontended shadow pass. No selected shadow/span outlier is manually removed. The unfiltered RT range is reported immediately below; all samples and rejected labels remain available.

Per-pass values below are medians across qualifying visit medians, milliseconds. Span includes gaps from the first rendered pass through tone mapping; it is not the sum of these independently summarized pass medians.

| Path/phase | Clean visits / 16 | GPU render span [visit range], ms | Primary (RT includes shadows), ms | Separate shadow, ms | Shade, ms | Tone, ms |
|---|---:|---:|---:|---:|---:|---:|
| No-RT / Static | 16 | 0.734 [0.725, 0.807] | 0.367 | 0.000 | 0.335 | 0.013 |
| No-RT / Sun | 16 | 1.327 [1.293, 1.393] | 0.367 | 0.595 | 0.326 | 0.012 |
| No-RT / Camera | 16 | 1.792 [1.771, 1.839] | 0.397 | 1.049 | 0.314 | 0.013 |
| No-RT / Caster | 16 | 2.368 [2.353, 2.415] | 0.395 | 1.591 | 0.349 | 0.013 |
| RT / Static | 12 | 0.867 [0.855, 0.889] | 0.487 | Fused | 0.352 | 0.013 |
| RT / Sun | 12 | 0.867 [0.854, 0.991] | 0.488 | Fused | 0.351 | 0.013 |
| RT / Camera | 12 | 0.868 [0.856, 0.880] | 0.484 | Fused | 0.350 | 0.013 |
| RT / Caster | 11 | 0.865 [0.857, 0.881] | 0.483 | Fused | 0.351 | 0.013 |

All 16 no-RT visits qualify in each phase. RT has slow visits despite the exclusive slot; their cause is unproven. It would be wrong to present only the filtered ranges as guaranteed runtime.

| RT, all 16 visits | Unfiltered GPU span median [visit range], ms |
|---|---:|
| Static | 0.872 [0.855, 2.836] |
| Sun | 0.874 [0.854, 2.835] |
| Camera | 0.871 [0.856, 1.627] |
| Caster | 0.869 [0.857, 1.370] |

The earlier `runtime/` batch also remains as evidence (RT static visit 0 reached 8.554 ms). `runtime-b/` is the retained batch: it moves the opt-in CPU timing endpoint after event delivery. It changes no renderer/shader behavior. These slow observations are not explained away as compiler contention.

### CPU caller critical path

This is the actual single caller thread’s wall interval from before camera/sun/caster updates and draw construction through admission/submission **and event collection**, including busy retries and yield gaps. It is a **critical-path proxy containing GPU backpressure**, not CPU instruction time. Its own JSON-record serialization is outside the boundary. Renderer counters describe the successful call; they exclude earlier busy attempts and are not an additive decomposition of caller wall. Phase-tail drains/readbacks and capture/harness work remain separate.

| Path/phase | Caller wall [visit range], ms | Prep | Shadow prep | Upload | Encode | Submit | Poll |
|---|---:|---:|---:|---:|---:|---:|---:|
| No-RT / Static | 0.617 [0.520, 0.712] | 0.0009 | 0.0054 | 0.0067 | 0.2323 | 0.0381 | 0.0132 |
| No-RT / Sun | 1.326 [1.249, 1.383] | 0.0008 | 0.0055 | 0.0089 | 0.2493 | 0.0392 | 0.0150 |
| No-RT / Camera | 1.822 [1.803, 1.934] | 0.0018 | 0.0057 | 0.0112 | 0.3593 | 0.0446 | 0.0186 |
| No-RT / Caster | 2.391 [2.375, 2.447] | 0.0012 | 0.0057 | 0.0112 | 0.3591 | 0.0445 | 0.0190 |
| RT / Static | 0.827 [0.764, 0.861] | 0.0009 | 0.0075 | 0.0080 | 0.0909 | 0.0342 | 0.0105 |
| RT / Sun | 0.885 [0.739, 0.898] | 0.0008 | 0.0072 | 0.0083 | 0.0844 | 0.0323 | 0.0091 |
| RT / Camera | 0.895 [0.879, 0.928] | 0.0018 | 0.0071 | 0.0099 | 0.0862 | 0.0324 | 0.0101 |
| RT / Caster | 0.928 [0.916, 0.945] | 0.0013 | 0.0272 | 0.0105 | 0.1269 | 0.0345 | 0.0188 |

GPU render spans exclude timestamp resolve/readback, presentation and any preparation/uploads outside the timestamped interval. The test’s per-phase wall file includes tail drains (and camera/caster verification readback), so it is **not** an uncontaminated complete-frame throughput benchmark. No new end-to-end throughput or dependency-traced pure CPU speedup is claimed.

### Memory, M1 allocation/lifetime inventory

MiB = 2^20 bytes. All sixteen warmed allocator reports agree exactly per path. WDDM Dedicated Usage is a sampled per-process local-memory observation, including driver costs; allocator reservation is a different quantity. The requested M1 harness retains every allocation/block/usage/lifetime CSV, post-upload report, warmed report and WDDM sample.

| Path | Allocated MiB | Allocator reserved MiB | WDDM warm/process sampled peak MiB | WDDM samples | Recorded Unreal DXGI MiB |
|---|---:|---:|---:|---:|---:|
| No-RT Surface | 3424.6 | 3503.7 | 3583.9 | 15 | VSM no-RT: 3,912 |
| Traced Interactive RT | 2509.6 | 2641.2 | 2716.3 | 9 | HRT: 5,410 |

Unreal totals are recorded UE 5.7.4 CL51494982 values from [M1](memory-m1.md)/[N8h](non-rt-n8h.md), not new runs. Compare WDDM with DXGI rather than treating allocator slack as a saving against engine process memory. Actual sample cadence is slower than the requested 250 ms because of PowerShell/PDH overhead: roughly 1.5 s; peaks are not instantaneous peak certificates.

## Recorded Unreal runtime comparison; no Unreal rerun

Reused native GPUTime and longest native critical-field proxy from the [matched quarry report](quarry-end-to-end-comparison.md): same GPU/driver, native 1080p/no AA, direct-light/material/environment subset, packaged Unreal Development/D3D12 versus Rust release/Vulkan. Values are medians of three process medians; they are not paired with this run.

| Phase | UE VSM GPU, ms | UE HRT GPU, ms | UE VSM CPU proxy, ms | UE HRT CPU proxy, ms |
|---|---:|---:|---:|---:|
| Static | 1.6086 | 1.7497 | 3.0650 | 2.8255 |
| Camera | 1.6035 | 1.7782 | 2.7417 | 2.4702 |
| Sun | 2.0649 | 1.7405 | 2.8096 | 2.9460 |
| Caster | Not recorded | Not recorded | Not recorded | Not recorded |

The Unreal camera workload alternates the registered 1 cm poses every frame; its sun alternates by 1 degree. This M1 motion sequence advances camera/caster by 1 mm and sun by 0.5 mrad each frame. Dynamic workloads are therefore **not exactly matched**; no numerical dynamic speedup certificate follows. The CPU proxies have different engine/thread/wait boundaries. Radiance, geometry/receiver representation, shadow estimator/filtering and quality parity also retain the cited report’s limits.

As a descriptive ratio of GPU medians only, static VSM / no-RT is about **2.19x**, whereas camera VSM / no-RT is about **0.89x** and sun about **1.56x** (motion mismatch applies). Static HRT / clean RT is about **2.02x**, but only **1.97x** against the slowest qualifying static visit, and much less against excluded slow visits; these RT ratios are **measured on the non-default fast setup (traced + Interactive)**. The full-quality/robust 2x floor is **not demonstrated**. Caster comparison is missing. Surface’s carried quality evidence is in [N8p](non-rt-n8p.md); film/reference and complete Unreal acceptance are not newly certified here.

## Unreal development-loop equivalents: cited, not measured

Epic UE 5.7 documentation, consulted 2026-11-07:

| Renderlab measurement | Unreal equivalent and evidence | Numeric comparison |
|---|---|---|
| Standard/custom shader save → image | `recompileshaders changed` / Ctrl+Shift+.; a single material Apply recompiles that material. Epic describes asynchronous workers and retry-on-error with `r.ShaderDevelopmentMode=1`. [Shader Development](https://dev.epicgames.com/documentation/en-us/unreal-engine/shader-development-in-unreal-engine?application_version=5.7) (**citation**) | No equivalent measured/citable fixed duration; no ratio |
| Rust build + viewer restart | C++ Live Coding recompiles and patches a running editor/PIE or attached desktop application; module preloading trades startup for faster iteration. It avoids the restart measured here. [Live Coding](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-live-coding-to-recompile-unreal-engine-applications-at-runtime?application_version=5.7) (**citation**) | Epic provides no fixed milliseconds for a comparable edit; no ratio |
| Cold-process viewer startup | Editor/project startup with prepared content and caches. The repository records a 128 s first SM6 asset-load/rebuild and 252 s full-residency preparation in the [full-detail comparison](unreal-quarry-full-detail-comparison.md) (**historical measurement**) | Neither span is editor-start → first-image latency; no startup-speedup claim |

## Coordination, rejected attempt and remaining misses

Both leases used atomic `D:/Coding/renderlab-work/gpu-slot.lock` plus reservation/release lines in `E:/renderlab-work/gpu-schedule.md`. No game, foreign GPU renderer or Unreal process was detected in any of 153 full contention samples. Session A observed two foreign-build samples, outside the retained runtime batch; session B observed zero. Own compiler processes are excluded by descendant ownership and occur only in reserved timed builds. Desktop/driver activity persists (preflight GPU utilization 0% / 13%); “exclusive slot” does not mean an idle OS or fixed clocks.

| Lease UTC | Seconds | Available RAM at launch, MiB | Minimum 10 Hz available RAM, MiB | Result |
|---|---:|---:|---:|---|
| 2026-11-07T11:07:55.477833+00:00 → 2026-11-07T11:14:10.409646+00:00 | 374.931 | 10073 | 6205.5 | Rejected shader-camera check; valid startup/runtime evidence kept |
| 2026-11-07T11:15:38.309643+00:00 → 2026-11-07T11:20:24.046966+00:00 | 285.737 | 10534 | 7345.2 | Completed all retained shader/Rust/runtime measurements |

All sampled availability stays above 4 GiB, including all launches; the 2 GiB abort floor was never crossed. No cargo ran inside the leases except explicitly timed preparation/incremental/restore builds. Every viewer used `--background`/`with_active(false)`, with no activation/input or desktop/game capture. Only owned windows received WM_CLOSE; none remains. Foreground endpoint records cannot certify uninterrupted OS focus stability, but no code requests focus.

The first session’s shader hash check rejected a restore because the supplied camera sequence advanced between poses. That was a harness mismatch, not an accepted shader result. The follow-up freezes registered pose 0, repeats all forty shader actions, and extends the opt-in CPU interval through event collection. This required a second lease rather than silently accepting the first batch. Both attempts, their source diffs and slow observations are retained.

Acceptance limits/misses:

1. Fresh shader edits miss 100 ms; library edits still cost seconds and quarry startup remains about 8.4 s. No project-wide “practical” threshold was invented or waived.
2. Cold-disk/empty-driver-cache startup, isolated driver/lazy-pipeline creation and exact linker event durations were not measured. The boundaries actually observed are tabulated.
3. Release GPU-pass spans and caller critical-path proxies cover all four requested phases, but this M1 run is not a clean end-to-end frame-throughput/presentation or dependency-traced CPU measurement. Slow RT visits remain unexplained.
4. Moving Unreal workloads differ, caster baselines are absent, and neither a full-fidelity quality comparison nor a robust combined 2x floor is established. No quality reduction was made to pass a timing gate.
5. The dev-loop window is the current raster viewer, not a new Surface/RT viewer. Custom-program timing uses its existing standard program slot.

## Build identities, validation and retained evidence

| Artifact | SHA-256 |
|---|---|
| Dev viewer for startup/shader runs | `05480969e8c102d518e18026bfdc7c7eaa2fcc35f646a7a3c05b09f46c97369d` |
| Retained release test executable | `e07f37ee0ebef50faff52611f7409951344c89d1fcd0d30e9826c563aee3ad72` |
| Earlier release probe (`runtime/`) | `b638af1cbacd4a30943ffb41b8a647add17171e80f06dfcef47b31646edd2096` |
| Reused packaged Unreal executable | `69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5` |

Per-incremental-build viewer hashes and visible source edit recipes are in `results.json`/`batch.py`; compiler flags/profiles, Cargo unit timings, process samples and complete command lines are preserved. Temporary library and viewer fixture hashes match their original 190-file freeze exactly after restoration. The final measured-source freeze records the CPU instrumentation endpoint correction.

Checks: both release M1 device/path runs pass, each with eight independent replays per pose; ten fresh edits and all thirty reversion/rejection/recovery image checks pass. Native viewer logs report zero validation errors. Final optimized CPU suite: **114 passed, 112 GPU tests ignored**, 0 failed, test execution 0.71 s. Formatting, Python AST parsing and whitespace checks pass. No new rendering-quality campaign or Unreal run was performed.

Evidence root: **`E:/renderlab-work/m5-dev-loop/`**, outside Git. `identity.json`, final source hashes/diffs, `results.json`, `summary.json`, `batch.py`, `summarize.py`, Cargo timing HTML/logs, `runtime-b/{nort,rt}/{timing,cpu,load,manifest,run}.json`, allocation/lifetime CSVs, WDDM JSONL and both lease/resource records supply the detailed tables. `runtime/` and `attempt-a-results.json` preserve the earlier attempt. Only inspected PNGs and small logs/JSON/CSV/HTML remain; raw HDR/PFM dumps over 5 MB and this worker’s Cargo target were removed after summarization.

Time accounting: initial isolated dev/release preparation **99.687 + 145.661 s**; corrected instrumented release preparation **55.989 s**. All fifteen timed builds including measured Rust edits and restores total **348.111 s**. Retained optimized device processes take **20.096 + 12.159 = 32.255 s**, excluding build time. Total reserved build/GPU/window verification time across both leases is **660.668 s**; this also includes rejected evidence, loading, image encoding and coordination. Implementation/research time was not independently stopwatched. Final CPU verification additionally rebuilt the test executable after the temporary Rust edits; its Cargo duration and test time are in `final-cpu-tests.log` (the overall command wall was not separately timed).

`docs/milestones-5-6-decisions.tsv` was not edited by this unit. The branch is delivered for integration; main is not pushed.
