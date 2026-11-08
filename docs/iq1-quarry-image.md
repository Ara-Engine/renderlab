# IQ1: quarry image pinholes, grain and shipped anti-aliasing

Branch `t3/iq1-quarry-image`, from main `5afaec5`, 2026-11-08. Starts from Islam's inspection of G1c's quarry image (`E:/renderlab-work/m7-g1c/runs/q10-2/qmot-rt/rt-hardware-pose0.png` against the 1,024-spp reference). His crops show reference | cache on | cache off at 3x: `zz-face.png` (x1150 y400, 240x160) and `zz-edge.png` (x880 y480), in the root session's scratch directory. Contract: [milestone-7.md](milestone-7.md). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## Status

| Item | Cause (M) | Result | Verdict |
|---|---|---|---|
| (c) Black single-pixel pinholes along edges and cracks | **One point sample per pixel over sub-pixel scan geometry.** The quarry draws 13.0 M triangles; no covered pixel's 3x3 block lies on one triangle. A pinhole pixel's centre lands on a micro-facet that faces away from the sun, is shadowed by its neighbour, or is lit at a grazing angle. The reference integrates the whole pixel, so it shows a slightly darker pixel there instead. Hit records, barycentrics, NaN/Inf, back-face handling and the receiver offset are each ruled out per pixel (section 2). | Pixel integration removes them. TR1, the opt-in temporal reconstruction: **1,290 → 40** (pose 0, still), **1,304–1,454 → 10–34 per frame** on the Q-walk, both paths. | **Cause found; the default image still has them.** The fix is default anti-aliasing, which is TR1b's gate (section 5) |
| (b) Grainy, over-sharp detail on rock faces and sand | The same cause. The lighting term on sub-pixel facets carries most of the excess high-frequency energy; texture filtering is correct. Traced ray-differential footprints equal raster quad derivatives within ±2% per pixel. 16x anisotropy is not a clean win at 1x. U1b's specular filter changes nothing here: the quarry has no normal maps and its roughness is 0.85 (section 3). | Pixel integration halves the high-frequency error (29.8 → 15.8 still, 29.5 → 19 moving) and improves every population at both exposures. | **Cause found; the default image still has it** (same gate) |
| (a) Shipped anti-aliasing | Neither path anti-aliases by default. Both ship at 1x with TR1 off and anisotropy 1; on RT the code default is raster primaries with Exact shadows. Every M7 gate except TR1's own scores an un-anti-aliased image (section 4). | Each gate's AA state is recorded in its harness and below. | **Proposal:** make TR1 the default on traced and visibility-buffer frames (section 5). Not switched here |

Nothing in the shipping image changes in this unit. The only renderer source change is the diagnostic test module (`src/iq1_tests.rs`, one `mod` line in `src/renderer.rs`) plus AA-state comments in the gate harnesses.

## 1. Method

`renderer::iq1_tests::gpu_iq1_quarry` (one process per path) loads the registered quarry once and renders U0b's full-radiance protocol, as G1c's full mode does:
- the quarry's own materials and 4K textures;
- the sun at 0.04 rad on the 4/12/40 m splits;
- the RLE2 sky and a black backdrop;
- exposure −14 stops, 1920x1080, radiance cache off unless stated.

Diagnostic programs are swapped into the standard program in place (`Renderer::reload_program`), so every variant shades the same materials, textures and hits:
- `shadow`: the engine's light visibility, shading N·L and viewed-side geometric N·L;
- `frame`: N·V, the vertex normal's N·L, and flags (source side, normal map, missing normal, N·V < 0);
- `footprint`: the uv gradient lengths in texels and the world footprint;
- `albedo`: base colour times vertex colour, unlit;
- `light05`: the standard program with albedo 0.5;
- `nomap`: the standard program without the normal map;
- `flat`: faceted shading;
- `specaa`: U1b's filter on;
- `lodm1`, `lodm05`: textures sampled at half the pixel footprint, or at 2^−0.5 of it.

The traced and visibility-buffer paths also dump their hit records.

Integrated images, with the same code:
- `jitter8`: the mean of 8x8 stratified sub-pixel offsets through `Camera::jittered`. This is the pixel integral without any reconstruction.
- `tr1xN`: TR1's measured settings (`Reconstruction::identity()`) after N still frames.
- The Q-walk runs TR1's own `gpu_tr1_sequence`, with TR1 off and on, frames 4–64.

Paths:
- `traced`: the RT device with traced primaries and Interactive shadows, the configuration M7's RT gates use.
- `raster`: the RT device's code default, raster primaries and Exact shadows.
- `nonrt`: the no-RT device, which selects the visibility buffer and Virtual shadows at creation.

Scoring (`scripts/iq1-analyse.py`) uses U0b's frozen F0 transform (Reinhard plus sRGB codes at −14 and −10 stops) against U0b's 4,096-spp full-radiance archives (`quarry-p0/p1`, `q-walk-NNN`). It scores their 20 registered populations, as G1c's section 5.4 did. Three measures:
- **RMS** per population, as U0b's floor tables.
- **High-frequency (HF) error:** the RMS of the difference between the candidate's and the reference's 3x3 Laplacians of luminance codes, on pixels whose 3x3 block lies in the population. The reference's own Laplacian noise, from its split halves, is subtracted. The **energy ratio** is the candidate's Laplacian RMS over the reference's, noise removed: 1 means equally sharp.
- **Pinholes:** a covered pixel below 0.4x the median of its 8 neighbours (median above 15 codes) and below half the reference there, where the reference itself is at least 0.6x its own neighbours' median. Every neighbour must be covered, so silhouette pixels against the backdrop don't count.

These are functional leases on a shared GPU, with no timing claims.

## 2. Pinholes, per cause (M)

### 2.1 Not a path or hit-record defect

- RT (traced) and non-RT (visibility buffer) give the same image. On G1c's own PNGs, 712 of RT's 716 cache-off pinholes are also non-RT pinholes (pose 0; 770 of 774 at pose 1). The RT device's raster default has 1,298 / 1,291 against traced's 1,290 / 1,283 (poses 0 / 1).
- **NaN/Inf:** no non-finite value in any HDR or diagnostic dump, and no exactly-black covered pixel, on either path or pose. `rl_finite` maps NaN to 0, so a NaN would have shown as exact black.
- **Missing or degenerate normals:** no covered pixel lacks a vertex normal.
- **Back faces:** 60–68 covered pixels per pose see a source-back face, and 13 / 10 of the pinholes (1%).
- **Texture footprint:** see 3.2. Traced ray differentials equal raster quad derivatives.

### 2.2 Classification (traced, pose 0; pose 1 in brackets)

`scripts/iq1-pinholes.py` reads the `shadow`, `frame`, `albedo` and hit dumps at each pinhole:

| Class | Pinholes |
|---|---:|
| Hit facet faces away from the sun (geometric N·L ≤ 0), so visibility 0 | 489 (471) |
| Front-facing, unshadowed, sun- and view-facing, ordinary albedo: a facet whose N·L is below its lit neighbours' (sampled examples 0.01–0.5) | 495 (502) |
| Shading normal faces away from the view (N·V < 0 on a front face) | 143 (155) |
| Shadowed while neighbours are lit | 62 (51) |
| Shadowed, neighbours shadowed or partly shadowed | 47 (61) |
| Shading normal faces away from the sun on a sunward facet | 39 (33) |
| Source-back hit | 13 (10) |
| Dark albedo texel | 2 (0) |
| **Total** | **1,290 (1,283)** |

The hit records put the median pinhole 0.086 barycentric from an edge, against 0.098 for all covered pixels. Pinholes are no closer to a triangle edge than ordinary pixels: the "edges and cracks" they follow are the scan's creases and grooves, not triangle seams.

### 2.3 The shadowed pinholes are real, not receiver-offset self-shadowing

- Of the 109 (112) shadowed pinholes on the traced path, **103 (101)** are also shadowed by the non-RT Virtual lookup and by the raster path's Exact 256-direction shadows. These are three independent visibility methods sharing only the receiver point.
- The reference's own pixel-integrated disk visibility at those pixels has a median of 0.32 (0.24): the reference sees them mostly occluded too.

### 2.4 The cause: no pixel integration

| Traced, pose 0 (pose 1) | Pinholes | All-covered RMS, −14 / −10 | HF error | Energy ratio |
|---|---:|---:|---:|---:|
| Shipping 1x | 1,290 (1,283) | 11.63 / 18.40 | 29.8 | 1.58 |
| `jitter8`: 64 jittered samples averaged | 72 (76) | 10.01 / 15.84 | 15.9 | 0.84 |
| TR1, 64 still frames | **40 (45)** | 9.98 / 15.76 | 15.8 | 0.80 |
| Non-RT TR1, 64 still frames | **40 (45)** | 9.98 / 15.76 | 15.8 | 0.80 |
| Faceted shading (`flat`) | 1,500 (1,602) | 12.84 / — | 39.2 | 1.91 |

The 40–72 left after integration are not single-pixel defects. 33 of `jitter8`'s 72 are also 1x pinholes; the rest form a few short streaks inside crevices, in the contact and backlit populations. There the 4,096-spp reference carries interreflected light, which Renderlab's cache-off image lacks (G1c 5.4). In motion, TR1 leaves 10–34 per Q-walk frame (section 5.2).

Crops (`D:/Coding/renderlab-work/iq1/crops/`; left to right):
- `a1-traced-std-jit-face.png` and `-edge.png`: reference | 1x | `jitter8`.
- `traced-1x-tr1-jit-edge.png`: reference | 1x | TR1 64 frames | `jitter8`.
- `nonrt-1x-tr1-mark-face.png`: reference | non-RT 1x | non-RT TR1, with pinholes ringed in red.
- `a1-traced-mark-edge.png`: reference | 1x | lighting only (albedo 0.5), ringed. The pinholes survive with every texture removed.

Inspected: the integrated columns lose the dotted crease lines and the grain, and read as the reference does, slightly softer (3.4).

### 2.5 The GI cache adds its own pinholes

With the radiance cache on (traced, G1c's default settings), there are **3,038** pinholes (3,011 at pose 1) against 1,290 off. Its 64-sample jittered average still has **386** (422), against 72 with the cache off. So the cache adds dark pixels that pixel integration does not fully remove. This is outside IQ1's files and is recorded for G2. The non-RT cache was not exercised here: this harness attaches no near-field package, so its cache-on image equals cache off.

## 3. Grain, per cause (M)

### 3.1 Decomposition

Log-luminance Laplacian RMS, traced, pose 0, on interior pixels:

| Image | Solid-lit | All-covered |
|---|---:|---:|
| Reference (4,096 spp; its own noise 0.02 / 0.04) | 0.528 | 0.694 |
| Shipping 1x | 0.809 | 1.450 |
| Albedo only | 0.342 | 0.410 |
| Albedo only, 16x anisotropy | 0.397 | 0.466 |
| Lighting only (albedo 0.5) | **0.706** | **1.355** |
| `jitter8` | 0.427 | 0.603 |

In log space the standard image's energy splits as albedo² + lighting² (0.342² + 0.706² ≈ 0.809²). The lighting term, N·L and visibility over sub-pixel facets, carries about three-quarters of it. The albedo term alone is below the reference's total.

### 3.2 Texture footprint on the traced path

Per pixel, the traced (and visibility-buffer) uv footprint from ray differentials against the raster path's quad derivatives. Ratio quantiles over every covered pixel, both poses:

| 1% | 10% | 25% | 50% | 75% | 90% | 99% |
|---:|---:|---:|---:|---:|---:|---:|
| 0.979 | 0.994 | 0.998 | 1.000 | 1.002 | 1.006 | 1.021 |

The visibility buffer's footprints are bit-equal to the traced path's. The footprints are about 9.7 texels per pixel (median; 10%–90%: 4.8–24.9), LOD about 3.3, with a median anisotropy of 1.33. No gradient is zero or at level 0 by mistake. MAT1's mip-level finding holds per pixel as well.

### 3.3 Anisotropy, specular filtering, normal maps

- **16x anisotropy** (traced, pose 0):
  - all-covered 11.63 / 18.40 → 11.56 / 18.48, HF error 29.8 → 29.3, pinholes 1,290 → 1,312;
  - at −14 stops 18 of 20 populations improve (by up to 0.10 codes) and backlit worsens; at −10 stops 17 of 20 worsen at pose 0 and 19 of 20 at pose 1, by up to 0.20 codes, and none improves.
  - It is not a clean win on the 1x image, so it is not proposed as a default change on its own. This agrees with MAT1.
- **U1b's specular filter (`SPECULAR_AA` true):** all-covered 11.63 → 11.63, HF error 29.81 → 29.81, pinholes 1,290 → 1,293, both paths. The quarry has no normal maps (MAT1; `nomap` is bit-identical to standard), its roughness is 0.85, and the filter sees only the hit triangle's own normal derivatives. So it cannot see the sub-pixel facets that cause the grain. U1b's own cost record stands; a cheap version would not change this image.
- **Normal maps:** none in this RLA1 quarry, so they play no part.

### 3.4 Integrated images are slightly too soft

The integrated images' energy ratio is 0.80–0.84. Each jittered sample still filters textures at the whole pixel's footprint, so integrating samples blurs the texture a second time. `lodm1` (half footprint) measures the correction: see 5.3.

## 4. Shipped anti-aliasing and each gate's AA state

### 4.1 What ships (M, from code)

| | RT device | Non-RT device | Viewer |
|---|---|---|---|
| Primary visibility | Raster (`PrimaryVisibility::default`; RT1c kept raster + Exact) | Visibility buffer, selected at creation | Raster |
| Shadows | Exact, 256 directions | Virtual | — |
| Samples | 1 (`RendererOptions::default`; 4x MSAA opt-in on raster only) | 1 | `--samples 1` (4 opt-in) |
| TR1 | Off (`Reconstruction::default`) | Off | Not exposed |
| Anisotropy | 1 | 1 | `--anisotropy 1` |

**Neither path anti-aliases by default.** M7's RT gates use traced primaries with Interactive shadows, the configuration milestone-7.md names as the RT baseline. At 1x the RT device's actual default (raster + Exact) scores the same within 0.01 codes on all-covered: 11.631 / 11.691 against traced 11.629 / 11.688, poses 0 / 1. 4x MSAA on raster is measured in 5.3.

### 4.2 Each gate's harness

| Gate (unit) | Harness | Samples | TR1 | Anisotropy | Primary, shadows | Scores the shipping AA? |
|---|---|---|---|---|---|---|
| SV sky visibility (G1, G1b, G1c) | `gi_cache_tests.rs` `rig` | 1 | off | 1 | traced / VB | **Yes**: un-anti-aliased |
| Quarry full image (G1c, G1d) | `gi_cache_tests.rs` `gpu_gi_quarry` | 1 | off | 1 | traced + Interactive / VB + Virtual | **Yes** |
| Environment specular (ES1) | `environment_specular_tests.rs` `es1_renderer` | 1 | off | 1 | traced + Interactive / VB + Virtual | **Yes** |
| Quarry material (MAT1) | `material_response_tests.rs` | 1 | off | manifest (1; one 16x row) | traced + Interactive / VB + Virtual | **Yes** at 1x |
| Reflections (R0a) | `reference_r0a_tests.rs` `gpu_r0a_current` | 1 | off | 1 | traced + Interactive / VB + Virtual | **Yes** |
| Reflections (R0b) | `reflection_probe.rs` `gpu_r0b_*` | 1 | off | 1 | per test | **Yes** |
| Temporal reconstruction (TR1, TR1b) | `temporal_reconstruction_tests.rs` | 1 | **on per variant** | **16** | traced + Interactive / VB + Virtual | No: scores the candidate, at 16x |
| M6 film, RT1/RT1b/RT1c, N8 shadow gates | `fresh_shadow_tests.rs` `renderer` (and its child modules) | 1 | off | **16** (`RENDERLAB_TEST_ANISOTROPY`) | per test | AA yes; anisotropy differs. These score visibility masks, which anisotropy does not affect. Their film-radiance rows differ from 1x by about 0.07 codes (3.3) |
| Quarry frame loop | `quarry_frame_loop_tests.rs` | 1 | off | 16 | traced + Interactive | AA yes; anisotropy differs (cost and stability only) |
| Specular filter (U1b) | `specular_aa_tests.rs` | 1 | off | 1 | per suite | **Yes** (fixture) |
| D0 denoising | CPU replays of saved R0b, G1c and RT1c buffers | — | — | — | — | Inherits them: un-anti-aliased |
| IQ1 | `iq1_tests.rs` | 1 (4 for the MSAA row) | per variant | 1 (16 row) | per path | Records samples, anisotropy and primary in each run's JSON |

So every edge, silhouette and interior population in M7's gates, apart from TR1's, scores a single point sample per pixel against a pixel-integrated reference. The edge population's large errors (G1c: 22.6 against Lumen's 11.3) and much of the interior gap are that, not lighting. The harness lines above each carry an AA-state comment. The shadow-owned `fresh_shadow_tests.rs` is left unedited and recorded here.

## 5. The default-AA decision (proposal, not switched)

**Proposal: make TR1 the default on traced and visibility-buffer lit frames, on both devices.** It is the one measured mechanism here that fixes (b) and (c). TR1b owns the switch.

### 5.1 Static (M; pose 0 rows, pose 1 within 0.1 codes, both paths equal within 0.03)

| Population | 1x, −14 / −10 | TR1 64 frames, −14 / −10 | HF error 1x → TR1 |
|---|---:|---:|---:|
| all-covered | 11.63 / 18.40 | **9.98 / 15.76** | 29.8 → 15.8 |
| lit | 8.89 / 12.97 | 7.49 / 11.00 | 21.0 → 12.7 |
| solid-lit | 8.77 / 12.78 | 7.44 / 10.90 | 20.7 → 12.7 |
| occluded | 28.26 / 48.75 | 27.15 / 46.81 | 40.3 → 19.4 |
| solid-occluded | 27.69 / 47.99 | 26.89 / 46.42 | 36.5 → 16.1 |
| silhouette | 27.02 / 48.06 | 24.47 / 43.43 | — |
| contact | 19.02 / 34.07 | 15.28 / 26.07 | 36.9 → 22.0 |
| backlit | 18.46 / 36.25 | 13.36 / 24.71 | 22.4 → 10.4 |
| faint | 13.68 / 20.42 | 10.33 / 15.27 | — |
| partial | 15.16 / 24.82 | 11.73 / 18.42 | 31.0 → 16.2 |
| multi-surface | 20.10 / 32.00 | 15.46 / 25.57 | — |
| narrow | 16.65 / 29.82 | 12.43 / 20.37 | — |
| edge | 22.60 / 50.57 | **15.59 / 26.47** | 92.8 → 36.8 |
| isolated | 14.33 / 22.02 | 7.49 / 9.96 | — |
| background-centre | 1.28 / 3.84 | 0.18 / 0.50 | 0.7 → 0.1 |
| image | 4.90 / 8.30 | 4.08 / 6.46 | 13.0 → 6.5 |
| pinholes, poses 0 / 1 | 1,290 / 1,283 | **40 / 45** | |

TR1 beats 1x on **every population at both exposures, both poses, both paths**. The non-RT rows equal the traced rows within 0.07 codes.

### 5.2 In motion: the Q-walk (M; TR1's harness, 16x anisotropy on both sides; frames 4, 8, 16 … 64)

| Per frame | RT off | RT TR1 | Non-RT off | Non-RT TR1 |
|---|---:|---:|---:|---:|
| Pinholes | 1,304–1,422 | **10–34** | 1,351–1,454 | **11–31** |
| All-covered RMS, −14 | 11.61–12.05 | 10.29–10.89 | 11.69–12.11 | 10.34–10.93 |
| All-covered RMS, −10 | 18.63–19.67 | 15.99–17.15 | 18.77–19.77 | 16.05–17.20 |
| HF error | 29.2–29.8 | 18.3–19.8 | 29.3–29.9 | 18.4–19.8 |
| Edge RMS, −14 | 22.28–23.12 | 19.72–20.46 | 22.37–23.16 | 19.69–20.39 |

TR1 beats TR1-off on every population, at both exposures, on every scored frame, on both paths. Against Unreal, TR1's own record stands: it loses to TSR on moving silhouettes (19.9 against 17.8), revealed pixels and background-centre pixels next to edges, and its p95/p99 cost misses (m7-tr1.md). Those are TR1b's gate items, not IQ1's.

### 5.3 Settle, texture LOD and MSAA

Traced, poses 0 / 1. The non-RT rows equal these within 0.01 codes. The last column lists populations worse than the row's baseline at either exposure.

| Image | Pinholes | All-covered, −14 / −10 | HF error | Energy | Worse populations |
|---|---:|---:|---:|---:|---|
| Shipping 1x | 1,290 / 1,283 | 11.63 / 18.40 | 29.8 | 1.58 | (baseline) |
| TR1, **8 still frames** (M7's settle limit) | **23 / 23** | 10.05 / 15.69 | 17.0 | 0.66 | none, against 1x |
| TR1, 64 still frames | 40 / 45 | 9.98 / 15.76 | 15.8 | 0.80 | none, against 1x |
| TR1, 64 frames, textures at LOD −1 | 43 / 48 | **9.74** / 15.75 | **12.5** | 0.89 | against TR1: backlit at −10 (+0.05 / +0.09), wide at −10 (+0.01, pose 1) |
| TR1, 64 frames, textures at LOD −0.5 | 41 / 45 | 9.83 / 15.74 | 14.1 | 0.84 | against TR1: backlit at −10 (+0.01 / +0.04) |
| Textures at LOD −1, 1x | 1,314 / 1,331 | 11.57 / 18.62 | 28.8 | 1.71 | against 1x: 17 of 20 at −10 |

- **TR1 meets the 8-frame settle limit on this measure.** After 8 still frames it already beats 1x on every population at both exposures, and leaves fewer pinholes than at 64 frames. It is softer at 8 frames: TR1's spatial start blurs a little.
- **Texture LOD under integration.** A sharper per-sample texture fetch recovers most of the double blur (3.4): high-frequency error falls 15.8 → 12.5 and energy rises 0.80 → 0.89. But backlit worsens at −10 by up to 0.09 codes, so neither −1 nor −0.5 passes the every-population rule.
  - It is recorded as a TR1b candidate (the engine would scale the program's `uv_dx`/`uv_dy` only on reconstructed frames), not changed here.
  - At 1x it is simply worse.
- **4x MSAA cannot render this protocol.** The raster path at 4x rejects the 0.04 rad soft sun: `soft visibility requires hardware 1x and exactly one shadowed directional light` (run `c1/raster-msaa4`). MSAA is the only anti-aliasing the viewer exposes (`--samples 4`). It is not an option for soft-sun scenes. Even with a hard sun, its four samples would integrate at most four of the facets a quarry pixel covers.

## 6. Misses and open items

1. **(b) and (c) are not fixed in the default image.** The cause is the missing pixel integration. The measured fix is default anti-aliasing, which this unit was told not to switch: it is TR1b's gate. Until TR1 (or another integrating AA) is the default, the shipping 1x image keeps its ~1,300 pinholes per frame and a 1.6x high-frequency energy ratio on the quarry.
2. **The raster primary path cannot take TR1** ("raster and diagnostic frames are unaffected"). The RT device's code default is still raster + Exact (RT1c), so a TR1 default only reaches RT users once traced primaries are the RT default too.
3. **The cache-on image has 2.4x the pinholes** (2.5), and integration leaves 386 of them. This is recorded for G2.
4. **Integrated images are about 20% too soft** on high-frequency energy (3.4); 5.3 records the texture-LOD measurement.
5. **Even integrated, the quarry's edge population stays above Lumen HW Epic's static floor:** TR1 15.59 / 26.47 against Lumen's 11.28 / 20.20 (G1c's table, pose 0). Interior populations close most of their gap; the edge remainder is TR1b's measure.
6. **Anisotropy differs between gate families** (4.2): the shipping 1 against 16 in the M6, N8, RT1 and TR1 harnesses. On the 1x quarry image it moves RMS by under 0.1 codes, but in opposite directions at the two exposures.

## 7. Evidence and process

**Retained under `D:/Coding/renderlab-work/iq1`:**
- `runs/a1`, `b1`, `c1`, `d1`: per-step process records, JSON logs and the lease plans (`plans/`);
- `analysis/`: every score JSON;
- `crops/`: the inspected PNGs;
- `scripts/`: early CPU probes of G1c's PNGs.

Raw f32 and hit dumps were deleted after summary (hashes in `deleted-raw.json`), as was the Cargo target.

**Code:**
- `src/iq1_tests.rs`: `gpu_iq1_quarry`, plus the CPU check `iq1_variants_compose` that every diagnostic program composes and validates on raster and traced stages;
- `scripts/iq1-run.py`: runner, owner `m7-iq1`, evidence and target on D:;
- `scripts/iq1-analyse.py`, `iq1-pinholes.py`, `iq1-crops.py`: CPU analysis.

**Reproduce:**
- `py -3 -B scripts/iq1-run.py build --release --as <name>`, then `plan <label> <plan.json>` with steps naming `renderer::iq1_tests::gpu_iq1_quarry` and `IQ1_*` variables (the test's doc comment lists them);
- `py -3 -B scripts/iq1-analyse.py <dump dir> <out.json>`, and `qwalk <data dir> <out.json>` for TR1's sequence frames.

**Process:**
- Every GPU process and test binary ran inside an `m7-iq1` lease holding `D:/Coding/renderlab-work/gpu-slot.lock`, with request lines in `gpu-schedule.md`. No game hold was in force.
- One build (`iq1-c`) overlapped the first two steps of lease `b1`. Those are functional captures with no timing claim, and their images were checked against lease `a1`'s identical configurations: equal scores.
- Q-walk sequences reused TR1's harness and manifests, with output redirected to D:.
- Time:
  - implementation: about 1 h (harness, scripts, docs);
  - builds: about 15 min (five incremental release builds plus clippy, all outside IQ1's leases except the one noted);
  - GPU in four leases: 5.2 min (a1 97 s, b1 109 s, c1 67 s, d1 39 s);
  - waiting for the shared lock: about 25 min;
  - CPU analysis: interleaved, under 2 min per batch.
- One clippy run overlapped the start of rt1d's queued QUIET window `quiet-cost-3` and probably stopped it. This was acknowledged in `gpu-schedule.md`; rt1d's next window ran clean.

**Verification:**
- The renderlab lib suite (release test binary `iq1-f`, inside lease `d1`): **144 passed, 0 failed**.
- Later edits are comments and docs only.
- `rustfmt --check` is clean on every changed Rust file.
- `cargo clippy -p renderlab --lib --tests` reports nothing in IQ1's files. Its remaining warnings are pre-existing, in files IQ1 does not touch.
