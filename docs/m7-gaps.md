# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

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

## Decals

DC1: [projected box decals](m7-dc1.md) modify resolved surface inputs before programmable shading on both paths. The API includes typed textures, alpha/multiply/add colour blends, stable sort order, angle fade, opacity and channel strengths. Placement activates them without another feature toggle.

- No matched Unreal or path-traced decal comparison was run in the overnight time-box; Unreal-equivalent decal quality and a 2× performance advantage are unproven.
- The one contended development-build cost run has large timing tails. Quiet optimized cost, empty-table overhead, table-publication latency and high-overlap scaling remain unmeasured; the median paired shading deltas (+0.011 ms RT, +0.008 ms non-RT) are diagnostic only.
- Decal textures use trilinear filtering, with no anisotropic filtering, compression, texture streaming or normal-variance contribution to specular AA yet.
- The world-space grid never truncates overlapping lists, but widely scattered or heavily overlapping decals can increase candidate counts and publication cost. There is no GPU grid builder or world partitioning yet.
- Opacity controls overlay coverage and reaches user programs as material alpha. It does not cut opaque silhouettes or alter shadow coverage; consuming alpha in transparency is an integration task.
- Secondary reflection hits, moving/attached decal transforms and temporal reconstruction under substantial motion remain integration verification gaps. GI cell shading consumes decals and notices their changes, but a measured decal-to-bounce propagation test remains open.
- Forward lit GPU images and combined GI/TR1 configurations remain unverified. The retained traced and actual no-RT tests passed with zero Vulkan validation errors; quarry still and 1 cm moving images were inspected on both paths.
