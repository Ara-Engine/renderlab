# Reusable engine code, ray-precision techniques and fast shader iteration

2026-11-06. Read-only research: web, GitHub API and local reading only. No build, GPU run, window or commit. Local inputs were [determinism](../determinism.md), [receiver policy](../m6-receiver-policy.md), [receiver adoption](../m6-receiver-adoption.md), [film gap](../m6-film-gap.md) (its earlier survey of engine offset policies is not repeated here), [fast shader reload](../fast-shader-reload.md), [programmable shading](../programmable-shading.md) and [non-RT path](../non-rt-path.md). The witness rays came from `E:/renderlab-work/receiver-origin/actual-ray/{rays,replayed-proof}.json`.

Companion surveys, which this file does not duplicate:
- [reuse-non-rt.md](reuse-non-rt.md): software BVH, the meshlet visibility buffer, non-RT shadows and GI.
- [reuse-temporal-denoise.md](reuse-temporal-denoise.md): TAA, TSR, SMAA, denoisers, ReSTIR and Solari sampling.

**Labels.**
- **V:** verified at the cited URL or file.
- **M:** computed here from local evidence.
- **I:** inference, not measured.

The +5% cost of the current adjacency fix, its approximately 1% cold-pass fraction and the concave-only inset come from the task brief. I did not find them recorded in a doc.

## Summary

| # | Item | License (V) | Verdict | What to take | Renderlab problem |
|---|---|---|---|---|---|
| P1 | **Front-face retrace at near-edge back-facing primary hits** (the idea is assembled here from the [DXR](https://microsoft.github.io/DirectX-Specs/d3d/Raytracing.html)/[Vulkan](https://docs.vulkan.org/spec/latest/chapters/raytraversal.html) watertightness and culling rules) | n/a | **REIMPLEMENT** (try first) | When the committed primary hit is `front_face == false` and within an error bound of an edge, re-trace once with `CULL_BACK_FACING` and keep the result only if its t is within the bound | Edge-hit slivers. It can replace the adjacency table and slope test for primaries |
| P2 | **NVIDIA self-intersection avoidance** (van Antwerpen 2023) | Sample code [BSD-3-Clause](https://github.com/NVIDIA/self-intersection-avoidance) (V) | **COPY** (port GLSL to WGSL) | `safeSpawnOffset`: hit point from object-space barycentrics, per-hit error bound (c0, c1, c2), and offsets in object and world space | Concave-fold false shadows. The bound is 0.6–1.2 µm at the witnesses, against 10 µm (M) |
| P3 | **Cycles self-hit retest** (`integrate_surface_ray_offset`) and prim skip | [Apache-2.0](https://github.com/blender/cycles/blob/main/LICENSE) (V) | **COPY** the retest idea; **AVOID** the any-hit prim skip | Offset only when a ray from the exact point would miss its own triangle | Shared-edge neighbour hits on secondary rays |
| P4 | **pbrt-v4** `OffsetRayOrigin`, `Point3fi` error bounds, watertight triangle test with a double fallback | [Apache-2.0](https://github.com/mmp/pbrt-v4/blob/master/LICENSE.txt) (V) | **COPY** (CPU oracle; non-RT kernel) | `src/pbrt/ray.h`, `interaction.h`, `shapes.h` | Exact CPU reference for edge and fold tests. Error bounds for the software backend |
| P5 | **Wächter & Binder 2019**, RTG ch. 6 | Code [MIT](https://github.com/Apress/ray-tracing-gems) (V) | **AVOID** at quarry scale | `offset_ray` (256-ulp integer offset) | It is 61–122 µm at 3–7 m (M), so it widens fold leaks. [Film gap](../m6-film-gap.md) already rejected it |
| P6 | **Hanika 2021**, RTG II ch. 4 terminator offset, as shipped in Cycles `shadow_ray_smooth_surface_offset` | Chapter CC BY-NC-ND 4.0 (V); Cycles code Apache-2.0 (V) | REIMPLEMENT-ONLY as a declared shading option | Vertex-normal pseudo-surface, clamped to zero in concave regions | The terminator, not precision. Already rejected as the reference in [film gap](../m6-film-gap.md) |
| P7 | **Wald 2021**, RTG II ch. 34: move the origin closer and re-intersect | Chapter open access | REIMPLEMENT-ONLY (fallback to P1) | Short re-trace from `o + (t−δ)d` | Edge-hit ordering when facing is ambiguous |
| A1 | **Bevy Solari** (`crates/bevy_solari`) | MIT OR Apache-2.0 (V) | **COPY** selectively | `realtime/world_cache_*.wesl` (spatial-hash world cache), `presample_light_tiles.wesl`, `scene/bindings.wesl` (barycentric hit reconstruction), `setup_tlas_instances` GPU TLAS fill | Future GI and many lights. Do **not** copy its ray policy (1 mm `RAY_T_MIN`, no offset) |
| A2 | **Bevy atmosphere** (`bevy_pbr/src/atmosphere`) | MIT OR Apache-2.0 (V); upstream [sebh/UnrealEngineSkyAtmosphere](https://github.com/sebh/UnrealEngineSkyAtmosphere) MIT (V) | **COPY** | Hillaire 2020 transmittance, multiscattering, sky-view and aerial-view LUTs in WESL | Physically based sky for the environment field |
| A3 | **Bevy post**: auto exposure, bloom, tonemapping LUTs | MIT OR Apache-2.0 (V); Tony McMapface LUT MIT OR Apache-2.0 (V); AgX LUT source repo has no license file (V) | **COPY**, except the AgX LUT | `bevy_post_process/src/auto_exposure/`, `bloom/`, `bevy_core_pipeline/src/tonemapping/` | Display pipeline. Regenerate AgX from a licensed source or the analytic fit |
| A4 | **Bevy meshlet**, TAA, SMAA, GTAO, PCSS | MIT OR Apache-2.0 (V) | See the companion surveys. GTAO/PCSS: **AVOID** as core | — | Screen-space or shadow-map cores conflict with the standing rules |
| A5 | **Bevy shader pipeline** (`bevy_render/.../pipeline_cache.rs`, `bevy_shader`, WESL 0.6) | MIT OR Apache-2.0 (V) | REIMPLEMENT-ONLY (Renderlab is ahead) | Async compilation on `AsyncComputeTaskPool`; error-scope capture | Compile latency. Bevy passes `cache: None` and drops the old pipeline on reload |
| A6 | **kajiya** (archived 2026-06-01) | MIT OR Apache-2.0 (V) | **COPY** (HLSL → WGSL) | `assets/shaders/ircache/` (irradiance cache), `rtdgi/` (ReSTIR GI with validation), `taa/`; shadow denoiser via MIT FidelityFX | Future GI. Its screen-space parts (ssgi, reprojection reuse) are not core |
| A7 | **Filament** | Apache-2.0 (V) | **COPY** | `shaders/src/surface_lighting.fs` `normalFiltering` (Kaplanyan 2016 / Tokuyoshi–Kaplanyan 2019 spec AA), `surface_brdf.fs`, `libs/ibl` (`CubemapIBL.cpp`), `tools/roughness-prefilter` | Lit-pixel shading aliasing on rough scans. Prefiltered IBL |
| A8 | **Mitsuba 3** | BSD-3-Clause (V) | **COPY** / use as a tool | Path-traced reference renderer; `Interaction::offset_p` + `p_err` | Independent film reference and a second offset policy |
| A9 | **Embree** | Apache-2.0 (V) | **COPY** / use as a tool | Robust-mode watertight CPU traversal | CPU oracle for edge-hit replay at quarry scale |
| A10 | **Wicked Engine** | MIT (V) | **COPY** (HLSL) | `wiGPUBVH.cpp` + `raytracingHF.hlsli`; `rtshadow*`/`rtao*` denoise CS; `*_rtapi` dual-backend shaders | One estimator on two ray backends (see [reuse-non-rt](reuse-non-rt.md) L1) |
| A11 | **Godot 4** | MIT (V) | REIMPLEMENT-ONLY (pattern) | Ubershader plus background-specialized pipelines ([PR 90400](https://github.com/godotengine/godot/pull/90400)) | Shader-edit latency |
| A12 | **The Forge** | Apache-2.0 (V) | **COPY** (reference) | `Examples_3/Visibility_Buffer`, `Visibility_Buffer2` | Non-RT raster visibility producer |
| A13 | **O3DE Atom** | Apache-2.0, or MIT at the user's option (V) | Low priority | Not inspected | — |
| A14 | **Falcor** | BSD-3-Clause for Falcor-owned files (V). It links DLSS/RTXDI/RTXGI/NRD under other licenses (V) | **COPY** Falcor-owned files only | `Source/Falcor/Utils/Geometry/GeometryHelpers.slang` `computeRayOrigin` (V: path exists) | Reference for P2-style offsets |
| X1 | **NVIDIA RTXDI, NRD, RTXGI, RTXPT** | **NVIDIA RTX SDKs License** (V): object-code distribution only, protective-terms flow-down | **AVOID** code; algorithms from the papers only | — | Incompatible with a public repo under an undecided OSI license |
| X2 | **AMD FidelityFX SDK, current main (2.x)** | `docs/license.md` permits **binary-only** redistribution, except listed files (V). Older v1.1.4 tag and the standalone [FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser) repo are MIT (V) | **AVOID** main; **COPY** only from MIT tags or repos | — | Check the tag before copying anything |
| X3 | **Unreal Engine source** | EULA: source may go only to other licensees; products ship object code only ([EULA](https://www.unrealengine.com/eula/unreal), via search summary) | **AVOID** code; talks and docs are fine | — | — |
| X4 | **Unity HDRP / SRP** | [Unity Companion License](https://unity.com/legal/licenses/unity-companion-license): use only with a Unity engine license, and "in no event … to develop a competing product" (V) | **AVOID** entirely | — | — |

## A. Engines whose code can be taken

### A1–A5 Bevy (main, 0.20-dev, wgpu 30)

**Version and license (V).** The repo is `bevyengine/bevy`. Every inspected crate declares `license = "MIT OR Apache-2.0"`, and both license files are in the root. The README says some crates carry extra BSD-like terms in their own README. Bevy uses wgpu 30, so its WGSL and WESL target the same naga as Renderlab.

**Solari** ([0.17 post](https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17/), [0.20 post](https://jms55.github.io/posts/2026-09-18-solari-bevy-0-20/), V).

Algorithms:
- **0.17:** ReSTIR DI with 128 light tiles of 1,024 presampled lights, 32 candidates and visibility reuse, plus ReSTIR GI and a spatial-hash world cache. That cache decays, compacts, updates with 2 rays per cell and blends.
- **0.20:** both merge into "unified" ReSTIR PT, with a 48 B reservoir and 2 dispatches. ReSTIR is now **off by default** for cost ("4 rays for resampling just two extra reservoirs"). DLSS-RR is the only denoiser.

Cost: RTX 3080, 1600×900 internal resolution, totals 8.22 ms (PICA PICA) and 14.55 ms (Bistro), of which DLSS-RR alone is 5.75 and 6.29 ms.

Code (`crates/bevy_solari/src/`):
- `realtime/{initial_path, restir, world_cache_update, world_cache_query, world_cache_compact, presample_light_tiles}.wesl`
- `scene/{bindings, brdf, sampling}.wesl`
- `pathtracer/pathtracer.wesl` (reference)

Ray policy (V, `scene/bindings.wesl`):
- `const RAY_T_MIN = 0.001f`. There is no origin offset, and every ray runs a `rayQueryProceed` candidate loop for alpha testing.
- The hit position comes from barycentrics on world-transformed vertices. That reconstruction is worth taking.
- **The 1 mm t_min is exactly the "geometry filter" that [film gap](../m6-film-gap.md) measured and rejected.**
- No non-RT path exists (V).
- Take the world cache and light-tile presampling as GI and many-light starting points. Do not take the precision policy.

**Atmosphere** (`bevy_pbr/src/atmosphere/`, V). The module docs cite [Hillaire 2020](https://sebh.github.io/publications/egsr2020.pdf) and `sebh/UnrealEngineSkyAtmosphere` (MIT, V). Files:
- `transmittance_lut.wesl`, `multiscattering_lut.wesl`, `sky_view_lut.wesl`, `aerial_view_lut.wesl`, `render_sky.wesl`
- `environment.wesl`, which renders the atmosphere into an environment map
- `bruneton_functions.wesl`

It is directly portable to WGSL. It suits a sun and sky that must match the 0.04 rad disk and environment field (I).

**Post.**
- `bevy_post_process/src/auto_exposure/` is a histogram compute exposure with a compensation curve.
- `bloom/` is a downsample/upsample chain.
- `bevy_core_pipeline/src/tonemapping/luts/` provenance (`info.txt`, V):
  - `tony_mc_mapface.ktx2` comes from [h3r2tic/tony-mc-mapface](https://github.com/h3r2tic/tony-mc-mapface), MIT OR Apache-2.0 (V).
  - `AgX-default_contrast.ktx2` comes from `MrLixm/AgXc`, which has **no license file the GitHub API recognises** (V). Treat that LUT as unlicensed.
- SMAA keeps Jimenez et al.'s MIT header (V).

**Shader and pipeline handling** (V, `bevy_render/src/render_resource/pipeline_cache.rs` and `bevy_shader/Cargo.toml`):
- Bevy now uses `wesl = 0.6` (`naga-ext`), not naga_oil.
- Pipelines are queued and created on `AsyncComputeTaskPool` behind `feature = "multi_threaded"`, except on macOS and wasm.
- `cache: None`: there is **no `wgpu::PipelineCache`**.
- On a shader change, dependent pipelines go back to `Queued`. **The old pipeline is overwritten, so `get_*_pipeline` returns `None` while recompiling or after an error.**

Renderlab already has a session `wgpu::PipelineCache`, on-demand set compilation and last-good-pipeline retention ([fast shader reload](../fast-shader-reload.md)). Nothing here is faster. Async creation is the only pattern worth borrowing, and Renderlab already lists it as a next step.

### A6 kajiya

[EmbarkStudios/kajiya](https://github.com/EmbarkStudios/kajiya): MIT OR Apache-2.0, archived 2026-06-01, "no longer maintained" (V). HLSL in `assets/shaders/`, plus rust-gpu shaders. From [gi-overview.md](https://github.com/EmbarkStudios/kajiya/blob/main/docs/gi-overview.md) (V):

**ReSTIR GI** (`rtdgi/`):
- Half-resolution rays, temporal M clamped to 10, and two spatial passes (8 and 5 samples).
- Every third frame is a validation frame that re-traces stored samples.
- Near-field hits bypass ReSTIR so that spatial reuse does not erode contact detail.

**Irradiance cache** (`ircache/`):
- 12 camera-aligned 32³ clipmaps, at most 65,536 entries, filled only by other rays' queries.
- Voting picks the trace origins.
- Irradiance comes from 4×4 octahedral reservoirs converted to L1 SH.

**Shadows** use a modified MIT FidelityFX Shadow Denoiser (`shadow_denoise/ffx`).

**Cost:** 8.4 ms total at 1080p on an RX 6800 XT. Indirect diffuse 2.3 ms, ircache 0.55, reflections 2.2, sun shadows 0.52.

**Limits** (stated by kajiya): leaks on thin geometry, a "very laggy" bounce without validation, and an ircache that is not temporally stable.

**Verdict:**
- Take: the validation-ray idea, the near/far split and the cache.
- Do not take as core:
  - SSGI;
  - screen-space reprojection of hit radiance;
  - the half-resolution depth-buffer micro-shadow march.

### A7 Filament

[google/filament](https://github.com/google/filament), Apache-2.0 (V). The specific takes:
- **Geometric specular AA** in `shaders/src/surface_lighting.fs`, `normalFiltering()` (V).
  - Kernel roughness is `min(2·variance, threshold)` from normal derivatives. It is added in squared-roughness space, citing Kaplanyan 2016 and Tokuyoshi–Kaplanyan 2019.
  - Filament uses `dFdx`/`dFdy` of the geometric normal. Renderlab's traced compute path would feed it ray-differential normal variance.
  - This targets the "fine two-signed speckle over every rough face" that [film gap](../m6-film-gap.md) attributes to shading aliasing. It approximates the footprint integral, so it is not a quality reduction, but it must pass the film gate rather than be assumed to (I).
- **BRDF and DFG** in `surface_brdf.fs` and `shaders/src/surface_shading_model_standard.fs`.
- **IBL prefiltering** in `libs/ibl/src/CubemapIBL.cpp` (cmgen).
- **Roughness prefilter** in `tools/roughness-prefilter/src/main.cpp`, which bakes normal-map variance into roughness.

### A8–A14 other engines and renderers (license verified via the GitHub API and file heads)

**Mitsuba 3** (BSD-3).
- The current `include/mitsuba/render/interaction.h` offsets by a per-shape error bound: `offset_p(d) = p + mulsign(p_err, n·d)·n`. Ray error is `dot(|n|, |o| + |d·t|)·PositionEpsilon`, with `PositionEpsilon = 32·unit roundoff` (V).
- It is a second error-bound policy beside pbrt's. Mitsuba is also usable as an independent offline path-traced reference for the film gates (I).

**Embree** (Apache-2.0). Its robust mode is a CPU oracle.

**Wicked Engine** (MIT).
- `WickedEngine/wiGPUBVH.cpp`, `shaders/{bvh_hierarchyCS,bvh_primitivesCS,bvh_propagateaabbCS}.hlsl` and `raytracingHF.hlsli` provide a software GPU BVH.
- `rtshadowCS.hlsl` with `rtshadow_denoise_{tileclassification,temporal,filter}CS.hlsl` and `rtaoCS.hlsl` handle RT shadows and AO.
- `ddgi_raytraceCS.hlsl`, `surfel_raytraceCS.hlsl` and `raytraceCS.hlsl` each have an `_rtapi` twin (V, file list).

**Godot 4** (MIT). `renderer_rd/shaders/environment/sdfgi_*.glsl` and `voxel_gi*.glsl`. The ubershader pattern is covered in §C.

**The Forge** (Apache-2.0). `Examples_3/Visibility_Buffer{,2}`.

**O3DE** (Apache-2.0, or MIT at the user's option, plus third-party components including LGPL Qt; V). Not inspected further.

**Falcor** (BSD-3 for Falcor itself). Its `LICENSE.md` says DLSS, RTXGI, RTXDI and NRD "are not licensed under the above license text" (V).

## B. Edge-hit and self-intersection robustness

### B1 What the specs guarantee (V)

**DXR, "Fixed function ray-triangle intersection specification".**
- "The implementation must use a ray-triangle intersection that is watertight". This holds anywhere in the 32-bit float range, without double-precision fallbacks.
- "A single ray striking an edge … must report an intersection with only one of the incident triangles", using some top-left rule.
- Scope is only a given BLAS "for geometries that have matching transforms".
- Ray extents for triangles are `TMin < t < TMax`. In inline traversal, a non-opaque candidate is one the system "has already determined … would be the closest hit so far".
- Nothing bounds the precision of t, or of the ordering between two *different* triangles that the ray truly crosses at nearby t.

**Vulkan, [Ray Traversal](https://docs.vulkan.org/spec/latest/chapters/raytraversal.html).**
- Intersection "may be performed with floating-point operations". "Inaccuracies are expected". Implementations "should take efforts to maintain as much precision as possible".
- Watertightness: implementations "should not double-hit or miss when a ray intersects a shared edge", within one instance, for edges with matching winding. It is "expected and tested", but phrased as *should*.
- Closest hit: a candidate becomes closest when `t < tmax`. "If t = tmax, the candidate may be set as the current closest hit or dropped."
- "There is no ordering guarantee between operations performed on different intersection candidates."

**Consequence.** Renderlab's witnesses are not shared-edge watertightness failures. In each, the ray truly crosses *both* triangles, 0.24–3.5 µm apart in t, and the hardware commits the farther one (determinism.md). No spec promises sub-ulp ordering of distinct true crossings, so no hardware guarantee can be relied on. The fix must come from the renderer (I, from the spec text).

### B2 Published spawn-point and offset methods

| Method | Hit point | Offset | t_min | At quarry scale (M) | Concave folds |
|---|---|---|---|---|---|
| **Wächter & Binder 2019** (RTG ch. 6; [code](https://github.com/Apress/ray-tracing-gems), MIT) | "As precisely as possible" (barycentrics) | `int_scale = 256` ulps of each coordinate along n; `float_scale = 1/65536` below `origin = 1/32` (V, via Cycles' Apache copy in `kernel/bvh/util.h`) | 0 | 61 µm at 3 m, 122 µm at 5–7 m | Larger offset, so a wider fold band |
| **pbrt-v4** `OffsetRayOrigin` (V, `src/pbrt/ray.h`) | Barycentrics with `pError = γ7·Σ|bᵢpᵢ|` (pbr-book 4e §6.8, V) | `d = dot(|n|, pError)`, flipped to the side of the outgoing direction w, then `NextFloatUp/Down` away from the surface | none needed | γ7·|p| ≈ 1.25 µm at 3 m, 2.9 µm at 7 m per component | Not discussed (V). Conservative only for its *own* plane |
| **NVIDIA, van Antwerpen 2023** ([blog](https://developer.nvidia.com/blog/solving-self-intersection-artifacts-in-directx-raytracing/); [GLSL/HLSL](https://github.com/NVIDIA/self-intersection-avoidance) BSD-3) | Object space `v0 + fma(b.x, e1, b.y·e2)` under `precise`; hand-expanded object-to-world with translation last | `objErr = c0·|v0| + c1·extent` plus world-to-object traversal error; `wldErr = c1·|M||p| + c2·|T|`; projected on |n|. `c1 = 3·2⁻²⁴`, "tuned for NVIDIA RTX hardware". Shadow rays shorten `tmax` by 1 ulp | 0 | **0.98 / 0.62 / 1.25 µm** on the three witness triangles, with their actual 0.01-scale instance matrices | Not mentioned (V). Smaller, so a narrower band |
| **Cycles** (V, `shade_surface.h`, `light/sample.h`, `bvh/util.h`, OptiX `bvh.h`) | `triangle_point_from_uv` (edge form, "slightly better precision") | None by default. Self-prim **skip** (`intersection_skip_self*`). `integrate_surface_ray_offset` applies the W&B offset only if `ray_triangle_intersect_self` *misses*, i.e. "precision issues can still cause neighboring triangles to be hit" | 0 | — | Plus an optional terminator offset (Hanika), which is zero in concave regions |
| **Mitsuba 3** | Shape-reported `p_err` | `mulsign(p_err, n·d)·n` | — | ≈ 32·2⁻²⁴·|p|·|n|₁ ≈ 12–25 µm (I; triangle `p_err` not inspected) | Not discussed |
| **Bevy Solari** | Barycentrics | none | **1 mm** | — | — |
| **Unreal path tracer / HRT** | — | Already documented from the licensee source in [film gap](../m6-film-gap.md): `max(2⁻⁸ cm, 16 ulp)` and 1 mm plus 1 cm near trace | — | 39 µm | — |

**Correction to the decision log.** `milestones-5-6-decisions.tsv` says the adopted `max(10 µm, 16 ulp)` viewed-side policy "matches Cycles and UE path tracer". It matches only the *viewed-side sign* and the UE magnitude form. Cycles uses **no positional offset** by default. It uses an exact surface origin, `tmin = 0` and primitive exclusion, and it pays for that with `OPTIX_RAY_FLAG_ENFORCE_ANYHIT` on every trace call (V, `kernel/device/optix/bvh.h`). It adds the 256-ulp offset only when the own-triangle retest fails.

**Cost of the prim skip on hardware ray queries.** It needs non-opaque candidate processing. NVIDIA calls forcing any-hit "a significant performance penalty" (V). MegaLights reports that even front- or back-face culling "adds about 10% overhead to tracing, since it prevents some early exits" (V, slides, section "Mesh Representation Mismatches"). That is an Unreal console figure, not an RTX 4070 SUPER one.

### B3 Does pbrt-v4's bound remove the concave-fold false shadow?

**Not by itself (I, geometry).** Take a concave crease with interior dihedral angle φ, and a receiver on face A at distance x from the crease. Its origin is offset by ε along n_A. The origin crosses behind the neighbour plane B when `x·sin φ < ε·cos φ`, i.e. `x < ε·cot φ`.

That happens only for folds sharper than 90° (`n_A·n_B = −cos φ < 0`). Shallower concave folds can never cross, at any ε. The error-bound methods (pbrt, NVIDIA) shrink ε from 10 µm to about 1–3 µm here (M, above), so the affected band shrinks 3–16×. It does not vanish, because those bounds protect only the receiver's own plane.

The cases that remove it:
- **Offset direction, not magnitude.** An origin offset along any direction u with `u·n_A > 0` and `u·n_B ≥ 0` cannot cross either plane. The bisector `n_A + n_B` satisfies both for every fold: `n_B·(n_A+n_B) = 1 − cos φ > 0`.
  - That needs n_B, which is what the adjacency table provides.
  - A cheaper proxy is the interpolated **area-weighted geometric vertex normal**. Near an edge it tends to the average of the faces incident on that edge's vertices. It satisfies `N·n_B ≥ 0` for a two-face fold. It is not guaranteed over a many-face one-ring (I).
  - This is direction-only: ε stays the adopted policy magnitude.
- **No positional offset** (Cycles), with own-primitive exclusion. This costs any-hit handling (above).
- **A short back-face-culled first segment.** A false hit at a concave fold is always a *back-face* hit on B, because the origin sits behind B. Culling back faces on `[0, L]` with L ≈ 1 mm, then tracing `[L, ∞)` without culling, removes it.
  - Cost: two traversals plus the culling overhead (MegaLights: about 10%).
  - Risk: a genuine back-face occluder within L at an open scan boundary would be missed (I). Not recommended over the inset.

### B4 The primary edge-hit: a cheaper fix than adjacency (P1)

**Observation (M).** These are the three returned (farther) witness triangles from `rays.json`, with the ray direction mapped into object space through each instance's model matrix (det > 0). In all three, the winding normal points **along** the ray: `cross(e1,e2)·d_obj` = +1.70, +0.28, +5.95. The stored normal gives n·d = +0.043, +0.081, +0.213. So all three returned triangles face away from the camera under the triangles' winding.

**Inference.** On a consistently oriented surface, a ray that crosses two edge-adjacent triangles near their shared edge enters through one face and leaves through the other's back. The nearer crossing is front-facing, so the mis-ordered farther hit is back-facing. That matches the witnesses and the proof's "nearer facet positive N·L, farther negative".

**Technique.**
1. Trace the primary as today, with no culling and opaque traversal.
2. If the committed hit has `front_face == false` and lies within an error bound of an edge (min barycentric × opposite altitude < k·ulp(t), or the NVIDIA bound), re-trace once from the same origin with `RAY_FLAG_CULL_BACK_FACING` (naga `RayFlag::CULL_BACK_FACING`, V in naga 30) and `tmax = t + bound`.
3. Accept the re-trace hit only if `|t′ − t| ≤ bound`. Otherwise keep the original. Genuine back-face views of open scan shells therefore stay unchanged.

Properties (I):
- It is stable whichever neighbour traversal returns first. A front-facing near-edge hit never re-traces. A back-facing one re-traces to the same front facet.
- It needs no adjacency table, slope comparison or neighbour fetch. It also covers neighbours that are not index-adjacent: other draws, T-junctions and overlapping scan shells.
- Cost: one extra ray query on far fewer than 1% of pixels, gated by a cheap predicate. Its real cost depends on whether it stays inside the fused primary pass under a wave-uniform branch, or needs a compacted cold pass. If the current +5% is mostly the cold pass's fixed overhead, fusing is the lever.
- wgpu 30 `TlasInstance` has no instance flags, only `mask` and `custom_data` (V). So per-instance `FACING_CULL_DISABLE` for thin materials is unavailable. The ray-flag form above does not need it.

**Fallback (P7, Wald RTG II ch. 34).** Use this when facing is ambiguous, e.g. thin two-sided materials. Re-trace from `o′ = o + (t − δ)d` with δ ≈ 1 mm and `tmax = 2δ`. Woop-style intersection subtracts the origin from the vertices. With o′ within about 1 mm of the vertices, those differences are near-exact and t is about 1e-3 m, so ordering resolves to far below the 0.24 µm worst witness gap (I). The new ray is laterally displaced by up to about 0.5 ulp of o′'s coordinates, under 0.5 µm here, so it is a consistent ray, not the identical one. On a `primary_ray`-style fused pass this matches P1's cost class.

**Reproducer before any production change** (one GPU process, existing harness):
1. Add `front_face` and both re-trace results to the `receiver_diagnostic` replay of the three witness rays.
2. Then run one quarry Interactive capture, counting:
   - pixels whose committed hit is back-facing and near an edge;
   - how many re-trace to a front face within the bound;
   - byte differences against the current adjacency fix.
3. Price it with paired in-process timing against the adjacency fix.

**Assessment.** No published technique fixes misordered closest hits more cheaply than an adjacency check *in general*. The literature addresses self-intersection, not ordering between distinct true crossings. The facing-gated re-trace (P1) uses information the hardware already returns. It is the best candidate to undercut +5% (I, unmeasured). For shadows, swapping the 10 µm magnitude for the NVIDIA bound narrows but does not remove fold leaks, and it is a receiver-policy change needing Islam's decision. The direction-only bisector or vertex-normal offset removes them for two-face folds without adjacency (I).

### B5 Arithmetic notes relevant to determinism

- **No precision qualifier.** NVIDIA's sample relies on `precise` (SPIR-V `NoContraction`) for the hit-point arithmetic. WGSL has no such qualifier, and naga 30 emits no `NoContraction` (V: no occurrence in `naga-30.0.1/src`).
  - Explicit `fma()` fixes the evaluation order, which Renderlab already does.
  - `Features::PASSTHROUGH_SHADERS` with `create_shader_module_passthrough` (V, wgpu-types 30) is the only route to `NoContraction`. Renderlab's determinism tests already use it.
  - The "GPU folded away compensated residuals" finding is consistent with optimizers treating `a*b − fma(a,b,…)` as contractible without `NoContraction` (I).
- **The non-RT backend can be exact by construction.** pbrt-v4's watertight test reruns in double precision when an edge function is zero ("less than 0.0000023% of the cases", V). The equivalent for WGSL is an exact or expansion comparison of the two candidate t values when `|t_a − t_b| <` bound, on the software path only (I).

## C. Fast shader iteration

**wgpu 30 facts (V, local `~/.cargo/registry` sources).**
- `Features::PIPELINE_CACHE` is supported on Vulkan only.
- `PASSTHROUGH_SHADERS` exists.
- `create_shader_module_trusted(desc, ShaderRuntimeChecks)` can drop bounds-check code.
- `EXPERIMENTAL_RAY_TRACING_PIPELINES` and `EXPERIMENTAL_RAY_HIT_VERTEX_RETURN` exist.
- There are **no** pipeline-creation flags such as `DISABLE_OPTIMIZATION`.
- There is no `VK_EXT_graphics_pipeline_library` or `VK_EXT_shader_object`: no occurrence in `wgpu-hal-30.0.1/src`, and no wgpu issue found.
- There is no `create_*_pipeline_from_hal`. Only devices, textures and buffers can be imported, so a raw ash pipeline cannot be wrapped.
- WGSL `override` constants are baked by naga (`pipeline_constants::process_overrides`) at pipeline creation. Every override value is a full driver compile, not Vulkan specialization (V, `wgpu-hal/src/vulkan/device.rs`).

**Other engines.**
- Bevy: async creation only, no driver cache, old pipeline dropped (§A5).
- Godot 4.4 (V, [dev4 post](https://godotengine.org/article/dev-snapshot-godot-4-4-dev-4/)): a load-time ubershader with all features, switched by specialization constants. Specialized pipelines compile in the background and replace it when ready.
- That pattern presumes enumerable *engine* variants. Renderlab's variable is arbitrary user code, so an ubershader cannot pre-cover an edit (I).

**Levers for the 100 ms target, ranked (I unless noted).** The measured facts are a driver compile of about 1 s per program pipeline, with the inlined environment code at about 80% of it, and a flat program compiling in 22–58 ms ([fast shader reload](../fast-shader-reload.md), M there).

1. **Keep heavy engine code out of the user program's pipeline.** The flat-program number shows the driver is fast when the program is small.
   - Do the environment lookup once per level instead of 15 inlined bilinear filters (already identified: 37–42% saved).
   - Beyond that, move the environment term into an engine-owned pipeline that program edits never recompile. It would consume the program's `surface()` outputs, e.g. a deferred environment pass.
   - This must keep `shade()` programmable. A program that needs the environment term would read it as a precomputed input, and that constraint must be recorded per programmable-shading rules.
2. **Async creation with last-good presentation.** This does not shorten edit-to-image, but removes the viewer stall (already listed).
3. **Two-tier compile.** Create an unoptimized pipeline first (`VK_PIPELINE_CREATE_DISABLE_OPTIMIZATION_BIT`), then the optimized one in the background.
   - This needs a small wgpu-hal patch, because wgpu exposes no flags (V).
   - Whether NVIDIA compiles much faster under that flag is **unknown**; measure with one driver-only probe before patching.
   - The preview pipeline must be byte-identical in output, or it must be labelled preview-only, because image gates run on the optimized pipeline.
4. **`ShaderRuntimeChecks::unchecked()` for engine-owned modules only.** It removes index clamps from the SPIR-V. Its compile-time effect is unmeasured. User programs should stay checked.
5. **Persistent `wgpu::PipelineCache` data.** This only helps drivers without NVIDIA's disk cache. Renderlab's notes record that NVIDIA's disk cache serves cross-process repeats, and that per-process unique edits are needed to measure cold compiles.

## D. Unreal public material (algorithms only, no code)

| Talk or doc | URL (V) | Idea relevant here |
|---|---|---|
| **MegaLights: Stochastic Direct Lighting in UE5** (Narkowicz, Costa; SIGGRAPH 2025 Advances) | [PDF](https://advances.realtimerendering.com/s2025/content/MegaLights_Stochastic_Direct_Lighting_2025.pdf) | See below |
| **Lumen** (Wright, Narkowicz, Kelly; SIGGRAPH 2022 Advances); **Radiance Caching for Real-time GI** (Wright, 2021) | [s2022 course page #Lumen](https://advances.realtimerendering.com/s2022/index.html), [s2021 course page](https://advances.realtimerendering.com/s2021/index.html) | Surface cache (mesh cards) and world-space radiance cache are usable for future GI on both backends. Mesh-SDF/global-SDF tracing is the non-RT denominator. Screen probes and screen traces conflict with the screen-space rule ([reuse-non-rt](reuse-non-rt.md) L3) |
| **A Deep Dive into Nanite Virtualized Geometry** (Karis, Stubbe, Wihlidal; SIGGRAPH 2021) | [PDF](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf) | Two-pass HZB occlusion culling, a visibility buffer written with 64-bit `InterlockedMax`, SW raster for clusters "with edges <32 pixels", "3x faster than hardware on average" (V, slides). This is the non-RT raster visibility producer ([non-RT path](../non-rt-path.md) decision 2) |
| **Virtual Shadow Maps** (docs) | [docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine) | Page caching and SMRT ray counts. Already rejected as core by measurement; read for page-cache invalidation only |
| **Authoring Materials That Matter – Substrate in UE5** (Hillaire, de Rousiers; SIGGRAPH 2023 Advances) | [s2023 course page #Strata](https://advances.realtimerendering.com/s2023/index.html) | Slab or closure-based layered materials with bounded per-pixel closure counts and tile classification. Relevant to keeping `surface()`/`shade()` general without a lighting-model-specific hit record (I: slides not re-read here) |
| **TSR** | No public talk: Abadie says TSR publications "had to be abandoned two year in row" ([forum](https://forums.unrealengine.com/t/tsr-feedback-thread/883977/20), via search). Docs plus [Karis 2014 TAA](https://advances.realtimerendering.com/s2014/index.html) | History rejection and resurrection ideas only. Renderlab rejected colour-clamp TAA ([film gap](../m6-film-gap.md)); details in [reuse-temporal-denoise](reuse-temporal-denoise.md) §1.5 |

MegaLights details (V, slides):
- **Fixed budget.** It traces a fixed number of rays per pixel regardless of light count, with "1 trace per pixel ≈ 0.8 spp".
- **Sampling.**
  - Plain weighted reservoir sampling (Efraimidis 2006) plus spatio-temporal blue noise.
  - No ReSTIR reuse, because "Reservoir resampling adds a high constant cost" and "Denoisers don't like correlation".
  - Guiding comes from the previous frame's per-8×8-tile visible-light list, with a hidden-light reservoir clamped to 20% and a stochastic bilinear tile lookup.
- **Denoising.** One SVGF-style temporal-variance denoiser runs for all lights, on demodulated diffuse and specular.
- **Mismatches.** Proxy-mesh mismatches cause self-shadowing. "Front / back face culling not enough … Would also add 10% overhead to tracing". The fix is HZB screen traces that offset the world-trace origin.
- **Transmission.** Flip the normal bias and offset toward the light.
- **Cost.** PS5, 1080p, 1 spp, 941 area lights: 5.51 ms total.
- **Non-RT.** VSM per light is the official alternative. The slides call it expensive for many lights; distance-field fallback is only community-reported.

Renderlab traces full-detail geometry, so it has no proxy mismatch and needs no screen traces. Take the fixed-budget WRS, tile guiding and single-denoiser structure for many-light work, on both ray backends.

## Recommended first takes

1. **P1: the front-face retrace for primary edge hits.**
   - Why: it uses `front_face` that the hardware already returns. It removes the adjacency table and slope test from the primary path, and covers non-index-adjacent neighbours.
   - First step: the witness replay plus one counted quarry capture (B4).
   - Risk: scan winding may be inconsistent in places, and an open shell can be viewed from behind. The t-window acceptance keeps genuine back-face views. Cost depends on staying inside the fused pass.
2. **P2: port NVIDIA `safeSpawnOffset` to WGSL as a measured bound, not a policy change.**
   - Why: it gives a per-hit, hardware-tuned error bound of about 1 µm here. It quantifies how much of the 10 µm floor is margin. It also feeds P1's edge bound and the software backend's tolerances.
   - Risk: c1 is "tuned for NVIDIA RTX hardware". Changing the receiver magnitude re-baselines every gate and needs Islam's decision.
3. **Direction-only fold-safe offset** (geometric vertex normal, or the bisector where adjacency exists), tested on the existing concave sawtooth fixture.
   - Why: it removes fold crossings for two-face folds without changing the adopted magnitude.
   - Risk: it is not guaranteed for many-face one-rings, so keep the inset as the fallback where `N·n_B` cannot be checked.
4. **Filament `normalFiltering` (Tokuyoshi–Kaplanyan) driven by ray-differential normal variance.**
   - Why: it targets the lit-pixel speckle that dominates the film radiance error.
   - Risk: it is an approximation that must pass Level F against the film reference, static and in motion. It must never be assumed to.
5. **Compile latency: environment code out of the program pipeline, then a one-shot `DISABLE_OPTIMIZATION` driver probe.**
   - Why: the flat program compiles in 22–58 ms, so the program's own size is not the obstacle.
   - Risk: the split must not block user `shade()` access to lighting terms. The wgpu-hal patch is only worth it if the probe shows a large win.

Also take, no risk: Bevy atmosphere LUTs and Tony McMapface (MIT/Apache) for the display and sky pipeline. Avoid: RTX SDK code (RTXDI, NRD, RTXGI), FidelityFX SDK main-branch code, the AgX LUT from AgXc, Unreal and Unity source.
