# M7-U0b: textured quarry reference and Unreal follow-through

Implementation unit on main `708e87c`, branch `t3/m7-u0b`, 2026-11-07.
Contract: [M7](milestone-7.md), [design](m7-design.md), [U0 evidence and misses](m7-u0.md).

**Work in progress.** Textured/environment validation, four 4,096-spp quarry references,
the new packaged benches and the completed static/camera/Sun quiet timing campaign are retained.
The full **76/76 Unreal bench matrix** is retained; motion references and the expanded
physical floor table remain incomplete.
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

- Library tests: **125 passed, 120 ignored**, exit 0. The new CPU checks cover cube-CDF support
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

### Monitor failures and strict quarry replay

Two later sequence processes are **unaccepted**. `process-a584c6cd` exited 0 after three
images, but a WMI inventory timeout silently ended the inherited conflict-monitor thread.
`process-dcfe3367` completed two images and then hit its old five-minute guard, with another
monitor timeout. The separate RAM/disk watcher remained active: minima were **5.145 / 7.921
GiB**. Five completed images and the sixth partial output were removed from the accepted
directory; hashes, original metadata, PNGs and rejection reasons are under
`reference/quarry-final/rejected-monitor-20261107`. Neither process is a clean validation pass.

The runner reuses **TR0 `b9760bc`**: `GlobalMemoryStatusEx` replaces the OS-memory WMI query;
the PDH launch/live counter remains the 4/2-GiB guard; inventory/counter errors fail closed
and stop only the owned child. A Cargo-free resource valve waits outside the lock after a
resource/monitor failure. A separate quarter-second hold callback stops the owned process
when a game hold is posted. CPU fault injection verifies monitor timeouts reject a job,
native memory reads bypass WMI, failed memory counters return an abort value, and a lost
atomic acquisition retries without repeating an acquired job's body.

New quarry states run **one state per lease**, with a **600-second functional guard inside an
11-minute reservation**, releasing between states to alternate with TR0. Each state requires
an independent acceleration-structure/pipeline rebuild replay: all mean/half float bits,
all 80-byte geometric records, D/V sums and zero-invalid status must match before compaction
and publication. A failed replay keeps its data unaccepted; no tolerance is substituted.

G1b-q2 released at **21:31:17.941996Z**. The coordinated CPU gap has anchored START/END lines
at **21:31:18.432756Z / 21:38:34.717228Z**. The warm-target build took **435.5 seconds wall**
under shared CPU load rather than the estimated 45 seconds, and ran no GPU lease. Library
tests exit 0: **125 passed, 120 ignored**. The resulting harness SHA256 is
`4e5ed1a1d3b8a64aab8f027516c3638a7125438f1f78d6db687a74dd071fe5c7`;
renderer and shader hashes are unchanged from D8, so the independent material/transport
controls above remain applicable. D8 stays pinned for TR0's S2G corpus.

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

All 31 prioritized distinct zero-shutter Q-walk cameras are retained at 1,024 spp, with
32 independent visibility rays per accepted primary sample. They cover TR0's original 60
scored frames (26 distinct cameras) plus requested adjacent witnesses, including frame 56
for the frame-72 changed mask. Sixty-two exact-camera aliases have JSON controls, including
hold frames 65-96, 80/81 and return 159/160. `quarry-t1b-witnesses.json` records the selection;
the full sequence remains a separate campaign. Finite-shutter rows are separate and never
substituted for Unreal's zero shutter.

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
All **19 U0 configurations × four benches = 76 captures** pass the runtime settings,
effective tracing, registered camera/FOV, final source-angle and native HDR/exposure checks.
Each process exits 0. Actual shader platform is **PCD3D_SM6**, with D3D12 selected; all glossy
runs read `r.EyeAdaptation.PreExposureOverride 0`. Minimum available system RAM across the
matrix is **5.389 GiB**. Captured manual pre-exposure is 2^-11 for SV, 2^-12 for S4/S2G and
2^-14 for S5, with unit tint/local exposure.

`bench-captures.json` retains the 76 source/candidate hashes and per-run readbacks. Each
case retains a full-resolution common-F0 PNG decoded from the actual Tonemap HDR consumer.
The four 19-configuration matrix sheets were inspected, including silhouettes, contacts and
reflections. Software S5 shows incorrect repeated-room reflections; S4's software paths wash
out the enclosed indirect-light contrast while the hardware paths retain the red cast. These
are recorded image discrepancies, not import failures or quality passes. Functional capture
costs remain ineligible for speed claims, including the foreign-CPU-load period.

The first S2G HW smoke inherited U0's quarry sun diameter (4.58366 degrees), rather than the
bench's registered 0.53285 degrees. It is marked a content smoke and **not scored**. The retained
runner restores the bench angle after the HRT preset, refreshes shadow proxies and checks the
actual final light-component readback. Every glossy capture uses D3D12 SM6 and
`r.EyeAdaptation.PreExposureOverride 0`.

### Quiet timing baseline

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, native 1080p; original U0 quarry
package and U0 presets. **80 completed processes**: four static and three each camera/Sun
visits for all eight configurations. The first 24 static processes precede the game hold;
the remainder follow the 15:36 UTC release. Each keeps all 64 CSV frames 96-159. The
exclusive lease excludes other GPU jobs; the supported
wallpaper pause setting is restored in `finally`. Start GPU utilization was 1%. No clock lock
was applied. The registered clean-visit rule is p95/p50 <=1.15, with no game-hold overlap;
all process ranges remain reported, including visits outside that rule. No pooled percentiles.

| U0 configuration | Static clean / total | Static clean p50 range ms | Camera clean p50 range ms (3/3) | Sun clean p50 range ms (3/3) |
|---|---:|---|---|---|
| ue-0 | 4/4 | 1.648-1.718 | 1.673-1.689 | 2.094-2.131 |
| ue-tsr | 4/4 | 3.100-3.507 | 3.107-3.180 | 3.514-3.546 |
| ue-sw-h | 4/4 | 3.758-4.272 | 3.726-3.793 | 4.180-4.204 |
| ue-sw-e | 4/4 | 4.003-4.052 | 4.027-4.085 | 4.365-4.395 |
| ue-0-hrt | 3/4 | 1.749-1.764 | 1.784-1.788 | 1.761-1.767 |
| ue-tsr-hrt | 4/4 | 3.156-3.193 | 3.188-3.222 | 3.165-3.187 |
| ue-hw-h | 3/4 | 4.106-4.160 | 4.159-4.216 | 4.131-4.170 |
| ue-hw-e | 4/4 | 4.452-4.526 | 4.504-4.547 | 4.478-4.501 |

The two excluded static visits remain in the all-process ranges: ue-0-hrt extends to
1.879 ms and ue-hw-h to 4.701 ms. `quiet-summary.json` retains every median, p95 range,
paired Lumen increment and tracing-mode readback. Quiet snapshots live under each study's
`windows/<UTC>` so retries preserve earlier proof. High here means **Lumen High with U0's
TSR Epic / 200% history**, not TR0's separate TSR-High preset.

Clean paired Lumen increments over the same-shadow TSR baseline range, static / camera / Sun:
SW Epic **0.496-0.930 / 0.905-0.927 / 0.849-0.856 ms**; HW Epic
**1.259-1.354 / 1.293-1.343 / 1.295-1.336 ms**. These are differences of process medians,
not pooled percentile differences. The broad static software increment reflects baseline drift;
the complete ranges should be used in later speed comparisons. No clocks were locked.

A foreign CPU build helper initially stopped the fourth static block; the narrow helper
classification was corrected. The first camera attempt later exceeded Unreal's internal
16-KiB command line (16,687 characters), exited without a log/CSV and contributes no timing.
Short macro paths plus a prelaunch length guard fixed it; all successful visits are retained.
The final quiet release is **17:38:46.573113Z**, completion recorded at 17:39:03Z. No timing
during play counts toward speed, and no new 2x claim is made here.

### S4 convergence and the S2G replay diagnostic

S4's inherited 4,096-spp interior reference has 5.7% / 15% median/p95 noise. Its approximately
64k run was deferred during the relaxed game hold because its estimated five-minute continuous
GPU load exceeded the permitted short screens. After the 15:36 UTC game release it is queued
behind the ordered quiet windows. The monolithic 65,536-spp attempt then hit the RAM guard:
**13.295 GiB initially, 1.831 GiB minimum**, stopped after **372.75 seconds**, lease released
17:54:07Z. It produced no accepted S4 image. A bounded 4,096-spp batch completed in 30.24
render-call seconds, zero invalid pixels and 14.456 GiB minimum RAM, but the following seed
lost the GPU device; its replay control had passed. No Windows display-reset event was found,
so the device-loss cause is not diagnosed.

The retained production plan uses sixteen independent 4,096-spp processes with **one sample
per dispatch**, the quarry's existing granularity, for the same total 65,536 spp at 1080p.
All **sixteen batches completed**, including the previously failing seed, with exit 0 and
zero invalid pixels. Combined median/p95/p99 relative split-half noise is
**1.329% / 3.966% / 5.308%**, about four times lower than the 4,096-spp batch. Minimum
available RAM across the batch processes was **13.891 GiB**; their wall times sum to
479.10 seconds, excluding shared-slot waits. Actual
even/odd halves are sample-weighted in f64 and rounded once to f32; duplicate seeds and scene
or settings mismatches fail closed. The final reference and first batch PNGs were inspected:
the expected indirect red cast remains, with no direct sun patch and substantially lower noise.
This is convergence evidence, not a complete-image quality pass for a renderer. The retained
archive is `reference/s4-64k-d1/combined/S4-interior.npz`, SHA256
`be7077e465398b568df56ca67b204d8802f619358e5727a6287f306f4dc8090a`.

The S4 production harness adds explicit seed/dispatch controls only. Its executable is
`40bbee1cc887aaee35c8ed368b781f4080f4a15962cbb670a9916ba0c087daf1`; the reference, input
and shader source hashes are unchanged from the qualified `bf169ef7` binary retained for
TR0. That build passed **125/125**, with 119 ignored GPU tests. The two CPU scorer tests
verify zero error for matching physical images, correlated-noise cancellation on held cameras,
exposure/background rejection and independent-half combination with duplicate-seed rejection.

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

**S2G frame-zero regression and constant-colour fix.** TR0's later camera-0.25px frame-zero fixture
fails strict same-seed replay on BF169: 2,072 mean pixels differ by at most two ULPs;
all 80-byte geometry records, accepted counts and D/V records remain identical, with zero
invalid pixels. Its raw data is unaccepted. A new loaded-resource run also fails with AS and
pipeline held fixed, ruling out AS rebuild as the sole cause. The earlier eight-replay f64
qualification remains specific to that pose. An explicit frame-zero eight-independent-rebuild
regression is now added. Exit 101 was the intended strict replay assertion, not invalid radiance.

The old weighted colour interpolation can turn constant white into 0.99999994: its separately
rounded barycentric weights do not always sum to exactly one. Replacing only that expression
with `c0 + (c1-c0)*u + (c2-c0)*v` preserves a constant attribute exactly, without quantization,
model changes or a tolerance relaxation. The exact failed fixture now passes **8/8 independent
AS/pipeline rebuilds**, including both HDR halves and every geometry byte. This is a causal
shader-level control; the test did not record all secondary hits, so it does not identify which
hardware-selected hit first exposed the rounding difference or guarantee every future pose.

The first candidate run was interrupted by a foreign-process guard, with no shader verdict.
G1 identified that process as a CPU library-suite invocation, not a competing GPU run. The
exclusive retry passed and released at **18:45:53.245958Z**, within its 90-second reservation.
Binary SHA256 is `d8bd9a8e1e1c7b5d39ee04d70c01ed4ee87bb654463db2b40daec1a96f35f139`.
Its library checks pass **125 tests, 120 ignored**, and workspace fmt is clean. The reference
model is unchanged; older archives retain their original shader identity and are not rewritten.
`reference/s2g-f0-affine-colour-retry/s2g-frame-zero.npz` retains the qualified 1,024-spp
state, with all original source identities, D/V/HDR halves, populations and the eight-replay
report hash. Its full PNG was inspected; noise median/p95/p99 is **0.539% / 3.381% / 7.135%**.
TR0 can reuse it and resume its other states with strict per-state acceptance. The older BF169
f64 archive remains a separate, pose-specific qualification; no old archive is silently changed.

The D8 frame-64 qualification is also retained at
`reference/colour-controls/S2G-f64-control-camera-1px-064.npz`: 1,024 spp, actual D/V/HDR
halves and geometry, **8/8** strict independent AS/pipeline replays. Its relative noise
median/p95/p99 is **0.384% / 2.475% / 6.200%**. The deleted scratch input was reconstructed
byte for byte from the registered scene and retained pose; its SHA256 matches the old input,
`4ae9076712f54637c7346e9485dfc0c4bf2570019d29730d7436d036cf0f1097`.
The archive metadata retains open/close cameras, light direction, FOV, primary background,
original/generated source hashes, binary identity and the eight-replay proof hash. Relative
to the BF169 f64 archive, the registered -12 display RMS change is **1.22e-6 F0 codes**,
maximum **0.00003052 codes**. TR0 owns the remaining D8 physical camera campaign.

Post-fix independent controls pass against the registered Mitsuba arrays. X3 at 32,768 spp
has bias **+0.0004431% (0.1074 sigma)**, L1 **0.32794%**, block z RMS **0.95952**, and
**0/900** blocks beyond 4 sigma. X2 at 16,384 spp has bias **+0.005099% (1.2058 sigma)**,
L1 **0.40243%**, block z RMS **0.83087**, and **0.1667%** of 3,600 blocks beyond 4 sigma.
Both full comparison sheets were inspected. A mistaken comparison against the rejected
V-flipped diagnostic X3 exporter failed and was excluded; `comparison-identities.json` and
`x3-registered-compare.log` pin the accepted independent inputs. Geometry export consumes
visibility RNG draws, so the new geometry-enabled X2 and older image-only control use different
sample streams; their bit difference is not attributed solely to colour interpolation.

The later harness binary `a7e168af334cb70be176d8d6f0ad909d874dbb877580383ad7928f1fee4b47f2`
adds only the `RENDERLAB_M7_MAX_DEPTH` test option. Its renderer and shader hashes match D8.
It reruns `cargo test -p renderlab --lib` successfully: **125 passed, 120 ignored**.

TR0's first qualified S2G score against an older reflection/GI-off native capture shows a
large highlight deficit. On its fixed top-5%-full-reference masks, physical D/full is
0.998963 for metal and 0.998458 for dielectric, so omitted interreflection does not explain
native/full ratios 0.159/0.675. Those are combined material/lighting/reconstruction errors,
not an isolated TSR defect. Fresh sun-only AA0/TSR controls verify the actual angle, camera,
light and exposure. Source-interpolated authored normals agree with unjittered GBuffer normals
to less than 0.090 degrees; metallic, roughness, specular and decoded base colours agree within
their storage quantization. No gross material or normal import mismatch is found.

In `gc-sun-tsr-a/rejection-readback.json`, resource versions and hashes identify the actual
RejectShading SceneColor input and Tonemap's TSR.Output consumer. Metal/dielectric highlight
energy ratios against physical D fall from **0.835607/0.910870** at that input to
**0.117872/0.799582** at TSR output. Whole-material ratios also fall,
**0.831121/0.914201** to **0.123069/0.836081**, excluding redistribution outside the highlight
mask as the whole explanation. The AA0 control retains **0.90225/0.90883**. This localizes the
large additional loss within the observed TSR interval; it does not identify its internal
cause. TR0's four-stop-lower exposure control measures pre-exposure **2^-16** exactly and
unexposes before scoring at the registered -12 stops. The RejectShading ratios remain
**0.835607/0.910870**; TSR output improves to **0.286557/0.821605**, with whole-material ratios
**0.289562/0.855207**. This demonstrates exposure dependence but leaves most metal energy
missing. `gc-sun-tsr-m4-a/colour-control-readback.json` records the control. No gain is fitted,
the registered -12 bar is unchanged, and no additional S2G reference campaign is duplicated here.
The stronger `pre-input-identity.json` audit multiplies the lower-exposure RejectShading f16
RGB by exactly 16, leaves alpha unchanged and reconstructs the entire original input byte
for byte (SHA256 `2d5862f030696277a17ceb1910f5dba269b5d1f3fe2cd5e256903dae86cd759e`).
The exposure control therefore preserves the physical input beyond agreement of masked means.

## Physical quarry floor

The eight final static captures retain the actual Tonemap HDR consumer, not a display PNG.
Camera readback matches p0, manual pre-exposure is **2^-14** on every capture, local exposure
is 1 and the colour scale is unit RGB. The native decoder follows resource versions and checks
the archive hash. AA0 consumes `PostDOFTranslucency.SceneColor`; these U0 TSR presets consume
`TSR.Output`. Formats and full resource/exposure proofs remain in each `native.json`.

The following are raw RMS **F0 codes at -14 / -10 stops**, against the 4,096-spp full-radiance
p0 reference. All registered geometric populations are retained; masks are defined from the
reference, never selected from candidate error. Covered-reference split-half display noise is
**0.209/0.302 codes**, edge noise **0.371/0.818**. Noise-subtracted RMS, p99, maximum, signed
mean, population size, archive hashes and exact per-capture settings remain in
`study/quiet-s/physical-static-floor.json` and `physical-floor-summary.json`.

| Population | SW High | SW Epic | HW High | HW Epic |
|---|---:|---:|---:|---:|
| all-covered | 9.949/15.039 | 9.318/13.990 | 9.182/13.081 | 8.411/11.649 |
| background-centre | 0.400/1.173 | 0.333/1.028 | 0.399/1.171 | 0.326/1.010 |
| backlit | 13.670/24.908 | 11.401/20.620 | 12.362/22.061 | 9.931/17.075 |
| contact | 15.626/26.854 | 14.018/23.592 | 15.146/25.712 | 13.425/22.178 |
| edge | 14.281/27.020 | 13.835/25.549 | 12.144/22.740 | 11.285/20.201 |
| faint | 11.028/14.918 | 10.400/13.661 | 10.723/13.923 | 9.902/12.278 |
| image | 4.084/6.240 | 3.821/5.796 | 3.771/5.453 | 3.451/4.851 |
| isolated | 8.713/10.406 | 8.123/9.419 | 8.636/10.215 | 8.076/9.356 |
| lit | 8.063/10.595 | 7.487/9.780 | 7.948/10.191 | 7.385/9.395 |
| multi-surface | 14.729/24.959 | 14.241/23.673 | 12.737/20.760 | 11.947/18.546 |
| narrow | 13.527/21.324 | 12.664/18.878 | 13.107/20.218 | 12.086/17.386 |
| occluded | 23.011/43.379 | 22.382/41.754 | 16.326/30.656 | 14.528/26.402 |
| partial | 12.153/18.164 | 11.143/16.148 | 11.774/17.055 | 10.592/14.659 |
| silhouette | 21.319/41.567 | 20.765/40.416 | 14.582/29.567 | 13.213/26.812 |
| single-surface | 9.824/14.759 | 9.186/13.713 | 9.093/12.870 | 8.320/11.460 |
| solid | 9.473/14.370 | 8.930/13.542 | 8.603/12.181 | 7.929/10.985 |
| solid-lit | 8.006/10.469 | 7.432/9.661 | 7.909/10.108 | 7.351/9.328 |
| solid-occluded | 22.828/42.991 | 22.190/41.369 | 16.070/30.098 | 14.246/25.842 |
| wide | 11.902/17.566 | 10.861/15.633 | 11.530/16.453 | 10.315/14.140 |
| wide6 | 11.351/16.467 | 10.247/14.608 | 11.140/15.707 | 9.926/13.639 |

The corresponding covered-pixel AA0 and TSR-only VSM baselines are **11.986/16.504** and
**10.615/15.205**; HRT baselines agree within 0.002 codes. These are physical combined
foreground/material/lighting/reconstruction errors. They do not isolate a Lumen or TSR defect.
Hardware Epic is best on these static populations, but no Renderlab quality pass is implied.
Full-frame reference/native/8x-difference sheets for both Epic paths were inspected, including
edges, contact shadows and the black background. Bright contact/occlusion discrepancies and
fine texture differences remain visible. Black-centre error includes mixed-edge pixels;
visible-sky silhouette behaviour is outside this protocol.

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

## G1's SV sky-visibility comparator

The request in main's [G1 report](m7-g1.md) is a separate diffuse-only sky-visibility
estimand. `/Game/M7U0b/SVSky/Bench` reuses SV's registered source meshes with component material
overrides: base colour 0.5, roughness 0.8, metallic/specular 0. It leaves the original SV and
S2G materials and `m7-u0b/ue-package` intact. Its separate package is
`E:/renderlab-work/m7-u0b/ue-svsky-package`; executable SHA256 remains
`69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5`, with new content
identities in `cook/sv-sky/identity.json`.

An unlit, nonshadowing primary sky sphere displays constant 4,000 cd/m² before TSR, matching
G1's visible-sky reference. It is excluded from ray tracing, distance fields, collision and
dynamic indirect lighting; illumination comes from the specified constant sky cube. This
explicit SV control does not change the shared black-primary quarry/S2G protocol. No sky
mask or post-compositing is used. A native background sample reads 4,000/4,000/3,968 cd/m²,
consistent with its R11G11B10 storage.

All runs use D3D12 SM6, `r.EyeAdaptation.PreExposureOverride 0`, actual manual pre-exposure
**2^-11**, `r.LumenScene.Radiosity 0`, reflection method 0, motion blur off and TSR Epic.
Screen traces retain Epic defaults. Quality captures fix the timestep at 1/60 s and set
`r.OneFrameThreadLag 0`; cost processes retain the default render-thread lag. The separate
async-off attribution processes are labelled. No SV speed claim is made from these functional
slots. CSV values for each GPU group are reported separately rather than summing inclusive
groups or treating async gather's CSV scope as the complete Lumen cost.

The original G1 reference caches were deleted at G1 close. Ten references regenerate its
registered estimator: both poses plus nine gaps-pan cameras, **2,048 spp, max depth 2,
specular 0, constant sky 4,000**. They are not claimed byte-identical to those deleted caches.
The pan uses the same float32 2 cm source-X translations. Static and first-cut captures retain
all geometric feature/near-ground populations, with 3×3 placement/face-normal edges separate.
The metric is G1's linear-luminance relative error, floored at 5% of open-ground p90; pixels
below 1e-6 or above 2% reference SE are excluded. Motion uses the final pan's covered nonedge
mask and frame-to-frame residual changes. Reference/candidate hashes and noise remain in JSON.

`sv-sky/a` validates both initial poses on the inherited software and hardware presets.
Actual RejectShading parameters prove the three captured cut frames have `bCameraCut`
**0/1/0**; all twelve camera readbacks and exposure records pass per process. GBufferB material
audits verify metallic/specular **0/0** and median roughness **0.8** on more than 1.3 million
interior covered pixels per pose. The inherited software preset reads `r.Lumen.TraceMeshSDFs 0`
and traces global distance-field voxels. A separate mesh-SDF, runtime-RT-disabled row is
required for the non-RT comparator; inherited global-DF results remain supplementary.

The prepare commandlet saved the level and assets successfully, then exited 1 on a
CachedAssetRegistryDiscovery temporary-file rename. Its failed process record is retained.
The subsequent cook/package and actual native content, camera and material readbacks pass;
the prepare process itself is not relabelled a clean pass.

Matched SV results (G1 metric; values are relative-luminance RMS, not F0 codes):

| Mode | Static overview/gaps | First-cut incoming overview/gaps | Mean pan residual |
|---|---:|---:|---:|
| SW Epic, inherited global DF | 35.917%/68.312% | 44.510%/81.895% | 140.460% |
| SW Epic, mesh SDF, runtime RT off | 38.911%/87.185% | 49.558%/101.047% | 166.166% |
| HW Epic | 10.708%/11.164% | 10.595%/9.506% | 20.182% |

The mean pan residual uses the overview-start process, which warms at the gaps pose before
the registered nine-frame pan. G1 resets its cache before its pan; these are the native
shipping Lumen/TSR histories after the stated warm-up, not a forced reset of Lumen world
state. The cut clears TSR history via the observed camera-cut flag while retaining Lumen's
normal world state. No reconstruction-only or integrated G1 acceptance claim is implied.

| Static population | Global-DF SW | Mesh-SDF non-RT SW | HW |
|---|---:|---:|---:|
| contact | 22.851%/? | 32.931%/? | 14.397%/? |
| crease-90 | 27.355%/41.492% | 33.891%/61.068% | 18.514%/5.401% |
| edges | 52.964%/73.652% | 61.132%/91.938% | 26.853%/18.391% |
| frame | 35.917%/68.312% | 38.911%/87.185% | 10.708%/11.164% |
| gaps | 84.660%/135.646% | 111.140%/175.938% | 30.524%/20.806% |
| ground-near-contact | 4.854%/3.128% | 5.077%/6.017% | 4.660%/4.840% |
| ground-near-crease-90 | 8.426%/22.461% | 10.001%/29.633% | 6.084%/7.425% |
| ground-near-gaps | 22.175%/23.551% | 25.132%/29.739% | 6.189%/5.669% |
| ground-near-underside | 63.010%/78.017% | 50.475%/69.622% | 10.776%/9.805% |
| ground-near-wedge-30 | 89.845%/4.222% | 93.406%/6.931% | 17.142%/6.243% |
| open | 33.638%/27.337% | 34.139%/30.424% | 7.953%/6.267% |
| underside | 31.929%/77.596% | 34.377%/88.985% | 15.713%/11.065% |
| wedge-30 | 33.575%/? | 45.768%/? | 24.829%/? |

Population pairs are overview/gaps; a dash means no valid visible samples in that pose.
Per-population counts, noise exclusions, p99, signed bias, crevice error and over-occluded
open counts are retained in `sv-sky/{a,b}/*/g1-scores.json`. Full native/reference/difference
sheets for both paths were inspected. The especially large software errors occur in narrow
gaps and undersides; they remain in the floor rather than being removed as outliers.

The Lumen/DFAO-off calibration intentionally removes occlusion. Across 1,118,310 / 1,092,197
nonedge ground pixels, the analytic Lambert value is 0.5 ? 4,000 = **2,000 cd/m?**. Native
mean RGB is about **2,011/2,011/2,009**, with **0.706% RGB RMS** relative to that analytic
value in both poses. This bounds the input-scale discrepancy; no fitted gain is applied to
the scored Lumen images. Comparing this control to the physically occluded reference would
measure deliberately missing occlusion, not a calibration error.

Separate uninstrumented cost processes retain **r.OneFrameThreadLag 1**, verified from
their timed CVar dump. The following attribution processes disable both RDG and Lumen async
compute, also read back at frame 95. Values are milliseconds. They are functional exclusive
slots, not quiet-window speed evidence. The async-on CSV gather scope is only 0.015?0.142 ms
here and omits the concurrent gather work; it must not be used as the full Lumen cost.

| Path / initial pose | Gather static | Gather first-cut | Gather pan mean/range | Warm GI tree total |
|---|---:|---:|---:|---:|
| sv-sky-hw / gaps | 1.0149 | 1.0138 | 1.0154 / 1.0086?1.0259 | 1.3930 |
| sv-sky-hw / overview | 0.8971 | 0.8672 | 1.0188 / 1.0086?1.0332 | 1.4540 |
| sv-sky-sw-mesh / gaps | 1.1838 | 1.4952 | 1.2571 / 1.1704?1.6086 | 2.4970 |
| sv-sky-sw-mesh / overview | 1.0321 | 1.0261 | 1.2229 / 1.1785?1.4736 | 1.4180 |

The warm GI total sums only outermost matched nodes in the separate frame-64 ProfileGPU
tree. It is not the first-cut cost. Frame-96 static, frame-97 cut and all nine pan frames
retain individual scene-buffer/update/lighting/gather/TSR CSV scopes in
`sv-sky/timing-async-off/*/g1-costs.json`; no inclusive groups are blindly added. Every selected
GPU CSV row has the registered camera position within **0.001 cm**. The ordinary `GPUTime`
counter can lag/repeat across adjacent frames and is not used as an exact cut-frame duration.
Default async-on totals and component fields remain separately in `sv-sky/timing`; the
instrumented quality-process costs remain under `sv-sky/{a,b}`.

**Foreign CPU-load exclusion.** Root identified stillkit's independent nightly builds. The
oldest still-running Cargo in the read-only snapshot began **20:31:39.041Z**. Both hardware
async-off attribution process intervals overlap it (the overview process spans the onset),
so those two rows above are **contended and ineligible for cost gates or timing claims**.
`foreign-cpu-control.json` records the intervals; replacements are queued after 60 observed
seconds without stillkit Cargo/rustc. No foreign process is paused or killed. The earlier
quarry quiet campaign ended before this load and is unaffected. Functional references and
captures retain strict validation, longer process guards and the normal memory abort guard;
near the start threshold the controller waits outside the GPU lock until the foreign build
is gone for 60 seconds.
