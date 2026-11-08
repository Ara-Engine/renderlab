# M7-R1n: software reflection rays on the non-RT path

Overnight unit (Islam's complete-base push, 2026-11-08; rules in AGENTS.md). Branch `t3/m7-r1n`, from main `ffdaee9` (R1 merged). Inputs: [R1](m7-r1.md) (the RT design and its hooks), [R0c](m7-r0c.md) (cook-time cluster-LOD BVH), [P0](m7-p0-claims.md) §4.11 (R1n's claim notes), [G1d](m7-g1d.md) (the GI cache's software traversal and merged world BVH), [R0a](m7-r0a.md) and its floors. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## Status

RESULTS_TABLE

## 1. Design

### 1.1 What is reused from R1

The non-RT path runs R1's reflection block (`shaders/reflections.wgsl`) unchanged except for its two ray queries:

- the same program ABI: `reflection_lobe` declares the GGX lobe, `specular_incident` returns `vec4` (rgb the mean exposed radiance over the lobe, w the traced fraction);
- the same bands: one mirror ray below roughness 0.05, four fixed VNDF strata to 0.35, the traced fraction fading to 0 at 0.5;
- the same per-pixel azimuth hash, turned by TR1's frame rotation word, so TR1 (on by default) integrates the glossy strata;
- the same ray-cone footprint, the same exact-epsilon origin on the viewed side of the receiver's plane;
- the same hit reconstruction (`rl_hit_input`): the hit's `surface()` and `shade()` run with its own material parameters, view vector −d, ray-cone uv and normal differentials, and DC1's decals through `decal_prepare`;
- the same constraints: one bounce (`specular_incident` is 0 at hits), hits read the open environment, hit light visibility is one hard ray.

`src/program.rs` composes this software variant when the head declares `rl_geometry` (only non-RT shading heads do) and the program declares both halves of the pair. It replaces the two ray-query blocks by text seams (asserted to match exactly once):

| R1 (RT) | R1n (non-RT) |
|---|---|
| `rayQuery` closest hit, opaque, both faces | `rs_closest(origin, d, epsilon, RL_FAR)` → the same `(placement + 1, primitive, barycentrics)` record |
| `rayQuery` first hit toward the light | `rs_blocked(origin, l, epsilon, tmax)` |

RT modules are byte-for-byte what R1 composed: the software text is added only on heads that declare `rl_geometry` (CPU test `software_reflection_composition`). Programs that name neither hook still get no reflection text on any head.

### 1.2 Geometry: why not R0c's LOD BVH or G1d's merged world, yet

The brief named R0c's cooked cluster-LOD BVH, chosen by ray-cone footprint, or G1d's merged near-field world BVH. Neither can feed R1's hit contract today:

- **Hit shading needs the drawn triangle.** R1 reconstructs a hit from `(placement, primitive, barycentrics)` in the draw's own index and vertex buffers, then runs the program with that triangle's normals, uvs, tangents and colours. G1d's world BVH is built from transformed far-field cuts and keeps only positions; its hits map to a placement's material evaluated over its whole texture (`gi_hit_primitive: none`). R0c's full-detail build (`near_field::build_bvh`) reorders triangles inside leaves and keeps only the first triangle's source offset. R0c's cooked cut keeps source cluster and page identity, but only the quarry has a cooked cut, and its LOD triangles are not the drawn triangles of a create-mesh draw.
- **RF content is small.** Every RF fixture mesh is a few hundred to a few hundred thousand triangles, where a full-detail tree is cheap and exact.

So R1n builds its own per-mesh BVH (`src/reflection_software.rs`), with R0c's and G1d's properties where they apply (object-space per-mesh trees, never refit, a flat instance list rebuilt on the CPU, global-first traversal per ray) and the one property they lack (drawn-triangle identity):

- **BVH2 over the drawn triangles**, median split on the longest centroid axis (deterministic; ties break by triangle index), at most four triangles per leaf.
- **Nodes** are 16 words: the two children's float boxes and their tagged IDs (`LEAF | first << 3 | count`).
- **Triangles** are 10 words: corner a, edges b−a and c−a in index-buffer corner order, and the primitive index. Möller–Trumbore's (u, v) are then the weights of corners b and c, exactly the hardware query's barycentrics, so `rl_hit_input` is unchanged.
- **Built once at upload** on non-RT visibility-buffer renderers, for meshes up to `MAX_TRIANGLES` (2^19). 56 bytes a triangle in all. Larger meshes get none and are not hit by reflection rays (gap 2).
- **Per frame** (`prepare_software_reflections`, in the traced/visibility-buffer frame preparation): every distinct drawn mesh's BVH is packed once into one storage buffer behind a 24-word instance record per traced placement (world-to-object rows, a world box, the mesh's bases, its root and placement slot). The buffer is repacked only when the drawn meshes or the instance count change; transform edits rewrite only the records. Group 0 binding 36 of the non-RT shading layouts (no new bindings on RT layouts).

LOD selection from the ray cone (R0c's `select_cone_level`) is not used: full detail is the only level for create-mesh geometry. It becomes useful once large meshes carry a cooked cut with drawn-triangle identity (gap 2).

### 1.3 Traversal (`shaders/reflections_software.wgsl`)

Each ray is its own work item. It loops over the instance records, rejects an instance by its world box, transforms the ray into object space (unnormalised, so t is shared), and walks the BVH2 with a 32-entry stack, nearer child first. Closest hit for reflection rays; first hit for light visibility. Both faces block, as R1's opaque queries.

### 1.4 Ownership and shared edits

New files: `src/reflection_software.rs`, `src/reflection_software_tests.rs`, `shaders/reflections_software.wgsl`, `scripts/m7-r1n-run.py`. Marked shared edits (`M7-R1n`):

- `src/program.rs`: the software branch of `reflection_api` and its seams;
- `src/primary_ray_gpu.rs`: the head declaration, binding 36 in the non-RT shade layouts and bind groups, the bindings key's generation word, `SoftwareReflections` and its preparation;
- `src/mesh.rs`, `src/renderer.rs`, `src/virtual_gpu.rs`: the mesh's `reflection` field and its upload-time build;
- `src/reference_r0a_tests.rs`: opt-in `RENDERLAB_R1N_PROGRAM` (environment-only baseline) and `RENDERLAB_R1N_TIMES` (timed frames) for the RF A/B.

The GI cache (G2b's) is not touched.

RESULTS_SECTIONS
