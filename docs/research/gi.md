# Dynamic diffuse GI for M7: Lumen, probe and cache families, and a Renderlab design

Research date 2026-11-07, base `cf001a5`. **Web and source reading only:** no build, GPU run, window, Unreal launch or commit. Scope: dynamic diffuse global illumination for a likely M7, on both the hardware-RT and the non-RT path. Reflections are covered in a separate note (`reflections.md`, written in parallel); temporal reuse, denoisers and ReSTIR DI are in [reuse-temporal-denoise.md](reuse-temporal-denoise.md). This note extends, and does not repeat, §4 of [reuse-non-rt.md](reuse-non-rt.md) and the Lumen cost table in [industry-rendering-costs.md](industry-rendering-costs.md#epic-gi-is-neither-the-whole-frame-nor-a-constant-cost).

Labels:

- **V**: read at the cited URL in this pass. Published numbers are the authors' own and were not reproduced here.
- **M**: a Renderlab measurement, from the cited repo doc.
- **I**: inference or estimate by this note. Treat it as a hypothesis for a probe, never as evidence.

## Licence rule

The rule is in [open-source-reuse.md](open-source-reuse.md#licence-rule): COPY (with notice) for MIT/Apache/BSD/zlib/CC0; REIMPLEMENT-ONLY for papers, GPL, source-available and proprietary SDKs; AVOID for the listed SDKs and for anything that breaks a standing rule. Additions found in this pass:

- **NVIDIA RTXGI 2.x, SHaRC and NRC** (`NVIDIA-RTX/RTXGI`, `NVIDIA-RTX/SHARC`, `NVIDIA-RTX/NRC`) all ship `License.md` = **NVIDIA RTX SDKs LICENSE**, the same licence as RTXGI-DDGI. §1(b) lets you derive only from *sample* code; §4(e) forbids any use that makes the SDK subject to an open-source licence (V). **AVOID code** from all three. NRC is binary-only (`NRC_D3D12.dll`, `NRC_Vulkan.dll`, CUDA 13 runtime) (V).
- **The DDGI papers (JCGT 2019 and 2021)** are **CC BY-ND 3.0**, including their GLSL listings (V, paper footers). Reimplement from the text; do not paste the listings.
- **Split Radiance Cascades** (arXiv 2607.20384) is CC BY-SA 4.0 text with no code link (V). REIMPLEMENT-ONLY.
- **tiny-cuda-nn** is BSD-3 but CUDA-only (V), so it is unusable on wgpu (I).
- **kajiya** has both `LICENSE-MIT` and `LICENSE-APACHE` (V). Take MIT. The repository has been archived since 2025-07 (V, GitHub API).

## Bottom line

1. **No published dynamic-GI system meets all of Renderlab's rules at once.** Every production system either uses screen-space data in its core (Lumen screen traces, screen probes and short-range AO; GI-1.x screen probes; Brixelizer GI's screen-fed cache; GIBS's G-buffer-spawned surfels; Split RC's rays from on-screen surfaces) or is low-frequency and laggy (DDGI, SDFGI, VoxelGI) (V, per section). The parts worth taking are **world-space hash-grid caches** (Solari, kajiya ircache, GI-1.0 hash cells, SHaRC), **cache-terminated paths** (Solari, SHaRC), **DDGI's change-detection hysteresis** and **Lumen's split by distance range** (near field per pixel, far field in coarse world probes) (I).
2. **Renderlab already owns the hardest piece.** The N8o surface shadow cache has the same structure as these GI caches: world cells about 1 px in size, keyed by placement, viewed side and position, with deterministic claim/resolve inserts and exact caster invalidation (M, [non-rt-n8o.md](../non-rt-n8o.md)). A GI cache is a second payload table on the same keys (I).
3. **Outdoors, the largest missing term is probably occluded sky, not bounce light (I).** Renderlab's environment light is unoccluded today (M, [m5-environment-runtime.md](../m5-environment-runtime.md)). Directional sky visibility does not depend on lighting. A cache of it in world space has **zero lag** under sun, yaw and intensity changes, where Lumen's global lighting changes "can take multiple seconds" (V). That makes it the first measurable unit.
4. **Non-RT cannot afford full-detail software GI rays per pixel.** N0 measured software rays at 11.9–21.9x the hardware cost and a 412.5 MB BVH (M, [non-rt-n0.md](../non-rt-n0.md)), against roughly 0.4 GB of memory headroom below Unreal VSM (M). Non-RT GI needs amortization in the cache plus a cheaper far-field geometry: a coarse cluster-LOD BVH, or a Brixelizer SDF (MIT, v1.1.4) used for tracing only (I).
5. **The Unreal denominator is unmeasured on the quarry.** Epic's own budgets are 4 ms (High, 60 Hz) and 8 ms (Epic, 30 Hz) for GI, reflections and fog at 1080p on console (V). The first step is to measure Lumen HW/SW × Epic/Cinematic on the quarry. That number sets every cost stop rule below.
6. **Patents:** NVIDIA's **US 11,315,310** covers ReSTIR with a world-space GI data structure, which is the Solari and GI-1.x shape. **US 11,925,860** covers "projective hash maps". Both are V on Google Patents. Get a patent review before shipping ReSTIR-plus-cache designs. No Epic patent on Lumen was found (V for the search, which is no clearance).

## Summary

| Technique | RT path | Non-RT path | Published cost (hardware, resolution) | Memory | Motion / disocclusion / lag failure | Reusable code, licence (V) | Verdict |
|---|---|---|---|---|---|---|---|
| **Lumen SW** (UE 5.0–5.8) | n/a | Global SDF (+ mesh SDF ≤ 2 m, deprecated in 5.6+), surface cache, screen traces first | PS5, 1080p→4K TSR, Land of Nanite: High 2.77 ms, Epic 4.63 ms (V). Budgets 4/8 ms (V) | Not documented (V); atlas 4096² default (V) | Screen traces fail off-screen; probe-gather temporal filter ghosts behind movers; global light changes take seconds; walls < 10 cm leak; large single meshes get poor SDFs (V) | Unreal EULA: none | **Non-RT denominator.** Take surface-cache and distance-split ideas only |
| **Lumen HW** | TLAS over fallback or full Nanite; surface cache or hit lighting; far field 1 km (V) | n/a | Matrix, PS5, 1080p: reflection tracing 2.44 ms (surface cache) vs 11.54 ms (hit lighting) (V) | Not documented | As SW, without SDF limits; overlap-sensitive TLAS (V) | None | **RT denominator** |
| **DDGI** (Majercik 2019/2021) | HW probe rays | Any ray backend; Wicked runs one shader on both (V) | RTX 2080 Ti, 1080p, 32×8×32 probes × 64 rays: 2.6 ms of GI passes (V) | ≈ 14 MB for 8,192 probes (I, from formats V) | Low frequency; hysteresis lag (≈ 20-frame time constant at 0.95) (I from V); leaks below probe spacing (V) | Papers CC BY-ND; RTXGI-DDGI RTX SDK licence: AVOID. Wicked MIT: COPY | Far field / volume only |
| **Wicked Engine** DDGI, surfel GI, VXGI | `RTAPI` build | Compute-BVH build of the same file (V) | None published | Small | As DDGI / GIBS / VCT | MIT: COPY (HLSL reference) | Pattern source: one estimator, two backends |
| **SHaRC** (RTXGI 2.x) | Path-traced update at ~4% of pixels, world hash grid, path termination in cache (V) | Algorithm portable (I) | None found | 40 B/entry; 2²² suggested = 160 MiB (V) | Accumulation lag; a "responsive lighting" mode (V) | RTX SDK licence: AVOID | Reimplement the idea (hash grid is generic, I) |
| **NRC** | Neural cache, tensor-core, CUDA (V) | No | None found | n/a | Training lag (I) | RTX SDK, binary: AVOID | Out (no wgpu path) |
| **Bevy Solari** (0.17–0.20) | wgpu ray queries; ReSTIR DI/GI → unified ReSTIR PT + world cache; DLSS-RR (V) | None (V) | RTX 3080, 1600×900, 0.17: GI passes 0.8 ms (Cornell) to 5.4 ms (Bistro); frame 8.2–14.6 ms incl. 5.8–6.3 ms DLSS-RR (V) | World cache 2²⁰ × 84 B ≈ 84 MiB (V) | Moving lights leave GI trails; shadow lag reduced, not gone; leaks "much better" (V) | MIT/Apache: COPY | **Closest code** (wgpu, WGSL-like `.wesl`, world cache) |
| **kajiya** rtdgi + ircache | HW RT, half-res ReSTIR GI, 12 × 32³ clipmap cache (V) | No | RX 6800 XT, 1080p: indirect diffuse ~2.3 ms, ircache ~0.55 ms, frame 8.4 ms (V) | Not stated | Hit radiance reuses screen irradiance when visible (screen-dependent); validation every 3rd frame (V) | MIT (archived): COPY | Take ircache voting/ranking; flag the screen reuse |
| **AMD GI-1.0 / 1.x** (Capsaicin) | Screen probes 8×8 per tile + hash-cell world cache, DXR 1.1 (V) | Algorithm only (V: any ray caster) | 1080p, ½ spp(?): RX 6900 XT 1.93–3.12 ms; RTX 3080 1.91–3.43 ms; 9.0 M tris 3.43 ms (V) | Not extracted | Multi-bounce only via reflectors visible last frame (V) | MIT (v1.3, 2025-11-20): COPY | Take hash cells and leak fix; avoid screen probes as core |
| **Brixelizer GI** (FidelityFX 1.1.4) | n/a | Sparse SDF bricks; screen-fed radiance cache (V, prior) | RX 7600 XT ~1.7 ms at 1080p (V, prior) | ~200 MB at 1080p (V, prior) | Unseen changes stay absent; small detail lost (V, prior) | MIT (pin v1.1.4): COPY | SDF as far-field tracer only |
| **ReSTIR GI / PT / Area / PT Enhanced** | Screen-space reservoir reuse of path samples (V) | Software rays possible (I) | ReSTIR GI: 9.3–166x lower MSE at 1 spp (V); PT Enhanced 2.08–3.05x faster than 2022 ReSTIR PT (V, news) | Reservoir 48 B/px (Solari, V) | Screen reuse: disocclusion starves, correlations, temporal lag (V Solari) | Papers; RTXDI = RTX SDK | REIMPLEMENT-ONLY, **patent review**, not core |
| **EA GIBS** (Frostbite) | Surfels spawned from G-buffer 16×16 tiles, HW RT (V) | No | PS5, 1080p screen passes (4K output): PvZ ~7 ms total (V) | Surfel stack; not stated | Surfels exist only where seen (V); over-spawn on detail (V) | Proprietary: REIMPLEMENT-ONLY | Avoid as core (screen-spawned) |
| **Godot SDFGI / HDDAGI** | n/a | Camera cascades of SDF + probes (V) | GTX 1650: 2.43 → 0.93 ms (HDDAGI, V prior) | Not stated | No dynamic occluders; 25 frames to converge; wrong until camera leaves after static changes (V) | MIT: COPY | Avoid as core |
| **VoxelGI / VCT / VXGI** | n/a | Voxel cone tracing (V) | None at 1080p (V) | Not stated | Leaks; bake required in Godot; Epic rejected VCT as "leaky" (V) | Godot MIT; VXGI proprietary (I) | Avoid |
| **Radiance cascades / Split RC** | HW RT from on-screen surfaces; world hash probes (V) | Possible (I) | RTX 3080 Laptop: 8.6–11.5 ms single frame (V), resolution not extracted | Not stated | Probes populated from screen; multibounce needs an off-screen cache; leaks below base spacing (V) | Paper CC BY-SA: REIMPLEMENT-ONLY | Research watch |
| **Path-traced reference** | Renderlab HW rays, high spp (I) | Same tracer (I) | Offline | — | — | pbrt-v4 Apache, Mitsuba BSD (V, prior) | **Required gate** (R_GI) |

## 1. Unreal Lumen (UE 5.0–5.8)

Sources: SIGGRAPH 2022 Advances talk (Wright, Narkowicz, Kelly; PDF text extracted), the UE 5.7 Lumen docs, and Tom Looman's 5.6/5.7 release-note digests. Docs now render as "UE 5.8" for some pages (V). UE 5.8 shipped in June 2026 and adds "Lumen Lite (Beta)", at twice the speed of High (V, search snippet of the 5.8 notes). Renderlab's matched target stays **5.7.4 CL51494982** (M, [quarry-end-to-end-comparison.md](../quarry-end-to-end-comparison.md)).

### 1.1 Scene representation

- **Lumen Scene:** a simplified copy of the scene around the camera, used for secondary rays: 200 m by default, up to 800 m via *Lumen Scene View Distance* (V). Beyond that only screen traces give GI (V).
- **Software tracing:**
  - Per-mesh signed distance fields (sparse 8³ bricks, narrow band of ±4 voxels in 1 byte, streamed by distance) (V).
  - Merged into a **global SDF** of 4 camera clipmaps of 256³, time-sliced so that far clipmaps update less often (V).
  - With *Detail Tracing*, mesh SDFs serve the first 2 m and the global SDF the rest; *Global Tracing* uses the global field only (V).
  - 5.6 deprecated detail traces, and 5.7 defaults `r.Lumen.TraceMeshSDFs 0` (V, Looman).
  - SDFs are half-voxel expanded to stop thin-surface leaks, which trades leaks for over-occlusion (V, talk).
- **Surface cache:**
  - **Mesh cards** are axis-aligned projections, 12 per mesh by default (V). They are generated offline by voxelizing the mesh into surfels and clustering them; Epic cites ~0.2 ms for a 1.5 M-triangle mesh (V).
  - Cards are captured at runtime with Nanite into a virtual atlas (`r.LumenScene.SurfaceCache.AtlasSize` 4096) under a fixed capture budget of 512² texels per frame (V).
  - The cache stores albedo, normal, emissive, depth and opacity. Uncovered areas show pink and **lose energy rather than leak** (V).
- **Lighting the cache:**
  - Direct lighting updates 1024² texels per frame and indirect 512² texels per frame, chosen by a priority histogram (V).
  - Indirect light comes from 4×4 hemispherical probes per 4×4-texel tile, accumulated over 4 frames (V).
  - A 4-clipmap 64³ **voxel lighting** volume (6 directions per voxel) shades global-SDF hits. Epic calls it "too coarse" (V).
- **Hardware tracing:**
  - Traces a TLAS over Nanite **fallback meshes** by default; `r.RayTracing.Nanite.Mode 1` uses full detail and is described as offline (V, MegaLights doc).
  - *Far Field* extends traces to 1 km using HLOD1 (V).
  - `r.Lumen.HardwareRayTracing.LightingMode` (V, UE 5.4.4 cvar wiki):
    - 0: surface cache only (the default);
    - 1: material at the hit plus cached irradiance;
    - 2: "Hit Lighting", the material and direct light at the hit with indirect from the cache;
    - 3: material, direct light and unshadowed sky.
  - The project UI exposes this as *Ray Lighting Mode → Hit Lighting for Reflections* (V).

### 1.2 Final gather (opaque diffuse)

- **Distance split** (V):
  - **Contact AO**, a 16 px screen-space term, applied after the temporal filter so that it has no lag;
  - the **screen-space radiance cache** for roughly 2–16 m;
  - the **world radiance cache** out to 200 m;
  - the skylight.

  Latency tolerance grows with distance, and each range is cached differently.
- **Screen probes** (V):
  - Probes are placed on pixels at 1/16 resolution per axis, with extra adaptive probes on detailed geometry.
  - Each traces an 8×8 octahedral set.
  - Directions are **product-importance sampled** from last frame's reprojected screen cache, falling back to the world cache at screen edges.
  - Filtering happens in probe space: 3×3 probes ≈ a 48×48 px screen kernel.
  - A full-resolution temporal filter uses depth/normal rejection, not neighbourhood clamping. Epic calls the result very stable but slow to react, with **ghosting behind moving objects**, partly mitigated by faster accumulation where traces hit fast movers.
- **World radiance cache** (V):
  - Sparse clipmap probes at 32×32 directional resolution (16:1 against screen probes).
  - Screen-probe rays are shortened and continue from world probes on a miss.
  - Allocations persist across frames; a GPU priority queue re-traces a fixed number per frame, so cost is fixed and lighting latency is variable.
- **Translucency/fog** (V):
  - A froxel grid of 3×3 or 4×4 octahedral probes, culled by HZB.
  - Spatial and temporal filters, pre-integrated to SH2 irradiance.
  - Its own world cache at 16×16 directions.
  - About 1/8 of the opaque budget.
- **Reflections link** (V): roughness above 0.4 reuses the screen radiance cache (−50–70% reflection cost); the glossy range shortens rays into the world cache.
- **Ray rate** (V): Epic uses 4x the rays per pixel of High. With the 5.4.4 config, a uniform probe density gives 1/16 rpp (High: downsample 32, 8×8) and 1/4 rpp (Epic: 16, 8×8) before adaptive probes; Cinematic (8, 16×16) gives 4 rpp (I, arithmetic on V values).

### 1.3 Version changes that matter for matching

| Version | Change (V unless marked) |
|---|---|
| 5.4 (cvar wiki, version I) | `DownsampleFactor` High 32 / Epic 16 / Cine 8; `TracingOctahedronResolution` 8 / 8 / 16. Final Gather Quality multiplies the octahedron size by √q (clamped to 4–16), and q ≥ 6 halves the downsample (clamped to 4–64). Reference mode forces 32×32 |
| 5.6 | HWRT "matches the frame budget of the software ray tracing mode" (release notes via Looman). SWRT detail traces deprecated. `MaxRayIntensity` 40 → 10. New adaptive probe placement. Radiance cache reworked (with direct sky sampling). ShortRangeAO at half resolution with a denoiser, ~2x faster on console. Surface cache updates driven by distance to the frustum (~2x fewer pages). Optional `IntegrateDownsampleFactor`. Far field ~30% faster |
| 5.7 | `r.Lumen.ScreenProbeGather.StochasticInterpolation 2` at High (half-res integration, "~0.5ms on console at 1080p", softer normals, noisier GI). `r.Lumen.TraceMeshSDFs 0`. G-buffer tile classification shared with MegaLights. Known bug UE-361139: line artifacts with Lumen/MegaLights |
| 5.7 MegaLights (Beta) | Stochastic direct lighting with RT shadows. It starts with a short screen trace, then switches to HWRT/SWRT. Directional lights are off by default and "suit dim, soft sources". It needs RT platform support. Epic recommends pairing it with Lumen HWRT to share the RT scene. Not a GI technique; it replaces shadow maps and light evaluation for local lights |
| 5.8 | Lumen Lite (Beta), half the cost of High. Distance fields on integrated GPUs. SSGI superseded by Lumen GI (V, search snippets) |

Default tracing mode: `r.Lumen.HardwareRayTracing` defaults to 0 (5.4.4 wiki, V). The 5.8 technical-details page contradicts itself on whether HWRT is the default (V). Epic recommends HWRT on consoles at both 30 and 60 Hz (V, 5.7 perf guide). **Record the effective value in every UE run (I).**

### 1.4 Costs and budgets

- Epic's 5.7 guide gives "8ms and 4ms frame budgets at 1080p" for GI, reflections (opaque and translucent) and volumetric fog, at 30/60 Hz on console (V).
- Component tables (PS5, 1080p internal → 4K TSR) are already transcribed in [industry-rendering-costs.md](industry-rendering-costs.md#epic-gi-is-neither-the-whole-frame-nor-a-constant-cost): Land of Nanite SW High 2.77 / Epic 4.63; Lyra SW 4.29 / 8.70; Matrix HW+FF 6.38 / 11.31 ms (V).
- **Tracing method** (V, talk): Land of Nanite final-gather tracing is 0.94 ms with the global SDF against 10.03 ms with HWRT, because ~100 overlapping meshes cover every surface point. Lyra and Matrix are "nearly the same" between the two methods.
- **No 4070-class Lumen number was found.** A PS5 → RTX 4070 SUPER scale of about 2–2.5x (I, rough) would put the quarry's Lumen at ~1–2.5 ms at Epic. Measure it (G0 below).

### 1.5 Failure modes relevant to the quarry

- **Screen-space core:** screen traces run first, and ShortRangeAO and contact AO are screen-space (V). Under the standing rule these are the off-screen and disocclusion failure modes that Renderlab must beat, not copy.
- **SDF content rules** (V, tech details): "Large single meshes (mountains...) get poor SDFs and self-occlusion artifacts". Walls should be ≥ 10 cm. SDFs "cannot represent very thin features or one-sided meshes seen from behind". The 12 quarry scans are large, single, open photogrammetry shells (M, [scan-results.md](../scan-results.md)). Expect SW Lumen to over-occlude or leak on them (I). This is a likely quality win to measure, not to assume.
- **Lag:** global light changes "can take multiple seconds to propagate" (V). Fast camera motion makes Lumen Scene updates lag and pop (V). *Final Gather Lighting Update Speed* trades this against cost (V).
- **HW:** fallback-mesh mismatch is hidden by screen traces; overlap and instance cost (guideline < 100 k RT instances on console) (V).
- **Patents:** none found for Lumen (V for the search).

### 1.6 What Renderlab takes

- The **distance split**: the nearest range per pixel and at full rate, distant light in coarse persistent world probes at higher angular resolution (I).
- **Fixed-budget priority updates** of persistent probes, and **filtering in cache space** instead of screen space (I).
- **Not taken:** screen traces, screen probes, screen-space contact AO and screen-reprojected history. Renderlab traces the same full-detail geometry for every ray, so it has no representation mismatch to hide ([reuse-non-rt.md §4.1](reuse-non-rt.md#41-lumen-reimplement-only-selectively)).

## 2. DDGI and RTXGI

- **How it works** (V, JCGT 2019):
  - A grid of probes, each storing an octahedral irradiance map plus the mean and mean-squared hit distance (moments).
  - Each frame every probe traces N rays, and texels blend in with hysteresis α (0.95 typical).
  - Shading interpolates 8 probes, weighted by a Chebyshev visibility test on the moments, the backface direction and the trilinear weight. This removes most classic probe leaks.
  - Multi-bounce comes from feeding last frame's probes back into hit shading, which causes lag (V).
- **2021 production extensions** (V, JCGT 10(2)):
  - a self-shadow bias;
  - per-texel hysteresis drops on large changes (for example, −50% for 10 frames after "large object change");
  - perceptual exponential encoding for faster light-to-dark convergence;
  - probe relocation out of walls (5 optimizer iterations) and probe states that prune work;
  - cascaded volumes.

  Remaining flaw: "ghosting... for small, bright light sources" (V).
- **Cost** (V): RTX 2080 Ti, 1920×1080, Greek temple (876 k primitives), 32×8×32 probes × 64 rays, 8×8 RGB10A2 irradiance + 16×16 RG16F depth, all probes updated every frame:
  - ray generation 0.1, ray cast 0.8, ray shade 0.4, probe update 0.7, sampling 0.5, direct 0.1: **2.6 ms**;
  - throughput 1.5–1.65 Grays/s.
- **Memory (I):** 8,192 probes × (10² + 18²) texels × 4 B ≈ 14 MB.
- **Non-RT:** Wicked Engine compiles `ddgi_raytraceCS.hlsl` twice: `ddgi_raytraceCS_rtapi.hlsl` defines `RTAPI` and includes the same file, so inline ray queries are used when RT is available and a compute BVH (`TraceRay_Closest`/`TraceRay_Any`) otherwise. Wicked stores colour as SH, uses 16×16 depth with borders, and caps rays at 512 (V, `ShaderInterop_DDGI.h`). MIT. Wicked also ships `surfel_*CS.hlsl` (surfel GI, both builds) and `vxgi_*CS.hlsl` (VCT + SDF jump flood) (V).
- **Fit (I):**
  - Irradiance at probe spacing (decimetres to metres) cannot carry mm–cm scan relief.
  - Use it for translucency/fog and as the far-field fallback for cache misses, never for contact detail.
  - Its hysteresis heuristics transfer directly to per-cell blending.
- **Licence:** papers CC BY-ND (V). RTXGI-DDGI is under the RTX SDK licence (V, [open-source-reuse.md](open-source-reuse.md)). **Reimplement**, or copy Wicked's MIT version.

## 3. World-space hash-grid caches (the family Renderlab should build on)

All four systems below key a world-space cell by quantized position (+ normal, + LOD by distance) in a GPU hash table. Paths or probe rays query cells at their hits and terminate there, which amortizes multi-bounce transport across frames (V, per item). The common ancestor is Binder et al.'s path-space filtering by spatial hashing (cited by GI-1.0, V). NVIDIA authorship there means a patent check is needed (I).

| System | Key | Payload, size | Update | Lifetime | Leak control | Source (V) |
|---|---|---|---|---|---|---|
| **Bevy Solari** world cache | quantized position + quantized *geometric* normal, cell size from `log2(1 + distance / lod_scale)`, tangent-plane jitter of the query | radiance + geometry; **2²⁰ cells × 84 B ≈ 84 MiB**, held under wgpu's 128 MiB binding limit | each active cell traces a RIS direct-light sample + a cosine ray that queries the cache (multi-bounce); GI rays capped at 4 m (PR #21776) | 10 frames unqueried (`WORLD_CACHE_CELL_LIFETIME`) | geometric normal in the key; 0.20 lets paths continue past the cache when terminating would leak | `crates/bevy_solari/src/realtime/world_cache_{query,update,compact}.wesl`, `prepare.rs` |
| **kajiya ircache** | 12 camera-aligned sparse 32³ clipmaps | irradiance, plus a stored lowest bounce index | one query point per voxel chosen by **voting** among the frame's queries; trace from it | a few frames after last use | **rank** candidates by bounce index (camera-near queries win), offset lookup by normal; leaks still possible on tables | `assets/shaders/ircache/*.hlsl`, `docs/gi-overview.md` |
| **AMD GI-1.0** hash cells | position + normal (+ ray direction to fix thin-occluder leaks) | two-level map: tiles of mipmapped 8×8 cells, open addressing + fingerprint | screen-probe rays write hits; world-space ReSTIR on cells to fix blotches | decay → deallocate | direction in the descriptor | `src/core/src/render_techniques/gi1/hash_grid_cache.hlsl`, `world_space_restir.hlsl` |
| **SHaRC** | log-based LOD hash grid, `sceneScale` independent of resolution | 8 + 16 + 16 = **40 B/voxel** (64 B with SH); 2²² suggested | path tracer at ~4% of pixels (e.g. 1 per 5×5 block), resolve pass accumulates | `staleFrameNumMax` | continue tracing until the segment exceeds the voxel size | `docs/Integration.md` (RTX SDK licence: **read only**) |

- **Renderlab's N8o table** already has the hard parts (M, [non-rt-n8o.md](../non-rt-n8o.md)):
  - 2²⁰ buckets × 4 slots × 8 B (32 MiB) with a 32-bit fingerprint;
  - cells keyed by placement + viewed side + a world cube of a power-of-two multiple of the finest texel, at most one pixel of footprint;
  - **two-pass claim/resolve** inserts that are order-independent;
  - per-cell (not per-pixel) epochs;
  - flush-by-clear;
  - heaviest-first queues.

  Solari's single-pass CAS insert is not deterministic in the N8o sense (I: insertion order decides collisions).
- **Lesson carried from N8o (M):** reused cells beat VSM but lost to the pixel's own exact walk on 4 of 5 shadow measures, because a cell computed at one camera position was shown at another. GI is lower-frequency, but the same reuse error appears as over- or under-occlusion in crevices (I). Gate it.

## 4. Bevy Solari: the closest code

- **Status** (V):
  - 0.17 (2025-09) introduced ReSTIR DI + ReSTIR GI + the world cache.
  - 0.18 split the bounces: DI first, GI second, cache for the rest.
  - 0.19 added cache leak reduction and stochastic updates.
  - 0.20 (2026-09) is "a complete rewrite": **unified ReSTIR PT** after ReSTIR PT Enhanced, with paths of up to 3 bounces terminating in the world cache, and 2 dispatches instead of 5. **ReSTIR is off by default** (PR #25513: "~twice as fast on my RTX 3080" without it).
- **Files** (V, `crates/bevy_solari/src/`):
  - `realtime/`: `initial_path.wesl`, `restir.wesl`, `no_restir.wesl`, `presample_light_tiles.wesl`, the `world_cache_*` files, `node.rs`, `prepare.rs`;
  - `scene/`: `blas.rs`, `brdf.wesl`, `sampling.wesl`, `binder`;
  - `pathtracer/pathtracer.wesl`, a reference path tracer "for validation purposes".
- **Requirements** (V): `EXPERIMENTAL_RAY_QUERY` plus binding arrays and non-uniform indexing, the same wgpu feature family Renderlab uses. Custom materials were "blocked on wgpu raytracing pipelines" (V, 0.17 post). Renderlab avoids that block: it shades hit records in per-program compute dispatches, not RT pipelines (M, [programmable-shading.md](../programmable-shading.md)).
- **Cost** (V, 0.17, RTX 3080, 1600×900 → 3200×1800 DLSS-RR performance):

| Pass | PICA PICA | Bistro | Cornell |
|---|---:|---:|---:|
| World cache, all passes | 0.15 | 2.25 | 0.13 |
| ReSTIR GI initial + temporal | 0.37 | 2.75 | 0.33 |
| ReSTIR GI spatial + shade | 0.44 | 0.60 | 0.46 |
| **GI subtotal** | **0.96** | **5.60** | **0.92** |
| Whole frame (incl. DI 1.4–2.5, DLSS-RR 5.8–6.3) | 8.22 | 14.55 | 8.25 |

  0.18 cut the Bistro GI passes (V: 2.28 + 1.37 ms in one column). Cache sampling fell from 1.42 to 0.09 ms with 4 m rays and 64-thread workgroups (V, PR #21776 snippet). 0.20 publishes no tables (V).
- **Failures** (V): no validation rays, so moving lights trail; bright voxels amplified by ReSTIR flicker; cell position and normal are frozen at creation; the voxelized cache loses energy; DLSS-RR misreads ReSTIR correlations as detail.
- **Take:** the cache-terminated path loop, cell LOD by distance, the geometric-normal key and the 4 m cap (I). **Avoid:** ReSTIR's screen reuse as core (screen history plus patents), and DLSS-RR (NVIDIA-only, not wgpu-portable, I).

## 5. kajiya rtdgi + ircache (MIT, archived)

- **Pipeline** (V, `docs/gi-overview.md`):
  - Half-resolution diffuse rays from G-buffer points, with one-sample ReSTIR reservoirs (temporal resampling plus permutation sampling).
  - Every third frame validates old samples instead of tracing new ones.
  - The resolve does a near/far-field split. Micro-shadowing is recovered with a ≤ 6-tap **screen-space** march between the pixel and the neighbour's hit.
- **ircache:** 12 sparse 32³ clipmaps, queried only by rays, never by screen pixels. The query point is chosen by voting. Kajiya calls it "very quick to react to lighting changes" and "quite resistant to light leaks" (V).
- **Cost** (V): RX 6800 XT, 1920×1080, whole frame 8.4 ms: G-buffer ~1.15, indirect diffuse ~2.3, ircache ~0.55, reflections ~2.2, sun shadows ~0.52 ms.
- **Screen dependence (V):** hit radiance uses the screen-space irradiance "whenever the ray's hit point is visible from the primary camera". The near field is screen-marched. Both conflict with the standing rule as core (I).
- **Take:** voting and bounce-ranking, and validation-ray staggering for lag (I).

## 6. AMD GI-1.0 / Capsaicin; Brixelizer GI

- **GI-1.0** (Boissé et al., arXiv 2310.19855, V):
  - Screen probes: one 8×8 hemisphere per 8×8 tile at a random pixel, reprojected, with guided sampling.
  - A hash-cell world cache with mipmapped tiles, the ray direction in the descriptor to fix thin-occluder leaks, and world-space ReSTIR on cells.
  - A UE5 plugin comparison: Lumen SW 2.6 ms against GI-1.0 HW 2.4 ms in Archviz, with approximated materials.
- **GI-1.0 costs** (V, 1080p, DXR 1.1, AMD internal testing Sept 2022):

  | Scene | RX 6900 XT | RTX 3080 |
  |---|---:|---:|
  | Gas station (99 k triangles) | 1.93 ms | 1.91 ms |
  | Kitchen #2 (9.0 M triangles) | 3.12 ms | 3.43 ms |

  The ray-tracing share is 0.13–0.68 ms. Caching and sampling dominates.
- **GI-1.0 limitation** (V): multi-bounce "only really works when the contributing reflector is visible inside the previous frame". That is a screen dependency.
- **Capsaicin** (V): MIT, "Copyright © 2023 Advanced Micro Devices", v1.3 (2025-11-20). GI code is in `src/core/src/render_techniques/gi1/` (`screen_probes.hlsl`, `hash_grid_cache.hlsl`, `world_space_restir.hlsl`, `gi_denoiser.hlsl`, `glossy_reflections.hlsl`), with a `reference_path_tracer` alongside. HLSL/D3D12.
- **Brixelizer GI:** see [reuse-non-rt.md §4.2](reuse-non-rt.md#42-brixelizer-and-brixelizer-gi-copy-allowed-far-field-only). Pin v1.1.4 (MIT). Use its sparse SDF as a **non-RT far-field ray target only** and replace its screen-fed radiance cache with the world cache (I).

## 7. ReSTIR GI, ReSTIR PT, Area ReSTIR, PT Enhanced

- **ReSTIR GI** (Ouyang et al., HPG 2021, V): reservoirs per pixel hold a secondary-hit sample (position, normal, radiance). Temporal and spatial reuse in **screen space** with a Jacobian; 9.3–166x lower MSE at 1 spp.
- **ReSTIR PT** (Lin et al. 2022): generalized RIS with shift mappings for full paths. **PT Enhanced** (Lin, Kettunen, Wyman, 2026, doi 10.1145/3804494): reciprocal neighbour selection, footprint-based reconnection and duplication maps; unified DI/GI; 2.08–3.05x faster (V, news coverage). Solari 0.20 follows it (V).
- **Area ReSTIR:** see [reuse-temporal-denoise.md §1.3](reuse-temporal-denoise.md#13-area-restir-sub-pixel-point-reuse-with-a-22-window) (Falcor BSD-3, but the key file carries an NVIDIA proprietary header).
- **Patents** (V):
  - **US 11,315,310**: "Reservoir-based spatiotemporal importance resampling utilizing a global illumination data structure"; Wyman, McGuire, Shirley, Lefohn; priority 2020-03-12; active to ~2041. It describes ReSTIR gathering with an irradiance/radiance cache, probes or a photon map as the structure. That is exactly "world cache + ReSTIR" (Solari, GI-1.x).
  - **US 11,925,860**: "Projective hash maps"; Salvi, Pantaleoni, Lefohn, Wyman, Gautron; priority 2021-06-09. It hashes points projected to fewer dimensions. A 3D cell key is a different construction (I), but needs a reviewer.
  - Further Wyman/NVIDIA ReSTIR filings are listed in [reuse-temporal-denoise.md §3.1](reuse-temporal-denoise.md#31-restir-di-and-rtxdi).
- **Verdict:** REIMPLEMENT-ONLY after a patent review. Not core, because screen reuse starves at disocclusion and lags. If used, use it to pick *which* cells to update (world-space RIS), not as screen reservoirs (I).

## 8. EA GIBS (Frostbite, SIGGRAPH 2021)

- **Mechanism** (V, slides PDF text):
  - Surfels are spawned from the G-buffer: per 16×16 tile, the texel with the least coverage spawns one by randomized threshold.
  - Surfels persist and follow their surface, with radius kept roughly constant in screen space.
  - They are indexed in a camera-centred uniform grid plus trapezoidal outer grids, and allocated from a GPU stack.
  - Each surfel is ray-traced with ray guiding, adaptive ray budgets (new surfels get more) and an MSME integrator.
  - Transparents use SH probe clipmaps (4 levels, shifted with the camera, blue-noise level dithering).
- **Cost** (V): PS5, 4K output with 1080p screen passes, 8-sample reservoirs, unbounded rays, worst case from an empty surfel scene:
  - PvZ town centre ~7 ms;
  - a 1,400-light stress scene ~11 ms while converging;
  - filtering ~0.2 ms; probe sampling ~0.2 ms at 900p.
- **Failure modes** (V): coverage exists only where geometry has been seen; procedural geometry respawns surfels continuously; high-detail geometry over-spawns. EA considered combining it with SSGI.
- **Fit (I):** N8o-style cells keyed by surface position are "surfels without screen spawning" if cells are also created at ray hits. The off-screen half is what GIBS lacks.

## 9. Godot SDFGI/HDDAGI and VoxelGI; voxel cone tracing

- **SDFGI** (Godot 4.7 docs, V):
  - Camera-following SDF cascades plus probes.
  - **No dynamic occluders or dynamic emissives**; a "Dynamic" mesh acts as "Disabled".
  - 25 frames to converge by default.
  - After a static object or light changes, GI is wrong until the camera leaves and returns.
  - Cascade shifts are visible, and fast cameras cost more.
- **HDDAGI status** (V, GitHub API 2026-11-07):
  - #86267 (reduz) is still an open, locked draft.
  - #119869 (OrangeeZ, 2026-05-29) rebases it. It is open, with known DX12 occlusion and RADV bugs.
  - #122999 (2026-08, draft) adds Lumen-style 8×8 screen probes, **screen-space detail tracing**, SVGF and an irradiance cache.

  None is merged. MIT.
- **VoxelGI** (V): **baking required**; Dynamic meshes are "much slower"; leaks need sealed geometry or bias; at most 8 nodes visible, 2 blended per pixel; custom shader materials bake as black.
- **VCT/VXGI** (I): Crassin et al. 2011 cone tracing; NVIDIA VXGI is a proprietary GameWorks SDK (licence not rechecked). The Lumen team tried runtime voxelization and VCT and rejected it: "merging geometry properties into a volume causes lots of leaking" (V, talk).
- **Verdict:** AVOID as core. SDF cascade scrolling stays a far-field reference (I).

## 10. Probe families, radiance cascades

- **Irradiance volumes and light probes** (I): Greger 1998; McGuire 2017 light-field probes are DDGI's ancestor. Epic's critique (V, talk): irradiance is "computed at the probe, not the pixel", which causes leaking or over-occlusion and flat GI.
- **Radiance cascades:** see [reuse-non-rt.md §4.5](reuse-non-rt.md#45-radiance-cascades-gi-1x-gibs) for the 2D/screen sources.
- **Split Radiance Cascades** (Freeman, Sannikov, Pitt-Francis, arXiv 2607.20384 v2, 2026-10, V):
  - World-space RC probes in a sparse hash map, with **ray splitting**: rays traced from visible surfaces are deposited into cascades by hit distance.
  - Cost, single frame on an RTX 3080 Laptop: Sponza 8.6 ms, San Miguel 11.5 ms; the Fig. 1 frame is 13.2 ms, 6.1 ms of it 1 spp ray tracing.
  - It leaks below the base probe spacing.
  - Rays and probe initialization iterate over on-screen pixels; multi-bounce needs a separate "offscreen cache".

  Research watch: screen-fed, slower than the budget, no code (I).

## 11. References, SHaRC and NRC

- **Reference.** No GI reference exists in Renderlab yet; film references cover direct light (I, from [m6-film-reference.md](../m6-film-reference.md) scope).
  - M7 needs **R_GI**: a deterministic path trace with Renderlab's own hardware rays, the same geometry, programs, sun disk, environment and ε policy, at high spp.
  - Cross-check it on crops against pbrt-v4 (Apache-2.0) or Mitsuba 3 (BSD) (V, prior).
  - Use the UE path tracer only as a sanity image, not as the authority (I).
  - Bevy's `pathtracer.wesl` and Capsaicin's `reference_path_tracer` are COPY-licensed models (V).
- **SHaRC:** read its integration guide for parameters (cell occupancy 10–20% with a static camera; responsive mode) (V). Reimplement only.
- **NRC:** Turing+ NVIDIA, binary DLLs, CUDA. **Out** for a wgpu renderer that must run without RT (I).

## Unreal 5.7 settings to match

Base everything on the existing matched harness: packaged UE 5.7.4 Development, D3D12 SM6, the twelve scans, 1920×1080 internal and output, the registered sun, environment and exposure, CSV `GPUTime` frames 96–159, ProfileGPU at frame 64 (M, [quarry-end-to-end-comparison.md](../quarry-end-to-end-comparison.md)). Add GI as follows (I unless marked).

| Item | RT config ("HW Lumen") | Non-RT config ("SW Lumen") |
|---|---|---|
| GI method | Project *Dynamic Global Illumination Method* = Lumen (V) | same |
| Tracing | *Use Hardware Ray Tracing when available* on; `r.Lumen.HardwareRayTracing 1` (V names). `r.DistanceFields.SupportEvenIfHardwareRayTracingSupported=0` in `DefaultEngine.ini` to avoid dual SDF+BVH memory (V) | `r.Lumen.HardwareRayTracing 0`; *Generate Mesh Distance Fields* on (V); run both `r.Lumen.TraceMeshSDFs 0` (5.7 default) and `1` (deprecated, best SW detail) (V) |
| Hit lighting | `r.Lumen.HardwareRayTracing.LightingMode 0` (default) **and** `2` as a quality ceiling (V values) | n/a |
| RT geometry | Keep the matched HRT setup (13,034,903 native Nanite RT triangles, M); record `r.RayTracing.Nanite.Mode` and Fallback Relative Error | SDF resolution: record Distance Field Resolution Scale per scan and `r.DistanceFields.LogAtlasStats 1` output (V cvar) |
| Shadows | HRT, as matched (M) | VSM, as matched (M) |
| Quality levels | `sg.GlobalIlluminationQuality` and `sg.ReflectionQuality` = **3 (Epic)** and **Cinematic** (`@Cine` section; index 4 is I) | same |
| Effective cvars to log | Dump at startup: `r.Lumen.ScreenProbeGather.DownsampleFactor`, `.TracingOctahedronResolution`, `.StochasticInterpolation`, `.MaxRayIntensity`, `.RadianceCache.ProbeResolution`, `.RadianceCache.NumProbesToTraceBudget`, `r.LumenScene.SurfaceCache.AtlasSize`, `r.LumenScene.DirectLighting.UpdateFactor`, `r.LumenScene.Radiosity.UpdateFactor`, `r.Lumen.HardwareRayTracing*`, `r.Lumen.TraceMeshSDFs` (names V) | same |
| Post-process | Defaults for Epic. Record Final Gather Quality, Lumen Scene Detail, Scene Lighting Quality, View Distance and Max Trace Distance, Final Gather / Scene Lighting Update Speed (names V; defaults not documented, V) | same |
| Reflections | GI-only run: reflection method none or sky, to match Renderlab's unoccluded environment specular. Full Lumen run for `reflections.md` | same |
| Async | Pass attribution: `r.Lumen.AsyncCompute 0` / `r.RDG.AsyncCompute 0` (V). Frame claims: defaults | same |

**Timing and evidence:**

- **Pass costs:** ProfileGPU tree rows for Lumen Scene Update, Lumen Scene Lighting (direct and radiosity), Lumen Screen Probe Gather (traces, filter, integrate, temporal, ShortRangeAO) and translucency volume (V pass groups) with async off. **Frame cost:** CSV `GPUTime` with async on. Both feed the 2x ratio.
- **Workloads:** static, camera (1 cm alternation) and sun (1° alternation) as matched (M). Add a **sun step**: capture frames 1, 2, 4, 8 … 128 after a 10° step, and record frames-to-settle against the settled image. Do the same for an environment yaw step. This measures Lumen's lag, the expected Renderlab win.
- **Memory:** `GPUMem/LocalUsedMB` with GI on, per config, using the N8h method. GI raises Unreal's totals, which moves Renderlab's budget (I).
- **Floor arithmetic (I):** let L be UE's Lumen cost.
  - RT: Renderlab's GI budget is (1.75 + L)/2 − 0.86 ≈ **L/2** (M for 1.75 and 0.86).
  - Non-RT static: (1.61 + L_sw)/2 − 1.30 ≈ L_sw/2 − 0.5 with Exact, or ≈ L_sw/2 + 0.08 with Surface's 0.73 ms static frame (M).
  - Non-RT camera-moving already misses 2x without GI (M), so record the miss and do not widen it.
- **Which UE level is the floor (I, decision for Islam):** claim the floor against **Epic** (shipping game quality), only when Renderlab beats Epic on every quality measure. Record Cinematic and hit lighting as the quality ceiling to work toward.

## Fit with Renderlab pieces

| Piece | How GI reuses it (I unless marked) |
|---|---|
| **(1) Cluster LOD** (M4, CPU cut, prepared assets) | Far-field GI geometry for non-RT. Build a software BVH over a coarse fixed cut (cm error) and trace rays beyond a near-field distance d₀ against it. This is the ray-cone rule that the shadow/primary LOD rejection ([non-rt-path.md](../non-rt-path.md#decisions), 7.3 mm vs 10 µm ε) does not cover: a GI ray's footprint at metres tolerates cm error. On RT, LOD BLASes could serve far field later; the 912 MB full-detail AS is already resident (M) |
| **(2) HW RT path** (TLAS/BLAS full detail, ray queries in compute, 16 B hit record, deferred compute shading) | GI rays are inline ray queries that write the same hit record. Hits are binned by program into per-program compute dispatches, the mechanism primaries use (M, [programmable-shading.md](../programmable-shading.md)). This sidesteps wgpu's missing RT pipelines, which blocked Solari's custom materials (V). Texture LOD at hits comes from ray cones. Sun visibility at hits costs one extra hardware shadow ray |
| **(3) Non-RT VSM + exact walks** (36 B walk records, Exact/Map/Hybrid/Surface) | Sun visibility at **off-screen GI hits**. Pages exist only where marked (M), so GI hit positions must also mark pages, the way MegaLights-driven VSM marking works in 5.7 (V). Bounce light is low-frequency, so a coarser page level at hits may pass (risk: gate it). The walk machinery is sun-specific. **Do not** extend it to "many suns" for sky visibility: per-direction records cost ~1.25 GB-class memory (M, N8g scale) |
| **(4) N8o surface cache** (≈ 1 px world cells, placement/side/position keys, claim/resolve, exact caster invalidation) | **The GI cache.** Same keys and coarser parent levels, with a separate payload table: directional sky visibility (L1 SH, 4 × f16 = 8 B) and incident irradiance (L1 SH RGB, 12 × f16 = 24 B) plus count and epochs (8 B). That is ≈ 40 B per cell, so 2²² cells ≈ 160 MiB or 2²¹ ≈ 80 MiB. Epochs: caster (exact invalidation as N8o), sun (radiance only; visibility unaffected) and **program** (flush cached hit radiance on program reload or parameter change). Determinism and hash-equality gates carry over |
| **(5) Prepared HDR environment** (owned f32 cubes, prefiltered, yaw/intensity) | The first GI term is **occluded sky**: per-cell directional visibility × environment radiance projected to SH. Yaw is an SH rotation and intensity a scale, so neither invalidates the cache. GI ray misses sample the prefiltered cube at a ray-cone mip |
| **(6) Software BVH** (N0: 412.5 MB, 11.9–21.9x hardware; obvhs/tray_racing plan) | The non-RT ray backend for near-field cell updates at small ray fractions. Compress to CWBVH (80 B nodes, [reuse-non-rt.md](reuse-non-rt.md)) before adding it to the non-RT memory total: uncompressed it eats the whole ~0.4 GB headroom below Unreal VSM (M) |
| **Programmable shading** (R4 open) | GI needs each program's response at off-screen hits: `surface()` at the hit plus an optional `scatter(s, input, wi, wo)` and emission ([programmable-shading.md R4](../programmable-shading.md#7-open-risks)). Caches store **incident** quantities (visibility, irradiance) per receiver, so every style consumes `indirect_diffuse` its own way. Cached outgoing radiance at hits is program output: key it by program epoch. A program without `scatter` is treated as black/opaque in GI and listed as a limitation |

## Recommended direction for M7

### Candidate designs

**RT path, "R-GI": a world-cell cache fed by hardware rays.**

1. **Receivers** are the exact primary hit points (16 B records).
2. **Pixels read cells.** Each pixel reads its cell, plus the parent level on a miss, with a sub-cell jitter. The N8o "consensus" lesson applies: GI is smoother, but crevice cells must not be shared across depth or normal breaks.
3. **Updates** go to a fixed per-frame ray budget, about 0.25 rpp to start (I). Priority: new cells first, then aged cells, then cells whose epoch changed (heaviest-first queues from N8m/N8o).
4. **Hits** are shaded by the hit's program (deferred, binned by program) with sun visibility from one hardware shadow ray and multi-bounce read from the hit's own cell (cache termination, as in Solari and SHaRC).
5. **Sky visibility** is accumulated separately and is lighting-invariant.
6. **Temporal behaviour:** per-cell blending with DDGI-2021-style change detection and validation re-traces (kajiya). No screen reprojection.
7. **Spatial filtering** happens in cell space (Lumen's probe-space filter idea).
8. **Estimate (I):**
   - one incoherent hardware ray ≈ 0.25–0.5 ms per rpp at 1080p on the quarry (from tray_racing's 0.51 ms for 1 primary + 1 AO ray on a 4070 Ti, V prior, and Renderlab's 0.23 ms traced primary, M);
   - hit shading ≈ 0.4–1.2 ms per rpp;
   - at 0.25 rpp, about **0.25–0.5 ms** plus 0.05–0.1 ms of cache passes, against an estimated L/2 ≈ 0.5–1.2 ms.

**Non-RT path, "N-GI": the same cache, estimator and keys, with a cheaper ray backend.**

1. **Near field (≤ d₀, start at 0.25 m):** software rays against compressed full-detail CWBVH at about 1/16 rpp per frame. Accumulation into cells converges a static view over frames. Estimate ≈ 0.07–0.15 ms (I, scaled from N0's 9.43 ms for 8 rpp, M).
2. **Far field (> d₀):** software rays against a coarse cluster-LOD BVH, or a Brixelizer SDF (MIT v1.1.4, tracing only), at the RT path's ray budget (I).
3. **Fresh cells** are seeded from parent levels and the far field, so disocclusion is never black. Measure the residual lag explicitly.
4. **Optional, for Islam:** prepared static sky visibility as a cook-time warm start, invalidated at runtime by caster epochs. Per-vertex L1 visibility ≈ 6.5 M vertices × 8 B ≈ 52 MB (I). Valid only for static placements; content-specific.
5. **Do not take** DDGI as the main term (low frequency), SDFGI/VoxelGI (static or leaky), or Lumen/GI-1.x/GIBS screen components (standing rule).

Both designs keep the non-RT plan's contract: the same estimator, cells and direction sequences on both backends, with the gates run on both ([non-rt-path.md](../non-rt-path.md#decisions)). The deliberate difference is non-RT's far-field geometry, so far-field error is gated separately.

### Units

**G0: denominators and reference (no Renderlab rendering code).**

- Unreal: HW/SW × Epic/Cinematic, plus hit lighting and detail traces, on static, camera, sun, sun-step and yaw-step workloads. Record costs, memory and captures as in "Unreal 5.7 settings to match".
- **R_GI:** Renderlab hardware path traces at poses 0/1 on the quarry and S1–S3. Two terms: environment only (visibility, no bounce) and full (sun + environment, ≥ 3 bounces). Converged by spp doubling. Pbrt or Mitsuba cross-check on two crops.
- Output: L_hw and L_sw per level, UE settle curves, UE GI memory, and UE's error against R_GI on every measure. This replaces the estimated budgets below.

**G1 (first Renderlab unit): occluded sky through world cells, on both backends.**

- **Files (proposed):** `src/gi_cache.rs` (table, epochs, passes) and `shaders/gi_cache.wgsl`. They reuse `virtual_shadow_cache` key, claim and resolve code by extraction, not copy.
- **Scope:** the environment diffuse term only, so that the sun bounce and R4 stay out of G1. Programs read `indirect_diffuse(n)`.
- **Gates:**
  - static converged error against R_GI(env) on RMS, high-occlusion (crevice) pixels, over-occlusion of open pixels, and flicker in motion;
  - against UE Lumen Epic (HW for RT, SW for non-RT) with the sun's indirect disabled (I: light Indirect Lighting Intensity 0);
  - determinism hash-equality across processes;
  - caster-move settle byte-identical to fresh;
  - yaw/intensity step: **0 frames lag**, the next frame equal to a fresh run.

### Stop rules (frozen before code)

- An unexplained difference between two visits or processes: fix the implementation, never the comparison (N8o standard).
- After at most **3 retained hillclimb mechanisms**, G1 still loses to UE Epic on any quality measure, static or moving: stop and report the limiter (reuse error, ray budget or cell size).
- **RT cost:** camera-moving G1 above **0.3 × L_hw** (sky's share; recompute after G0), or full R-GI above L_hw/2: stop, or record the 2x miss with the measured limiter. Never cut quality to pass.
- **Non-RT cost:** above L_sw/2 means proceed only as a "quality-first non-RT mode", recording the miss, as with N0's rule.
- **Memory:** a GI table above 160 MiB, or a non-RT total at or above UE SW Lumen's measured total: compress before merging (memory rule).
- **Lag:** a sun or yaw step on the sky term must equal fresh on the next frame. Disoccluded cells must have first-frame error at or below UE's first-frame error at the same disocclusion.

**G2:** sun one-bounce plus cache-terminated multi-bounce on both paths. It starts only after Islam settles R4 (the `scatter` contract).

### Risks

1. **R4 is undecided.** Without a secondary-response contract, user programs are invisible to GI or forced into one model (M, [programmable-shading.md](../programmable-shading.md)).
2. **Cell reuse error.** N8o's Surface lost to Exact on 4 of 5 measures for this reason (M). Crevices in scans are where GI error concentrates (I).
3. **Software ray cost** is 11.9–21.9x hardware (M). The non-RT near field may not converge fast enough under motion, which would give a quality-first-only result.
4. **Memory.** The N0 BVH (412.5 MB) plus caches against ~0.4 GB of headroom (M), until UE's Lumen totals are measured.
5. **Hit-point sun visibility on non-RT** requires page marking at off-screen hits. That is new VSM work, possibly costly (I).
6. **Fairness disputes.** UE SW Lumen is likely weak on single open scan shells (V content rules). Report all four UE configs, and claim against the best-quality config Renderlab matches or beats (I).
7. **Patents:** US 11,315,310 (world-space structure + ReSTIR) and US 11,925,860 (projective hash maps). Hash-grid path-space filtering has NVIDIA authorship (I). Review before shipping any ReSTIR-plus-cache combination.
8. **Motion labelling.** Primaries remain "Interactive 1x in motion, below the film bar" (M). GI under motion inherits that label until a motion method passes.

## Sources

All accessed 2026-11-07. PDFs were text-extracted under `E:/renderlab-work/research-tmp/gi/` and deleted afterwards.

- Lumen talk, SIGGRAPH 2022 Advances: https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
- Lumen docs (5.7/5.8): https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine ; https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine?application_version=5.7 ; https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-global-illumination-and-reflections-in-unreal-engine?application_version=5.7 ; https://dev.epicgames.com/documentation/en-us/unreal-engine/megalights-in-unreal-engine?application_version=5.7 ; https://dev.epicgames.com/documentation/en-us/unreal-engine/scalability-reference-for-unreal-engine?application_version=5.7
- UE cvar defaults (5.4.4 wiki): https://indxzero.github.io/ue544cvarwiki/articles/r.lumen.screenprobegather.downsamplefactor/ ; …/r.lumen.screenprobegather.tracingoctahedronresolution/ ; …/r.lumen.hardwareraytracing.lightingmode/ ; …/r.lumen.hardwareraytracing/ ; …/r.lumenscene.surfacecache.atlassize/
- UE release digests: https://tomlooman.com/unreal-engine-5-6-performance-highlights/ ; https://tomlooman.com/unreal-engine-5-7-performance-highlights/ ; https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-8-release-notes (via search snippet)
- DDGI: https://jcgt.org/published/0008/02/01/ (paper-lowres.pdf) ; https://jcgt.org/published/0010/02/01/ (paper-lowres.pdf)
- RTXGI 2.x / SHaRC / NRC: https://github.com/NVIDIA-RTX/RTXGI (`License.md`, `Readme.md`, `Docs/NrcGuide.md`) ; https://github.com/NVIDIA-RTX/SHARC (`License.md`, `docs/Integration.md`) ; https://github.com/NVIDIA-RTX/NRC (`License.md`, `Readme.md`) ; https://github.com/NVIDIAGameWorks/RTXGI-DDGI
- tiny-cuda-nn: https://github.com/NVlabs/tiny-cuda-nn (`LICENSE.txt`)
- Bevy Solari: https://github.com/bevyengine/bevy/tree/main/crates/bevy_solari (`src/lib.rs`, `realtime/mod.rs`, `realtime/prepare.rs`, `realtime/world_cache_query.wesl`) ; https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17/ ; https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18/ ; https://jms55.github.io/posts/2026-04-12-solari-bevy-0-19/ ; https://jms55.github.io/posts/2026-09-18-solari-bevy-0-20/ ; https://github.com/bevyengine/bevy/pull/21776 ; https://github.com/bevyengine/bevy/pull/25513
- kajiya: https://github.com/EmbarkStudios/kajiya (`LICENSE-MIT`, `LICENSE-APACHE`, `docs/gi-overview.md`, `assets/shaders/ircache`, `assets/shaders/rtdgi`)
- GI-1.0: https://arxiv.org/abs/2310.19855 ; Capsaicin: https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin (`LICENSE`, `src/core/src/render_techniques/gi1`, releases)
- Brixelizer: see [reuse-non-rt.md](reuse-non-rt.md) source ledger (FidelityFX SDK v1.1.4, GDC 2024 deck)
- ReSTIR GI: https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing ; ReSTIR PT Enhanced: https://dl.acm.org/doi/10.1145/3804494 ; https://videocardz.com/newz/nvidia-says-new-restir-path-tracing-update-is-2-to-3-times-faster-but-this-is-still-research
- Patents: https://patents.google.com/patent/US11315310B2/en ; https://patents.google.com/patent/US11925860B2/en ; http://cwyman.org/papers.html
- GIBS: https://advances.realtimerendering.com/s2021/SIGGRAPH%20Advances%202021%20-%20Surfel%20GI.pdf ; https://www.ea.com/seed/news/siggraph21-global-illumination-surfels
- Godot: https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html ; https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_voxel_gi.html ; https://github.com/godotengine/godot/pull/86267 ; https://github.com/godotengine/godot/pull/119869 ; https://github.com/godotengine/godot/pull/122999
- Wicked Engine: https://github.com/turanszkij/WickedEngine (`LICENSE.txt`, `WickedEngine/shaders/ddgi_raytraceCS.hlsl`, `ddgi_raytraceCS_rtapi.hlsl`, `ShaderInterop_DDGI.h`, `surfel_*`, `vxgi_*`, `Content/Documentation/WickedEngine-Documentation.md`)
- Split Radiance Cascades: https://arxiv.org/abs/2607.20384 (v2 PDF)
- Local: [non-rt-path.md](../non-rt-path.md), [non-rt-path-2.md](../non-rt-path-2.md), [non-rt-n0.md](../non-rt-n0.md), [non-rt-n8n.md](../non-rt-n8n.md), [non-rt-n8o.md](../non-rt-n8o.md), [non-rt-n8k.md](../non-rt-n8k.md), [m5-environment-runtime.md](../m5-environment-runtime.md), [quarry-end-to-end-comparison.md](../quarry-end-to-end-comparison.md), [unreal-comparison-setup.md](../unreal-comparison-setup.md), [programmable-shading.md](../programmable-shading.md), [open-source-reuse.md](open-source-reuse.md), [reuse-non-rt.md](reuse-non-rt.md), [reuse-temporal-denoise.md](reuse-temporal-denoise.md), [industry-rendering-costs.md](industry-rendering-costs.md)
