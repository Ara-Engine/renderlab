# Renderlab 0.1.0

The first complete renderer base, from 2026-11-09. Every feature listed below is on by default on both paths:
- **RT:** hardware ray tracing.
- **Non-RT:** no ray tracing hardware needed.

This release ships source only, with no binaries. The private quarry scan isn't distributed and isn't needed. The license is still undecided.

## Use

- **Engine API.** `renderlab::engine` is frozen except for the additions listed in [api.md](api.md): handles, meshes, materials, programs, lights, sun and sky, decals, particles, skinned meshes, settings, and render and capture. Lower-level control stays on `Engine::renderer_mut()`.
- **Example.** `cargo run --example game_scene -- --path rt|nonrt` renders a dusk courtyard with every feature on.
  - The walking Fox is optional. Fetch it with `scripts/m7-sk1-asset.py`; credit is in `third-party/Fox.md`.
  - Pipelines are cached under `measurements/pipeline-cache` (`--no-pipeline-cache` disables the cache).
- **Platform.** Tested on Windows with Vulkan and an NVIDIA RTX 4070 SUPER. RT needs Vulkan ray queries; non-RT doesn't.

## On by default

| Area | What ships |
|---|---|
| Anti-aliasing and upscaling | TR1 temporal reconstruction, with quality (2/3) and performance (1/2) presets. History follows moving and skinned objects through per-object motion vectors, and reflections stay sharp while the view turns. |
| Global illumination | Dynamic world radiance cache with sky occlusion and bounce, on hardware rays or a software BVH. A cell filter and crevice refinement remove mottle and pinholes. Every light reaches GI. |
| Reflections | Traced mirror and glossy reflections on both paths: hardware rays on RT, software rays on non-RT. Glossy surfaces trace up to 16 rays by roughness. `ReflectionSettings` controls them. |
| Shadows | RT sun shadows with an adaptive ray count. On non-RT, a virtual and surface-cached sun. |
| Local lights | Clustered point and spot lights. Soft local shadows come from `source_radius`. RT shadows every contributing light; non-RT shadows up to 128 from a cached atlas, fading as lights enter and leave it. |
| Transparency | Forward glass with refraction tint, coloured sun shadows and RT reflections. Cutout foliage. GPU particles. |
| Atmosphere | Physical sky, aerial perspective, height fog and volumetric fog with light shafts on both paths. Time of day without a GI flush. |
| Characters | GPU skinning from glTF (RLS cook), with RT BLAS refit and GI invalidation for moving casters. |
| Materials | Decals. Per-object shading programs, up to 32 per frame; user programs stay fully programmable. |
| Post | Bloom, motion blur, tone mapping and grading, colour LUT, vignette. DOF is opt-in. |
| Startup | Persistent pipeline cache: warm startup is about 0.5-0.6 s. |

## Known gaps

The full list, per feature, is in [m7-gaps.md](m7-gaps.md). The largest:

- **Speed against Ara** (measured in [m7-ara1-report.md](m7-ara1-report.md), 1080p):
  - Non-RT is 2.2-5.8x slower than Ara's raster path.
  - RT is slower than Ara hybrid on small scenes.
  - First-cut frames take 12-31 ms.
  - 64 lights take 12 ms on RT and 28 ms on non-RT.
  - GPU memory is about twice Ara's.

  Image quality is better than Ara's on every ranked aspect. Milestone 8 ([milestone-8.md](milestone-8.md)) targets the speed gap with no quality loss.
- **Against Unreal:** GI costs about 3-5x the 2x-Lumen target. Overall Unreal-quality parity at 2x speed isn't demonstrated.
- **Development loop:** the first edit to a new shader stalls for about 23 s, and cold startup takes about 21 s.
- **Not traced on non-RT reflections:** meshes over 2^19 triangles, streamed meshes and skinned meshes.
- **Diagnostics:** Vulkan validation reports inherited SPIR-V layout diagnostics (VUID-StandaloneSpirv-None-10684).
