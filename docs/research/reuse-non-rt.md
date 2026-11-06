# Reusable code and algorithms for the non-RT path

Research date: 2026-11-06, base `c2b4def`. Scope: open-source code and published algorithms Renderlab could copy (with attribution) or reimplement for the **non-hardware-RT** path: virtual geometry and visibility buffers, non-RT shadows, software ray tracing on compute, and non-RT GI. Method: web reading plus local reading of [non-rt-path.md](../non-rt-path.md), [m6-path-to-2x.md](../m6-path-to-2x.md) and [industry-rendering-costs.md](industry-rendering-costs.md). No build, GPU run, window or commit.

Labels:

- **V**: verified in this pass from the cited URL. Licenses are verified from the repository's LICENSE file unless marked otherwise.
- **A**: author-reported number, not reproduced here.
- **I**: inference by this note. Treat it as a hypothesis for a probe, never as evidence.

Verdicts:

- **COPY**: permissive license (MIT, Apache-2.0, BSD, zlib, CC0, Unlicense). Code may be copied with its notice.
- **REIMPLEMENT-ONLY**: paper or talk, copyleft (GPL, LGPL, MPL), source-available, proprietary SDK license, unlicensed code, or CC-BY-NC/ND. Implement from the description; do not paste code.
- **AVOID**: conflicts with a standing rule (screen-space core lighting, quality reduction) or with a Renderlab measurement.

**Attribution requirements, which hold for every COPY below:**

- **MIT**: keep the copyright line and permission notice with every copied file or substantial portion. A `THIRD_PARTY_NOTICES` file plus per-file headers is enough.
- **Apache-2.0**: ship the license text, keep any `NOTICE` file, and mark modified files as changed. It also carries a patent grant and a patent-retaliation clause.
- **MIT OR Apache-2.0** (Bevy, obvhs, tray_racing): pick one. MIT is the simpler obligation.
- Renderlab's own license is undecided. All COPY items here are compatible with MIT, Apache-2.0 or a proprietary license. Apache-2.0 code would conflict with a future GPLv2-only choice (I).

## Summary

| # | Item | License (V) | Verdict | What to take | Relevance to Renderlab's non-RT problem |
|---|---|---|---|---|---|
| S1 | **obvhs** (Rust BVH crate, DGriffin91) | MIT OR Apache-2.0 | **COPY**, or depend on the crate | PLOC BVH2 builder with reinsertion and pre-splits; BVH2 to **CWBVH** (80 B node) collapse; CPU CWBVH traversal as the reference oracle | **N0 builder.** It replaces the planned in-house binned-SAH builder and satisfies the "third-party builder crate needs a license check" note in N0 |
| S2 | **tray_racing** (DGriffin91) | MIT OR Apache-2.0 (README) | **COPY** | `src/rt_gpu/rt_gpu_software*.hlsl`: software CWBVH traversal with a split VGPR/LDS stack, TLAS variant, query variant. Port to WGSL | **N0 traversal kernel** and the only found software-vs-hardware measurement on a 48 MB-L2 Ada card: **5.4x** (RTX 4070 Ti, A) |
| S3 | **tinybvh** (Bikker) v1.9.1 | MIT | **COPY** (check kernel provenance, see §3.2) | `kernels/traverse_cwbvh.cl`: `traverse_cwbvh` (closest) and `isoccluded_cwbvh` (any-hit), 8-bit dequantization trick, octant ordering | Second N0 kernel reference; has an **any-hit** early-out path that maps directly onto shadow rays |
| S4 | **Ylitie, Karras, Laine 2017**, Compressed Wide BVH | Paper | REIMPLEMENT-ONLY (already implemented by S1–S3) | Node format, SAH-optimal wide collapse, octant traversal order, triangle postponing | N0 hillclimb step "quantized nodes" |
| G1 | **Bevy meshlet** (`bevy_pbr/src/meshlet`) | MIT OR Apache-2.0 | **COPY** | `cull_instances`/`cull_bvh`/`cull_clusters.wesl` (frustum + two-pass HZB), `visibility_buffer_software_raster.wesl`, `visibility_buffer_hardware_raster.wesl`, `resolve_render_targets.wesl`, `fill_counts.wesl`, `remap_1d_to_2d_dispatch.wesl`, plus `required_wgpu_features()` | **N1b culled raster visibility buffer.** Closest API match found: wgpu + WGSL dialect, 64-bit texture atomics |
| G2 | **meshoptimizer** 1.3 + `demo/clusterlod.h` | MIT | **COPY** (already cooked via `meshopt` 0.7.0) | `clodBuildHierarchy` (BVH forest over cluster groups) for GPU hierarchy culling; `meshopt_computeClusterBounds`; `partitionClusters` | N1b cluster build and the later larger-than-memory LOD path |
| G3 | **Unreal Nanite**, Karis et al., SIGGRAPH 2021 | Talk | REIMPLEMENT-ONLY | Persistent-thread hierarchy culling, two-pass HZB, SW/HW raster split per cluster, 64-bit atomic visibility buffer, material tile classification | Algorithm source for G1. Its LOD part is rejected for in-memory content |
| G4 | **niagara** (zeux) | MIT | **COPY** (GLSL reference) | `src/shaders/clustercull.comp.glsl`, `drawcull.comp.glsl`, `depthreduce.comp.glsl` | Simple, readable two-pass occlusion and depth pyramid |
| G5 | **nanite-webgpu** (Scthe) | MIT | **COPY** (reference only) | `src/passes/{cullMeshlets,depthPyramid,rasterizeSw,rasterizeHw}` (WGSL in `.wgsl.ts`) | WGSL proof that the pattern runs under WebGPU limits; weaker than G1 (16-bit depth, previous-frame HZB only) |
| G6 | **AMD SPD** (FidelityFX SDK v1.1.4, `sdk/src/components/spd`) | MIT | **COPY** | Single-pass mip/HZB downsample | HZB build for G1 in one dispatch |
| H1 | **Tiled per-triangle soft shadow volumes** (Kalbertodt 2023) | Blog + gist, **no license** | REIMPLEMENT-ONLY | Per-tile cone culling of casters, then per-pixel rasterization of projected casters into a 16x16 sun-disk bitmask (208 cells) | **Non-RT exact soft-shadow candidate**, measured on wgpu, an RTX 4070 and ~13 M triangles (A). Possible accelerator for the Exact mode, which N0 prices at 15–50 ms |
| H2 | **Frustum-traced irregular Z-buffer** (Wyman et al. 2015/2016) and **HFTS** (Story & Wyman 2016) | Papers; HFTS shipped in NVIDIA ShadowWorks (GameWorks, proprietary) | REIMPLEMENT-ONLY, deprioritized | Light-space per-pixel lists, conservative raster, 32 spp visibility masks | Pixel-exact hard shadows without rays, but 20–32 ms for 12.3 M triangles on a GTX 980 (A). Slower than a software hard ray (I) |
| H3 | **Per-triangle shadow volumes with a view-sample cluster hierarchy** (Sintorn et al. 2014) | Paper; no code found | REIMPLEMENT-ONLY, deprioritized | Exact hard shadows by culling per-triangle volumes against a view-sample tree | Same trade as H2 |
| H4 | **Virtual shadow maps + SMRT**: Unreal docs; open code in Frogfood (MIT), Timberdoodle (Apache-2.0), StratusGFX (MPL-2.0) | Mixed | **AVOID as core** (already rejected by measurement); COPY of Frogfood/Timberdoodle allowed for side uses | Page marking, GPU page allocator, hierarchical page buffer (HPB) culling | Rejected in non-rt-path.md option (b): texel aliasing and bias against mm relief. Only HPB culling is reusable |
| H5 | **Moment shadow maps, PCSS, SDSM** | Papers; code licenses unverified | AVOID as core | None | Filtered approximations of a depth map; same failure class as H4 |
| L1 | **Wicked Engine** `wiGPUBVH` + DDGI | MIT | **COPY** (HLSL reference) | `ddgi_raytraceCS.hlsl` is compiled twice, with and without `RTAPI`: one source for hardware and compute traversal | Template for "one estimator, two ray backends" applied to GI |
| L2 | **DDGI** (Majercik et al., JCGT 2019) and **RTXGI-DDGI** SDK | Paper; RTXGI under **NVIDIA RTX SDKs License** (proprietary) | Paper: REIMPLEMENT-ONLY. RTXGI code: **AVOID** | Probe irradiance + depth moments, relocation, classification | World-space GI traced through the shared ray backend; no screen-space dependency |
| L3 | **Lumen**, Wright, Narkowicz, Kelly, SIGGRAPH 2022 | Talk | REIMPLEMENT-ONLY for the surface cache and world radiance cache; **AVOID** screen traces as core | Mesh cards and surface cache; world-space radiance cache | The non-RT denominator once GI lands. Its screen traces and screen-space radiance cache conflict with the standing rule |
| L4 | **FidelityFX Brixelizer + Brixelizer GI** (SDK v1.1.4) | MIT (SDK `LICENSE.txt`) | **COPY** allowed; use for **far field only** | `sdk/src/components/brixelizer` (sparse SDF bricks), `.../brixelizergi` | Its radiance cache is filled from screen space (AMD-stated limitation), and it "loses small-scale details" |
| L5 | **Godot SDFGI** / **HDDAGI** (draft PR #86267) | MIT | COPY allowed; **AVOID as core** | `servers/rendering/renderer_rd/shaders/environment/sdfgi_*.glsl`, `environment/gi.cpp` | No dynamic occluders, lag after light changes, cascade shifts; HDDAGI is unmerged |
| L6 | **AMD GI-1.x / Capsaicin** | MIT | Reference only | Two-level radiance caching (screen probes + hash-grid cache) | Requires D3D12 Ultimate (hardware RT). Its algorithm could run on the shared software backend (I) |
| L7 | **Radiance cascades** (Sannikov) | Paper repo labelled MIT, with a CC-BY-ND image in the repo; 3D ports carry no license | REIMPLEMENT-ONLY, research watch | Cascade interval/angle scaling | 3D world-space variants are research-grade |
| L8 | **EA GIBS** (Halen et al., SIGGRAPH 2021) | Talk; proprietary tech | REIMPLEMENT-ONLY | Surfel spawning and caching | Hardware-RT design; surfels could be traced in software (I) |
| X1 | **Embree**, **HIPRT**, **jan-van-bergen/GPU-Raytracer** | Apache-2.0, MIT, MIT | COPY allowed, low priority | Embree CPU BVH8 builds; HIPRT LBVH/PLOC/SBVH (HIP); a CUDA CWBVH builder with triangle postponing | Fallback references if S1 falls short |

## 1. Virtual geometry and GPU-driven cluster rendering

Renderlab context: [non-rt-path.md](../non-rt-path.md) rejects cluster LOD for content that fits in memory (receivers would move about 7.3 mm against a 10 µm ε). It keeps **N1b**: a full-detail, frustum + two-phase HZB culled **raster visibility buffer** of the finest clusters, used only if software primaries exceed 0.40 ms. Everything below is judged against N1b first and the later larger-than-memory LOD path second.

### 1.1 Bevy virtual geometry (meshlets): COPY, the closest match

- **Source:** https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/meshlet (V). Twenty-six files. Shaders are `.wesl` (a WGSL superset with imports): `cull_instances.wesl`, `cull_bvh.wesl`, `cull_clusters.wesl`, `cull_shared.wesl`, `clear_visibility_buffer.wesl`, `visibility_buffer_software_raster.wesl`, `visibility_buffer_hardware_raster.wesl`, `visibility_buffer_resolve.wesl`, `resolve_render_targets.wesl`, `fill_counts.wesl`, `remap_1d_to_2d_dispatch.wesl`, `bindings.wesl`, `meshlet_mesh_material.wesl`. The Rust side is `visibility_buffer_raster_node.rs`, `resource_manager.rs`, `instance_manager.rs`, `pipelines.rs`, `from_mesh.rs` and `asset.rs`.
- **License:** MIT OR Apache-2.0, per the Bevy README (V). Asset licenses differ, but no assets are needed.
- **Required wgpu features** (V, `mod.rs` `required_wgpu_features()`): `TEXTURE_INT64_ATOMIC | TEXTURE_ATOMIC | SHADER_INT64 | SUBGROUP | DEPTH_CLIP_CONTROL | IMMEDIATES`. The plugin is Vulkan and Metal only, requires little-endian hosts and `Msaa::Off`. Local check of wgpu-types 30.0.1 (V): `TEXTURE_INT64_ATOMIC` needs Vulkan `VK_EXT_shader_image_atomic_int64`, DX12 SM 6.6 or MSL 3.1. `SHADER_INT64_ATOMIC_MIN_MAX` and `CONSERVATIVE_RASTERIZATION` (Vulkan only) also exist.
- **Software rasterizer** (V, `visibility_buffer_software_raster.wesl`, 189 lines): one 128-thread workgroup per cluster, vertices projected into workgroup memory, one thread per triangle. Each subgroup picks scanline or bounding-box iteration with `subgroupAny(width > 4 px)`. The u64 `(depth_bits << 32) | (cluster_id << 7 | triangle_id)` is written with `textureAtomicMax` (reverse Z). Sampling is at pixel centres.
- **Known gaps** (V, source TODOs): "TODO: Fixed-point math and top-left rule"; no near-plane clipping in the software path; at most 128 triangles per cluster (7-bit triangle id). The inclusive `>= 0` edge test double-covers shared edges and lets the atomic max choose.
- **Material path** (V, JMS55's 0.16 post): fullscreen passes resolve depth and "material depth". A background early-out doubled their speed. Clearing the R64 texture with `clear_texture()` cost 0.68 ms, so Bevy wrote a compute clear (A).
- **Status:** BVH culling (`cull_bvh.wesl`) is on main. JMS55 called culling "the biggest bottleneck" in March 2025 (A). His August 2026 post lists going "back to virtual geometry" as a possible plan, so the code is maintained but not actively developed (V).
- **What Renderlab takes:** the cull chain (instance → BVH → cluster with frustum + two-pass HZB), the software/hardware raster split and the u64 atomic visibility buffer layout, at **finest level only** for N1b. Renderlab's shading recomputes exact pixel-centre barycentrics with the shared ray-triangle test, so edge-tie double coverage is acceptable. It is classified as a tie, as N1b already plans.
- **Must change (I):** the 7-bit triangle id / 25-bit cluster id packing. 13.03 M triangles at 128 per cluster is about 102 k clusters, which fits, but the hit record needs placement + primitive. Keep the visibility buffer as `(depth, global_cluster, local_tri)` and map to `(placement, primitive)` in resolve. Add the top-left rule or accept classified ties. Convert `.wesl` imports into Renderlab's own composition.

### 1.2 meshoptimizer and clusterlod.h: COPY, already in the cook

- **Source:** https://github.com/zeux/meshoptimizer, `demo/clusterlod.h`. MIT, "Copyright (c) 2016-2026 Arseny Kapoulkine" (V). Release v1.3 (V, README install line). Renderlab cooks with `meshopt = "=0.7.0"`, which packages meshoptimizer 1.3 and vendors `clusterlod.h` (V, `crates/cook/Cargo.toml`, `docs/virtual-cooking.md`).
- **API** (V): `clodBuild(clodConfig, clodMesh, ctx, clodOutput)`, `clodDefaultConfig(max_triangles)`, `clodDefaultConfigRT(max_triangles)`, `clodLocalIndices`, `clodBuildHierarchyBound` and `clodBuildHierarchy(nodes, groups, group_count, node_width, level_count)`. The last builds one BVH per DAG level with `meshopt_spatialClusterPoints`.
- **Selection rule** (V, header): render a cluster when its group error exceeds the threshold and its `refined` group's error is at or below it, or it has none. Error propagation is monotonic (`max(prev * merge_previous, cur) + cur * merge_additive`).
- **What Renderlab takes:** `clodBuildHierarchy` for Bevy-style GPU BVH culling in N1b, and `clodDefaultConfigRT` (min = max/4 triangles, up to 256 vertices, spatial clustering) when cluster BLASes are built for the larger-than-memory path. The README notes all LOD levels must share one quantization exponent and that "Nanite uses up to 21 bits per axis for this reason" (V).

### 1.3 Nanite (Karis, Stubbe, Wihlidal, SIGGRAPH 2021 Advances): algorithm source

PDF: https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf (V, text extracted locally).

- **Two-pass occlusion:** test with the previous HZB and previous transforms; draw; build the HZB; retest the rejected set; draw the newly visible; build the full HZB for the next frame (V).
- **Persistent threads** for hierarchy culling: "25% faster on average than the naive one", with cluster culling fused into the same shader (A).
- **Software raster:** "3x faster than hardware on average compared to our fastest primitive shader implementation"; HW or SW chosen per cluster; both write the visibility buffer with 64-bit `InterlockedMax`; "The vast majority are SW rasterized in our demos"; following the DirectX rasterization rules exactly avoids cracks between SW and HW clusters (A).
- **Cost:** about 2496x1404 upsampled to 4K on PS5; "~2.5ms to draw entire VisBuffer" and "~2ms deferred material pass", with 25 M triangles rasterized out of more than a billion in the source scene (A).
- **Materials:** material depth + an 8x4 tile mask per material; culled tiles are killed in the vertex shader (A).
- **Patents:** a search for Epic patents on Nanite or virtualized geometry found none (V for the search, which is no clearance).

### 1.4 Other cluster renderers

| Repo | License (V) | API | Take | Notes |
|---|---|---|---|---|
| [zeux/niagara](https://github.com/zeux/niagara) | MIT | Vulkan 1.4, GLSL | `src/shaders/clustercull.comp.glsl`, `drawcull.comp.glsl`, `depthreduce.comp.glsl`, `meshlet.task/mesh.glsl` | Clean two-pass occlusion with draw compaction via `KHR_draw_indirect_count`. No performance numbers published (V) |
| [Scthe/nanite-webgpu](https://github.com/Scthe/nanite-webgpu) | MIT, "Copyright (c) 2024 Marcin Matuszczyk" | WebGPU, WGSL embedded in TypeScript | `src/passes/cullMeshlets`, `depthPyramid`, `rasterizeSw`, `rasterizeHw`; `_shaderSnippets/nanite.wgsl.ts` (LOD error) | No `atomic<u64>` in WebGPU, so it packs u16 depth + octahedral normal in 32 bits; previous-frame HZB, "No reprojection and no two-pass"; author has "moved on" (V). Reference only |
| [JuanDiegoMontoya/Frogfood](https://github.com/JuanDiegoMontoya/Frogfood) | MIT, "Copyright (c) 2023 Jake Ryan" | Vulkan 1.3, GLSL | `data/shaders/visbuffer/`, `data/shaders/hzb/`, `data/shaders/shadows/`, `src/techniques/VirtualShadowMaps.cpp` | Meshlets without mesh shaders, 32-bit visibility buffer, hi-z + frustum meshlet culling, triangle small-primitive culling (V) |
| [Sunset-Flock/Timberdoodle](https://github.com/Sunset-Flock/Timberdoodle) | Apache-2.0, "Copyright [2023] [Patrick Ahrens, Matěj Sakmary]" (template brackets left in) | Vulkan, C++ | One-drawcall visibility buffer with "power of two expansion", single-pass HiZ, two-pass occlusion, VSM, DDGI-derived probe GI | Copy needs the Apache notice |
| [AMD SPD](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v1.1.4/sdk/src/components/spd) | MIT, "Copyright (C) 2024 Advanced Micro Devices, Inc." | HLSL/GLSL | Single-pass downsampler for HZB | Port to WGSL; needs subgroup ops or a workgroup-memory fallback |

## 2. Non-RT shadows

Renderlab context: non-rt-path.md chose **option (a)**: the same shadow estimators traced in software against a cooked BVH. It rejected VSM + SMRT, hybrids and distance fields on measured grounds: 1024 maps stipple, 4096 maps still speckle, and UE VSM loses 31% of contacts with 2,191 solid flips against the reference. This section asks whether any non-ray method can be **pixel-exact for hard shadows** and **correct for soft shadows** on a 13 M-triangle scan, and whether it is cheaper than software rays.

| Method | Hard shadows pixel-exact? | Soft shadows correct? | Published cost on large scenes | Verdict for Renderlab |
|---|---|---|---|---|
| Software BVH ray (option a) | Yes, with the shared watertight test and ε | Yes, same estimator as RT | §3: about 5.4x hardware per ray (A, different workload) | Chosen |
| Tiled per-triangle soft shadow volumes (H1) | Yes, if the bitmask has one cell | Sampled over 208 disk cells, exact per cell | ~13 M triangles, RTX 4070, 4K: eval ~5 ms + gather ~1.3 ms (A) | **Probe as an Exact-mode accelerator** |
| FTIZB (H2) | Yes, 1 to 32 sub-pixel samples | No; HFTS switches to PCSS away from contacts | 12.3 M triangles, 1080p, GTX 980: 20.0 ms (1 spp), 32.2 ms (32 spp) with conservative raster (A) | Deprioritized: costlier than a software hard ray (I) |
| Per-triangle shadow volumes (H3) | Yes, for point or directional lights | No (the soft extension is Sintorn 2008 alias-free maps) | No large-scan numbers found | Deprioritized |
| VSM + SMRT (H4) | No: texel-limited, bias needed | Approximate: march against one depth layer | UE: VSM depths 0.312 ms + SMRT 0.141 ms on the quarry fallback mesh (M, Renderlab) | Rejected as core (M) |
| MSM / PCSS / SDSM (H5) | No | No | n/a | Rejected as core |

### 2.1 Tiled per-triangle soft shadow volumes (Kalbertodt): REIMPLEMENT-ONLY, worth a probe

- **Source:** http://lukaskalbertodt.github.io/2023/11/18/tiled-soft-shadow-volumes.html (V). Only the shader rasterization code is public, in gist `LukasKalbertodt/0b809d501ee0262c361616a85dec2846`, with **no license stated** (V). Reimplement from the article.
- **Algorithm** (V):
  1. Directional sun with an angular radius. Each triangle and mesh cluster gets a truncated infinite cone with the sun's angle.
  2. **Gather** (compute): frustum-cull clusters, then triangles; drop light-facing triangles.
  3. **Eval** (compute): one 256-thread workgroup per 16x16 pixel tile. A sphere or capsule is built from the tile's min/max depth. Clusters, then triangles, are culled into a shared-memory list. Each pixel rasterizes every listed caster, projected onto the sun disk, into a 16x16 bitmask (`array<u32, 8>`, 208 cells inside the disk). Visibility is `1 - set_bits / 208`. The grid is randomly rotated per pixel to hide banding.
  4. The author tried analytic overlap first and rejected it: overlapping triangles were double-counted.
- **Numbers** (A, RTX 4070, wgpu): ~13 M triangles in ~30 K clusters, ~60 FPS at 4K. Gather ~1.3 ms, reducing to ~2.5 M triangles. 16x16 rasterization ~5 ms at 4K for ~30 triangles per pixel on average. Cheaper 31x3/5/7 line grids cost ~2.1/2.5/3.1 ms.
- **Limits** (V): cost varies with view and sun elevation (low sun is worst); shared memory caps each tile at a few thousand triangles, and overflowing tiles show a checkerboard; no LOD; "still nowhere near production-ready".
- **Why it matters (I):** the quarry has the same triangle count on the same GPU class. Per pixel it evaluates **208 sun-disk directions exactly**, against software Exact's 256 rays estimated at 15–50 ms. Eval cost scales with pixels, so 1080p could land near 1.3–2 ms + 1.3 ms gather. That would put a non-RT Exact-class estimator in the single-digit milliseconds, below the hardware Exact mode's 7.6–10 ms.
- **Risks to verify before believing that (I):**
  - **Dropping light-facing triangles is a closed-mesh assumption.** Photogrammetry scans are open surfaces, so this would drop real casters. Keep all triangles, which raises cost.
  - Per-tile list overflow at depth discontinuities. The quarry's cliffs make discontinuities common.
  - Receiver ε, self-shadowing and Renderlab's 10 µm view-side policy are not addressed by the article.
  - 208 regular cells are a different estimator from 256 stratified rays. Gate it against R_L/R_D/R_F like any other mode, never as a substitute reference.
  - It is light-specific (one disk). Local lights need a per-light pass.

### 2.2 Frustum-traced IZB and HFTS: REIMPLEMENT-ONLY, deprioritized

- **Papers:** Wyman, Hoetzlein, Lefohn, "Frustum-Traced Irregular Z-Buffers: Fast, Sub-pixel Accurate Hard Shadows", IEEE TVCG 22(10), 2016, https://cwyman.org/papers/tvcg16_ftizbExtended.pdf (V, text extracted). I3D 2015 original: https://cwyman.org/papers/i3d15_ftizb.pdf. HFTS: Story & Wyman, SIGGRAPH 2016 Talks, https://cwyman.org/abstracts/sig2016_hfts.pdf (V).
- **Mechanism** (V): rasterize the view samples into light space as per-texel lists (an IZB). Rasterize casters with **conservative raster**. Each covered list tests its samples against the triangle's frustum and writes a 32-bit visibility mask (32 sub-pixel samples).
- **Numbers** (A, Table 2, GeForce 980 GTX, 1920x1080, with/without hardware conservative raster):
  - Citadel (613 K): 8.1/10.1 ms at 32 spp, 4.3/5.1 ms at 1 spp.
  - Hairball (2.9 M): 15.4/30.8 and 6.8/12.7 ms.
  - Powerplant (12.3 M): 32.2/91.3 and 20.0/49.7 ms.
  - "32 sample per pixel hard shadows at roughly twice the cost of a single sample". The 12.3 M Powerplant at 3840x2160 runs "in under 53 ms" at 32 spp.
- **HFTS** (A, GTX 980, 1080p, The Division): 7.9–9.7 ms against PCSS 4.6–5.7 ms and PCF 2.8–3.4 ms. It uses IZB only near contacts and PCSS farther away, so its soft shadows are a filter, not an estimator. Shipped in NVIDIA's proprietary ShadowWorks; no open code found.
- **Assessment (I):** pixel-exact hard shadows are achievable in wgpu (`CONSERVATIVE_RASTERIZATION` exists on Vulkan). Even a generous ~5x GTX 980 → 4070 SUPER scaling leaves 12 M-triangle IZB at roughly 4–6 ms, which loses to a software hard ray (N0 Hard estimate 0.1–0.5 ms). Soft shadows need multiple light samples, multiplying the cost. Keep as a citation, not a candidate.

### 2.3 Per-triangle shadow volumes (Sintorn, Kämpe, Olsson, Assarsson, I3D 2014)

- PDF: https://efficientshading.com/wp-content/uploads/clustered_ptsv_2014.pdf (V, link listed). Exact hard shadows; the authors call it "the fastest pixel-accurate shadow algorithm to date" in 2014. No code found (V for the search). Later work, "Scalable real-time shadows using clustering and metric trees" (2018), was tested on scenes over 70 M triangles (V, abstract-level). Same verdict as 2.2. H1 is the soft-shadow descendant that matters.

### 2.4 Virtual shadow maps and SMRT: AVOID as core

- **Unreal docs** (V, UE 5.8 page): 16K virtual resolution, 128x128 pages, directional clipmap levels 6–22 (64 cm to about 40 km). SMRT defaults to 8 rays (one caption says 7) and 4–8 samples per ray, marching against the depth map. Documented artifacts: light leaking (one depth layer, with a gap-fill heuristic), `NormalBias` default 0.5 that "loses fine detail", projective aliasing at shallow angles, and a statement that VSM is "likely not a good fit" without Nanite. Moving or rotating a light throws away every cached page.
- **Open implementation write-up:** Sakmary, Jaker, Stephano and LVSTRI, GPU Zen 3 chapter as a blog, https://graphics-programming.org/blog/virtual-shadow-maps (V):
  - 16 cascades of 4096² with 128² pages; GPU-only allocator; HPB culling.
  - Bistro (~4 M triangles), RTX 3070: uncached 4096/128 at 2.37 ms, cached at 0.39 ms; 1080p shading 0.31–0.89 ms for 0–16 samples (A).
  - "rotating the light invalidates every page" (V).
- **Code:** Frogfood `src/techniques/VirtualShadowMaps.cpp` (MIT) and Timberdoodle (Apache-2.0) can be copied. StratusGFX is MPL-2.0 (file-level copyleft), so REIMPLEMENT-ONLY under this note's rules.
- **Renderlab position:** rejected as core by measurement. The only reusable idea is **HPB-style culling**: a mip chain of "dirty/needed" flags used to cull clusters, which also works for any light-space pass such as H1's gather (I).

### 2.5 Moment shadow maps, PCSS, SDSM

- MSM: Peters & Klein, I3D 2015, https://momentsingraphics.de/I3D2015.html. The page offers demo, data and shader code but states **no license** (V). PCSS: Fernando 2005 (NVIDIA). SDSM: Lauritzen, Salvi, Lefohn, I3D 2011; Intel sample `sdsm_2012-01.zip`, license not verified (V for the page).
- All three filter or partition a single depth map. They cannot be pixel-exact or contact-correct on millimetre relief. AVOID as core; no reuse value.

## 3. Software ray tracing on compute

Renderlab context: **N0** ([non-rt-path.md](../non-rt-path.md#units-and-stop-rules)) is the deciding unit. It needs a CPU builder that collapses to 8-wide, a WGSL closest-hit and any-hit traversal, and a watertight triangle test. Its 2–5x software/hardware ratio is unmeasured.

### 3.1 Published software-versus-hardware evidence

| Source | GPU | Workload | Software | Hardware | Ratio |
|---|---|---|---|---|---|
| tray_racing `results/gpu_traversal_bench.PNG` (V, chart read) | **RTX 4070 Ti** (AD104, same 48 MB L2 class as the 4070 SUPER) | Avg over 8 scenes (Bistro 3.9 M, San Miguel 5.1 M, Hairball 2.9 M, Caldera Hotel 19.3 M, …), 1920x1080, 1 primary + 1 unbounded closest-hit AO ray per pixel | Best: PLOC CWBVH "vsb" preset **2.75 ms**; Embree CWBVH 3.53 ms | Vulkan hardware RT **0.51 ms** | **5.4x** (A) |
| Ylitie et al. 2017, Table 4 (V, text extracted) | Titan X (Pascal) | San Miguel 10.5 M: primary / bounce 1 / bounce 4 | CWBVH 629 / 311 / 162 Mrays/s; Aila-Laine baseline 423 / 118 / 59 | n/a | CWBVH 1.5x / 2.6x / 2.7x over Aila-Laine on this scene; geometric mean over all scenes 1.36 / 2.36 / 2.70 |
| Ylitie 2017, Table 4 | Titan X (Pascal) | Powerplant 12.8 M | CWBVH 322 / 271 / 172 Mrays/s | n/a | |
| tinybvh README (V) | RTX 5080 laptop | Crytek Sponza, binary BVH | "up to 5 billion rays per second" | n/a | Coherent-ray best case |
| Lighthouse 2 benchmark (V) | RTX 3090 / RTX 2080 | Path tracing | n/a | Peak shadow rays 8,503 / 3,758 Mrays/s | Hardware reference |

**Inference for N0, to be measured rather than trusted:**

- Applying the tray_racing ratio to Renderlab's measured hardware costs gives software primaries at about 0.23 x 5.4 ≈ 1.2 ms and Interactive 8-ray shadows at about 0.12 x 5.4 ≈ 0.65 ms.
- That primary cost is above N0's 0.40 ms gate, so N1b (raster visibility buffer, §1) would be needed.
- The shadow cost is under N0's 1.0 ms stop rule but above its 0.35 ms budget.
- With N1b at ≤ 0.40 ms, the projected non-RT Interactive frame is about 0.40 + 0.65 + 0.07 + 0.45 + 0.03 ≈ 1.6 ms. That is near the 1.61 ms VSM line; 2x is not reachable, as non-rt-path.md already states.
- Reasons the ratio could differ:
  - Renderlab's sun rays are any-hit with early exit and similar directions, which should be cheaper than unbounded AO.
  - WGSL through naga lacks the PTX intrinsics (`bfind`, `popc`) and may spill a private-array stack.
  - tray_racing compiles HLSL with DXC.

### 3.2 Code to take

| Item | License (V) | Language | Files | What it gives N0 | Caveats |
|---|---|---|---|---|---|
| **[obvhs](https://github.com/DGriffin91/obvhs)** 0.4.0 (crates.io, updated 2026-10-05) | MIT OR Apache-2.0 | Rust | `src/` (PLOC, CWBVH, BVH2 tools); `examples/demoscene.rs` | PLOC BVH2 with parallel reinsertion and spatial pre-splits; BVH2 → CWBVH (80 B nodes); CPU SIMD traversal of both, usable as N0's CPU oracle; optional rayon | GPU traversal lives in tray_racing, not here. Check that its CWBVH triangle and leaf layout can reference Renderlab's existing index buffer without copies (N0 wants "no triangle copy") |
| **[tray_racing](https://github.com/DGriffin91/tray_racing)** | MIT OR Apache-2.0 (README; assets excluded) | Rust + HLSL (DXC) on wgpu | `src/rt_gpu/rt_gpu_software.hlsl` (144 lines: 8x8 groups, primary + AO), `rt_gpu_software_query.hlsl`, `rt_gpu_software_tlas.hlsl`, `rt_gpu_software_query_tlas.hlsl`, `src/cwbvh.rs`, `src/auto_tune.rs` | Software CWBVH traversal already wired to wgpu buffers, with a **split VGPR (5) + LDS (4) stack**, a TLAS variant and a hardware comparison harness | HLSL → WGSL port. Triangle postponing is "Unimplemented"; work stealing "seemed slower" |
| **[tinybvh](https://github.com/jbikker/tinybvh)** v1.9.1 | MIT, "Copyright (c) 2024 Jacco Bikker" | C++ header + OpenCL kernels | `kernels/traverse_cwbvh.cl` (~580 lines): `traverse_cwbvh`, `isoccluded_cwbvh`, `batch_cwbvh_any`; 24-entry `uint2` private stack; `unpack_q8x4` places each byte into the mantissa to dequantize without int→float conversion; Möller-Trumbore (`e1, e2, v0`) or Baldwin-Weber compressed triangles | **Any-hit** shadow kernel with early exit; builders: Bonsai, binned SAH, SBVH, H-PLOC; layouts `BVH_GPU`, `BVH4_GPU`, `BVH8_CWBVH` | **Provenance:** the kernel says it is "based on CUDA code by AlanWBFT" and has no file header (V). The repo-level MIT covers it, but confirm the upstream before copying verbatim. EA SEED's Gigi used tinybvh traversal in WebGPU and reported it "as fast as when using the DXR api in DX12" (A) |
| [jan-van-bergen/GPU-Raytracer](https://github.com/jan-van-bergen/GPU-Raytracer) | MIT | C++/CUDA | `Src/` (CWBVH collapse with Ylitie's dynamic fetch heuristic and triangle postponing) | Second CWBVH builder and traversal reference | No published numbers (V) |
| [Embree](https://github.com/RenderKit/embree) | Apache-2.0 | C++ | BVH8 builder (`RTCBuildQuality::HIGH`) | High-quality CPU builds; tray_racing's `embree_cwbvh` path | Large C++ dependency; slightly slower to traverse than PLOC in tray_racing's chart (A) |
| [HIPRT](https://github.com/GPUOpen-LibrariesAndSDKs/HIPRT) | MIT, "Copyright (C) 2024 Advanced Micro Devices, Inc." | HIP | `hiprt/` | GPU LBVH/PLOC/SBVH builders (per Harada's HIPRT page and the HPG 2024 paper, Meister et al., DOI 10.1145/3675378) | Only relevant if BVH builds move to the GPU (movers, N2) |

- **Watertight test:** Woop, Benthin and Wald 2013 (JCGT) is what N0 specifies. None of S1–S3 uses it by default (tinybvh uses Möller-Trumbore or Baldwin-Weber). Keep Renderlab's own test for parity with the hardware barycentric convention (I).
- **Ylitie 2017:** https://research.nvidia.com/sites/default/files/publications/ylitie2017hpg-paper.pdf. The abstract claims "1.9-2.1x improvement in incoherent ray traversal performance" and "35-60% of a typical uncompressed BVH" (V). Its sections 4.1–4.3 (compressed stack with a hit mask, node decompression, state management) are what S2 and S3 implement.

### 3.3 Running the same 8-ray estimator in software

Yes, structurally. Non-rt-path.md already decides one contract with two backends. S2 and S3 provide exactly the two kernels it needs: closest-hit for primaries and any-hit for shadows. Wicked Engine shows the production version of the pattern: `ddgi_raytraceCS.hlsl` is compiled with and without `RTAPI`, so hardware and compute traversal share one source (V, Wicked docs and a third-party port). Renderlab's WGSL equivalent is a `trace_any(ray) -> bool` / `trace_closest(ray) -> Hit` pair. One implementation uses `rayQuery`, the other the CWBVH loop, selected at pipeline composition (I).

## 4. Software GI without RT

Renderlab context: GI is not yet in the matched workload. Lumen software becomes the non-RT denominator when Renderlab's GI lands (non-rt-path.md). The standing rules exclude screen-space techniques for core lighting. Read every GI option through that filter.

| System | Representation traced | Screen-space dependency | Published cost (A) | Fit |
|---|---|---|---|---|
| Lumen SW (L3) | Mesh SDFs + global SDF; surface cache (mesh cards) for hit lighting | **Yes:** HZB screen traces run first; the final gather is a screen-space radiance cache at 1/16 resolution per axis, backed by a world radiance cache | PS5, 1080p → TSR 4K, "Lumen in the Land of Nanite", global SDF only: High 2.77 ms total (surface cache 0.47, final gather 2.07, reflections 0.23); Epic 4.63 ms. Lyra SWRT: High 4.29 ms, Epic 8.70 ms | Take the **surface cache** and **world radiance cache** ideas; avoid screen traces as core |
| Brixelizer GI (L4) | Sparse SDF bricks (8³, R8_UNORM atlas), 64³ voxels per cascade, 3-level AABB tree per cascade | **Yes:** screen probes per 8x8 tile, and "Radiance cache filled using screen space information" ("Surfaces must be in view before bounced lighting can take on its colour") | RX 7900 XTX ~2 ms (4K); RTX 4080 ~2.2 ms (4K); RX 7600 XT ~1.7 ms (1080p). GI memory ~200 MB at 1080p native | Far field only; "Loses small-scale details" |
| Godot SDFGI (L5) | Camera-following SDF cascades + probes | No for tracing; the probe/SDF update is camera-driven | HDDAGI PR: GTX 1650 static scene, SDFGI 2.43 ms → HDDAGI 0.93 ms | No dynamic occluders; GI lags light changes (default 25 frames to settle); cascade shifts visible |
| DDGI (L2) via Wicked compute BVH (L1) | World probes with irradiance + depth moments, rays through any backend | No | n/a | **Best structural fit**: world-space, traced through the shared ray backend |
| GI-1.x / Capsaicin (L6) | Screen probes + world hash-grid cache, DXR | Screen-probe placement | n/a in README | Needs hardware RT as shipped |
| Radiance cascades 3D (L7) | Probe hierarchy with angular/spatial trade | Varies; three-rc is screen-space | n/a | Research |
| GIBS (L8) | Surfels on G-buffer, hardware RT | Surfels spawn from the screen | n/a | Hardware-RT design |

### 4.1 Lumen: REIMPLEMENT-ONLY, selectively

- PDF: https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf (V, text extracted).
- **Surface cache:** axis-aligned mesh cards per mesh, generated at import ("~0.2ms to build a large 1.5M tri mesh"). Card capture is rendered with Nanite into a fixed texel budget (512x512 per frame). Direct lighting updates 1024x1024 texels and indirect 512x512 per frame, chosen by a used/updated histogram (A). This is world-space and could be traced by Renderlab's software backend (I).
- **Screen traces** are an HZB walk "to handle mismatches" between the traced representation and the raster G-buffer (V). Renderlab traces the same full-detail geometry for primaries and secondary rays, so it has no such mismatch to hide (I). That removes Lumen's main reason for screen traces.
- **Patents:** none checked for Lumen in this pass.

### 4.2 Brixelizer and Brixelizer GI: COPY allowed, far field only

- Code: https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v1.1.4/sdk/src/components/brixelizer and `.../brixelizergi`. The root `LICENSE.txt` is MIT, "Copyright (C) 2024 Advanced Micro Devices, Inc." (V). AMD's v1.1 announcement says "full source code and binaries" were released (V, search snippet). The current SDK (renamed "AMD FSR SDK" 2.4.0) no longer lists Brixelizer on its README (V), so **pin v1.1.4**. HLSL/GLSL; DX12 and Vulkan backends.
- GDC 2024 deck (Wijetunga), https://gpuopen.com/download/GDC2024_GI_with_AMD_FidelityFX_Brixelizer.pdf (V, text extracted): numbers and limitations as in the table above.
- **Use:** a far-field occlusion and irradiance proxy beyond the near-field ray range (I). Never the source of contact or mid-range lighting. Its screen-fed radiance cache must be replaced by world-space injection (from the shadow-ray backend or the surface cache) to satisfy the off-screen rule (I).

### 4.3 DDGI: reimplement from the paper, or copy Wicked's MIT version; avoid RTXGI code

- RTXGI-DDGI (https://github.com/NVIDIAGameWorks/RTXGI-DDGI) is under the **NVIDIA RTX SDKs License** (V, `License.txt`). It is proprietary: object-code redistribution only, derivatives only of sample code, and no open-source relicensing. Its README says DDGI needs "graphics APIs and platforms with support for GPU ray tracing" (V). **AVOID copying.**
- Paper: Majercik, Guertin, Nowrouzezahrai, McGuire, "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields", JCGT 8(2), 2019. Its license was not checked in this pass. Reimplement from it.
- Wicked Engine (https://github.com/turanszkij/WickedEngine, MIT, "Copyright (c) 2026 Turánszki János", V): `WickedEngine/wiGPUBVH.*` builds a scene BVH in compute, and DDGI falls back to it "if hardware accelerated ray tracing is not available" (V, Wicked docs). Files: `shaders/ddgi_raytraceCS.hlsl` (compute) and `ddgi_raytraceCS_rtapi.hlsl` (same source with `RTAPI`), `ddgi_updateCS.hlsl`, `ddgi_updateCS_depth.hlsl` (names from a third-party port, not opened here).
- Timberdoodle's "PGI" extends DDGI with variable-rate updates, a tighter depth test, sparse probe requests and 8 cascades of up to 32³ probes (V, README). It is Apache-2.0.
- **Caveat (I):** DDGI is an irradiance volume. On millimetre scan relief it carries only low-frequency indirect light. Near-field indirect occlusion still needs rays or a surface cache, and DDGI alone will not meet the film gate.

### 4.4 Godot SDFGI and HDDAGI

- Godot is MIT ("Copyright (c) 2014-present Godot Engine contributors", V). Shaders: `servers/rendering/renderer_rd/shaders/environment/sdfgi_preprocess.glsl`, `sdfgi_integrate.glsl`, `sdfgi_direct_light.glsl`, `sdfgi_debug*.glsl`, plus `environment/gi.cpp` (V).
- Documented limits (V, Godot docs): "not dynamic occluders or dynamic emissive surfaces"; static-light changes look wrong "until the camera moves away from the light and back"; "cascade shifts may be visible".
- HDDAGI (PR #86267 by reduz) is a draft, locked since December 2024 and unmerged (V). It replaces jump-flood SDF rebuilds with local HDDA updates (A: GTX 1650 2.43 → 0.93 ms).
- AVOID as core lighting. A possible source for SDF cascade scrolling code if a far-field field is ever needed; Brixelizer is the better candidate.

### 4.5 Radiance cascades, GI-1.x, GIBS

- **Radiance cascades:** paper source at https://github.com/Raikiri/RadianceCascadesPaper, compiled PDF at `out_latexmk2/RadianceCascades.pdf`. GitHub shows MIT, but the repo also contains a `CC-BY-ND.png`, and the paper was "never published" (V). 3D world-space ports (`tmpvar/radiance-cascades-3d-grid`, `hornsovsatan/RCGI`, `jumeder/dx12-radiance-cascades`, `JTLee98/RadianceCascadesVK3D`) showed no license in search results (V). Bevy's Solari tried radiance cascades and replaced them (A, JMS55 2026). REIMPLEMENT-ONLY; watch.
- **GI-1.x:** Capsaicin (https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin) is MIT, requires "Direct3D12 Ultimate capable hardware", and ships GI-1.2 with "two levels of radiance caching" (V). Algorithm reference only.
- **GIBS:** SIGGRAPH 2021 Advances, slides at https://advances.realtimerendering.com/s2021/index.html. EA describes it as proprietary technology that "leverages hardware ray tracing" (V, search). Open Falcor port at `W298/SurfelGI`, license not checked. REIMPLEMENT-ONLY.

## 5. Patents

No concrete patent was found in this pass for Nanite-style virtualized geometry, VSM, CWBVH or IZB (V for the searches; this is **not** a freedom-to-operate check). The Apache-2.0 patent grants (Embree, Timberdoodle, Bevy if taken under Apache) cover only their contributors' patents. If the project's license is decided as Apache-2.0, that clause also binds Renderlab's contributors (I). Before a commercial claim, run a proper patent search on: GPU cluster DAG LOD with locked group boundaries; 64-bit atomic visibility buffers; compressed wide BVH node formats (NVIDIA authors); irregular z-buffer hardware (Johnson et al. 2005).

## Recommended first takes

1. **obvhs + tray_racing as N0's builder and kernel** (S1, S2; MIT/Apache).
   - Why: Rust-native PLOC + CWBVH builder, a CPU oracle, and an HLSL software traversal already driven from wgpu. The only measured software/hardware ratio found, 5.4x on an RTX 4070 Ti, also comes from this pair. It replaces most of N0's from-scratch builder work, and the 5.4x figure can sharpen the stop rules before code is written.
   - Risk: HLSL → WGSL port quality (no `bfind`/`popc`, stack spills); leaf layout copying triangles against N0's no-copy goal; 5.4x is an average over unbounded AO rays, not sun shadows. Use tinybvh's `isoccluded_cwbvh` (S3) for the any-hit loop after resolving its "AlanWBFT" provenance.
2. **Bevy meshlet cull chain + software raster for N1b** (G1; MIT/Apache), with `clodBuildHierarchy` from the meshoptimizer already in the cook (G2) and AMD SPD for the HZB (G6).
   - Why: the 5.4x inference puts software primaries near 1.2 ms, which would trigger N1b. Bevy is the only production wgpu/WGSL implementation of two-pass HZB culling with a 64-bit atomic visibility buffer, and its required features exist in wgpu 30.
   - Risk: no top-left rule or fixed-point math (tie classification must absorb it); `.wesl` import syntax; Bevy calls culling its bottleneck; the R64 clear cost (0.68 ms through `clear_texture`) needs Bevy's compute clear.
3. **A probe of tiled per-triangle soft shadow volumes as a non-RT Exact-mode accelerator** (H1; reimplement).
   - Why: the article's measured case is ~13 M triangles on an RTX 4070 in wgpu, with 208 sun-disk samples per pixel. If its 4K numbers scale to 1080p, it could cut non-RT Exact from an estimated 15–50 ms to a few milliseconds, and possibly undercut hardware Exact (7.6–10 ms).
   - Risk: the light-facing cull is invalid for open scan surfaces; tile list overflow at cliffs; ε/self-shadowing policy; strong sun-angle sensitivity; no license on the gist. Run a crop-sized CPU-replay probe first, gated against R_L.
4. **The single-source dual-backend trace pattern for future GI** (L1 Wicked, MIT; L2 DDGI paper; L3 Lumen surface cache).
   - Why: it keeps GI world-space and identical on both backends, the same contract as shadows. It avoids the screen-space radiance caches in Lumen, Brixelizer GI, GI-1.x and GIBS.
   - Risk: software ray cost multiplies with GI ray counts (5.4x); DDGI alone is low-frequency and will not meet the film gate near contacts.
5. **Brixelizer (pinned v1.1.4) as a far-field-only proxy, if GI needs range beyond the ray budget** (L4; MIT).
   - Why: a mature, open, compute-only sparse SDF with published cost (~1.7 ms at 1080p on an RX 7600 XT).
   - Risk: its GI's screen-fed radiance cache violates the off-screen rule and must be replaced; it is gone from the current AMD SDK README; detail loss means it must never shade the near field.

**Not recommended:** VSM/SMRT, MSM, PCSS and SDSM as core shadows (measured and structural failures); FTIZB/HFTS and per-triangle shadow volumes (exact but slower than software rays at 12 M triangles); RTXGI code (proprietary license); Lumen screen traces and SDFGI as core lighting (screen-space or static-only).

## Source ledger

All accessed 2026-11-06. PDFs were text-extracted with `pdftotext` under `E:/renderlab-work/research-pdf/`; raw PDFs over 5 MB were deleted after summarizing.

- Bevy meshlet: https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/meshlet ; `mod.rs` ; `visibility_buffer_software_raster.wesl` ; README license section ; https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16/ ; https://jms55.github.io/posts/2026-08-29-bevy-sixth-birthday/
- meshoptimizer: https://github.com/zeux/meshoptimizer ; `LICENSE.md` ; `demo/clusterlod.h`
- nanite-webgpu: https://github.com/Scthe/nanite-webgpu ; `LICENSE`
- niagara: https://github.com/zeux/niagara ; `src/shaders`
- Frogfood: https://github.com/JuanDiegoMontoya/Frogfood ; `LICENSE` ; `src/techniques` ; `data/shaders`
- Timberdoodle: https://github.com/Ipotrick/Timberdoodle (fork README) ; https://github.com/Sunset-Flock/Timberdoodle `LICENSE`
- VSM chapter: https://graphics-programming.org/blog/virtual-shadow-maps ; Unreal VSM docs: https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine ; StratusGFX: https://github.com/KTStephano/StratusGFX
- Nanite talk: https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf
- Lumen talk: https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
- FTIZB / HFTS: https://cwyman.org/papers/tvcg16_ftizbExtended.pdf ; https://cwyman.org/abstracts/sig2016_hfts.pdf ; https://www.nvidia.com/en-us/geforce/news/nvidia-hfts-hybrid-frustum-traced-shadows/
- Sintorn 2014: https://efficientshading.com/2014/01/01/per-triangle-shadow-volumes-using-a-view-sample-cluster-hierarchy/
- Kalbertodt: http://lukaskalbertodt.github.io/2023/11/18/tiled-soft-shadow-volumes.html
- MSM: https://momentsingraphics.de/I3D2015.html ; SDSM: https://www.intel.com/content/www/us/en/developer/articles/technical/sample-distribution-shadow-maps.html
- tinybvh: https://github.com/jbikker/tinybvh ; `LICENSE` ; `kernels/` ; `kernels/traverse_cwbvh.cl` ; https://electronicarts.github.io/gigi/Demos/tinybvh/index.html
- obvhs: https://github.com/DGriffin91/obvhs ; https://crates.io/api/v1/crates/obvhs
- tray_racing: https://github.com/DGriffin91/tray_racing ; `README.md` ; `src/rt_gpu/` ; `src/rt_gpu/rt_gpu_software.hlsl` ; `results/gpu_traversal_bench.PNG`
- Ylitie 2017: https://research.nvidia.com/publication/2017-07_efficient-incoherent-ray-traversal-gpus-through-compressed-wide-bvhs ; PDF linked above
- jan-van-bergen/GPU-Raytracer: https://github.com/jan-van-bergen/GPU-Raytracer
- HIPRT: https://github.com/GPUOpen-LibrariesAndSDKs/HIPRT ; `license.txt` ; https://doi.org/10.1145/3675378
- Embree: https://github.com/RenderKit/embree `LICENSE.txt`
- Lighthouse 2 RTX numbers: https://jacco.ompf2.com/lighthouse-2-rtx-path-tracing-benchmark/
- FidelityFX SDK v1.1.4: https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v1.1.4 ; `LICENSE.txt` ; `sdk/src/components` ; https://gpuopen.com/fidelityfx-brixelizer/ ; https://gpuopen.com/download/GDC2024_GI_with_AMD_FidelityFX_Brixelizer.pdf
- Capsaicin: https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin
- Godot: `LICENSE.txt` ; https://github.com/godotengine/godot/pull/86267 ; https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html ; `servers/rendering/renderer_rd/shaders/environment`
- RTXGI-DDGI: https://github.com/NVIDIAGameWorks/RTXGI-DDGI ; `License.txt`
- Wicked Engine: https://github.com/turanszkij/WickedEngine ; `LICENSE.txt` ; `features.txt` ; `Content/Documentation/WickedEngine-Documentation.md`
- Radiance cascades: https://github.com/Raikiri/RadianceCascadesPaper
- GIBS: https://www.ea.com/seed/news/siggraph21-global-illumination-surfels
- Local: wgpu-types 30.0.1 `src/features.rs` (feature flags) ; `crates/cook/Cargo.toml`, `docs/virtual-cooking.md` (meshopt 0.7.0 / meshoptimizer 1.3)
