# Renderlab

Provisional name for an independent Rust renderer. Public repository: [Ara-Engine/renderlab](https://github.com/Ara-Engine/renderlab). The license remains undecided.

## Run

`cargo run -p renderlab-viewer` opens a persistent 960x640 viewer. The viewer uses the library's owned `Renderer`, with Vulkan/WGSL via wgpu 30 and winit 0.30. Vulkan drivers are required. Windows is the only tested platform.

- Arrow keys: camera orbit/elevation. WASD: translate the camera and its look-at point.
- Space: animate the first cube. R: reset. Escape: exit.
- Edit `shaders/scene.wgsl` while running. The viewer checks content every 50 ms. Syntax or pipeline-layout errors appear in the terminal/title; the last valid pipeline keeps rendering. Save a correction to recover.
- An optional first argument or `--shader PATH` specifies a different shader path. The default uses the source checkout path, so this is currently a developer tool, not a distributable application.
- `--frames N` or `--seconds N` bounds a run. `--log-frames` prints CPU phases, GPU pass samples, uploads, and presentation-call costs. `--no-timestamps` exercises rendering without GPU timing.

The fixture contains two indexed cubes and a triangle with perspective projection and depth testing. Mesh buffers persist across frames; camera and model updates upload only changed matrices. The library owns its depth attachment and accepts a caller-owned color texture, independently of winit. Uploads and rendering share a bounded, nonblocking submission gate; callers retry when it returns `None`.

Milestone 1 is implemented but acceptance is still in progress. No GUI toolkit, asset importer, ECS, external shader compiler, or Ara source is included. Lighting, shadows, transparency, antialiasing, GI, and scalable scene systems remain outside this foundation milestone.

## Verify

`cargo fmt --all --check`

`cargo clippy --workspace --all-targets -- -D warnings`

`cargo test -p renderlab -- --ignored --test-threads=1` explicitly runs the three Vulkan GPU tests, writing full-frame PPM captures into ignored `captures/m1/`. They cover depth, reload recovery, persistent uploads, asynchronous timing, faults, and replacement under in-flight work. Ordinary `cargo test` runs CPU validation and skips these hardware-dependent tests.

`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/m1-check.ps1` records formatting, workspace checks, Clippy, and CPU tests. Add `-Gpu` for the GPU suite. GPU tests and viewer runs compete with other GPU applications; current project guidance defers them during Islam's gaming session.

See [the Milestone 0 record](docs/milestone-0.md), [Milestone 1 evidence and open gates](docs/milestone-1.md), [brief](docs/brief.md), and [rendering research](docs/research/industry-rendering-costs.md). Fast development builds use two compiler jobs, incremental compilation, and line-table debug info. They are not optimized GPU benchmarks.


Development timing: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-dev-loop.ps1` (add `-Cold` for a fresh target). It temporarily edits and restores source files; run without concurrent edits in this checkout. Measurements and captures remain ignored.
