# P-PREVIEW: first-edit preview compilation

Implementation unit, 2026-11-10. Base `caff1005cb14c8ba7465c35e227b90916ec9cd34`.
Status: release build and CPU library suite pass; GPU verification is queued. No default changed.

## Design

Reuse the existing transactional background program watcher. A `SpirvPreview`
policy preprocesses the native SPIR-V immediately before Vulkan module creation;
the preview publishes only after all active ordinary/list/dependent glass pipelines
succeed. After at least one submitted preview frame, the watcher compiles naga's
original SPIR-V and installs the full pipeline between frames. Existing snapshot
invalidation and coalescing remain in force. A same-source optimized promotion
preserves the edited revision, avoiding a second temporal-history reset. WGSL validation and the central
P-FREE module helper's checked/user versus trusted/engine distinction are preserved.
There is no second WGSL writer, binding map, resource lifetime, or dispatch path.
The native interception device is required, as for the existing development policies.

SPIRV-Tools is an external development executable, selected by `RENDERLAB_SPIRV_OPT`,
then `VULKAN_SDK/Bin/spirv-opt.exe`, then PATH. No Cargo dependency, native library
build, third-party source or binary is imported. SPIRV-Tools uses
[Apache-2.0](https://github.com/KhronosGroup/SPIRV-Tools/blob/main/LICENSE).
The experiment uses installed Vulkan SDK 1.4.363.0 / SPIRV-Tools 2026.4 rc2
`ef96ed76`. Optimizer errors log a diagnostic and compile the original module;
invalid WGSL retains the last working transaction. Optimizer pipes run on the
compile worker and drain output while writing input. Driver optimization remains
enabled for the transformed preview (the rejected DISABLE_OPTIMIZATION policy is
not used).

`--skip-validation` is necessary because naga output already triggers
`VUID-StandaloneSpirv-None-10684`, as recorded in gpu-layer-bench.md. All five
sampled transformations report that same layout rule as their first validator
error; this is not a clean-validation certificate or evidence that no later error
exists.

## CPU preparation screen

Saved 226,852-byte non-RT deferred-shading module from the GPU-layer benchmark;
these are preprocessing times, **not driver compilation or edit latency**. One
sample each on this host, while waiting for the build queue:

| Pass family | Preprocess seconds | Output bytes |
|---|---:|---:|
| SSA rewrite | 0.0685 | 227,588 |
| Local cleanup + SSA + CCP + simplify + dead code | 0.1501 | 210,668 |
| Scalar replacement + local access chains + cleanup + SSA | 0.0692 | 214,488 |
| Dead branches + merge return + inline + cleanup + SSA | 1.2610 | 1,813,648 |
| Full `-O` | 8.5792 | 1,420,696 |

The inlining candidate first failed because merge-return requires unreachable
branches removed first; adding dead-branch elimination fixed that CPU screen.
No GPU conclusion follows from module size or these CPU timings.

## Measurement protocol

Windows, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Rust 1.96.0,
wgpu/naga 30.0.1, Vulkan. Release lib test executable, two Cargo jobs.
`gpu_shc_mixed_reload` uses a 1920x1080 floor/eight-box mixed-program fixture:
RT traced primaries/Interactive shadows or non-RT visibility buffer/Virtual shadows.
GI/TR1 are off at runtime for stable HDR comparison, while the full GI-capable
shader graph compiles. This does not certify defaults-on game/quarry quality.

Every process generates a fresh live tint using the current nanosecond timestamp;
application and NVIDIA private cache behavior must not be inferred from cached
reverts. Save-to-image ends at GPU readback, before PNG encoding, and includes
watch polling. The same executable measures the full-only baseline and preview
variants. The full-upgrade publication time, poll duration, first full frame with
HDR readback, and maximum frame/readback during each compilation are separate
boundaries. Physical scanout is not measured. Preview/full HDR differences and
inspected PNG pairs decide correctness; exact restored and full-versus-synchronous
HDR comparisons remain assertions.

Builds acquire the global stillkit mutex and atomic owner-tagged build slot for
one Cargo command. Builds and GPU leases yield to Ara and the coordinator order,
require no foreign compiler, at least 8 GiB available RAM and 5 GB free on D:.
GPU runs use scripts/n8-run.py, a maximum 14-minute lease and an 8 GiB runtime floor.
All temporary output and the owned target are under D:/Coding/renderlab-work/p-preview.

## Build and CPU verification

Release test binary SHA-256:
`edf235153527c0c2469859f259153825713e6308fe697865bde2432b7ca53f28`.
The guarded release build completed in 216.29 seconds (Cargo reports 216 s),
finishing at 06:45:26 UTC. Default CPU library suite: **200 passed, 0 failed,
164 ignored**, two test threads, 3.50 seconds process / 3.01 seconds framework.
GPU tests remain ignored in that suite. The native preview callback and rendered
behavior are not certified by those CPU results.

## Results

GPU measurements pending. The historical P-SHC2 game numbers (26.18 s RT, 24.08 s non-RT) and
mixed-watch numbers (40.70 / 32.31 s) are context, not this unit's baseline.

## Reproduction

The measurement-only `scripts/p-preview-build.py`, `p-preview-lease.py` and
`p-preview-batch.py` preserve the guarded build, queue and experiment recipes.
The pass-list environment override is experimental scaffolding, not a shipping
quality setting. No quality budget, shading interface or production shader changes.
