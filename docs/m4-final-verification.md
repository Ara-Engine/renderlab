# M4 final viewer and development evidence

M4 is accepted after independent Opus review and the verified corrections below. The capacity/failure matrix, equal-budget reference images and optimized observations are in [m4-acceptance-matrix.md](m4-acceptance-matrix.md). Root reran that campaign's identity audit over all 28 recorded processes and inspected both complete contact sheets. The production renderer, selector and runtime were unchanged by that campaign.

## Live viewer findings and corrections

Root used a PID-scoped Win32 helper because the installed Computer Use skill's Node runtime was unavailable. Only owned Renderlab windows received input or client-area captures. Gameplay and quiet-window guards were clear. The helper is `.refs/m4-window.py`; actual commands, executable hashes, action times and log-byte offsets are saved under `measurements/m4-window/`. No private assets were used.

The initial attempts are retained. A foreground check refused input after minimize/restore; the helper now refreshes the owned HWND and does not require activation to place its own occluding fixture. Those helper failures are separate from two actual viewer defects found by the tests:

- A valid reload before the first image left the startup timing marker waiting for revision 1 forever. It now binds to the first actually presented revision. In the corrected delayed-loading run both first-frame markers report revision 2.
- Progressive startup acquired and dropped two swapchain images while waiting for root pages. Subsequent acquisition stopped progressing, so the page-advance path never ran again. The native Vulkan discard in wgpu-hal 30.0.1 (`src/vulkan/swapchain/native.rs`, `discard_texture`) is explicitly a no-op. Progressive startup now advances minimum-cover preparation before acquiring. The viewer also retains an acquired frame across nonblocking render backpressure instead of discarding it. Resizing releases that frame before reconfiguration, and an explicit `View::drop` releases it before surface teardown on every exit path. Normal frames introduce no completion wait.

The original `progressive` log preserves the no-image timeout with two `accepted=false` attempts and clean validation. The first fix run is `progressive2`. The final source was built with two Cargo jobs and passed formatting and viewer Clippy before the final sessions. Their viewer SHA-256 is `0a624e1ea857b3392bc7f1efe13e54581a5910600fe50f37b684edb5b8315d46`. `summary.json` records 695, 2117 and 2019 presented-frame log entries respectively:

| Session | Exercised behavior | Result |
| --- | --- | --- |
| `loading6` | Registered overview, 3000ms artificial delay per page; malformed/corrected shader, resize, minimize/restore and own-window occlusion before first scene | All loading actions finish before first scene; settled first scene at 12.702s, present proxy 12.707s, completion-observed proxy 12.707s. Revision 2 is reported correctly. The deliberate IO delay is not normal startup latency. |
| `motion2` | Free camera with real ArrowRight/ArrowUp/D input; 24 resize requests, portrait extent, three minimize/restore cycles, physical occlusion, invalid/corrected reload | Input-to-present-call proxies 7.875/7.427/7.644ms. Captures show the camera movement, whole portrait scene and restored landscape. These are three observed key presses, not a latency distribution. |
| `progressive3` | Explicit coarse preview with 3000ms page delay; reload while four pages remain queued and two roots are resident, followed by resize/minimize/occlusion | First scene at 6.856s, before final detail. Reload and correction retain the displayed coarse cover while streaming continues. This opt-in coarse image is not certified by the default prepared-first-image quality gate. |
| `failed-root` | Isolated cache containing the unchanged package but no page files | Explicit pinned-root failure and exit 1 in 0.552s. The original cache is untouched. |

All four final sessions have an active Khronos layer and zero validation errors or missing-layer reports. Client pixels for reload baseline, malformed shader and correction are byte-identical within each of the three interactive sessions. Root inspected the fresh loading baseline, progressive cover, moved scene and portrait captures. Single-sample thin fins still alias; no new antialiasing claim is made. Registered-camera sessions do not prove camera response to keys; `motion2` supplies that evidence.

Actual minimize events produce zero-sized resize requests, and rendering resumes after restoration. The owned opaque window physically covers and uncovers the viewer during loading and streaming. In winit 0.30.13, the Windows backend does not emit `WindowEvent::Occluded`; that event branch cannot run on this host. Physical cover/uncover and minimize/restore provide the applicable Windows evidence.

Root recomputed acquisition and presentation from the final logs using `.refs/m4-final-review/acquire_present.py`. For loading6/motion2/progressive3 respectively, excluding frames that configure the surface, acquire p50/p99/max is 1.532/2.240/18.858ms, 1.269/1.973/54.245ms and 1.493/2.949/42.409ms; present-call p50/p99/max is 0.212/0.306/0.547ms, 0.232/0.452/3.710ms and 0.228/0.446/4.215ms. These are 694/2087/1989 host-wall call samples in development builds with validation, frame logging and FIFO presentation, not scanout or measured vsync latency.

An earlier `progressive2` uncover/resize recorded a 4933.549ms surface configure. Its cause is unproven and the log remains saved. The final `motion2` and `progressive3` runs did not reproduce it: post-startup configure calls span 11.777-103.142ms and 11.863-136.560ms respectively. This does not prove the earlier stall fixed. Surface reconfiguration can wait internally and stall the UI; no universal resize-latency bound is claimed. The retained-frame fallback is defensive for valid backpressure; these final runs do not separately force that branch.

## Development loop

`scripts/measure-dev-loop.ps1` now accepts a virtual manifest/cache and optional camera file while preserving its source lock, exact-byte restoration, unique edit anchors and successful-frame/reload requirements. Root ran it once with a fresh target, five no-op builds and five startup/shader launches. The library edit is one clear-color constant change; it triggers a real library/viewer rebuild, not importer work. The shader tint edit creates and installs a new pipeline without a Rust build.

Evidence is `measurements/dev-loop/20261103-155512/{results.json,summary.json}` and its process logs. Windows PowerShell wrote this original results file in the local ANSI encoding; the helper now explicitly writes UTF-8. Its original scene text also incorrectly says 960x640. The supplied held overview camera and configure logs establish **1920x1080**, 1x, opaque unlit, Depth32Float. The helper text is corrected without replacing the original evidence. Those two metadata-only changes happened after the measurement; no measured Rust or shader code changed.

Host: Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, ordinary unoptimized development profile, two Cargo jobs. Every launch uses active validation. The pre-run snapshot is 23% GPU utilization and 2630MiB GPU memory, so contention is uncontrolled. No competing Renderlab build or GPU task ran. Asset/package content is unchanged, and OS/download/toolchain caches were not flushed. The held overview is derived from registered pose 0 and its exact file hash is in results.json.

| Measurement | Observation |
| --- | --- |
| Fresh target build, one run | 44.243s process wall, 44.01s reported by Cargo; dependency downloads and OS caches warm |
| No-op build, five runs | Median 0.388s, range 0.381-0.406s, process start through exit |
| Incremental library edit build, one run | 3.213s process wall |
| Edit through observed first-frame log, one run | 5.088s; includes build, process creation, viewer setup, preparation and log polling |
| Startup to first present call, five launches | Median 859.509ms, range 782.348-886.605ms |
| Startup to completion observed, five launches | Median 862.061ms, range 784.829-888.945ms |
| Shader write to first present call, five reloads | Median 47.108ms, range 37.508-58.392ms |
| Shader write to completion observed, five reloads | Median 48.685ms, range 39.360-60.400ms |
| Shader validation/pipeline compile, five reloads | Median 4.093ms, range 3.673-4.591ms |

These are observed proxies, not scanout, cold-disk startup, maximum throughput or a comparison against an earlier milestone. The 50ms shader polling interval explains much of edit-to-image variation; compilation is separately measured. Device/renderer setup reaches 445.869-525.691ms from main entry, initial configure costs 186.255-202.288ms, and the first scene is submitted at 669.969-771.818ms. Completion and present markers include further first-frame work and scheduling. Percentiles or medians of different phases must not be summed.

All six launches (the incremental launch plus five shader launches) succeed and report clean validation. Rust and shader before/after hashes match exactly. The restored-source viewer used for the five launches is `6343e22c191f00baf1fb6e351a49f192f557822a81bc2635746eddad1dd80543`; the edited-library binary is `a686b66208d3ee55272a89313547d344c1b6f93126c5669df614f6ae5be27265`. The interaction binary differs after the rebuild, and is retained by hash rather than as an archived executable. The source inventory recorded before the measured edit and exact restoration bind the viewer source to both runs; no byte-identical executable claim is made.

## Independent acceptance and limits

The recorded production/test suite has 63 CPU checks and 19 GPU cases. New replay changes are tooling-only; both example tests and focused check/Clippy pass. The viewer corrections were checked and exercised against actual surfaces as above. Independent Opus review (`.refs/m4-final-review.md`) recomputed the raw capacity, image, timing, interaction and development evidence and accepted M4 after three corrections, with no further review or GPU run required.

Review found that a retained image could outlive its surface on exit because of field drop order. Root added explicit teardown in `View::drop`, then checked formatting, Clippy and the viewer CPU test. The other required corrections clarify Windows occlusion support and report acquisition/presentation above. The reviewer explicitly accepts the unexplained resize outlier and the unforced retained-frame branch after the teardown fix, based on source evidence.

Root also resolved the comment review: removed 16 narrative comment lines and trimmed one redundant sentence, retained public contracts and the verified wgpu constraint, named the checked maximum page/staging accounting bound, and added a passing golden test for the published default cache key and nondefault source-policy invalidation. The dependency-tree guard now rejects serde in the libraries while allowing it in tools. These additions do not change rendering or cooking policy. The viewer's virtual-scene log label is corrected. The other optional cleanup suggestions are deferred; no redesign or new performance campaign was needed. The separate `deslop` add-on was unavailable and was not claimed as run.

These review corrections postdate the measured runs. The rebuilt normal viewer is `722dbfdc8a17c9b17716917a51be129e5ed7627d1a44ed7750a40a08fd0296e4`; it is not claimed byte-identical to a measured executable. `.refs/m4-root-final/post-review-sources.json` records its final source state, while the earlier source records remain unchanged. Formatting, viewer/cooker Clippy, the viewer test, the new golden cache-key test and normal-library dependency checks pass. Review accepted source reasoning for explicit retained-frame teardown, so no additional GPU run was required.

This milestone demonstrates bounded virtual geometry and covered degradation on registered self-authored content. It does not demonstrate Unreal/Nanite superiority, arbitrary-history image quality, driver-memory bounds, an out-of-core importer, or general large production scenes. The normal GPU timing spread remains materially contended. No GI, advanced lighting, transparency, topology editing or Ara integration is added.
