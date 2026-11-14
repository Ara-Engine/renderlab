# Camera cuts, first change: a 64-direction cold frame with a three-frame catch-up

Implementable spec for the first change of [design-cuts.md](design-cuts.md), written on main `5eb6009` (2026-11-11). Reading, a CPU model of the sun estimator and arithmetic only: no build, no GPU, no capture. Patents no longer constrain methods (`AGENTS.md`, 2026-11-10); the one habit kept is to name a flagged number where a method had one, and nothing here uses a flagged method.

Labels: **M** measured in a repo document (named); **C** read from code (file and line on `5eb6009`); **S** reported by a source; **E** this spec's estimate, model or proposed parameter. Every forecast is **E**.

**The gap.** The frame after a camera cut costs 12.60 ms on RT against Unreal's 4.52 (2.8x) and 43.11 ms on non-RT against 4.11 (10.5x) (M, [p-yard.md](p-yard.md) table, quarry, 1080p, RTX 4070 SUPER). The RT cut excess over its recovery window (3.53 ms) is 9.07 ms; P-RTS attributed an earlier head's 11.60 ms fresh-primary and shadow cut to the per-receiver sun rays (fitted 0.044 ms per full-screen ray), so most of it is probably sun work, which is the premise of this change (E: not re-attributed on this head). On the non-RT quarry the sun march alone is about 28 of 32.8 ms of the cut sun pass (M, P-NSQ2 via [m7-gaps.md](m7-gaps.md)).

**The change, in one paragraph.** Trace 64 sun directions per receiver on the cut frame instead of 256 (RT: 64 rays; non-RT: 8 batches of 8). Spread the other 192 over the next three frames, 64 each, taking each frame's directions from where the previous one stopped (a direction cursor), so TR1's running mean over frames 0-3 is exactly the 256-direction estimate the old cut frame made alone. Nothing else about the cut changes: history reset, fog reset, texture verification, GI and shading are as they are.

## 1. The cut path on current main

What changed since the first plan (C, `git log 73eee44..5eb6009`): P-STILL (GI idles on the unjittered camera, `src/gi_cache.rs`), P-FLICKER (TR1 Catmull-Rom window, `sharp_blur_budget 2`), P-PIPE, shared CPU planners, the sun-shadow spec (PR #14). None of them moves the cut: P-STILL's table has the cut at 12.67 → 12.74 ms RT and 44.46 → 40.61 ms non-RT with an identical image (M, [p-still.md](p-still.md); the non-RT difference is single-frame noise). The cut-time sun budget is unchanged: cold 256, reset 64, active 32, held 8 (C `src/temporal_reconstruction.rs:310-317`).

| Step | Where (C) |
|---|---|
| Entry: `Engine::cut()` calls `cut_reconstruction()`, `reset_atmosphere_history()`, `mark_camera_cut()`; it does not clear the GI table | `src/engine/mod.rs:1043-1047` |
| `cut_reconstruction` sets `state.cut = true` | `src/primary_ray_gpu.rs:403-407` |
| Next prepared frame: `cold = previous.is_none() \|\| state.cut`; `state.reset = ... \|\| state.cut ...`; `state.cut = false` | `temporal_reconstruction.rs:1131`, `:1168-1169` |
| Budget choice: cold 256, else reset 64, else held 8 after 16 still frames, else active 32 | `ReconstructionShadowBudget::rays`, `:336-346`; called `:1236` |
| RT: the budget reaches the trace kernels as `primary_frame.sampling.x`; the direction index is `j + primary_frame.sided.y` (the rotation word) | `ROTATION_SEAM` `:289-292`; word written `src/primary_ray_gpu.rs:1666-1670` |
| Non-RT: budget becomes `batches = rays / 8`; Surface march or, when `chosen`, the S4 gather | `src/virtual_shadow_gpu.rs:1411-1445`, `sunvis_s4.rs:28`, `:44-46` |
| Direction sequence: `r = fract(index * (0.75488, 0.56984) + per-pixel shift)`, mapped to the sun disk by `sqrt(r.x)` and `2 pi r.y` | `shaders/fresh_shadows.wgsl:25-29`; shift from the pixel hash `:52-54` |
| Frame rotation: `rotation(k) = 8 * (k & 31)`; used by the RT word above and by `virtual_rotation` | `temporal_reconstruction.rs:376-378`, `virtual_shadow_gpu.rs:1401-1406` |
| TR1 resolve: a reset pixel starts from this frame's samples; a continued one takes `m' = m + w (x - m) / n`, `n = history weight + w` | `shaders/temporal_resolve.wgsl:608-633` |

Two facts in that table decide the design:

1. **The old rotation duplicates directions when the budget is above 8.** Frame k's directions are indices `8 (k mod 32) ... + rays - 1`, a shift of 8 per frame. With 64 rays, frame 1 repeats 56 of frame 0's 64 indices, and the per-pixel shift is fixed, so TR1's mean over those frames gains almost nothing. (At a budget of 8 the shift is one full batch, which is what the held state wants.) Spreading the cut's budget over later frames therefore needs the cursor, not the shipped rotation.
2. **The accumulation is an equal-weight running mean** (`n` counts frames until the window), so four frames of 64 consecutive indices average to the same 256-direction estimate the cut frame makes today.

Model (E, a CPU program of the engine's sequence and shift, half-plane occluder across the sun disk, 4,000 random per-pixel shifts, 41 penumbra positions): four averaged 64-direction frames starting at `8 k` (shipped rotation) leave an rms visibility error of **1.32%** (max 6.6%); starting at `64 k` (cursor) **0.63%** (max 2.9%), the 256-direction value.

## 2. Specification

**Parameters** (`ReconstructionShadowBudget`, `temporal_reconstruction.rs:301-317`):

| Field | Now | Change |
|---|---|---|
| `cold_rays` | 256 | **64** |
| `catchup_rays` | n/a | new; 64 (RT). Non-RT: 32 in the first screen (see section 3). Power of two in 8..=256, as the others (`validate`, `:321-334`) |
| `catchup_frames` | n/a | new; **3**. Zero disables the catch-up (variant B below) |
| others | 64 / 32 / 8 / 16 | unchanged |

**State.** `ReconstructionState` (`:630-640`) gains `frames_since_cut: u32` and `cursor: u32`. In the prepare step that sets `cold` (`:1131`): on a cold frame set `frames_since_cut = 0`, `cursor = 0`. On later frames `frames_since_cut += 1`.

**Budget rule** (replaces `rays`'s body for the frames after a cut):
- cold frame: `cold_rays`;
- else if `frames_since_cut <= catchup_frames` and the frame is not a reset (`state.reset`, a sun turn, a light restart): `catchup_rays`;
- else the existing order (reset 64, held 8, active 32).
A reset during the catch-up (sun turn, another cut) ends it: the rule falls to the existing branches and the counters restart on the next cold frame.

**Direction start.** While `frames_since_cut <= catchup_frames`, the frame's first direction index is `cursor` (mod 256), then `cursor += rays`. After the catch-up the shipped `rotation(k)` resumes. Both writers take it from one function so they cannot drift: the RT word (`primary_ray_gpu.rs:1666-1670`) and `virtual_rotation` (`virtual_shadow_gpu.rs:1401-1406`). The budget is chosen in `prepare_reconstruction` (`:1236`) and the word is written when the frame words are built; confirm the order puts the budget first (not checked here).

**Unchanged, on purpose.** The cut flag, TR1's cold start (`start_radius 1`, frame 0 unjittered), fog reset, residency verification, the GI table, reflections, the S4 selection rule, and every frame from the fourth on. `set_reconstruction_shadow_budget` still resets the history when the budget changes (`:753-763`), so variants switch with a cut after the change.

**Non-RT note.** 64 rays are 8 batches, exactly `MIN_BATCHES` (`sunvis_s4.rs:28`), so S4-eligible scenes still choose S4 on the cold frame, and the quarry (not eligible, full-detail BVH too large) takes the march. P-S4C estimated an 8-batch restart as close to a tie between the two (M, [p-s4c.md](p-s4c.md), not timed): remeasure the selector at 8 batches.

## 3. Per-frame budget and forecast (E)

Cost model from two measured states per path: moving (32 directions) and cut (256): the slope is `(cut − moving) / 224` directions. RT: `(12.60 − 4.26) / 224 = 0.0372 ms per direction`. Non-RT: `(43.11 − 10.57) / 224 = 0.145 ms`; P-NSQ2's march attribution gives a lower slope, `28 / 256 = 0.109 ms`, so a range. The first plan's fit (0.044 ms per RT ray, P-RTS) gives 4.15 ms for the RT cold frame; the first-pass range is 4.2-5.5.

| Frame | Directions (RT) | RT ms (E) | Unreal | Directions (non-RT) | non-RT ms (E) | Unreal |
|---|---|---|---|---|---|---|
| now, F0 | 256 | 12.60 M | 4.52 | 256 | 43.11 M | 4.11 |
| now, F1-F3 | 32 each | about 4.26 | 4.52 | 32 each | about 10.57 | 4.11 |
| **new F0** | 64 | `12.60 − 192 × 0.0372 =` **5.45** | | 64 | `43.11 − 192 × (0.109..0.145) =` **15.2-22.2** | |
| **new F1-F3** | 64 each | `4.26 + 32 × 0.0372 =` **5.45** | | 32 each (no catch-up first) | **10.57** | |
| directions F0-F3 | 256 (was 352) | | | 160 (was 352) | | |
| sum of 4 frames | | **21.8** (was 25.4; Unreal 18.1) | | | **46.9-53.9** (was 74.8; Unreal 16.4) | |

- **RT:** peak 12.60 → about 5.45 ms (2.8x → about 1.2x of Unreal), four-frame sum down 3.6 ms, directions equal the old cut frame's by F3.
- **Non-RT:** peak 43.11 → 15-22 ms (10.5x → 3.7-5.4x). The rest of the non-RT cut is not sun: the first plan estimated 22-27 ms from a similar scaling; the new number uses the moving-frame floor. A catch-up of 64 on non-RT costs another 3.5-4.6 ms each frame (total directions 256, four-frame sum 61-64 ms): screen it as variant C-64, keep 32 as the default until it shows a visible gain.
- **S4-eligible scenes (game class):** S4 spends 5.08 of its 9.07 ms game cut on traversal and classification regardless of batches and 3.99 on direction tests (M, p-s4c.md); at 8 batches the tests fall to about 1 ms, so the cut falls to roughly 6 ms (E).
- **Not covered:** GI cell creation, texture-residency replay and shading, which this change leaves at what they were.

## 4. What the first three frames look like

**Sun estimator (E, the model of section 1; visibility error across a penumbra, rms / worst position / max):**

| Cumulative directions | Frame | rms | worst position | max |
|---|---|---|---|---|
| 256 (old F0) | | 0.63% | 0.93% | 2.9% |
| **64 (new F0)** | 0 | **1.65%** | 2.5% | 7.8% |
| 128 (new F1 mean) | 1 | 1.07% | 1.47% | 4.8% |
| 192 (new F2 mean) | 2 | 0.78% | 1.09% | 3.8% |
| 256 (new F3 mean) | 3 | 0.63% | 0.93% | 2.9% |
| 32 (a moving frame today) | | 3.1% | 5.4% | 12.6% |

So the new cold frame is noisier than the old one (2.6x in penumbra rms), but still cleaner than every moving frame the renderer shows now (3.1%), and by the fourth frame it equals the old cut frame. In 8-bit terms, a penumbra pixel whose lit value is about 120 codes moves by about 2 codes rms on F0 (3 at the worst position, up to 9 at the single worst pixel), against 1 code on the old frame. Only penumbra pixels are affected; umbra and fully lit pixels are exact at any count.

**Image against the old frame (E).** Penumbra pixels are a few percent of a quarry frame (unmeasured). Full-frame error barely moves: the cut frame's error is 7.71 RT and 8.26 non-RT against Unreal's 9.57 and 10.76 (M, p-yard.md), so the new F0 has room. The edge metric is the one to watch: 14.72 against HW Epic's 14.28 at the old cut frame (M); adding about 2 codes in quadrature raises it to about 14.8-15.0 (E).

**Unreal's first frames (S, M).** TSR documents a camera cut as a frame with current data only: a spatial anti-aliasing fallback hides aliasing while detail accumulates, so the first frames are softer and less detailed; at 50% input and 60 Hz its sample acquisition takes about 4 frames ([UE 5.7 TSR](https://dev.epicgames.com/documentation/en-us/unreal-engine/temporal-super-resolution-in-unreal-engine?application_version=5.7), as cited in design-cuts.md). Lumen amortizes surface-cache lighting over frames, so uncached regions can start dark or incomplete ([UE 5.7 Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine?application_version=5.7)). P-YARD2's own warm same-region cut shows neither: "no engine shows a visible cold-history artefact on the cut frame or the next frame at display scale" (M, p-yard.md, inspected). So against Unreal the new frames are judged by three things, not by a darkness comparison that the captured pair does not support:
- F0-F3 full-frame and edge error at or below Unreal's cut-frame numbers (above), not above the old Renderlab frame by more than the model's amount;
- no new penumbra grain, edge dots, flicker or brightness pulse at display scale in a frame-by-frame and normal-speed view;
- the frame-to-frame step F0 → F1 → F2 smaller than Unreal's across the same frames (their frames are in the PC's P-YARD2 captures, frames 320-323; not in the repo).

## 5. Code locations to change

| File | Change |
|---|---|
| `src/temporal_reconstruction.rs` | `ReconstructionShadowBudget`: add `catchup_rays`, `catchup_frames`; `cold_rays` default 64; `validate` covers the new field; `rays()` takes the state's `frames_since_cut` and the reset flag; `ReconstructionState` gains `frames_since_cut`, `cursor`; set/advance them next to `cold` (`:1131`) and the budget call (`:1236`); one function `direction_start(k)` returning the cursor during catch-up else `rotation(k)` |
| `src/primary_ray_gpu.rs:1666-1670` | write `direction_start` instead of `rotation` |
| `src/virtual_shadow_gpu.rs:1401-1406` | `virtual_rotation` returns the same value |
| `src/temporal_reconstruction.rs:1669-1676` (test) | budget cases for cold, catch-up, reset during catch-up; a test that frames 0-3's index ranges are `0-63, 64-127, 128-191, 192-255` with no repeat |
| `examples/p_yard_capture.rs:88-97` | a `RENDERLAB_P_CUT_BUDGET=cold,catchup_rays,catchup_frames` knob beside the P-FLICKER ones (measurement only, unset changes nothing) |
| Not touched | shaders (the shift and sequence are unchanged), `ROTATION_SEAM`, S4, GI, atmosphere, residency |

Variant B needs no code: the `--diag shadow-budget COLD,RESET,ACTIVE,HELD,SETTLE` knob already exists (`support/estimator_diag.rs:104-114`) and `p_yard_capture` sets the budget the same way (`:93-96`).

## 6. Screening experiment

One quarry, one lease under 15 minutes, release binary, both paths, fixed exposure, the P-YARD2 manifest and cut (frame 320, `tools/p-yard`). Variants in one process, switching the budget and then cutting, as the setter resets history:
- **A** baseline: 256 / 64 / 32 / 8.
- **B** cold 64, no catch-up (existing knob).
- **C** cold 64 and catch-up 3 x 64 (RT), 3 x 32 and 3 x 64 (non-RT).
Alternate A B C A, three cuts each, with `RENDERLAB_PASS_LOG` for per-pass times, and save raw lighting and final images at frames 0, 1, 2, 3, 4, 8, 16 of each cut, plus a normal-speed clip of A and the winner.

Before the lease (CPU, minutes, uses a saved dense cut sun-visibility buffer if one survives): replay the estimator at 64 directions with the shipped `8 k` start and with the cursor against the 256-direction values pixelwise, and report the penumbra-pixel fraction and rms. The section 4 model predicts 1.65% and, for the 4-frame mean, 1.32% against 0.63%. If the real fraction of penumbra pixels is far above a few percent, or the rms differs from the model, stop before building.

**Pass.** Peak cut frame and four-frame sum both lower than A, with the RT cut frame near 5.5 ms and non-RT near 15-22 ms; F0-F3 full-frame error not above A's by more than 0.1 codes and edge error not above A's by more than 0.3 codes; no visible new grain or edge dots at display scale; frame 3 equal to A's frame 0 within the sun term (this also tests the cursor: variant C with the shipped rotation as a fourth arm should show the 1.32% error).
**Fail.** Visible penumbra grain, a contact speck increase or a brightness step at F0 or F1; the cut peak moving to F1-F3 (non-RT with a 64 catch-up) without a lower sum; the S4 selector at 8 batches costing more than the march it replaces.

## 7. If it fails

- Grain at F0 only: raise `cold_rays` to 128 (cumulative 128 + 3 x 64 = 320), cost about 3.5 ms RT more; do not add a blur, and do not borrow pre-cut history.
- Timing gain smaller than the model on non-RT: report which pass holds the rest (GI claims, residency replay) before touching the sun again; the second plan of design-cuts.md (prefetch an announced destination) is the next candidate.
- The cursor and catch-up are independent: B alone is the cheapest to ship, C only if frames 1-3 show the benefit.

## 8. What reading cannot determine

- The real fraction of penumbra pixels in a quarry cut frame, and so the pixel-count effect on error; the model is for one half-plane occluder, not for the quarry's mix of contacts and thin casters.
- The cost slope per direction on the current head (derived from two measured states, not a sweep), the non-RT split between sun and the rest, and whether frames 1-3 at 64 are cheaper than the arithmetic (cached rays) or dearer (the 8-batch selector).
- That the budget is chosen before the frame words are written (only the ordering was read).
- Unreal's first three frames: the repo has the metrics, not the frames.
