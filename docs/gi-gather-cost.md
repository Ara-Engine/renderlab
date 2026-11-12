# Why non-RT GI gathers are slow on large scenes (U-RTVIS study, 2026-11-12)

Outpost spends 45-60 ms a frame in `gi_gather` on the non-RT path (docs/m7-gaps.md). This page
measures the cause on a stand-in and ranks the fixes. No code changed for it.

## Measured split
RTX 4070 SUPER, 1080p, non-RT, release game_scene (main 776dcd0f + U-RTVIS), 60 moving frames,
medians over frames 8-59. `RENDERLAB_PASS_LOG_DETAIL` for the passes; the GI counters
(`RENDERLAB_GI_LOG`) from separate runs, since that log's readback blocks.

| scene (`--diag`, sun `--hour 17.651`) | software-ray instances | gi_gather | gather rays / frame | ns / gather ray | gi_update | gi_cell_shading | frame |
|---|---|---|---|---|---|---|---|
| forest walk (`forest=1`) | ~9,900 | 175.8 ms | 1.09 M | 161 | 74.9 ms | 34.4 ms | 296 ms |
| same terrain, no foliage (`forest=bare`) | ~40 | 12.8 ms | 1.07 M | 12 | 3.3 ms | 1.0 ms | 30 ms |
| game flight | ~30 | 4.6 ms | 1.13 M | 4 | 4.6 ms | 0.8 ms | 35 ms |
| forest, `gather_cells=8192` | ~9,900 | 42.0 ms | about a quarter | ~160 | 64.1 ms | 26.7 ms | 155 ms |

The forest traces about as many gather rays as the other two scenes. Each ray costs 13 to 40
times more. The budget variant scales linearly with cells, so a per-frame budget moves the cost
without lowering it per ray. Sky traces (`gi_update`) and cell light visibility
(`gi_cell_shading`) show the same blow-up: all three run `gi_traverse`.

## The cause
`gi_traverse` (shaders/gi_rays_software.wgsl) has no top level. Every ray loops over every
instance (`for i in 0..instances`), loads its world box and slab-tests it, and only then
descends into that instance's BVH4. The forest's ~9,900 placements (900 trunks, 3,600 canopy
cards, 5,400 bush cards, terrain tiles) mean about 9,900 box tests per ray, about 10^10 per frame
for the gathers alone. If all of the forest's extra per-ray cost were this loop, it would be
about 15 ps per instance test, which is plausible for uniform, cached loads. The foliage's own
triangles add some (rays through a canopy do descend into its cards), so this is an upper bound
for the loop's share, not a split. Outpost's instance count is not logged (`IndirectStats` has
no instance count); a scene of a few thousand placements would put its 45-60 ms on the same
line.

## Options, cheapest exact first
1. **A top-level BVH over the instances' world boxes** (built on the CPU when transforms change,
   in the same BVH4 node format, src/gi_bvh.rs; leaves are instance indices). Every ray then
   visits O(log N) instance boxes. It returns the same closest hit and the same any-hit answer
   (only exact-distance ties between instances could resolve differently), so no visible change.
   Expected: the forest's gather from 176 ms toward the bare terrain's 13 ms plus the canopy
   triangles a ray really meets; `gi_update` and `gi_cell_shading` drop with it. It also applies
   to every other software ray on `gi_traverse`. Build cost: one CPU BVH over N boxes per
   transform change (the flat list is already rebuilt then).
2. **Merge static small instances into world-space BVHs** (the near-field `World` path already
   does this for cooked placements): removes the per-instance transform and stack restart for
   many-card foliage. Exact. Worth it only if 1 leaves the forest above budget.
3. **A per-frame gather budget in time** (`gather_cells` today counts cells): a guard against
   spikes, not a fix. A quarter of the budget cut the gather to 42 ms here but bounces converge
   four times more slowly (lag on lighting changes and after cuts): a picture risk.
4. **Distance fields or coarse proxy meshes per object** (Lumen-like): cheapest per ray in the
   long run, but occlusion becomes approximate (thin cards, contact darkening under foliage), a
   new authoring/cooking path and memory. Visible change likely; last resort.

## Recommendation
Build 1 first and measure it on the forest and the game scene (byte-identical pictures
expected), then log the instance count in `IndirectStats` and the GI log so Ara's Outpost
captures show where it sits. Expected outcome on the forest: under 20 ms of gathers; Outpost
under 10 ms if its instance count is the driver. Only then decide on 2.

Side note: with the forest walk turned 150 degrees onto the courtyard and foliage (the
`cut=turn` runs), non-RT `shading` rises from 4 ms to 200-250 ms. The software reflection rays
(shaders/reflections_software.wgsl `rs_trace`, line 41, M7-R1n) have the same shape:
per-mesh BVH2s behind a flat instance list that every ray walks. The same top level would help
there, and it is a candidate for Outpost's unexplained non-RT shading cost (33-46 ms, backlog
item 2 in docs/m7-gaps.md). Not measured further here.
