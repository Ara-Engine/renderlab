# M7-LC1: many lights

Overnight unit (2026-11-08 push, [milestone-7.md](milestone-7.md)). Branch `t3/m7-lc1`, from main `984a18b`. Code: [light_cluster.rs](../src/light_cluster.rs) (CPU: selection, matrices, frame layout), [light_cluster_gpu.rs](../src/light_cluster_gpu.rs) (GPU state, kernel dispatch, pooled maps), [light_cluster.wgsl](../shaders/light_cluster.wgsl) (culling kernel), [light_cluster_api.wgsl](../shaders/light_cluster_api.wgsl) (shading side), [light_cluster_tests.rs](../src/light_cluster_tests.rs).

## What it does

1. **Clustered culling, on by default on both paths.** Every lit frame a compute pass bins all lights into the camera's froxel grid: 32 x 18 screen tiles x 24 depth slices (13,824 clusters), logarithmic from the near to the far plane. One thread per cluster tests every light's bounding sphere (range sphere for points, the cone's bounding sphere for spots) against its cluster's view-space box. It writes up to 128 u16 light indices per cluster, in light order. Directional and unlimited-range lights go into every cluster. Cost is O(clusters x lights); the kernel stages lights through workgroup memory in batches of 64.
2. **The light loop walks the receiver's cluster.** The generated raster and traced entries call `rl_lights_prepare(world)` once per receiver, next to `rl_indirect_prepare`. `light_count()` and `light_sample(i, input)` then cover that cluster's lights. The program API's loop is unchanged, so the standard program, toon and user programs get culling without source edits. Falloff is the M5 physical inverse square with a 1 cm regularization and the windowed cutoff `saturate(1-(d/r)^4)^2`. A culled light contributes exactly zero there, so the clustered sum is bit-identical to walking every light (checked on the GPU below).
3. **Shadows for the N most important local lights** (`LocalShadows::count`, default 4, at most 16). Each frame the CPU ranks the point and spot lights whose range meets the view frustum by illuminance at the camera, candela / max(d, 1 m)^2. A held light's score gets a 25 % boost (hysteresis), and held lights keep their slot. The chosen lights' records carry their slot in `kind.w`, and `light_visibility` routes them to `rl_local_shadow`:
   - **RT** (frames whose shading module binds an acceleration structure: traced/VB primaries on RT devices, and raster frames with the hardware hard-shadow head): one inline shadow ray per pixel per shadowed light, toward the light's position, terminating on first hit.
   - **Non-RT and map-based frames** (visibility buffer + Virtual, plain raster, and the RT raster default with the fresh sun, whose fragment layout binds no acceleration structure): pooled maps in a Depth32Float array, six layers per slot. Points and spots wider than 1 rad get cube faces; other spots get one cone view. Maps use reversed depth and are drawn through the existing shadow raster pipeline. Sampling uses a 3x3 bilinear PCF (16 loads), compared on the receiver's own plane, as `hard_shadow` does, with a half-texel slope bias. A slot's faces redraw only when its light moves, the settings change or any caster placement changes (SHA-256 key). Casters are culled to the light's range sphere.
   - Lights beyond N are unshadowed. Their direct light still counts, and GI picks up local lights only through the existing cache path (below).
4. **GI.** The world radiance cache's cell shading (`gi_shade_entry.wgsl`) never prepares a cluster, so it walks the lighting vector as before. Its light pass (`gi_light`) traces one visibility ray per cell to each of the **first 32** lights, which include local lights, on both paths. So local lights add bounce light through the existing hit-lighting path at no new cost, but only the first 32 lights in the vector. Recorded as a gap.

## API

```rust
pub struct LocalShadows { pub count: u32, pub resolution: u32, pub near: f32 } // default 4, 1024, 0.02 m
renderer.set_local_shadows(LocalShadows { .. })?;  // validated; applies from the next lit frame
renderer.local_shadows() -> LocalShadows;
renderer.set_light_clusters(bool);                  // default true; false walks every light
renderer.light_cluster_stats() -> LightClusterStats; // clustered, shadowed slots, maps, faces drawn
Camera::far(self) -> f32;                            // new accessor
```

`Light::Point` / `Light::Spot` are unchanged. `range: Some(r)` is the culling radius and the window's cutoff. `None` (unlimited inverse square) puts the light in every cluster. No ShadowRequest is needed for local shadows: they are automatic and independent of the explicit `Shadows::requests`. The legacy per-light Point/Spot requests still work on the raster map path.

Program API contract (shaders/program_api.wgsl):
- `light_sample(i, input)` is the receiver's i-th light. `LightSample.index` is the light's position in the lighting vector, and `light_visibility` uses it. On stages without a cluster (cell shading, film accumulation), i is the lighting-vector index, as before.
- **Constraint on programmable shading:** `light_count()` now varies per receiver. A raster program must keep implicit-derivative sampling (`textureSample`, `dpdx`) outside the light loop; naga's uniformity analysis rejects it inside, with a program-line diagnostic. Every engine program and the toon example already comply.
- A program addressing lights by a fixed index (`light_sample(0u, ..)`) gets its cluster's first light. Light 0 is still the first entry when it is directional or unlimited-range, which covers the IQ1 diagnostic programs.

Bindings taken (group 3, the lighting group): **23** uniform `LcFrame` (6,256 B: grid camera, projection, counts, 96 pooled-map matrices), **24** `texture_2d<u32>` cluster lists (260 x 3,456 R32Uint, 3.6 MB), **25** `texture_depth_2d_array` pooled local shadow maps. No storage buffers in any shading stage, so the per-stage storage budget, DC1's 12, is unchanged. Bindings 21 and 22 are untouched (DC1 owns 21).
