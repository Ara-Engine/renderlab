# M7 design: global illumination, reflections and temporal reconstruction

Design worker record, 2026-11-07, base main `7a4fee6`. Contract and decisions: [milestone-7.md](milestone-7.md). This is CPU-only design: no implementation, build, GPU run or Unreal launch.

**Status: draft 2.2.**
- Grounded in [gi.md](research/gi.md), [reflections.md](research/reflections.md) and [reuse-temporal-denoise.md](research/reuse-temporal-denoise.md) §6-§11.
- Applies Islam's rulings on draft 2 and the root decisions in milestone-7.md.
- Applies the **patent policy** in AGENTS.md and milestone-7.md, which replaces draft 2's "hold until Islam rules".
- Budgets frame time from M6-close's measured Unreal shadow and frame costs ([m6-acceptance.md](m6-acceptance.md)), in sections 2, 4.1 and 8.1.
- **New in 2.2:** absorbs P0's mapping from the primary claim text of 53 US documents ([m7-p0-claims.md](m7-p0-claims.md)). P0's design changes W1-W8 and constraints C1-C8 are adopted, and §9.1 is corrected from the claims.

**Patent policy, as applied here ([section 9](#9-patents-and-licences)).**
1. **Design around first.**
2. **Use unclaimed parts** where a claim needs the full combination. Each part carries a note saying which claim element it avoids, and why.
3. **Use a patented method only on evidence.**
   - Every design-around must measurably fail the quality gates, or be clearly worse in the final image.
   - It ships only behind a named setting.
   - The failed alternatives, their numbers and the patent number are recorded.
   - **A speed-only miss is recorded, not a trigger.**

The quality floor is unchanged.

The claim-avoidance notes follow P0's reading of the independent claims. That reading is an engineering reading, not a clearance. milestone-7.md records that it is enough for a design-around or unclaimed-part tier to become a default, and Islam can commission a legal review at any time. "Not flagged" means no patent was found.

Labels: **M** is measured in the cited record, **A** is author-reported, **I** is an inference or estimate.

## Summary

1. **One world radiance cache (WRC) on both paths.** It is a second payload table on N8o-style surface-cell keys. Lighting-invariant directional sky visibility comes first, then cached incident irradiance.
   - The cache is filled by deterministic direction sets and cache-terminated rays.
   - **P0 corrects its patent exposure.** It is exposed to US 10,614,613 (hashed cells with a counter-and-average) and US 11,790,594 (ranked cache updates), not to US 9,754,407. So:
     - a cell value is either **replaced by a full fixed-direction re-trace** or **combined from per-direction slots with fixed weights**, never counted and averaged;
     - new and changed cells are always re-traced, and the rest **refresh on a fixed rotation**, with no ranking;
     - each cell has a **fixed query point**, with no voting;
     - **lookups are deterministic**, with no jitter.
   - **No reservoir resampling** anywhere that reads the WRC: US 11,315,310 (C1).
   - **RT:** hardware rays.
   - **Non-RT:** cook-time near-field occlusion (approved), plus far-field software rays against a coarse cluster-LOD BVH. Brixelizer SDF bricks are the fallback far field.
2. **Reflections shade every hit with that surface's own program.** Programs opt in to traced reflections through an optional `reflection_lobe()`.
   - Lumen's lit-surface-cache shortcut is rejected, because it caches program output.
   - **RT:** mirror, glossy and rough bands.
   - **Non-RT, three tiers:**
     - planar views: one per plane per frame, glossy only by prefiltering that view, and only on truly flat geometry (W7);
     - the cone-sized cluster-LOD BVH (unit R1n): coupled per-pixel rays with no shared projection, traversed from the top (W8);
     - SDF bricks (unit R1n), with no per-brick hit registration or launches (W8).
3. **Temporal reconstruction revives T1b** with identity fixes and no colour statistics. It becomes the default only if it beats TSR Epic on every measure at no more than half TSR's cost.
   - T1b has never been compared with TSR, so TSR is measured first (TR0).
   - TR1 relies on engine events and surface identity.
   - Own-pixel temporal moments are off by default, as `Reconstruction::OwnMoments`, until the pending US 2026/0162231's claims settle (W4).
   - T1b's outputs (W, age, anchor bit, moments, masks) are never read downstream (W5).
   - Variance clipping, SVGF-style filtering, temporal gradients and fast/short-history designs are **evidence-based fallbacks** behind named settings, not first choices (section 9).
   - 4K upscaling is its own later unit, and ships only if it beats TSR Epic on every measure.
4. **The programmable-shading ABI grows by five optional pieces.**
   - Three engine inputs: `indirect_irradiance(input, n)`, `indirect_radiance(input, direction, roughness)` and `specular_incident(input)`.
   - Two program hooks: `reflection_lobe(s, input)`, and `scatter()`/`emission()` for bounce light.
   - Every recorded constraint is in [section 3.4](#34-recorded-programmability-constraints).
5. **Unit order.**
   - First: G0 (= M7-U0, running) and M7-U1 (merged opt-in; iterating).
   - Then: TR0, R0c (CPU only, can run now), R0a, G1, R0b, TR1, D0, G2, R1, R1n and integrated acceptance. P0 is done.
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
| Glossy parity with Unreal | 84.75% relative L1 on a roughness-0.1 metal; UE peaks 11.5-16.8x weaker. The running G0 reports UE's Vulkan SM5 half-precision N·H in the fixture capture as the cause (not yet on main); glossy comparisons use D3D12 SM6 only | — | M: [setup](unreal-comparison-setup.md) |

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
- The geometric normal joins the key as a small discrete bin; the position is never projected (C2). Cells stay cubic (C3). Capsaicin's ray direction in the descriptor is a fix for thin-occluder leaks.
- **Fixed query point (W2).** Each cell's rays start from a canonical point fixed by the key: the cube centre moved onto the surface by a deterministic probe. There is no kajiya-style voting or bounce-rank among candidates. N8n's canonical point lost on shadow contacts, so G1 measures crevice error against R_GI.

**Payload, about 40 B per cell (gi.md).**
- Directional sky visibility: L1 SH, 4 x f16 = 8 B.
- Incident irradiance: L1 SH RGB, 12 x f16 = 24 B.
- Epochs and age: 8 B. **No field counts accumulated samples (W1, C8).**

**Sizing.** 2^21 cells = 80 MiB, 2^22 = 160 MiB. **Memory stop: 160 MiB.**

**Value update (W1). G1 and D0 choose one by measurement.**
1. **Replace on update.** One work item traces the cell's full fixed direction set, reduces it in the workgroup into the K-direction quadrature, and **replaces** the value. There is no blend across updates and no counter.
   - Because each cell's directions are fixed, an unchanged cell re-traces to the same value, so stability needs no running mean.
   - Cost: K rays per update, so fewer cells are updated per frame.
2. **Per-direction slots.** Each direction's latest result is kept in its own slot and replaced when re-traced. The value is a fixed-weight quadrature over the slots.
   - This doubles as the transport-record variant below.
   - Memory: about 6-8 B x K per cell, which at K = 8 and 2^21 cells exceeds the 160 MiB stop. It fits only at fewer cells or a smaller K.

A fixed-α blend without a count is not used: P0 reads it as weaker against US 10,614,613's dependents.

**Fill and update.**
- **Direction sets.** Each cell traces K deterministic, cosine-stratified directions, rotated per cell. There are no reservoirs and no resampling (C1).
- **Scheduling (W2), with no ranking of cells.**
  - Every new or epoch-changed cell is re-traced unconditionally, with no comparison between cells. The frame is budgeted to cover it, and spikes are recorded as measured cost.
  - Other cells refresh on a **fixed rotation**: a cell refreshes when (slot + frame) mod P = 0, like N8o's per-block refresh phases.
  - Heaviest-first ordering is allowed only *within* the set that runs this frame, where it orders work and doesn't decide which rays are cast.
- **Starting ray rate** is about 0.25 rays per covered pixel, then set by the scheduling above (I).
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
- Cell values change only by replacement on engine-known events (light rate, moved casters, parameter edits) or on the fixed rotation.
- Disoccluded cells are seeded from parent levels.
- Spatial filtering runs in cell space with fixed weights, after Lumen's probe-space idea.
- The default has no screen reprojection, no SVGF-family filter, no gradient estimator and no fast history. Those exist only as the evidence-based fallbacks in 9.1.

**Lookups (W3).**
- A pixel reads its cell by fixed-weight interpolation (trilinear within a level), or by a plain nearest-cell read. G1 picks one by measurement.
- On a miss it falls back to the parent level.
- **No sub-cell jitter**: US 12,423,782 reads a jittered lookup as a stochastic spatial filter.
- A pixel's GI source is never switched between per-pixel samples and cache values by sample count or disocclusion (C5).

**Determinism.** Claim/resolve inserts (an atomicMax per slot, after which each winner writes, never per-entry serial request lists; C4), fixed directions and fixed query points, and an F8 hash identity across processes.

**Transport records (G2 variant) = W1's per-direction slots.** Each slot also keeps its ray's hit cell.
- A sun turn relights the stored hits instead of re-tracing them.
- A moved caster invalidates only the records whose segment crosses its swept bounds.

This competes with replace-on-update re-tracing on cost and memory in G2.

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
| **G-R1** Cell cache, hardware rays | RT | WRC filled by hardware rays at ~0.25 rays per covered pixel, with W1-W3 (3.1). Pixels read their cell deterministically (parent on a miss). Crevice cells never share across depth or normal breaks: N8o's reuse lesson | Design-around of US 10,614,613 and US 11,790,594 via W1-W2; US 12,423,782 via W3. US 11,315,310 lacks its resampling element (C1). US 9,754,407's claims don't reach it (P0) | Cell reuse error in crevices (N8o's Surface lost to Exact on 4 of 5 shadow measures, M) | **Traversal:** about 1/32 of Interactive's ray count, whose eight fused rays per receiver cost 0.276 ms (M), so 0.03-0.1 ms allowing 3-5x for longer, incoherent rays. **Hit shading:** 0.1-0.3 ms (2-4x the 0.396 ms full-frame shading's per-pixel cost). **Cache passes:** 0.05-0.1 ms. **Total 0.2-0.5 ms**, against L_hw/2 (8.1) |
| **G-R1n** Per-pixel near rays in addition | RT | Up to 8 short hardware rays per receiver below R_near (draft 1) | Same | Grain in motion | **+0.25-0.3 ms**: the same ray count and length class as Interactive's measured 0.276 ms. Kept only if G-R1 loses on contacts |
| **G-N1** Cook-time near field plus coarse far field | Non-RT | **Near field (Islam-approved, 50-150 MB):** per-vertex visibility of K fixed directions within R_near, against static geometry, from exact rays at cook. Movers use short software rays against their own small BLASes; masks combine by AND. **Far field:** software rays from cells against a coarse cluster-LOD BVH (R0c cook) | Same as G-R1 | Crevice bounce approximated; cook time; self-hits at the LOD/source boundary | Near lookup 0.03-0.08 ms; far fill 0.2-0.6 ms |
| **G-N1r** Runtime near field instead | Non-RT | Software rays against a CWBVH-compressed full-detail BVH at 1/16 rays per pixel, accumulating into cells (gi.md N-GI) | Same | 412.5 MB uncompressed is above the headroom; slow convergence in motion | 0.07-0.15 ms (I, scaled from N0) |
| **G-N2** SDF bricks for the far field | Non-RT | Brixelizer v1.1.4 (MIT) sparse SDF, used **for tracing only**; its screen-fed radiance cache is replaced by the WRC | Not flagged | Detail loss (far only); about 200 MB | Fallback if G-N1's far-field rays miss budget |
| G-DDGI | Both | Probe irradiance with depth moments, own code from the CC BY-ND papers or Wicked (MIT) | Not flagged | Low frequency, lag, leaks below probe spacing | Out as core; at most a cache-miss far fallback |
| ReSTIR GI/PT + world cache | Both | Reservoirs for cell updates or the per-pixel gather, reading the WRC | **US 11,315,310** and family | — | **Patented fallback only:** `GiSampling::Restir`, behind the 9.1 trigger |
| ~~Lumen-style screen probes, screen traces, GIBS surfels, GI-1.x screen probes, SDFGI, VCT~~ | — | Screen-fed, static or leaky | — | — | Rejected (standing rule) |

**ReSTIR dependency (stated explicitly).**
- The default `GiSampling::Stratified` uses no reservoir resampling in any form.
- That includes gi.md's "world-space RIS to pick which cells to update"; scheduling is W2's unconditional event re-trace plus a fixed rotation.
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
- **Variant comparison.** Replace-on-update re-trace against per-direction slots with transport records (3.1, W1), on moving-sun cost and memory.

## 5. Reflection design

### 5.1 Bands (thresholds set by measurement, not copied from Unreal's 0.4)

| Band | Roughness | RT path | Non-RT path |
|---|---|---|---|
| **B0 mirror** | below ~0.05 | 1 ray at the pixel centre, deterministic, undenoised; hit shaded by its own program | **Planar raster view** for truly flat reflectors only (W7). Otherwise the **cone-sized LOD-BVH tier** (R1n): coupled per-pixel rays, each starting at its own pixel's hit (C7), traversed from the top (W8); the hit is shaded by its own program |
| **B1 glossy** | to ~0.4-0.5 | K fixed sub-cones from `reflection_lobe`, rotated per pixel (draft 1's R-R1), each starting at its own pixel's hit. Stochastic VNDF without resampling is the variant compared. Each hit shaded by its own program | Planar view, glossy **only by prefiltering that one view** by the lobe (W7). Otherwise the LOD-BVH tier (R1n), with the cut chosen so its error stays below the lobe's blur at the hit (cone width x t) |
| **B2 rough** | above the cutoff | RLE2 prefiltered lobe x the traced-lobe ratio estimator (k = 1-2 visibility rays; reflections.md §8). Occluded rays add their hit radiance | `indirect_radiance` from WRC sky visibility and irradiance, with no per-pixel ray (default-eligible). Variant: **SDF-brick visibility rays** for the ratio estimator, sphere-traced against static Brixelizer bricks with no per-brick hit registration or launches (R1n, W8) |

**Planar views (W7).**
- **One raster view per plane per frame.** Glossy comes only from prefiltering that view by the lobe, never from repeated renders at deviated directions.
- **Jitter** the view only by the main view's mirrored sub-pixel lattice offset, for anti-aliasing. Never deviate its direction to sample a glossy lobe across frames.
- **Only truly flat reflectors:** every triangle lies in the plane within a registered tolerance. Never fit a mirrored camera to curved geometry, or per triangle.
- **Clip with the analytic plane**, never with a reflector depth map.
- **Why:** P0 finds planar views thin against US 10,565,776 and US 10,789,759. They lack only the repeated-deviated-projection element. A stronger design-around, if ever needed, shades declared mirrors with R1n's per-pixel rays at their measured cost.

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
| R-RT: traced primaries plus lobe rays plus own-program hit shading | RT | **Clear with constraint (P0):** coupled per-pixel rays from each pixel's own hit, with no common projection and no matching step (US 12,051,145). Traced primaries are not the operative avoidance. No RIS (C1) | **In plan** (R0b, R1) |
| Planar raster mirror views | Non-RT | **Thin (P0):** lacks only US 10,565,776 / 10,789,759's repeated deviated projections, kept absent by W7 | **In plan** (R1), default-eligible under W7 |
| B2 from the WRC (no per-pixel ray) | Both | No reflection rays; WRC as in 9.1 | **In plan** (R1), default-eligible |
| Cone-sized cluster-LOD BVH for B0/B1 rays | Non-RT | **Clear (P0):** per-pixel independent rays with no grouping, packets or shared frustum (US 12,051,145 (c)); global-first traversal with no per-cell hit registration (US 10,380,785). A rebuilt top level is not a claim element, so it is allowed if speed needs it | **In plan, R1n.** R0c is already building the BVH. Default-eligible on P0's mapping |
| SDF-brick visibility rays for B2 and far glossy | Non-RT | **Clear with constraint (P0), thinner:** rays are per-pixel or per-cell work items, never collected into per-brick lists or launched per brick (US 10,380,785 (a7), (b1)); W8 | **In plan, R1n.** Default-eligible under W8 |
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
| **T1b revived (TR1)** | Resampled history through the exact hit record, plus an **anchor bit** against silhouette trails, **engine-event W caps** (light rate, moved casters, parameter edits, exposure, declared program bandwidth), a fix for Catmull-Rom undershoot at silhouettes, and **no colour clamp**. Reverse-visibility validation, as T1 already has. **Relies only on engine events and surface identity** | **Clear with constraint (P0):** it practises US 10,600,167's back-projection and same-surface steps. It stays clear only while no later stage reads its auxiliary outputs (W5). No neighbourhood statistics; one value per pixel, no retained list | **In plan, first** |
| Own-pixel temporal moments | The pixel's own accepted samples over time (mean and σ) shorten W or force a reset | **At risk (P0):** every element of the pending US 2026/0162231 claim 1 as published; thin against US 12,437,371 and US 11,847,737 | **`Reconstruction::OwnMoments`, off by default**, until the continuation's claims settle (W4). If ever enabled: every validated pixel, no luminance-based selection, full-reset response only |
| **History-free boost (T-A)** | Extra primary samples only where history is missing or rejected: revealed pixels, listed discontinuities (ATAA idea, reimplemented from the paper). The settle mechanism for never-seen surfaces inside TR1, and a standalone fallback if TR1 fails | Not flagged | In plan, within TR1 |
| Two-layer silhouette history | Heavier trail fix | Not flagged | One named fix allowed by TR1's stop rule |
| A-SVGF re-shade of anchored samples | Re-shade with the old seed to detect shading change | **US 12,437,371** | **Patented fallback only:** `Reconstruction::GradientReshade` (9.1) |
| Variance clip / TAA box clip | Neighbourhood mean ± γσ, applied after identity validation | **US 10,116,916** (to 2037); US 10,362,289 | **Patented fallback only:** `Reconstruction::VarianceClip` (9.1) |
| Colour or luma rejection (TSR, FSR) | Reads output colour | Style-dependent (programmability), apart from patents | Rejected |
| Fast/slow history | A short or fast history used to clamp or reset the long one | History-clamp family (US 11,216,916, 12,299,855 and members); two value lists: US 12,536,735 | **Patented fallback only:** `Denoise::FastHistory` (9.1) |
| T2 world-anchored records with scatter | As T2b | Not flagged | Held as the film-target reference (4.25 ms, M) |
| 4K temporal upsampling | T1b at sub-native input | Same as T1b | **Later, its own unit.** Ships only if it beats TSR Epic on every measure at the same screen percentage |

**Rules.**
- **Default rule (research §6.7).** TR1 becomes the default only if it beats TSR Epic on every measured population, static and moving, with no trails or flicker beyond TSR's, at ≤ ½ of TSR's measured end-to-end cost. Otherwise it is opt-in, labelled "temporal reconstruction, below film".
- **Both paths.** Non-RT needs a jittered N1c visibility buffer that accepts Temporal. Rigid motion is exact through hit records and previous transforms on both paths. No velocity buffer is needed.
- **TR1's outputs stay internal (W5).** Its per-pixel W, age, anchor bit, moments (if enabled) and rejection mask are read only by TR1 itself, next frame.
  - No later stage takes them as inputs: no sharpen pass, GI or reflection denoiser, upscaler or other filter.
  - The revealed-pixel boost is a sampler inside TR1 and may use the rejection mask.
  - This is a recorded constraint on the later 4K upscaling unit.

### 6.1 Denoising GI and reflections

Islam's order: more samples and better caches first, then unflagged filters, then patented ones on evidence.

1. **Design-around: the WRC is the denoiser.**
   - World-space incident quantities, replaced or fixed-weight per W1, re-traced on engine events, with fixed-weight cell-space filtering.
   - More cell rays and finer cell levels.
   - Exact relight from transport records for sun and light changes.
2. **Unclaimed parts, for residual cleanup.**
   - **Edge-aware à-trous (Dammertz et al. 2010), in W6's form:**
     - a non-separable 2D 5x5 kernel at each iteration;
     - fixed kernel size and iteration count, never derived from hit distance, ray-traced data or any history count;
     - a fixed tap pattern, with no per-frame or per-pixel rotation;
     - fixed geometric edge stops (depth, normal, instance);
     - **no inputs from TR1 or from any screen-reprojecting stage**: its input is WRC output or current-frame data only.
     - If US 11,688,042's claim 9 (doubling step "in a filter direction") must be avoided entirely, use a single-radius 2D filter or the WRC's one-pass cell-space filter instead.
   - **One history:** one value per pixel or cell, with no fast history and no retained list of previous values.

   Either must settle within 8 frames, and both are tested in D0. Claim notes are in 9.1.
3. **Patented fallbacks, on evidence only:**
   - `Denoise::Svgf`, a Falcor SVGF port (BSD-3, notice kept): US 10,600,167;
   - `Denoise::FastHistory`, a ReBLUR-like fast plus slow history: US 11,216,916, US 12,299,855, and US 12,536,735 if it keeps two value lists;
   - `Denoise::ValidationRays`, kajiya-style secondary-ray gradients: probably the pending US 2026/0162231 if it issues, and US 12,437,371 if validated pixels are chosen by lighting data.

## 7. Units and order

Sizes are rough agent estimates (I), given as implementation / GPU verification.

| # | Unit | Content | Depends on | Size | Gates and stop rules |
|---|---|---|---|---|---|
| 1 | **G0 = M7-U0** (running) | Unreal denominators (section 8): GI HW/SW x High/Epic/Cinematic, hit lighting, mesh SDFs on and off. Sun-step and yaw-step settle curves. Memory with Lumen on. **R_GI** references from the in-process GPU path tracer, environment-only and full, at quarry poses 0/1 and S1-S3, cross-checked with pbrt or Mitsuba on two crops. The glossy 84.75% disagreement: reported as UE Vulkan SM5 half precision, to be confirmed in G0's record | — | running | Every Unreal configuration's effective settings read back, or it is not scored. R_GI agrees with pbrt/Mitsuba within a registered tolerance |
| 2 | **M7-U1** (merged opt-in; iterating) | Specular AA (5.3): second moments and lobe shape next | G0's glossy item, for the Unreal comparison only | running | Level F gain on every population |
| 3 | **TR0** | TSR Epic and Cinematic, `r.AntiAliasingMethod=4`, `r.ScreenPercentage=100`, plus a 1x row and an informative 66.662% row, on Q-walk, Q-disocclusion and Q-sun. Scored with `scripts/m6-temporal-motion-score.py` (from `b8fa1f3`) on T1b's populations: F1, F2 by age, trail width, background-centre error, hold flicker. End-to-end A/B timing in one process. Also settles whether TSR leaks at shadow edges | Unreal harness; batch with G0 if its session is still open | 3-6 h / 1-2 h | Output: TSR's error per population as a multiple of the bar, and its end-to-end ms |
| 4 | **R0c** (CPU only, can run now) | Cook the cone-sized cluster-LOD BVH for the quarry at two error bounds: bytes and build time. Report the per-mesh BVHs as **static** (built once, never refit). CPU-trace 10,000 RF1 reflection rays and 10,000 GI cell rays against the N0 full-detail oracle: hit differences against footprint | — | 4-8 h / 0 | Feeds G-N1's far field and R1n's B0/B1 tier |
| 5 | **R0a** | Reflection fixtures RF1-RF10 (8.2) in both engines. UE-HW-Hit, UE-HW-SC and UE-SW at Epic and Cinematic: ProfileGPU nodes, images, memory. Roughness histogram of the plain quarry | G0 harness (same batch where possible) | 4-8 h / 2-3 h | Effective readback, as G0 |
| 6 | **P0** (done) | Primary-claim mapping of 53 US documents: [m7-p0-claims.md](m7-p0-claims.md). Adopted as W1-W8 and C1-C8 | — | done | — |
| 7 | **G1** | WRC v1 with W1-W3 (replace or slots; event re-trace plus fixed rotation; fixed query point; deterministic lookup), `indirect_irradiance` for sky only, G-R1 and G-N1 (near precompute + far LOD BVH), both paths, `GiSampling::Stratified` | G0 (R_GI, L), R0c | 16-30 h / 3-5 h | gi.md's G1 gates and stop rules, plus the measured cost of each W1-W3 choice (8.3) |
| 8 | **R0b** (test-only) | R-RT B0 and B1 at 1 ray per pixel on RF1/RF2, no denoiser, static accumulation. Trace and hit-shading cost; error against the glossy film reference. **Mirror-camera identity gate** on the standard and toon programs | R0a, U1 | 8-12 h / 2 h | Any identity failure is a contract bug and blocks everything else |
| 9 | **TR1** | T1b revived (section 6) on both paths, including the jittered visibility buffer and the revealed-pixel boost. Engine events and surface identity only; `Reconstruction::OwnMoments` stays off; outputs stay internal (W5) | TR0, U1 | 12-20 h / 3-4 h | Section 8.3. If a population is still worse than TSR after the anchor bit plus one named fix (two-layer history or the ±1 px reverse search), try the min/max box over identity-validated taps (9.1 B), then record the per-measure numbers; that record is `Reconstruction::VarianceClip`'s trigger input. Stop if the cost delta is more than ½ of TSR's after one optimisation pass. If TSR is worse than T1b everywhere, gate T1b as is |
| 10 | **D0** (CPU replay) | 1-spp incident-irradiance dumps on Q-walk, Q-sun and Q-disocclusion. Compare: (a) WRC values per W1 (replace, or slots), re-traced on events; (a′) (a) plus fixed-weight cell-space filtering; (b′) W6 à-trous on the residual; (c) a sample-count control at 2x and 4x rays | G1 dumps | 4-8 h / 1 h | Drop a filter that cannot reach the bar by frame 8 after each event, or whose GPU estimate is above ½ of Lumen's measured denoise share. If none passes, spend on samples or caches; the records feed `Denoise::*` triggers |
| 11 | **G2** | Bounce: `scatter`/`emission`, sun one-bounce, cache-terminated multi-bounce. Replace-on-update against per-direction slots with transport records (W1) | G1, D0 | 12-24 h / 3-5 h | All GI measures, including S4 events and off-screen bounce |
| 12 | **R1** | Reflections: R-RT B0-B2, `reflection_lobe`, `specular_incident`, hit shading by program; non-RT planar tier under W7, and WRC B2 | R0b, G1, U1 | 16-30 h / 3-5 h | 8.3. If RT cost is above 0.5x UE-HW-Hit's "Lumen Reflections" after three hillclimb mechanisms, record the miss |
| 13 | **R1n** | Non-RT B0/B1 through the LOD-BVH tier: coupled per-pixel rays from each pixel's own hit against R0c's BVHs, traversed from the top, with no per-cell hit registration or launches. Movers are per-instance BVHs; a rebuilt top level is allowed if speed needs it. B2 variant: SDF-brick visibility rays (W8). Same hit contract and programs as R1 | R0c, R1 | 16-28 h / 3-5 h | All RF measures against UE-SW Epic. If LOD-BVH rays cost more than 3x hardware rays on the same rays after three hillclimb mechanisms, record the speed miss (not a patent trigger). A quality failure on every tier is the `CoherentHybrid` trigger input |
| 14 | **Integrated acceptance** | Every gate on the integrated build, both paths, against the Epic floor; 2x end to end; memory; quiet window; independent Opus review | all | 6-10 h / 4 h | 8.3 |
| 15 | **Upscaling** (later) | 4K output | TR1 | — | Ships only if it beats TSR Epic on every measure |

**Why this order.**
- G0 and U1 are already running.
- TR0 and R0c are cheap. TR0 decides whether T1b already beats TSR; R0c sizes the non-RT far field.
- G1 is the first Renderlab unit. It is lighting-invariant, runs on both paths, needs no `scatter` contract, and its lag gate is clear.
- R0b proves the hit contract before R1 builds on it.
- D0 picks the GI accumulation before G2 commits to one.
- R1n follows R1 because it reuses R1's hit contract and programs, and R0c's BVH. Its tiers are default-eligible on P0's mapping, under W8 and C7.

**Parallel ownership.**
- G1 and R0c are CPU-heavy and separable. G1 owns `gi_cache*` and the cook visibility module; R0c owns the LOD-BVH cook.
- R1n owns the new `reflection_nonrt*` files.
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

**Reported by the running G0, to be confirmed from its record (not yet on main).**
- Lumen runs need `r.EyeAdaptation.PreExposureOverride 0`. The earlier captures' override of 1 clamps traced light to black.
- The 84.75% roughness-0.1 disagreement comes from UE's Vulkan SM5 half-precision N·H in the fixture capture. Glossy comparisons therefore run on the packaged D3D12 SM6 build only.

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
| G1 | **Error against R_GI(env):** RMS, crevice pixels, over-occlusion of open pixels, flicker in motion. **Against Unreal:** UE-sky at Epic, HW for RT and SW for non-RT. **Determinism and settle:** hash-equal across processes; a caster move settles byte-identical to fresh; yaw and intensity steps equal fresh on the next frame (0 frames lag). **Stop rules:** after 3 retained mechanisms still losing a measure; RT camera-moving cost above 0.3 x L_hw; non-RT above L_sw/2 continues only as quality-first and is recorded. Disoccluded first-frame error ≤ UE's first frame. **The patent design-arounds' own cost, measured and recorded per choice, on both paths:**<br>• W1, replace against slots: settle by frame 8, flicker, update ms per frame, slot memory;<br>• W2, unconditional event re-trace: p95/p99 spike on caster, sun and cut events (F9);<br>• W2, fixed-rotation refresh: staleness error by cell age;<br>• W2, fixed query point: crevice error against R_GI, the N8n P1 risk;<br>• W3, trilinear against nearest lookup: cell-seam error on flat surfaces, and flicker in motion.<br>A choice that fails a gate is the trigger input for its `GiCache::*` fallback (9.1 A) |
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
- **Claim elements** are P0's reading of each patent's independent claims ([m7-p0-claims.md](m7-p0-claims.md) §3-§4), which replaces the research summaries. Letters such as (e)-(g) are P0's.
- **"Avoids element X"** means the M7 part lacks X. That is an engineering reading, not a clearance.
- **"Thin"** means a part lacks only one element, so the constraint that keeps it absent is binding.

**Fallback rule (policy step 3).**
- A patented method lives only behind its named setting.
- Its default is off until the trigger evidence is recorded in `docs/milestone-7-decisions.tsv` with:
  - the patent number;
  - every failed design-around and unclaimed-part candidate, with its per-measure numbers on both paths;
  - the fallback's numbers on the same fixtures and frames.
- The trigger is always a quality failure against the Unreal Epic floor, static or moving, on either path, or a clearly worse final image.
- **A speed-only miss is recorded and never triggers a fallback.**

**A. GI cache: US 10,614,613, US 11,790,594, US 12,423,782 and US 11,315,310.**

| | Content |
|---|---|
| Claim elements (P0) | **US 10,614,613 cl. 1** (path-space filtering by hashing, to ~2038): (a) a query point at a path vertex; (b) position quantized to a cell, plus side and normal; (c) hashed; (d) fingerprint and insert; (e) samples' radiance accumulated into the cell; (f) a count; (g) an average. **US 11,790,594 cl. 1** (irradiance caches, to ~2041): (a) cache locations ranked by characteristics such as age, under a ray budget; (b) rays cast from the higher-ranked locations; (c) irradiance from the hits; (d) the frame rendered from the cache. **US 12,423,782 cl. 1:** jittered tap locations in a spatial filter. **US 11,315,310 cl. 1:** (a) reservoir importance resampling; (b) GI on visible surfaces; (c) a world-space data structure |
| As drafted in 2.1 | **Appeared to practise every element** of US 10,614,613 (counter and average) and US 11,790,594 (age priority, bounce-rank voting) |
| **Design-around (W1-W3)** | **W1:** replace on update, or per-direction slots with fixed weights. This lacks (f)-(g), and (e) as well in the replace form. **W2:** unconditional re-trace of new and changed cells, a fixed rotation for the rest, and a fixed query point. This lacks 11,790,594's ranking element (a) and its "higher-ranked" in (b). **W3:** deterministic lookups, lacking 12,423,782's jitter (a). **C1:** no reservoir or RIS anywhere in GI or reflections that reads or writes the WRC, including cell-update selection. This lacks 11,315,310's (a), while the WRC plainly is its (c) |
| **Unclaimed parts used** | Hashed cubic cells with fingerprint and claim/resolve inserts: 10,614,613 (a)-(d) without (e)-(g). A world-space cache without resampling: 11,315,310 without (a). Resampling alone is not a usable part, because the rest of the ReSTIR family claims resampling itself |
| Constraints | C1 above. **C2:** unprojected 3D key position (US 11,925,860). **C3:** cubic cells (US 11,756,254 / 12,374,024). **C4:** atomicMax plus winner-writes inserts (US 11,609,899 family). **C5:** no per-pixel switch between samples and cache by sample count or disocclusion (US 2024/0029338). **C8:** the shadow cache never accumulates with a counter. Cook-time near field: clear (P0 §4.1) |
| Not exposed | **US 9,754,407 / 9,747,718** claim per-triangle object-space shading grids. Cube cells from `floor(world/size)` lack their (b) and (c), for both the WRC and the N8o shadow cache (P0 §4.13) |
| **Patented fallbacks** | `GiCache::CountedAverage` (US 10,614,613); `GiCache::RankedUpdates` (US 11,790,594); `GiCache::JitteredLookup` (US 12,423,782); `GiSampling::Restir` (US 11,315,310, plus US 12,361,635 and 11,948,246 / 12,646,247 depending on reuse) |
| Trigger evidence | **CountedAverage:** both W1 forms fail settle, flicker or noise against Unreal Epic after more rays and a finer level. **RankedUpdates:** event re-trace plus rotation fails the 8-frame settle or response gates in event storms, where ranked scheduling passes on the same sequence. Spikes alone are a recorded speed cost, not a trigger. **JitteredLookup:** trilinear and nearest both leave cell seams or blockiness that lose to Unreal on a measure. **Restir:** G1/G2 still lose a measure after their three mechanisms, 2x rays and a finer level, and a replay of the same rays shows the reservoir variant passing |

**B. Temporal history: US 10,116,916, US 10,362,289, the history-clamp family, US 12,437,371 and pending US 2026/0162231.**

| | Content |
|---|---|
| Claim elements (P0) | **US 10,116,916:** history adjusted by a box from the neighbourhood's per-component mean and σ; P0 confirms the summary. **US 10,362,289:** a box that excludes at least one neighbourhood sample. **History-clamp family** (11,216,916; 11,232,544; 11,783,455; 12,299,855; pending 2026/0051032): a fast or short history used to clamp the full one. **US 12,437,371:** gradients from re-shaded samples, chosen by lighting data. **US 2026/0162231 (pending)** drops the lighting-based selection |
| **Design-around** | `Reconstruction::Identity` (TR1): engine events and surface identity only, a single history and no neighbourhood statistics. It practises US 10,600,167's back-projection and same-surface check, so it stays clear only under W5 (C below) |
| **Unclaimed part** | A **plain min/max box over identity-validated taps**. It lacks 10,116,916's mean/σ element, avoids 10,362,289 because no sample is excluded, and with one history avoids every history-clamp claim read. It reads output colour, which is a programmability problem, so it is the last option and must pass on the standard, toon and flat programs. A box shrunk toward the mean would *not* avoid 10,362,289 |
| **Watch-listed, off by default** | `Reconstruction::OwnMoments` (W4). It practises every element of the pending US 2026/0162231 claim 1 as published, and is thin against 12,437,371 and 11,847,737. It stays off until that continuation's claims settle. If enabled: every validated pixel, and full-reset responses only |
| **Patented fallbacks** | `Reconstruction::VarianceClip` (US 10,116,916); `Reconstruction::GradientReshade` (US 12,437,371, plus US 2026/0162231 if granted) |
| Trigger evidence | **VarianceClip:** TR1, one named fix (two-layer history or the ±1 px reverse search) and the min/max box still lose to TSR Epic on a registered population on either path (trail energy, ghost bias F5, shading-change lag). A replay of saved TR1 buffers must show the clip passing on the standard, toon and flat programs. **GradientReshade:** shading change that no engine event covers fails F2 or F5 after declared-bandwidth W caps |

**C. Denoisers: US 10,600,167, US 11,847,737, US 11,113,792, US 11,688,042 and US 12,423,782.**

| | Content |
|---|---|
| Claim elements (P0) | **US 10,600,167 cl. 1 has no variance element.** It is screen back-projection, a same-surface check, and auxiliary data passed to subsequent filters. **US 11,847,737 cl. 1 has no dual-history element.** It is temporal lag judged from pixel information, plus a subset of previous values; dual lists are in its divisional US 12,536,735. **US 11,113,792:** filter size or taps derived from render data or history counts. **US 11,688,042:** separable multi-iteration sub-kernels; cl. 9, a step that doubles "in a filter direction", is thin. **US 12,423,782:** jittered taps |
| **Design-around** | The WRC as denoiser (6.1): world-space, with no screen back-projection, so it lacks 10,600,167's (a1)-(a4). Engine-event re-trace and exact record relight: no lighting difference is estimated (12,437,371, 2026/0162231) |
| **Unclaimed parts and constraints** | **W5:** TR1's auxiliary outputs are never read downstream, which keeps 10,600,167's (b)-(c) absent. This, not "no variance guidance", is the operative avoidance. **W6 à-trous:** 2D non-separable (11,688,042); fixed size and iteration count (11,113,792); fixed taps (12,423,782); no reprojected inputs (10,600,167). **One history:** W from engine state and one value per pixel or cell, lacking 11,847,737's (c)-(d), with one ambiguity for W-capped blends |
| **Patented fallbacks** | `Denoise::Svgf` (US 10,600,167; Falcor port, BSD-3 notice kept); `Denoise::FastHistory` (US 11,216,916, 12,299,855; 12,536,735 if two lists); `Denoise::ValidationRays` (probably US 2026/0162231 if it issues) |
| Trigger evidence | Islam's order. D0's WRC and W6 filters fail the bar by frame 8 after an event, or lose to Unreal on contact blur or flicker. Then the 2x and 4x sample control and a finer cell level also fail. Only then is the matching patented filter measured on the same dumps and adopted if it passes |

**D. Reflections: Adshir/Snap and ReSTIR.**

The Adshir/Snap family is US 10,699,468, **US 10,950,030**, US 11,302,058, **US 12,051,145**, US 12,573,127 and pending US 2025/0363721, plus these CIP parents:
- US 10,565,776 and 10,789,759;
- US 10,614,612 and 10,930,053;
- US 10,614,614, 10,380,785 and 10,818,072;
- US 10,410,401 and 10,332,304.

| | Content |
|---|---|
| Claim elements (P0) | **No independent claim recites a dynamic acceleration structure.** That phrase comes from the specs' summary; in the 2015 family "DAS" means a "dynamically aligned structure". The operative elements: **12,051,145:** secondary rays decoupled from primaries, handled as a common projection, results matched back. **10,699,468 / 10,950,030 / 11,302,058:** per-reflector-triangle projection with object triangles transformed and shaded onto it. **12,573,127:** a phantom camera on a non-planar surface. **10,565,776 / 10,789,759:** carrier-ray projections into a render target, repeated at deviated directions (g). **10,380,785:** per-cell local structures with registered hits and per-cell launches. Raster primaries appear in several claims but are not decisive. **ReSTIR:** if any sampling is RIS, 11,315,310 reads on it, because reflection hits read the WRC |
| **Design-around** | **RT and LOD-BVH:** coupled per-pixel rays from each pixel's own hit (C7), with no common projection and no matching, so they lack 12,051,145's decoupling, (c) and (d). Traced primaries are not the operative avoidance. **Glossy:** deterministic sub-cones, or plain importance sampling (VNDF, BRDF, environment PDF) without resampling (C1). P0 finds the earlier US 12,361,635 worry unfounded |
| **Unclaimed parts (R1n, default-eligible)** | **LOD-BVH tier:** global-first traversal, with no per-cell hit registration or per-cell launches (10,380,785). A static cook-time BVH is not an operative avoidance, and a rebuilt top level is allowed. **SDF-brick tier:** no per-brick hit lists or per-brick launches (10,380,785 (a7), (b1)); thinner (W8) |
| **Thin part (W7)** | **Planar views** lack only 10,565,776 / 10,789,759's (g). So: one view per plane per frame; glossy only by prefiltering; flat geometry only; mirrored-lattice jitter only; analytic-plane clip (10,332,304) |
| **Patented fallbacks** | `NonRtReflections::CoherentHybrid` (US 12,051,145 cl. 1: decoupled rays, common projection, matching; also the per-reflector-triangle forms); `ReflectionSampling::Restir` (US 11,315,310 plus 11,948,246 / 12,646,247 / 12,361,635) |
| Trigger evidence | **CoherentHybrid:** the planar, LOD-BVH and SDF-brick tiers all fail a registered RF measure against UE-SW Epic, such as RF1 off-screen mirror error, RF4 thin features or RF3/RF5 motion. A cost miss alone never triggers it. **Restir:** B1 with K = 8 sub-cones, then 2x rays, still fails RF2's band error or RF3/RF5 motion against Unreal Epic on either path |

### 9.2 Other flags and unflagged techniques

| Technique | Exposure | In this design |
|---|---|---|
| History clamping | Fast/short-history designs only (P0) | Any single-history design avoids it; only `Denoise::FastHistory` touches it |
| Object-space shading grids | US 9,747,718, US 9,754,407 | Not reached by cube cells (P0 §4.13) |
| Future local lights | US 11,663,773, 12,299,801, 11,948,246 / 12,646,247 / 12,361,635 | **C6:** no random set-then-subset presampling, no per-cell sampled light lists, and no reuse of prior-frame or neighbour light choices |
| DDGI, ratio estimator, ATAA, ray cones, Toksvig/LEAN, Kaplanyan-Tokuyoshi, Dammertz à-trous (W6 form) | Not flagged | Used as described (no clearance) |

P0 §8 lists the ambiguous terms that decide some verdicts, and notes that the WRC's exposure came from three web searches, not a structured search. A broader search is advisable before any commercial claim.

### 9.3 Licences

| Rule | Items |
|---|---|
| **Reimplement only: read docs, copy no code** | Epic, Unity, RTXGI 2.x, SHaRC, NRC, NRD, RTXDI, DLSS / DLSS-RR, XeSS, FSR 4, FidelityFX 2.x binaries |
| **Own code from the text** | DDGI papers (CC BY-ND); RTG ch. 25 (CC BY-NC-ND) |
| **Copy with notice** | Bevy Solari's world-cache files only (`world_cache_{query,update,compact}.wesl`, MIT/Apache; never its ReSTIR files); kajiya ircache (MIT); Capsaicin hash cells and leak fix (MIT; not `world_space_restir.hlsl`); Falcor `TexLODHelpers` (BSD-3); Filament `normalFiltering` (Apache-2.0); obvhs and tray_racing (MIT/Apache); Brixelizer, FidelityFX SDK pinned at v1.1.4 (MIT); Wicked DDGI (MIT) |
| **Before copying any cache code** | Strip whatever W1-W3, C1 and C6 rule out from Solari's world cache, kajiya's ircache and Capsaicin's hash cells: counter-and-average blends (10,614,613), age- or visibility-ranked updates (11,790,594), any reservoir use (11,315,310) and light-tile presampling (11,663,773). Their code was not read by P0 |
| **Tools** | pbrt-v4, Mitsuba 3 |
| **Copyable, patented: fallback settings only** | Falcor SVGF (BSD-3) as `Denoise::Svgf`; the FidelityFX reflection denoiser's temporal clip (MIT) is near US 10,116,916 and is not used |

## 10. Rulings applied

**From milestone-7.md.**
- **Patents:** P0's recorded mapping is enough for a design-around or unclaimed-part tier to become a default, with no separate human review first (this answers draft 2.1's question). Islam may commission a legal review at any time.
- **GI cache:** default-eligible under W1-W3 and C1-C5. P0 finds US 9,754,407 does not reach it.
- **Adshir/Snap:** superseded by the patent policy; the non-RT tiers are in R1n.
- **Denoisers:** samples and caches first, then unflagged filters, then fallbacks on evidence.
- **`reflection_lobe`:** GGX-shaped, accepted (3.4).
- **DLAA:** an informative row only.
- **Cache code:** copied into M7 files, possibly merged after the hillclimb.
- **RT baseline:** 2,641 MiB.

No new questions for Islam.

## Evidence and time

Reading and design only:
- the three research notes and milestone-7.md's decisions;
- the M5-DL, N8p and memory records;
- M6-close's acceptance record (measured Unreal shadow and frame costs; Interactive's 0.276 ms) and the patent policy (draft 2.1);
- P0's claim mapping, sections 4-7 (draft 2.2);
- draft 1 and its inputs.

Of the running M7-U0 and M7-U1 worktrees, only their branch names and changed-file lists were seen. None of their uncommitted work was read or used. No implementation, verification, GPU or Unreal time was spent.
