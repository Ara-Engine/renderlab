# DL1: shader reload latency

2026-11-07. Implementation base **476cdfe**, branch **t3/dl1-shader-latency**.

**The 100 ms fresh-edit target is missed.** Background compilation keeps the previous
revision rendering, and cached restores/recovery are faster, but the NVIDIA driver
still spends about one second compiling each fresh graphics pipeline. The completed
standard-program batch measures 1,112.510 ms median to a completed image; the user
program measures 1,241.739 ms with compiler contention in its resumed samples. All
forty edit/restore/invalid/recovery image checks pass, and a separate process confirms
application pipeline-cache hits. The implementation checkpoint is **5461230**; the
final commit only completes this report. No fresh-edit speedup is claimed.

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
The custom file uses the viewer's existing standard-program slot, as in M5-DL. The
user batch resumes after a guard abort: two repeats in one process, three in another,
with identical baseline hashes. Every measured fresh source is distinct and all
twenty downstream fresh pipeline calls report no application-cache hit.

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

For the user program, the corresponding worker medians are **8.720 ms** for modules
and **1,201.931 ms** for pipelines. The ten actual driver calls take **1,181.128
[1,093.871, 3,697.033] ms**. The long observation is retained: foreign compilers ran
during the resumed process. These are wall durations, not isolated CPU instruction
time; the shader/frontend share must not be blamed for that driver-stage outlier.

A separate uncontended fixture screen measured the two downstream driver calls at
**951.974 and 953.153 ms**, overlapping. This is the observed compile cost for that
edited program on this driver, not a universal lower bound for all shaders/drivers.
It alone exceeds the entire 100 ms target. Splitting work off the frame thread cannot
remove that cost.

## Edit-to-image

Milliseconds, median [min, max], five repeats. M5 before values are historical evidence.

| Program/action | M5 save to completed image | DL1 save to completed image | DL1 reload worker wall | DL1 save to detection |
|---|---:|---:|---:|---:|
| Standard/edit | 1083.233 [970.028, 1130.180] | 1112.510 [1102.994, 1395.171] | 1090.256 [1082.225, 1351.488] | 10.967 [6.624, 32.936] |
| Standard/restore | 107.408 [98.381, 108.763] | 62.735 [34.394, 100.015] | 18.249 [15.843, 18.534] | 5.938 [1.422, 12.467] |
| Standard/recover | 90.329 [87.853, 102.508] | 74.178 [33.640, 94.064] | 13.636 [11.914, 14.165] | 5.117 [1.165, 12.958] |
| User/edit | 1094.718 [1063.901, 1254.439] | 1241.739 [1148.737, 3762.801] | 1211.218 [1120.167, 3727.454] | 15.731 [12.976, 19.536] |
| User/restore | 119.858 [95.736, 134.328] | 49.959 [42.184, 85.841] | 19.241 [15.371, 24.695] | 9.807 [3.791, 11.402] |
| User/recover | 114.735 [84.113, 121.100] | 58.163 [34.821, 93.281] | 12.495 [10.948, 16.858] | 11.565 [2.403, 12.791] |

Standard invalid saves are observed rejected externally in **33.025 [22.659, 38.969]
ms**; user invalid saves take **28.427 [17.442, 33.597] ms**. The asynchronous
rejection's logged `compile_ms` includes finished-job pickup;
it must not be compared to M5's inline parser-only rejection timing. All five invalid
saves per program preserve their baseline HDR hash, and all ten corrections restore it.

Separate save-to-present-call proxies:

| Program | Fresh edit | Restore | Recover |
|---|---:|---:|---:|
| Standard | 1108.345 [1098.850, 1390.708] | 34.120 [30.472, 56.409] | 33.132 [26.647, 46.151] |
| User | 1230.184 [1144.408, 3758.523] | 41.672 [38.764, 48.649] | 35.726 [30.148, 53.078] |

The viewer submits **221-312 old-revision frames per fresh standard edit** before
installation, and **260-386** per user edit. These are completed caller/presentation-loop
records, not a measured display-refresh guarantee. The fresh edit latency itself did
not improve in this run. The user outlier is not removed to make a favorable comparison.

## Verification, evidence and remaining work

- `cargo test -p renderlab --lib`: **115 passed, 113 ignored, 0 failed**. The final
  default CPU test execution takes 0.48 s; compilation time is recorded separately.
- Formatting and whitespace checks pass. Strict Clippy has pre-existing
  `nonminimal_bool` diagnostics at `virtual_shadow_gpu.rs:203` and
  `items_after_test_module` at `visibility_buffer.rs:1076`. These files are outside
  DL1 ownership. Affected-package all-target Clippy passes with only those two lint
  categories allowed. Unqualified strict Clippy is therefore **not clean**.
- The final GPU registered-program test passes on raster and traced paths (**1 passed,
  8.21 s**, zero validation errors). Added checks
  compile every available hot ABI from owned recipes, compare complete HDR images
  with synchronous creation, reject stale snapshots and coalesce saves through
  invalid/recovery states. It also retains the existing parameter/default/array tests.
- The standard quarry baseline HDR hash is exactly the M5 hash:
  `1c4462b43466188d58f0e6e47ef03060e8adb64a1c0e8f3aff88e470172c12f1`.
  The custom baseline also exactly matches M5:
  `10ee30f19e0f50ecde34c3cca32c3bc806e539aa9eba73f2d0cd1c259ae33690`.
  All ten fresh tints change their baseline; all thirty restore/reject/recover images
  match it. Inspected the fixture restore, both twenty-image contact sheets, and the
  complete 1080p recovered user image.
- A new process restores **1,217,821 driver-cache bytes**. Its two edited-program
  graphics pipelines report `VALID | APPLICATION_PIPELINE_CACHE_HIT` and complete
  inside Vulkan in **0.641/0.898 ms**. This distinguishes an application-cache hit
  from merely assuming NVIDIA's own disk cache served the shader.
- Completed viewer processes and the final API test report zero Vulkan validation
  errors. The guard-terminated user process has no shutdown validation summary.
  All windows use `--background` with no focus
  or input requests; captures are renderer HDR captures, never desktop/game captures.
  Lease guards check games, foreign renderers/builds and available memory, enforcing
  4 GiB launch / 2 GiB abort floors. No Cargo runs inside these leases.
- `final-a` completed the standard batch, then aborted on a foreign Cargo process at
  12:14:25 UTC before launching the user-program half. Its final standard capture
  is timestamped 12:14:24.110; no build was observed in its earlier samples. `final-b`
  rejected an existing empty output directory without launching a viewer. `final-g`
  completed two user repeats, then similarly stopped on a foreign Cargo process.
  `final-h` resumed the last three repeats and completed cache/API verification. It
  recorded foreign builds in **13/13** contention samples; it retained the game and
  memory guards but recorded, rather than aborted on, compiler activity. Consequently
  the user timings are **contended observations**, not an uncontended candidate
  performance certificate. The uncontended fixture driver screen independently
  establishes the 100 ms miss. Other lock-held attempts simply deferred.

| Lease | Seconds | Available RAM at launch / minimum sampled, MiB |
|---|---:|---:|
| screen-a | 14.644 | 15095 / 14684.8 |
| final-a | 65.376 | 12647 / 9110.8 |
| final-b | 2.666 | 12169 / 12085.2 |
| final-g | 37.108 | 9455 / 6173.0 |
| final-h | 66.471 | 8117 / 5310.9 |

All five owned leases are released; no owned GPU/window process remains. The Cargo
target and standalone verification executables are deleted. Raw HDR/PFM dumps are
deleted after hashing and inspection. Only small source/tool files, JSON/CSV/log
evidence, the tested cache and inspected PNGs remain on E:.

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

Time accounting is separate from the shader endpoint: implementation/research/docs
time was not independently stopwatched; it interleaved with builds and shared-lease
waits. Recorded verification includes the initial test build (**112 s**), the naga
profile rebuild (**115 s**), and the final CPU/check/viewer preparation batch
(**75.525 s** inclusive: test compilation 40.11 s, CPU tests 0.48 s, Clippy 16.02 s,
viewer build 16.77 s). Earlier preparation/check logs are retained too. The five GPU
lease holds total **186.300 s**, including loading, capture encoding, checks and
aborted attempts; this is not pure GPU execution time. Warm incremental Rust edit
latency and startup performance were not re-benchmarked by DL1.

The fresh-edit target and strict baseline Clippy remain explicit misses. M7/shadow
bodies and `milestones-5-6-decisions.tsv` are unchanged. Final source identities,
binary SHA-256 values and cleanup evidence are retained outside Git.
