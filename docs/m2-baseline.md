# M2 scene-scale baseline

This is a reproducible pre-optimization baseline, not M2 acceptance. Renderer, shader and runtime APIs remain the accepted M1 implementation. `examples/scene-scale.rs` adds an offscreen fixture using only public library APIs and no new dependencies. Base is `268187d85aac89ec18c9a3445b5b48532ec80ae0` plus the example in this checkpoint.

## Workload and method

One self-authored indexed cube (8 vertices, 12 triangles, 336 uploaded geometry bytes) is reused by 1,000, 10,000 or 100,000 static instances. Centers form a deterministic XY grid at 1.5-unit spacing. The wide camera fits the grid; the narrow camera stays 12 units away so most objects are offscreen at larger counts. M1 still submits every object separately in both cases. There is no application visibility selection, batching, occlusion algorithm or changing geometry in this baseline.

The color target is 960x640 RGBA8 linear, single sample, with the existing depth-tested vertex-color shader. There is no window, presentation, lighting, importer or reconstruction. The draw list is prepared once; its setup time is separate. Accepted-frame CPU wall time encloses the entire `try_render` call, including work outside the internal phase clocks. External polling, rejected-admission CPU cost, harness backpressure sleep and admission interval have separate CSV columns. A 100-microsecond requested sleep only occurs when admission is Busy; its actual elapsed time is recorded. No per-frame GPU completion wait is introduced. The final drain is outside frame measurements.

GPU timestamps cover only the mesh pass, excluding uploads and query readback. Samples are joined by frame ID. Every run requires all expected GPU samples, zero timing failures/dropped observations, and zero geometry, transform and camera uploads after warmup. CSV output happens after timing, so terminal/file writes are outside measured intervals. Statistics discard the first 20 of 60 frames, leaving 40 samples per case; medians average the two central samples and p95 uses nearest rank. These small single-run samples do not establish tail latency.

## Recorded environment and evidence

Windows, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Rust 1.96.0, development profile, two Cargo jobs. This is an unoptimized-build study for the fast development path, not shipping-performance evidence. Timing runs explicitly disable instance debug/validation flags; timestamp queries and the existing wgpu counters feature remain enabled. The independent 100k narrow validation run explicitly enables debug/validation flags and is excluded from the performance table. It completed 25 frames with positive Khronos activation, zero errors and no missing layer.

All runs set a process-local empty `VK_IMPLICIT_LAYER_PATH` to exclude the stale overlay registrations documented in M1. No global settings were changed. No game/viewer process or Ara quiet-window lock was present when work began. The shared desktop was not isolated: pre-run snapshots ranged from 1-37% GPU use and 1,740-1,826 MiB total GPU memory in the final sweep. Snapshots may include residual work from the preceding run and do not identify another process's load. These are not physical per-process memory measurements or a quiet-machine benchmark.

Raw evidence: `measurements/m2-baseline/final/` contains six CSV/log pairs, per-case hardware snapshots, validation CSV/log, source/binary hashes and `summary-dev.json`. The earlier sibling files are pilot runs before a CLI argument-validation change; use `final/` for the checkpoint results. Source SHA-256 for the example is `3F64D5D5EBD140676091D37D0583364D9D1C89D7D42442A871D03637A624758B`; executable SHA-256 is `5FF5B95B568E45C4D61F5C786DD3443E46640A58AB57C67387D622D58F24F42C`.

## Development-build results

All values below are per-frame medians in milliseconds, 40 measured frames each. Whole renderer CPU time includes validation, packing, upload bookkeeping, encoding, submit, internal poll and remaining work inside `try_render`; the displayed phase medians need not add to the whole-call median.

| Instances | Camera | Renderer CPU | Preparation | Encoding | GPU mesh pass |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1,000 | Wide | 3.140 | 0.844 | 2.137 | 0.015360 |
| 1,000 | Narrow | 3.250 | 0.846 | 2.202 | 0.017408 |
| 10,000 | Wide | 30.289 | 8.494 | 21.098 | 0.103424 |
| 10,000 | Narrow | 30.207 | 8.453 | 21.109 | 0.103424 |
| 100,000 | Wide | 329.988 | 91.268 | 231.358 | 2.906112 |
| 100,000 | Narrow | 331.760 | 91.861 | 233.671 | 2.911744 |

Encoding dominates this development-build workload. Static frames satisfy the zero-upload assertions, yet still incur model scanning/packing and one indexed draw per instance. The narrow view has no application culling and therefore still pays that CPU work. This motivates testing batching and visibility; it does not predict optimized speedups or establish a comparison with Unreal/Nanite. GPU clock state and contention were uncontrolled, so small GPU differences between camera cases have no attributed cause.

## Reproduction and limits

```powershell
cargo build --example scene-scale -j2
$env:VK_IMPLICIT_LAYER_PATH = (Resolve-Path measurements/empty-vulkan-implicit).Path
target/debug/examples/scene-scale.exe 100000 25 narrow --validate
target/debug/examples/scene-scale.exe 1000 60 wide
target/debug/examples/scene-scale.exe 10000 60 narrow
target/debug/examples/scene-scale.exe 100000 60 wide
```

Create that empty ignored directory first on a fresh checkout. Run both wide and narrow for each count. The same fixture supports an optimized build with `cargo run --release --example scene-scale -- 100000 120 wide`; no release result is claimed here. Check current GPU ownership/load before reproducing.

Formatting and workspace/all-target Clippy with warnings denied pass. This harness does not capture images or prove culling equivalence, motion, material quality, streaming, or M2 completion. M2 still needs its selected implementation, full-image/reference checks, actual interaction, optimized paired measurements, memory/churn analysis and development-loop verification. Shader reload and renderer source are unchanged by this checkpoint.
