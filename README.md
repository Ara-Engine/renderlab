# Renderlab

Provisional name for an independent Rust renderer. Public repository: [Ara-Engine/renderlab](https://github.com/Ara-Engine/renderlab). The license remains undecided.

## Run

On the scan-development host, `powershell -File scripts/quarry-demo.ps1` opens the private twelve-scan quarry with the existing optimized viewer, 4x MSAA and interactive camera controls. It does not build or import. See [the demo settings and measured tradeoffs](docs/scan-tuning.md). The source scans and cooked cache are not distributed with this repository.

`cargo run -p renderlab-viewer` opens a persistent 960x640 viewer. The viewer uses the library's owned `Renderer`, with Vulkan/WGSL via wgpu 30 and winit 0.30. Vulkan drivers are required. Windows is the only tested platform.

- Arrow keys: camera orbit/elevation. WASD: translate the camera and its look-at point.
- Space: animate the first cube or prepared asset. R: reset. Escape: exit.
- `--asset PATH` loads a prepared payload. `--samples 4` enables 4x MSAA; the default is one sample. Keys 1–4 select base color, world normal, metallic and roughness views; `--view color|normal|metallic|roughness` sets the initial view.
- `--instances 10000` opens a repeated-cube grid. C toggles conservative culling; N switches between overview and near view. The title reports visible instances.
- Edit `shaders/scene.wgsl` while running. The viewer checks content every 50 ms. Syntax or pipeline-layout errors appear in the terminal/title; the last valid pipeline keeps rendering. Save a correction to recover.
- An optional first argument or `--shader PATH` specifies a different shader path. The default uses the source checkout path, so this is currently a developer tool, not a distributable application.
- `--frames N` or `--seconds N` bounds a run. `--log-frames` prints CPU phases, GPU pass samples, uploads, and presentation-call costs. `--no-timestamps` exercises rendering without GPU timing.
- `--cameras PATH --interactive-camera` starts from the first registered pose and allows the same manual controls; without that flag, registered poses drive the camera.

The fixture contains two indexed cubes and a triangle with perspective projection and depth testing. Mesh buffers persist across frames; camera and model updates upload only changed matrices. The library owns its depth attachment and accepts a caller-owned color texture, independently of winit. Uploads and rendering share a bounded, nonblocking submission gate; callers retry when it returns `None`.

Milestones 0-4 are complete on the recorded Windows/Vulkan host. [Milestone 3](docs/milestone-3.md) records prepared assets, owned textures/materials, diagnostic views and basic MSAA. [Milestone 4](docs/milestone-4.md) adds clustered pages, bounded streaming, prepared first views and [configurable limits](docs/limits.md), with [independent acceptance and recorded limitations](docs/m4-final-verification.md). No Unreal or Nanite superiority is demonstrated. Lighting, shadows, transparency, temporal reconstruction and GI remain future work. Thin features and texture detail can still alias, including with MSAA.

## Prepare an asset

`cargo run -p renderlab-cook -- fixtures measurements/assets` generates two self-authored GLBs. Then `cargo run -p renderlab-cook -- cook measurements/assets/instanced-boxes.glb measurements/cache measurements/current.manifest` prepares one. Pass `measurements/cache/assets/<payload_sha256>` from the JSON output to the viewer's `--asset` option. The viewer reads the payload directly; manifest integrity verification is available through `renderlab-cook inspect measurements/cache measurements/current.manifest`.

The cooker is a separate compilation boundary. Ordinary viewer builds include the dependency-free prepared-format crate, not glTF or image codecs. See [the supported GLB subset](docs/asset-subset.md), [format](docs/asset-format.md) and [dependency notices](THIRD_PARTY_NOTICES.md). Unsupported visual features return errors. Materials currently show unlit base color and diagnostics, not physical lighting.

Normals and single-sided materials require nonsingular affine transforms with a max-element condition estimate at most 10,000. Raw double-sided unlit rendering retains general finite transforms. Source winding is preserved for mirrored placements through fragment-side rejection. Each instance retains an additional 64-byte CPU normal transform and 64-byte GPU normal record; changed transforms upload both model and normal data.

## Verify

`cargo fmt --all --check`

`cargo clippy --workspace --all-targets -- -D warnings`

`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/m3-asset-check.ps1` verifies the prepared format, cooker and generated assets without GPU work.

`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/m3-gpu-check.ps1 -Gpu` runs workspace checks, CPU tests and ten serialized Vulkan GPU tests, recording source/binary identity and full-image evidence. Material tests require the two generated payloads: set `RENDERLAB_M3_ASSETS` to their cache/assets directory on a fresh checkout. Tests cover color/depth ordering, culling, materials/samplers/normals, both assets, 1x/4x AA, reload, persistent uploads and in-flight replacement. Ordinary `cargo test` skips hardware-dependent tests.

The GPU suite requires the Khronos validation layer and fails on HAL errors. The viewer reports diagnostics too; `--validate` makes layer availability and clean diagnostics mandatory at exit. GPU tests and viewer runs compete with other GPU applications, so coordinate them with foreground use.

The recorded final validation run isolated stale overlay registrations with a process-local `VK_IMPLICIT_LAYER_PATH` pointing at an empty directory. See the evidence record for exact environment and reproduction details. Normal runs use the host's Vulkan loader configuration.

See [the Milestone 0 record](docs/milestone-0.md), [Milestone 1 evidence and open gates](docs/milestone-1.md), [brief](docs/brief.md), and [rendering research](docs/research/industry-rendering-costs.md). Fast development builds use two compiler jobs, incremental compilation, and line-table debug info. They are not optimized GPU benchmarks.


Development timing: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-dev-loop.ps1` (add `-Cold` for a fresh target, or `-Asset PATH -Samples 4` for prepared content). It temporarily edits and restores source files; run without concurrent edits in this checkout. `cargo run --release --example asset-cost -- PATH 4 static` records optimized headless prepared-asset costs; use `motion` for camera/object changes. Measurements and captures remain ignored.
