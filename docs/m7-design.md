# M7 design: global illumination, reflections and temporal reconstruction

Design worker record, 2026-11-07, base main `bce1e39`. Contract and decisions: [milestone-7.md](milestone-7.md). This is CPU-only design: no implementation, build, GPU run or Unreal launch.

**Status: draft 2.1.**
- Grounded in [gi.md](research/gi.md), [reflections.md](research/reflections.md) and [reuse-temporal-denoise.md](research/reuse-temporal-denoise.md) §6-§11.
- Applies Islam's rulings on draft 2 and the root decisions in milestone-7.md.
- Applies the **patent policy** in AGENTS.md and milestone-7.md, which replaces draft 2's "hold until Islam rules".
- Budgets frame time from M6-close's measured Unreal shadow and frame costs ([m6-acceptance.md](m6-acceptance.md)), in sections 2, 4.1 and 8.1.

**Patent policy, as applied here ([section 9](#9-patents-and-licences)).**
1. **Design around first.**
2. **Use unclaimed parts** where a claim needs the full combination. Each part carries a note saying which claim element it avoids, and why.
3. **Use a patented method only on evidence.**
   - Every design-around must measurably fail the quality gates, or be clearly worse in the final image.
   - It ships only behind a named setting.
   - The failed alternatives, their numbers and the patent number are recorded.
   - **A speed-only miss is recorded, not a trigger.**

The quality floor is unchanged. The claim-avoidance notes are engineering readings of the claim elements as the research notes summarize them. They are not clearances, and unit P0 extracts the actual independent claims. "Not flagged" means no patent was found.

Labels: **M** is measured in the cited record, **A** is author-reported, **I** is an inference or estimate.

## Summary

1. **One world radiance cache (WRC) on both paths.** It is a second payload table on N8o-style surface-cell keys. Lighting-invariant directional sky visibility comes first, then cached incident irradiance.
   - The cache is filled by deterministic direction sets and cache-terminated rays. **There is no reservoir resampling**, so this is the world-space cache alone, an unclaimed part of US 11,315,310's ReSTIR-plus-cache combination.
   - It may become a default on the same basis as the surface shadow cache while the US 9,754,407 review is pending (Islam).
   - **RT:** hardware rays.
   - **Non-RT:** cook-time near-field occlusion (approved), plus far-field software rays against a coarse cluster-LOD BVH. Brixelizer SDF bricks are the fallback far field.
2. **Reflections shade every hit with that surface's own program.** Programs opt in to traced reflections through an optional `reflection_lobe()`.
   - Lumen's lit-surface-cache shortcut is rejected, because it caches program output.
   - **RT:** mirror, glossy and rough bands.
   - **Non-RT:** planar views, plus two tiers that are now **back in the plan** (unit R1n), each built from parts that avoid elements of the Adshir/Snap combination (section 9):
     - the cone-sized cluster-LOD BVH, traced with independent per-pixel rays against a static cooked BVH;
     - SDF bricks, sphere-traced with no triangle acceleration structure.
3. **Temporal reconstruction revives T1b** with identity fixes and no colour statistics. It becomes the default only if it beats TSR Epic on every measure at no more than half TSR's cost.
   - T1b has never been compared with TSR, so TSR is measured first (TR0).
   - Variance clipping, SVGF-style filtering, temporal gradients and dual fast/slow history are **evidence-based fallbacks** behind named settings, not first choices (section 9).
   - 4K upscaling is its own later unit, and ships only if it beats TSR Epic on every measure.
4. **The programmable-shading ABI grows by five optional pieces.**
   - Three engine inputs: `indirect_irradiance(input, n)`, `indirect_radiance(input, direction, roughness)` and `specular_incident(input)`.
   - Two program hooks: `reflection_lobe(s, input)`, and `scatter()`/`emission()` for bounce light.
   - Every recorded constraint is in [section 3.4](#34-recorded-programmability-constraints).
5. **Unit order.**
   - First: G0 (= M7-U0, running) and M7-U1 (merged opt-in; iterating).
   - Then: TR0, R0c (CPU only, can run now), R0a, P0 (claim extraction), G1, R0b, TR1, D0, G2, R1, R1n and integrated acceptance.
   - Upscaling comes last.

   See [section 7](#7-units-and-order).

## 1. Scope

**In scope.**
- Sky visibility and indirect diffuse bounce (M5-EV folds in).
- Glossy and mirror reflections, and specular anti-aliasing (M5-GM folds in).
- Temporal reconstruction of the final image under motion, cuts, disocclusion and lighting changes.

The comparison is at native 1920x1080, against Unreal TSR at 100% (Islam).

**Out of scope.**
- Transparency, volumes and fog, and atmosphere.
- Larger-than-memory streaming.
- Other GPUs.
- 4K upscaling, which comes later in its own unit.

Local-light soft visibility stays with the shadow hillclimb.

## 2. Starting point

| Item | RT path | Non-RT path | Source |
|---|---|---|---|
| Release GPU span (direct light, environment, shadows): static / sun / camera / caster | 0.867-0.868 ms in every phase | 0.734 / 1.327 / 1.792 / 2.37 ms (Surface lookup, the default) | M: [M5-DL](m5-dev-loop.md), [M6-close](m6-acceptance.md) |
| Unreal whole GPU frame, same subset (Lumen and TSR off), our motions: static / sun / camera / caster | HRT 1.743-1.753 / 1.738-1.751 / 1.764-1.766 / 1.776-1.778. VSM RT on 1.554-1.583 / 1.990-2.039 / 1.572-1.587 / 1.783-1.805 | VSM no-RT 1.555-1.559 / 1.921-2.059 / 1.553-1.574 / 1.736-1.790 | M: M6-close §1 |
| Ratio, Unreal / ours (2x = pass) | vs HRT 2.01-2.03x in every phase. vs VSM RT on: static 1.79-1.83x, sun 2.30-2.35x, camera 1.81-1.83x | static 2.12x; sun 1.45-1.55x; **camera 0.87-0.88x** | M: M6-close §2 |
| Shadow pass alone, camera moving | Interactive's eight fused rays **0.276 ms** (paired median; 0.237-0.327). Unreal HRT 0.627-0.635 (2.3x, pass); VSM RT on 0.501-0.505 (1.8x; the 2x floor 0.250-0.253 is missed by 0.02-0.03) | Surface 1.049-1.065 ms against VSM no-RT **0.495-0.500** (2.1x slower, miss; the shadow hillclimb owns this) | M: M6-close §1-§2 |
| VRAM, allocator reserved, against Unreal DXGI | **2,641 MiB** (traced + Interactive; root's M7 baseline) against HRT 5,410 | 3,503.7 against VSM 3,912 (408 MiB headroom) | M: [memory M1](memory-m1.md), N8p, milestone-7.md |
| Sky and environment | Prepared RLE2 fields, unoccluded (`shaders/programs/standard.wgsl:27-33`) | same | M: [M5-EV](m5-environment-visibility.md) |
| Specular AA | M7-U1 merged, opt-in, off by default: Toksvig plus geometric AA cuts full-frame error 4.67 → 1.71 and camera flicker 6.25 → 1.83. It fails the per-population gate: the r = .08 normal-mapped patch gets worse (.024 → .751) and energy bias doubles. Iteration continues (second moments, lobe shape) | same | M: [M7-U1](m7-u1.md) |
| Temporal | T1a converges static frames only and resets on motion. T1b (`b8fa1f3`, reverted): +0.20-0.22 ms, 5-8 px silhouette trails, **never compared with TSR**. T2: 3.55-5.76 ms, stopped | No temporal mode: the visibility buffer rejects Interactive and Temporal | M: [m6-temporal-gpu](m6-temporal-gpu.md), [N1b](non-rt-n1b.md) |
| Software rays | — | N0: 11.9-21.9x hardware; full-detail BVH 412.5 MB, more than the memory headroom | M: [N0](non-rt-n0.md) |
| Glossy parity with Unreal | 84.75% relative L1 on a roughness-0.1 metal; UE peaks 11.5-16.8x weaker; cause unproven | — | M: [setup](unreal-comparison-setup.md) |

**Machinery M7 reuses:**
- **N8o surface cells.** Keys are placement, viewed side and a power-of-two cube. They have claim/resolve inserts, flush by clear, per-cell epochs, exact caster invalidation and heaviest-first queues.
- **The cluster-LOD DAG.** 214,062 quarry clusters, with every DAG triangle indexing source vertices.
- **The N1c raster visibility buffer** (0.39 ms).
- **The virtual clipmap**, for sun visibility at hits on non-RT.
- **RLE2 environment fields.**
- **Interactive's deterministic direction sets.**
- **T1a/T1b's lattice and hit-record validation.**
- **Level F gates F1-F10.**

## 3. Shared architecture and ABI

### 3.1 World radiance cache (WRC)

**Keys and levels.**
- N8o's key, plus coarser parent levels by distance, following Solari's `log2(1 + distance / lod_scale)` cell sizing.
- The geometric normal is in the key (Solari). Capsaicin's ray direction in the descriptor is a fix for thin-occluder leaks.
- Kajiya-style voting and bounce-rank choose each cell's query point (gi.md §3-§6).

**Payload, about 40 B per cell (gi.md).**
- Directional sky visibility: L1 SH, 4 x f16 = 8 B.
- Incident irradiance: L1 SH RGB, 12 x f16 = 24 B.
- Count and epochs: 8 B.

**Sizing.** 2^21 cells = 80 MiB, 2^22 = 160 MiB. **Memory stop: 160 MiB.**

**Fill and update.**
- **Direction sets.** Each cell traces K deterministic, cosine-stratified directions, rotated per cell. There are no reservoirs and no resampling.
- **Budget.** A fixed per-frame ray budget, starting at about 0.25 rays per pixel (I). Priority runs: new cells, then epoch-changed cells, then aged cells, using N8m/N8o's heaviest-first queues.
- **Cache-terminated paths.** A ray hitting a surface reads the hit cell, which carries multi-bounce from previous frames (Solari, SHaRC idea, reimplemented). An escaped ray reads RLE2 at a ray-cone mip.
- **Off-screen cells.** Ray hits create cells. That is the off-screen half the screen-spawned systems (GIBS, GI-1.x) lack.

**Epochs.**

| Epoch | Invalidates | Leaves alone |
|---|---|---|
| Caster | Visibility and radiance, by N8o's exact region test | — |
| Sun | Radiance | Visibility |
| Environment yaw / intensity | Nothing: yaw is an SH rotation and intensity a scale, so the response lags 0 frames | — |
| Program | Radiance cached from that program's `scatter`/`emission` | Other programs' cells |

**Temporal behaviour.**
- Per-cell blending is driven by engine-known events: light rate, moved casters, parameter edits.
- Disoccluded cells are seeded from parent levels.
- Spatial filtering runs in cell space, after Lumen's probe-space idea.
- The default has no screen reprojection, no SVGF-family filter, no gradient estimator and no dual history. Those exist only as the evidence-based fallbacks in 9.1.

**Determinism.** Claim/resolve inserts, fixed directions, and an F8 hash identity across processes.

**Optional transport records (G2 variant).** A cell stores each ray's hit cell, about 6-8 B x K.
- A sun turn relights the stored hits instead of re-tracing them.
- A moved caster invalidates only the records whose segment crosses its swept bounds.

This competes with plain priority re-tracing (Lumen and Solari style) on cost and memory in G2.

**Ownership.**
- The surface cache's files belong to the shadow hillclimb. gi.md proposes extracting their key/claim/resolve code.
- Until root and the hillclimb agree on a shared module, M7 copies the pattern into `src/gi_cache.rs` and `shaders/gi_cache.wgsl`. This is a recorded, temporary duplication.

### 3.2 Engine inputs to programs

| Function | Meaning | Path and band |
|---|---|---|
| `indirect_irradiance(input, n) -> vec3<f32>` | Incident irradiance in lux, unexposed, around `n` at the engine receiver: occluded sky (G1), then bounce (G2). Replaces `environment_irradiance(n)` in the standard program | Both |
| `indirect_radiance(input, direction, roughness) -> vec3<f32>` | Occluded incident radiance in a lobe about any direction, from cells and RLE2, with no per-pixel ray. It gives specular occlusion where nothing is traced. Replaces `environment_reflection` | Both, rough band |
| `specular_incident(input) -> vec3<f32>` | Traced incident radiance integrated over the program's declared `reflection_lobe`. Zero when no lobe is declared | Traced bands |

- **Computed by the engine.** All three are computed at the engine's receiver, identically for every program, as `light_visibility` already is.
- **No cost for non-callers.** Programs that call none of them run no M7 pass. Naga reflection of the visible modules decides this, and it is gated.
- **Rejected inputs:** a bent normal or cone, a scalar sky visibility, and screen inputs.

### 3.3 Program hooks

- **`fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe { normal, alpha, weight }`**, optional.
  - The engine runs `surface()` in the reflection trace pass to obtain the lobe. This settles draft 1's question of inline versus declared lobes.
- **`fn scatter(s, input, wi, wo) -> vec3<f32>` and `fn emission(s, input) -> vec3<f32>`**, optional (Islam).
  - The engine evaluates them at cell centres, with a ray-cone footprint. It integrates `scatter` against the cell's incident SH to get one view-independent outgoing radiance for bounce.
  - A program without them blocks light but reflects none.
- **Reflection hits run the hit surface's own `surface()` and `shade()`.**
  - The view vector is −d. The footprint comes from ray cones (Falcor `TexLODHelpers`, BSD-3), since programs have no derivatives.
  - Hits are compacted per program and use the existing per-program tile worklists (sorted-deferred).
  - At a hit, `indirect_irradiance` reads the hit's cell, and `specular_incident` returns the untraced rough-band value. Reflection bounces start at 1.
  - **Sun visibility at a hit.** RT: one hardware shadow ray. Non-RT: an exact walk at the hit, with page marking at hit positions. That marking is shadow-file work, requested from the hillclimb owner.

### 3.4 Recorded programmability constraints

1. `SpecularLobe` is GGX-shaped. A non-GGX style (anisotropic, toon-banded) gets its nearest GGX lobe, or none. **Accepted by Islam** (milestone-7.md).
2. Bounce light sees a program only through `scatter`/`emission`, evaluated per cell and view-independent. That is exact for Lambert. Glossy interreflection at secondary hits is approximated. Programs without `scatter` reflect no bounce.
3. Programs without `reflection_lobe` get no traced reflections, only `indirect_radiance`.
4. Hit shading runs `shade()` with ray-cone footprints and no screen access, as traced primaries already do.
5. A filter guide for glossy denoising, if one is ever adopted, comes from `reflection_lobe`. No albedo export is ever required.
6. The WRC caches incident quantities: sky visibility and irradiance. The only program-output-derived value is the `scatter`-integrated outgoing radiance at secondary cells, keyed by program epoch.

## 4. GI design

### 4.1 Sketches, revised from gi.md

| Sketch | Path | Mechanism | Patent status | Risk | Cost (I) |
|---|---|---|---|---|---|
| **G-R1** Cell cache, hardware rays | RT | WRC filled by hardware rays at ~0.25 rays per covered pixel. Pixels read their cell (parent on a miss) with sub-cell jitter. Crevice cells never share across depth or normal breaks: N8o's reuse lesson | WRC alone: an unclaimed part of US 11,315,310 (9.1). US 9,754,407 review open; default allowed on the shadow-cache basis (Islam) | Cell reuse error in crevices (N8o's Surface lost to Exact on 4 of 5 shadow measures, M) | **Traversal:** about 1/32 of Interactive's ray count, whose eight fused rays per receiver cost 0.276 ms (M), so 0.03-0.1 ms allowing 3-5x for longer, incoherent rays. **Hit shading:** 0.1-0.3 ms (2-4x the 0.396 ms full-frame shading's per-pixel cost). **Cache passes:** 0.05-0.1 ms. **Total 0.2-0.5 ms**, against L_hw/2 (8.1) |
| **G-R1n** Per-pixel near rays in addition | RT | Up to 8 short hardware rays per receiver below R_near (draft 1) | Same | Grain in motion | **+0.25-0.3 ms**: the same ray count and length class as Interactive's measured 0.276 ms. Kept only if G-R1 loses on contacts |
| **G-N1** Cook-time near field plus coarse far field | Non-RT | **Near field (Islam-approved, 50-150 MB):** per-vertex visibility of K fixed directions within R_near, against static geometry, from exact rays at cook. Movers use short software rays against their own small BLASes; masks combine by AND. **Far field:** software rays from cells against a coarse cluster-LOD BVH (R0c cook) | Same as G-R1 | Crevice bounce approximated; cook time; self-hits at the LOD/source boundary | Near lookup 0.03-0.08 ms; far fill 0.2-0.6 ms |
| **G-N1r** Runtime near field instead | Non-RT | Software rays against a CWBVH-compressed full-detail BVH at 1/16 rays per pixel, accumulating into cells (gi.md N-GI) | Same | 412.5 MB uncompressed is above the headroom; slow convergence in motion | 0.07-0.15 ms (I, scaled from N0) |
| **G-N2** SDF bricks for the far field | Non-RT | Brixelizer v1.1.4 (MIT) sparse SDF, used **for tracing only**; its screen-fed radiance cache is replaced by the WRC | Not flagged | Detail loss (far only); about 200 MB | Fallback if G-N1's far-field rays miss budget |
| G-DDGI | Both | Probe irradiance with depth moments, own code from the CC BY-ND papers or Wicked (MIT) | Not flagged | Low frequency, lag, leaks below probe spacing | Out as core; at most a cache-miss far fallback |
| ReSTIR GI/PT + world cache | Both | Reservoirs for cell updates or the per-pixel gather, reading the WRC | **US 11,315,310** and family | — | **Patented fallback only:** `GiSampling::Restir`, behind the 9.1 trigger |
| ~~Lumen-style screen probes, screen traces, GIBS surfels, GI-1.x screen probes, SDFGI, VCT~~ | — | Screen-fed, static or leaky | — | — | Rejected (standing rule) |

**ReSTIR dependency (stated explicitly).**
- The default `GiSampling::Stratified` uses no reservoir resampling in any form.
- That includes gi.md's "world-space RIS to pick which cells to update"; cell priority is a deterministic queue instead.
- Adding resampling to anything that reads or writes the WRC forms US 11,315,310's combination. It therefore exists only as the `GiSampling::Restir` fallback, and only on 9.1's evidence.

### 4.2 Sky first (G1)

- Occluded sky is likely the largest missing outdoor term (I, gi.md).
- It is lighting-invariant, so the sky term should beat Lumen's "multiple seconds" lighting lag outright: yaw and intensity steps must equal a fresh run on the next frame.
- It exercises the cache, the ABI and both producers without the `scatter` contract.
- The [M5-EV handover](m5-environment-visibility.md)'s candidates map onto it: its precomputed visibility is G-N1's near field, its surface-cache idea is the WRC, and the single-direction clipmap and grid are excluded.

### 4.3 Bounce (G2)

- **Sun one-bounce plus cache-terminated multi-bounce**, with `scatter`/`emission`.
- **Sun visibility at hits.**
  - RT: one hardware shadow ray.
  - Non-RT: requested page marking at hits, or a coarser page level at hits if it passes the gate (gi.md risk 5).
  - The non-RT receiver walk already costs 1.049-1.065 ms against VSM's 0.495-0.500 while the camera moves (M). Off-screen hit walks would add to a measured miss, so the cheaper per-cell or coarser-level visibility is tried first.
- **Variant comparison.** Priority re-trace against transport records (3.1), on moving-sun cost and memory.

## 5. Reflection design

### 5.1 Bands (thresholds set by measurement, not copied from Unreal's 0.4)

| Band | Roughness | RT path | Non-RT path |
|---|---|---|---|
| **B0 mirror** | below ~0.05 | 1 ray at the pixel centre, deterministic, undenoised; hit shaded by its own program | **Planar raster view** for declared planes (N1c from the mirrored camera). Non-planar: the **cone-sized LOD-BVH tier** (R1n), with independent per-pixel rays against the static cooked BVH; the hit is shaded by its own program |
| **B1 glossy** | to ~0.4-0.5 | K fixed sub-cones from `reflection_lobe`, rotated per pixel (draft 1's R-R1); stochastic VNDF is the variant compared. Each hit shaded by its own program | Planar view, prefiltered by the lobe, where planar. Otherwise the LOD-BVH tier (R1n), with the cut chosen so its error stays below the lobe's blur at the hit (cone width x t) |
| **B2 rough** | above the cutoff | RLE2 prefiltered lobe x the traced-lobe ratio estimator (k = 1-2 visibility rays; reflections.md §8). Occluded rays add their hit radiance | `indirect_radiance` from WRC sky visibility and irradiance, with no per-pixel ray (default-eligible). Variant: **SDF-brick visibility rays** for the ratio estimator, sphere-traced against static Brixelizer bricks (R1n) |

**Glossy temporal integration.**
- **Design-around:**
  - B1 spends deterministic sub-cone rays, with no screen denoiser.
  - Temporal integration comes only from the final-image reconstruction (TR1). It validates history by identity: the reflected hit's instance, primitive and reverse visibility, with no colour statistics.
- **Not used first:**
  - reflections.md's B1 denoiser used a variance clamp (US 10,116,916);
  - FidelityFX's mean ± σ temporal clip is near the same claim.

  Both are evidence-based fallbacks only, as `Reconstruction::VarianceClip` and `Denoise::Svgf` (9.1).
- If B1 still fails the motion gates, it stays opt-in and labelled below the bar until a fallback's trigger is met.

### 5.2 Tiers and patent status

| Tier | Path | Patent status | Plan status |
|---|---|---|---|
| R-RT: traced primaries plus lobe rays plus own-program hit shading | RT | **Design-around** of Adshir/Snap: primaries are traced, not rasterized (I). No ReSTIR | **In plan** (R0b, R1) |
| Planar raster mirror views | Non-RT | **Design-around:** no secondary rays at all (I) | **In plan** (R1), default-eligible |
| B2 from the WRC (no per-pixel ray) | Both | No reflection rays; WRC as in 9.1 | **In plan** (R1), default-eligible |
| Cone-sized cluster-LOD BVH for B0/B1 rays | Non-RT | **Unclaimed parts** of Adshir/Snap: independent per-pixel rays (no coherent ray groups) against a static cook-time BVH (no dynamic acceleration structure). Note in 9.1 | **In plan, R1n.** R0c is already building the BVH. Default once P0 confirms the claim mapping |
| SDF-brick visibility rays for B2 and far glossy | Non-RT | **Unclaimed parts:** sphere tracing of a static SDF, with no triangle acceleration structure and no coherent ray groups. Note in 9.1 | **In plan, R1n.** Default as above |
| Coherent hybrid reflection rays from the raster G-buffer against a rebuilt structure | Non-RT | **Adshir/Snap**, the claimed combination | **Patented fallback only:** `NonRtReflections::CoherentHybrid`, behind 9.1's trigger |
| ReSTIR for glossy | Both | NVIDIA ReSTIR family | **Patented fallback only:** `ReflectionSampling::Restir`, behind 9.1's trigger |
| Lumen-style lit surface cache at hits | Both | — | **Rejected:** caches program output (breaks programmability) |
| SSR, or SSR verified in world space | Both | — | Rejected as core. The verified-hint cost lever is not planned |

**Consequence.** This supersedes the draft-2 ruling that non-RT non-planar reflections were a recorded miss (milestone-7.md, patent policy).
- **Non-RT reflections:** planar views, plus the LOD-BVH tier for B0/B1, plus WRC B2, with SDF-brick visibility rays as B2's variant.
- **If any tier fails a gate,** the per-measure numbers are recorded. Only failure on every design-around tier triggers the patented form.
- **Lumen software's mirrors are weak** ("HWRT is the only way to high-quality mirrors", V), so the LOD-BVH tier is the likely quality win. Its cost is the risk: software rays measured 11.9-21.9x hardware at full detail (M).

### 5.3 Specular AA (M7-U1, merged opt-in; iterating)

- **Toksvig/LEAN** variance kept at cook (`crates/cook/src/normal_map.rs`).
- **Kaplanyan/Tokuyoshi** widening from ray-differential normal variance (Filament `normalFiltering`, Apache-2.0).
- **Geometric multi-scale vertex variance** from the DAG, for sub-pixel triangle relief.
- **Acceptance:** keep only on a Level F film-error gain on every population, static and moving.
- **Status (M, [m7-u1.md](m7-u1.md)):** merged opt-in.
  - The full frame improves (error 4.67 → 1.71, camera flicker 6.25 → 1.83).
  - The per-population gate fails: the r = .08 normal-mapped patch gets worse and energy bias doubles.
  - Iteration continues with second moments and lobe shape.
- It also reduces how many pixels need B0/B1 rays (reflections.md §8).

## 6. Temporal reconstruction design

| Sketch | Mechanism | Patent status | Plan |
|---|---|---|---|
| **T1b revived (TR1)** | Resampled history through the exact hit record, plus an **anchor bit** against silhouette trails, **engine-event W caps** (light rate, moved casters, parameter edits, exposure, declared program bandwidth), a fix for Catmull-Rom undershoot at silhouettes, and **no colour clamp**. Reverse-visibility validation, as T1 already has | **Design-around:** single history with capped windows; no neighbourhood statistics, no gradient, no dual history (I) | **In plan, first** |
| Own-pixel temporal moments as confidence | The pixel's own accepted samples over time (mean and σ) shorten W or force a reset. No spatial box and no clip | **Unclaimed parts** of US 10,116,916 (9.1) | In plan as TR1's first extra mechanism if trails or shading-change lag remain |
| **History-free boost (T-A)** | Extra primary samples only where history is missing or rejected: revealed pixels, listed discontinuities (ATAA idea, reimplemented from the paper). The settle mechanism for never-seen surfaces inside TR1, and a standalone fallback if TR1 fails | Not flagged | In plan, within TR1 |
| Two-layer silhouette history | Heavier trail fix | Not flagged | One named fix allowed by TR1's stop rule |
| A-SVGF re-shade of anchored samples | Re-shade with the old seed to detect shading change | **US 12,437,371** | **Patented fallback only:** `Reconstruction::GradientReshade` (9.1) |
| Variance clip / TAA box clip | Neighbourhood mean ± γσ, applied after identity validation | **US 10,116,916** (to 2037); US 10,362,289 | **Patented fallback only:** `Reconstruction::VarianceClip` (9.1) |
| Colour or luma rejection (TSR, FSR) | Reads output colour | Style-dependent (programmability), apart from patents | Rejected |
| Fast/slow dual history | Short average plus exponential history, dropped on events | **US 11,847,737** | **Patented fallback only:** `Denoise::FastHistory` (9.1) |
| T2 world-anchored records with scatter | As T2b | Not flagged | Held as the film-target reference (4.25 ms, M) |
| 4K temporal upsampling | T1b at sub-native input | Same as T1b | **Later, its own unit.** Ships only if it beats TSR Epic on every measure at the same screen percentage |

**Rules.**
- **Default rule (research §6.7).** TR1 becomes the default only if it beats TSR Epic on every measured population, static and moving, with no trails or flicker beyond TSR's, at ≤ ½ of TSR's measured end-to-end cost. Otherwise it is opt-in, labelled "temporal reconstruction, below film".
- **Both paths.** Non-RT needs a jittered N1c visibility buffer that accepts Temporal. Rigid motion is exact through hit records and previous transforms on both paths. No velocity buffer is needed.

### 6.1 Denoising GI and reflections

Islam's order: more samples and better caches first, then unflagged filters, then patented ones on evidence.

1. **Design-around: the WRC is the denoiser.**
   - World-space accumulation of incident quantities, with engine-event W caps and cell-space filtering.
   - More cell rays and finer cell levels.
   - Exact relight from transport records for sun and light changes.
2. **Unclaimed parts, for residual cleanup.**
   - An edge-avoiding à-trous filter with fixed geometric edge stops (depth, normal, instance) and **no variance guidance** (Dammertz et al. 2010).
   - Temporal accumulation alone, as one history.

   Either must settle within 8 frames, and both are tested in D0. Claim notes are in 9.1.
3. **Patented fallbacks, on evidence only:**
   - `Denoise::Svgf`, a Falcor SVGF port (BSD-3, notice kept): US 10,600,167;
   - `Denoise::FastHistory`, a ReBLUR-like fast plus slow history: US 11,847,737;
   - `Denoise::ValidationRays`, kajiya-style secondary-ray gradients: possibly within US 12,437,371.

## 7. Units and order

Sizes are rough agent estimates (I), given as implementation / GPU verification.

| # | Unit | Content | Depends on | Size | Gates and stop rules |
|---|---|---|---|---|---|
| 1 | **G0 = M7-U0** (running) | Unreal denominators (section 8): GI HW/SW x High/Epic/Cinematic, hit lighting, mesh SDFs on and off. Sun-step and yaw-step settle curves. Memory with Lumen on. **R_GI** references from the in-process GPU path tracer, environment-only and full, at quarry poses 0/1 and S1-S3, cross-checked with pbrt or Mitsuba on two crops. The glossy 84.75% disagreement is localized by roughness and source-angle sweeps | — | running | Every Unreal configuration's effective settings read back, or it is not scored. R_GI agrees with pbrt/Mitsuba within a registered tolerance |
| 2 | **M7-U1** (merged opt-in; iterating) | Specular AA (5.3): second moments and lobe shape next | G0's glossy item, for the Unreal comparison only | running | Level F gain on every population |
| 3 | **TR0** | TSR Epic and Cinematic, `r.AntiAliasingMethod=4`, `r.ScreenPercentage=100`, plus a 1x row and an informative 66.662% row, on Q-walk, Q-disocclusion and Q-sun. Scored with `scripts/m6-temporal-motion-score.py` (from `b8fa1f3`) on T1b's populations: F1, F2 by age, trail width, background-centre error, hold flicker. End-to-end A/B timing in one process. Also settles whether TSR leaks at shadow edges | Unreal harness; batch with G0 if its session is still open | 3-6 h / 1-2 h | Output: TSR's error per population as a multiple of the bar, and its end-to-end ms |
| 4 | **R0c** (CPU only, can run now) | Cook the cone-sized cluster-LOD BVH for the quarry at two error bounds: bytes and build time. Report the per-mesh BVHs as **static** (built once, never refit). CPU-trace 10,000 RF1 reflection rays and 10,000 GI cell rays against the N0 full-detail oracle: hit differences against footprint | — | 4-8 h / 0 | Feeds G-N1's far field and R1n's B0/B1 tier |
| 5 | **R0a** | Reflection fixtures RF1-RF10 (8.2) in both engines. UE-HW-Hit, UE-HW-SC and UE-SW at Epic and Cinematic: ProfileGPU nodes, images, memory. Roughness histogram of the plain quarry | G0 harness (same batch where possible) | 4-8 h / 2-3 h | Effective readback, as G0 |
| 6 | **P0** (CPU research, parallel) | Read the independent claims of every flagged patent (9.1) and list their elements. Map each M7 part to the elements it lacks, and correct 9.1's notes. An agent's reading, not a clearance | — | 3-6 h / 0 | Must finish before any unclaimed-part tier becomes a default (WRC excepted: Islam's ruling) |
| 7 | **G1** | WRC v1, `indirect_irradiance` for sky only, G-R1 and G-N1 (near precompute + far LOD BVH), both paths, `GiSampling::Stratified` | G0 (R_GI, L), R0c | 16-30 h / 3-5 h | gi.md's G1 gates and stop rules (8.3) |
| 8 | **R0b** (test-only) | R-RT B0 and B1 at 1 ray per pixel on RF1/RF2, no denoiser, static accumulation. Trace and hit-shading cost; error against the glossy film reference. **Mirror-camera identity gate** on the standard and toon programs | R0a, U1 | 8-12 h / 2 h | Any identity failure is a contract bug and blocks everything else |
| 9 | **TR1** | T1b revived (section 6) on both paths, including the jittered visibility buffer and the revealed-pixel boost. Own-pixel moments are the first extra mechanism | TR0, U1 | 12-20 h / 3-4 h | Section 8.3. If a population is still worse than TSR after the anchor bit plus one named fix (moments, two-layer history or the ±1 px reverse search), record the per-measure numbers; that record is `Reconstruction::VarianceClip`'s trigger input. Stop if the cost delta is more than ½ of TSR's after one optimisation pass. If TSR is worse than T1b everywhere, gate T1b as is |
| 10 | **D0** (CPU replay) | 1-spp incident-irradiance dumps on Q-walk, Q-sun and Q-disocclusion. Compare: (a) world-cell accumulation with event W; (a′) (a) plus cell-space spatial filtering; (b′) non-variance à-trous on the residual; (c) a sample-count control at 2x and 4x rays | G1 dumps | 4-8 h / 1 h | Drop a filter that cannot reach the bar by frame 8 after each event, or whose GPU estimate is above ½ of Lumen's measured denoise share. If none passes, spend on samples or caches; the records feed `Denoise::*` triggers |
| 11 | **G2** | Bounce: `scatter`/`emission`, sun one-bounce, cache-terminated multi-bounce. Records against priority re-trace | G1, D0 | 12-24 h / 3-5 h | All GI measures, including S4 events and off-screen bounce |
| 12 | **R1** | Reflections: R-RT B0-B2, `reflection_lobe`, `specular_incident`, hit shading by program; non-RT planar tier and WRC B2 | R0b, G1, U1 | 16-30 h / 3-5 h | 8.3. If RT cost is above 0.5x UE-HW-Hit's "Lumen Reflections" after three hillclimb mechanisms, record the miss |
| 13 | **R1n** | Non-RT B0/B1 through the LOD-BVH tier: independent per-pixel rays against R0c's static BVHs, movers as static per-instance BVHs in a flat per-frame list, with no rebuilt top level. B2 variant: SDF-brick visibility rays against static bricks. Same hit contract and programs as R1 | R0c, R1, P0 (for default) | 16-28 h / 3-5 h | All RF measures against UE-SW Epic. If LOD-BVH rays cost more than 3x hardware rays on the same rays after three hillclimb mechanisms, record the speed miss (not a patent trigger). A quality failure on every tier is the `CoherentHybrid` trigger input |
| 14 | **Integrated acceptance** | Every gate on the integrated build, both paths, against the Epic floor; 2x end to end; memory; quiet window; independent Opus review | all | 6-10 h / 4 h | 8.3 |
| 15 | **Upscaling** (later) | 4K output | TR1 | — | Ships only if it beats TSR Epic on every measure |

**Why this order.**
- G0 and U1 are already running.
- TR0 and R0c are cheap. TR0 decides whether T1b already beats TSR; R0c sizes the non-RT far field.
- G1 is the first Renderlab unit. It is lighting-invariant, runs on both paths, needs no `scatter` contract, and its lag gate is clear.
- R0b proves the hit contract before R1 builds on it.
- D0 picks the GI accumulation before G2 commits to one.
- P0 runs in parallel and only gates defaults.
- R1n follows R1 because it reuses R1's hit contract and programs, and R0c's BVH. It ships non-default until P0 confirms its claim mapping.

**Parallel ownership.**
- G1 and R0c are CPU-heavy and separable. G1 owns `gi_cache*` and the cook visibility module; R0c owns the LOD-BVH cook.
- R1n owns the new `reflection_nonrt*` files.
- P0 owns only `docs/research/patent-claims.md`.
- TR1 owns `film_accumulation.rs` and the new `temporal*` files, which root assigned to M7.
- None of them edits the shadow files.

## 8. Plan against Unreal 5.7 and gates

### 8.1 Unreal method

**Reused harness.**
- Packaged Development UE 5.7.4 CL51494982, D3D12 SM6, `-RenderOffscreen`, hidden start.
- No focus changes, input automation or captures.
- CSV `GPUTime` frames 96-159, ProfileGPU at frame 64, DumpGPU replays separate from timing, DXGI `LocalUsedMB` memory.

**Scalability.**
- Read the installed `C:/Program Files/Epic Games/UE_5.7/Engine/Config/BaseScalability.ini` and record its SHA-256 per run.
- Record the `sg.GlobalIlluminationQuality`, `sg.ReflectionQuality` and `sg.AntiAliasingQuality` group values in effect, then read back every derived cvar in the process. The ini sets values; readback proves them.
- The floor is **Epic (3)** and High (2) is the cost curve (Islam). **Cinematic** is recorded as the quality ceiling.

**Effective Lumen tracing mode, logged in every run.**
- Unreal's own documentation contradicts itself on the default, so each run logs `r.Lumen.HardwareRayTracing`, `r.Lumen.HardwareRayTracing.LightingMode`, `r.Lumen.Reflections.HardwareRayTracing.HitLighting`, `r.Lumen.TraceMeshSDFs` and `r.RayTracing.Nanite.Mode`.
- It also logs `DumpCVars r.Lumen.`, `r.TSR.` and `r.LumenScene.` into the evidence JSON.
- A run whose logged mode differs from its label is discarded.
- LightingMode is set through the device profile or the command line, not saved config (5.7 editor-crash report).

**Content.**
- Software Lumen needs a recook with mesh distance fields, recorded as a new content identity.
- Log `r.DistanceFields.LogAtlasStats 1` and each scan's distance-field resolution scale.
- Hardware Lumen sets `r.DistanceFields.SupportEvenIfHardwareRayTracingSupported=0` so Unreal doesn't hold both SDFs and BVHs.

**Configurations.**

| ID | Settings | Role |
|---|---|---|
| UE-0 | Lumen and TSR off (existing subset) | Baseline for increments |
| UE-SW-{H,E,C} | Lumen GI and reflections, `HardwareRayTracing 0`, with `TraceMeshSDFs` 0 (the 5.7 default) and 1, VSM, TSR Epic | **Non-RT floor at E** |
| UE-HW-SC-{H,E,C} | `HardwareRayTracing 1`, LightingMode 0, TSR. Run once with **VSM RT-on shadows** and once with HRT shadows | RT cost curve. VSM RT on is Unreal's cheaper RT-mode shadow (0.501-0.505 ms camera-moving, against HRT's 0.627-0.635, M) |
| UE-HW-Hit-{E,C} | LightingMode 2 (hit lighting for reflections), both shadow variants | **RT reflection floor** and RT quality ceiling |
| UE-GI-only | As above, with reflection method none or sky | Matches G1/G2 scope |
| UE-sky | As UE-GI-only, with the sun's Indirect Lighting Intensity 0 (to verify) | G1's sky-only comparator |
| UE-TSR | `AntiAliasingMethod=4`, `ScreenPercentage=100`, `sg.AntiAliasingQuality` 3 and Cinematic; plus 1x (`=0`) and 66.662% rows | TR0 |
| UE-DLAA (optional) | DLAA preset K, if installed | Informative only, never a bar (root). DLSS is not linked into Renderlab |

**Timing.**
- End to end: same-process A/B toggles, measured in CSV `GPUTime` with async at its defaults.
- Attribution: ProfileGPU with `r.Lumen.AsyncCompute=0` and `r.TSR.AsyncCompute=0`, for these nodes:
  - Lumen Scene Update and Lumen Scene Lighting;
  - Screen Probe Gather (traces, filter, integrate, temporal, ShortRangeAO);
  - Lumen Reflections;
  - TemporalSuperResolution.
- Overlapping queues are never summed.

**Lag.** A 10° sun step and an environment yaw step, captured at frames 1, 2, 4, …, 128 after the step: frames-to-settle against the settled image.

**Frame budget from measured frames (M6-close; L unmeasured until G0 and TR0).**
- **Definitions.**
  - L is Unreal's measured increment for Lumen plus TSR in the matched configuration.
  - B is what Renderlab may add for GI, reflections and reconstruction and still keep 2x.
  - B = (Unreal frame + L)/2 − our frame, using range midpoints of the measured frames in section 2.

| Path, comparator | Static | Sun turning | Camera moving | Caster moving |
|---|---:|---:|---:|---:|
| RT vs HRT | L/2 + 0.01 | L/2 + 0.01 | L/2 + 0.01 | L/2 + 0.02 |
| RT vs VSM RT on | L/2 − 0.08 | L/2 + 0.14 | L/2 − 0.08 | L/2 + 0.03 |
| Non-RT vs VSM no-RT | L/2 + 0.04 | L/2 − 0.33 | **L/2 − 1.01** | **L/2 − 1.49** |

- **The binding RT row** is the faster Unreal configuration whose quality Renderlab beats. M6 found Renderlab's shadows beat both, so VSM RT on binds unless G0 shows otherwise.
- **The RT shadow margin is already 0.02-0.03 ms short.** Interactive's 0.276 ms against VSM RT on's 0.501-0.505 (M). M7's 2x margin on the RT path therefore has to come from GI, reflections and reconstruction costing less than half of Unreal's L.
- **Non-RT camera and caster rows need L_sw ≥ 2.0-3.0 ms** before Renderlab adds anything, because the non-RT shadow pass (1.049-1.065 ms against 0.495-0.500) already misses.
  - That shadow miss belongs to the hillclimb.
  - M7 records its own B per phase separately, so the two misses are never mixed.
  - Per Islam, non-RT GI still becomes the default if it beats Unreal's quality on every measure; the speed miss is recorded.

### 8.2 References and fixtures

**References.**
- **R_GI:** in-process GPU path tracer (root decision), with the same geometry, programs, sun disk, environment and ε policy. Converged by spp doubling. Bevy's `pathtracer.wesl` and Capsaicin's reference tracer are copy-licensed models.
- **Independent oracle:** pbrt-v4 or Mitsuba 3, on crops.
- **UE path tracer:** a sanity image only.
- **Glossy film reference:** the same tracer at high spp per pixel footprint.

**Fixtures.**

| Set | Fixtures | Tests |
|---|---|---|
| GI | Quarry poses 0/1; S1-S3; **SV** (concavities, overhangs, 1-100 mm gaps, sky only); **S4** (thin room, door, off-screen panel) | Occlusion, leaks, off-screen bounce, events |
| Reflections (RF prefix to avoid clashing with Level F) | **RF1** puddle at roughness 0/0.05/0.1, camera pitched 20-40°; **RF2** roughness sweep 0-1 on dielectric and metal; **RF3** moving occluder and reveal; **RF4** rails and poles; **RF5** moving sun and reflected object; **RF6** grazing view; **RF7** facing panels; **RF8** wet quarry rock at 0.3-0.5; **RF9** shells seen from behind; **RF10** toon object in RF1 | Bands, off-screen reflections, thin features, lag, programmability |
| Temporal | Registered Q-walk, Q-disocclusion, Q-sun; S2-glossy (M5-GM plan) | TR0/TR1, glossy stability |

### 8.3 Gates

**Common to every unit.**
- **Both paths.** Every gate runs on RT and non-RT. Non-RT is proved on a device created without ray-query features.
- **Unreal floor.** Beat Epic on every registered measure, static and moving. Cinematic is tracked as the ceiling, film as the target.
- **Settle.** ≤ Unreal at every age, and ≤ the bar by frame 8 after every cut, disocclusion and lighting event (root: the 8-frame limit extends to GI and reflections).
- **Cost.** p50/p95/p99 per stage for static, camera, sun turn, sun step, caster/door and cut. No frame above 1.5x steady except registered cold starts. 2x end to end against the Epic configuration, with a quiet-window final run.
- **Memory.** Renderlab total < Unreal's total with Lumen on, per path. That includes the cook-time near field and the WRC. WRC ≤ 160 MiB.
- **Programmability (3.4).**
  - Standard, toon and flat programs at the same pixels get byte-equal engine inputs.
  - Non-callers run no M7 pass and are byte-identical to pre-M7.
  - A toon `scatter` changes bounce colour.
  - The mirror-camera identity gate passes.
  - The hit record stays 16 B.
  - A program reload relights only that program's cells.
- **Determinism (F8).** Replays are hash-equal across processes. Any unexplained difference is fixed in the implementation, never in the comparison.
- **Inspection (F10).** Full frames and flipbooks. Numbers never override them.

**Per unit.**

| Unit | Must pass |
|---|---|
| G1 | **Error against R_GI(env):** RMS, crevice pixels, over-occlusion of open pixels, flicker in motion. **Against Unreal:** UE-sky at Epic, HW for RT and SW for non-RT. **Determinism and settle:** hash-equal across processes; a caster move settles byte-identical to fresh; yaw and intensity steps equal fresh on the next frame (0 frames lag). **Stop rules:** after 3 retained mechanisms still losing a measure; RT camera-moving cost above 0.3 x L_hw; non-RT above L_sw/2 continues only as quality-first and is recorded. Disoccluded first-frame error ≤ UE's first frame |
| R0b | Mirror-camera identity on standard and toon; converged static error per band ≤ Unreal's |
| TR1 | ≥ TSR on every population at every age (Q-walk, Q-disocclusion, Q-sun); trail width and hold flicker ≤ TSR; F8; non-RT run byte-identical to its paired software run; cost delta ≤ ½ TSR's, total frame ≤ ½ Unreal's |
| D0 | Bar by frame 8 after each event; GPU estimate ≤ ½ Lumen's denoise share |
| G2 | All GI measures, including S4 leaks behind 1/5/100 mm walls, off-screen bounce, and door/light response (t90, overshoot) |
| R1 | All RF measures per band on RT, and on non-RT for planar and WRC B2. B1 motion (RF3, RF5) ≤ Unreal, or opt-in and labelled |
| R1n | All RF measures on non-RT against UE-SW Epic, per tier (LOD-BVH B0/B1, SDF-brick B2), static and moving, including RF4 thin features and RF9 back faces. LOD-BVH hits match the N0 full-detail oracle within the lobe footprint (R0c's metric) |
| Integrated | Everything above together, on the quarry and fixtures, against the Epic floor |

## 9. Patents and licences

### 9.1 Flagged areas: design-around, unclaimed parts, fallback

**How to read this section.**
- **Claim elements** are those the research notes summarize. Unit P0 replaces them with each patent's independent claims.
- **"Avoids element X"** means the M7 part lacks X. That is an engineering reading, not a clearance.

**Fallback rule (policy step 3).**
- A patented method lives only behind its named setting.
- Its default is off until the trigger evidence is recorded in `docs/milestone-7-decisions.tsv` with:
  - the patent number;
  - every failed design-around and unclaimed-part candidate, with its per-measure numbers on both paths;
  - the fallback's numbers on the same fixtures and frames.
- The trigger is always a quality failure against the Unreal Epic floor, static or moving, on either path, or a clearly worse final image.
- **A speed-only miss is recorded and never triggers a fallback.**

**A. GI cache: US 11,315,310** ("Reservoir-based spatiotemporal importance resampling utilizing a global illumination data structure").

| | Content |
|---|---|
| Claim elements (as summarized) | Reservoir-based spatiotemporal resampling, used together with a GI data structure (cache, probes, photon map) |
| **Design-around** | `GiSampling::Stratified`: WRC filled by deterministic cosine-stratified direction sets, deterministic priority queues, cell-space filtering, cache-terminated rays (4.1) |
| **Unclaimed part used** | The world-space cache alone. It **avoids the reservoir-resampling element**: no step selects or reuses samples through reservoirs or RIS, including cell-update selection. "Resampling alone" is not a useful second part: the rest of the ReSTIR family (US 11,948,246; 11,663,773 / 12,361,633; 12,361,635; 12,299,801) claims resampling itself, and screen-space reuse breaks the core rule |
| **Patented fallback** | `GiSampling::Restir`: reservoirs for cell updates or the per-pixel gather, reading the WRC |
| Trigger evidence | G1/G2 still lose a registered measure to Unreal Epic after all of: their three retained mechanisms, 2x the cell ray budget, and one finer cell level ("more samples and better caches"). Examples: crevice error, flicker in motion, settle by frame 8. A CPU replay of the same saved rays must also show the reservoir variant passing that measure without failing another |
| Related | **US 9,754,407** (dynamic object-space shading grid): the WRC stores incident light, not shaded samples, except `scatter`-integrated bounce cells (I). Islam allows a default on the shadow-cache basis while the human review is open. **US 11,925,860** (projective hash maps): our key hashes the 3D cube, placement and side directly, with no projection to fewer dimensions; it avoids the projection element (I) |

**B. Temporal history: US 10,116,916** (variance clipping, to 2037; continuation US 10,362,289).

| | Content |
|---|---|
| Claim elements (as summarized) | TAA history adjusted by an axis-aligned box built from the per-component mean and standard deviation of the spatial neighbourhood |
| **Design-around** | `Reconstruction::Identity` (TR1): T1b's hit-record validation (instance, surface continuity, reverse visibility), the anchor bit, engine-event W caps, the Catmull-Rom undershoot fix and the revealed-pixel boost. No colour statistics |
| **Unclaimed part** | **Own-pixel temporal moments** (mean and σ of the pixel's accepted samples over time) used only to shorten W or reset. It **avoids the spatial-neighbourhood element and the history-adjustment (clip) element**. A min/max box over identity-validated taps would avoid the mean/σ element. But it sits near the history-clamping patents (US 11,783,455, US 11,113,792) and is colour-based, so it is last and must also pass on toon and flat programs |
| **Patented fallback** | `Reconstruction::VarianceClip`: a mean ± γσ clip applied after identity validation |
| Trigger evidence | TR1, its first extra mechanism (moments) and one named fix (two-layer history or the ±1 px reverse search) still lose to TSR Epic on a registered population on either path: trail energy, ghost bias (F5), or shading-change lag. A replay of saved TR1 buffers must show the clip variant passing that population without failing another, on the standard, toon and flat programs |
| Related | **US 12,437,371** (temporal gradients): `Reconstruction::GradientReshade`. Its design-around is engine-known events, which are exact. Trigger: shading change that no engine event covers (for example a program's undeclared time use) fails F2 or F5 after declared-bandwidth W caps |

**C. Denoisers: SVGF US 10,600,167, gradients US 12,437,371, fast history US 11,847,737.**

| | Content |
|---|---|
| Claim elements (as summarized) | **SVGF:** temporal accumulation plus a variance estimate from temporal luminance moments, guiding an edge-avoiding spatial filter. **Gradients:** history reweighted by a temporal gradient from re-shaded old samples. **Fast history:** a short moving average alongside an exponential history, the latter dropped on detected events |
| **Design-around** | WRC accumulation of incident quantities with engine-event W caps; cell-space filtering; more cell rays and finer levels; exact relight from transport records on light changes (6.1) |
| **Unclaimed parts** | (i) **Non-variance à-trous** with fixed geometric edge stops. It avoids SVGF's variance-guidance element. (ii) **One exponential history.** It avoids fast history's dual-history element and SVGF's spatial-filter element. (iii) **Record relight on engine events.** It avoids the gradient element: nothing is estimated, the cell is recomputed when an input changes |
| **Patented fallbacks** | `Denoise::Svgf` (Falcor SVGF, BSD-3 notice kept); `Denoise::FastHistory`; `Denoise::ValidationRays` (kajiya-style gradients) |
| Trigger evidence | Islam's order. D0's design-around and unclaimed filters fail the bar by frame 8 after an event, or lose to Unreal on contact blur or flicker. Then the sample-count control at 2x and 4x rays and one finer cell level also fail. Only then is the matching patented filter measured on the same dumps and adopted if it passes |

**D. Reflections: Adshir/Snap and the ReSTIR family.**

Adshir/Snap is US 10,950,030 B2, US 10,699,468, US 11,302,058, US 12,051,145, US 12,573,127 and pending US 2025/0363721.

| | Content |
|---|---|
| Claim elements (as summarized) | **Adshir/Snap:** raster primaries, plus coherent secondary reflection rays, plus dynamic acceleration structures. **ReSTIR:** spatiotemporal reservoir reuse |
| **Design-around** | **RT:** traced primaries plus per-pixel lobe rays plus own-program hit shading. It avoids the raster-primary element. **Non-RT:** planar raster mirror views, which avoid every ray element. **Glossy sampling:** deterministic sub-cones plus the B2 ratio estimator, with no reservoirs |
| **Unclaimed parts** (now in plan, R1n) | **LOD-BVH tier:** independent per-pixel rays, with no grouping, packets, beams or ray projection. It **avoids the coherent-secondary-ray element**. The BVHs are built once at cook and never refit or rebuilt, which **avoids the dynamic-structure element**. Movers are static per-instance BVHs in a flat per-frame list. A rebuilt top-level tree would bring the dynamic element back, so it is not used before P0. **SDF-brick tier:** sphere tracing of static Brixelizer bricks, with no triangle acceleration structure and no coherent groups; it avoids both elements. Movers use their own static object SDFs under their transform, with no per-frame brick rebuild. **Plain per-pixel weighted sampling** within one frame (no spatial or temporal reuse) would avoid ReSTIR's reuse element. US 12,361,635 (initial candidates) may still read on it, so it waits for P0 |
| **Patented fallbacks** | `NonRtReflections::CoherentHybrid`: coherent reflection rays from the raster G-buffer against a rebuilt structure. `ReflectionSampling::Restir`: reservoir reuse for B1 |
| Trigger evidence | **CoherentHybrid:** the planar, LOD-BVH and SDF-brick tiers all fail a registered RF measure against UE-SW Epic. Examples: RF1 off-screen mirror error, RF4 thin features, RF3/RF5 motion. A cost miss alone never triggers it. **Restir:** B1 with K = 8 sub-cones, then 2x rays, still fails RF2's band error or RF3/RF5 motion against Unreal Epic on either path |

### 9.2 Other flags and unflagged techniques

| Technique | Exposure | In this design |
|---|---|---|
| History clamping | US 11,783,455, US 11,113,792 | Not used. Section B's min/max option is last and P0-checked |
| Object-space shading | US 9,747,718 | Not used |
| DDGI, ratio estimator, ATAA, ray cones, Toksvig/LEAN, Kaplanyan-Tokuyoshi, planar reflections, Dammertz à-trous | Not flagged | Used as described (no clearance) |

### 9.3 Licences

| Rule | Items |
|---|---|
| **Reimplement only: read docs, copy no code** | Epic, Unity, RTXGI 2.x, SHaRC, NRC, NRD, RTXDI, DLSS / DLSS-RR, XeSS, FSR 4, FidelityFX 2.x binaries |
| **Own code from the text** | DDGI papers (CC BY-ND); RTG ch. 25 (CC BY-NC-ND) |
| **Copy with notice** | Bevy Solari's world-cache files only (`world_cache_{query,update,compact}.wesl`, MIT/Apache; never its ReSTIR files); kajiya ircache (MIT); Capsaicin hash cells and leak fix (MIT; not `world_space_restir.hlsl`); Falcor `TexLODHelpers` (BSD-3); Filament `normalFiltering` (Apache-2.0); obvhs and tray_racing (MIT/Apache); Brixelizer, FidelityFX SDK pinned at v1.1.4 (MIT); Wicked DDGI (MIT) |
| **Tools** | pbrt-v4, Mitsuba 3 |
| **Copyable, patented: fallback settings only** | Falcor SVGF (BSD-3) as `Denoise::Svgf`; the FidelityFX reflection denoiser's temporal clip (MIT) is near US 10,116,916 and is not used |

## 10. Rulings applied, and one question

**Applied from milestone-7.md.**
- **GI cache:** a default on the shadow-cache basis while US 9,754,407 is reviewed.
- **Adshir/Snap:** superseded by the patent policy; the non-RT tiers are in R1n.
- **Denoisers:** samples and caches first, then unflagged filters, then fallbacks on evidence.
- **`reflection_lobe`:** GGX-shaped, accepted (3.4).
- **DLAA:** an informative row only.
- **Cache code:** copied into M7 files, possibly merged after the hillclimb.
- **RT baseline:** 2,641 MiB.

**Question for Islam.** The policy allows unclaimed parts when the design notes which claim each part avoids. milestone-7.md still says a human patent review is open before ReSTIR- or SVGF-family code ships. Is P0's agent-read claim mapping enough for the R1n tiers and TR1 to become defaults? Or does each also wait for the human review, as the WRC's US 9,754,407 review does?

## Evidence and time

Reading and design only:
- the three research notes and milestone-7.md's decisions;
- the M5-DL, N8p and memory records;
- M6-close's acceptance record (measured Unreal shadow and frame costs; Interactive's 0.276 ms) and the patent policy (draft 2.1);
- draft 1 and its inputs.

Of the running M7-U0 and M7-U1 worktrees, only their branch names and changed-file lists were seen. None of their uncommitted work was read or used. No implementation, verification, GPU or Unreal time was spent.
