# MAT1: quarry material response

Base: `41b01d1`, branch `t3/mat1-quarry-material`, 2026-11-08.

**The dominant darkening is the runtime's split-sum environment specular approximation.**
The reference material path is correct in the controls exercised here. This is not a
base-colour decoding defect. A numerical GGX environment-integral probe largely repairs
the open-ground deficit, but **fails the required no-regression gate** on the occluded,
solid-occluded and silhouette populations. It is rejected as a default change.

**MAT1 remains incomplete.** This branch retains the diagnosis and reproducible measurement
tools. The standard program, texture upload/sampling, reference tracer, GI cache, temporal
and shadow implementation remain unchanged. The production before/after image is therefore
unchanged. The numerical probe results below are a rejected candidate, not a shipped fix.

## Cause

The declared material is plain nonmetallic Lambert plus single-scatter GGX, perceptual
roughness squared, height-correlated Smith and Schlick F0 0.04. The reference evaluates
the environment integral of that BRDF. The standard program instead evaluates

```
prefiltered_environment(reflect(-V, N), roughness) * (F0 * DFG.x + DFG.y)
```

Its RLE2 prefilter is normalized and assumes `N = V` about the reflection direction.
At grazing views, its hemisphere and angular weighting differ substantially from the
actual `N,V` BRDF. The quarry sky has a highly concentrated bright source: source-cube
RGB spans about 0.02 to 1.99 million cd/mÂ². The approximation loses a large part of its
rough specular response when that source is away from the mirror direction. Dark scan
albedos make the missing dielectric specular energy proportionately much more visible
than an albedo-0.5 Lambert control, which has no specular term at all.

This is the known assumption described in Karis's
[Real Shading in Unreal Engine 4, split-sum approximation](https://www.gamedevs.org/uploads/real-shading-in-unreal-engine-4.pdf),
identified here by independent quadrature and actual Renderlab captures. It does not
establish how Unreal's complete current lighting pipeline responds.

## Material and texture audit

- All twelve prepared materials have white factors, metallic 0, roughness 0.85,
  double-sided shading and one 4096-square base-colour sRGB texture, with 13 mips.
  There are no metallic/roughness maps or normal maps in this RLA1 quarry.
- Every source vertex colour is exactly `[1,1,1,1]`. Per-placement GPU albedo agrees
  with an independent CPU trilinear replay to about 0.1% at the medians.
- Runtime upload uses `Rgba8UnormSrgb`; reference texel loads perform the sRGB transfer
  before interpolation. Factors and vertex colours are linear. Both multiply them once.
  The mip cooker filters decoded linear colour and re-encodes each mip.
- Both material paths use MR green for roughness and blue for metallic. Normal-map
  strength/handedness cannot cause this quarry result because the maps are absent.
- The reference intentionally samples bilinear level zero and integrates pixel jitter.
  Runtime filtering uses pixel-footprint derivatives. On open up-facing lit ground,
  the independent 8Ã—8 level-zero footprint integration versus trilinear sampling gives
  placement-median ratios approximately 0.98â€“1.03. This explains small local differences,
  not the systematic 0.72â€“0.89 radiance ratios.
- Both paths use F0 0.04 and the same single-scatter convention; neither applies
  multiple-scatter compensation. Changing F0, albedo or colour-space conventions would
  change the declared material rather than repair its environment integral.

The ground replay uses the frozen reference's centre placement/primitive/world records,
the actual source triangles and UVs, analytic projected footprints and the source texture
mips. It excludes registered edges, requires geometric normal Y > 0.97 and the registered
lit population. It is not a full pixel-footprint oracle across triangle boundaries.

## Independent physical controls

`scripts/mat1-mitsuba.py` evaluates 80 view/light combinations against installed Mitsuba
3.9.1, `scalar_rgb`. Its metallic Principled material with base 0.04 isolates the same
Schlick F0-0.04 GGX specular law, without diffuse or other lobes. Maximum absolute error
is **1.665e-8**, maximum relative error **7.412e-7**. Mitsuba uses separable Smith here;
this independently checks the shared GGX/Fresnel/roughness conventions, not Renderlab's
height-correlated Smith variant. See the version-pinned
[Mitsuba Principled implementation](https://github.com/mitsuba-renderer/mitsuba3/blob/v3.9.1/src/bsdfs/principled.cpp).

The existing U0b texture/transport qualification against Mitsuba is retained unchanged:
X3 at 32,768 spp against two 16,384-spp Mitsuba seeds has +0.000443% bias and 0.328% L1,
versus 0.576% seed-to-seed L1. See [U0b](m7-u0b.md). No reference images are altered or
recalibrated by MAT1.

An independent CPU integration of the actual source cube, at N=(0,1,0), roughness 0.85,
V=(sqrt(1âˆ’nvÂ²),nv,0), and albedo 0.1 gives these RGB-red radiance examples:

| NÂ·V | Direct environment specular integral | Runtime split-sum model | Total predicted runtime/reference ratio |
|---:|---:|---:|---:|
| 0.15 | 378.47 | 0.02 | 0.796 |
| 0.30 | 316.65 | 0.27 | 0.823 |
| 0.50 | 271.81 | 5.85 | 0.848 |
| 0.80 | 252.06 | 76.83 | 0.898 |
| 1.00 | 289.29 | 244.46 | 0.975 |

These are unoccluded material integrals, not quarry pixel predictions. They exclude the
separate sun and use midpoint source-cube quadrature; actual matched GPU results follow.

The GPU control uses an **untextured** albedo-0.1, roughness-0.85 plane, 17Ã—17 pixels,
and a 16,384-spp depth-2 reference. Centre-pixel red-channel results, in cd/mÂ²:

| Sky | NÂ·V | Reference Â± split-half SE | Standard | 256-stratum probe |
|---|---:|---:|---:|---:|
| Constant 16,384 | 0.30 | 2309.10 Â± 6.03 | 2306.00 | same constant-sky path |
| Constant 16,384 | 0.80 | 1983.41 Â± 0.17 | 1985.00 | same constant-sky path |
| Quarry RLE2 | 0.30 | 1788.32 Â± 4.40 | 1467.00 | 1784.00 |
| Quarry RLE2 | 0.80 | 1722.75 Â± 4.27 | 1543.00 | 1720.00 |

The constant-sky control passes its 1% plus four-SE tolerance. The mapped fixture
reproduces the 0.820/0.896 response ratios without any material textures, while the
integrated probe reads 0.998/0.998. `fixture-rt/sky-fixture.json` retains all channels,
all three cubature resolutions and split-half errors. This isolates the environment
specular approximation from the scan's material sampling.

## Candidate and rejection

`scripts/mat1-material.py prepare MANIFEST` generates a diagnostic program that replaces
only the standard program's environment specular expression. It integrates the actual
GGX/Schlick response over fixed radiance-importance strata of the source cube. Diffuse,
direct lights, geometry, material sampling, exposure, shadows and the black backdrop are
unchanged. The generated scene-specific sample tables stay in ignored evidence on E:.
They are measurement probes, not public renderer assets or a general environment API.

For pose 0, the open-ground placement medians move as follows (RT, 1Ã— anisotropy):

| Placement | Pixels | Baseline/reference luminance | Probe/reference luminance |
|---:|---:|---:|---:|
| 1 | 1,456 | 0.862 | 0.951 |
| 3 | 16,133 | 0.858 | 0.960 |
| 4 | 4,326 | 0.886 | 0.935 |
| 6 | 2,444 | 0.831 | 0.980 |
| 9 | 1,195 | 0.724 | 0.996 |
| 11 | 2,406 | 0.730 | 0.980 |
| 12 | 10,953 | 0.741 | 0.969 |

**The full-population gate fails.** At pose 0, all-covered RMS improves 11.629 â†’ 9.775
codes, while occluded worsens 28.259 â†’ 28.817, solid-occluded 27.691 â†’ 28.189, and
silhouette 27.023 â†’ 27.194. Pose 1 repeats all three regressions. These populations
already receive too much unoccluded lighting with the cache off. Restoring more of the
physical specular integral without its scene visibility makes that error worse.

The correction still misses Lumen HW Epic's 8.410-code all-covered floor. A lower
whole-image average does not satisfy the requirement to improve or preserve every
population.

Convergence and filtering do not remove the failure:

| RT, 16Ã— anisotropy | Baseline | 64 strata | 256 strata | 1,024 strata |
|---|---:|---:|---:|---:|
| Pose 0 all-covered RMS | 11.5577 | 9.6103 | 9.6169 | 9.6158 |
| Pose 1 all-covered RMS | 11.6190 | 9.6710 | 9.6783 | 9.6773 |
| Pose 0 occluded RMS | 28.2345 | 28.7757 | 28.7667 | 28.7578 |

16Ã— anisotropy alone also fails the every-population gate at the brighter exposure.
It is not promoted as a workaround. No texture LOD is forced to zero in the renderer.

## Measurement protocol and retained artifacts

RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Quarry: 13,034,903 source triangles, twelve 4K
base textures, registered poses 0 and 1, 1920Ã—1080, 1Ã— sampling, full resident geometry,
RLE2 `sun128.rle2`, separate 0.04-radian sun, black camera background, exposure âˆ’14 stops.
Cache off. RT uses traced primaries and Interactive shadows; non-RT uses the software
visibility buffer and Virtual Surface shadows. Explicit table labels distinguish
1Ã— and 16Ã— anisotropic filtering.

Input identities: RLA SHA-256
`1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`;
RLE2 `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764`;
pose-0 reference archive `fd2152fa7b578c03987391caf6ecffa54daabf07f60b3651876e72d5a0b0ec50`.
The first capture binary is `310129855883708b069d0f884db0e185dd6886193b3f85f05e227cd927683007`;
the final CPU/control binary is
`1bf1516a9982fad47e293f142a8d29df471e21258ad26519237f9595d6af4225`.

Scores reuse the qualified U0b 4,096-spp full-radiance archives, their split halves and
all twenty nonempty registered populations. They use the frozen F0 per-channel
Reinhard+sRGB transform at âˆ’14 and âˆ’10 stops, with unrounded code-space RGB RMS, as in
G1c/U0b. No mask or exposure is fitted to a candidate. These are functional headless
leases with active game/process/RAM guards, not performance measurements. Vulkan
validation is disabled for these quality captures.

Evidence root: `E:/renderlab-work/mat1/`. Retain the score JSONs, fixture results,
Mitsuba result, CPU/GPU process logs, generation/replay scripts, identity manifest and
inspected PNGs. Canonical U0b reference archives are shared retained references and
are not deleted. Own raw f32 captures and own Cargo target are deleted after scoring.

Reproduction:

```powershell
# MANIFEST has asset, cameras, environment, lighting and output absolute paths.
py -3 -B scripts/mat1-material.py prepare MANIFEST
# Compile the ignored probes outside GPU leases, with two jobs and the shared QUIET guard.
cargo test -p renderlab --lib --no-run -j 2 --target-dir E:/renderlab-work/mat1/target
# mat1-run.py queues inside its process, takes the shared lock and releases it before exit.
py -3 -B scripts/mat1-run.py LABEL GENERATED_MANIFEST --binary TEST_BINARY --path rt
py -3 -B scripts/mat1-run.py LABEL GENERATED_MANIFEST --binary TEST_BINARY --path nonrt
py -3 -B scripts/mat1-material.py score CAPTURES REFERENCE_DIRECTORY SCORE_JSON
# Run through the existing Mitsuba venv's python.exe, with -I:
python.exe -I scripts/mat1-mitsuba.py MITSUBA_RESULT_JSON
```

The test harness uploads the quarry once per process and reuses it for every program
and both poses. It changes programs through the ordinary programmable-material reload
API. No receiver/hit/dispatch data is specialized to a BRDF.

## Integration consequence

The standard program and reference tracer retain their exact production sources, so
**TR1b's and R0a's existing numbers are not invalidated by this branch**. Their full-image
numbers still include this known runtime environment-specular approximation error.
Changing the standard response later will require recapturing their Renderlab radiance
candidates; the reference images remain valid if the material and lighting definitions
are retained.

A safe default fix needs the actual specular integral together with world-space
environment/reflection visibility, or another measured correction that passes every
population. That transport work belongs with the reflection owner. MAT1 does not modify
GI, temporal or shadow files to hide the failed gate, and does not lower the quality bar.

## Verification

`cargo test -p renderlab --lib` exits **0**: **137 passed, 132 ignored**. CPU tests take
1.61 s; the measured Cargo invocation including incremental compilation takes 73.82 s.
Formatting is checked with `cargo fmt --all -- --check`.

The first M5 GPU photometry attempt completed its numeric body but failed the required
validation check: the launcher had disabled Khronos validation and the Vulkan loader
found stale PRISM/Overwolf implicit layer registrations. Its failed log remains in
`runs/controls-c/run-1`. The launcher now supports `--validate`, enabling Khronos and
using an owned empty implicit-layer directory. This changes only the launched process's
environment, not the machine's registry or installed layers.

The corrected M5 direct-light photometry and constant-environment compatibility runs
then **pass on both the RT and non-RT suite devices, at 1× and 4×**, with active Khronos
validation, zero errors and zero missing layers. Their logs are
`runs/nonrt-d/run-1`, `runs/nonrt-d/run-2`, `runs/rt-m5-e/run-0` and
`runs/rt-m5-e/run-1`. These exercise the unchanged production program; they do not
certify the rejected cubature probe as a general M5 replacement. The separate sky
fixture above compares that probe against the path tracer.

Full-frame reference/baseline/probe images were inspected at the registered pose, including
the ground, rock silhouettes, recesses and contact shadows. The reference is smoother at
edges, and the probe visibly restores the ground and broad rock response while leaving
the existing visibility differences. There is no native window, desktop capture or input
automation in MAT1.

## Every registered population: rejected probe

All values are RGB RMS in F0 codes at -14 / -10 stops. Before is the unchanged
standard program; probe is the rejected 256-stratum numerical integral. Both use
1× anisotropy and the same frozen reference. Bold probe cells worsen at least one
exposure. No population is omitted, and these values are not a promoted default.

Pose 1's non-RT background-centre change is smaller than the three-decimal table:
1.247676 → 1.247742 at −14 stops and 3.732891 → 3.732931 at −10. It is recorded
as a regression too; the large occluded/silhouette misses already reject promotion.

### Pose 0

| Population | Pixels | RT before | RT probe | Non-RT before | Non-RT probe |
|---|---:|---:|---:|---:|---:|
| all-covered | 346,517 | 11.629/18.400 | 9.775/14.977 | 11.640/18.429 | 9.787/15.009 |
| background-centre | 1,727,083 | 1.284/3.840 | 1.284/3.840 | 1.276/3.827 | 1.276/3.826 |
| backlit | 11,824 | 18.457/36.253 | 14.967/29.010 | 18.451/36.270 | 14.965/29.036 |
| contact | 14,438 | 19.022/34.075 | 17.518/30.341 | 19.030/34.117 | 17.527/30.381 |
| edge | 14,929 | 22.600/50.568 | 22.378/49.881 | 22.554/50.457 | 22.312/49.726 |
| faint | 24,549 | 13.681/20.424 | 12.005/17.078 | 13.687/20.430 | 11.994/17.033 |
| image | 2,073,600 | 4.896/8.298 | 4.164/7.055 | 4.899/8.304 | 4.167/7.059 |
| isolated | 1,290 | 14.332/22.016 | 12.448/17.961 | 14.332/22.020 | 12.449/17.967 |
| lit | 275,725 | 8.888/12.973 | 6.172/7.870 | 8.901/13.009 | 6.192/7.931 |
| multi-surface | 7,153 | 20.098/31.997 | 19.560/29.885 | 20.106/32.017 | 19.527/29.789 |
| narrow | 8,076 | 16.655/29.821 | 14.980/25.899 | 16.719/29.984 | 15.052/26.079 |
| occluded | 15,508 | 28.259/48.748 | **28.817/48.492** | 28.265/48.775 | **28.820/48.514** |
| partial | 55,284 | 15.156/24.820 | 13.265/20.914 | 15.166/24.846 | 13.271/20.928 |
| silhouette | 621 | 27.023/48.057 | **27.194/47.059** | 27.033/48.102 | **27.198/47.082** |
| single-surface | 339,364 | 11.383/18.003 | 9.460/14.499 | 11.394/18.033 | 9.474/14.536 |
| solid | 291,233 | 10.831/16.908 | 8.960/13.559 | 10.842/16.938 | 8.974/13.597 |
| solid-lit | 271,806 | 8.767/12.784 | 5.985/7.576 | 8.781/12.821 | 6.006/7.640 |
| solid-occluded | 14,612 | 27.691/47.990 | **28.189/47.666** | 27.695/48.014 | **28.192/47.689** |
| wide | 47,208 | 14.885/23.860 | 12.949/19.937 | 14.884/23.857 | 12.942/19.914 |
| wide6 | 33,148 | 14.625/23.170 | 12.533/18.998 | 14.619/23.153 | 12.518/18.954 |

### Pose 1

| Population | Pixels | RT before | RT probe | Non-RT before | Non-RT probe |
|---|---:|---:|---:|---:|---:|
| all-covered | 345,882 | 11.688/18.548 | 9.833/15.123 | 11.692/18.558 | 9.839/15.139 |
| background-centre | 1,727,718 | 1.250/3.738 | 1.250/3.738 | 1.248/3.733 | **1.248/3.733** |
| backlit | 11,825 | 18.671/36.911 | 15.196/29.657 | 18.652/36.870 | 15.178/29.622 |
| contact | 14,420 | 18.992/34.293 | 17.448/30.423 | 19.004/34.282 | 17.463/30.426 |
| edge | 14,850 | 22.320/49.724 | 22.090/49.026 | 22.331/49.721 | 22.103/49.018 |
| faint | 24,414 | 13.866/20.877 | 12.189/17.553 | 13.878/20.910 | 12.209/17.626 |
| image | 2,073,600 | 4.908/8.308 | 4.175/7.056 | 4.909/8.310 | 4.177/7.060 |
| isolated | 1,312 | 13.822/19.773 | 11.818/15.081 | 13.980/20.153 | 12.102/16.215 |
| lit | 275,262 | 8.946/13.103 | 6.244/8.045 | 8.949/13.115 | 6.249/8.064 |
| multi-surface | 7,184 | 20.719/32.949 | 20.190/30.881 | 20.734/33.004 | 20.212/30.936 |
| narrow | 7,882 | 16.200/29.053 | 14.475/24.981 | 16.247/29.122 | 14.526/25.057 |
| occluded | 15,454 | 28.241/48.682 | **28.826/48.495** | 28.241/48.686 | **28.827/48.498** |
| partial | 55,166 | 15.289/25.225 | 13.375/21.265 | 15.299/25.238 | 13.391/21.299 |
| silhouette | 628 | 27.548/48.593 | **27.635/47.450** | 27.537/48.591 | **27.625/47.445** |
| single-surface | 338,698 | 11.419/18.119 | 9.492/14.606 | 11.423/18.127 | 9.497/14.620 |
| solid | 290,716 | 10.871/16.987 | 9.005/13.649 | 10.873/16.996 | 9.008/13.660 |
| solid-lit | 271,305 | 8.833/12.927 | 6.071/7.773 | 8.837/12.940 | 6.077/7.793 |
| solid-occluded | 14,555 | 27.624/47.884 | **28.136/47.594** | 27.624/47.884 | **28.136/47.594** |
| wide | 47,284 | 15.132/24.529 | 13.183/20.581 | 15.135/24.531 | 13.192/20.606 |
| wide6 | 33,140 | 14.826/23.700 | 12.705/19.466 | 14.824/23.687 | 12.710/19.486 |

AA state (IQ1, 2026-11-08; [iq1-quarry-image.md](iq1-quarry-image.md)): this unit rendered 1x with TR1 off; anisotropy 1 is the shipping default, and the 16x rows are labelled.
