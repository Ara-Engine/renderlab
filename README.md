# Renderlab

Provisional name for an independent Rust renderer. Local only; license and public hosting are undecided.

## Run

`cargo run -p renderlab-viewer` opens a persistent 960x640 viewer. The viewer uses the library's owned `Renderer`, with Vulkan/WGSL via wgpu 30 and winit 0.30. Vulkan drivers are required. Windows is the only tested platform.

- Arrow keys: camera orbit/elevation. A/D: light position.
- Q/E: roughness. Z/X: opacity. Space: animate camera and object. R: reset parameters. Escape: exit.
- Edit `shaders/scene.wgsl` while running. The viewer checks content every 50 ms. Syntax or pipeline-layout errors appear in the terminal/title; the last valid pipeline keeps rendering. Save a correction to recover.
- An optional first argument specifies a different shader path. The default uses the source checkout path, so this is currently a developer tool, not a distributable application.

No GUI toolkit, asset importer, ECS, external shader compiler, or Ara source is included. The analytic scene is a loop-validation fixture, not a mesh-rendering architecture. No GI, reflections, physical glass, or scalable scene system is claimed.

## Verify

`cargo fmt --all --check`

`cargo clippy --workspace --all-targets -- -D warnings`

`cargo test -p renderlab -- --ignored` explicitly runs the Vulkan GPU test, writing full-frame PPM captures into ignored `captures/`. It verifies failed syntax/binding reloads preserve the exact image, recovery works, and each scene parameter changes the image. Ordinary `cargo test` skips this hardware-dependent test.

See [the milestone record](docs/milestone-1.md), [brief](docs/brief.md), and [rendering research](docs/research/industry-rendering-costs.md). Fast development builds use two compiler jobs, incremental compilation, and line-table debug info. They are not optimized GPU benchmarks.
