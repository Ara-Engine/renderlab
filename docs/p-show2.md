# P-SHOW2: Showcase follow-through

Unit P-SHOW2, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10, branch `t3/p-show2` from main `9e343a0` (P-YARD2 recorded; renderer code as at `4652ea3`). Showcase is Ara's sample scene, drawn by Ara's player through Ara's Renderlab bridge ([p-show.md](p-show.md)). Labels: **M** measured here, **A** reported by another unit, **C** read from code, **I** inference.

## Result

| Step | Verdict | Showcase (Ara span, still, constant sky) | Quarry vs Unreal Epic |
|---|---|---|---|
| 1. Named per-pass GPU timeline, ablation switches, harness pass medians | **MERGE** (`ec8b635`) | measurement only: zero cost when unset | unchanged (tooling) |
| 2. Delete P-SHOW's non-RT relight scaffolding | **MERGE** (`a77f613`) | non-RT 55.2 → 33.1 ms (timeline 50.4 → 26.5; GI 20.5 → 0.9 ms); 30.9 ms on the head; plinths clean | unchanged: non-RT 8.66 ms still, 2.15x slower than SW Lumen Epic (4.02) |
| Regression on main (root steer 1) | **explained, fixed by step 2** | see section 2 | |
| Sky map: bank a legacy chain's narrow levels at load | **MERGE** (`930e843` + `374265f`, one change; the second supersedes the first) | with the map sky: RT still 52.5 → 33.9 ms (shading 43.6 → 25.1); image within 2 codes of main | unchanged: RT 4.16 ms still, 1.08x faster than HW Lumen Epic (4.50) |
| Sky map: the remaining ~17 ms | **OPEN** | RT shading with the map is still 25.1 ms against 8.5 on the constant sky | |
| 3. Glossy at a lower rate | **NOT RUN** (time box, re-prioritized) | | |
| 4. Aluminium ball | **cause found (both sides); Renderlab half built, not screened** | | |

The main score is Showcase on the RTX 4070 SUPER at 1080p, Ara release build (Ara `eb20a48b5`, the integration thread's Sky::Map bridge, patched to this tree). Its constant-sky switch `ARA_RENDERLAB_SKY=constant` gives the like-for-like rows. All rows are single captures (`ara capture --stability 60`: 16 settle frames, then the median of 60 live GPU samples) in leases `pshow2-e` (main's behaviour, through screen switches) and `pshow2-h` (the head, the careful restart), on a shared machine. The Ara span is `GpuSample::render_span`, and "timeline" is section 1's span.

## 1. Per-pass GPU timeline (step 1, MERGE)

**What it is (C).** `src/pass_timeline.rs`. With `RENDERLAB_PASS_LOG=<file>` set when the renderer is created, each frame writes one timestamp at the start of every named stage and one at the end. Each completed frame appends a JSON line, `{"frame":n,"span":ms,"passes":{"skinning":ms,...}}`. A stage lasts from its mark to the next mark. Marks are empty compute passes, so they need only `TIMESTAMP_QUERY`, which Renderlab's own device requests; embedders change nothing. Unset, nothing is allocated, and each mark is one `Option` test.

Stages, in frame order:
- `skinning`, `light_clusters`;
- `tlas`: TLAS build and skinned BLAS refits;
- `sun_shadow`: raster, virtual or fresh sun visibility;
- `transmission`;
- `atmosphere`: LUTs, aerial perspective and froxel fog;
- `visibility`: primaries, either the visibility buffer or traced;
- `gi_probe`, `gi_gather`, `gi_claim`;
- `gi_update`: scan, setup, rays and finish;
- `gi_integrate`: integrate and the cell filter;
- `gi_cell_shading`;
- `shading`: the deferred shading pass, including reflection rays and hit shading, local lights and GI lookups;
- `tr1`, `transparency`;
- `post`: post and present.

Reflections, local lights and GI lookups run inside the one shading pass. `RENDERLAB_ABLATE` (`src/engine/settings.rs`: `gi`, `reflections`, `local_shadows`, `media`, `post`, `tr1`) switches features off whatever the embedder asks for, so a harness can split that pass by ablation. Unset, it changes nothing.

**Harness (C).** `tools/showcase/showcase.py capture` sets the log on every capture. It adds the medians over the measured frames (the last `--stability N`, or the `--video N` frames) to the capture JSON as `renderlab_passes`; `summary` prints them. `-e K=V` passes the switches. `--profile release` builds and runs Ara's release profile, the one the integration thread measures with.

**Reading it (I, M).** A timestamp between passes lands when the preceding work drains, so stages the GPU overlaps share a boundary, and the times sum to the span. On RT, Renderlab submits a frame in two halves with CPU work between them (`split` in `submit_frame`). The idle gap falls in the stage before the second half, `visibility` (3.8-5 ms on RT Showcase). In motion the timeline span can exceed `render_span`, for example RT moving 23.6 against 16.2 ms. Read RT stage times other than `visibility` as GPU work.

**Measured split, Showcase still, constant sky (M)**, ms:

| Run | Ara span | timeline | sun shadow | atmos | visibility | GI (all) | shading | TR1 | transp | post |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| RT | 17.87 | 13.76 | - | - | 3.79 | 0.49 | 7.74 | 0.47 | 0.53 | 0.14 |
| RT, reflections off (map sky) | 14.32 | 11.14 | - | - | 3.46 | 0.53 | 5.34 | 0.53 | 0.54 | 0.14 |
| non-RT, main (relight switch on) | 55.21 | 50.38 | 2.34 | 2.57 | 0.45 | 20.49 | 21.48 | 0.47 | 0.42 | 0.13 |
| non-RT, step 2 | 33.14 | 26.54 | 2.31 | 1.05 | 0.46 | 0.91 | 20.76 | 0.47 | 0.41 | 0.13 |
| non-RT, step 2, moving | 30.72 | 26.82 | 5.18 | 0.02 | 0.46 | 1.50 | 18.70 | 0.74 | 0.31 | 0.13 |
| non-RT, step 2, cut | 32.85 | 19.48 | 4.34 | 0.02 | 0.32 | 4.47 | 8.59 | 0.35 | 0.00 | 0.14 |

Main's non-RT GI on Showcase (relight on) splits into gather 3.55, update 8.33 (scan, setup, rays and finish), integrate and filter 6.08, and cell shading 2.23 ms. Skinning is 0.52 ms on RT and 0.04 on non-RT; TLAS 0.07 ms. Full table: `D:/Coding/renderlab-work/p-show2/e-summary.txt`.

**Ablation split of RT shading.** With the map sky, reflections off saves 38.3 of 43.6 ms, and GI off saves 6.2 (`e-rt-map-nogi` 37.4). On the constant sky, reflections are about 2.4 ms of the 7.7 (P-SHOW's ablation, A). The two runs of `local_shadows`, `media` and `post` ablations were dropped when lease AB was withdrawn for the root steer.

## 2. Regression on main and the relight switch (step 2, MERGE)

Root's report (A, integration thread): from `71ea355` to `4652ea3`, constant sky, Showcase went from 11.0 to 13.3 ms on RT and from 43.1 to 52.8 ms on non-RT. Bisected with one screen per change in one binary, using screen-only switches that are not merged: P-SHOW's relight rule (`RENDERLAB_SCREEN_OLD_RELIGHT`), the gather budget (`RENDERLAB_SCREEN_GATHER`) and P-DEFECT's hit regularization (`RENDERLAB_SCREEN_NO_HITREG`). They are kept on branch `t3/p-show2-screens`.

| non-RT still, constant sky | Ara span | timeline | GI | shading |
|---|---:|---:|---:|---:|
| relight on, gather 8,192 (as at `71ea355`) | 47.78 | 46.74 | 18.17 | 21.15 |
| relight on, gather 32,768 (main `4652ea3`) | 55.21 | 50.38 | 20.49 | 21.48 |
| relight on, gather 32,768, no hit regularization | 47.64 | 47.55 | 20.35 | 20.70 |
| **relight deleted, gather 32,768 (step 2)** | **33.14** | **26.54** | **0.91** | 20.76 |
| relight deleted, gather 8,192 | 27.85 | 29.01 | 2.10 | 20.76 |

- **Non-RT: root's suspicion is right (M).**
  - P-SHOW's switch kept every non-RT frame of a skinned scene a drain frame. P-DEFECT's 4x gather budget made each one dearer: GI 18.2 → 20.5 ms, span +3.6 ms on the timeline and +7.4 on the Ara span (single captures; spans spread by a few ms).
  - Hit regularization is flat on shading (21.48 vs 20.70).
  - Deleting the switch removes the drain altogether: GI 0.9 ms, non-RT still 55.2 → 33.1 ms (Ara span), 23 ms below the pre-regression `71ea355` figure. On a settled scene the 32,768 budget costs nothing; it only shortens drains after cuts, which is what P-DEFECT bought.
- **RT: the +2 ms is not P-DEFECT's (M, I).** RT never had the switch, and its gather budget was already 32,768. Regularization measures flat on shading: 7.74 with, 7.71 without, still; moving 14.45 vs 13.13 in one pair, within this lease's spread (Ara spans 16.21 vs 15.70). Root's 11.0 was the integration thread's release build of `71ea355`, and 13.3 its dev build of `4652ea3`. Our release Ara spans of the same code read 16.3-17.9 ms on this shared machine. The two figures are therefore not a paired comparison. Recorded as unexplained run-to-run and build spread, not a P-DEFECT cost.
- **Images (M, inspected).** `crops/step2-plinths.png`: main against step 2, still, moving frame 59 and cut frame 7, the left plinth group, 1x. `crops/step2-plinth-zoom.png`: one flat white face at x3.
  - At 1x the two cannot be told apart, still, moving or on the cut.
  - At x3 the step-2 still shows a very faint cell mottle on the flat face that main's churn averaged; the cut frame matches main.
  - Mean luma moves by at most 0.7 codes. No blotches, stray white pixels or crawl.
  - P-DEFECT's 32,768-cell budget is what keeps the settled cache clean here (P-DEFECT section 3); P-SHOW's blotches were its 8,192-cell drain.

## 3. Sky::Map: the top cost (root steer 2), half fixed

**Measured (M)**, Showcase still, the integration thread's bridge preparing Ara's gradient sky into a `PreparedEnvironment` (LegacyMipChain, 16-texel faces), ms:

| Run | Ara span | shading |
|---|---:|---:|
| RT, constant sky | 17.87 | 7.74 |
| RT, map | 52.48 | 43.60 |
| RT, map, reflections off | 14.32 | 5.34 |
| RT, map, GI off | 42.09 | 37.43 |
| RT, map, escaped glossy rays read the constant (timing only) | 40.36 | 29.36 |
| non-RT, constant sky | 33.14 | 20.76 |
| non-RT, map | 64.53 | 53.18 |
| non-RT, map, reflections off | 16.87 | 5.27 |
| non-RT, map, escaped rays constant | 49.86 | 37.66 |

The map's cost is reflection rays' environment lookups: about 14 ms on misses and the rest in hit shading's open-environment reads. It is not GI, and not camera receivers: with reflections off, map and constant sky cost the same.

**Cause (C, I; not profiled).** In the legacy layout, `environment_radiance` maps roughness linearly onto the chain's mips (`r × (levels − 1)`). With 16-texel faces, glossy cones and rough hits land on the 8-, 4-, 2- and 1-texel levels. There nearly every bilinear footprint crosses a face edge, and `environment_bilinear` leaves its 4-load in-face fast path for the exact cross-face path: up to four remapped taps (`environment_remap`, a face direction and a reselection each), plus a 3-tap corner loop. Lanes diverge between the two paths. That runs 16 times per glossy pixel and once or more per hit.

**Fixes.**
- **Bridge, available now (sent to the integration thread).** Cook the sky with `renderlab_cook::environment::prepare_environment` (the IndependentRoughness layout). Every roughness level is stored at full face size in array banks, so lookups stay on the in-face path, and the sky gains its dominant lobes.
- **Renderlab: banked narrow levels, MERGE (`930e843` + `374265f`, C, M).** Root's steer: a renderer should not get 38 ms slower because a caller hands it a valid map in the common mip-chain layout.
  - At `create_environment` (`try_upload_environment`), a legacy chain's levels narrower than 32 texels are resampled into 32-texel array banks in the second specular texture (`environment::bank_legacy`). Each texel is the level's own exact cube-bilinear value at its centre, through a CPU mirror of `environment_bilinear`, seams and corners included. Wider levels keep their mips unchanged.
  - The shader reads banked levels from those banks (`environment_level`, `environment_bank_start`). The uniform marks the layout as levels + 0.5.
  - Cost: at most about 0.9 MiB per chain, plus a CPU resample at load. The atmosphere's own GPU-made chain is untouched.
  - Test: `environment::bank_tests::legacy_chains_bank_their_narrow_levels`. A level linear in texel coordinates reads back exactly inside every face, a constant stays constant, wide levels stay mips, and a chain without narrow levels is not banked. Recorded in [api.md](api.md) under `create_environment`.
  - Lib: **207 passed, 0 failed, 164 ignored** (release test binary of the head, inside lease `pshow2-h`; main had 206).

| Showcase, map sky (head `374265f`, lease `pshow2-h`) | Ara span | shading |
|---|---:|---:|
| RT still, main (`pshow2-e`) | 52.48 | 43.60 |
| **RT still, banked** | **33.89** | **25.10** |
| RT moving, banked | 33.14 | 27.77 |
| non-RT still, main (`pshow2-e`) | 64.53 | 53.18 |
| non-RT moving, banked | 45.68 | 34.88 |
| RT constant sky (head) | 19.74 | 8.50 |
| non-RT constant sky (head) | 30.91 | 20.85 |

  - **Images (M, inspected).** RT still, main against banked: whole frame mean 0.19 codes, max 2, no pixel over 8 (`crops/sky-balls.png`, balls and stage at x2). Moving frame 30 on both paths (`crops/sky-move.png`): clean reflections on the balls and turntable, no stray white pixels, and RT and non-RT agree.
  - The non-RT map still timed out on its first, cold-pipeline run, as did the warm-up; both are 120 s player timeouts while every pipeline recompiled for the changed shader text. Non-RT therefore has moving numbers only. Against main's non-RT map still (64.5), the banked moving frame is 45.7 ms.
- **Still open: about 17 ms of RT shading under a map** (25.1 against 8.5 ms on the constant sky). The integration thread measured about the same with the full-size independent layout (A: 23.8 against 8.9 ms). So it is not the face-edge path; it is the lookups' own cost at every glossy ray and hit. Candidates:
  - RGBA32F texels: half precision halves the bytes;
  - full-size faces at high roughness in the independent layout: about 25 MB of banks;
  - two levels and eight loads per lookup on every one of 16 strata.

  The next unit should profile it with Nsight on the hit and miss lookups.

## 4. Showcase's brushed aluminium ball reads matte white (both paths)

**Which side it's on (C, M).** Both, and not in the material mapping.
- **The material is a plain rough metal.** `ball_aluminium` is `Material::flat("brushed aluminium", [0.91, 0.92, 0.92], 0.42, 1.0)` (Ara `crates/ara-samples/src/build/showcase.rs`). It has no anisotropy and no texture; "brushed" is only its name. The bridge (`scene.rs`, `material`) carries base colour, metallic and roughness through unchanged, so anisotropic GGX is not the missing feature.
- **Bridge: the sky (C, A).** Before `eb20a48b5` the bridge drew Showcase's gradient sky as `Sky::Uniform` at its mean radiance, so a 0.42 metal had no horizon to reflect. Ara's ball shows a floor/sky horizon; ours was uniform (`crops/pre-alu.png`: the integration thread's release captures, Ara Epic, Renderlab RT and non-RT, x4). Correction to my first message to that thread: Showcase's player sky is a gradient, not the studio HDRI; the HDRI is only used by `ara-samples`' own capture tool (A). The bridge now passes it as `Sky::Map`, and section 3 is the cost that exposed.
- **Renderlab: the untraced half of a rough lobe sees the sky about the normal (C).**
  - Roughness 0.42 sits in the fade band, traced fully to 0.35 and untraced from 0.5, so 53 % of its lobe is traced.
  - The untraced half is `indirect_radiance(reflected, roughness)`: the sky's prefiltered radiance about the reflection, times `es_visibility`, plus the cache's bounce about the reflection.
  - `es_visibility` is the receiver's cosine-weighted sky visibility about its normal (`es_prepare`). On the ball's lower front the reflection points at the floor while the normal still sees half the sky, so the lobe reads half the sky where the floor is dark. On F0 0.91 that is a uniform white.
- **Built, not screened.** `es_visibility_about(direction)` takes the cached occluded sky's L2 SH irradiance about the reflection direction, against the open sky's, for `indirect_radiance` and the lobes' shares. It sits behind a screen switch (`RENDERLAB_SCREEN_SPEC_VIS`, branch `t3/p-show2-screens`). `examples/reflection_matched.rs --metal-balls` (merged in step 1) adds Showcase's aluminium and copper balls on the turntable, in the path-traced reference too, for that screen. Its leases were withdrawn for the root steers, so it is unmeasured and not merged.

## 5. Glossy reflections at a lower rate (step 3): not run

The screens were written and built (lease plans `ab`, `c`), but the root steers moved them behind the regression and the sky map. RL_GLOSSY_MAX is capped at 4 and 8 through `RENDERLAB_GLOSSY_MAX` on `t3/p-show2-screens`. Section 3 changes their premise: with a sky map, most of the glossy cost is the per-ray environment lookup, so a ray-count change should be timed after the lookup fix. The design and patent mapping stand as in [p-defect.md](p-defect.md) section 5: no reservoirs or resampling (US 11,315,310), no variance clipping (US 10,116,916), not SVGF.

## Quarry rows (root steer: ratio against Unreal Epic)

Matched quarry, 1080p, GPU p50, one visit per path in lease `pshow2-h`, the head's `p_yard_capture` example run with `tools/p-yard/rl.py`'s command line (manifest, `quarry`, path, out, `timing`). rl.py itself writes into P-YARD2's evidence folder, so the binary was called directly. Unreal from [p-yard.md](p-yard.md) (P-YARD2).

| Path | State | Renderlab head | P-YARD2 Renderlab | Unreal Epic | Ratio vs Epic |
|---|---|---:|---:|---:|---|
| RT | still | 4.16 | 3.82 | 4.50 (HW Lumen) | 1.08x faster |
| RT | moving | 4.32 | 4.26 | 4.57 | 1.06x faster |
| RT | cut | 12.70 | 12.6 | ~4.5 | 2.8x slower |
| non-RT | still | 8.66 | 8.65 | 4.02 (SW Lumen) | 2.15x slower |
| non-RT | moving | 10.74 | 10.57 | 4.12 | 2.61x slower |
| non-RT | cut | 43.34 | 43.1 | ~4.1 | 10.5x slower |

None of this unit's commits move the quarry; the quarry has no skinned mesh and draws the atmosphere's own chain, which is not banked. The differences are within P-YARD2's visit spread (RT still visits 3.33 and 4.30).

## Process

- **Hardware:** RTX 4070 SUPER (12,282 MiB), NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11; shared machine (Ara units building and capturing between leases; the integration thread's captures ran just before ours).
- **Build identity:**
  - Showcase screens: Ara `eb20a48b5` (detached worktree, `renderlab` patched to this tree) built at this branch plus the screen commits (`t3/p-show2-screens` `91bf40d` plus the miss-lookup switch), release, 275 s.
  - Head `374265f` (careful restart): one build session of Ara release, the lib test binary (release) and `p_yard_capture` (release), 743 s, at 20 GiB free.
  - Lease E's main-behaviour rows come from the screen binary with every switch unset. The switches only rewrite shader text, or bypass a branch, when set.
- **Builds:** build slot plus stillkit mutex, two jobs, one at a time:
  - Ara cold 631 s;
  - Ara incremental 46 s and 275 s;
  - one examples build stopped before it ran, to lease first;
  - quarry example and lib (section "Quarry rows").
- **Leases:** `scripts/n8-run.py`, owner `p-show2`:
  - `pshow2-a`: 17 runs, all failed. The timeline's resolve offsets were 384-byte strides; `QUERY_RESOLVE_BUFFER_ALIGNMENT` is 256. Fixed (64 marks a frame, 512 bytes, checked at compile time).
  - `pshow2-a2` and `pshow2-ab`: withdrawn before admission for the root steers.
  - `pshow2-e`: 22 runs, about 11 min.
  - `pshow2-f`: withdrawn before admission, for the 15:02 admission rules and the machine stop.
  - `pshow2-h` (careful restart, only unit): lib, 7 Showcase captures, 2 quarry visits; 12.3 min, at 19-20 GiB free.
- **Time:** about 2 h of code and analysis; about 37 min of GPU leases; about 2 h 30 min queued behind Ara builds and captures, the 15:02 rule change and the machine stop.
- **Runners:** `tools/build.py` checks admission first and then takes the stillkit mutex and slot without waiting (the 15:02 rule). It starts at 10 GiB free and stops its build tree below 5 GiB. Leases start at 10 GiB and abort at 5 GiB. Every line posted to the schedule starts with the free RAM.
- **Evidence:** `D:/Coding/renderlab-work/p-show2/`:
  - `showcase/captures/*.json`, with `renderlab_passes` and per-frame `*.passes.jsonl`;
  - `e-summary.txt`;
  - `crops/`, `plans/`, `runs/`;
  - `tools/`: `plans.py`, `batch.py`, `build.py`, `lease.py`, `summarize.py`, `sheet.py`.

  Video frames and targets were deleted at the end; the Ara worktree (source only) stays for the next Showcase unit.
