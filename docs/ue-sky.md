# UE-SKY: native TSR on TR1b's visible-sky quarry walk

2026-11-08, branch `t3/ue-sky`, based on `4a2d08f`. Measurement scripts and
documentation only. No renderer, default, or decisions-TSV change. Contract:
[M7's visible-sky ruling](milestone-7.md),
[TR1b §4.5 at its recorded commit](https://github.com/Ara-Engine/renderlab/blob/41575360d45128afd01f52fbcf9cb2824a80b273/docs/m7-tr1b.md),
[U0b](m7-u0b.md), and [TR0](m7-tr0.md).

**Measurement unit complete:** the package, physical calibration, 160 quality
frames, complete TSR/TR1b row export and eight-pair cost curve are retained.

Islam's later overnight direction (`AGENTS.md` on main `984a18b`) stops new
Unreal/reference campaigns. This unit's package and all 160 quality captures
were already finished; only three visits of its existing cost follow-up remained.
It closes under the explicit nearly-finished UE-SKY exception, without starting
another capture/reference campaign. The overnight default policy supersedes the
older strict promotion gate; these frozen comparison rows do not make a new
default decision.

The new package is `D:/Coding/renderlab-work/ue-sky-package`; evidence is
`D:/Coding/renderlab-work/ue-sky`. Existing E: packages and source content are
preserved. Captures are native Unreal main-view HDR, before Tonemap, with a real
sky material behind silhouettes before TSR. There is no sky compositing or
sky/edge exclusion in the quality scores. High supplies the cost curve; Epic
remains the comparator floor. These measurements do not promote TR1 or establish
integrated Renderlab acceptance.

## Package and input identity

Unreal **5.7.4 CL51494982**, packaged Development executable
`Windows/Engine/Binaries/Win64/UnrealGame.exe`, SHA256
`69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5`.
Its unchanged executable hash does not identify the new cooked content:

| File in `Windows/M5Calibration/Content/Paks` | SHA256 |
|---|---|
| `M5Calibration-Windows.pak` | `44b1d0462a4c63a216efacb39137dcb70022a690fdc30ced7598d41c16aaa882` |
| `M5Calibration-Windows.ucas` | `d8a456a4fa26be35d00c5b09fb162f018a687e5cf349a42c9f831749d755fd59` |
| `M5Calibration-Windows.utoc` | `81fa6965eb7fea7b6b84ca7b025820266b09ba8dd72c08abecb5994da48090b9` |
| `global.ucas` | `cacd20d1ec919616ba31aaf2d5a897cfd5ef1ea02e175b1bdb5b7ede23842f84` |
| `global.utoc` | `5cdb0eccb5af619d2a867556686e77ec12e3f138cdc4e8b954042c3cea389298` |

`cook/identity.json` inventories every packaged file. The package is
**1,522,176,364 bytes** before runtime Saved files. Both cooked maps are explicit
launch targets:

- `/Game/UESky/SkyWalk`: the twelve original quarry meshes/materials, plus the
  primary sky sphere.
- `/Game/UESky/SkyCalibration`: the same camera/exposure, with quarry geometry
  removed and only the sphere rendered.

The private project is an initially byte-verified copy of
`E:/renderlab-work/m7-u0/ue-project/{Content,Config}` onto D:. Only the new maps and
sky material are authored. Its private filesystem DDC also points to D:.
No Epic source is copied into the repository.

The exact 40-frame input is
`D:/Coding/renderlab-work/m7-refs-sky/sky-walk.json`, SHA256
`ef28be7bed91ce3dd24a432f799b55005632b5e45214639cdd2e731f8a1f4e58`:
frames 0–7 still, 8–31 moving sideways 1 cm/frame, 32–39 still, 25 distinct
states, 1920×1080, 60 Hz, zero shutter. All camera coordinates and FOVs are reused.

**Correction to the source's “8°” description:** its numerical first view points
along `[0.370075291, 0.406835189, 0.835182261]`. This is **4.688506°** from the
authored RLE2 sun-lobe axis `[0.36,0.48,0.8]`
(`crates/cook/src/environment_fixture.rs::sun_sky`), and 4.785200° from its
brightest level-zero texel. The separate directional light is 39.870565° away.
This unit preserves the saved path and references; it does not substitute a new
8° camera path or claim that this numerical input is 8° from the map's sun.

## Physical sky calibration

The unlit, two-sided `M_PrimarySky` samples
`/Game/M6Quarry/Environment/quarry-sky-v2` at explicit mip 0 along
`-CameraVectorWS`. U0's DDS basis is source-to-Unreal `(-Z,X,Y)`, yaw 0. The cube's
exact `/64` storage scale is reversed by emissive ×64; source and output units
are cd/m². The sphere casts no shadow and is excluded from ray tracing, distance
fields, collision and indirect-light construction. Lighting retains the original
specified quarry cube at intensity 64.

RLE2 SHA256 is
`3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764`.
The cube reads back as linear `TC_HDR_F32`, 128 square. Manual exposure is −14
stops, `r.EyeAdaptation.PreExposureOverride=0`. No intensity, exposure, orientation
or gain is fitted to an image.

Two independent no-AA no-geometry processes, `calibration-nonrt-b` and
`calibration-rt`, produce **bit-identical native HDR pixels**. CPU evaluation of
the RLE2 level-zero cube uses the same fixed camera rays, bilinear filtering and
adjacent-face taps. Every image pixel is included; cube seams are also reported
separately. Native R11G11B10 quantization remains visible:

| Calibration population | Pixels | Linear relative RMS | F0 RMS codes |
|---|---:|---:|---:|
| Full image | 2,073,600 | 0.753503% | 0.163142 |
| Cube seams | 20,944 | 0.775524% | 0.238357 |
| Map sun, expected max RGB >1 exposed | 353,233 | 0.753504% | 0.231462 |
| Remaining sky | 1,720,367 | 0.740389% | 0.145190 |

The predeclared calibration checks are <2% linear relative RMS and <0.5 F0 code
RMS on every calibration population. Both processes pass. Full-frame maximum
absolute code error is 1.497496. `calibration.json`, native archive hashes and
inspected native/reference/16×-difference PNGs retain the proof.

The first package attempt is rejected. Unreal silently returned false for a
texture input named `Coordinates`; the cook exited 0 despite a missing cubemap
UVW and invalid material shader map. Its no-geometry HDR was black. The retained
prepare script uses the real `UVs` input and checks every graph connection; the
cook and runtime now reject material compilation/fallback warnings. Failed
attempt logs, hashes and its inspected PNG remain separate from accepted evidence.

## Capture and scoring protocol

The four cases are `nonrt-tsr-epic`, `nonrt-tsr-high`, `rt-tsr-epic`, and
`rt-tsr-high`. Non-RT disables `r.RayTracing` globally and uses the original VSM
8×8 settings. HRT uses one-sample ray-traced shadows and its native denoiser.
This is TR0's direct-light/environment subset, with GI/reflections disabled;
it is not an integrated Lumen comparison.

All runs are hidden and offscreen under their own
`D:/Coding/renderlab-work/gpu-slot.lock` lease. The inherited fail-closed guards
check game directives/processes, foreign renderers, RAM and disk headroom.
CPU tools announce themselves in the schedule and defer to active foreign QUIET
windows. No Renderlab test binary or Cargo build is needed by this unit.

All captures use D3D12 SM6, native 100% rendering, fixed 1/60-second simulation,
motion blur/dynamic resolution off, and `r.OneFrameThreadLag=0`. Runtime CVar
readback verifies every installed AA scalability setting: Epic has 200% history,
High 100%. Each frame separately verifies camera position/direction/FOV, sun
direction and **4.583662361° diameter**, measured global exposure **2^-14**, local
exposure 1, white tint, TSR jitter and camera-cut state. Exposure conversion uses
the measured native value, never a fitted scale.

TR0's reset/arming order is unchanged: CSV95 resets history before arming, and
CSV96 is recorded frame 0. The preparation sample means its first recorded
jitter is `[0.25,-0.3888889]` and `bCameraCut=0`; all 40 cut flags are 0. The full
native-resolution 11-sample Halton cycle is checked numerically for every frame.
Start-age pools therefore compare the registered timelines, not equal empty
histories. The inherited UE 1 cm near-clip clamp versus the reference's 0.5 cm
also remains disclosed.

`scripts/ue-sky-tr1-score.py` is the exact repository-owned scorer snapshot from
TR1b `41575360d45128afd01f52fbcf9cb2824a80b273`, SHA256
`ece9e52c0bdb710372738f879f8f0d9d866a12c6912fafe05c248670ede472be`.
It reproduces the recorded TR1b/non-RT-or-RT sky gate arithmetic (256 all-frame
rows, RT 101 better/149 worse). It reads the same 25 accepted 1024-spp compact
physical references and fixed geometry ages from TR1b's `m7-refs-sky` release, with the same
L0/+4-stop transforms, masks, reference-noise subtraction, maxima, signed means,
revealed/start ages and motion residuals. The comparison verifies identical
reference hashes and per-frame population counts before pooling any rows.

During the last CPU score the producer moved its references into
`m7-refs-sky/release-tr1b-k` before regenerating a later set. The interrupted score
is discarded and repeated. All **25/25** archived hashes match the already scored
captures and original TR1b rows. `reference-tr1b-k/`, `baseline-scores/`, and
`frozen-inputs.json` now pin those original references, age masks and score files
inside this unit; subsequent producer output is not mixed in.

Native HDR is retained losslessly as `runs/<case>/frames/frames.npz` plus measured
exposure factors. Each archive round-trips exactly to its native R11G11B10 values
represented as float16. `readback.json`, `capture-readback.json`, `frame-proof.json`,
`native-archive.json` and process logs retain settings, every camera/sample and
byte identities after intermediate buffers are deleted.

## TSR rows and cost curve

All **160/160** quality frames pass runtime, camera, light, exposure, jitter and
cut readback, with **zero missing reference frames**. `comparison/all-rows.csv`
contains **2,952 pooled rows**, each with no-AA, TR1b, TSR Epic and TSR High side
by side; `comparison/summary.json` pins every input score and proves identical
reference hashes and population counts. Lower is better; there is no tolerance
added to TR1b's exact row comparison.

TR1b against TSR Epic on the **physical path-traced target**:

| Segment | Rows | RT better / worse / tied | Non-RT better / worse / tied |
|---|---:|---:|---:|
| All 40 frames | 256 | 37 / **216** / 3 | 40 / **213** / 3 |
| First still, 0–7 | 166 | 28 / 132 / 6 | 28 / 132 / 6 |
| Moving, 8–31 | 172 | 24 / 142 / 6 | 24 / 142 / 6 |
| Last still, 32–39 | 256 | 46 / 207 / 3 | 46 / 207 / 3 |
| Aged start/revealed pools | 626 | 40 / 586 / 0 | 40 / 586 / 0 |

Selected rows below are **L0 / L4 F0 codes**. RMS is reference-noise-subtracted;
flicker is the reference-subtracted residual on same-instance covered pixels of
geometric age ≥8. The hold step is the ordinary frame-to-frame RMS on that pool.
These rows are taken directly from the complete CSV, not a separately selected
pixel mask.

| Path | Row | No AA | TR1b | TSR Epic | TSR High |
|---|---|---:|---:|---:|---:|
| RT | Moving covered RMS | 26.663 / 85.181 | 26.377 / 86.588 | 15.788 / 68.215 | 15.877 / 68.632 |
| RT | Moving edge RMS | 49.173 / 102.131 | 36.882 / 104.357 | 36.190 / 76.987 | 32.480 / 77.891 |
| RT | Moving background-centre RMS | 0.202 / 0.313 | 0.370 / 1.219 | 1.648 / 1.148 | 0.524 / 0.743 |
| RT | Revealed covered, age 0 | 51.068 / 128.201 | 40.304 / 124.246 | 27.307 / 96.162 | 26.983 / 95.732 |
| RT | Revealed covered, age 3 | 43.733 / 118.420 | 39.288 / 120.271 | 26.854 / 90.818 | 28.186 / 92.770 |
| RT | Moving flicker | 16.234 / 46.341 | 13.763 / 40.909 | 8.619 / 22.701 | 8.385 / 22.235 |
| RT | Final-hold step | 0.000 / 0.000 | 0.800 / 1.878 | 0.323 / 0.796 | 0.405 / 0.974 |
| Non-RT | Moving covered RMS | 26.664 / 85.182 | 26.378 / 86.589 | 15.790 / 68.219 | 15.878 / 68.636 |
| Non-RT | Moving edge RMS | 49.187 / 102.138 | 36.950 / 104.375 | 36.191 / 76.981 | 32.481 / 77.891 |
| Non-RT | Moving background-centre RMS | 0.230 / 0.322 | 0.370 / 1.220 | 1.648 / 1.148 | 0.524 / 0.743 |
| Non-RT | Revealed covered, age 0 | 51.059 / 128.193 | 40.283 / 124.226 | 27.303 / 96.152 | 26.986 / 95.738 |
| Non-RT | Revealed covered, age 3 | 43.738 / 118.424 | 39.275 / 120.287 | 26.857 / 90.827 | 28.187 / 92.774 |
| Non-RT | Moving flicker | 16.235 / 46.343 | 13.765 / 40.907 | 8.619 / 22.702 | 8.386 / 22.235 |
| Non-RT | Final-hold step | 0.000 / 0.000 | 0.804 / 1.889 | 0.322 / 0.795 | 0.406 / 0.975 |

These are complete-image comparator rows, combining lighting/material/shadow and
reconstruction error. They do not attribute the covered-pixel gap solely to AA.
The nearly black reference rock and brighter foreground in both real-time
renderers are retained, as TR1b §4.5 requires. TR1b's separate lighting-matched
64-spp control and its 31 all-frame losses to no-AA are not replaced by this
physical-target comparison. The retained native archives permit later rescoring.

Full-frame native/reference/error sheets at frames 0, 7, 8, 16, 31 and 39, plus
native-size silhouette strips at frames 8, 16, 31 and 39, were inspected at both
exposures on both paths. They show aligned sun/background and moving silhouettes,
the foreground lighting discrepancy, and transient edge roughness during motion.
They are under `inspection/`; their hashes and observations are retained with the
campaign. TR1b's loose native HDR files had already been deleted by its owner;
its recorded numeric rows are preserved, and no replacement TR1b image is invented.

The cost curve uses the same package, walk, native resolution and **thread lag
0**, on RTX 4070 SUPER / NVIDIA **617.14**. Native CSV `GPUTime` is the GPU
critical-path total; `GPU/TemporalSuperResolution` is its separately measured
scope. Their difference is not obtained by adding inclusive pass groups.
Each configuration has four processes with two adjacent no-AA/TSR pairs each:
**eight pairs, 256 samples per method**, excluding frames 0–7 of each replay.
All no-AA samples have zero TSR scope; all TSR samples have a positive scope.
Readback and independent process guards pass for every retained visit.

| Path | Quality | No-AA p50 | TSR total p50 / p95 / p99 | TSR scope p50 / p95 / p99 | Paired total increment p50 / p95 / p99 |
|---|---|---:|---:|---:|---:|
| VSM, globally no RT | Epic | 1.715 | 3.104 / 3.495 / 4.233 | 1.233 / 1.567 / 1.891 | 1.391 / 1.761 / 2.534 |
| VSM, globally no RT | High | 1.719 | 2.697 / 3.328 / 3.664 | 0.827 / 1.156 / 1.309 | 0.976 / 1.402 / 1.764 |
| HRT | Epic | 1.727 | 3.102 / 3.311 / 3.669 | 1.240 / 1.272 / 1.616 | 1.379 / 1.598 / 1.945 |
| HRT | High | 1.723 | 2.690 / 2.726 / 2.757 | 0.832 / 0.836 / 0.844 | 0.970 / 1.025 / 1.051 |

All values are milliseconds. `timing/q3/costs.json` retains means and exact
quantiles; each visit retains CSV, commands, readback and process records.
These are TSR costs for this walk, not a Renderlab speed/acceptance claim or an
integrated Lumen frame cost. Unlike TR0's original timing runs, these explicitly
use `r.OneFrameThreadLag=0`.

The quiet GPU leases pause/restore the supported wallpaper setting and verify
a ≤3% idle GPU start. A full compiler-free minute precedes GPU work; each child
also stops on a foreign compiler. `q1` never acquired a lease or launched a GPU
job. `q2` completed thirteen independently guarded visits before its fourteenth
was stopped on a foreign compiler at **15:52:19Z**. That rejected process is
excluded. All thirteen retained processes ended before it started. `q3` collected
only the three missing visits after a fresh quiet-start check and compiler-free
minute, and released at **16:01:46.213050Z**. Its certificate pins the earlier
thirteen records and their qualification cutoff. Wider tails in the resumed
visits remain in the table; no new repeat campaign was started at the overnight
cutoff.

## Cleanup and retained evidence

The verified package and private source project's config/uproject remain on D:.
This cook's DDC, Intermediate, Saved/Cooked, duplicate staging/build output and
duplicate source Content are removed; original E: source Content and all existing
packages are preserved. The rejected black-sky package is also removed, keeping
its identity, logs and inspected failure image. `cook/cleanup.json` records exact
paths, byte counts, package verification and the original-source hash check.
The removed cook/source/staging/rejected-package duplicates total
**7,070,623,380 bytes**; GPU-buffer deletion ledgers total **61,859,635,200 bytes**.
All **442 original Content/Config files** still match the initial source hashes.

Accepted native HDR archives, the frozen TR1b reference release and ages, score
JSON/CSV, native parameter/readback records, logs and inspected PNGs remain for
integrated rescoring. The large intermediate GPU buffers are hash-recorded and
deleted. No Cargo target was created by UE-SKY. `docs/milestones-5-6-decisions.tsv`
and renderer sources are unchanged.

`final-verification.json` records 40 camera, light, temporal and exposure checks
per quality case, all four score hashes, equal population/reference identities,
Python syntax validation, the frozen scorer hash, inspected-image hashes, cleanup
receipts and the absence of an owned GPU process or lease. The accepted quality
processes took **245.3 / 99.7 / 153.9 / 117.9 seconds wall** (non-RT Epic/High,
RT Epic/High): DumpGPU/readback/IO overhead, not frame time. Their minimum
available RAM was **9.749 / 11.093 / 11.775 / 10.195 GiB**. No Rust build or test
was run because renderer code is unchanged.

## Reproduction

Use Python with NumPy, SciPy and Pillow on the recorded Windows/Unreal host.
The existing source project and retained TR1b references are required; private
scan assets are not committed. Run from the repository root:

The recorded destinations already exist and are protected. To rebuild without
replacing them, pass fresh `--root D:/Coding/renderlab-work/ue-sky-rebuild` and
`--package D:/Coding/renderlab-work/ue-sky-package-rebuild` **before the subcommand
on each invocation**. The commands below show the original campaign's defaults.

```powershell
py -3 -B scripts/ue-sky.py clone
py -3 -B scripts/ue-sky.py freeze
py -3 -B scripts/ue-sky.py prepare
py -3 -B scripts/ue-sky.py cook
py -3 -B scripts/ue-sky.py capture calibration-nonrt-b --calibration --path vsm-nort --minutes 3
py -3 -B scripts/ue-sky.py decode calibration-nonrt-b
py -3 -B scripts/ue-sky.py calibration calibration-nonrt-b
py -3 -B scripts/ue-sky.py capture calibration-rt --calibration --path hrt --minutes 3
py -3 -B scripts/ue-sky.py decode calibration-rt
py -3 -B scripts/ue-sky.py calibration calibration-rt
```

For each path (`nonrt` → `vsm-nort`, `rt` → `hrt`) and quality (`epic` → 3,
`high` → 2), capture its case, then run `decode`, `score` and `prune` with that
case label. `inspect rt`, `inspect nonrt`, and `report` produce the retained
comparison sheets and full CSV. `timing <new-label>` owns a separate quiet
window; `costs <label>` validates and summarizes it. `cleanup` verifies the
retained package before removing this cook's regenerable project output.
Existing package destinations are rejected rather than overwritten.
`timing <new-label> --resume <earlier-label>` can reuse independently guarded
successful visits that ended before an earlier window's failed process; it never
reuses that failed process. No new Unreal campaign is authorized by this
reproduction recipe during the overnight stop.
