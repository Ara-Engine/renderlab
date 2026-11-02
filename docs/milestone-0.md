# Milestone 0: development environment

This report was originally numbered Milestone 1. Islam renamed that completed work Milestone 0. Historical measurements are preserved; identity-only history correction changed commit IDs without changing code trees. The measured implementation d3d3f23 now corresponds to 82e9549.

## Decision

Use wgpu 30 with only std/parking_lot/Vulkan/WGSL enabled and winit 0.30 in a separate viewer crate. The library has no windowing dependency. Explicit bind layouts and scoped validation allow transactional shader reload without replacing buffers. Native Vulkan alone would require more unsafe lifetime/synchronization infrastructure; an engine framework would bring unrelated scene/tooling policy. This decision prioritizes a small owned implementation and measurable edit loops, not a claim that wgpu minimizes dependency compilation. Cargo.lock includes other platforms' dependencies, which are not all built on this host.

Upstream API references: [wgpu](https://docs.rs/wgpu/30.0.1/wgpu/), [winit ApplicationHandler](https://docs.rs/winit/0.30.13/winit/application/trait.ApplicationHandler.html). Implementation was checked against installed crate sources. No third-party source or assets were copied; the procedural scene and implementation are original. Dependencies retain upstream licenses through Cargo metadata; this does not choose a project license.

The full-screen procedural scene uses exact ray/sphere intersections, a checker floor, deterministic 2x2 spatial supersampling and 4x4 area-light visibility samples. This requires no asset preparation and avoids introducing mesh import/caching before the viewer loop works. It is intentionally tiny: two spheres and one plane, a single area light, simple diffuse/specular shading and tinted thin-surface compositing. It is not a scalable geometry backend, physically based material reference, or GI prototype.

The research describes culling, shared classification, cached visibility, reduced shading frequency and amortized transport. This fixture exploits none of those savings beyond fixed geometry and persistent GPU resources. It repeats intersection/visibility work per sample. Before growing geometry or lighting, measure those costs and choose representations from evidence; do not extrapolate this scene to engine workloads.

## Validation and limits

Full-image GPU test passes: malformed WGSL and incompatible binding layouts preserve the exact baseline image; valid reload changes it; restoring the source restores identical output. Camera, light, roughness, opacity and object displacement produce distinct full-frame captures. Images are in ignored captures/, raw logs in measurements/.

Inspected baseline and moved-object images and the actual persistent window after a failed live reload. The window retained the previously reloaded blue sphere; its title reported the shader error. A synthetic right-arrow window event changed yaw from 0.35 to 0.43. Other key mappings and resize/minimize require additional interactive coverage; parameter rendering is covered directly by the GPU test.

Known image limitations: fixed light samples show penumbra bands; distant checkerboard aliases; ambient fill is a constant, not indirect lighting; transparency is single thin-surface tint with approximate shadow attenuation, no refraction or internal interfaces. Motion has no temporal history; two displaced frames were inspected, but continuous-motion stability is not certified. Clean sphere outlines are not evidence about imported model silhouettes. No quality gate or optimized GPU timing has been passed.

Reload compiles synchronously on the event thread, briefly pausing it. File polling retries after read failures and compares content (not only timestamps), including atomic replacement saves. An invalid initial shader fails startup; the last-valid guarantee applies after successful startup. Device/surface loss exits with a restart diagnostic. Zero-sized surfaces skip rendering. Platform portability, device-loss recovery and a distributable asset path are deferred.

## Measurement protocol

All timings are development builds, not GPU frame timings. Host: Windows x86_64, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, rustc 1.96.0 (ac68faa20), Vulkan, 960x640 sRGB/FIFO. Two Cargo jobs, incremental enabled, line-table debug information. Default scene and quality above. No exclusive GPU window was taken: no active Ara quiet-window lock existed, but browsers, T3, desktop compositor and NVIDIA overlay were active. A pre-run GPU utilization snapshot was 40%; contention is uncontrolled.

Cold means a fresh Cargo target directory with registry downloads already cached and OS/driver caches warm, not a reboot or cold disk. No-op includes Cargo process overhead. Incremental changes an actual library default and rebuilds/relinks the viewer. Startup runs from main() through first GPU completion and presentation call; it excludes OS process creation. Shader latency starts at the file modification timestamp and includes polling, validation, compilation, GPU completion and the presentation call. Neither measures photons/scanout; these are image-ready/present proxies. External process-start-to-log measurements are upper bounds with polling overhead.

## Results (2026-11-02)

Measured implementation: `d3d3f234d3379f0499c64554a77f8ddb6b95aba1`. Documentation/script updates do not change the renderer. Final warm run: `measurements/20261102-191912/results.json`; clean build: `measurements/clean-build.json` and `.log`.

| Operation | Observed wall time | Samples / interpretation |
|---|---:|---|
| Clean-target dev compile | 23.286 s | One run, registry and OS caches warm; Cargo reported 23.06 s |
| No-op build | 0.334�0.337 s | Three runs; median 0.336 s including Cargo invocation |
| Representative library edit + viewer rebuild | 1.509 s | One run, default camera yaw 0.35 ? 0.36 |
| Rust edit ? running viewer's first-present log | 2.566 s | One run; includes build, process launch and external log polling |
| Startup main() ? GPU complete + present | 624�652 ms | Three runs: 651.857, 624.148, 639.558; median 639.558 |
| Shader file write ? GPU complete + present | 43�81 ms | Three runs: 80.575, 72.126, 43.055; median 72.126 |
| Shader validation/pipeline creation alone | 26�31 ms | Same three reloads: 27.941, 30.833, 26.159 |

Initial exploratory startup was 949.663 ms; initial live color reload was 66.729 ms. Keep these rather than calling the later warm sample universally representative. Initial dependency compilation failed after 37.00 s on API mismatches; the accepted clean-target number above comes from a successful fresh-target rebuild after fixes. A discarded harness used Start-Process -Wait for Cargo and added coarse wait overhead (about one second on no-op); the final script invokes Cargo directly. Raw failed/discarded runs remain ignored, not silently presented as successful samples.

Provisional regression thresholds for this fixture on this host: warm startup under 1 s, shader-to-present under 100 ms, representative Rust rebuild under 2 s. These are evidence-based local checks, not p99 guarantees or production budgets. More repetitions and contention control are needed before tightening them. GPU frame time has deliberately not been inferred from these values or measured in an unoptimized build.

Reproduce with `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-dev-loop.ps1`, optionally `-Cold`. Run while nobody is editing this checkout: the script temporarily changes a library default and shader color, restores exact source bytes in finally, closes its viewer processes, and writes logs to ignored measurements/. The script checks the historical Ara quiet-window lock but does not take an exclusive GPU measurement window.

Verified final contact sheet: `captures/contact-sheet.png` (camera, lighting, roughness, transparency endpoints, displacement and shader color). Actual failed-reload window capture: `captures/live-window.png`. These artifacts are local/ignored; reproduce the GPU captures with the explicit test command in README.
