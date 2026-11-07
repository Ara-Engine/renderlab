# M7 design: global illumination, reflections and temporal reconstruction

Design worker record, 2026-11-07, base main `476cdfe`. Contract and decisions: [milestone-7.md](milestone-7.md). This is CPU-only design: no implementation, build, GPU run or Unreal launch.

**Status: draft 2.** It is grounded in the research notes [gi.md](research/gi.md), [reflections.md](research/reflections.md) and [reuse-temporal-denoise.md](research/reuse-temporal-denoise.md) §6-§11. No research placeholders remain. Islam's and root's answers in milestone-7.md are applied.

**Patent rule (Islam).** Every technique the research flags stays out of defaults and out of the unit plan until Islam rules on it. Where a flagged technique would otherwise fit, this draft gives a variant that avoids it and states the dependency. "Not flagged" means the research found no patent, which is not a clearance ([section 9](#9-patents-and-licences)).

Labels: **M** is measured in the cited record, **A** is author-reported, **I** is an inference or estimate.

## Summary

1. **One world radiance cache (WRC) on both paths.** It is a second payload table on N8o-style surface-cell keys. Lighting-invariant directional sky visibility comes first, then cached incident irradiance.
   - The cache is filled by deterministic direction sets and cache-terminated rays. **There is no ReSTIR anywhere, in screen or world space**, which avoids US 11,315,310's ReSTIR-plus-cache combination.
   - **RT:** hardware rays.
   - **Non-RT:** cook-time near-field occlusion (approved), plus far-field software rays against a coarse cluster-LOD BVH. Brixelizer SDF bricks are the fallback far field.
2. **Reflections shade every hit with that surface's own program.** Programs opt in to traced reflections through an optional `reflection_lobe()`.
   - Lumen's lit-surface-cache shortcut is rejected, because it caches program output.
   - **RT:** mirror, glossy and rough bands.
   - **Non-RT:** planar views are the default-eligible tier. The cone-sized LOD-BVH and SDF-brick tiers are designed, but held from the plan until Islam rules on the Adshir/Snap hybrid-reflection patents.
3. **Temporal reconstruction revives T1b** with identity fixes and no colour statistics. It becomes the default only if it beats TSR Epic on every measure at no more than half TSR's cost.
   - T1b has never been compared with TSR, so TSR is measured first (TR0).
   - Variance clipping, SVGF-style filtering, temporal gradients and dual fast/slow history are excluded as patent-flagged.
   - 4K upscaling is its own later unit, and ships only if it beats TSR Epic on every measure.
4. **The programmable-shading ABI grows by five optional pieces.**
   - Three engine inputs: `indirect_irradiance(input, n)`, `indirect_radiance(input, direction, roughness)` and `specular_incident(input)`.
   - Two program hooks: `reflection_lobe(s, input)`, and `scatter()`/`emission()` for bounce light.
   - Every recorded constraint is in [section 3.4](#34-recorded-programmability-constraints).
5. **Unit order.** G0 (= M7-U0, running) and M7-U1 (specular AA, running) first. Then TR0, then R0c (CPU only, can run now), then G1, then R0b, TR1, D0, G2, R1 and integrated acceptance. Upscaling comes last. See [section 7](#7-units-and-order).

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
| Release frame (direct light, environment, shadows), static / camera / caster | ~0.87 ms in every phase | 0.73 / 1.79 / 2.37 ms, with the Surface lookup as default | M: [M5-DL](m5-dev-loop.md), [N8p](non-rt-n8p.md) |
| Against recorded Unreal GPU (same subset, Lumen and TSR off) | HRT: 2.02x | VSM: static 2.19x; camera 0.89x (a miss) | M: M5-DL |
| VRAM, allocator reserved, against Unreal DXGI | 2,641 MiB (traced + Interactive) against HRT 5,410. The RT device with Virtual lookups reads 4,138-4,202 MiB, so G0 settles which configuration M7 uses | 3,503.7 against VSM 3,912 (408 MiB headroom) | M: [memory M1](memory-m1.md), N8p |
| Sky and environment | Prepared RLE2 fields, unoccluded (`shaders/programs/standard.wgsl:27-33`) | same | M: [M5-EV](m5-environment-visibility.md) |
| Specular AA | none: normal mips renormalized; `alpha = max(rough², 1e-4)` unfiltered. M7-U1 is running | same | M: [M5-GM](m5-glossy-motion.md) |
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
- There is no screen reprojection, no SVGF-family filter, no gradient estimator and no dual history (section 9).

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

1. `SpecularLobe` is GGX-shaped. A non-GGX style (anisotropic, toon-banded) gets its nearest GGX lobe, or none. **Needs Islam's acceptance (Q4).**
2. Bounce light sees a program only through `scatter`/`emission`, evaluated per cell and view-independent. That is exact for Lambert. Glossy interreflection at secondary hits is approximated. Programs without `scatter` reflect no bounce.
3. Programs without `reflection_lobe` get no traced reflections, only `indirect_radiance`.
4. Hit shading runs `shade()` with ray-cone footprints and no screen access, as traced primaries already do.
5. A filter guide for glossy denoising, if one is ever adopted, comes from `reflection_lobe`. No albedo export is ever required.
6. The WRC caches incident quantities: sky visibility and irradiance. The only program-output-derived value is the `scatter`-integrated outgoing radiance at secondary cells, keyed by program epoch.

## 4. GI design

### 4.1 Sketches, revised from gi.md

| Sketch | Path | Mechanism | Patent status | Risk | Cost (I) |
|---|---|---|---|---|---|
| **G-R1** Cell cache, hardware rays | RT | WRC filled by hardware rays at ~0.25 rays per pixel. Pixels read their cell (parent on a miss) with sub-cell jitter. Crevice cells never share across depth or normal breaks: N8o's reuse lesson | WRC: US 9,754,407 review open. No ReSTIR | Cell reuse error in crevices (N8o's Surface lost to Exact on 4 of 5 shadow measures, M) | 0.25-0.5 ms + 0.05-0.1 ms of cache passes, against an estimated L_hw/2 ≈ 0.5-1.2 |
| **G-R1n** Per-pixel near rays in addition | RT | Up to 8 short hardware rays per receiver below R_near (draft 1) | Same | Grain in motion | +0.10-0.25 ms. Kept only if G-R1 loses on contacts |
| **G-N1** Cook-time near field plus coarse far field | Non-RT | **Near field (Islam-approved, 50-150 MB):** per-vertex visibility of K fixed directions within R_near, against static geometry, from exact rays at cook. Movers use short software rays against their own small BLASes; masks combine by AND. **Far field:** software rays from cells against a coarse cluster-LOD BVH (R0c cook) | Same as G-R1 | Crevice bounce approximated; cook time; self-hits at the LOD/source boundary | Near lookup 0.03-0.08 ms; far fill 0.2-0.6 ms |
| **G-N1r** Runtime near field instead | Non-RT | Software rays against a CWBVH-compressed full-detail BVH at 1/16 rays per pixel, accumulating into cells (gi.md N-GI) | Same | 412.5 MB uncompressed is above the headroom; slow convergence in motion | 0.07-0.15 ms (I, scaled from N0) |
| **G-N2** SDF bricks for the far field | Non-RT | Brixelizer v1.1.4 (MIT) sparse SDF, used **for tracing only**; its screen-fed radiance cache is replaced by the WRC | Not flagged | Detail loss (far only); about 200 MB | Fallback if G-N1's far-field rays miss budget |
| G-DDGI | Both | Probe irradiance with depth moments, own code from the CC BY-ND papers or Wicked (MIT) | Not flagged | Low frequency, lag, leaks below probe spacing | Out as core; at most a cache-miss far fallback |
| ~~ReSTIR GI/PT + world cache~~ | — | — | **US 11,315,310** | — | **Excluded from the plan** |
| ~~Lumen-style screen probes, screen traces, GIBS surfels, GI-1.x screen probes, SDFGI, VCT~~ | — | Screen-fed, static or leaky | — | — | Rejected (standing rule) |

**ReSTIR dependency (stated explicitly).** No GI unit uses reservoir resampling in any form. That includes gi.md's "world-space RIS to pick which cells to update"; cell priority is a deterministic queue. Adding ReSTIR to anything that reads the WRC would form US 11,315,310's combination. That needs Islam's ruling before it is designed further.

### 4.2 Sky first (G1)

- Occluded sky is likely the largest missing outdoor term (I, gi.md).
- It is lighting-invariant, so the sky term should beat Lumen's "multiple seconds" lighting lag outright: yaw and intensity steps must equal a fresh run on the next frame.
- It exercises the cache, the ABI and both producers without the `scatter` contract.
- The [M5-EV handover](m5-environment-visibility.md)'s candidates map onto it: its precomputed visibility is G-N1's near field, its surface-cache idea is the WRC, and the single-direction clipmap and grid are excluded.

### 4.3 Bounce (G2)

- **Sun one-bounce plus cache-terminated multi-bounce**, with `scatter`/`emission`.
- **Sun visibility at hits.** RT: one hardware shadow ray. Non-RT: requested page marking at hits, or a coarser page level at hits if it passes the gate (gi.md risk 5).
- **Variant comparison.** Priority re-trace against transport records (3.1), on moving-sun cost and memory.

## 5. Reflection design

### 5.1 Bands (thresholds set by measurement, not copied from Unreal's 0.4)

| Band | Roughness | RT path | Non-RT path |
|---|---|---|---|
| **B0 mirror** | below ~0.05 | 1 ray at the pixel centre, deterministic, undenoised; hit shaded by its own program | **Planar raster view** for declared planes (N1c from the mirrored camera). Non-planar mirrors use the held LOD-BVH tier |
| **B1 glossy** | to ~0.4-0.5 | k lobe rays from `reflection_lobe`: either stochastic VNDF, or K fixed sub-cones rotated per pixel (draft 1's R-R1). Each hit shaded by its own program | Planar view, prefiltered by the lobe, where planar. Otherwise the held LOD-BVH tier |
| **B2 rough** | above the cutoff | RLE2 prefiltered lobe x the traced-lobe ratio estimator (k = 1-2 visibility rays; reflections.md §8). Occluded rays add their hit radiance | `indirect_radiance` from WRC sky visibility and irradiance, with no per-pixel ray: default-eligible. The SDF-brick visibility-ray variant is held |

**Glossy temporal integration (patent-safe variant).**
- reflections.md's B1 denoiser used a variance clamp, which is flagged under US 10,116,916. It also used FidelityFX's mean ± σ temporal clip, which needs a claims check. Both are out of the plan.
- **Instead:**
  - B1 spends deterministic sub-cone rays, with no screen denoiser.
  - Temporal integration comes only from the final-image reconstruction (TR1). It validates history by identity: the reflected hit's instance, primitive and reverse visibility, with no colour statistics.
- If B1 still fails the motion gates, it stays opt-in and is labelled below the bar. Any SVGF- or clamp-class filter waits for Islam's ruling.

### 5.2 Tiers and patent status

| Tier | Path | Patent status | Plan status |
|---|---|---|---|
| R-RT: traced primaries plus lobe rays plus own-program hit shading | RT | Adshir/Snap is worded as raster primaries plus reflection rays; RT primaries are traced, which is outside that wording (I, claims unread). No ReSTIR | **In plan** (R0b, R1) |
| Planar raster mirror views | Non-RT | No rays; outside the flagged combination as worded (I) | **In plan, default-eligible** |
| B2 from the WRC (no per-pixel ray) | Both | WRC review open (US 9,754,407) | In plan |
| Cone-sized cluster-LOD BVH for B0/B1 rays | Non-RT | **Adshir/Snap family** (US 10,950,030 B2 and members): raster primaries plus reflection rays | **Designed, held.** R0c's CPU LOD-BVH cook proceeds, because the same BVH serves G-N1's GI far field (rays from cells, not reflections) |
| SDF-brick visibility rays for B2 and far glossy | Non-RT | Adshir/Snap (same combination) | **Designed, held** |
| ReSTIR for glossy | Both | NVIDIA ReSTIR family | **Excluded** |
| Lumen-style lit surface cache at hits | Both | — | **Rejected:** caches program output (breaks programmability) |
| SSR, or SSR verified in world space | Both | — | Rejected as core. The verified-hint cost lever is not planned |

**Consequence.** Until Islam rules on Adshir/Snap:
- non-RT non-planar sharp reflections have no in-plan producer;
- the non-RT reflection default is planar views plus WRC-based B2.

Lumen software's mirrors are weak ("HWRT is the only way to high-quality mirrors", V). Whether the non-RT floor holds on RF2-RF4 without the held tiers is G0's measurement to make. This is Q2.

### 5.3 Specular AA (M7-U1, running)

- **Toksvig/LEAN** variance kept at cook (`crates/cook/src/normal_map.rs`).
- **Kaplanyan/Tokuyoshi** widening from ray-differential normal variance (Filament `normalFiltering`, Apache-2.0).
- **Geometric multi-scale vertex variance** from the DAG, for sub-pixel triangle relief.
- **Acceptance:** keep only on a Level F film-error gain on every population, static and moving.
- It also reduces how many pixels need B0/B1 rays (reflections.md §8).

## 6. Temporal reconstruction design

| Sketch | Mechanism | Patent status | Plan |
|---|---|---|---|
| **T1b revived (TR1)** | Resampled history through the exact hit record, plus an **anchor bit** against silhouette trails, **engine-event W caps** (light rate, moved casters, parameter edits, exposure, declared program bandwidth), a fix for Catmull-Rom undershoot at silhouettes, and **no colour clamp**. Reverse-visibility validation, as T1 already has | Single history with capped windows; no variance clip, no gradient, no dual history (I, no clearance) | **In plan, first** |
| **History-free boost (T-A)** | Extra primary samples only where history is missing or rejected: revealed pixels, listed discontinuities (ATAA idea, reimplemented from the paper). The settle mechanism for never-seen surfaces inside TR1, and a standalone fallback if TR1 fails | Not flagged | In plan, within TR1 |
| Two-layer silhouette history | Heavier trail fix | Not flagged | One named fix allowed by TR1's stop rule |
| A-SVGF re-shade of anchored samples | Re-shade with the old seed to detect shading change | **US 12,437,371** | **Excluded** |
| Variance clip / TAA box clip | Neighbourhood mean ± γσ | **US 10,116,916** (to 2037); US 10,362,289 | **Excluded** |
| Colour or luma rejection (TSR, FSR) | Reads output colour | Style-dependent (programmability) | Rejected |
| Fast/slow dual history | Short average plus exponential history, dropped on events | **US 11,847,737** | **Excluded** |
| T2 world-anchored records with scatter | As T2b | Not flagged | Held as the film-target reference (4.25 ms, M) |
| 4K temporal upsampling | T1b at sub-native input | Same as T1b | **Later, its own unit.** Ships only if it beats TSR Epic on every measure at the same screen percentage |

**Rules.**
- **Default rule (research §6.7).** TR1 becomes the default only if it beats TSR Epic on every measured population, static and moving, with no trails or flicker beyond TSR's, at ≤ ½ of TSR's measured end-to-end cost. Otherwise it is opt-in, labelled "temporal reconstruction, below film".
- **Both paths.** Non-RT needs a jittered N1c visibility buffer that accepts Temporal. Rigid motion is exact through hit records and previous transforms on both paths. No velocity buffer is needed.

### 6.1 Denoising GI and reflections

- **The WRC is the denoiser:** world-space accumulation of incident quantities, filtered in cell space.
- **Screen-space cleanup of a residual** is allowed only if it settles within 8 frames. Its non-exposed candidates are decided in D0.
- **Held: Falcor's SVGF (BSD-3).** It is copyable with notice, but **US 10,600,167** is open, so it is out of the plan.
- **Held: ReBLUR-like recurrent blur with history fix and fast history.** It is close to **US 11,847,737**, so it is also out of the plan.
- **Held: kajiya-style validation rays.** They are a secondary-ray temporal gradient, possibly within US 12,437,371. The replacement is engine-known events plus G2's record relight.

## 7. Units and order

Sizes are rough agent estimates (I), given as implementation / GPU verification.

| # | Unit | Content | Depends on | Size | Gates and stop rules |
|---|---|---|---|---|---|
| 1 | **G0 = M7-U0** (running) | Unreal denominators (section 8): GI HW/SW x High/Epic/Cinematic, hit lighting, mesh SDFs on and off. Sun-step and yaw-step settle curves. Memory with Lumen on. **R_GI** references from the in-process GPU path tracer, environment-only and full, at quarry poses 0/1 and S1-S3, cross-checked with pbrt or Mitsuba on two crops. The glossy 84.75% disagreement is localized by roughness and source-angle sweeps | — | running | Every Unreal configuration's effective settings read back, or it is not scored. R_GI agrees with pbrt/Mitsuba within a registered tolerance |
| 2 | **M7-U1** (running) | Specular AA (5.3) | G0's glossy item, for the Unreal comparison only | running | Level F gain on every population |
| 3 | **TR0** | TSR Epic and Cinematic, `r.AntiAliasingMethod=4`, `r.ScreenPercentage=100`, plus a 1x row and an informative 66.662% row, on Q-walk, Q-disocclusion and Q-sun. Scored with `scripts/m6-temporal-motion-score.py` (from `b8fa1f3`) on T1b's populations: F1, F2 by age, trail width, background-centre error, hold flicker. End-to-end A/B timing in one process. Also settles whether TSR leaks at shadow edges | Unreal harness; batch with G0 if its session is still open | 3-6 h / 1-2 h | Output: TSR's error per population as a multiple of the bar, and its end-to-end ms |
| 4 | **R0c** (CPU only, can run now) | Cook the cone-sized cluster-LOD BVH for the quarry at two error bounds: bytes and build time. CPU-trace 10,000 RF1 reflection rays and 10,000 GI cell rays against the N0 full-detail oracle: hit differences against footprint | — | 4-8 h / 0 | Feeds G-N1's far field. The reflection use stays held |
| 5 | **R0a** | Reflection fixtures RF1-RF10 (8.2) in both engines. UE-HW-Hit, UE-HW-SC and UE-SW at Epic and Cinematic: ProfileGPU nodes, images, memory. Roughness histogram of the plain quarry | G0 harness (same batch where possible) | 4-8 h / 2-3 h | Effective readback, as G0 |
| 6 | **G1** | WRC v1, `indirect_irradiance` for sky only, G-R1 and G-N1 (near precompute + far LOD BVH), both paths | G0 (R_GI, L), R0c | 16-30 h / 3-5 h | gi.md's G1 gates and stop rules (8.3) |
| 7 | **R0b** (test-only) | R-RT B0 and B1 at 1 ray per pixel on RF1/RF2, no denoiser, static accumulation. Trace and hit-shading cost; error against the glossy film reference. **Mirror-camera identity gate** on the standard and toon programs | R0a, U1 | 8-12 h / 2 h | Any identity failure is a contract bug and blocks everything else |
| 8 | **TR1** | T1b revived (section 6) on both paths, including the jittered visibility buffer and the revealed-pixel boost | TR0, U1 | 12-20 h / 3-4 h | Section 8.3. If a population is still worse than TSR after the anchor bit plus one named fix, record the miss. Stop if the cost delta is more than ½ of TSR's after one optimisation pass. If TSR is worse than T1b everywhere, gate T1b as is |
| 9 | **D0** (CPU replay) | 1-spp incident-irradiance dumps on Q-walk, Q-sun and Q-disocclusion. Compare only non-exposed filters: (a) world-cell accumulation with event W; (a′) (a) plus cell-space spatial filtering; (c) a sample-count control | G1 dumps | 4-8 h / 1 h | Drop a filter that cannot reach the bar by frame 8 after each event, or whose GPU estimate is above ½ of Lumen's measured denoise share. If none passes, spend on samples or caches, not blur |
| 10 | **G2** | Bounce: `scatter`/`emission`, sun one-bounce, cache-terminated multi-bounce. Records against priority re-trace | G1, D0 | 12-24 h / 3-5 h | All GI measures, including S4 events and off-screen bounce |
| 11 | **R1** | Reflections: R-RT B0-B2, `reflection_lobe`, `specular_incident`, hit shading by program; non-RT planar tier and WRC B2. Held tiers only after the Adshir/Snap ruling | R0b, G1, U1 | 16-30 h / 3-5 h | 8.3. If RT cost is above 0.5x UE-HW-Hit's "Lumen Reflections" after three hillclimb mechanisms, record the miss |
| 12 | **Integrated acceptance** | Every gate on the integrated build, both paths, against the Epic floor; 2x end to end; memory; quiet window; independent Opus review | all | 6-10 h / 4 h | 8.3 |
| 13 | **Upscaling** (later) | 4K output | TR1 | — | Ships only if it beats TSR Epic on every measure |

**Why this order.**
- G0 and U1 are already running.
- TR0 and R0c are cheap. TR0 decides whether T1b already beats TSR; R0c sizes the non-RT far field.
- G1 is the first Renderlab unit. It is lighting-invariant, runs on both paths, needs no `scatter` contract, and its lag gate is clear.
- R0b proves the hit contract before R1 builds on it.
- D0 picks the GI accumulation before G2 commits to one.

**Parallel ownership.**
- G1 and R0c are CPU-heavy and separable. G1 owns `gi_cache*` and the cook visibility module; R0c owns the LOD-BVH cook.
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
| UE-HW-SC-{H,E,C} | `HardwareRayTracing 1`, LightingMode 0, HRT shadows, TSR | RT cost curve |
| UE-HW-Hit-{E,C} | LightingMode 2 (hit lighting for reflections) | **RT reflection floor** and RT quality ceiling |
| UE-GI-only | As above, with reflection method none or sky | Matches G1/G2 scope |
| UE-sky | As UE-GI-only, with the sun's Indirect Lighting Intensity 0 (to verify) | G1's sky-only comparator |
| UE-TSR | `AntiAliasingMethod=4`, `ScreenPercentage=100`, `sg.AntiAliasingQuality` 3 and Cinematic; plus 1x (`=0`) and 66.662% rows | TR0 |
| UE-DLAA (optional) | DLAA preset K, if installed | Informative only, never a bar (Q5) |

**Timing.**
- End to end: same-process A/B toggles, measured in CSV `GPUTime` with async at its defaults.
- Attribution: ProfileGPU with `r.Lumen.AsyncCompute=0` and `r.TSR.AsyncCompute=0`, for these nodes:
  - Lumen Scene Update and Lumen Scene Lighting;
  - Screen Probe Gather (traces, filter, integrate, temporal, ShortRangeAO);
  - Lumen Reflections;
  - TemporalSuperResolution.
- Overlapping queues are never summed.

**Lag.** A 10° sun step and an environment yaw step, captured at frames 1, 2, 4, …, 128 after the step: frames-to-settle against the settled image.

**Floor arithmetic (gi.md, I until G0).**
- RT: Renderlab's M7 budget is ≈ L_hw/2.
- Non-RT static: ≈ L_sw/2 + 0.08 ms on the Surface default.
- Non-RT camera-moving already misses 2x without GI (0.89x, M). Per Islam, non-RT GI still becomes the default if it beats Unreal's quality on every measure; the miss is recorded.

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
| R1 | All RF measures per band, both paths. Non-RT reported against UE-SW with the held tiers named as absent. B1 motion (RF3, RF5) ≤ Unreal, or opt-in and labelled |
| Integrated | Everything above together, on the quarry and fixtures, against the Epic floor |

## 9. Patents and licences

**Patents.** "Not flagged" is not a clearance. A human review is open for the starred rows (milestone-7.md).

| Technique | Exposure | In this design |
|---|---|---|
| World-space surface cells (WRC; also the N8n/o shadow default) | **US 9,754,407\*** | WRC is built opt-in. Default awaits the review (Q1) |
| ReSTIR (screen or world space), with any cache | **US 11,315,310\*** and family | **Excluded.** No unit depends on it (4.1) |
| Hash-grid path-space filtering, projective hash maps | NVIDIA authorship; **US 11,925,860** | Our 3D key is not a projection (I); include in the review |
| Variance clipping | **US 10,116,916** (to 2037), US 10,362,289 | Excluded |
| SVGF / à-trous variance-guided filtering | **US 10,600,167\*** | Excluded (Falcor code licence-clean, patent open) |
| Temporal gradients, A-SVGF re-shade, validation-ray gradients | **US 12,437,371** | Excluded; engine events and record relight instead |
| Fast/slow history | **US 11,847,737** | Excluded |
| History clamping | US 11,783,455, US 11,113,792 | Excluded (no clamp anywhere) |
| Raster primaries plus reflection rays | **Adshir/Snap** US 10,950,030 B2, 10,699,468, 11,302,058, 12,051,145, 12,573,127, pending 2025/0363721 | Non-RT LOD-BVH and SDF reflection tiers held. RT traced-primary reflections are outside the wording (I, claims unread) |
| Object-space shading | US 9,747,718 | Not used |
| DDGI, ratio estimator, ATAA, ray cones, Toksvig/LEAN, Kaplanyan-Tokuyoshi, planar reflections | Not flagged | Used as described |

**Licences.**

| Rule | Items |
|---|---|
| **Reimplement only: read docs, copy no code** | Epic, Unity, RTXGI 2.x, SHaRC, NRC, NRD, RTXDI, DLSS / DLSS-RR, XeSS, FSR 4, FidelityFX 2.x binaries |
| **Own code from the text** | DDGI papers (CC BY-ND); RTG ch. 25 (CC BY-NC-ND) |
| **Copy with notice** | Bevy Solari's world-cache files only (`world_cache_{query,update,compact}.wesl`, MIT/Apache; never its ReSTIR files); kajiya ircache (MIT); Capsaicin hash cells and leak fix (MIT; not `world_space_restir.hlsl`); Falcor `TexLODHelpers` (BSD-3); Filament `normalFiltering` (Apache-2.0); obvhs and tray_racing (MIT/Apache); Brixelizer, FidelityFX SDK pinned at v1.1.4 (MIT); Wicked DDGI (MIT) |
| **Tools** | pbrt-v4, Mitsuba 3 |
| **Copyable but held on patents** | Falcor SVGF (BSD-3); FidelityFX reflection denoiser (MIT; temporal clip under review) |

## 10. Questions for Islam

1. **US 9,754,407.** The N8n/o surface cache is already the non-RT shadow default (N8p) while its review is open. May the WRC, which has the same structure, become a GI default on the same basis? Or must both wait for the review?
2. **Adshir/Snap ruling for non-RT reflections.** Until you rule, non-RT non-planar sharp and glossy reflections have no in-plan producer (5.2). The non-RT reflection floor may then fail against Lumen software on RF2-RF4. Do you want this review prioritized, or the miss recorded meanwhile?
3. **Held denoisers.** SVGF-class and recurrent-blur filters are out of the plan (9). If D0's non-exposed filters miss the 8-frame and quality gates, do you want a patent review to unlock them? Or record the miss and spend on samples and caches?
4. **`reflection_lobe` is GGX-shaped.** Non-GGX styles get their nearest GGX lobe, or no traced reflections. Accept this recorded constraint?
5. **DLAA.** Add an informative DLAA preset K row to the Unreal runs, never a bar? Linking `dlss_wgpu` into Renderlab is not proposed.

**For root, not Islam:**
- Extracting the surface cache's key/claim/resolve code into a module shared with the shadow hillclimb, against the M7 copy recorded in 3.1.
- Which RT configuration's VRAM (2,641 or 4,138-4,202 MiB) is M7's baseline.

## Evidence and time

Reading and design only:
- the three research notes and milestone-7.md's decisions;
- the M5-DL, N8p and memory records;
- draft 1 and its inputs.

Of the running M7-U0 and M7-U1 worktrees, only their branch names and changed-file lists were seen. None of their uncommitted work was read or used. No implementation, verification, GPU or Unreal time was spent.
