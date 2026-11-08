# RTDEF: the RT device default with TR1

RTDEF2 update (2026-11-08): the current traced-capable RT default is traced + Interactive + TR1 with the adaptive 256/64/32/8 budget; see [RTDEF2](rtdef2.md). Earlier decisions below are historical. Their fixed-eight-ray, TR1-off timings and memory are not measurements of the new profile.

2026-11-08, branch `t3/rtdef`, base main `976a952`, merged main `ae95d3a`
(TR1b `fa6f8c1`). That main merge changes no production Rust/WGSL relative to
the capture binary. Main's subsequent R1 checkpoint `ffdaee9` is also merged;
its final library integration check is separate from the measurements below.
This unit applies Islam's overnight promotion rule: visibly improve the quarry
and S2 without a visible regression in the inspected images. The retained HRT
and film comparisons are recorded; they are not tonight's promotion gate.

**Decision: retain raster + Exact.** Traced + Interactive + TR1 is clearly better
on the settled quarry and settled S2, but S2's +4-stop cold and moving penumbrae
are visibly grainier than the current default. This fails the requested
"not visibly worse anywhere" condition. The failure is against the current
Renderlab default, independently of HRT's non-blocking comparison.

No library/viewer default changes. Raster and Exact remain the RT default and
explicit settings. Historical RT measurements retain their non-default label:
calling them default measurements would be inaccurate, and their TR1-off
timings do not price reconstruction. No historical gate needs a new pin when
the settings it records have not changed.

![RTDEF measured grain and quiet cost](D:/Coding/renderlab-work/rtdef/comparison.png)

## Comparison

- Legacy RT default: raster primaries, 256-direction Exact, no reconstruction.
- Control: traced primaries, eight-direction Interactive, reconstruction off.
- Candidate: traced primaries, Interactive, native-resolution TR1b defaults.

The headless 1920×1080 tests use the RTX 4070 SUPER, Vulkan, the textured
13,034,903-triangle quarry at both registered poses, and S2's four camera/shadow
tracks at point and disk light radii. No new reference or Unreal campaign runs.
The existing `bench-contact-fold-score.py::radiance` supplies linear HDR and
Reinhard/sRGB display RMS at +0 and +4 stops. Quarry motion is inspected; only
its registered still poses have film scores in this unit. S2 uses every retained
per-frame film and its recorded HRT1 radiance scores.

Evidence is external/ignored at `D:/Coding/renderlab-work/rtdef/`; original
references on E: and D: remain read-only. The lease wrapper is
`scripts/rtdef.py`; `scripts/rtdef-score.py` performs CPU scoring only.

## Quarry

Full-frame display RMS in 8-bit codes, existing 256-primary × 256-shadow-ray
film, +0 / +4 stops. Lower is better. Both variants share the same source,
lighting, environment, 1x sampling, anisotropy 16 and full prepared asset.

| Pose / frame | Raster + Exact | Traced + Interactive + TR1 |
|---|---:|---:|
| 0, cold 0 | 2.5891 / 4.4037 | 2.5873 / 4.3983 |
| 0, settled 31 | 2.5891 / 4.4037 | 0.2715 / 0.3911 |
| 0, return 63 | 2.5891 / 4.4037 | 1.4422 / 2.0034 |
| 1, cold 0 | 2.6317 / 4.4784 | 2.6311 / 4.4745 |
| 1, settled 31 | 2.6317 / 4.4784 | 0.2686 / 0.3879 |
| 1, return 63 | 2.6317 / 4.4784 | 1.4435 / 2.0014 |

Each pose holds for 32 frames, translates 1 mm/frame for 16 frames, then
returns over 16 frames to its registered pose. The moving frames are inspected
without inventing a moving film reference. TR1's apparent softening matches
the integrated film; the old image's extra sharpness contains aliasing.
The inspected complete images and silhouette/ground/rock crops show no
additional quarry defect in this short, black-background loop. This is not
the visible-sky walk or a general motion-quality certificate.

Machine-local images (captures stay ignored):

![Pose 0 settled: film, raster Exact, TR1](D:/Coding/renderlab-work/rtdef/quarry-p0/film-still-crop.png)

![Pose 0 moving: raster Exact, Interactive, TR1](D:/Coding/renderlab-work/rtdef/quarry-p0/crop-f039.png)

![Pose 1 moving complete images](D:/Coding/renderlab-work/rtdef/quarry-p1/comparison-f047.png)

## S2: the promotion failure

The disk witness (`camera-0.25px-r0.04`), complete registered native 512×288
crop, display RMS codes at +4 stops:

| Frame | Raster + Exact | Interactive, TR1 off | Interactive + TR1 |
|---|---:|---:|---:|
| 0, cold | 12.6000 | 14.0832 | 14.0832 |
| 31, moving | 12.6361 | 14.8326 | 12.6370 |
| 95, hold | 12.6442 | 14.7215 | 1.4197 |

The moving crop's nearly tied whole-image RMS does not certify the final
image: TR1 improves geometric coverage while leaving more visible grain in
the soft-shadow boundary. It is visible in the paired images below. An
additional diagnostic uses the original all-256-primaries-on-centre-surface
population, floor placement 1, visibility in (.1,.9), and the same display
error summary; it does not replace the complete-crop metric:

| Case / frame | Pixels | Raster + Exact +4 RMS | TR1 +4 RMS |
|---|---:|---:|---:|
| .25 px camera, cold 0 | 3,629 | 6.4666 | 22.1735 |
| .25 px camera, moving 31 | 2,802 | 7.9109 | 12.0027 |
| 1 px camera, moving 31 | 2,781 | 7.9213 | 12.2559 |
| Moving blocker, 31 | 3,540 | 6.5464 | 9.2968 |
| Moving blocker, 63 | 3,462 | 6.6179 | 9.8755 |
| .25 px camera, held 95 | 2,801 | 8.0177 | 1.7901 |

TR1b's latest overnight settings intentionally start at the unjittered own
sample, so the cold frame reproduces Interactive's eight-ray grain. In
`Renderer::reconstruction_key`, geometry/binding generations and cumulative
model/instance uploads invalidate all history. S2's reveal plate changes
placement during its camera tracks; its shadow-step track moves the blocker
every frame. Thus a held view can integrate the grain, but moving scene
content still restarts history. This is a source-level explanation, not a
claim that every moving-pixel error has the same cause. Fixing transform-aware
history is a reconstruction unit, outside this default-selection change.

![S2 disk cold +4 stops: film, raster Exact, Interactive, TR1](D:/Coding/renderlab-work/rtdef/S2/camera-0.25px-r0.04/plus4-f000.png)

![S2 disk moving +4 stops](D:/Coding/renderlab-work/rtdef/S2/camera-0.25px-r0.04/plus4-f031.png)

![S2 disk settled +4 stops](D:/Coding/renderlab-work/rtdef/S2/camera-0.25px-r0.04/plus4-f095.png)

![S2 moving blocker +4 stops](D:/Coding/renderlab-work/rtdef/S2/shadow-step-r0.04/plus4-f031.png)

## All S2 phases against the retained HRT

All **24/24** phase pools improve on raster + Exact and retained HRT1 in each
of linear-HDR RMS, +0 display RMS and +4 display RMS. This is a real aggregate
improvement; it does not erase the inspected local regression above.
Each pool is `sqrt(mean(per-frame RMS²))` on the unchanged complete native
crop, over all 32 hold or 64 moving/return frames. The CSV retains all three
metrics and the un-reconstructed Interactive control. The JSON retains
all 1,280 frames × four configurations, without dropping cold/revealed frames.

| Case / phase | Raster Exact +4 | TR1 +4 | Retained HRT1 +4 |
|---|---:|---:|---:|
| camera .25 px, point / out | 16.5708 | 7.4707 | 14.2818 |
| camera .25 px, point / hold | 17.0835 | 4.4435 | 15.2845 |
| camera .25 px, point / return | 16.0697 | 6.2554 | 13.9654 |
| camera .25 px, disk / out | 12.4871 | 5.7917 | 13.5222 |
| camera .25 px, disk / hold | 12.6442 | 1.7586 | 13.6650 |
| camera .25 px, disk / return | 12.4061 | 4.3874 | 13.7420 |
| camera .5 px, point / out | 16.5158 | 7.5320 | 14.9105 |
| camera .5 px, point / hold | 16.7800 | 4.5224 | 15.2423 |
| camera .5 px, point / return | 16.0218 | 6.3019 | 14.4748 |
| camera .5 px, disk / out | 12.4656 | 5.8148 | 13.7454 |
| camera .5 px, disk / hold | 12.3855 | 1.8646 | 13.6486 |
| camera .5 px, disk / return | 12.3901 | 4.3376 | 13.8301 |
| camera 1 px, point / out | 16.3450 | 7.0183 | 14.6317 |
| camera 1 px, point / hold | 16.9374 | 2.7112 | 15.1824 |
| camera 1 px, point / return | 15.9176 | 5.5504 | 14.0665 |
| camera 1 px, disk / out | 12.5044 | 5.8253 | 13.6308 |
| camera 1 px, disk / hold | 12.7807 | 1.7494 | 13.8114 |
| camera 1 px, disk / return | 12.4363 | 4.3905 | 13.8032 |
| shadow step, point / out | 15.5889 | 7.2691 | 14.6491 |
| shadow step, point / hold | 15.5687 | 3.1494 | 14.4514 |
| shadow step, point / return | 15.5889 | 7.1812 | 14.5875 |
| shadow step, disk / out | 12.6393 | 6.5615 | 13.7179 |
| shadow step, disk / hold | 12.6415 | 2.7069 | 13.7691 |
| shadow step, disk / return | 12.6393 | 6.5191 | 13.8636 |

The per-frame HRT audit loses +4 RMS on **10/1,280 frames**: every case's cold
frame 0, .25 px camera/point frame 1 and 1 px camera/point frame 122
(return/reveal). Linear HDR and +0 RMS win on all 1,280 frames. Against
raster + Exact, TR1 loses on 70 linear-HDR frames and 37 frames at each display
exposure. The cold disk witness still loses +4 RMS to HRT1 (14.0832 versus
13.6655). The Interactive control reproduces the retained rows to within
0.000070 display codes RMS (linear-HDR RMS within 3.5e-9); no new Unreal run
or fitted reference was used. The old visibility/flicker metrics measure
the raw shadow sample, not TR1's reconstructed final radiance; they are not
relabelled as resolved-image passes. HRT's retained per-frame rows do not
provide a final-HDR flicker series. No new full-frame/motion film or
transparency acceptance is claimed. [m7-gaps.md](m7-gaps.md) records these
remaining RT shadow issues.

## Quiet timing

Ryzen 7 7800X3D / RTX 4070 SUPER 12 GB, NVIDIA 617.14, Vulkan, optimized
release binary `ed68f2a4b4ca61d22c5983963f9cccaead9fe35f2fac7477bc1f6ce6db15f106`,
1920×1080, full-detail textured quarry, the same direct-light/environment
subset as the images. GI is off; no probe reflections or other newly landing
features are priced. One process loads once and alternates eight retained
visits per configuration after the initial warm visit. Each visit holds both
poses for 32 frames; its last 16 frames per pose supply all 256 samples per
configuration. No readbacks or diagnostic shading run in this timing track.

| Configuration | GPU rendered span p50 / p95 / p99, ms |
|---|---:|
| Raster + Exact | 16.514 / 21.264 / 23.125 |
| Traced + Interactive, TR1 off | 1.041 / 1.662 / 1.816 |
| Traced + Interactive + TR1 | 1.549 / 2.260 / 2.466 |

TR1's resolve pass p50 / p95 / p99 is .467 / .739 / .916 ms. The rendered
span runs from the first rendered pass through tone mapping, excluding asset
loading/upload and presentation wait. This is a static direct-light subset,
not an engine-wide or motion-performance comparison. The faster configuration
did not meet the visual promotion condition; this table cannot establish the
combined quality/speed floor.

The shared-lock QUIET interval was **17:02:31.177–17:02:51.464 UTC**.
The preflight and all live guard samples contain no games, foreign renderers
or builds; the 10 Hz samples have zero rustc and at least 6,736 MiB available
host memory. All samples and tails are retained in `timing/tr1.json` and
`timing-summary.json`. No new Unreal timing was run.

## Verification and scope

- The release library suite ran under its own n8 lock: **151 passed, zero
  failed, 145 ignored**, 4.45 s in the test harness / 7.638 s guarded command.
  After merging R1/main `ffdaee9`, the final suite passed **152 tests, zero
  failed, 146 ignored**, 5.01 s / 8.588 s guarded. Its separate binary is
  `63d9d6a7d04df15faa0e315e2b77f2e6b87d01fafa2cd6490f45e18f4f9c26ac`;
  `final-integration.json` pins the merge and both library lease records.
- Quality capture used one successful 235.455 s guarded command. Both quarry
  poses' cold/held/moving images, film crops, S2 full images at ordinary
  exposure, point/disk +4 crops and moving-blocker crops were inspected.
  No viewer or desktop capture was used.
- Main `ae95d3a` was merged; production Rust/WGSL matched the capture binary.
  The only authored Rust changes add explicit historical raster/Exact
  variants to the existing capture harnesses. No renderer, shader, material
  ABI or sampling-quality setting changed. Non-RT's implementation is
  unchanged; no new non-RT reference campaign was needed for this rejected
  RT-only default selection.
- Formatting and diff whitespace checks pass. The quality harness disables
  its validation callback; its first lease's native child stdout was not
  retained. Captures, finite-HDR assertions and success records are retained;
  the final library/timing commands explicitly retain native child stdout.
  This unit does not claim a Vulkan-validation certificate.
- Implementation was measurement instrumentation, not a runtime optimization.
  Initial Rust harness edits and build preparation took approximately four
  wall-clock minutes, with scoring/report script work overlapping the leases.
  Initial compilation took 202 s; the post-R1 incremental compile took 131 s.
  CPU quarry/S2 scoring took 42.393 / 257.778 s;
  the diagnostic population scoring took 1.636 s. GPU queue waits, library
  checks and the 20.280 s timing lease are separate from implementation time.

Evidence root: `D:/Coding/renderlab-work/rtdef/`. Retained summaries include
`quarry-scores.json`, `s2-per-frame.json`, `s2-summary.{json,csv}`,
`s2-comparison.json`, `s2-grain-diagnostic.json`, `timing-summary.json`,
the manifests, per-track `tr1.json`, lease/resource logs and inspected PNGs.
Source/reference/capture identities and cleanup are recorded alongside them.

Retention cleanup removed **6,997 raw/uninspected captures / 14,139,314,859
bytes**, retaining **18 inspected PNGs** and the summaries. `cleanup.json`
pins every removed path, byte count and SHA-256; `reference-identities.json`
pins all 1,280 read-only S2 film archives. The owned Cargo target was cleaned
(1,066 files / 536.0 MiB) and the staged test binary removed after its hashes
were recorded. No owned GPU/test/build process or lease remains.
