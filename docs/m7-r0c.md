# M7-R0c: cook-time cluster-LOD BVH

CPU-only implementation from `ab167bc`, for GI distant-field rays (G1/G2) and blur-sized reflection rays. Patent policy follows Islam's update in `8d864c3`. There is no GPU traversal, reflection producer, shading change or quality/default promotion in this unit.

## Representation and selection

`renderlab-asset::cluster_bvh` owns the dependency-free format and f64 diagnostic traversal. `renderlab-cook::cluster_bvh` owns construction, `cluster_bvh_cache` owns preparation/publication, and `cluster_bvh_validation` owns the CPU measurement. No dependency was added. Importers, viewer, shader reload, lighting, shadows, surface cache and U0/U1 implementations are unchanged.

The existing meshoptimizer cluster hierarchy is a DAG, not a tree. A fine group replaces *all* incoming coarse shards atomically. The cook therefore uses the existing legal-cut selector at each requested object-space error. It spatially indexes each complete cut, sharing identical cluster leaves and page vertices across levels. Zero error explicitly selects original-source clusters (`refined == None`), including coplanar geometry that a zero-error simplification could otherwise remove. Selection never mixes independent coarse/fine shards in one ray query.

| Array | Bytes per record | Contents |
|---|---:|---|
| BVH4 internal node | 80 | origin xyz (12), step xyz (12), four tagged child IDs (16), six axis-major byte4 child planes (24), four maximum child errors (16) |
| Cluster leaf | 32 | mesh, cluster, source page, source index offset, ray-index span, accumulated geometric error, material ID |
| Level/root | 24 | mesh, root, requested error, actual maximum error, legal-cut leaf-list span |
| Cut membership | 4 | leaf ID; useful for CPU brute force and cut validation |
| Position | 16 | source xyz, zero padding; vec4 storage-buffer stride |
| Index | 4 | index into position array |

All records have explicit little-endian encodings. A child ID's high bit tags a leaf; `0xffffffff` is empty. Nodes are in postorder, so internal links always point backward. Six packed u32 words hold all quantized child planes. GPU consumers must load a node as 20 raw u32 words (or five vec4 words), **not** declare two WGSL vec3 fields whose alignment would change the layout. Byte offsets are origin 0, step 12, children 24, lower planes 40, upper planes 52 and errors 64. Bounds round outward and are verified against actual geometry and both fused/unfused f32 decode arithmetic. Quantization changes only traversal bounds; triangle positions remain source f32 values.

The builder is a deterministic median split on the longest centroid axis, with cluster ID breaking ties. It recursively splits into at most four children. This is a small independent implementation informed by the quantized-wide-bound idea in [Ylitie, Karras and Laine, HPG 2017](https://research.nvidia.com/sites/default/files/publications/ylitie2017hpg-paper.pdf); it copies no NVIDIA code and implements neither their compressed traversal stack nor their SAH conversion algorithm. GPU throughput remains unmeasured.

`select_level(mesh, w, k)` returns the coarsest **cooked** level with `max_error <= k*w`; absence is explicit (`None`). `select_cone_level` accepts width and growth per ray parameter. It uses the cone's minimum width over the traced interval, so an expanding cone remains within that budget farther out. G1 can start rays beyond its near field with a nonzero minimum distance. This first implementation selects one complete cut per ray; it does not change cuts within a traversal. Transform ray and cone conservatively into object space before querying. The quarry placements scale object space by approximately 0.01.

Leaf error uses the refined group's **accumulated simplified error**, or zero for original geometry. Raw cluster culling errors are not monotone across the DAG and are unsuitable for LOD selection. Internal children and level roots store maximum descendant error. These are the existing simplifier's geometric error estimates, **not independently certified tight Hausdorff bounds**. The distance experiment below reports separate conservative surface bounds and measured ray errors. In particular, a small surface-distance error cannot bound grazing-ray hit-distance error or prevent all silhouette hit/miss changes.

The geometry is object-space and material-independent. Source cluster/page/triangle identity survives for later programmable hit shading. No BRDF, irradiance, cached program output, surface normals or lighting-model-specific fields are baked into traversal. The caller continues to own placement transforms; a runtime TLAS and moving-object updates belong to G1.

## Format and cache

The standalone `RLB\0` acceleration blob is version 1, with a 32-byte header and explicit counts checked against file length and byte policy before allocation. Structural validation checks links, spans, finite scalars, enclosure, error envelopes, reachable records and exact root/cut membership. The default cook cap is 150,000,000 bytes and the library caller can choose another cap.

Optional attachments use **RLA v4** and **RVG v3** (the existing virtual file magic is `RLV\0`). `encode_with_bvh`/`decode_with_bvh` expose the attachment without adding acceleration ownership to the ordinary `Asset`/`Package` structures. Ordinary encode remains byte-compatible RLA v1/v2/v3 and RVG v1/v2. Ordinary decode validates and discards an attachment. RVG attachments also validate source cluster IDs, pages, spans, materials and accumulated errors. An RLA attachment preserves mesh/material identity, while source cluster identity belongs to its paired RVG.

The explicit `m7-bvh` cook consumes an already-cooked virtual manifest and its immutable pages. It writes a sidecar instead of rewriting the quarry's 1.55 GB RLA or its texture-bearing RVG. Its namespace is `cluster-bvh-v1/rla<S>-rvg<S>/{keys,packages}`. Keys include algorithm identity, source SHA-256, source RLA/RVG versions, acceleration version and every requested error's f32 bits. Policy caps are checked on cache hits. SHA-256 content addressing, complete-file rename publication and manifest identity checks prevent partial output from replacing the last complete result. Neither raster cache keys nor raster manifests change.

The source package is SHA-256 verified in a 1 MiB streaming buffer, then only geometry metadata is decoded with placeholder materials. Textures are skipped, original material IDs retained. This geometry-only source is explicitly unsuitable for rendering. Selected pages are hash-verified and decoded individually. The exact CPU oracle is built one mesh at a time, avoiding a full-detail texture/geometry load.

## Validation and measurements

U1b integration: RLA v4 BVH attachments remain unchanged. Moments without a
BVH now write RLA v5; moments plus BVH write RLA v6. RVG v3 can carry the v5
resource section and its existing BVH trailer. The geometry-only sidecar loader
recognizes both sources, with separate `rla5-rvg2` keys when appropriate. The
old/new source-cache and combined-attachment tests exercise coexistence.

Measured on Windows, AMD Ryzen 7 7800X3D, 32 GB RAM, Rust/Cargo 1.96.0, source base `ab167bc` plus this unit's recorded source hashes. No resolution or GPU quality setting applies to this CPU unit. The cook binary uses dev dependencies and a cook-package opt-level of 1; this is not a release renderer measurement. Other desktop applications and U0's Unreal study remained open; launch preflight found no competing Cargo/rustc/cook/active shader compiler. These are observed cook timings, not a quiet-window comparative performance claim.

Evidence stays in `E:/renderlab-work/m7-r0c/`, outside Git: `quarry-1.stdout` holds every mesh/level result, `summary.json` the aggregate, and `*.resource.json` the launch/peak-memory records. The Windows runner requires at least 8 GiB available at launch, rejects competing heavy CPU tools, and owns an atomic CPU slot. A Windows job object now hard-caps aggregate private memory at 1.4 GiB, with a sampled working-set stop at the same threshold. No window, desktop capture or GPU call is used.

Final metadata-only lint checking uses the runner's explicit `--light` mode: a 512 MiB hard cap permits overlap with foreign compilation while retaining the 8 GiB launch guard. A broader lint attempt including the unchanged renderer dependencies exceeded that light allowance and was stopped; the clean Clippy result below is scoped to the two changed crates.

**Initial resource-policy miss:** the first opt-level-2 compiler batch used a sampled stop only; it observed 1.748 GiB aggregate private memory before terminating its own process tree. It never reached quarry validation. The guard was corrected to enforce the hard job cap, and the retained opt-level-1 cook/asset test batch passed in 71.98 s with 359.49 MiB peak job-private memory and 415.41 MiB sampled tree working set. The over-budget attempt is preserved in `cook-tests-1.resource.json`, not presented as satisfying the limit.

The quarry is the existing twelve-scan, **13,034,903 original triangles** asset, whose placements scale object coordinates by approximately 0.01. The two requested errors are 1 and 4 object units (approximately 1 and 4 cm). The combined resident array requirement is **31,054,776 bytes = 31.05 MB = 29.616 MiB**; the standalone encoded file is 31,054,808 bytes including its header. It contains 6,910 nodes, 11,913 shared leaf records, 24 level/root records, 847,672 positions and 4,127,445 indices. Positions and indices account for 30,072,532 bytes of that total.

This buffer accounting is an actual encoded/layout count, **not a GPU allocator-residency measurement**. Added arithmetically to the recorded non-RT 3,503.7 MiB baseline, it projects 3,533.32 MiB, leaving 378.68 MiB below Unreal's recorded 3,912 MiB. This fits R0c's 150 MB target. Near-field visibility, WRC, runtime traversal/instance structures, hit-shading attribute residency and allocator overhead still have to fit in the remaining M7 budget; this does not certify total integrated M7 VRAM. The sidecar preserves the source page/triangle identity needed to fetch programmable hit attributes, but stores only positions/indices itself.

| Cook stage | Observed wall time |
|---|---:|
| Streaming source verification and geometry metadata decode | 2.786 s |
| Two-level build, selected page IO/hash/decode, validation | 5.120 s |
| Cook including encode, decode round trip and publication | 5.360 s |
| Rebuild identity plus full CPU verification | 49.575 s |
| Whole quarry command | 57.723 s (guarded process span 57.780 s) |

The quarry process started with 9.066 GiB available. Peak job-private memory was **122.78 MiB**, peak sampled tree working set **112.91 MiB**; minimum observed available memory was 8.938 GiB. The exact oracle was transient per mesh. Implementation and checking are separate: initial reading/coding took approximately 25 minutes, followed by the measured verification batches; the renderer's frame time and development-loop latency were not measured here.

The fixture batch covers a sphere and an open thin sheet, three error levels, eight byte-identical rebuilds, closest hits against same-LOD brute force, zero-direction components, optional attachment round trips, malformed records/budgets, cold/warm cache reuse, corruption repair and changed settings. The quarry validation rebuilds for byte identity, then generates 10,000 GI-cell-like rays and 10,000 RF1-like reflection rays, divided deterministically across twelve meshes, and replays both populations at every cooked level against an exact original-triangle CPU oracle. Eight rays per population per mesh per level also walk every same-LOD triangle without BVH culling.

For surface distances, each source cluster's exact AABB ball `(c,r)` covers its entire triangle surface. A nearest-triangle query on the target gives a conservative directed bound `distance(c,target) + r`, by the distance function's Lipschitz property. Both directions are evaluated for every cluster. The maximum is a deliberately loose symmetric surface-distance upper bound (with diagnostic roundoff inflation), not the sampled centroid maximum. Separate first-triangle centroid samples show tighter *sampled*, uncertified errors. All distances are object-space; multiply by a conservative placement linear-operator norm for world units.

The full-detail oracle uses the original cluster triangles from the already validated virtual hierarchy, the same source-triangle domain as N0; it is not an invocation of N0's GPU shader. This tests acceleration/LOD geometry, not a final reflected or illuminated image. Inter-object rays, native RF1 images, motion, self-hit handling, GPU arithmetic, traversal cost and Unreal image-quality acceptance remain G1/reflection work.

Final checks exit 0:

- `cargo --config 'profile.dev.package.renderlab-cook.opt-level=1' test --locked -j 1 -p renderlab-cook -p renderlab-asset`: **87 passed**, including the focused nearest-triangle analytic test and new BVH tests.
- `cargo test --locked -j 1 -p renderlab --lib`: **116 passed, 113 ignored**; ignored GPU/opt-in tests were not invoked.
- `cargo clippy --locked -j 1 -p renderlab-cook -p renderlab-asset --all-targets -- -D warnings`: **clean**, no lint exceptions.
- `cargo fmt --all -- --check` and `git diff --check`: clean.

The final cook/asset test batch took 65.88 s (including incremental compilation), renderer library tests 257.49 s (including their cold dependency compilation), and final Clippy 5.10 s. The renderer test job peaked at 1,142.46 MiB private memory, below the hard cap. Its minimum observed available memory was 4.786 GiB after a compliant 8.412 GiB start; the guard does not claim system-wide memory stayed above 8 GiB throughout other applications' work.

| Requested error (object units) | Cut triangles | Actual maximum simplifier error | Maximum sampled centroid distance | Conservative symmetric surface-distance upper bound |
|---|---:|---:|---:|---:|
| 1 | 1,172,108 | 0.999753 | 1.079883 | 336.382713 |
| 4 | 203,707 | 3.988662 | 2.471843 | 334.870547 |

The surface upper bounds are loose cluster-ball bounds (approximately 3.36/3.35 m at the quarry scale), not claims of actual displacement. The sampled centroid maxima are approximately 1.08/2.47 cm and do not certify the unsampled surface. In particular, the fine-level sampled maximum exceeds its simplifier estimate; callers must not treat the estimate as a proven Hausdorff certificate.

| Error | Population (10,000 rays each) | Same-LOD brute mismatches / rays | Full-detail hit/miss changes | Both-hit distance delta mean / max (object units) | Delta exceeds cone width |
|---|---|---:|---:|---:|---:|
| 1 | GI cells | 0 / 96 | 52 | 0.378980 / 99.233959 | 140 |
| 1 | RF1-like puddle reflection | 0 / 96 | 23 | 0.285343 / 117.970272 | 47 |
| 4 | GI cells | 0 / 96 | 143 | 1.031790 / 114.396194 | 101 |
| 4 | RF1-like puddle reflection | 0 / 96 | 86 | 0.673422 / 129.431522 | 36 |

Both populations replay the same deterministic rays at each level. Reflection rays originate on a per-mesh horizontal plane with reflected pitch 20–40 degrees and deterministic perturbations for roughness 0/0.05/0.1; this is an RF1-like geometry screen, not RF1's eventual native image fixture or BRDF oracle. The comparison cone starts at four times the requested error (`k=0.25`), grows at 0.01 for GI and 0/0.05/0.1 for reflection, and is evaluated at the exact hit. Width selection is checked on each ray. Delta-over-width counts include both-hit rays only; hit/miss changes are reported separately. There are **zero acceleration-vs-brute discrepancies**, but the full-detail differences and approximately 1.29 m worst hit-distance tail remain explicit G1/reflection quality risks. No film-quality, Unreal-quality or reflection-quality pass is claimed.

## Claim-avoidance analysis

This is engineering design analysis, not legal advice. [US 10,950,030 B2 claim 1](https://patents.google.com/patent/US10950030B2/en#C1) combines per-reflecting-triangle ray construction, transforming/shading visible object triangles into that triangle, matching secondary samples to primary rays, and image compositing. This unit does none of those operations: an immutable object-space tree returns an independently queried ray's triangle and barycentrics. It has no reflector-space projection, shared ray bundle, primary/secondary matching or pixel/light-value representation.

[US 12,051,145 B2 claims 1, 14 and 20](https://patents.google.com/patent/US12051145B2/en#C1) emphasize decoupled secondary rays handled as a common projection and matched back to a primary ray. Width-based selection here controls only geometric resolution for one caller-provided ray; it neither groups reflector triangles nor builds a common projected reflection. The proposed later reflection consumer keeps each independently sampled ray tied to its originating shading query, traces scene geometry directly and evaluates the hit's own program. It should preserve these differences rather than add projected triangle batches or decoupled sample matching. GI rays originate at world cells and do not form the claimed reflection construction either.

These missing combination elements motivate reuse of the BVH for both GI and blur-sized reflections under the updated policy. They do not clear the entire Adshir/Snap family or an eventual integrated renderer. No patented fallback or measured-failure exception is used here; later reflection integration needs its own claim-avoidance record and image evidence.

## Reproduction

The explicit opt-in cook command, using an already prepared virtual package:

```powershell
cargo --config 'profile.dev.package.renderlab-cook.opt-level=1' test --locked -j 1 -p renderlab-cook -p renderlab-asset --target-dir E:/renderlab-work/m7-r0c/target
py scripts/m7-r0c-run.py --label quarry -- E:/renderlab-work/m7-r0c/target/debug/renderlab-cook.exe --limits <quarry-limits.json> m7-bvh <source-cache> <source-virtual.manifest> E:/renderlab-work/m7-r0c/cache E:/renderlab-work/m7-r0c/quarry.bvh.manifest 1,4 --validate
```

The first Cargo command itself was also launched through the CPU guard. `--validate` requests the explicit rebuild, CPU ray screen and surface-distance calculation; without it the cook performs only format/identity validation and cache publication. The guard exits with a deferral when the start-memory or competing-job condition fails. It neither runs the renderer nor acquires a GPU lease.
