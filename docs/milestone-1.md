# Milestone 1 evidence record

Status: implementation under review. Final GPU and window-interaction acceptance remains open. This record does not declare Milestone 1 complete.

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

Neither proxy measures scanout or photons. M0 waited before presenting, so its timings are not directly comparable to these asynchronous proxies. Five samples are insufficient to establish tail latency. The script restored exact source bytes, as shown by matching before/after hashes. Fresh-target build time, instrumentation cost and controlled repetitions remain pending.

## Remaining acceptance gates

Islam's active gaming session takes priority. No viewer launches, focus changes, input automation, desktop captures, GPU tests or heavy timing builds are allowed until he says he is done playing.

1. Recheck host contention and GPU ownership, then run the GPU suite on the final source. Inspect all relevant saved images.
2. Exercise actual moving, resizing, minimized and restored viewer behavior. Verify invalid shader reload and correction during interaction. Measure applied-input latency and synchronous reload stalls; use the recorded threshold in the design to decide whether a reload worker is needed.
3. Record separate representative CPU/GPU/acquire/present measurements, resource churn and upload counts. Measure a fresh-target build and instrumentation overhead with honest contention labels. Assess physical-memory evidence separately from allocator counters.
4. Obtain the reserved independent Opus acceptance review, fix findings, update this record, and push the final validated checkpoint. Stop at M1.

Pstack comment review resulted in four deletions, including one stale test description, with no structural findings. Dependency constraints and public API contracts were retained. No rerun, architecture sketch, constraint encoding, or unresolved comment finding was required.

Root review found that a key event could be logged against a redraw before held-key movement was applied. The viewer now promotes its input marker only after updating the scene. Runtime verification of that correction remains pending.
