# M7-U0b: textured quarry reference and Unreal follow-through

Implementation unit on main `708e87c`, branch `t3/m7-u0b`, 2026-11-07.
Contract: [M7](milestone-7.md), [design](m7-design.md), [U0 evidence and misses](m7-u0.md).

**Work in progress.** Textured/environment validation, four 4,096-spp quarry references,
the new packaged benches and three quiet static visits per configuration are retained.
The full Unreal bench matrix, motion witnesses and physical floor table remain incomplete.
The S2G replay failure and a small-sun precision error are fixed and qualified. No new 2x
or integrated quality-pass claim is made.

## Reference implementation

`reference::Scene::from_prepared` consumes the validated RLA without an importer or image-codec
dependency. It preserves full source triangles, placements, vertex colours, UVs and tangent
frames, base-colour sRGB textures, linear metallic/roughness and linear normal maps. Sampler
wrap and point/bilinear filtering are explicit. Complete mip payloads are uploaded, with a
controlled explicit LOD; physical references use level zero and integrate the pixel footprint
through their samples, avoiding a second prefilter of the material.

The standard model remains Lambert plus GGX/Schlick with height-correlated Smith. Specular AA
is off, as in the standard program at this base. The Mitsuba control uses separable Smith,
as U0 did; its texture/transport agreement does not independently validate the correlated form.
The tracer remains a two-sided opaque reference; all twelve registered quarry materials are
double-sided. It does not evaluate arbitrary user programs.

The prepared environment uses the unblurred RLE2 specular level zero, source yaw and intensity.
A cube CDF uses luminance and exact texel solid angles, with a support floor for seam-aware
bilinear evaluation. Samples within a texel are uniform in cube UV, with the matching
solid-angle Jacobian in the PDF. BSDF/light MIS is unchanged. Prepared sky and the separate
sun are additive. A camera-only black backdrop matches Unreal's specified-lighting-cube scene;
secondary rays still see the environment.

No reservoirs, radiance caches, temporal filters, denoisers or flagged patent mechanisms are
introduced. This is plain unidirectional path tracing, NEE and power-heuristic MIS. No Epic,
NVIDIA SDK or Mitsuba source code is copied. Generated benches remain our own geometry;
private scan payloads and captures stay out of Git.

## Validation retained so far

- Library tests: **125 passed, 119 ignored**, exit 0. The new CPU checks cover cube-CDF support
  and RGB-correlated uncertainty; Naga validates the complete measurement shader.
- GPU analytic run `reference/analytic-final`: all pass, zero invalid pixels. Delta-sun and
  constant-sky Lambert floors are 1.000000. The prepared-cube floor is
  **0.999968 +/- 0.000187**. A constant normal-mapped plane is **1.000000**.
- Textured X3: 32,768 spp against two independent Mitsuba 3.9.1 seeds of 16,384 spp each.
  Global bias **+0.000441% (0.107 sigma)**, relative L1 **0.328%** against Mitsuba seed-to-seed
  **0.576%**, block z RMS **0.9595**, **0/900** blocks beyond 4 sigma. All registered U0
  tolerances pass, and the same-seed replay is bit-identical. The retained side-by-side image
  and 8x difference were inspected: structureless noise. The exporter V-axis flip was found
  and corrected before these results; its rejected evidence remains separate.
- Quarry smoke: full **13,034,903 triangles**, **12 x 4096-square base-colour textures**, each
  with **13 mips**, the registered RLE2 identity `3ccd61b1...c1764`. There are no normal maps in
  this schema-1 quarry asset. At 480x270 and 16 spp it passes with zero invalid pixels; its
  retained PNG was inspected. This is a smoke, not a converged reference.

### Furnace's low reading

U0's region uncertainty treated RGB as three independent paths. The enclosure is grayscale:
the channels are identical and fully correlated. Pooling channel means per pixel before
combining independent pixels corrects sigma by sqrt(3). The same reading is
**0.999403834 +/- 0.000369432**, **1.61 sigma** low, rather than 2.8 sigma.

Eight independent seeds at 4,096 spp span **0.9994038-1.0005028**. Their aggregate mean is
**0.99993875**, between-seed SD **0.00035965**, SE **0.00012715**. With eight seeds, Student's
95% interval is approximately **[0.999638, 1.000239]**: zero loss is inside it, and a systematic
loss larger than **0.0362%** is excluded at this confidence under the independent-seed model.
The test does not establish an
exact zero-bias guarantee; it bounds a systematic loss at the measured precision.

## Retained quarry references

`E:/renderlab-work/m7-u0b/reference/quarry-final` retains actual independent even/odd
halves, not reconstructed halves from a mean/error image. Each image integrates the pixel,
uses the full prepared geometry and textures, sun plus RLE2 (or RLE2 only), MIS, RR and a
1,024-depth safety ceiling. No denoising is applied. Resolution is 1920x1080.

| Reference | spp | Median / p95 / p99 relative noise | Render call seconds |
|---|---:|---|---:|
| quarry-p0 | 4,096 | 0.289% / 1.213% / 3.218% | 22.27 |
| quarry-p0-sky | 4,096 | 0.292% / 1.224% / 3.342% | 17.04 |
| quarry-p1 | 4,096 | 0.291% / 1.215% / 3.255% | 22.69 |
| quarry-p1-sky | 4,096 | 0.293% / 1.221% / 3.385% | 17.09 |

Noise quantiles describe the absolute split-half luminance difference divided by signal,
on pixels above 0.01% of peak luminance. They are noisy per-pixel estimates, not confidence
intervals. All four images have zero invalid pixels. Render-call time includes resource
creation, AS construction, trace and readback; compaction adds approximately 3-5 seconds.
These are reference-production costs, not optimized renderer timings.

The prepared asset SHA256 is `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`;
the RLE2 is `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764`.
Each archive's full hash, source pose, sample count and deleted-raw hashes are in its JSON.
The static JSON initially omitted the black-background field. `black-primary-provenance.json`
and the annotated per-image JSON record a **retrospective source validation**, not a claimed
runtime readback: the implementation hard-coded `Some([0;3])`, and all L/D split halves in
the upper 64 wholly missed rows are exactly zero. No archive bytes were changed. No immutable
per-run source snapshot was saved for those older runs; later recipes retain build/input hashes.

Zero-shutter Q-walk frames 0, 1, 47, 48, 49, 56, 63 and 64 are retained at 1,024 spp, with
32 independent visibility rays per accepted primary sample. Thirty-eight exact-camera aliases
also have JSON controls, including hold frames 65-96, 80/81 and return 159/160. The rest of
TR0's original 60 scored frames (26 distinct cameras) are prioritized ahead
of the full sequence. The extra adjacent witnesses increase the prioritized distinct total
to 31. The finite-shutter rows are separate; they are not substituted for Unreal's zero shutter.

The 15:13 UTC game-time batch completed frame 64, then stopped cleanly when its completed
frame exceeded the 45-second contention threshold (52.41 seconds including compaction).
Whole process exit was 0; minimum available RAM was 3.117 GiB. Further quarry batches paused
at that point. These game-time costs are contended and excluded from speed claims.

## Unreal content and retention

All four benches have been imported into the retained E: project, using hidden NullRHI
commandlets. Each mesh's source triangle count and transformed bounds are checked. The
distance-field resolution scale is 1, with two-sided generation for the thin geometry.

- `/Game/M7U0b/SV/Bench`
- `/Game/M7U0b/S4/Bench`
- `/Game/M7U0b/S5/Bench`
- `/Game/M7U0b/S2G/Bench`

The new package is archived separately at **E:/renderlab-work/m7-u0b/ue-package**, preserving
**E:/renderlab-work/m7-u0/ue-package** and its original content identity for TR0's measurements.
The first cook was stopped by a CPU-test process misclassified as a renderer. The U0b wrapper
now distinguishes non-ignored library tests from ignored GPU tests by the live command line;
game and actual renderer guards remain active. Import also resumed through the installed
Interchange commandlet's known post-save Slate assertion; only completely saved mesh sets
were used, and final import inventories verify them.

Package identity: executable `69d6f88a...a6e5`, pak `c1fcded5...d122`, content utoc
`50686635...3189`; complete hashes are in `cook/b/identity.json`. Import inventories retain
every source/mesh triangle count, bounds error and distance-field build setting.
SV and S2G have SW/HW Epic content smokes with successful CVar and effective-tracing readback.
The native HDR Tonemap inputs decode with captured manual pre-exposure and unit tint/local
exposure. Full-frame SV and S2G PNGs were inspected.

The first S2G HW smoke inherited U0's quarry sun diameter (4.58366 degrees), rather than the
bench's registered 0.53285 degrees. It is marked a content smoke and **not scored**. The retained
runner restores the bench angle after the HRT preset, refreshes shadow proxies and checks the
actual final light-component readback. Every glossy capture uses D3D12 SM6 and
`r.EyeAdaptation.PreExposureOverride 0`.

### Quiet static timing checkpoint

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, native 1080p; original U0 quarry
package and presets. These 24 completed processes precede the 14:39 UTC game hold. Each
keeps all 64 CSV frames 96-159. The exclusive lease excludes other GPU jobs; the supported
wallpaper pause setting is restored in `finally`. Start GPU utilization was 1%. No clock lock
was applied. The registered clean-visit rule is p95/p50 <=1.15, with no game-hold overlap;
all process ranges remain reported, including visits outside that rule. No pooled percentiles.

| U0 configuration | Clean / total processes | Clean process p50 range ms | All process p50 range ms |
|---|---:|---|---|
| ue-0 | 3/3 | 1.667-1.718 | 1.667-1.718 |
| ue-tsr | 3/3 | 3.108-3.507 | 3.108-3.507 |
| ue-sw-h | 3/3 | 4.002-4.272 | 4.002-4.272 |
| ue-sw-e | 3/3 | 4.003-4.052 | 4.003-4.052 |
| ue-0-hrt | 2/3 | 1.764-1.764 | 1.764-1.879 |
| ue-tsr-hrt | 3/3 | 3.156-3.172 | 3.156-3.172 |
| ue-hw-h | 2/3 | 4.106-4.160 | 4.106-4.701 |
| ue-hw-e | 3/3 | 4.461-4.526 | 4.461-4.526 |

`quiet-summary.json` retains exact medians, p95 ranges, every visit and tracing-mode readback.
A fourth-process block stopped on a foreign CPU build helper misclassified as a renderer;
completed visits are retained, and the narrow helper exception is corrected. Remaining quiet
visits and moving workloads wait for the explicit game release. High/Epic differences should
not be inferred from overlapping short-visit ranges. No timing during play counts toward speed.

### S4 convergence and the S2G replay diagnostic

S4's inherited 4,096-spp interior reference has 5.7% / 15% median/p95 noise. Its approximately
64k run was deferred during the relaxed game hold because its estimated five-minute continuous
GPU load exceeded the permitted short screens. After the 15:36 UTC game release it is queued
behind the ordered quiet windows. It remains too noisy for a gate;
the sample count is not lowered to obtain a pass.

TR0's `gr-1p-64-a` S2G probe used the unmodified registered geometry, sky and sun, black
primary control and the 1-pixel camera track's frame 64. Its first 1,024-spp image was finite
(5.06 seconds), but an independent AS rebuild with the same seed failed bit identity. Process
exit 101, 11.65 seconds wall, minimum available RAM 4.37 GiB. This is a failed reference
test, not accepted score data. Instrumentation then localized the failure: fixed-AS repeats
are bit-identical, including when the shader module and pipeline are recreated. Independent
AS builds can differ by at most two f32 ULPs in secondary RGB; all 80-byte geometric records
(centre point/normal/identity, counts, D and V sums) remain byte-identical. Changing AS build
preference did not fix it and was discarded.

The retained fix recomputes barycentrics from the hardware-selected source triangle using
Moller-Trumbore arithmetic. Closest-hit traversal and the material model are retained. The
original 1,024-spp S2G replay now passes on mean RGB, both halves and every geometry byte.
The textured Mitsuba cross-check rerun also passes: +0.000443% bias, 0.107 sigma, 0.328% L1,
block z RMS 0.9595 and zero of 900 blocks beyond 4 sigma. The strict assertion is retained;
failed probes remain separate and explicitly unaccepted for scoring.

A second, independent precision error affected small sun disks: encoding only f32 cos(r)
lost **0.2108%** of irradiance at S2G's r=0.00465 (997.892 lux for 1,000 requested), and
rounded r=0.0001 to a delta. A failing CPU energy regression reproduced this before the fix.
The frame now also carries stable cone height `2*sin(r/2)^2`; sampling avoids subtracting
nearly equal numbers, the disk test uses cross-product sine squared, and radiance/PDF use
the same cone. GPU Lambert controls read **1.00000030** and **1.00000036** respectively,
with zero invalid pixels. At the quarry's r=0.04, the old encoding's irradiance error was
only **+0.000127%**, far below the retained split-half noise. Older quarry archives remain
the same physical target at that recorded precision; no archive is silently replaced.

The exact S2G fixture passes **eight independent AS-rebuild replays** on the final cone
encoding, with both HDR halves and every geometry byte identical. The qualified executable
SHA256 is `bf169ef7f8599bbff53bc8edcdb1875e8c117b09630e1c912e0f0f5d73335dfb`.
TR0 pins a copy for its own S2G campaign; this unit does not duplicate that campaign.
The qualified 1-pixel-track frame 64 archive includes its exact pose/light/FOV, black-primary
control, original and generated source-scene hashes, D/V/HDR halves and geometric populations.
Geometry export is opt-in
for generic benches via `RENDERLAB_M7_CAPTURE_GEOMETRY=1`; quarry exports always include it.
The diagnostic loaded-resource test also verifies explicit sum/geometry clearing between
repeats. Build recipes retain binary and source hashes. Final independent checks of the same
binary pass: textured X3 at 32,768 spp retains the figures above; finite-disk X2 at 16,384 spp
has **+0.005112%** bias (1.212 sigma), **0.4026%** L1 against Mitsuba seed-to-seed **0.7731%**,
block z RMS **0.8220**, and **0.0833%** of 3,600 blocks beyond 4 sigma. Both sheets were inspected.

TR0's first qualified S2G score against an older reflection/GI-off native capture shows a
large highlight deficit. On its fixed top-5%-full-reference masks, physical D/full is
0.998963 for metal and 0.998458 for dielectric, so omitted interreflection does not explain
native/full ratios 0.159/0.675. Those are combined material/lighting/reconstruction errors,
not an isolated TSR defect. A fresh sun-only control with actual angle, camera, light,
exposure and material/normal readbacks is planned before expanding that S2G comparison.

## Temporal reference interface

**Shared primary-background decision (Islam, 2026-11-07): black everywhere.** TR0's
`t1b-black-b` control applies black to primary misses **before** accumulation, with RLE2
still bound. Its audit preserves all 60 age masks, 161 per-frame counters and all 346,517
cold covered HDR pixels byte for byte, and verifies zero primary-miss RGB. Unreal already
clears black; the reference's black control therefore matches before reconstruction. No
post-compositing or sky mask is used. TR0 records the same choice in its own report.
**Visible-sky background/silhouette behaviour is untested under this protocol.**

The original T1b full-radiance clear was `[30,35,45]` times exposure, quantized to f16;
at -14 stops it is `[0.0018310546875,0.00213623046875,0.00274658203125]`. Its old full-image
scores cannot be compared to black-primary Unreal as reconstruction dominance. The original
sun-only/visibility programs already cleared black. The black-control data audit passes,
but its process hit a teardown timeout; it is not a clean-process or timing result. Physical
combined error also includes foreground material/lighting differences, and is not an isolated
reconstruction measurement. If this control had failed, the fallback was constant clear in
all three renderers; no coloured-clear Unreal maps were launched.

`scripts/m7-u0b-scenes.py` registers the exact Q-walk and Q-disocclusion sources. Zero shutter
is the Unreal comparison default; finite one-frame box shutters are separate rows. The
161-frame Q-walk includes every requested witness and adjacent frame. Identical cameras may
alias a lossless reference archive; the source frame and alias remain explicit in JSON.

The `.npz` interface retains actual full-radiance split halves, sun-only direct halves,
centre world point, geometric normal, placement/primitive/material identity, accepted-sample
counts, centre-conditioned disk visibility and the registered geometric population masks.
The visibility condition matches T1b's placement plus 2% forward-depth band. Physical sun-only
radiance integrates the disk; it is explicitly **not** the old delta-BRDF T1b estimand.

Reference units are unexposed cd/m^2. Native candidate HDR requires a measured pre-exposure
value; the score CLI accepts it explicitly and applies the frozen F0 Reinhard/sRGB transform
at -14 and -10 stops. TR0 identified the native Tonemap EyeAdaptationBuffer and colour-scale
readbacks; per-capture proof is being added. No fitted exposure or gain is used.

Raw per-frame buffers are compacted immediately into lossless reference archives, then
deleted with hashes retained in JSON. These archives are intentional retained references
for TR0, not disposable GPU dumps. Output is exclusively under E:/renderlab-work.
