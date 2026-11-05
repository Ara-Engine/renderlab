# Reusable temporal reconstruction, denoising and sampling work for motion AA

Research date: 2026-11-06. Read-only: public web sources, GitHub license files read via the API or raw files, and the local docs [m6-motion-aa.md](../m6-motion-aa.md), [m6-motion-aa-gpu.md](../m6-motion-aa-gpu.md), [m6-temporal-primaries.md](../m6-temporal-primaries.md) and [m6-film-gap.md](../m6-film-gap.md). There were no builds, GPU runs or commits. This is not legal advice. Licence verdicts read the LICENSE file in each repository on the research date. Patent notes list only what was found and do not analyse claims.

**Labels.** **V** means verified at the cited URL or file. **Inference** is my reasoning, not a published result. Author-reported numbers keep the authors' hardware and resolution. They are not Renderlab measurements.

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
| AMD FSR 2 (`GPUOpen-Effects/FidelityFX-FSR2`) | MIT (LICENSE.txt, v2.2) | **COPY** selected pieces | `ffx_fsr2_reconstruct_dilated_velocity_and_previous_depth.h`: a forward scatter of depth with `InterlockedMax`. The reactive-mask *interface* idea | Forward depth splat. Programmable "reactivity" declaration |
| AMD FSR 3 upscaler (FidelityFX SDK v1.1.4 is all MIT. In v2.x the `upscalers/fsr3/include/gpu/...` files are MIT and the rest is binary-only) | MIT for those files | COPY possible, but it is **AVOID as the core** | Same class as T1 (resampled history plus clamping and locks) | Reference only |
| AMD FSR 4 (ML) | FidelityFX SDK 2.x default license: binary-only, no reverse engineering | **AVOID** | — | — |
| Unreal TSR | Public docs only. UE source is under the Unreal EULA | **REIMPLEMENT-ONLY** (ideas from public docs) | Public statements that support (c). History resurrection idea | (c) |
| Intel XeSS (`intel/xess`) | Intel Simplified Software License: binary only, no modification | **AVOID** | — | — |
| Bevy TAA (`crates/bevy_anti_alias/src/taa/taa.wesl`) | MIT OR Apache-2.0 | COPY possible. **AVOID as the core** | Catmull-Rom plus YCoCg variance clip, which is the T1 class | Reference only |
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
| AMD FidelityFX Shadow Denoiser (`GPUOpen-Effects/FidelityFX-Denoiser/ffx-shadows-dnsr/*.h`) | MIT | COPY possible. **AVOID for the film path** | Tile classification over 8×4 bitmasks | Cross-pixel filter. Not the gap |
| NVIDIA NRD (SIGMA, ReBLUR, ReLAX) | **NVIDIA RTX SDKs License**: object-code distribution only, no use that makes it open source | **REIMPLEMENT-ONLY** (papers/talks) | — | Not the gap |
| SVGF (Schied 2017). A-SVGF (2018) | Papers. Falcor SVGF pass is BSD-3. Q2RTX is GPL-2 | Falcor: COPY possible. Low value | Temporal-gradient idea for fast history drop | Light-change reaction |
| Heitz et al. 2018 ratio estimator | Paper | REIMPLEMENT-ONLY. Conflicts with programmability | — | — |
| NVIDIA STBN (`NVIDIA-RTX/STBN`) | **Non-Commercial Use License** | **AVOID** | — | — |
| **EA FAST noise** (`electronicarts/fastnoise`) | BSD-3-Clause | **COPY** (attribution) | Spatiotemporal blue-noise textures and generator | Stratifying the 8 shadow rays and jitter across frames |
| Intel OIDN (`RenderKit/oidn`) | Apache-2.0 | COPY possible. **AVOID in real time** | Offline/reference denoising only. No Vulkan compute backend | Reference generation at most |
| ReSTIR DI (Bitterli 2020). NVIDIA RTXDI (`NVIDIA-RTX/RTXDI`) | Paper. RTXDI is under the RTX SDKs License. NVIDIA holds ReSTIR-family patents | REIMPLEMENT-ONLY. Low value now | One sun, so little to gain | Patent risk noted |
| AMD Capsaicin (`GPUOpen-LibrariesAndSDKs/Capsaicin`) | MIT | COPY possible. Later | GI-1.0 (`gi1/world_space_restir.hlsl`, `hash_grid_cache.hlsl`) | Future GI, not AA |
| Bevy Solari (`crates/bevy_solari/src/realtime/restir.wesl`) | MIT OR Apache-2.0 | COPY possible. Later | WESL ReSTIR on wgpu ray queries | Future many-light/GI. RT only; relies on DLSS-RR |

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

## Sources

Primary repositories and license files read: [FidelityFX-FSR2 LICENSE.txt](https://github.com/GPUOpen-Effects/FidelityFX-FSR2/blob/master/LICENSE.txt), [FidelityFX-SDK docs/license.md](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/main/docs/license.md), [FidelityFX-SDK v1.1.4 LICENSE.txt](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/v1.1.4/LICENSE.txt), [FidelityFX-Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser), [NRD LICENSE.txt](https://github.com/NVIDIA-RTX/NRD/blob/master/LICENSE.txt), [RTXDI LICENSE.txt](https://github.com/NVIDIA-RTX/RTXDI/blob/main/LICENSE.txt), [STBN License.txt](https://github.com/NVIDIA-RTX/STBN/blob/main/License.txt), [XeSS LICENSE.txt](https://github.com/intel/xess/blob/main/LICENSE.txt), [CMAA2](https://github.com/GameTechDev/CMAA2), [OIDN](https://github.com/RenderKit/oidn), [Q2RTX license.txt](https://github.com/NVIDIA/Q2RTX/blob/master/license.txt), [compute_rasterizer LICENSE.md](https://github.com/m-schuetz/compute_rasterizer/blob/master/LICENSE.md), [Falcor LICENSE.md](https://github.com/NVIDIAGameWorks/Falcor/blob/master/LICENSE.md), [Area-ReSTIR](https://github.com/guiqi134/Area-ReSTIR), [Capsaicin](https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin), [Bevy](https://github.com/bevyengine/bevy), [EA fastnoise](https://github.com/electronicarts/fastnoise), [iryoku/smaa](https://github.com/iryoku/smaa), and `wgpu-types-30.0.1/src/features.rs` (local cargo registry).

Papers and talks: [Schütz 2021](https://arxiv.org/abs/2104.07526), [Schütz 2022](https://arxiv.org/abs/2204.01287), [A-SVGF](https://cg.ivd.kit.edu/publications/2018/adaptive_temporal_filtering/adaptive_temporal_filtering.pdf), [Area ReSTIR](https://graphics.cs.utah.edu/research/projects/area-restir/), [ATAA chapter](https://casual-effects.com/research/Marrs2019ATAAGem/Marrs2019ATAAGem.pdf), [ATAA HPG](https://research.nvidia.com/publication/2018-08_Adaptive-Temporal-Antialiasing), [Amortized supersampling](https://www.cse.ust.hk/~psander/docs/amss.pdf), [render cache](http://www-sop.inria.fr/reves/Basilic/1999/WDP99/), [TRMV](https://sites.cs.ucsb.edu/~lingqi/publications/paper_trmv.pdf), [AGAA](https://research.nvidia.com/sites/default/files/pubs/2015-02_Aggregate-G-Buffer-Anti-Aliasing/AGAA_I3D2015_authors.pdf), [DCAA](https://research.nvidia.com/sites/default/files/pubs/2015-08_Decoupled-Coverage-Anti-Aliasing/hpg15_DCAA.pdf), [Heitz 2018](https://eheitzresearch.wordpress.com/705-2/), [HRAA](https://michaldrobot.com/2014/08/13/hraa-siggraph-2014-slides-available/), [Filmic SMAA](https://advances.realtimerendering.com/s2016/), [Epic TSR docs](https://dev.epicgames.com/documentation/unreal-engine/temporal-super-resolution-in-unreal-engine?lang=en-US), [TSR thin geometry](https://dev.epicgames.com/documentation/unreal-engine/thin-geometry-detection-with-temporal-super-resolution?lang=en-US), [TSR forum thread](https://forums.unrealengine.com/t/tsr-feedback-thread/883977/20), [Solari 0.18 post](https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18/), [Wyman patent list](https://cwyman.org/paperList.html), and [US 11,315,310](https://patents.justia.com/patent/11315310).
