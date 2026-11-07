# Glossy and specular reflections: survey for M7

Research date 2026-11-07, base `cf001a5`. **Web and source reading only.** No build, GPU run, window, prototype or commit. Reading the local UE 5.7 install's `Engine/Config/BaseScalability.ini` is the only local evidence. It holds configuration values only; no Unreal source was opened.

**Labels**
- **V:** verified at the cited URL, file or repo LICENSE during this session.
- **M:** measured in a cited Renderlab record.
- **I:** inference or estimate, not measured.

Companion surveys, not repeated here:
- [open-source-reuse.md](open-source-reuse.md): licence rule and ranked takes.
- [reuse-engines-precision.md](reuse-engines-precision.md): Bevy Solari, kajiya, Filament, ray-origin policy, Unreal talks.
- [reuse-non-rt.md](reuse-non-rt.md): software BVH, Brixelizer, SDFGI, Lumen SW.
- [reuse-temporal-denoise.md](reuse-temporal-denoise.md): denoisers, ReSTIR, temporal reuse.
- `gi.md`: being written in parallel; GI and the radiance cache belong there.

Renderlab inputs: [non-rt-path.md](../non-rt-path.md), [non-rt-path-2.md](../non-rt-path-2.md), [N0](../non-rt-n0.md), [N1c](../non-rt-n1c.md), [N8o](../non-rt-n8o.md), [m5-environment-runtime.md](../m5-environment-runtime.md), [m5-environment-field.md](../m5-environment-field.md), [programmable-shading.md](../programmable-shading.md) and [quarry-end-to-end-comparison.md](../quarry-end-to-end-comparison.md).

## Licence

The rule is [open-source-reuse.md § Licence rule](open-source-reuse.md#licence-rule). Facts specific to reflections:

| Code | Licence | Verdict |
|---|---|---|
| AMD [FidelityFX-SSSR](https://github.com/GPUOpen-Effects/FidelityFX-SSSR) standalone (tags v1.0–v1.3; GPUOpen lists 1.4) | `license.txt` MIT, "Copyright (c) 2021 Advanced Micro Devices" | **COPY with notice** |
| AMD [FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser) `ffx-reflection-dnsr/` (common, config, prefilter, reproject, resolve_temporal) | MIT | **COPY with notice** |
| FidelityFX SDK **v1.1.4**: `sdk/src/components/{sssr,denoiser,classifier,brixelizer}`, `samples/hybridreflections` | Root `LICENSE.txt` MIT, "Copyright (C) 2024" | **COPY**, pinned to v1.1.4 only. Current main is binary-only ([open-source-reuse](open-source-reuse.md)) |
| Bevy `crates/bevy_pbr/src/ssr/raymarch.wesl` | File header: "Copyright (c) 2023 Tomasz Stachowiak", MIT OR Apache-2.0. It ports his `raymarch.hlsl` gist | **COPY**, as a refinement only (screen-space) |
| kajiya `assets/shaders/rtr/`: `reflection.rgen`, `reflection_validate.rgen`, `rtr_restir_temporal`, `resolve`, `spatial_cleanup`, `temporal_filter` | MIT OR Apache-2.0 ([prior survey](reuse-engines-precision.md#a6-kajiya)) | **COPY** (HLSL to WGSL). Its screen-irradiance reuse is not core |
| Bevy Solari `crates/bevy_solari/src/realtime/` | MIT OR Apache-2.0 | **COPY** selectively. Its ray policy is rejected ([prior survey](reuse-engines-precision.md)) |
| NVIDIA NRD (ReBLUR, ReLAX), SHARC, RTXDI, RTXPT | `LICENSE.txt` / `License.md`: "NVIDIA RTX SDKs LICENSE" | **AVOID code.** Papers only |
| Unreal source | EULA | **AVOID.** Docs, talks and config values only |
| Ray Tracing Gems ch. 25 (PICA PICA hybrid rendering) | CC BY-NC-ND 4.0 | **REIMPLEMENT-ONLY**, from the text |

**Patents found (V; no freedom-to-operate check was done):**
- **US 11,315,310** (NVIDIA, granted 2022-04-26): "Reservoir-based spatiotemporal importance resampling utilizing a global illumination data structure" ([Justia](https://patents.justia.com/patent/11315310)). It covers ReSTIR used with a GI cache. The prior survey's ReSTIR-family and SVGF (likely US 10,600,167) warnings still apply.
- **Adshir Ltd, now apparently Snap** ([Google Patents](https://patents.google.com/patent/US10950030B2/en)): a family on hybrid ray-traced specular reflections. It covers raster primaries plus coherent secondary reflection rays and dynamic acceleration structures. Members: **US 10,950,030 B2**, US 10,699,468, US 11,302,058, US 12,051,145, US 12,573,127, and pending US 2025/0363721.
  - Before shipping any "raster G-buffer + coherent reflection rays" design, read the claims. That covers the non-RT path, which rasterizes primaries.
- Not searched: Epic patents on Lumen, AMD patents on SSSR or Brixelizer.

## Summary

Costs are as published. Hardware and resolution are stated exactly; "none" means nothing was published.

| Technique | RT path | Non-RT path | Published cost (hardware, resolution) | Memory | Motion and disocclusion failure | Reusable code + licence | Verdict |
|---|---|---|---|---|---|---|---|
| **Unreal Lumen reflections** (5.0–5.7) | HWRT: surface-cache lighting or hit lighting | SW: global SDF; mesh SDF deprecated in 5.6/5.7 | PS5, 1080p internal: reflections **0.23 / 0.24 ms** (Land of Nanite, High/Epic), **1.65 / 3.30** (Lyra SW), **2.30 / 4.62** (Matrix HWRT). 2080 Ti: **1.56** (Lake House). Matrix hit lighting **11.54** against surface cache **2.44** (V) | Surface cache plus SDFs; not published | Screen traces first; a disocclusion-forced bilateral filter; GI lag of "multiple seconds" (V) | None (EULA) | **The competitor.** Match settings below |
| UE standalone RT reflections (4.22–5.3) | Yes | — | none found | — | "No proper specular occlusion" (V, Lumen 2022) | — | History only. Removed in 5.4 (V, forum) |
| **SSR / stochastic SSR** (Stachowiak 2015; Hi-Z, Uludag 2014) | Refinement only | Refinement only | PS4, 1600×900, all pixels reflective: **2.19 ms** (1 ray per half-res pixel, 4 resolve samples) to 4.41 ms (V) | Hi-Z plus history | Off-screen, back faces, thickness, disocclusion (V) | FidelityFX SSSR MIT; Bevy SSR MIT/Apache | **AVOID as core.** At most a verified hint (§2) |
| SSR + traced cubemaps (Mafia III) | — | Probe hybrid | PS4, 1080p: **3.3 ms** total; 8 active 512 px geometry cubemaps (V) | 8 × 512² cubes plus G-buffer cubes | Probe placement, thin objects, dynamic objects (V) | — | Reject as core: probe-parallax failures |
| **Relit reflection probes** (Ghost of Tsushima) | — | Far-field and rough fallback | Base PS4: **700 µs per probe**, one probe per frame round-robin (V) | 256² × 6 BC1 albedo + BC6H normal/depth per probe; 235 probes (V) | Parallax; no dynamic geometry; slow round-robin | — | Pattern only. The prepared HDR field already covers distant light |
| **Planar reflections** | Raster or RT | **Raster mirror view, exact** | UE docs: "entire scene … rendered twice"; Kite demo +23.07 ms (V; old hardware, unstated) | One extra visibility buffer | None for a true plane; planes only | — | **Keep for flat mirrors and water** (non-RT) |
| Mesh SDF + global SDF (Lumen SW) | — | Glossy and rough | See the Lumen row | Not published | Leaks; SDF surface bias trades over-occlusion for leaking (V); "HWRT the only way" to good mirrors (V) | None | **Reject for mirrors**; possible for glossy far field |
| **Brixelizer sparse SDF** (SDK v1.1.4) | — | Glossy far field, rough visibility | GI total: RX 7900 XTX ~2 ms (4K), RTX 4080 ~2.2 ms (4K), RX 7600 XT ~1.7 ms (1080p) ([reuse-non-rt](reuse-non-rt.md)) | GI ~200 MB at 1080p | Its radiance cache is filled from the screen (V); "loses small-scale details" (V) | MIT (v1.1.4) | **SDF only, never its radiance cache** |
| Godot SDFGI reflections | — | Rough only | none | Cascades | No dynamic occluders; 25-frame light lag; cascade shifts (V) | MIT | AVOID as core |
| Voxel cone traced specular (Crassin 2011) | — | Glossy | 25–70 fps whole frame, GTX 480, 512³ octree (V) | Sparse octree | Leaks; voxel blur; no mirrors | — | Reject (prior survey) |
| **Software BVH reflection rays**, full detail | — | Exact, same estimator | Renderlab N0: camera rays **11.9×** and shadow rays **21.9×** hardware; primaries 2.84 ms (M). tray_racing: 5.4× (RTX 4070 Ti) | BVH **412.5 MB** (M) | None beyond the estimator's | obvhs, tray_racing MIT/Apache | Too slow and too large at full detail (I) |
| **Cluster-LOD BVH, cone-sized** | Optional | **Main non-RT candidate** for glossy rays | none | I: ~30–60 MB for a ~1–2 M-triangle cut | LOD pops; near-field contact error | meshoptimizer clusterlod (in the cook) | **Probe in M7** (§9) |
| **HW RT reflections + hit shading** (BFV, PICA PICA, kajiya, Lumen hit lighting) | **Main RT candidate** | — | BFV pipeline **6.29 ms**, GPU and resolution not stated in the slide text (V). kajiya **2.2 ms**, RX 6800 XT, 1080p, half-res (V) | Hit records plus history | Depends on the denoiser (below) | kajiya MIT/Apache | **Take the structure** |
| Surface-cache hit lighting (Lumen default) | Fast mode | Same | 2.44 ms against 11.54 ms hit lighting (Matrix, PS5) (V) | Card atlas | Black where cards are missing; cached output lags | — | **Conflicts with programmable shading** (§5) |
| Reflection denoisers: NRD ReBLUR/ReLAX; FidelityFX reflections; Lumen spatial+temporal+bilateral | Both | Both | NRD RTX 4080, 1440p: ReBLUR diffuse+specular **2.55 ms**, ReLAX **3.25 ms** (V). FidelityFX: none | 2–4 full-res history targets | Ghosting and lag; reprojected screen history | FidelityFX MIT; NRD AVOID | Mirror band undenoised; glossy band gated on motion |
| ReSTIR for specular (GI / PT, Solari, kajiya temporal) | Glossy band | Same | Solari 0.20: ReSTIR off by default ("4 rays for … two extra reservoirs") (V) | Reservoirs, 48 B (Solari) | Correlation on smooth specular; "only a hard roughness cutoff worked" (V) | kajiya, Solari MIT/Apache | Later, after patent review |
| Ray cones (Akenine-Möller 2019/2021) | Required | Required | none for cost | 4 B per ray | — | Falcor `TexLODHelpers.slang` BSD-3 | **Take**: hit footprints with no derivatives |
| Specular occlusion (GTSO, Frostbite, GoT horizon, traced lobe visibility) | Implicit where traced | Implicit where traced | GoT horizon occlusion ~3% of deferred lighting (V) | — | Screen-space GTSO is barred as core | — | Rough band: **traced-visibility ratio estimator** (§8) |
| Specular AA (Kaplanyan 2016; Tokuyoshi–Kaplanyan 2019/2021) | Both | Both | none | — | Reduces specular shimmer | Filament Apache-2.0 | **Take** (already ranked #9) |

## 1. Unreal Lumen reflections, 5.0–5.7

### How it works (own words)

**Pipeline** (V, [Lumen SIGGRAPH 2022](https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf), reflections section):
1. Classify screen tiles, then compact the tile list in z-order to keep ray origins coherent.
2. Generate rays by importance sampling the visible GGX lobe.
3. Trace.
4. Spatial reuse: screen neighbours' hits, reweighted by the local BRDF (after Stachowiak 2015/2018).
5. Temporal accumulation.
6. A bilateral filter, run only where variance stays high after spatial reuse. It is forced at double strength on disoccluded pixels and uses tonemapped weights to remove fireflies.
7. Full-frame TAA/TSR does the final cleanup.

The whole pipeline reruns for front-layer translucency and for water.

**Trace order** (V, [technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine)): screen traces first. When a ray finds nothing on screen or passes behind a surface, Lumen falls back to SWRT (SDF) or HWRT.
- HWRT is "the only way to achieve high quality mirror reflections."
- A Nanite fallback mesh (a simplified proxy) is what HWRT hits. Its mismatch against screen traces causes self-intersection, tuned by **Fallback Relative Error**.
- Renderlab traces full detail, so it has no proxy mismatch.

**Hit lighting** (V, 2022 slides):
- **Surface-cache mode** forces every BVH instance opaque, so there is no any-hit shader. One closest-hit shader fetches only the normal and card parameters, through a 20-byte payload instead of the old 64-byte G-buffer payload. Lighting is a normal-weighted surface-cache lookup that already contains direct and indirect light.
- **Hit-lighting mode** traces the surface-cache pass first, then sorts by material and retraces with full material and light evaluation. This is the sorted-deferred scheme (Aaltonen 2018; Kelly et al. 2021).
- Indirect light at hits still comes from the surface cache.
- Partial dynamic evaluation was "just as expensive as full".
- **Opacity:** translucent meshes are skipped. Alpha-masked surfaces are skipped when surface-cache opacity is below 50%, iterating in the ray-generation shader up to `MaxTranslucentSkipCount`.
- **Surface cache for reflections:** sparse on-demand high-resolution card pages, allocated from reflection-hit GPU feedback. For mirrors, cards must match screen pixel density.

**Roughness bands** (V):
- 2022 slides:
  - **0–0.3** traces dedicated rays.
  - **0.3–0.4** shortens the ray and interpolates the world radiance cache on a miss (32×32 directional resolution). This cut road reflections by 16% in Matrix and removed binning.
  - **0.4–1** traces no ray. It resamples the screen radiance cache (8×8), which cuts reflection cost by 50–70%.
- 5.8 docs ([performance guide](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine)): "all pixels with roughness below 0.4 will trace a reflection ray". Above that, the free Lumen GI approximation is used.
  - `r.Lumen.Reflections.RadianceCache=1` is an experimental path that reuses diffuse rays. It helps only when many pixels lie at roughness 0.2–0.4.
- Epic forum cheat sheet ([link](https://forums.unrealengine.com/t/rough-lumen-reflections-cheat-sheet/875826), community): Lumen "struggles immensely" with glossy roughness ~0.2–0.39. **A natural place for Renderlab to beat it (I).**

**Translucency** (V):
- Front layer: depth-peeled into a minimal G-buffer, forced to mirror, denoiser off.
- Other layers: forward shading reads the world radiance cache ("can only produce glossy reflections").
- Clear coat: one reflection ray per pixel, so the bottom layer is treated as rough.
- Single-layer water is forced to mirror.

**Lag** (V): global lighting changes "can take multiple seconds to propagate" through the surface cache. Fixture F5 must test this.

### 5.0 to 5.7 changes (V unless noted)

- **5.0–5.3:** standalone RT Reflections deprecated. **5.4:** removed in favour of Lumen HWRT. Source: Epic and community [forum](https://forums.unrealengine.com/t/please-dont-remove-raytracing-reflection-deprecated-in-5-4/1646182) posts, not release notes.
  - Users report the old denoiser was sharper and allowed up to 8 rays per pixel, against Lumen's 1.
  - The 2022 slides confirm the UE 4.22 RTR model lacked specular occlusion: an unshadowed skylight showed blue inside interiors.
- **5.5** ([release notes](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-5-5-release-notes)):
  - "the new reflection denoiser that substantially improves the sharpness of low roughness reflections";
  - translucency with refraction in reflections;
  - hit lighting for GI;
  - Vulkan HWRT parity with DX12;
  - "colored refracted light support for Lumen reflection".
- **5.6** (via [Looman](https://tomlooman.com/unreal-engine-5-6-performance-highlights/)):
  - reflection radiance cache reworked;
  - new ray weighting;
  - 32-bit reflection output (−0.02 ms, console, 900p);
  - SER for hit lighting (`r.Lumen.HardwareRayTracing.ShaderExecutionReordering`);
  - water reflection downsampling and denoising;
  - SWRT detail traces deprecated.
- **5.7** ([release notes](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-5-7-release-notes?application_version=5.7), read from the page source):
  - "Screen tile marking optimizations, which speed up reflections, GI, and water reflections";
  - one shared G-buffer tile classification pass for Lumen and MegaLights;
  - `r.Lumen.TraceMeshSDFs 0` by default ("SWRT detail traces … deprecated … use HWRT");
  - `r.Lumen.ScreenProbeGather.MaxRayIntensity 10`;
  - High-scalability half-res integration (`StochasticInterpolation 2`, ~0.5 ms on console);
  - Substrate production-ready and enabled by default.
- **Substrate** (V, [overview](https://dev.epicgames.com/documentation/unreal-engine/overview-of-substrate-materials-in-unreal-engine)): the docs do not say how Lumen splits reflection rays across slabs or second-roughness lobes.
  - Stated: lighting cost is "twice as slow … for two BSDFs", and closures are simplified to fit `r.Substrate.MaxClosureCount` (80 B per pixel by default).
  - 5.5 fixed "missing translucent in Lumen's reflection with Substrate".
  - (I) Expect one traced lobe per pixel, as with clear coat. A second glossy lobe is a likely Unreal weakness.

### Fit

Renderlab traces full detail, so it needs no screen traces and no fallback-mesh bias. It must not adopt surface-cache lighting as the default, because that caches program output (§5).

## 2. SSR and why it cannot be core

**How it works.** March the reflected ray through the depth buffer. Hi-Z traversal (Uludag 2014, GPU Pro 5) steps through the min-depth mip chain to skip empty space.

Stochastic SSR (Stachowiak, Frostbite, [SIGGRAPH 2015](https://advances.realtimerendering.com/s2015/index.html), V from the speaker notes):
- Tile classification and adaptive ray allocation.
- GGX importance sampling with rays regenerated below the horizon.
- A precise Hi-Z tracer for smooth pixels and a cheap linear marcher for rough ones.
- A ray-reuse resolve that weighs neighbours' hits by the local BRDF divided by their pdf, with weight normalization ("worse than without reuse" until normalized).
- Sparse half-res tracing with a full-res resolve.
- A temporal filter with deliberately loose neighbourhood clamping, because "reflection color is from previous frame".

**Published cost (V).** PS4, 1600×900, every pixel reflective. Passes: tile classify 0.16, ray alloc 0.24, linear 0.20, Hi-Z 0.37, resolve 0.81, temporal 0.30 ms. Total **2.19 ms** for 1 ray per half-res pixel and 4 resolve samples. 2 rays: 3.34 ms. 4 rays and 1 resolve: 4.41 ms.

**FidelityFX SSSR** (MIT; SDK v1.1.4 manual calls it SSSR 1.5):
- SPD builds a 7-mip R32 Hi-Z.
- The classifier sends rough pixels (`roughnessThreshold`) to the environment map.
- `samplesPerQuad` sets 1–4 rays per quad, with variance-guided extra rays.
- 128² blue noise; denoised by the FidelityFX Reflections Denoiser.
- Misses fall back to the cubemap. AMD publishes no timings.

**Bevy SSR** (MIT/Apache, deferred only):
- Linear march plus bisection and secant refinement.
- A constant "thickness" parameter, because "depth buffer fragments are cuboids with a constant thickness".
- Edge-of-screen fade.
- The docs say: "When objects leave the camera, they will disappear from reflections."

**Failure modes, quantified where possible:**
- **Off-screen (I, geometry).** Take a horizontal mirror and a camera pitched θ below the horizon with vertical FOV φ. A reflected ray rises at the incidence angle, so it sees content above the horizon. That content is on screen only if it lies below the top frustum edge, which sits at pitch φ/2 − θ.
  - At θ = 30° and φ = 60°, the top edge is the horizon. No cliff top or sky above the camera's horizon is ever on screen, so a puddle at the camera's feet reflects nothing from SSR.
  - This is the common quarry viewpoint.
- **Back faces and hidden layers (V, Stachowiak notes).** "The depth buffer only contains the first visible surface, so we don't actually know the thickness". Tracing behind objects would need depth peeling.
  - The quarry's scans are open shells, so any surface seen from behind is absent from the depth buffer (I).
- **Thickness (V, Bevy docs).** A constant thickness is guessed per scene, so thin rails false-miss and thick walls false-hit. Mafia III lists "thin objects (rails, poles, signs) interrupt rays" for its cube tracer.
- **Disocclusion and lag (V).** The colour comes from the previous frame, so Lumen forces a double-strength bilateral filter on revealed pixels. SSR adds a one-frame lag even in steady state.
- **Production evidence (V):**
  - Battlefield V ran SSR first, then RT on rejection.
  - Metro Exodus EE uses RT "to fill these gaps where SSR fails" (background objects, off-screen). Digital Foundry reports leaves still come only from SSR.
  - PICA PICA dropped SSR "for simplicity and uniformity".

**Verdict.** **AVOID as core** (AGENTS.md). The only acceptable use is a **hint verified in world space** (I, not published):
1. An SSR hit on the visibility buffer gives a candidate triangle id and distance t.
2. Accept only if the exact ray-triangle test hits that triangle and an any-hit ray on [ε, t) finds nothing nearer.
3. Otherwise run full traversal.

The image then never depends on screen content, only the cost does. Flag it as a refinement and measure it only on the non-RT backend. N0 found software cost concentrated within 25 cm of the origin, so shortened rays may save little (M, N0; I for reflections).

## 3. Non-RT tracing representations

| Option | How (own words) | Cost (published) | Failure modes | Fit with Renderlab | Verdict |
|---|---|---|---|---|---|
| **Software BVH, full detail** (N0) | Same 16 B hit ABI and watertight test as hardware | N0 (M): camera rays 11.9×, shadow rays 21.9× hardware; BVH 412.5 MB; build 13–19 s | None beyond cost | The BVH alone exceeds the non-RT memory headroom of 408 MB against UE VSM (M, [N8o](../non-rt-n8o.md#memory-against-unreal-m)) | **Reject at full detail** (I) |
| **Cluster-LOD BVH sized by the ray cone** | Build a small BVH over a cut of the existing clusterlod DAG (214,062 clusters, [N1c](../non-rt-n1c.md)). Choose the cut per ray class so the simplification error stays below the reflection footprint at the hit (cone width × t, widened by roughness). Mirror near field uses finer clusters | none | LOD pops; near-field contacts (rock meeting puddle) need full detail; receiver ε is irrelevant at the far end | non-rt-path.md already plans per-cluster BVHs for larger-than-memory scenes; N1c already lifts LOD receivers | **Probe** (§9). Memory I: ~30–60 MB for a 1–2 M-triangle cut, scaling N0's 412.5 MB per 13 M |
| **Planar mirror view** | Rasterize the visibility buffer from the mirrored camera with a clip plane, then shade with the same programs. Exact for a plane | UE: "entire scene … rendered twice" (V). Renderlab N1c primaries 0.39 ms (M), so one plane costs ~0.4 + shading (I) | Planes only; one view per plane | Reuses N1c raster and programs directly; exactly world-space | **Keep** for water and mirror fixtures |
| **Mesh SDF / global SDF** (Lumen SW) | Sphere-trace per-mesh SDFs near, the global SDF far; light hits from the surface cache | PS5 reflection tracing, global SDF: 0.04 ms (Land of Nanite), 1.58 (Lyra), 1.76 (Matrix SWRT) (V) | Leaks; blurred thin features; mirrors poor ("HWRT the only way") | Needs a cooked SDF; mm relief lost | Glossy far field only |
| **Brixelizer** (v1.1.4) | Sparse 8³ SDF bricks per cascade, 3-level AABB tree. Specular: quarter-res pre-trace stores brick ids, then a full-res pass marches within the brick (V, GDC 2024) | GI total as in the summary | Radiance cache filled from screen space (V) | SDF and traversal only; never its cache | **Candidate** for rough-band visibility and glossy far field |
| **Relit probes** (GoT; Mafia III traced cubes) | GoT: per-probe albedo (BC1) and normal/depth (BC6H) cubes at 256², relit round-robin (V). Mafia III: SSR, then march 8 depth cubemaps, then a fallback (V) | GoT 700 µs per probe (Base PS4); Mafia 3.3 ms (PS4, 1080p) | Parallax only approximate; dynamic objects missing; placement | The prepared RLE2 field is already the distant-light "probe" | Reject as core; distant light is solved |
| SDFGI / VCT | Camera cascades / voxel octree with cone traces | as above | Lag, leaks, blur | — | Reject |

**Unreal SW Lumen sets a low quality bar for mirrors** (V: "HWRT is the only way…"). Non-RT can win on quality with exact or LOD-bounded geometry. Cost is the risk (§9).

## 4. Hardware-RT reflections

**Battlefield V** (DICE, GDC/GTC 2019, [slides](https://developer.download.nvidia.com/video/gputechconf/gtc/2019/presentation/s91023-it-just-works-ray-traced-reflections-in-battlefield-v.pdf), V):
- The naive version was slow and noisy, with rays wasted on low-reflectance surfaces.
- Fixes:
  - **variable-rate tracing** (rays per tile by maximum specular ratio; more on water and grazing angles);
  - **SSR hybridization**, with only rejected rays sent to DXR;
  - **ray binning** by direction;
  - **defrag** of hits before lighting;
  - **per-cell light lists**.
- Pipeline costs (ms): VRT 0.37, ray generation 0.19, binning 0.15, screen-space hybrid 0.36, intersect plus material data 1.98, defrag 0.08, lighting 0.46, spatial filter 1.45, temporal 0.24, image filter 1.00. **Total 6.29 ms.** GPU and resolution are not in the extracted slide text.
- Alpha: an any-hit shader with `IgnoreHit`. For transparency, a closest-hit loop took 0.96 ms against a weighted-OIT any-hit at 0.34 ms ("slightly different look").

**PICA PICA** (SEED; RTG ch. 25, [PDF](https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_25.pdf), V):
- Rays launch from the G-buffer at **half resolution, ¼ ray per pixel**.
- Halton sampling with Cranley–Patterson rotation; below-horizon rays regenerated; the NDF is sampled for a layered BRDF and pdfs are summed across layers.
- Spatial reconstruction reuses 16 half-res hits per full-res pixel, BRDF/pdf-weighted ("biased, but works well").
- A variance-guided bilateral filter, then TAA.
- Reprojection is discussed both by reflector motion and by the average hit point, since reflections have their own parallax.
- The performance table is an image; not extracted.

**Metro Exodus EE** (4A, [blog](https://www.4a-games.com.mt/4a-dna/in-depth-technical-dive-into-metro-exodus-pc-enhanced-edition), V):
- RTR only on highly glossy surfaces.
- Mirror-direction rays, no stochastic scatter.
- Hits use the low-detail material system (one flat average colour per object) with full PBR analytic lights.
- SSR is used first; RT fills the gaps.
- Glossy specular is otherwise reconstructed from the RTGI denoiser's SH. No cost is given.

**kajiya** (V, gi-overview):
- Half-res rays: 0.25 G-buffer rays + 0.25 shadow rays per pixel, plus 0.0625 each for validation.
- Hits are lit by the sun with a shadow ray and by the irradiance cache.
- Above a roughness threshold, diffuse rays are reused with pdf weighting.
- ReSTIR temporal only (M ≤ 8), with validation every frame at quarter resolution.
- An 8-sample full-res resolve, then a temporal filter.
- **2.2 ms on an RX 6800 XT at 1080p.**
- Limits: no recursion; exposes the irradiance cache's blockiness and flicker; bias can lose thin floor lines.

**Bevy Solari 0.20** (V):
- One unified ReSTIR PT kernel, reconnection shift only.
- Sharp lobes skip reservoirs.
- A "specular boost" lobe probability for smooth dielectrics: `mix(0.5, energy_probability, perceptual_roughness)`.
- Paths terminate in the world cache only when wide enough.
- Primary surface replacement (PSR) extended to dielectrics and slightly rough metals for DLSS-RR guides.
- **No timings.**
- Failure: smooth-specular correlations persist; only a hard roughness cutoff fixed them.

**Cyberpunk RT Overdrive** (NVIDIA [GDC 2023 blog](https://blogs.nvidia.com/blog/cyberpunk-2077-path-traced-visuals-gdc/), V for the feature list only): path-traced multi-bounce reflections. NRD and DLSS-RR are NVIDIA SDK. Use it only as a quality reference.

**Mirror/glossy split and resolution, collected (V):**
- Lumen traces below 0.4.
- UE High: DownsampleFactor 2 (one ray per quad). Epic and Cinematic: 1.
- SSSR: `roughnessThreshold` plus 1–4 rays per quad.
- PICA PICA: ¼ ray per pixel.
- kajiya: half-res.
- Mafia III: half-res, full-res resolve for low roughness.
- Metro: glossy-only.

**Hit-lighting cost (V).** Matrix on PS5 at 1080p: hit lighting **11.54 ms** against surface cache **2.44 ms**, from "dozens of 4K virtual textures per material", with little visible gain because of a skylight-dominated scene.
- (I) Renderlab's shading is gather-bound but measured: the full-screen standard program costs 0.396 ms (M). Incoherent hits fetch scattered texels, so expect 2–4× per hit.

**Any-hit for alpha (V).**
- Lumen avoids any-hit entirely (forced opaque plus a skip loop).
- NVIDIA calls forced any-hit "a significant performance penalty"; MegaLights saw ~10% from cull flags (prior survey).
- (I) The quarry is opaque. Keep reflection rays opaque until alpha fixtures exist.

## 5. Shading at off-screen hits (programmable shading)

**Contract (I, from [programmable-shading §2.4](../programmable-shading.md)).** A single-bounce reflection needs exactly L_o(hit, −d): the reflected surface's outgoing radiance toward the reflector. That is what `shade(s, input)` already returns when `input`'s view vector is −d and its footprint comes from the ray cone. Programs are compiled into compute modules and cannot use derivatives or screen access (M, naga rejects them).

So **reflections need no `scatter()` contract**:
- The reflected surface runs its own program. A toon object reflects as toon.
- At the reflector, the engine needs the program's specular lobe to sample rays. Proposal: an optional `fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe { normal, alpha, weight }`.
  - Programs without it get only what their own `shade()` builds from `environment_reflection(direction, roughness)`.
  - **Constraint to record:** `SpecularLobe` is GGX-shaped. A non-GGX style (anisotropic, toon-banded) gets the nearest GGX lobe or none.
- The engine returns a filtered incident quantity, `specular_incident(lobe)`. It is traced radiance integrated over that lobe and demodulated by the split-sum DFG term, as NRD and Stachowiak apply FG after filtering (V).
  - The program composes it, so the "engine accumulates incident quantities, never program output" rule holds.
- Multi-bounce needs `reflection_lobe` at the hit as well; rough GI bounces still need R4's `scatter()`.

**Lumen's surface-cache mode caches lighting-model output** (albedo × lighting per card). Adopting it would force programs to export an albedo/radiance proxy, which is exactly what programmable-shading forbids.
- Default to **hit evaluation** (Lumen's "hit lighting" equivalent), dispatched through the existing per-program tile worklists, with hits compacted by program (sorted-deferred).
- A cache is allowed only for incident quantities: sun visibility counts (N8o cells already are) and, later, irradiance from GI.

## 6. Denoising and temporal reuse

- **Lumen** (V): spatial BRDF reuse → temporal → variance- and disocclusion-driven bilateral → TSR.
  - 5.7 Epic: `r.Lumen.Reflections.ScreenSpaceReconstruction.NumSamples=5`, `MinWeight=0`. High: 3 and 1 (V, local BaseScalability.ini).
  - 5.5 added a sharper low-roughness denoiser.
- **NRD** (V README): ReBLUR (recurrent blur) and ReLAX (à-trous) take demodulated specular radiance plus normalized hit distance.
  - RTX 4080, 1440p: ReBLUR diffuse+specular 2.55 ms (3.40 SH); ReLAX 3.25 ms (4.80 SH).
  - VNDF sampling recommended; hit distance for the first bounce only; anti-firefly needs a confident background.
  - Licence: AVOID. ReBLUR's paper (Zhdan, RTG II) is reimplementable.
- **FidelityFX Reflections Denoiser** (MIT): reproject → prefilter → resolve-temporal, with motion vectors and tile lists from the Classifier. AMD publishes no cost.
- **Reprojection for reflections** (V, RTG 25): reflector motion vectors smear reflected content. Hit-point (virtual depth) reprojection follows the reflection's own parallax.

**Policy fit (I):**
- **Mirror band:** one ray per pixel centre is exact. No denoiser at all; AA comes from T1a jittered primaries, so motion behaves as primaries do.
- **Glossy band:** needs either enough rays (cost) or temporal reuse, and reuse is where Unreal's quality fails under motion. Gate it on the motion fixtures; never default it if it fails.
- World-space reuse (cache hit radiance by world cell) is view-dependent for glossy and program-dependent, so cache only incident quantities (§5).

## 7. ReSTIR for specular

- **ReSTIR GI** (Ouyang et al. 2021, NVIDIA): resamples secondary path vertices, 9.3–166× lower MSE at 1 spp (V, abstract).
- **GRIS / ReSTIR PT** (Lin et al. 2022): shift mappings (reconnection, random replay, hybrid) make glossy reuse correct. RTXDI 3.0 ships ReSTIR PT (V, search). Code is NVIDIA SDK: AVOID.
- **In practice (V):**
  - Solari keeps a high roughness gate and turned ReSTIR off by default.
  - kajiya uses temporal-only ReSTIR (M ≤ 8) with validation rays for reflections.
- **Patent:** US 11,315,310 plus the family noted in the prior surveys.
- **Verdict:** a hillclimb candidate for the glossy band after a patent review, not M7's first unit.

## 8. Specular occlusion, specular AA, ray cones

- **Specular occlusion** is implicit wherever reflection rays are traced. It is needed only in the untraced rough band, where Renderlab today multiplies the unoccluded prepared field. [m5-environment-runtime](../m5-environment-runtime.md): "Distant environment illumination is unoccluded."
  - That is the UE 4.22 RTR failure Lumen's team called out (V). On rough rock it brightens crevices (I).
  - **Published approximations:**
    - Frostbite 2014: specular occlusion from AO, n·v and roughness ([course notes](https://seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/), §4.10);
    - GTSO (Jimenez et al. 2016): a tabulated cone-cone overlap of a bent-normal visibility cone and the specular lobe ([TR](https://www.activision.com/cdn/research/PracticalRealtimeStrategiesTRfinal.pdf));
    - GoT: horizon occlusion of the lobe tilted by the normal map, ~3% of deferred lighting (V).
    - GTAO/GTSO inputs are screen-space, so they are barred as core.
  - **Proposal (I): a traced-lobe ratio estimator.** Take k ≥ 1 lobe-sampled visibility rays and compute `specular ≈ prefiltered_env(lobe) × Σ V_i·env_i / Σ env_i`, with occluded rays adding their hit radiance.
    - It is a ratio estimator in the style of Heitz et al. 2018, applied to the environment.
    - Noise stays low because the unoccluded part is analytic.
    - The rays are AO-like, so they can be shared with GI's rays when GI lands.
    - Non-RT: trace them against Brixelizer-style SDF bricks or the LOD BVH. A rough lobe tolerates cm error (I).
- **Specular AA** (V: Tokuyoshi–Kaplanyan [I3D 2019](https://www.jp.square-enix.com/tech/library/pdf/ImprovedGeometricSpecularAA.pdf), [JCGT 2021](http://jcgt.org/published/0010/02/02/); Filament `normalFiltering`, Apache-2.0):
  - Widen α² by normal variance from ray differentials, not `dFdx`.
  - On 4K-normal-mapped rock this moves sparkle pixels into wider lobes, so fewer pixels need traced rays and noise drops (I).
  - Must pass the film gate.
- **Ray cones** (V, [JCGT 2021](https://jcgt.org/published/0010/01/01/); Falcor `TexLODHelpers.slang` BSD-3):
  - Propagate the cone width w and spread γ along each ray, widening γ at each glossy bounce by the lobe.
  - Hit texture LOD and anisotropy come from w, the hit normal and the UV Jacobian. They feed `SurfaceInput`'s footprint, which programs already require.
  - RTG II: anisotropic cones come close to anisotropic ray differentials, but blurrier in places (V).

## Unreal 5.7 settings to match

Local install: `C:\Program Files\Epic Games\UE_5.7\Engine\Config\BaseScalability.ini`. Values were read; nothing was copied but setting names. The matched build is the packaged UE 5.7.4 CL51494982, D3D12 SM6 ([end-to-end comparison](../quarry-end-to-end-comparison.md)).

**`sg.ReflectionQuality` (V, local ini):**

| cvar | High (2) | Epic (3) | Cinematic |
|---|---|---|---|
| `r.Lumen.Reflections.Allow` | 1 | 1 | 1 |
| `r.Lumen.Reflections.DownsampleFactor` | **2** | 1 | 1 |
| `r.Lumen.Reflections.MaxRoughnessToTraceForFoliage` | 0.2 | 0.4 | 0.4 |
| `r.Lumen.ScreenProbeGather.MaxRoughnessToEvaluateRoughSpecularForFoliage` | 0.4 | 0.8 | 0.8 |
| `r.Lumen.Reflections.ScreenSpaceReconstruction.NumSamples` | 3 | 5 | 5 |
| `r.Lumen.Reflections.ScreenSpaceReconstruction.MinWeight` | 1 | 0 | 0 |
| `r.Lumen.TranslucencyReflections.FrontLayer.Allow` / `.Enable` | 0 / 0 | 1 / 0 | 1 / **1** |
| `r.SSR.Quality` (used only when Lumen reflections are off) | 2 | 3 | 4 |

Medium and Low set `r.Lumen.Reflections.Allow=0` (SSR only).

**`sg.GlobalIlluminationQuality`, Epic → Cinematic (V, local ini).** These matter for the >0.4 band, which comes from the screen probe gather:
- `ScreenProbeGather.DownsampleFactor` 16 → 8
- `TracingOctahedronResolution` 8 → 16
- `RadianceCache.NumProbesToTraceBudget` 100 → 350
- `NumAdaptiveProbes` 8 → 4
- `FullResolutionJitterWidth` 1 → 0.5
- Both: `TraceMeshSDFs.Allow=1` (the 5.7 default `r.Lumen.TraceMeshSDFs 0` overrides it per the release note), `ShortRangeAO.BentNormal=1`.

**Post-process and project settings, with the docs' defaults (V unless marked):**
- Reflection Method = Lumen. Lumen GI on as well: rough specular comes from GI, and standalone reflections need HWRT, which turns on hit lighting.
- Max Roughness To Trace: **0.4**, plus `r.Lumen.Reflections.MaxRoughnessToTraceClamp`.
- Max Reflection Bounces: 1. `r.Lumen.Reflections.MaxBounces` goes up to 64 with hit lighting.
- Max Refraction Bounces.
- Quality.
- Screen traces: PPV toggle or Show > Lumen > Screen Traces.
- Ray Lighting Mode = `r.Lumen.HardwareRayTracing.LightingMode`: 0 surface cache, 1 hit lighting for GI and reflections, 2 hit lighting for reflections (V via a secondary source; confirm at the console).
- HWRT: "Use Hardware Ray Tracing when available" (`r.Lumen.HardwareRayTracing`).
- High Quality Translucency Reflections.
- Community-cited, names to confirm at the console (I): `r.Lumen.Reflections.Temporal`, `.BilateralFilter`, `.ScreenTraces`.
- 5.7 forum: editor crash at startup with LightingMode ≠ 0 saved in config. Set it in the device profile or command line for the packaged build (V, forum).

**Configurations to measure, each at Epic and Cinematic:**
1. **UE-HW-Hit:** HWRT + hit lighting for reflections. UE's best quality; the RT-path denominator.
2. **UE-HW-SC:** HWRT + surface cache (UE default).
3. **UE-SW:** SWRT, global SDF (5.7 default without mesh SDFs). The non-RT denominator. Optionally with mesh SDFs re-enabled, as UE's best non-RT quality.
4. Optionally screen traces off, to isolate their contribution.

**Timing.**
- Existing harness: ProfileGPU at a fixed frame, CSV block of 64 frames.
- Read the **"Lumen Reflections"** node, plus "Lumen Screen Probe Gather" (rough band) and "Lumen Scene Lighting" (the cache reflections read). Pass names per the docs (V); exact tree labels to confirm in the dump.
- Use `r.Lumen.AsyncCompute 0` to isolate (V).
- Report the whole frame too: reflections cost includes scene lighting.
- Memory: the existing DXGI LocalUsedMB method, against UE's total **with Lumen on**. The VSM-only 3,912 MiB is no longer the denominator.

## Fit with Renderlab pieces

| Piece | Reflection reuse (I unless M) |
|---|---|
| **Traced primaries + 16 B hit record + deferred compute shading** | Reflection rays write the same 16 B record per reflection sample. The existing per-program tile worklists shade the hits. Origins come from the exact receiver with the adopted viewed-side ε policy. Primary costs (M): 0.23 ms traversal, 0.396 ms shading |
| **TLAS/BLAS over full detail** (912 MB AS) | Reflection rays use it directly; no fallback mesh. RT memory headroom against UE HRT's 5,410 MiB is 1.2–2.8 GB. The brief gives RT ~2.6 GB, while N8o's RT device reads 4,138–4,202 MiB reserved (M); settle which figure applies before budgeting |
| **Ray queries in compute** | Inline traversal suits a single closest-hit and keeps live state small. Lumen prefers inline for the same reason (V) |
| **Programs (`surface`/`shade`)** | Hit shading evaluates the reflected surface's own program (§5). Add `reflection_lobe` and `specular_incident` (constraint recorded) |
| **Prepared HDR environment** (RLE2: 65 roughness knots, 128² faces; 0.835% delta-sun error, M) | Miss shading for every band. Prefiltered lobe for the ratio estimator. RLE2 fixed the rough-chain resolution limit (M), so it is fit as the rough-band base |
| **Non-RT VSM with exact walks over 36 B walk records** (N8) | Sun visibility at reflection hits on the non-RT path, via an exact walk at the hit point. (I) Off-screen hits need their pages marked: a second marking pass from hit positions |
| **World-space surface shadow cache** (N8n/N8o: ~1 px cells, deterministic claim/resolve inserts, exact caster invalidation) | It holds visibility counts only, a compliant incident quantity. Reflection hits could read or insert sun visibility at a cell level chosen from the ray-cone width. Deterministic inserts keep reflections hash-stable. (I) Hits need their own consensus certification; off-screen cells start empty. It is not a radiance cache for program output |
| **Cluster LOD virtual geometry** (M4; N1c 0.35 px cut, 214k clusters) | Source for the cone-sized LOD BVH on non-RT. N1c's receiver lift shows how to keep LOD hits consistent |
| **N1c raster visibility buffer** (0.39 ms, M) | Planar mirror views on non-RT; exact |
| **Planned software BVH** (obvhs + tray_racing) | The builder and traversal for the LOD BVH. Full detail is rejected on cost (M: 12–22×) and memory (412.5 MB) |
| **T1a static accumulation** | Mirror-band AA and static convergence come free with primaries |
| **Film-reference infrastructure** (m6-film-reference) | Extend to glossy: high-spp accumulation of the same estimator, with Mitsuba 3 (BSD-3) or pbrt-v4 (Apache) as the independent oracle |

## Recommended direction for M7

### Principle

One reflection estimator, one hit contract and one set of references on both backends. The secondary-ray representation may differ on non-RT: N0 rules out full-detail software rays. Every difference is labelled and gated against the same references.

**Bands** (thresholds set by measurement against the reference, not copied from Unreal's 0.4):
- **B0 mirror** (perceptual roughness below ~0.05): 1 ray per pixel centre, deterministic, undenoised.
- **B1 glossy** (to ~0.4–0.5): k VNDF rays per pixel. Start with 1 at full res; half-res is a cost lever behind the gate.
- **B2 rough** (above the cutoff): RLE2 prefiltered lobe × the traced-lobe ratio estimator (§8).

### RT candidate (R-RT)

1. A trace kernel fused after primaries.
   - For B0/B1 pixels (from `reflection_lobe`), sample with VNDF (Heitz 2018; Dupuy–Benyoub 2023, not opened this session).
   - Opaque inline query, ray cone carried.
   - Write a 16 B hit record plus cone, pdf and weight.
2. Compact hits per program tile. Run `surface`/`shade` at hits with view = −d, sun visibility from one shadow ray (Interactive's estimator for B0 at near hits, I), environment at the hit, and no GI until GI lands.
3. **B0** composes directly. **B1** gets an engine-owned denoiser: BRDF-weighted spatial reuse with weight normalization (Stachowiak, V), hit-point reprojection and a variance clamp.
   - Port FidelityFX's reflection denoiser (MIT) as the baseline.
4. **B2** traces k = 1–2 visibility rays and applies the ratio estimator.

(I) Budget for a fixture with 30% glossy coverage at 1 ray per pixel:
- trace 0.2–0.35 ms;
- hit shading 0.25–0.5;
- shadow rays 0.05–0.1;
- denoise 0.2–0.5;
- **≈0.7–1.4 ms.**

Memory ≈110–130 MB: a full-res hit buffer of 33 MB, three RGBA16F targets of 50 MB, plus moments and lobe data.

### Non-RT candidate (R-NRT)

Tiered, every tier world-space:
1. **Planar mirror views** for flat B0/B1 surfaces: N1c raster + programs, exact.
2. A **cone-sized cluster-LOD software BVH** for other B0/B1 rays, sharing the hit contract and kernels. Sun visibility at hits from N8 exact walks, with hit-driven page marking.
3. **Brixelizer-style SDF bricks** (v1.1.4, MIT; SDF only) for B2 visibility rays and far glossy hits beyond the LOD BVH's trusted range.
4. The **RLE2 environment** on miss.
5. Optional, flagged: the **SSR hint verified by an exact any-hit on [ε, t)** (§2), measured only as a cost lever.

### First measurable unit: R0, Unreal baseline + RT glossy probe

**R0a (no Renderlab code; needs Islam's GPU window):**
- Build fixtures F1, F2 and F8 (below) in both engines through the existing comparison-setup pipeline.
- Capture UE-HW-Hit, UE-HW-SC and UE-SW at Epic and Cinematic: ProfileGPU nodes, images, memory.
- Also log a **roughness histogram of the plain quarry**. If its rock is mostly above 0.4, Unreal traces almost no reflection rays there (compare Land of Nanite's 0.23 ms, V). The cost contest is then on the fixtures, and the quarry's contest is B2 specular occlusion.

**R0b (test-only, Sol implements, Opus accepts):**
- R-RT steps 1–2 on F1/F2: B0 and B1 at 1 ray per pixel, no denoiser, static accumulation to convergence.
- Measure trace and hit-shading cost (paired in-process) and error against the glossy film reference.
- **Mirror-camera identity gate:** a roughness-0 planar mirror's reflection must equal the frame rendered from the mirrored camera, times Fresnel, within rounding, for the standard program and the toon program. This proves the hit contract and programmability in one check.

**R0c (CPU only, can run while Islam plays):**
- Cook the cone-sized LOD BVH for the quarry at two error bounds. Report bytes and build time.
- Run a CPU traversal of 10,000 F1 reflection rays against the full-detail N0 oracle, reporting hit differences against footprint.

### Stop rules (proposed; freeze before R0b)

- **Cost, RT:** R-RT B0+B1 on F2 above 0.5× UE-HW-Hit's "Lumen Reflections" (Epic, same fixture): profile the limiter, then hillclimb at most three mechanisms one at a time (program and direction binning, half-res B1 with a full-res resolve, cone texture LOD). Then record the miss. No quality reduction.
- **Cost, non-RT:** LOD-BVH rays above 3× hardware on the same rays after the hillclimb, or R-NRT above 0.5× UE-SW: do not productionize the BVH tier; price the planar and SDF tiers alone.
- **Quality:** converged static error against the reference worse than Unreal's on any band, measure or fixture: fix the estimator, never the tolerance.
- **Motion:** if B1's denoised error in motion (F3, F5) is worse than Unreal's, B1 stays opt-in and is labelled below the bar. B0 must pass without a denoiser.
- **Memory:** additions beyond (UE total with Lumen on − Renderlab total) on either path: compress first.
- **Programmability:** any mirror-camera identity failure is a contract bug and blocks everything else.

### Glossy test fixtures needed

The quarry has few mirror-like surfaces. All fixtures must be reproducible in UE through the existing setup.

| # | Fixture | Tests |
|---|---|---|
| F1 | Quarry floor puddle, roughness 0 / 0.05 / 0.1, camera pitched down 20–40° | Off-screen reflections of cliffs and sky (SSR's worst case); mirror-camera identity |
| F2 | Roughness sweep of dielectric and metal plates or spheres beside a rock wall: 0, .05, .1, .2, .3, .4, .5, .7, 1 | Band transitions; Unreal's 0.2–0.39 weakness and its 0.4 cutover |
| F3 | Moving occluder in front of F1, plus a camera strafe revealing the reflector | Disocclusion recovery frames, ghosting |
| F4 | Thin rails and poles above F1 | Thickness and thin-feature presence |
| F5 | Moving sun and a moving reflected object | Lighting lag in reflections (Lumen "multiple seconds") and hit motion |
| F6 | Grazing view over F1 | Fresnel, stretched highlights, cone LOD |
| F7 | Two facing glossy panels | Multi-bounce (MaxReflectionBounces) |
| F8 | Quarry with a wet-rock variant (roughness 0.3–0.5) | B2 specular occlusion in crevices against the reference |
| F9 | Reflection rays reaching scan shells from behind | Open-shell back faces; Lumen's missing-card black |
| F10 | Toon-program object reflected in F1 | Programmability at hits |

### Risks

- **Non-RT cost** is the main risk. Software rays measured 12–22× hardware (M). The LOD BVH's gain is unmeasured, and Unreal SW's reflection cost on this GPU is unknown until R0a.
- **B1 under motion.** Every published glossy path relies on screen-space reuse (Lumen, NRD, FidelityFX, PICA PICA). Passing the motion gates may need more rays and therefore more cost.
- **The `reflection_lobe` contract is GGX-shaped.** Record it as a programmability constraint, and keep programs without it valid.
- **Hit-shading cost.** Incoherent 4K texture gathers could dominate (Matrix: 11.54 ms against 2.44, V).
- **Patents.** The Adshir/Snap hybrid-reflection family covers raster primaries plus reflection rays. NVIDIA ReSTIR applies if B1 adopts it.
- **Reference cost.** Glossy film references need many samples per pixel on both paths. Use the Mitsuba or pbrt oracle for spot checks only.
- **Unreal 5.7 settings drift.** 5.8 is current (released 2026-06-17); the 5.8 docs mention a `HitLighting.Allowed` scalability cvar. Pin and record every value read back from the packaged 5.7.4 build.

## Sources

- Lumen, SIGGRAPH 2022 (Wright, Narkowicz, Kelly): https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf (V, text extracted)
- Lumen technical details: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine ; GI and reflections: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-global-illumination-and-reflections-in-unreal-engine ; performance guide: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine (V; pages labelled 5.8)
- UE release notes 5.5 / 5.7: https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-5-5-release-notes , https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-5-7-release-notes?application_version=5.7 (V)
- Looman 5.6 / 5.7 highlights: https://tomlooman.com/unreal-engine-5-6-performance-highlights/ , https://tomlooman.com/unreal-engine-5-7-performance-highlights/ (V, secondary)
- Planar reflections: https://dev.epicgames.com/documentation/en-us/unreal-engine/planar-reflections-in-unreal-engine (V); Substrate overview: https://dev.epicgames.com/documentation/unreal-engine/overview-of-substrate-materials-in-unreal-engine (V)
- Forum: RT reflections removed in 5.4: https://forums.unrealengine.com/t/please-dont-remove-raytracing-reflection-deprecated-in-5-4/1646182 ; rough reflections cheat sheet: https://forums.unrealengine.com/t/rough-lumen-reflections-cheat-sheet/875826 ; 5.7 release thread: https://forums.unrealengine.com/t/unreal-engine-5-7-released/2673913 (V, community)
- Local: `C:\Program Files\Epic Games\UE_5.7\Engine\Config\BaseScalability.ini` (V, values only)
- Stachowiak, Stochastic SSR, SIGGRAPH 2015: https://advances.realtimerendering.com/s2015/Stochastic%20Screen-Space%20Reflections.pptx (V, notes extracted); Uludag 2014, Hi-Z cone-traced reflections, GPU Pro 5 (cited by Lumen 2022)
- FidelityFX SSSR: https://github.com/GPUOpen-Effects/FidelityFX-SSSR , https://gpuopen.com/fidelityfx-sssr/ , https://gpuopen.com/manuals/fidelityfx_sdk/fidelityfx_sdk-page_techniques_stochastic-screen-space-reflections/ (V)
- FidelityFX Denoiser: https://github.com/GPUOpen-Effects/FidelityFX-Denoiser (V); SDK v1.1.4: https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v1.1.4 (V: LICENSE.txt, `samples/hybridreflections`, `docs/techniques`)
- Brixelizer GI, GDC 2024: https://gpuopen.com/download/GDC2024_GI_with_AMD_FidelityFX_Brixelizer.pdf (V)
- Bevy SSR: https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/ssr (V); Solari 0.20: https://jms55.github.io/posts/2026-09-18-solari-bevy-0-20/ (V)
- kajiya GI overview: https://github.com/EmbarkStudios/kajiya/blob/main/docs/gi-overview.md ; `assets/shaders/rtr/` (V)
- Battlefield V, GTC 2019: https://developer.download.nvidia.com/video/gputechconf/gtc/2019/presentation/s91023-it-just-works-ray-traced-reflections-in-battlefield-v.pdf (V)
- PICA PICA, RTG ch. 25: https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_25.pdf (V); SEED DD18: https://media.contentapi.ea.com/content/dam/ea/seed/presentations/dd18-seed-raytracing-in-hybrid-real-time-rendering.pdf (V)
- Metro Exodus EE: https://www.4a-games.com.mt/4a-dna/in-depth-technical-dive-into-metro-exodus-pc-enhanced-edition (V)
- Cyberpunk RT Overdrive: https://blogs.nvidia.com/blog/cyberpunk-2077-path-traced-visuals-gdc/ (V, feature list)
- Mafia III reflections, GDC 2018 (Sobek): https://media.gdcvault.com/gdc2018/presentations/Sobek_Martin_Real-time_Reflections_in_MafiaIII.pdf (V)
- Ghost of Tsushima, SIGGRAPH 2021 (Patry): https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html (V)
- Lagarde 2012, parallax-corrected cubemaps: https://seblagarde.files.wordpress.com/2012/08/parallax_corrected_cubemap-siggraph2012.pdf (cited by Mafia III; not opened)
- Godot SDFGI: https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html (V)
- Crassin et al. 2011, voxel cone tracing: https://research.nvidia.com/sites/default/files/publications/GIVoxels-pg2011-authors.pdf (V)
- NRD: https://github.com/NVIDIA-RTX/NRD (V: README costs, LICENSE)
- ReSTIR: Bitterli et al. 2020 https://research.nvidia.com/sites/default/files/pubs/2020-07_Spatiotemporal-reservoir-resampling/ReSTIR.pdf ; ReSTIR GI (Ouyang et al. 2021) https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.14378 ; GRIS / ReSTIR PT (Lin et al. 2022, TOG 41(4)) (V via abstracts)
- Ray cones: Akenine-Möller et al., JCGT 2021: https://jcgt.org/published/0010/01/01/ (V); RTG 2019 ch. 20 (not opened)
- Specular AA: Tokuyoshi and Kaplanyan, I3D 2019: https://www.jp.square-enix.com/tech/library/pdf/ImprovedGeometricSpecularAA.pdf ; JCGT 2021: http://jcgt.org/published/0010/02/02/ (V)
- Specular occlusion: Jimenez et al. 2016: https://www.activision.com/cdn/research/PracticalRealtimeStrategiesTRfinal.pdf ; Frostbite PBR 2014: https://seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/ (V, pages; formulas not re-read)
- VNDF sampling: Heitz, JCGT 2018; Dupuy and Benyoub 2023 (not opened this session)
- Patents: https://patents.justia.com/patent/11315310 ; https://patents.google.com/patent/US10950030B2/en ; https://patents.google.com/patent/US11302058 (V)

Temporary downloads went to `E:/renderlab-work/research-tmp/reflections/` and were deleted. Nothing was retained.
