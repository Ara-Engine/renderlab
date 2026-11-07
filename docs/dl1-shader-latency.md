# DL1: shader reload latency

2026-11-07. Implementation base **476cdfe**, branch **t3/dl1-shader-latency**.

**The 100 ms fresh-edit target is missed.** Background compilation keeps the previous
revision rendering, and cached restores/recovery are faster, but the NVIDIA driver
still spends about one second compiling each fresh graphics pipeline. The completed
standard-program batch measures 1,112.510 ms median to a completed image. User-program
and cross-process cache verification are pending the shared GPU lease at this checkpoint.

## Workload and boundaries

The method is [M5-DL](m5-dev-loop.md): a real background native viewer, full-resident
quarry, registered static camera 0, 1920x1080, 1x, 16x anisotropy, original material,
lighting and RLE2 environment. RTX 4070 SUPER/NVIDIA 617.14, Ryzen 7 7800X3D,
Windows 11, Rust 1.96, wgpu 30.0.1/Vulkan. The viewer retains its development profile
and active Vulkan validation; naga now has opt-level 2 alongside the existing wgpu
profile overrides. No shader body, rendering algorithm or quality setting changed.

Each program gets five distinct visible tint edits, each followed by restore, invalid
save and correction. Source hashes and Vulkan pipeline-creation feedback verify fresh
edits rather than cache hits. Save-to-image ends when the accepted revision's first
frame is observed GPU-complete. Present-call timing is separate; neither is display
scanout. Renderer-owned HDR hashes/readback and PNG encoding are outside this endpoint.
The custom file uses the viewer's existing standard-program slot, as in M5-DL.

The baseline numbers below are reused M5-DL evidence, not a new paired before run.
Baseline shader sources are unchanged in this checkout. A small fixture separately
checks live reload and recovery. No frame-time or Unreal comparison is claimed.

## Design

- `ShaderWatch::poll_async` owns at most one compile job. New saves coalesce per file.
  The worker owns device, module-cache and explicit layout handles, and compiles only
  the edited program's hot sets, with one thread per pipeline. It never mutates the
  renderer, source identities or material buffers.
- The old program keeps drawing while compilation runs. A completed result installs
  all its modules, pipelines and reflected parameters between frames. Revision,
  primary-source identity and hot-set checks discard stale snapshots; the watcher
  retries the newest text against a new snapshot. Invalid edits keep the accepted
  image. Watcher destruction joins its worker, including on viewer shutdown.
- Owned recipes reconstruct the current explicit four-bind-group shading ABI from
  the accepted pipelines. They use the same mesh/depth/target descriptors and compute
  entries. Shader functions remain programmable. Fresh, traced and temporal shader
  bodies/layout owners were not modified.
- A bounded 32-composition source-hash cache retains wgpu shader modules, including
  validated naga IR and lowered module data. Only successfully validated modules enter
  it. Validation/reflection of the program interface still runs on every transaction.
- The existing shared `wgpu::PipelineCache` still bounds session history at 512
  successful pipelines. `set_program_cache_directory` adds persistence: wgpu-version,
  adapter and driver keys; checksummed, size-limited files; same-directory atomic
  replacement. Cache data is saved at renderer shutdown, outside the frame loop.
  Missing, damaged or incompatible data falls back to compilation. Unsupported
  devices keep the normal path. The viewer uses `%LOCALAPPDATA%/Renderlab/pipelines`,
  overridable with `RENDERLAB_PIPELINE_CACHE_DIR` (evidence uses E:).
- Shader polling changes from 50 to 10 ms. Lighting/environment file polling remains
  50 ms. Optimizing naga in development reduces the runtime shader frontend cost;
  it does not move shader editing into a Rust rebuild.

The synchronous library reload APIs remain available. Prelude edits retain their
existing synchronous all-program transaction. Cold sets, and streamed variants first
needed after a reload, retain first-use compilation. This is not a general asynchronous
pipeline scheduler for every renderer configuration.

## Where the time goes

Before DL1, M5 measured module/hot-pipeline creation together: **1,038.546 ms** standard
and **1,051.737 ms** user, with **34.305/37.120 ms** save-to-detection. The earlier
[reload design/profile](fast-shader-reload.md) already identified driver compilation
and already selected hot sets and parallel pipeline creation. DL1 does not claim to
have introduced those existing optimizations or to have newly measured a before-side
driver-only split.

Standalone CPU frontend medians, five repeats, milliseconds:

| Program | naga parse before / after | Validation before / after | SPIR-V emission before / after |
|---|---:|---:|---:|
| Standard | 26.974 / 2.639 | 1.814 / 0.227 | 4.124 / 0.473 |
| User | 24.561 / 2.544 | 1.834 / 0.224 | 4.477 / 0.454 |

These are isolated composed-module CPU timings, not additive components of the viewer
endpoint. The before CPU repeats use identical input without any cache; the after
repeats use unique tint sources. SPIR-V emission uses naga's Vulkan 1.3 writer with
the recorded options; it is not an instrumentation hook inside wgpu. This isolates
the development-profile change without claiming exact per-call wgpu internals.

The local Vulkan observer times the actual downstream `vkCreateGraphicsPipelines`
calls and collects the driver's `VkPipelineCreationFeedback`. For the standard batch,
the ten calls take **1,067.386 [1,059.575, 1,330.147] ms** (median [min, max]); driver
reported duration agrees within 0.015 ms. Every edit creates exactly **two pipelines**,
on separate overlapping threads. None reports an application-cache hit. The worker's
module stage is **7.417 ms** median and pipeline stage **1,083.350 ms** median.

A separate uncontended fixture screen measured the two downstream driver calls at
**951.974 and 953.153 ms**, overlapping. This is the observed compile cost for that
edited program on this driver, not a universal lower bound for all shaders/drivers.
It alone exceeds the entire 100 ms target. Splitting work off the frame thread cannot
remove that cost.

## Edit-to-image

Milliseconds, median [min, max], five repeats. M5 before values are historical evidence.

| Program/action | M5 save to completed image | DL1 save to completed image | DL1 reload worker wall | DL1 save to detection |
|---|---:|---:|---:|---:|
| Standard/edit | 1083.233 [970.028, 1130.180] | 1112.510 [1102.994, 1395.171] | 1090.256 | 10.967 |
| Standard/restore | 107.408 [98.381, 108.763] | 62.735 [34.394, 100.015] | 18.249 | 5.938 |
| Standard/recover | 90.329 [87.853, 102.508] | 74.178 [33.640, 94.064] | 13.636 | 5.117 |
| User/edit | 1094.718 [1063.901, 1254.439] | Pending | Pending | Pending |
| User/restore | 119.858 [95.736, 134.328] | Pending | Pending | Pending |
| User/recover | 114.735 [84.113, 121.100] | Pending | Pending | Pending |

Standard invalid saves are observed rejected externally in **33.025 [22.659, 38.969]
ms**. The asynchronous rejection's logged `compile_ms` includes finished-job pickup;
it must not be compared to M5's inline parser-only rejection timing. All five invalid
saves preserve the baseline HDR hash, and all five corrections restore it.

The viewer submits **221-312 old-revision frames per fresh standard edit** before
installation. These are completed caller/presentation-loop records, not a measured
display-refresh guarantee. The fresh edit latency itself did not improve in this run.

## Verification, evidence and remaining work

- `cargo test -p renderlab --lib`: **115 passed, 113 ignored, 0 failed**. The final
  default CPU test execution takes 0.48 s; compilation time is recorded separately.
- Formatting and whitespace checks pass. Strict Clippy has pre-existing
  `nonminimal_bool` diagnostics at `virtual_shadow_gpu.rs:203` and
  `items_after_test_module` at `visibility_buffer.rs:1076`. These files are outside
  DL1 ownership. Affected-package all-target Clippy passes with only those two lint
  categories allowed. Unqualified strict Clippy is therefore **not clean**.
- The GPU registered-program test passes on raster and traced paths. Added checks
  compile every available hot ABI from owned recipes, compare complete HDR images
  with synchronous creation, reject stale snapshots and coalesce saves through
  invalid/recovery states. It also retains the existing parameter/default/array tests.
- The standard quarry baseline HDR hash is exactly the M5 hash:
  `1c4462b43466188d58f0e6e47ef03060e8adb64a1c0e8f3aff88e470172c12f1`.
  All five fresh tints change it; all fifteen restore/reject/recover images match it.
  Inspected the fixture restore and the twenty-image standard contact sheet.
- Vulkan validation reports zero errors. All windows use `--background` with no focus
  or input requests; captures are renderer HDR captures, never desktop/game captures.
  Lease guards check games, foreign renderers/builds and available memory, enforcing
  4 GiB launch / 2 GiB abort floors. No Cargo runs inside these leases.
- `final-a` completed the standard batch, then aborted on a foreign Cargo process at
  12:14:25 UTC before launching the user-program half. No game was detected and its
  minimum sampled available memory was 9,110.8 MiB. `final-b` rejected an existing
  empty output directory; `final-c` deferred on the shared lease. Those attempts are
  preserved rather than counted as completed user measurements.

Evidence: **E:/renderlab-work/dl1/**. `results.json`, `summary.json`, per-viewer logs,
Vulkan JSONL, checksums, CPU profiles, source identities and guarded lease records
hold the measurements. The local C observer and its manifest live there; they are
enabled only in owned child-process environments, with no system Vulkan changes.

Remaining options belong to a follow-up with the appropriate owners: reduce the
inlined environment/helper code while proving identical output (the previous profile
identified it as dominant), or investigate reusable graphics shader stages/pipeline
libraries below wgpu's current public pipeline API. Splitting WGSL source strings
alone does not create separately linkable driver stages. Runtime material parameter
edits already avoid recompilation and should remain the immediate path for value
changes; they do not replace arbitrary user shader edits.

The fresh-edit target and strict baseline Clippy remain explicit misses. User-program
measurements, persistent-cache restart proof and final target cleanup are pending at
this checkpoint. M7/shadow bodies and `milestones-5-6-decisions.tsv` are unchanged.
