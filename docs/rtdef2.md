# RTDEF2: traced RT defaults with a shadow ray budget

2026-11-08, branch `t3/rtdef2`, main `f159fcd` (GI/G2b and AT1 included).
This follows [RTDEF](rtdef.md) under Islam's approximately 90-minute unit:
make the RT default useful for games, including TR1 and R1, while matching
raster + Exact's visible shadow grain on the retained cold/moving witnesses.

## Engine policy

The selected profile is traced primaries, Interactive shadows and native TR1:

| State | Shadow rays per receiver |
|---|---:|
| New reconstruction target, explicit camera cut, first frame | 256 |
| Scene/material/lighting changes that restart existing history | 64 |
| Camera/light motion, or fewer than 16 unchanged frames | 32 |
| Stable hold after 16 unchanged frames | 8 |

`ReconstructionShadowBudget` and `Renderer::set_reconstruction_shadow_budget`
expose the policy. Counts are powers of two in 8..=256. The engine publishes
one private frame word; the same fused kernels accept every count without
pipeline compilation. Normalized visibility remains in 0..256. Material
inputs, hit records, custom programs and their lighting/scatter hooks are
unchanged. No luminance/variance guide, spatial-filter radius, reservoir or
new temporal acceptance rule is introduced.

The first 8/16/32/64/128/256 screen showed that an unjittered cold frame needs
256 to match Exact unambiguously. The +4-stop single-surface floor-penumbra
RMS was 22.173 / 10.574 / 7.722 / 6.779 / 6.552 / 6.460 codes, against
raster Exact's 6.467. With a moving blocker, 64 rays were already cleaner
(frame 31: 3.734 against 6.546). Distinguishing a fresh target/cut from a
scene-history reset keeps animation from forcing 256 rays every frame.

This is a sampling scheduler around Renderlab's existing estimator and TR1.
It does not derive a filter size from ray results/history counts, apply
variance clipping, or add a reuse/cache combination. The existing
[TR1 claim mapping](m7-p0-claims.md) remains the relevant design-around record.

## Default and explicit settings

A 1x hardware-shadow renderer whose caller enables `TRACED_FEATURES` and
`traced_limits` selects traced primaries and Interactive automatically.
The portable enum defaults remain Raster/Exact; insufficient capabilities
and 4x retain the previous raster path. Non-RT visibility/Virtual defaults
are unchanged.

The viewer now negotiates the RT features and limits on capable adapters for
regular 1x content and uses the hardware-shadow constructor. It exposes
`--primary raster|traced`, `--soft-shadows exact|interactive`, `--no-rt` and
`--rt-max-bytes` (8 GiB estimate policy, 0 disables it). Raster + Exact is
available with `--primary raster --soft-shadows exact`.

Historical raster/Exact and program-output gates pin their old settings in
the common fresh renderer, IQ1, R0a, programmable-program and toon fixtures.
Their TR1-off/eight-ray measurements are not relabelled as the new profile's
cost. The present profile enables TR1 and makes R1's traced path available;
it does not certify the outstanding TR1/R1 quality gaps.

## Verification workload

- Retained S2 films/HRT rows, native 1920×1080, registered 512×288 score crop,
  0.25/0.5/1 px camera and moving-blocker tracks, point/disk sources. The
  exploratory screen captures 16 cold/moving/hold checkpoints per case.
  Final confirmation uses the disk witnesses and complete ordinary-exposure
  images. No new reference or Unreal campaign runs.
- S2 explicitly disables GI and atmosphere to preserve its registered
  direct-light reference. The complete quarry comparison retains main's
  GI/atmosphere defaults, captured RLE2 environment and anisotropy 16, both
  registered poses, 32-frame hold and 32-frame outward/return camera loop.
- Non-RT quarry/S2G and the actual viewer provide integration smoke images.
  These are not a new non-RT or reflection-fidelity campaign.

The existing `bench-contact-fold-score.py::radiance` supplies HDR and +0/+4
display RMS. The grain diagnostic uses the original all-256-on-centre-surface
floor population with film visibility in (.1,.9); it supplements the whole
crop and image inspection. Point-source shadows use the hard path, so their
ray-budget telemetry is not a disk-ray count.

Evidence: `D:/Coding/renderlab-work/rtdef2/` (ignored/external). The runner is
`scripts/rtdef2.py`; scoring is `scripts/rtdef2-score.py`.

## Paired image result

RMS is against each checkpoint's existing film; lower is better. The grain
column is the registered floor-penumbra diagnostic at +4 stops. These rows
use the final 256/64/32/8 profile, with 16 checkpoints on each of three disk
tracks (48 pairs), not a new full-film campaign.

| S2 witness | Frame/state | Grain: Exact / RTDEF2 | Whole +4: Exact / RTDEF2 / HRT |
|---|---|---:|---:|
| Camera 0.25 px | 0, cold | 6.467 / 6.460 | 12.600 / 12.600 / 13.665 |
| Camera 0.25 px | 31, moving blocker/reset | 7.911 / 4.421 | 12.636 / 11.909 / 15.077 |
| Camera 0.25 px | 63, moving camera | 9.114 / 2.464 | 12.754 / 2.837 / 13.280 |
| Camera 0.25 px | 95, hold | 8.018 / 1.454 | 12.644 / 1.293 / 13.946 |
| Camera 1 px | 0, cold | 6.467 / 6.460 | 12.600 / 12.600 / 13.668 |
| Camera 1 px | 31, moving blocker/reset | 7.921 / 4.985 | 12.479 / 11.993 / 14.818 |
| Camera 1 px | 63, moving camera | 7.448 / 1.683 | 12.839 / 2.518 / 13.789 |
| Camera 1 px | 95, hold | 6.363 / 1.239 | 12.781 / 1.386 / 14.138 |
| Blocker step | 31, moving/reset | 6.546 / 3.734 | 12.646 / 11.769 / 13.652 |
| Blocker step | 63, moving/reset | 6.618 / 2.062 | 12.648 / 6.502 / 13.719 |
| Blocker step | 95, hold | 6.619 / 1.159 | 12.641 / 0.822 / 13.852 |

The diagnostic improves on 47/48 sampled pairs. The exception is camera 1 px,
frame 1: **4.857 versus Exact's 4.103** codes; whole +4 RMS still improves
11.833 versus 12.448. Inspection of that +4-stop pair did not show additional
speckle, but this is not a claim that every numerical grain row wins. Against
retained HRT, all 48 final disk checkpoints have lower whole-crop HDR, +0 and
+4 RMS. The broader point-source exploratory rows still contain cold geometric
aliasing and remain gaps; HRT is a recorded comparator, not a promotion gate.

Both final quarry poses were inspected cold, held and moving with GI and AT1
enabled. TR1 reduces silhouette/texture shimmer but also softens fine texture
in native crops. That existing TR1 tradeoff remains visible; this unit changes
shadow sampling, not the resolve filter. It does not establish film-exact texture
reconstruction or reflection fidelity. No new holes or extra shadow speckle were
seen in the retained complete images. Non-RT quarry/S2G and both actual viewer
modes completed and were inspected.

Selected inspected images (external, not redistributed scan content):

- [S2 cold +4](D:/Coding/renderlab-work/rtdef2/S2-final/camera-0.25px-r0.04/plus4-f000.png),
  [moving +4](D:/Coding/renderlab-work/rtdef2/S2-final/camera-0.25px-r0.04/plus4-f063.png),
  [early exception +4](D:/Coding/renderlab-work/rtdef2/S2-final/camera-1px-r0.04/plus4-f001.png),
  [blocker reset +4](D:/Coding/renderlab-work/rtdef2/S2-final/shadow-step-r0.04/plus4-f031.png),
  [held +4](D:/Coding/renderlab-work/rtdef2/S2-final/shadow-step-r0.04/plus4-f095.png).
- [Quarry pose 0 held](D:/Coding/renderlab-work/rtdef2/quarry-final-p0-at1/full-f031.png),
  [pose 1 moving](D:/Coding/renderlab-work/rtdef2/quarry-final-p1-at1/full-f039.png),
  [native moving crop](D:/Coding/renderlab-work/rtdef2/quarry-final-p0-at1/crop-f039.png),
  [native held crop](D:/Coding/renderlab-work/rtdef2/quarry-final-p0-at1/crop-f031.png),
  [pose 1 cold crop](D:/Coding/renderlab-work/rtdef2/quarry-final-p1-at1/crop-f000.png).
- [Complete S2 cold](D:/Coding/renderlab-work/rtdef2/S2-final/camera-0.25px-r0.04/full-f000.png),
  [complete S2 moving](D:/Coding/renderlab-work/rtdef2/S2-final/camera-0.25px-r0.04/full-f031.png),
  [complete S2 hold](D:/Coding/renderlab-work/rtdef2/S2-final/shadow-step-r0.04/full-f095.png).
- [Non-RT quarry](D:/Coding/renderlab-work/rtdef2/nonrt-quarry/f031.png),
  [non-RT S2G](D:/Coding/renderlab-work/rtdef2/nonrt-s2g/f031.png),
  [viewer default](D:/Coding/renderlab-work/rtdef2/viewer/default.png),
  [viewer raster/Exact](D:/Coding/renderlab-work/rtdef2/viewer/raster.png).

## Quiet quarry cost

Windows, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, optimized release,
native 1920×1080, anisotropy 16, full-detail quarry, captured environment and
main f159fcd's GI/AT1 defaults. GPU timestamp span covers the render through
tone mapping; this is not presentation latency or a CPU frame-time claim.
Each variant has eight alternating measured 128-frame visits, after an initial
visit. Both poses have a 32-frame hold and a 32-frame out/return camera loop.
There are no image readbacks in these timing visits.

| State | Exact p50 / p95 ms | RTDEF2 p50 / p95 ms | Samples per mode |
|---|---:|---:|---:|
| Held, eight rays | 17.038 / 23.800 | 3.710 / 4.991 | 256 |
| Moving camera, 32 rays | 17.055 / 23.978 | 4.655 / 6.133 | 512 |
| All measured frames | 16.972 / 23.967 | 4.611 / 6.106 | 1,024 |
| Cold TR1 target, initialized GI cache | 16.585 / 24.226 | 14.850 / 15.633 | 8 |

Cold rows are each visit's first frame, not a startup average. The new path's
first measured visit began at **89.526 ms** while GI initialized. An earlier
functional first GI visit was 120.9 ms. No paired first-initialization latency
claim is made: both modes share one renderer/cache in this batch. A 256-ray
cold-history frame is only modestly cheaper than Exact; the large saving is
the normal held/moving workload (4.59× / 3.66× on these medians). The camera
loop does not measure scene-transform restart cost separately at 64 rays.

Lease `timing-1794164134-3`, 18:55:34–18:56:24 UTC, exit 0, failures empty.
Preflight and all eight resource samples contain no games, foreign GPU jobs,
compilers or helpers; minimum available RAM was 8,899 MiB. The foreground
window handle/PID was unchanged. An earlier attempted quiet admission was
rejected for compiler contention before rendering and is excluded. Baseline
visit medians range 16.258–23.038 ms, so the p95 and full samples are retained;
no extra repeats were needed to distinguish this gap.

![Paired grain and timing](D:/Coding/renderlab-work/rtdef2/comparison.png)

## Validation and decision

Promote the adaptive profile under the overnight rule: the inspected cold and
moving disk witnesses show no extra speckle, the held/moving quarry is much
cheaper, and it enables the already-selected TR1 and traced reflection path.
This is a scoped default decision, not a claim that every image metric wins
or that cold startup, TR1 texture detail, R1 or Unreal parity is solved.

The final merged-main integration lease passed **158 lib tests, zero failures**
(147 ignored), both AT1 quarry tracks, non-RT quarry/S2G and two actual
24-frame background viewer runs (automatic RT and explicit raster/Exact).
The final quiet timing test also passed. Formatting, Python syntax and diff
whitespace checks pass. All test binaries and renderer/viewer processes ran
under this unit's n8 lease with hold/QUIET checks. No desktop/game capture or
input automation was used.

The native Vulkan output still contains the pre-existing
`VUID-StandaloneSpirv-None-10684`; callbacks report no errors, but the layer is
not active in these release runs. This is not an allowance-free validation
certificate. The S2 image batch predates the telemetry-getter correction:
its `shadow_rays` JSON fields are null; the shader used the manifest's budget.
The final integration/timing captures contain the corrected per-frame counts.

Implementation, integration fixes, builds and the functional screens ran from
approximately 17:21 to 18:43 UTC (82 minutes). GPU queueing and the quiet
retry extended verification to 18:56 UTC; documentation/retention followed.
The final incremental library compile was 206 s; the viewer build logs retain
the overlapping 210 s compile and 37.54 s warm completion. Final S2 CPU scoring
took 13.38 s; the final integration and quiet leases took 125.53 / 50.49 s.
This records verification latency separately from the render cost.

Evidence summaries: `S2-final/scores.json`, exploratory `S2*/scores.json`,
`quarry-final-p*-at1/tr1.json`, `timing/tr1.json`,
`timing-summary.{json,csv}`, `gpu-windows.json`, `runs/*`, manifests and
`build-identities.json`. The latter pins the actual test and viewer executables;
the original RTDEF reference identity manifest remains read-only and reusable.

Retention removed 1,475 raw/uninspected files (3,904,222,176 bytes), preserving
23 inspected PNGs and JSON/CSV/log summaries. `cleanup.json` records paths,
sizes and SHA-256 for removed captures and retained images. The owned Cargo
target was cleaned (1,190 files, 628.1 MiB), and staged executables were removed
after hashing. No owned renderer/test/build process or GPU lease remains.

## Integration corrections and limits

The new private `PrimaryFrame` tail is present in both traced and no-RT shader
headers and their binding sizes. An initial CPU shader test caught the second
layout before the pilot; the corrected library suite passed.

The pilot exposed main R1's loop-local opaque `ray_query` null initializer
(`VUID-VkShaderModuleCreateInfo-pCode-08737`, Naga 30). Moving its declaration
to function scope preserves per-ray initialization and removes that invalid
SPIR-V pattern; the reflection algorithm and shader interface are unchanged.

Main GI's first traced quarry visit has a large initialization cost (the
functional screen observed 120.9 ms at pose 0). This is separate from a warm
reconstruction reset and the steady frame timings. It is not hidden by the
policy's later 8-ray hold. AT1's volume setup also accessed a TLAS for unshadowed raster frames even when volumetric fog was off; lookup now occurs only for volume injection, and enabled RT volumes prepare their scene explicitly. This preserves the raster override.

Global transform/material edits still restart all
TR1 history; they now use the measured 64-ray restart budget. Engines must
signal camera cuts with `cut_reconstruction`. TR1's sky/silhouette, revealed
pixel and specular-motion gaps remain in [m7-gaps.md](m7-gaps.md).
