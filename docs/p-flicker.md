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
- **Candidate (code ready, GPU screen pending).** `Reconstruction::sharp_blur_budget` caps the Catmull-Rom window at `1 + budget / sum f(1-f)`, the bilinear rule with its own budget. Zero, the current default, keeps the fixed window, so frames with it at zero are unchanged.
  - In the 1-D model, a budget of 1 lowers flicker by 15-34% at walk-to-pan speeds and lowers static error too. Slow motion is unchanged.
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

A budget of 1 wins only where the fresh sample is clean, such as sunlit texture. Where it is noisy (penumbrae, GI cell borders), the shorter window lets that noise through. A budget of 2 keeps most of the texture gain and is roughly neutral at σ = 10. In 2-D, `sum f(1-f)` reaches 0.5 on diagonal motion, so a given budget cuts the window up to twice as hard as in this 1-D model. The screen therefore runs budgets 1, 2 and 4, and checks penumbrae and the S4 walls by eye. Sizing the budget from a per-pixel noise estimate is not an option: a colour-variance confidence is P-B/P-D's element. A sun-visibility-aware cap would bake a lighting-model assumption into TR1, against programmable shading.

Longer windows are worse and shorter ones better. A sharper kernel helps a little; capping the window by the fraction helps most. The model has no lighting noise, so a shorter window is free there. On the quarry it may expose sampling noise (penumbrae, GI cell edges), which only the GPU screen can show.

**The change (code on this branch, default off).**
- `Reconstruction::sharp_blur_budget`, in `relit_params.y` (an unused word).
- In the Catmull-Rom branch: `if budget > 0 { window = min(window, 1 + budget / sum f(1-f)) }`.
- At 0 the uniform words are the same as main's, so the default image is unchanged.
- The adapter knob `RENDERLAB_P_FLICKER_TR1=sharpblur=K` sets it for screens.

**Patent notes.** The cap is sized from the fetch geometry alone (f), as T1b's bilinear budget is. No colour statistic, variance, history moment or lighting gradient enters, so P-B/P-D are not engaged. TR1's state stays private (W5), and no accumulation of jittered samples per GI cell is involved (M8-REJECT-JITTER-ACCUM untouched).

## 4. GPU screen

**Pending.** Planned lease `pflk-l3` (one binary, `sharp_blur_budget` 0 / 1 / 2 / 4):
- the quarry yardstick on both paths: flicker, full-frame error still/moving/cut, timing;
- the S4 door and S5 mirror benches at K = 0 and 1 on both paths, for ghosting and grain, judged by eye;
- Unreal HW/SW Epic quarry recaptures (`tools/p-flicker/ue.py`) for side-by-side clips and the per-contrast split against Unreal. P-YARD2's Unreal captures were deleted after scoring, so P-YARD2's scores and timings are reused, but its pixels are gone.

Blocked from 21:28 UTC: `cs2` (Islam's game, 10 GB private) was running, and AGENTS.md defers GPU work and measurement builds while he plays. Free RAM was 8.8-9.3 GiB, under the brief's 10 GiB start bar.

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
- Unreal side: P-YARD2's evidence (UE 5.7.4, D3D12 SM6).
