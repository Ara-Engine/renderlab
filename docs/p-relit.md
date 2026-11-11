# P-RELIT: a flickering light no longer restarts the frame

Milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-11, branch `t3/p-relit` from main `cf3a6b9` (main `506075d` merged in). Decision row M8-FIRE-RESTART; follow-up of [p-flat-tr1.md](p-flat-tr1.md) and the unmerged P-FLAT2. Labels: **M** measured here, **C** read from code, **I** inference.

## Result

| Part | Verdict | What it does |
|---|---|---|
| (c) Sun budget | **MERGE** (with the rest; alone it is superseded by b) | A local light's intensity change never selects the frame-wide restart shadow budget. |
| (a) Relit by default | **MERGE** | A changed local light restarts only its own restart sphere. Everything else continues its history. Light changes never restart the frame. |
| (b) Per-pixel restart budget | **MERGE** | Only the receivers TR1 will restart trace the 64-ray restart count; the rest of the frame keeps the moving or held count. Both paths. |

Game scene, 1080p, moving camera, fire flickering, RTX 4070 SUPER, GPU span p50 (M):

| | main | this branch | steady fire (floor) |
|---|---:|---:|---:|
| Non-RT frame, two-sphere hero (root's capture) | 92.6 ms | **70.3 ms** | 68.3-71.1 |
| Non-RT sun pass | 54.2 | **33.6** | 32.1-32.6 |
| RT frame | 30.2 | **27.8** | 26.9-27.1 |
| RT visibility | 7.27 | **4.28** | 3.74-3.77 |
| Non-RT frame, walking Fox | 100.0 | **79.8** | 78.7 (main: 101.4) |
| RT frame, walking Fox | 33.5 | **32.6** | 31.9 (main: 34.0) |

- **Picture (M, inspected).** Away from the brazier the courtyard accumulates again: on a held camera the frame-to-frame step outside the fire's sphere falls to the steady-light floor (RT 0.370 → 0.299 codes, floor 0.291; non-RT 0.435 → 0.350, floor 0.343). In flight every flat-surface speckle score is at or below main's.
- **No lag near the brazier (M).** Flicker slope on the fire-lit wall and ground: RT 0.984 / 0.994 against main's 0.985 / 0.994; non-RT 0.986 / 0.994 against 0.986 / 0.994. Inside the sphere the image equals main's except at silhouettes, which now keep their anti-aliasing.
- **The Fox was the second cause.** With the Fox walking, main traces 64 rays frame-wide whatever the fire does (P-FLAT-TR1's restart budget for its swept shadow): 101 ms non-RT even with a steady fire. Part (b) fixes that too: 78.7 ms.
- **Quarry yardstick: unchanged** (no local lights or skinned casters there). RT still 3.15 ms (1.43x faster than HW Lumen Epic), moving 4.02 (1.14x faster), cut 13.2 (2.9x slower); non-RT still 8.4 (2.1x slower), moving 10.6 (2.6x slower), cut 43-45 (10.5x slower). The courtyard has no Unreal build, so it has no Epic ratio.
- **Known limit, by design.** Outside the sphere the fire's remaining flicker is under the budget (well under one display code here) and a long history smooths it: on a held camera the arch shows 82 % of its 0.27-code flicker step (section 3.3). In flight it shows 99.7 %.

## 1. Cause (C, M)

1. `Reconstruction::light_budget` defaulted to 0: any local light's intensity change restarted every pixel's TR1 history. The courtyard fire changes every frame, so nothing accumulated.
2. Every restarted frame took `ReconstructionShadowBudget::reset_rays` (64) instead of `active_rays` (32) for every pixel: one frame-wide word.
3. Independently, a frame that dropped history anywhere (a skinned caster's swept shadow) also took 64 rays frame-wide.

## 2. The change

### 2.1 Restart spheres (a)

- **Swing.** The planner keeps each local light's candela range per channel since every pixel last restarted (`widen_swings`). Every intensity a continued history can hold lies inside it. The swing is the largest channel distance from the current candela to either end.
- **Sphere.** A changed light restarts the points where its swing could change the exposed illuminance by `light_budget` or more: `swing x exposure x attenuation(d) >= light_budget`, with the shading's own attenuation (inverse square and range window, `program_api.wgsl`). Attenuation falls with distance, so that set is a sphere; the CPU solves its radius (`restart_radius`) and uploads centre and squared radius. The shader test is one distance comparison per sphere.
- **Why it has no lag inside.** A pixel in the sphere restarts on every frame the light changes, exactly as main's full restart did. A pixel outside holds stale light below the budget whatever its window. This is P-FLAT's named follow-up (size by the change since the history's oldest sample), as a restart instead of a cap: P-FLAT's cap saturated at the budget and lagged.
- **Budget, exposure-referred.** `light_budget` is exposed illuminance (lux x exposure), default 0.05: about 0.8 % of display white on a mid-grey diffuse surface. P-FLAT2's budget was relative to the light's own peak, which restarts 6 m round a candle at noon and too little round a fire in a dark cave. At the courtyard's exposure (2^-11) the two agree: the sphere is 5.4-6.7 m as the swing moves between 0.15 and 0.30 of the fire's peak.
- **Continued pixels** stay under MQ1's 3x3 box on relit frames, as P-FLAT. That box is what removes a switched-off light's remainder beyond the sphere.
- **Never a frame restart.** Up to eight spheres are uploaded; with more changed lights the smallest share one bounding sphere. A zero budget restarts each changed light's whole range.
- **Silhouettes.** Coverage history continues inside a sphere (it is geometry). A still silhouette's kept third-surface state is not carried there: it held both colours at the light's old intensity (P-FLAT-TR1's known limit 2.5).
- **Patent note.** The decision uses engine light data, the exposure and the receiver's position only. Policy is suspended; no flagged method is used.

### 2.2 Per-pixel restart budget (b, c)

- `Tr1Zones` (192 bytes) holds what the resolve restarts from engine data alone: the moving caster's swept shadow volume (SK1/P-FLAT) and the restart spheres, plus the restart ray count.
- The same record is bound to the sun-shadow passes, and `shaders/temporal_zones.wgsl` is compiled into each with its own record name, so the three ask one question:
  - **RT:** TR1's trace kernels take each receiver's count from `tr1_zone_rays` and normalise by it.
  - **Non-RT:** the Exact and Surface lookups walk a prepared pixel's own batches (`vsm_batches`).
  - **Resolve:** `tr1_restarts`.
- A frame that restarts every pixel (cut, key change, sun turn, first frame) still takes the restart or cold count frame-wide.
- A still camera with a flickering light now reaches the held count (8 rays) for continued pixels; main never left 64 there.
- Not covered: pixels restarted by disocclusion keep the frame's count, as on main. P-RTS's penumbra listing (off) keeps the frame-wide words. S4's occluder gather sits out frames whose zones would trace more than the frame's batches (non-default budgets only).

### 2.3 Records and resources (for the ara-gpu port)

- **No new buffer or texture kind, binding or device feature.**
- **`PrimaryFrame` grows 144 → 336 bytes**: `zones:Tr1Zones` appended. It is the shared prelude uniform at binding 10; only TR1's trace kernels read the new member.
- **`VsmFrame` grows by 192 bytes**: `zones:Tr1Zones` appended after `nsq`.
- **`Tr1Frame` grows 448 → 512 bytes**: the relit block becomes `zones:Tr1Zones` (same first 48 bytes) and `resample` (`sharp_blur_budget` moved from `relit_params.y` to `resample.x`). Option bit 5 (`TR1_RELIT`) is gone.
- **`temporal_reconstruction::Planner`**: `PlanInputs` unchanged. `Planned` gains `zones()`; `Planner` gains `restart_rays`. `Planned::frame` has the new layout. The backend must write `planned.zones()` to `PrimaryFrame` at byte 144 and to `VsmFrame` at `ZONES_OFFSET`.
- **Shader composition**: `zones_source(record)` must be appended to the resolve (`tr1.zones`), TR1's trace module (`primary_frame.zones`) and the virtual-shadow module (`vsm.zones`). `resolve_source`, `trace_source` and `shader_source` already do.
- `shadow_gpu::plan_frame`: untouched.

## 3. Measurements

Hardware and build: RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11 Pro 26300. Release `game_scene` and `p_yard_capture`, `-j 2`. `main` is `cf3a6b9` plus this branch's example switches (default output unchanged); `c` is `19dee43`, `a` is `f700d69`, the candidate is `29c10b4`. The branch tip adds comments, one equivalent lint rewrite (`restart_radius`'s NaN test) and these docs; the lib suite and the three GPU tests were rerun on it (lease `g2`). No validation layer. Machine shared with the wallpaper and another unit's builds: timings are p50 over frames 30-149 of one run each, not quiet-window claims.

### 3.1 Flicker slope and speckle in flight (`tools/p-relit/score.py flights`)

`game_scene --fox`, 1280x720, 48 frames, every frame written; slope against main's no-history control over frames 8-47 (P-FLAT's `flicker_split2.py` masks and regression); speckle from `scripts/p-flat-score.py`, frames 8, 16, 24, 32, 40, 47.

| Run | wall slope | near-ground slope | arch slope | crate top | crate side | ground | arch | wall |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| RT main (= c, 48/48 frames identical) | 0.985 | 0.994 | 1.001 | 0.079 | 0.044 | 0.012 | 0.015 | 0.026 |
| RT a | 0.984 | 0.994 | 0.996 | 0.059 | 0.043 | 0.009 | 0.011 | 0.026 |
| **RT a+b+c** | **0.984** | **0.994** | 0.997 | **0.073** | 0.043 | 0.009 | 0.013 | 0.026 |
| non-RT main (= c) | 0.986 | 0.994 | 1.001 | 0.074 | 0.019 | 0.016 | 0.015 | 0.012 |
| non-RT a | 0.986 | 0.994 | 0.993 | 0.055 | 0.019 | 0.011 | 0.012 | 0.012 |
| **non-RT a+b+c** | **0.986** | **0.994** | 0.993 | **0.067** | 0.019 | 0.013 | 0.013 | 0.012 |

- (c) alone changes nothing in this flight: the Fox's swept shadow already selected the 64-ray budget. It acts with the two-sphere hero (section 3.4).
- (b) gives back part of (a)'s crate gain (0.059 → 0.073 RT): continued pixels now trace 32 rays, the ordinary moving count, where (a) alone still traced 64 frame-wide. Still below main on every surface, at 20 ms less.
- Ground between the sphere and the fire's 10 m range: slope 0.957 RT / 0.949 non-RT against main's 0.976. That is the sub-budget remainder.

### 3.2 Held camera, scene animating (`score.py hold`, `--diag hold=1`)

Frame-to-frame step of static surfaces in display codes, RMS over pixels and frames 16-47. "Outside" is the far ground, arch and crates (412,618 px); "inside" the wall and ground in the sphere (61,270 px).

| Run | RT outside | RT inside | RT arch | non-RT outside | non-RT inside | non-RT arch |
|---|---:|---:|---:|---:|---:|---:|
| no history (control) | 0.329 | 1.614 | 0.265 | 0.547 | 1.628 | 0.259 |
| main | 0.370 | 1.623 | 0.296 | 0.435 | 1.610 | 0.291 |
| **this branch** | **0.299** | 1.598 | 0.243 | **0.350** | 1.586 | 0.240 |
| steady fire (main / branch) | 0.291 / 0.292 | 1.420 / 1.421 | 0.100 / 0.101 | 0.343 / 0.345 | 1.419 / 1.420 | 0.096 / 0.099 |

Inside the sphere the step is the fire's real flicker, equal to main's (wall slope against the control 1.014 on both builds).

### 3.3 Budget screen (lease `s1`)

| `light_budget` | held arch slope (RT / non-RT) | held outside step | flight arch slope | flight wall slope | RT crate top |
|---|---|---|---|---|---:|
| 0.02 | 0.947 / 0.944 | 0.310 / 0.360 | 1.003 / 0.996 | 0.984 / 0.986 | 0.073 |
| **0.05 (default)** | 0.817 / 0.813 | 0.299 / 0.350 | 0.997 / 0.993 | 0.984 / 0.986 | 0.073 |
| 0.1 | 0.717 / 0.710 | 0.295 / 0.347 | 0.996 / 0.999 | 0.982 / 0.984 | 0.073 |

The arch is 4.8 m and more from the fire; part of it lies outside the sphere. Its flicker step is 0.27 codes RMS, so the smoothing is not visible at any of these budgets; 0.05 keeps more of the frame accumulating than 0.02 and keeps the wall exact where 0.1 starts to lose it. One constant (`LIGHT_BUDGET`) if a later scene argues otherwise.

### 3.4 Frame time per pass (`tools/p-relit/yard.py passes`, lease `t1`)

1920x1080, 150 flight frames, no images, `RENDERLAB_PASS_LOG`; p50 over frames 30-149, ms. "steady" is `--diag flicker=0`.

| Two-sphere hero | span | sun pass / visibility | tr1 | steady span |
|---|---:|---:|---:|---:|
| non-RT main | 92.56 | 54.21 | 0.46 | 71.12 |
| non-RT c | 68.28 | 32.21 | 0.46 | 67.77 |
| non-RT a | 68.14 | 32.11 | 1.04 | 68.28 |
| **non-RT a+b+c** | **70.33** | 33.57 | 1.02 | 68.29 |
| RT main | 30.23 | 7.27 | 0.46 | 26.94 |
| RT c | 27.01 | 3.74 | 0.46 | 27.18 |
| RT a | 27.40 | 3.75 | 1.05 | 27.03 |
| **RT a+b+c** | **27.80** | 4.28 | 1.02 | 27.06 |

| Walking Fox | span | sun pass / visibility | steady span |
|---|---:|---:|---:|
| non-RT main | 99.98 | 53.48 | 101.38 |
| non-RT c / a | 97.09 / 97.82 | 52.99 / 53.19 | 97.07 / 97.61 |
| **non-RT a+b+c** | **79.84** | 36.55 | 78.66 |
| RT main | 33.51 | 7.36 | 33.96 |
| RT c / a | 34.07 / 34.65 | 7.38 / 7.35 | 34.12 / 34.40 |
| **RT a+b+c** | **32.55** | 5.59 | 31.91 |

- (b) costs 1.4-2.2 ms over (c)+(a) with the two-sphere hero: the sphere's pixels trace 64 rays again, the count main gave them. That buys main's per-frame penumbra quality where pixels restart.
- The other passes are unchanged (shading 9.2 / 16.3, GI 11-23 ms).

### 3.5 Quarry yardstick (`yard.py run` / `timing`, lease `q-y1`, interleaved visits)

| GPU p50 ms | main visits | branch visits | vs Unreal Epic |
|---|---|---|---|
| RT still | 3.17, 3.12 | 3.23, 3.15 | 1.40-1.44x faster (HW 4.50) |
| RT moving | 4.06, 4.02 | 4.03, 4.02 | 1.13-1.14x faster (4.57) |
| RT cut | 12.90, 12.78 | 14.56, 13.24 | 2.8-3.2x slower (4.52); single frames |
| non-RT still | 9.04, 8.42 | 8.39, 8.46 | 2.1x slower (SW 4.02) |
| non-RT moving | 11.05, 10.66 | 10.53, 10.65 | 2.6x slower (4.12) |
| non-RT cut | 45.63, 43.39 | 45.13, 43.24 | 10.5x slower (4.11); single frames |

Per-pass medians in motion are the same on both builds (RT visibility 1.46 / 1.46, non-RT sun pass 6.32 / 6.26).

### 3.6 Identity and tests

| Check | Result |
|---|---|
| Steady fire, two-sphere hero, 1080p, frames 0 and 149, main vs branch | byte-identical, both paths (no zone is up, so nothing may change) |
| (c) vs main, Fox flight | 48/48 frames identical, both paths |
| Moving fire light (`fire=move`), main vs branch | 40/40 frames identical, both paths (a position change is still a full restart) |
| Lib suite (lease `g1`) | 211 passed, 0 failed, 167 ignored |
| `gpu_mq1_moving_placement`, `gpu_mv1_mq1_static_identity`, `gpu_skinned_streams_history_and_paths` | pass |
| New unit tests | swing and sphere sizing, exposure, zero budget, the 1 cm clamp, bounding sphere for more than eight lights, per-pixel counts, the trace seam, `Tr1Zones` declared alike in both shader files |
| Clippy (`--lib --tests --example game_scene`) | no warning in the changed code (one fixed; the tree's existing warnings are untouched) |
| **Not run** | RF1-RF10 and the quarry Q-walk byte identity, `gpu_object_motion`, the quarry capture hashes (the 14 GiB quarry admission was not met again) |

## 4. The cases that share this restart (`score.py case`, events at frame 24)

Each run is compared with main's no-history control of the same scene; "ghost pixels" are pixels more than 8 codes brighter than the control on the ground between the sphere and the fire's range and on the far ground and arch.

| Case | Mechanism (C) | Result (M), both paths |
|---|---|---|
| Light switched off (candela → 0) | swing 1.0: the sphere is nearly the whole range (8.2 of 10 m); beyond it MQ1's box | No ghost: 0.0000 ghost pixels in frames 24-27 on the fringe and far masks; lit-mask mean within 0.01 code of main's on every frame. Sheet below. |
| Light switched on | same | Lit mask equals main's to 0.01 code from frame 24; fringe at its ordinary accumulated bias (-0.06 code). |
| Moving local light | position is in the history key: full restart, as main | 40/40 frames identical to main. Still restarts every pixel every frame: see the gap entry. |
| Emissive strength change | a material parameter write bumps the key: full restart, as main | Frame 24 restarts; no difference from main beyond accumulation. |
| Camera cut (`Engine::cut`) | cold restart; the swing starts again | Frame 24 equals the control as on main (frame RMS 0.97 on both); the next frames differ from main by under 0.1 code mean. |

## 5. Sheets (inspected)

- `docs/p-relit/courtyard-f040.jpg`: flight frame 40, main and branch, RT and non-RT. No difference a viewer would find in a still; the gain is temporal.
- `docs/p-relit/brazier-moving-rt.jpg`, `-nonrt.jpg`: the brazier at flight frames 30-32 (a flicker step each): control, main, branch, and |main - branch| x 8. The wall and ground are black in the difference. The lines are silhouettes, where the branch keeps coverage history (smoother edges), and fire glints in the gold sphere.
- `docs/p-relit/brazier-hold-rt.jpg`: the same on a held camera.
- `docs/p-relit/crates-moving-rt.jpg`, `-nonrt.jpg`: the crates at frames 40 and 47, 3x.
- `docs/p-relit/fire-off-rt.jpg`, `-nonrt.jpg`: frames 23, 24, 25, 27 round the switch-off: control, main, branch. The wall's glow is gone at frame 24 in all three.

Looked for and not found on the branch: stray bright pixels that main lacks, a ring at the sphere's boundary, trails behind the Fox.

## 6. Process

- Leases, all headless, prebuilt binaries, under `scripts/n8-run.py`'s watchdog: `f1`, `f2` (flights, 72 + 65 s), `t1` (timing, 317 s), `h1`, `k1` (held camera, cases, 65 + 168 s), `g1` (tests, 58 s), `q-y1` (quarry, 143 s after a 40 min wait for 14 GiB), `s1` (budget screen, 60 s). No game was running at any lease.
- Builds: seven release example builds (37-177 s each) and two lib-test builds, one at a time through `tools/p-relit/host.py` in `D:/Coding/renderlab-work/p-relit/target` (deleted).
- Evidence in `D:/Coding/renderlab-work/p-relit/`: `scores/*.json`, `runs/*/run.json` and logs, `plans/`, `crops/`, `bin/*/build.json`, `lease/`. Frame images other than the sheets' were deleted.

```powershell
py -3 -B tools/p-relit/host.py build NAME [TREE]
py -3 -B tools/p-relit/plan.py flights|hold|cases|timing|tests NAME...
py -3 -B tools/p-relit/host.py lease LABEL 15 -- py -3 -B tools/p-relit/batch.py PLAN.json D:/Coding/renderlab-work/p-relit/runs 13.5
py -3 -B tools/p-relit/score.py flights|hold|case OUT.json CONTROL RUN...
py -3 -B tools/p-relit/yard.py passes OUT.json RUN_DIR...
```
