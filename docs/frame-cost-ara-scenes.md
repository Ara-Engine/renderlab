# Why Showcase and Outpost are slow, and the plan to URP-class frames

Root, 2026-11-12, after Islam's word ("fix the performance... scenes running on 10 fps on a RTX 4070 super"). First bar: 16.7 ms. **The direction moved the same day**, when Islam saw Unity's URP on the same scenes (built player, 1920x1080, D3D12, this RTX, the same camera path; preliminary: exposure and sky not matched, no GI, no traced reflections, MSAA 4x): Showcase 0.63 ms GPU, Outpost 4.78 ms, against Renderlab 29.8 / 35.2 and 44.4 / 69-82. His words: "the difference is crazy see that 0. something that what I think we should be going towards". It is a goal, not a rule; the method is ours, and the picture rule (no visible loss against a reference, still, moving, after cuts) stays.

**Targets (root's reading with the coordinator's):**
1. **The feature-matched floor:** with GI and reflections off, within about 1.5x of URP on both scenes on both paths: Showcase about 1 ms, Outpost about 7 ms.
2. **The extras:** GI and traced reflections together about 3 ms on top of the floor, so Showcase lands near 4 ms with everything on, the class of Renderlab's own quarry frame.
3. **Cost follows the scene:** anything paid per pixel whatever the scene holds is the first thing to find. The quarry is a heavier scene than Showcase and costs about 7 times less.

## 0. Per-pass targets (1080p, RTX 4070 SUPER, camera moving, ms)
Today's numbers: Ara lease 5 (Renderlab 835701d4, clean, five rounds) and earlier detail captures; "floor" is GI and reflections off. Quarry for comparison: docs/unreal-pass-budget.md (non-RT with the Disk lookup).

**Showcase** (43 instances, 380k triangles, sun and two spots with no radius, 2 skinned characters, 5 glass and 3 blend primitives, glossy and mirror balls):

| pass | today RT / non-RT | quarry | floor target | full target | mechanism | why above the quarry today |
|---|---|---|---|---|---|---|
| visibility (RT: camera and sun rays) | 3.7-5.5 / 0.5 | 1.45 / 0.65 | 0.3 / 0.3 | 0.4 / 0.3 | RT: raster primaries (the non-RT visibility buffer) and the sun from the held budget or the cached map; the two skinned characters keep their swept shadows on the full budget today | skinned movers and their swept shadows keep the full 32 sun directions; the quarry has no movers |
| shading: base (entry, decode, material, BRDF) | about 6.5 (M5) | 0.65 total | 0.4 | 0.4 | material work is small (textures 0.8, BRDF 1.3 on the M5); the rest is the entry and decode, to be priced on the RTX | the quarry's shading is 0.65 for the whole stage |
| shading: camera GI lookup | about 4-6 | (in 0.65) | 0 (off) | under 1 | Mac: the repeated sample (1.2-1.9 ms found), then the second level and fills | per pixel, two levels of 8 hashed corners each |
| shading: camera lights | about 4 | 0 (sun only) | 0.3 | 0.3 | one hard ray per spot already; `cast_shadows`; cluster walk | two spots reaching most of the view |
| shading: reflections | about 17-19 | 0 (rough rock, untraced) | 0 (off) | about 2 | U-RTVIS: fewer strata (step 1), one ray per pixel or quad with an accumulator (step 3); Mac: compacted hit shading; `max_roughness` ruling pending | 96% of rays from 12.6% of pixels at 15.2 strata each |
| atmosphere | 0 (elsewhere) / 2.5-2.7 | small | 0.2 | 0.2 | find the part (`cpu.wait` suspected) | fixtures 0.34 ms |
| TR1 | 1.3 / 1.0 | 0.53 | 0.4 | 0.5 | after-cut frames cost 5.7 on RT (lease data): find why | |
| transparency | 0.9 / 0.4 | none | 0.2 | 0.2 | per-pixel panes (as the port did, 8.06 -> 0.63 on the M5) if the RTX shows a cost | no glass in the quarry |
| skinning | 0.7-0.8 / 0.05 | none | 0.1 | 0.1 | two characters; RT also refits their BLAS each frame | no skinned meshes in the quarry |
| GI passes (update, gather, cells) | 0.8 / 1.6 | 0.7 / 1.8 | 0 (off) | 0.5 | already small | |
| post | 0.2 | 0.64 | 0.2 | 0.2 | | |
| **frame** | **29.8 / 35.2** | **4.0 / 4.5** | **about 1.5-2** | **about 4** | | |

**The measured floor (2026-11-12, perf7, ara-integration perf/table-floor.md):** Ara main 4ff23f14a, pin 835701d4, 1080p, moving, 5 interleaved rounds, clean lease. Unity URP on the same scene: 0.66 ms.

| Showcase, ms (RT / non-RT) | default | reflections, gi off | + local_shadows, media off | + post, tr1 off (the floor) | floor target |
|---|---|---|---|---|---|
| frame | 29.41 / 34.72 | 12.39 / 8.20 | 11.72 / 8.51 | **9.41 / 5.06** | 1.5-2 |
| visibility | 3.78 / 0.50 | 4.27 / 0.89 | 4.27 / 0.93 | 3.17 / 0.95 | 0.3 / 0.3 |
| sun_shadow (non-RT) | - / 0.46 | - / 0.69 | - / 0.70 | - / 0.71 | - / 0.3 |
| skinning (RT: plus the BLAS update) | 0.78 / 0.05 | 1.28 / 0.07 | 1.23 / 0.07 | 1.53 / 0.07 | 0.1 |
| shading | 22.02 / 27.06 | 3.18 / 2.57 | 2.72 / 2.40 | 3.26 / 2.57 | 0.4 |
| transparency | 0.68 / 0.45 | 0.95 / 0.73 | 0.88 / 0.74 | 1.30 / 0.70 | 0.2 |
| tr1 | 1.41 / 1.10 | 2.19 / 2.29 | 2.19 / 2.58 | 0 | 0.4 (on) |
| atmosphere | 0 / 3.57 | 0 / 0.31 | 0 / 0.51 | 0 / 0.02 | 0.2 |
| gi | 0.82 / 1.34 | 0 | 0 | 0 | - |
| still GPU / wall per frame | 19.96 / 22.96 (RT) | 5.16 / 8.43 | 4.97 / 7.71 | 3.58 / 6.70 (RT), 2.87 / 5.67 (non-RT) | wall = GPU |

At the floor the frame is 14 times URP on RT and 8 times on non-RT. Four fixed costs, paid by every scene, make up most of it:

1. **Base shading, 2.6-3.3 ms** for a sun and two unshadowed spots, against 0.65 ms for the quarry's whole stage. What it contains is not priced on the RTX yet. On the M5, textures are 0.8 and the BRDF 1.3, and the rest is the entry and the decode. Candidates where Showcase differs from the quarry:
   - textured materials, sampled with derivatives from the visibility-buffer decode (the quarry has vertex colours);
   - specular AA;
   - the per-light panes transmission lookup (`light_transmission`);
   - the cluster walk;
   - the environment terms (DFG, prefiltered map);
   - one program's entry over several Ara materials through MP1's slot table.

   Mechanism: price it first, with RL#50's term switches (`shading.without_*` under `RENDERLAB_PASS_LOG_DETAIL`) at the floor row on the RTX, then cut the largest terms. Owner: the Mac GPU thread (program_api.wgsl, programs/*). It can start the price while `reflection_hits` waits for `rl_trace`. Target 0.4-0.6.
2. **RT visibility 3.17 ms and skinning 1.53 ms, against non-RT's 0.95 and 0.07.** RT traces a camera ray per pixel. Non-RT rasterizes a visibility buffer and pays 0.71 for the sun.
   - Mechanism (a): the visibility buffer on the RT path too (`PrimaryVisibility::VisibilityBuffer` exists; engine/mod.rs picks `Traced` for `HardwareRt`), with hardware sun rays from the buffer's positions. The held and cold probe logic moves with it; today it exists only under `Traced` (primary_ray_gpu.rs HeldTargets).
   - Mechanism (b): skinned BLAS updated in place (refit, allow-update flags) instead of rebuilt. Find why skinning grows from 0.78 to 1.53 as other passes switch off: if it is waiting on or overlapping other work, the cost is not the skinning itself.
   - Worth about 3.5 ms of the 9.4. Owner: proposed new unit U-FIXED (below). Target: visibility 0.3 plus the sun's 0.3, skinning 0.1-0.2.
3. **Transparency, 1.30 / 0.70 ms** for five glass and three blend primitives. Mechanism: the per-pixel pane shading RL#48 gave the ara-gpu port (8.06 to 0.63 ms on the M5), on the wgpu path too. Owner: the Mac GPU thread, after `reflection_hits`. Target 0.2.
4. **TR1, 2.2-2.6 ms when on, against 0.53 on the quarry**, plus 1.1 ms of RT visibility: visibility is 4.27 with TR1 and 3.17 without. Shading does not run at a reduced rate with TR1 (3.18 on, 3.26 off), so on this scene TR1 is a pure cost.
   - **What it buys:** anti-aliasing of edges and specular, and the history every stochastic pass relies on: the held and cold sun probes, the reflection strata, GI noise, soft local shadows. In the full frame it is structural; at the features-off floor it buys only anti-aliasing.
   - Mechanism: price TR1's parts on Showcase (detail marks): motion vectors for the skinned meshes, the history of transparent pixels, disocclusion on many small objects. Find why jitter costs RT visibility 1.1 ms. Native-rate shading with a cheap AA (SMAA or FXAA, about 0.2-0.3 ms at 1080p) is measured only as a named comparison, judged on crops for aliasing and flicker against the reference. TR1 stays the default unless that alternative is visibly no worse in motion.
   - Owner: U-FIXED. Target 0.4-0.5.

Also measured:
- **Non-RT atmosphere shares work with GI:** 3.57 ms with GI on, 0.31 with it off. The `atmosphere.*` and `cpu.wait` detail marks name the part. Owner: U-RTVIS (atmosphere*), with GI on Outpost.
- **CPU:** the still wall frame is about twice the GPU frame at the floor (3.58 ms GPU against 6.70 wall on RT, 2.87 against 5.67 on non-RT, 0.30 of it handoff). So about 3 ms a frame goes to CPU and presenter work. `FrameReport::cpu` (src/observe.rs: preparation, visibility, packing, upload, encode, submit, poll, and the shadow parts) gives Renderlab's share once the integration thread logs it beside the wall time and the bridge's own extraction and sync time. That limit binds once the GPU frame is under about 6 ms. Owner: root reads the split, then assigns.

The floor adds to about 2 ms, not 1: a visibility buffer, a reconstruction and post at the quarry's own costs already take about 1 ms. Going below needs TR1 and post priced against URP's (no temporal reconstruction, MSAA instead), which is a picture question for later.

**Outpost** (98 mesh instances of 53 models plus about 14M foliage triangles a frame, 3.1M prop triangles, 1,108 instances for software rays, 22 lights: sun plus 21 shadowed local lights, twelve of them 65 lm 3 m markers, all radius 0, 1 skinned character):

| pass | today RT / non-RT | floor target | full target | mechanism |
|---|---|---|---|---|
| visibility (RT: camera and sun rays) | 12.7 / 0.95 | 1.0 / 1.0 | 1.5 / 1.0 | RT: why the held sun budget doesn't engage (a sun moving with the day cycle is the suspect: `relight_parts` capture); raster primaries; foliage LOD and size culls (Ara #137) |
| shading without reflections (base, camera GI lookup, 22 lights) | at most 8.4 / 5.5 (shading at cutoff 0.25, which still traces below 0.25) | 2.0 | 2.8 | 21 hard rays per pixel at most today; `cast_shadows` off for the markers (Ara's flag, RL#51); light importance; the GI lookup's share as on Showcase |
| reflections | about 8.6 / about 26 (shading 17.0 -> 8.4 RT and 31.3 -> 5.5 non-RT from cutoff 0.5 to 0.25) | 0 (off) | 0.7 / 0.7 | trace where it can show (below); steps 1 and 3 for the pixels that still trace. The picture barely depends on it: at cutoff 0.25, full-frame PSNR 51.9 dB, crops 54-60, ground crop identical, no popping |
| sun shadow (non-RT) | in visibility / 5.1 | 1.0 | 1.0 | foliage as non-casters or culled (object flags), page pool |
| GI (all stages) | 9.1 / 23-54 | 0 (off) | 2.0 | the relight cause (every cell requeued every frame while moving), then the gather budget |
| atmosphere | 1.9 / 3.9-13.6 | 0.5 | 0.5 | name the part (`cpu.wait` suspected: CPU GI preparation with 1,108 instances) |
| TR1 | 1.6 / 1.1 | 0.5 | 0.5 | |
| transparency | 1.5 / 0.4 | 0.3 | 0.3 | |
| skinning, post | 0.4 / 0.2 | 0.3 | 0.3 | |
| **frame** | **44.4 / 69-82** | **about 6-7** | **about 9-10** | |

**Reflections: the cost isn't where the picture is.** The roughness sweeps on the RTX (Ara #138, ara-integration perf/table-lease6sc.md and table-lease6op.md, moving, cutoff 0.5 / 0.35 / 0.25):

| | 0.5 | 0.35 | 0.25 | picture at 0.35 |
|---|---|---|---|---|
| Showcase RT frame (shading) | 30.4 (22.5) | 20.3 (10.4) | 19.0 | copper ball loses its ground reflection and rim, marble its band: visibly worse |
| Showcase non-RT frame | 35.1 | 27.4 | 20.0 | same |
| Outpost RT frame (shading) | 48.2 (17.0) | 47.0 | 36.3 (8.4) | unchanged (PSNR 52.3, crops 54-60) |
| Outpost non-RT frame (shading) | 74.0 (31.3) | 48.4 (9.7) | 43.6 (5.5) | unchanged |

Outpost's rough-glossy ground and props pay the full strata (about 22 ms of non-RT shading for the 0.35-0.5 band, 9 ms of RT shading for 0.25-0.35) for reflections that don't show, while Showcase's two balls need the same band. So a roughness number is the wrong switch. Each pixel decides whether its traced result can differ visibly from the untraced estimate, and traces only if it can. This sits on top of steps 1 and 3.

1. **A bound before any ray.** The visible change a trace can make is the lobe's reflectance (F0 and the DFG term, which the program already computes) times how far the traced incident light can sit from the estimate. That second factor is bounded from what the receiver's GI cell holds (its bounce light, its sky visibility, the brightest outgoing cell value nearby) and the environment's maximum in the lobe. Where reflectance times that bound stays under half an output code after exposure and tone mapping, the pixel doesn't trace. A rough dielectric (F0 0.04) under even light falls out here. Copper (F0 about 0.6-0.9) next to dark ground doesn't. Cost: a few loads in the shading pass, no ray.
2. **A measured difference for pixels that pass the bound.** Step 3's accumulator keeps each traced pixel's traced-minus-estimate in output codes. A pixel whose difference has stayed under one code for 8 frames drops to one ray every 8 frames, and its history holds the traced value. It comes back to the full rate on disocclusion, a cut, a light, environment or GI-key change (the cache's `relight_parts`), or as soon as one sparse ray shows a difference over one code. Being wrong costs at most one code until the next sparse ray, under 8 frames.
3. **The resolve stays per hit:** a cone wider than the hit cell reads the GI cache's cell (`gi_hit_radiance`), narrower cones shade the hit (`reflection_hits`), as given to U-RTVIS on 2026-11-12.

The threshold is in output codes, so a skipped pixel is under one code of what a full trace gives, measured against the trace itself, not against a roughness. The gate is still crops against the path-traced reference, still and moving. Expected result: Outpost reflections go from about 26 ms (non-RT) and 8.6 ms (RT) to under 1 ms, because most of its band falls out at test 1 or 2. Showcase keeps its traced balls and lands on step 3's about 2 ms. Owner: U-RTVIS (reflections*, gi_cache.wgsl). The bound gets the reflectance from the program, through the lobe the program already declares. `reflection_lobe`'s `weight` (today 1.0 in standard.wgsl, and greater than 0 to request rays) becomes the lobe's reflectance, so the standard program returns the luminance of `f0*ab.x+ab.y`. The engine still knows nothing about any one lighting model, and a program that wants rays everywhere declares a weight of 1.

**Owners:** reflections (U-RTVIS: strata, trace, accumulator; Mac: hit shading), the camera GI lookup (Mac), local-light shadows and `cast_shadows` (Mac, RL#51), the relight cause and the held-budget question on Outpost (root, through the integration and bridge threads; the fix lands with U-RTVIS), object flags and foliage culls (U-RTVIS after reflections; Ara #137 for LODs), the unexplained per-pass costs (visibility on RT, atmosphere, TR1, transparency, skinning): root prices them once the features-off capture lands.

## 1. The three scenes, pass by pass (1080p, RTX 4070 SUPER, camera moving, ms, medians)
Quarry: Renderlab's own matched quarry (docs/unreal-pass-budget.md, before the Disk default; the Disk lookup has since cut the non-RT sun pass to 0.22 ms and the moving frame to 4.53 ms). Showcase and Outpost: Ara's player through `RENDERLAB_PASS_LOG`, Renderlab pin f3d0605 or 5955d4a (Ara integration captures `root-detail-*`, `mo-u2-*`). None of tonight's merges below are in these numbers.

| pass | quarry non-RT | quarry RT | Showcase non-RT | Showcase RT | Outpost non-RT | Outpost RT |
|---|---|---|---|---|---|---|
| **frame** | **10.5 (4.53 with Disk)** | **4.0** | **27.1-33.1** | **19.1-24.6** | **100.9** | **35.4** |
| shading | 0.67 | 0.65 | 21.4-21.8 | 12.4-16.9 | 32.8 | 11.6 |
| visibility (RT: camera and sun rays) | 0.65 | 1.45 | 0.5 | 3.7-4.0 | 0.9 | 12.5 |
| sun shadow (non-RT) | 6.2 (0.22 with Disk) | in visibility | 0.1-0.3 | in visibility | 5.1 | in visibility |
| GI (all `gi_` stages) | 1.8 | 0.7 | 1.3 | 0.7 | 53.9 | 6.6 |
| ↳ gi_gather | | | 0.3 | 0.1 | 43.5 | 2.9 |
| atmosphere | (in post) | | 2.5 | 0 (elsewhere) | 3.9-6.8 | 1.2 |
| TR1 | 0.53 | 0.52 | 0.6-1.0 | 0.5-1.3 | 1.1 | 1.2 |
| transparency | | | 0.4 | 0.6-0.7 | 0.4 | 1.3 |
| skinning | | | 0.05 | 0.5-0.7 | 0.05 | 0.2 |
| post | 0.68 | 0.64 | 0.14-0.2 | 0.16-0.2 | 0.14 | 0.19 |

**What the table says:**
- **Showcase is the shading stage**, 12-22 ms of 19-33, against 0.65 ms on the quarry: 20-30 times the quarry's. Everything else in Showcase is about quarry-sized. The quarry has a sun and no local lights, and rough rock with no traced reflections; Showcase has local lights with shadows and glossy and mirror materials. On Renderlab's game flight (a lamp-lit, reflective stand-in) the split is reflections about half and local-light shadows about half (they overlap: reflection hits are shaded with full local-light shadows).
- **Outpost non-RT is GI, then shading:** gi_gather 43.5 ms through software rays that walked every instance (U-RTVIS's forest fixture: 161 ns a ray with ~9,900 instances against 4-12 ns with 30-44), then shading 32.8 (the software reflections walked the same flat list), then sun shadow 5.1 and atmosphere 3.9-6.8.
- **Outpost RT is visibility, then shading:** visibility 12.5 ms is the sun-shadow ray budget (32 directions a pixel while moving, 0.29 ms each on the forest fixture), then shading 11.6.

**Scene facts (pending; asked of the Ara bridge thread and the integration thread):** triangles, instances (now in `IndirectStats::instances` and the `RENDERLAB_GI_LOG` line), local lights and how many cast shadows, their ranges, shadowed-light rays per pixel, materials and how many are glossy or mirror, skinned meshes, transparent draws, terrain tiles, GI cells alive.

## 2. Already on Renderlab main since those numbers (not yet in Ara's pin)
| change | measured on | from → to |
|---|---|---|
| software-ray top-level BVH (non-RT GI and reflections) | forest fixture, non-RT moving | frame 286 → 36 ms; gi_gather 145 → 12.3; gi_update 99 → 6.1; gi_cell_shading 31 → 1.5 |
| RT moving sun budget (held probes) | forest, RT moving | visibility 9.6 → 2.7 ms |
| RT cut frame (cold probes) | forest, RT cut | visibility 74 → 10 ms |
| reflection-hit shadow split | game flight non-RT | shading 21.9 → 15.7 ms |
| local soft shadow inline compare | night lanterns non-RT | shading about 45 → 37 ms |
| pane transmission for lit samples only (RL#47) | Showcase on the M5 | -2 ms; byte-identical on the RTX game flight |

All byte-identical or within run-to-run noise on the picture, except where stated in docs/milestones-5-6-decisions.tsv.

## 3. Order of attack against 16.7 ms (estimates until Ara's capture at the new pin)
| # | scene, path | pass | from → to (estimate) | change | picture risk | owner |
|---|---|---|---|---|---|---|
| 1 | Outpost non-RT | gi_gather + gi_update + gi_cell_shading | 51.7 → about 6 | instance BVH (merged) | none (same hits) | done; confirm in Ara |
| 2 | Outpost non-RT | shading | 32.8 → about 15 | instance BVH for reflection rays (merged), reflection-hit split (merged) | none / within 1-4 codes | done; confirm |
| 3 | Showcase both, Outpost both | shading: the light loop | Showcase 12-22 → under 8 | per-pixel light importance (a pixel shadows only the lights that matter there), tighter clusters, shadow work shared across nearby pixels or accumulated by TR1 | penumbra noise and light leaking if a light is wrongly skipped; judged against the path tracer | Mac GPU thread (shared shading WGSL) |
| 4 | all | shading: reflections | about half of shading on lamp-lit glossy scenes | rough reflections from the GI cache or environment above a roughness, traced below; fewer strata where the lobe is wide | dull or wrong reflections on rough metal; judged on the gold and mirror spheres | U-RTVIS |
| 5 | Outpost RT | visibility | 12.5 → about 4 | held probes (merged) | none measured | done; confirm |
| 6 | Ara bridge | everything | unknown | light ranges and counts, 144 terrain tiles as groups, skinned meshes marked moving, material variants, RGBA8 textures, every material double-sided | none (scene feed) | Linux bridge thread |
| 7 | Outpost non-RT | atmosphere | 3.9-6.8 → under 1 | depends on which part (sky, inject, CPU wait): detail capture pending | none expected | U-RTVIS after the capture |
| 8 | Outpost non-RT | sun shadow | 5.1 (max 38.6) → about 1 | page pool (N-POOL brief) or Outpost's caster set | none expected | after 1-7 |

**Update, 2026-11-12 afternoon (RTX lease 5, clean, five rounds):** Showcase RT 29.8 ms, non-RT 35.2; Outpost RT 44.4. `RENDERLAB_RL_SPLIT=0` is within noise everywhere (lease 3's 3-6 ms was noise). `RENDERLAB_ABLATE=gi` saves a steady 6 ms on Showcase on both paths while the GI passes are 0.8-1.6 ms: the camera GI lookup inside shading (`gi_prepare`/`es_prepare`, about 4 ms on the M5) is the second-largest Showcase item after reflections. Reflections: 96% of Showcase's reflection rays come from the 12.6% of pixels at 9-16 strata (roughness about 0.33 up to the traced cutoff; Mac counting build).

**File ownership, so nobody collides** (revised 2026-11-12 afternoon):
- **Mac GPU thread:** `shaders/program_api.wgsl` (the light loop and its helpers), `shaders/programs/*.wgsl`, `shaders/light_cluster*.wgsl`, `shaders/local_light_ray.wgsl`, `src/light_cluster*.rs`, `src/lighting.rs`; the camera GI lookup in `shaders/gi_api.wgsl` (`gi_prepare`, `es_prepare`, `gi_sample_refined`); the compacted reflection hit-shading pass (new `shaders/reflection_hits.wgsl`, `src/reflection_hits.rs`) against U-RTVIS's interface (docs/design-reflection-pass.md).
- **U-RTVIS:** `shaders/reflections*.wgsl`, `src/reflection*.rs` (strata selection, the trace, the lobe output, the reduced-rate pass and its accumulator), the GI cache's update, gather and cell shading (`shaders/gi_cache.wgsl`, `gi_cells.wgsl`, `gi_near.wgsl`, `gi_rays_*.wgsl`, `src/gi_cache.rs`), `src/instance_bvh.rs`, `shaders/primary_*.wgsl`, `shaders/virtual_shadow*.wgsl`, `shaders/atmosphere*.wgsl`, `src/atmosphere_gpu.rs`, `examples/game_scene.rs` (additive `--diag` only).
- **Root:** docs, records, merges, tools/blobs.py, scripts.
- **Linux bridge thread:** Ara's `crates/ara-renderlab` only; Renderlab API additions it needs go through root.
- **PC integration thread:** measurements only; no Renderlab changes.

## 4. Flicker
Quarry yardstick (P-YARD2, 2026-11-10): Renderlab 4.32 (RT) / 4.38 (non-RT) against Unreal 3.68, frame-to-frame step against the reference's step on consecutive moving frames, lower is better. Tonight's quarry batch (Disk and Surface lookups, non-RT): flicker L0 4.375 for both, so the sun lookup is not the gap. Not measured on Showcase or Outpost in motion. Known contributors from earlier work: TR1's history on thin geometry and silhouettes (docs/m7-gaps.md, TR1), the GI cache's per-cell noise as cells are created in motion (mottle), and rotated sun directions in penumbrae. Plan: after the speed items above, rerun the yardstick at the new main to see where it stands, then split flicker by population (edges, flat surfaces, penumbrae) on the quarry, the cause found decides the owner.
