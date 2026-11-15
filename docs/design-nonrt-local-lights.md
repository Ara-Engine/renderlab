# Non-RT local lights: keeping many shadowed lights cheap without hardware ray tracing

Reading-only study, 2026-11-11, on main `0c1c03f` (patent policy suspended, `AGENTS.md`). No build, no GPU, no new measurement. It starts from tonight's figure (RTX 4070 SUPER, 1080p, `examples/game_scene.rs`: 21 ms per moving frame on the ray-traced path, 85 ms on the non-RT path, steady at 83-88 ms) and is written so it stays useful whichever pass the in-flight attribution blames.

Labels: **C** read from code (file and line on `0c1c03f`); **M** measured in a repo document (named); **S** reported by a source (section 8); **E** this study's estimate or proposed parameter. Every forecast is **E**, not a measurement.

**Result.**
1. Before choosing, check two facts about the input (section 0): the checked-in example creates 3 local lights (27 with `--night-lanterns`), not 128, and volumetric fog does not read local lights at all and runs on both paths, so it cannot account for a 64 ms difference between the paths.
2. Renderlab's cost in local lights has five separate terms that scale differently with the light count N (section 1). Each lands in a different pass-log stage, so one run matrix (section 4, step 0) splits them. I rank what to screen as: (1) a static/dynamic split of the map cache with demand-driven faces; (2) a world-space light list for the stages that still walk every light (reflection hits, GI cells); (3) a penumbra-adaptive filter; (4) only the *premise* of virtual local-light pages. Stochastic light sampling is not proposed (section 5). If step 0 puts the time elsewhere, section 4 gives the reorder.
3. Shipping engines keep many shadowed lights cheap by *not redrawing and not sampling*: caches with a static/dynamic split, demand-driven pages, and per-pixel light lists. None gets there by making each shadow lookup cheaper. Renderlab has the lists (LC1) and a whole-light cache (LC3) but neither the split nor the demand.

## 0. Checking the input

**The scene has 3 local lights by default, 27 with `--night-lanterns` (C).** `examples/game_scene.rs:650-692`: an arch point light (range 14 m), a post spot (12 m) and the fire's point light (10 m, flickering intensity), plus 24 lanterns (range 4 m) only under `--night-lanterns` (`:685-692`). "128" is the map budget `LocalShadows::default().count` (`:309-313`, `docs/m7-lc3.md`), not a light count. If tonight's scene was modified to 128 real lights, say so in the measurement record, because every term below scales with the number of lights, not the budget.

**Moving frames are always moving frames here (C).** The spinning crate (`:760-763`) and the hero (the Fox with `--fox`, else two toon spheres) move every frame, so any light whose range sphere touches one of them redraws its whole map (section 1, T1). Ranges of 10-14 m cover the whole courtyard.

**Fog is common to both paths and independent of N (C).** `at_fog_inject` (`shaders/atmosphere.wgsl:230-256`) takes `at_sun_visible(world)` per sample and nothing from the lighting vector (the host passes only the first directional light, `src/atmosphere_gpu.rs:408-416, 709-716`); its grid is 120 x 68 x 64 = 522,240 froxels at 1080p (`docs/m7-vf2.md`, M) with two samples each (four after a history reset). RT reads one TLAS ray per froxel, non-RT one clipmap height test (`docs/m7-vf1.md`, M). So fog costs about the same or less on non-RT, and a 64 ms path difference lives in work that differs between the paths: local shadows (rays against maps), reflections (hardware against software), GI rays, the sun's shadow (ray against Virtual clipmap), primary visibility. Fog would only matter if non-RT's lookup stalls on page residency, which VF1 designed against (it "makes no page requests", M).

## 1. What Renderlab does today

Five terms, with the pass-log stage that contains each (`RENDERLAB_PASS_LOG`, `src/pass_timeline.rs`; stage marks at `src/renderer.rs:3209-3485`). A stage lasts from its mark to the next, so overlapping GPU work shares a boundary.

| Term | What scales | Where (C) | Stage | Measured elsewhere (M) |
|---|---|---|---|---|
| **T1 Map redraw** | faces redrawn per moving frame = sum over lights with a changed caster in range of their faces (6 per point, 1 per spot); casters per face | `light_cluster_gpu.rs:525-612` (key per slot hashes every caster inside the light's sphere, `:564`; any change redraws all faces of the slot, `:584-612`), `:704-749` (one render pass per face, a clear draw, then one `draw_indexed` per caster instance, full mesh, no LOD, no occlusion test beyond the face's clip volume) | `light_clusters` | four bright lights moving: non-RT 22.75 -> 45.62 ms on the quarry, +22.9 ms (`docs/m7-lc3.md:76-78`) |
| **T2 Shading lookups** | pixels x lights in the pixel's cluster x taps | `shaders/light_cluster_api.wgsl:65-130`: with `source_radius > 0` (all example lights), 12 blocker taps and, only if one blocks, 24 x 4 bilinear taps (96 loads); a fully lit receiver exits after 12 (`:107-114`); hard lights use one 4x4 PCF | `shading` | `rl_local_tap` is 58.0% of sampled states in the shadowed non-RT trace; 34.98 ms frame / 30.24 ms shading with local shadows against 11.23 / 6.44 ms without (`docs/p-lts.md:61-67`) |
| **T3 Reflection hits** | glossy pixels x strata (4-16) x lights at the hit | stages with no cluster walk every light: R1 hits, GI cell shading, film accumulation (`light_cluster_api.wgsl:8-14, 33-40`; `docs/m7-gaps.md:499`); hit shadow skips lights without shadow views | `shading` | not measured per light |
| **T4 GI cell lighting** | queued cells x lights reaching the cell; moved casters rescan all N lights per cell | `shaders/gi_cache.wgsl:1574-1590` (`gi_light_reached`: moved boxes x all lights), `:1605-1630` (`gi_light`: one visibility ray per light that reaches the cell) | `shading` (inside the frame's GI work) | `docs/m7-gaps.md:498`: "bounded by queued cells times light count, without a separate ray budget" |
| **T5 Cluster cull, fog** | clusters x lights; froxels | `light_cluster.wgsl`; 32 x 18 x 24 = 13,824 clusters, 128 lights per cluster (`light_cluster.rs:18-23`) | `light_clusters`, `atmosphere` | cull 0.0098 ms (`docs/p-lts.md:56, 63`); fog independent of N (section 0) |

**What exists and works.** The cluster lists give the standard program a per-pixel list (mean 3.8, longest 99 on the 261-light quarry, `docs/m7-lc1.md:76`), exact zero-attenuation rejection is in (`docs/p-lts.md`), the cache key avoids invalidating lights whose range a moved caster does not reach (`light_cluster_gpu.rs:564`), the pool fades slot changes by 1/8 per frame (`docs/m7-lc3.md`), and the atlas is bounded at 384 MiB for 128 lights (4096 x 4096 x 6 x 4 B).

**What does not exist.** A static/dynamic split of casters, per-face demand (all faces of a point light are drawn even when visible receivers sample two), a receiver-driven caster cull, and any light list for stages that shade away from the camera's clusters. `docs/m7-gaps.md:496`: "no virtual page cache or static/dynamic-caster split". The sun's non-RT shadow already has page-driven rasterization with moved-caster invalidation by light-space bounds (`src/virtual_shadow_gpu.rs:1-17`), but local lights do not use it.

**The measurements Renderlab already has do not separate T1-T4 for this scene.** P-LTS profiled ARA1's game scene with 64 shadowed lights (+27 ms of deferred shading, `docs/m7-gaps.md:165`); I did not check how many of that scene's casters move. LC3's moving-light run is the quarry. The courtyard with 27 lanterns measured 36.7 ms non-RT against 13.2 ms RT still, in a throughput probe that includes readback (`docs/p-lts.md`, the "night-courtyard scoreboard"), so 85 ms is within reach of the same terms at a larger N or with motion (E), not evidence of a new class of cost.

## 2. How shipping engines keep many shadowed local lights cheap

### 2.1 Virtual shadow maps for local lights (Unreal; Olsson et al.)

**How.** Unreal gives a spot light one 16k x 16k virtual map with a mip chain and a point light a cube of six, in 128 x 128 pages backed by one physical pool; pages are allocated only for screen pixels that need them, chosen by projecting pixel size into shadow space (S1). Olsson et al. reach the same end with hardware sparse cube maps: clusters of visible samples fix each light's resolution and the pages to commit, and the same clusters cull casters per face, "at least six times more efficient" than light-sphere culling into all six faces (S2). Memory stays proportional to shaded samples, and they report 23-34 ms at 1080p for 256-356 lights on a GTX Titan (S2).

**Cost.** Unreal states projection cost depends on shadow-map samples per screen (average lights per pixel, rays per pixel, samples per ray), not on page counts or caching, and calls many-local-light performance "still a work in progress" (S1). Olsson et al. find 9-tap PCF "a quite high proportion of the total cost, especially in the scenes with relatively many lights affecting each sample" (S2), which agrees with Renderlab's own profile (T2).

**Where it fails (S1).** Only the first depth layer is stored, so overlapping occluders leak; very large local lights and shallow angles run out of virtual resolution ("box-like shadows"); exhausting the page pool corrupts shadows; Subsurface Profile and transmission shade as opaque; non-Nanite casters are far costlier. Sparse-texture commit was too slow on the drivers of the time, so S2 measured with a pool of fixed-size maps instead.

**Fit with user-programmable shading.** The lookup is `light_visibility`'s inside; a program does not see it. Page requests need the receiver positions, which the visibility buffer already provides (the sun's Virtual clipmap does exactly this, `virtual_shadow_gpu.rs:1-17`). Cost to adopt: days, not hours; it reuses N8's page machinery but not its single-direction clipmap layout.

### 2.2 One-pass projection and per-pixel light caps (Unreal)

**How.** Unreal's guidance for many local lights is to enable one-pass projection and keep pages cached (S1). Its mechanism, as I could confirm it only through a search summary of Epic's Fortnite VSM article (the page returned 403), projects all local shadows in one pass into a temporary buffer instead of one mask per light, with a console variable bounding the lights per pixel; too low a bound drops shadows from lights that still contribute (S11, **unverified**).

**Cost / failure.** A bound trades exactness for time: missing shadows when more lights overlap than the bound. S1 itself still lists a direct pixel-to-lights cost.

**Fit.** A mask written before shading assumes the receiver is the pixel's surface; Renderlab's program API passes `receiver` explicitly to `light_visibility`, so programs that query another point (transmission, volume samples) would need an escape to the inline path. A per-pixel *bitset over the cluster's list* (at most 128 bits = 16 B per pixel, 32 MiB at 1080p, **E**) is the safe shape.

### 2.3 Cached shadow atlases with update budgets and static/dynamic splits

**How.** Unity HDRP keeps cached lights in their own atlases and updates a light's map only on enable, on demand, or past a movement threshold (S7); "mixed cached shadows" cache static casters, draw dynamic casters every frame, and "HDRP performs a blit from the cached shadow map to the dynamic atlas" so one map needs space in both atlases (S7). Unreal's VSM stores "effectively two copies of depth, one for each category of geometry"; when dynamic geometry moves, the cached static page is composited, not re-rendered, and the invalidated static pages should stay near zero (S1). Moving lights get a different LOD bias, easing back when they stop (S1).

**Cost.** Memory: a second atlas (S7). Time: a blit or composite per updated map plus the dynamic casters' draws. Invalidation is by bounds: Unreal invalidates the pages overlapping a moved caster's light-space bounds, and any light movement invalidates all its pages; skeletal deformation, world-position offset and pixel-depth offset invalidate every frame (S1).

**Where it fails.** A cached map is stale until updated: HDRP notes a moved light with an un-updated cached counterpart "looks wrong", view-dependent effects mismatch, and a full cached atlas gives that light no shadow (S7). Budgeted updates (a few maps a frame) therefore trade cost for visible lag on whatever is not refreshed; that is the "shadow pop" to expect, and exact composition of static and dynamic depth does not have it.

**Fit.** Invisible to programs; it is entirely on the map side. For Renderlab it removes T1.

### 2.4 Clustered and tiled culling for the shading side

**How.** Clustered shading groups view samples into 3D clusters (tiles split exponentially in depth) and assigns each cluster its lights; in S3's Figure 1 about 2,400 lights render in 17 ms on a GTX 480 (2.3 ms clustering, 1.5 ms assignment, 5.6 ms shading) against 26 ms for tiled shading (17.7 ms shading). Both papers (S2, S3) also use the clusters for shadow work: resolution, page commit and caster culling.

**Cost / failure.** Cost is O(clusters x lights) for assignment. A conservative cluster includes lights that do not reach a receiver; Renderlab removes those exactly (zero attenuation). The cap (128 lights per cluster, `light_cluster.rs:22`) drops lights beyond it.

**Fit.** Already adopted (LC1). The gap is the stages that shade *away from the camera's clusters* (reflection hits, GI cells): they use a view-space grid they are not in, so they walk every light (section 1, T3, T4). A world-space grid is the same structure in a different frame.

### 2.5 Stochastic light sampling with a denoiser

**How.** Choose one or a few lights per pixel by importance and rely on temporal and spatial reuse: a two-level light BVH refit per frame (Moreau et al.: "thousands of lights ... ray-traced shadows in 16.1 ms", under 1 ms hierarchy updates, S8), or resampled reservoirs reusing neighbours and prior frames (ReSTIR: "at most 8 rays per pixel", 15 ms for 11,000 emissive triangles and 50 ms for 3.4 million at 1080p, with an unbiased and a faster biased estimator that loses some energy, S9). Wyman and Panteleev rework it for production (S10, not opened).

**Cost / failure.** Few visibility queries per pixel, but noise that a reconstruction pass must remove, and "a second temporal-noise source" Renderlab's LC3 chose not to add (`docs/m7-lc3.md`, "Candidate decision").

**Fit: poor.** Renderlab's programs loop over `light_count()` and `light_sample(i)` themselves and sum their own result. A sampled subset changes what the loop means (each term needs weight 1/p), so it needs a new light API and breaks existing programs. It also needs a visibility query per sample, which on non-RT is still a map lookup, so it does not remove T1.

### 2.6 Froxel fog with temporal reprojection, and how its shadow lookups are bounded

**How.** A frustum-aligned volume (Wronski: 160 x 90 x 64 or x 128; Frostbite: 8 x 8 pixel tiles, 64 slices) is lit once per froxel, then integrated along the ray; the result is looked up per pixel (S4, S5). The shadow lookups are bounded by fixing the *number of froxels*, not pixels, and by what is looked up: Wronski shadows the main light from downsampled exponential shadow maps and adds artist-marked point lights, for which the slides name no shadow term (S4); Frostbite reuses the tile light lists, skips local lights where scattering is zero and shadows local lights from regular maps or "volumetric shadow maps" (transmittance volumes), using them on 2 of 14 lights in its test (S5); Unreal makes local-light fog shadows opt-in per light ("Cast Volumetric Shadow", about three times the cost of a Point or Spot light, S6).

**Cost (S4, S5, S6).** Wronski: about 1.1 ms in total (platform not stated in the slides). Frostbite on a PS4 at 900p, 64 slices: 2.95 ms at 8 x 8 tiles (light scattering 2.00 ms: local lights 1.1, sun 0.5, temporal 0.4) and 0.83 ms at 16 x 16.

**Where it fails.** Under-sampling and aliasing under motion before temporal integration; after it, "moving lights leave trails" and shadows "can flicker for moving lights" (S5); Unreal's heavy reprojection leaves lighting trails behind flashlights and muzzle flashes, with the workaround of zero scattering for such lights (S6). Frostbite's fixed 5% current-frame blend (S5) is the same kind of trade as Renderlab's 0.9 history weight (`docs/m7-vf1.md`).

**Fit.** Renderlab already composes fog after user shading (VF1), so programs are unaffected. Today fog reads only the sun (section 0), so the fog's shadow-lookup bound is already tight: about 1 M sun lookups per frame. Adding local lights to fog would multiply froxels by lights per froxel; the LC1 cluster lists can serve if the fog grid maps onto them. That is a feature, not a cost reduction, and is out of scope here.

## 3. Summary

| Technique | Removes | Costs | Visible failure | Programmable shading | Renderlab status |
|---|---|---|---|---|---|
| Static/dynamic cached depth (2.3) | T1 | second atlas, a copy per updated map | none if depth is composited exactly; lag if updates are budgeted | invisible | absent |
| Demand-driven faces / pages (2.1) | T1, memory | request pass, page pool | leaks at page and mip edges, pool overflow | invisible | absent (sun has it) |
| Per-pixel light list in world space (2.4) | T3, T4 | grid build | none (conservative) | invisible | view-space only |
| Per-pixel shadowed-light cap (2.2) | T2 | bitset or mask | missing shadows past the cap | mask must not assume receiver = pixel | absent |
| Penumbra-adaptive filter (section 4) | T2 | one branch | seam where hard and soft filters meet | invisible | absent |
| Stochastic selection (2.5) | T2 (not T1) | denoiser, light BVH | noise, ghosting | breaks the light loop | absent |
| Froxel fog (2.6) | n/a | fixed | trails | after user shading | present, sun only |

## 4. Proposal: what to try first, each within one two-hour unit

**Step 0 (30 min, no code): attribute.** On the non-RT path run `examples/game_scene.rs` with `RENDERLAB_PASS_LOG=<file>` (per-stage ms per frame, 64 marks, `src/pass_timeline.rs:1-12`) in this matrix, recording the number of local lights alongside:
- R0 default; R1 `--still` (frozen camera and objects); R2 `--local-shadow-count 0`; R3 `--no-reflections`; R4 `--no-gi`; R5 `--no-volumetric-fog`.
- Add N = 3, 27, 64, 128 by changing the lantern count (`game_scene.rs:686`) for R0 and R3.

Read: `light_clusters` (T1, T5), `shading` (T2-T4), `atmosphere` (fog), `sun_shadow`, `visibility`. A flag that removes a feature removes its image too, so each difference is an upper bound on that term, not its isolated cost. `Renderer::light_cluster_stats()` (`light_cluster_gpu.rs:352`) gives `faces_drawn` and `caster_draws` per frame, but I found no reach to it from the `engine` facade (`grep` of `src/engine`), so the example cannot print it without a small patch.

**Decision from step 0 (E).**

| Observation | Meaning | First screen |
|---|---|---|
| `light_clusters` >= ~25% of the frame and R1 (`--still`) removes most of it | T1 | proposal 1 |
| `shading` falls most with R3 and grows with N in R0 minus R3 | T3 (and T4 under R4) | proposal 2 |
| `shading` falls most with R2, flat in R3 | T2 | proposal 3 |
| `atmosphere` > ~5 ms | fog; contradicts section 0, look for a residency stall | profile the fog kernels |
| nothing above explains it | an input difference, not a pass | re-check section 0 |

**Proposal 1: split static from dynamic, and draw only faces that are asked for (T1).**
- *Idea.* Classify a caster static once its model matrix and skin revision have not changed for 8 frames (the per-caster key already exists, `light_cluster_gpu.rs:525-563`). Key each slot on its static casters only; keep a second depth atlas holding the static result; on a dynamic change copy the tile and draw only the dynamic casters onto it (HDRP's blit, S7; Unreal's composite, S1). Add a per-face "occupied" bit from the clusters that hold visible receivers (S2's page marking at face granularity) so unsampled faces are not drawn.
- *Expected (E).* Draw calls fall from (all casters in range) x faces to (copy + dynamic casters) per changed light; with 2 moving casters in a courtyard of dozens of static casters that is an order of magnitude fewer draws, but the render-pass count (one per face) is unchanged unless unsampled faces are skipped.
- *Confirm or reject in two hours.* (a) 30 min: log per frame the number of distinct casters changed and, per slot, static versus dynamic draw counts (CPU only, from the existing candidates). Reject if dynamic casters are more than ~30% of draws (the split saves little). (b) 90 min: prototype the second atlas for the lights that move-contact only (the 3 example lights) and compare `light_clusters` ms, R0 against `--still`. Confirm if the moving-frame cost falls to within 1.5x of still and the still image is byte-identical.
- *Visible failure.* None while the static copy and dynamic draw produce the same depth as a full redraw (compare the per-slot depth buffers byte for byte). Memory: +384 MiB at defaults, so limit it to tiles of 256 or more.

**Proposal 2: a world-space light list for stages that walk every light (T3, T4).**
- *Idea.* Build a 3D uniform (or cascaded) grid of light indices around the camera with the LC1 kernel's test on world boxes, and let reflection-hit and GI-cell stages read it instead of `lighting.count` (section 2.4). Lists are conservative, so the sums are unchanged; hits outside the grid keep the walk.
- *Expected (E).* Cost per hit falls from N to the lights reaching the hit. At N = 128 and 16 strata that is up to two orders of magnitude fewer light evaluations per glossy pixel; at N = 3 it saves nothing.
- *Confirm or reject in two hours.* From step 0: R0 minus R3 and `shading` against N. Reject if the slope is under ~0.05 ms per light at N >= 27 or the shading difference with `--no-reflections` is flat in N (the walk is not the cost). Confirm, then spend the unit on a CPU oracle: for random hit points, the grid's list equals the brute-force list of lights with non-zero attenuation (the form of the LC1 oracle test, `docs/m7-lc1.md`, "4,000 random receivers").
- *Visible failure.* None if conservative. A dense region past the per-cell cap needs the same overflow rule as LC1.

**Proposal 3: a penumbra-adaptive filter (T2).**
- *Idea.* The soft filter (12 blocker taps, then 96 loads) assumes penumbrae of many texels. With `source = 0.5 * projection * radius * extent` and `penumbra = source * (w - blocker) / (blocker * w)` (`light_cluster_api.wgsl:103-116`), a 0.06 m lantern in a 128-texel tile with a 90 degree face gives about 1 texel of penumbra at 4 m with the blocker at 2 m (E, assuming `projection` = 1), but about 8 texels in a 1024-texel tile. For small penumbrae use the hard 4x4 PCF path; for large keep the soft path; blend over one texel of penumbra width to avoid a seam.
- *Expected (E).* Pixels in or near a shadow edge drop from 108 loads to 16. T2's share falls in proportion to the share of (pixel, light) pairs with penumbra under ~1.5 texels; this is the lanterns in a 128-light pool, not the arch light.
- *Confirm or reject in two hours.* (a) Override every `source_radius` to 0 and time `shading` (R0 against R-radius0): the difference is the upper bound of what the soft taps cost. Reject if under ~15% of shading. (b) Add a debug counter of (pixel, light) pairs that exit after the blocker search, that reach the 96 loads, and the mean penumbra in texels among them. Confirm if more than half of the pairs that reach 96 loads have penumbra under 1.5 texels.
- *Visible failure.* A step in softness where the branch flips; keep the blend. It will not be byte-identical (P-LTS's rule), so it needs an image gate on the LC fixtures, not a hash.

**Proposal 4: premise check for virtual local-light pages, no code.**
- *Idea.* Reuse N8's page-request and software-raster machinery for local lights (section 2.1). It is not a two-hour build, but its payoff has a cheap bound.
- *Confirm or reject in two hours.* A CPU model from the scene's lights, camera and occupied clusters: pages of 128 x 128 that visible receivers actually sample at the resolution the cluster projections require (S2's method), against the texels the 128 allocated tiles hold. Confirm if needed texels are under ~25% of allocated texels (the atlas is then mostly waste) *and* proposal 1 leaves `light_clusters` above ~20% of the frame. Reject otherwise.
- *Visible failure.* From S1: first-layer leaks, box-like shadows from resolution exhaustion, page-pool overflow.

## 5. Not proposed

- **Stochastic light selection with reuse (2.5).** It would not remove T1, it changes the meaning of the program's light loop, it adds a noise source LC3 chose not to add, and it brings a flagged patent family. Revisit only if proposals 1-3 are done and a scene with 128 overlapping lights still spends over a third of its frame in T2.
- **Lower resolution or fewer shadowed lights.** It cuts cost by cutting the quality the P-LTS and LC3 units held fixed.
- **Fog changes.** Fog is independent of N and common to both paths (section 0); touch it only if `atmosphere` is large in step 0.

## 6. Patents

Policy is suspended (owner, 2026-11-10); numbers only, from [m7-p0-claims.md](m7-p0-claims.md), which I did not re-read claim by claim: ReSTIR resampling US 11,315,310; many-light importance resampling US 11,663,773. I searched for no patent on cached shadow atlases, virtual shadow maps, clustered shading or froxel fog; P0 records none.

## 7. What reading cannot determine

- Which term holds the 64 ms. That is step 0.
- The real light count of tonight's scene (section 0).
- Whether `shading` time that moves with `--no-reflections` is hit light walks (T3) or the hits' own `shade()` work (`docs/design-nonrt-glossy-cost.md` splits the latter).
- Frostbite's and Wronski's numbers are on a PS4-class GPU; I did not scale them to the 4070 SUPER beyond noting the gap, and S4 does not name its platform.
- Whether the penumbra estimate in proposal 3 matches the pool's actual tile sizes; it assumes a 90 degree face and `projection` = 1.

## 8. Sources

Read in full or in the sections used: S2, S3, S4, S8 (PDF text extracted locally). Read through the fetch tool's page extraction, so quotations are the tool's, not mine: S1, S5, S6, S7. Not opened: S10. Search-summary only (page returned 403): S11.

- **S1** Epic Games, "Virtual Shadow Maps in Unreal Engine", Unreal Engine documentation, `dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine`.
- **S2** O. Olsson, E. Sintorn, V. Kämpe, M. Billeter, U. Assarsson, "Efficient Virtual Shadow Maps for Many Lights", author preprint, Chalmers, `cse.chalmers.se/~uffe/ClusteredWithShadows.pdf`. The preprint does not name a venue; I recall I3D 2014 and did not verify it.
- **S3** O. Olsson, M. Billeter, U. Assarsson, "Clustered Deferred and Forward Shading", High-Performance Graphics 2012, pp. 87-96, `cse.chalmers.se/~uffe/clustered_shading_preprint.pdf`.
- **S4** B. Wronski, "Volumetric Fog: Unified Compute Shader Based Solution to Atmospheric Scattering", SIGGRAPH 2014 (Advances in Real-Time Rendering in Games), `bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf`.
- **S5** S. Hillaire, "Physically Based and Unified Volumetric Rendering in Frostbite", SIGGRAPH 2015 (Advances in Real-Time Rendering in Games), `advances.realtimerendering.com/s2015/`; text read from the author's SlideShare copy, `slideshare.net/slideshow/physically-based-and-unified-volumetric-rendering-in-frostbite/51840934`.
- **S6** Epic Games, "Volumetric Fog", Unreal Engine 4.27 documentation, `dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog?application_version=4.27`.
- **S7** Unity Technologies, "Update shadows less frequently" (HDRP 17.0), `docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/shadow-update-mode.html`, and "Shadows in the High Definition Render Pipeline" (HDRP 16.0), `docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@16.0/manual/Shadows-in-HDRP.html`.
- **S8** P. Moreau, M. Pharr, P. Clarberg, "Dynamic Many-Light Sampling for Real-Time Ray Tracing", High-Performance Graphics 2019 (short papers), `research.nvidia.com/sites/default/files/pubs/2019-07_Dynamic-Many-Light-Sampling//MPC19.pdf`.
- **S9** B. Bitterli, C. Wyman, M. Pharr, P. Shirley, A. Lefohn, W. Jarosz, "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting", ACM TOG (SIGGRAPH) 2020, `benedikt-bitterli.me/restir/` (project page abstract only).
- **S10** C. Wyman, A. Panteleev, "Rearchitecting Spatiotemporal Resampling for Production", High-Performance Graphics 2021, DOI 10.2312/hpg.20211281 (listing only).
- **S11** Epic Games, "Virtual Shadow Maps in 'Fortnite Battle Royale' Chapter 4", `unrealengine.com/tech-blog/virtual-shadow-maps-in-fortnite-battle-royale-chapter-4` (not read; 403). The one-pass-projection mechanism in 2.2 is taken from a search-tool summary of this page and is unverified.
- id Tech 6 (Sousa and Geffroy, SIGGRAPH 2016) is the usual reference for a cached shadow atlas with dynamic compositing; its slides are image-only in the PDF I extracted (`advances.realtimerendering.com/s2016/Siggraph2016_idTech6.pdf`) and I did not rely on them.
