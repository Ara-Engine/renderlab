# Reusable temporal reconstruction, denoising and sampling work for motion AA

Research date: 2026-11-06. Read-only: public web sources, GitHub license files read via the API or raw files, and the local docs [m6-motion-aa.md](../m6-motion-aa.md), [m6-motion-aa-gpu.md](../m6-motion-aa-gpu.md), [m6-temporal-primaries.md](../m6-temporal-primaries.md) and [m6-film-gap.md](../m6-film-gap.md). There were no builds, GPU runs or commits. This is not legal advice. Licence verdicts read the LICENSE file in each repository on the research date. Patent notes list only what was found and do not analyse claims.

**Extended 2026-11-07 for M7.** See [2026-11-07 M7 extension](#2026-11-07-m7-extension) (§6–§11): TSR-class reconstruction as a shipping mode, denoisers for GI and reflections, lighting-change response, Unreal 5.7 settings, and both RT paths. Verdicts that change carry a dated note in the summary table.

**Labels.** **V** means verified at the cited URL or file. **Inference** (**I**) is my reasoning, not a published result. **M** marks a Renderlab measurement from a local doc. Author-reported numbers keep the authors' hardware and resolution. They are not Renderlab measurements.

**Target recap (from the local docs).** The film gap is 1x point-sample aliasing over the whole image. Interactive costs 0.82–0.87 ms, and the stop is 1.075 ms. T1 (a resampled radiance history) was rejected for structural leaks across shadow edges. T2 (world-anchored sample records) gives the best moving image. In motion it costs 3.55 ms at K = 4 and 5.76 ms at K = 8. The measured attribution, from m6-motion-aa-gpu §6, splits that cost as follows:

| Part | K = 4 | K = 8 |
|---|---:|---:|
| Gap gather | 1.3 ms | 2.4 ms |
| Exact discontinuity rays | 0.6 ms | 1.15 ms |
| Validation gather | 0.6 ms | 1.0 ms |

## Summary table

| Item | License (verified) | Verdict | What Renderlab would take | Renderlab problem addressed |
|---|---|---|---|---|
| **Deterministic scatter with 64-bit atomics**: Schütz et al. compute point rasterizer (`m-schuetz/compute_rasterizer`) | MIT (LICENSE.md; some three.js-derived files are also MIT) | **COPY** the pattern (attribution) | Pack depth and payload into a u64 key and resolve it with `atomicMin`/`atomicMax`. Early-depth and warp-reduce tricks | T2 gather cost (a). Deterministic replay (F8) |
| Bevy meshlet software raster (`crates/bevy_pbr/src/meshlet/visibility_buffer_software_raster.wesl`) | MIT OR Apache-2.0 | **COPY** (WGSL/WESL syntax precedent) | `textureAtomicMax` on an R64Uint storage texture under wgpu `TEXTURE_INT64_ATOMIC` and `SHADER_INT64` | Same stack: wgpu/naga 64-bit atomics |
| A-SVGF forward projection (Schied et al. HPG 2018). Code in `NVIDIA/Q2RTX` | Paper. Q2RTX is **GPL-2.0** | **REIMPLEMENT-ONLY** | Forward-project old surface samples, merge them by atomics, and re-shade without interpolation | Prior art for T2 scatter. Its atomic conflict rule is first-come and *nondeterministic* |
| **Area ReSTIR** (Zhang, Lin et al. SIGGRAPH 2024) `guiqi134/Area-ReSTIR` | Repo LICENSE.md is Falcor BSD-3, but module file headers are **NVIDIA proprietary** | **REIMPLEMENT-ONLY** (conflicting notices) | Sub-pixel area reservoirs. The translation-only backprojection reads only **2×2** prior pixels. Stored sub-pixel positions are reused with the identity shift | T2 gather window (a). Supports T2's principle |
| AMD FSR 2 (`GPUOpen-Effects/FidelityFX-FSR2`) | MIT (LICENSE.txt, v2.2) | **COPY** selected pieces | `ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h`: a forward scatter of depth with `InterlockedMax`. The reactive-mask *interface* idea | Forward depth splat. Programmable "reactivity" declaration. *2026-11-07:* also the published cost reference for a shipping reconstruction mode (§6.2) |
| AMD FSR 3 upscaler (FidelityFX SDK v1.1.4 is all MIT. In v2.x the `upscalers/fsr3/include/gpu/...` files are MIT and the rest is binary-only; rechecked on v2.4.0, 2026-10-28) | MIT for those files | COPY possible, but it is **AVOID as the core**. *2026-11-07:* reading **reference** for a shipping TSR-class mode; the core stays Renderlab's T1b lineage (§6.7) | Same class as T1 (resampled history plus clamping and locks). Shading-change pyramid and 4-frame luma-instability ideas | Reference only |
| AMD FSR 4 (ML) | FidelityFX SDK 2.x default license: binary-only, no reverse engineering. An SDK 2.0.0 drop briefly carried FSR 4 source under MIT and was withdrawn (press reports, not verified with AMD) | **AVOID** (unchanged 2026-11-07: provenance, RDNA 4-only ML) | — | — |
| Unreal TSR | Public docs only. UE source is under the Unreal EULA | **REIMPLEMENT-ONLY** (ideas from public docs). *2026-11-07:* now the **acceptance comparator** for M7 reconstruction: TSR Epic at `r.ScreenPercentage=100` (§9) | Public statements that support (c). History resurrection idea | (c). The bar a shipping mode must beat |
| Intel XeSS (`intel/xess`) | Intel Simplified Software License: binary only, no modification | **AVOID**. *2026-11-07:* informative comparator at most (§6.3) | — | — |
| **NVIDIA DLSS SR / DLAA / Ray Reconstruction** (`NVIDIA/DLSS`); wrapper `bevyengine/dlss_wgpu` | DLSS: **NVIDIA RTX SDKs License** (object code only, no reverse engineering, §4(e) no open-source subjection). Wrapper: MIT OR Apache-2.0 | **AVOID** vendoring. Informative comparator only; an optional user-supplied plugin is a licence decision for Islam (§6.3) | — | Published per-GPU cost table (§6.3) |
| Bevy TAA (`crates/bevy_anti_alias/src/taa/taa.wesl`) | MIT OR Apache-2.0 | COPY possible. **AVOID as the core**. *2026-11-07:* do not copy its variance clip: **NVIDIA US 10,116,916** claims it, active to 2037 (§6.4) | Catmull-Rom plus YCoCg variance clip, which is the T1 class | Reference only |
| Karis 2014 TAA; Salvi 2016 variance clipping; Jimenez 2017 CoD dynamic TAA | Talks. Variance clipping patented (US 10,116,916 and continuation US 10,362,289, NVIDIA) | **REIMPLEMENT-ONLY**; variance clip **AVOID** without patent review | Packed depth-velocity `min` for nearest velocity (CoD). Responsive-AA flag idea | PS4 1080p cost points for the class (§6.4) |
| Surface-ID history validation: SVGF mesh-ID taps, A-SVGF visibility-buffer forward projection, Yang et al. 2020 survey | Papers | **REIMPLEMENT-ONLY** (prior art) | Same-surface tests that TSR lacks; triangle identity alone is too strict on scans (M) | Silhouette trails and disocclusion (§6.6) |
| Bevy SMAA (`crates/bevy_anti_alias/src/smaa/smaa.wesl`) and iryoku/smaa | Bevy MIT/Apache plus Jimenez et al. MIT-style notice | COPY possible. **AVOID as the core** | Morphological edge AA from 1 sample | Cannot integrate the footprint |
| Intel CMAA2 (`GameTechDev/CMAA2`, `Projects/CMAA2/CMAA2/CMAA2.hlsl`) | Apache-2.0 | COPY possible. **AVOID as the core** | Same as SMAA | — |
| Filmic SMAA T2x (Jimenez 2016) | Talk only | REIMPLEMENT-ONLY. Low value | Resampled 2-frame history | — |
| **ATAA** (Marrs et al. HPG 2018, Ray Tracing Gems ch. 22) | Paper | **REIMPLEMENT-ONLY** (idea) | Segment out the failure pixels and supersample only those with exact rays. **Ray workload compaction** | Revealed-pixel F2 boost. Exact-ray cost (a) |
| Nehab et al. 2007 reverse reprojection caching. Yang et al. 2009 amortized supersampling | Papers | REIMPLEMENT-ONLY. Already absorbed | Blur analysis that supports the T1 rejection | (c) |
| Walter et al. 1999 render cache | Paper | REIMPLEMENT-ONLY (prior art) | Point reprojection with a depth z-test | Prior art for T2 scatter |
| Drobot HRAA (2014) | Talk/GPU Pro 6 | **AVOID** | Relies on GCN EQAA coverage, which wgpu does not expose | — |
| Zeng et al. 2021 temporally reliable motion vectors | Paper | REIMPLEMENT-ONLY. Later | Shadow motion vectors from blocker and light motion | Light and mover caps (T1 §5) |
| AGAA (Crassin et al. 2015), DCAA (Wang et al. 2015) | Papers | REIMPLEMENT-ONLY. Low value | Decoupling coverage and visibility from shading | Silhouette coverage only |
| Texture-/object-space shading caches | Papers. NVIDIA patents US 9,747,718 and 9,754,407 listed | **AVOID** | — | Already rejected locally (no UVs, bakes program output) |
| AMD FidelityFX Shadow Denoiser (`GPUOpen-Effects/FidelityFX-Denoiser/ffx-shadows-dnsr/*.h`). *2026-11-07:* plus the reflection denoiser `ffx-reflection-dnsr/ffx_denoiser_reflections_{reproject,prefilter,resolve_temporal,common,config}.h` | MIT (LICENSE.txt, 2021) | COPY possible. **AVOID for the film path**. *2026-11-07:* reflection denoiser is a **reference** for M7; its 9×9 statistical history clip needs the variance-clip patent check | Tile classification over 8×4 bitmasks; tile lists that skip non-reflective areas; variable-rate tracing | Cross-pixel filter. Not the AA gap. M7 reflections (§7.3) |
| NVIDIA NRD (SIGMA, ReBLUR, ReLAX) | **NVIDIA RTX SDKs License**: object-code distribution only, no use that makes it open source | **REIMPLEMENT-ONLY** (papers/talks). *2026-11-07:* design **reference** for M7 denoising; RTX 4080 1440p costs 0.40–3.25 ms (§7.1). NVIDIA US 11,847,737 covers a fast-history reset pattern | Recurrent blur driven by history length and hit distance; history fix after resets; inputs contract | Not the AA gap. M7 GI/reflection denoise |
| SVGF (Schied 2017). A-SVGF (2018) | Papers. Falcor SVGF pass is BSD-3 (file headers checked 2026-11-07). Q2RTX is GPL-2 | Falcor: COPY possible. *2026-11-07:* raised from "low value" to **the closest copyable M7 denoiser**; patents US 10,600,167 (to 2038) and US 12,437,371 (temporal gradients, to 2043) need review first | Temporal-gradient idea for fast history drop; mesh-ID tap validation; à-trous with variance-driven edge stops | Light-change reaction. M7 residual denoise (§7.2) |
| kajiya denoisers (`EmbarkStudios/kajiya` `assets/shaders/{rtdgi,rtr,ircache,shadow_denoise,taa}/`), archived 2025-07 | MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE); the shadow denoiser keeps AMD's MIT file | COPY possible (reference). HW RT only | Validation rays that re-trace last frame's GI ray and turn the radiance change into an invalidity signal | M7 lighting-change detection (§7.4) |
| Lumen and MegaLights denoisers (Unreal 5.7) | EULA; cvar names read from the installed binary | Comparator only | Settings to pin in matched runs (§9) | M7 comparator |
| Open neural denoisers: NPPD (`balintio/nppd`), Neural Bilateral Grid (`xmeng525/...`), BMFR-DXR (`gtong-nv/...`, regression, not neural) | Apache-2.0; MIT; BSD-3 (repo licence API, checked 2026-11-07) | **REFERENCE** only. No wgpu runtime; WGSL tensor-core access is experimental | — | Later, if ever (§7.7) |
| Heitz et al. 2018 ratio estimator | Paper | REIMPLEMENT-ONLY. Conflicts with programmability | — | — |
| NVIDIA STBN (`NVIDIA-RTX/STBN`) | **Non-Commercial Use License** | **AVOID** | — | — |
| **EA FAST noise** (`electronicarts/fastnoise`) | BSD-3-Clause | **COPY** (attribution) | Spatiotemporal blue-noise textures and generator | Stratifying the 8 shadow rays and jitter across frames |
| Intel OIDN (`RenderKit/oidn`) | Apache-2.0 | COPY possible. **AVOID in real time** (unchanged 2026-11-07, v2.5.1: single-frame, Vulkan only by external-memory interop, ~15 ms at 1080p reported) | Offline/reference denoising only. No Vulkan compute backend | Reference generation at most |
| ReSTIR DI (Bitterli 2020). NVIDIA RTXDI (`NVIDIA-RTX/RTXDI`) | Paper. RTXDI is under the RTX SDKs License. NVIDIA holds ReSTIR-family patents | REIMPLEMENT-ONLY. Low value now | One sun, so little to gain | Patent risk noted |
| AMD Capsaicin (`GPUOpen-LibrariesAndSDKs/Capsaicin`) | MIT | COPY possible. Later | GI-1.0 (`gi1/world_space_restir.hlsl`, `hash_grid_cache.hlsl`) | Future GI, not AA |
| Bevy Solari (`crates/bevy_solari/src/realtime/restir.wesl`) | MIT OR Apache-2.0 | COPY possible. Later | WESL ReSTIR on wgpu ray queries | Future many-light/GI. RT only; relies on DLSS-RR. *2026-11-07, verified for 0.20:* no built-in denoiser at all (none on macOS), ReSTIR off by default |

## 1. Temporal reconstruction and AA

### 1.1 Forward scatter with deterministic atomics (the key find for T2)

**Schütz, Kerbl, Wimmer, "Rendering Point Clouds with Compute Shaders and Vertex Order Optimization", EGSR 2021 / CGF 40(4)** ([arXiv 2104.07526](https://arxiv.org/abs/2104.07526); code [m-schuetz/compute_rasterizer](https://github.com/m-schuetz/compute_rasterizer), LICENSE.md: MIT, V).

- **Method (V).** Depth and colour are encoded in one 64-bit integer, and "64bit atomicMin [is used] to pick points with the lowest depth" (paper §3.1, Listing 1). Further variants add an early depth test before the atomic and warp-level reduction that "reduc[es] the amount of expensive atomicMin calls from 32 to just one".
- **Throughput (V, RTX 3090).**
  - Retz scene, 145 M points: basic atomicMin 5.07 ms zoomed in and 21.45 ms zoomed out; reduce&early-z 3.37 ms and 3.33 ms.
  - Peak: 796 M points at 62–64 fps, which the authors give as 50 G points/s and 802 GB/s.
  - Morton or "shuffled Morton" order matters by up to about 4×.
- **Follow-up (V):** ["Software Rasterization of 2 Billion Points in Real Time", HPG 2022](https://arxiv.org/abs/2204.01287).
- **Why min/max atomics fix T2's stated objection (Inference).** T2's design rejected unsorted scatter because arrival order breaks F8 ([m6-motion-aa §8](../m6-motion-aa.md#8-rejected-options)). That applies to append and add-with-slot. **It does not apply to `atomicMin`/`atomicMax` on a total-order key:** min and max are commutative and associative, so the result is independent of arrival order. Ties are broken deterministically by putting the source record index in the low bits.
- **wgpu support (V).** The installed `wgpu-types-30.0.1/src/features.rs` defines:
  - `SHADER_INT64_ATOMIC_MIN_MAX`: Vulkan with `VK_KHR_shader_atomic_int64`;
  - `SHADER_INT64_ATOMIC_ALL_OPS`;
  - `TEXTURE_INT64_ATOMIC`: R64Uint image min/max with `VK_EXT_shader_image_atomic_int64`.

  Renderlab's `src/` does not use them yet.
- **Bevy precedent (V).** Bevy's meshlet renderer requires `TEXTURE_INT64_ATOMIC | TEXTURE_ATOMIC | SHADER_INT64` (`crates/bevy_pbr/src/meshlet/mod.rs`). It calls `textureAtomicMax(meshlet_visibility_buffer, …, visibility)` (`visibility_buffer_software_raster.wesl:176`). That is a working WGSL/naga precedent on the same stack. License: MIT OR Apache-2.0 ([bevy README](https://github.com/bevyengine/bevy)).
- **A portable 32-bit fallback (Inference).** It needs two scatter passes:
  1. `atomicMax` of reversed depth per cell;
  2. records whose depth equals the winner do `atomicMin` of their source index.

  This is still order-independent, at about 2× the atomics.

**What Renderlab would take:** the u64 key-packing pattern and the early-test idea. The atomics are a few lines and need no copying, but attribute the pattern to Schütz et al. and Bevy if code is adapted.

### 1.2 Other forward projection and splatting implementations

- **A-SVGF forward projection.** Schied, Peters, Dachsbacher, "Gradient Estimation for Real-time Adaptive Temporal Filtering", HPG 2018 ([PDF](https://cg.ivd.kit.edu/publications/2018/adaptive_temporal_filtering/adaptive_temporal_filtering.pdf)). The paper says (V, §3.3):
  - one previous-frame sample per 3×3 stratum is forward-projected;
  - occluded ones are discarded against the current depth;
  - conflicts in a stratum are resolved "by means of GPU atomics. Whichever sample finishes the reprojection computations first will be merged". That rule is nondeterministic; Renderlab would replace it with the §1.1 key;
  - shading samples are reprojected "without interpolation". Forward projection was chosen because it gives a gradient "entirely free of spatial offsets", which is the same reason T2 avoids resampling;
  - the temporal filter runs in under 2 ms at 1920×1080 on a Titan Xp.

  The code is in [NVIDIA/Q2RTX](https://github.com/NVIDIA/Q2RTX) `src/refresh/vkpt/shader/asvgf_gradient_reproject.comp`, with file headers **GPL-2.0-or-later**. The repo license.txt is GPL-2 (V). **REIMPLEMENT-ONLY**, since Renderlab's license is undecided.
- **FSR 2 previous-depth reconstruction.** [`src/ffx-fsr2-api/shaders/ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h`](https://github.com/GPUOpen-Effects/FidelityFX-FSR2/blob/master/src/ffx-fsr2-api/shaders/ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h) scatters each pixel's depth to its reprojected bilinear footprint. `StoreReconstructedDepth` uses `InterlockedMax` on `asuint(depth)` (`ffx_fsr2_callbacks_hlsl.h:629`), which is a deterministic 32-bit max-scatter (V). It is MIT (LICENSE.txt, V).
- **Unreal TSR** also has a `ForwardScatterDepth` pass. It appears in Guillaume Abadie's profile output in the [TSR feedback thread](https://forums.unrealengine.com/t/tsr-feedback-thread/883977/20) (V). The source cannot be copied.
- **Render cache.** Walter, Drettakis, Parker, EGWR 1999 ([INRIA page](http://www-sop.inria.fr/reves/Basilic/1999/WDP99/)) reprojects cached point samples each frame with depth culling. Walter et al. 2002 made it scale. These are prior art for T2 and are already cited locally. Paper only.

### 1.3 Area ReSTIR: sub-pixel point reuse with a 2×2 window

Zhang\*, Lin\*, Kettunen, Yuksel, Wyman, "Area ReSTIR: Resampling for Real-Time Defocus and Antialiasing", ToG 43(4) / SIGGRAPH 2024 ([Utah project page](https://graphics.cs.utah.edu/research/projects/area-restir/), [PDF](https://graphics.cs.utah.edu/research/projects/area-restir/AreaReSTIR-SIGGRAPH2024.pdf), code [guiqi134/Area-ReSTIR](https://github.com/guiqi134/Area-ReSTIR)).

- **Mechanism (V, §4.2–4.3).**
  - Reservoirs store paths that include the sub-pixel coordinate u.
  - Temporal reuse backprojects the pixel footprint by motion vector δ, which may be fractional. The footprint "overlaps 2 × 2 prior pixels".
  - It keeps candidates whose stored u lies inside the backprojected support. Others get zero MIS weight; this is the "fast option".
  - Selected samples are shifted by −δ with the identity shift. The ratio of target functions is exactly 1 for a static scene (eq. 8), whereas integer-pixel point reuse is "sensitive to visual frequencies … for any non-integer motion" (eq. 10).
  - The "robust" option shifts out-of-support samples by one pixel so every candidate contributes.
  - This is T2's acceptance rule ("count a sample only in the footprint it lands in") derived independently in a resampling framework.
- **Cost (V, RTX 4090, Ryzen 9 7950X, 1920×1080).**
  - Teaser: path tracing 1 spp 4.2 ms, ReSTIR PT 9.5 ms, Area ReSTIR 14.0 ms.
  - Second figure: 4.4, 13.8 and 16.0 ms.

  That is full path tracing with one shaded sample per pixel, not a bolt-on AA pass.
- **License (V).** The repo `LICENSE.md` is Falcor's BSD-3. However, `Source/Modules/AreaReSTIR/TemporalResampling.cs.slang` carries an NVIDIA proprietary header: "Any use, reproduction, disclosure or distribution … without an express license agreement from NVIDIA CORPORATION is strictly prohibited". With conflicting notices, treat it as **REIMPLEMENT-ONLY**. NVIDIA holds ReSTIR-family patents (§3.1).
- **What Renderlab would take (Inference).**
  - The **translation-model backprojection**: each pixel's footprint maps to an offset footprint touching exactly 2×2 source pixels. It is used in a gather variant of T2, applied per motion layer at silhouettes (≤ 2 layers, so ≤ 8 sources).
  - That would cut T2's 9–25-source window by about 2.25–6×.
  - Its approximation is that per-pixel motion is a translation. T2 currently uses the full projective motion and a 3×3 to 5×5 bounding box. The F5/F3 risk at folds must be re-measured.
  - The RIS/reservoir machinery itself is not needed for AA: T2 averages K exact records, which is lower variance than one resampled sample.

### 1.4 ATAA: rays only where history fails

Marrs, Spjut, Gruen, Sathe, McGuire, HPG 2018 ([NVIDIA page](https://research.nvidia.com/publication/2018-08_Adaptive-Temporal-Antialiasing)). The extended version is [Ray Tracing Gems ch. 22](https://casual-effects.com/research/Marrs2019ATAAGem/Marrs2019ATAAGem.pdf).

- **Method (V).**
  - A segmentation mask from motion vectors and screen data selects FXAA for off-screen disocclusions, ray-traced supersampling for disocclusions and undersampled edges and materials, and TAA elsewhere.
  - Rays use "8×, 4×, or 2× MSAA n-rooks" patterns and run the full UE4 material graph.
  - Example: "8× ray traced supersampling for 6% of the total image resolution at a cost of fewer than 0.5 rays per pixel".
- **Numbers (V, UE4 prototype, RTX 2080/2070, 1080p).**
  - The test view selects 103,838 pixels (5%).
  - ATAA is "approximately 2× to 4× faster" than equivalent SSAA.
  - **Ray workload compaction** (a 1D list of selected pixels) gives "13% to 29%" more, or "2.5× to 5×" over SSAA.
  - The FXAA pass costs up to 0.75 ms when the whole frame is new. Under typical camera motion, fewer than 5% of pixels are FXAA.
- **Limitations (V).** Sub-pixel geometry missed by the 1-sample mask flickers between modes. Ray-traced textures used the top mip.
- **Renderlab take (Inference).** The 95% of pixels that ATAA hands to TAA are the class Renderlab rejected, so ATAA is not a fix by itself. Two parts do transfer:
  - **"Supersample exactly where reuse is missing":** this is T2's frozen F2 remedy for revealed pixels, the cold-pixel boost. Local pricing is ≤ 0.01 ms for 7 extra samples on 79–622 revealed pixels.
  - **Compaction:** T2's exact discontinuity rays are traced inline inside a full-screen validation pass. Compacting them into a dense list is the published fix for that divergence.

### 1.5 Unreal TSR (public material only)

Sources: [Epic, "Temporal Super Resolution in Unreal Engine"](https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-in-unreal-engine?lang=en-US) (UE 5.8 docs), [Thin geometry detection](https://dev.epicgames.com/documentation/unreal-engine/thin-geometry-detection-with-temporal-super-resolution?lang=en-US), [TSR FAQ](https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-frequently-asked-questions-for-unreal-engine?lang=en-US) and the [TSR feedback thread](https://forums.unrealengine.com/t/tsr-feedback-thread/883977).

**Verified statements (V).**

- "TSR and other temporal upscalers have to interpolate the previous frame's pixels, which introduces blur." Because of history reprojection, upscalers "can look like they have a resolution of 540p when displaying at 1080p" while things move. (Docs, "TSR Nyquist-Shannon History".)
- Epic and Cinematic store the history at 200% so that reprojection "relies on the Nyquist-Shannon sampling theorem through a Mitchell-Netravali downsample kernel". The docs present this as removing reprojection blur. The history update then costs about 4× more.
- The reprojection field "embeds a Jacobian 2x2 matrix for each pixel", aimed at sharp *geometry* edges in motion.
- Shading rejection is "built with a handwritten network of convolutions". Its output shows as "History Rejection and History Clamp" masks. After rejection, a spatial anti-aliaser takes over that "cannot create information it doesn't have".
- Flickering temporal analysis relaxes rejection by "letting ghosting happen within the luminance boundary tied to the amplitude of flickering". Thin-geometry detection also relaxes clamping, with a default max relaxation weight of 0.037.
- History resurrection keeps older persistent frames, by default 2 frames at an interval of 31.
- Cost examples: Valley of the Ancient "~0.79ms at r.ScreenPercentage=100". Fortnite on PS5/XSX has an effective TSR cost of 1.5 ms and 1.1 ms on the critical path. A forum profile shows RejectShading at 1.88 ms, or 0.51 ms with 16-bit, at 1857×721 on an undisclosed GPU.

The verdict is **REIMPLEMENT-ONLY** for ideas. Do not copy Unreal source into a non-UE public repository.

### 1.6 FSR 2/3 (open), FSR 4, XeSS

- **FSR 2** ([repo](https://github.com/GPUOpen-Effects/FidelityFX-FSR2), LICENSE.txt MIT, V). Shaders are in `src/ffx-fsr2-api/shaders/`:
  - `ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h`
  - `ffx_fsr2_depth_clip.h`: disocclusion from reconstructed previous depth
  - `ffx_fsr2_lock.h` and `ffx_fsr2_postprocess_lock_status.h`: thin-feature luma locks
  - `ffx_fsr2_accumulate.h`, `ffx_fsr2_reproject.h` and `ffx_fsr2_sample.h`: Lanczos history resampling
  - `ffx_fsr2_tcr_autogen.h`: auto reactive/transparency masks
  - `ffx_fsr2_rcas.h`

  Author-reported DX12 timings at 1080p output are 0.2–1.3 ms (Quality) and 0.1–0.9 ms (Ultra Performance), from RX 7900 XTX down to RX 590 (V, README).
- **FSR 3 upscaler.** It is MIT in [FidelityFX SDK v1.1.4](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/v1.1.4) (`sdk/include/FidelityFX/gpu/fsr3upscaler/`), V. In SDK 2.x (`docs/license.md`, V), the default is binary-only with "No reverse engineering". Explicitly listed files are MIT, including `Kits/FidelityFX/upscalers/fsr3/include/gpu/fsr3upscaler/*.h` (reproject, accumulate, shading_change, luma_instability, prepare_reactivity) and the denoiser headers.
- **FSR 4 / ML components.** FSR 4, ML frame generation and Ray Regeneration are not in the MIT list. They are under the binary-only license, and Ray Regeneration does not support Vulkan (V, SDK README). **AVOID.**
- **XeSS** ([intel/xess](https://github.com/intel/xess) LICENSE.txt, V): "provided in binary form only … No reverse engineering … nor any modification". **AVOID.**
- **Renderlab take (Inference).**
  - FSR's accumulation is the T1 class: history resampled by Lanczos, then clamped and locked with luma heuristics. Locks and clamps are style-dependent heuristics, which the film-gap §8 rejects.
  - Copyable value is limited to the 32-bit depth max-scatter and the *reactive mask interface*: a per-pixel engine input saying "trust history less here". For user-programmable shading, the analogue is a program-declared view bandwidth or reactivity value feeding T2's record age cap. That is the open "view-angle cap" decision in m6-motion-aa-gpu §1.

### 1.7 Bevy TAA and SMAA, CMAA2, Filmic SMAA, HRAA

- **Bevy TAA** (`crates/bevy_anti_alias/src/taa/taa.wesl`, V; Bevy is now on WESL and v0.20.1). It uses 5-sample Catmull-Rom history (MJP's form) and 3×3 YCoCg variance clipping (Salvi 2016), with blend rates 0.1/0.015. It is the T1 class with clamping. MIT OR Apache-2.0. Good code reference, **AVOID as core**.
- **Bevy SMAA** (`crates/bevy_anti_alias/src/smaa/smaa.wesl` plus `SMAAAreaLUT.ktx2`/`SMAASearchLUT.ktx2`, V). It is a WGSL port of [iryoku/smaa](https://github.com/iryoku/smaa), whose MIT-style licence does not require the notice in binaries (V). It does not include SMAA's temporal/S2x/4x modes (V, `mod.rs`).
- **CMAA2** ([GameTechDev/CMAA2](https://github.com/GameTechDev/CMAA2), Apache-2.0, V). DX11/DX12 HLSL compute in `Projects/CMAA2/CMAA2/CMAA2.hlsl`.
- **Filmic SMAA** ([Jimenez, SIGGRAPH 2016 Advances](https://advances.realtimerendering.com/s2016/); [slides mirror](http://www.klayge.org/material/4_11/Filmic%20SMAA%20v7.pdf)). The goal was 0.75–1.05 ms on PS4 at 1080p across modes, and the morphological part was optimised from 0.774 to 0.475 ms (V, slides). T2x reprojects a 2-frame history.
- **HRAA** ([Drobot, SIGGRAPH 2014 Advances](https://advances.realtimerendering.com/s2014/index.html); [slides](https://michaldrobot.com/2014/08/13/hraa-siggraph-2014-slides-available/)). It uses GCN EQAA coverage at higher-than-depth resolution plus MSAA depth-only samples. wgpu does not expose EQAA or custom sample locations (Inference from the wgpu feature list). **AVOID.**
- **Inference.** Morphological AA reconstructs edges from a single sample per pixel. It cannot recover the 43% lit-pixel shading aliasing ([film gap §2](../m6-film-gap.md#2-radiance-against-the-film-image)), so none of these can meet the film bar.

### 1.8 Classic reprojection theory and motion vectors

- **Nehab, Sander, Lawrence, Tatarchuk, Isidoro, "Accelerating real-time shading with reverse reprojection caching"**, Graphics Hardware 2007. This is the origin of per-pixel history fetch. Paper only.
- **Yang, Nehab, Sander, Sitthi-amorn, Lawrence, Hoppe, "Amortized supersampling"**, ToG 28(5) 2009 ([PDF](https://www.cse.ust.hk/~psander/docs/amss.pdf), [project](https://hhoppe.com/proj/supersample/)). The abstract says (V): "The key difficulty is to prevent accumulated blurring during successive reprojections. We present a theoretical analysis of the blur introduced by reprojection methods." It keeps 2×2 sub-pixel estimates and limits α. This is the T1/TSR lineage, and its blur analysis is what Renderlab's §1 table quantifies.
- **Zeng, Liu, Yang, Wang, Yan, "Temporally Reliable Motion Vectors for Real-time Ray Tracing"**, CGF 40(2), EG 2021 ([PDF](https://sites.cs.ucsb.edu/~lingqi/publications/paper_trmv.pdf)). Its shadow motion vectors "track the movement of the blocker and the light source, and re-calculate the shadow position in the previous frame" (V). That helps where history must follow *moving shadows*: Renderlab's moving-sun and moving-blocker cases, currently capped by W_light/W_mover. It does nothing for camera-motion AA. Because the method feeds a resampled history, it inherits T1's leak. REIMPLEMENT-ONLY; later.

### 1.9 Decoupled coverage and object-space caches

- **AGAA**, Crassin, McGuire, Fatahalian, Lefohn, I3D 2015 ([PDF](https://research.nvidia.com/sites/default/files/pubs/2015-02_Aggregate-G-Buffer-Anti-Aliasing/AGAA_I3D2015_authors.pdf)). It prefilters sub-pixel geometry into aggregates. With 2 aggregates it is "comparable to 32x MSAA" at up to 2.6× faster (V). Aggregation prefilters geometry and normals before shading, which bakes a lighting-model assumption. That conflicts with programmable shading (Inference).
- **DCAA**, Wang, Wyman, He, Sen, HPG 2015 ([PDF](https://research.nvidia.com/sites/default/files/pubs/2015-08_Decoupled-Coverage-Anti-Aliasing/hpg15_DCAA.pdf)). It stores a 64-bit coverage mask plus one depth per merged surface. It addresses geometric aliasing only.
- **Texture- and object-space shading.** NVIDIA patents **US 9,747,718** ("performing object-space shading") and **US 9,754,407** ("shading using a dynamic object-space grid") are listed on [Wyman's page](https://cwyman.org/paperList.html) (V). Renderlab already rejected this class ([m6-motion-aa §2](../m6-motion-aa.md#2-alternatives)). **AVOID.**
- **Inference.** Coverage-only methods address the silhouette share, 15% of squared error, but not interior shading aliasing.

## 2. Denoisers for shadows and stochastic lighting

Context: the film gap is footprint integration, not 8-ray shadow noise, which is under 1% of the N = 8 error ([m6-motion-aa §3](../m6-motion-aa.md#3-cpu-prototype)). Cross-pixel filters are rejected for bias ([film gap §8](../m6-film-gap.md#8-rejected-options)). Denoisers therefore matter only for future stochastic GI or many lights.

- **AMD FidelityFX Shadow Denoiser** ([GPUOpen-Effects/FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser), MIT, V).
  - Files: `ffx-shadows-dnsr/ffx_denoiser_shadows_{prepare,tileclassification,filter,util}.h`. They are also in SDK v1.1.4 `sdk/include/FidelityFX/gpu/denoiser/`.
  - Design (V, [docs](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/v1.1.4/docs/techniques/denoiser.md)): input is "at most one jittered shadow ray per pixel" packed as 8×4 bitmasks. A tile classification pass skips tiles without variance. It uses 17×17 separable moments, neighbourhood-clamped history and 3 À-Trous passes.
  - The **tile classification over packed bitmasks** is a reusable cost trick. The filter is a cross-pixel filter and clamping heuristic. **AVOID for the film path.**
- **NVIDIA NRD** (SIGMA, ReBLUR, ReLAX; [NVIDIA-RTX/NRD](https://github.com/NVIDIA-RTX/NRD), v4.18.0, 2026-10-30).
  - **License (V):** the "NVIDIA RTX SDKs LICENSE" (LICENSE.txt, last changed 2021-04-13). It allows distribution only "as incorporated in object code format". §4(e): "You may not use the SDK in any manner that would cause it to become subject to an open source software license", including licences requiring source distribution. **REIMPLEMENT-ONLY** from published talks; never vendor NRD shaders into this public repository.
  - SIGMA timing (V, README): SIGMA_SHADOW 0.40 ms at native 1440p on an RTX 4080.
- **SVGF**, Schied et al. HPG 2017, and **A-SVGF** (§1.2).
  - Falcor ([NVIDIAGameWorks/Falcor](https://github.com/NVIDIAGameWorks/Falcor), LICENSE.md BSD-3, V) has an SVGF render pass. Linked RTXDI and DLSS keep their own licenses.
  - A-SVGF's **temporal gradient** re-shades forward-projected samples with the old random seed to detect lighting change and drop history quickly. That is the published way to react to light and program changes without lag (Inference). It applies to T1a/T2 record age caps under moving lights, at about 1/9 of samples re-shaded.
  - **Patents (V, [Wyman's page](https://cwyman.org/paperList.html)):** US 10,600,167, "Performing spatiotemporal filtering", is likely the SVGF filing (Inference: same title as Schied's US 2018/0204307). NVIDIA's US 11,113,792 (temporal-spatial denoising with clamping) and US 11,783,455 (history clamping) were also found ([Google Patents](https://patents.google.com/patent/US11783455)).
- **Heitz, Hill, McGuire, "Combining analytic direct illumination and stochastic shadows"**, I3D 2018 ([author page](https://eheitzresearch.wordpress.com/705-2/)). It denoises the ratio (illumination-weighted shadow) separately from unshadowed analytic shading. That requires the engine to know the unshadowed light integral analytically, which is a lighting-model-specific decomposition. It conflicts with user-programmable shading unless the program exports it (Inference). REIMPLEMENT-ONLY; not now.
- **Blue noise.**
  - [NVIDIA-RTX/STBN](https://github.com/NVIDIA-RTX/STBN) License.txt is a "Non-Commercial Use License" (V). **AVOID**, including derived textures; the [three-geospatial issue #117](https://github.com/takram-design-engineering/three-geospatial/issues/117) shows the contamination risk.
  - **EA FAST** ([electronicarts/fastnoise](https://github.com/electronicarts/fastnoise), BSD-3-Clause, V) provides filter-adapted spatiotemporal sampling textures and a generator. **COPY** with attribution.
  - Use: per-pixel 8-ray light-disk sample sets that are stratified *across frames*. Each T2 record and each T1a static frame then contributes complementary shadow samples (Inference: about 1% of error now, but free).
- **Intel OIDN** ([RenderKit/oidn](https://github.com/RenderKit/oidn), Apache-2.0, V; v2.5.1). It runs on CPUs and GPUs: Intel Xe via SYCL, NVIDIA via CUDA, AMD via HIP and Apple via Metal, and is described as capable "for interactive or even real-time ray tracing" (V). It has no Vulkan compute device; interop would go through external memory (Inference). It is a neural cross-pixel filter. Use at most for offline reference cleanup, and never on the gate references themselves.
- **Neural real-time denoisers.** DLSS-RR, FSR Ray Regeneration and XeSS are closed or binary. Bevy Solari depends on DLSS-RR: about 6.1 ms of its 7.3–14.1 ms frame at 1600×900 to 3200×1800 on an RTX 3080 at base clocks ([jms55 0.18 post](https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18/), V). No open real-time neural denoiser with a permissive license and wgpu support was found. **AVOID.**

## 3. Sampling efficiency

### 3.1 ReSTIR DI and RTXDI

- **ReSTIR DI.** Bitterli, Wyman, Pharr, Shirley, Lefohn, Jarosz, SIGGRAPH 2020 ([project](https://cs.dartmouth.edu/~wjarosz/publications/bitterli20spatiotemporal.html)). The [2023 course notes](https://intro-to-restir.cwyman.org/presentations/2023ReSTIR_Course_Notes.pdf) are cited by Bevy's `restir.wesl`.
- **RTXDI** ([NVIDIA-RTX/RTXDI](https://github.com/NVIDIA-RTX/RTXDI), v3.1.0) is under the **NVIDIA RTX SDKs License** (V), with the same object-code-only terms as NRD. **REIMPLEMENT-ONLY.**
- **Patents (V, [Wyman's page](https://cwyman.org/paperList.html); [US 11,315,310 on Justia](https://patents.justia.com/patent/11315310)).** NVIDIA holds:
  - US 11,315,310: ReSTIR with a world-space GI structure;
  - US 11,663,773 and its continuation US 12,361,633: importance resampling for light-sampling memory coherence;
  - US 11,948,246: spatiotemporal resampling with decoupled shading and reuse;
  - US 12,361,635: initial candidates;
  - US 12,437,371: temporal gradients;
  - US 12,299,801: grid-based light sampling.

  A patent on the 2020 DI algorithm itself was not found in this search. That is not a clearance. Any ReSTIR-family implementation needs a real patent review before shipping.
- **Renderlab relevance (Inference).**
  - The registered scene has one sun disk (0.04 rad) and a uniform environment. Resampling one light's disk samples gains little over stratified 8 rays.
  - ReSTIR's target function needs a BRDF and light evaluation, so with user programs the engine would need a program-provided target. Record that as a programmable-shading constraint before adopting it.
  - It matters for future many-light scenes.

### 3.2 Area ReSTIR

See §1.3. It is the only published ReSTIR variant aimed at AA. Reported cost is +1.5 to +4.5 ms over ReSTIR PT on an RTX 4090 at 1080p (V). That is far over budget as a whole, but its 2×2 sub-pixel matching is reusable.

### 3.3 AMD Capsaicin

[GPUOpen-LibrariesAndSDKs/Capsaicin](https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin), LICENSE MIT (V). It is HLSL. Notable paths are `src/core/src/render_techniques/gi1/` (`world_space_restir.hlsl`, `hash_grid_cache.hlsl`, `screen_probes.hlsl`, `gi_denoiser.hlsl`) and `visibility_buffer/disocclusion_mask.comp`. Screen probes are screen-space GI, which conflicts with the rule against screen-space lighting. The hash-grid world cache could be **COPY**able for future GI. It is not relevant to AA.

### 3.4 Bevy Solari

- Files are in `crates/bevy_solari/src/realtime/` (V): `restir.wesl` (unified initial and temporal plus spatial-and-shade, `enable wgpu_ray_query;`, 30-px spatial radius), `world_cache_*.wesl`, `presample_light_tiles.wesl` and `initial_path.wesl`. License: MIT OR Apache-2.0.
- **Same stack, so the most directly portable ReSTIR**: WESL on wgpu ray queries, with an RTX 3080 per-pass table (V, §2).
- Solari is hardware-RT-only in practice. jms55 calls ReSTIR "likely too expensive for software-based RT" in [issue #20203](https://github.com/bevyengine/bevy/issues/20203) (V, search summary). It relies on DLSS-RR.
- Its BRDF is baked into reservoir evaluation (`package::scene::brdf`), which is a lighting-model coupling.
- COPY later for GI. Not for AA.

## 4. Answers to the three questions

### (a) Could T2's cost be cut with a known technique?

**Yes, substantially, but no published technique shows it reaching the 1.075 ms stop.** The levers map onto the measured attribution:

1. **Gap gather (1.3 / 2.4 ms) disappears with stratum-binned deterministic scatter.** Sources: Schütz key-packing, A-SVGF per-stratum forward projection, render cache.
   - Each record is reprojected once and scattered into one of K **sub-cells** of the pixel it lands in.
   - Example key: `atomicMax(cell, [8-bit frame tag | 24-bit reversed depth | 32-bit source index])`.
   - The frame tag makes stale entries lose without a clear pass. Reads check the tag.
   - The nearest record per sub-cell wins, deterministically.
   - The fresh-sample "gap choice" becomes "take an empty sub-cell's lattice position". That is a K-entry read of the pixel's own cells, with no neighbour gather.
   - Each record keeps its exact sub-pixel position. The sub-cell is only a bin, so nothing is resampled.
   - Cost estimate (Inference):
     - 1.4 M (K = 4) to 2.9 M (K = 8) records on the 357 k explicit-record pixels;
     - Schütz's basic atomicMin throughput is about 29 G points/s on a 3090. Scale to about half for the 4070 SUPER's bandwidth;
     - record traffic is about 50–150 MB, priced with T1a's measured ~0.1 ms per 100 MB.
     - Scatter plus resolve: about 0.1–0.2 ms (K = 4) and 0.2–0.35 ms (K = 8).
     - Key buffers: 66 MB (K = 4) or 133 MB (K = 8) at full 1080p allocation. K = 8 then exceeds the 0.6 GB budget unless the records' two sides are pooled.
   - Behaviour change: at most one record per sub-cell, where T2 keeps up to K−1 youngest anywhere. This gives better stratification but sometimes fewer records. Re-measure F1, F3 and the CPU-model check.
   - Still required: the plane test (F5 hidden records) and exact rays for background records at silhouettes.
2. **Validation gather (0.6 / 1.0 ms) shrinks to an own-pixel read** under the same scatter: K keys plus K record fetches, with the plane tests of 5 pixels unchanged. A cheaper alternative needs no atomics: Area ReSTIR's **2×2 translation backprojection** per motion layer, which is about 2–6× fewer sources than 3×3–5×5.
3. **Exact discontinuity rays (0.6 / 1.15 ms).**
   - The rays are only about 0.1–0.25 M per frame (local §5 estimate). At the measured all-primary rate (2 M rays in 0.23–0.27 ms), the work should cost tens of µs, so the measured time is mostly scheduling: rays inline in a divergent full-screen loop (Inference).
   - Published fix: **ray workload compaction** (ATAA-C, 13–29% in its scene; more expected here because the band is about 5% of pixels).
   - With scatter there is one ray per sub-cell winner, not per arrived record.
   - Estimate (Inference): 0.1–0.4 ms.
4. **Projected total (Inference, unverified).**

   | | K = 4 | K = 8 |
   |---|---|---|
   | Moving frame | ≈ 0.866 + 0.3 to 0.7 ms ≈ **1.15–1.55 ms** | ≈ **1.3–1.9 ms** |
   | Against the 1.075 ms stop | Likely still above; close only if the rays compact well and the scatter hits the low end | Above |

   This is a 2.5–4× cut from 3.55 / 5.76 ms, but no published system proves the stop is reachable. The first step should be the short scatter-only reproducer the T2a doc already proposes, run against these estimates.

### (b) Is there a known technique that reaches film-quality AA under motion within about 0.2–0.3 ms extra?

**No published technique with author-reported evidence does this on 1x input with a path-traced-film bar.**

- FSR 2/3, TSR, Bevy TAA and Filmic SMAA T2x all resample history and clamp. Epic itself says interpolating the previous frame "introduces blur" and can look like 540p at 1080p in motion (V, §1.5).
- ATAA reaches about 8× SSAA only on its 5% ray-traced pixels and relies on TAA elsewhere (V).
- Area ReSTIR costs milliseconds (V).
- DLSS, DLSS-RR, XeSS and FSR 4 are closed, and their motion quality against a film reference is not published.
- The closest **known-mechanism** candidate is T2 with stratum-binned deterministic scatter (§4a). Its estimated extra cost of 0.3–0.7 ms at K = 4 is 1.5–3× the 0.2–0.3 ms window.

Record this as a miss, not a reason to relax the bar.

*2026-11-07 note.* The miss still stands against the film target. Islam has since set the shipping floor at Unreal 5.7 matched or beaten on every measure, at 2x its performance. A TSR-class mode that beats TSR in motion can therefore ship, labelled below film. §6.7 records the changed verdict and §11 the unit that tests it.

### (c) Is "TSR leaks across hard shadow edges like T1" supported or contradicted by public material?

- **Supported in mechanism (V):**
  - Epic states that temporal upscalers including TSR "interpolate the previous frame's pixels, which introduces blur" and look like 540p at 1080p in motion.
  - TSR's controls are colour/luma rejection and clamping ("History Rejection and History Clamp", a "handwritten network of convolutions").
  - Flicker analysis and thin-geometry detection deliberately *relax* rejection, letting ghosting happen "within the luminance boundary tied to the amplitude of flickering".
  - Yang et al. 2009 identify accumulated reprojection blur as "the key difficulty" of the whole class.
- **Partly contradicted (V):** Epic presents the 200% Nyquist-Shannon history with Mitchell-Netravali downsampling as *removing* reprojection blur.
  - **Inference:** that argument assumes band-limited content. A hard shadow edge inside a solid pixel's neighbourhood is a step, not band-limited. Renderlab's analytic table shows 200% roughly halves the gap-0 leak (0.21 → 0.097) rather than removing it.
  - Neighbourhood clamping does not stop this leak at a shadow edge. The current 3×3 neighbourhood already spans both lit and shadowed values, so the leaked history lies inside the clamp box.
  - Where TSR's shading rejection *does* fire, it trades the leak for 1x spatial-AA output, which "cannot create information it doesn't have" (V).
- **Not established:** no public Epic material discusses shadows, ghosting or blur at shadow edges specifically, and no public measurement of TSR against a film reference was found.
- **Status:** the inference is plausible and consistent with Epic's own statements, but its magnitude for TSR is unmeasured. The planned TSR-on Unreal capture at the registered poses ([film gap §3](../m6-film-gap.md#3-unreal-with-tsr-off)) is the only way to settle it. Until then, make no comparative claim either way.

## 5. Recommended first takes

1. **Stratum-binned deterministic scatter for T2** (Schütz u64 atomic keys, A-SVGF and render-cache forward projection). It removes the gap gather and most of the validation gather, keeps F8 bit-identity by construction, and runs on wgpu 30 (`SHADER_INT64` plus `SHADER_INT64_ATOMIC_MIN_MAX`, or `TEXTURE_INT64_ATOMIC`, verified in the installed crate). There is a 32-bit two-pass fallback. Bevy meshlet WGSL is the syntax precedent.
   - *Risks:* sub-cell collisions change the kept set (F1/F3 re-measure); key memory at K = 8; atomic contention when zoomed out (Schütz: 4× slowdown for naive atomicMin in dense views, cured by early-z and reduction).
   - *Cost to try:* one scatter-only GPU reproducer on Q-walk, priced before any quality work, as T2a §6 already proposes.
2. **ATAA-style compaction of exact discontinuity rays**, plus the cold-pixel boost for revealed pixels. Both are ideas only (paper).
   - It attacks the 0.6 / 1.15 ms ray share and F2.
   - *Risk:* the compaction pass overhead. ATAA notes it "may not always be beneficial".
3. **Area ReSTIR's 2×2 translation backprojection** as the no-atomics fallback for T2's gather (reimplement only).
   - *Risk:* the translation approximation at folds and silhouettes may raise F5 hidden records and F3 edge bias. Use per-layer windows at silhouettes.
4. **EA FAST spatiotemporal noise** (BSD-3, COPY) for cross-frame stratification of light-disk samples and of the sub-pixel lattice scramble.
   - Small, cheap, and license-clean. It replaces any temptation to use NVIDIA STBN, which is non-commercial.
5. **A-SVGF temporal gradients** (reimplement) as the principled history-drop signal for moving lights and animated user programs, instead of colour clamping.
   - It re-shades about 1/9 of forward-projected samples with the old seed.
   - *Risk:* it needs seed and visibility-record storage, and its cost is part of A-SVGF's 2 ms (Titan Xp, 1080p).

**Licence hygiene for all of these.**

- Never vendor NRD, RTXDI, XeSS, FSR 4, STBN or Area ReSTIR module files.
- Q2RTX is GPL; read it for ideas only.
- Unreal source stays a reading reference under its EULA.
- If MIT, BSD or Apache code is adapted (Schütz, Bevy, FSR 2 files, FAST, CMAA2), keep its notice in the file header and in a third-party notices file once Renderlab's license is chosen.
- ReSTIR-family work needs a patent review first.

## 2026-11-07 M7 extension

Research date 2026-11-07, base `cf001a5`. This covers [milestone-7.md](../milestone-7.md): GI, reflections and temporal reconstruction. It uses the same method as above: public sources, licence files read from each repository, no builds, no GPU runs. Two local reads were added:

- the installed Unreal 5.7 `Engine/Config/BaseScalability.ini`;
- console-variable names and help strings read as text from `Engine/Binaries/Win64/UnrealEditor-Renderer.dll`.

Neither involved Unreal source.

**The rule change behind this section.** On 2026-11-06/07 Islam set the shipping floor at Unreal 5.7 quality matched or beaten on every measure, at ≥ 2x Unreal's performance, with film quality as the long-term target. §1–§5 judged TSR-class methods only against the film bar. Here they are judged against the floor.

### 6. Temporal reconstruction as a shipping mode

**What Renderlab already has (M).** T1b ([m6-temporal-gpu §S2](../m6-temporal-gpu.md#s2-t1b-camera-motion), commit `b8fa1f3`, reverted) is a TSR-class camera-motion reconstruction:

- It resamples history through the exact hit record, keeps no colour clamp, and uses a blur budget.
- **Cost:** +0.20–0.22 ms over Interactive, or about 1.07 ms on the quiet base.
- **Motion residual:** 3.1–3.8 codes against Exact's 7.0–7.2.
- **Converged pixels:** all-covered pixels sit at 0.60–0.79 of the Exact bar while moving.
- **Failures against the film bar:**
  - silhouette ghost trails of 5–8 px;
  - background-centre pixels at 1.37–2.21x the bar;
  - Catmull-Rom undershoot specks;
  - shadow-edge blur.

**None of this has been compared to TSR.** Unreal's TSR-on image and cost at the registered cameras are still unmeasured ([film gap §3](../m6-film-gap.md#3-unreal-with-tsr-off)). Whether T1b already beats the floor is open, and it is the first thing to measure (§11).

#### 6.1 Unreal TSR (UE 5.0–5.7, public material)

**Technique (own words, V unless marked).**

- **History.** TSR keeps its history at display resolution and reprojects it each frame.
  - Epic and Cinematic keep that history at **200%** of output. Every lower group uses 100% (BaseScalability.ini 5.7, `AntiAliasingQuality@0–@Cine`). Cinematic also stores full-precision history (`r.TSR.History.R11G11B10=0`).
  - The history holds up to 16 samples by default, configurable from 8 to 32 (`r.TSR.History.SampleCount`).
- **Disocclusion** is inferred "solely" from the current frame's depth and velocity. TSR has no surface identity.
- **Shading rejection** is a network of convolutions computed per 32×32 tile on chip (5.7 cvar help). Where it rejects, a spatial anti-aliaser fills in (`r.TSR.RejectionAntiAliasingQuality`, off at Low).
- **Anti-flicker** (`r.TSR.ShadingRejection.Flickering`, on at High and above):
  - Luminance that oscillates within a period counts as flicker. The cvar help gives the period as 3 frames; the docs say both 2 and 3.
  - For flicker, TSR deliberately allows ghosting up to the flicker's amplitude.
  - It tracks luminance only, and switches off per pixel for moving objects and large parallax.
- **Thin-geometry detection** relaxes the clamp box up to `Coverage.MaxRelaxationWeight`. In 5.7 it is on by default only when Nanite Foliage is enabled in the project (cvar help).
- **Resurrection** stores 2 persistent frames, one every 31 frames.
- **Velocity weight clamping** defaults to 4 samples, described as "cinematic"; Fortnite uses 2.
- **Reprojection field** (High and above) stores a per-pixel Jacobian.
- **Convergence.** "TSR 1spp" is the time to one sample per output pixel, `(100/ScreenPercentage)^2` frames: 1 frame at 100%, 4 at 50%.
  - (I) A full 16-sample history at native resolution takes about 16 frames.

**Motion-vector requirements (V, TSR docs).**

- **WPO:** animated World Position Offset needs the Previous Frame Switch. "Output velocities due to vertex deformation" is on by default.
- **Instancing:** per-instance transform motion gets no automatic velocity.
- **Translucency** draws "no or at most one" velocity.
- **Pixel animation:** panning textures need the material's **Has Pixel Animation** flag.

**Published cost (V).**

| Case | Hardware and resolution | Cost |
|---|---|---|
| Valley of the Ancient | Not stated | ~0.79 ms at 100%, ~0.43 ms at 50% |
| Fortnite | PS5/XSX, 4K output | 1.5 ms effective, 1.1 ms on the critical path, after async compute hides ~0.5 ms |
| Forum ProfileGPU, sync mode | Steam Deck, 640×360 → 1280×720, `sg.AntiAliasingQuality=1` | 2.63 ms total, of which flicker analysis ~0.58 ms and resurrection ~0.1 ms |

- The history update is the most expensive pass, and 200% history makes it about 4x dearer.
- No 1080p-native or 40-series number was found, so it has to be measured locally (§9).

**Failure modes Epic documents.**

- reprojection blur, the "540p in motion" look;
- 1-px-thick details that move half a pixel;
- intentional ghosting within the flicker amplitude;
- colour-only flicker that passes the luminance test;
- translucency without velocity;
- parallax-occlusion interiors with wrong motion vectors.

**Licence and patents.** EULA, so **REIMPLEMENT-ONLY** from docs. No Epic-assigned patent on temporal reconstruction was found. That is not a clearance.

#### 6.2 FSR 2 and FSR 3.1 (MIT)

**Files (V).**

- **FSR 2:** see §1.6.
- **FSR 3.1:** FidelityFX SDK v2.4.0 (2026-10-28) `docs/license.md` still lists these as MIT, with the rest binary-only and "No reverse engineering":
  - the GPU headers `Kits/FidelityFX/upscalers/fsr3/include/gpu/fsr3upscaler/ffx_fsr3upscaler_{accumulate,callbacks_hlsl,common,debug_view,luma_instability,luma_pyramid,prepare_inputs,prepare_reactivity,rcas,reproject,resources,sample,shading_change,shading_change_pyramid,upsample}.h`;
  - the pass shaders `Kits/FidelityFX/upscalers/fsr3/internal/shaders/ffx_fsr3upscaler_*_pass.hlsl`.

**Technique (own words, V from the FSR 3.1.4 docs).**

1. Reconstruct dilated motion and previous depth by max-scatter.
2. Flag disocclusion from that depth.
3. Resample the history with Lanczos.
4. Measure shading change in a half-resolution luma pyramid: the minimum relative luma difference in each quad, times the consistency of its sign, maximised over mips.
5. Compute luma instability by comparing current luma with the last 4 frames. Where luma flickers, colour clamping is reduced, as TSR does. This is ignored at high velocity, at disocclusion and under strong reactivity.

Two engine masks steer it:

- The **reactive mask** (0–1; write alpha; clamp to about 0.9) cuts history influence.
- The **transparency-and-composition mask** only removes locks and clamping.

**Motion vectors (V, FSR 2 README).**

- All opaque, alpha-tested and alpha-blended objects should write motion.
- Vertex-shader effects such as scrolling UVs should be included in the motion.
- Vectors are unjittered. An inverted infinite depth buffer is recommended.

**Cost (V, AMD README, FSR 2.2, DX12, Quality = 1.5x).**

| Output | RX 7900 XTX | RX 6800 XT | RX 6700 XT | RX 590 |
|---|---:|---:|---:|---:|
| 1920×1080, Quality | 0.2 ms | 0.3 ms | 0.5 ms | 1.3 ms |
| 3840×2160, Quality | 0.7 ms | 1.2 ms | 2.0 ms | 5.4 ms |

- FSR 3.1 publishes no time table, only that it is "slightly lower" in performance than 3.0.
- Its memory at 1080p Quality is a 61 MB working set (RX 6700 XT).
- (I) The 4070 SUPER is roughly 6800 XT class: about 0.3 ms at 1080p from 720p input, and more at native input, since there are 2.25x the pixels. This is unmeasured.

#### 6.3 FSR 4, DLSS/DLAA, XeSS: licences and use as comparators

- **FSR 4:**
  - Officially it ships as signed DLLs through `amd_fidelityfx_loader.dll` and needs RDNA 4 (press reports).
  - The SDK 2.0.0 drop briefly contained its source under MIT, and AMD withdrew it ([VideoCardz](https://videocardz.com/newz/amd-releases-fidelityfx-sdk-2-0), [Guru3D](https://www.guru3d.com/story/amd-accidentally-opensources-fsr-4-code-on-github/)).
  - **AVOID**: the provenance is contested, and it is vendor-specific ML.
- **DLSS SR, DLAA and Ray Reconstruction:**
  - **Licence (V).** [`NVIDIA/DLSS` LICENSE.txt](https://github.com/NVIDIA/DLSS/blob/main/LICENSE.txt) is the NVIDIA RTX SDKs License: distribution "as incorporated in object code format" (§2c), no reverse engineering (§4a), and no use that subjects it to an open-source licence (§4e).
  - **Wrapper.** [`bevyengine/dlss_wgpu`](https://github.com/bevyengine/dlss_wgpu) is MIT OR Apache-2.0 (LICENSE-MIT, V). Its README says it builds against a user-supplied DLSS SDK on Vulkan, and that apps ship the DLL plus NVIDIA's licence text.
  - **Cost (V, DLSS Programming Guide 310.6.0, 31 March 2026).** Performance mode (input 1/4 of output pixels), standalone utility:

    | GPU | Output | E/F (CNN) | J/K (transformer) | L | M |
    |---|---|---:|---:|---:|---:|
    | RTX 4070 Ti | 1080p | 0.26 ms | 0.56 ms | 0.90 ms | 0.68 ms |
    | RTX 4070 Ti | 4K | 1.02 ms | 1.97 ms | 3.18 ms | 2.74 ms |
    | RTX 4080 | 1080p | 0.20 ms | 0.47 ms | 0.79 ms | 0.54 ms |

    - Memory at 1080p is 85.77 MB for J/K.
    - Preset K is the default for DLAA, Quality and Balanced. Preset J "might exhibit slightly less ghosting at the cost of extra flickering".
    - DLAA at native input costs more than these figures. The guide has no DLAA row.
- **XeSS:** binary-only, with no modification or reverse engineering (V, §1.6).
- **Are they valid comparators? (I)**
  - M7's acceptance names Unreal 5.7 and TSR, so the **gate is TSR Epic**.
  - DLAA preset K, and XeSS where its plugin is installed, are worth one *informative* row in a matched Unreal run, because many shipping Unreal titles use them. They are never the pass bar: they are vendor-locked, closed, and not Unreal's own quality.
  - An optional Renderlab DLSS plugin through `dlss_wgpu` is technically possible on wgpu 30 + Vulkan. It would be NVIDIA-only, never a default, and never presented as Renderlab's own quality. Whether the public repository may link it at all is **a licence decision for Islam**.

#### 6.4 Classic TAA lineage

- **Karis 2014**, "High Quality Temporal Supersampling", SIGGRAPH Advances ([slides](https://de45xmedrsdbp.cloudfront.net/Resources/files/TemporalAA_small-59732822.pdf), V):
  - Jittered samples are spread over frames, with tonemap-weighted accumulation.
  - "Need velocity (motion vectors) for everything", with the front-most velocity dilated.
  - A depth compare is rejected because "all samples don't share same depth".
  - A 3×3 neighbourhood clamp, shaped, in a YCoCg box, with *clip* toward the neighbourhood average rather than clamp.
  - A "Responsive AA" material flag sets stencil for translucency and minimises feedback.
  - Anti-flicker by biasing toward impulse frames.
- **Salvi 2016**, variance clipping: a box of mean ± γσ over the neighbourhood, γ = 1 (V via [Yang et al. 2020](http://behindthepixels.io/assets/files/TemporalAA.pdf)).
  - **Patent (V, [Google Patents](https://patents.google.com/patent/US10116916B2/en)):** US 10,116,916, NVIDIA (Salvi, Patney, Lefohn). Claim 1 covers TAA history adjusted by an AABB built from the per-component mean and standard deviation of the neighbourhood. Status active, expiring 2037-04-04. Continuation: US 10,362,289.
  - Bevy TAA (§1.7) implements this kind of clip. Copying Bevy's code is licence-clean but not patent-clean.
- **Jimenez**, "Dynamic Temporal Antialiasing in Call of Duty: Infinite Warfare", SIGGRAPH 2017 Advances ([slides v4](https://www.activision.com/cdn/research/Dynamic_Temporal_Antialiasing_and_Upsampling_in_Call_of_Duty_v4.pdf), V):
  - temporal upsampling layered on dynamic resolution;
  - a 1-sample spatio-temporal bicubic resample, which reduces the 5-sample bicubic to one tap across time;
  - jitter scaled by velocity, with no jitter near 0.5 px/frame;
  - a half-resolution nearest velocity, packed with depth in the high bits of a `uint` so that a single integer `min` picks it. That is the same trick as §1.1's key packing.
  - **PS4 cost, 1080p output:**

    | Mode | Input | Cost |
    |---|---|---|
    | T2x | 1920×1080 | 0.8–0.99 ms |
    | TU2x | 1440×1080 | 0.75–0.94 ms |
    | TU4x | 1440×1080 | 1.0–1.26 ms |

  - Talk only, so **REIMPLEMENT-ONLY**.
- **Survey.** Yang, Liu and Salvi, "A Survey of Temporal Antialiasing Techniques", CGF 39(2) 2020 (V). It lists depth, normal, **object ID** and motion as the geometry data for rejecting history, and Schied's forward-projected gradients for shading change.

#### 6.5 Motion vectors, reactive masks and programmable shading

Every engine above asks for the same three things: complete motion for everything, vertex-animation motion, and a per-pixel "trust history less" input. Renderlab's mapping (I, building on [T1 §6](../m6-temporal-primaries.md#6-programmable-shading)):

- **Rigid motion is exact by construction.** Each hit record is taken back through the previous camera and its instance's previous transform. No velocity buffer, dilation or front-most-velocity trick is needed, and instancing gets motion automatically. TSR's per-instance caveat does not apply.
- **Vertex deformation** is unsupported today (programmable shading C13). When it lands:
  - The deformation must be a pure function of rest position, time and parameters, so the engine can evaluate it at t−1. This is TSR's Previous Frame Switch, done automatically.
  - Otherwise the deforming instance's history weight is 1.
  - Record this constraint with C13.
- **Pixel animation and time.** The already-recorded rule is the reactive mask, declared rather than painted: programs that reference `time` get W = 1 unless they declare a temporal bandwidth. The same declaration covers panning UVs.
- **Translucency** is out of M7 scope. When it arrives, each program declares its reactivity.
- **No luminance-based reactivity.** FSR's auto-reactive mask and TSR's shading rejection read colour. Renderlab's T1 weights read none ([T1 decision 5](../m6-temporal-primaries.md#decisions)), so every style converges to its own integral. §6.6 gives the style-safe replacement.

#### 6.6 Surface-ID validation: where Renderlab can beat TSR

**Prior art (V).**

- **Nehab et al. 2007:** depth test.
- **SVGF 2017 §4.1:**
  - each of 2×2 bilinear taps tests backprojected depth, object-space normal and **mesh ID**;
  - inconsistent taps lose their weight to consistent ones;
  - a 3×3 search follows "to help find thin geometry such as foliage".
- **A-SVGF 2018 §3.3:** forward-projects **visibility-buffer** samples ("information about the triangle intersection"), so the current frame sees "the same point on the surface".
- **Yang et al. 2020:** object ID in the survey's validation inputs.

No paper was found that validates history with an exact (instance, primitive, barycentrics) record beyond A-SVGF's visibility-buffer forward projection.

**Local evidence (M).** Triangle identity is too strict on these scans. T1 measured triangle-identity and micro-normal tests and dropped them: at the registered 1 cm step they reject 4.5–70% of pixels whose history is 2–3x better than Exact ([T1 decision 3](../m6-temporal-primaries.md#decisions)). Scan triangles of about 6 mm are close to the 8.2 mm footprint. The useful ID test is therefore **instance, then surface continuity (depth within 4 footprints), then reverse visibility** (the tap's own surface is still visible within 1.5 px). T1 already has exactly that.

**Where identity beats TSR's depth and velocity (I).**

1. **Disocclusion.** Reverse visibility asks whether this pixel's previous surface point is the one visible now. That catches disocclusion behind thin occluders, which current-frame depth and velocity cannot see.
2. **Silhouette trails**, T1b's main failure:
   - An **anchor bit** marks history that mixes surfaces, and only the anchored surface may accept it. Edge pixels then restart instead of trailing.
   - The heavier fix is two-layer silhouette history.
   - Both are identity tests that TSR cannot make.
3. **Shading change without colour clamps.** Re-shade the anchored previous sample with the current program and inputs, using its old seed, as in A-SVGF. The difference is the program's *own* output change, so the test is style-correct.
   - Run it only on pixels the engine flags: light angular rate, a moving caster, a parameter edit, exposure.
   - Or run it on 1 of 9 strata, as A-SVGF does.
4. **Engine-known causes.** Light rates and mover transforms already set T1's lag-bounded windows. The N8o surface cache has exact caster invalidation ([non-rt-n8o](../non-rt-n8o.md)). Together they give deterministic per-pixel W caps without reading colour.

**Where identity does not help.** Sub-pixel resampling blur across hard shadow edges is structural (§4c; T1b's shadow-edge blur). TSR has the same class, halved by its 200% history (analytic, [m6-motion-aa §1](../m6-motion-aa.md#1-why-resampled-pixel-history-fails)). Whether TSR's clamp or flicker logic hides it, and what its error is at those pixels, is still unmeasured.

#### 6.7 Verdict (changed 2026-11-07)

- **The TSR class is allowed as a shipping mode** under the floor. It is labelled "temporal reconstruction, below film", and becomes the default only if it beats TSR Epic on every measured population, static and moving, with no trails or flicker beyond TSR's, at ≤ ½ of TSR's measured cost. T1a stays the converging static path. Film convergence in motion (T2-class) remains the target and stays recorded as a miss.
- **Base it on T1b plus identity fixes, not on FSR or TSR copies.** FSR 3.1's MIT files and Bevy TAA are reading references only.
  - Do not adopt variance clipping without patent review (US 10,116,916).
  - Do not adopt colour or luma rejection: it is style-dependent.
- **Native first.** Match TSR at `r.ScreenPercentage=100` (TSR acting as AA). Sub-native upscaling only enters if M7 GI and reflections cost so much per pixel that 2x is lost at native (I).

### 7. Denoisers for M7 GI and reflections

**Constraints from the project.**

- **Incident quantities only** ([programmable shading §2.4](../programmable-shading.md#24-realistic-path-quality-bar-and-world-space-inputs)): the engine accumulates and denoises incident quantities, never program output.
- **Clean inputs** ([milestone-7](../milestone-7.md)): GI and reflection inputs reach programs as clean values.
- **No screen-space core lighting.**

That ranks the denoiser classes (I):

1. **World-space accumulation keyed by surface.** It keeps off-screen and disoccluded history, and fits the N8n/N8o surface cache's deterministic claim/resolve and exact invalidation.
2. **Screen-space spatiotemporal filters over traced incident signals** (NRD, SVGF, FidelityFX). The lighting comes from world-space rays; only the filtering is screen-space. Disocclusion loses history, so these are acceptable only as residual cleanup that settles within the transient limit (§8).
3. **Neural.**

**Demodulation, restated for user programs (I).** NRD and SVGF divide radiance by albedo, or by a pre-integrated BRDF, before filtering. Renderlab can skip that division: the filtered signal *is* the incident quantity, irradiance for diffuse and lobe-prefiltered radiance for specular, and the program applies its own albedo or BRDF afterwards. Programs therefore never have to export albedo. They do have to export the **filter guides**:

- **Normal:** geometric from the hit record, or the program's shading normal.
- **Specular lobe:** roughness, or a lobe width.

That is the open `scatter` / secondary-hit contract (programmable shading R4). A program that declares no lobe gets diffuse-only filtered inputs. **Record this constraint with R4.** Material and program identity come free from the hit record, as NRD's material ID or SVGF's mesh ID.

#### 7.1 NVIDIA NRD v4.18.0 (ReBLUR, ReLAX, SIGMA)

**Technique (own words).**

- **ReBLUR** is a "recurrent blur":
  - it accumulates temporally with a per-pixel history length;
  - then it blurs with a radius that shrinks as history grows;
  - hit distance drives the specular reprojection and the blur radius;
  - a **history-fix** pass rebuilds the first frames after a reset, and a stabilization pass follows.
- **ReLAX** is an à-trous filter in the SVGF lineage, "designed for RTXDI".
- **SIGMA** is a shadow denoiser driven by penumbra size.

**Inputs (V, [README](https://github.com/NVIDIA-RTX/NRD) and `Include/NRDDescs.h`).**

- **Guides:**
  - `IN_MV`: unjittered, with old = new + MV; 2.5D recommended;
  - `IN_NORMAL_ROUGHNESS`: world normal plus *linear* roughness;
  - `IN_VIEWZ`.
- **Optional:** `IN_DIFF/SPEC_CONFIDENCE` ("highly recommended") and `IN_DISOCCLUSION_THRESHOLD_MIX`.
- **Signals:**
  - `IN_DIFF_RADIANCE_HITDIST`, `IN_SPEC_RADIANCE_HITDIST`, and SH variants;
  - `IN_PENUMBRA` and `IN_TRANSLUCENCY` for SIGMA.
- **Rules:**
  - Materials must be demodulated by the app. NRD never reads base colour or metalness.
  - Material IDs work only with R10G10B10A2 normals.
  - `hitT` must be ≥ 0, must exclude the primary hit's distance, and must not be divided by the PDF.
  - "A solid baseline is to use hit distance for the 1st bounce only."
  - Under probabilistic lobe selection, the skipped lobe gets 0.

**Defaults (V, `Include/NRDSettings.h`, master).**

| Setting | ReBLUR | ReLAX | SIGMA |
|---|---:|---:|---:|
| Max accumulated frames | 30 | 30 (diffuse and specular) | — |
| Max fast-history frames | 6 | 6 | — |
| History-fix frames after a reset | 3 | 3 | — |
| Max stabilized frames | — | — | 5 |

- `disocclusionThreshold` is 0.01, and accumulation should be set "in seconds, not frames".
- Anti-lag: "an application should not rely solely" on it, and initial integration should start with it disabled.

**Cost (V, README).** RTX 4080, native 1440p, defaults:

| Denoiser | Cost | SH mode |
|---|---:|---:|
| REBLUR_DIFFUSE_SPECULAR | 2.55 ms | 3.40 ms |
| RELAX_DIFFUSE_SPECULAR | 3.25 ms | 4.80 ms |
| SIGMA_SHADOW | 0.40 ms | — |
| SIGMA_SHADOW_TRANSLUCENCY | 0.45 ms | — |

- Anti-firefly adds 0–2% on ReBLUR and 7–10% on ReLAX.
- No 30-series table was found.
- (I) Scaling to the 4070 SUPER at 1080p (0.56x the pixels, about 0.75x a 4080's throughput) gives ReBLUR ≈ 1.9 ms, ReLAX ≈ 2.4 ms and SIGMA ≈ 0.3 ms.

**Licence and patents.**

- RTX SDKs licence (V), so **REIMPLEMENT-ONLY**.
- **Patent (V, [Google Patents](https://patents.google.com/patent/US11847737)):** US 11,847,737, NVIDIA (Kozlowski), "Temporal denoiser quality in dynamic scenes", priority 2021, granted 2023-12-19. It covers a short moving average plus an exponential history, with the exponential part dropped when a dynamic event is detected: the "fast history" pattern. The history-clamping patents listed in §2 also apply.

**Fit (I).**

- A ReBLUR-like recurrent blur over *incident* diffuse and specular needs no demodulation.
- Guides come exact from hit records: view Z, normal, motion, instance.
- Hit distance exists wherever a ray was traced, on either backend.
- Roughness needs the R4 lobe contract.

#### 7.2 SVGF and A-SVGF; ReLAX against SVGF

**SVGF** (Schied et al., HPG 2017, [preprint PDF](https://research.nvidia.com/sites/default/files/pubs/2017-07_Spatiotemporal-Variance-Guided-Filtering%3A//svgf_preprint.pdf), V).

- **Method:**
  - input is 1 spp, with direct and indirect light separated and albedo demodulated;
  - an exponential moving average with fixed **α = 0.2** ("trading temporal stability for lag");
  - the 2×2 and 3×3 ID-tested reprojection of §6.6;
  - temporal luminance moments give variance;
  - a 5-level à-trous wavelet with 5×5 cross-bilateral taps, using depth, normal and variance-scaled luminance edge stops;
  - TAA afterwards.
- **Cost:** 4.4 ms at 1280×720 and **10.2 ms at 1920×1080 on an NVIDIA TITAN X (Pascal)**, about 200 MP/s.
  - Frames vary from 4.1 to 5.8 ms, driven by disocclusions.
  - Worst frames, at cuts, run about 50% slower.
- **Limitations stated by the authors:**
  - over-blurred chrominance, because variance tracks luminance only;
  - **detached shadows in motion**, because accumulation is unaware of illumination change;
  - unstable noise in very low light;
  - over-blurred specular;
  - shadows over-blurred during camera motion.

**A-SVGF** (Schied, Peters, Dachsbacher, HPG 2018, V).

- **Method:**
  - one sample per 3×3 stratum is forward-projected through the visibility buffer with its stored seed and re-shaded;
  - the normalised temporal gradient sets a per-pixel α, from the global α up to full rejection (maximum over 3×3);
  - a 5-level 3×3-box à-trous gives a 49×49 effective footprint;
  - when history drops, variance comes from the spatial estimate, which "tends to introduce stronger spatial blur".
- **Cost:** the temporal filter runs in < 2 ms and gradient sampling plus reconstruction in < 1 ms, on a **Titan Xp at 1920×1080**.
- **Limitations stated by the authors:**
  - it needs a temporally coherent primary sample space, so adding lights or changing importance sampling drops history;
  - it cannot handle stochastic primary visibility such as motion blur or depth of field;
  - it resolves stratum conflicts first-come (§1.2).

**Code.**

- **Falcor** `Source/RenderPasses/SVGFPass/` contains `SVGFReproject`, `SVGFFilterMoments`, `SVGFAtrous`, `SVGFFinalModulate` and `SVGFPackLinearZAndNormal` (`.ps.slang`), plus `SVGFPass.cpp/.h`. File headers are BSD-3 NVIDIA 2015-23 and the repo LICENSE.md is BSD-3 (V). **COPY with notice**, ported from Slang to WGSL.
- **Q2RTX** `asvgf*.glsl/.comp` is GPL-2, so **REIMPLEMENT-ONLY**.

**Patents (V, Google Patents).**

- US 10,600,167, "Performing spatiotemporal filtering", NVIDIA, active to 2038-07-07.
- US 12,437,371, "Illumination resampling using temporal gradients…", NVIDIA, active to 2043-07-21.

Review both before shipping SVGF-style filtering or gradient-driven history.

**ReLAX against SVGF.** On the same RTX 4080 at 1440p (V), ReLAX costs 3.25 ms against ReBLUR's 2.55 ms. Default lobe-angle fractions are 0.5 for ReLAX and 0.15 for ReBLUR (`NRDSettings.h`).

- (I) ReLAX is SVGF's à-trous plus NRD's fast history, history fix and anti-lag. Those three are what SVGF lacks against the 8-frame limit (§8).

#### 7.3 AMD FidelityFX Denoiser (MIT)

- **Repository (V):** [GPUOpen-Effects/FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser), LICENSE.txt MIT (AMD, 2021).
  - Shadows: `ffx-shadows-dnsr/` (§2).
  - Reflections: `ffx-reflection-dnsr/ffx_denoiser_reflections_{common,config,prefilter,reproject,resolve_temporal}.h`.
  - The same denoisers are MIT in SDK v1.1.4.
  - In SDK 2.x, the "FSR Ray Regeneration" denoiser is ML and binary-only (`signedbin/amd_fidelityfx_denoiser_dx12.dll`, V licence listing), and DX12-only (§1.6). **AVOID** that one.
- **Reflections design (V, [v1.1.4 docs](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/v1.1.4/docs/techniques/denoiser.md)).**
  - **Inputs:** depth hierarchy, motion vectors, normals, extracted roughness, ping-pong radiance and variance, and a tile list with indirect arguments. There is **no hit-distance input**.
  - **Reprojection:** finds disocclusions and estimates variance, with an 8×8 radiance mip.
    - The fast path uses mirror parallax from neighbourhood similarity, or surface motion with neighbourhood distance checks.
    - A slow path for edges builds a 2×2 interpolated sample.
  - **Spatial:** variance-guided, 15 samples in 7×7, weighted by similarity to the 8×8 average.
  - **Temporal:** 9×9 Gaussian-region statistics clip the history.
  - It supports variable-rate tracing, from full rate for mirrors to quarter rate for glossy.
  - No timings are published.
- **Fit (I).**
  - The tile lists and variable-rate tracing are directly useful.
  - The temporal clip is statistical, mean ± σ style, so check it against US 10,116,916's claims before copying.

#### 7.4 kajiya (MIT OR Apache-2.0; archived 2025-07)

- **Files (V, repository tree):**
  - `assets/shaders/rtdgi/`: ReSTIR GI with `restir_temporal`, `restir_spatial`, `restir_resolve`, `temporal_filter`, `spatial_filter`, `diffuse_validate.rgen`, `temporal_validity_integrate` and `fullres_reproject`;
  - `rtr/`: reflections with `rtr_restir_temporal`, `temporal_filter`, `spatial_cleanup` and `reflection_validate.rgen`;
  - `ircache/`: world-space irradiance cache;
  - `shadow_denoise/`: based on FFX, keeping AMD's `ffx/LICENSE.txt`;
  - `taa/`.
- HLSL. The visual features need an NVIDIA RTX GPU, and "RTX Off" disables most of them. No timings are published, and the README says "denoising needs more work (always)".
- **Notable technique (V, source read).** `diffuse_validate.rgen.hlsl` re-traces last frame's reservoir ray from its stored origin. When the hit radiance differs, it writes an invalidity value (a smoothstep of the relative difference). `temporal_validity_integrate.hlsl` blurs that value spatially, and the temporal filter uses it to shorten history. This is a ray-based temporal gradient on *secondary* rays: it catches indirect-light changes that are far from the pixel.
- **COPY with notice is possible.** It is HW RT only, and the ReSTIR patent note (§3.1) applies.

#### 7.5 Bevy Solari and DLSS Ray Reconstruction

- **Bevy 0.20** (2026-10-08, [notes](https://bevy.org/news/bevy-0-20/), V):
  - Solari's denoising is **DLSS-RR** through `dlss_wgpu`, now DLSS-RR 4.5;
  - "there is currently no built-in denoiser" on macOS;
  - ReSTIR is off by default because, with DLSS-RR, "ReSTIR costs a decent chunk of performance, and does not significantly improve image quality".

  Solari has no open denoiser to take.
- **DLSS-RR:** the RTX SDKs licence and NVIDIA-only (§6.3), so **AVOID** as a default. It is useful only as an informative comparator.

#### 7.6 Intel OIDN 2.5.1

- **Licence:** Apache-2.0 (LICENSE.txt, V).
- **Modes:** high, balanced and fast. The README names balanced and fast "for interactive and real-time use cases". OIDN 2.3 added fast mode at about 1.5–2x the speed of balanced.
- **Devices:** SYCL, CUDA, HIP and Metal. Vulkan is reached only through external-memory and semaphore interop (README §Interop, V).
- **Speed:** one third-party report gives about 15 ms at 1080p in device memory, and 220 ms with the default managed memory on Windows (GPU unnamed, [blendity PR](https://github.com/EmilianoManaloIV/blendity/pull/85)).
- It is a single-frame filter with no temporal input. **Unchanged:** reference cleanup only.

#### 7.7 Open neural real-time denoisers

- **NPPD**, Bálint et al., SIGGRAPH 2023 ([balintio/nppd](https://github.com/balintio/nppd)): LICENSE Apache-2.0 (V; the paper says MIT). PyTorch.
- **Neural Bilateral Grid**, Meng et al., EGSR 2020 ([xmeng525/RealTimeDenoisingNeuralBilateralGrid](https://github.com/xmeng525/RealTimeDenoisingNeuralBilateralGrid)): MIT (V). TensorFlow. The authors claim "a few milliseconds per frame".
- **BMFR**, as a DXR port ([gtong-nv/BMFR-DXR-Denoiser](https://github.com/gtong-nv/BMFR-DXR-Denoiser)): BSD-3 (V). Blockwise regression, not neural.

None has a wgpu or WGSL runtime. wgpu 30's `EXPERIMENTAL_COOPERATIVE_MATRIX` supports only 8×8 f32 tiles, and its own docs warn that Vulkan support "may be limited" (local `features.rs`, V). Tensor-core inference from WGSL is therefore impractical now (I).

A network trained on one look also risks the style dependence that programmable shading forbids, unless it filters incident quantities only. **REFERENCE, later.**

### 8. Lighting-change responsiveness and latency

The project caps the soft-shadow settle after cuts and disocclusion at **8 frames** (Islam, `M6-transient-limit`). T1's F2 gate is ≤ Interactive at every age and ≤ the bar by the 8th frame after any event.

| Method | History | Reaction to light, emissive or material change | Disocclusion | Settle (published or derived) |
|---|---|---|---|---|
| TSR Epic | ≤ 16 samples | Same-frame shading rejection, then spatial AA. Anti-flicker ghosts on purpose within the flicker amplitude over a 3-frame period (V) | Inferred from depth and velocity; spatial AA fills | 1 frame to 1 spp at 100% (V); ~16 frames to a full history (I) |
| FSR 3.1 | Lanczos history | Shading-change pyramid; 4-frame luma instability; reactive mask from the app (V) | Previous-depth test | Not published |
| DLSS / DLAA | Closed | Preset J vs K trades ghosting against flicker (V) | Closed | Not published |
| SVGF | EMA α = 0.2 | None: lag and detached shadows (V) | 2×2/3×3 ID-tested taps, then spatial variance | Stale share 0.8^n: 33% at frame 5, **17% at frame 8** (I, arithmetic) |
| A-SVGF | Adaptive α | Gradient drops history in the same frame; spatial blur while recovering (V) | Forward projection | About 1 frame to react; recovery blur not quantified |
| NRD ReBLUR / ReLAX | 30 frames | History fix 3 frames, fast history 6; anti-lag optional and not to be relied on (V) | Handled internally | Designed for ≤ 6-frame recovery (I) |
| kajiya RTDGI | Temporal filter plus ReSTIR | Validation rays shorten history where hit radiance changed (V) | Reprojection map | Not published |
| Lumen GI (UE 5.7) | Surface cache plus screen-probe temporal filter | Surface cache amortised: `DirectLighting.UpdateFactor=32`, `Radiosity.UpdateFactor=64` at Epic (V ini). Docs admit lighting can "fade rather than quickly turn on or off". A fast-update mode applies where traces hit moving objects (V, cvar help) | World-space `DistanceThreshold` history discard (V, cvar help) | Not published. `MaxFramesAccumulated` default unverified (community reports 10) |
| MegaLights (UE 5.7) | `MaxFramesAccumulated` 12 | SVGF-style temporal variance ([Epic, SIGGRAPH 2025 Advances](https://advances.realtimerendering.com/s2025/content/MegaLights_Stochastic_Direct_Lighting_2025.pdf), via a search summary, not read in full) | — | 12-frame history ([Epic Games Japan, Tokyo DevDays 2026](https://www.docswell.com/s/EpicGamesJapan/Z27EW1-DayDays26-MegaLights), via a search summary) |

**Implications (I).**

- **Use engine-known events before detection.** The engine knows when a light moves (with its angular rate), when a caster moves (exactly, as in N8o), when parameters change, when exposure changes, and which time bandwidth a program declared. Each event caps W on the receivers it affects, at no detection cost and deterministically. Gradients, A-SVGF re-shade or kajiya-style validation rays then cover what events miss, mainly far-field indirect change.
- **Moving sun.** Direct shadows are fresh every frame. Indirect light changes everywhere at once, so a fixed-α filter such as SVGF fails the 8-frame limit: 17% stale at frame 8 against a hard change. A rate-derived W, or a gradient, is required.
- **Emissive changes.** Emitters are program output, so their changes are known events: parameter edits, `time`. Their effect on far receivers needs validation rays or gradients.
- **Disocclusion.** This is where world-space caches beat screen-space denoisers. A surface the camera saw earlier keeps its cached incident light, while NRD-style filters must rebuild through history fix. Never-seen surfaces start cold, as T2's revealed pixels did, and need the same "supersample where reuse is missing" boost (§1.4) to settle within 8 frames.
- **Proposed for Islam:** apply the 8-frame settle limit to GI and reflections as well, measured as ≤ Unreal at every age and ≤ the bar by frame 8.

### 9. Unreal 5.7 settings to match

UE **5.7.4 CL 51494982** is installed at `C:/Program Files/Epic Games/UE_5.7` ([comparison setup](../unreal-comparison-setup.md)).

**Reconstruction.**

- **`r.AntiAliasingMethod=4`** for TSR (V, TSR docs) and **0** for the existing 1x captures (V, [shadow comparison](../unreal-quarry-shadow-comparison.md)). Values 1–3 are FXAA, TAA and MSAA from general knowledge, not checked this session.
- **`sg.AntiAliasingQuality=3` (Epic)** sets the following (V, BaseScalability.ini 5.7):

  | Setting | Epic `@3` | Cinematic `@Cine` | High `@2` |
  |---|---|---|---|
  | `r.TSR.History.ScreenPercentage` | 200 | 200 | 100 |
  | `r.TSR.History.R11G11B10` | 1 | **0** | 1 |
  | `r.TSR.History.UpdateQuality` | 3 | 3 | 2 |
  | `r.TSR.ShadingRejection.Flickering` | 1 | 1 | 1 |
  | `r.TSR.RejectionAntiAliasingQuality` | 2 | 2 | 1 |
  | `r.TSR.ReprojectionField` | 1 | 1 | 1 |
  | `r.TSR.Resurrection` | 1 | 1 | 1 |
  | `r.TemporalAA.Quality` | 2 | 2 | 1 |
  | `r.FXAA.Quality` | 4 | 5 | 3 |

  - Match **Epic** as the shipping comparator.
  - Add **Cinematic** as the stronger still-frame row.
- **`r.ScreenPercentage=100`** at 1920×1080. TSR then acts as AA, matching Renderlab's native 1x input.
  - `sg.ResolutionQuality` maps directly to `r.ScreenPercentage`. Presets are Performance 50, Balanced 58.328, Quality 66.662 and Native 100 (V ini).
  - Add a 66.662% row so TSR's cheaper upscaled mode is also on record (I).
- **Record and pin, at defaults:**
  - `r.TSR.History.SampleCount` (16);
  - `r.TSR.ShadingRejection.Flickering.Period` (3);
  - `r.TSR.Velocity.WeightClampingSampleCount` (4);
  - `r.TSR.ThinGeometryDetection` (on only with Nanite Foliage);
  - `r.TSR.AsyncCompute` (default 2).
- **Motion vectors.** The quarry is static meshes under camera motion, so velocity is complete. For movers, check deformation velocity output and the per-instance caveat (§6.1).

**GI and reflections (Lumen).**

- **`sg.GlobalIlluminationQuality=3` and `sg.ReflectionQuality=3`** set the following (V ini):

  | Setting | GI Epic `@3` | GI Cinematic `@Cine` |
  |---|---:|---:|
  | `r.Lumen.ScreenProbeGather.DownsampleFactor` | 16 | 8 |
  | `TracingOctahedronResolution` | 8 | 16 |
  | `NumAdaptiveProbes` | 8 | 4 |
  | `RadianceCache.ProbeResolution` | 32 | 32 |
  | `RadianceCache.NumProbesToTraceBudget` | 100 | 350 |
  | `FullResolutionJitterWidth` | 1 | 0.5 |
  | `r.LumenScene.DirectLighting.UpdateFactor` | 32 | 32 |
  | `r.LumenScene.Radiosity.UpdateFactor` | 64 | 64 |

  At reflection Epic:
  - `r.Lumen.Reflections.DownsampleFactor=1`;
  - `ScreenSpaceReconstruction.NumSamples=5`;
  - `ScreenSpaceReconstruction.MinWeight=0`;
  - `MaxRoughnessToTraceForFoliage=0.4`.
- **Denoiser cvars present in 5.7** (names V from the renderer binary). Their defaults are not in the scalability ini, so dump them with `DumpCVars r.Lumen.` (or `<cvar> ?`) in the matched process and store them in the evidence JSON. Keep them at default; do not tune Unreal.
  - `r.Lumen.ScreenProbeGather.Temporal`, with `.MaxFramesAccumulated`, `.DistanceThreshold`, `.NormalThreshold`, `.RejectBasedOnNormal`, `.MaxFastUpdateModeAmount`, `.FastUpdateModeUseNeighborhoodClamp`, `.FractionOfLightingMovingForFastUpdateMode` and `.RelativeSpeedDifferenceToConsiderLightingMoving`;
  - `r.Lumen.ScreenProbeGather.TemporalFilterProbes`, with `.HistoryWeight` and `.HistoryDistanceThreshold`;
  - `r.Lumen.Reflections.Temporal`, with `.MaxFramesAccumulated`, `.NeighborhoodClampScale`, `.DistanceThreshold` and `.MaxRayDirections`;
  - `r.Lumen.Reflections.BilateralFilter`, with `.KernelRadius`, `.NumSamples` and `.DepthWeightScale`;
  - `r.Lumen.Reflections.ScreenSpaceReconstruction`, with `.NumSamples`, `.KernelRadius`, `.MinWeight` and `.RoughnessScale`;
  - `r.MegaLights.Temporal.*`.
- **Hardware and software paths** (names V):
  - `r.Lumen.HardwareRayTracing` 0 or 1;
  - `r.Lumen.HardwareRayTracing.LightingMode`;
  - `r.Lumen.Reflections.HardwareRayTracing.HitLighting`.

  Software Lumen traces mesh SDFs and reads the surface cache. It has no hit lighting and does not support WPO. Hardware Lumen can light the hit for reflections (V, Lumen technical details).

**How to time TSR and Lumen.**

1. **End to end (the honest number).** Run paired, same-process A/B frames with `r.AntiAliasingMethod` 4 and 0, using the existing CSV harness (160 frames, measured rows 96–159). The `GPUTime` difference includes TSR's side costs: jitter, full velocity and mip bias. For Lumen, toggle GI and reflections the same way.
2. **Attribution with ProfileGPU** at CSV frame 64 (existing harness).
   - The TSR node is "Scene > PostProcessing > TemporalSuperResolution". The Lumen passes are "Lumen Scene Lighting", "Lumen Screen Probe Gather" and "Lumen Reflections" (V docs).
   - In 5.6 and later, ProfileGPU prints per-queue tables. `r.ProfileGPU.Root`, `.ShowLeafEvents` and `.ThresholdPercent` exist in 5.7 (names V).
   - Set `r.TSR.AsyncCompute=0` and `r.Lumen.AsyncCompute=0` for attribution runs only, because overlapping async work skews pass times (V, Lumen performance guide). Never sum overlapping queues (local practice).
3. **`stat tsr`** gives "TSR feed" and "TSR 1spp" (V).
4. **Visual diagnosis:**
   - `r.TSR.Visualize`: 0 accumulated samples, 1 parallax disocclusion, 2 rejection, 3 clamp, 4/5 resurrection, 6 spatial AA, 7 flicker;
   - the `VisualizeTSR` show flag;
   - `DumpGPU` with `Root="*TemporalSuperResolution*"` for buffers. Put dumps on E: and delete them after summarising.

### 10. With and without hardware RT

| Aspect | Hardware RT | Non-RT |
|---|---|---|
| Reconstruction (T1/T1b) | Traced primaries with exact hit records | Raster or visibility buffer with exact surface identity and a jittered projection ([non-rt-path](../non-rt-path.md#motion-aa-and-film-convergence)). **Same code and cost**, except T2-class discontinuity rays at 2–5x (I) |
| GI and reflection rays | Inline ray queries: hit distance, and an instance/primitive/barycentric record at the secondary hit, so the program's `surface` can run there | Software BVH. N0 measured 8-ray shadows at **9.43 ms, 21.9x hardware** (M, [non-rt-n0](../non-rt-n0.md)). Per-pixel secondary rays must be few or short. Most incident light comes from world-space caches (surface-cache cells, virtual clipmap, cluster-LOD proxies) |
| Hit distance for the denoiser | Always available | Only for traced rays, not for cache lookups. The denoiser needs a no-hitT mode: normal, depth and roughness guides, or the cache cell's position as a virtual hit point for specular parallax (I) |
| Confidence and age inputs | Ray-based validation (kajiya) or gradients | Cache cell age and consensus (N8o certifies solid cells by consensus) can serve as NRD-style confidence (I) |
| Unreal comparator | Lumen hardware: hit lighting, far field, skinned meshes (V) | Lumen software: SDFs and surface cache, no hit lighting, no WPO, 200 m scene range extendable to 800 m (V). The non-RT path must beat this (AGENTS) |

**Rule (I).** Reconstruction and denoising are one implementation with one set of gates, run on both backends. Any input that exists only with hardware RT (per-pixel `hitT`, secondary hit records) needs a named non-RT substitute before the feature counts as complete.

### 11. Recommended direction for M7 (reconstruction + denoising)

**Principles (I).**

- One path-agnostic reconstruction, extending T1b.
- Validity from identity and engine events, not colour statistics.
- Denoise incident quantities: world-space surface-keyed accumulation first, then screen-space spatiotemporal cleanup only for the residual, validated by instance and reverse visibility.
- No vendored NVIDIA, AMD 2.x-binary or Epic code.
- A patent review before any variance clip, SVGF-style filter, temporal gradient or fast-history reset ships.

**Reconstruction.** The variants for RT and non-RT are identical by construction (§10).

- **R0, the first measurable unit.** Unreal capture plus CPU scoring; no Renderlab GPU change.
  - **Capture:** Unreal TSR Epic, `r.AntiAliasingMethod=4`, `r.ScreenPercentage=100`, 1080p, offscreen. Run the registered Q-walk (161 frames), Q-disocclusion (`sequences/registered.json`) and Q-sun at the same cameras. Include a 1x row (`=0`) and a Cinematic row.
  - **Score** with `scripts/m6-temporal-motion-score.py` (from `b8fa1f3`) against the existing Level F references, on T1b's populations:
    - F1 converged and F2 by age;
    - silhouette trail width and background-centre error;
    - flicker in the hold.
  - **Time** with §9's A/B and attribution.
  - **Output:** TSR's error per population as a multiple of the bar, and TSR's end-to-end ms.
  - **Payoff:** it also settles §4(c), whether TSR leaks at shadow edges.
  - **Cost:** one offscreen Unreal batch plus a CPU score, with no Renderlab build.
- **R1, T1b revived as a reconstruction mode.**
  - **Build:** add the anchor bit (§6.6), event-driven W caps, and the Catmull-Rom undershoot fix (clamp the kernel's negative lobes at silhouettes, or switch to bilinear there). Use no colour clamp.
  - **Gate:** at least as good as TSR (R0) on every population at every age, on Q-walk, Q-disocclusion and Q-sun.
    - Trail width and hold flicker no worse than TSR's;
    - F8 replay identical;
    - non-RT run byte-identical to the paired software run;
    - cost delta ≤ ½ of TSR's measured end-to-end delta, with Renderlab's total frame time ≤ ½ of Unreal's.
  - **Stop rules:**
    1. If a population remains worse than TSR after the anchor bit plus *one* named fix (two-layer silhouette history, or the ±1 px reverse search), stop and record the miss.
    2. If the delta exceeds half of TSR's measured cost after one optimisation pass, stop.
    3. If R0 shows TSR worse than T1b everywhere already, skip the extra fixes and gate T1b as is.
- **R2, only if R1 fails on shading change.** A-SVGF-style re-shade of the anchored samples on event-flagged pixels, after the US 12,437,371 review.
- **Upscaling:** deferred (§6.7).

**Denoising.** The GI and reflection estimators themselves belong to gi.md and reflections.md. This is the denoising side.

- **RT variant:**
  1. Per-pixel traced incident signals with `hitT`: irradiance, and radiance per specular lobe.
  2. Accumulate them into surface-keyed world cells where certified, using N8o's deterministic claim/resolve and exact invalidation.
  3. Clean the residual in screen space with either a Falcor SVGF port (BSD-3, notice kept) or a ReBLUR-like reimplementation, with history fix ≤ 3 frames, fast history ≤ 6, and ID-tested taps.
  4. Use kajiya-style validation rays on 1/9 of pixels for far-field change.
- **Non-RT variant:**
  1. The same caches as the primary source, with short software rays.
  2. The same denoiser in its no-hitT mode, with the same guides and gates.
  3. Validation by gradient re-shade instead of rays where rays are too dear.
- **D0, the first measurable denoise unit.** CPU replay of saved buffers, per AGENTS.
  - **Input:** a 1-spp cosine-weighted diffuse incident-irradiance sequence dumped from the RT path on Q-walk and Q-sun, plus Q-disocclusion. That is one ray per pixel against the BVH, with no GI integration. Pair it with a high-sample reference of the same quantity.
  - **Compare** three filters on one process and one asset load:
    - (a) world-cell accumulation with event-driven W;
    - (b) SVGF semantics;
    - (c) a recurrent blur with history fix.
  - **Measure** error by age after a cut, a disocclusion and a 5° sun step, plus frames to ≤ the bar, temporal flicker and blur at contacts.
  - **Price only the winner on the GPU.**
  - **Stop rules:**
    - drop a filter that cannot reach the bar by frame 8 after each event;
    - drop a filter whose GPU estimate exceeds half of Lumen's measured denoise share (to be taken from ProfileGPU in the matched Unreal run);
    - if none passes, record the miss and move effort to more samples or better caches, not to a blurrier filter.

**Risks.**

1. **Patents.** US 10,116,916 (variance clip), US 10,600,167 (SVGF), US 12,437,371 (temporal gradients), US 11,847,737 (fast history), plus the §2 history-clamp and ReSTIR filings. Each needs a review before shipping.
2. **The R4 lobe contract.** Specular denoising needs a program-declared lobe. Programs without one lose glossy filtering.
3. **Non-RT hitT.** Specular parallax without hit distance may ghost on moving glossy surfaces.
4. **TSR measurement fairness.** Async overlap and the 200% history memory, which counts toward Unreal's VRAM total, must be reported as measured, not modelled.
5. **Structural shadow-edge blur.** T1b may beat TSR on every population yet remain below film at hard shadow edges. That is allowed under the floor, but it must stay labelled.
6. **Cold surfaces after disocclusion.** World caches help only with surfaces seen before. Never-seen ones need the revealed-pixel boost.
7. **Time-dependent programs.** W = 1 forfeits reconstruction on animated materials until programs declare a bandwidth.
8. **Determinism.** Any atomic-based cache insert must keep N8o's claim/resolve identity.

## Sources

Primary repositories and license files read: [FidelityFX-FSR2 LICENSE.txt](https://github.com/GPUOpen-Effects/FidelityFX-FSR2/blob/master/LICENSE.txt), [FidelityFX-SDK docs/license.md](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/main/docs/license.md), [FidelityFX-SDK v1.1.4 LICENSE.txt](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/v1.1.4/LICENSE.txt), [FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser), [NRD LICENSE.txt](https://github.com/NVIDIA-RTX/NRD/blob/master/LICENSE.txt), [RTXDI LICENSE.txt](https://github.com/NVIDIA-RTX/RTXDI/blob/main/LICENSE.txt), [STBN License.txt](https://github.com/NVIDIA-RTX/STBN/blob/main/License.txt), [XeSS LICENSE.txt](https://github.com/intel/xess/blob/main/LICENSE.txt), [CMAA2](https://github.com/GameTechDev/CMAA2), [OIDN](https://github.com/RenderKit/oidn), [Q2RTX license.txt](https://github.com/NVIDIA/Q2RTX/blob/master/license.txt), [compute_rasterizer LICENSE.md](https://github.com/m-schuetz/compute_rasterizer/blob/master/LICENSE.md), [Falcor LICENSE.md](https://github.com/NVIDIAGameWorks/Falcor/blob/master/LICENSE.md), [Area-ReSTIR](https://github.com/guiqi134/Area-ReSTIR), [Capsaicin](https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin), [Bevy](https://github.com/bevyengine/bevy), [EA fastnoise](https://github.com/electronicarts/fastnoise), [iryoku/smaa](https://github.com/iryoku/smaa), and `wgpu-types-30.0.1/src/features.rs` (local cargo registry).

Papers and talks: [Schütz 2021](https://arxiv.org/abs/2104.07526), [Schütz 2022](https://arxiv.org/abs/2204.01287), [A-SVGF](https://cg.ivd.kit.edu/publications/2018/adaptive_temporal_filtering/adaptive_temporal_filtering.pdf), [Area ReSTIR](https://graphics.cs.utah.edu/research/projects/area-restir/), [ATAA chapter](https://casual-effects.com/research/Marrs2019ATAAGem/Marrs2019ATAAGem.pdf), [ATAA HPG](https://research.nvidia.com/publication/2018-08_Adaptive-Temporal-Antialiasing), [Amortized supersampling](https://www.cse.ust.hk/~psander/docs/amss.pdf), [render cache](http://www-sop.inria.fr/reves/Basilic/1999/WDP99/), [TRMV](https://sites.cs.ucsb.edu/~lingqi/publications/paper_trmv.pdf), [AGAA](https://research.nvidia.com/sites/default/files/pubs/2015-02_Aggregate-G-Buffer-Anti-Aliasing/AGAA_I3D2015_authors.pdf), [DCAA](https://research.nvidia.com/sites/default/files/pubs/2015-08_Decoupled-Coverage-Anti-Aliasing/hpg15_DCAA.pdf), [Heitz 2018](https://eheitzresearch.wordpress.com/705-2/), [HRAA](https://michaldrobot.com/2014/08/13/hraa-siggraph-2014-slides-available/), [Filmic SMAA](https://advances.realtimerendering.com/s2016/), [Epic TSR docs](https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-in-unreal-engine?lang=en-US), [TSR thin geometry](https://dev.epicgames.com/documentation/unreal-engine/thin-geometry-detection-with-temporal-super-resolution?lang=en-US), [TSR forum thread](https://forums.unrealengine.com/t/tsr-feedback-thread/883977/20), [Solari 0.18 post](https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18/), [Wyman patent list](https://cwyman.org/paperList.html), and [US 11,315,310](https://patents.justia.com/patent/11315310).

**2026-11-07 additions.**

- **Licence files and repositories read:**
  - [NVIDIA/DLSS LICENSE.txt](https://github.com/NVIDIA/DLSS/blob/main/LICENSE.txt) and the [DLSS Programming Guide 310.6.0](https://github.com/NVIDIA/DLSS/blob/main/doc/DLSS_Programming_Guide_Release.pdf);
  - [bevyengine/dlss_wgpu LICENSE-MIT](https://github.com/bevyengine/dlss_wgpu);
  - [FidelityFX SDK docs/license.md](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/main/docs/license.md) at v2.4.0;
  - [FSR 3.1.4 upscaler docs](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-upscaler/) and the [FSR 2 README](https://github.com/GPUOpen-Effects/FidelityFX-FSR2);
  - [FidelityFX-Denoiser LICENSE.txt](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser/blob/master/LICENSE.txt) and the [v1.1.4 denoiser docs](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/v1.1.4/docs/techniques/denoiser.md);
  - the [NRD README](https://github.com/NVIDIA-RTX/NRD), `Include/NRDSettings.h` and `Include/NRDDescs.h`;
  - [Falcor SVGFPass](https://github.com/NVIDIAGameWorks/Falcor/tree/master/Source/RenderPasses/SVGFPass) file headers;
  - [kajiya](https://github.com/EmbarkStudios/kajiya): LICENSE-MIT, LICENSE-APACHE, the README and `assets/shaders/rtdgi/`;
  - [OIDN LICENSE.txt and README](https://github.com/RenderKit/oidn);
  - [intel/xess LICENSE.txt](https://github.com/intel/xess/blob/main/LICENSE.txt);
  - [balintio/nppd](https://github.com/balintio/nppd), [xmeng525/RealTimeDenoisingNeuralBilateralGrid](https://github.com/xmeng525/RealTimeDenoisingNeuralBilateralGrid) and [gtong-nv/BMFR-DXR-Denoiser](https://github.com/gtong-nv/BMFR-DXR-Denoiser), via the GitHub licence API;
  - `wgpu-types-30.0.1/src/features.rs` (`EXPERIMENTAL_COOPERATIVE_MATRIX`).
- **Papers and talks:**
  - [SVGF preprint](https://research.nvidia.com/sites/default/files/pubs/2017-07_Spatiotemporal-Variance-Guided-Filtering%3A//svgf_preprint.pdf) and [A-SVGF](https://cg.ivd.kit.edu/publications/2018/adaptive_temporal_filtering/adaptive_temporal_filtering.pdf);
  - [Yang, Liu, Salvi 2020 TAA survey](http://behindthepixels.io/assets/files/TemporalAA.pdf);
  - [Karis 2014 slides](https://de45xmedrsdbp.cloudfront.net/Resources/files/TemporalAA_small-59732822.pdf) and [Jimenez 2017 slides](https://www.activision.com/cdn/research/Dynamic_Temporal_Antialiasing_and_Upsampling_in_Call_of_Duty_v4.pdf);
  - [ReBLUR, Ray Tracing Gems II ch. 49](https://link.springer.com/chapter/10.1007/978-1-4842-7185-8_49), cited but not read in full.
- **Epic documentation:**
  - [TSR](https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-in-unreal-engine?lang=en-US) (5.8 docs);
  - [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine) and the [Lumen performance guide](https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine);
  - [Anti-aliasing and upscaling](https://dev.epicgames.com/documentation/unreal-engine/anti-aliasing-and-upscaling-in-unreal-engine);
  - [TSR RejectShading forum profile](https://forums.unrealengine.com/t/understanding-tsr-rejectshading-gpu-cost/2598124).
- **Installed UE 5.7.4:** `Engine/Config/BaseScalability.ini`, and cvar names and help strings from `UnrealEditor-Renderer.dll`, read as text.
- **Bevy:** [Bevy 0.20 notes](https://bevy.org/news/bevy-0-20/).
- **Patents (Google Patents):** [US 10,116,916](https://patents.google.com/patent/US10116916B2/en), [US 10,600,167](https://patents.google.com/patent/US10600167B2/en), [US 12,437,371](https://patents.google.com/patent/US12437371B2/en), [US 11,847,737](https://patents.google.com/patent/US11847737) and [US 12,374,006](https://patents.google.com/patent/US12374006B2/en). The last is Intel's neural TAA history-validation patent, active to 2043. It was found but is not relevant unless a neural validator is considered.
- **Press, secondary:** the FSR 4 source episode, via [VideoCardz](https://videocardz.com/newz/amd-releases-fidelityfx-sdk-2-0) and [Guru3D](https://www.guru3d.com/story/amd-accidentally-opensources-fsr-4-code-on-github/); the OIDN 1080p report in the [blendity PR](https://github.com/EmilianoManaloIV/blendity/pull/85).
