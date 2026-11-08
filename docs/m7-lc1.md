# M7-LC1: many lights

Overnight unit (2026-11-08 push, [milestone-7.md](milestone-7.md)). Branch `t3/m7-lc1`, from main `984a18b`, with main `f159fcd` (G2b GI default, AT1 atmosphere, DC1, R1, TR1b) merged before the final checks. Code: [light_cluster.rs](../src/light_cluster.rs) (CPU: selection, matrices, frame layout), [light_cluster_gpu.rs](../src/light_cluster_gpu.rs) (GPU state, kernel dispatch, pooled maps), [light_cluster.wgsl](../shaders/light_cluster.wgsl) (culling kernel), [light_cluster_api.wgsl](../shaders/light_cluster_api.wgsl) (shading side), [light_cluster_tests.rs](../src/light_cluster_tests.rs).

**Status: on by default on both paths.** Clustering returns the same image bit for bit (0 differing channels on all three paths) and is faster than walking every light on every path. Local shadows are new: before LC1, no local light could render beside the default soft sun at all. Gaps are listed in [m7-gaps.md](m7-gaps.md#lights).

## What it does

1. **Clustered culling, on by default on both paths.** Every lit frame, a compute pass bins all lights into the camera's froxel grid: 32 x 18 screen tiles x 24 depth slices (13,824 clusters), logarithmic from the near to the far plane.
   - One thread per cluster tests every light's bounding sphere against its cluster's view-space box: the range sphere for points, the cone's bounding sphere for spots.
   - It writes up to 128 u16 light indices per cluster, in light order. Directional and unlimited-range lights go into every cluster.
   - Cost is O(clusters x lights). The kernel stages lights through workgroup memory in batches of 64.
2. **The light loop walks the receiver's cluster.** The generated raster and traced entries call `rl_lights_prepare(world)` once per receiver. `light_count()` and `light_sample(i, input)` then cover that cluster's lights.
   - The program API's loop is unchanged, so the standard program, toon and user programs get culling without source edits.
   - Falloff is M5's physical inverse square, with a 1 cm regularization and the windowed cutoff `saturate(1-(d/r)^4)^2`. A culled light contributes exactly zero, so the clustered sum is bit-identical to walking every light (verified on the GPU below).
3. **Shadows for the N most important local lights** (`LocalShadows::count`, default 4, at most 16).
   - **Choice.** Each frame the CPU ranks the point and spot lights whose range meets the view frustum by illuminance at the camera: candela / max(d, 1 m)^2. A held light's score gets a 25 % boost (hysteresis), and held lights keep their slot. The chosen lights' records carry their slot in `kind.w`, and `light_visibility` routes them to `rl_local_shadow`.
   - **RT.** Frames whose shading module binds an acceleration structure use one inline shadow ray per pixel per shadowed light, toward the light's position, terminating on first hit. That covers traced and visibility-buffer primaries on RT devices, and raster frames with the hardware hard-shadow head.
   - **Non-RT and map-based frames** use pooled maps: visibility buffer + Virtual, plain raster, and the RT raster default with the fresh sun, whose fragment layout binds no acceleration structure.
     - Maps live in a Depth32Float array with six layers per slot. Points, and spots wider than 1 rad, get cube faces; other spots get one cone view.
     - Maps use reversed depth and are drawn through the existing shadow raster pipeline.
     - Sampling is a 3x3 bilinear PCF (16 loads), compared on the receiver's own plane as `hard_shadow` does, with a half-texel slope bias.
     - A slot's faces redraw only when its light moves, the settings change or a caster placement changes (SHA-256 key). Casters are culled to the light's range sphere.
   - Lights beyond N are unshadowed. Their direct light still counts.
4. **The soft sun plus local lights (API1's blocker).** The soft directional disk (Fresh, Interactive and Virtual, the defaults on both paths) used to reject any lighting vector other than exactly one directional light. Every soft path reads `lights[0]` only, and other lights' records carry no shadow views (`kind.z` 0). So LC1 relaxes both checks to "light 0 is the one shadowed directional light; other lights take no shadow request":
   - `validate_shadow_settings` in shadow_gpu.rs;
   - `admit_targets` in fresh_shadow_gpu.rs.

   The sun-plus-260-lights scene renders on all three paths (below).
5. **GI.** Cell shading in the world radiance cache (`gi_shade_entry.wgsl`) never prepares a cluster, so it walks the lighting vector as before. Its light pass (`gi_light`) traces one visibility ray per cell to each of the **first 32** lights, on both paths, and these include local lights. So local lights add bounce light through the existing hit-lighting path at no new cost, but only the first 32 lights in the vector do. Recorded as a gap.

## API

```rust
pub struct LocalShadows { pub count: u32, pub resolution: u32, pub near: f32 } // default 4, 1024, 0.02 m
renderer.set_local_shadows(LocalShadows { .. })?;  // validated; applies from the next lit frame
renderer.local_shadows() -> LocalShadows;
renderer.set_light_clusters(bool);                  // default true; false walks every light
renderer.light_cluster_stats() -> LightClusterStats; // clustered, shadowed slots, maps, faces drawn
Camera::far(self) -> f32;                            // new accessor
```

`Light::Point` and `Light::Spot` are unchanged.
- `range: Some(r)` is both the culling radius and the window's cutoff. `None` (unlimited inverse square) puts the light in every cluster.
- Local shadows need no ShadowRequest: they are automatic and independent of `Shadows::requests`.
- With a soft sun, requests after the first must be `ShadowRequest::None`. The legacy per-light Point/Spot requests still work on the non-default raster-map path.

Program API contract (shaders/program_api.wgsl):
- `light_sample(i, input)` is the receiver's i-th light. `LightSample.index` is the light's position in the lighting vector, and `light_visibility` uses it.
- Stages without a cluster walk the lighting vector in order, so i is the lighting-vector index there, as before. These are cell shading, film accumulation and R1's reflection hits, which shade before the receiver's cluster is prepared.
- **Constraint on programmable shading:** `light_count()` now varies per receiver. A raster program must keep implicit-derivative sampling (`textureSample`, `dpdx`) outside the light loop. naga's uniformity analysis rejects it inside, with a program-line diagnostic. Every engine program and the toon example already comply.
- A program addressing a light by fixed index (`light_sample(0u, ..)`) gets its cluster's first light. Light 0 is still that light when it is directional or unlimited-range, which covers the IQ1 diagnostic programs.
- R1's seams are kept: `light_visibility` still ends in R1's exact `hard_shadow` line, and the traced entry prepares the cluster after `surface`.

**Bindings taken** (group 3, the lighting group):
- **23**: uniform `LcFrame`, 6,256 B (grid camera, projection, counts and 96 pooled-map matrices);
- **24**: `texture_2d<u32>` cluster lists, 260 x 3,456 R32Uint, 3.6 MB;
- **25**: `texture_depth_2d_array` pooled local shadow maps.

No shading stage gets a storage buffer, so DC1's budget of 12 storage buffers per stage is unchanged. DC1's 21 and AT1's 50-53 are untouched.

**Limit raised:** AT1's three sky textures plus LC1's two take the temporal film shading layout to 18 sampled textures per compute stage. `visibility_limits` (and so `traced_limits`) now requests 24 (`max_sampled_textures_per_shader_stage`). Devices below that keep the portable raster path, as before. The raster lit fragment stages need 14-15, within the default 16.

## Test scene and verification

`gpu_lc1_quarry_night` ([light_cluster_tests.rs](../src/light_cluster_tests.rs)): the registered quarry (12 scans, 13 M triangles, G1c's manifest) at 1920x1080, pose 0. The lighting:
- a dim bluish moon (directional, soft 0.04 rad disk, shadowed by the default sun path);
- 256 coloured lights over the quarry floor: points of 3-12 cd and spots of 9-36 cd, ranges 0.8-1.8 m;
- four bright lights: two warm 150 cd spots and two cool 60 cd points.

Exposure is -1 stop. One process per path, all under lease `lc1-h`, with Vulkan validation off. Evidence is in `D:/Coding/renderlab-work/m7-lc1/runs/h`. The binary is a release build of `t3/m7-lc1` after merging main `f159fcd`; it is the pushed code minus formatting-only changes.

| Check | Raster (RT default) | Traced + Interactive (RT) | VB + Virtual (non-RT) |
|---|---|---|---|
| GPU cluster lists vs CPU reference, 13,824 clusters | 0 mismatched | 0 | 0 |
| Overflowing clusters (cap 128) | 0 (longest 99, mean 3.8) | 0 | 0 |
| Clustered vs walking every light, GI and TR1 off | bit-identical (0 channels) | 0 | 0 |
| Shadowed lights chosen | 257, 258 (bright spots), 196, 187 | same | same |
| Local shadow method | maps (4 faces drawn once, then cached) | rays | maps |
| Pixels >10 % darker with the 4 local shadows | 14,432 | 17,124 | 16,503 |
| Soft sun + 260 lights (day -3 stops, dusk) | renders | renders | renders |

Lib suite in the same lease: 161 passed, 0 failed. It includes the CPU tests below.
- `cluster_lists_hold_every_light_that_reaches_a_receiver`: 4,000 random receivers, every light with nonzero attenuation present.
- `shadowed_lights_rank_by_illuminance_and_keep_their_slots`.
- `light_cluster_kernel_and_every_shading_head_validate`: naga validation of the kernel. Also the standard program on the raster, hardware-raster, fresh-raster, traced RT and non-RT heads, each checked for its own ray or map variant.

Inspected images (PNG beside each path's raw run; the raw f32 dumps were deleted after summarizing):
- `<path>-night.png`: the default, with GI, TR1 on traced and VB frames, and atmosphere.
- `<path>-night-noshadow.png`: with LocalShadows count 0.
- `<path>-night-gi-off.png`, `<path>-moon-only.png`, `<path>-sun-day.png`, `<path>-sun-dusk.png`.
- `runs/h/<path>-shadow-diff.png`: red where the local shadows darken by more than 10 %.

Findings from inspection:
- Coloured pools from the 256 lights read across rock and floor on all paths, with no tile or slice seams.
- The local shadows form coherent contact shadows under the angular rock, the slab and the pyramid rock on both paths.
- The raster diff, with no temporal noise, shows no acne bands.
- Dusk shows the sun's soft shadow and the local lights together, consistent between rays (traced) and maps (raster, VB).
- Two diffs are unclassified: fine speckle on the gravel pile on the map paths (pebble shadows, or acne), and one region behind the large rock that rays shadow and maps don't.

## Timing (one run; diagnostic)

Drained frame time (submission to GPU idle, CPU + GPU, median of 24 after 4 warm-ups, min in brackets). Shipped defaults: GI on, TR1 on traced and VB frames, atmosphere on. Release build, RTX 4070 SUPER, contended (other leases queued; wallpaper GPU load not excluded). Not a speed claim.

| ms | Raster (RT default) | Traced + Interactive | VB + Virtual (non-RT) |
|---|---:|---:|---:|
| Moon only (no LC1 lights) | 16.75 (14.9) | 3.90 (3.04) | 4.03 (3.62) |
| 260 lights, clustered, 4 shadowed (default) | 43.06 (40.6) | 8.55 (4.79) | 5.87 (4.84) |
| Same, clusters off (walk all 261) | 123.4 (87.8) | 8.98 (8.20) | 9.26 (8.24) |
| Clustered, no local shadows | 41.47 (39.2) | 5.03 (4.53) | 5.56 (4.86) |
| 4 shadowed lights moving every frame | 48.73 (46.8) | 10.2 (8.70) | 11.55 (10.1) |

Readings:
- Clustering is a large win on every path: walking all lights costs 2.9x (raster) and 1.6x (non-RT) more.
- On the deferred paths (traced, VB), 256 lights plus 4 local shadows cost about 1.2-1.8 ms by minimums.
- Static shadowed lights are cached on the map paths. Moving them redraws 4 full-detail quarry faces per frame: +5.7 ms non-RT, +5.7 ms raster.
- The **raster RT default** pays the light loop per rasterized fragment, so 13 M scan triangles' overdraw and quad overshading multiply it: +24.7 ms. This is the main cost gap; see the gap list.

Development time: about 2 h implementation, about 1.2 h verification, including six lease attempts. Four lease runs failed on test or harness issues:
- a reserved WGSL word;
- an over-broad string assertion;
- the single-light soft-sun checks, which needed this unit's fix;
- the sampled-texture limit after AT1.

Each was fixed before the final run.
