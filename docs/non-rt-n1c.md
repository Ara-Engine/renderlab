# N1c: clusterLOD-cut primaries for the no-RT visibility buffer

2026-11-06, base `59bbb6f` (N1b), branch `t3/n1c-cluster-visibility`. Labels: **M** measured here, **I** inference. **Primary visibility meets its cost target. The full-frame target is not met: the shadow pass alone exceeds it.** On the quarry, with RT features disabled on the device, N1c's primaries cost **0.386-0.404 ms** at the chosen bound (clean visits, M). That is against N1b's 2.53 ms and Unreal's Nanite visibility pass at 0.379 ms. Shadows and the final image stay above Unreal VSM on every measured quantity with N8b's lookup, with a thin margin on one. The complete no-RT frame misses VSM's 1.61 ms: 2.24-2.27 ms with N8 on main and 6.44-6.50 ms with N8b. Shadow cost is the limiter there (1.51 and 5.7 ms). This is not film quality.

## Chosen error bound and its effect

- **Bound: 0.35 px** of projected simplification error (meshoptimizer clusterlod's estimate, conservative affine scale), `Renderer::set_lod_error_pixels`. Negative selects original geometry only. Meshes without an attached hierarchy always draw original geometry.
- **LOD receivers are lifted by 1x their cluster's simplification error** toward the viewed side (`set_lod_receiver_lift`, default 1). Shadow casters remain full-detail source triangles. A coarse surface can lie below them, and N8b's exact lookup then shadows it. Without the lift, N8b + LOD gives 14,162 lit-darkened receivers at 0.25 px; with it, 19 (M). Lifts above 1 were worse (§Quality).
- **Visible effect (M, inspected):** no holes. No coverage change lies more than 1 px from the traced silhouette (19-20 changed pixels per frame at 0.35 px). 36% of covered pixels land on coarse triangles; the rest stay on original source triangles with their exact receivers. On LOD pixels receivers move up to about 3.1 mm (p99), against a pixel footprint of about 6.5 mm. The x16-amplified difference against traced primaries shows fine per-pixel speckle from shading normals and shadow samples on LOD surfaces, concentrated on the ground and far rocks. Film-image 8-bit code RMS rises from 2.5765 to 2.6001 at pose 0 (+0.9%) and from 2.6146 to 2.6413 at pose 1 (+1.0%). At 0.25 px the image is indistinguishable from full detail (2.5764); at 0.5 px it rises 2.5% and the speckle covers more of the ground.

## Implementation

**LOD data.** The quarry's cooked virtual package (`f33547c5...`, 214,062 clusters, 13,126 groups, 16,339 pages) was built by meshoptimizer clusterlod. Its simplification collapses onto existing vertices, so every DAG triangle indexes its source primitive's vertex buffer. `LodPackage::open` reads only the package metadata, skipping its 1.07 GB of embedded textures. `LodPackage::mesh_lod` reads and SHA-256-verifies each page and maps every page vertex to its source vertex by exact bits. Original-geometry triangles keep their source primitive IDs (all 13,034,903 matched, 0 unmatched). The 13,017,435 coarser triangles become primitives T..T+L of the same mesh. `Renderer::try_attach_lod` copies the index buffer on the GPU into `[3T source | 3L LOD | 3(T+L) neighbours | pad]`. `Mesh::index_count` stays 3T, so forward raster, BLASes and N8/N8b casters draw source triangles unchanged. LOD neighbours are exact-position shared edges within a cluster. Edges across clusters depend on the cut and are marked open.

**Per frame.**

1. One flat GPU pass over all cluster instances applies clusterlod's rule. A cluster is drawn when its owner group's projected error exceeds the bound and its refined group's does not. The pass then culls the frustum and tests the previous frame's HZB with the **previous** camera. Pass-1 rejects go to a list.
2. Clusters whose projected box is at most 32 px and entirely in front of the near plane are rasterized in software. Everything else uses hardware with native clipping. Both write `max(depth << 32 | cluster << 7 | triangle)` into one 64-bit buffer.
3. The software raster runs one 128-thread workgroup per cluster. Its at most 128 vertices are projected once, from contiguous exact-copy positions. Then one thread rasterizes each triangle with 8-bit subpixel snapping and exact integer edge functions. The inclusive edges cannot crack between software triangles.
4. The hardware depth test is GreaterEqual, so equal-depth ties reach the 64-bit max regardless of draw order.
5. A half-resolution, power-of-two min-depth pyramid lets pass 2 retest only the rejects. Each pyramid is skipped on the GPU when nothing reads it.
6. Resolve maps the hit back to (placement, primitive), with LOD triangles in the same index space. It reads that triangle's exact positions from the cluster data, which follows the index buffer's corner order and bit-equals `primary_triangle_vertices`. Then it applies N1b's unchanged pixel-centre re-intersection, edge repair, receivers and hit ABI.
7. Resolve zeroes the hits it reads, and a reset dispatch zeroes the counters, so the next frame needs no buffer clears.

Final visibility never depends on the previous frame: for each of 24 configuration/pose changes, the first frame's hit records equal the settled frame's byte for byte (M).

**Programmability.** Nothing lighting-specific was added to hit records, clusters or dispatch. Shading reads LOD triangles through the same `(placement, primitive, u, v)` and the source vertices' own attributes. The lift moves only the shadow receiver, geometrically, by the recorded simplification error. User shading programs are unaffected. Standard and toon programs, reload and moved/returned instances pass the fixture with validation active (M).

**Requirements and constraints.** `VisibilityBuffer` now also needs `VISIBILITY_RASTER_FEATURES` (`SHADER_INT64 | SHADER_INT64_ATOMIC_MIN_MAX`). A device without 64-bit storage max atomics no longer selects it; N1b's hardware-only raster was not kept as a fallback. Each mesh's cluster data stores exact f32 positions per cluster vertex. Estimated extra GPU memory on the quarry is about 0.6 GB (I): LOD index triples and neighbours of about 312 MB, plus positions, local triangles and leaf ID tables. No Bevy, Nanite or other third-party source was copied; the design follows [reuse-non-rt](research/reuse-non-rt.md).

## GPU cost

Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, wgpu/naga 30.0.1, development test profile (renderer opt-level 1, `-j 2`). Each device was created without `EXPERIMENTAL_RAY_QUERY` or `EXPERIMENTAL_RAY_HIT_VERTEX_RETURN` and without AS limits; validation was off for timing. The quarry was loaded once per process at 1920x1080, 1x, with Virtual shadows. Each visit had 32 pipelined warm frames and 8 measured; stage times are the median of 16 repeats in one submission. Binary SHA-256: `fb539cae716acf4c84237861f34c6cc3278e5959d2fe07c66571b9302f4ebbba`.

**Contention.** The live wallpaper renderer `mypaper-wallpaper.exe` intermittently used about 34% of the SMs and 42% of memory bandwidth (`nvidia-smi pmon`, M). Affected visits run about 2x slower in every stage, and no foreign process was stopped. "Clean" below is the range of visits whose stages match the uncontended profile. Contended medians are in `timing-summary.json` and reach 0.69-0.89 ms.

| Quarry, no-RT device | Primary, clean (ms) | Drawn clusters / triangles | Complete no-RT frame, clean (ms) |
|---|---:|---:|---:|
| N1b full detail (recorded) | 2.528-2.605 | 60,397 / ~7.7 M | 4.70-4.73 |
| N1c original geometry only | 0.516-0.534 | 54,827 / 6.94 M | 2.80-2.96 |
| N1c 0.25 px | 0.438-0.457 | 37,997 / 4.72 M | 2.47-2.61 |
| **N1c 0.35 px (default)** | **0.386-0.404** | **31,518 / 3.88 M** | **2.24-2.27** |
| N1c 0.5 px | 0.355-0.371 | 26,182 / 3.20 M | 2.08-2.11 |
| Unreal Nanite visibility, MaxPixelsPerEdge .25 (recorded) | 0.379 | 22,639 / 2.85 M | VSM frame 1.61 |

Stages at 0.35 px, clean visit:

| Stage | ms |
|---|---:|
| Cut + cull (pass 1 / pass 2) | 0.044 / 0.009 |
| Software raster | 0.129 / 0.001 |
| Hardware raster | 0.005 / 0.001 |
| HZB | 0.028 / 0.002 (skipped) |
| Resolve | 0.122 |
| **Sum** | **0.342** |

The 0.387 ms span adds about 0.045 ms of pass transitions. In static frames pass 2 draws nothing. The software raster takes 6.9 M triangles in 0.21 ms at full detail.

**Paired combined run (M).** My branch with N8b merged locally (not pushed, quiet visits) gave primaries of 0.390-0.392 ms at 0.35 px and 0.518-0.526 ms at full detail. The frame is 6.44-6.50 ms against 7.17-7.30 ms, with N8b shadows at 5.70-5.76 ms.

**Full frame vs the 1.61 ms VSM target: missed.** With N8 the frame is primaries 0.39 + shadow 1.51 + shading/tone map about 0.34 ms. Only the shadow pass exceeds the target alone. N8b improves quality but costs 5.7 ms. No shadow speedup was attempted here; those files belong to N8b.

**S1/S2/S3, original geometry only (no package), no-RT, M:**

| Bench | Primary (ms) | Frame (ms) | N1b primary (ms) | Traced-identity agreement |
|---|---:|---:|---:|---|
| S1 | 0.147-0.151 | 0.60-0.69 | 0.306 | 99.994-99.997%, all differences classified edge/vertex ties |
| S2 | 0.145-0.147 | 0.63-0.64 | 0.301 | 99.998-99.9996% |
| S3 | 0.208-0.212 | 0.99 | 0.401 | 99.668% |

These match N1b's agreement exactly.

**Startup (M, development build).** Building 12 hierarchies took 7.2 s wall, four meshes at a time: page read and verify 6.3 s summed, build 19.1 s summed. Attach took 1.5 s. Upload took 10.6 s and includes original-geometry clusters for every mesh, which attach replaces. No optimized-build or cached-hierarchy startup was measured; a cached sidecar would remove the page pass (I).

## Quality against references

High-sample visibility: the mean of 64 independent 256-ray Exact frames (16,384 rays/pixel), on pixels both producers cover. The film column is 8-bit code RMS against the archived 1024 x 32 film radiance PNG. Unreal rows are the recorded native VSM results ([N8](non-rt-n8.md)). Pose 1 is within a few percent of pose 0 on every count (`summary.json`).

**With N8b's exact lookup** (local merge of `origin/t3/n8b-virtual-shadow-quality`, pose 0):

| Configuration | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges | Film |
|---|---:|---:|---:|---:|---:|---:|
| Traced control | .1985 | 504 | 2,462 | 0 | 491 | 2.5712 |
| Original geometry | .1985 | 510 | 2,466 | 3 | 494 | 2.5764 |
| 0.25 px, no lift | .2300 | 14,275 | 2,858 | 14,162 | 669 | 2.5768 |
| 0.25 px, lift 1 | .2863 | 2,458 | 5,306 | 19 | 2,412 | 2.5764 |
| **0.35 px, lift 1** | **.3125** | **3,196** | **5,959** | **66** | **3,104** | **2.6001** |
| 0.35 px, lift 1.5 / 2 | .3254 / .3354 | 3,643 / 4,047 | 6,345 / 6,659 | 84 / 103 | 3,530 / 3,904 | 2.6001 |
| 0.5 px, lift 1 | .3724 | 4,846 | 7,589 | **143** | 4,680 | 2.6419 |
| Unreal VSM | .483 | 12,390 | 14,935 | 87 | 11,884 | — |

At pose 1, 0.35 px with lift 1 gives .3083 / 3,230 / 5,990 / **70** / 3,139 against VSM's 88 lit darkened. **0.35 px beats VSM on all five; 0.5 px misses lit darkened.** Against original geometry with the same lookup, 0.35 px costs about 6x the flips and lost edges and 2.4x the leaks. It remains far from film quality, which is the trade the floor allows.

**With N8 on this branch's base** (main):

| Configuration | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|
| Original geometry | .2975 | 6,498 | 2,559 | 8,521 | 908 |
| 0.35 px, lift 1 | .3307 | 4,307 | 5,244 | 3,618 | 2,563 |

0.35 px is above VSM on four measures. On lit darkened, N8's known texel-thickness miss stands, worse than VSM even at full detail; LOD with the lift lowers it from 8,521 to 3,618.

## Coordination with N8b

The shared LOD layout was agreed through `gpu-schedule.md`. N8b addresses primitives through the shared index buffer and adjacency rule, so its walks accept LOD primitives unchanged. N8b did not add caster LOD in its unit. Casters rasterized from the same cut would put receivers and casters on one surface and remove the need for the lift. They would also make the LOD-receiver flips, leaks and lost edges above recoverable (I). The cut is one flat GPU pass over the cluster table, so a shadow pass can reuse it with its own frustum.

## Misses and limits

- Complete no-RT frame ≤ 1.61 ms: **missed** (2.24 ms with N8, 6.44 ms with N8b; the shadow pass alone exceeds the target).
- Primaries ≤ about 0.4 ms: met at 0.35 px in clean visits only, and missed under the wallpaper contention. 0.25 px (0.44-0.46 ms) and original geometry (0.52-0.53 ms) miss.
- Film quality: not met. The lit-darkened margin over VSM with N8b is thin (66-70 vs 87-88) and measured on one scene and two static poses. Motion is covered only by byte-identity of first changed frames, not a scored motion-film suite.
- LOD pixels lose the µm receiver guarantee of original geometry (lift up to about 3 mm). Their repair treats cross-cluster edges as open. The cut uses a flat test (0.044 ms over 214 k clusters), not Nanite's hierarchy.
- The package file's own digest is not rehashed (its 1 GB of textures are skipped); every page is verified. Requires 64-bit storage atomics. Tested on one GPU/driver.

## Validation and evidence

- `cargo test -p renderlab --lib`: **105 passed, 0 failed, 102 ignored, exit 0**. fmt and Clippy (`-D warnings -A clippy::nonminimal_bool`) pass. New CPU tests cover cluster partition/caps/positions, LOD neighbours and rotation, and naga validation of every composition with no ray-query declarations.
- The GPU fixture passes with validation active on a device without RT. Quarry quality and timing runs exit 0. 24/24 first-changed frames equal settled frames, and hit records are byte-identical across the cost changes.
- Every GPU run used `scripts/n1c-run.py` with the shared lock, `gpu-schedule.md` lines and the 4 GiB start / 2 GiB abort guard. No game was running (the Riot launcher only); no window, focus or desktop capture was used.
- Evidence is under `E:/renderlab-work/n1c/`: `final8/` (timing, stages, cull counts, `timing-summary.json`), `final/` (quality on N8 plus `summary.json`, PNGs and crops), `final-n8b/`, `r10/` and `r11/` (N8b quality and timing), `r9/` (bench agreement) and `runs/` (lease, resource and exit logs). Raw arrays over 5 MB were deleted after scoring.
- Reproduce with `scripts/n1c-batch.py`, `n1c-run.py`, `n1c-score.py`, `n1c-timing.py`, `n1c-check.py` and `n1c-cut-probe.py` (CPU cut sizes from package metadata).
- **Time:** about 10:33-12:05 UTC in total. Registered GPU/CPU verification processes took 1,058 s (17.6 min), including three failed attempts. The combined N8b build and other Cargo builds took about 12 min more. Implementation, diagnosis and scoring took the remainder, interleaved; these windows overlap and are not sequential phases.
