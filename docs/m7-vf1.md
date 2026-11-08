# M7-VF1: shadowed volumetric fog on both paths

Overnight unit, 2026-11-08, branch `t3/m7-vf1`, following [AT1](m7-at1.md).
Scope: AT1's fog, its engine setting, and a read-only shadow accessor. The shadow
cache's algorithms, requests, rasterization and lookup internals are unchanged.

## Implementation

- Non-RT visibility-buffer frames read the Virtual clipmap's top depth layer: one
  height test at the finest resident page covering the froxel, falling back through
  coarser levels. A page must match its level epoch. Uncovered volumes are lit;
  fog makes no page requests and never walks triangles. The accessor reuses the
  shadow owner's `VsmFrame` declaration and coordinate helpers verbatim.
- Conventional non-RT raster frames sample their existing reversed-Z sun cascade
  maps. Hardware renderers keep AT1's one centre-sun TLAS ray per froxel.
- Fog injection runs after Virtual page generation, before shading. No additional
  shadow raster, BVH or material-dependent fog dispatch is introduced.
- History reprojects froxel centres rather than jittered sample positions, with
  half-cell coverage rejection. Lighting, exposure, medium, sky, path and uploaded
  caster changes invalidate it. Density changes reject history; the old source is
  clamped to the current medium's sun/ambient range, with reduced weight when
  visibility changes rapidly. Extinction itself is current-frame data.
- The visible atmosphere backdrop now includes the near volumetric range. AT1
  applied froxels only to surfaces but skipped the near range on the sky.
- `Atmosphere::default()` and `engine::Settings::default().volumetric_fog` select
  `Some(VolumetricFog::default())`: 128 m, extra extinction 0.001/m, history weight
  0.9, plus the existing height fog. This is about 1.2% extinction over 10 m at
  ground level. `None` disables volumetric fog. The game example inherits that
  setting, keeps `--volumetric-fog` for 0.01/m shafts, and adds
  `--no-volumetric-fog` for comparisons.

No new patent-flagged method is used: this is depth-map visibility and AT1's
existing jitter/reprojection. Shading remains programmable; fog is composed in
the generated camera-ray entries after user shading.

## Verification and evidence

Reproducer: `py -3 -B scripts/m7-vf1.py build`, then
`py -3 -B scripts/m7-vf1.py gpu LABEL`. The second command runs through
`scripts/n8-run.py` and the shared `D:/Coding/renderlab-work/gpu-slot.lock`, checking
game holds and foreign QUIET windows throughout. The two-job compiler watchdog
also stops for QUIET windows. Every renderer test binary runs inside the lease.
Targets, staged binaries, images, JSON and logs are under
`D:/Coding/renderlab-work/m7-vf1/`.

The batch includes the lib suite; a synthetic no-RT clipmap contract test
(lit/blocked height, coarse fallback, stale epochs, no coverage); the registered
13 M-triangle quarry at 1920x1080, including 4-degree sun; a short camera move and
hold; and `examples/game_scene.rs` at 1280x720 with fog off, default and 0.01/m on
both paths. Quarry comparisons share one asset load per path. Timings use 32
static frames after 20 warm frames, two rounds per configuration, within that
same process. These are diagnostic shared-host throughput measurements, not a
quiet performance claim or a comparison with Unreal.

Batch `a` completed inside lease `a-1` at 19:00:02–19:03:34 UTC (211.98 s),
exit 0, with no guard failures. The lib suite passed **163 tests**, zero failed,
150 ignored; the no-RT clipmap contract test and all three quarry paths passed.
The six game-scene runs (fog off/default/dense, both paths) completed.

Inspected: `images/a/quarry-sheet.png`, `quarry-default-sheet.png`,
`game-sheet.png`, `motion-sheet.png` and full-size dense quarry images on non-RT
and traced RT. The default keeps silhouettes and small material detail, adds
subtle warm low-sun depth, and changes the captured-sky quarry only slightly.
Dense fog remains darker under the rocks on non-RT, as on traced RT. This wide
quarry camera is a weak long-shaft witness; a lower camera is included in the
merged-main recheck. The moving deferred images retain thin bright structures
on some dark rock features; this is not certified fog-motion acceptance.

**Validation caveat:** each quarry/game process logged ten Vulkan
`VUID-StandaloneSpirv-None-10684` diagnostics about explicit Offset/ArrayStride
decorations in Function/Workgroup storage. They start during renderer pipeline
creation, before fog is enabled. The lib suite and isolated clip reader logged
none. Rendering succeeds, but this is not a validation-clean result; the Naga
SPIR-V layout issue needs separate investigation.

Batch `a`'s low-sun static throughput (ms/frame, the two paired rounds):

| Path | Analytic only | Default volumetric | Dense volumetric |
|---|---:|---:|---:|
| non-RT VB + Virtual | 3.537 / 3.686 | 3.646 / 3.561 | 3.587 / 3.662 |
| RT traced + Interactive | 5.621 / 5.526 | 5.588 / 5.631 | 5.663 / 5.597 |
| RT raster + Exact | 18.238 / 16.880 | 17.316 / 17.102 | 17.033 / 17.091 |

Noise changes the sign of several deltas. These are not a demonstrated fog
speedup or a precise overhead bound. Raw paired timing and finite-HDR summaries
are in `images/a/summary.json`; source/executable SHA-256 hashes are in
`source-identity.json`. The final merged-main batch is recorded below when complete.

## Remaining gaps

- Fog does not request Virtual pages. Empty regions without valid coverage can
  leak sunlight; cascade reach bounds conventional raster fog. Virtual caster
  LOD, map texels and cached sun-basis hysteresis can differ from RT visibility.
- Both paths use centre-sun visibility for fog, without a soft disk integral.
- Selected visible-environment and constant-clear backgrounds still bypass fog;
  the backdrop correction here applies to the procedural atmosphere sky. Their
  surface hits remain fogged.
- Changing casters or sunlight resets history instead of preserving unaffected
  cells. Fast changes can repeatedly expose noisy starts.
- No new Unreal or participating-media reference campaign under the overnight
  rules. Scored motion/disocclusion, broad content and transparency interactions
  remain open; see [the gap list](m7-gaps.md).
