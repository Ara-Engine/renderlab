# M7-TP1: transparency, cutout foliage and GPU particles

Unit TP1 of the overnight complete-base push (Islam, 2026-11-08; rules in [AGENTS.md](../AGENTS.md) and [milestone-7.md](milestone-7.md#overnight-complete-base-push-islam-2026-11-08-about-1900)). Branch `t3/m7-tp1`. Labels: **M** measured here, **I** inference.

RESULTS_PLACEHOLDER

## 1. What TP1 builds

### 1.1 Alpha-tested (cutout) foliage: cut at load

`renderlab::cutout::cutout(mesh, mask, opacity, cutoff, cells)` cuts a mesh to the contour where `opacity × mask(uv) × vertex alpha ≥ cutoff`, once, on the CPU. The result is ordinary opaque geometry. So every path sees the exact cutout with no new shader stage:
- raster and visibility-buffer primaries;
- traced primaries;
- Virtual and Surface shadows (non-RT), Interactive and Exact shadows (RT);
- world radiance cache rays.

How it works:
- Each source triangle is cut on a uv grid (`cells` per uv unit).
- Marching squares runs on the bilinear alpha field, with saddles decided by the cell centre.
- Each cell polygon is clipped to the triangle in uv (Sutherland-Hodgman), fanned, and its vertices interpolate the triangle's attributes.
- The winding follows the source triangle, including mirrored uv mappings.
- Neighbouring cells share their corner values, so their edges agree exactly.

Why this instead of an alpha test in each kernel:
- One mechanism covers all seven consumers above. An in-kernel test would need uv interpolation and a texture fetch in each: the trace candidate loop, the visibility-buffer raster, the clipmap raster, the surface-cache walks and both GI ray paths. Most of those files belong to the shadow hillclimb.
- RT any-hit or candidate-loop alpha testing is a known traversal cost. Cut geometry stays on the opaque fast path (`RAY_FLAG_FORCE_OPAQUE`).
- Coverage doesn't thin with distance, as alpha-tested mips do.
- Exact surface identity, so TR1 keeps its silhouette and history rules.
- No patent exposure (I): marching squares and polygon clipping are textbook methods.

API: `AlphaMask { width, height, alpha }` (`AlphaMask::from_rgba8` takes a base-colour level), `cutout(...) -> CutoutMesh`, `CutoutMesh::data()` for `try_upload_mesh`. The material stays opaque, usually `double_sided` and `thin`.

### 1.2 Transparent materials: a forward pass after reconstruction

`MaterialDescriptor::transparency: Option<Transparency { opacity, ior, thickness }>`. A transparent draw leaves every opaque path:
- `try_render_view` stably moves it after the opaque draws, and restores the caller's order after the frame;
- so it is in no TLAS, visibility buffer, shadow map, clipmap or radiance-cache table;
- it casts no shadow (gap).

Lit frames with transparent draws or live emitters run the pass. It runs after opaque shading and TR1, before presentation, into the resolved HDR target (`transparency_gpu.rs`, `shaders/transparency.wgsl`):

1. **Opaque view depth** (R32F, metres along the camera axis):
   - traced and visibility-buffer frames rebuild it from the frame's hit records and geometry tables, within the scene rectangle and covered trace tiles;
   - 1x raster frames unproject the raster depth.
2. **Sun shadow map of the transparent receivers**: a 2048² orthographic depth map of every opaque instance, fitted to the union of the transparent draws' and emitters' bounds. It is re-rendered only when the sun, that footprint or the opaque casters (meshes, transforms) change: static scenes render it once.
3. **Particles**: per emitter, one simulation dispatch (spawn, integrate, light) and one bitonic sort in a workgroup.
4. **Back to front by object.** A transparent draw's key is its visible instances' mean bounds centre along the view axis; an emitter's is its spawn centre. Before each transparent draw, the HDR colour so far is copied. The draw then composites over a refracted read of that copy, so glass shows everything behind it, including particles and other glass.

Shading is the material's own program. Its `surface` and `shade` are composed with the pass's fragment entry (`fs_transparent`), against the same lighting group: lights, the environment, ES1 lobes and the radiance cache through `indirect_irradiance`/`indirect_radiance` (open sky while the cache is off). Sun visibility:
- RT renderers: the hardware ray (`hardware_shadows.wgsl`), as raster Exact frames use;
- non-RT renderers: the map from step 2 (3x3 PCF).

The composite:
- a program declaring `fn transmittance(s: Surface, input: SurfaceInput) -> vec3<f32>` gets `shade + transmittance × behind`;
- other programs get coverage blending, `mix(behind, shade, coverage)`, with `coverage = transparent_coverage(material, base alpha × texture alpha × vertex alpha)`.

The standard program declares the hook (marked TP1 edits in `shaders/programs/standard.wgsl`):
- its `Surface` gains `alpha` (exactly 1 for opaque materials, so opaque images are unchanged);
- the diffuse share is scaled by coverage;
- `transmittance = (1 − alpha)(1 − F_schlick(ior))·base`, with the base colour as tint.

Fresnel reflection is the program's own specular, from the sky, the ES1 lobes and the sun.

**Refraction (thin slab):** the view ray refracts into the slab through the shading normal (`material_normal`, so normal maps and rippled vertex normals distort), crosses `thickness`, and leaves through the flat back face. It continues to the opaque depth behind, and the colour copy is read at that point's projection. A read that lands on something nearer than the slab falls back to the unshifted pixel. This is a visual effect on transparent surfaces only, not core lighting (allowed by the brief).

New program API (shared `program_api.wgsl`): `transparent_material`, `transparent_opacity`, `transparent_ior`, `transparent_coverage`. The material's last uniform word packs opacity and IOR as two halves; opaque materials keep zero.

### 1.3 GPU particles

`Renderer::try_create_emitter(&EmitterDescriptor) -> Emitter`. The emitter draws in every lit frame until its handle drops. Also `try_update_emitter`, `set_particle_step` (simulation seconds per lit frame, default 1/60) and `transparency_stats`.

`EmitterDescriptor` fields:
- spawn sphere (`position`, `spawn_radius`) and `spawn_rate`;
- `lifetime` [min, max];
- `velocity`, `velocity_spread`, `acceleration` and `drag`;
- size, colour (rgb albedo, a opacity) and emission, each at birth and death;
- `fade` in and out;
- `soft_distance` for soft particles;
- an optional sprite `texture`;
- `capacity` (ring, up to `MAX_PARTICLES` = 2048) and `seed`.

Simulation, lighting and drawing:
- **Spawning.** The CPU accumulates `rate × step` and sends the spawn count. Spawn k of a step lands in ring slot `(first + k) % capacity`, with a hashed seed per spawn index, so runs are deterministic. Births spread evenly over the step.
- **Lighting**, once per particle per step in the simulation pass: each directional light times the sun map's visibility at the particle, halved for an isotropic scatterer, plus the environment's mean irradiance over six axes. Drawn radiance is albedo/π × that, plus emission over life, exposed.
- **Sort.** Keys are the top 21 bits of the view depth over an 11-bit slot. One 256-thread workgroup runs the bitonic sort in 8 KiB of shared memory. Dead and padding slots sort last and draw degenerate.
- **Draw.** Camera-facing billboards with premultiplied alpha. Fragments behind opaque depth are discarded, and opacity fades over `soft_distance` toward the opaque surface behind.

## 2. Files and ownership

New, owned by TP1: `src/cutout.rs`, `src/transparency.rs`, `src/transparency_gpu.rs`, `src/transparency_tests.rs`, `shaders/transparency.wgsl`, `scripts/m7-tp1-run.py`, this doc.

Shared, minimal and marked `TP1`:
- `src/renderer.rs`: the split/restore wrapper around `render_view`, the unjittered camera, transparent visibility and ID publication, the pass call before presentation, the `transparency` field, and `TEXTURE_BINDING` on the raster depth;
- `src/material.rs`: the `transparency` field, its validation and the packed word;
- `src/lib.rs`: exports;
- `src/program.rs`: `declares_transmittance`;
- `src/hardware_shadow_gpu.rs`: `active_scene()`;
- `shaders/program_api.wgsl`: the material helpers;
- `shaders/programs/standard.wgsl`: `alpha`, the coverage-scaled diffuse and `transmittance`;
- `src/reflection_probe.rs` (R0b's adapter composes the new helpers) and `src/reload_tests.rs` (its invalid-program edit follows the new `return` line).

No shadow, GI, visibility or primary kernel changed.

**DC1 interface (root, 2026-11-08):** DC1 appends `decals: DecalInput` to `SurfaceInput`. `fs_transparent` builds `SurfaceInput` positionally, so it needs `decal_none()` appended when DC1 merges. TP1 adds no lighting-group binding. Its own group 0 uses 0, 2, 30-37, 40-42 and 50-53.
