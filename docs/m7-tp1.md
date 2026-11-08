# M7-TP1: transparency, cutout foliage and GPU particles

Unit TP1 of the overnight complete-base push (Islam, 2026-11-08; rules in [AGENTS.md](../AGENTS.md) and [milestone-7.md](milestone-7.md#overnight-complete-base-push-islam-2026-11-08-about-1900)). Branch `t3/m7-tp1`. Labels: **M** measured here, **I** inference.

**On by default on both paths.** Nothing switches TP1 on: content opts in.
- A transparent material, a live emitter or an uploaded cutout mesh draws on every lit 1x frame:
  - RT traced primaries, Interactive shadows;
  - RT raster primaries, Exact shadows: the RT device's code default;
  - non-RT visibility buffer, Virtual shadows: the non-RT default;
  - with TR1, now the default, on traced and visibility-buffer frames.
- A scene without TP1 content is unchanged: **0 differing pixels** on all three paths (M).
- The pass is skipped on 4x raster frames and on TR1b upscaled frames (`RenderScale` below native). Both are in the gaps.

## 0. Results (M)

Evidence is under `D:/Coding/renderlab-work/m7-tp1/`:
- `runs/g6`: merged build `b6`, at main `ae95d3a` with DC1 and TR1b; the verification and timing lease;
- `runs/g7`: build `b7`, with the test pane's ripple toned to window glass; the final images;
- `inspect/`: brightened crops (×2.2) and diffs.

Runs on the RTX 4070 SUPER, NVIDIA 617.14, headless at 1920×1080. These were functional leases, not QUIET windows: other units' compilers ran during them.

| Check | Result | Verdict |
|---|---|---|
| Lib suite inside the lease (`b6`, merged) | 161 passed, 0 failed, 146 ignored | **Pass** |
| TP1 CPU tests | Pass. 5 transparency tests: naga validation of every TP1 module (the standard and toon programs on the map and RT heads, particles, both depth kernels, the shadow map), packing, map fit, pre-TP1 source exactness. 5 cutout tests: area and winding, a disk contour within 1 %, a mirrored uv mapping, saddles, a welded connected mesh | **Pass** |
| Default image unchanged | Opaque quarry with the TP1 standard program against main's pre-TP1 one (TR1 off for the pair): **0 differing pixels** on RT traced, non-RT visibility buffer and RT raster | **Pass** |
| Test scene renders on all three paths | Quarry pose 0 with 18 cutout fern cards (6 bushes), a rippled clear glass pane (IOR 1.5, 2 cm) in front of the island, and fire (192) and smoke (256) emitters. 240 still frames, then a 40-frame pan at 1.5 cm a frame. No faults, no validation errors | **Pass** |
| Inspected images | Section 0.1 | **Pass**, gaps listed |
| Cutout mesh | Fern card at 96 cells: 7,722 triangles, 4,395 welded vertices, cut in 17 ms on the CPU. Receiver adjacency: 0 ambiguous and 0 degenerate half-edges, 1,076 open contour half-edges | **Pass** |
| One timing run | Section 0.2 | Recorded |

### 0.1 Images

Files are in `runs/g7`: `{rt,nonrt,rtraster}-{opaque,foliage,full-f60,full-f240,full-pan40,depth}.png`.

Inspected:
- **Cutout foliage.** Fern silhouettes are exact and smooth at the cut resolution on every path, with no alpha fringe. Crossed cards occlude each other and the island. The images match across the three paths (`inspect/g7-nonrt-raster.png` against `inspect/g7-rt.png`).
- **Glass** (`inspect/g7-glass-zoom.png`, without and with the pane):
  - tinted transmission;
  - Fresnel sky reflection over the dark rock;
  - a few pixels of ripple refraction, which bends the gravel and fern edges behind it.

  Where the refracted ray lands on the black backdrop beside a silhouette, the backdrop shows through. That is correct for refraction, and visible as thin dark edges along the rock and floor rims.
- **Particles.** The smoke is a soft, sun- and sky-lit column with no hard intersections where it meets the cliff: the soft fade works. The flame is emissive at its base. Through the glass, the smoke is tinted, so the pane's edge is a visible seam across the column, as expected.
- **Pan** (`inspect/g7-pan.png`). Stable, with no ghosting on particles or glass: they draw after TR1.
- **Opaque depth** (`*-depth.png`). It matches the image on all three paths: the hit-record reconstruction and the raster unprojection agree. Coverage is 346,517-346,522 pixels; the island floats over the black backdrop.
- **The earlier strong-ripple content** (±8.5°, 5 cm, `runs/g6`, `inspect/g6-raster.png`) shows the refraction model working at large distortion. It reads as patterned bathroom glass.

Bug found by inspection and fixed: the exit refraction used the wrong-facing normal, so no shift was ever accepted (`runs/g4`). The naga test found a name collision with DC1's `material_opacity` after the main merge; TP1's helpers are now `transparent_*`.

### 0.2 Cost

GPU render-span medians: first receiver pass to the end of present, 96 frames each, in two phases of 8 interleaved 16-frame blocks:
- phase 1, with emitters: foliage plus particles, then full;
- phase 2, without emitters: opaque, then foliage.

Particle deltas cross the two phases, so treat them as indicative. Build `b6` (`runs/g6/*-tp1.json`).

| Path | Opaque | + foliage (18 cards, 139k triangles) | + particles (448) | + glass (1 pane) | TP1 total |
|---|---:|---:|---:|---:|---:|
| RT traced, Interactive, TR1 | 1.511 ms | 1.406 (−0.11, noise) | 2.054 (+0.65) | 2.220 (+0.17) | +0.71 |
| Non-RT visibility buffer, Virtual, TR1 | 3.178 | 3.381 (+0.20) | 3.468 (+0.09) | 3.631 (+0.16) | +0.45 |
| RT raster, Exact (RT default) | 16.869 | 17.262 (+0.39) | 17.592 (+0.33) | 18.119 (+0.53) | +1.25 |

What the costs are:
- The particle increment includes the pass's fixed work: the opaque-depth kernel, and the shadow map's key check (the map rendered twice per process: once for the probe, once for the scene).
- The glass increment is the HDR copy plus the pane's fragments.

**Unwelded cutouts were expensive on non-RT.** Before welding (`runs/g4`), the same foliage cost **+6.0 ms** on non-RT (1.16 → 7.16 ms), against +0.08 ms on RT traced. That mesh had per-cell vertices: 16,028 vertices, with 802 ambiguous and 486 degenerate half-edges. Welding brought it to +0.20 ms (I: the visibility buffer's exact receiver repair treats every unshared edge as a silhouette).

No Unreal comparison was run (overnight rule 3).

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

**R1 interface:** R1's traced reflections reach the standard program through `specular_incident`. In the transparent pass's raster composition that input is untraced (w = 0), so glass keeps its sky, lobe and sun specular (gap).

**DC1 interface (root, 2026-11-08):** `fs_transparent` passes `decal_none()`: decals project onto opaque receivers only. DC1 owns `material_opacity(input)`, so TP1's helpers are named `transparent_*`. TP1 adds no lighting-group binding. Its own group 0 uses 0, 2, 30-37, 40-42 and 50-53.
