# Traced-frame determinism: arithmetic contraction

2026-11-05. Main base `79870a9`; phase-1 comparison base
`e593c13edb1daae2d13d58075bb2710684f5836e` (`integrate/prog-shading-p1`).
Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, wgpu/naga 30.0.1, optimized
library test binaries, two Cargo jobs. All GPU work was offscreen.
Renderer acceptance used the full-resident registered quarry (12 draws,
13,034,903 triangles), 1920×1080 at 1x, 16x anisotropy, Exact 256-direction
visibility and Interactive eight-direction visibility. Settings and assets match
the accepted reference. These are correctness runs, not contention-controlled
frame-time measurements.

## Result

The reproduced failure originates in floating-point camera-ray construction,
before traversal, visibility counts, material sampling or shading. The backend
changes the numerical evaluation of implicit multiply-add expressions in
`primary_ray`. Explicit `fma` calls in **both** pixel-to-NDC mapping and inverse
projection preserve the accepted bytes and remove the reproduced instability.

The production change is confined to `shaders/primary_rays.wgsl`. It keeps the
same matrix, pixel centers, near/far planes, ray differentials, resource layouts
and programmable shading interface. It introduces no approximation or quality
setting. It does not establish a performance improvement.

| Patched production source | Processes | Static comparisons per process | Total identical comparisons | Complete reference captures |
|---|---:|---:|---:|---:|
| Main | 3 | 32 repeats × 4 seeds × 3 poses = 384 | 1,152 | 108/108 identical |
| Phase 1 | 3 | 32 repeats × 4 seeds × 3 poses = 384 | 1,152 | 108/108 identical |

Each process exercises camera poses 0, 1, 0, Exact receivers/counts, Interactive
seeds 0–3, hard shading, invalid reload preservation, tint/revert reload and
raster fallback. Every passing process logged active **Core + Synchronization**
validation and `errors=0`. The 36 captures include entire HDR images, receivers
and counts; they match `m6-interactive-climb/runs/accept-c11` byte for byte.
Exact images are checked at each pose and across processes against that reference;
the 32-repeat loop is Interactive. These are three process launches per branch,
not thirty process launches.

Phase-1 verification used an archive of its committed source plus the identical
production shader change and a test-only repeat loop. Its shared, actively edited
worktree was not changed. `E:/renderlab-work/determinism/integrate.patch` records
that complete verification delta.

## Reproduction and the first divergence

The original `b0t` baseline failed `static HDR seed=1` in 2/3 fresh processes
with synchronization validation enabled. A simplified harness initially hid the
failure, so instrumentation was moved behind the **original** failing comparison.
No diagnostic work precedes that comparison.

One captured trace manifestation has:

- 22,381 differing HDR pixels;
- 178,120 differing hit records, including 58 different primitive IDs; placement
  IDs remain identical;
- three differing visibility counts;
- a fresh shading pipeline reproducing the deviant image;
- the old shading pipeline reproducing that same image in 32 frozen-input runs;
- a fresh, identically seeded **trace** pipeline restoring every original hit,
  count and HDR byte without changing the frame inputs or rebuilding geometry.

Thus the soft-shading-only explanation does not cover every recurrence. Small
camera-ray arithmetic changes can select a neighboring microtriangle, then alter
interpolation and a few shadow decisions. The historical 13-pixel shading-only
capture was not separately reproduced here. The shared `primary_ray` helper also
feeds deferred shading's analytic texture gradients, so the retained correction
covers that use without changing material code.

`gpu_ray_arithmetic_repeat` reduces this to one 64-byte inverse-matrix buffer and
one `rgba32uint` output image. The final harness constructs only a Vulkan device,
this buffer, its bindings and a compute pipeline: no renderer, scene, TLAS, BLAS,
history, scratch, sampled textures, atomics or shared memory. Each invocation
writes one distinct pixel, storing raw f32 bits rather than rounded HDR values.
It runs two camera matrices and returns to the first, comparing repeated and
return outputs.

The final independent-device harness reproduces the original failure in a
256-repeat run. With `explicit_fma: true`, all 256 repeats at each of three poses
(768 comparisons) and the return comparison pass; its initial raw-ray hashes
remain the original accepted hashes.

The original reduced kernel changed 597,313 pixels in one static sequence.
A far-point-only reduction (`b.xyz / b.w`, with `b = inverse * vec4(ndc,0,1)`)
changed 1,090,211 pixels. Fresh pipelines can restore the original bits. Individual
NDC and clip-coordinate probes, plain division, and a scalar multiply-add/divide
probe stayed stable in their sampled runs; changing the shader's liveness changes
the optimization problem, so those negative probes are not universal guarantees.

## Separating the causes

| Candidate | Evidence and conclusion |
|---|---|
| Driver recompilation / uniform specialization | A long-lived pipeline changes while frame inputs stay fixed; a fresh pipeline restores them. A matrix stored in a storage buffer still reproduces it. Uniform-only specialization is not a sufficient explanation. Lazy driver optimization/specialization is consistent with the observations, but no driver recompilation counters or machine-code capture were obtained. |
| Contraction / reassociation | Matched raw SPIR-V controls: the unannotated kernel and far-point variant drift; adding `NoContraction` to floating arithmetic prevents drift in 128-repeat controls. That control changes the baseline bits, so it was rejected as a production fix. Explicit FMA choices can preserve the baseline while fixing the relevant evaluation order. |
| Uninitialized or stale memory | The standalone kernel reads only the initialized matrix, writes every in-range pixel once, and never reads its output. It reproduces without any renderer history or scratch. |
| Visibility races / atomic order | The standalone reproducer has no traversal or atomics. Frozen shading stays stable while fresh tracing restores the reference. Visibility differences in that capture are downstream of changed rays. |
| Texture LOD / derivatives | The reproducer has no sampled textures or derivatives. Texture sampling is not necessary for the failure. The correction leaves all sampler and material settings intact. |
| wgpu/naga emitting different SPIR-V | Actual `vkCreateShaderModule` input was captured with LunarG API dump, not approximated using an independent writer configuration. Identical seeded pipeline creations supplied identical SPIR-V, including the fresh restoring pipeline in a failing process. |
| Missing synchronization | Core and synchronization validation were explicitly active during failures and final acceptance. No validation errors were logged before the reproduced failures. Clean validation alone cannot prove absence of every driver or application defect; the independent arithmetic reproducer provides the stronger separation. |

The actual seed-1 SPIR-V hash in `dump1` is
`a1361c7d89417946cbbc775b549a7b3e1453127fe96f9266e5c76993307e738e`:
four identical creations, including the restoring trace. The repeated soft-shading
binary is `faee1d16ca6d695cd002b5b8f7ec5ec79e98838baab6a2f4aabd5c6add628fc0`.
Neither contains `NoContraction`. Vulkan repeatability/invariance rules and their
side-effect qualifications are documented by [Khronos](https://docs.vulkan.org/spec/latest/appendices/invariance.html).
The driver-internal mechanism remains an inference; arithmetic instability and
the output-preserving correction are directly tested.

## Why the earlier 16/16 did not settle it

There is **no production renderer or shader change** in `af6578c..79870a9` to
bisect. That range adds documents, scripts and test/reference work. The d17 source
was also reconstructed from `7de1ede` and its recorded `source.diff`: its primary
shader is the same pre-fix shader as main. The `PrimaryState` difference is a
`#[cfg(test)]` film-reference helper, not production compilation or dispatch.

Moving the dispatch region into storage was a useful mitigation, not an
arithmetic-invariance guarantee. The historical 16/16 is a finite observation;
it does not demonstrate that the later failures were introduced by a new main
commit. This investigation did not identify a changed external driver/cache
condition responsible for the different observed failure rates.

## Retained fix and rejected partial fixes

Pin the accepted contraction order explicitly:

```wgsl
let ndc=fma((vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size),
            vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
let xy=fma(inverse[0],vec4<f32>(ndc.x),inverse[1]*ndc.y);
let a=xy+inverse[2]+inverse[3];
let b=xy+inverse[3];
```

Other explicit FMA associations changed the reference ray bits and were rejected.
The selected order matches the original raw ray hashes at both poses. Matrix-only
FMA passed three short acceptance processes and 256 reduced repeats per pose, but
failed the extended renderer gate at seed 2, repeat 25: 177,603 changed hits and six
counts. **Pinning NDC's multiply-add is also necessary.** Moving the entire primary
frame from uniform to storage likewise failed and was reverted.

No `NoContraction` injection or unsafe shader passthrough is used by production.
Those paths exist only in the ignored arithmetic experiment. No lighting-model
constraint entered hit records, receiver data or dispatch structure.

## Running the committed tools

Build the library test binary with two jobs and put the target on E: (the user
explicitly authorized these research builds):

```powershell
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir E:/renderlab-work/determinism/target
```

`src/determinism_tests.rs` and `support/determinism_ray.wgsl` are the test-only
reducer. A minimal manifest contains `output`, `validate`, `repetitions` (default
128) and `cameras`, pointing to the two-pose camera JSON; this run used
`D:/Coding/renderlab-work/m6-hardware-msaa/quarry-cameras.json`. Optional fields:

- `explicit_fma: true`: test the retained expression order; false reproduces the
  original arithmetic even when the renderer is fixed.
- `storage: true`: move the matrix input to storage as an isolation control.
- `passthrough: true`: use naga-generated raw SPIR-V as the matched control.
- `no_contraction: true`: annotate its floating arithmetic with `NoContraction`.
  The emitted `ray.spv` can also be checked with SDK `spirv-val`.
- `width`, `height` (default 1920×1080), and exact-once WGSL `edits` for reductions.

The full acceptance manifest additionally uses the existing `asset`, `limits`,
`lighting`, `environment`, `replay` and `reference` fields.
`static_repeats: 32` repeats each seeded frame in-process; the `static` test requires
at least 30. Its default remains one in the ordinary acceptance test.

```powershell
py -3 scripts/determinism-run.py --binary E:/path/to/renderlab-test.exe --manifest E:/path/to/arithmetic.json --output E:/new-evidence-folder --test arithmetic
py -3 scripts/determinism-run.py --binary E:/path/to/renderlab-test.exe --manifest E:/path/to/acceptance.json --output E:/another-new-folder --test static --iterations 3 --reference D:/Coding/renderlab-work/m6-interactive-climb/runs/accept-c11
```

The runner atomically acquires `D:/Coding/renderlab-work/gpu-slot.lock`, writes
`owner.json`, preflights every process for games, Unreal, other renderer jobs and
at least 6 GiB available RAM, monitors for conflicts, bounds the process timeout,
and records acquire/release times. Every hold is below 45 minutes. It explicitly
enables `khronos_validation.validate_sync = true`; the test log must list
Synchronization under `CURRENT-VALIDATION-ENABLED`. Settings follow the
[validation-layer configuration](https://github.com/KhronosGroup/Vulkan-ValidationLayers/blob/main/docs/updating_from_VK_EXT_validation_features.md).

`--api-dump` captures actual Vulkan shader bytes. Extract and group them with:

```powershell
py -3 scripts/determinism-spirv.py E:/evidence/0/api.txt E:/evidence/spirv
```

Passing image files are hashed and removed by default to bound disk use;
`--keep-images` retains them. Failures keep bounded diagnostics and the first
reference/deviant pair instead of printing megabytes of pixel arrays.

## Evidence, checks and limits

All new bulk evidence is under `E:/renderlab-work/determinism/`; no asset or
measurement is committed. Useful records:

- `runs/original-trace-hook-1/` and `runs/dump1/`: failing real frames, restored
  hits/counts/HDR, frozen shading and SPIR-V identities.
- `dump1-api.txt`, `extracted/creations.json`, `ray-batch*.json`, `fma-screen.json`:
  actual Vulkan modules and arithmetic controls.
- `gate-main-fma2/`, `gate-main-final/`, `gate-integrate-fma2/`: final 32-repeat
  gates, binary hashes, validation logs, complete capture hashes and leases.
- `minimal-original-final/`, `minimal-fixed-final*/`: final standalone-device
  reducer checks; `summary.json` and the `gpu-windows.json` ledgers record timings.
- `offscreen-preview.png`: inspected Exact / Interactive / hard readback contact
  sheet (CPU display mapping of HDR). Silhouettes, background, surface appearance
  and shadow footprints match the unchanged accepted fixture; complete byte
  comparison, rather than the downscaled preview, establishes equivalence.

Affected-crate release Clippy (`--lib --tests -D warnings`), formatting, Python
syntax checks and `git diff --check` pass. Ten focused primary CPU tests pass,
including WGSL→SPIR-V compilation; 14 GPU cases are deliberately ignored by that
CPU invocation. The GPU gates above were run separately under the lease.

Implementation/analysis and verification were interleaved, beginning about 07:17
UTC; editing time was not separately instrumented. Incremental library test builds
took approximately 26–39 seconds each with two Cargo jobs; warm Clippy took
3–4 seconds. GPU leases and child durations are recorded separately in the evidence.
No frame-time benchmark or broader rendering-quality claim is made.

The final lease ledger contains 56 owned windows from **07:20:06.488917 to
08:18:14.150562 UTC**, totaling 1,405.7 seconds; the longest is **121.8 seconds**.
Every owned window was released. This total includes guarded builds and failed or
deferred experiments, and is not GPU execution time.
The 70 clean preflights had at least 8,266 MiB available; no game or Unreal Editor
was present in them. Passing image captures were compacted after retaining their
hashes, reclaiming about 5 GiB of this task's evidence space.

Only this GPU/driver, the registered quarry and these explicit expression orders
were verified. This is not a guarantee that arbitrary user WGSL, other drivers or
future optimizers are bit-identical. All game/Unreal/foreign-process or RAM
deferrals and an overlapping-renderer interruption are excluded from pass/fail
counts. No viewer, desktop input or game capture was used. The GPU slot was released
after every owned window. No commits were pushed.


## Receiver investigation: same-ray traversal precision and residual arithmetic

2026-11-05, receiver-origin work based on `539c233`; RTX 4070 SUPER,
NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1, optimized tests with two
Cargo jobs. This finding is separate from the camera-ray contraction diagnosis
below. Evidence: `E:/renderlab-work/receiver-origin/actual-ray/` and
`fine-debug-summary.json`. Runs were offscreen under the shared GPU lease.

The three T1 quarry witnesses are pose 0 **(1347,380)**, pose 1
**(1119,713)** and pose 2 **(1325,276)**. A diagnostic writes the actual
GPU f32 ray origin/direction bits, returned primitive/barycentrics, returned
object-space vertices and the GPU's object-to-world matrix. All three returned
hit records match the original T1 watch records exactly. Every one of their
32 diagnostic shadow rays is blocked.

The following intersections use **that same ray**, with its f32 bit patterns
interpreted exactly. The affine transform and Möller–Trumbore determinants use
Python rational arithmetic; there is no rounded world-vertex array or rebuilt
f64 camera ray. Both triangles contain the ray intersection. The source RLA
supplies the index-adjacent triangle; the returned triangle's source vertices
match the GPU vertex readback. Reproduce with
`scripts/m6-receiver-ray-proof.py --rays <rays.json> --asset <full.rla> --out <proof.json>`.

| Pose | GPU primitive | Exact nearer primitive | Near t (m) | Far t (m) | Gap (µm) | Gap / f32 spacing at t |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 792224 | 96182 | 10.371475181292949 | 10.371478089780888 | 2.908488 | 3.049771 |
| 1 | 683251 | 130985 | 7.456214173203602 | 7.456217666098777 | 3.492895 | 7.325132 |
| 2 | 330253 | 73189 | 5.317873491192970 | 5.317873730252765 | 0.239060 | 0.501345 |

Origin and direction words, in x/y/z order:

| Pose | Origin u32 bits | Direction u32 bits |
|---|---|---|
| 0 | 1084643078, 1079609728, 1086733976 | 3199554938, 3191725192, 3211437939 |
| 1 | 1084663096, 1079604188, 1086735981 | 3203026834, 3202835874, 3208906120 |
| 2 | 1076249851, 1073737051, 1078020408 | 3200478687, 3185187307, 3211463342 |

This demonstrates finite-precision traversal/instance-transform/triangle-hit
ordering on this tested implementation. It does **not** isolate the hardware
triangle unit from the instance inverse transform, establish a Vulkan violation,
or establish the same behaviour on other GPUs. The exact scene's nearer facet
has positive viewed-side N·L at all three witnesses; the returned farther facet
has negative N·L. These witnesses therefore are not a real backlit sliver created
solely by using a different f64 camera ray. An earlier independently reconstructed
f64 camera ray was a different experiment and cannot establish this conclusion.

Pose 2 is especially important: ordering two long f32 t values is insufficient
when their exact separation is half a float spacing. A repair must also produce
a stable receiver if traversal initially selects either edge neighbour. The
receiver investigation uses a canonical shared edge and local facet depth slopes,
with an error-bounded primitive-ID tie; it does not rank facets by illumination.

### Rejected compensated-arithmetic experiment

A two-float camera reconstruction/intersection experiment tried `p=a*b` followed
by `fma(a,b,-p)`, compensated sums and explicit FMA evaluation orders. Direct GPU
readback returned zero for **all six** low components of each diagnostic ray,
although CPU expansion of the same read-back f32 matrix had nonzero residuals.
The matrix matched bit for bit. The purported fine direction still differed
from the CPU result by approximately 5e-6; the near point differed by approximately
1e-7. Consequently its intersection ordering was ordinary f32 accuracy, and its
apparently higher-precision results were rejected.

The observation establishes loss of those residuals in the tested shader path;
the optimizer stage responsible was not isolated. Explicit `fma` remains useful
for the accepted contraction order, but it is not proof of an error-free
two-product expansion. [WGSL's floating-point and FMA rules](https://www.w3.org/TR/WGSL/#floating-point-evaluation)
must be considered alongside actual bit readback. No compensated arithmetic from
this experiment is retained in the production receiver repair.

### Separate pre-existing Vulkan layout warnings

The original main baseline executable and the receiver candidate both emit
`VUID-StandaloneSpirv-None-10684` during validation-off startup on this SDK.
The diagnostic text identifies explicit `Offset` decorations on a function-local
struct. This is a separate pre-existing item, not evidence of a new receiver
race or of the compensated-arithmetic diagnosis. The original-main control is
`receiver-origin/runs/baseline-layout-control.json` and its log. It runs zero
measurement pairs after initialization. Validation-on receiver quality runs
report active Core + Synchronization validation and zero errors. Timing reports
must state their validation setting; the validation-off warnings must not be
silently represented as a clean validation run.
