# M7-TP2: glass lighting, sun transmission and edge coverage

Follow-up to [TP1](m7-tp1.md), branch `t3/m7-tp2`, under the 2026-11-08 overnight rules. Implementation and verification are limited to this checkpoint, with remaining gaps recorded below. No Unreal or path-traced campaign is part of this unit.

## Evidence and cost

Verification is in progress. Evidence lives in `D:/Coding/renderlab-work/m7-tp2/`. This section will be replaced with the inspected results and measured costs before delivery.

The profiler loads the quarry once, keeps the foliage and emitters, and switches only the transparent shader. It compares the full frame with foliage plus particles in eight interleaved 48-frame blocks, retaining the last 16 frames of each block (128 frames per configuration). Its five variants isolate the radiance-cache lookup, hardware sun visibility and atmosphere. These diagnostic variants disable TP2 reflections, coloured shadows and coverage AA, so those features cannot contaminate attribution. All test binaries and GPU work run inside `scripts/n8-run.py` and `D:/Coding/renderlab-work/gpu-slot.lock`.

## Implementation

### Sun transmission on both paths

`src/transparency_shadow.rs` owns an independently cached 1024-square RGBA16F map, with one layer per transparent draw. RGB stores transmitted sun light and alpha stores light-space depth. Each layer renders the nearest transparent sheet, including off-camera instances. A receiver multiplies layers lying between it and the first directional sun, with depth-aware bilinear filtering. Coverage includes material opacity, texture alpha and vertex alpha; tint and Schlick Fresnel filter what remains. Opaque visibility stays on its existing RT or non-RT implementation.

The shared lighting group binds the 96-byte header at 26 and the map at 27. `light_sample` filters sun RGB before the program evaluates its own BRDF; existing programs receive coloured shadows without a new mandatory hook. Unshadowed lights keep their requested behaviour. A changed transparent caster restarts reconstruction and relights GI through the existing lighting-event key. The map rerenders only when its source, sun, geometry, material identities or transforms change.

### R1 reflections on RT glass

`src/transparency_reflection.rs` adapts R1 instead of duplicating its hit shader. A small raster pass writes the glass program's declared `reflection_lobe`, world position and independent geometric plane. A compute pass runs R1's ray loop on the frame's opaque geometry/material tables and shades reflected hits with the opaque frame's program. The final transparent program reads that exposed incident radiance through `specular_incident`, retaining control over its reflection response and transmittance. Ray offsets and geometric-side rejection remain independent of the program's shading normal.

Mirror lobes use one ray; glossy lobes use R1's four fixed VNDF strata and roughness cutoff. Glass draws after TR1, so glossy strata stay fixed instead of rotating without a history filter. Reflected hits use open-environment indirect inputs, as in R1. The adapter follows the actual primary-frame buffer size rather than freezing that ABI. Non-RT currently uses the environment fallback.

### Transparent edge antialiasing

The glass composite draws with four geometric coverage samples. A fullscreen seed copies the exact background into the multisampled attachment; uncovered samples preserve that background, then the attachment resolves into HDR. Shading remains per pixel. This smooths geometric pane edges without temporal history or glass trails. Particles retain their existing pass.

### Shared hooks

The owned files are `transparency_gpu.rs`, `transparency_shadow.rs`, `transparency_reflection.rs`, `transparency_tests.rs`, `transparency.wgsl`, `transparency_shadow.wgsl` and `scripts/m7-tp2-run.py`. Minimal shared hooks, marked TP2, are in `renderer.rs`, `lighting_gpu.rs`, `program.rs`, `program_api.wgsl`, `gi_cache.rs` and `primary_ray_gpu.rs`. The isolated reference probe gets a no-transparent-caster stub. A duplicated R1 ray-query declaration inherited from the PP1/SK1 merge was repaired; subsequent main merges retain exactly one function-scope declaration.

These are ordinary light-space maps, coverage MSAA and R1's existing per-receiver rays. No reservoir, resampling, spatially shared reflection estimate or new patented GI combination is introduced.

## Remaining gaps

- A shadow-map layer stores the nearest sheet of one draw. Overlapping instances or several sheets in the same draw need separate layers to multiply correctly. The map does not reproduce volumetric absorption, caustics or finite-sun soft penumbras.
- Transmission shadows cover the first directional sun. Local lights, additional directional lights, environment visibility and GI traversal still omit transparent intersections. The map uses the engine material's tint/coverage/IOR, not an arbitrary program's `transmittance` implementation.
- Reflections see opaque geometry; transparent objects are absent from the TLAS. No recursive glass, multiple glass interfaces or rough-glass history/denoiser is provided. RT raster-primary frames and non-RT retain the environment fallback at this checkpoint.
- Coverage AA addresses geometry edges. Texture/shading aliasing, particle AA and transparent depth intersections still need broader treatment. The extra HDR seed/resolve and attachments have a cost.
- TP1's 4x raster and sub-native upscaling omissions, per-object sorting, thin screen-space refraction and full HDR copy per transparent draw remain.
- No claim of Unreal parity or superiority is established by this unit.
