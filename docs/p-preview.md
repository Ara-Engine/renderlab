# P-PREVIEW: first-edit preview compilation

Implementation unit, 2026-11-10. Base `caff1005cb14c8ba7465c35e227b90916ec9cd34`.
**DON'T MERGE as a completed optimization.** The opt-in plumbing compiles and the
RT SSA preview is HDR-identical to full, but SSA alone does not clearly shorten
first-edit latency. The three-hour unit reached its first GPU lease only at
08:00:26 UTC. Remaining candidate / non-RT preview gates are incomplete. No default
or cheaper-pass choice is promoted. The branch is an experiment for follow-through.

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
GPU runs use scripts/n8-run.py and a maximum 14-minute lease. The outer wrapper
requires 8 GiB at admission; effective and observed runtime memory are recorded below.
All temporary output and the owned target are under D:/Coding/renderlab-work/p-preview.

## Build and CPU verification

Release test binary SHA-256:
`edf235153527c0c2469859f259153825713e6308fe697865bde2432b7ca53f28`.
The guarded release build completed in 216.29 seconds (Cargo reports 216 s),
finishing at 06:45:26 UTC. Default CPU library suite: **200 passed, 0 failed,
164 ignored**, two test threads, 3.50 seconds process / 3.01 seconds framework.
GPU tests remain ignored in that suite. The native preview callback and rendered
behavior are not certified by those CPU results.

## Fresh-edit results

One process-unique live edit per case, identical executable and fixture, quiet
lease with no foreign compiler admitted. These are readback endpoints, not scanout.
The earlier P-SHC2 game numbers (26.18 s RT, 24.08 s non-RT) and mixed-watch numbers
(40.70 / 32.31 s) are historical context, not the baseline for this unit.

| Endpoint | RT full-only baseline | RT SSA preview | Non-RT full-only baseline | Non-RT preview |
|---|---:|---:|---:|---|
| Save to edited HDR readback, s | 49.625 | 48.989 | 51.567 | Not completed |
| Full pipeline published after save, s | 49.568 | 97.682 | 51.515 | Not completed |
| Full promotion poll, ms | n/a | 0.252 | n/a | Not completed |
| First full frame including HDR/RGBA readback, ms | 55.547 | 56.586 | 53.171 | Not completed |

SSA preprocessing took **51.520 ms** for the edited RT module (206,256 to
207,056 bytes); the driver still took **48.670 s**, against **49.292 s** for the
full-only fresh edit. The roughly 1.3% save-to-readback difference from one pair
is not a clear win, and full optimization then adds another ~49 s of compilation.
Reject SSA alone as the solution. This does not reject the full `-O` design:
its earlier GPU-layer driver result remains valid, but this unit has not finished
its end-to-end gates or established the cheapest successful subset.

RT SSA correctness / swap:

- Complete preview and full HDR are **byte-identical** (zero differing half-float
  channels, maximum absolute difference zero). Full output also equals the
  synchronous full reload; restoration equals original HDR.
- Both 1920x1080 PNGs were inspected: all eight box silhouettes, planar faces,
  colors, ground and sun-shadow edges agree. This fixture contains no glass or
  moving objects; it does not certify defaults-on game/quarry images or motion.
- The same-source full promotion preserves the edited revision; the focused
  assertion passes. TR1 is off here, so no claim of visually verified temporal
  reconstruction continuity follows from that assertion alone.
- Largest old-image frame/readback during preview compilation: **113.371 ms**.
  Largest preview frame/readback during full compilation: **75.520 ms**.
  The **0.252 ms** promotion poll shows no large install stall in this sample;
  these readback timings are not a hitch-free presentation certificate.
- Both full-only path processes pass the existing syntax rejection, last-working
  HDR retention, restore, active-list recipe invalidation and lazy-variant gates.

Completed source hashes:

- RT baseline: `cdfda2c03df4934790e29cb18971eb469190c2a190775695455376983f737739`.
- RT SSA: `20f2f1d1c0ce050bdd5bfa170bec4bc5eb2d254d06c3c1625b811f1df77e5534`.

The local-cleanup + SSA + CCP + simplify/dead-code candidate also failed to
clearly improve latency: **49.458 s** RT, full swap **99.700 s**, promotion poll
**0.313 ms**, first full readback **66.145 ms**. Preview/full HDR is byte-identical;
its two PNGs were inspected and agree in geometry, face colors and shadow edges.
Maximum old / preview frame readbacks are 80.635 / 88.208 ms. Its source hash is
`4de93d8e6cbf819dc82d5cc420cec5dc50d31ea21665aa64ce359e17f3c9010b`.
The non-RT baseline source hash is
`e9e0b67110cae2f24953df7742a49e634fa43de36f5d428f410ea9514dc82da0`.
The non-RT baseline PNG was inspected as well. No non-RT preview image was produced.

## Time, limits and follow-through

Implementation/design time was not separately instrumented; it overlapped the
build and GPU queue waits. Measured Cargo work: **216.29 s**; CPU lib verification:
**3.50 s**; CPU optimizer screens are reported separately above. First build request
was 05:22:33 UTC and build admission was approximately 06:41:50, about **79 minutes
queued**. First GPU request was 06:45:35 and admission was 08:00:26, about **75 minutes
queued**. Latest root orders put Ara first and added P-SHOW ahead of this unit.
The only admitted lease finished at **08:12:56 UTC**, **750.109 s** elapsed. The
runner stopped the partial inlining case with `bounded lease deadline`; there
was no completed inlining, scalar, full `-O`, or non-RT preview sample. Completed
cases remain valid. Resource samples show **12389.9 MiB minimum available RAM** and
**0 maximum foreign rustc processes**. The inherited runner reported effective
4 GiB admission / 2 GiB runtime guards despite the outer 8 GiB check and requested
8 GiB runtime argument; actual sampled RAM stayed above the required 8 GiB.
Do not reuse that inherited guard configuration as proof of an 8 GiB runtime guard.

The measured first baseline process took 170.134 s, the non-RT baseline 120.281 s,
RT SSA 216.678 s and RT local cleanup 219.793 s. Repeated startup and restoration compilation are material
verification costs. A follow-through should run the known-positive full `-O`
transaction on both paths first, then screen cheap subsets using saved SPIR-V or
a fixture that retains canonical pipelines instead of recompiling them for every
case. This unit's fixed pass matrix did not prioritize that known-positive end-to-end
case early enough once the late GPU window became clear.

Still open: a successful both-path latency improvement, matched preview/full
images for that successful candidate, native driver selection of the cheapest
passes, defaults-on game/quarry and motion inspection, and ordinary engine-prelude
edits (still the existing synchronous path). Keep the feature opt-in; do not call
this a completed first-edit optimization.

## Reproduction

The measurement-only `scripts/p-preview-build.py`, `p-preview-lease.py` and
`p-preview-batch.py` preserve the guarded build, queue and experiment recipes.
The pass-list environment override is experimental scaffolding, not a shipping
quality setting. No quality budget, shading interface or production shader changes.

The own GPU job and lease waiter have ended. No new build or lease is requested.

## Cleanup

Automatic approval review rejected the exact-path, containment-checked PowerShell
recursive deletion of `D:/Coding/renderlab-work/p-preview`, stating **blocked by
policy**. No alternate deletion method was attempted. The owned Cargo target and
run output remain there for coordinator cleanup. No owned GPU job, build, waiter
or lock remains; another owner's subsequent lease was left untouched.
