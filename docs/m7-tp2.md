# M7-TP2: glass lighting, sun transmission and edge coverage

Follow-up to [TP1](m7-tp1.md), branch `t3/m7-tp2`, under the 2026-11-08 overnight rules. Implementation and verification are limited to this checkpoint, with remaining gaps recorded below. No Unreal or path-traced campaign is part of this unit.

## Evidence and cost

Evidence lives in `D:/Coding/renderlab-work/m7-tp2/`. **Retained and verified on both paths at native scale.** Validated renderer source: `eb0f4a2` (`b12`, clean source diff); later edits are documentation and the GPU queue guard only. The test binary's SHA-256 and build identity are in `build-b12.json`.

Integration boundary: repeatedly merged main through `1cd8438` (MP1/MQ1, TP1, G2c, RTDEF2 and VF1 included). Later AT2/LC2 and R1n work are outside this verified source and need the coordinator's combined merge gate. No R1n glass integration is claimed.

**Cause measured, fix retained:** the expensive cache search at glass receivers, which create no cache cells. Glass now reads the open environment instead. RT keeps its original hardware sun ray and atmosphere; the smaller sun-map saving did not justify changing its visibility quality.

P6, binary `b11` (`0f11b99`), RTX 4070 SUPER, NVIDIA 617.14, Vulkan/wgpu 30.0.1, quarry pose 0, 1920×1080 native, 1x primaries, TR1/GI/atmosphere/post defaults. This was a functional lease with foreign compilers present, **not a QUIET performance certification**. One process loaded the asset once. GPU render-span medians, 128 frames per row and configuration:

| Transparent shader | Foliage + particles | + glass | Glass increment |
|---|---:|---:|---:|
| TP1 lighting: cache search + RT sun | 4.483 ms | 5.606 ms | **1.123 ms** |
| Open environment, RT sun (retained) | 4.418 | 4.591 | **0.173** |
| Cache search, mapped sun | 4.532 | 5.518 | 0.986 |
| Open environment, mapped sun | 4.406 | 4.543 | 0.137 |
| Open environment, mapped sun, no atmosphere | 4.405 | 4.549 | 0.143 |

The cache change removes about **0.95 ms** of the measured increment. Atmosphere was not the regression. These rows isolate the fix; they do not include TP2's new reflection, coloured-shadow and AA costs. Raw data: `runs/p6/0-rt-all/rt-tp1.json`.

The profiler loads the quarry once, keeps the foliage and emitters, and switches only the transparent shader. It compares the full frame with foliage plus particles in eight interleaved 48-frame blocks, retaining the last 16 frames of each block (128 frames per configuration). Its five variants isolate the radiance-cache lookup, hardware sun visibility and atmosphere. These diagnostic variants disable TP2 reflections, coloured shadows and coverage AA, so those features cannot contaminate attribution. All test binaries and GPU work run inside `scripts/n8-run.py` and `D:/Coding/renderlab-work/gpu-slot.lock`.

### Final feature cost

V1 (`b12`), same hardware/resolution/quality and settled-block protocol, all TP2 features enabled. A functional lease with compiler contention, not a QUIET certification. Each path loads the quarry once for its images and timings. GPU render span, ms:

| Path | Opaque | Foliage | Foliage + particles | Full, including glass | Glass increment |
|---|---:|---:|---:|---:|---:|
| RT traced / Interactive | 4.374 | 4.073 | 4.507 | 4.927 | **0.421** |
| Non-RT visibility buffer / Virtual | 3.724 | 3.865 | 4.310 | 4.433 | **0.123** |

The RT foliage row running faster than opaque is noise, not a speedup claim. Glass increments are within-phase differences; particle increments across phases remain indicative. The RT increment includes traced reflections, coloured sun attenuation and coverage AA, whereas P6's 0.173 ms isolates the cache fix. The map is cached; moving-sun/map-update cost and cold reflection-pipeline compilation are not priced by these steady-state rows. Raw data: `runs/v1/{3-rt-final/rt,4-nonrt-final/nonrt}-tp1.json`.

### Verification and inspected images

- **Lib suite:** 184 passed, 0 failed, 156 ignored, inside V1's n8 lease. This includes all TP2 shader modules, standard and toon reflected-hit programs, post primary-frame ABI/span, SPIR-V ray-query-null and MP1 list-composition checks.
- **Opaque control:** 0 differing pixels on each path for the in-process pre-TP1 standard-program control (GI/TR1 disabled for that identity pair).
- **Tinted fixture:** `runs/v1/1-rt-final/{rt-control,rt-glass}.png` and `2-nonrt-final/{nonrt-control,nonrt-glass}.png`. A green, coverage/Fresnel-filtered sun shadow extends across the ground on both paths. The red foreground target appears in the RT pane's reflection and is absent from the non-RT environment fallback. Pane edges have partial coverage; the opaque card's shadow remains. The nearest-neighbour inspection crop is `inspect/glass-fixture.png`.
- **Quarry, both paths:** full frames at 60 and 240 and the 40-frame pan endpoint in `3-rt-final` and `4-nonrt-final`. Inspected the complete still and pan-end images: rock/ground silhouettes, cutout fern silhouettes/occlusion, the soft smoke column, fire, refraction and pane boundaries remain intact. No obvious new trails or missing surfaces in those images. This is not continuous-motion or exact-repeat certification.
- **Example, both paths:** 48 frames at 1280×720, captures at 0/16/32/47 in `5-rt-final` and `6-nonrt-final`; inspected frame 47, including arch, moving/toon objects, spheres, decals and shadows. RT retains scene reflections; non-RT retains its environment estimate. The example has no transparent content. Its binary uses `0f11b99`, before the glass-only cache-default change; its rendering path is unchanged by that change.
- **Validation:** all seven V1 steps exited 0; no GPU validation fault. The final lease ended at **20:53:18 UTC**, with no owned job or queued launcher left. V1 incorrectly used stale queue authorization and jumped LC2's turn; no further GPU work was taken after V1 completed and the coordinator corrected that mistake. The launcher now checks the latest order as described below.
- **CPU check:** final two-job `cargo clippy --lib --tests` completed successfully; existing warnings remain.

Implementation/verification time: the first feature checkpoint took **22 min** from branch creation (19:17:27–19:39:52 UTC), with compilation overlapping implementation. Main integration, shader seams and profiling then continued; the final cost-fix source was committed at 20:42:45, and its two-job Rust test build took **76 s**. Verification commands occupied about **8.8 min** of leases, including failed integration attempts; the first queued run to final release spanned about **89 min**, mostly shared-GPU waiting and overlapping implementation. These intervals overlap and should not be summed. No new Unreal/reference or exact-repeat campaign was run.

The queue runner now reads the **latest exact `[root order]` line before every lease and subprocess step**. A changed order blocks a new lease; a running batch yields after its current step and saves remaining steps. A release from an older order cannot authorize a newer one. CPU checks of before-release, after-release and changed-order states pass (`order-gate-check/result.json`).

Cleanup gap: automatic approval review rejected the native PowerShell removal of this unit's verified `D:/Coding/renderlab-work/m7-tp2/target` path with “blocked by policy.” The Cargo target and staged `bin` directory remain (2.97 GB combined); the coordinator must remove them after handoff. No raw GPU buffer dumps were produced. JSON/log summaries and inspected PNGs remain under the evidence root.

## Implementation

### Sun transmission on both paths

`src/transparency_shadow.rs` owns an independently cached 1024-square RGBA16F map, with one layer per transparent draw. RGB stores transmitted sun light and alpha stores light-space depth. Each layer renders the nearest transparent sheet, including off-camera instances. A receiver multiplies layers lying between it and the first directional sun, with depth-aware bilinear filtering. Coverage includes material opacity, texture alpha and vertex alpha; tint and Schlick Fresnel filter what remains. Opaque visibility stays on its existing RT or non-RT implementation.

The shared lighting group binds the 96-byte header at 26 and the map at 27. `light_sample` filters sun RGB before the program evaluates its own BRDF; existing programs receive coloured shadows without a new mandatory hook. Unshadowed lights keep their requested behaviour. A changed transparent caster restarts reconstruction and relights GI through the existing lighting-event key. The map rerenders only when its source, sun, geometry, material identities or transforms change.

Particles receive the same coloured sun transmission in their per-particle lighting step. They still do not cast shadows.

### R1 reflections on RT glass

`src/transparency_reflection.rs` adapts R1 instead of duplicating its hit shader. A small raster pass writes the glass program's declared `reflection_lobe`, world position and independent geometric plane. A compute pass runs R1's ray loop on the frame's opaque geometry/material tables and shades reflected hits with the opaque frame's program. The final transparent program reads that exposed incident radiance through `specular_incident`, retaining control over its reflection response and transmittance. Ray offsets and geometric-side rejection remain independent of the program's shading normal.

Mirror lobes use one ray; glossy lobes use R1's four fixed VNDF strata and roughness cutoff. Glass draws after TR1, so glossy strata stay fixed instead of rotating without a history filter. Reflected hits use open-environment indirect inputs, as in R1. The adapter follows the actual primary-frame buffer size rather than freezing that ABI. Non-RT currently uses the environment fallback.

The opaque hit program need not request reflections of its own. In mixed-program frames, cross-program hits use the first opaque program's response and its matching parameter block: MP1's existing fallback, without reading a foreign parameter layout. Fully matching each hit's program needs a per-program hit pass.

### Transparent edge antialiasing

The glass composite draws with four geometric coverage samples. A fullscreen seed copies the exact background into the multisampled attachment; uncovered samples preserve that background, then the attachment resolves into HDR. Shading remains per pixel. This smooths geometric pane edges without temporal history or glass trails. Particles retain their existing pass.

### Shared hooks

The owned files are `transparency_gpu.rs`, `transparency_shadow.rs`, `transparency_reflection.rs`, `transparency_tests.rs`, `transparency.wgsl`, `transparency_shadow.wgsl` and `scripts/m7-tp2-run.py`. Minimal shared hooks, marked TP2, are in `renderer.rs`, `lighting_gpu.rs`, `program.rs`, `program_api.wgsl`, `gi_cache.rs` and `primary_ray_gpu.rs`. The isolated reference probe gets a no-transparent-caster stub. A duplicated R1 ray-query declaration inherited from the PP1/SK1 merge was repaired; subsequent main merges retain exactly one function-scope declaration.

Integration repairs: post-processing now binds the shared primary-frame size (128 bytes after RTDEF2, previously a stale literal 112); PP1's naga check asserts the WGSL ABI span. Reflection-source seams tolerate CRLF checkouts, including MP1's hit-parameter edit.

These are ordinary light-space maps, coverage MSAA and R1's existing per-receiver rays. No reservoir, resampling, spatially shared reflection estimate or new patented GI combination is introduced.

## Remaining gaps

- A shadow-map layer stores the nearest sheet of one draw. Overlapping instances or several sheets in the same draw need separate layers to multiply correctly. The map does not reproduce volumetric absorption, caustics or finite-sun soft penumbras.
- Transmission shadows cover the first directional sun. Local lights, additional directional lights, environment visibility and GI traversal still omit transparent intersections. The map uses the engine material's tint/coverage/IOR, not an arbitrary program's `transmittance` implementation.
- Reflections see opaque geometry; transparent objects are absent from the TLAS. No recursive glass, multiple glass interfaces or rough-glass history/denoiser is provided. RT raster-primary frames and non-RT retain the environment fallback at this checkpoint.
- Cross-program reflected hits retain MP1's response fallback; a hit pass per program remains open. Skinned transparent casters and palette-based invalidation of TP1's auxiliary opaque map are not covered by this unit.
- Coverage AA addresses geometry edges. Texture/shading aliasing, particle AA and transparent depth intersections still need broader treatment. The extra HDR seed/resolve and attachments have a cost.
- TP1's 4x raster and sub-native upscaling omissions, per-object sorting, thin screen-space refraction and full HDR copy per transparent draw remain.
- The engine facade's reserved transparency material slot still needs API1 wiring; the crate's `Renderer`/`MaterialDescriptor` API works. Transparent rendering requests 19 sampled textures per stage; RT reflections also require three colour attachments and 32 bytes per sample. The standard RT/non-RT device configurations request sufficient limits.
- No claim of Unreal parity or superiority is established by this unit.
