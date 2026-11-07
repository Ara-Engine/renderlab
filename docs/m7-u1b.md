# M7 U1b: non-temporal specular distributions

Branch `t3/m7-u1b`, base main `6877ea4`. Continues [U1](m7-u1.md).
Patent policy at `8d864c3` read before implementation: this unit uses no history,
temporal variance clipping, SVGF filter or temporal gradient. It does not build
any patented temporal fallback. **The spatial-distribution variant passes this
fixture's quality gate on both paths; `SPECULAR_AA=false` remains opt-in because
the speed floor and quarry memory budget are not met.** Its substantial cost and unmeasured cases below
remain misses; this is not an Unreal or whole-renderer acceptance claim.

## Diagnosis and registered variants

Integration update after `938a63f`: main `16ebd5b` is merged. R0c keeps RLA v4
for BVH attachments and RVG v3. New moment-only writes use RLA v5 and
`normal-v5/keys`; combined moment/BVH writes use RLA v6. RLA v1-v3 and R0c v4
remain compatible. Historical U1b v4 is read by its distinct moment texture
role and absence of a trailer; new writers never emit that overlapping version.
The sidecar cache remains `cluster-bvh-v1/rla<S>-rvg<S>` and now recognizes
moment resources as well. Combined RLA/RVG round trips and old/new source-cache
cold/warm/corruption tests pass. The figures below are the initial U1b screen;
their original v4 moment bytes are historical evidence, not the new write format.
The standard default is restored to false; current GPU tests require exact
unfiltered default identity, rather than equality to the enabled helper.

A 500,000-sample CPU probe of the exact quantized fixture base map found point
slope variances .00122842/.00122585, but bilinear-field variances
.00054427/.00054453: ratios .44307/.44421. Cross covariance is tiny on this
isotropic fixture. The reference integrates bilinearly reconstructed normals;
U1's mip variance described the discrete texels instead. That mismatch is in
addition to the wider GGX lobe's extra tail energy.

RLA v4 adds `NormalSlopeMoments`: two RGBA16Float layers, 16 bytes per logical
texel (four times the RGBA8 normal map). Layer 0 stores mean slope XY and raw
second moments XX/YY; layer 1 stores raw XY and the original normal XYZ.
Covariance is formed after filtering, never precomputed and then averaged.
Normal texture bindings become array views; legacy maps remain one-layer RGBA8
arrays, with unchanged helper values. No extra binding, hit-record data or
lighting-dependent receiver fields are added. Mips remain independently streamable.

The corrected preparation integrates each bilinear slope field over a base
texel's Voronoi area, then averages those raw moments through all mips. The
one-dimensional first-moment weights are `[1/8,3/4,1/8]`; product-basis integrals
give the full 2x2 second moment, including neighbour correlations and wrap modes.
The 4/9 factor for independent noise emerges from integration; it is not a
fixture-fitted multiplier. Slopes approximate the ratio of bilinearly filtered
normal XYZ; this approximation is recorded and must be measured.

The existing generated 320x180 screen, all seven patch populations, full-frame
population, 25 output times, both devices and the prior 256-subframe reference
are reused. Before is remeasured with the old normalized mip recipe. Its saved
image hashes are compared before reusing the reference. The capture process
loads the asset once and evaluates all variants:

1. Full point-slope covariance into an anisotropic GGX lobe.
2. The same anisotropic GGX with continuous-bilinear moments.
3. Nine original-alpha GGX lobes using Gaussian covariance quadrature.
4. Twenty-five original-alpha lobes.
5. Nine- and 25-lobe mixtures combined with direct single-frame geometric
   footprint integration. Normal and position differentials are geometric engine
   inputs; no neighbouring colour, lighting or visibility is read.
6. Distribution/roughness gating of the mixture for low-variance footprints.
7. Direct 16x16 centred spatial quadrature of the actual normal field and
   geometric footprint, retaining the original GGX lobe in every sample.
8. A hybrid: use that spatial quadrature outside three covariance standard
   deviations when intrinsic alpha-squared is smaller than the map variance;
   otherwise use the 25-lobe geometric-footprint mixture.

The mixtures retain the material's original GGX alpha in every summand. Their
positive weights sum to one; no wider GGX tail is introduced. The distribution
is still approximated by a Gaussian, so covariance alone cannot describe arbitrary
multi-modal scan normals. Geometric footprint integration uses current-frame
Gauss-Legendre samples; it is not temporal accumulation or screen-space lighting.

The gate remains per-population static/moving error, flicker and F3 energy bias
no worse than before, on both paths. No default is changed before that gate.

## Measurements and decisions

Evidence is at `E:/renderlab-work/m7-u1b/`. All measurements use the generated
seven-patch screen from U1, not a private scan. There are three mapped metallic
patches at roughness .08/.15/.30, three interpolated-normal geometric patches at
the same roughnesses, and one flat control; 14 triangles, 512x512 normal map,
one directional light, no environment or shadows, exposure -8. Static is one
frame, camera motion is twelve half-pixel steps, light motion is twelve steps.
Both paths use native 1x samples, with no reconstruction/history.

Before remains the normalized RGB8 mip chain. Every saved before image on both
paths is SHA-256 identical to U1's before image. The non-RT device has no ray
query feature; the RT device uses the traced primary path. All runs are
offscreen, with active Vulkan validation, zero validation errors, no window
creation and the shared lock plus reservation/release lines. With filtering enabled, the initial
standard program matched the accepted helper **every pixel, all 25 frames,
both paths**, rather than merely matching a population average.

### Reference and unchanged gate

The original reference has 256 independently jittered spatial strata per output
time, zero shutter. A 4096-stratum reference was added after the first mixtures
missed by a few millionths of HDR flicker RMS. Its normal sampling gradients
remain `/16`, exactly as in the original reference; increasing reference samples
does not change material blur. The reference alone accumulates subframes. The
shipping helper has no frame history and uses a distinct, centred sample grid.

Both original and refined scores are retained. The original-to-refined
full-image reference difference is .07269-.09146 HDR RMS across the 25 frames.
The refined reference's interleaved-half difference/2 is .05275 for the static
full image, including .00412 on mapped r=.08 and .32227 on geometric r=.08.
This is a convergence diagnostic, not a confidence interval or proof of an
exact film reference. Refinement reduced the mixture-25 misses from six to
one, but did **not** turn that mixture into a pass. The retained spatial variant
passes against **both** reference sets.

Each frame checks HDR RMS, common-display RMS and absolute relative energy
bias, for each of seven patches and the complete image. Every adjacent moving
frame also checks RMS of the temporal error difference against the reference.
That is 776 comparisons per path, per variant, per reference. Equality tolerance
is only `max(1e-7, abs(before)*1e-6)` for numerical noise. No quality threshold,
population, frame or F3 bias check was removed. Display conversion is the same
Reinhard-plus-sRGB transform for before, candidate and reference.

### Every variant

Counts below are failed **individual comparisons**, not failed populations.
SW/RT columns use the 4096 reference; the last column retains the original
256-reference counts. All variants were captured on the same screen and both
paths before selection.

| Variant | SW failures | RT failures | Original reference SW/RT | Decision |
|---|---:|---:|---:|---|
| Point covariance, anisotropic GGX | 283 | 283 | 286 / 286 | Reject |
| Bilinear covariance, anisotropic GGX | 213 | 214 | 217 / 217 | Reject |
| Bilinear Gaussian mixture, 9 lobes | 7 | 7 | 15 / 15 | Reject |
| Bilinear Gaussian mixture, 25 lobes | 1 | 1 | 6 / 6 | Reject |
| 9-lobe mixture x geometric GL9 | 3 | 3 | 11 / 11 | Reject |
| 25-lobe mixture x geometric GL9 | 1 | 1 | 6 / 6 | Reject |
| Same, low-variance gate | 2 | 1 | 7 / 6 | Reject |
| Actual spatial distribution, 256 samples | **0** | **0** | **0 / 0** | Retain, opt-in |
| Adaptive tail hybrid | 11 | 11 | 11 / 11 | Reject |

The full covariance filter is `A = alpha^2 I + 2 C`, with anisotropic GGX D and
Smith visibility. Correcting bilinear variance halves its worst r=.08 bias, but
it still has much too much tail energy: maximum F3 bias across all populations
and times is 41.465 -> 20.072, against before 1.077 (SW). A wider GGX is still
the wrong distribution. RT shows the same failure, 41.467 -> 20.072 against
before 1.080. The fixture's tiny XY covariance means this does not certify
strongly anisotropic content, even though the format and focused test retain XY.

Positive Gauss-Hermite mixtures preserve the original GGX tails and fix that
large bias. The 25-lobe mixture still approximates a finite, correlated pixel
distribution by a Gaussian. Its refined-reference failure is mapped r=.08,
camera frame 2: SW flicker .01971959 against before .01971945 (RT also fails).
Adding geometric GL9 fixes the large geometric error but not that map tail:
SW .01972003. Low-variance gating adds a small geometric threshold-switch miss.
The adaptive hybrid fixes that patch but creates eleven r=.15 moving-light
flicker failures: for example RT light frame 1 .12841 against before .09000.
Switching between two individually plausible distributions is not stable enough.

### Retained method and static changes

`specular_brdf` integrates the current pixel's normal and position footprint on
a deterministic 16x16 grid. Each positive-weight summand uses the material's
original roughness and original GGX BRDF. Mapped samples read the actual
bilinearly reconstructed normal field from layer 1 with explicit gradients;
geometric normals/positions use the engine's differentials. The grid evaluates
the view vector at each position. No synthetic tail is added, and no sample
normal outside this measured spatial footprint is invented. A zero-distribution
gate returns the original response where variance metadata exists. The later
compatibility update below integrates legacy maps' fine field directly, using
the full rule when no variance estimate exists.
The accepted result therefore does not rely on a scalar variance-to-alpha fit.

Against the refined reference, full-image results are:

| Path / phase | Before HDR RMS | After HDR RMS | Before flicker | After flicker |
|---|---:|---:|---:|---:|
| SW static | 4.617292 | .151831 | - | - |
| SW camera | 5.758218 | .158041 | 6.221763 | .080453 |
| SW light | 5.320172 | .160742 | 3.352697 | .014975 |
| RT static | 4.646099 | .153193 | - | - |
| RT camera | 5.770448 | .159225 | 6.234204 | .080699 |
| RT light | 5.332668 | .161787 | 3.359473 | .014965 |

The original-reference full-image static errors explicitly change from
4.673604 -> .179287 (SW), 4.702418 -> .180965 (RT); camera flicker changes from
6.251400 -> .129599 (SW), 6.263884 -> .130630 (RT). These retain the original
U1 reference and before values; they are not compared with a different baseline.

Static changes by population against the refined reference:

| Population | SW RMS before -> after | RT RMS before -> after |
|---|---:|---:|
| Mapped r=.08 | .014244 -> .010184 | .014244 -> .009870 |
| Mapped r=.15 | 1.697286 -> .451229 | 1.697164 -> .455354 |
| Mapped r=.30 | .167493 -> .031118 | .167466 -> .030955 |
| Geometric r=.08 | 28.915568 -> .015450 | 29.097391 -> .030554 |
| Geometric r=.15 | 1.362537 -> .007068 | 1.366056 -> .004719 |
| Geometric r=.30 | .019174 -> .000748 | .019231 -> .000654 |
| Flat control | .00001085 -> .00001085 | .00000910 -> .00000910 |

Static mapped r=.08 absolute relative energy bias drops .290388 -> .003928
(SW), .290361 -> .012914 (RT). The maximum F3 bias over **every** population
and output time drops 1.077110 -> .039223 (SW), 1.079686 -> .041739 (RT).
Every individual bias comparison passes; this is not just a lower maximum.
The r=.08 mapped camera RMS drops .015914 -> .012547 (SW) and
.015913 -> .012287 (RT); its flicker drops .019097 -> .017835 and
.019097 -> .017656. The low-roughness dark patch is improved without widening
the whole GGX tail. It still has residual quadrature error.

Inspected `contact-sheet-r2.png` and `contact-sheet.png`: complete images for
static, camera frame 2 and light frame 11 on both paths, including silhouettes,
the dark mapped patch and geometric highlights. The covariance variants visibly
brighten the dark patch; the spatial variant restores the reference's sparse
highlights while retaining sharp geometric highlight structure. This fixture
has no transparency or shadows, so it cannot certify those interactions.

### Cost and memory

Host: RTX 4070 SUPER 12 GiB, NVIDIA 617.14, Vulkan/wgpu 30.0.1, Ryzen 7 7800X3D.
Native cost is 1920x1080 on the same 14-triangle screen. GPU timestamps exclude
capture, readback, cooking and pipeline creation. Eight measured repeats follow
warmup; moving phases contain twelve poses per repeat. The executable is a
development test build (renderer opt-level 1, GPU validation active), **not** a
release full-scene performance result. Foreign CPU compiler work was observed;
no other GPU lease ran concurrently. Per-run process/RAM records are retained.

The first native screen exposed large idle/clock-transition outliers. It is
retained as `manifest-cold.json`. The repeated screen adds a two-second busy
shader warmup. Residual RT static timing variation across runs remains unresolved
(3.73 ms cold-run shade versus 8.08 ms repeated); report it rather than choosing
the faster value. These are scheduled-workload observations, not a hardware
clock-controlled performance claim.

Camera-phase median shading cost, ms, for **every** measured variant:

| Variant | SW | RT |
|---|---:|---:|
| Before | .032768 | .233664 |
| Point covariance | .047104 | .198016 |
| Bilinear covariance | .047104 | .197552 |
| Mixture 9 | .084800 | .269824 |
| Mixture 25 | .149392 | .346144 |
| Mixture 9 x GL9 | .382944 | .564816 |
| Mixture 25 x GL9 | .596992 | 1.097728 |
| Low-variance gate | .589168 | 1.026800 |
| Spatial 256, retained | 1.658272 | 3.386240 |
| Adaptive tail | 1.084928 | 1.810336 |

The RT before being slower than covariance in this phase also exposes the
clock/scheduling sensitivity. Per-stage medians for the retained method below
use the repeated run; complete minima/maxima are in `native-cost.json`.

| Path / phase | Shade before -> after | Primary before -> after | Tone before -> after | Span before -> after |
|---|---:|---:|---:|---:|
| SW static | .028672 -> 1.858560 | raster included | .012256 -> .014304 | .047104 -> 1.888768 |
| SW camera | .032768 -> 1.658272 | raster included | .013312 -> .012288 | .053184 -> 1.989120 |
| SW light | .029600 -> 1.672688 | raster included | .012288 -> .015296 | .047104 -> 1.703424 |
| RT static | .143760 -> 8.077472 | .101968 -> .210512 | .013312 -> .029696 | .270400 -> 8.334672 |
| RT camera | .233664 -> 3.386240 | .165376 -> .091152 | .023552 -> .012288 | .439296 -> 3.499792 |
| RT light | .130000 -> 3.375808 | .091456 -> .090864 | .012288 -> .012288 | .243632 -> 3.489424 |

No shadow pass exists in this screen. The retained filter is substantially more
expensive; the 2x-Unreal combined quality/performance target is **not established**.
The cheap mixtures cannot replace it until they pass the same gate.

One complete 512 normal mip chain grows from 1,398,100 to 5,592,400 bytes,
1.3333 -> 5.3333 MiB, **+4 MiB, 4x**. A complete 4096 chain would be
85.3333 -> 341.3333 MiB (+256 MiB), arithmetic rather than measured scan VRAM.
The test carries both moment recipes only for comparison; production needs one.
Bindings do not increase. Array layer count is two; layer-major mip payloads
are uploaded/copied together and charged at their full byte count. There is no
history allocation. Cooking budgets conservatively charge intermediate vectors;
large maps may require explicit documented limits, never an unchecked allocation.

### Asset boundary and programmable constraints

- Cook with `--normal-maps --normal-moments`, or `Settings.normal_moments`.
  This supersedes the optional v3 recipe; default cooking and old asset bytes
  remain compatible. The v4 recipe has a separate cache identity and key path.
  RVG remains version 2 with nested RLA v4 materials/textures.
- Sampler reconstruction and wrap affect the integrals. The cooker keeps
  separate moment textures for distinct source samplers, even for the same
  image. Use the matching sampler at runtime; changing it requires recooking.
  The bilinear correction applies when min and mag are both linear; other
  combinations retain point moments and are not quality-qualified here.
- The moment cooker accepts positive tangent-space Z and finite half-representable
  slopes. Unsupported maps fail explicitly. Raw second moments are filtered
  first; runtime derives covariance and repairs only negative/indefinite values
  introduced by half rounding. It does not clamp positive variance to fit a test.
- Normal bindings are `texture_2d_array` on both stages. Legacy one-layer maps
  retain their old helper sampling and before image hashes. Arbitrary raw-binding
  access is engine-owned; user programs use the documented helpers.
- The standard program keeps `SPECULAR_AA=false` (restored after review).
  It calls `specular_brdf` for direct lights and keeps intrinsic roughness;
  it does **not** also apply U1's rejected scalar roughness widening. Editing
  that constant enables the helper. Custom programs opt in by calling it,
  consume `normal_statistics` for their own lobe, or leave their shading alone.
- The filter requires geometric position/normal differentials (`world_dx/dy`,
  `normal_dx/dy`). They are engine inputs on raster and traced paths, including
  the non-RT device. No BRDF data is added to hit records, receiver data or
  dispatch. The material block remains 48 bytes, hit record 16 bytes.
- The helper currently integrates only the original GGX normal/view response.
  Base/roughness textures, tangent, light direction, attenuation and visibility
  are evaluated at the centre. Environment reflection/DFG filtering is unchanged.
  Custom perturbed normals must supply their matching distribution; this helper
  cannot infer them. No environment, local-light, tangent-discontinuity,
  multi-triangle, coverage, 4x-MSAA, streaming-convergence or scan-quality gate
  is claimed from this screen. Fine normal samples can request finer resident
  mips; whole-scene streaming/memory cost needs separate measurement.

### Patent policy, verification and misses

This is the first design-around tier of the `8d864c3` policy. LEAN-style
filterable moments and current-footprint quadrature use no history, history
bounding box, prior colour distribution, temporal gradient or variance-guided
denoising. In particular, the temporal variance clipping combination associated
with US 10,116,916 and the SVGF temporal/spatial filtering combination associated
with US 10,600,167 are absent. Reference accumulation is a measurement tool,
not the runtime method. No vendor shader implementation was copied. The spatial
variant passes, so the condition for considering a patented fallback was not
reached; there is no evidence here that a temporal method is necessary.

Validation at the initial `938a63f` checkpoint (current verification is below):

- `cargo test -p renderlab --lib -j 2`: exit 0, 116 passed, 113 GPU tests ignored.
- Cook and asset suites: exit 0, 86 passed, including v4 roundtrip, downgrade, byte budget,
  NaN/Inf rejection, correlated XY moments, bilinear reconstruction and distinct
  sampler recipes. Two pre-existing GPU tests remain ignored.
- `cargo fmt --all --check` and `git diff --check`: exit 0.
- Cook/asset Clippy `--all-targets -- -D warnings`: exit 0.
- Renderer Clippy with `-D warnings -A clippy::nonminimal_bool
  -A clippy::items_after_test_module`: exit 0, no new warnings.
  **Unrestricted renderer Clippy remains a delivery miss**: four pre-existing
  nonminimal-boolean diagnostics at `virtual_shadow_gpu.rs:203`, and
  items-after-test-module at `visibility_buffer.rs:1076`. These files belong to
  the shadow worker and were left untouched; no source suppression was added.
- All thirteen owned offscreen runs exit 0, including final-source captures and
  enabled-helper identity on both paths (historical true-default checkpoint). Total child-run verification time
  307.83 s; refined-reference run 109.07 s. Implementation time was not
  stopwatched across the interrupted session and is not inferred from GPU time.
  Final warm CPU verification took 21.90 s renderer Clippy and 8.55 s cook/asset
  Clippy; the final incremental library-test build took 48.73 s with foreign
  compilation present. These are verification durations, not measured
  edit-to-running-viewer latency. Shader edit-to-image, viewer startup and the
  incremental viewer loop were not remeasured in this no-window unit.

The final source/executable identities, run times, RAM and foreign-build samples,
gates and per-frame results are retained with the evidence. Final executable
SHA-256 at that initial checkpoint is `b714245e683221152a205d7cfe5d5a69bea04f8679032162a91804c9177daf74`.
`bilinear-diagnosis.json`
contains the probe; `reference-origin.json` preserves the reused reference's
identity; `verification-runs.json`, `native-cost.json`, `gates[-r2].json`,
`per-frame[-r2].json`, `reference-convergence.json` and inspected PNGs retain
the numerical evidence. Raw HDR captures and this worker's Cargo target are
removed at delivery. Shadow files, `film_accumulation.rs` and
`milestones-5-6-decisions.tsv` are untouched.
The minimum observed free RAM was 4,597.8 MiB. Up to eight foreign rustc
processes were observed; this worker used two Cargo jobs and none in its leases.

Reproduction: generate the owned fixture with the cook crate's
`specular_fixture` example into `E:/renderlab-work/m7-u1b/fixture.rla`. Compile
the library test executable outside a lease. Set `RENDERLAB_U1B=1`,
`RENDERLAB_U1_ROOT=E:/renderlab-work/m7-u1b`, and `RENDERLAB_U1_MODE` to
`reference`, `software` or `hardware`; only hardware sets
`RENDERLAB_TEST_PRIMARY=traced`. Use `m7-u1b-run.py` to run
`renderer::specular_aa_tests::gpu_specular_aa_motion_screen --ignored --exact
--nocapture --test-threads=1` from the compiled executable, never Cargo in a
lease. `RENDERLAB_REFERENCE_SIDE=64` and mode `reference-r2` produce the
4096-stratum reference; the default side is 16. Native cost sets
`RENDERLAB_U1_NATIVE=1` and a distinct output mode. The scorer accepts
`--reference reference-r2 --suffix=-r2`. All configurations load one asset per
process; eight repeated timing batches share the renderer. The CPU replay is
diagnostic only, not substituted for GPU gate results.

## Cost continuation after review

Main `16ebd5b` was merged and the format/default reconciliation was pushed as
`1296360`. RLA v4 remains R0c's BVH format, v5 is moments alone, and v6 combines
moments and BVH. Historical U1b v4 moment payloads remain readable by their
distinct role; new writes use v5/v6. Raster moment keys move to `normal-v5/keys`;
the separate `cluster-bvh-v1/rla<S>-rvg<S>` namespace is unchanged and recognizes
the new nested resources. Old/new cold/warm/corruption cache tests and combined
RLA/RVG attachment round trips pass. An invalid-version fixture was advanced to
7, since 5 is now valid. A library seam test caught CRLF in the edited standard
WGSL; restoring LF fixed it without changing any shadow file.

**`SPECULAR_AA=false` is the final standard setting.** Every current GPU capture
checks exact default-versus-unfiltered equality for all 25 frames on each path.
The complete unfiltered captures remain hash-identical to U1. The original
256-stratum reference is reused and the same 4096-stratum reference was
recaptured (fixed material gradients, zero shutter). Evidence is in
`E:/renderlab-work/m7-u1b/cost-iteration-1/`.

### Measured steps

The original 776 comparisons/path, numerical tolerance, populations and both
reference sets are unchanged. Counts are individual failed comparisons; `0/0`
means no failure on SW or RT. All listed GPU variants also have native timings.
The CPU replay rejected the smallest grids first; it is diagnostic, not the gate.

| Variant (benchmark index) | Original-reference failures SW/RT | Refined-reference failures SW/RT | Native camera shade ms SW/RT |
|---|---:|---:|---:|
| Retained full-moment spatial 256 (10) | 0/0 | 0/0 | 1.779 / 4.041 |
| Spatial 256 using v3 RGBA8 (12) | 0/0 | 0/0 | 1.773 / 4.041 |
| Adaptive factor 4, v3 + geometric GL9 (13) | 72/72 | 72/72 | .943 / 1.829 |
| Adaptive factor 8, v3 + geometric GL9 (14) | 0/0 | 0/0 | .905 / 1.984 |
| Variance hybrid + geometric GL9 (15) | 0/0 | 0/0 | .810 / 1.471 |
| Fixed spatial GL64 + geometric GL9 (16) | 43/42 | 42/34 | .315 / .705 |
| Spatial 144 + geometric GL9 (17) | 19/19 | 19/19 | .635 / 1.294 |
| Spatial 64 + geometric GL9 (18) | 8/19 | 23/28 | .315 / .696 |
| Spatial 256 v3 + geometric GL25 (19) | 0/0 | 0/0 | 1.067 / 2.240 |
| Variance hybrid + geometric GL25 (20) | 0/0 | 0/0 | .845 / 1.643 |
| Separable hybrid + geometric GL25 (21) | 0/0 | 0/0 | .643 / 1.099 |
| Spatial 256 using legacy RGBA8 (22) | 0/0 | 0/0 | 1.632 / 3.437 |
| Legacy spatial 256 + geometric GL25 (23) | 0/0 | 0/0 | 1.008 / 2.165 |

Adaptive count is the next power-of-two side length at least
`factor * sqrt(map_variance / alpha^2)`, clamped to 1..16. Side 1 returns the
original response. This uses cooked variance and the intrinsic lobe width,
not a lighting-change detector. The conservative factor 8 passes the registered
gate; factor 4 loses on the r=.30 mapped patch. Fixed Gauss-Legendre spatial
locations avoid jitter, but their 64 samples still miss the mapped distribution.
Uniform 144 and 64 samples also miss; fewer samples alone are not a solution.

The hybrids spend 256 actual-field samples when map variance exceeds
`4 * alpha^2`, otherwise 25 positive original-alpha Gaussian-mixture lobes.
Unlike the rejected angular-tail hybrid, their branch does not depend on the
moving light's half-vector. The first variants also integrate nine geometric
positions. The separable variant evaluates the broad mixture at the centre,
and integrates unmapped geometric normals with GL25. This removes redundant
view-position evaluations on broad mapped lobes. Full covariance remains
required for that mixture; the scalar v3 data is not presented as anisotropic.


### Quality change versus the retained result

Passing the original **before** gate is not equality to the achieved 256-sample
result. A separate comparison against variant 10 is retained in
`versus-spatial256.json`. No cheaper candidate is a lossless replacement on
all 776 comparisons; it is therefore not silently substituted for the existing
`specular_brdf` quality option. The smaller-count and mixture routes remain
experimental `filtered_brdf` selectors. Static changes are explicit:

| Variant | Static full RMS SW/RT | Camera full RMS SW/RT | Camera flicker SW/RT |
|---|---:|---:|---:|
| Original before | 4.617292 / 4.646099 | 5.758218 / 5.770448 | 6.221763 / 6.234204 |
| Retained 256 (10) | .151831 / .153193 | .158041 / .159225 | .080453 / .080699 |
| Adaptive factor 8 (14) | .244777 / .249019 | .325079 / .328344 | .224842 / .226641 |
| Separable hybrid (21) | .171656 / .171414 | .174849 / .174881 | .138961 / .139071 |
| Full legacy 256 (22) | .151833 / .153203 | .158066 / .159254 | .080472 / .080727 |
| Legacy 256 + GL25 (23) | .152486 / .153513 | .158178 / .159334 | .080572 / .080751 |

The separable hybrid preserves the r=.08 mapped result exactly, but static
mapped r=.15 RMS rises .451229 -> .589086 (SW) and .455354 -> .588874 (RT)
against retained 256; mapped r=.30 rises .031118 -> .090770 and
.030956 -> .090750. Geometric r=.08 static RMS rises .015450 -> .090156
(SW) and .030554 -> .068613 (RT) with GL25. These are still improvements on
the original before image, but are real changes. Static r=.15/.30 geometric
errors improve slightly. The flat control remains unchanged.

Maximum absolute F3 bias across every population/time remains better than the
original before: SW before 1.077110, retained 256 .039223, separable hybrid
.034203, full legacy .039096; RT before 1.079686, retained 256 .041739,
separable hybrid .041739, full legacy .041825. Every original per-frame bias
gate passes for those modes; the small legacy RT increase versus retained is
not hidden behind its original-before pass.

### Native stages and resource limits

Same RTX 4070 SUPER/617.14, 1920x1080, 14-triangle screen, development test
build, active validation, two-second busy warmup and eight repeats. No shadow
pass or environment is enabled. The final matched batch loads the asset once
per path and measures every candidate with the same 256 control (1.779/4.041 ms
camera shading). Earlier timing batches remain retained, rather than selecting
an earlier cold outlier. These are isolated algorithm timings,
not matched end-to-end Unreal results. Foreign CPU compilation was present;
each GPU lease is exclusive. All stage p50/p95/p99 and ranges are retained.

| Path / variant | Static shade / span | Camera shade / primary / tone / span | Light shade / span |
|---|---:|---:|---:|
| SW hybrid 21 | .672 / .690 | .643 / included / .012 / .662 | .648 / .667 |
| RT hybrid 21 | 2.840 / 3.111 | 1.099 / .096 / .012 / 1.223 | 1.095 / 1.213 |
| SW legacy 22 | 1.636 / 1.752 | 1.632 / included / .012 / 1.655 | 1.633 / 1.660 |
| RT legacy 22 | 14.194 / 14.526 | 3.437 / .121 / .012 / 3.580 | 3.439 / 3.553 |
| SW legacy + GL25 23 | 1.227 / 1.248 | 1.008 / included / .012 / 1.026 | 1.004 / 1.022 |
| RT legacy + GL25 23 | 2.519 / 2.651 | 2.165 / .121 / .012 / 2.310 | 2.168 / 2.278 |

The static RT idle/clock anomaly persists even after warmup (14.194 ms legacy
static versus 3.437 ms moving). It is disclosed, not used as the speedup
denominator. The separable hybrid saves about 2.77x/3.68x camera shading against
its paired 256 control, with the quality changes above. That does **not** meet
the combined quality/performance floor on the quarry; nothing is promoted to
default. The closest lower-memory geometric variant is still expensive.

### Normal-map memory and quarry accounting

Fixed spatial quadrature does not require moments: it integrates the actual
fine normal field already present in legacy maps. The helper now takes that
conservative full-256 route on legacy data instead of returning unfiltered.
Existing RGBA8 maps need **no recook, extra channel or companion texture**.
V3 and legacy full-256 captures are byte-identical for all 25 frames on both
paths. Storage per complete 512 chain is 1,398,100 bytes (1.3333 MiB) rather
than 5,592,400 (5.3333 MiB); a 4096 chain is 85.3333 rather than 341.3333 MiB.
This is ordinary map storage, not compressed full covariance. The full moment
encoding remains 4x for users/experiments that need it. The one-load comparison
fixture keeps all formats resident, so its total process VRAM does not measure
the storage saving of a production asset that omits unused moment textures.

`m7-u1b-texture-bytes.py` streams actual prepared headers without loading scan
geometry/images. The current quarry is RLA v1, 1,548,588,300 bytes, twelve meshes
and twelve materials. Its twelve 4096 base-color chains each occupy 89,478,484
bytes; **normal-map bytes are zero**. It cannot validate a normal-mapped quarry
VRAM budget. If twelve 4K normal maps were added, plain RGBA8 would add about
1 GiB and full moments about 4 GiB, before allocator/streaming effects; those
are projections, not measured residency. The recorded roughly 379 MiB M7
headroom after R0c would not cover fully resident versions of either addition.
Fine-field quadrature can also demand finer resident mips. A normal-mapped
quarry budget/quality run remains a miss; the lower-memory route does not erase it.

### TR1 handoff, constraints and checks

**Likely cheap path:** TR1 can spread current-footprint samples across frames
instead of evaluating 256 every frame. Use its identity/event-valid reconstruction,
with explicit responses to cuts, revealed surfaces and lighting/material changes.
It must pass every age and the motion/transient gates; steady-state convergence
alone is insufficient. This unit builds no temporal method. Keep W5's auxiliary
outputs internal, as required by [P0](m7-p0-claims.md) and the current M7 design;
do not add variance clipping (US 10,116,916) or an SVGF-style pipeline here.

The new spatial variants use fixed deterministic nodes and no tap jitter or
pattern rotation. Their adaptive order uses normal-distribution data, not
history counts. In P0's mapping, US 11,113,792 claims 1/11 require jittered tap
selection, and claim 15 requires temporal/history-count filtering; those pieces
are absent. US 12,423,782's jittered-location element is absent. No previous
pixels, reprojected auxiliary values or neighbouring lighting are read.

All earlier centre-light/visibility, environment, coverage, tangent-discontinuity
and custom-perturbation constraints remain. The methods are optional GGX
helpers, not receiver/hit-record constraints. Offline fitted mixtures and packed
full covariance were **not built** in this continuation; they remain explicit
next experiments, not claimed failures. A fitted mixture must preserve its
actual tail distribution and interpolation across neighbouring texels/mips.

Final checks: library **122 passed, 117 ignored**; cook/asset **91 passed,
2 ignored**; fmt and diff checks pass. Cook/asset strict Clippy passes; renderer
Clippy passes with the two previously documented baseline lint classes excluded.
Unrestricted renderer Clippy still has the existing shadow/visibility failures;
no suppression or shadow-file edit was added. Main remains an ancestor of the
branch. No U1b edit touches `film_accumulation.rs` or the decisions TSV; the
merge imports main's already-committed changes.

Fifteen successful guarded GPU child runs took 344.77 s, including 103.94 s for
the refined reference; all exit 0 with no validation error. One stale executable
path was rejected before launching a child and corrected. Minimum observed
free RAM was 7010 MiB; foreign compiler contention is recorded (up to fourteen
rustc processes). This worker used two Cargo jobs and none in its GPU leases.
Implementation/merge/CPU-probe time was not isolated from queue waits; it is not
reported as GPU time. Cargo durations and each verification run are retained.

Current executable SHA-256:
`de6f847f200be0abdced96ae369166c13b81bf79980a43d6118a46f919a8b1b4`.
The evidence retains `gates-r2.json`, `gates-original.json`, per-frame and
summary JSON, `versus-spatial256.json`, native manifests/stage summaries,
`quarry-textures.json`, CPU probes and inspected `contact-sheet-r2.png` and
`contact-sheet-original.png`. Raw captures and this worker's target are removed
at delivery. Reproduce this batch with variants `0,10,12,...,23`, then give the
same explicit list to the scorer's `--variants` argument; `--contact-variants`
chooses the inspected figure columns. The standard program remains opt-in.
