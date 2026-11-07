# M7 design: global illumination, reflections and temporal reconstruction

Design worker record, 2026-11-07, base main `281e29f`. Contract: [milestone-7.md](milestone-7.md). This is CPU-only design: no implementation, build, GPU run or Unreal launch.

**Status: draft 1.** The sketches are provisional until the research thread's `docs/research/gi.md`, `docs/research/reflections.md` and the extended `docs/research/reuse-temporal-denoise.md` land. Their plug-in points are marked **[R-gi]**, **[R-refl]** and **[R-temporal]** and listed in [section 11](#11-research-plug-ins). No citation below comes from those notes yet.

Labels: **M** is measured in the cited record, **A** is author-reported, **I** is an inference or estimate.

## Summary

1. **One world radiance cache (WRC) on both paths.** It is engine-owned and built on the N8n/N8o surface-cell pattern. It serves sky visibility, bounce GI and rough reflections. Only the ray producer differs between the paths:
   - **RT:** hardware rays.
   - **Non-RT:** near-field visibility precomputed at cook time, plus software rays that start beyond the near field and run against a coarse cluster-LOD BVH.

   Full-detail software rays are ruled out: N0 measured them at 12-22x the cost of hardware rays (M, [N0](non-rt-n0.md)).
2. **Programs receive incident quantities.**
   - `indirect_irradiance(input, n)` and `indirect_radiance(input, direction, roughness)` are the occluded, local counterparts of today's `environment_irradiance` and `environment_reflection`.
   - There is no bent normal or cone in the ABI ([M5-EV handover](m5-environment-visibility.md)).
   - Secondary hits see a program through an optional `scatter()`/`emission()` pair, evaluated per cache cell. This settles programmable-shading risk R4.
3. **Lighting accumulates in world space, not screen history.** Final-image temporal reconstruction is a separate, style-independent stage. It is held to the TSR floor first and the film target second.
4. **Specular AA comes first.** It keeps normal variance at cook time and filters roughness. It is a small unit that serves both reflections and temporal work, and it folds in [M5 glossy motion](m5-glossy-motion.md).
5. **Largest risk: the non-RT 2x floor.**
   - Today's no-RT quarry frame is 1.55-2.26 ms, against Unreal VSM's 1.61 ms (M).
   - Whether 2x is reachable depends on what Lumen software actually costs on this GPU. So Unreal is measured before any GI design is finalized.
6. **Recommended start: U0 and U1 in parallel.**
   - U0: the Unreal Lumen/TSR configuration study and measurements, the references, the benches, and localizing the glossy disagreement.
   - U1: specular AA.
   - Then U2: sky visibility.

## 1. Scope

**In scope.**
- Sky/environment visibility (M5-EV folds in) and indirect diffuse bounce.
- Glossy and specular scene reflections, and specular anti-aliasing (M5-GM folds in).
- Temporal reconstruction of the final image under camera motion, cuts, disocclusion and lighting changes.

Light injection uses every `Lighting` light, with whatever visibility the engine provides for it. Local-light soft visibility (R5) stays with the shadow hillclimb.

**Out of scope.**
- Transparency, volumes and fog, and the atmosphere/sky model (constant and prepared environments only).
- Larger-than-memory streaming.
- Mid-range and other-vendor GPUs: no other device is available ([decision Non-RT-test-target](milestones-5-6-decisions.tsv)).
- Temporal upscaling, unless Q1 asks for it.

## 2. Starting point

| Item | RT path | Non-RT path | Source |
|---|---|---|---|
| Quarry frame: direct light, environment, shadows | 0.82-0.87 ms (Interactive) | 1.55-1.71 ms static (N8e); camera moving 1.76-1.80 ms (Surface lookup), 2.21-2.26 ms (Exact) | M: [non-rt-path](non-rt-path.md), [N8n](non-rt-n8n.md) |
| Unreal, same subset (Lumen and TSR off) | HRT 1.75 ms | VSM 1.61 ms | M: [end-to-end](quarry-end-to-end-comparison.md) |
| VRAM, reserved against Unreal DXGI | 2,641 against 5,410 MiB | 3,440 against 3,912 MiB | M: [memory M1](memory-m1.md) |
| Primary visibility | traced, 0.30 ms | cluster-LOD raster at 0.35 px, 0.39-0.40 ms | M: [N1c](non-rt-n1c.md) |
| Sky and environment | prepared RLE2 fields, unoccluded (`shaders/programs/standard.wgsl:27-33`) | same | M |
| Reflections | environment field only (split-sum) | same | M |
| Specular AA | none: normal mips are renormalized at cook, and `alpha = max(rough², 1e-4)` is unfiltered | none | M: [M5-GM](m5-glossy-motion.md) |
| Temporal | T1a converges static frames only and resets on every moving frame. T2 costs 3.55-5.76 ms and is stopped | none: the visibility buffer rejects Interactive and Temporal; the M5 raster path has 1x/4x MSAA only | M: [T2a](m6-motion-aa-gpu.md), [N1b](non-rt-n1b.md) |
| Glossy parity with Unreal | 84.75% relative L1 disagreement on a roughness-0.1 metal; UE peaks are 11.5-16.8x weaker; cause unproven | — | M: [setup](unreal-comparison-setup.md) |
| TSR evidence so far | S2 crop at 0.25 px/frame: Renderlab Interactive 1x final HDR RMS .01045, against UE VSM + TSR Epic .01508 | — | M: [bench-motion](bench-motion.md). Shadow-dominated, untextured, BRDF parity not certified |

**Reusable machinery.**
- **Surface cells (N8n/N8o)** are the main reusable piece. They provide:
  - world-space cubes keyed by placement, viewed side and a power-of-two edge, in a 2^20 x 4-slot hash table (32 MiB);
  - deterministic claim/resolve inserts and a flush on reset;
  - exact caster invalidation over a region;
  - sun hysteresis with a lag-bounded refresh (N8e).

  Today the cells hold visibility counts only.
- **Cluster-LOD DAG (N1c).** 214,062 quarry clusters. Every DAG triangle indexes source vertices, and one flat GPU pass (0.044 ms) selects a cut. Per-level caster cuts already exist ([N8c](non-rt-n8c.md)).
- **Virtual clipmap (N8).** Sun-basis pages with two depth layers and triangle ids, requested by receivers. It covers one direction only, so it is not a hemisphere structure.
- **Estimators and gates.** Interactive's deterministic direction sets with per-pixel rotation; T1a's centre-first lattice and hit-record validation; the R_L, R_D and R_F references; Level F gates F1-F10 ([temporal primaries §7](m6-temporal-primaries.md)).
- **Program API.**
  - `light_visibility` is the precedent for engine-computed inputs that are identical for every program.
  - Programs compile into compute modules, without derivatives, and get their footprints from ray differentials.

## 3. Shared architecture and the program ABI

### 3.1 World radiance cache

**Cells.**
- Each cell uses N8n's key, plus a level chosen from distance: the edge is the receiver footprint x 2^k, with a GI resolution bias. Cells are fine at contacts and coarse in the far field.
- The hash layout and claim/resolve are copied as a pattern into new M7 files, not shared with the shadow code ([section 12](#12-ownership-licences-and-patents)).

**Payload (I).**
- Incident radiance: L1 SH in RGB fp16 (24 B), or a 4x4 octahedral fp16 tile (about 128 B) if rough reflections need it **[R-gi]**.
- Geometric normal, age and epochs.
- One ray record per fixed direction (K = 8-16): the hit cell, or "escaped" to the environment, plus the hit distance. That is about 6-8 B per record.

**Transport is cached separately from light (I).**
- Ray records depend only on geometry.
- When the sun turns, a light is edited or a program reloads, hits are relit from their records without re-tracing.
- When a caster moves, only records whose segment crosses its swept bounds are invalidated. This is N8o's exact region test, applied to ray segments.
- This is the cache's main lever against Lumen under moving light. Lumen also amortizes surface-cache relighting, on a budget (A, [industry costs](research/industry-rendering-costs.md)).

**Fill.**
- Visible receivers request cells, as in N8n.
- So do the hits of GI and reflection rays, which gives off-screen surfaces cells of their own. That is the off-screen half of the no-screen-space rule.
- The per-frame refresh budget and age caps are tied to a settle limit (Q5).

**Determinism.** Claim/resolve inserts, fixed direction sets and no reservoir resampling. Replays must be bit-identical (F8).

**Memory (I).** 2^20 cells cost about 135 MiB with K = 12; 2^21 cost about 270 MiB. The total must stay under Unreal's on the matched Lumen configuration (section 7).

### 3.2 Program inputs

| Function | Returns | Replaces in the standard program |
|---|---|---|
| `indirect_irradiance(input, n) -> vec3<f32>` | Incident irradiance in lux, unexposed, around `n` at the engine receiver: occluded environment plus bounce | `environment_irradiance(n)` |
| `indirect_radiance(input, direction, roughness) -> vec3<f32>` | Incident radiance prefiltered over the engine's roughness lobe about `direction`, with RLE2's roughness convention. On the quarry this mainly supplies specular occlusion of the sky | `environment_reflection(direction, roughness)` |

- **Same values for every program.** Both are computed by the engine at its own receiver, so every program sees the same values, as with `light_visibility`. The `environment_*` functions stay for programs that want the raw sky.
- **Programs that call neither pay nothing.** Naga reflection of each visible program's module decides whether the M7 passes run (I). This is a gate (8.4).
- **Rejected inputs:**
  - a bent normal or cone, which bakes in one occlusion model;
  - a scalar sky visibility, for the same reason;
  - screen inputs, which the core rule forbids.
- **Open within the ABI: when `indirect_radiance` traces.**
  - **Inline:** it traces inside the shading pass. This is fully programmable, but divergent.
  - **Declared lobe:** the program declares an optional `fn lobe(s, input) -> Lobe`, which the engine traces in a prior pass. This is cheaper per ray, but it runs `surface()` twice or stores a request.

  Start inline, and move only on a measured cost. Record whichever constraint lands.

### 3.3 Secondary-hit contract (R4)

| Option | Mechanism | Verdict |
|---|---|---|
| (a) Optional `fn scatter(s, input, wi, wo) -> vec3<f32>` and `fn emission(s, input) -> vec3<f32>` | The engine evaluates `surface()` at a cell centre, with a ray-cone footprint. It integrates `scatter` against the cell's incident radiance into one view-independent outgoing radiance | **Recommended for bounces.** Exact for Lambert. Glossy interreflection at secondary hits is approximated: a recorded constraint |
| (b) Engine-fixed Lambert from `MaterialDescriptor` base colour | No program code runs at hits | Rejected: a lighting-model shortcut that ignores program textures |
| (c) Full program evaluation at each ray hit | `surface()` plus a view-dependent response per ray | **Used only at the first reflection hit of low-roughness lobes** (hit lighting). Too costly for GI |

- **Programs without `scatter`.** Their surfaces still occlude but reflect nothing into GI, and a diagnostic reports it. The engine never assumes Lambert silently. A one-line helper, `rl_lambert(albedo)`, makes opting in trivial.
- **Program reload.** A reload bumps that program's epoch and relights its cells without re-tracing.

## 4. GI sketches

| Sketch | Path | Mechanism | Reuses | Main risk | Quarry cost (I) |
|---|---|---|---|---|---|
| **G-R1** Cached gather, hardware rays | RT | Cells trace K rotated deterministic rays. A hit reads the hit cell's outgoing radiance, so multi-bounce comes from feedback; an escaped ray reads the environment. Receivers combine 8 short hardware rays at the receiver (r ≤ R_near ≈ 0.25-1 m, Interactive-style directions) with cell irradiance beyond R_near | Surface-cell pattern, Interactive directions, RLE2, BLAS/TLAS | Near-ray grain in motion; leaks through cells that straddle thin walls (the viewed-side key helps); lag after light changes | Near rays 0.10-0.25 ms, fill 0.05-0.15, gather 0.05-0.10: **0.2-0.5 ms** |
| **G-R2** Per-pixel path tracing, ReSTIR GI and a spatiotemporal denoiser | RT | Reservoir reuse plus a denoiser | kajiya / Solari code (MIT/Apache) | **Patents** (ReSTIR family, e.g. US 11,315,310; SVGF US 10,600,167). Noise and ghosting in motion. kajiya's GI alone is 2.3 ms at 1080p on an RX 6800 XT (A) | **Held:** only after patent review, and only if G-R1 misses quality |
| **G-N1** Precomputed near field plus coarse far-field rays | Non-RT | **Near field:** at cook, per source vertex (or a per-triangle lattice on large triangles), visibility of K = 32-64 fixed directions within R_near, from exact rays against static geometry, plus a mean occluder distance. **Movers:** runtime short software rays against the movers' own small BLASes only; per-direction masks combine by AND. **Far field:** software rays (obvhs builder, tray_racing kernel ported to WGSL) against a BVH over a static coarse DAG cut (0.5-2 cm error), starting at R_near, filling cells | Cluster DAG, cell cache, N8o reach test | Near-field bounce inside crevices is approximated. Cook time, and 50-150 MB of VRAM (about 6.5 M vertices x 8-16 B). Self-hits at the LOD/source boundary at R_near. Software throughput on coarse BVHs is unmeasured. N0's limiter was dense near-surface traversal, which this avoids (I) | Near lookup 0.03-0.08 ms, far fill 0.2-0.6: **0.3-0.8 ms** |
| **G-N2** Lumen-software-style distance fields | Non-RT | Per-mesh SDFs at cook plus a global SDF clipmap, sphere-traced from cells; hit radiance from cells. Brixelizer v1.1.4 (MIT) is the copyable far-field form, with its screen-fed cache replaced by world injection | — | Unreal's own docs say walls under 10 cm and thin features leak (A). Millimetre contacts still need a near field. SDF memory for 13 M scan triangles is unknown | **Fallback far field** if G-N1's coarse rays miss their budget |
| **G-N3** Raster probe captures (DDGI-style) | Non-RT | Low-resolution cube captures per probe with cluster LOD; irradiance plus depth moments | N1c raster | Low frequency and probe leaks; DDGI alone won't meet contacts ([reuse-non-rt L2](research/reuse-non-rt.md)) | Rejected as core; possible far-field irradiance |

**Sky visibility (GI-0) comes first.** It is the same pipeline with bounce off: cells and the near field return occluded environment radiance.
- On the outdoor quarry it is likely the dominant indirect term (I).
- It is verifiable against an environment-only path-traced reference.
- It exercises the cache, the ABI, both producers and invalidation before the R4 contract lands.
- The [M5-EV handover](m5-environment-visibility.md)'s candidates map onto it: its precomputed visibility is G-N1's near field, its surface-cache idea is the WRC, and the single-direction grid and clipmap are excluded.

**Recommendation.**
- G-R1 and G-N1 share everything except the producer.
- If G-N1's near field passes the gates, the RT path may adopt it too. That would remove one estimator difference between the paths.
- G-N2 is the documented far-field fallback, and G-R2 is held.

**[R-gi]** Finalize these items from gi.md:
- the cache layout (kajiya ircache, GI-1.0 hash grid, Solari world cache);
- prior art on invalidation and precomputed visibility;
- any published non-RT GI without screen-space inputs;
- Lumen software and hardware per-pass cost on PC GPUs;
- software ray throughput on coarse BVHs.

## 5. Reflection sketches

**Prerequisites, from [M5-GM](m5-glossy-motion.md).**
- Localize the 84.75% roughness-0.1 disagreement with Unreal (U0c) before any glossy or TSR comparison is scored.
- Land specular AA (U1) before scoring glossy motion.

| Sketch | Path | Mechanism | Main risk |
|---|---|---|---|
| **R-R1** Lobe sub-cone rays plus hit lighting | RT | **Below r_trace** (≈ 0.3-0.4, to be set from **[R-refl]**): split the GGX lobe into K fixed sub-cones (K = 1 at a mirror, up to 8). Trace one hardware ray per sub-cone axis, rotated per pixel like Interactive. **At the hit:** the lowest roughness band gets full program evaluation (3.3c) with a hardware shadow ray plus cell irradiance; other bands read the hit cell's radiance, filtered to the sub-cone footprint. **Above r_trace:** cells only, cone-filtered. Deterministic and history-free | Structured error at mid roughness; a seam at r_trace; divergent shading cost |
| **R-R2** Stochastic GGX at 1 spp plus a temporal/spatial denoiser (Lumen-like) | RT | One sampled ray, then denoise | Ghosting and disocclusion failures of screen-reprojected denoising; SVGF-family patents. **Rejected as core** |
| **R-N1** Segmented-LOD software rays | Non-RT | The same lobes and directions. Each ray traverses a fine cut near its origin, then coarser cuts, switching where the ray-cone width exceeds the cut's error; hit radiance from cells | Mirrors need the fine segment, where N0's near-surface cost applies; cracks and self-hits between segments; 2-3 BVHs of memory |
| **R-N2** SDF/card tracing (Lumen software reflections) | Non-RT | As G-N2 | Mirrors are blurry. Rejected for mirrors; possible for rough far field |
| **R-N3** Planar raster reflections for declared flat mirrors | Non-RT | N1c raster from the mirrored camera | Exact and cheap on planes, but not general. An optional accelerator, never the only answer |

Screen-space reflections are excluded. **Recommendation:** R-R1 plus R-N1, sharing lobes, sub-cone directions and the WRC. Non-RT mirror cost is the open risk; mirrors are rare on the quarry but central on S5.

### Specular AA (U1, both paths)

- **S-AA1: Toksvig/LEAN at cook.**
  - Keep each normal mip's mean length, or its second moments, in `crates/cook/src/normal_map.rs`. This is a versioned format change.
  - The standard program widens alpha through a helper, `filtered_roughness`.
- **S-AA2: geometric variance.**
  - Store a per-vertex, multi-scale normal variance from the cluster DAG, and look it up at the receiver's footprint.
  - This targets sub-pixel triangle relief: 74% of covered quarry pixels have at least 20° of normal spread (M, [film gap](m6-film-gap.md)).
  - Per-triangle curvature filtering (Kaplanyan/Tokuyoshi, Filament's `normalFiltering`) cannot see that spread, because a hit sees only one triangle.
- **Both are offered to programs as geometric data** (`input.normal_variance`), not lighting-model data.
- **Acceptance:** keep either only if Level F film error falls on every population, static and moving. A filter that merely dims highlights fails F3 (bias).

## 6. Temporal reconstruction sketches

**Framing.**
- Lighting accumulates in world-space cells, so screen history only has to reconstruct the final image: coverage, texture and shading aliasing.
- The floor is TSR in motion, on every measure; the target is film (F1-F10).

**Prior results.**
- **T1:** resampled history leaks 0.16-0.24 into solids beside hard shadow edges for 2-4 px (M).
- **T1b:** failed on silhouette trails, shadow-edge blur and R_D F4 (M).
- **T2b:** scatter costs 4.25 ms at K = 8 (M).

Those failures were judged against the film gate; against TSR they are unmeasured. On S2's shadow crop, 1x Interactive already has lower final HDR RMS than UE + TSR (M). That crop does not test textured or glossy aliasing.

| Sketch | Mechanism | Non-RT | Main risk |
|---|---|---|---|
| **T-A** History-free | Specular AA (U1) plus extra primary samples only at listed discontinuities: silhouettes, shadow partials, high normal variance. The list is compacted (ATAA's idea; reimplement from the paper). No history, so no trails, lag or disocclusion failures by construction | Extra samples from a jittered re-raster of the listed clusters only (I) | Interior aliasing above TSR where nothing is flagged; non-RT sample cost |
| **T-B** Hit-validated resampled history (T1 class), reopened against the TSR floor | Exact hit-record tap validation, centre-first lattice, Catmull-Rom, blur budget. No colour clamping (cf. US 11,783,455) | Needs a jittered visibility buffer, which the current one rejects | Known leaks across shadow edges, and trails |
| **T-C** T2 world-anchored records with stratum scatter | As T2b | Same | 4.25 ms measured. **Held** as the film-target reference, not a floor candidate |
| **T-D** Temporal upsampling | T-B, rendered at a lower internal resolution | Same | Only if Q1 sets an upscaled output |

**Recommendation.**
1. U0 measures TSR on S2-glossy and the quarry first: final post-TSR HDR, sharpness, trails, flicker and revealed-pixel recovery. It reuses bench-motion's replay and dump decoder.
2. Then try T-A on RT first: lowest ghosting risk, lowest cost.
3. Try T-B only where T-A leaves interior aliasing above TSR.
4. Non-RT needs a jittered visibility buffer for either; it belongs to U5.

**Measures added from M5-GM.**
- Glossy flicker: the RMS of `(c_t − c_{t−1}) − (r_t − r_{t−1})`.
- Reference-masked highlight energy and peak-position drift.
- A shutter choice per gate. Zero shutter matches Unreal with motion blur off.

**[R-temporal]** Finalize from the extended survey: its TSR findings, any new history-free or adaptive AA results, and its patent notes.

## 7. Measured plan against Unreal 5.7

### Method (reused)

**Unreal.**
- Packaged Development UE 5.7.4 CL51494982, D3D12 SM6, `-RenderOffscreen`, hidden start.
- No focus changes, input automation or desktop capture.
- Native CSV frames 96-159 after warmup; ProfileGPU outside that block.
- DumpGPU replays for images and pass attribution, run separately from timing replays.

**Shared rules.**
- Effective-CVar readback with `SetBy`.
- DXGI memory sampling.
- The same manifests and camera/light tracks for both engines.

**Renderlab.** One process per asset runs every variant with 8 paired repeats. Every speed claim gets a final quiet-window run.

**Content preparation.**
- Software Lumen needs mesh distance fields, and the packaged project has not been recooked since the [full-detail comparison](unreal-quarry-full-detail-comparison.md).
- U0a recooks with distance fields as a new, recorded content identity, and reads back the distance-field resolution of the Nanite scans.
- Hardware Lumen reuses the ray-tracing support the HRT package already has.

**Configuration study first (U0a).**
- Dump the installed build's CVar list and help text, for configuration only. No engine source is copied.
- Map each configuration below to its effective settings, and confirm every one by readback.
- No CVar name is assumed from documentation. Whether 5.7.4 still has standalone RT AO or sky-light controls is unchecked.

### Configurations

| ID | GI and reflections | Shadows | AA | Role |
|---|---|---|---|---|
| UE-0 | Off (the existing subset) | VSM / HRT | Off | Baseline. Lumen's increment is UE-x minus UE-0 |
| UE-TSR | Off | VSM / HRT | TSR, native 100%, Epic, 200% history | Prices TSR alone; U5's floor |
| UE-SW-H, UE-SW-E | Lumen software (distance fields), High and Epic scalability | VSM | TSR as above | **Non-RT comparator** |
| UE-HW-E | Lumen hardware with surface-cache hit lighting, Epic | VSM, and an HRT variant | TSR | **RT comparator** |
| UE-HW-HL | Lumen hardware with hit lighting for reflections | As UE-HW-E | TSR | Strongest RT reflection comparator |
| UE-PT | Path tracer, high spp | — | — | Diagnostic cross-check of our references only, never the sole reference |
| UE-up | As UE-SW / UE-HW | | TSR upscaling at the agreed screen percentage | Only if Q1 asks for it |

**Which configuration is the floor.** The floor is the best-quality configuration per path that runs at game rates. That is probably Epic; High is kept for the cost curve. "Matched quality" means Renderlab's image beats that configuration on every registered measure, and the 2x floor is then measured against that configuration's end-to-end time.

**Workloads.**
- Quarry poses 0 and 1: static, camera, sun turning, sun jump, cut.
- S2-glossy, S4, S5 and SV (section 8).

Per workload: native timing, final post-TSR pre-tonemap HDR, native Lumen pass attribution (diagnostic) and memory.

**Checks before scoring.**
- Light-proxy readback (the stale-proxy lesson), exposure and material readback.
- U0c localizes the glossy disagreement. It sweeps roughness (0.05-0.5) and source angle on the fixture, and compares against our reference and an independent pbrt-v4 or Mitsuba 3 image.

**Targets (I, to be replaced by measurement).**
- **RT path:** if UE-HW-E adds about 2-4 ms, Renderlab's GI, reflections and AA together get about 1-1.5 ms.
- **Non-RT path:** the budget is half of UE-SW-E's total, minus our current frame. If Lumen software adds about 2.5 ms, that leaves about 0.3-0.5 ms, which is likely a miss. A miss is recorded, never closed by lowering quality.

## 8. Gates

### 8.1 References and benches

**Quarry reference.**
- An in-process GPU path tracer on the RT device: hardware rays, full-detail geometry, program materials through `scatter`, pixel-integrated, with motion-integrated sequences.
- A CPU quarry reference would break the 1.5 GiB CPU-run memory rule (13 M triangles plus 1.07 GB of textures).
- **Independence check:** on the small benches, pbrt-v4 or Mitsuba 3 (Apache/BSD) must agree with the GPU path tracer within a registered tolerance before the quarry reference is trusted (Q7).

**New benches.** All are generated, redistributable and authored once for both engines.

| Bench | Content | Tests |
|---|---|---|
| **SV** | S1-like concavity fixture: creases, overhangs, undersides, 1-100 mm gaps; sky only | Sky visibility (U2) |
| **S4** | Thin room ([competitor note](research/competitor-weaknesses.md)): 1/5/100 mm walls, sliding door, red panel off-screen. v1 is lit by sun through an opening plus the environment; a hard point-light variant follows once its visibility runs on both paths | Leaks, moving occluders, off-screen bounce |
| **S5** | Mirror corner: 1 m mirror, roughness-0.15 surface, off-screen moving cube | Mirrors, off-screen reflection, mover in mirror |
| **S2-glossy** | The M5-GM plan: a normal-mapped low-roughness scan crop, small grazing-sun highlights, 0.25-1 px/frame tracks, a moving light | Glossy stability, specular AA, TSR floor |

Populations and thresholds are registered before capture.

### 8.2 Quality measures

Each must beat the Unreal floor configuration. The film reference is tracked as the target.

| Topic | Measures |
|---|---|
| GI | Indirect HDR error per population (contact/crevice, open, underside, dark side of thin walls); leak energy behind 1/5/100 mm walls; indirect contact error; off-screen bounce (S4 panel); response after light and door events (t90, overshoot, frames to within the bar); cut and disocclusion recovery; flicker residual |
| Reflections | Error per roughness band; mirror edge spread; off-screen object energy and position in the mirror (S5); energy bias; highlight energy and peak drift; seam at r_trace; flicker |
| Final image | F1-F10 on S2-glossy, S5 and the quarry; sharpness (band-pass energy ratio against film and TSR); trail energy; revealed-pixel age curve |
| Inspection | Full-frame images and flipbooks of every sequence (F10). Numbers never override it |

### 8.3 Cost and memory

- **Per stage:** p50, p95 and p99 for static, camera, sun turn, sun jump, caster/door and cut.
- **Spikes:** no frame above 1.5x steady (F9), except registered cold starts.
- **2x:** end to end against the floor configuration, on both paths, with a quiet-window final run.
- **VRAM:** the total stays below Unreal's in the matched Lumen configuration. Allocator-reserved, WDDM and DXGI figures are measured as in [M1](memory-m1.md).

### 8.4 Programmability and robustness

- **Same inputs for every program.** Standard, toon and flat programs at the same pixels receive byte-equal `indirect_*` values.
- **No cost for non-callers.** A scene whose programs call no `indirect_*` runs no M7 pass and is byte-identical to pre-M7, at equal cost.
- **Programs drive bounce.** A toon `scatter` visibly changes bounce colour.
- **No lighting-model data in receivers.** The hit record stays 16 B. Cells hold only incident radiance, transport records and geometric data.
- **Reload.** A program reload relights only that program's cells and keeps the last working pipelines.
- **Determinism (F8).** Replays are bit-identical across processes.
- **Both paths.** Every gate runs on RT and non-RT. Non-RT is proved on a device created without ray-query features.

### 8.5 Gates per unit

| Unit | Must pass to be kept |
|---|---|
| U0 | Effective readback of every configuration; GPU path tracer agrees with pbrt/Mitsuba on the benches; glossy disagreement explained or bounded; floor values recorded per measure |
| U1 | Level F film error lower on every population, static and moving; F3 bias; both paths; no change for programs that don't call the helper |
| U2 | Sky-only error and SV/quarry measures beat UE-SW-E (non-RT) and UE-HW-E (RT); 8.4 in full; cost and memory gates |
| U3 | All GI measures, including S4 events and off-screen bounce; relight-without-retrace equals a full re-trace on a static camera (byte or registered tolerance) |
| U4 | All reflection measures on S5, S2-glossy and the quarry; hit lighting identical across programs except through their `scatter`/`shade` |
| U5 | F1-F10 against TSR on S2-glossy and the quarry, on both paths; no ghost weight from surfaces invisible in the previous frame (F5) |
| U6 | Every gate together on the integrated build, 2x end to end, memory, quiet window, independent Opus review |

## 9. Units

Sizes are rough agent estimates (I). They are implementation time plus GPU verification time, kept separate.

| Unit | Content | Depends on | Size | Stop rule |
|---|---|---|---|---|
| **U0** | (a) Unreal configuration study, recook, readback; (b) GPU path-tracer reference mode and pbrt/Mitsuba cross-check; (c) glossy disagreement; (d) SV, S4, S5 and S2-glossy scenes and scorers; (e) UE-0, UE-TSR, UE-SW and UE-HW timing, images and memory | — | 10-16 h / 3-5 h of GPU and Unreal | A configuration whose effective settings can't be read back is recorded and not scored |
| **U1** | S-AA1, then S-AA2 | U0c, for the Unreal comparison only | 4-8 h / 1 h | Keep only on a film-error gain on every population |
| **U2** | WRC v1, `indirect_irradiance`, G-R1 and G-N1 for sky only | U0b, U0d | 16-30 h / 3-5 h | If coarse software rays exceed budget, probe G-N2's far field |
| **U3** | Bounce: `scatter`/`emission`, relight from records, caster and door invalidation of records, sun relight | U2 | 12-24 h / 3 h | If cell leaks on S4 exceed UE-SW-E, the cell key is redesigned before more tuning |
| **U4** | Reflections: R-R1 and R-N1, `indirect_radiance`, hit lighting | U2 cells, U1 | 16-30 h / 3-5 h | If non-RT mirrors exceed budget, R-N3 is tried only for declared planes |
| **U5** | T-A, then T-B; non-RT jittered visibility buffer | U0e's TSR floor, U1 | 12-24 h / 3-5 h | Each candidate stops at its first floor miss |
| **U6** | Integrated acceptance | All | 6-10 h / 4 h | — |

**Start with U0 and U1 in parallel.** U1 is mostly CPU and cook work, so it can proceed while the lease serves U0 or the hillclimb. U2's CPU parts can also start alongside: the cook-time near-field visibility, the coarse BVH build and the cache layout tests. Record decisions and misses in a new `docs/milestone-7-decisions.tsv`.

## 10. Open questions for Islam

1. **Output and upscaling.** Compare at 1080p native with TSR at 100%, as all comparisons so far? Or also at 4K output with Unreal's default screen percentage? The latter needs our own upscaler (T-D), or concedes the time ratio.
2. **Floor scalability.** Is Lumen Epic the floor on both paths, with High recorded for the cost curve? Lumen High and Epic differ by about 1.7-2x in Epic's own tables (A).
3. **Secondary-hit contract.** Accept an optional `scatter()`/`emission()`, view-independent per cell for bounces, with full program evaluation only at first low-roughness reflection hits? Programs without `scatter` would occlude but not reflect.
4. **Cook-time near field.** Accept the prepared visibility data for static geometry (about 50-150 MB, extra cook time) and the static/mover split it implies?
5. **Settle limit.** Extend the 8-frame soft-shadow settle limit to GI and reflections after cuts, disocclusion and lighting changes?
6. **Patent review.** Who does it, and when? First item: the surface-keyed radiance cache against US 9,754,407, a dynamic object-space shading grid. It must be checked before U2 ships beyond opt-in. The ReSTIR family stays held.
7. **Reference independence.** Accept an in-process GPU path tracer for the quarry, validated by pbrt/Mitsuba on the benches? Or allow one CPU quarry reference above the 1.5 GiB rule while nothing else runs?
8. **The non-RT 2x miss.** If Lumen software proves cheap and non-RT 2x is out of reach at matched quality, the miss is recorded and quality is not reduced. Confirm that this is the reporting, and say whether non-RT GI may meanwhile ship opt-in.

## 11. Research plug-ins

| Note | Section here | Decides |
|---|---|---|
| `docs/research/gi.md` | 3.1, 4 | Cache layout and payload; invalidation prior art; precomputed visibility prior art and patents; Lumen per-pass cost on PC; coarse-BVH software throughput; any non-screen-space non-RT GI |
| `docs/research/reflections.md` | 5 | r_trace and sub-cone count; Lumen reflection cost and quality by roughness and hit lighting; ray-cone LOD traversal; reflecting LOD geometry |
| extended `reuse-temporal-denoise.md` | 6 | TSR behaviour; history-free and adaptive AA results; patent notes on history methods |

When they land, the sketch tables are finalized, the I-labelled costs are replaced where the notes give evidence, and the status line changes to draft 2.

## 12. Ownership, licences and patents

**M7 owns.**
- New files: `src/world_cache*.rs`, `shaders/world_cache*.wgsl`, `src/gi*.rs`, `src/reflection*.rs`, `src/temporal*.rs`.
- Existing files: `shaders/program_api.wgsl`, `shaders/programs/*`, `src/lighting*.rs`, `src/environment*.rs` and `crates/cook/src/normal_map.rs`.
- A new cook visibility module, plus the bench, reference and scorer files.

**The shadow hillclimb owns.** `shadows*`, `vsm*`, `virtual_shadow*` and the surface cache (`src/virtual_shadow_cache.rs`, `shaders/virtual_shadow_cache.wgsl`). The WRC copies that pattern into new files and does not edit them. M7 reads shadow outputs, such as receiver sun visibility for visible cells, through existing bindings. Any new shadow-side API is requested from the hillclimb owner.

**To settle with root.** `SoftShadows::Temporal` lives in `src/shadows.rs`, and T1a lives in `src/film_accumulation.rs`. M7's reconstruction should be a separate engine setting in new files. Whether `film_accumulation.rs` moves to M7 is root's decision.

**Licences.**
- Copy only MIT, Apache or BSD code, with its notices: kajiya's ircache, Solari's world cache, Capsaicin's hash grid, obvhs, tray_racing, Filament, and Brixelizer pinned at v1.1.4.
- pbrt-v4 and Mitsuba 3 are used as tools.
- Never copy Epic, Unity, NVIDIA RTX SDK or FidelityFX 2.x main-branch code.

**Patents: review before shipping.**
- The ReSTIR family: held.
- SVGF and history clamping: none planned.
- US 9,754,407, against the surface-keyed radiance cache.
- Occlusion-field prior art, if used.
- Lumen-specific patents have not been checked.

## Evidence and time

Reading and design only:
- the contract;
- the brief, handoff and M5-M6 contract;
- the research notes;
- the N8, N8n, N8o, N1b, N1c and N0 records;
- programmable-shading, the memory record, bench-motion, the Unreal setup and end-to-end records;
- both M5 handovers.

No implementation, verification, GPU, Unreal or E: work was done, and nothing was deleted.
