# M7-U0b: textured quarry reference and Unreal follow-through

Implementation unit on main `708e87c`, branch `t3/m7-u0b`, 2026-11-07.
Contract: [M7](milestone-7.md), [design](m7-design.md), [U0 evidence and misses](m7-u0.md).

**Work in progress.** This checkpoint validates the reference inputs and imports the benches.
The high-sample quarry references, repeated quiet timings, native Unreal floor scores and
final package verification are still pending. No new 2x claim is made.

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

- Library tests: **124 passed, 118 ignored**, exit 0. The new CPU checks cover cube-CDF support
  and RGB-correlated uncertainty; Naga validates the complete measurement shader.
- GPU analytic run `reference/analytic-c`: all pass, zero invalid pixels. Delta-sun and
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

## Temporal reference interface

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
