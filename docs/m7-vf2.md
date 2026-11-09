# M7-VF2: volumetric light shafts

Overnight unit on `t3/m7-vf2`, starting from `origin/main` (`b4e97dc`),
with AT2 merged at `ef189dc`. No Unreal or participating-media reference
campaign. Production ownership is atmosphere/fog only. The small prerequisite
outside that ownership shares `primary_ray_gpu::FRAME_BYTES` with post GPU's
primary-surface layout (128 bytes, previously a stale 112-byte literal).

Retained: 8-pixel columns, decorrelated two-sample shadow injection (four on
history resets), visibility-only reprojection with a plain neighborhood min/max
bound, current-view phase with default g=0.8, and fog bounded by the backdrop's
planet-ground hit. The two-window room shows distinct beams on both paths,
with dark air between them. Quarry backgrounds stop becoming an opaque
unshadowed underground fog colour. This is an overnight improvement, not a
claim of Unreal equivalence or complete temporal stability: the moving-sun
non-RT window sequence still exposes a transient light leak.

## Measured causes before changes

Source analysis is retained in
`D:/Coding/renderlab-work/vf2/baseline-analysis.json`.

- VF1's grid at 1920x1080 is 120x68x64: about 16 pixels per column.
  The fog's exponential distribution has a **0.22976 m first slice**,
  **0.75626 m thickness in the slice containing 10 m** (9.46172-10.21798 m),
  and a **6.96479 m last slice** at its
  128 m range. The gap list's 7.8 m first-fog-slice diagnosis was incorrect.
  In this source the separate 32-slice/32 km aerial volume starts at 31.25 m;
  neither number describes the fog's near sampling.
- VF1 already jitters a point within each froxel. Its jitter is identical
  across all cells in a frame. Shadow queries are single centre-sun rays on
  RT, or single top-depth-layer Virtual lookups on non-RT. Those lookups
  depend on resident pages, and do not request fog coverage.
- VF1 already reprojects cell centres, checks half-cell coverage and density,
  clamps old light to the entire physical lit/shadowed range, and reduces
  history weight on large light changes. It is not an unchecked 0.9 blend.
  A full 0.9 history takes 21.85 frames to discard 90% of an old signal;
  VF1's edge-change factor can reduce that weight to 0.27. Its broad clamp
  still admits sunlight into a currently shadowed cell.
- Henyey-Greenstein g=0.6 gives 0.79577 sr^-1 straight toward the sun;
  g=0.8 gives 3.58099, a 4.5x ratio. At 10 degrees the ratio is 2.595x;
  at 45 degrees g=0.8 is only 0.567x as bright. Anisotropy redistributes
  energy, so stronger g needs inspection away from the sun too.
- Default ground extinction is 0.0012/m (0.001 uniform extra density plus
  0.0002 height fog): 98.807% transmittance over 10 m. The height term's
  50 m scale height affects only one sixth of the ground extinction. Simply
  increasing uniform density strengthens the veil as well as the shafts.
- The backdrop applied height fog to 100 km and near volumetrics to their
  full 128 m even when the sky LUT's ray hit the planet ground much sooner.
  From a 1 m eye, a ray 10 degrees down hits at 5.7588 m. Continuing the
  height medium underground hits its 10,000 optical-depth cap: an opaque,
  unshadowed fog colour replaces the ground. Even using only a constant
  0.0012/m ground coefficient, the old 128 m near range gives T=0.8576
  instead of T=0.9931 at that ground hit (dense: 0.2710 versus 0.9430).
  This is a separate source of the quarry's broad veil, not a near-slice
  resolution problem. The CPU geometric calculation is in the analysis JSON.

## Verification protocol

`scripts/m7-vf2.py build` compiles the lib test executable with two jobs and
stages it under `D:/Coding/renderlab-work/vf2/bin`. `gpu LABEL` waits in-turn
for the exact reviewed root directive and its release gates in `gpu-after.json`,
then uses `scripts/n8-run.py` with a 15-minute cap. It rereads the directive
before acquisition and every subprocess step. Lib binaries only run in this
unit's lease. Targets, images and measurements stay on D:.

The synthetic scene has eleven 0.75 m slats with 0.45 m gaps, a floor and
side walls, an 8-degree low sun with a lateral component, a 960x540 image,
and -14.5 stop exposure. A 0.9 m camera move and hold and a continuously
moving sun exercise history. The second scene replaces the slats with a wall
containing two 0.8x2 m windows and an exact 8-degree sun aligned with the room.
The registered 13,034,903-triangle quarry uses the existing loader at
1920x1080, -14 stops, 120,000 lux, and 25/4-degree sun configurations;
both paths retain their regular shading. Quarry motion combines a 0.45 m
camera move and a 4-to-5.15-degree sun change over 24 frames.
Actual no-RT devices use VB + Virtual; RT uses traced primary visibility.
One sample per pixel, anisotropy clamp 1 and native-resolution TR1 remain
the renderer defaults. Small HDR mid-row profiles are kept in JSON alongside
full inspected PNGs. No new Unreal or reference campaign was run.

## Baseline GPU observations

The same frozen baseline executable completed the non-RT fixture in
`images/baseline-b/fixture-nonrt` and RT in `images/baseline-c/fixture-rt`.
These completed steps are reused; the guard aborted subsequent startup work
in those leases. `images/baseline-provenance.json` identifies both directories;
the binary identity and its source hashes are in `bin/baseline.json`.

Inspection confirms soft bands around the slats rather than clean shafts of
lit air. The straight-on camera mostly sees bright sky through the openings.
At the wall's 16 m camera distance, each 0.45 m opening projects to about
15.7 pixels: roughly one old froxel column. This makes it a useful angular
sampling witness, but the angled camera is also needed to inspect lit air
against a dark wall.

Changing only g from 0.6 to 0.8 in the baseline non-RT fixture raises the whole
mid-row mean exposed RGB from 0.11468 to 0.17615. The shadowed left segment
(pixels 80-239) stays 0.01173/0.01168. Turning history off at g=0.6 gives a
whole-row mean of 0.11571. These are diagnostic profiles, not shaft-quality
scores. They show that phase strength alone brightens the existing pattern;
history is not the sole explanation for missing static shafts.

After the camera stops, mean absolute exposed-RGB mid-row change between
frames 23 and 31 is 0.000838 non-RT and 0.000815 RT. This is a short held-camera
diagnostic, not a scored temporal comparison or a reference result.

## Rejected first candidate

`bin/candidate.exe` passed all 184 lib tests and completed the non-RT fixture
in lease `candidate-d-1` (32.56 s, no guard failures). It used a min/max bound
of just the two samples inside each current froxel. Its held-camera mid-row
change was **0.002544**, about 3.04x the old non-RT diagnostic. Inspected
images show patchy lighting at slat edges. This box was too tight to retain
accumulation and is rejected. The initial candidate images and source identity
remain in `images/candidate-d/` and `bin/candidate.json`.

The raw-neighborhood RGB-history revision (`resolve-a-1`, 184 lib tests and
the non-RT fixture passed in 29.04 s) gave 0.003081 in the same held-pair
diagnostic. Investigation found two confounds: the test quality override
cleared fog history but not TR1, and reprojection stored view-dependent phase
inside RGB. The final harness cuts TR1 when changing diagnostic quality. The
earlier held-pair ratios are **not isolated fog variance measurements**; their
configuration order and reconstruction history differ. They are retained as
rejected diagnostic observations, not a repeat-certified quality score.

## Retained implementation

- Eight-pixel columns: 240x135x64 at 1080p. Camera, raw injection and two
  resolved scattering volumes use 74,649,600 bytes, versus 14,622,720 bytes
  before. This also refines the
  shared aerial volume. Slice count and exponential distribution remain 64/5.
- Two opposite-half-cell samples per froxel, with per-cell decorrelation of
  the temporal sequence. Depth samples are uniform in physical distance
  within each slice. Both paths still test the centre sun; this is volume
  sampling, not a soft solar disk integral. Queries at 1080p increase from
  522,240 to 4,147,200 per steady frame. Frames without valid history spend
  four samples (8,294,400 queries), including moving-sun/caster resets.
- Injection writes an unfiltered raw volume. A separate resolve clamps
  reprojected visibility to the plain scalar min/max of the raw cell and
  its six face neighbors. It does not spatially average the source. A fully
  shadowed neighborhood cannot support stale direct light. Existing coverage
  and density rejection remain. Moving sunlight and casters still reset
  history, with the fresh-sample budget above.
- History stores density-weighted sun visibility and extinction, rather than
  phased RGB. Integration evaluates current sky light, albedo and HG phase
  for the current camera. World reprojection can otherwise import the old
  viewing angle's phase during camera translation, especially at g=0.8.
  This also removes per-sample phase and transmittance lookups from injection.
- Backdrop fog stops at the implicit planet ground. Its near root is computed
  from separate altitude and radius and rationalized, preserving metre-scale
  distances in f32 beside a 6360 km planet. The ignored `gpu_ground_limit`
  test evaluates the actual WGSL helper at four rays; it is run in the lease.
- Default height-fog anisotropy is 0.8. Density, range, albedo and height
  falloff are unchanged. The test-only quality accessor runs the 8/16-pixel,
  one/two-sample combinations without reloading geometry; it is absent from
  production builds and adds no public API.

Patent-policy design-around: no moments, variance estimates, variance clipping
(US 10,116,916), reservoir resampling, SVGF filter or world-space cache is
introduced. This uses the policy's permitted plain min/max bounds and existing
reprojection. This is an implementation record, not legal clearance. Material
programs, hit records and shading dispatch remain programmable and unchanged.

## Final leased verification

Lease `ship-a-2` ran 2026-11-08 23:32:39.603 to 23:34:45.190 UTC,
125.587 seconds, exit 0 and no guard failures. It followed the latest root
order and the literal `m7-mv1 FINAL release` after that order. No new lease
is needed. The earlier attempts/deferrals did not satisfy that gate. The
CPU-only schedule-gate test passes seven cases, including rejection of quoted,
old and attempt-release markers and of an unacknowledged new root order.

- Lib suite: **184 passed, 0 failed, 157 ignored**, within this unit's lease.
- Actual WGSL backdrop-distance regression: **1 passed**; distances were
  `[5.7587857, 0.99999994, 1e30, 1e30]` metres for down-10-degree,
  straight-down, up and horizontal rays from 1 m altitude.
- Four shaft runs (slats/windows, actual non-RT/RT) and four quarry runs
  (frozen baseline/retained, non-RT/RT): all eight focused tests passed.
  Readbacks assert finite, nonnegative complete images.
- `rustfmt --check`, Python syntax checks, schedule-gate cases and
  `git diff --check` pass. The source hashes in `bin/ship.json` match every
  built source/shader at final verification.
- Renderer processes retain **ten** Vulkan
  `VUID-StandaloneSpirv-None-10684` diagnostics each, the same count as the
  frozen baseline, starting during pipeline creation. The lib suite and
  isolated WGSL regression log none. This is not a clean-validation claim.

The final executable SHA-256 is
`069d6d7622faef7da1a4ee9c4e55098d7e417c17d01a347efa676d3a5b967494`.
`images/ship-a/build-identity.json` records its base, complete source hashes
and tracked patch; the new harness is committed with this report.
`baseline-identity.json`, `steps.json`, and `runs/ship-a-2.json` retain the
control identity, exact commands, per-step times and resource samples.

## Image inspection and limits

All paths below are under `D:/Coding/renderlab-work/vf2/`.

- `images/causes-chart.png`: source-derived phase, depth and sample-count
  plots; no quality or speed inference from this chart.
- `images/ship-a/windows-fixture-sheet.png` and native
  `fixture-{nonrt,rt}-windows/dense-g08.png`: distinct lit-air beams and a
  dark intervening wedge on both paths. Default density keeps a subtle
  version; density 0.01/m makes the beams clear. No density increase ships.
- `windows-ablation-sheet.png`: retained-code 16/8-pixel and one/two-sample
  pairs. Finer columns refine the edges; neither sample count eliminates
  their softness. These are not the old RGB-history algorithm.
- `windows-motion-sheet.png`, `windows-angle-motion-sheet.png`,
  `windows-sun-sheet.png`, `windows-angle-sun-sheet.png`: the beams follow
  the 0.9 m move/hold and changing sun without a conspicuous old beam hanging
  behind in the sampled frames. **Non-RT angle-sun-07 has extra light along
  the wall beside the apertures.** The exact source (resident shadow lookup,
  sun-basis update or reconstruction) is not isolated. Record this miss;
  do not call the moving-sun path leak-free or temporally equivalent to RT.
- `fixture-sheet.png`, `angle-motion-sheet.png`, `angle-sun-sheet.png`:
  slat silhouettes and floor shadows remain coherent; viewed obliquely,
  many repeated gaps still merge into a broad lit volume. Repeated floor
  stripes alone are not evidence of volumetric shafts.
- `quarry-comparison-sheet.png` and `quarry-motion-sheet.png`: baseline and
  retained stills at ordinary/low sun and dense fog, then sampled motion
  frames on both paths. The retained background correctly stops fog at
  planet ground instead of glowing through an underground medium. Rock
  silhouettes, surface detail and cast shadows remain visible. The quarry
  itself is not an aperture-shaft quality witness. Native
  `nonrt/low-dense.png`, `baseline-nonrt/low-dense.png`,
  `traced/default.png`, `baseline-traced/default.png` were also inspected.

Additional misses: no scored disocclusion/long camera walk or moving-caster
campaign, no participating-media reference and no matched Unreal evidence.
The raw neighborhoods and 64 depth slices still soften thin/distant beams;
there is no screen-resolution edge-preserving integration. Small startup
speckles and an elongated sun in retained quarry motion frames remain visible
in the complete image; the sun/reconstruction interaction is unisolated.
The held-camera mid-row mean absolute change is 0.002034/0.002003
non-RT/RT for slats, and 0.002740/0.002730 for windows. These include TR1,
GI and changing image content; they do not establish a stability win over
the earlier baseline diagnostic.

Non-RT remains dependent on resident Virtual top-layer coverage, with no fog
page-demand pass. Both paths use centre-sun visibility, not soft-disk rays.
The analytic height-fog source beyond 128 m remains unshadowed. Captured-map
and constant-clear misses still bypass fog. Only the first sun participates.

## One timing batch and development cost

Hardware: AMD Ryzen 7 7800X3D, NVIDIA RTX 4070 SUPER, driver 617.14,
Vulkan/wgpu, release lib-test executable with validation enabled. No game,
foreign GPU process or build was sampled during the final batch; rustc count
was zero and available RAM stayed 7,535-10,585 MiB. This is one short
sequential batch, not a quiet repeat-certified performance campaign.

Drained CPU wall time per frame, 32 frames after warmup (fixture 16,
quarry 32), milliseconds:

| Content/configuration | Non-RT | RT |
| --- | ---: | ---: |
| Slats 960x540, volume off | 3.793 | 2.094 |
| Slats, defaults | 3.954 | 1.697 |
| Slats, dense g=0.8 | 3.713 | 1.863 |
| Windows 960x540, defaults | 3.789 | 1.854 |
| Windows, dense g=0.8 | 3.800 | 1.728 |
| Quarry 1920x1080, defaults | 41.619 | 6.017 |
| Quarry, low sun/default density | 38.007 | 6.165 |
| Quarry, low sun/dense | 25.517 | 6.025 |

The sequential results are order/state sensitive: some fog-on runs are faster
than off, and non-RT quarry configurations differ substantially. Do not
subtract these to claim isolated fog cost or compare them to earlier leases
as a speedup. Stage attribution and repeat measurements remain open. The
new volume storage and query counts above are real costs, not hidden by a
quality reduction.

Implementation and verification were interleaved during the approximately
three-hour overnight box, including coordinator GPU holds and rejected
prototypes. Active coding time was not separately instrumented. Final warm
two-job release compilation/staging took 88.35 s; final leased lib/image/
timing verification took 125.59 s, followed by CPU contact sheets and visual
inspection. Shader-edit-to-image and viewer startup were not separately timed.
Cleanup miss: automatic approval review rejected deletion of the disposable
`D:/Coding/renderlab-work/vf2/target`, including a literal-path PowerShell
retry, with the reason "blocked by policy". The target and staged executables
remain; no alternate deletion mechanism was used. JSON/log identities and
inspected PNGs remain too. `verification-summary.json` collects final results.
No raw HDR dump larger than 5 MB was generated or retained.


## Integration with current main

The coordinator requested a follow-up merge after the original push. Main through
`8e3c06f` (including RQ1 and MV1) is merged at `537511a`. The resolution keeps
`atmosphere_volume_groups(injection, scene)` and API1b's generation key
(`tlas_builds + hardware_build`), rather than the in-place TLAS object's address.
Shadow-resource selection remains below the scatter check. VF2's raw/temporal
visibility volumes, current-view phase, finer froxels and integration bindings
are retained. Post and primary match main exactly, sharing
`primary_ray_gpu::FRAME_BYTES` (now 144 bytes with object motion).

Small additional edit outside atmosphere: the engine's headless adapter honors
`InstanceFlags::from_env_or_default()`. Previously a release example ignored
`WGPU_VALIDATION=1`; the follow-up fox run explicitly enables that flag. No
validation cost is used for a performance comparison.

The integration runner can stage the example with `example`, and `VF2_FOX`
adds a 48-frame RT flight with explicit volumetric fog to the same guarded batch.
Final integrated test/example rebuild and staging took 115.79/30.70 seconds,
respectively, using two jobs and this unit's target on D:.


Integration lease `integrated-a-1` ran **2026-11-09 00:00:16.821 to
00:02:37.068 UTC**, **140.25 seconds**, exit 0, no guard failures. The runner
reread and acknowledged the latest root order giving RD1 priority, waited for
`m7-rd1 rd1-a window released`, then acquired its single <=15-minute lease.
No game, foreign GPU process or compiler was sampled; available RAM was
7,682-10,616 MiB. Hardware remains the RTX 4070 SUPER / Ryzen 7800X3D above.

- **194 lib tests passed**, 0 failed, 162 ignored (3.23 s test time).
- WGSL ground-distance regression passed.
- Both actual device paths passed the window scene's stills, 0.9 m camera
  move/hold and moving-sun sequences. The native dense images and
  `windows-{fixture,motion,sun,angle-sun}-sheet.png` under
  `D:/Coding/renderlab-work/vf2/images/integrated-a/` were inspected.
  Beams remain distinct, with dark intervening air. Soft edges, startup
  speckling and transient non-RT moving-sun light leakage remain recorded gaps.
- **RT example with `--fox --volumetric-fog`, 48 frames at 960x540 passed**
  with `WGPU_VALIDATION=1`. `fox-rt/run.json` confirms HardwareRt/Traced,
  animation, fog g=0.8, post/motion blur, GI and native TR1. All seven saved
  frames were inspected in `fox-rt-sheet.png`, and `fox-rt/frame-047.png`
  at native size. The fox changes pose while the camera moves; shadows and
  surrounding objects remain coherent. No BLAS/TLAS validation fault or any
  new validation-error type appears.
- The two window processes and fox each log **ten existing
  `VUID-StandaloneSpirv-None-10684` diagnostics**; lib and isolated ground
  regression log zero. This is a successful TLAS regression check, not a
  clean-Vulkan-validation claim.

Final test binary SHA-256:
`553e1af0a6d71c3294fede794bb9c3d6a15c825ad1482220140f28bacc7125b6`.
Final example SHA-256:
`f9b66554c540fb4d7e93cc3aeaed4aa91a9f38126d9fa0d908e693fb67c17f83`.
Source hashes match the verified merged checkout. `integration-summary.json`,
`images/integrated-a/{build-identity,example-identity,steps}.json` and
`runs/integrated-a-1.json` retain identities, exact settings and resources.
Rustfmt (edition 2024), runner syntax and diff checks pass. No additional
Unreal, reference or quarry campaign was run for this integration request.

The coordinator deleted the old target before this follow-up. Automatic
approval review again rejected deletion of the newly rebuilt
`D:/Coding/renderlab-work/vf2/target` with "blocked by policy"; the target
remains for coordinator cleanup. No new raw HDR dump over 5 MB was generated.
