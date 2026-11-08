# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Skinning

- SK1 uses ALLOW_UPDATE/PreferUpdate for animated BLASes and the frame's TLAS;
  wgpu 30 currently rebuilds both. Actual incremental refit remains open.
- The non-RT path skins full-detail source clusters. Skinned cluster LOD is
  unavailable; bounds are conservative mesh-wide bounds. CPU software-GI BVHs
  rebuild for changed poses, and caster invalidation is conservative. Replacing
  animated geometry also repacks the static software scene each pose: the default
  GI quarry run is currently too expensive for game frame rates (**242 ms CPU,
  310 ms synchronized frame** in SK1's contended opt-level-1 diagnostic run).
  Persistent dynamic geometry ranges are needed; see SK1's measured frame cost.
- RLS v1 supports four influences, TRS nodes and one active sampled clip. Matrix
  nodes, additional influence sets and morph targets are explicitly rejected.
  Clip blending, events, dual-quaternion skinning and automatic root motion remain
  outside the overnight base.
- Existing TR1 model-transform edits still reset history. SK1 supplies previous
  deformed vertex positions; its walk example puts translation in the palette.
- SK1 exposes typed renderer upload/palette APIs and a clip player. API1's planned
  `Engine` facade adapter (`MeshId`/`ObjectId` pose ownership) remains to be wired.
- Moving-caster shadow regions reject old receiver lighting history to avoid
  trailing shadows. With dynamic GI enabled the region is conservatively
  scene-wide; finer rejection and stable shadow sampling remain open.
- Hardware shadows retain the existing opaque, double-sided caster contract. The
  example enables double-sided materials explicitly.
- No new Unreal or path-traced comparison was run for this overnight unit. Its
  visual and timing evidence, and their limits, are in [m7-sk1.md](m7-sk1.md).

## Post

PP1 provides bloom, camera motion blur, camera-requested near/far gather DOF, tone/white balance/contrast/saturation/3D LUT and optional lens effects on both paths. [Implementation and evidence](m7-pp1.md).

- Motion vectors currently reconstruct camera motion over static geometry from TR1's hit/projection inputs. Prior object/skinned/particle transforms and transparent-layer velocities still need a shared attachment. Hidden/disoccluded colour cannot be reconstructed from one surface.
- DOF needs better near-field coverage, hidden-background reconstruction, shaped apertures and temporal stability for large bokeh. MSAA raster depth/velocity uses sample zero.
- The filmic option is an ACES-like fit, not full ACES; RGB white balance is approximate. Display-linear RGBA8 LUTs have no HDR/log shaper or file importer.
- Bloom uses a fixed normalized six-level optical approximation. Grain is simple noise; lens dirt/diffraction/anamorphic calibration are absent. Post shader reload is not wired into the editable prelude transaction.
- No new Unreal/reference campaign tonight; full post quality parity remains unproven. Final effect timings and image findings are in the PP1 report.
- Optional DOF needs release profiling: the retained contended test-profile run reached 11.219 ms for non-RT quarry near DOF and 5.581 ms for raster far DOF. Camera-requested DOF remains off by default.
- API1's planned `Settings::post` and engine-module re-exports still need wiring; the typed crate-root API works through `Engine::renderer_mut()`. Native final checks include AT1/GI defaults; upscale-preset evidence predates that merge.

## Transparency

TP1 ([m7-tp1.md](m7-tp1.md)) and TP2 ([m7-tp2.md](m7-tp2.md)). Cutout foliage is load-time geometry; transparent materials draw in a forward pass after reconstruction. TP2 adds coloured sun transmission on both paths, R1 reflections on traced RT frames and four-sample geometric coverage.

- **Shadows from transparent surfaces.** A cached 1024-square layered map filters the first directional sun by tint, coverage and Fresnel on both paths. Each draw stores only its nearest sheet; overlapping sheets/instances in that draw do not compound. Other lights, environment visibility and GI traversal still omit transparent intersections. No volumetric absorption, caustics or finite-sun transmission penumbra. The map uses engine material inputs rather than an arbitrary program's transmission model.
- **Order.** Sorting is per object: one key per draw, from its visible instances' mean bounds centre. Instances inside a draw, and intersecting transparent objects, aren't sorted per pixel, and no OIT fallback exists. Unreal sorts per primitive with optional OIT.
- **Refraction** is screen-space and thin-slab only. Content off screen or behind a nearer occluder falls back to the unshifted pixel. No thick or volumetric refraction, no dispersion, no rough (blurred) refraction: the colour copy has no mip chain. Each transparent draw copies the full HDR target before it draws.
- **Anti-aliasing.** TP2's four-sample coverage resolves geometric edges after TR1. Texture/shading aliasing, intersections and particles still need broader treatment; glass has no temporal history. The seed/resolve and additional attachments have a cost.
- **Frames that skip the pass.**
  - 4x raster frames: there is no single-sample depth to read.
  - TR1b upscaled frames (`RenderScale` below native): hit records are at the internal size and presentation reads the output HDR.

  1x raster, traced and visibility-buffer frames at native scale run it.
- **Cost** (M, quarry, TP2 `eb0f4a2`, functional lease, not QUIET-certified).
  - Cache searches at glass receivers caused the regression: isolating an open-environment input reduced the glass increment from 1.123 to 0.173 ms. Retained RT sun rays and atmosphere.
  - With TP2 reflections, transmission shadows and AA enabled, the measured pane increment is **0.421 ms RT traced / 0.123 ms non-RT**. Cold pipelines and moving-sun map updates are not priced by these steady-state rows.
  - Each transparent draw still copies the full HDR target. Full data and contention limits are in the TP2 report.
- **Reflections on glass.** Traced RT frames now run R1 for the glass program's lobe. RT raster and non-RT retain the environment fallback (R1n was not merged at this checkpoint). Hits see only opaque geometry, with no recursive glass. Glossy strata are fixed without a history denoiser; mixed-program hits retain MP1's response fallback.
- **Lighting inputs.** Glass now uses the open environment: the radiance cache creates no cells for it, and receiver searches were expensive. Non-RT sun visibility remains a 2048² opaque map over the transparent receivers' footprint, not the Virtual clipmap: it's harder-edged than the sun's disk and its texel size scales with the footprint. Other lights' visibility on non-RT transparent surfaces is 1. Skinned transparent casters and palette-based invalidation of TP1's auxiliary opaque map remain unverified.
- **Programmability.** A program shades transparent surfaces through its own `surface` and `shade`, plus an optional `transmittance` hook. Without the hook, it blends by coverage. Refraction uses the engine's `material_normal`, not a program normal. Streamed (non-resident) textures on transparent materials bind their resident levels without feedback.
- **Cutout foliage.**
  - The cut is static: no alpha animation, no runtime change of cutoff or texture.
  - The contour is linear per grid cell, and the triangle cost is high: the fern card is 7,722 triangles at 96 cells. Measured cost of 18 cards: +0.20 ms non-RT, +0.39 ms raster, within noise on RT traced. There's no LOD of the cut.
  - Cuts must stay welded: an unwelded cut cost +6 ms on non-RT.
  - No tangent stream, so cut meshes take no normal map.
  - No alpha-to-coverage softening of the cut edge.
- **No Unreal or reference comparison** this push (rule 3).

## Particles

TP1 GPU emitters ([m7-tp1.md](m7-tp1.md) §1.3).

- **Lighting.**
  - Each particle carries one irradiance value per step, at its centre: directional lights with the sun map's visibility and TP2's coloured transmission, plus the environment's six-axis mean irradiance.
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
  - Overdraw isn't bounded: no low-resolution particle buffer. Fire (192) and smoke (256) together add +0.44 to +1.21 ms (M, indicative), mostly smoke overdraw (I).
  - AT1 fog and aerial perspective apply at each fragment's billboard point. Particles neither receive nor cast volumetric fog lighting.
  - Soft fading uses opaque depth only: particles don't fade against glass.
- **No Unreal (Niagara) comparison** this push (rule 3).

## RT shadows

[RTDEF2](rtdef2.md) selects traced primaries, Interactive and TR1 for capable
1x RT devices: 256 rays on a fresh target/cut, 64 on scene-history resets,
32 for camera/light motion or young history, and eight after 16 held frames.
The registered cold/moving disk witnesses no longer show RTDEF's extra grain.
The viewer negotiates the required RT capabilities; explicit Raster/Exact
settings and the old recorded gates remain available.

- Transform edits of existing placements no longer restart TR1 (MQ1,
  [m7-mq1.md](m7-mq1.md)): they reproject through the previous transforms and
  clamp to a 3x3 min/max box. Material changes and new placements still restart
  every TR1 pixel; the 64-ray restart budget improves their grain without fixing
  that coarse invalidation. Broader animated-game coverage remains work.
- The final disk grain diagnostic wins 47/48 sampled checkpoints. The early
  camera-1px frame 1 row is 4.857 versus Exact's 4.103 +4-stop codes, though
  inspected speckle is not visibly worse and whole-crop error is lower.
  Native quarry crops also retain TR1's fine-texture softness; this sampling
  change does not fix its resolve filter or certify film-exact detail.
- A first GI-cache initialization can dominate a cold frame (RTDEF2's
  quiet first traced quarry visit reached 89.53 ms; an earlier functional
  visit reached 120.9 ms). Warm-cache cold TR1 targets cost 14.85 ms median;
  held/camera frames cost 3.71/4.65 ms versus Exact's 17.04/17.05 ms with
  GI/AT1 enabled. These costs are reported separately; no startup/frame-time equivalence
  or engine-wide Unreal performance claim is made.
- The retained HRT comparison stays non-blocking. Disk cold grain is repaired;
  point-source first-frame geometric aliasing, TR1's revealed-pixel/sky/specular
  gaps and missing full-HDR flicker comparisons remain. RTDEF2's sparse
  checkpoints do not replace RTDEF's full 1,280-frame audit.
- RT1d's spatial visibility filter remains opt-in. Its false-shadow,
  lit-darkened and strict-solid losses are not repaired by this ray budget.
- Raster/4x, virtual geometry and RT devices without traced binding-array/
  hit-vertex capabilities retain the fallback path. The traced draw/geometry
  admission limits remain; this is not acceptance of every game workload.
- Historical eight-ray, TR1-off speed/memory rows are retained as historical
  evidence, not prices for the new GI/AT1/TR1 default. Unreal/film parity and
  a combined 2x advantage are not certified by this unit.

## Atmosphere

AT1, [m7-at1.md](m7-at1.md), followed by [VF1](m7-vf1.md) and [AT2](m7-at2.md). The sky, aerial perspective, height fog, sun transmittance, procedural environment and shadowed volumetric fog are on by default on both paths.

- **No distinct light shafts.** Islam's review of VF1's sheets (2026-11-08): at low sun the fog shows only soft haze, with no distinct shafts even at "dense". Likely causes to measure: froxel resolution (16 px columns, 7.8 m first slice), the 0.9 history blend, phase-function anisotropy and the default density. Next-level item.

- **Non-RT volumetric coverage is bounded by resident shadow pages.** VF1 reads Virtual's top depth layer without triangle walks or new page requests. Missing or stale fine pages fall back through coarser valid levels, then to sunlight. Fog in a volume with no resident pages can still leak sunlight. Depth-map resolution, the caster LOD and sun-basis hysteresis can differ from the RT fog's exact centre-sun rays. Conventional raster fog has the sun cascades' coverage limit. Dedicated fog page demand and a measured soft sun disk remain open.
- **The analytic height fog's sun term is unshadowed** beyond the volumetric range, and everywhere while volumetric fog is off, as in Unreal's exponential height fog. Under the default density (about 2 % per 100 m) it isn't visible on the quarry (10 m scale). Dense fog in shadowed spaces will glow.
- **No matched Unreal comparison.** Neither Unreal's SkyAtmosphere nor its ExponentialHeightFog was measured tonight (no new Unreal campaigns). The medium is Hillaire 2020's Earth, the same model Unreal's SkyAtmosphere implements, but no image or timing comparison exists.
- **No path-traced reference** for the sky or the fog. The reference path tracer has no participating media.
- **A captured sky with a non-physical sun shows a dim backdrop.** The procedural sky scales with the first directional light's lux. The quarry's captured-sky protocol (a 1000 lux sun beside an RLE2 sky) therefore shows a near-black sky on misses, as it did before AT1 (a black clear colour). Scenes with a captured sky should turn on `set_visible_environment` until misses follow the selected map automatically.
- **Visible captured-environment and constant-clear backgrounds bypass fog.** VF1 composes near volumetrics onto the procedural sky, but TR1b's selected visible-map branch returns the environment directly; a sky-off miss returns the clear colour. Surface hits remain fogged. Fogging these backgrounds consistently remains an integration gap.
- **Only one sun.** Only the first directional light drives the sky, the sun disk, the fog and transmittance. A second directional light (a moon) gets none.
- **Continuous time of day relights progressively.** AT2 updates the procedural environment in place for sun, lux, medium and camera-altitude changes, without replacing bindings or clearing geometric GI visibility. The cache keeps its existing eight-group environment/bounce relight schedule and gather caps, so indirect light can lag a fast day cycle. Every changed frame still convolves the environment; there is no staggered cube update or altitude quantization. Changed environment yaw still replaces storage. Extreme sunrise/sunset, high-altitude travel and long day/night cycles remain unscored.
- **Moving-sun quarry cost remains high.** AT2's contended release/validation-enabled 32-frame pair measured 5.95 to 10.16 ms RT traced and 15.45 to 43.35 ms non-RT, including GI and shadow updates. The game fixture added about 0.52/0.58 ms RT/non-RT. Removing map churn did not establish cheap quarry time of day; stage attribution and a quiet measurement on the subsequently merged GI/lighting work remain open. No quality reduction was used. [AT2](m7-at2.md).
- **Fog temporal acceptance remains limited.** VF1 reprojects cell centres, rejects lighting/media/sky/caster changes and density discontinuities, and clamps/adapts old lighting at visibility changes. Moving casters and changing sun reset fog history, so noisy starts can recur. A scored motion/disocclusion comparison against Unreal remains open; sky temporal integration is still TR1's.
- **VF1 validation is not clean.** Its quarry and game processes each log ten Vulkan `VUID-StandaloneSpirv-None-10684` SPIR-V layout diagnostics, starting in pipeline creation before fog is enabled. The lib suite and isolated clipmap reader log none; renders complete, but the layout issue still needs investigation. Conventional non-RT raster-map fog was shader-validated only; actual non-RT captures use VB + Virtual. See [VF1](m7-vf1.md).
- **Transparent surfaces:** `atmosphere_surface` is ready for TP1's transparent entry. It isn't wired there in this unit.
- **Film accumulation (`SoftShadows::Temporal`)** fogs its hits, but its misses keep `film_background()` (no sky).
- **Slice resolution:** the aerial perspective's first slice ends at 7.8 m, and froxels are 16 px columns. VF1 inspected a short 7.5 cm camera move and hold; broader banding, aliasing and disocclusion coverage remain unscored.
- **Sun disk integration is approximate.** AT2 replaces the hard rim with analytic square-pixel coverage from explicit camera-ray footprints on raster and deferred misses, using stable chord distance. The rim is locally straight and limb intensity is sampled at the pixel centre; unusually small, subpixel custom suns need a full curved-disk/limb integral. The HDR ceiling, extreme exposure, horizon crossings and broad motion remain unscored; the old hard-rim result is superseded by AT2's measurements: deferred RT/non-RT crops match exactly across eight subpixel positions; raster retains 0.00276-0.00346 exposed-HDR crop RMS against deferred (maximum 0.0977). Disk energy varies by about 0.4% across those positions; no full curved-disk/limb reference or broad motion score was run.
- **The procedural environment uses the legacy mip-chain layout** (no ES1 lobes; the sun is the directional light, so no lobe is lost). Its smallest face is 8 texels: at 1 texel, rough lookups cost +0.47 ms of quarry shading. Roughness is quantized in 0.25 steps between levels.
- **The planet ground below the horizon** is a flat Lambertian albedo (0.3) lit only by the sun through the atmosphere. Misses below the horizon show that, not terrain.

## Decals

DC1: [projected box decals](m7-dc1.md) modify resolved surface inputs before programmable shading on both paths. The API includes typed textures, alpha/multiply/add colour blends, stable sort order, angle fade, opacity and channel strengths. Placement activates them without another feature toggle.

- No matched Unreal or path-traced decal comparison was run in the overnight time-box; Unreal-equivalent decal quality and a 2× performance advantage are unproven.
- The one contended development-build cost run has large timing tails. Quiet optimized cost, empty-table overhead, table-publication latency and high-overlap scaling remain unmeasured; the median paired shading deltas (+0.011 ms RT, +0.008 ms non-RT) are diagnostic only.
- Decal textures use trilinear filtering, with no anisotropic filtering, compression, texture streaming or normal-variance contribution to specular AA yet.
- The world-space grid never truncates overlapping lists, but widely scattered or heavily overlapping decals can increase candidate counts and publication cost. There is no GPU grid builder or world partitioning yet.
- Opacity controls overlay coverage and reaches user programs as material alpha. It does not cut opaque silhouettes or alter shadow coverage; consuming alpha in transparency is an integration task.
- Secondary reflection hits, moving/attached decal transforms and temporal reconstruction under substantial motion remain integration verification gaps. GI cell shading consumes decals and notices their changes, but a measured decal-to-bounce propagation test remains open.
- Forward lit GPU images and combined GI/TR1 configurations remain unverified. The retained traced and actual no-RT tests passed with zero Vulkan validation errors; quarry still and 1 cm moving images were inspected on both paths.

## Reflections

From M7-R1 ([m7-r1.md](m7-r1.md)). Scores are whole-reflector display RMS at -14 stops (8-bit F0 codes) against R0a's references, static, 1x, GI cache off; Unreal floors are R0a's.

1. **Non-RT has no traced reflections.** Mirror and glossy surfaces reflect only the environment, through ES1's `indirect_radiance` (occluded, with bounce, once the world radiance cache is on). RF1-RF9 are unchanged and miss UE SW Epic on every fixture: RF1 46.1 vs 37.2, RF2 21.0 vs 15.9, RF3 45.8 vs 32.2, RF4 45.7 vs 40.6, RF6 52.3 vs 12.3, RF7 48.0 vs 35.6, RF8 9.9 vs 5.2, RF9 46.2 vs 37.0. Local objects never appear in non-RT reflections. The fix is R1n's LOD-BVH software rays (design 5.2, R0c's BVH), which did not fit the time-box. The patent-fallback trigger (`NonRtReflections::CoherentHybrid`) has not been evaluated, because no design-around tier has been measured yet.
2. **Raster primaries get no traced reflections.** The RT device's code default is still raster primaries (IQ1, RT1d), and only traced-primary frames trace reflections. Raster frames keep the environment estimate, as non-RT does.
3. **Hits on another program's object** (MP1, [m7-mp1.md](m7-mp1.md)): mixed-program frames now render on both paths, but such a hit is shaded by the reflecting surface's program with the hit's material inputs, not by the hit's own program (Programs, below). The toon program is still untested as a reflection-hit program in production; only R0b's probe has tested it.
4. **One bounce.** Hits take their own untraced specular. RF7 (facing glossy panels) misses at 23.3 vs UE HW-Hit Epic 20.2; its recursive panel-to-panel reflections are missing.
5. **Rough band (roughness ≥ 0.5) is untraced.** RF8's wet rocks miss at 7.9 vs 3.9 (traced 0.3/0.4 bands 6.8/7.5; untraced 0.5 band 9.3). RF2's 0.5/0.7/1 spheres miss at 15.9/13.1/10.4 vs 15.3/11.9/5.5, with no local colour bleed, which needs the cache's bounce in `indirect_radiance` (G2b).
6. **RF2's mirror spheres** miss at roughness 0 and 0.05: 14.1/15.0 vs 11.6/14.4. The reference split noise is 4.8 codes, so this ranking isn't certified.
7. **Glossy grain without TR1.** Four static rays leave visible sparkle on roughness 0.3-0.4 metals that reflect lit and shadowed surfaces (RF2). TR1's rotated strata integrate it (m7-r1.md §5); with TR1 off, the sparkle remains.
8. **Hit lighting is simpler than camera-hit lighting.** At hits:
   - the sun is one hard shadow ray, with no disk penumbra;
   - the indirect inputs read the open environment, never the world radiance cache's occlusion or bounce. A cache lookup per hit cost +0.62 ms of quarry shading with the cache on (m7-r1.md §4);
   - footprints are an isotropic ray cone only;
   - decals (DC1) project onto hits through `decal_prepare`, as onto camera receivers.
9. **Motion is unmeasured.** RF3/RF5 motion, trails and settle against R0a's continuous Unreal runs: no new campaign tonight.
10. **Primary-miss background** (environment where the references are black) is R0a's registered renderer item, not a reflection error, but it dominates the full-image difference panels.
11. **Cost.**
   - On reflective content the cost is unmeasured; no comparison with Lumen's 0.07/0.22 ms reflection pass has been made.
   - The quarry traces no reflection rays (every material is roughness 0.85). It pays nothing measurable with the cache off (the default). With the cache on, the shading increment depends on code generation: +0.29 ms in one build, +0.04 ms after merging DC1, for the same R1 code (m7-r1.md §4). If G2b makes the cache the default, the increment can return with any compile. Candidate fixes are a separate hit-shading pass, or a second program composition with open-only indirect inputs.
12. **RT quarry with the cache on:** with no ray traced, the same 309 small pixel clusters differ from main's program in two runs, so the difference is deterministic. It is attributed (I) to codegen changes in the cache lookup's inputs, now prepared after `surface`. It is unscored against the reference; the cache is off by default (m7-r1.md §4).
13. **Programmability constraint:** in reflecting modules, `surface()` reads the open environment from the indirect inputs; `shade()` reads the prepared values. Reflection hits always read the open environment.

## TR1 (temporal reconstruction, default on; TR1b)

TR1 is the default on traced and visibility-buffer lit frames, both paths, from TR1b's overnight pass; `Reconstruction::off()` switches it off ([m7-tr1b.md](m7-tr1b.md)). It is never presented as meeting the Unreal bar.

**Reach.** RTDEF2 makes traced primaries and TR1 the capable 1x RT default; the viewer negotiates that path. Raster/4x and unsupported-capability fallbacks still do not reconstruct. Temporal soft-shadow frames keep their own film history and are not reconstructed.

**Animated scenes (MQ1, [m7-mq1.md](m7-mq1.md)).** Before MQ1, any transform edit restarted every pixel, so the example scene (and any game with a moving object) got no anti-aliasing and no glossy integration. Moved placements now reproject through their previous transforms, and frames where something moved clamp history to a plain 3x3 min/max box. Remaining gaps, measured on the example flight against a 2x2 supersampled TR1-off control:
- **RT mirror and glossy reflections lag under camera motion**: chrome sphere 3.07 → 6.44 codes RMS and flicker 4.05 → 9.05 versus main's restart; gold sphere static 4.34 → 4.84, although its grain falls 62 %. Surface-identity history follows the surface, not the reflected image. The fix needs a program-declared view sensitivity (window capped by lobe width) or reflection hit distances. Static scenes under camera motion have the same lag without the box (TR1b's U1b failure).
- **Skinned characters with GI on still restart every static pixel's history** on animated frames (SK1's scene-wide swept-volume widening). Recommended: set MQ1's box for skinned motion and keep the swept volume for direct shadows only.
- Revealed pixels behind moving objects start from one jittered sample for about four frames; edges between two differently moving objects keep some stepping.
- The example's `--quarry` mode fails on both paths (mixed programs, MP1).

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
- The capable 1x RT viewer now renders traced primaries with TR1 (RTDEF2). Upscaling presets remain library settings; the viewer has no preset flag.

**Reference instrument.** The visible-sky references were traced by a release-profile test binary; on fixed resources its first renders differ from later ones by ≤2 ULP, which the bounded-AS policy's late fixed-resource control does not see. A dev-profile build of the same source (U0b's profile) reproduces quarry-final byte for byte on 8 independent and 4 fixed replays. The sky references stay accepted under the policy; a dev-profile re-trace was stopped by the overnight no-new-references rule.

## GI

G2b ([m7-g2b.md](m7-g2b.md)): the world radiance cache is on by default on both paths, with bounce light and ES1's blocked sky specular, in G2's measured configuration (4 px cells) plus a per-frame gather cap. `IndirectSettings::off()` switches it off. It beats the cache-off image on 20 of 22 quarry populations after TR1 and Lumen HW/SW Epic on all-covered quarry error (G2's tables).

- **Cost against Lumen** (GC1, [m7-gc1.md](m7-gc1.md); functional runs, not quiet-classified). With the camera moving after a cut has drained, the default cache adds +1.60-2.01 ms on RT and +1.90-2.98 ms on non-RT (p50 1.68 / 2.51) over the cache-off frame, down from +1.98-2.22 / +2.75-2.95 on main before GC1. Lumen Epic's GI pass on the same quarry is 1.24 ms (HW) and 1.21 ms (SW), so the 2x target still misses by about 2.6-3.2x (RT) and 3.1-4.9x (non-RT).
  - **Largest stage: the shading pass's increment**, +1.01 ms (RT) and +0.78-0.88 ms (non-RT). It grew since G2b's +0.6 as main's shading grew, and is codegen-sensitive (a lookup variant that saved 0.1 ms on RT cost 0.07 ms with the cache off and lost on non-RT).
  - Next: the probe (0.43-0.45 ms on both paths), and on non-RT the query points of new cells (0.34 ms), gathers (0.22 ms) and their visibility rays (0.14 ms).
  - Not built: a half-resolution lookup with geometry-aware upsampling (a separate pass would take the lookup's registers out of the shading shader), level hysteresis, and an on-screen-first cut gather order.
- **Camera cuts.** The 20-67 ms gather frames are gone (gather cap). The worst cut frame is now 11.3 ms (RT) and 18.9 ms (non-RT, cell creation in the first frame, unbounded by default). Bounce then settles over about 23 frames (RT) and 93 frames, about 1.5 s (non-RT), not 8 frames (GC1: gathers whose hits are all final stop early, a fifth fewer cut gathers; G2b read 28 frames and 2-4 s). While non-RT drains, flat surfaces speckle (SV test scene); `gather_cells: u32::MAX` restores fast settle with the big cut frames. An opt-in claim cap bounds cell creation but settled the test scene worse.
- **Crevice mouths.** Dark streaks and pinholes at crevice mouths remain (TR1x64 pinholes on the merged TR1b default: RT 142 and non-RT 128, against cache-off's 74). Measured fix: 2 px cells (pinholes at the cache-off level, every quarry population better), opt-in only: it speckles flat surfaces on SV and needs a 448 MiB table, over the non-RT device's 256 MiB buffer limit. No per-pixel short-ray check was built.
- **Enclosed cavities.** A coarse per-cell mottle shows in cavities lit through narrow openings (the quarry's cave and hollow) where the reference is smooth. The hollow is also about 5.5 codes too dark: es1open beats the cache there by 3.6-4.5 codes. More gathers (up to 7) do not change it. It needs a fixed-weight cell-space filter or more directions per cell.
- **Other losses kept from G2.** On isolated rocks and background-centre the cache loses to cache-off at TR1x64. Against ES1's unoccluded diagnostic it loses on 10-16 populations by 0.1-0.6 codes. It loses to Lumen HW on edge and multi-surface.
- **Path-traced re-score not done.** TR1's visible-sky walk was not re-scored against the path-traced reference with the cache on: no new reference campaign tonight. The shared lighting error should shrink with the cache's sky occlusion; this is unmeasured.
- **Still frames after a cut** run the cache passes for up to 16 + 64 x (gathers + 1) frames while capped gathers may still be pending, about 0.5-1 ms each.
- **Other units' harnesses.** `src/reference_r0a_tests.rs` asserts the cache starts off, and `src/iq1_tests.rs` uses `IndirectSettings::default()` to mean off. Both need `IndirectSettings::off()`.

G2c ([m7-g2c.md](m7-g2c.md)) fixed API1's game-scene ground speckles (glossy and mirror cells sent their sun highlight in every direction) and arch-edge streaks (not-yet-gathered cells read as zero bounce at level switches). What it leaves:
- **Shiny metals bounce too little.** A cell reflects as a Lambertian surface with the program's directional albedo over 16 fixed directions, which cannot integrate lobes much narrower than their ~36° spacing. Gold at roughness 0.25 reflects about 0.4-0.5 of a true ~0.8, and chrome at 0.02 about 0. Near shiny metals the bounce is too dark (energy lost). Mirror caustics are not represented at all: no view-independent cache does that.
- **Light-direction response beyond the cosine does not reach bounce.** Lights reflect through the albedo times max(n·l, 0). A program's wrap lighting, transmission or retro-reflection shades its own pixels but bounces as Lambert.
- **Energy cap.** The albedo is capped at 1 per channel. It binds only for programs whose `scatter` is not energy-conserving, or for a normal-mapped mirror whose lobe meets a quadrature direction head-on.
- **Bounce latency at level switches** is now filled from the coarser level, which is blurrier than the settled finer level until the new cells' first gather (a few frames under the gather cap). Moving toward a surface shows a softer bounce band, not a dark one.

## Lights

LC1 ([m7-lc1.md](m7-lc1.md)), extended by LC2 ([m7-lc2.md](m7-lc2.md)): clustered culling of point and spot lights, plus shadows for the 4 most important local lights (rays where the frame binds an acceleration structure, pooled PCF maps otherwise), on by default on both paths. The soft sun now renders with any number of further lights. Measured on the quarry at night with 260 lights; no Unreal or path-traced comparison was run tonight.

- **Forward raster cost.** RTDEF2 now defaults capable RT frames to traced primaries with TR1. Explicit forward raster still shades every rasterized fragment; LC2's diagnostic 261-light quarry costs 43.94 ms with hard local maps and 67.45 ms with soft maps, versus 11.93/12.27 ms traced and 17.55/18.08 ms non-RT VB. This is one contended drained-frame run, not a comparative performance certification.
- **Only N lights are shadowed** (default 4, at most 16). All other local lights leak light through geometry. Unreal's VSM shadows every movable local light, with page caching.
- **Selection by illuminance at the camera** (candela / d², d >= 1 m, frustum-tested, 25 % hysteresis). It is not screen-coverage weighted: in the test it picked two dim lights near the camera over the two bright points. Selection changes pop shadows on and off with no fade.
- **Local penumbra limits (LC2).** Point/spot `source_radius` now enables four disk-sampled RT rays integrated by TR1, or PCSS blocker search and variable PCF. Radius zero preserves hard visibility. Maps cap support at 96 texels and use 12 blocker-search taps; sparse blockers can be missed. RT visibility uses centre incidence and retains four-ray grain without TR1. No new local-shadow reference or motion campaign certifies the Unreal floor.
- **Maps draw full-detail meshes** (no cluster-LOD raster) and only regular draws. Virtual-geometry casters are missing from local maps. Moving shadowed lights redraw every frame: +5.7 ms for 4 spot faces on the quarry, more for cube lights. Radius-zero PCF clamps at cube-face edges; LC2's positive-radius PCSS reprojects cross-face taps. The 0.02 m near plane drops casters closer to the light (rays have no near plane).
- **Unclassified map/ray differences.** Fine speckle on the gravel pile with maps (pebble shadows or acne), and one region behind the large rock that rays shadow and maps don't. Neither was classified tonight.
- **GI light-list scaling (LC2).** Every light now reaches bounce through a dynamically sized per-cell bitset, with range/cone/hemisphere rejection. Both deferred paths record 171 contributing lights after the first 32 in the night quarry. At 261 lights the default table grows by 16 MiB to 128 MiB plus its header. Work is bounded by queued cells times light count, without a separate ray budget; dense overlap remains expensive. Bounce visibility still uses hard centre rays. The explicit forward-raster control has no GI-cache pass.
- **Unculled stages walk every light:** R1 reflection hits, which run before the cluster is prepared; film accumulation (Temporal soft mode, opt-in); and GI visibility selection. Cell shading walks the recorded visible-light bits. Their cost grows with the total light count on reflective content. Reflection hits shadow unselected local lights not at all (R1's hit shadow skips lights without shadow views).
- **Cluster capacity.** 128 lights per cluster (the test's longest was 99); beyond that, a cluster drops its highest-indexed lights, and the dropped count is recorded in the list. Sets above 65,535 lights walk every light.
- **Programmability constraints:** implicit-derivative sampling inside the light loop is rejected (per-receiver loop bound), and fixed-index light addressing now means "the cluster's i-th light".
- **Memory:** pooled maps take 96 MB at the default 4 slots x 6 faces x 1024² Depth32Float, allocated on the first map frame (cube layers are reserved even for spot slots), plus 3.6 MB of lists and a 6 KB uniform.
- **Timing** is one contended drained-frame run (median and min of 24), not a quiet-window measurement.

## Programs

From MP1 ([m7-mp1.md](m7-mp1.md)): per-object shading programs on both paths, by per-program tile lists.

1. **Reflection hits on another program's object** are shaded by the reflecting surface's program, with the hit's material inputs and the receiver's parameters (m7-mp1.md 1.4). RF10's reflected toon sphere is smooth, not banded: RT 22.92 on that population against UE HW-Hit Epic 10.42 and SW Epic 12.99 (whole reflector 12.34, better than both floors). The fix is a per-program hit pass: trace and record hits, shade them in per-program lists by the hit's program, then let receivers read the result. It would also take R1's inlined hit shading out of the receiver's kernel (R1's cost gap 11).
2. **Non-RT RF10** scores 62.27 against UE SW Epic 53.12: non-RT has no traced reflections yet (Reflections 1). R1n's software hits must use the same seam as R1's (`program::compose_list` replaces the hit's parameter load once); a second hit-shading site needs the same treatment.
3. **Temporal film frames** (`SoftShadows::Temporal`, not a default) still admit one program per frame.
4. **Limits.** 32 programs per frame (the binning mask is one u32); 64 draws per frame as before. Programs are per material, not per instance.
5. **Compile hitches.** The first mixed frame compiles one list pipeline per program (the frame's shading variant only), concurrently, and a reload's list pipelines recompile at the next mixed frame rather than in the reload transaction. Debug example build, first frame: 49.0 s RT and 24.8 s non-RT for everything, against API1's 11.6-43.8 s with one program. Not measured in a release build.
6. **Mixed tiles** run every present program's dispatch over the whole tile; the others' pixels return after one hit-record read. Overhead grows with the number of program boundaries on screen. Quarry, cache off: +0.06-0.11 ms of shading with two programs and +0.05-0.07 ms with eight (standard plus toon variants), above the design's 0.03-0.05 ms estimate. The pure dispatch overhead (identical-cost programs) was not isolated: every attempt was stopped by the lease guard (m7-mp1.md 3).
7. **GI cells.** Each reflecting program's cell pipeline scans the whole cell queue, so cell shading grows linearly with the number of reflecting programs. Unmeasured beyond the quarry rows; the queue is bounded (32 slots).
8. **Unverified:** mixed frames with streamed textures (`TextureResidency` other than `Full`; composed and naga-checked, never rendered), mixed frames with RT1d's visibility filter, motion and disocclusion across program boundaries under TR1 (only static frames and the example's fly path were inspected), and Hard (inline-ray) shading in mixed frames (the gates use Interactive and Virtual).
9. **Memory with many photoreal programs.** The eight-standard-program quarry run took available memory below the lease guard's 2 GiB twice (32 GB machine, other units running), where the toon-program run bottomed at 5.1 GiB. Not diagnosed: concurrent list-pipeline compiles, per-program GI cell pipelines and per-switch material churn are candidates.
10. **No Unreal comparison** of per-object materials: Unreal assigns materials per mesh natively; the 2x speed gate on matched workloads was not measured for mixed frames.


## Startup

PC1 ([m7-pc1.md](m7-pc1.md)) persists engine and program Vulkan pipeline caches before
initialization. Full-image identity passes on game/quarry, RT/non-RT, dev/release.
The shared-machine release RT game sample falls from 48.771 s with an empty application
cache to 0.745 s with a populated one. These are single diagnostic samples with an
uncontrolled NVIDIA private cache, not a universal speedup or a QUIET-window claim.

- Cold startup remains expensive: release RT game was 48.771 s empty-cache versus
  37.497 s with persistence off. A real unseen program edit still took 31.335 s to
  readback; 95.7 ms of module work was followed by a 31.081 s first-use mixed-program
  list pipeline compile. Background/prewarmed creation of unseen variants remains open.
- A driver-private-cache hit can already make startup fast. The dev matrix shows no
  consistent extra benefit from persistence, and one non-RT warm sample is slower.
- Warm quarry preparation remains 9.479/27.306 s in dev and 4.415/15.236 s in release
  (RT/non-RT); resource/geometry/ray preparation and upload dominate non-RT. This unit
  changes no GI, acceleration-structure or asset-cooking algorithms.
- The timed quarry is the engine example with the complete registered scan asset and
  game props, using its game camera. Startup of the low-level viewer/quarry walk was
  not measured; its legacy program-only cache setter does not persist all startup kernels.
- The program edit check uses the API, without GI/TR1, on RT. File-watch debounce,
  physical scanout, a separate non-RT reload timing and native-only driver/SPIR-V
  attribution were not measured. Corrupt-file/key tests pass on CPU; GPU replay after
  corruption or an actual driver update remains untested. Cache files are bounded to
  64 MiB and concurrent writers retain the last complete snapshot.
