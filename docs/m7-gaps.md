# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Transparency

TP1 ([m7-tp1.md](m7-tp1.md)). Cutout foliage is load-time geometry; transparent materials draw in a forward pass after reconstruction.

- **Shadows from transparent surfaces.** Glass casts no shadow: it isn't in the TLAS, the clipmap, the surface cache or the radiance cache. Unreal's translucent shadows (Lumen and VSM opacity) aren't matched: no tinted or coverage shadow.
- **Order.** Sorting is per object: one key per draw, from its visible instances' mean bounds centre. Instances inside a draw, and intersecting transparent objects, aren't sorted per pixel, and no OIT fallback exists. Unreal sorts per primitive with optional OIT.
- **Refraction** is screen-space and thin-slab only. Content off screen or behind a nearer occluder falls back to the unshifted pixel. No thick or volumetric refraction, no dispersion, no rough (blurred) refraction: the colour copy has no mip chain. Each transparent draw copies the full HDR target before it draws.
- **Anti-aliasing.** The pass runs after TR1 at one sample per pixel, so transparent edges alias. Unreal composites translucency before TSR, with responsive AA.
- **4x raster frames** skip the pass: there is no single-sample depth to read. 1x raster, traced and visibility-buffer frames run it.
- **Lighting inputs.** Transparent surfaces read the same lights, sky, ES1 lobes and radiance-cache lookups as opaque ones. The cache holds no cells of its own for transparent surfaces, so they read nearby opaque cells or the open sky. Non-RT sun visibility is a 2048² map over the transparent receivers' footprint, not the Virtual clipmap: it's harder-edged than the sun's disk and its texel size scales with the footprint. Other lights' visibility on non-RT transparent surfaces is 1.
- **Programmability.** A program shades transparent surfaces through its own `surface` and `shade`, plus an optional `transmittance` hook. Without the hook, it blends by coverage. Refraction uses the engine's `material_normal`, not a program normal. Streamed (non-resident) textures on transparent materials bind their resident levels without feedback.
- **Cutout foliage.**
  - The cut is static: no alpha animation, no runtime change of cutoff or texture.
  - The contour is linear per grid cell, and the triangle cost is high: the fern card is about 8k triangles at 96 cells.
  - No tangent stream, so cut meshes take no normal map.
  - No alpha-to-coverage softening of the cut edge.
  - Vertices aren't shared between cells (no index reuse).
- **No Unreal or reference comparison** this push (rule 3).

## Particles

TP1 GPU emitters ([m7-tp1.md](m7-tp1.md) §1.3).

- **Lighting.**
  - Each particle carries one irradiance value per step, at its centre: directional lights with the sun map's visibility, plus the environment's six-axis mean irradiance.
  - No point or spot lights, no radiance cache, and no self-shadowing or volumetric shadowing between particles.
  - Particles cast no shadows.
  - The phase function is a fixed isotropic half.
- **Sorting** is per emitter, in one workgroup: at most `MAX_PARTICLES` = 2048 per emitter. Emitters sort against each other and against transparent draws by their spawn centre, so interleaved emitters can order wrongly.
- **Simulation.**
  - No collision, curl noise, or vector fields.
  - Colour, size and emission are linear between two keys, not curves. No flipbook animation, no motion-vector blending, no ribbons or meshes.
  - Spawns per step are capped at the capacity.
- **Rendering.**
  - Billboards render at full resolution after TR1, with no temporal stability or motion blur.
  - Overdraw isn't bounded: no low-resolution particle buffer.
  - Soft fading uses opaque depth only: particles don't fade against glass.
- **No Unreal (Niagara) comparison** this push (rule 3).
