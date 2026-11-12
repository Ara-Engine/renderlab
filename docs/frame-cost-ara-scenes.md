# Why Showcase and Outpost are slow, and the plan to 16.7 ms

Root, 2026-11-12, after Islam's word ("fix the performance... scenes running on 10 fps on a RTX 4070 super"). Bar set by the coordinator: Showcase and Outpost at 1080p on the RTX 4070 SUPER, camera moving, both paths, at or under 16.7 ms, with no visible loss. This page is the evidence and the plan; it fills in as captures land.

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
