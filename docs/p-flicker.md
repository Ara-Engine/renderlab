# P-FLICKER: motion flicker against Unreal

Unit P-FLICKER, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10/11, branch `t3/p-flicker` on main `f64c82e`. Labels: **M** measured here, **C** read from code, **A** another unit's measurement, **I** inference.

## Result

- **Step 1 (M, C): the validation layer never touched the yardstick.**
  - P-YARD2's `tools/p-yard/rl.py` (and P-STILL's `run.py`) removed `VK_INSTANCE_LAYERS` and `VK_LAYER_PATH` from every Renderlab child, and Unreal runs `-dx12 -sm6`, where no Vulkan layer can load.
  - The rerun with the layer off at current main reproduces every P-YARD2/P-STILL GPU number within run-to-run noise, so the Unreal numbers are reused.
  - The non-RT gap (2.1-2.6x slower than SW Lumen Epic) and the cut spike (10.5x) do not shrink, so the order after this unit stands: non-RT sun shadows, then cuts.
  - P-YARD2's RT still 3.82 ms was the median of a bimodal pair of visits (3.33 and 4.30). RT still is 3.35 ms, 1.34x faster than HW Lumen Epic.
- **What the layer did cost (M).** With it on, GPU passes are unchanged. CPU submit is 2.5-2.9x slower, and that adds a +0.8-0.9 ms CPU bubble inside the RT still span only. Units that timed through `n8-run.py` before `901c1d2` without removing the layer have inflated `cpu_submit` and short RT spans; their GPU pass times stand.
- **Step 2 (M): the flicker is TR1 resampling texture under motion, not lighting noise.**
  - GI off moves flicker 4.39 → 4.36; reflections off changes nothing (byte-identical score); the two paths differ by 0.05.
  - 96% of the residual energy is on surface interiors, not edges. It rises steeply with the reference's texture contrast: 65% of it lies where the reference changes by 10 or more codes per pixel. There the residual is 9.5 codes RMS; on flat areas it is 2.3.
  - No jitter reaches only 4.15. The heat map ([energy-base-cap-rt.png](#2-flicker-split)) is spread over every textured rock and gravel face, brightest on the sunlit gravel.
- **Cause (C, 1-D model).** A moving pixel whose 16 history taps validate resamples history by Catmull-Rom with a fixed 16-frame window. Catmull-Rom is free of second-moment blur, but it still softens texture by an amount that depends on the fetch fraction, which changes every frame. A fixed window lets that frame-varying blur accumulate, so texture shimmers. Bilinear already caps its window by T1b's blur budget; Catmull-Rom has no such cap.
- **MERGE: `Reconstruction::sharp_blur_budget` = 2, default on (`d053fca`).** It caps the Catmull-Rom window at `1 + 2 / sum f(1-f)`, the bilinear rule with its own budget.
  - Flicker: RT 4.392 → **4.323** (-1.6%), non-RT 4.438 → **4.375** (-1.4%).
  - Moving full-frame error: RT 7.237 → 7.123, non-RT 7.722 → 7.633.
  - Still, cut and recover frames are byte-identical, because no other frame takes that path. Moving edge error is unchanged.
  - Cost is unchanged in interleaved visits: RT moving 4.30 / 4.30 → 4.30 / 4.29 ms, non-RT 10.66 / 10.69 → 10.65 / 10.66.
  - The S4 door changes only on its moving edge, with no added trail. S5 is identical.
  - Lib tests: 207 passed.
  - The model's gain (15-34%) did not carry over: the quarry's excess is mostly elsewhere (next point).
- **Gate missed: flicker is still above Unreal's.** 4.32 / 4.38 against 3.68 / 3.69. Excluding steps above 20 codes, Renderlab is at 3.73 and Unreal at 3.38. The remaining gap is single-pixel flashes along thin cracks and crevice edges:
  - 1,792 pixels a frame flash by more than 20 codes, against Unreal's 1,109;
  - steps above 40 codes carry 8% of Renderlab's energy and 2% of Unreal's;
  - the rock step sheet shows them as dotted bright lines along cracks that neither Unreal's step nor the reference's has.
  - `cr=0` (no Catmull-Rom/bilinear switching) does not remove them (4.424), and neither does GI off. They are the next unit's target (section 4).
- **Not the cause (M):** the GI cell churn P-STILL suspected. In motion the cache creates 56 cells a frame and bucket-full fires about 9 times a frame, but GI off moves flicker by only 0.03, so the eviction follow-up cannot close this gap. It stays a cost item (P-STILL §4.2), not a flicker item.

| Quarry 1080p, GPU span p50 (ms) | P-YARD2 / P-STILL (old) | P-FLICKER, layer off (new) | vs Unreal Epic, new |
|---|---:|---:|---|
| RT still | 3.82 (visits 3.33 / 4.30) | 3.35-3.36 | 1.34x faster (HW Epic 4.50) |
| RT moving | 4.26 | 4.29-4.31 | 1.06x faster (4.57) |
| RT cut | 12.60 | 12.68-13.96 | 2.8-3.1x slower (4.52) |
| RT after cut | 3.53 | 3.64-3.73 | 1.20-1.23x faster (4.48) |
| non-RT still | 8.65 | 8.58-8.63 | 2.14x slower (SW Epic 4.02) |
| non-RT moving | 10.57 | 10.67-10.73 | 2.6x slower (4.12) |
| non-RT cut | 43.11 | 42.81-44.30 | 10.4-10.8x slower (4.11) |
| non-RT after cut | 8.71 | 9.00-9.03 | 2.2x slower (4.04) |
| Flicker, moving (RT / non-RT) | 4.392 / 4.438 | 4.392 / 4.438 | HW Epic 3.680, SW Epic 3.686 |
| Flicker with budget 2 (merged) | | 4.323 / 4.375 | still above Epic by 17% / 19% |

New ranges are the two timing visits (`base-t0`, `base-t1`). Cut frames are single observations. Unreal Epic is P-YARD2's (D3D12, unaffected).

## 1. Validation layer A/B (M)

One lease (`pflk-l2`), one binary, the quarry yardstick manifest. `passes` runs add `RENDERLAB_PASS_LOG`; `layer` runs set `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` as `m6-film-run.py` did before `901c1d2`.

| p50 | RT off | RT on (passes / plain) | non-RT off | non-RT on (passes / plain) |
|---|---:|---:|---:|---:|
| still span ms | 3.38 | 4.32 / 4.14 | 8.61 | 8.60 / 8.63 |
| moving span ms | 4.33 | 4.40 / 4.37 | 10.70 | 10.73 / 10.71 |
| cut span ms | 13.83 | 12.72 / 12.80 | 43.21 | 43.76 / 45.17 |
| still CPU submit ms | 0.67 | 2.15 / 1.93 | 1.40 | 3.74 / 3.45 |
| moving CPU submit ms | 0.91 | 2.25 / 2.02 | 1.47 | 4.22 / 3.49 |

**Which numbers had the layer (C, from each runner's code and lease records).** Before `901c1d2`, `m6-film-run.py` set `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` on every leased child. A unit's numbers were affected unless its own runner removed the variable.

| Harness | Layer | Evidence |
|---|---|---|
| P-YARD2 `tools/p-yard/rl.py` (the Unreal yardstick, Renderlab side) | **off** | removes `VK_INSTANCE_LAYERS`, `VK_LAYER_PATH` |
| Unreal, every run (`tools/p-yard/ue.py`, `scripts/m7-u0*.py`) | n/a | `-dx12 -sm6`; Vulkan layers cannot load |
| P-STILL `tools/p-still/run.py` | off | as rl.py |
| ARA1 `tools/ara1/capture.py` (both engines, release profile) | off | removes the layer unless `--validate`, since `697bc97` (2026-11-09 05:36) |
| P-FREE (`gpu-layer-bench/tools/plan.py`) | off | removes `VK_INSTANCE_LAYERS`, `VK_LAYER_ENABLES` |
| P-NRT `tools/p-nrt/plan.py`, P-RTS `ab.py`/`final.py`, P-SHC2, P-LTS4, P-RETIME, P-MEM retime | off | each removes it |
| **P-NSQ, P-NSQ2, P-NSQ3** (`D:/Coding/renderlab-work/p-nsq*/tools/plan.py` running `p_nsq_screen` and ARA1 binaries) | **on** | `env = dict(os.environ, ...)`; leases `l1`... run under n8-run (P-NSQ with `--fast-check`) |

- **Ratios against Unreal: none change.** Every Unreal ratio quoted since P-YARD2 comes from layer-free runs on both sides.
- **P-NSQ1-3's timings:**
  - their non-RT GPU pass times stand, because the layer moved no non-RT pass by more than 0.07 ms;
  - their CPU submit and any short RT spans read high.
- **The RT still correction:**
  - I posted "RT still 3.35 vs 3.82 (P-STILL key; 1.34x faster, was 1.18x)" in the interim note. That attribution was wrong.
  - P-YARD2's 3.82 ms is the median of a bimodal pair of visits, 3.33 and 4.30, likely wallpaper contention.
  - P-STILL measured 3.35 on the yardstick before and after its change, with byte-identical frames.
  - So RT still was 3.35 ms all along: 1.34x faster than HW Lumen Epic, not 1.18x. Nothing in the code changed it, and the layer did not cause it.

Per pass (sum of named stages): RT still 3.40 → 4.31, all in the `visibility` stage (+0.93), which is where the frame waits on the slower CPU encode. RT moving gains 0.07, and non-RT still/moving gain at most 0.07 (`gi_gather` +0.02-0.06). No GPU pass got slower.

## 2. Flicker split

`tools/p-flicker/split.py` pools P-YARD2's flicker residual exactly: `(image_t − image_{t−1}) − (ref_t − ref_{t−1})` at L0, over the 32 moving pairs, on the all-covered same-placement mask. The headline matches `score.py` to three decimals. The same squared sum is split by the reference's populations and by bins of its local contrast (codes, 4-neighbour gradient). Shares are of the headline's squared sum.

| Run | Flicker | interior | edge | contact | lit | contrast <2 | 2-5 | 5-10 | 10-20 | ≥20 |
|---|---:|---|---|---|---|---|---|---|---|---|
| RT, main | 4.392 | 4.36 / 96% | 5.19 / 4% | 5.88 / 7% | 4.08 / 70% | 2.25 / 3% | 2.68 / 11% | 3.59 / 22% | 5.34 / 31% | 9.51 / 34% |
| non-RT, main | 4.438 | 4.41 / 96% | 5.27 / 4% | 5.90 / 7% | 4.13 / 70% | 2.29 / 3% | 2.72 / 11% | 3.63 / 22% | 5.39 / 31% | 9.58 / 34% |
| RT, GI off | 4.361 | 4.30 / 94% | 5.89 / 6% | 6.04 / 8% | 4.03 / 69% | 2.28 / 3% | 2.71 / 11% | 3.58 / 22% | 5.29 / 30% | 9.38 / 33% |
| RT, reflections off | 4.392 | identical to main |  |  |  |  |  |  |  |  |
| RT, no jitter | 4.153 | 4.12 / 96% | 4.99 / 4% | 5.25 / 7% | 3.93 / 72% | 2.23 / 3% | 2.65 / 12% | 3.51 / 24% | 5.11 / 31% | 8.49 / 30% |

Cells are RMS codes / share. Non-RT with GI off scored 4.395 on the earlier pairing, the same as RT with GI off.

- **Sun shadows** cannot be the main term: 70% of the energy is on sunlit pixels, whose sun visibility is 1 and carries no sampling noise.
- **GI** is deterministic per cell (W1). Removing it changes 0.03.
- **Reflections** are outside the quarry's traced roughness band.
- **Edges and silhouettes** carry 4%. TR1b's edge work already took them to TSR parity.
- **What is left** is the reconstructed texture itself. It shimmers as it moves, in proportion to its contrast.

Heat map of mean squared residual per pixel (RT, `D:/Coding/renderlab-work/p-flicker/sheets/energy-base-cap-rt.png`; no-jitter alongside): averaged over the walk, the energy covers every textured face and is brightest on the sunlit gravel at the left.

**Single-pixel flashes (M, from the saved frames).** Frame by frame, the steps look different from that average. In the step sheet (`sheets/test-base-steps.png`: |frame t − frame t−1| × 4 for RT, non-RT and the reference, frames 161-164, crop 600,560 240x140 at 2x), Renderlab's steps carry scattered single bright pixels on the rock, on both paths, that the reference's step lacks. There are also a few short horizontal streaks. The reference step is broader and fills the fine gravel more evenly.

| RT walk pairs | energy share |
|---|---:|
| pixels with residual > 10 codes | 55% |
| > 20 codes | 25% |
| > 40 codes | 8% (5% without jitter, 6% with GI off) |

- **Count.** 1,200-2,800 pixels a frame are above 20 codes. Half brighten and half darken.
- **Flashes.** At those pixels the next pair's residual correlates at −0.3 to −0.65, so they are mostly one-frame flashes: the pixel jumps and comes back.
- **Where.** They are slightly enriched in contacts (10-16% against 7% of pixels), not on edges (4%), and on brighter sunlit rock (median 83 against 76 codes).
- **Budget.** Removing the >20-code flashes alone would take RT to about 3.8. Unreal is at 3.68.
- **Cause, two candidates; the saved frames cannot separate them.**
  - Sparse bright HDR samples (sun glints off the normal-mapped rock) entering at weight 1/16.
  - Pixels switching between Catmull-Rom (window 16) and the bilinear fallback (window about 3-5 at f ≈ 0.5) when one outer tap fails a geometric test on the 13 M-triangle scan.
  - The screen's `cr=0` capture (bilinear everywhere, no switching) separates them.

## 3. Cause and candidate

**C:** `temporal_resolve.wgsl` `tr1_history`. A moving pixel with all 16 taps accepted resamples by Catmull-Rom, clamped to its 4 inner taps' range, and takes `window = min(window, sharp_window)` (16). The bilinear path instead caps the window at `1 + blur_budget / sum f(1-f)` (T1b). Catmull-Rom cancels the second moment of the bilinear blur but still attenuates texture frequencies near Nyquist by an amount that depends on the fraction f. Over a 16-frame window that frame-varying softening accumulates, so the output's sharpness moves every frame.

**Model (`tools/p-flicker/tr1_model.py`, CPU).** A band-limited textured line moves by a fraction of a pixel each frame; TR1's moving interior is replayed exactly (Catmull-Rom, inner clamp, one jittered sample per frame). It is scored with the same step residual against the box-filtered truth. Speed is in pixels per frame. The quarry walk's 0.05° yaw is about 1.6 px/frame at 1080p.

| Variant | flicker at 0.11 / 0.37 / 1.3 / 1.6 px/frame | static error, same speeds |
|---|---|---|
| main (Catmull-Rom window 16) | 0.637 / 1.959 / 3.431 / 4.113 | 2.116 / 2.483 / 2.463 / 2.920 |
| window 8 | - / 1.754 / 2.644 / - | - / 2.063 / 1.675 / - |
| window 32 | - / 2.148 / 4.193 / - | - / 3.004 / 3.827 / - |
| no jitter | 0.612 / 1.879 / 3.160 / 3.904 | 2.029 / 2.294 / 2.325 / 2.812 |
| **Catmull-Rom blur budget 1** | 0.636 / **1.656 / 2.358 / 2.720** | 1.772 / 1.893 / 1.479 / 1.719 |
| Catmull-Rom blur budget 2 | 0.637 / 1.810 / 2.949 / 3.443 | 2.115 / 2.183 / 1.934 / 2.226 |
| Keys a = -0.75 kernel, window 16 | - / 1.888 / 3.137 / - | - / 2.468 / 2.277 / - |
| Lanczos-2 kernel, window 16 | - / 1.955 / 3.440 / - | - / 2.524 / 2.472 / - |

With sampling noise added to each frame's sample (Gaussian, codes), the trade appears:

| per-sample noise σ | flicker at 0.37 px/frame: main / budget 1 / budget 2 | at 1.3 px/frame: main / 1 / 2 |
|---:|---|---|
| 0 | 1.96 / **1.66** / 1.81 | 3.43 / **2.36** / 2.95 |
| 2 | 1.97 / **1.72** / 1.83 | 3.45 / **2.45** / 2.98 |
| 5 | 2.00 / 2.00 / **1.92** | 3.54 / **2.92** / 3.15 |
| 10 | **2.12** / 2.79 / 2.20 | 3.82 / 4.23 / **3.79** |
| 20 | **2.54** / 4.76 / 3.12 | **4.76** / 7.48 / 5.45 |

A budget of 1 wins only where the fresh sample is clean, such as sunlit texture. Where it is noisy (penumbrae, GI cell borders), the shorter window lets that noise through. A budget of 2 keeps most of the texture gain and is roughly neutral at σ = 10. In 2-D, `sum f(1-f)` reaches 0.5 on diagonal motion, so a given budget cuts the window up to twice as hard as in this 1-D model. The screen therefore runs budgets 1, 2 and 4, and checks penumbrae and the S4 walls by eye. With the patent policy suspended (Islam, 2026-11-10 22:48 UTC), the obvious next step is allowed: size the budget per pixel from a noise estimate, the local variance of this frame's 3x3 samples of the surface. Clean texture gets the short fraction-capped window and noisy pixels keep the long one. That is the patented element of US 10,116,916 / US 10,362,289 (P-B). A sun-visibility-aware cap is still not taken: it would bake a lighting-model assumption into TR1, against programmable shading.

Longer windows are worse and shorter ones better. A sharper kernel helps a little; capping the window by the fraction helps most. The model has no lighting noise, so a shorter window is free there. On the quarry it may expose sampling noise (penumbrae, GI cell edges), which only the GPU screen can show.

**The change (code on this branch, default off).**
- `Reconstruction::sharp_blur_budget`, in `relit_params.y` (an unused word).
- In the Catmull-Rom branch: `if budget > 0 { window = min(window, 1 + budget / sum f(1-f)) }`.
- At 0 the uniform words are the same as main's, so the default image is unchanged.
- The adapter knob `RENDERLAB_P_FLICKER_TR1=sharpblur=K` sets it for screens.

**Patents (policy suspended 2026-11-10; numbers noted only).** The fraction cap uses fetch geometry alone and touches no flagged claim. The variance-sized variant above would use US 10,116,916 / US 10,362,289. Per-cell accumulation of jittered GI samples (US 10,614,613, M8-REJECT-JITTER-ACCUM) is now allowed, but this split shows GI is not the quarry flicker's source (GI off: 4.39 → 4.36). It does not compete with the TR1 fix here. It stays a lead for GI grain (S4 interior) and for letting still views idle, which P-STILL's cell-age-follows-unjittered-visibility option also targets.

## 4. GPU screen

GPU work was held from 21:28 to 23:09 UTC while Islam played (`cs2`). The screen then ran as leases `pflk-l3` (183 s), `pflk-l4` (timing and lib) and `pflk-ue2` (Unreal).

**Quarry yardstick** (P-YARD2's scorer and references, 32 moving pairs; the control from the new binary at budget 0 is byte-identical to main on all 125 frames):

| Run | Flicker L0 | Flicker L4 | Error still | Error moving | Error cut | Error recover | Edge moving |
|---|---:|---:|---:|---:|---:|---:|---:|
| RT main | 4.392 | 5.053 | 6.630 | 7.237 | 7.711 | 6.939 | 11.56 |
| RT budget 1 | 4.314 | 4.993 | 6.630 | 7.070 | 7.711 | 6.939 | 11.54 |
| **RT budget 2 (default)** | **4.323** | 4.985 | 6.630 | 7.123 | 7.711 | 6.939 | 11.54 |
| RT budget 4 | 4.379 | 5.039 | 6.630 | 7.215 | 7.711 | 6.939 | 11.56 |
| RT `cr=0` (bilinear only) | 4.424 | 5.076 | 6.630 | 7.197 | 7.711 | 6.939 | 11.54 |
| non-RT main | 4.438 | 5.111 | 7.278 | 7.722 | 8.257 | 7.524 | 11.95 |
| non-RT budget 1 | 4.377 | 5.077 | 7.278 | 7.609 | 8.257 | 7.524 | 11.94 |
| **non-RT budget 2 (default)** | **4.375** | 5.052 | 7.278 | 7.633 | 8.257 | 7.524 | 11.93 |
| UE HW Lumen Epic (recaptured) | 3.680 | | | 8.94 (P-YARD2) | | | |
| UE SW Lumen Epic (recaptured) | 3.686 | | | 9.90 (P-YARD2) | | | |

- **Budgets 1 and 2** are within noise of each other on the quarry. 2 is the default because the noise model shows budget 1 letting sampling noise through in penumbrae.
- **Contrast split.** The Unreal recapture reproduces P-YARD2's flicker to three decimals. Its split has the same shape as Renderlab's (96% interior, energy rising with contrast) at about 15% lower RMS in every bin: 7.74 against 9.42 at contrast 20 and above, 1.97 against 2.22 on flat areas.
- **Timing, interleaved visits in one lease** (A = budget 0, B = default):

| GPU p50 ms (vs Epic) | A0 | B0 | A1 | B1 |
|---|---:|---:|---:|---:|
| RT still | 3.48 | 3.38 | 3.37 | 3.38 (1.33x faster than HW Epic 4.50) |
| RT moving | 4.30 | 4.30 | 4.30 | 4.29 (1.07x faster than 4.57) |
| RT cut | 12.71 | 12.64 | 12.66 | 12.71 (2.8x slower than 4.52) |
| non-RT still | 8.55 | 8.64 | 8.68 | 8.58 (2.1x slower than SW Epic 4.02) |
| non-RT moving | 10.66 | 10.65 | 10.69 | 10.66 (2.6x slower than 4.12) |
| non-RT cut | 43.10 | 42.19 | 43.99 | 44.18 (10.7x slower than 4.11) |

  An earlier single budget-2 visit read RT moving 4.74 ms, with CPU submit also up 30%. That was contention, and the interleaved visits replace it.
- **Ghosting shots (RL#10 shots our benches can show), inspected by eye.**
  - The S4 sliding door (frames 64, 76, 88, 110, both paths) differs only along the moving door edge: 70-280 pixels over 8 codes, with the same look and no trail or smear (`docs/p-flicker/s4-door-edge.jpg`, `s4-door-rt.jpg`).
  - The pre-existing stray bright pixels on that edge are present with and without the change.
  - The flat-wall grain (GI, P-YARD2 item 1) is untouched.
  - The S5 mirror with the moving cube is byte-identical: no receiver moves.
- **Clips at playback speed** (30 fps, walk frames 160-223, Renderlab main | budget 2 | Unreal HW Epic):
  - `D:/Coding/renderlab-work/p-flicker/sheets/rock.gif` and `gravel.gif`;
  - step sheets `rock-steps.png` / `gravel-steps.png`, reduced copies in `docs/p-flicker/`.
  - Renderlab is visibly sharper and more detailed than Unreal in both clips.
  - Its steps show dotted bright flashes along cracks that Unreal's do not. That is the shimmer a viewer sees.

**Next (for the next flicker unit):**
- The crack flashes are one-frame, sign-balanced and contact-enriched.
- They are neither Catmull-Rom/bilinear switching (`cr=0` keeps them) nor GI.
- Candidates:
  - TR1's geometric tap test intermittently rejecting history on the scan's tiny crack triangles (a restart shows the raw jittered sample);
  - sun-visibility or normal-map glints in the fresh sample.
- One diagnostic capture with TR1's per-pixel restart/accept mask would separate them.
- If restarts dominate, a geometry-tolerant tap test on near-coplanar crack faces is the fix. With the patent policy suspended, a per-pixel variance clamp of the fresh sample is also open.

## 5. Harness

- `examples/p_yard_capture.rs`: measurement knobs, unset changes nothing:
  - `RENDERLAB_P_FLICKER_SUN_ACTIVE` (TR1's moving sun rays);
  - `RENDERLAB_P_FLICKER_TR1` (`sharp`, `window`, `blur`, `sharpblur`, `cr`);
  - `RENDERLAB_P_FLICKER_GI_EVICT`.
- `tools/p-flicker/`:
  - `host.py`: P-STILL's host, with `--quarry-lease`, no validation layer, 14 GiB lease admission, 10 GiB build;
  - `run.py`: steps with ablations, layer, pass log, counts and scene options;
  - `split.py`: the flicker split above;
  - `timing.py`: per-state span and CPU submit with Epic ratios and per-pass medians;
  - `tr1_model.py`;
  - `ue.py`: P-YARD's Unreal driver writing under `p-flicker/ue`.

## 6. Hardware, build, process

- RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11 Pro 26300, 1080p native.
- Release `p_yard_capture` from main `f64c82e` plus the adapter knobs; renderer source clean for the base runs.
- Builds: 151 s for the first build, with one Cargo command, the build slot and the stillkit mutex, after admission.
- Leases through `scripts/n8-run.py --quarry-lease` (12 GiB child cap, 8 GiB machine floor): `pflk-l1` (235 s) and `pflk-l2` (149 s). Free RAM was 17-20 GiB at both starts.
- Further leases:
  - `pflk-l3` (183 s, screen);
  - `pflk-l4` (interleaved timing, default-build identity, then lib tests: 207 passed, 167 ignored);
  - `pflk-ue2` (Unreal HW/SW Epic captures, 33 + 26 s, camera readback OK).
- Candidate builds: 40 s each, plus the lib test binary (3 min 33 s), one at a time after admission.
- Unreal: UE 5.7.4 CL51494982, U0's Lumen package, D3D12 SM6. P-YARD2's timings are reused.
- Time: about 1 h 40 min of code and analysis and about 12 min of GPU leases, with GPU work held for 1 h 41 min while Islam played.
- Cleanup: the Cargo target and the raw HDR captures (16.6 MB per frame) were deleted. Unreal's raw DumpGPU folders were deleted (44 GB of raw files in all). Its decoded natives are kept, about 1.1 GB of npz files under 5 MB each in `p-flicker/ue`, so the next flicker unit can re-score Unreal without a recapture. `D:/Coding/renderlab-work/p-flicker` keeps the logs, `engine.json` timings and counts, `split/split.json`, `score/quality.json`, the timing JSONs, the energy maps and the inspected sheets and clips.
