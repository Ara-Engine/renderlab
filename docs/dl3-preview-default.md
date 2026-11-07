# DL3: development preview default

Base **156405d**, branch **t3/dl3-preview-default**, 2026-11-07.

The development viewer now selects helper-boundary preview compilation automatically
when the Vulkan interop device is available. `--shader-preview optimized` selects the
ordinary device/reload path. Unsupported interop returns to ordinary compilation;
device initialization errors are logged before attempting that fallback. Library
`ShaderWatch` defaults and release viewer defaults remain ordinary optimized
compilation. Preview remains transient, visibly marked, and replaced in the background.

## Call-boundary screen

The ignored `gpu_vulkan_helper_boundaries` test compiles fresh, unique standard
program sources on raster and traced-compute pipelines. It changes only SPIR-V
`DontInline` hints through the program compiler. No WGSL helper/shading body is edited.
Each candidate has two repeats; pipeline caches are disabled for this diagnostic.
These native-call screens isolate compiler work and are **not edit-to-image timings**.
Only a retained candidate proceeds to the full five-repeat M5-DL method.

Initial driver medians in ms (two repeats):

| Boundary choice | Graphics | Traced compute |
|---|---:|---:|
| DL2 environment set | 259.653 | 351.197 |
| + `shade` | 256.819 | 348.543 |
| + `surface` | 258.855 | 367.515 |
| + `brdf` | 249.689 | 326.946 |
| + `material_normal` | 248.466 | 369.063 |
| + `light_sample` | 270.025 | 385.865 |
| + `exposed_product` | 226.803 | 327.634 |
| + `hard_shadow` | 245.974 | 365.441 |
| + `unit_normal` | 232.545 | 335.978 |
| + `surface`, `shade` | 282.212 | 349.050 |
| + `surface`, `shade`, `material_normal`, `brdf` | 271.962 | 345.880 |
| Only `environment_bilinear` | 903.241 | 1,144.090 |
| Only `environment_radiance` | 8,389.583 | 8,523.373 |
| Only bilinear + radiance | 927.818 | 1,124.816 |
| Only `environment_tap` | 504.576 | 605.971 |
| All non-entry functions | 312.710 | 449.623 |

The reduced-entry idea is tested by retaining `surface` and `shade` call boundaries:
their existing generated entry calls remain small instead of inlining those bodies.
It supplies no material win. Retaining only a top environment function makes its
remaining expanded body dramatically more expensive. The promising individual
additions are small relative to variance, so their combinations require another screen
before adoption. No speed claim is made from these two-repeat observations.

The screen completed before the game started. Its resource samples recorded foreign
compiler activity in 8/15 samples. Evidence is **E:/renderlab-work/dl3/boundary-a/**;
the source hashes, native/API/module durations and shader sizes are recorded per call.

A CPU-only call-site replay explains why naming a small outer function is insufficient:
`environment_radiance` has two `environment_level` call sites; each level has three
conditional bilinear sites, each bilinear has four taps, and each tap has four
conditional remap sites. Fully expanding every syntactic branch therefore yields
**2 × 3 × 4 × 4 = 96 remap code sites**. This is not 96 runtime samples or an ISA
measurement; branches and driver optimization determine actual execution. The saved
call graph and source hash are in `environment-callgraph.json`. M7 owns any change
to that graph. This unit adjusts compiler boundaries only.

## Verification protocol

The final window run uses [M5-DL](m5-dev-loop.md): full-resident quarry, static camera
0, 1920x1080, 1x, 16x anisotropy, original lighting/environment, development profile,
active Vulkan validation, five unique visible edits per standard/user file, each
followed by restore, invalid save and recovery. Save-to-first-preview GPU completion,
present call and optimized completion are separate endpoints. Readback/PNG encoding
happens after those endpoints. No explicit preview flag or control-file command is
used in the default timing run.

An untimed quality run queues an HDR copy of the actual first-preview revision and
compares it to the optimized replacement and exact-source ordinary reference. The
ordinary CLI option gets a separate inactive fixture check without the interop
measurement environment. Library default behavior remains covered by existing tests.

## Game hold and queued work

No DL3 GPU lease or viewer was active when the game hold arrived. The CPU guard
stopped the in-progress preparation build when CS2 appeared. Its interrupted Rust
incremental cache caused a subsequent link failure; the worker's own incremental
directory was removed before rebuilding, without deleting shared dependency artifacts.

Under the explicitly relaxed hold, only short, headless compilation screens run at
below-normal CPU priority. They use the shared atomic GPU lease, the reservation and
release log, six-GiB launch/four-GiB runtime memory floors, and no Cargo inside a lease.
All timings during play are labelled contended and excluded from speed claims. Viewer,
image comparisons/captures and windowed M5-DL measurements remain queued until the
`[GAME HOLD RELEASED]` line and user release. No windows, focus/input automation or
desktop/game captures are used during play.

The M7 helper-graph proposal remains with M7. This unit does not edit environment,
lighting, shading, GI/temporal or shadow bodies, or the M5/M6 decision TSV.
