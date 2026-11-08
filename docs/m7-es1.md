# M7-ES1: accurate environment specular with cache occlusion

Branch `t3/m7-es1`, from main `50c35da` (MAT1's diagnosis), main `4e65cd1` merged. Builds on [MAT1](mat1-quarry-material.md), [G1c](m7-g1c.md), the design's `indirect_radiance` input ([m7-design.md](m7-design.md) 3.2) and P0's cache constraints ([m7-p0-claims.md](m7-p0-claims.md)). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## Status

| Item | Result | Verdict |
|---|---|---|
| 1. Accurate environment specular, no per-pixel quadrature | The RLE2 map's dominant lobes are fitted at upload and evaluated against the program's own GGX BRDF; their N=V prefiltered share is subtracted from the prefiltered field. Sky-lit plane against the path tracer: median ratio **0.804 → 0.998**, 10 → 70 of 96 cases within 1% + 4 SE. Quarry open ground: 0.72-0.89 → **0.84-0.99** with the cache on (MAT1's numerical probe, cache off: 0.94-1.00). | **Pass** where the cache runs: dielectrics at every tested roughness (0.89-1.01), metals at r ≥ 0.85; **miss** on glossy metals (r 0.3-0.5: 0.60-1.02) |
| 2. Sky specular occlusion through `indirect_radiance` | The cache's occluded-over-open irradiance at the receiver scales the prefiltered radiance and every lobe; computed once per receiver; no cache-file change. Quarry occluded **20.71 → 18.29**, silhouette **20.20 → 17.87** (RT, -14 stops, against G1c's cache-on image); both far below the cache-off 28.26 / 27.02. | **Pass** against both baselines on those populations |
| 3. Default image | The cache is off by default (`IndirectSettings::enabled`). Without it the standard program takes exactly its old arithmetic: **RT bit-identical** at both poses; non-RT equal to a repeated baseline within one pixel (section 4.5). | **Unchanged** (no population could improve or regress) |
| 4. Every population, cache on | ES1 equals or beats G1c's cache-on image on **all 20 populations at both exposures, both poses, both paths**, static. Moving (U0b's pan, four witnesses pooled): every population except `isolated` at -10 stops (RT 34.40 → 35.68, non-RT 31.96 → 32.87; -14 improves). Against the cache-off default, the cache-on image still loses lit and partial populations: G1c's open-ground bias and the missing bounce (section 5). | **Cache-on mode improved; not a default candidate yet** |
| 5. Cost | Quiet paired run: **+0.20-0.25 ms** of shading with the cache on (both paths), **+0.02-0.03 ms** on the cache-off default. One cache lookup per receiver, shared by every ES1 input; the lobe evaluation itself costs nothing measurable. | **Miss** against Lumen's 0.07 ms reflection pass; the default's +0.02 is recorded |
| 6. Gates | `cargo test -p renderlab --lib` exits 0 on the merged code (141 passed, 141 ignored); every retained M5 environment and photometry gate passes on both suites with active validation; fmt and clippy clean in ES1's files. | **Pass** |

## 1. Method

### 1.1 Why the split sum fails here

MAT1 traced the quarry's 11-27% darkening to `prefiltered(reflect(-V,N), r) * (F0 A + B)`, whose RLE2 prefilter assumes N = V. The registered `sun128` map is, exactly, a dark sky of RGB (0.0195, 0.0396, 0.0996) plus a `cos^128` lobe of peak (2.0, 1.6, 1.0)e6 cd/m² toward (0.36, 0.48, 0.80) (M, CPU: 99.96% of the up-facing irradiance lies within 20° of that axis). A rough GGX lobe at grazing view picks this source up far from the mirror direction, where the N=V kernel has almost no weight.

A CPU replay over 20,936 covered quarry pixels at pose 0 (geometric normals and eye from U0b's reference archive, roughness 0.85, F0 0.04, exact quadrature over the source cube) quantifies it (M, `E:/renderlab-work/m7-es1/cpu/quarry_replay.py`):

| Estimate of the environment specular | Ratio to exact, quantiles 5 / 25 / 50 / 75 / 95% |
|---|---|
| Runtime split sum | 0.000 / 0.000 / 0.086 / 0.629 / 1.16 |
| Dominant lobe evaluated against the BRDF, its N=V share subtracted (ES1) | 1.002 / 1.002 / 1.003 / 1.006 / 1.014 |

At the median pixel the split sum keeps 9% of the specular integral; with albedo 0.15 the pixel total reads 0.92 (p1 0.81). The cooked prefilter and an exact N=V convolution of the fitted lobe agree within ±1.6% (p1-p99), so the subtraction is consistent with the cooked field.

### 1.2 Dominant lobes (`src/environment_lobes.rs`)

At upload, `try_upload_environment` fits up to four lobes `G(l) = A exp(lambda (l.s - 1))` to an RLE2 map's knot 0 (the unblurred source), greedily and deterministically:

- residual = source minus its per-channel minimum (the cooker's constant control);
- the brightest residual texel seeds a lobe; von Mises-Fisher moments within the cap `l.s >= 1 - 12/lambda` give the axis and the sharpness (`lambda = 1/(1 - mean resultant length)`), starting from the half-maximum solid angle; a sub-texel source is capped at a texel's sharpness;
- the amplitude matches the cap's flux;
- fitting stops at the first candidate that is too broad (sharpness below 32, about a 12° half width; the split sum handles broader structure better) or too faint (under 2% of the residual's luminous flux). Constant and smooth fields get no lobe (CPU tests).

On `sun128` the fit recovers axis (0.36, 0.48, 0.80), sharpness 129.9 (cos^128 is vMF 129 to first order) and amplitude 2.009e6 (M, `es1_lobe_fit_cost`).

Each lobe's **share** of the N=V prefiltered field is tabulated exactly: the cooker's normalized kernel `D(h) max(R.l,0)/4 / Z` (Z from `crates/cook/src/environment/specular.rs`) averaged over the lobe, by 48² fixed strata of whichever of kernel and lobe is narrower, at every one of the 65 roughness knots and 128 angle bins uniform in sqrt(angle/pi). Knot 0 is the lobe's own shape. A dense quadrature agrees within 1% (CPU test). The table is an `Rgba32Float` 128 x 66 texture, lobe k in channel k, row 0 holding the axes, sharpness, amplitudes, solid angles and the count; lighting group 3 binding 10. RLE1 mip chains and fields without lobes bind a 9 x 1 empty table.

### 1.3 Standard program (`shaders/programs/standard.wgsl`)

```
specular = environment_product(indirect_radiance(R, r), F0 A + B)
         + sum_k w_k * [ environment_product(lobe_k.illuminance, f_GGX'(n, v, s_k))
                       - environment_product(environment_lobe_reflection(k, R, r), F0 A + B) ]
```

- `f_GGX'` is the program's own GGX/Schlick/height-correlated-Smith response at the lobe axis, with exact Fresnel. The lobe's width widens the NDF: `alpha'^2 = alpha^2 + 1/(2 lambda v.h)` (the light's variance mapped to half-vector slopes, geometric mean of the two axes). On the quarry this point evaluation matches the exact lobe integral within 1.4% at p95.
- `w_k = clamp(0.4 + 0.4 log10(lambda alpha^2), 0, 1)`: full replacement when the specular lobe is much wider than the light (`lambda alpha^2 >= 31.6`; the quarry's 0.85 roughness gives 67), none when it is much narrower (`<= 0.1`), where the N=V prefilter is the better estimate. It was fitted on a CPU grid of 1,008 configurations (roughness 0.1-1, sharpness 16-8,192, four views, nine lobe directions) against MIS Monte Carlo; it beats the split sum's RMS in every cell.
- With no lobes (cache off, RLE1, smooth maps) the expression reduces to the old one, operation for operation.

**Rejected on the same grid (M, CPU):** a weight that also grows with the lobe's angle from the reflection (mean cell RMS 0.029 → 0.087-0.32, worst cell 0.065 → 0.26-3.4: a broad light's tail still overlaps a narrow BRDF's peak), a point-sampled control variate `share + f(s) - k(s) DFG` (0.079-0.152, worst 0.48-1.58), and tail-preserving widenings (exp/rational transitions; no better). The fitted rule's worst case is a lobe exactly in the mirror direction at 80° view (+20% at r 0.85, lambda 128).

### 1.4 Occlusion (`shaders/environment_lobes.wgsl`)

The engine inputs (each composed only for programs that call one of them):

| Input | Value |
|---|---|
| `indirect_radiance(input, direction, roughness)` | The prefiltered radiance about `direction` (or the constant environment) times this receiver's sky visibility. Without the cache, exactly `environment_reflection` / `environment_constant`. Units as those (before intensity and exposure) |
| `environment_lobe_count()` | The lobes the engine separates; **0 without the world radiance cache**, which alone can occlude them, so they then stay inside the prefiltered value |
| `environment_lobe(input, i)` | World axis, sharpness, and occluded illuminance (radiance integrated over the sphere) |
| `environment_lobe_reflection(input, i, direction, roughness)` | Lobe i's part of `indirect_radiance(input, direction, roughness)`, interpolated across knots exactly as `environment_radiance` brackets them |

**Visibility** = `clamp(indirect_irradiance(n) / open irradiance(n), 0, 1)` per channel, at the receiver's vertex normal on its viewed side: the existing G1-G1c lookup (fixed weights, fixed query points, no jitter, no counters, no ranking; W1-W3, C1-C5), read through `indirect_irradiance` only. For a concentrated sky it is the visibility of the dominant lobe; for an even sky, cosine-weighted sky visibility. Because the cache's open-sky scale makes its unoccluded value exact at the cell normal, an open receiver reads about 1.

**Once per receiver.** The generated entry calls `rl_indirect_prepare(input)` before `surface()` (traced) or before `shade()` (raster): `es_prepare` for programs that read the ES1 inputs, an empty function otherwise (`src/program.rs`). A first version called the lookup from each input; its three inlined copies cost +0.53 ms of quarry shading with the cache on and +0.12 ms with it off (M, functional), against +0.00 for the lobe math alone.

## 2. Programmability constraints (recorded)

1. **The lobe inputs are BRDF-free** (axis, sharpness, illuminance), so any program can shade them like lights. `environment_lobe_reflection` is defined against the N=V GGX prefilter, as `environment_reflection` already is.
2. **Lobes exist only with the world radiance cache.** Without it a program sees `environment_lobe_count() == 0` and the old prefiltered value. This keeps the default image unchanged: an unoccluded accurate integral fails MAT1's occluded populations.
3. **Specular visibility is cosine-weighted at the vertex normal**, shared by the prefiltered radiance and every lobe; it is not directional within the lobe. Glossy reflections belong to R1's traced bands.
4. **One engine hook per receiver** (`rl_indirect_prepare`, engine-private): new entries that call `shade()` must call it too. R0b's reflection probe embeds the standard program with stub inputs; ES1 adds matching stubs (`shaders/reflection_probe.wgsl`, exactly the old response).
5. RLE1 legacy maps get no lobes.

## 3. Sky-lit plane against the path tracer (both paths, M)

The M5-style material fixture: an open 200 m plane under `sun128` only, 17 x 17 pixels at a 0.01 rad field of view, scored at its centre against a 16,384-spp depth-2 reference (the reference tracer, RT process; the non-RT process reads the same references). Dielectric (base 0.1) and metal (base 0.9), roughness 0.3/0.5/0.85/1.0, N.V 0.15/0.3/0.6/0.95, three azimuths (beside the sun, toward its glint, sun behind): 96 cases. Final binary `es1-i`, run `final`. Luminance ratio to the reference:

| Material | Roughness | Default (cache off): range (median) | ES1, cache on: range (median) | ES1 within 1% + 4 SE |
|---|---:|---|---|---:|
| Dielectric 0.1 | 0.3 | 0.424-0.993 (0.988) | 0.888-0.999 (0.995) | 9/12 |
| Dielectric 0.1 | 0.5 | 0.308-0.974 (0.949) | 0.948-0.994 (0.989) | 10/12 |
| Dielectric 0.1 | 0.85 | 0.548-1.044 (0.868) | 0.998-1.007 (0.999) | 12/12 |
| Dielectric 0.1 | 1.0 | 0.755-1.073 (0.889) | 0.998-0.999 (0.999) | 12/12 |
| Metal 0.9 | 0.3 | 0.000-0.808 (0.231) | 0.596-1.018 (0.810) | 1/12 |
| Metal 0.9 | 0.5 | 0.000-0.889 (0.276) | 0.804-1.014 (0.883) | 3/12 |
| Metal 0.9 | 0.85 | 0.000-1.358 (0.414) | 1.006-1.024 (1.010) | 11/12 |
| Metal 0.9 | 1.0 | 0.000-1.837 (0.440) | 1.006-1.012 (1.010) | 12/12 |

RT and non-RT read identically (the plane is open, so visibility is 1 on both). Cache off, ES1 equals the baseline in every case. With the cache on, **no case is further from the reference than the default**, on either path. **Glossy metals miss** the bar nonetheless: with the sun behind the camera the reference sees it through the GGX tail, which `w_k` (0.41 at r 0.3) only partly takes from the point evaluation (0.60-0.81 there).

## 4. Quarry, every registered population (M)

### 4.1 Protocol

U0b's full-radiance protocol, as MAT1 and G1c: RTX 4070 SUPER, NVIDIA 617.14, Vulkan; quarry RLA `1ce53c38...` (13,034,903 triangles, twelve 4K textures), poses 0 and 1, 1920x1080, 1x, `sun128.rle2` (`3ccd61b1...`), the separate 0.04 rad sun on the 4/12/40 m splits, black camera background, exposure -14. RT: traced primaries and Interactive shadows; non-RT: the visibility buffer and its Virtual shadows, with G1c's cooked 1 cm near field (`743a2fce...`). Cache on means a reset and eight settle frames. One process per path renders the baseline standard source (main `50c35da`), ES1 and the baseline again (`--repeat`). Scores: U0b's 4,096-spp `quarry-p0/p1` archives (`fd2152fa...` for pose 0) and 256-spp pan witnesses, every nonempty registered population, unrounded RGB RMS in F0 codes at -14 / -10 stops (`scripts/m7-es1-score.py`, U0b's frozen transform). Lumen columns are U0b's static Epic floors (pose 0) and its Q-walk motion floors (70 witnesses; a different frame set from the pan, so indicative only). Final binary `es1-i`, run `final`; functional leases, not timing.

Bold marks an ES1 cell worse than G1c's cache-on image at either exposure.

### 4.2 Static: RT

**Pose 0.**

| Population | Pixels | Default (cache off) | G1c cache on | ES1 cache on | Lumen HW Epic | Lumen SW Epic |
|---|---|---|---|---|---|---|
| all-covered | 346,517 | 11.63/18.40 | 14.22/22.03 | 12.17/18.82 | 8.41/11.65 | 9.32/13.99 |
| background-centre | 1,727,083 | 1.28/3.84 | 1.28/3.84 | 1.28/3.84 | 0.33/1.01 | 0.33/1.03 |
| backlit | 11,824 | 18.46/36.25 | 22.79/46.33 | 19.07/39.23 | 9.93/17.08 | 11.40/20.62 |
| contact | 14,438 | 19.02/34.07 | 21.90/42.25 | 19.56/38.07 | 13.42/22.17 | 14.02/23.59 |
| edge | 14,929 | 22.60/50.57 | 24.32/53.82 | 23.34/52.60 | 11.28/20.20 | 13.83/25.54 |
| faint | 24,549 | 13.68/20.42 | 19.31/28.41 | 18.33/27.26 | 9.90/12.29 | 10.39/13.65 |
| image | 2,073,600 | 4.90/8.30 | 5.93/9.66 | 5.11/8.45 | 3.45/4.85 | 3.82/5.79 |
| isolated | 1,290 | 14.33/22.02 | 21.51/32.40 | 20.38/31.10 | 8.11/9.38 | 8.08/9.40 |
| lit | 275,725 | 8.89/12.97 | 12.36/17.28 | 10.16/13.79 | 7.39/9.39 | 7.49/9.78 |
| multi-surface | 7,153 | 20.10/32.00 | 24.79/42.70 | 23.50/40.45 | 11.95/18.55 | 14.24/23.67 |
| narrow | 8,076 | 16.65/29.82 | 20.71/38.30 | 18.71/34.72 | 12.09/17.39 | 12.66/18.87 |
| occluded | 15,508 | 28.26/48.75 | 20.71/42.07 | 18.29/37.48 | 14.53/26.40 | 22.38/41.75 |
| partial | 55,284 | 15.16/24.82 | 19.65/32.50 | 17.87/29.61 | 10.59/14.66 | 11.14/16.14 |
| silhouette | 621 | 27.02/48.06 | 20.20/42.00 | 17.87/37.70 | 13.21/26.80 | 20.72/40.36 |
| single-surface | 339,364 | 11.38/18.00 | 13.91/21.38 | 11.81/18.08 | 8.32/11.46 | 9.18/13.71 |
| solid | 291,233 | 10.83/16.91 | 12.94/19.42 | 10.75/15.97 | 7.93/10.99 | 8.93/13.54 |
| solid-lit | 271,806 | 8.77/12.78 | 12.20/17.00 | 9.97/13.44 | 7.35/9.33 | 7.43/9.66 |
| solid-occluded | 14,612 | 27.69/47.99 | 19.86/40.39 | 17.30/35.47 | 14.25/25.84 | 22.18/41.36 |
| wide | 47,208 | 14.88/23.86 | 19.46/31.40 | 17.72/28.64 | 10.31/14.14 | 10.86/15.63 |
| wide6 | 33,148 | 14.62/23.17 | 19.80/31.44 | 18.22/29.02 | 9.92/13.64 | 10.24/14.60 |

**Pose 1.**

| Population | Pixels | Default (cache off) | G1c cache on | ES1 cache on |
|---|---|---|---|---|
| all-covered | 345,882 | 11.69/18.55 | 14.26/22.07 | 12.20/18.86 |
| background-centre | 1,727,718 | 1.25/3.74 | 1.25/3.74 | 1.25/3.74 |
| backlit | 11,825 | 18.67/36.91 | 23.00/46.89 | 19.35/39.92 |
| contact | 14,420 | 18.99/34.29 | 22.36/43.21 | 20.13/39.19 |
| edge | 14,850 | 22.32/49.72 | 24.03/52.68 | 23.03/51.47 |
| faint | 24,414 | 13.87/20.88 | 19.24/28.30 | 18.20/27.19 |
| image | 2,073,600 | 4.91/8.31 | 5.93/9.64 | 5.11/8.42 |
| isolated | 1,312 | 13.82/19.77 | 20.60/30.00 | 19.64/29.13 |
| lit | 275,262 | 8.95/13.10 | 12.38/17.33 | 10.19/13.82 |
| multi-surface | 7,184 | 20.72/32.95 | 24.72/42.42 | 23.30/39.98 |
| narrow | 7,882 | 16.20/29.05 | 21.01/38.74 | 19.08/35.26 |
| occluded | 15,454 | 28.24/48.68 | 20.44/41.32 | 18.05/36.95 |
| partial | 55,166 | 15.29/25.23 | 19.80/32.80 | 18.01/29.90 |
| silhouette | 628 | 27.55/48.59 | 19.64/40.90 | 17.08/36.22 |
| single-surface | 338,698 | 11.42/18.12 | 13.95/21.43 | 11.86/18.15 |
| solid | 290,716 | 10.87/16.99 | 12.94/19.37 | 10.75/15.92 |
| solid-lit | 271,305 | 8.83/12.93 | 12.22/17.04 | 9.99/13.47 |
| solid-occluded | 14,555 | 27.62/47.88 | 19.74/39.79 | 17.25/35.16 |
| wide | 47,284 | 15.13/24.53 | 19.60/31.71 | 17.83/28.91 |
| wide6 | 33,140 | 14.83/23.70 | 19.91/31.58 | 18.29/29.16 |

### 4.3 Static: non-RT

**Pose 0.**

| Population | Pixels | Default (cache off) | G1c cache on | ES1 cache on | Lumen HW Epic | Lumen SW Epic |
|---|---|---|---|---|---|---|
| all-covered | 346,517 | 11.64/18.43 | 13.35/21.07 | 11.38/18.08 | 8.41/11.65 | 9.32/13.99 |
| background-centre | 1,727,083 | 1.28/3.83 | 1.28/3.83 | 1.28/3.83 | 0.33/1.01 | 0.33/1.03 |
| backlit | 11,824 | 18.45/36.27 | 22.25/45.23 | 18.52/38.22 | 9.93/17.08 | 11.40/20.62 |
| contact | 14,438 | 19.03/34.12 | 21.85/42.01 | 19.60/38.09 | 13.42/22.17 | 14.02/23.59 |
| edge | 14,929 | 22.55/50.46 | 23.81/53.29 | 22.85/52.06 | 11.28/20.20 | 13.83/25.54 |
| faint | 24,549 | 13.69/20.43 | 18.42/27.63 | 17.51/26.70 | 9.90/12.29 | 10.39/13.65 |
| image | 2,073,600 | 4.90/8.30 | 5.58/9.30 | 4.80/8.18 | 3.45/4.85 | 3.82/5.79 |
| isolated | 1,290 | 14.33/22.02 | 18.34/28.12 | 17.38/27.07 | 8.11/9.38 | 8.08/9.40 |
| lit | 275,725 | 8.90/13.01 | 11.27/16.08 | 9.15/12.82 | 7.39/9.39 | 7.49/9.78 |
| multi-surface | 7,153 | 20.11/32.02 | 24.23/41.85 | 23.00/39.59 | 11.95/18.55 | 14.24/23.67 |
| narrow | 8,076 | 16.72/29.98 | 20.70/37.91 | 18.76/34.39 | 12.09/17.39 | 12.66/18.87 |
| occluded | 15,508 | 28.26/48.78 | 20.58/41.28 | 18.19/36.70 | 14.53/26.40 | 22.38/41.75 |
| partial | 55,284 | 15.17/24.85 | 19.09/31.87 | 17.36/29.19 | 10.59/14.66 | 11.14/16.14 |
| silhouette | 621 | 27.03/48.10 | 19.98/41.63 | 17.52/37.48 | 13.21/26.80 | 20.72/40.36 |
| single-surface | 339,364 | 11.39/18.03 | 13.02/20.41 | 11.00/17.35 | 8.32/11.46 | 9.18/13.71 |
| solid | 291,233 | 10.84/16.94 | 11.95/18.32 | 9.84/15.08 | 7.93/10.99 | 8.93/13.54 |
| solid-lit | 271,806 | 8.78/12.82 | 11.10/15.80 | 8.93/12.47 | 7.35/9.33 | 7.43/9.66 |
| solid-occluded | 14,612 | 27.69/48.01 | 19.83/39.70 | 17.33/34.90 | 14.25/25.84 | 22.18/41.36 |
| wide | 47,208 | 14.88/23.86 | 18.80/30.72 | 17.11/28.20 | 10.31/14.14 | 10.86/15.63 |
| wide6 | 33,148 | 14.62/23.15 | 18.97/30.53 | 17.45/28.43 | 9.92/13.64 | 10.24/14.60 |

**Pose 1.**

| Population | Pixels | Default (cache off) | G1c cache on | ES1 cache on |
|---|---|---|---|---|
| all-covered | 345,882 | 11.69/18.56 | 13.36/21.11 | 11.39/18.13 |
| background-centre | 1,727,718 | 1.25/3.73 | 1.25/3.73 | 1.25/3.73 |
| backlit | 11,825 | 18.65/36.87 | 22.50/46.07 | 18.85/39.24 |
| contact | 14,420 | 19.00/34.28 | 22.32/43.26 | 20.15/39.43 |
| edge | 14,850 | 22.33/49.72 | 23.51/52.24 | 22.56/51.07 |
| faint | 24,414 | 13.88/20.91 | 18.28/27.35 | 17.36/26.53 |
| image | 2,073,600 | 4.91/8.31 | 5.57/9.27 | 4.79/8.15 |
| isolated | 1,312 | 13.98/20.15 | 17.74/26.17 | 17.06/26.10 |
| lit | 275,262 | 8.95/13.12 | 11.27/16.09 | 9.13/12.80 |
| multi-surface | 7,184 | 20.73/33.00 | 24.19/41.91 | 22.84/39.55 |
| narrow | 7,882 | 16.25/29.12 | 21.00/38.86 | 19.14/35.51 |
| occluded | 15,454 | 28.24/48.69 | 20.50/40.82 | 18.19/36.50 |
| partial | 55,166 | 15.30/25.24 | 19.17/32.16 | 17.43/29.50 |
| silhouette | 628 | 27.54/48.59 | 19.44/40.64 | 16.63/35.67 |
| single-surface | 338,698 | 11.42/18.13 | 13.03/20.44 | 11.02/17.39 |
| solid | 290,716 | 10.87/17.00 | 11.94/18.27 | 9.83/15.03 |
| solid-lit | 271,305 | 8.84/12.94 | 11.10/15.80 | 8.93/12.45 |
| solid-occluded | 14,555 | 27.62/47.88 | 19.78/39.23 | 17.39/34.72 |
| wide | 47,284 | 15.13/24.53 | 18.84/30.90 | 17.13/28.38 |
| wide6 | 33,140 | 14.82/23.68 | 18.87/30.36 | 17.29/28.21 |

### 4.4 In motion: U0b's pan

The camera settles at pose 0, then moves 2 cm a frame as U0b's pan; captures at its witness frames 97, 112, 128 and 159 (256-spp references), RMS pooled over the four by pixel count. The cache updates while moving, as in a walk.

**RT.**

| Population | Pixels | G1c cache on | ES1 cache on | Lumen HW Epic Q-walk | Lumen SW Epic Q-walk |
|---|---|---|---|---|---|
| all-covered | 1,252,164 | 14.43/22.50 | 12.48/19.49 | 8.92/12.66 | 9.90/15.12 |
| background-centre | 7,042,236 | 1.23/3.66 | 1.23/3.66 | 0.40/1.24 | 0.41/1.25 |
| backlit | 41,054 | 23.41/48.01 | 19.88/41.44 | 11.70/20.70 | 13.34/24.33 |
| contact | 53,165 | 22.40/43.55 | 20.19/39.74 | 14.76/25.25 | 15.45/26.90 |
| edge | 56,203 | 24.00/53.03 | 23.09/51.92 | 12.14/22.82 | 14.63/27.86 |
| faint | 71,787 | 18.97/28.87 | 17.96/27.72 | 10.95/14.33 | 11.64/16.29 |
| image | 8,294,400 | 5.72/9.37 | 4.98/8.29 | 3.67/5.30 | 4.07/6.29 |
| isolated | 3,997 | 22.54/34.40 | **22.52/35.68** | 9.16/10.83 | 9.35/11.38 |
| lit | 1,015,911 | 12.77/17.92 | 10.75/14.80 | 7.81/10.05 | 8.01/10.73 |
| multi-surface | 25,657 | 24.53/42.27 | 23.36/40.38 | 12.86/20.54 | 15.10/25.45 |
| narrow | 30,680 | 21.16/38.70 | 19.42/35.68 | 13.37/19.84 | 14.11/21.73 |
| occluded | 55,183 | 20.72/42.79 | 18.32/38.07 | 15.22/28.67 | 23.31/43.74 |
| partial | 181,070 | 19.85/33.77 | 18.06/30.92 | 11.70/17.06 | 12.41/18.97 |
| silhouette | 2,125 | 19.82/40.12 | 17.77/36.11 | 13.97/28.68 | 21.18/41.69 |
| single-surface | 1,226,507 | 14.14/21.89 | 12.15/18.81 | 8.82/12.44 | 9.77/14.83 |
| solid | 1,071,094 | 13.30/19.97 | 11.27/16.81 | 8.34/11.72 | 9.40/14.35 |
| solid-lit | 1,000,501 | 12.60/17.59 | 10.54/14.40 | 7.77/9.97 | 7.95/10.60 |
| solid-occluded | 52,014 | 19.99/41.22 | 17.47/36.18 | 14.93/28.02 | 23.13/43.30 |
| wide | 150,390 | 19.57/32.67 | 17.76/29.86 | 11.37/16.50 | 12.08/18.42 |
| wide6 | 101,352 | 19.73/32.59 | 18.04/30.01 | 11.05/16.03 | 11.58/17.57 |

**Non-RT.**

| Population | Pixels | G1c cache on | ES1 cache on | Lumen HW Epic Q-walk | Lumen SW Epic Q-walk |
|---|---|---|---|---|---|
| all-covered | 1,252,164 | 13.59/21.64 | 11.73/18.88 | 8.92/12.66 | 9.90/15.12 |
| background-centre | 7,042,236 | 1.22/3.66 | 1.22/3.66 | 0.40/1.24 | 0.41/1.25 |
| backlit | 41,054 | 23.05/47.28 | 19.56/40.84 | 11.70/20.70 | 13.34/24.33 |
| contact | 53,165 | 22.50/43.71 | 20.37/40.02 | 14.76/25.25 | 15.45/26.90 |
| edge | 56,203 | 23.60/52.81 | 22.75/51.75 | 12.14/22.82 | 14.63/27.86 |
| faint | 71,787 | 18.52/28.51 | 17.67/27.77 | 10.95/14.33 | 11.64/16.29 |
| image | 8,294,400 | 5.40/9.06 | 4.70/8.07 | 3.67/5.30 | 4.07/6.29 |
| isolated | 3,997 | 20.62/31.96 | **20.54/32.87** | 9.16/10.83 | 9.35/11.38 |
| lit | 1,015,911 | 11.68/16.75 | 9.73/13.87 | 7.81/10.05 | 8.01/10.73 |
| multi-surface | 25,657 | 24.21/41.94 | 23.12/40.05 | 12.86/20.54 | 15.10/25.45 |
| narrow | 30,680 | 21.21/38.82 | 19.54/35.94 | 13.37/19.84 | 14.11/21.73 |
| occluded | 55,183 | 20.73/42.30 | 18.38/37.68 | 15.22/28.67 | 23.31/43.74 |
| partial | 181,070 | 19.52/33.44 | 17.82/30.87 | 11.70/17.06 | 12.41/18.97 |
| silhouette | 2,125 | 20.00/41.02 | 17.85/36.99 | 13.97/28.68 | 21.18/41.69 |
| single-surface | 1,226,507 | 13.28/21.01 | 11.37/18.18 | 8.82/12.44 | 9.77/14.83 |
| solid | 1,071,094 | 12.31/18.93 | 10.35/15.99 | 8.34/11.72 | 9.40/14.35 |
| solid-lit | 1,000,501 | 11.49/16.41 | 9.49/13.45 | 7.77/9.97 | 7.95/10.60 |
| solid-occluded | 52,014 | 20.06/40.83 | 17.61/36.00 | 14.93/28.02 | 23.13/43.30 |
| wide | 150,390 | 19.15/32.23 | 17.45/29.73 | 11.37/16.50 | 12.08/18.42 |
| wide6 | 101,352 | 19.19/31.93 | 17.62/29.74 | 11.05/16.03 | 11.58/17.57 |

`isolated` (about 1,000 pixels a frame) is the one regression: ES1 halves its bias toward zero (RT pose 0 -12.6 → -8.7 codes, pan -12.8 → -9.8) but adds pixel variance on small rocks, where the cache's cell-level visibility is coarse; against 256-spp references the variance wins at -10 stops.

### 4.5 Open-ground ratios and the default's identity

Median luminance ratio to the reference on open, up-facing, lit ground per placement (MAT1's mask), default / G1c cache on / ES1 cache on:

- rt pose 0: 1 0.862/0.793/0.875, 3 0.858/0.778/0.874, 4 0.886/0.803/0.849, 6 0.831/0.804/0.952, 9 0.724/0.707/0.980, 11 0.730/0.704/0.951, 12 0.741/0.695/0.925
- rt pose 1: 1 0.861/0.785/0.872, 3 0.858/0.779/0.875, 4 0.881/0.792/0.837, 6 0.830/0.801/0.950, 9 0.724/0.710/0.978, 11 0.730/0.700/0.950, 12 0.739/0.695/0.925
- nonrt pose 0: 1 0.862/0.797/0.881, 3 0.858/0.817/0.916, 4 0.886/0.862/0.910, 6 0.831/0.820/0.968, 9 0.724/0.716/0.988, 11 0.730/0.710/0.959, 12 0.741/0.715/0.945
- nonrt pose 1: 1 0.861/0.792/0.877, 3 0.858/0.818/0.918, 4 0.881/0.857/0.904, 6 0.830/0.818/0.967, 9 0.724/0.717/0.989, 11 0.730/0.707/0.958, 12 0.739/0.712/0.943

MAT1's numerical probe (cache off, unoccluded) read 0.94-1.00 on the same placements. ES1's residual darkness on placements 1, 3 and 4 is the cache's own open-ground bias (G1c section 5.3: -7% RT, -5% non-RT), which now reaches specular through the shared visibility; non-RT reads closer for the same reason as in G1c.

**The default is unchanged.** Cache off, ES1's quarry images equal the baseline's bit for bit on RT (both poses, and a repeated baseline). On non-RT, ES1 equals a repeated baseline rendered after it to within one pixel, while the first-rendered baseline differs from its own repeat by 58 / 32 pixels: run-order state of the non-RT path, present without ES1 (M, runs `a3` and `final`). The cache-on mode differs between processes by 3-98 pixels for every program (G1's cache), so its tables carry that noise.

## 5. Why the cache-on image still loses to the default on lit populations (M)

Against the cache-off default, ES1's cache-on image wins occluded, solid-occluded, silhouette and solid (RT pose 0: 28.26 → 18.29, 27.69 → 17.30, 27.02 → 17.87, 10.83 → 10.75) but loses lit, solid-lit, partial, faint, isolated, wide, narrow, multi-surface, contact and backlit. A diagnostic separates the two causes: the same process with the sky specular **unoccluded** (shader variant `open`, diffuse still cache-occluded):

**RT, pose 0.**

| Population | Default (cache off) | ES1 cache on | Open specular, cache on |
|---|---|---|---|
| all-covered | 11.63/18.40 | 12.17/18.82 | 10.31/14.08 |
| background-centre | 1.28/3.84 | 1.28/3.84 | 1.28/3.84 |
| backlit | 18.46/36.25 | 19.07/39.23 | 16.35/30.58 |
| contact | 19.02/34.07 | 19.56/38.07 | 16.47/28.93 |
| edge | 22.60/50.57 | 23.34/52.60 | 20.77/47.56 |
| faint | 13.68/20.42 | 18.33/27.26 | 15.24/19.20 |
| image | 4.90/8.30 | 5.11/8.45 | 4.37/6.74 |
| isolated | 14.33/22.02 | 20.38/31.10 | 17.04/22.63 |
| lit | 8.89/12.97 | 10.16/13.79 | 8.58/9.82 |
| multi-surface | 20.10/32.00 | 23.50/40.45 | 19.15/27.98 |
| narrow | 16.65/29.82 | 18.71/34.72 | 15.73/26.54 |
| occluded | 28.26/48.75 | 18.29/37.48 | 16.36/32.10 |
| partial | 15.16/24.82 | 17.87/29.61 | 14.97/21.74 |
| silhouette | 27.02/48.06 | 17.87/37.70 | 16.60/34.84 |
| single-surface | 11.38/18.00 | 11.81/18.08 | 10.04/13.64 |
| solid | 10.83/16.91 | 10.75/15.97 | 9.16/12.09 |
| solid-lit | 8.77/12.78 | 9.97/13.44 | 8.42/9.57 |
| solid-occluded | 27.69/47.99 | 17.30/35.47 | 15.71/31.24 |
| wide | 14.88/23.86 | 17.72/28.64 | 14.83/20.81 |
| wide6 | 14.62/23.17 | 18.22/29.02 | 15.14/20.70 |

**Non-RT, pose 0.**

| Population | Default (cache off) | ES1 cache on | Open specular, cache on |
|---|---|---|---|
| all-covered | 11.64/18.43 | 11.38/18.08 | 9.68/13.66 |
| background-centre | 1.28/3.83 | 1.28/3.83 | 1.28/3.83 |
| backlit | 18.45/36.27 | 18.52/38.22 | 15.98/29.86 |
| contact | 19.03/34.12 | 19.60/38.09 | 16.52/28.81 |
| edge | 22.55/50.46 | 22.85/52.06 | 20.48/47.37 |
| faint | 13.69/20.43 | 17.51/26.70 | 14.52/18.66 |
| image | 4.90/8.30 | 4.80/8.18 | 4.13/6.59 |
| isolated | 14.33/22.02 | 17.38/27.07 | 14.84/20.43 |
| lit | 8.90/13.01 | 9.15/12.82 | 7.73/9.10 |
| multi-surface | 20.11/32.02 | 23.00/39.59 | 18.88/27.69 |
| narrow | 16.72/29.98 | 18.76/34.39 | 15.85/26.39 |
| occluded | 28.26/48.78 | 18.19/36.70 | 16.63/32.65 |
| partial | 15.17/24.85 | 17.36/29.19 | 14.56/21.38 |
| silhouette | 27.03/48.10 | 17.52/37.48 | 16.67/35.04 |
| single-surface | 11.39/18.03 | 11.00/17.35 | 9.39/13.20 |
| solid | 10.84/16.94 | 9.84/15.08 | 8.44/11.63 |
| solid-lit | 8.78/12.82 | 8.93/12.47 | 7.55/8.83 |
| solid-occluded | 27.69/48.01 | 17.33/34.90 | 16.07/31.87 |
| wide | 14.88/23.86 | 17.11/28.20 | 14.33/20.40 |
| wide6 | 14.62/23.15 | 17.45/28.43 | 14.49/20.16 |

Unoccluded sky specular reads better than occluded on **every** population, occluded and silhouette included (RT 18.29 → 16.36, 17.87 → 16.60). The occluded population's bias is **-8.9 codes** with ES1 (too dark), against +6.1 with the cache off (too bright). The reference's occluded pixels see sunlit rock in the occluded directions: bounce, which the cache does not carry yet (G2). Occluding the sky without adding that radiance undershoots, exactly as G1c's cache-on diffuse does. ES1 keeps the occluded form because it is the physical estimand and because its visibility is the cache's incident-over-open ratio: once G2 puts bounce into the cache's incident radiance, the same ratio scales specular by incident light rather than by sky visibility alone (I). The open variant is a diagnostic, not a candidate: it would double-count once bounce arrives.

Even the open variant does not beat the default everywhere (faint, isolated and wide6 lose at -14 on both paths), so the cache's diffuse is the remaining limit there: G1c's -5 to -7% open-ground bias and the missing bounce.

## 6. Cost (quarry)

**Quiet window `es1-q2`** (2026-11-08, 120 s without a foreign stillkit compiler before it, **no stillkit sample inside**; announced in the schedule; binary `es1-k`, the pushed code). One process per path, the quarry loaded once; baseline, ES1 and the in-shade placement variant interleaved in each of four visits per cache state, so intermittent foreign GPU load falls on all alike. Per visit: a cut (reset, ten frames at pose 0), then 24 frames moving 1 cm a frame. Ranges over visits 2-4 (visit 1 follows the first program reload and warms the driver's shader cache); one non-RT cache-off visit was contended and is excluded from its range. Shading is the traced deferred shading pass (RT) or the visibility-buffer shading pass (non-RT), `GpuSample::mesh_pass`; span is the whole frame, `render_span`. Milliseconds:

| Path | Cache | Phase | Baseline shading | ES1 shading | Increment | Baseline frame | ES1 frame |
|---|---|---|---:|---:|---:|---:|---:|
| RT | off (default) | camera moving | 0.329-0.331 | 0.352-0.360 | **+0.02-0.03** | 0.883-0.888 | 0.868-0.912 |
| RT | off | cut, frame 1 | 0.335-0.342 | 0.366-0.377 | +0.03 | 0.91-0.92 | 0.89-0.95 |
| RT | on | camera moving | 0.415-0.421 | 0.618-0.621 | **+0.20** | 2.04-2.12 | 2.26-2.27 |
| RT | on | cut, frame 1 | 0.492-0.516 | 0.727-0.731 | +0.22-0.24 | 5.11-5.21 | 5.31-5.37 |
| RT | on | after a cut, settled | 0.423-0.426 | 0.618-0.620 | +0.19-0.20 | | |
| Non-RT | off (default) | camera moving | 0.331-0.333 | 0.353-0.358 | **+0.02** | 2.06-2.15 | 2.07-2.11 |
| Non-RT | on | camera moving | 0.418-0.420 | 0.628-0.632 | **+0.21** | 3.93-4.10 | 4.17-4.28 |
| Non-RT | on | cut, frame 1 | 0.499-0.502 | 0.753-0.758 | +0.25 | 25.2-25.5 | 25.7 |
| Non-RT | on | after a cut, settled | 0.423-0.425 | 0.635-0.638 | +0.21 | | |

The in-shade placement (the lookup inside `shade`) reads the same within 0.01 ms (RT on 0.611-0.614, off 0.346-0.355; non-RT on 0.623-0.625, off 0.349), so the entry hook costs nothing over it.

**Where it goes** (functional screens, `a4`, paired against the same baseline; RT shading with the cache on): lobes with no visibility **+0.00 ms**; the visibility lookup alone **+0.14 ms**; both, one lookup site, **+0.19 ms**; the first version with a lookup at each of three call sites +0.53 ms. The increment is the second cache lookup, the one `indirect_irradiance(n)` already performs for diffuse at the same receiver; open item 4 proposes sharing it.

**Against the budget.**
- The cache-off default pays **+0.02-0.03 ms** of shading (about 7% of the pass, within the whole-frame noise here) for code it does not execute: a recorded cost of the default. Specializing the shading pipelines on the cache state (a WGSL `override`) would remove it; not done.
- With the cache on, ES1 adds **+0.20-0.25 ms** a frame on both paths, on top of G1c's measured +1.3-1.6 ms (RT) and +1.8-2.4 ms (non-RT), which already miss every reading of Lumen's budget (G1c section 3.2).
- Lumen's own reflection pass on the quarry costs **0.07 ms** at Epic (0.22 ms with hit lighting; [M7-U0](m7-u0.md)), because few quarry pixels fall below its trace roughness; its rough specular comes through the GI gather. Half of it, the 2x line, is 0.035 ms: ES1's rough-specular increment **misses** it by about 6x. Against Lumen GI plus reflections (1.28-1.31 ms) halved, 0.64-0.66 ms, the cache and ES1 together miss as well. Speed misses are recorded; quality is not reduced to meet them.

## 7. Memory (M)

- **GPU:** one lobe table per RLE2 map that has lobes, 128 x 66 Rgba32Float = **135,168 B** (one lobe or four: one texel holds all four), counted with the DFG as a lookup table, not as environment cube bytes; the empty table is 144 B. No buffer, cache or per-pixel storage is added. Against G1c's totals (RT 2,763 MiB, non-RT 3,663 MiB, under Lumen's 6,444 and 5,142) this is +0.13 MiB.
- **CPU, once per upload** (debug test build): fit **13.9 ms**, share table **396 ms**, after admission (a busy upload still packs and allocates nothing). The table is single-threaded f64; a release build or threads would shorten it (not measured).

## 8. Claim notes (engineering reading, not clearance)

- **The world radiance cache is read, never changed.** ES1 calls `indirect_irradiance` through its existing interface: no counter, average, ranking, jitter or new query point (W1-W3, C1-C5, C8 untouched). No reservoirs or resampling anywhere.
- **Lobe fitting and analytic lobe evaluation, the N=V prefilter and its share table, the irradiance-ratio specular occlusion** are not among P0's 53 mapped documents or the flags in milestone-7.md. No patent search was run for them in this unit; one is advisable before a commercial claim, as P0 notes for its own scope.

## 9. Misses and open items

1. **Glossy metals under a concentrated sky** (plane, r 0.3-0.5): 0.60-1.02 of the reference (median 0.81 / 0.88), from 0.00-0.89 before. The fitted weight keeps most of the split sum there; a lobe-and-tail estimator did not beat it on the CPU grid. Glossy bands are R1's traced reflections.
2. **Moving `isolated` population at -10 stops** (both paths, pan): ES1 lowers its bias but raises its variance (section 4.4).
3. **The cache-on image still loses lit and partial populations to the default**, so ES1 does not make the cache a default candidate. Causes, measured: the cache's open-ground bias (G1c item 5) and the missing bounce (section 5; G2). Neither is in ES1's files.
4. **ES1's one extra cache lookup** (section 6). **Interface change requested from G1d** rather than made: let `indirect_irradiance` keep the receiver's blended SH (independent of `n`) for the receiver, so `indirect_irradiance(n)` and ES1's visibility evaluate one lookup. The lobe evaluation alone costs nothing measurable, so this would remove nearly all of ES1's shading increment.
5. The lobe weight's worst case is a lobe exactly in the mirror direction at grazing view (+20% at r 0.85, CPU grid); no registered population isolates it.
6. RLE1 maps keep the plain split sum.

## 10. Verification

| Gate | Result |
|---|---|
| `cargo test -p renderlab --lib` (merged code, staged binary `es1-k`, inside lease `merged`) | **exit 0: 141 passed, 0 failed, 141 ignored**. Includes ES1's three CPU tests (lobe fit on sun, constant, smooth and two-sun fields; shares against a dense quadrature within 1%; table layout), G1's composition test (the standard program composes and validates on raster and traced stages; toon composes no cache) and R0b's probe validation with ES1's stubs |
| M5 environment gates, RT suite, Khronos validation | constant photometry and compatibility, independent banks without float filtering, registered runtime reconstruction (538 references), axes/yaw/poles/seams/mips, refined sun angular and convolution reference, ownership/admission/reload, lit photometry 1x/4x: **all pass, 0 validation errors, 0 missing layers** (`m5-a` 0-5, `m5-b` 0). The two regular/virtual image gates cannot run on the traced suite ("hardware shadows do not yet support virtual geometry"), as before ES1 |
| Same gates, raster suite | all nine, including both regular/virtual normal-shadow image gates: **pass, 0 validation errors** (`m5-b` 1-9) |
| Sky-lit plane, both paths | section 3: 70/96 within 1% + 4 SE with the cache on; no case worse than the default |
| Quarry, both paths | section 4: cache-off default unchanged; cache-on image equal or better on every static population |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy -p renderlab --lib --tests` | clean in every file ES1 changes; six warnings remain in untouched files (`visibility_buffer.rs`, `reference_inputs.rs`, `reference_tests.rs`, `virtual_shadow_gpu.rs`, `material_response_tests.rs`) |

Timing of the checks: the lib suite runs in 3 s of tests after an incremental build of about a minute; a quarry process (load, 16 captures) takes 20-30 s; the plane takes 35-50 s on RT with its 96 references and 8-22 s on non-RT.

## 11. Evidence and reproduction

Evidence root `E:/renderlab-work/m7-es1/` (target `D:/Coding/renderlab-work/m7-es1/target`, deleted at close):

- `cpu/`: the CPU replays and grid (`quarry_replay.py`, `lobe_model.py`, `lobe_grid.py`, `lobe_split.py`, `blend.py`, `conv.py`, `cv.py`, `lobe-grid.json`).
- `runs/final/`: plane JSONs and summary, quarry `score.json` per path and the diagnostic, `doc-tables.md`; `runs/a2`-`a6`, `m5-a`, `m5-b`, `merged`, `q2`: earlier functional runs, M5 gate logs, cost rows.
- `baseline-standard.wgsl` (the standard program at `50c35da`), `variants/` (screened shader variants), `quarry-manifest.json`.
- Raw f32 captures were deleted after scoring.

```powershell
py -3 -B scripts/m7-es1-run.py build LABEL                       # two Cargo jobs, never inside a lease or QUIET window
py -3 -B scripts/m7-es1-run.py gpu RUN rt:plane nonrt:plane rt:quarry:static,pan nonrt:quarry:static,pan --binary LABEL --repeat
py -3 -B scripts/m7-es1-run.py gpu RUN rt:quarry:abcost nonrt:quarry:abcost --binary LABEL --quiet --idle 120
py -3 -B scripts/m7-es1-run.py gpu RUN rt:lib --binary LABEL      # the lib suite, inside the lease
py -3 -B scripts/m7-es1-score.py score E:/renderlab-work/m7-es1/runs/RUN
py -3 -B scripts/m7-es1-score.py doc E:/renderlab-work/m7-es1/runs/RUN
py -3 -B scripts/m7-es1-plane.py E:/renderlab-work/m7-es1/runs/RUN
```

**Process.** Every GPU process ran inside an m7-es1 lease holding `gpu-slot.lock`, polled in-process. Two process faults are recorded in the schedule: m7-es1's CPU-only renderlab test binaries, run outside a lease (announced, but test binaries count as renderers), stopped m7-g1d's leases b12, b13 and g3 and its QUIET window q2-1; and m7-es1 stopped its own quiet launcher q1 by mistake, leaving its lock for about a minute before releasing it by token. Every later test binary ran inside m7-es1's own lease.
