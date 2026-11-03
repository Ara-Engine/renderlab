# Quarry demo and iteration settings

Run from the checkout containing the private cooked quarry:

```powershell
.\scripts\quarry-demo.ps1
.\scripts\quarry-demo.ps1 -Profile dev -Build
.\scripts\quarry-demo.ps1 -QualityMode full -Pose 4
```

The final default opens the optimized viewer at 1920x1080 with full resident geometry, twelve 4K textures, 4x MSAA, 16x anisotropic filtering and an interactive camera. This scene fits in GPU memory, and the resident path is much cheaper on the CPU. See [the final comparison](scan-results.md). `-QualityMode streamed` selects bounded streaming, `-TextureQuality baseline` uses the original 2K content, and `-Anisotropy 1` restores ordinary filtering. The private detail cache is reached through `measurements/quarry-quality/detail`; `-DetailDirectory` overrides it. Arrows orbit, WASD translates, R resets, and Escape exits. `-Pose 4`, `8`, or `12` starts at the close, grazing, or reverse view. `-CameraMode held` disables manual camera changes; `path` replays the registered poses. `-Progressive` shows a coarse cover during preparation. The default prepares the first detailed view before showing the scene.

Ordinary launches reuse an existing binary and never build or recook. `-Build` builds only the viewer with two Cargo jobs. `-PrintCommand` prepares local JSON and prints the invocation without running it. Asset, manifest, cache, limits, cameras and binary paths are configurable. The live checkout shader is watched. Private content stays outside Git. The launcher isolates implicit Vulkan overlay layers for its child process because stale PRISM registrations on this host broke earlier runs; explicit Khronos validation remains available with `-Validate`. The previous environment value is restored afterward.

## Development profile

The retained dev profile optimizes `renderlab-asset`, `sha2`, `wgpu`, `wgpu-core` and `wgpu-hal` at level 2, Renderlab at level 1, and leaves viewer code at opt-0. Debug assertions, overflow checks and validation are preserved. This is a compilation setting, not an algorithm improvement. The final root opt-1 trial reduced large-scene upload preparation from 8.584 to 3.590 s; a real renderer body edit rebuilt in 3.597 s. The full method and edit-to-image cost are in [scan-results.md](scan-results.md).

Before that final root opt-1 adjustment, on Ryzen 7800X3D / RTX 4070 SUPER / NVIDIA 617.14, the unchanged 338-pose quarry replay at 1080p/4x with active validation gave:

| CPU advance plus drawing, excluding pacing/presentation | Previous opt-0 dependencies | Selected opt-2 dependencies |
| --- | ---: | ---: |
| Motion p50 | 200.75 ms | 41.69 ms |
| Motion p95 | 251.76 ms | 49.30 ms |
| Held-view p50 | 120.85 ms | 32.63 ms |

This reuses one of the three preserved baseline runs and adds one candidate run, rather than repeating the earlier campaign. Both rendered the same aggregate triangle count and retained the same final debt. Recorded host CPU peaks were 72.1% and 57.0%; this is a directional, non-concurrent comparison, not a controlled speedup bound. The dev smooth-motion target is still unmet.

The initial dependency rebuild took 26.73 s. A warm viewer build took 2.54 s Cargo time. A real default-camera body edit took 5.40 s including the pending validation-source change; restoring that body value, with no other source change, took 2.73 s wall / 2.52 s Cargo. No clean build was repeated. Editing the asset crate now incurs its optimization cost, which has not been separately timed. Shader editing requires no Cargo invocation.

Evidence: `.refs/quarry-responsiveness/bin/tuning-dev-b.json`, `measurements/quarry-responsiveness/tuning-dev-b-path/`, the preserved `cr-dev-3-after-dev` baseline, and `.refs/quarry-tuning/edit-latency.json`.

## Responsive streaming policy

The streamed demo mode uses 128 queued/completed read slots and at most 64 page uploads per tick. `-RuntimePolicy baseline` retains the registered 4-read/2-upload policy. `-ReadSlots` and `-UploadsPerTick` configure the responsive policy. Canonical JSON is never changed. The renderer's three-flight admission gate and geometry accounting still apply; 64 configured uploads does not promise 64 admitted uploads.

For the original 2K policy experiment below, geometry, metadata and texture budgets remain 512 MiB each, and the error threshold remains 1 pixel. The final detail preset separately raises texture/bootstrap admission for the measured 4K chains; geometry and metadata remain unchanged. The CPU IO payload bound rises from 327,681 to 8,454,145 bytes. The one IO worker is unchanged. Higher admission allows more detail to become resident and increases both memory use and work; it is not an equal-work algorithm speedup.

| Optimized, validation off, same preserved binary and camera path | Registered policy | Responsive policy |
| --- | ---: | ---: |
| First prepared scene, warm process-start proxy | 16.22 s | 1.42 s |
| Motion CPU p50 / p95 | 13.55 / 14.71 ms | 20.53 / 31.71 ms |
| Final held CPU p50 | 5.18 ms | 2.42 ms |
| Final unresolved detail groups | 1,244 | 0 |
| Peak charged geometry | 150.44 MiB | 491.98 MiB |

One new responsive sample is compared with a preserved baseline. Motion gets more expensive while delivering substantially more detail: roughly 4.0 million final triangles, versus roughly 2.5 million with the original debt. The camera cut at frame 253 first reaches zero debt at frame 261; small new demands recur during continuing movement, and the final hold settles completely. This does not claim uniform 60 Hz movement or an 8 ms CPU target. The same responsive configuration also completed a separate active-validation run, with zero errors/failures/retries and zero final debt. Validation-on CPU p95 was 95.14 ms; do not mix it with the validation-off comparison.

The smaller 8-read/4-upload candidate reduced preparation to 9.67 s but retained 945 unresolved groups. Its validation-on p95 was 47.56 ms. It was not selected for the demo. An earlier invocation with duplicate `--limits` was rejected before GPU work and is preserved as a tooling failure.

Raw evidence: `.refs/quarry-tuning/policy-summary.json`, `fast-summary.json`, `policy-r128-u64-fast.process.json`, and the matching `measurements/quarry-tuning/` directories. These runs have continuing host/GPU observations; background contention was not eliminated. They are finite measurements on this scene, not engine-wide guarantees.

## Native interaction and validation

The native optimized viewer was inspected at `D:/RenderlabEvidence/quarry-interactive-3/`: actual orbit and translation, reset, invalid shader rejection, and correction. The recovered scene matches the initial scene; the only full-client capture difference is a 5x4-pixel Windows rounded-corner decoration. Corrected-shader pipeline compilation took 1.91 ms. The run had active Khronos validation and zero errors. It overlapped CPU texture preparation, so its 2.39 s startup and input proxies are functional evidence, not quiet timing measurements.

The release viewer previously logged validation diagnostics without explicitly requesting instance validation when `--validate` was passed. This is corrected. The shared logger now requires positive layer activation whenever clean validation is requested in either build profile. Earlier interactive run 1 is visual evidence only; run 2 was interrupted by the capture guard detecting another window, and run 3 completed. Resize/minimize and visible-client capture evidence is in [scan-window.md](scan-window.md).
