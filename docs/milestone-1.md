# Milestone 1 evidence record

Status: Milestone 1 accepted after independent Opus review and root verification of its required corrections. No further rendering milestone is authorized automatically.

Checkpoint CPU verification passed after root's instrumentation correction and comment cleanup: formatting, workspace/all-target checks, Clippy with warnings denied, and the CPU boundary test. The three GPU tests were skipped. Logs and source hashes are saved in `measurements/m1-implementation/check-20261102-210132/`. All Cargo work used two jobs. No window or GPU test was launched for this checkpoint.

## Implemented foundation

The library renders indexed meshes with a validated perspective camera, per-object matrices, and an internal Depth32Float attachment. The viewer uploads one cube mesh and one triangle mesh, reuses the cube twice, and changes matrices for movement. The library has no winit dependency.

Three submission records bound admitted uploads and draws. Mesh upload immediately submits its staging work, including when no render follows. Ordinary rendering polls without waiting for completion. Optional GPU timestamps use those same records for asynchronous readback. Shader reload installs only a valid candidate. Resource errors become persistent renderer faults. Device or surface loss currently requires restarting the viewer.

Counters distinguish application-owned handles, backend-retained resources, allocator bytes, uploads, pipeline creation, and submissions. CPU preparation, upload, encoding, submission and polling have separate fields. The viewer also records configure, acquire and present-call wall times. GPU samples cover the mesh pass, excluding earlier upload work and presentation. These are separate costs, not additive measurements of one frame's elapsed time.

## Recorded verification before the gaming hold

The implementation agent recorded three passing GPU tests in `measurements/m1-implementation/check-20261102-204415/`. The source manifest there identifies the tested working tree above base commit `130429f8fb4f2ae8d2fb8ec51af83306edd52af6`. These results precede subsequent viewer instrumentation corrections and are not final-checkpoint certification.

- Full-image equality across reversed submission order establishes depth/occlusion for overlapping triangles. The front pixel is red. Syntax, binding and vertex-input shader errors preserve the complete image; a corrected tint changes it and restoring the shader restores it exactly.
- A 641-pixel-wide capture exercises padded readback rows. Saved captures cover the indexed cube, translated model, camera orbit, portrait target and reload correction.
- 300 static frames produce no new geometry, camera or model uploads. 200 model-only frames and 200 camera-only frames check the expected matrix uploads without recreating persistent resources or pipelines.
- Tests exercise unavailable timestamps, asynchronous sample reuse, failed-map handling, Busy admission, and persistent rejection after a foreign-device resource fault. The foreign-device test deliberately causes a validation fault; it is separate from clean ordinary rendering.
- Replacement stress admits 2,176 frames without per-frame waits, surface configuration or captures. Each frame replaces mesh and color target; the depth target follows its extent. Pipelines are replaced every 32 frames. Another 256 mesh replacements run without rendering to check immediate upload submission.

Root has inspected the saved indexed-cube and depth captures. That inspection supports those static fixtures only. It does not certify live motion, window resizing, minimization or input responsiveness.

## Retention evidence and upstream limitation

The stress scene uses 48 draws of 2,048 overlapping triangles, at alternating 512/513 by 384 targets. After 64 warmup frames, all three 704-frame waves observed incomplete prior draws and retained resources in every iteration. Peak admitted flights were three. Each wave reached the same measured high-water values: 17 HAL buffers, eight texture views, two pipelines, 665,104 buffer bytes, 3,194,880 texture bytes, 3,861,760 allocated bytes and 268,435,456 reserved bytes. Draining returned the asserted resource counts to the initial baseline. The upload-only phase also returned buffer counts and allocated bytes to baseline.

The Vulkan implementation in wgpu-hal 30.0.1 increments `texture_memory` during normal `create_texture`, but does not increment `textures`; `destroy_texture` decrements both. `texture_from_raw` also does not increment the object counter. Only `add_raw_texture` increments it. Root confirmed these paths in the installed dependency source. The raw texture-object counter is therefore preserved but marked unreliable, and is excluded from acceptance assertions. Buffer counts, texture-view counts, texture bytes, and allocator observations provide the retention evidence instead. External physical driver memory remains unmeasured. These results show a plateau for this workload, not a universal memory bound for arbitrary external device use.

## Preliminary development-loop measurements

Five runs are saved in `measurements/m1-dev-loop/20261102-204635/results.json`. Windows, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development profile, two Cargo jobs, 960 by 640 initial viewer, two cubes and a triangle, single sample. Contention was uncontrolled. The earlier GPU-check snapshot reported 7% GPU utilization and 4,379 MiB used; that is not a per-run contention measurement. No optimized rendering-performance claim follows from these data.

| Measurement | Recorded result | Method |
| --- | --- | --- |
| No-op build | 0.418–0.444 s, five runs | Cargo process start through exit, including launch overhead |
| Incremental Rust build | 2.563 s, one edit | Same process timing |
| Rust edit to externally observed viewer log | 4.284 s, one edit | Includes build, launch and log polling |
| Startup to present-call proxy | 941.078–1008.062 ms | In-process start through first present call |
| Startup to completion-observed proxy | 948.829–1012.389 ms | Nonblocking observation of submitted frame completion |
| Shader candidate compilation | 1.798–3.199 ms | Five valid reloads |
| Shader write to present-call proxy | 7.507–53.176 ms | File modification time through first matching-revision present call |
| Shader write to completion-observed proxy | 9.925–55.190 ms | File modification time through nonblocking completion observation |

Neither proxy measures scanout or photons. M0 waited before presenting, so its timings are not directly comparable to these asynchronous proxies. Five samples are insufficient to establish tail latency. The script restored exact source bytes, as shown by matching before/after hashes. The later runs below resolve fresh-target build time and repeat the practical loops. Counters-feature cost and exclusive contention control remain unmeasured.

## Final verification after gameplay ended

Islam explicitly ended the gameplay hold. The GPU suite passed on `144d1a1edfc68b3277de403c14d2ce77c6ccbeef`, with only the check helper changed. The helper's `Start-Process -Wait` hung after Cargo had exited; it now waits on the direct process and drains redirected output asynchronously. Logs are in `measurements/m1-implementation/check-20261102-220357/`. All three GPU tests passed in 26.13 s. The three replacement waves reproduced the exact high-water values above, with 704/704 incomplete-and-retained observations in each wave. GPU tests include deliberate invalid shader and foreign-device failures; no unexpected validation error occurred.

Root inspected all seven offscreen results in `captures/m1/final-contact.png`, including depth, camera/model movement, portrait extent, indexed cube, corrected shader and final stress image. Root also exercised the actual viewer with a process-bound Windows helper and inspected client-area captures under `measurements/m1-window/`. The Computer Use package required an unavailable `node_repl` tool, so a local Python/Win32 helper was used. It required the exact viewer process/title and verified foreground ownership before input or capture.

The successful interaction sequence reset the scene, held Right for 0.5 s, toggled cube motion, applied a 24-size resize burst ending at a 664x761 client area, and minimized/restored five times. Logs show all five 0x0 transitions and resumed rendering. The reset, orbit, moving cube, portrait and restored images were inspected. Early helper attempts misencoded Right as Numpad6. A separate unreleased KeyA caused the translated scene. Both attempts were excluded; KeyA was released, the arrow scancode was corrected, and the successful log explicitly reports `ArrowRight`. A foreground reacquisition failed once after restoration; retry succeeded and that failed attempt produced no capture. The 25 ms-spaced resize events produced separate configures, so event coalescing was inspected in code but not observed in this run.

At a static reset, invalid shader text retained a pixel-identical full client image. Corrected tint produced the expected changed image. `reload-baseline.png`, `reload-invalid.png`, and `reload-fixed.png` retain that evidence. No unexpected viewer validation fault or stopped diagnostic appeared. The last four applied-input-to-present-call proxies in the successful sequence were 5.012, 8.095, 8.315 and 7.885 ms. These are event-to-call observations, not physical keyboard latency or scanout. A live valid shader correction compiled in 13.405 ms; the invalid parse took 0.560 ms. The small sample does not establish p95 input latency, but it did not trigger the design's 50 ms reload-worker threshold.

The single-sample silhouettes visibly alias. There is no temporal reconstruction, lighting, shadow or transparency implementation in this fixture. Stable full-frame geometry and correct occlusion here do not certify future image quality. Device/surface loss has a diagnostic restart policy; physical device-loss recovery and other platforms are untested.

## Measured implementation checkpoint

Run `measurements/m1-dev-loop/20261102-220640/results.json` completed successfully and restored exact source bytes. Host remains Windows, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, rustc 1.96.0. Saved snapshots report 0% GPU utilization and 1,795 MiB used before the GPU suite, and 11% with 1,802 MiB used before the development loop. No Ara quiet-window lock or game process was present. This was a shared desktop without exclusive contention control. Two Cargo jobs, development profile, warm registry/OS/driver caches. Fresh-target means no build artifacts in that target directory, not a cold machine.

| Measurement | Final observation | Samples |
| --- | --- | --- |
| Fresh-target build | 24.035 s | One |
| No-op build | Median 0.376 s, range 0.349-0.378 s | Five |
| Library edit build | 1.828 s | One |
| Library edit to externally observed viewer log | 2.832 s | One |
| Startup to present-call proxy | 575.318-616.643 ms | Five |
| Startup to completion-observed proxy | 582.749-624.074 ms | Five |
| Shader candidate compilation | 2.019-2.506 ms | Five |
| Shader write to present-call proxy | 28.758-65.081 ms | Five |
| Shader write to completion-observed proxy | 30.859-66.779 ms | Five |

The shader-write proxy is limited mainly by the arbitrary phase of the 50 ms file poll, versus 2.0-2.5 ms compilation; the preliminary and later ranges overlap, so these samples do not establish a regression. These development builds used wgpu debug/validation flags; validation overhead is an inferred contributor, not an isolated measurement. The first surface configure accounted for 180-190 ms of the 575-617 ms startup proxy.

Frame measurements are saved in `measurements/m1-frames/results.json` and ten individual logs. Five runs each with timestamps enabled/disabled alternated, each presenting 320 frames and analyzing the last 300. Reproduce with `target/debug/renderlab-viewer.exe --frames 320 --log-frames`, alternating with `--no-timestamps`. Every run exited successfully after 320 presentations. Every analyzed static frame uploaded zero geometry, model and camera bytes. This is the 960x640 two-cube/triangle fixture in a development build with FIFO presentation, not an optimized throughput benchmark.

| Timed run phase | Median of five run medians |
| --- | --- |
| CPU preparation | 0.0062 ms |
| CPU upload stage | 0.0005 ms |
| Command encoding | 0.2774 ms |
| Submit call | 0.2126 ms |
| Render admission poll | 0.0117 ms |
| Surface acquisition | 3.11685 ms |
| Present call | 0.2089 ms |
| GPU mesh pass | 0.003072 ms |

GPU mesh-pass p95 was 0.004096 ms in each run; timestamps are quantized and exclude uploads, query readback and presentation. The CPU sum of renderer phases had run medians of 0.4842-0.52535 ms with timing and 0.41085-0.4311 ms without. This observes the cost of optional timestamp collection in this build; it does not isolate the HAL counters feature, allocator reporting, logging, event-loop polling or fixture scene preparation. No counters-feature overhead claim is made. End-to-end 320-frame runs overlapped at 2.090-2.118 s versus 2.066-2.183 s, so there is no demonstrated end-to-end speed difference. Surface acquisition/FIFO pacing dominates this tiny workload. No engine throughput claim follows from the three-microsecond pass.

All ten runs ended with zero Busy frame/upload results and peak flights of one or two. The interactive session later measured 0.013-0.034 ms passes; the cause of that variation is unverified, including possible GPU clock-state changes. Do not extrapolate either range to a production scene.

Pre-run global VRAM observations across the ten runs were 1,803-1,804 MiB, with 4-7% utilization. They include the compositor and other applications and do not establish per-process driver allocation. HAL and allocator plateau evidence remains the resource-retention acceptance basis. Physical per-process memory, optimized rendering benchmarks, statistical tail latency, and instrumentation-free performance remain explicit measurement limits.

## Review corrections and final validation

Opus requested two corrections: capture the Vulkan layer diagnostics, and repair measurement provenance/context. It found no renderer correctness defect and explicitly allowed root to close M1 after these changes without a second review. The review is retained locally in `.refs/m1-final-review.md`.

`support/validation.rs` now installs a test/viewer logger for HAL warnings/errors and Vulkan activation messages. GPU tests require zero errors and missing-layer messages and positive Khronos activation evidence. The viewer always reports diagnostics; `--validate` makes those assertions strict at exit. Ordinary viewer use does not require the SDK layer. The library does not install a global logger or gain a runtime logging dependency. Tests keep the validation scope alive until resources are dropped.

The first logged GPU run failed because the Vulkan loader reported two stale PRISM Live Studio implicit-layer JSON registrations. These were environmental loader errors, not ignored validation errors. `check-20261102-222840` preserves the failure. A process-local `VK_IMPLICIT_LAYER_PATH` pointing at the empty `measurements/empty-vulkan-implicit` directory excludes implicit overlays while retaining the explicit Khronos layer. No registry or machine settings changed. This behavior is documented by the [Khronos loader](https://github.com/KhronosGroup/Vulkan-Loader/blob/main/docs/LoaderLayerInterface.md#windows-layer-discovery). This controlled environment differs from the earlier timing runs, which included implicit layers.

Final source hashes, environment and checks are in `measurements/m1-implementation/check-20261102-223129/`. Formatting, workspace/all-target checks, Clippy and CPU tests passed. All three GPU tests passed with `errors=0`, `missing_layer=0` and positive `active_layer` counts. Activation messages explicitly list core, synchronization, object-lifetime, stateless-parameter and thread-safety checks. No HAL warning or error occurred in this isolated run.

The rebuilt viewer was rerun with `--validate` under the same isolated loader environment. `measurements/m1-window-validation/pid.json` records the executable SHA-256. Logs and inspected portrait/reload captures confirm another resize burst, five minimize/restore cycles, exact whole-image preservation on malformed shader, and visible correction. A foreground reacquisition initially failed after restore; the subsequent reset/capture succeeded. At exit it reported `errors=0 missing_layer=0 active_layer=1`. This resolves R1. Saved contention figures, poll-phase explanation, initial-configure cost and debug-build labels above resolve R2.

Reproduce the isolated acceptance environment from the repository root in a disposable PowerShell session:

```powershell
New-Item -ItemType Directory -Force measurements/empty-vulkan-implicit | Out-Null
$env:VK_IMPLICIT_LAYER_PATH = (Resolve-Path measurements/empty-vulkan-implicit).Path
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/m1-check.ps1 -Gpu
```

The final logger-enabled development-loop run is `measurements/m1-dev-loop/20261102-223238/results.json`, using that same loader isolation and otherwise the previously specified dev profile/host/fixture. Pre-run GPU snapshot was 5% utilization and 1,954 MiB used. All six viewer launches ended with zero diagnostic errors and positive layer activation. Each launch records the executable hash; Rust and shader bytes were restored exactly.

| Final logger-enabled development measurement | Observation | Samples |
| --- | --- | --- |
| Fresh-target build | 23.612 s | One |
| No-op build | Median 0.352 s, range 0.350-0.366 s | Five |
| Library edit build | 1.982 s | One |
| Library edit to externally observed viewer log | 3.453 s | One |
| Startup to present-call proxy | 549.891-581.475 ms | Five |
| Startup to completion-observed proxy | 552.005-583.966 ms | Five |
| Shader compilation | 1.954-2.007 ms | Five |
| Shader write to present-call proxy | 35.439-53.476 ms | Five |
| Shader write to completion-observed proxy | 36.971-55.123 ms | Five |

Initial configure was 186-203 ms. The 50 ms polling period remains the main shader-write latency component. These runs add logging and remove implicit overlays compared with the earlier checkpoint; they are not a controlled before/after comparison or a latency-regression claim. Incremental builds and clean-target compilation retain practical development speed, with the stated small sample sizes and shared-desktop limitations.

Raw `HalStats.memory_allocations` is not maintained by this Vulkan backend and is documented accordingly; allocator byte reports provide the usable memory observation. Counter-feature build/runtime overhead remains deliberately unmeasured. Physical per-process driver memory, optimized performance, portability and physical device-loss recovery remain the accepted limits above. There are no outstanding M1 acceptance failures in the recorded controlled validation environment. Stop at M1.

Pstack comment review resulted in four deletions, including one stale test description, with no structural findings. Dependency constraints and public API contracts were retained. No rerun, architecture sketch, constraint encoding, or unresolved comment finding was required.

Root review found that a key event could be logged against a redraw before held-key movement was applied. The viewer now promotes its input marker only after updating the scene; the successful interaction sequence above exercised the corrected instrumentation. Prove It Works required inspection of both actual-window images and offscreen assertions. Explain the Number kept pass timing, CPU phases, presentation pacing and completion proxies distinct.
