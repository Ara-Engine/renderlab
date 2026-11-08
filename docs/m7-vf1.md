# M7-VF1: shadowed volumetric fog on both paths

Overnight unit, 2026-11-08, branch `t3/m7-vf1`, following [AT1](m7-at1.md).
Scope: AT1's fog, its engine setting, and a read-only shadow accessor. The shadow
cache's algorithms, requests, rasterization and lookup internals are unchanged.

**Retained:** shadowed volumetric fog is on by default in the renderer and engine
facade, with 0.001/m extra density. The inspected quarry and game-scene defaults
add atmospheric depth without an obvious new silhouette or material-detail
regression. This is the overnight default decision, not an Unreal-quality claim.

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
- Editing atmosphere settings cuts TR1 history as well as froxel history. AT1's
  reconstruction key only tracked whether the sky was visible, so changing fog
  density could otherwise keep pixels shaded under the old medium.
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

The retained batch is **`b`**, after merging main through LC1 (`fa3b74b`, merge
`062d8dc`) and fixing the atmosphere-to-TR1 reset. Rust 1.96.0, Cargo release
profile, Ryzen 7 7800X3D / RTX 4070 SUPER, Vulkan. Source and executable SHA-256
hashes are in `source-identity-b.json`; `b-binaries.json` pins the staged
`bin/c/{lib,example}.exe` and the batch verifies those hashes before running.

Lease `b-1`: **19:21:11–19:24:54 UTC, 223.70 s**, exit 0, no guard failures.
Lib suite: **167 passed, zero failed, 151 ignored**. The isolated no-RT clipmap
test passed, all three quarry processes passed, and all six game runs completed.
Fmt and whitespace checks pass. Available RAM stayed at 4841–9829 MiB; one to
six compiler processes were sampled.

Verified by inspecting `images/b/shafts-sheet.png`, `quarry-default-sheet.png`,
`game-sheet.png`, `motion-sheet.png`, and full-size non-RT low/default, both
paths' dense lower-camera quarry, both paths' noon quarry and the RT dense game
image. The low quarry camera is at y=1.1 m, looking horizontally toward the
4-degree sun. Lit haze around the rock edges and the darker medium behind the
rocks appear on both paths; the subtle default keeps the dark pockets and fine
rock relief. Dense fog gives a stronger warm veil. This small floating scan
arrangement is still a weak witness for long, room-scale shafts.

The captured-sky quarry is only slightly faded by the default. The game scene's
arch, crates, glossy spheres and their shadows stay readable on both paths.
The 7.5 cm camera move and hold show no obvious fog trail in the inspected
frames. A handful of thin bright rock structures remain on deferred frames;
their source is not isolated here, and this is not scored motion acceptance.

![Low quarry: fog off, subtle default, dense; non-RT, traced RT, raster RT](D:/Coding/renderlab-work/m7-vf1/images/b/shafts-sheet.png)

![Captured-sky quarry: previous and new defaults](D:/Coding/renderlab-work/m7-vf1/images/b/quarry-default-sheet.png)

![Game scene: fog off, default and dense on both paths](D:/Coding/renderlab-work/m7-vf1/images/b/game-sheet.png)

![Short camera move and hold on all three paths](D:/Coding/renderlab-work/m7-vf1/images/b/motion-sheet.png)

**Validation caveat:** each quarry/game process logged ten Vulkan
`VUID-StandaloneSpirv-None-10684` diagnostics about explicit Offset/ArrayStride
decorations in Function/Workgroup storage. They start during renderer pipeline
creation, before fog is enabled. The lib suite and isolated clip reader logged
none. Rendering succeeds, but this is not a validation-clean result; the Naga
SPIR-V layout issue needs separate investigation.

Low-sun static throughput (ms/frame wall clock, the two paired rounds):

| Path | Analytic only | Default volumetric | Dense volumetric |
|---|---:|---:|---:|
| non-RT VB + Virtual | 4.222 / 4.491 | 4.106 / 4.125 | 3.672 / 4.857 |
| RT traced + Interactive | 6.064 / 6.021 | 5.873 / 5.677 | 5.999 / 5.632 |
| RT raster + Exact | 17.070 / 17.259 | 17.243 / 17.174 | 17.116 / 17.434 |

Noise changes the sign of several deltas. These are not a demonstrated fog
speedup or a precise overhead bound. Wall throughput includes all atmosphere
work on both paths. The game logs also retain CPU/frame and GPU-span samples;
their RT GPU spans exclude atmosphere work before primary visibility and must
not be read as whole-frame fog cost. Raw timings and finite-HDR comparisons are
in `images/b/summary.json` and each path's `*-at1.json` / game `run.json`.

![Paired diagnostic timing samples; no speed claim](D:/Coding/renderlab-work/m7-vf1/images/b/timing-chart.png)

Time spent: implementation/integration about 30 minutes of active work
(estimate); final compilation 115.1 s, retained GPU verification 223.7 s.
Initial verification used a separate 212.0 s lease. The two GPU queues took
11m23s and 11m25s; compiler interruptions and earlier attempts are separate from
the retained run. Raw HDR dumps over 5 MB and the owned Cargo target were
deleted after summarizing; logs, JSON and inspected PNGs remain on D:.

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
- Conventional non-RT raster-map fog is shader-validated but was not rendered
  in this unit; the tested non-RT path is the visibility buffer plus Virtual.
- Vulkan's SPIR-V explicit-layout diagnostics above remain unresolved.
- No new Unreal or participating-media reference campaign under the overnight
  rules. Scored motion/disocclusion, broad content and transparency interactions
  remain open; see [the gap list](m7-gaps.md).
