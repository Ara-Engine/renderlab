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

## RT shadows

[RTDEF](rtdef.md) compares raster + Exact with traced + Interactive + the latest
default TR1b on main `ae95d3a`. The quarry improves, but **the RT default is not
promoted**: S2's cold and moving disk penumbrae are visibly grainier than
raster + Exact, including at ordinary exposure and at the registered +4-stop stress.
The cold disk witness is 14.0832 display codes RMS versus raster + Exact's
12.6000 and retained HRT1's 13.6655. Settled TR1 substantially improves it.

- History restarts on scene-transform/instance uploads. Moving casters can
  therefore retain eight-ray startup grain indefinitely; a long still hold
  does not establish camera/reveal/caster-motion acceptance. In S2's moving
  blocker frame 63, the single-surface floor penumbra is 9.8755 codes RMS at
  +4 stops versus raster + Exact's 6.6179. This diagnostic does not replace
  the complete-image scorer.
- All 24 S2 phase pools beat retained HRT1 in linear HDR and +0/+4 display
  RMS. Per-frame +4 RMS still loses on **10/1,280 frames**: all eight cold
  frame-0 cases, camera .25 px/point frame 1 and camera 1 px/point frame 122
  (return/reveal). Linear HDR and +0 RMS win on all 1,280 frames. The HRT
  comparison is non-blocking tonight; the default decision is held by the
  visible regression against raster + Exact, not by these HRT rows.
- The previous unfiltered Interactive visibility/flicker results in
  [RT1c](rt1c-rt-default.md) still apply only to those registered masks.
  Post-reconstruction visibility is not separately measured by the diagnostic
  shadow shader, and is not inferred from its raw eight-ray visibility.
- RT1d's fixed spatial filter remains opt-in: its quarry false-shadow,
  lit-darkened and strict-solid losses were not repaired by this unit. No
  film/Unreal quality floor or combined 2× performance floor is claimed here.
- RT library and viewer defaults remain raster + Exact, so the RT default
  still receives no TR1 anti-aliasing. Traced primaries and Interactive remain
  settings; their historical TR1-off speed labels remain explicitly scoped.

## Atmosphere

AT1, [m7-at1.md](m7-at1.md). The sky, aerial perspective, height fog, sun transmittance and procedural environment are on by default. Volumetric fog is opt-in.

- **Non-RT volumetric shadows are missing.** Froxels on renderers without ray queries see the sun unshadowed, so fog in a shadowed canyon glows. This is why volumetric fog is not a default on either path. Candidates:
  - the Virtual clipmap's top layer (needs a read accessor in the shadow-owned files and a rule for unrequested pages);
  - the radiance cache's software BVH (exists only with the cache on; one ray per froxel is too slow, so it needs visibility amortized over frames);
  - a cached low-resolution fog shadow map.
- **The analytic height fog's sun term is unshadowed** beyond the volumetric range, and everywhere while volumetric fog is off, as in Unreal's exponential height fog. Under the default density (about 2 % per 100 m) it isn't visible on the quarry (10 m scale). Dense fog in shadowed spaces will glow.
- **No matched Unreal comparison.** Neither Unreal's SkyAtmosphere nor its ExponentialHeightFog was measured tonight (no new Unreal campaigns). The medium is Hillaire 2020's Earth, the same model Unreal's SkyAtmosphere implements, but no image or timing comparison exists.
- **No path-traced reference** for the sky or the fog. The reference path tracer has no participating media.
- **A captured sky with a non-physical sun shows a dim backdrop.** The procedural sky scales with the first directional light's lux. The quarry's captured-sky protocol (a 1000 lux sun beside an RLE2 sky) therefore shows a near-black sky on misses, as it did before AT1 (a black clear colour). Scenes with a captured sky should turn on `set_visible_environment` until misses follow the selected map automatically.
- **Only one sun.** Only the first directional light drives the sky, the sun disk, the fog and transmittance. A second directional light (a moon) gets none.
- **The procedural environment regenerates on every sun change.** Each regeneration is a new map, so the radiance cache flushes too. A moving sun (time of day) therefore flushes GI every frame it moves, and time of day needs in-place updates the cache can track. The environment ignores camera altitude changes after generation.
- **No temporal visibility on the sky or fog.** Volumetric reprojection uses plain history blending (0.9) with no rejection. Fast camera moves or a moving sun leave froxel lag (not measured tonight).
- **Transparent surfaces:** `atmosphere_surface` is ready for TP1's transparent entry. It isn't wired there in this unit.
- **Film accumulation (`SoftShadows::Temporal`)** fogs its hits, but its misses keep `film_background()` (no sky).
- **Slice resolution:** the aerial perspective's first slice ends at 7.8 m, and froxels are 16 px columns. Neither was screened for banding or aliasing in motion.
- **The sun disk has a hard, unfiltered edge** (true 0.00465 rad radius, saturating at the HDR ceiling). Its rim aliases, and raster and deferred misses differ on 117 rim pixels. It needs an analytic coverage edge, or TR1 on misses.
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

1. **Non-RT reflections (R1n, [m7-r1n.md](m7-r1n.md)) trace in software, with limits.** R1's rays now run on non-RT visibility-buffer frames over per-mesh software BVHs, on by default. They beat UE SW Epic on RF1-RF7 and RF9 (RF1 4.15 vs 37.2, RF2 13.8 vs 15.9, RF6 6.7 vs 12.3, RF7 23.3 vs 35.6) and miss it on RF8 (7.96 vs 5.24, the untraced rough band, item 5). Against UE HW-Hit Epic they miss RF7 and RF8, as R1 does. They match R1's RT scores within 0.04 codes. What is still missing:
   - **Large and virtual meshes have no software geometry.** Meshes over 2^19 triangles and streamed clusterLOD meshes get none, and a frame drawing any such mesh traces nothing (the coverage rule, so no ray leaks sky through a missing mesh). The quarry (twelve ~1 M-triangle scans) therefore keeps the environment estimate. The fix is R0c's cooked cluster-LOD cut chosen by ray-cone footprint, carrying drawn-triangle identity (or position-free leaves into the draw's own buffers), so hit shading keeps R1's contract.
   - **Skinned meshes (SK1) are absent from non-RT reflections.** Their vertices move on the GPU every frame, so they are left out of the software geometry: they neither appear in nor block non-RT reflection rays, and frames that draw them still trace the rest. The fix is a per-frame CPU skin or a GPU-built tree for skinned meshes.
   - **No LOD selection by ray cone.** Full detail is the only level; R0c's `select_cone_level` is not used yet.
   - **Cost on reflective content is high** (m7-r1n.md §3): +1.6 to +3.5 ms of shading at 1080p on the mirror and puddle fixtures, +6.7 ms on RF8's glossy rocks (functional, contended), against Lumen's 0.07/0.22 ms reflection pass (U0, A). A flat instance loop, a 32-entry stack in the shading kernel and hit shading inlined beside it are the likely causes (I); a separate trace pass, a two-level tree and wider nodes are the levers.
   - **Quarry with the cache on (the default): +0.30 to +0.52 ms of shading although nothing is traced** (m7-r1n.md §4), R1's codegen effect on RT now on the default path too. Images are bit-identical (cache on) with TR1 off. Fix as R1's item 11: a separate hit-shading pass or an open-only composition.
   - **Edge slack.** Möller–Trumbore is not watertight; a 4e-6 barycentric slack closes shared-edge seams (the identity quad leaked sky without it). Hits can lie up to that fraction outside their triangle.
   - **Transforms are rewritten whole** each frame they change; there is no refit and no moving-object special case.
2. **Raster primaries get no traced reflections.** The RT device's code default is still raster primaries (IQ1, RT1d), and only traced-primary frames trace reflections. Raster frames keep the environment estimate, as non-RT does.
3. **Mixed-program traced frames are rejected**, so a reflection hit is always the frame's one program. RF10 (a toon sphere in RF1) still produces no frame on either path. The toon program is untested as a reflection-hit program in production; only R0b's probe has tested it.
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

## GI

G2b ([m7-g2b.md](m7-g2b.md)): the world radiance cache is on by default on both paths, with bounce light and ES1's blocked sky specular, in G2's measured configuration (4 px cells) plus a per-frame gather cap. `IndirectSettings::off()` switches it off. It beats the cache-off image on 20 of 22 quarry populations after TR1 and Lumen HW/SW Epic on all-covered quarry error (G2's tables).

- **Cost against Lumen.** One functional timing run with the camera moving: +1.32-1.62 ms on RT and +2.59-2.83 ms on non-RT over the cache-off frame. Lumen Epic's GI pass on the same quarry is 1.24 ms (HW) and 1.21 ms (SW), so the 2x target misses by about 2.2-2.6x (RT) and 4.3-4.7x (non-RT), and the ~0.45 ms budget by 3-6x. Largest items: the shading lookup (+0.6 ms on both paths) and the probe (0.43 ms); on non-RT also gathers (0.74 ms) and query points for new cells (0.53 ms). The run was not quiet-classified.
- **Camera cuts.** The 20-67 ms gather frames are gone (gather cap). The worst cut frame is now 11.3 ms (RT) and 18.9 ms (non-RT, cell creation in the first frame, unbounded by default). Bounce then settles over about 28 frames (RT) and 2-4 s (non-RT), not 8 frames. While non-RT drains, flat surfaces speckle (SV test scene); `gather_cells: u32::MAX` restores fast settle with the big cut frames. An opt-in claim cap bounds cell creation but settled the test scene worse.
- **Crevice mouths.** Dark streaks and pinholes at crevice mouths remain (TR1x64 pinholes on the merged TR1b default: RT 142 and non-RT 128, against cache-off's 74). Measured fix: 2 px cells (pinholes at the cache-off level, every quarry population better), opt-in only: it speckles flat surfaces on SV and needs a 448 MiB table, over the non-RT device's 256 MiB buffer limit. No per-pixel short-ray check was built.
- **Enclosed cavities.** A coarse per-cell mottle shows in cavities lit through narrow openings (the quarry's cave and hollow) where the reference is smooth. The hollow is also about 5.5 codes too dark: es1open beats the cache there by 3.6-4.5 codes. More gathers (up to 7) do not change it. It needs a fixed-weight cell-space filter or more directions per cell.
- **Other losses kept from G2.** On isolated rocks and background-centre the cache loses to cache-off at TR1x64. Against ES1's unoccluded diagnostic it loses on 10-16 populations by 0.1-0.6 codes. It loses to Lumen HW on edge and multi-surface.
- **Path-traced re-score not done.** TR1's visible-sky walk was not re-scored against the path-traced reference with the cache on: no new reference campaign tonight. The shared lighting error should shrink with the cache's sky occlusion; this is unmeasured.
- **Still frames after a cut** run the cache passes for up to 16 + 64 x (gathers + 1) frames while capped gathers may still be pending, about 0.5-1 ms each.
- **Other units' harnesses.** `src/reference_r0a_tests.rs` asserts the cache starts off, and `src/iq1_tests.rs` uses `IndirectSettings::default()` to mean off. Both need `IndirectSettings::off()`.

## Lights

LC1 ([m7-lc1.md](m7-lc1.md)): clustered culling of point and spot lights, plus shadows for the 4 most important local lights (rays where the frame binds an acceleration structure, pooled PCF maps otherwise), on by default on both paths. The soft sun now renders with any number of further lights. Measured on the quarry at night with 260 lights; no Unreal or path-traced comparison was run tonight.

- **Raster RT default cost.** Forward raster shading runs the light loop for every rasterized fragment. The quarry's overdraw and quad overshading on 13 M scan triangles make 256 lights cost +24.7 ms there (16.8 to 41.5 ms), against about +1.2-1.8 ms on the deferred traced and VB paths. The fix is a depth prepass or deferred primaries on the RT default. RTDEF kept raster + Exact as the RT default.
- **Only N lights are shadowed** (default 4, at most 16). All other local lights leak light through geometry. Unreal's VSM shadows every movable local light, with page caching.
- **Selection by illuminance at the camera** (candela / d², d >= 1 m, frustum-tested, 25 % hysteresis). It is not screen-coverage weighted: in the test it picked two dim lights near the camera over the two bright points. Selection changes pop shadows on and off with no fade.
- **Hard local shadows.** The Light API has no source radius. Rays give point-source hard shadows, and map PCF gives a fixed 3x3-texel softening, not a physical penumbra. Unreal has source-radius soft shadows on both paths.
- **Maps draw full-detail meshes** (no cluster-LOD raster) and only regular draws. Virtual-geometry casters are missing from local maps. Moving shadowed lights redraw every frame: +5.7 ms for 4 spot faces on the quarry, more for cube lights. PCF clamps at cube-face edges (no cross-face filtering). The 0.02 m near plane drops casters closer to the light (rays have no near plane).
- **Unclassified map/ray differences.** Fine speckle on the gravel pile with maps (pebble shadows or acne), and one region behind the large rock that rays shadow and maps don't. Neither was classified tonight.
- **GI bounce takes only the first 32 lights** in the lighting vector: the cache's per-cell light bits. Later lights add direct light only; in the test scene that includes the four bright ones. Importance-ordered GI lights would need cache-stable reindexing.
- **Unculled stages walk every light:** R1 reflection hits, which run before the cluster is prepared; film accumulation (Temporal soft mode, opt-in); and cell shading. Their cost grows with the total light count on reflective content. Reflection hits shadow unselected local lights not at all (R1's hit shadow skips lights without shadow views).
- **Cluster capacity.** 128 lights per cluster (the test's longest was 99); beyond that, a cluster drops its highest-indexed lights, and the dropped count is recorded in the list. Sets above 65,535 lights walk every light.
- **Programmability constraints:** implicit-derivative sampling inside the light loop is rejected (per-receiver loop bound), and fixed-index light addressing now means "the cluster's i-th light".
- **Memory:** pooled maps take 96 MB at the default 4 slots x 6 faces x 1024² Depth32Float, allocated on the first map frame (cube layers are reserved even for spot slots), plus 3.6 MB of lists and a 6 KB uniform.
- **Timing** is one contended drained-frame run (median and min of 24), not a quiet-window measurement.
