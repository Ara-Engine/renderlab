# M7-RQ1: soft, smeared glossy reflections

2026-11-08 overnight unit. Branch `t3/m7-rq1` from main `2d1c084`, with main `4c8ef7d` (R1n), `dc3fa11` and `577b4aa` (MQ2, API1b) merged. Islam, after the example frames in `D:/Coding/renderlab-work/mq1/deliver/final-pp1/`: "reflections still look mushy and low quality specially on the golden ball", then "not just the golden sphere it is just way more visible there". The root coordinator scoped this to every glossy surface on both paths, judged against path-traced references. Rules: AGENTS.md's overnight push, [P0](m7-p0-claims.md). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

**Status: shipped on both paths. Glossy rays now scale with roughness** (`shaders/reflections.wgsl`): 4 strata up to roughness 0.08, 12 at 0.25 (the gold sphere), 16 from 0.33. Mirrors still trace one ray, and the fade to the untraced estimate is unchanged.

**Cause** (M; section 2). In a still frame, R1's estimate is as sharp as the path-traced reference: on the gold sphere, the mid-band energy ratio is 1.01. The mush appears in motion:
- On any frame where something moves, TR1 clamps history to a 3×3 min/max box (MQ1).
- With four glossy rays, each frame's grain makes that box wide, so lagging history passes through it in blocks.
- That is the blocky, smeared look on the gold sphere, and a speckled smear on non-RT.

**Effect** (M; matched example against a 16,384-spp path-traced reference, frame 47 of the 48-frame flight). On the final merged code (main `577b4aa` with MQ2's history cap, §4.5):

| Path | Gold-sphere error | Fine-band error | Mid-band error |
|---|---|---|---|
| RT | 5.87 → 5.27 | 1.70 → 1.18 | 1.27 → 1.05 |
| Non-RT | 5.87 → 5.27 | 1.70 → 1.18 | 1.27 → 1.05 |

- Before MQ2 merged: RT 6.30 → 5.68 and non-RT 5.97 → 5.08 (§4.1).
- Frames 16 and 32 improve the same way.
- RF2 per band: roughness 0.3 goes 10.71 → 10.05 and 0.4 goes 9.97 → 9.43 on both paths. Mirror bands are unchanged.

**Cost** (M; functional, contended):
- Example at 1280×720 (GPU span p50, merged code): RT 9.12 → 11.07 ms (+1.95), non-RT 13.17 → 16.15 ms (+3.0).
- RF fixtures at 1080p (shading pass): up to +2.1 ms on RT and **+13.6 ms on non-RT** when glossy roughness 0.3–0.5 fills the screen (RF8).
- Recorded as a miss rather than lowering quality (AGENTS.md). Section 5 gives the 8-ray alternative's numbers.

**What is not R1's** (M; recorded as gaps):
- the remaining softness in motion: TR1 history (MQ2's cap, merged, trades lag for grain, which these rays now supply);
- the post stack's motion blur and bloom: 15–27 % of the spheres' fine band, 3–5 % of the mid band;
- one-bounce content: the gold sphere seen in chrome is olive;
- hit lighting from the open environment;
- caustics, which the reference has and no real-time path computes.

## 1. Ground truth

The shipped example can't be path-traced as it stands: it uses the physical atmosphere, aerial perspective, height fog, volumetric fog, decals, the toon hero and post. So the measurements add two modes to the example over the same placements, camera path and time. When API1b reworked `examples/game_scene.rs` into a dusk scene with lanterns, glass, foliage and the Fox, this scene was frozen as `examples/reflection_matched.rs` (the example at main `4c8ef7d` plus these modes), so reflection measurements stay comparable as the game example changes:
- **`--matched`** renders what the tracer reproduces: a uniform sky (3500, 5000, 7500 cd/m²), the same 120 klux sun with a 0.04 rad disk, no aerial perspective or fog, no decals, the hero on the photoreal program (base 0.95/0.55/0.2, roughness 0.5) and post off. Display is exposure −14.5, x/(1+x), sRGB.
- **`--reference SPP`** builds `renderlab::reference` meshes, standard-model materials, instances and the sun from the recorded spawns, then path-traces the requested frames on its own ray-query device. It writes the mean and both independent half-sample images in the same display transform. Tracing at 16,384 spp takes 16 s at 1280×720; 4,096 spp takes 6 s per frame.

Other flags:
- `--freeze FRAME` holds every frame at FRAME's time, giving a still camera that TR1 converges on.
- `--write-frames LIST` writes only the listed frames.
- `--no-post` turns post off in the shipped configuration.

**References.**
- Frame 47 at 16,384 spp: the split-half noise is 0.63 codes RMS over the frame, 0.85 on the gold sphere and 1.84 on chrome.
- Frames 16 and 32 at 4,096 spp: 1.0 codes over the frame, 2.1–2.9 on the spheres.
- An earlier 2,048-spp trace showed caustic fireflies: the gold and chrome spheres focusing the sun onto the ground and the hero. They converge slowly, which is why the final references use 16,384 spp at frame 47.

**Scorer** (`scripts/m7-rq1-score.py`). Regions are the gold and chrome sphere interiors at 0.9 radius (MQ1's per-frame projection) and the full frame. All values are 8-bit display codes:
- `rms`: the RMS error;
- `fine`: the high-pass band, the image minus a σ = 1.5 px blur;
- `mid`: the band that carries reflected shapes 2–8 px across, σ1 − σ4;
- `bias`: the σ = 4 low-pass error.

For each band the scorer reports the render-to-reference energy ratio and the RMS of the difference. A ratio below 1 means detail was lost (mush). Above 1 means extra grain or blocks.

## 2. Diagnosis: where the detail goes (M)

All runs: RT path, `--matched`, frame 47, development build, one binary per batch. Variants are the same reflection shader with one constant changed, screened through a temporary debug-only composition hook. The hook was removed at delivery; the final binary carries the change built in, and its images are byte-identical to the screened variant's.

| Candidate | Run | Gold rms / fine err / mid ratio / mid err / bias | Chrome rms / mid ratio | Reading |
|---|---|---|---|---|
| R1's estimate itself | TR1 off, 64 rays | 4.82 / 1.15 / 1.03 / 0.90 / 4.43 | 8.83 / 0.84 | Converged per-frame estimate. |
| TR1, still camera | `--freeze 47`, 4 rays | 4.83 / 1.23 / 1.01 / 0.87 / 4.42 | 7.79 / 0.84 | Equal to the converged estimate, so a still TR1 adds no blur. |
| TR1, moving (the flight) | 4 rays | **6.30 / 1.60** / 1.05 / 1.48 / 5.48 | 9.40 / 0.79 | Blocky smears and lag (crops below). |
| Glossy ray count, moving | 8 / 16 / 64 rays | 5.81 / 1.26; 5.63 / 1.11; 5.49 / 1.01 | 9.40 (mirror: 1 ray) | The smear tracks per-frame grain: 16 rays remove it visibly; 64 add little. |
| Fade to the untraced estimate | band to roughness 1.0 (`wide`) | gold unchanged (0.25 is fully traced) | — | Whole-frame rms 3.72 → 3.42, but mean abs gain only 0.015 codes, all on the 0.5-roughness hero. It traces every rough pixel (ground, stone). Not adopted. |
| Hit lighting | hits prepare the GI cache lookup (`hitgi`), still, 64 rays | 4.35 / 0.90 / 1.05 / 0.81 / **3.97** | 7.60 / 0.88 | Reflected objects get occlusion and bounce: −9 % gold error. Not adopted: needs a quarry cost check (gap 3). |
| Filtering before demodulation | — | — | — | Not a cause. R1 has no spatial filter or denoiser; the trace returns mean incident radiance and the program multiplies by its split-sum reflectance. TR1 sees only final colour. |
| Upscaling preset | `--render-scale quality` | 6.71 (vs 6.30) | 10.65 (vs 9.40) | Worse, but the example renders native 1280×720 by default, so this is not the shipped cause. |
| Post (shipped configuration) | default post vs `--no-post`, frames 40/47 | fine band ×0.73–0.79, mid ×0.96–0.97 | fine ×0.85, mid ×0.97–1.0 | 180° motion blur and bloom soften edges, not reflected shapes (PP1). |

**Content, not blur.** The chrome sphere's mid band sits at 0.84 even with TR1 off and 64 rays. Its error maps (`scratch/chrome-err-still.png`) show where the error is:
- the reflected gold sphere: olive at the hit, because hits take only their untraced specular (one bounce);
- the sun glint: a point light against the reference's 0.04 rad disk;
- the reflected ground's lighting;
- caustic fireflies in the reference.

The gold sphere's remaining 4.4-code bias is the hero's brightness: the reference lights it by the gold sphere's caustic and bounce.

Crops (3× nearest), `D:/Coding/renderlab-work/rq1/deliver/`:
- `gold-raycount-rt-047.png`: reference | flight, 4 rays | 16 | 64 | still. The 4-ray flight shows the blocky smear on the right limb and around the hero's reflection; 16 rays remove it.
- `gold-matched-047.png`, `chrome-matched-047.png`: reference | RT before | RT after | non-RT before | non-RT after.
- `gold-shipped-047.png`, `chrome-shipped-047.png`: the shipped configuration (atmosphere, fog, decals, toon hero, post): RT before | RT after | non-RT before | non-RT after.
- `frame-matched-{rt,nonrt}-047.png`: whole frames, reference / before / after.
- `frame-shipped-{rt,nonrt}-047.png`: whole frames, before / after.

## 3. The fix: glossy strata grow with the lobe

```wgsl
const RL_GLOSSY_RAYS:u32=4u; const RL_GLOSSY_MAX:u32=16u; const RL_RAYS_PER_ROUGHNESS:f32=48.0;
let count=select(clamp(u32(ceil(RL_RAYS_PER_ROUGHNESS*rough)),RL_GLOSSY_RAYS,RL_GLOSSY_MAX),1u,mirror);
```

- **The count depends only on the program-declared lobe.** Each pixel's estimate is still a stratified mean of its own VNDF samples, so it is unbiased.
- **Strata stay R1's:** u.x on N radial strata turned by TR1's frame word, u.y a golden-ratio azimuth. The escaped rays' environment sub-cone narrows with the count (alpha/√N).
- **Wider lobes get more rays** because they see more varied radiance, and their per-frame spread is what widens TR1's motion box.
- **Both paths, one mechanism.** R1n composes the same block with its two ray queries replaced by text seams. The seams are untouched, so non-RT gets the same count.

**Rejected: adaptive count from the samples' own contrast.** Trace 4; if their luminance range exceeds 25 % of their sum, add 12 more and average all 16. It cost less on smooth lobes, but it is biased: a decision taken on the same samples that are averaged skews the mean. Still-frame gold error rose 4.83 → 5.43 and the low-pass bias 4.42 → 4.99. An unbiased variant would decide from independent pilot samples, which halves the base estimate's samples; that was not built.

## 4. Results (M)

### 4.1 Example scene, matched, against path-traced references

| Path | Frame | Run | Gold rms | Gold fine err | Gold mid err | Chrome rms | Frame rms |
|---|---:|---|---:|---:|---:|---:|---:|
| RT | 47 still | before / after | 4.83 / **4.76** | 1.23 / **0.97** | 0.87 / 0.86 | 7.79 / 7.79 | 2.38 / 2.37 |
| RT | 16 flight | before / after | 8.54 / **8.11** | 3.07 / 2.97 | 2.80 / 2.65 | 9.39 / 9.39 | — |
| RT | 32 flight | before / after | 7.47 / **7.05** | 2.66 / 2.54 | 2.39 / 2.21 | 9.52 / 9.52 | — |
| RT | 47 flight | before / after | 6.30 / **5.68** | 1.60 / **1.16** | 1.48 / **1.26** | 9.40 / 9.40 | 3.72 / 3.70 |
| Non-RT | 47 still | before / after | 4.83 / **4.76** | 1.23 / **0.97** | 0.87 / 0.86 | 7.79 / 7.79 | 3.08 / 3.07 |
| Non-RT | 16 flight | before / after | 8.19 / **7.51** | 3.41 / 3.03 | 2.76 / 2.53 | 8.06 / 8.06 | — |
| Non-RT | 32 flight | before / after | 6.95 / **6.27** | 3.64 / 2.75 | 2.24 / 2.00 | 8.01 / 8.01 | — |
| Non-RT | 47 flight | before / after | 5.97 / **5.08** | 2.06 / **1.32** | 1.47 / **1.03** | 7.84 / 7.84 | 3.98 / 3.94 |

- The reference noise at frames 16 and 32 is 2–3 codes on the spheres, so their fine-band values are noise-limited; rms and mid are the readable columns.
- Chrome is a mirror (one ray) and is unchanged by construction.
- Converged stills are identical on both paths.
- No region in the table got worse.

### 4.2 RF1–RF10, per roughness band (R0a's protocol)

Setup: 32 still TR1 frames per pose, 1920×1080, GI cache off, whole-reflector and per-band display RMS at −14 stops in 8-bit F0 codes. Before is main's shader and after is RQ1's, from one binary.

| Fixture / band | RT before → after | Non-RT before → after | UE HW-Hit / SW Epic |
|---|---|---|---|
| RF2 whole reflector | 12.855 → **12.739** | 12.858 → **12.742** | 15.195 / 15.920 |
| RF2 r0 / r0.05 (mirror) | 13.307 / 14.357, unchanged | 13.311 / 14.361, unchanged | |
| RF2 r0.1 | 14.407 → 14.430 (+0.022) | 14.410 → 14.432 (+0.022) | |
| RF2 r0.2 | 12.672 → **12.496** | 12.674 → **12.499** | |
| RF2 r0.3 | 10.708 → **10.047** | 10.709 → **10.048** | |
| RF2 r0.4 | 9.970 → **9.425** | 9.971 → **9.426** | |
| RF2 r0.5 / r0.7 / r1 (untraced) | unchanged | unchanged | |
| RF8 r0.3 / r0.4 | 6.284 → 6.279 / 6.919 → **6.896** | 6.356 → 6.351 / 6.948 → **6.925** | |
| RF8 whole | 7.496 → 7.488 | 7.526 → 7.518 | 3.913 / 5.241 |
| RF1 (three poses), RF3, RF5, RF6, RF10 whole | −0.001 to −0.008 each; r0.1 bands −0.002 to −0.020 | the same | |
| RF4 whole / r0.1 | 3.535 → 3.542 / 3.570 → 3.591 | 3.563 → 3.570 / 3.652 → 3.672 | |
| RF7 (mirror panels), RF9 | unchanged (RF9 r0.1 +0.001) | unchanged | |

**Worse rows.** Five rows rise:
- RF2 r0.1: +0.022 on both paths;
- RF4 r0.1: +0.021 on both paths, with its whole reflector +0.007;
- RF9 r0.1: +0.001.

At roughness 0.1 the count goes from 4 to 5 strata, so the converged sample pattern changes. RF2 r0.1's reference split noise is 7.65 codes, so its change is not resolvable. RF4's band noise is 0.39 codes and RF9's 0.27, so their +0.021 and +0.001 are measured, though small against the noise. The full table is in `runs/f1/rf-bands.md`. Inspection sheets (`runs/f1/{base,scaled}/sheets/`) show no visible change on the mirror fixtures. On RF2's 0.3/0.4 spheres the residual rim sparkle is reduced and closer to the reference's soft glow (`scratch/rf2-03-04.png`: reference, before, after).

### 4.3 Cost (M)

The RF rows are R1n's timed frames: GPU timestamps, medians of frames 5–16 after each capture, shading pass and frame span.
- Hardware and software: RTX 4070 SUPER, NVIDIA 617.14, Vulkan, Ryzen 7 7800X3D.
- Build: dev-profile test binary. Shaders are the same as release.
- **Functional, not QUIET.** Other units' leases and builds ran between mine.

| Fixture (1080p) | RT shading before → after | Non-RT shading before → after |
|---|---:|---:|
| RF1 pitch20 / 30 / 40 | 2.10 → 2.20 / 2.49 → 2.62 / 2.41 → 2.54 | 2.67 → 2.87 / 3.25 → 3.54 / 3.00 → 3.75 |
| RF2 sweep | 1.33 → 1.87 (+0.54) | 3.51 → 8.27 (+4.76) |
| RF4 | 2.72 → 3.10 | 3.92 → 4.65 |
| RF6 grazing 5 / 10 | 1.24 → 1.27 / 1.56 → 1.60 | 1.33 → 1.38 / 1.80 → 1.90 |
| RF7 (mirror) | 3.10 → 3.09 | 4.31 → 4.29 |
| RF8 (glossy rocks fill the screen) | 1.83 → 3.97 (**+2.13**) | 5.57 → 19.19 (**+13.62**) |
| RF9 / RF10 | 2.59 → 2.71 / 3.30 → 3.55 | 3.39 → 3.67 / 4.93 → 5.20 |

The example's timed segment covers 96 drained frames after the 48-frame flight, shipped settings, 1280×720, two interleaved visits per shader:

| | Before | After |
|---|---:|---:|
| RT wall | 5.20 / 5.20 ms per frame | 7.01 / 7.04 |
| RT GPU span p50, flight frames 8–47 | 8.94 / 8.72 ms | 10.28 / 10.27 |
| Non-RT wall | 8.17 / 8.14 ms | 11.75 / 11.72 |
| Non-RT GPU span p50 | 12.43 / 12.55 ms | 15.61 / 15.60 |

The quarry traces no reflection ray: every material is roughness 0.85, and non-RT has no reflection geometry under the coverage rule. Its image cannot change, but its timing was not rerun (gap 6).

### 4.4 Tests

| Check | Result |
|---|---|
| `renderlab` lib suite, final binary (`bin/f1`), in lease `rq1-v2` | **191 passed, 0 failed, 158 ignored; exit 0**. Includes R1n's `software_reflection_composition` (seams match once; naga-valid, SPIR-V) and `reflection_settings_bands` |
| RF1–RF10 both paths, `gpu_r0a_current` | `test result: ok`, both shaders, both paths |
| Built-in shader vs screened variant | final matched and shipped frames byte-identical to the screened `scaled` runs, both paths |
| `cargo fmt --check`; Clippy on the example | clean |

### 4.5 Final merged code (main `577b4aa`: MQ2, API1b) (M)

Main merged MQ2 while RQ1 was in verification. MQ2 makes programs declare a view-dependent share, and TR1 caps those pixels' history while the view turns. It also found that R1n's per-frame instance-record rewrite counted as a geometry upload, so non-RT TR1 restarted every pixel of animated frames on main `4c8ef7d`. Main's gap list records that fix as shipping in RQ1, so this branch carries MQ2's one-line change in `src/primary_ray_gpu.rs`: the records count as transform edits.

The earlier non-RT flight rows in §4.1 were measured without that fix, so non-RT had no TR1 history in motion there.

Lease `rq1-g2` ran two builds of this merged code: 4 glossy rays (`RL_GLOSSY_MAX` 4) against RQ1's scaled rays. Both used the frozen `reflection_matched` scene and the same references.

| Path | Frame | Gold rms | Gold fine err | Gold mid err | Chrome rms | Frame rms |
|---|---:|---|---|---|---|---|
| RT | 47 still | 4.83 → **4.76** | 1.23 → **0.97** | 0.87 → 0.86 | 7.79 → 7.79 | 2.38 → 2.37 |
| RT | 16 / 32 flight | 8.21 → **7.72** / 6.84 → **6.53** | 3.11 → 2.96 / 2.77 → 2.54 | 2.70 → 2.54 / 2.14 → 2.05 | unchanged | 2.42 → 2.40 / 3.04 → 3.04 |
| RT | 47 flight | 5.87 → **5.27** | 1.70 → **1.18** | 1.27 → **1.05** | 8.41 → 8.41 | 3.66 → 3.63 |
| Non-RT | 47 still | 4.83 → **4.76** | 1.23 → **0.97** | 0.87 → 0.86 | 7.79 → 7.79 | 3.08 → 3.07 |
| Non-RT | 16 / 32 flight | 8.21 → **7.72** / 6.84 → **6.53** | as RT | as RT | unchanged | 2.88 → 2.86 / 3.24 → 3.23 |
| Non-RT | 47 flight | 5.87 → **5.27** | 1.70 → **1.18** | 1.27 → **1.05** | 8.41 → 8.41 | 4.06 → 4.04 |

- **MQ2 already improved the 4-ray base** (6.30 → 5.87 on RT). With its history cap, the 4-ray grain shows as a speckled smear rather than blocks (`deliver/gold-merged-047.png`: reference | RT before | RT after | non-RT before | non-RT after). The scaled rays remove it on both paths.
- **The two paths are now identical** on both spheres in motion: non-RT TR1 keeps history with the fix.
- **Timing** (96 drained frames, one visit each):

  | Path | Wall, ms per frame | GPU span p50, ms |
  |---|---|---|
  | RT | 5.27 → 7.34 | 9.12 → 11.07 |
  | Non-RT | 8.34 → 11.93 | 13.17 → 16.15 |

- **Lib suite** (`bin/g-scaled`): **193 passed, 0 failed, 158 ignored; exit 0**.
- **Not run:** the new dusk `game_scene`. The batch passed `--write-frames`, which only `reflection_matched` has, so the four dusk runs exited at argument parsing. The dusk scene's own gold-sphere view is not inspected here.

## 5. Trade-off for Islam: 16 strata or 8

| Glossy cap | RT gold, flight 47 | Non-RT gold, flight 47 | Cost |
|---|---:|---:|---|
| 4 (main) | 6.30 | 5.97 | — |
| 8 (fixed, screened) | 5.81 | 5.41 | about half the increments above (I: rays scale linearly) |
| scaled to 16 (shipped) | 5.68 | 5.08 | as measured in 4.3 |

AGENTS.md says to record misses rather than reduce quality to pass the speed gate, so the shipped default is the measured-best curve. If the non-RT cost on large glossy surfaces is unacceptable, setting `RL_GLOSSY_MAX` to 8 is a one-line change. The better route is fewer rays once MQ2's reflection reprojection lands, because history would then carry what rays carry now (gap 1).

## 6. How others keep glossy metals sharp (public sources; engineering reading)

- **Unreal's Lumen reflections** (Wright et al., SIGGRAPH 2022 "Lumen: Real-time Global Illumination in Unreal Engine 5"; Epic's public Lumen documentation):
  - one traced ray per pixel or half-resolution pixel, below a maximum roughness;
  - a spatial reconstruction that reuses neighbouring rays weighted by the BRDF;
  - a temporal filter that reprojects reflections by their hit distance.
  - Rough surfaces fall back to the screen probes or radiance cache.
- **Frostbite's stochastic screen-space reflections** (Stachowiak, SIGGRAPH 2015): ray reuse across neighbours with BRDF/pdf weights, and reprojection by the reflected point's virtual depth.
- **NVIDIA's ReBLUR/RELAX** (Ray Tracing Gems II, Zhdan): demodulate specular by its pre-integrated BRDF, filter radiance with hit-distance-scaled kernels, remodulate.

The common thread is that sharp glossy motion comes from reprojecting by the reflected image (virtual depth or hit distance), not by the surface. Renderlab reuses no rays across pixels, so it has no filter to blur, and its still image already matches the reference. What it lacks is reflection-aware history, which is MQ2's open work. Neighbour ray reuse is also near the ReSTIR and stochastic-SSR patent families (P0), so RQ1 spends rays per pixel instead.

## 7. Claim notes (engineering reading, not clearance)

R1's readings hold (m7-r1.md §7, P0 C1/C7). RQ1 changes only how many fixed VNDF strata a pixel traces, as a function of its own program-declared roughness:
- no reservoir, resampling, neighbour, history, variance statistic or colour-based decision;
- no common projection;
- each pixel writes only its own value.

The rejected contrast-adaptive variant would also have used only the pixel's own samples.

## 8. Programmability

- **No new constraint.** The count reads only the lobe's `alpha` from `reflection_lobe`. A program that declares a sharper or rougher lobe gets the matching count.
- **Example program unchanged.** `--matched` swaps the hero onto the photoreal program only in measurement runs; the shipped toon program is untouched.

## 9. Gaps

See [m7-gaps.md](m7-gaps.md), Reflections items 14–20.

## 10. Evidence, reproduction and time

Everything is under `D:/Coding/renderlab-work/rq1/`; nothing was written to E:.
- `deliver/`: the crops and sheets above.
- `runs/d1/`: the first references (2,048 and 16,384 spp) and partial runs.
- `runs/d2/`: the diagnosis and candidate renders, the references (`ref/`), `score-47.json`, `score-motion.json`, `crops-*`.
- `runs/f1/`: RF scores and sheets (`base/`, `scaled/`), `rf-bands.{md,json}`, timed segments, final flights (`*-final-*`), `lib.log`.
- `bin/f1/`: the binaries verified before the MQ2 merge (`game_scene.exe`, `renderlab-lib.exe`); `bin/g-scaled/` and `bin/g-base/`: the merged code's after and 4-ray before builds; `runs/g1/`: their renders, timings, scores, crops and lib log.
- `variants/`: the screened shaders.
- `d*.ps1`, `f1.ps1`, `rf.ps1`: the lease batches.

Raw RF radiance dumps (24.9 MB each) were deleted after scoring. The Cargo target is deleted at delivery.

```powershell
cargo build --example reflection_matched --target-dir D:/Coding/renderlab-work/rq1/target
reflection_matched --path nonrt --reference 16384 --write-frames 47 --out REF
reflection_matched --path rt --matched --write-frames 16,32,47 --out RUN/rt-flight
reflection_matched --path rt --matched --freeze 47 --write-frames 47 --out RUN/rt-still
py -3 -B scripts/m7-rq1-score.py score RUN --ref REF --runs rt-flight,rt-still --frames 47 --out score.json
py -3 -B scripts/m7-rq1-score.py crops RUN --ref REF --runs rt-flight,rt-still --frame 47 --out crops
py -3 -B scripts/m7-r1-score.py RF_RUN_BEFORE; py -3 -B scripts/m7-r1-score.py RF_RUN_AFTER
py -3 -B scripts/m7-rq1-score.py rf RF_RUN_BEFORE RF_RUN_AFTER --out rf-bands.json
py -3 -B scripts/m7-rq1-run.py LABEL 15 120 -- powershell -NoProfile -File batch.ps1
```

`scripts/m7-rq1-run.py` is MQ1's runner with three additions:
- a gate on the latest `[root order]` line: owners and root labels held until their `released` lines;
- relayed holds: `RENDERLAB_RQ1_AFTER`, `RENDERLAB_RQ1_AFTER_TEXT`;
- a 6 GiB free-RAM start gate, because the shared guard stopped three of my leases below 2 GiB while other units compiled.

With `RENDERLAB_RQ1_FUNCTIONAL=1`, the image-only leases ignore the Ara unit's `ara1-ara.exe`, which stopped one lease. The batches are resumable: a run that has a `run.json` is kept.

**Time (UTC, 2026-11-08).**
- Reading and setup: 20:58–21:15. Example modes, reference path, runner and scorer: 21:15–21:45, with the first lease at 21:18. Merging R1n and rebuilding: 21:45–21:52.
- Candidates: 21:55–22:30.
- Leases:
  - `rq1-d1` 21:18–21:20: RAM guard stop;
  - `rq1-d1b`: foreign-renderer stop;
  - `rq1-d1c`: RAM stop;
  - `rq1-d2-1` 22:02–22:07: RAM stop;
  - `rq1-d2-2` 22:27–22:34: stopped for root's MV1 order;
  - `rq1-v1` 22:40–22:45: stopped for root's order;
  - `rq1-v2` 22:58–23:02: RF, timing, final flights, lib;
  - `rq1-g2` 23:20–23:26: the merged-code A/B and lib.
- Scoring and the document: 22:45–23:30.
- Implementation took about 1 h 10 min; verification, including about 1 h of lease waits, about 1 h 20 min. Development builds were 0.3–2.5 min each with two jobs.

No window, viewer, focus change or input automation. No Unreal launch.
