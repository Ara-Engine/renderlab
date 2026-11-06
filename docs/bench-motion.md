# S2/S3 moving slivers and moving sun

Based on `db3bb2d`; branch `t3/s2-s3-competitor-benches`.
Generated geometry and public test code only. The private Unreal project and
captures remain outside Git at `E:/renderlab-work/motion-benches/`.

## Scope and registration

Production images are 1920 x 1080, zero shutter, fixed 60 Hz simulation. S2
has 32 triangles: S1's two-triangle floor, four double-sided wires with starting
widths 0.25/0.5/1/2 pixels, a diagonal 0.5 m bar, glossy/diffuse faceted patches,
a tilted shadow blocker and a moving foreground plate. Camera translation
tracks are 0.25/0.5/1 pixel per frame: 64 out, 32 hold, 64 return. The plate
reveals the background over frames 16-32 and returns during 112-128, so the final
scene equals the initial scene. A fourth track moves only the shadow blocker.
Each track runs with a point source and a 0.04-radian directional disk.

S3 uses 512 x 512 cells / 524,288 slab triangles plus 768 triangles in 64 pillars:
**525,056 triangles total**. Height is fixed-seed smooth Fourier noise with
1-5 mm ridges. Native meshes use the same finest triangles, Nanite disabled;
this is not a Nanite performance result. The 385-frame track is 32 static,
128 at 0.25 degrees/second, 32 hold, 128 at 1 degree/second, 32 hold, then a
5-degree jump and recovery ages 0-32. Initial azimuth is 35 degrees, elevation
15 degrees, radius 0.04 radians. No sun quantization or deliberately stale shadow.

The shared mesh/track JSON drives both engines, using S1's glTF basis and cm
transport. Camera matrices and the floor's projected displacement are exported
to `camera-matrices.json`; actual native camera and shader-light readbacks are
audited separately. UE uses the private `/Game/BenchMotionS2/Bench` and
`/Game/BenchMotionS3/Bench` maps in `M5Calibration.uproject`.

Black environment and visible black background match. The constant-environment
case is not covered. Roughness is 0.1 on the glossy patch and 0.6 elsewhere;
metallic zero, grey base colour 0.5 linear, double-sided opaque geometry, no
textures/GI/scene reflections/fog/blur. Renderlab's thin receiver declarations
and UE's native receiver policies remain distinct, as in S1. Only the standard
program is covered; the proposed toon-program extension is not certified.
The white sun is 1,000 lux; registered exposure is -8 stops in both engines.

## Reference and metrics

The S1 F0/R1 generator is reused: 256 stratified footprint primaries, each with
256 independently salted sun directions, full RGB accumulation and the same
centre-placement / 2%-depth conditioned visibility. These are **native-pixel
crop scores**, not lower-resolution production images: S2 512 x 288 at
`[704,480,1216,768]`, S3 384 x 216 at `[768,480,1152,696]`. Full production
frames are retained for inspection; these crops cannot certify a full-frame
film gate. Uncertainty comes from the existing odd/even split halves.

Each disk fixture checks its cropped GPU reference against the unchanged S1
full-dispatch generator by byte equality. The point-source reference collapses
256 identical source directions to one exact visibility query, retaining all
256 primary strata. Static references are reused only for bit-identical camera,
light and object transforms; saved hold-frame arrays established identity.

Population construction and thresholds are S1's unchanged metrics: partial RMS,
signed mean/p99/max, contact loss (`candidate-film > 0.5` on reference contact
pixels), umbra leaks (`>0.1`), solid flips (`>0.5`), the stricter solid `0.002`
gate, missing receivers, missed/false shadows and nine-pixel contour error.
Counts aggregated over a phase count **pixel-frames**; peak defects remain
per-frame maxima. Missing output is never inserted as zero error or lit visibility.
Final RGB includes linear HDR RMS and common Reinhard/sRGB display errors at
registered exposure and +4 stops, including the crop's background and silhouettes.

Sun-jump ghosting retains the age 0-32 curve on pixels whose reference visibility
changed by more than 0.05. Projection of residual error onto the old-minus-new
reference is a **diagnostic**, not the denoiser's actual history weight.
Diagnostic lag90 is the first age whose absolute projection stays <=0.1 through
all remaining measured recovery frames; null means it did not recover by age 32.
This definition was registered before S3 capture. Persistent receiver bias can
affect this diagnostic. Per-frame residual-error changes are also retained.
The final-HDR extension was registered before matched-material S3 capture: on
the same visibility-change mask, project RGB radiance error onto the previous
minus current film RGB. Its own lag90 uses the same sustained absolute <=0.1
criterion. RGB ghost RMS and projection are reported separately from mask
ghosting, so a prompt native shadow mask cannot conceal post-TSR colour trails.

Renderlab runs hard, Exact, Interactive and Temporal. At zero source radius all
four use the production hard path; Temporal does not integrate hard-source
footprints. The `hard` control always uses zero radius, including in disk-reference
cases; it is not a matched soft-source quality candidate. With the disk, actual
temporal reset flags and reset-frame equality
against Interactive are retained. UE uses native VSM, HRT 1-spp denoised and
HRT 256-spp raw diagnostic. The HRT diagnostic is never the film reference.
Both benches retain native-resolution TSR Epic with 200% history.
Every native mask uses S1's versioned last-writer decoder and
`R^2 * B^2` attenuation, with effective CVars and source radius/SPP/direction checks.

Native masks and final post-TSR pre-tonemap HDR are separate contiguous replays
inside the same session. Their stochastic realizations can differ. Final HDR
comes from the full-size Tonemap input, pre-exposure override 1, scaled by 2^-8;
native main-view images are retained separately from the common display transform.
Each capture replay is one multi-frame RDG dump, grouped by its recorded engine
frame scopes. Narrow native producer passes avoid copying the 512 MiB VSM atlas
on every frame. A reset-view-state prepass replay supplies receiver availability;
its covered/background mask must match every HRT256 ray pass's actual depth input.
Actual ray depth is used where available. Prepass depth **values** differ from
final base-pass depth in S2; the rejected bit-identity check is retained separately.
HRT1 also has
a parameter-only ray pass capture to check the shader's source radius and SPP.
That capture checks the deferred light's actual consumed mask: UE bypasses the
denoiser for the point-source case and consumes `RayTracingOcclusion` directly.
Native mask scores precede TSR and do not measure TSR reconstruction quality.
Final post-TSR HDR scores and full images are the native final-image evidence.
The RGB reference uses Renderlab's standard BSDF. Material parameters and fixed
exposure match, but independent Unreal/Renderlab BRDF parity is not certified;
native HDR error therefore includes possible shading-model disagreement.
The decoder compacts each completed replay before the next starts. Additional
stationary frames occur outside the tracks, during warmup and compaction.
Timing replays are separate and exclude all dump/readback/compaction work.
After repeated RAM-floor stops, S3's memoryless HRT256 mask export uses six
64/65-frame subsets from separate full input replays. Each replay still advances
all 385 requested poses; completed export groups survive a retry. The last group
contains the full pre-jump hold and recovery. No mask-history fidelity or
cross-segment flicker claim is made for this raw diagnostic. Its final post-TSR
HDR, and both production native modes, remain uninterrupted full-track replays.
Native capture replays warm for 64 frames. Renderlab's captured Temporal track
starts with a fresh history after a mode round trip; its initial static phase
therefore does not have the same history age as native TSR. Initial-static means
are not a matched convergence comparison. Moving phases and the explicit
post-jump curves remain separately reported; no full motion gate is certified.

## Results

Retained runs use the shared **metallic-zero** material. Image inspection caught
an early fixture default of metallic one; all affected Renderlab RGB and timing
results were discarded and regenerated. The fixture now deserializes base colour,
metallic and sidedness from the shared manifest. Visibility-only control hashes
remain separate; invalid material evidence is not included in the tables.

The following phase summaries pool partial visibility by pixel count and RGB
error by equal-size frame. Contact counts are pixel-frames; leaks/flips are the
worst individual frame. All seven modes have zero missing covered pixels in
these principal phases. GPU columns retain their distinct engine scopes.

**S2, 0.25 px/frame outward motion, disk radius 0.04:**

| Mode | Partial RMS | HDR RMS | Lost contact / contact | Max leaks | Max flips | GPU p50 / p95 / p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RL hard | 0.24181 | 0.01653 | 687 / 19,396 | 0 | 0 | 0.1992 / 0.2064 / 0.2107 |
| RL Exact | 0.01780 | 0.00966 | 0 / 19,396 | 0 | 0 | 3.9230 / 4.7505 / 5.3227 |
| RL Interactive | 0.07488 | 0.01045 | 0 / 19,396 | 0 | 0 | 0.2519 / 0.7223 / 1.3593 |
| RL Temporal | 0.07488 | 0.01045 | 0 / 19,396 | 0 | 0 | 0.3041 / 0.3215 / 0.3411 |
| UE VSM | 0.17524 | 0.01508 | 2,101 / 19,396 | 62 | 167 | 2.1322 / 2.6999 / 3.1167 |
| UE HRT1 | 0.14432 | 0.01485 | 2,498 / 19,396 | 126 | 195 | 2.3835 / 3.0052 / 3.3933 |
| UE HRT256 | 0.07959 | 0.01239 | 891 / 19,396 | 59 | 186 | 9.3689 / 11.0952 / 11.9234 |

Exact has the smallest mask and HDR errors in this moving crop. Interactive
retains visible grain; Temporal resets and produces the same moving result.
Native VSM has blockier shadow edges. HRT1 is noisy and HRT256 smooths the mask,
but neither matches the footprint film integral or eliminates all solid defects.
Native masks precede TSR; the HDR column is the final post-TSR image, including
coverage and possible BRDF disagreement. These are different measurements.

**S2, hard-source outward motion:** all four Renderlab labels invoke the same
production hard-source algorithm; none integrates the primary footprint here.

| Mode | Partial RMS | HDR RMS | Lost contact / contact | Max leaks | Max flips | GPU p50 / p95 / p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RL hard | 0.27330 | 0.01141 | 343 / 93,689 | 0 | 0 | 0.2044 / 0.2102 / 0.2130 |
| RL Exact | 0.27330 | 0.01141 | 343 / 93,689 | 0 | 0 | 0.2395 / 1.8352 / 5.7441 |
| RL Interactive | 0.27330 | 0.01141 | 343 / 93,689 | 0 | 0 | 0.2264 / 1.0969 / 3.2592 |
| RL Temporal | 0.27330 | 0.01141 | 343 / 93,689 | 0 | 0 | 0.2155 / 0.9586 / 1.9230 |
| UE VSM | 0.31736 | 0.01455 | 7,996 / 93,689 | 212 | 333 | 2.1317 / 2.5613 / 2.7572 |
| UE HRT1 | 0.37890 | 0.01701 | 8,223 / 93,689 | 123 | 301 | 2.0230 / 2.5637 / 2.7107 |
| UE HRT256 | 0.37890 | 0.01703 | 8,223 / 93,689 | 123 | 301 | 9.2401 / 10.7845 / 11.5919 |

**S2, disk-source motion across all tracks, partial RMS:**

| Track | RL hard | RL Exact | RL Interactive | RL Temporal | UE VSM | UE HRT1 | UE HRT256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| camera-0.25px | 0.24181 | 0.01780 | 0.07488 | 0.07488 | 0.17524 | 0.14432 | 0.07959 |
| camera-0.5px | 0.24171 | 0.01774 | 0.07508 | 0.07508 | 0.17555 | 0.14363 | 0.07958 |
| camera-1px | 0.24140 | 0.01757 | 0.07517 | 0.07517 | 0.17514 | 0.14447 | 0.07838 |
| shadow-step | 0.23607 | 0.01855 | 0.07424 | 0.07424 | 0.16969 | 0.13731 | 0.07413 |

All **516** S2 disk reset frames are byte-identical to Interactive; all **32**
first/return comparisons match. At the end of the 0.25 px/frame stationary hold,
Temporal partial RMS falls to **0.00925**, versus Interactive **0.07471**; final
HDR RMS is **0.000783** versus **0.011147**. That stationary improvement does not
apply during motion. Crops at reveal ages 0/1/3/7 (frames 16/17/19/23), hold end,
and return are retained, including all three camera speeds and the shadow step.

**S3, 1 degree/second sun rotation:**

| Mode | Partial RMS | HDR RMS | Lost contact / contact | Max leaks | Max flips | GPU p50 / p95 / p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RL hard | 0.23724 | 0.01890 | 33 / 196,292 | 0 | 0 | 0.1964 / 0.2185 / 0.2250 |
| RL Exact | 0.03131 | 0.01356 | 0 / 196,292 | 0 | 0 | 4.7495 / 5.7945 / 6.0517 |
| RL Interactive | 0.07790 | 0.01414 | 0 / 196,292 | 0 | 0 | 0.2980 / 0.3195 / 0.3348 |
| RL Temporal | 0.07790 | 0.01414 | 0 / 196,292 | 0 | 0 | 0.3666 / 0.4105 / 0.5704 |
| UE VSM | 0.18628 | 0.02001 | 12,289 / 196,292 | 106 | 192 | 2.4357 / 3.0292 / 3.2436 |
| UE HRT1 | 0.25387 | 0.02712 | 23,005 / 196,292 | 126 | 200 | 2.4063 / 2.9772 / 3.4004 |
| UE HRT256 | 0.07828 | 0.01760 | 9,535 / 196,292 | 106 | 196 | 9.2466 / 11.2316 / 11.9859 |

**S3 sun jump, diagnostic lag90 and final HDR ghost RMS:**

| Mode | Mask lag90 frames | HDR lag90 frames | HDR age 0 | HDR age 1 | HDR age 3 | HDR age 7 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RL hard | 0 | 0 | 0.039636 | 0.039636 | 0.039636 | 0.039636 |
| RL Exact | 0 | 0 | 0.016480 | 0.016480 | 0.016480 | 0.016480 |
| RL Interactive | 0 | 0 | 0.019821 | 0.019821 | 0.019821 | 0.019821 |
| RL Temporal | 0 | 0 | 0.019823 | 0.014801 | 0.008716 | 0.004496 |
| UE VSM | 0 | 1 | 0.040927 | 0.038278 | 0.036076 | 0.034707 |
| UE HRT1 | 0 | 3 | 0.055018 | 0.050616 | 0.049966 | 0.051173 |
| UE HRT256 | 0 | 3 | 0.036093 | 0.030466 | 0.026747 | 0.023278 |

Mask and HDR projection/ghost curves at **every age 0-32** remain in
`S3/jump-recovery.json`. A small projection is prompt response along the old-sun
direction; it does not imply low total error, no noise, or no persistent bias.
The native high-sample diagnostic is a candidate, never the reference.
Both native HRT modes reach the registered final-HDR projection threshold at
age 3. VSM reaches it at age 1, with an age-zero projection of 0.10009: that
one-frame distinction is very close to the 0.1 threshold. Total changed-pixel
error remains nonzero after the old-sun component settles.

S3 Temporal resets on **258** frames. Its strict RGB equality audit **fails on
257**: at most **10** crop pixels differ from Interactive, maximum RGB difference
**0.034058**, maximum whole-crop RGB RMS difference **0.00022420**. These are
retained renderer failures. No renderer algorithm was changed to hide them.

**VSM rotation cost:** steady-phase native means, first/last two boundary frames
excluded, eight replays. Page counters are the engine's separate asynchronous
buckets, not interchangeable measures of the same physical pages.

| Phase | Shadow depth ms | Projection ms | Whole GPU ms | Static invalidated | Dynamic invalidated | Pages to clear |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| static | 0.266 | 0.111 | 2.312 | 0.000 | 73.857 | 8.438 |
| rotate-0.25 | 0.498 | 0.113 | 2.521 | 0.000 | 0.817 | 79.252 |
| hold-slow | 0.264 | 0.114 | 2.323 | 0.000 | 77.013 | 8.125 |
| rotate-1 | 0.499 | 0.112 | 2.510 | 0.000 | 0.734 | 78.275 |
| hold-fast | 0.257 | 0.114 | 2.298 | 0.000 | 73.902 | 7.549 |
| jump-recovery | 0.259 | 0.115 | 2.292 | 0.000 | 75.522 | 6.069 |

Rotation raises shadow-depth work from about **0.266 ms to 0.498 ms**, while
pages to clear rise from **8.44 to 78-79** per frame. The separately named dynamic
invalidation bucket decreases; interpreting that alone as improved caching would
be wrong. No explicit rasterized-physical-page counter was exposed by this CSV;
pages-to-clear is reported under its actual name, not relabelled rendered pages.
All available VSM counters, including requested/cached/HZB-built pages, are saved.

**Verdict:** Exact wins these selected moving visibility crops; Interactive and
Temporal trade that accuracy for lower measured spans and retain moving noise.
The hard-source path retains footprint aliasing. None is certified as a complete
film-quality, 2x-faster Unreal replacement. The software/non-RT Renderlab path,
toon program, constant environment, full-frame per-time reference, matched initial
history age and matched optimized timing scopes are still missing acceptance
evidence. Native VSM is measured, but that alone cannot establish the required
Renderlab non-RT win.

**Recorded native session memory, MiB available:**

| Session | At launch | Runtime minimum (10 Hz) | Outcome |
| --- | ---: | ---: | ---: |
| S2 01 | 8379 | 2743.1 | setup correction |
| S2 02 | 8213 | 4329.8 | setup correction |
| S2 03 | 8927 | 5538.0 | setup correction |
| S2 04 | 9324 | 1960.2 | RAM abort |
| S2 05 | 9494 | 2037.6 | RAM abort |
| S2 06 | 9715 | 1968.7 | RAM abort |
| S2 07 | 8963 | 1988.1 | RAM abort |
| S2 08 | 7415 | 2015.5 | RAM abort |
| S2 09 | 11512 | 3264.0 | complete |
| S2 10 | 6635 | 4983.6 | cached completion; no new track |
| S3 01 | 10280 | 2041.6 | RAM abort |
| S3 02 | 7191 | 2043.5 | RAM abort |
| S3 03 | 7951 | 2045.8 | RAM abort |
| S3 04 | 8022 | 2041.5 | RAM abort |
| S3 05 | 8375 | 2043.0 | RAM abort |
| S3 06 | 8943 | 5413.9 | complete |

Every listed native launch cleared the requested **6,144 MiB** performance
counter preflight. The fast runtime monitor retained the **2,048 MiB** abort.
Foreign compiler activity was recorded; no foreign build was paused or killed.
Full per-session memory, process/context records and retry times remain on E:.
The UE reservation was released at **05:06:03 UTC**, after native completion;
the corrected Renderlab GPU batches ended at **05:29:55 UTC**.

**Validation and work cost:** `cargo test -p renderlab --lib -j 2` passed by
checked subprocess exit **0** before both retained GPU batches: **97 passed,
0 failed, 94 ignored**. The separately invoked ignored benchmark passed in
**857.35 s (S2)** and **321.79 s (S3)**. Guarded build/library/GPU work took
**897.78 s** and **353.59 s** respectively. These are verification durations,
not engine frame times. Python registration/metric checks pass; native camera
audits cover **7,680 + 2,310** capture frames. Per-case `scoring-paired.json`
records CPU scoring/PNG/compaction duration separately. Implementation time was
not independently stopwatch-instrumented; no estimate is presented as measured.
The source and executable SHA256 identities are retained in `renderlab-build.json`.
All **1,665** corrected reference visibility/geometry records match the archived
control field hashes. S2's eight sampled CPU visibility witnesses therefore remain
valid: point-source disagreement zero, disk maximum 0.0006331 within the registered
bound. This is sampled visibility validation, not independent physical RGB truth.

The private evidence root contains `comparison.html` (all tracks, per-frame
curves and crop selector), `s2-s3-evidence.html` (compact overview), and each
bench's `comparison.csv`, `per-frame.json`, `timing-frames.json`,
`jump-recovery.json`, `vsm-invalidation.json` and `images/`. S2 retains **8,960**
quality rows / **71,680** timing samples; S3 **2,695** / **21,560**. The native
and Renderlab audit files retain the reset/return witnesses and session origins.
After summarization, 22,830 raw-array/dump files (20.37 GB) and this worker's
0.96 GB Cargo target were deleted. Fixture inputs, summaries, logs and inspected
PNGs remain on E:. No private asset or generated measurement is committed.

## Timing and limits

Eight uninterrupted production replays per mode retain every frame. Renderlab
records direct first-rendered-pass through final-tone GPU spans and CPU service
including scene/light updates; upload/acquire/present and work outside that span
are not silently added. UE records its whole-GPU CSV field, CPU thread fields,
shadow-depth/projection fields and the native `VSM` CSV category. The camera
columns validate CSV alignment. Different scopes, development/test code,
editor-game UE and shared-desktop contention prevent a certified 2x speed claim.
No pass medians are added to invent a frame critical path.

Hardware: RTX 4070 SUPER 12 GB / NVIDIA 617.14, Ryzen 7 7800X3D. Renderlab uses
Vulkan/wgpu 30.0.1; Unreal is 5.7.4 CL51494982, D3D12 SM6. Both run on an RT-capable
device. Native VSM uses no RT shadows, but Renderlab's software execution is
**missing on this base**. Hardware results do not transfer to a feature-disabled
software device. No mode is promoted as film quality from these results.

The shared atomic lease is `D:/Coding/renderlab-work/gpu-slot.lock`, with windows
in `E:/renderlab-work/gpu-schedule.md`, at most 45 minutes each. All launches
require 4 GiB available RAM and abort below 2 GiB at 10 Hz, with game/foreign
renderer checks. After two Unreal RAM-floor stops, Islam requested a UE reservation,
6 GiB available through `\Memory\Available MBytes` before each native launch,
the same 2 GiB abort, and automatic retries every two minutes while unrelated
mypaper builds finish. `bench-motion-native-queue.py` implements that queue;
N4 builds/GPU jobs defer while the reservation is listed. Only owned process
trees are terminated. UE runs offscreen with
hidden startup; there is no input automation, activation or desktop/game capture.
Foreground endpoint reads are context, not proof of continuous focus stability.

Early S2 development required resumes: an initial full-image readback path was
stopped for excessive verification latency, a Python cache suffix bug was fixed,
and a RAM-floor stop interrupted CPU scoring alongside unrelated compiler activity.
The retained, corrected-material Renderlab measurements each completed in one
process per bench. Unreal required several sessions under the memory guard, so
the requested single native session per bench was not achieved. Completed native
stages survive retries; their session identities remain in the decoder metadata.
NullRHI imports encounter the already-recorded
installed engine post-save Slate assertion and resume from saved assets.
All partial attempts and retained origins remain identified in run records.
The first Unreal session was an unmeasured two-frame dump-layout probe. Its RAM
guard stopped the setup hold while unrelated memory use increased. It produced
no native comparison track or timing matrix.

## Reproduction

Use a fresh output root when reproducing captures: completed evidence roots
retain summaries and PNGs after raw-array cleanup, and their resume markers
deliberately skip already completed engine configurations.

```powershell
py -3 -B scripts/bench-motion-scene.py S2 --output E:/renderlab-work/motion-benches/S2/fixture
py -3 -B scripts/bench-motion-scene.py S3 --output E:/renderlab-work/motion-benches/S3/fixture
py -3 -B scripts/bench-motion-test.py
# Set RENDERLAB_MOTION_ROOT to the individual S2 or S3 root.
# Use bench-motion-run.py for every build/engine lease, --fast-check and -j 2.
# Renderlab: bench-motion-renderlab-run.py under --manifest <root>/manifest.json.
# UE: bench-motion-unreal-run.py prepare, then render (NullRHI vs offscreen).
# Native queue: bench-motion-native-queue.py; UE guard uses --ue-startup (6/2 GiB).
# The manifest names scene/output/root/score_script as in the saved run recipes.
py -3 -B scripts/bench-motion-score.py --root <root>
py -3 -B scripts/bench-motion-score.py --root <root> --comparison-only
py -3 -B scripts/bench-motion-audit.py --root <root>
py -3 -B scripts/bench-motion-oracle.py --root E:/renderlab-work/motion-benches/S2
py -3 -B scripts/bench-motion-inspect.py --root <root>
py -3 -B scripts/bench-motion-html.py --root E:/renderlab-work/motion-benches
```

The documented [VSM CSV category](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine)
and [fixed-step/FPS flags](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-engine-command-line-arguments-reference)
guide instrumentation; the live 5.7 readbacks, not the current documentation's
5.8 default, determine what was actually measured.
