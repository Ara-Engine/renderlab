# DL2: Vulkan shader compilation

2026-11-07. Base **17b89e8**, branch **t3/dl2-shader-latency**.

The fresh-edit **100 ms target remains missed**. Retaining SPIR-V helper call
boundaries materially reduces the NVIDIA compiler cost; the Vulkan disable-optimization
flag alone and graphics pipeline libraries do not remove it. This is an opt-in,
**transient development preview**, followed by a background optimized replacement.
The ordinary device and default rendering policy are unchanged.

## Method

Use the [M5-DL workload](m5-dev-loop.md) and [DL1 baseline](dl1-shader-latency.md):
RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11, Rust 1.96,
wgpu 30.0.1/Vulkan, development build with naga/wgpu profile overrides and Vulkan
validation. Full-resident quarry, static camera 0, 1920x1080, 1x MSAA,
16x anisotropy, original lighting and RLE2 environment. Shader bodies and all quality
settings are unchanged. The custom user file uses the standard-program slot, exactly
as in M5-DL/DL1; the separate registered-program fixture covers the public API.

Each timing batch uses five fresh, unique visible tint edits for each program, each
followed by restore, invalid save and recovery. Save-to-image ends at observation of
the accepted revision's first GPU-complete frame; present-call timing is separate.
Neither endpoint is display scanout. Native-call timing surrounds `vkCreate*Pipelines`;
source hashes and pipeline-creation feedback distinguish fresh work from cache hits.
The baseline is reused evidence, not a newly paired baseline run. No frame-time or
Unreal comparison is claimed.

The first helper-preview batch requested a preview readback after its completion.
Its preview endpoint is valid, but its later optimized endpoint can include PNG
encoding, so that optimized endpoint is excluded from performance claims. Capture
metadata also revealed that most external preview requests arrived after promotion;
those images are not described as preview evidence. A separate, untimed quality
probe queues the HDR copy with the preview's first frame and asserts its revision.
Other timing batches request readback only after both timed endpoints.

Measurements, image identities, source hashes, logs and guarded lease records live
under **E:/renderlab-work/dl2/**. Generated/private evidence is not committed.
`summary.json` contains every selected sample's source hash and median/range;
`final-identity.json` records the verified source and binary hashes. The ignored
library tests `gpu_vulkan_compile_screen`, `gpu_vulkan_helper_size` and
`gpu_registered_program_files_defaults_arrays` are the public diagnostic entry points.
They require the existing guarded lease/manifest protocol; set
`RENDERLAB_VULKAN_INTEROP=1` to use the interop device. The viewer's measurement-only
policy/reference and first-preview-capture controls are active only with
`RENDERLAB_LATENCY_PROBE` and `RENDERLAB_SHADER_COMPILE_POLICY_FILE`.

## Implementation

`shader_compiler::request_device` uses wgpu-hal's Vulkan device creation callback,
preserving the requested features, limits, memory policy and queue. It imports that
same device through `create_device_from_hal`, with a typed ash dispatch table that
intercepts shader/pipeline creation. Executable pipelines, binding, synchronization
and retirement remain owned by wgpu. No wgpu fork, second Vulkan loader or raw
pipeline import is needed. The direct ash version matches wgpu-hal's dependency.
Non-Vulkan/Vulkan-before-1.3 devices use the ordinary device path.

Policies are scoped to the creating thread and only edited program jobs opt in:

| Viewer option | First pipeline | Background replacement |
|---|---|---|
| default / `--shader-preview optimized` | Ordinary optimized | None |
| `--shader-preview preview` | `DISABLE_OPTIMIZATION` | Ordinary optimized |
| `--shader-preview no-inline` | Same flag + `DontInline` on non-entry functions | Ordinary optimized, original SPIR-V |
| `--shader-preview helpers` | Same flag + `DontInline` on named environment helpers | Ordinary optimized, original SPIR-V |
| `--shader-preview library` | Cached graphics subsets + fresh fragment + fast link | Link retained library IR with LTO |

Preview selection requires a debug build. The title identifies transient preview
output. `ShaderWatch::set_preview_policy` must be configured while idle on the
opt-in device. Normal/release callers retain `None`. The helpers policy uses SPIR-V
debug names; when unavailable, it falls back to retaining all call boundaries.
This changes compile hints only, not WGSL bodies or shader interfaces.

The previous pipeline renders while a bounded worker compiles the edited program's
hot sets in parallel. A preview installs atomically between frames, submits at least
one frame, then starts its optimized replacement. Source, ABI revision and hot-set
checks discard stale work. Saves coalesce while a compile is pending. Invalid edits
retain the last accepted source and its working pipeline; promotion of that source
does not clear the invalid-file diagnostic. Switching to temporal rendering forces
ordinary optimized pipelines before admitting history, without changing M7 code.

NVIDIA exposes `VK_EXT_graphics_pipeline_library`, fast linking and independent
interpolation decoration. The implementation caches vertex input, pre-rasterization,
fragment output, and fragment libraries separately. The first three are prewarmed
with ordinary initial pipelines. A program-only edit reuses the engine vertex stage;
its key includes the prelude identity, vertex entry, state and ABI. Fragment libraries
use actual SPIR-V hashes. Layout keys use bind-group creation generations, not recycled
handles. Render-pass destruction clears the cache. Unknown state/extensions fall back
to monolithic creation. Libraries are bounded to 128 entries and held through linking;
only linked executables go to wgpu. The extension provides **graphics**, not compute,
libraries; compute previews use the flag/call-boundary paths.

DL1's source-module cache and persistent, adapter/driver-keyed wgpu pipeline cache
remain active. Restores can reuse them. A previously unseen fragment still requires
driver compilation even with fixed graphics subsets cached.

## Driver breakdown

Initial raw flag screen, two unique sources per cell, milliseconds:

| Pipeline | Ordinary optimized range | Disable optimization range |
|---|---:|---:|
| Graphics | 886.976–904.544 | 883.630–952.486 |
| Traced compute | 1,281.799–1,309.594 | 1,196.065–1,316.186 |

The flag alone supplies no reliable improvement. In the subsequent contended
call-boundary screen, standard graphics fell to **375.258–396.398 ms**, and compute
to **474.880–497.115 ms**. A quiet-start screen observed named-helper graphics
creation at **226.741 and 234.964 ms**, versus ordinary **825.331 and 912.439 ms**.
That lease later aborted when a foreign build began; its completed native calls are
retained as diagnostic observations, not a completed quiet performance certificate.
The fastest observed standard compile still exceeds 100 ms before detection,
naga/SPIR-V processing, installation or drawing.

GPL fixed-state reuse works. Fast links take roughly **0.1–0.2 ms**; the initial
contended library screen still spends **1.6–2.1 s** creating each fresh fragment
library. The quiet-start screen observes **836–860 ms** for the same fresh-library
operation, including sub-millisecond linking. LTO cannot remove work already spent
compiling the new fragment. Unchanged vertex/input/output subsets are absent from
edited-pipeline timing records, demonstrating reuse rather than merely faster rebuilds.

## Edit-to-image results

Milliseconds, **median (min–max), five unique edits per program/policy**. These are
observations with recorded foreign compiler activity, not a completed uncontended
performance certificate. The helper-boundary preview is the retained latency win.

| Policy / program | Fresh edit to completed preview | Restore to completed preview | Recovery to completed preview |
|---|---:|---:|---:|
| Disable optimization / standard | 1,103.022 (1,073.949–1,286.034) | 43.962 (37.141–45.308) | 45.897 (39.280–53.213) |
| Disable optimization / user | 1,169.097 (1,103.178–1,216.398) | 79.881 (43.800–103.095) | 77.128 (52.865–93.241) |
| **Helper boundaries / standard** | **356.959 (332.315–782.572)** | 89.107 (84.191–356.346) | 50.157 (45.495–140.695) |
| **Helper boundaries / user** | **364.356 (355.357–404.007)** | 68.894 (53.471–327.299) | 52.240 (45.557–117.401) |
| GPL fast link / standard | 1,129.258 (1,070.014–1,359.446) | 57.780 (33.847–1,362.880) | 49.149 (42.701–80.532) |
| GPL fast link / user | 1,125.828 (1,044.331–1,906.494) | 56.412 (39.191–1,387.013) | 48.958 (34.678–133.222) |

DL1 completed-image medians were **1,112.510 / 1,241.739 ms** (standard/user);
M5-DL was about **1,083 / 1,095 ms**. These are historical, unpaired comparisons.
Cold first restores under a new preview policy must compile that policy's version
of the original source; their costs remain in the ranges above. No outlier is removed.
The library user batch retains four repeats from the original viewer and restarts its
last repeat after the viewer's 240-second lifetime expired. The partial earlier last
repeat is retained in evidence but excluded from the five complete paired repeats.

Separate fresh-edit present-call proxies, same five samples:

| Policy / program | Save to present call, median (range) | Save to optimized completed image, median (range) |
|---|---:|---:|
| Flag / standard | 1,098.511 (1,069.721–1,281.787) | 1,140.360 (1,112.147–1,323.498) |
| Flag / user | 1,164.874 (1,098.655–1,207.910) | 1,206.457 (1,140.489–1,255.122) |
| Helpers / standard | 343.549 (318.170–740.708) | Excluded: preview capture before handoff |
| Helpers / user | 350.681 (340.928–390.198) | Excluded: preview capture before handoff |
| GPL / standard | 1,125.107 (1,065.851–1,355.144) | 1,166.782 (1,107.609–1,396.740) |
| GPL / user | 1,122.151 (1,040.121–1,904.235) | 1,151.345 (1,072.352–1,946.222) |

The native helper-preview promotion still spends approximately one second compiling
the original optimized SPIR-V on its worker; it is independent of the preview hint.
The other modes often promote quickly because their preview already compiled the
same source/retained library IR. Invalid-source worker rejection takes **0.186–0.377
ms** across the batches; that is worker time, not save-to-diagnostic latency. All
thirty invalid saves retain the preceding accepted image, and every recovery/restored
image passes its full HDR hash check. All thirty fresh edits change the image.

Before/after breakdown for the retained helper preview, medians in ms:

| Component | M5 standard / user | DL2 standard / user |
|---|---:|---:|
| Save to detection | 34.305 / 37.120 | 11.710 / 12.851 |
| Module + hot pipeline creation | 1,038.546 / 1,051.737 | 317.119 / 321.652 |
| Module/reflection portion | Not separately measured in M5 | 7.614 / 8.791 |
| Hot-pipeline portion | Not separately measured in M5 | 310.559 / 314.550 |
| Save to completed image | ~1,083 / ~1,095 | 356.959 / 364.356 |

Each edit rebuilds two hot graphics pipelines concurrently. The native compiler
dominates their wall time; these medians are not additive because component medians
come from different repeats. DL1's isolated optimized-naga parse/validation/SPIR-V
medians remain **2.639/0.227/0.473 ms** for standard and
**2.544/0.224/0.454 ms** for user; DL2 does not change that frontend or relabel its
standalone profile as instrumentation inside wgpu.

## Helper-size experiment and M7 proposal

`gpu_vulkan_helper_size` composes synthetic user programs against the unchanged
engine prelude. It varies only the number of distinct calls to the public environment
reflection service (0, 1, 2, 4); directions/roughness vary to prevent identical-call
folding. No environment, lighting or shadow helper file was edited. Values below are
two-repeat medians from the contended screen, in ms; they establish a diagnostic
relationship, not an uncontended optimization certificate.

| Calls | Graphics normal | Graphics no-inline | Compute normal | Compute no-inline |
|---:|---:|---:|---:|---:|
| 0 | 26.627 | 45.456 | 39.680 | 110.737 |
| 1 | 630.020 | 107.900 | 656.860 | 177.623 |
| 2 | 1,327.297 | 96.745 | 1,371.990 | 180.598 |
| 4 | 3,727.560 | 98.728 | 4,048.440 | 192.169 |

Fragment+vertex SPIR-V grows only from 79,980 to 96,332 bytes; compute grows from
81,532 to 97,384 bytes. The steep compile-time growth is associated with the call
graph the driver expands, rather than proportional source/module byte growth.
This is evidence about compiler behavior, not a measurement of final ISA size.

Proposal for M7: preserve the exact environment sampling, seam handling, bank/mip
selection and finite-value arithmetic while reducing repeated expansion of the
environment call graph. First profile individual helper boundaries and retained-call
subsets under both optimized and preview flags; the all-function hint has measurable
overhead even with zero environment calls. Then test a smaller equivalent helper graph
on this same synthetic sweep and the complete quarry. Require byte-identical optimized
HDR on default images, complete static/motion/disocclusion quality gates on RT and
non-RT paths, and separate frame-time and edit-latency measurements before adoption.
No material-model-specific receiver data or dispatch shortcut is proposed.

Splitting WGSL strings alone cannot separately link functions inside a Vulkan fragment
stage. GPL only avoids rebuilding the other graphics subsets. A future backend route,
such as shader objects, still needs a measured fresh-shader compile result; its API
name does not establish a sub-100-ms compiler. Persistent caches and parameters help
repeated sources/value edits, not previously unseen program bodies.

## Validation and limits

`cargo test -p renderlab --lib`: **117 passed, 116 ignored, exit 0**. Formatting passes.
Clippy passes for the library/viewer and all targets with the same two documented
baseline lint exceptions as DL1: `nonminimal_bool` in `virtual_shadow_gpu.rs` and
`items_after_test_module` in `visibility_buffer.rs`. Strict repository-wide Clippy
therefore remains a baseline miss; those out-of-ownership files are unchanged.

The registered-program GPU fixture covers raster and traced compute, all four preview
policies, preview-to-optimized promotion, an invalid save during promotion, stale
transactions, recovery, parameter/default/schema preservation, and in-flight old-frame
submission. Every fixture preview and optimized output differs from the ordinary
optimized reference by **0/4,096 HDR pixels**. Validation reports zero errors.

The separate quarry quality probe compares the actual first-preview revision,
optimized replacement and exact-source ordinary monolithic reference. Flag, named
helper and GPL previews each differ by **0/2,073,600 pixels**, maximum and mean
absolute HDR error **0**, for both standard and custom user programs. Their optimized
replacements also differ by zero pixels. PNGs are visually inspected for the complete
scene and silhouettes. The original default standard HDR hash remains
`1c4462b43466188d58f0e6e47ef03060e8adb64a1c0e8f3aff88e470172c12f1`;
the custom baseline remains
`10ee30f19e0f50ecde34c3cca32c3bc806e539aa9eba73f2d0cd1c259ae33690`.
This is output preservation on the existing workload, not a new M7 quality certificate.

All GPU/window runs use the shared atomic lease, reservation/release lines, game and
4/2 GiB memory guards. Viewers start inactive, use renderer-owned captures, and close
after the run. No desktop/game capture or input/focus automation is used. Foreign
compiler contention is recorded; it is not disguised as an uncontended result.

| Owned lease | Hold, s | Samples with foreign builds / samples | Minimum available RAM, MiB |
|---|---:|---:|---:|
| raw-screen-a | 16.854 | 2/3 | 7,243 |
| library-screen-b | 36.222 | 4/4 | 3,867 |
| screen-c | 60.361 | 8/13 | 13,646 |
| screen-d (aborted on foreign build) | 15.012 | 1/3 | 15,001 |
| screen-e | 34.669 | 6/7 | 13,388 |
| window-helpers | 132.400 | 11/30 | 12,120 |
| quality-a | 68.450 | 0/15 | 12,088 |
| window-controls (viewer lifetime reached) | 245.246 | 13/56 | 11,474 |
| window-library-tail | 37.819 | 7/7 | 10,320 |

All launched runs passed the 4 GiB preflight; none fell below the 2 GiB runtime floor.
The quiet-start abort was foreign Cargo activity, not memory pressure. Lock-held
deferrals launched no viewer. All leases are released and owned viewers are closed.

Implementation/research time was not independently stopwatched; it interleaved with
shared-resource waits. Recorded CPU preparation/check time is **505.433 s**, including
the initial dependency/test build; the final unchanged-library/viewer/check batch
takes **4.936 s**. Owned GPU lease holds total **647.032 s**, including driver CPU
compilation, scene loading, image readback/encoding and the aborted attempts, not
GPU execution time. Startup, Rust edit-to-running-viewer and frame time were not
re-benchmarked. Own Cargo target/binaries and large raw image dumps are deleted after
the measurements; compact JSON/log summaries and inspected PNGs remain on E:.

Native Vulkan dispatch, lifetime and cache behavior have been verified on this one
Windows/NVIDIA configuration, not certified across vendors. Preview hints can affect
other shaders' floating-point results and runtime performance; preview is never the
shipping default. Optimized comparisons are explicit per tested source, not a proof
for all possible user programs. No M7/shadow bodies or decision TSV are changed.

Primary references: [Vulkan pipeline flags](https://docs.vulkan.org/refpages/latest/refpages/source/VkPipelineCreateFlagBits.html),
[graphics pipeline library design](https://docs.vulkan.org/features/latest/features/proposals/VK_EXT_graphics_pipeline_library.html),
[graphics library state](https://docs.vulkan.org/refpages/latest/refpages/source/VkGraphicsPipelineLibraryCreateInfoEXT.html),
[SPIR-V function control](https://registry.khronos.org/SPIR-V/specs/unified1/SPIRV.pdf).
