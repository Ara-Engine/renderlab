# P-DEFECT: Showcase's three visible defects

Unit P-DEFECT, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10, branch `t3/p-defect` rebased on main `71ea355` (P-SHOW). Ara's integration reported three defects on its Showcase scene, still, moving and on a cut: stray white pixels on glossy surfaces (both paths), a stair-stepped glass rim (both paths) and strong grain on lit flat faces (non-RT). Labels: **M** measured here, **I** inference, **C** read from code.

## Result

| Defect | Cause | Change | Verdict |
|---|---|---|---|
| White pixels in glossy reflections | **Caustic paths (M, C).** A glossy ray that reaches a near-mirror hit (chrome 0.02-0.04, gold) where that hit's own sun highlight is gets the hit's exact delta-light GGX lobe: up to 65504 from one ray in hundreds. The reflected highlight's energy was missing everywhere else. | **Path regularization at reflection hits**: while a hit is shaded, every engine `brdf` lobe widens by the reflection ray's cone (GGX alpha² + 8 spread²). Energy kept, spread over the cone. | **MERGE** (`7556121`, `6b61d19`) |
| Non-RT blotches on lit flat faces | **The capped gather drain (M).** Non-RT got a quarter of RT's gather budget (8,192 vs 32,768 cells a frame), so bounce drained for about 100 frames after a cut. P-SHOW's "settled" plinth blotches were frames 16 after a cut at that budget, which the old every-frame relight had been averaging away. | **One gather budget, 32,768, on both paths.** | **MERGE** (`d0538f3`). The relight scaffolding stays until a Showcase capture confirms (below) |
| Stair-stepped glass rim | **Thin-slab refraction magnifies the opaque image (M, I)**: near a sphere's rim the slab's shift grows quickly, so the refracted band enlarges the pixel steps of whatever is behind. RT adds a dotted rim: one mirror ray per pixel at grazing, with no history for glass (C). | Centroid varyings, a footprint-filtered refraction lookup, a lobe floor for glass reflections and a neighbour fallback for edge fragments (`t3/p-defect-glass`). | **DON'T MERGE**: not visibly better. The fix is a refraction model, not filtering (below) |

## 1. Repro (M)

Showcase itself could not be rerun here. P-SHOW's Ara tree and target were deleted before this unit started, and a cold Ara perf build at two jobs did not fit between Ara's own builds (section 6). The repro is therefore Showcase's materials in our scenes, plus the Showcase captures from Ara's integration thread (main before P-SHOW) and P-SHOW's crops.

- `examples/reflection_matched.rs --stage` lays Showcase's turntable (black dielectric: base 0.06, roughness 0.35, 16 traced glossy strata) under the chrome and gold spheres, in the path-traced reference too. `--glass-balls` adds Showcase's clear and tinted glass balls (opacity 0, IOR 1.5, 0.6 m slab), which the tracer cannot render.
- `support/estimator_diag.rs` gains `gi-gather-cells` and `gi-gather-stride`.
- Runs: 1280x720, release builds, RTX 4070 SUPER (NVIDIA 617.14), shared machine. Flight frames 0 (cut) to 47 (moving); still = `--freeze 16` (frame 0 a cut, 47 held); 192-frame holds for settled GI. Reference: 16,384 spp at frames 0, 16 and 47.

All three defects reproduce:
- **White pixels**: on the turntable, base shows isolated white pixels at the cut and while held, and no glow where the reference has one: the chrome sphere's sun highlight seen through the turntable's glossy lobe (`crops/turntable-ref.png`, `blob-still.png`).
- **Blotches**: non-RT crates and the red wall are blocky at frames 4-8 after the cut; RT is clean (`gi-budget-zoom.png`).
- **Glass**: a stepped band inside the top rim on both paths, and a dotted rim on RT (`glass-zoom.png`).

## 2. White pixels: path regularization at reflection hits

**Cause (C, M).** `rl_reflection_prepare` shades every hit with the program's `shade`, including its direct lights through `brdf`. A delta sun on a 0.04-roughness metal has a peak GGX D near 1.2e5. A glossy stratum from the turntable hits that highlight with a probability of about 1/200, so the estimate is a white pixel or nothing: the path tracer's own "caustic fireflies" ([path-traced-scene-references-are-cheap] memory), made worse by having only 16 rays. P-SHOW saw the same thing from the other side: fewer strata on the dim turntable made the specks brighter and denser.

**Change.** `shaders/scene.wgsl` declares `rl_hit_variance` (0 everywhere), and `brdf` uses alpha² + `rl_hit_variance`. `shaders/reflections.wgsl` sets it to 8 spread² around its hit loop, where `spread` is the ray cone's half-angle (pixel angle, normal curvature and the stratum's own cone). Mirror rays from flat receivers stay near the exact lobe. This is path regularization (Kaplanyan and Dachsbacher 2013, as in Cycles' filter-glossy). The lobe stays normalized, so a highlight keeps its energy and spreads over the cone. Camera receivers, GI cells and the frozen R0b probe are unchanged.

**Width.** Three widths were screened:
- 0.5 spread²: the glow appeared, but grainy on cuts.
- 2 spread²: smooth when held, but white speckle on the game scene's spinning glossy crate, a moving object with short history (`regress-check.png`).
- 8 spread² (committed): the highlight spans about all of a pixel's strata. The crate's speckle is gone on both paths (`crate-3way.png`: base, 2 and 8 spread²), and its reflection is smooth and slightly brighter than base's.

The reflected highlight widens by about √2 of the receiver's lobe (I). That is invisible next to the turntable's own 0.35 lobe.

**Before and after (M)**, turntable box (300,450)-(560,560), still pose, both paths within 0.2 codes of each other:

| | white pixels, cut (frame 0 / 1) | white pixels, held (47) | mean luma, held | speckle, held |
|---|---|---|---|---|
| base (main) | 12 / 15 | 24 | 87.2 | 0.25 |
| 0.5 spread² | 244 / 264 | 2 | 89.1 | 0.94 |
| 2 spread² | 121 / 19 | 1 | 89.2 | 0.59 |
| **8 spread² (merged)** | **4 / 2** | **2** | **89.3** | **0.33** |
| reference (16k spp, same pose) | | 3 | 90.8 | 1.54 (its own noise) |

"White pixels" counts pixels at least 40 codes over their 5x5 median and above 120. Inspected sheets:
- `p-defect-turntable-final.png`: base against the merged 8 spread², at the cut (RT frame 0, non-RT frame 1), held, moving (frame 16) and at dusk, plus the reference;
- `turntable-still-3way.png`: base, 0.5 and 2 spread², cut and held, both paths, and the reference;
- `turntable-motion.png`: moving frames 4 and 16;
- `turntable-ref.png`.

With 8 spread², base's white pixels are gone at the cut and while held, on both paths. The glow the reference shows is back, which restores about half of base's missing 3.6 codes. In motion the glow carries the reference's own fine grain. It also removes base's golden sparkle where the turntable reflects the gold sphere next to the glass ball.

**Cost (M).** One add per `brdf` call. Drained 120-frame timing, matched scene, 720p: RT 2.85 → 2.90 ms a frame, non-RT 6.89 → 6.87 ms: within noise.

**Programmability.** Programs that call the engine's `brdf` (standard and the game example's) get regularized hits automatically. A program with its own lobe gets exact hits, as before. Recorded as a constraint in the gaps.

## 3. Non-RT blotches: one gather budget

**Cause (M).** The bisect ran on 192-frame holds at the new budget. Settled non-RT is clean on the matched scene: crates, wall and pillars match RT (`gi-hold-zoom.png`). Only the cell filter switched off (`gi-filter=0`) shows faint mottle, as G3 recorded. The blotches are cells whose capped gathers have not run yet. At 8,192 cells a frame, a cut drains over about 100 frames, and Showcase's capture is taken 16 frames after its cut (`settle_frames: 16`). The old every-frame relight of skinned scenes re-gathered an eighth of the cells each frame, and TR1 averaged them, which hid this until P-SHOW removed it.

**Change.** `src/gi_cache.rs`: `GATHER_CELLS = 32768` on both paths (was 32,768 hardware and 8,192 software). The schedule is unchanged: a fixed slot rotation, no ranking (W2).

**Before and after (M).**
- At frame 4 after the cut, non-RT's red wall and crates lose most of their blotches.
- At frame 8 they match RT (`gi-budget-zoom.png`: 8,192, 16,384, 32,768 and RT).
- 16,384 still leaves the crates blotchy at frame 4.

**Cost (M).** Warm A/B on one binary, 720p, contended machine:
- matched scene, frames 1-23: 11.16 ms (16,384) and 11.59 ms (32,768);
- game scene, frames 1-23: 45.52 ms (8,192) and 46.28 ms (32,768).

So about +0.4-0.8 ms on drain frames, for about a quarter as many drain frames. Settled frames are unchanged.

**The relight switch is NOT removed.** Root asked to delete P-SHOW's software-ray relight scaffolding with this fix. On our scenes the cause is gone, but the scaffolding exists for Showcase's skinned mannequins, and no Showcase capture ran (section 6). Next: one Showcase non-RT still with the switch removed (`src/gi_cache.rs`, the `if software` arm), compared with P-SHOW's `crop-plinth-skin-nonrt.png`. If the plinths are clean there, delete it: that is P-SHOW's measured 43 → 31 ms on Showcase non-RT.

## 4. Glass rim: not fixed

**What was built** (`t3/p-defect-glass`, `939e3f1`, on top of the merged fixes):
1. Centroid interpolation for the glass module's varyings, so 4x-coverage edge fragments are not shaded from attributes extrapolated past the silhouette.
2. A footprint-filtered refraction lookup: up to 4x4 sub-pixel shifts where the shift's footprint exceeds a pixel, and the single centre lookup elsewhere.
3. On RT, a glass reflection lobe at least a quarter of the pixel's normal-curvature footprint, plus a neighbour fallback for edge fragments with no lobe. Glass traces one mirror ray a pixel and has no history.

**Result (M):** `glass-zoom-b2.png` (base and candidate, both paths) shows no visible change in the stepped band, and at most a slightly softer RT rim. A closer look shows the band is magnification, not minification. Near the rim, the slab's lateral shift (thickness / cos θ, here 0.6 m) maps a few pixels of the opaque image onto many, so the opaque silhouettes' own pixel steps become large stairs. No footprint filter removes that. The thin-slab model is the cause. A solid ball refracts through a curved back face and inverts its background, as Ara's does.

**Next:** a thick-glass refraction model. Trace the refracted ray through the mesh's back face, with hardware rays on RT and R1n's software geometry on non-RT, and shade the exit ray's hit as reflections do, so the light behind a ball is a traced image rather than a shifted screen copy. AGENTS.md forbids screen-space core lighting, and the shifted screen copy is exactly that, so it should be replaced, not filtered. Cost and quality unmeasured.

## 5. Glossy reflections: method change (proposal, not built)

Root's steer: trace at a lower rate and reconstruct, robust to rare bright samples without darkening them. Measure on both paths, with Showcase as the main score: non-RT about 21 ms of shading, RT about 5.5-6 of 14.8 ms still.

Section 2 changes the premise. The specks were caustic paths, not estimator variance, and regularization bounds them. A lower ray rate no longer amplifies them, so P-SHOW's rejected 4-strata variant (non-RT shading 25 → 11.5 ms, RT 7.8 → 4.7 ms) should be re-screened on top of this branch first.

The fuller design (R3):
- a quarter of the strata, continued across frames by TR1's rotation;
- per-pixel hit radiance and distance;
- history reprojected through the reflection's virtual image;
- energy-keeping firefly handling: the excess over a per-pixel robust bound goes to a long-window buffer spread over the lobe footprint, never clipped;
- fixed-weight spatial reuse on cuts.

Patent mapping:
- No reservoirs or resampling, so not ReSTIR. Resampled reuse alone stays allowed, and there is no world-space cache in this path (US 11,315,310).
- No variance clipping (US 10,116,916).
- Not SVGF or gradient estimation.
- Adshir/Snap unchanged from R1/R1n.

## 6. Process

- **Builds**, one at a time through the build slot and the stillkit mutex, target `D:/Coding/renderlab-work/p-defect/target`:
  - base examples 175 s;
  - candidate 1 149 s;
  - candidate 2 (plus Ara's perf build, stopped after 22 min to free the slot for verification);
  - candidate 3.
- **Leases** through `scripts/n8-run.py`, owner `p-defect`:
  - `pdefect-a`: 22 runs, 8.6 min. Its quick lib run caught the R0b probe's missing declaration, fixed in `7556121`.
  - `pdefect-b2`: 20 runs, 5.3 min. Lib: **206 passed, 0 failed, 164 ignored**.
  - `pdefect-c`: candidate 3 (the merged code), 11 runs, 4.6 min. Lib: **206 passed, 0 failed, 164 ignored**.
- **Time:** about 1 h 15 min of code and analysis; about 15 min of GPU leases; about 2 h queued behind Ara's builds and leases and P-SHOW's.
- **Showcase:** a worktree of Ara `d92e8f66a` patched to this branch is prepared at `D:/Coding/renderlab-work/p-defect/showcase` (`tools/showcase/showcase.py`). Its cold perf build did not complete; resume with `showcase.py --root D:/Coding/renderlab-work/p-defect/showcase build`.
- **Evidence** in `D:/Coding/renderlab-work/p-defect/`: `runs/{a,b2,c}` (frames and `run.json`), `scores-*.json`, `crops/`, `plans/`, `tools/` (lease, build, batch and scorer) and `design-glossy.md`. Binary identity: base = `27bd7e3` + diag keys (main plus harness), candidate 1 = `939e3f1`'s parent state, candidate 2 = `939e3f1` (`t3/p-defect-glass`), candidate 3 = `6b61d19`. Targets, binaries and pipeline caches were deleted at the end; the Showcase Ara worktree (source only) remains for the next capture.
