# M7 U1b: non-temporal specular distributions

Branch `t3/m7-u1b`, base main `6877ea4`. Continues [U1](m7-u1.md).
Patent policy at `8d864c3` read before implementation: this unit uses no history,
temporal variance clipping, SVGF filter or temporal gradient. It does not build
any patented temporal fallback. **The spatial-distribution variant passes this
fixture's quality gate on both paths; `SPECULAR_AA=true` is now the explicit
standard-program default.** Its substantial cost and unmeasured cases below
remain misses; this is not an Unreal or whole-renderer acceptance claim.

## Diagnosis and registered variants

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
creation and the shared lock plus reservation/release lines. The final
standard program matches the accepted helper **every pixel, all 25 frames,
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
| Actual spatial distribution, 256 samples | **0** | **0** | **0 / 0** | Retain, default |
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
gate returns the original response; legacy mapped textures also retain their
original response because their discarded distribution cannot be recovered.
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
- The standard program explicitly changes `SPECULAR_AA` from false to true.
  It calls `specular_brdf` for direct lights and keeps intrinsic roughness;
  it does **not** also apply U1's rejected scalar roughness widening. Editing
  that constant disables the helper. Custom programs opt in by calling it,
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

Validation on the final runtime sources:

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
  standard-default identity on both paths. Total child-run verification time
  307.83 s; refined-reference run 109.07 s. Implementation time was not
  stopwatched across the interrupted session and is not inferred from GPU time.
  Final warm CPU verification took 21.90 s renderer Clippy and 8.55 s cook/asset
  Clippy; the final incremental library-test build took 48.73 s with foreign
  compilation present. These are verification durations, not measured
  edit-to-running-viewer latency. Shader edit-to-image, viewer startup and the
  incremental viewer loop were not remeasured in this no-window unit.

The final source/executable identities, run times, RAM and foreign-build samples,
gates and per-frame results are retained with the evidence. Final executable
SHA-256 is `b714245e683221152a205d7cfe5d5a69bea04f8679032162a91804c9177daf74`.
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
