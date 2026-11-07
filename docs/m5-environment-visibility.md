# M5-EV: environment visibility (folded into M7, not implemented)

2026-11-07, base main `6b6c84a`. This unit stopped during grounding: Islam authorized [M7](milestone-7.md), and its GI work replaces the mechanism. No code, fixture, path-traced reference, bench, Unreal capture, GPU lease or build was produced. This note hands the grounding to M7's design step. Labels: **M** is measured in the cited record, **A** is author-reported, **I** is an inference from reading only.

## Current state

- The standard program adds the prepared diffuse irradiance and prefiltered specular radiance with no occlusion (`shaders/programs/standard.wgsl:27-33`). The constant-environment term on line 26 is unoccluded too. Concave scan regions and undersides therefore get the full sky.
- The program API (`shaders/program_api.wgsl:78-95`) offers `ambient(n)`, `environment_irradiance(n)` and `environment_reflection(direction, roughness)`, with no visibility input. The precedent for one is `light_visibility` (line 72): the engine computes it at its own receiver, so it is identical for every program.

## Unreal comparator

- In the registered matched quarry setup, Unreal's sky is an unoccluded movable sky light with a specified cubemap, and GI, AO, SSR and fog are all off ([full-detail comparison](unreal-quarry-full-detail-comparison.md), "Full-frame environment" row). Lumen is off. Both engines therefore render the environment unoccluded today, and that matched subset has no Unreal environment-visibility term to compare against (M, as recorded).
- For an M5-only subset, the candidate comparator was distance-field AO on the movable sky light. It is the non-RT option, and it intersects a bent-normal cone for specular occlusion. Nothing about it was verified on the installed 5.7.4 build: no CVar readback, no mesh-distance-field build for the 13M-triangle Nanite meshes, and no capture (I).
- M7 names Lumen software and hardware GI and Lumen reflections as its comparators. Lumen's sky visibility sits inside its final gather and reflections, so separating it from bounce needs a configuration study. Whether 5.7.4 still has standalone RT AO or RT sky-light CVars was not checked. Read the installed engine's CVar list for configuration only, and copy no source (I).
- **Unreal cost: not measured.** The only Unreal denominators are the existing ones, all without environment occlusion. Packaged static GPU medians are 1.61-1.64 ms for VSM and 1.77-2.16 ms for HRT (M, same record).

## Candidate designs (none built)

- **RT path.** Short hemisphere rays against the existing hardware acceleration structure, which holds all 13,034,903 triangles, with accumulation and denoising per receiver or in world space. The only published cost is from tray_racing: 0.51 ms for one primary plus one unbounded closest-hit AO ray per pixel at 1080p, Vulkan hardware RT on an RTX 4070 Ti (A, [reuse-non-rt.md](research/reuse-non-rt.md) section 3.1). Short any-hit rays should cost less (I).
- **Non-RT path.**
  - Software BVH (N0) is exact, but 12-22x slower than hardware: 8 coherent sun rays cost 9.43 ms (M, [non-rt-path-2.md](non-rt-path-2.md)). Hemisphere rays are incoherent, so they would sit at the expensive end (I).
  - The light-space caster grid and the virtual clipmap are built for one directional light. A set of fixed sky directions, each with its own grid, would turn the integral into a directional-sample approximation. Untested (I).
  - Visibility precomputed at preparation is the cheapest runtime candidate, because the scans are static: a bent normal and visibility cone per vertex or per cluster. It needs a separate term for moving casters (S2-style motion). Its resolution must hold at millimetre contacts. For sun rays on the quarry, 39% of blocked rays are blocked within 1 mm of the receiver (M, non-rt-path-2 section 1). The equivalent distribution for sky rays was not measured.
  - SDF and voxel fields: the research survey rates Brixelizer as far-field only ("loses small-scale details"), and finds that Godot SDFGI lacks dynamic occluders (A, reuse-non-rt.md L4 and L5). Neither is suitable as the near-field core.
  - The surface-keyed cache (N8n, N8o) is the closest existing world-space cache, and M7 names it. If visibility is cached per surface cell, its lessons apply: consensus-certified cells, and flushes on rebase (I).
- Screen-space AO is excluded by AGENTS.md.

## Programmability note for M7

Compute visibility in the engine, at the engine receiver, as `light_visibility` is computed. The standard program uses it, and custom programs may use it or ignore it. Once GI exists, a separate visibility value may be redundant. The planned `indirect_diffuse` and `indirect_specular` services ([programmable-shading.md](programmable-shading.md) section 2.1) could return occluded environment plus bounce as incident radiance. An API that only returns a bent normal and cone would bake one occlusion model into the program interface, so prefer returning incident quantities (I).

## Not done

- A fixture with controllable concavities.
- A path-traced reference of the environment term on the quarry or the fixture.
- An Unreal DFAO or Lumen capture, and its cost.
- An implementation on either path, with per-stage costs, memory against Unreal, and static or motion quality.

No GPU lock or schedule reservation was taken, no Cargo target was created, and nothing was written to `E:/renderlab-work`.
