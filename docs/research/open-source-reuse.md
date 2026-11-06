# Open-source reuse: what to take, in order

Research date 2026-11-06, base `3df2e48`. Web and source reading only: no build, GPU run or prototype. This page ranks the findings of three detailed surveys against Renderlab's current open problems:

- [reuse-non-rt.md](reuse-non-rt.md): virtual geometry, non-RT shadows, software ray tracing, software GI.
- [reuse-temporal-denoise.md](reuse-temporal-denoise.md): temporal reconstruction, motion AA, denoisers, ReSTIR.
- [reuse-engines-precision.md](reuse-engines-precision.md): same-stack engines, edge-hit and self-intersection robustness, shader-iteration speed, Unreal talks.

Every licence below was read from the project's own LICENSE file; the surveys cite each one. Estimates are labelled **I**.

## Licence rule

- **Copy with notice:** MIT, Apache-2.0, BSD, zlib, CC0. Keep the copyright notice in the file and add it to a third-party notices list. MIT-or-Apache projects: take MIT.
- **Reimplement from the paper only:** GPL (Renderlab's licence is undecided), CC BY-NC(-ND) book chapters, and source-available or proprietary SDKs.
- **Never vendor:**
  - **NVIDIA RTX SDKs licence:** RTXGI-DDGI, RTXDI, NRD, RTXPT. §4(e) forbids any use that makes them subject to an open-source licence.
  - **AMD FidelityFX SDK 2.x:** binary-only except the files it lists as MIT. v1.1.4 and the standalone FSR 2 and denoiser repos are MIT.
  - **Unreal source:** EULA. Talks and docs are fine for algorithms.
  - **Unity HDRP:** Unity Companion Licence.
  - **SpartanEngine:** current versions are non-commercial; earlier MIT copies stay MIT.
  - **NVIDIA spatiotemporal blue noise:** non-commercial. EA FAST is BSD-3.
- **Patents:** NVIDIA holds many ReSTIR-family patents, and US 10,600,167 is likely SVGF. Get a patent review before shipping ReSTIR or SVGF-derived work. The surveys found no concrete patents on the other recommended items, but that is not a freedom-to-operate check.

## Ranked takes against open problems

| # | Problem | Take | Licence | Why first | Risk |
|---|---|---|---|---|---|
| 1 | Non-RT **N0** software traversal ([non-rt-path.md](../non-rt-path.md)) | **obvhs** crate: PLOC builder, compressed 8-wide BVH collapse, CPU traversal oracle | MIT/Apache | Replaces N0's from-scratch binned-SAH builder and satisfies its "third-party builder needs a licence check" note | Leaf layout may copy triangles, against N0's no-copy goal |
| 2 | N0 kernel | **tray_racing** `src/rt_gpu/rt_gpu_software*.hlsl` ported to WGSL; **tinybvh** `isoccluded_cwbvh` loop for any-hit shadows | MIT/Apache; MIT | Already driven from wgpu with a hardware comparison harness. tinybvh's "AlanWBFT" credit needs checking before verbatim copy | HLSL→WGSL: no `bfind`/`popc`, stack spills |
| 3 | N0 stop rules | tray_racing's published **RTX 4070 Ti** chart: software 2.75 ms vs Vulkan hardware RT 0.51 ms, 1080p, 1 primary + 1 unbounded AO ray, 8-scene average = **5.4×** | — | The only measured software/hardware ratio on a 48 MB-L2 Ada card. N0 assumes 2–5×, so this is at the top of that range. (I): software primaries ≈ 1.2 ms > 0.40 ms, which predicts **N1b is needed**; software 8 rays ≈ 0.65 ms, under the 1.0 ms stop | Different workload (unbounded AO rays, not sun shadows) |
| 4 | **N1b** culled raster visibility buffer | **Bevy** `bevy_pbr/src/meshlet`: two-pass HZB culling, software raster, 64-bit atomic visibility buffer, in WGSL on wgpu. Pair it with meshoptimizer `clodBuildHierarchy` (already a dependency) and AMD SPD (MIT) for the depth pyramid | MIT/Apache | The only production wgpu/WGSL implementation found. A raster win also transfers to the RT path's 0.30 ms primaries | Bevy's raster lacks a top-left fill rule. Shared-edge pixels must be classified as ties against the traced hit records |
| 5 | Edge-hit fix at +5% Interactive | **Back-face re-trace.** All three witness hits the hardware returned are back faces. When the committed primary is back-facing and within an error bound of an edge, re-trace once with `CULL_BACK_FACING` and `tmax = t + bound`, and accept only if within the bound | Algorithm | Needs no 156 MB adjacency table and no cold pass for ~20k pixels. Also covers neighbours that share no indices | Unmeasured. Scan winding may be inconsistent. MegaLights reports ~10% traversal cost for face culling, so keep it on the rare path only |
| 6 | Concave-fold false shadows | **Direction-only fold-safe offset:** offset along a direction away from both faces (bisector or area-weighted vertex normal), keeping the adopted 10 µm magnitude. Only folds sharper than 90° can cross. NVIDIA's BSD-3 `safeSpawnOffset` measures the true error bound (≈0.6–1.2 µm at the witnesses) | Algorithm; BSD-3 | Could replace the concave inset without changing policy magnitude or re-baselining gates | Changing the magnitude itself re-baselines every gate and needs Islam |
| 7 | Revealed pixels and selective film quality | **ATAA** (Marrs et al. 2018): supersample only where reuse fails, with compacted ray lists (13–29% faster in its scene) | Algorithm | The only published "pay only where history fails" pattern. Fits T1a's static-only status and future motion work | Doesn't by itself make motion meet the film bar |
| 8 | Shadow-ray sampling | **EA FAST** blue-noise / spatiotemporal sampling | BSD-3 | Drop-in for stratifying 8-ray sequences across frames; replaces NVIDIA's non-commercial STBN | Must keep the frozen direction sequences byte-stable where gates depend on them |
| 9 | Specular speckle on rough lit faces | **Filament** specular AA (`normalFiltering`, Tokuyoshi/Kaplanyan), fed by ray-differential normal variance | Apache-2.0 | Small, well-understood, works on both backends | Must pass the film gate; it's a filter, not more samples |
| 10 | Shader edit-to-image (100 ms target) | Move environment code out of each user program's pipeline (a flat program compiles in 22–58 ms); probe NVIDIA compile time with optimization disabled | — | wgpu 30 has no pipeline libraries or shader objects. Bevy is behind us here, so there is no code to take | May need a wgpu-hal patch |

Copy-safe extras for later: Bevy's Hillaire 2020 atmosphere LUTs and Tony McMapface tonemapping (MIT/Apache; Bevy's AgX LUT source has no licence file). Use pbrt-v4 (Apache), Embree (Apache) and Mitsuba 3 (BSD) as independent CPU oracles and path-traced references.

## Already settled; don't redo

- **Scatter/atomic T2.** The temporal survey's top idea was 64-bit atomic forward splatting. Main already tried it: **T2b scatter stopped** at K=8 moving 4.25 ms (`9828f51`, [m6-motion-aa-gpu.md §9](../m6-motion-aa-gpu.md)). Motion stays Interactive 1x, labelled below the film bar.
- **TAA/TSR-family reconstruction.** FSR 2/3, Bevy TAA/SMAA, CMAA2: read only. They resample history or filter one sample. No published technique reaches film-quality AA in motion within 0.2–0.3 ms. Epic's own TSR docs say interpolating history "introduces blur". Nothing public covers hard shadow edges, so only a TSR-on Unreal capture can settle the T1 leak inference.
- **Non-RT core shadows other than rays.** VSM/SMRT, moment maps, PCSS, SDSM, distance fields: rejected in [non-rt-path.md](../non-rt-path.md). Frustum-traced irregular z-buffers are exact but measured 20–32 ms on a 12.3M-triangle scene (GTX 980).
- **Screen-fed GI as core lighting.** Lumen screen traces, Godot SDFGI, AMD GI-1.x, Brixelizer GI's screen caches, EA GIBS. Brixelizer (MIT, pin SDK v1.1.4) is acceptable only as a far-field proxy.

## Worth a probe, not a take

- **Tiled per-triangle soft shadow volumes** (Kalbertodt gist, no licence: reimplement only). The author reports ~13M triangles, 208 sun-disk directions per pixel on an RTX 4070 with wgpu. It might bring non-RT Exact from an estimated 15–50 ms to a few ms. It assumes closed meshes and discards light-facing triangles; the quarry scans are open surfaces. A CPU probe on one scan decides.
- **One shader, two backends** for future GI: Wicked Engine (MIT) compiles one DDGI ray shader for hardware and compute traversal. This matches the non-RT plan's contract. DDGI alone is low-frequency and won't meet the film gate near contacts.

## User-suggested repos (checked 2026-11-06)

| Repo | Licence | Verdict | Relevance |
|---|---|---|---|
| [NVIDIAGameWorks/RTXGI-DDGI](https://github.com/NVIDIAGameWorks/RTXGI-DDGI) | NVIDIA RTX SDKs licence | **Avoid code**; reimplement DDGI from Majercik et al. | Needs hardware RT for probe updates; the non-RT story is only "bake and load static" |
| [EmbarkStudios/kajiya](https://github.com/EmbarkStudios/kajiya) | MIT/Apache | Copy for later GI | ReSTIR GI, irradiance cache, RT shadow denoiser (see engines survey). Hardware RT only |
| [Patryk27/strolle](https://github.com/Patryk27/strolle) | MIT (archived 2025-04) | Copy, as a reference | Rust/wgpu ReSTIR DI/GI + SVGF with software BVH traversal for GPUs without RT cores. Shaders are rust-gpu, not WGSL. No published performance. Patent note on ReSTIR/SVGF applies |
| [pettett/multires](https://github.com/pettett/multires) | **GPL-3.0** | Reimplement only | Rust/Vulkan cluster DAG with METIS, own QEM vs meshoptimizer, "adaptive select" re-testing last frame's clusters. Dissertation at pettett.github.io/projects/meshes/main.pdf |
| [owenyuwono/minos](https://github.com/owenyuwono/minos) | MIT | Little to take | Rust + WGSL via naga. Its Nanite module is feature-gated and unplugged. Camera-relative f64→f32 transforms are a small relevant pattern |
| [PanosK92/SpartanEngine](https://github.com/PanosK92/SpartanEngine) | Spartan Engine Licence 1.0 (non-commercial) | Avoid current code | GPU-driven layout reference: meshlet frustum/Hi-Z/cone culling, one indirect draw per pass. Shadows are maps + screen-space, below our bar |
| [Friduric/voxel-cone-tracing](https://github.com/Friduric/voxel-cone-tracing) | MIT | Low value | 2011 VCT in OpenGL 4.4; leak and blur are far from the film gate |

## Correction found

[milestones-5-6-decisions.tsv](../milestones-5-6-decisions.tsv) says the adopted viewed-side receiver rule "matches Cycles and UE path tracer". Cycles does not offset by default. It starts rays on the exact surface point, skips its own primitive with any-hit, and applies the Wächter–Binder offset only when a re-test against its own triangle misses (`integrate_surface_ray_offset`, `kernel/bvh/util.h`). Details are in the [engines survey](reuse-engines-precision.md). The decision itself doesn't depend on that comparison, but the record should be corrected.
