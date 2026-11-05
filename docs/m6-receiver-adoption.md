# Production receiver policy

2026-11-05, base `767c28c`, branch `t3/receiver-shadow-policy`. Implements Islam's
approved [Q2 policy](m6-film-gap.md#6-receiver-policy) and promotes the
[R1 viewed experiment](m6-receiver-policy.md) into production.

**Implementation is complete; acceptance has two explicit misses.** Every R1
fixture passes, all 36 fixture/quarry film and direct buffers reproduce R1 byte
for byte, Level B passes, and the final-source 32-repeat test passes. Frozen
Level A G2 fails at one receiver, and complete frame cost regresses beyond
noise. Neither threshold was relaxed. This is not film-quality or matched
Unreal acceptance, and does not authorize F1-AA.

## Contract and API

- `MaterialDescriptor::thin: bool`, default **false**, is independent of
  `double_sided`. Programs can read `input.material.thin` (u32, 0/1). The 48-byte
  material block reuses padding; no lighting model enters hit records or dispatch.
- Receivers start on the hit triangle. The offset is geometric, on the viewed
  side for opaque materials, and toward the sampled light direction for thin
  materials: `max(10 um, 16 ulp(max abs world triangle vertex coordinate))`.
  Ray-distance `t_min` is that same positive epsilon. Normal maps and program
  shading do not move it. The 16-byte hit record remains unchanged; thinness is
  bit 29 in the per-placement material table.
- Traced reconstruction pins R1's accepted order explicitly:
  `fma(c, w.z, fma(b, w.y, a*w.x))`, before the instance transform. The program's
  existing interpolation order is preserved separately. Camera-ray FMA remains
  as established in [determinism](determinism.md).
- Raster uses the winning fragment's interpolated triangle point (per sample at
  4x), and native per-vertex fragment inputs for the actual three world vertices.
  **Callers must enable `renderlab::RECEIVER_FEATURES`**, currently
  `wgpu::Features::SHADER_PER_VERTEX`. This is a Vulkan feature requirement;
  unsupported devices are rejected before renderer allocation. Repository
  callers negotiate it. No geometry copy, mesh-wide epsilon approximation or
  additional per-frame binding is introduced.
- Hard, Exact, Interactive, raster receivers, traced receivers, inline hardware
  shadows and the independent fragment oracle use the policy. Thin disk rays
  choose their side separately for each direction. Positive epsilon allows
  unconditional first-hit termination and checking hit presence alone.
- Legacy `Shadows::{normal_bias, depth_bias}` remain accepted, validated and
  serialized for compatibility; they no longer change visibility. Their old
  values are still available to the frozen old-policy timing/control modules.

Removing the large bias exposed forward-Z shadow-map quantization: a perspective
map with a 1 mm near plane cannot reliably resolve the receiver floor metres
away. Raster maps now use reversed depth, clear 0 and Greater comparison. The
depth row is constructed in f64 before f32 rounding, retaining the planner's
XY/W rows. Hardware ray records explicitly keep their original forward light
volume. The existing receiver-plane texel/subpixel correction stays, with its
reversed-depth sign. No larger receiver offset was substituted.

`renderer.rs` changes are limited to device-feature admission, material thinness
and the shadow depth comparison; pipeline caching/compilation remains the other
worker's unit.

## References and fixtures

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan, release
library test binaries, two Cargo jobs. Quarry: twelve full-resident draws,
13,034,903 triangles, 1920x1080, 1x, 16x anisotropy, RLE2 environment, -14-stop
exposure and a 0.04-rad disk. Pose 1 is +1 cm X; pose 2 independently repeats
pose 0. Correctness runs have active Core and Synchronization validation and
zero validation errors. Timings disable validation.

The point reference is regenerated from **64 independently salted 256-ray
estimates (16,384 directions)**, plus production seeds 0-3 and spread seeds
65-80. Film references use all 256 independently randomized 16x16 footprint
strata, each with 256 independently salted shadow rays; full and sun-only
radiance and the first 16 subsamples are retained.

R1 equivalence fixes its original centre surface-acceptance anchor for the 2%
depth test. Reconstructing the production centre on its triangle otherwise
changes a few boundary memberships, independently of shadow queries. Production
centre receivers are captured separately; **all shadow rays still use production
origins**, and R_L/R_D always include every primary sample. Metadata names the
anchor. With matched acceptance, all 30 fixture film/direct buffers and all six
quarry buffers are byte-identical to R1. The equivalence test forbids arithmetic
edits or frozen modules. Its fallback radiance bound is R1's documented
1.7881393e-7, with identical visibility/acceptance sums; no fallback was needed.

| Production fixture | Partial pixels | RMS | sigma_F RMS | Result |
|---|---:|---:|---:|---|
| Sawtooth | 13,632 | 0.000267 | 0.022610 | Pass |
| Thin/contact | 933 | 0.000408 | 0.004714 | Pass |
| Stress | 2,296 | 0.000310 | 0.001394 | Pass |

All **12 distinct acne cases** pass: flat/convex, 1/10 km, 80/85/89 degrees,
35,328 covered pixels each, zero false shadows. R1's two historical policy
variants duplicated these twelve geometries; production has one adopted policy.
Thin accepted counts match; stress retains the documented sixteen one-sample
analytic-boundary mismatches, without discarding them. The separate material
test passes all twelve combinations of raster/traced, hard/Exact/Interactive and
opaque/thin: grazing backlighting gives visibility 0 for opaque and 1 for thin.
The raster Interactive preference uses its documented Exact fallback.

## Frozen gates, old and new

Old results are reused from the accepted primary/Level-B checkpoint; those bytes
were retained through main `767c28c`. New gates use the regenerated point
reference and the unchanged gate code. Populations and native Unreal bounds are
recomputed as the frozen rules require.

| Check | Old | New |
|---|---|---|
| Level A G1-G6 numerical | Pass | **G2 fails**, other checks pass |
| Level A q, poses 0/1 | 0.018169 / 0.017986 | 0.018092 / 0.017944 |
| Level B composite, 3 poses x 4 seeds | Pass | Pass |
| B maximum partial RMS | 0.081751 | 0.080903 |
| B maximum faint RMS | 0.055783 | 0.054770 |
| B maximum contact RMS | 0.043088 | 0.042651 |
| B maximum motion residual RMS | 0.099844 | 0.099625 |
| B maximum contour rho | 1.266903 | 1.267512 |
| B solid flips / lost contacts | 0 / 0 | 0 / 0 |
| S5 repeat and return | Byte-identical | Byte-identical |

G2 fails only at **(1295,513), seed 0, poses 0 and 2**: the reference is 1 and
Exact is 255/256, giving 0.00390625 against the unchanged 0.002 solid bound.
The viewed geometric N dot L is 0.03161, inside the 0.04-rad grazing disk.
This is a sampled one-ray occlusion versus the finite reference; an independent
geometric classification of that ray is not claimed. It remains a failed gate.

The final-source acceptance repeats each of four Interactive seeds **32 times
at each of three poses: 384 repeated frames with identical HDR and counts**, with exact return,
the regenerated Exact counts, invalid reload preservation, tint/revert, program
reload and raster fallback passing. This is one final-source process, not 32
process launches. The earlier retained-policy repeat process also passed.

## Film results

Each column uses its own policy's film reference. Partial populations change:
46,514 -> 49,928 pixels at pose 0 and 46,423 -> 49,889 at pose 1. These numbers
therefore do not establish a policy-independent perceptual improvement.

| Pose | Mode | R_F partial RMS old -> new | Missed shadows old -> new | False shadows old -> new |
|---|---|---|---|---|
| 0 | Hard | 0.266478 -> 0.253246 | 13,128 -> 13,362 | 9 -> 5 |
| 0 | Exact | 0.231586 -> 0.203344 | 9,199 -> 8,570 | 9 -> 15 |
| 0 | Interactive | 0.234188 -> 0.207204 | 9,768 -> 9,245 | 9 -> 27 |
| 1 | Hard | 0.267065 -> 0.254196 | 13,139 -> 13,276 | 9 -> 7 |
| 1 | Exact | 0.232030 -> 0.204690 | 9,118 -> 8,484 | 11 -> 18 |
| 1 | Interactive | 0.234588 -> 0.208537 | 9,682 -> 9,166 | 11 -> 26 |

Missed/false definitions remain R_F<=0.9 with output>=0.99, and R_F>=0.99 with
output<=0.9. These are static endpoints, not temporal film-defect acceptance.

| Pose | Mode | Whole-image R_L RMS codes, registered, old -> new | Whole-image R_D RMS codes, +4 stops, old -> new |
|---|---|---|---|
| 0 | Hard | 2.581610 -> 2.581711 | 1.537099 -> 1.554133 |
| 0 | Exact | 2.581593 -> 2.581746 | 1.418587 -> 1.415564 |
| 0 | Interactive | 2.581601 -> 2.581751 | 1.427449 -> 1.425657 |
| 1 | Hard | 2.625282 -> 2.625394 | 1.528156 -> 1.552403 |
| 1 | Exact | 2.625275 -> 2.625478 | 1.408939 -> 1.415159 |
| 1 | Interactive | 2.625263 -> 2.625484 | 1.416523 -> 1.425422 |

Pose 2 reproduces pose 0. CSV/JSON retain all populations, linear HDR and both
exposures, including missing/false counts, p99 and maximum error. The complete
old/new centre Exact HDR images are byte-identical to their respective frozen
F0/new-reference centre captures. Old F0 R_L is reused; old R_D was regenerated
with the frozen `767c28c` modules at poses 0/1, with the static pose-0 direct
reference reused for old pose 2. Old-control visibility/acceptance sums equal
F0; the reordered 256-sample R_L summation's small floating differences are
recorded separately. R_L/R_D are integrals of this renderer's shading, not GI or
a physical path-traced reference. Whole-image aliasing remains substantial.

## Paired cost: failed non-regression gate

Three clean processes, each 24 warm rotations and **48 pairs per mode**, rotate
old/new order and mode in one renderer on the same ABI. Frozen old kernel and
shading modules and production modules run the complete frame. No foreign
build, game, Unreal or renderer job is present in the clean timing preflights;
minimum observed available RAM is 10,400 MiB. Bootstrap intervals are paired
median-ratio diagnostics, not an engine-wide performance guarantee.

Representative fast-state process, milliseconds:

| Mode | Old span | New span | Paired increase across 3 processes |
|---|---:|---:|---|
| Hard | 0.845328 | 0.880096 | **4.1-4.6%** |
| Interactive | 0.816960 | 0.863456 | **5.4-5.8%** |
| Exact | 8.336320 | 9.187872 | **10.1-10.8%** |

Every paired 95% interval excludes 1. Soft shading stays effectively unchanged;
visibility accounts for the increase. Unconditional first-hit plus hit-presence
queries preserve R1 exactly but do not recover the loss. A diagnostic using the
new dispatch with the old origin/side/t_min rules gives old/control medians of
0.847776/0.848048 hard, 0.826560/0.827824 Interactive and 9.941104/10.018496 Exact.
That control also lets the compiler eliminate unused epsilon work; it does not
prove which traversal/register mechanism dominates. It is never a quality
candidate. The cost gate remains failed rather than restoring the old bias.

## Inspection, verification and reproduction

Inspected complete native-resolution quarry images at both poses: Exact,
Interactive, hard, film R_L, sun-only R_D at registered/+4 exposure; visibility
and signed errors; seven frozen contact/silhouette probes and the G2 location.
Also inspected all acne panels and sawtooth/thin/stress oracle sheets. Silhouettes,
slab/wedge contacts, rubble, cracks and thin features remain complete. The film
image integrates the rough-face detail; point modes retain fine aliasing/grain.
The stronger micro-shadow structure is clearest in sun-only crops. No continuous
camera, blocker or lighting sequence, nor transparency acceptance, is claimed.

- 83 CPU library tests pass; affected library/tests release Clippy with
  `-D warnings`, renderer/viewer all-target release checks, formatting and diff
  whitespace checks pass.
- Five hardware suites pass, including 1x/4x sample receivers, mirrored/sided
  geometry, normal-map origins, queued replacement and release.
- All four software shadow-map suites pass after the depth correction, including
  local seams, self-shadowing, offscreen blockers, motion and virtual geometry.
- Full corpus and native masks stay local. Bulk evidence is
  `E:/renderlab-work/receiver-adoption/`: `primary/`, `replay/`, `static/`,
  `quarry-viewed-p{0,1,2}/`, `centres/`, `level-b/`, `images/`, clean
  `paired-final-{1,2,3}/`, `runs/`, `provenance.json` and `identities.sha256`.
  No private asset or generated capture is committed.
- `scripts/m6-receiver-adoption-run.py` acquires the fixed D: lock by atomic
  mkdir and token-bearing owner.json, preflights every run and records UTC
  acquisition/release times. Each lease is below 45 minutes and every owned
  lease is released. All processes are hidden/offscreen; no desktop input or
  foreground capture was used.
- Initial centre/repeat attempts aborted under the strict runtime 6 GiB RAM
  guard and are excluded. The final repeat preflight had 8,585 MiB; its bounded
  correctness run allowed owned allocation transients and logged a 4,793 MiB
  minimum. The requested **preflight** 6 GiB rule and continuous game/foreign-GPU
  guards remain enforced. Clean timings retain the strict runtime RAM guard.

Reproduction entry points are `m6-receiver-adoption-collect.py` (fixtures,
quarry, primary, replay, static) and `m6-receiver-adoption-score.py` (gates,
film, timing). Their ignored-test manifests and exact commands are in `runs/`.
Reference acceptance uses production policy code; frozen modules/arithmetic edits
are restricted to named diagnostic controls.

Implementation and diagnosis began about 08:45 UTC, interleaved with verification;
editing time was not separately stopwatched. Build/check wall times, correctness
processes, timing processes and every lease are recorded separately in the
provenance/ledger. These wall times include compilation, readback and validation
where applicable; they are not GPU execution time.

Recorded totals: 16 guarded builds/CPU checks **542.6 s**; 60 GPU correctness,
timing and diagnostic process windows **1,241.1 s**; all 76 owned leases
**1,784.9 s**, longest **118.2 s**. Leases span
**08:51:20.027117-10:13:19.165155 UTC**. The ledger retains every individual
acquisition/release timestamp, including failed/aborted experiments. Only the
three named clean final timing processes support the cost comparison.
