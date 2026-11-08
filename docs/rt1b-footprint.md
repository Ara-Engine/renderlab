# RT1b: footprint-distributed shadow receivers

2026-11-07 UTC. Branch `t3/rt1b-footprint`, from main `554a662`.

**Later ruling:** [RT1c](rt1c-rt-default.md) uses HRT as the RT shadow floor; the
VSM gap is no longer a blocker. This document preserves the earlier experiment's
scores and screening decision. RT1c's current-source S2 final-image witness is
the reason defaults remain unchanged after the ruling.

**The first footprint implementations fail the static quarry gate.** They reduce
partial-shadow RMS while introducing false shadows on previously lit receivers.
The bounded adjacency walk also costs much more than the previous fast setup.
No production rendering code or default has changed. The experiments are isolated
in an ignored GPU test and its shader helper; the CPU replay is reusable.

The RT-device default remains raster primaries plus 256-direction Exact. Traced
primaries plus Interactive are explicit settings. Historical RT speed claims are
**measured on the historical eight-ray setup with TR1 off (traced + Interactive)**, including the
0.276 ms shadow increment and 2.3x comparison with Unreal HRT. The diagnostic
timings below do not replace that optimized measurement.

## Receivers, origins and sampling

The first batch keeps the eight production R2 disk directions and distributes
their origins over a 4x2 pixel grid, with a deterministic per-pixel Cranley shift.
Multiplication by five permutes the pairing of origin and disk direction. The
256-direction control uses the same construction on a 16x16 origin grid.
Analytic ray-plane and barycentric differentials come from the centre hit's
triangle and the production camera-ray derivatives. The projected samples were
checked to remain inside their own pixel.

Three receiver constructions were tested:

1. **Clipped:** move on the centre triangle, scaling a displacement to the first
   barycentric edge when necessary. This crowds samples onto triangle boundaries
   and does not integrate the entire visible pixel footprint.
2. **Plane:** extend the centre triangle's plane outside its actual triangle.
   This is inexpensive but frequently puts the origin underneath another facet.
3. **Surface:** walk the existing edge adjacency, intersecting each facet with
   the displaced camera ray. Stop after 16 crossings. Open/unknown topology or a
   bounded-walk miss retains the centre receiver; it never becomes a clear ray.
   Recompute the accepted facet's geometric normal and precision scale.

Surface additionally tests a rotated 2-radial x 4-angular disk stratification and
an eight-radial-strata rotated sunflower. All patterns are deterministic per
pixel and unchanged from frame to frame. No history, denoiser, spatial filtering,
threshold adjustment or BRDF-specific visibility rule is introduced.

The centre remains the programmable shading hit. The experiments change only the
visibility estimate supplied to that program. They add no receiver, hit-record,
material or bind-group ABI, persistent buffer, or image target. They do not
integrate the program's radiance over different material samples, and therefore
are not a complete film-radiance reconstruction. M7, cook, shader-reload and
pipeline-cache implementation files are untouched.

## Replay before the GPU batch

`scripts/rt1b-replay.py` reconstructs the current viewed-side receiver from saved
GPU hit barycentrics and the source asset. F0 contributes only its unchanged
camera metadata, never its obsolete 1 mm origins. Local CPU triangle candidates
are a screening aid, not a global empty-space certificate. CPU intersections and
transforms are not a bit-exact hardware-traversal replay.

The replay includes all 53 current baseline false shadows, the three old-origin
witnesses, and the two historical estimator witnesses: 58 pixels in total. Its
eight-direction centre replay matches all 58 captured production counts.

| CPU witness screen, 58 pixels | Outputs <= .9 |
|---|---:|
| Current centre / original eight directions | 53 |
| Clipped / original directions | 43 |
| Extended plane / original directions | 58 |
| Extended plane / stratified directions | 58 |
| Clipped / stratified directions | 36 |
| Surface walk / original directions | 25 |
| Surface walk / stratified directions | 23 |
| Surface walk / sunflower directions | 23 |

These counts select hypotheses; they are not full-image scores. In particular,
the witness list excludes new failures created elsewhere. The full-image GPU
screen below is necessary even when a selected witness improves.

## Current receiver policy at the old witnesses

The current GPU world/normal attachments explicitly confirm the viewed-side rule,
`max(10 um, 16 ulp)` epsilon, and **thin=false** at all three original CPU probes.
Their epsilon and t-min are float32 `9.999999747378752e-6` (10 um), not 1 mm.
Viewed normals face the captured eye. `receiver-policy.json` stores the actual
attachments and an explicitly labelled CPU FMA replay of the displaced origin.

| Pose / pixel | Current film RF | Current baseline | Clipped 8 | Surface 8 | Surface 256 |
|---|---:|---:|---:|---:|---:|
| 0 / (1002,491) | 1 | 1 | .5 | .5 | .511719 |
| 0 / (910,635) | 1 | 1 | .875 | .625 | .613281 |
| 0 / (1212,653) | 1 | 1 | .625 | .625 | .5625 |
| 0 / (974,640), old estimator witness | .994665 | 1 | .5 | .5 | .492188 |
| 1 / (1169,384), old estimator witness | .970230 | 1 | .75 | 1 | .972656 |

All five are already clear in the current baseline. The last pixel is no longer
in the adopted film-lit population (RF >= .99). The two historical estimator
witnesses therefore cannot be treated as surviving current-prefix failures.
Stratification was nevertheless tested on the newly enumerated current failures.

## Complete static quarry results

All values are pose 0 / pose 1, against the adopted viewed-side R1 film reference.
The reference files and the original gate populations are unchanged. Lower is
better for every error/defect row. Native availability is reported, not filled
with zero error. The previous fast setup is captured again in the same process.

| Measure | Previous fast setup | Clipped 8 | Extended plane 8 | Surface 8 |
|---|---:|---:|---:|---:|
| Partial RMS | .207157 / .208537 | .175254 / .175360 | .322768 / .323070 | .125580 / .124398 |
| False shadows | 27 / 26 | 614 / 616 | 225,504 / 224,929 | 1,158 / 1,193 |
| New false shadows versus baseline | 0 / 0 | 598 / 594 | 225,477 / 224,903 | 1,150 / 1,181 |
| Missed film shadows | 9,245 / 9,166 | 7,433 / 7,374 | 2,343 / 2,385 | 3,044 / 2,994 |
| Solid flips | 0 / 1 | 11 / 16 | 54,135 / 53,998 | 28 / 31 |
| Umbra leaks | 0 / 0 | 1 / 0 | 214 / 235 | 1 / 1 |
| Lit darkened | 0 / 2 | 470 / 445 | 219,582 / 219,033 | 848 / 855 |
| Lost umbra edges | 0 / 0 | 0 / 0 | 1 / 0 | 0 / 0 |
| Missing covered receivers | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

| Measure | Surface stratified 8 | Surface sunflower 8 | Surface 256 | Unreal VSM | Unreal HRT1 |
|---|---:|---:|---:|---:|---:|
| Partial RMS | .122577 / .121869 | .124235 / .123827 | .085024 / .084813 | .398791 / .399889 | .370873 / .371257 |
| False shadows | 1,170 / 1,167 | 1,155 / 1,177 | 721 / 719 | **11 / 9** | 743 / 758 |
| New false shadows versus baseline | 1,161 / 1,157 | 1,147 / 1,167 | 714 / 710 | n/a | n/a |
| Missed film shadows | 2,864 / 2,802 | 2,959 / 2,971 | 913 / 899 | 27,071 / 27,070 | 23,931 / 23,789 |
| Solid flips | 28 / 32 | 28 / 32 | 26 / 21 | 4,129 / 4,118 | 3,306 / 3,215 |
| Umbra leaks | 1 / 1 | 1 / 2 | 1 / 0 | 5,537 / 5,524 | 2,718 / 2,616 |
| Lit darkened | 845 / 857 | 848 / 863 | 645 / 634 | **5 / 4** | 724 / 736 |
| Lost umbra edges | 0 / 0 | 0 / 0 | 0 / 0 | 3,825 / 3,810 | 2,565 / 2,461 |
| Missing covered receivers | 0 / 0 | 0 / 0 | 0 / 0 | 20 / 26 | 20 / 26 |

Renderlab has 49,928 / 49,889 available partial pixels; each native comparator has
49,922 / 49,881. False means RF >= .99 and output <= .9; missed means RF <= .9
and output >= .99, as in the existing replay. Solid flips, leaks, lit-darkened and
lost-edge definitions remain those in the gate, not these relaxed thresholds.

### Complete radiance and static repeatability

| Candidate | Whole HDR RMS versus film | Display RMS, 8-bit codes |
|---|---:|---:|
| Previous fast setup | .007503586 / .007558837 | 2.581752 / 2.625484 |
| Clipped | .007502957 / .007558548 | 2.581807 / 2.625624 |
| Extended plane | .007485711 / .007543289 | 2.594121 / 2.638495 |
| Surface | .007501492 / .007557107 | 2.581680 / 2.625431 |
| Surface stratified | .007501447 / .007557079 | 2.581680 / 2.625401 |
| Surface sunflower | .007501518 / .007557014 | 2.581702 / 2.625413 |
| Surface 256 | .007501332 / .007557096 | 2.581670 / 2.625403 |

Display conversion is Reinhard followed by sRGB. Whole-image radiance averages
do not override the false-shadow and lit-darkened failures. Verified visually:
saved complete baseline and stratified HDR images, full visibility-error images,
clipped new-failure crops, and pose-1 Surface-256 failure crops in
`E:/renderlab-work/rt1b/gpu/images/`. The crops show darkening of film-lit regions.

Eight repeats of every variant at each pose have identical complete primary-hit,
visibility-count and HDR SHA-256 hashes. Primary hit records also match the
explicitly pinned production Exact pass. This establishes sampled static
repeatability, not motion quality or a universal arithmetic-invariance guarantee.

### Diagnostic cost, not a release performance claim

RTX 4070 SUPER, NVIDIA 617.14, Vulkan/wgpu 30, 1920x1080, 1x, 16x anisotropy,
original fully resident quarry: 12 draws, 13,034,903 triangles. Development/test
profile, validation active. Eight paired samples after two warm pairs, reversed
variant order on alternating pairs, no readbacks inside the timing block.

| Candidate | Fused primary + shadows + repair, ms | Render span, ms |
|---|---:|---:|
| Previous fast setup | .504832 / .501248 | .881152 / .885760 |
| Clipped | .498688 / .497664 | .878080 / .888320 |
| Extended plane | .483840 / .484352 | .865792 / .866304 |
| Surface | 2.362880 / 2.360832 | 2.749952 / 2.744320 |
| Surface stratified | 2.353152 / 2.543616 | 2.724352 / 3.026944 |
| Surface sunflower | 2.366976 / 2.883584 | 3.074048 / 3.265024 |
| Surface 256 | 84.944384 / 82.990592 | 85.361152 / 83.642880 |

These are medians of individual timestamp intervals, not additive components.
Foreign compilation peaked at 14 rustc processes; timing was not quiet. The
historical .276 ms is an isolated optimized shadow increment, not the fused
interval above. The 256 control serializes its samples in the experimental fused
one-lane-per-pixel path. It is **not** the shipping Exact eight-lane schedule;
its timing cannot be used to claim a slowdown of production Exact.

## Why connected surface origins still fail

Increasing the sample count does not fix a wrong receiver distribution. Of the
721 / 719 Surface-256 false pixels, 622 / 605 have fewer than 256 accepted film
primary samples. Conditional footprint boundaries matter, but the remaining
99 / 114 have all 256 accepted samples, so conditioning alone is insufficient.

Four current-policy witnesses reproduce the GPU Surface-256 visibility exactly
in the CPU replay. Replacing the walk/fallback receivers with local closest-camera
hits, retaining the original same-placement/two-percent-depth condition, gives:

| Pose / pixel | Film RF | Surface 256, GPU and CPU | Closest-camera CPU | Accepted origins / 256 | Different facets among resolved walk samples |
|---|---:|---:|---:|---:|---:|
| 0 / (1197,741) | .990448 | .324219 | .984375 | 256 | 2 |
| 0 / (1388,528) | 1 | .566406 | 1 | 245 | 111 |
| 1 / (1022,470) | 1 | .335938 | .993506 | 154 | 64 |
| 1 / (1561,638) | .992188 | .457031 | .988281 | 256 | 68 |

The bounded walk falls back on 168, 32, 108 and 70 origins respectively. On some
resolved samples it reaches a different, hidden facet: maximum point gaps from
the closest camera intersection are 2.448, 1.659, 7.224 and 4.814 mm. A separate
whole-source closest intersection agrees with the walk for the first moved
sample at each witness; that does not certify the other samples. The per-origin
replay is the evidence for the differences above. Missing local primary hits
remain unknown, not lit. Local geometry is not a global visibility certificate.

This supports a concrete failure mechanism: extrapolation, edge crowding,
topology misses/step limits and hidden facets do not reproduce the camera-visible
footprint. Matching eight or 256 GPU counts in the CPU replay argues against a
fused-versus-separate/FMA explanation for these four reproduced failures. It does
not prove an exhaustive cause for every pixel.

## Closest-primary follow-up

After the static rejection, a further CPU screen replaces the surface walk with
actual nearest camera intersections. On the 58 original witnesses, original,
stratified and sunflower directions leave 3 / 1 / 4 outputs <= .9. Retaining
centre receivers for unknown/rejected local samples instead leaves 7 / 5 / 8.
On 48 evenly selected new Surface-stratified failure pixels (24 per pose), the
same closest-camera estimates leave 3 / 3 / 2; all 48 baseline counts replay
exactly. These are selected-pixel diagnostics, not an all-image pass.

The follow-up GPU control traces the actual subpixel primary, applies the current
primary repair and receiver policy, and conditions visibility on the same
placement and depth as RF. Unknown/rejected samples do not contribute a clear
shadow ray; if none are accepted the centre estimate is retained. It uses the
same joint origin/direction patterns, but adds camera queries, so the unchanged
shadow-ray budget does not imply unchanged cost.

| Measure, poses 0 / 1 | Closest R2 8 | Closest stratified 8 | Closest sunflower 8 | Closest 256 control |
|---|---:|---:|---:|---:|
| Partial RMS | .099853 / .098769 | .096282 / .095176 | .098432 / .097656 | .013267 / .013315 |
| False shadows | **231 / 221** | **241 / 193** | **216 / 202** | **0 / 0** |
| New false shadows versus baseline | 231 / 218 | 241 / 192 | 215 / 199 | 0 / 0 |
| Missed film shadows | 1,842 / 1,869 | 1,690 / 1,661 | 1,779 / 1,839 | 0 / 0 |
| Solid flips | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| Umbra leaks | 0 / 1 | 0 / 1 | 0 / 1 | 0 / 0 |
| Lit darkened | **17 / 9** | **14 / 10** | **11 / 9** | **0 / 0** |
| Lost umbra edges | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| Missing covered receivers | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| Whole HDR RMS | .007502007 / .007557915 | .007501980 / .007557848 | .007502046 / .007557855 | .007502212 / .007557799 |
| Display RMS, 8-bit codes | 2.581784 / 2.625502 | 2.581800 / 2.625472 | 2.581790 / 2.625511 | 2.581777 / 2.625561 |
| Fused primary + shadows + repair, ms | 1.984512 / 1.965568 | 1.980416 / 1.962496 | 1.985024 / 1.960960 | 55.428096 / 55.963136 |
| Render span, ms | 2.372608 / 2.358272 | 2.369536 / 2.350080 | 2.373632 / 2.352640 | 55.823360 / 56.351744 |

The recaptured baseline remains exactly 27 / 26 false, .207157 / .208537 partial
RMS, with zero differing count pixels between batches. Its paired fused interval
is .516608 / .513536 ms and span .890368 / .891904 ms. The same development-profile
and timing qualifications apply; foreign compilation peaked at nine rustc
processes. The closest-256 control is serialized, not the shipping Exact schedule.

Actual footprint receivers plus 256 samples clear all recorded static shadow
defect counts and greatly reduce partial RMS. Eight joint samples introduce
finite-sample/stratification error at many new almost-lit pixels: one blocked ray
out of eight already means .875, crossing the false-shadow threshold. Changing
the tested direction pattern does not approach VSM's **11 / 9** false or **5 / 4**
lit-darkened counts. The complete-image failure survives after correcting receiver
selection; it cannot be diagnosed solely as the two frozen F0 prefix witnesses.
This does not prove that every possible eight-ray estimator must fail.

The CPU local closest-hit replay predicts the quantized GPU counts at 105/106
R2, 104/106 stratified and 106/106 sunflower witness/pattern combinations. Its
exceptions are (1239,538) pose 1 for R2 and stratified, and (1096,505) pose 0 for
stratified: CPU 256 versus GPU 224. Local geometry and floating transforms remain
a stated limit; this is not a hardware-bit proof at those pixels. Full GPU scores,
not the selected CPU screen, decide rejection. All eight complete hit/count/HDR
repeat hashes match for the follow-up too. Verified visually: its pose-0
stratified false-shadow crops and the pose-1 closest-256 full HDR image in
`E:/renderlab-work/rt1b/camera-gpu/images/`.

## Default decision and remaining gates

**Do not promote traced + Interactive. RT1b is not a passing default candidate.**
Correcting the visible footprint works in the 256-sample static control, but the
tested eight-sample estimators fail false shadows and lit darkening. The added
camera queries also cost far above the previous setup in the paired screen.
The inexpensive clipped/plane alternatives already fail static image quality.
No threshold was widened and no geometry or receiver was excluded to obtain a pass.

| Required measure | RT1b status |
|---|---|
| Quarry partial RMS / false / flips / leaks / lit darkened / edges | All tabulated above; no eight-sample candidate meets every Unreal measure |
| Complete static image | HDR/error images and crops inspected; averages do not erase the defect failures |
| S1-S3 static and motion | Not run for rejected candidates; no pass inferred from prior production evidence |
| Camera/object/light motion, disocclusion, flicker | Not certified; fixed-pose repeatability is insufficient |
| Near-current optimized shadow cost | Not established; accurate-primary diagnostic is substantially slower |
| Memory | No additional persistent GPU targets/buffers/bindings in the prototypes; driver pipeline allocation and a full matched memory gate are not measured |
| Default and programmable shading | Production defaults and all user-program interfaces unchanged |

Static failure is sufficient to reject these variants before another broad motion
campaign. The remaining problem is variance reduction over the **visible**
footprint at a small ray budget without paying for many primary queries. Repeating
the bounded walk or increasing a point-centre direction reference does not solve
that problem. Temporal/radiance integration belongs to the separately owned M7
work and was not changed here. No speed or memory pass is inferred for a future
method.

## Verification, leases and retained evidence

Final CPU checks: `cargo test -p renderlab --lib -j 2` exits 0, **118 passed,
0 failed, 116 ignored**; `cargo fmt --all` exits 0. Clippy exits 0 with `-D warnings`
and the two existing main-branch allowances `clippy::nonminimal_bool` and
`clippy::items_after_test_module` (in unowned virtual-shadow/visibility code).
It is not a claim that an allowance-free main is clean. Both Python files parse.
The focused WGSL test validates and specializes all ten modes through Naga with
Interactive's `FRESH_DIRECTIONS=8`. The successful GPU batches also instantiate
the unchanged ordinary fresh entry with its default 256 directions.

Verification wall times are recorded separately from coding/analysis: the initial
cold library check took 156.996 s (test execution .52 s), the retained source's
final library check 5.650 s (test execution .52 s), final formatting 1.536 s and
Clippy 4.488 s. The first 58-pixel CPU screen took 7.956 s; closest-primary screens
on 58 and 48 pixels took 18.545 and 10.785 s. Full CPU scoring took 32.164 s and
12.736 s for the two batches. Implementation/interactive diagnosis was not
separately instrumented; no precise coding-only time is claimed.

There were two failed GPU setups, disclosed rather than counted as passes:

- 26.811 s: initially changed the ordinary `fresh_compute` ABI by sharing the
  experimental geometry estimator. Fixed by keeping its original estimator
  unchanged and restricting geometry reads to the fused test entries.
- 16.631 s: the closest-primary extension exposed a Naga override-compaction
  `ExpressionAlreadyInScope` error for a constant-folded vector `select`. Fixed
  with an explicit branch and added CPU pipeline-specialization validation.

The two successful guarded batches load the asset once each and take 123.266 s
(test 119.06 s) and 84.120 s (test 80.17 s). Each runs every relevant configuration
and pose with eight repeat hashes, then the paired timing screen. Validation
reports `errors=0`, `missing_layer=0`, `active_layer=1` for both. Exact is explicitly
pinned for the current receiver/primary identity control; no Exact output gate is
silently switched to Interactive.

| Successful batch | UTC lease start | UTC guard end | Free host RAM preflight / minimum, MiB | Test executable SHA-256 |
|---|---|---|---:|---|
| Footprint variants | 13:23:30.092110 | 13:25:33.358434 | 10,028 / 5,421.828 | `e75ad658eb8187dc5822323370f89dc280fe9700910a44d4197b80ea094d600c` |
| Closest-primary control | 14:06:21.786628 | 14:07:45.907356 | 15,857 / 12,165.344 | `6f8bc3ae274ec03161eb5f7d409b239655ba93c79e4be04fa821f3756f720f2d` |

`gpu-windows.json` includes reservation-to-release durations (including launcher
cleanup): 26.823 + 123.285 + 16.644 + 84.138 = **250.890 s**. Each lease uses
`D:/Coding/renderlab-work/gpu-slot.lock` and reservation/release lines in
`E:/renderlab-work/gpu-schedule.md`. Preflight rejects a game or less than 4 GiB
free RAM; the live guard aborts below 2 GiB. No owned Cargo process ran during a
lease. Every run was headless, blocked to completion and released. No viewer,
window, desktop capture, input or focus automation was used.

Evidence root: `E:/renderlab-work/rt1b/`. Keep `gpu/scores.json`,
`camera-gpu/scores.json`, capture hashes, complete defect CSVs, inspected PNGs,
`cpu-*/pixels.json`, replay summaries, `camera-replay-comparison.json`, manifests,
guard logs and `checks/final-summary.json`. The two large plane-defect CSVs are
enumerations to keep, not raw captures to delete. Asset and canonical R1/native
references are read-only and are not included in this cleanup list.

### Raw captures retained for Islam to remove

Earlier automatic approval review rejected raw-file deletion as blocked by
policy. No alternative deletion method was used. As requested, the exact paths,
byte lengths and SHA-256 hashes of all **64 retained captures** are listed in
`E:/renderlab-work/rt1b/retained-raw.csv` and `.json`:

| Directory | Files | Bytes | Names (expand each pose over 0 and 1) |
|---|---:|---:|---|
| `E:/renderlab-work/rt1b/gpu/` | 30 | 414,720,000 | `pose-{pose}-baseline.hit.u32`; `pose-{pose}-{variant}.u32` and `.rgba16f`, variants `baseline`, `clipped`, `plane`, `surface`, `surface-stratified`, `surface-sunflower`, `surface-exact256` |
| `E:/renderlab-work/rt1b/camera-gpu/` | 22 | 315,187,200 | `pose-{pose}-baseline.hit.u32`; `pose-{pose}-{variant}.u32` and `.rgba16f`, variants `baseline`, `camera`, `camera-stratified`, `camera-sunflower`, `camera-exact256` |
| Prior RT1: `E:/renderlab-work/rt1/current/` | 12 | 215,654,400 | `pose-{pose}-exact.hit.u32`, `-exact.u32`, `-interactive.hit.u32`, `-interactive.u32`, `-interactive.rgba16f`, `-separate.u32` |

Total: **945,561,600 bytes**. The old `rt1/deleted-raw.json` was a proposed inventory,
not evidence of deletion. CPU replay needs these captures or a fresh capture
before they are removed. After all runs, `cargo clean --target-dir
E:/renderlab-work/rt1b/target` exits 0 and removes 2,870 files / 1.3 GiB; the target
directory is absent. `checks/cleanup.json` records the result at 14:14:25 UTC.

### Reproduction entry points

From this checkout, with the private asset and evidence paths available:

```powershell
py -3 -B scripts/rt1b-replay.py --asset D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla --output E:/renderlab-work/rt1b/replay-new --captures E:/renderlab-work/rt1b/gpu --camera-replay
py -3 -B scripts/rt1b-score.py E:/renderlab-work/rt1b/gpu
py -3 -B scripts/rt1b-score.py E:/renderlab-work/rt1b/camera-gpu
```

The ignored `gpu_rt1b_footprint_batch` test uses the existing
`RENDERLAB_PRIMARY_MANIFEST` quarry manifest. `camera_control: true` selects the
closest-primary follow-up; false/absent selects the first footprint batch.
Run the compiled test binary through `scripts/n8-run.py`, with `N8_OWNER=rt1b`,
an E:-local `N8_OUT`, `--fast-check`, a reservation label and the manifest path,
only after Cargo has finished and outside any game. The saved `batch.json` and
`camera-batch.json` manifests retain the exact inputs for the measurements above.

## Proposed decisions rows

`docs/milestones-5-6-decisions.tsv` is deliberately unchanged. Proposed row text:

- **RT1b footprint receiver diagnosis:** current-policy barycentric footprints,
  edge clipping and bounded adjacency walking fail complete static quarry scores;
  accurate closest-primary footprints with 256 samples give 0/0 false shadows
  versus VSM 11/9, but eight samples leave 193-241 and cost about 1.96-1.99 ms for
  the fused primary/shadow interval in a contended development screen. No motion,
  matched memory, or optimized timing pass. Retain experimental replay/harness
  only; see this report.
- **RT default retained:** raster primaries plus Exact remain the RT-device
  default; traced + Interactive remain settings. Historical RT speed is
  **measured on the historical eight-ray setup with TR1 off (traced + Interactive)**. No below-floor
  mode is promoted and no new superiority claim is made.
