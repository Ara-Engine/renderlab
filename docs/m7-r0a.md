# M7-R0a: reflection fixtures and measured floors

Delivered: **104 Unreal static captures**, six continuous motion runs (126 readbacks),
**130 static comparison rows and 130 motion witnesses**, and eight finite-shutter reference
archives. RF1–RF10 are shared procedural fixtures. **RF10's HDR floor remains provisional
because the reference is not converged**; both current Renderlab paths also fail to produce
its mixed-program image. Those misses are retained below.

Base `79719917c0fba4d159ec81a3d466b33f6937abed`, branch `t3/m7-r0a`.
This unit adds shared fixtures, measurement tools and reference controls. It adds no
reflection implementation. Production shading, GI, shadows, temporal and pipeline files
are unchanged.

## Shared scenes

`scripts/m7-r0a-scenes.py` uses the existing U0 Builder and JSON/glTF/Mitsuba writers.
Geometry, materials, cameras and tracks come from one authored source. All fixture geometry
is procedural and redistributable; no scan geometry or third-party source is embedded.
Generated inputs and SHA256 identities are under `E:/renderlab-work/m7-r0a/scenes`.

Fixtures use 1920×1080, 45° vertical FOV, metres/Y-up, a constant sky of
1,500/1,800/2,400 cd/m² and a 100,000/98,000/94,000-lux sun with radius .00465 radians
(0.532851° diameter). Camera misses are black; secondary rays see the sky. RF10 uses
a front sun so its nonzero toon bands are unoccluded.

| Fixture | Content | Purpose |
|---|---|---|
| RF1 | Puddles at roughness 0/.05/.1; pitch 20/30/40°; tall red/blue targets | Planar mirror/glossy bands, off-screen targets, energy, position, blur |
| RF2 | Curved dielectric/metal spheres at 0/.05/.1/.2/.3/.4/.5/.7/1 and a coloured wall | Exact material/roughness populations and B0/B1/B2 cutoffs |
| RF3 | Sliding blocker (2.8 m) and camera strafe (0.8 m) | Occlusion/reveal, motion residuals, trails and recovery |
| RF4 | 1/3/10/30/100 mm rails and poles above RF1 | Thin and subpixel reflection coverage |
| RF5 | Target moves 1.2 m with 0.3 m vertical excursion; sun turns 30° | Object/lighting response and settling |
| RF6 | Puddles at 5/10° grazing views | Grazing Fresnel and elongated highlights |
| RF7 | Facing glossy panels at .05 | Recursive specular transport |
| RF8 | Generated wet quarry-like rocks at .3/.4/.5 with geometric normals | Rough-band crevices, occlusion and faceted surfaces |
| RF9 | Open two-sided shell seen from behind through the puddles | Back-face reflection coverage |
| RF10 | Default toon-program sphere in RF1 | Secondary-hit view direction, bands and rim shading |

RF8 does not certify the private scan pack. A separate CPU histogram reads the plain
quarry without exporting geometry: **12 materials, 13,034,903 placed triangles, 100%
at roughness .85**, no MR/normal texture. Counts are triangle-weighted, not pixel/area
weights. Asset SHA `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`;
details are in `scores/plain-quarry-roughness.json`.

RF3/RF5 run at 60 Hz: frames 0–31 held, 32–79 moving, 80–95 held. Witnesses are
31/32/33/35/39/48/64/79/80/81/83/87/95. Start ages 1/2/4/8 are 32/33/35/39;
settled ages are 80/81/83/87. The track-index correction preceded motion capture;
`motion/RF{3,5}-static-equivalence.json` proves only tracks changed, preserving every
imported static render input.

RF5 moves toward **−X**, away from the blue target. The earlier +X track made different
materials overlap on the same front plane near the end of motion; that moving dataset is
excluded under `*-before-overlap`. The corrected track keeps the slabs separate at all
96 frames, checked on CPU, and replaces RF5's moving references/captures only. Its complete
static-input hash is unchanged, so package C needs no recook and the static tables remain valid.
RF3 score reuse requires matching every candidate/reference archive hash and scoring version.

Finite shutters are separate estimands: centred 180° at 60 Hz, common camera/object/sun
time, with 8/16 midpoint nodes at frames 48 and 64. Native captures are zero-shutter.
The 8→16-node differences are 0.195/0.195 codes on RF3 and 0.032/0.028 on RF5 at −14;
actual-half noise is 1.159–1.260 codes there. At −10 the differences are 0.044–0.153,
against 1.734–2.348 codes of half noise. Thus quadrature differences are below the measured
Monte Carlo noise, not proof of a noiseless film reference. The complete values are tabulated.

## Reference and independent validation

The tracer already supports depth beyond two: a 1,024-segment safety ceiling and Russian
roulette. No depth extension was needed. Its default shader and FastTrace construction
remain unchanged. The original analytic compatibility control is **byte-identical on all
60 image arrays** against pinned U0b (`compatibility/a/comparison.json`).

Each camera/sample-count state reuses **bounded-as-v1**: exact geometry bytes/centre hits,
counts and visibility sums, finite images, hard four-ULP and registered relative/noise-scaled
limits, plus an exact fixed-resource replay for bounded independent-AS differences. No limit
was relaxed. Rejected states are excluded; valid partial states survive interrupted processes.

`AccelerationBuild::{FastTrace, FastBuild}` makes the construction preference explicit;
FastTrace remains default. RF5 FastBuild and RF8 FastTrace each passed eight strict paired
screens. `--ordered-hits` is a measurement-only shader override: enumerate triangle candidates,
compute source-triangle distances and order numerical ties by placement/primitive. Its tie
range is `4 × f32 epsilon × max(1, |t|)` metres; the smallest RF4 feature is 1 mm. RF10 passed
eight strict paired screens. The production reference shader is untouched.
All retained static/motion references use FastBuild. Ordered hits are used for RF8/RF10
static references and RF3/RF5 motion references; other static cases use the original query.

Actual HDR halves, primary IDs/normals and centre-reflection guides are retained. The R0a
override repurposes `normal.w` for secondary placement and `direct_even.xyz` for the guide
point. Those fields are not U0b sun/direct AOVs. Radiance is not replaced by guide data.

The toon closure uses the local ray's view vector and terminates at a program without
scatter. Unreal exports the same authored radiance through an unlit material. Mitsuba uses
an independently expressed directional emitter with zero next-event sampling weight.
No Mitsuba source is copied.

Mitsuba 3.9.1 `scalar_rgb` runs with two CPU threads, independent seeds and its own path
integrator/intersections/GGX sampler. Controls use separable Smith with pure Lambert or
F0=1 conductors. Primary fixtures retain height-correlated Smith; coloured-Schlick and
height-correlated BSDFs are not independently certified here (U0's disclosed limits carry).

Registered U0 limits stay fixed: absolute global bias <0.5% and four combined sigma;
8×8 block z RMS <1.5; <0.5% of signal blocks beyond four sigma. Edges/noisy pixels remain.

| Control | Reference / Mitsuba spp | Bias | Block z RMS | Beyond 4σ | Result |
|---|---:|---:|---:|---:|---|
| Facing panels XRF7 | 16,384 / 2×8,192 | −0.0191% | 1.1621 | 0% | pass |
| Initial narrow-sun mirror | 16,384 / 2×8,192 | −0.8355% | 1.8598 | 5.9028% | excluded |
| Narrow-sun broad GGX | 8,192 / 2×8,192 | −0.2989% | 2.0942 | 5.2083% | excluded |
| Wider-sun broad GGX | 4,096 / 2×8,192 | +0.0024% | 1.0389 | 0% | pass |
| Ordered mirror, lower count | 4,096 / 2×8,192 | −0.0296% | 1.2604 | 1.3889% | excluded |
| Ordered mirror, higher count | 16,384 / 2×16,384 | −0.1344% | 1.1540 | 0% | pass |
| Fixed proposal, lower count | 4,096 / 2×16,384 | +0.3020% | 1.3316 | 1.7361% | excluded |
| Fixed proposal, higher count | 16,384 / 2×16,384 | +0.1992% | 1.1278 | 0% | pass |

A direct Mitsuba emitter-sampler integral isolates **0.3405–0.3432% excess irradiance**
for its far sphere at radius .00465; at .05 radians it is −0.0007% to +0.0019%. The wider
disk is a validation control only; RF1–RF10 retain the original sun, with no gain correction.
The sharp mirror control keeps the 0/.05/.1 receivers; broad GGX isolates the closure separately.

RF10 at 4,096 spp has substantial bright-path variance: reflector split RMS 75.7%, display
split RMS 5.01 codes at −14. The retained reference uses a validated fixed planar-sun proposal:
three cones around the sun reflected across y=0, mixed 50:50 with the original BSDF proposal
on eligible diffuse hits. Throughput and sun/sky MIS use the complete mixture PDF. It changes
no transport/depth, and no production shader or estimator default.

**RF10 is not HDR-converged at 16,384 spp:** reflector split RMS is **48.427%**, with
8,192→16,384 image change **39.054%** and energy change **+0.0454%**. Display split noise
is **4.636/1.823 codes** at −14/−10; HW-Hit Epic errors are **15.611/12.336 codes**.
All pixels remain. The large missing-feature/energy differences are visible, but the RF10
HDR floor is provisional and cannot certify a close ranking. This is a reference-convergence
miss, not permission to relax the later R0b/R1 quality gate.

Static reference samples are 4,096 per pixel for RF1–RF9 and 16,384 for RF10. Motion
centres use 4,096; finite-shutter nodes use 256 each. `reference/final/manifest.json`
identifies all 13 static camera cases and their exact qualified producer recipes.

## Unreal identity and settings

Final captures use **package C only**, `E:/renderlab-work/m7-r0a/ue-package-c`,
`cook/c/identity.json`, parent U0b `cook/b` SHA
`ece21fda136c832c7c7d7982ee39bc81a81f200f418f640dead82e6ddf0c8dc7`.
Executable SHA is `69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5`;
the cook manifest identifies pak/utoc/ucas and exact toon code/source hashes. Executable
identity alone is insufficient.

U0b's project/importer supplies mesh bounds/triangle checks, preserved normals, Nanite off,
distance fields at scale 1 and two-sided distance fields. Content is under `/Game/M7R0a`.
An exhausted retry loop initially skipped the toon basis correction; image inspection caught
diagonal bands. It now requires commandlet success and a scene/adapter-hashed marker;
cooking rejects stale markers. Package C's checked pilot has the correct concentric bands.
A/B captures are superseded.

Runs are hidden/offscreen D3D12 SM6, Unreal 5.7.4 CL51494982, native 1920×1080, TSR Epic
at 100%/200% history, fixed 1/60 s and OneFrameThreadLag 0. GI/reflections use Epic (3) or
Cinematic (4); AA stays Epic. PreExposureOverride is 0; actual exposure comes from captured
EyeAdaptation/Tonemap inputs. Camera, sun angle, sky intensity and motion transforms are checked.

| Row | Effective tracing/lighting | Shadows | Role |
|---|---|---|---|
| SW Epic/Cinematic | Runtime RT off; mesh-SDF Lumen | VSM | non-RT bar / higher target |
| HW-SC Epic/Cinematic | Hardware Lumen, lighting mode 0 | HRT | reported comparator |
| HW-Hit Epic/Cinematic | Hardware Lumen, hit lighting mode 2 | HRT | RT bar / higher target |
| SSR off/on | ReflectionMethod 0/2, SSR quality 0/4; software Lumen GI | VSM | informative only |

Lumen's own reflection/screen-probe screen traces remain 1. Hardware rows disable duplicate
distance-field storage. Each run retains settings/scalability dumps and actual tracing/pass
proof. The HRT preset's angle is reset to .532851°, proxy/cache flushed and queried again.

## Measures and baseline limits

Complete per-population tables are in [m7-r0a-floors.md](m7-r0a-floors.md); source identities
and qualification summaries are in [m7-r0a-evidence.json](m7-r0a-evidence.json).

Whole-reflector display RMS at −14, in 8-bit F0 codes (lower is better):

| Fixture / pose | SW Epic | HW-SC Epic | HW-Hit Epic | Current nonRT | Current RT | Reference split noise |
|---|---:|---:|---:|---:|---:|---:|
| RF1 / pitch30 | 37.165 | 11.024 | 9.331 | 46.100 | 46.100 | 0.438 |
| RF1 / pitch20 | 29.193 | 8.789 | 8.184 | 50.191 | 50.191 | 0.459 |
| RF1 / pitch40 | 34.707 | 9.824 | 7.325 | 38.857 | 38.857 | 0.427 |
| RF2 / sweep | 15.920 | 14.862 | 15.195 | 20.956 | 20.954 | 4.811 |
| RF3 / pitch30 | 32.202 | 10.819 | 10.140 | 45.757 | 45.757 | 0.537 |
| RF4 / pitch30 | 40.611 | 12.065 | 10.469 | 45.697 | 45.697 | 0.762 |
| RF5 / pitch30 | 37.165 | 11.025 | 9.331 | 46.100 | 46.100 | 0.438 |
| RF6 / grazing5 | 12.273 | 9.581 | 9.529 | 52.315 | 52.315 | 0.516 |
| RF6 / grazing10 | 12.457 | 6.995 | 6.750 | 51.754 | 51.754 | 0.507 |
| RF7 / between-panels | 35.645 | 20.298 | 20.203 | 48.035 | 48.035 | 4.354 |
| RF8 / pitch30 | 5.241 | 3.918 | 3.913 | 9.901 | 9.873 | 0.357 |
| RF9 / pitch30 | 37.020 | 10.521 | 9.606 | 46.163 | 46.163 | 0.509 |
| RF10 / pitch30 | 53.124 | 26.196 | 15.611 | no frame | no frame | 4.636 |

These aggregate rows do not replace the exact-roughness, dielectric/metal, off-screen,
thin-feature, energy, position, blur, p99 and motion tables. In particular, RF2/RF7/RF10
reference noise precludes treating a small aggregate mode difference as a ranking.

Motion reflector display RMS at −14 (current rows are warmed independent snapshots,
not continuous-history measurements):

| Fixture / mode | Start age 1 (32) | Start age 8 (39) | Stop age 1 (80) | Stop age 8 (87) |
|---|---:|---:|---:|---:|
| RF3 / SW Epic | 32.754 | 35.505 | 38.941 | 39.030 |
| RF3 / HW-SC Epic | 10.670 | 10.766 | 10.638 | 10.663 |
| RF3 / HW-Hit Epic | 10.406 | 10.410 | 6.293 | 6.270 |
| RF3 / current nonRT | 46.327 | 49.701 | 53.833 | 53.833 |
| RF3 / current RT | 46.327 | 49.693 | 53.833 | 53.833 |
| RF5 / SW Epic | 36.422 | 36.276 | 35.312 | 35.243 |
| RF5 / HW-SC Epic | 11.637 | 12.190 | 11.971 | 11.960 |
| RF5 / HW-Hit Epic | 10.209 | 9.996 | 9.112 | 9.044 |
| RF5 / current nonRT | 46.154 | 46.516 | 43.748 | 43.748 |
| RF5 / current RT | 46.154 | 46.516 | 43.746 | 43.746 |

Visual inspection retains the complete frames at both exposures, original-pixel RF4 thin
crops, every motion witness/mode and difference sheets. Software Lumen has shortened
reflections, spurious reflected structure and fragmented thin/back-face geometry; those
misses remain after motion stops. Hardware preserves the main target extents but has
lighting, recursive-reflection and disocclusion errors. Current Renderlab omits local
reflected objects on both paths. The sparse witness flipbooks do not certify unsampled
60 Hz flicker. Native motion uses 21 readbacks per run, of which 13 are scored witnesses.

Full primary populations, exact roughness/material groups and reference-defined off-screen
guides are scored with RGB relative RMS, luminance relative L1, signed/absolute energy bias,
actual-half noise, F0 Reinhard/sRGB RMS and p99 maximum-channel errors at −14/−10 stops.
The brighter display uses the same unexposed HDR. No gain fit, denoising or pixel rejection.

Feature energy uses reference footprints; colour energy/centroids span the whole reflector,
retaining spill/trails. Blur uses reference-level 10–90% transitions; missing edges stay misses.
RF4 also has fixed projected bounds plus one AA pixel, retaining subpixel rails with no centre
guide. Conservative ROIs include background and accompany full-reflector errors.

Motion retains every witness/age, adjacent residuals and visible trails: >2 excess colour codes
in an old-only footprint, Chebyshev extent, >32 px retained as a lower bound. Zero trails never
replace missing-feature/image errors. Finite shutters are reported separately.

Renderlab remains environment-only for reflections. Non-RT has a separate device with no
ray-query feature, visibility-buffer primaries and 4096 Virtual disk shadows; RT uses traced
primaries and Interactive disk shadows. The harness requests current Virtual's device buffer
capacity. States warm eight frames; indirect GI stays off.

**RF10 is unsupported on both current paths:** the shared deferred path rejects mixed
standard/toon programs. It is an image-production miss, with no substitute material/image.
Current motion rows are independent warmed snapshots, not continuous history/settling claims.
Unreal rows are continuous 96-frame runs with per-frame transform/sun proof.
The current renderer also retains its environment background on primary misses, unlike the
registered black reference/native background. Full-image errors retain that visible miss;
the reflector populations are independent of it.

Reference noise is reported, not an error allowance; close rankings need more samples.
These are comparator inputs, not film-perfect convergence, a passing reflection implementation
or a 2× performance claim.

## Checks and retention

`cargo test -p renderlab --lib -j 2 -- --test-threads=2`: **134 passed, 130 ignored**, exit 0.
Fmt exits 0. Total Cargo/fmt wall is 5.886 s, tests 1.42 s. `checks.json` records Rust source
hashes. Reference binary SHA is `139066a4b65c0d534a6e3cb9d53773f653e72a0a7ec4408d0b0d6aa42e9a9742`;
baseline binary is `b296a8d94d0e21e3c6ed04bf40edb9ab46532b839d0c6129a762323217e06b6f`.
Five focused CPU checks, all eight Python syntax parses, fmt and protected-file checks pass.
Rust source hashes still match the passed library-test executable. The final identity audit
checks all static/motion settings, package identity, qualified references, shutter-member
hashes and independent controls; its committed result is `m7-r0a-evidence.json`.

Every GPU run owns the D: lock/token. Controllers block in-unit, respect game holds/actual
QUIET, start at ≥4 GiB RAM/≥5 GiB disk and abort below 2 GiB of either. CPU tools announce
themselves. Compiler work is allowed for correctness; Cargo build scripts are CPU-classified.
Schedule sharing errors retry for ≤2 s, then fail closed. No visible window, desktop/game
capture or input automation is used.

Functional CSV/ProfileGPU/memory telemetry retains configuration identities; timings are
ineligible for speed claims. Hardware is an RTX 4070 SUPER (12 GB), driver 617.14, Ryzen
7 7800X3D and 32 GB RAM. Foreign compilers were active. Memory records cover Lumen component
buffers, RT geometry and transient pools; they do not establish a total-VRAM budget.

Recorded guarded-process wall is **157.438 minutes** over 512 attempts; GPU-lock occupancy
is **172.153 minutes**, including CPU archiving while held. Failed/excluded attempts are
included. Neither number is GPU busy time or end-to-end verification duration. Active
implementation time was not instrumented; coding and preparation overlapped verification.
Library/fmt verification is separately recorded above.

Raw arrays >5 MB are hashed and removed after archival/scoring. Intentional reference archives
retain actual halves/geometry for R0b/R1. Native dumps are disposable after summaries/inspected
PNGs, including 260 full-resolution motion triptychs and 20 lossless witness flipbooks.
Staged duplicates are deleted only after full hash proof. The owned Cargo target is deleted;
pinned executables remain. `docs/milestones-5-6-decisions.tsv` is unchanged.

Canonical inputs for R0b/R1 are `reference/final/manifest.json`, `reference/m-ordered/`
(motion centres), and `motion/reference/` (integrated halves). Scores are
`scores/c-static.json` and `scores/c-motion.json`; readbacks stay beside each capture.
`qa/c/visual-audit.json` lists inspected files and observations. The controllers are
`scripts/m7-r0a.py`, `m7-r0a-report.py`, `m7-r0a-audit.py` and `m7-r0a-cleanup.py`.
Recipes retain exact commands, binary/shader hashes and settings. Native arrays over 5 MB
are disposable after scoring: use a fresh capture `--label` to regenerate them before rescoring.

After final verification R0a needs no recook: DDC and Saved/Cooked may be released. Keep
package C/source Content for later units; original U0/U0b packages are preserved.

Patent policy: independent path tracing, numerical ordering and optional fixed proposals stay
in measurement overrides. No WRC, reservoir reuse, common projected-ray reuse, production
denoiser or patented fallback is introduced, and no runtime shading constraint is added.

AA state (IQ1, 2026-11-08; [iq1-quarry-image.md](iq1-quarry-image.md)): this unit's current captures rendered 1x with TR1 off and anisotropy 1, the shipping default (un-anti-aliased).
