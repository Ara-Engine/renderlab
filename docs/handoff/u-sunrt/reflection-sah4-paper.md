# Reflection BLAS: GI binned-SAH BVH4, CPU feasibility paper

CPU/paper only. No renderer source edits, build, GPU job or counter instrumentation. Code reference: Renderlab0b3f1698. The CPU sample harness imports NumPy only and reads owned cache objects.

## Runtime packing inventory: what is known and missing

The existing files do NOT contain a final prepared-draw export or the packed reflection buffer. The authored/cooked inventory is not promoted to a runtime inventory. Exact final BLAS/instance counts, terrain/foliage admitted totals, batch mesh triangles and runtime TLAS SAH remain unavailable under the no-renderer-edits constraint.

Runtime observations from owned player logs remain:144 terrain tiles;136 source draws merged into10 world-space static material batches;5234 foliage parts omitted under a2M-triangle budget and15558-part ceiling. Actual retained foliage count is unlogged. The resolved authored baseline is629 versus74 primitive placements; two missing drone sources become placeholders at runtime. Source primary_ray_gpu.rs packs one instance per nonsingular model in software-BLAS-enabled draws, deduplicates meshes by Arc identity, then adds/refits the median instance BVH. This cannot be reversed exactly from the aggregate memory log.

The smallest missing-data seam is specified in runtime-packing-export-proposal.md. It is CPU-only and does not instrument traversal. It has NOT been implemented because root requested no renderer edits.

## Layout and code change

Use near_field::build_bvh_sah on the reflection mesh's ORIGINAL full-detail drawn positions/indices. It uses16 centroid bins per axis, chooses area*count planes, falls back to median if bins cannot separate, and performs two split rounds for up to4 children with8 triangles per leaf. Do not substitute the current GI packed buffer: it may be a merged world or a near/far-field cut and is not necessarily the reflection geometry.

Replace the reflection BLAS median-node format (64 bytes, two float AABB children) with the GI runtime node format (64 bytes: origin/step,4 children,6 packed bounds words), and an8-byte leaf(offset,count). The source asset Node is80 bytes/Leaf32 bytes; those CPU structs are NOT the compact uploaded sizes. GI's outward quantization is needed to enclose both fused and unfused f32 decode paths.

The first compatibility-oriented option keeps today's48-byte precomputed triangle(edge/primitive) payload. Leaves index a4-byte triangle-record permutation. This preserves the original primitive and current intersection arithmetic instead of changing simultaneously to indexed-position triangle construction. The existing GI builder must expose a per-triangle source permutation: Leaf.source_index_offset is only the first source triangle, and SAH leaves need not be contiguous in source order. Reconstructing identity from vertex-index triples is ambiguous for duplicates.

rs_instance would decode/test up to4 child bounds, order entries near-to-far, push eligible children, then load leaf record IDs and use the existing triangle intersection code. Keep the top-level instance BVH unchanged initially. A BVH4 needs a reviewed stack bound: up to3 siblings can be pending per level; do not assume32 entries covers every unbalanced SAH tree. Build-time max pending depth or a safe fallback is needed. Shaders inline and pass share rs_trace; changes belong to U-RTVIS. H's48-byte records, original placement/primitive/barycentrics, bindings64/65 and hit shading stay unchanged. No material-model restriction is introduced.

## CPU sample comparison

Six real cached mesh artifacts: the two largest and median-by-triangle-count cache meshes per scene (ties/order explicit in sample.json). These are NOT a scene-weighted selection, do not include final world batches/terrain/foliage placements, and are not the requested full runtime ray replay. Each has96 deterministic rays:48 rays starting just above sampled triangle surfaces and pointing outward in a broad glossy-like hemisphere, plus48 rays aimed at sampled surfaces from outside its box. These are synthetic reflection-like BLAS-entry rays, not captured game reflection directions.

Both builders use the same decoded finest-LOD triangles from ARAF Mesh. CPU binned partitioning mirrors16-bin/two-round GI rules. The CPU model uses f64 intersection arithmetic and a conservative approximate8-bit outward-bound decode (one ULP wider); it is NOT a bit-exact Rust/WGSL port. It measures structural visits, not GPU speed. BLAS only: no TLAS, per-ray instance transform, hardware occupancy, divergence or stack spilling is simulated.

| Scene | Triangles | Ray family | Median2 nodes/ray | SAH4 nodes/ray | Median2 boxes/ray | SAH4 boxes/ray | Median2 triangles/ray | SAH4 triangles/ray |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| outpost | 113,154 | surface_outward | 60.27 | 23.02 | 120.54 | 86.52 | 32.06 | 32.27 |
| outpost | 113,154 | toward_surface | 55.31 | 20.79 | 110.62 | 79.27 | 23.42 | 21.81 |
| outpost | 101,802 | surface_outward | 22.77 | 9.31 | 45.54 | 36.00 | 8.42 | 12.50 |
| outpost | 101,802 | toward_surface | 29.23 | 13.48 | 58.46 | 52.02 | 12.69 | 16.42 |
| outpost | 1,158 | surface_outward | 37.38 | 8.27 | 74.75 | 29.73 | 23.83 | 17.94 |
| outpost | 1,158 | toward_surface | 38.92 | 8.06 | 77.83 | 29.60 | 25.79 | 16.73 |
| showcase | 30,830 | surface_outward | 48.69 | 14.73 | 97.38 | 55.35 | 27.23 | 20.25 |
| showcase | 30,830 | toward_surface | 41.85 | 14.75 | 83.71 | 56.29 | 21.21 | 18.44 |
| showcase | 24,178 | surface_outward | 44.29 | 14.85 | 88.58 | 54.88 | 22.65 | 26.27 |
| showcase | 24,178 | toward_surface | 42.67 | 14.65 | 85.33 | 54.88 | 17.85 | 21.69 |
| showcase | 4,510 | surface_outward | 21.69 | 7.02 | 43.38 | 25.98 | 14.17 | 11.44 |
| showcase | 4,510 | toward_surface | 25.50 | 8.71 | 51.00 | 32.46 | 14.65 | 12.00 |

Equal-weight sample mean: Outpost median2/SAH4 internal nodes40.65/13.82, box tests81.29/52.19, triangle tests21.03/19.61. Showcase nodes37.45/12.45, boxes74.90/46.64, triangles19.63/18.35. Fewer wide-node visits do not equal the same factor less arithmetic: each node can test four boxes, and an8-triangle leaf can do MORE triangle tests. Some sampled meshes do show that trade-off.

This sample favors SAH4 structurally on both scenes, but does not expose a12x Outpost-specific BLAS visit cost. It therefore cannot settle the runtime cause or justify a GPU speed claim. Final runtime batches/TLAS and actual ray origins/directions remain the important missing inputs. Python build seconds are retained only for reproducibility, not as estimates of Rust startup cost.

## Hit identity and ties

576 sample rays had zero distance/miss mismatches and zero primitive changes in this CPU model. This is not general identity proof. A saved synthetic coplanar-overlap test gives median2 primitive0 versus SAH4 primitive6, both at t=1.0. See cpu-builder-sample/tie-counterexample.json.

Today rs_limit preserves lower-instance-index ties (using next-up t bounds) across instances. Within a mesh, strict t<best retains the first equal-distance triangle encountered. SAH changes that encounter order. Near shared edges, slacks and floating-point arithmetic can also expose ties. A new stable primitive-ID tie policy may be sensible, but it is a behavior change relative to today, not guaranteed byte identity. Preserving the old winner for every tie could require a rare baseline-traversal replay and retained baseline nodes, with memory/cost implications; it must not be hidden in an exactness claim.

Use the existing precomputed edge payload to avoid additional intersection-rounding changes in the first comparison. Keep original primitive IDs and barycentric winding. Require CPU adversarial overlap/shared-edge/instance-tie checks, then actual shader image and record comparison on both paths after authorization. Neither GI's existing hit normal nor its triangle identity mapping may replace the reflection hit contract.

## Memory

Current uploaded BLAS bytes =64*N2 +48*T. Indexed-position GI-style reflection candidate =64*N4 +8*L4 +12*V +12*T +4*T source-primitive map. Compatibility payload candidate =64*N4 +8*L4 +4*T permutation +48*T old triangle data. TLAS/instance records and shader hit buffers are unchanged. No claim that two candidate layouts coexist for free; switch-off should allocate only baseline. Reusing GI geometry is not assumed.

| Scene / mesh triangles | Current bytes | Indexed-position candidate bytes | Compatibility-payload bytes |
| --- | ---: | ---: | ---: |
| outpost / 113,154 | 7,528,480 | 3,462,536 | 6,700,448 |
| outpost / 101,802 | 6,983,584 | 2,928,420 | 5,962,176 |
| outpost / 1,158 | 80,480 | 43,232 | 68,864 |
| showcase / 30,830 | 2,004,064 | 939,136 | 1,821,448 |
| showcase / 24,178 | 1,684,768 | 718,984 | 1,425,736 |
| showcase / 4,510 | 308,448 | 134,148 | 265,920 |

These are geometry-buffer estimates on the sampled meshes, excluding allocator alignment, CPU build temporaries, runtime batches and any retained old BVH for tie replay. Source BVH positions use16-byte CPU records and CPU nodes/leaves80/32 bytes, so peak CPU memory differs. Indexed-position estimates save more but also change data access and potentially f32 operation ordering; compatibility payload saves less while preserving intersection inputs.

## One paragraph on the reflection far bound

A conservative per-ray exit bound from the union of ALL traceable world instance AABBs can replace min(1e6, next-up padded exit distance) without changing pictures, provided moving/skinned/off-screen geometry and transform rounding are covered, and a hit exactly on the boundary is not lost to strict t<tmax. A camera far plane, current visible bounds or an arbitrary shorter distance cannot give that guarantee: they may delete distant/off-screen reflections or emitters. In these scenes the geometry already lies far inside1e6 and BVH traversal skips empty space instead of marching to tmax, so a safe scene bound may save little or no traversal work. It is not an established explanation of the12x gap. No typical game reflection-ray length is logged yet.

## Recommended sequence

1. Review/authorize the CPU-only final packing export; no shader step counter yet. Replay exact final meshes, instances and camera/receiver samples before deciding on BVH4.
2. Resolve primitive-tie semantics before promising exact identity. A source-permutation-preserving compatibility layout is the lower-risk first design, but order ties still need a decision.
3. If the exact CPU replay still cannot explain the measured slope, use the already-proposed measurement-only traversal-step counter through root/U-RTVIS. No build or implementation has started.

## Reviewed leaf-size comparison (CPU sample only)

Same six cache artifacts and96 synthetic rays per mesh as above. Four-triangle leaves are now included beside8; this is still not real-ray/runtime-AS evidence. Both sets had0 distance/miss mismatches on the sampled rays; synthetic order-tie counterexample remains valid.

| Scene | SAH4 leaf triangles | Nodes/ray | Box tests/ray | Triangle tests/ray |
| --- | ---: | ---: | ---: | ---: |
| outpost | 4 | 15.36 | 57.95 | 11.20 |
| outpost | 8 | 13.82 | 52.19 | 19.61 |
| showcase | 4 | 13.81 | 52.11 | 10.35 |
| showcase | 8 | 12.45 | 46.64 | 18.35 |

Four-triangle leaves trade more boxes and nodes for fewer triangle tests. Python construction time is not a Rust startup estimate. No leaf choice is promoted before real-ray replay. Root-approved candidate tie policy is now explicit: equal t uses the lower ORIGINAL primitive ID, preserving the current across-instance tie policy. It is a declared switch-on behavior change; switch-off remains byte-identical, and switch-on must be stable run to run with pixel counts/crops. The future CPU builder must report/limit maximum pending stack entries and never drop nodes. U-RTVIS owns the shared rs_trace implementation behind the same single default-off candidate switch.
