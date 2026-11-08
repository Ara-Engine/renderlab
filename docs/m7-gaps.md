# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Transparency

TP1 ([m7-tp1.md](m7-tp1.md)). Cutout foliage is load-time geometry; transparent materials draw in a forward pass after reconstruction.

- **Shadows from transparent surfaces.** Glass casts no shadow: it isn't in the TLAS, the clipmap, the surface cache or the radiance cache. Unreal's translucent shadows (Lumen and VSM opacity) aren't matched: no tinted or coverage shadow.
- **Order.** Sorting is per object: one key per draw, from its visible instances' mean bounds centre. Instances inside a draw, and intersecting transparent objects, aren't sorted per pixel, and no OIT fallback exists. Unreal sorts per primitive with optional OIT.
- **Refraction** is screen-space and thin-slab only. Content off screen or behind a nearer occluder falls back to the unshifted pixel. No thick or volumetric refraction, no dispersion, no rough (blurred) refraction: the colour copy has no mip chain. Each transparent draw copies the full HDR target before it draws.
- **Anti-aliasing.** The pass runs after TR1 at one sample per pixel, so transparent edges alias. Unreal composites translucency before TSR, with responsive AA.
- **4x raster frames** skip the pass: there is no single-sample depth to read. 1x raster, traced and visibility-buffer frames run it.
- **Lighting inputs.** Transparent surfaces read the same lights, sky, ES1 lobes and radiance-cache lookups as opaque ones. The cache holds no cells of its own for transparent surfaces, so they read nearby opaque cells or the open sky. Non-RT sun visibility is a 2048² map over the transparent receivers' footprint, not the Virtual clipmap: it's harder-edged than the sun's disk and its texel size scales with the footprint. Other lights' visibility on non-RT transparent surfaces is 1.
- **Programmability.** A program shades transparent surfaces through its own `surface` and `shade`, plus an optional `transmittance` hook. Without the hook, it blends by coverage. Refraction uses the engine's `material_normal`, not a program normal. Streamed (non-resident) textures on transparent materials bind their resident levels without feedback.
- **Cutout foliage.**
  - The cut is static: no alpha animation, no runtime change of cutoff or texture.
  - The contour is linear per grid cell, and the triangle cost is high: the fern card is about 8k triangles at 96 cells.
  - No tangent stream, so cut meshes take no normal map.
  - No alpha-to-coverage softening of the cut edge.
  - Vertices aren't shared between cells (no index reuse).
- **No Unreal or reference comparison** this push (rule 3).

## Particles

TP1 GPU emitters ([m7-tp1.md](m7-tp1.md) §1.3).

- **Lighting.**
  - Each particle carries one irradiance value per step, at its centre: directional lights with the sun map's visibility, plus the environment's six-axis mean irradiance.
  - No point or spot lights, no radiance cache, and no self-shadowing or volumetric shadowing between particles.
  - Particles cast no shadows.
  - The phase function is a fixed isotropic half.
- **Sorting** is per emitter, in one workgroup: at most `MAX_PARTICLES` = 2048 per emitter. Emitters sort against each other and against transparent draws by their spawn centre, so interleaved emitters can order wrongly.
- **Simulation.**
  - No collision, curl noise, or vector fields.
  - Colour, size and emission are linear between two keys, not curves. No flipbook animation, no motion-vector blending, no ribbons or meshes.
  - Spawns per step are capped at the capacity.
- **Rendering.**
  - Billboards render at full resolution after TR1, with no temporal stability or motion blur.
  - Overdraw isn't bounded: no low-resolution particle buffer.
  - Soft fading uses opaque depth only: particles don't fade against glass.
- **No Unreal (Niagara) comparison** this push (rule 3).

## Decals

DC1: [projected box decals](m7-dc1.md) modify resolved surface inputs before programmable shading on both paths. The API includes typed textures, alpha/multiply/add colour blends, stable sort order, angle fade, opacity and channel strengths. Placement activates them without another feature toggle.

- No matched Unreal or path-traced decal comparison was run in the overnight time-box; Unreal-equivalent decal quality and a 2× performance advantage are unproven.
- The one contended development-build cost run has large timing tails. Quiet optimized cost, empty-table overhead, table-publication latency and high-overlap scaling remain unmeasured; the median paired shading deltas (+0.011 ms RT, +0.008 ms non-RT) are diagnostic only.
- Decal textures use trilinear filtering, with no anisotropic filtering, compression, texture streaming or normal-variance contribution to specular AA yet.
- The world-space grid never truncates overlapping lists, but widely scattered or heavily overlapping decals can increase candidate counts and publication cost. There is no GPU grid builder or world partitioning yet.
- Opacity controls overlay coverage and reaches user programs as material alpha. It does not cut opaque silhouettes or alter shadow coverage; consuming alpha in transparency is an integration task.
- Secondary reflection hits, moving/attached decal transforms and temporal reconstruction under substantial motion remain integration verification gaps. GI cell shading consumes decals and notices their changes, but a measured decal-to-bounce propagation test remains open.
- Forward lit GPU images and combined GI/TR1 configurations remain unverified. The retained traced and actual no-RT tests passed with zero Vulkan validation errors; quarry still and 1 cm moving images were inspected on both paths.

## TR1 (temporal reconstruction, default on; TR1b)

TR1 is the default on traced and visibility-buffer lit frames, both paths, from TR1b's overnight pass; `Reconstruction::off()` switches it off ([m7-tr1b.md](m7-tr1b.md)). It is never presented as meeting the Unreal bar.

**Reach.** The RT device's code default is still raster primaries (RT1c/RT1d), which TR1 does not reconstruct: RT users get TR1 only with traced primaries. Temporal soft-shadow frames keep their own film history and are not reconstructed.

**Against no-AA, judged on each path's own 64-spp pixel integral** (Islam's ruling; the visible-sky quarry walk, 40 frames, every row pooled per segment). Before tonight TR1b lost 31 of 256 pooled rows on both paths. With the overnight changes (radius-1 spatial start with a radius-2 fallback, point-symmetric jitter pairs, exact single-layer output at whole coverage, sub-1e-3-px fetch snap) it loses 26 on both paths; the cold first frame now equals no-AA bit for bit (all start-age-0 rows tie). What still loses, by kind (RT; non-RT within a few rows of it):

| Kind | Rows (RT: still start / moving / still end / aged pools) | Example | Cause |
|---|---|---|---|
| Single-pixel maxima | 2 / 12 / 3 / – | single-surface L0 216 vs 108 (moving) | a rock pixel under the sun-bright sky takes 4–5 % sky through a transported coverage estimate; at 10⁴ contrast a 4 % coverage error is 200 codes. Also a lit pixel on a thin dark crease (lit L4 212 vs 81) |
| Signed means of small near-edge populations (lit, faint, silhouette) | 1 / 5 / 5 / – | silhouette L0 0.61 vs 0.22 (moving) | the same sky bleed, as a bias |
| Sky (background-centre, distance bands) | 1 / 1 / 9 / – | 0.009 vs 0.004 codes RMS far from edges (L0) | f16 output rounding of a mean against a centre sample on a smooth glow; paired jitter halved it, pending pairs (rejected: +6 aged rows) brought it to 0.0074 |
| Still-hold step | – / – / 4 / – | 0.66 codes vs 0 (L0) | any accumulation changes still frames while converging; no-AA is exactly constant. Structural |
| Revealed pockets | – / – / – / 37 | revealed ages 0–7 (4, 4, 3, 9, 5, 4, 4, 4 rows) | young history on revealed pixels, mostly next to bright-sky silhouettes |

Non-RT: 8 / 16 / 23 / 37 rows by segment, the same kinds.

Not fixed tonight; each needs either an exact coverage model at very high background contrast or a decision on the step row (no accumulating method can tie a constant image there).

**Against the path-traced reference** (kept for the record; re-scored once G2's cache is the default): TR1b is worse than no-AA in 149 of 256 pooled rows on both paths, almost all from the lighting error both images share in that backlit view (shadowed rock 3,600× brighter than the reference), which pixel integration moves further from a near-black reference.

**Against TSR Epic** (TR1b's settings before tonight; not re-scored with tonight's defaults):
- Q-walk: revealed pixels (age 3, all-covered L0 9.64 vs 7.70), background-centre next to edges (0.419 vs 0.371 at frame 64), edges at L0 by under 1 % (17.98 vs 17.81); +4-stop populations where the converged still image is itself above TSR's.
- U1b's 776-comparison specular gate fails (458–462 failed comparisons): view-dependent shading lags under camera motion; light-track turns restart the history.
- S2G sun 0.25°/frame on non-RT loses (15,476 wins, 26,772 losses): Virtual's shadowed shading under a turning sun.
- Cost: p95/p99 increments over the half-TSR ceilings (TR1's quiet run: RT increment p95 0.815 / p99 1.009 ms vs 0.769 / 0.816); the quiet re-time with TR1b's binary is outstanding.
- The visible-sky walk ([ue-sky.md](ue-sky.md), TSR Epic and High on the same 40 frames, scored against the same path-traced references; TR1b rows from its pre-tonight binary `tr1b-k`): TR1b is worse than TSR Epic in **216 of 256** pooled rows on RT (37 better, 3 tied) and 213 on non-RT (40 better), and in 586 of 626 aged pools. Selected rows, L0 / L4 codes (RT; non-RT within 0.07):

  | Row | No AA | TR1b | TSR Epic | TSR High |
  |---|---:|---:|---:|---:|
  | Moving covered RMS | 26.66 / 85.18 | 26.38 / 86.59 | 15.79 / 68.22 | 15.88 / 68.63 |
  | Moving edge RMS | 49.17 / 102.13 | 36.88 / 104.36 | 36.19 / 76.99 | 32.48 / 77.89 |
  | Moving background-centre RMS | 0.20 / 0.31 | 0.37 / 1.22 | 1.65 / 1.15 | 0.52 / 0.74 |
  | Revealed covered, age 0 | 51.07 / 128.20 | 40.30 / 124.25 | 27.31 / 96.16 | 26.98 / 95.73 |
  | Revealed covered, age 3 | 43.73 / 118.42 | 39.29 / 120.27 | 26.85 / 90.82 | 28.19 / 92.77 |
  | Moving flicker | 16.23 / 46.34 | 13.76 / 40.91 | 8.62 / 22.70 | 8.39 / 22.24 |
  | Final-hold step | 0 / 0 | 0.80 / 1.88 | 0.32 / 0.80 | 0.41 / 0.97 |

  These are complete-image rows: the covered-pixel gap also contains the lighting error in the backlit shadows, which no-AA shares, so it is not attributed to reconstruction alone. TR1b beats TSR Epic only on moving background-centre pixels at L0. TSR's cost on this walk: Epic total increment p50 1.38–1.39 ms (p99 1.95–2.53), High 0.97 ms.

**Upscaling presets** (quality 2/3, performance 1/2; opt-in, native is the default):
- Inspected only: no scores against references or TSR at reduced internal resolution, and no 4K output (a later unit).
- On S2G's moving glossy reliefs both presets alias the finest specular stripes (stepped edges) where TR1 native does not; on the quarry they are slightly softer than native.
- Timing is one functional (not QUIET) run: Q-walk RT p50 0.93 ms (quality) and 0.73 ms (performance) against TR1 native's 1.47 ms; non-RT 1.71 and 1.45 against 2.37.
- The internal size is rounded per axis, so targets whose size isn't divisible by 3 (quality) or 2 (performance) render at a very slightly different aspect.
- The viewer renders raster primaries, so it gets neither TR1 nor the presets.

**Reference instrument.** The visible-sky references were traced by a release-profile test binary; on fixed resources its first renders differ from later ones by ≤2 ULP, which the bounded-AS policy's late fixed-resource control does not see. A dev-profile build of the same source (U0b's profile) reproduces quarry-final byte for byte on 8 independent and 4 fixed replays. The sky references stay accepted under the policy; a dev-profile re-trace was stopped by the overnight no-new-references rule.
