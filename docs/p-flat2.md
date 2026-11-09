# P-FLAT2: TR1 history through a flickering light, with no flicker lag

Milestone 8 ([milestone-8.md](milestone-8.md)), the follow-up to [p-flat-tr1.md](p-flat-tr1.md). Branch `t3/p-flat2` from main `ee361f5`. Owns TR1 history: `src/temporal_reconstruction.rs` (keys, `prepare_reconstruction`) and `shaders/temporal_resolve.wgsl`. Labels: **M** measured, **C** read from code, **I** inference.

## Status

| Item | Result |
|---|---|
| Outcome | **Not the default; `light_budget` stays 0 (main's restart).** The swing restart improves every crate crop on both paths, and its wall and ground slopes are within noise of main's. But two things are missing. RT's wall and ground and non-RT's ground read 0.001 under main, so "at or above" is not shown. And the acceptance runs never ran. |
| Why the runs never ran | The 3 h box ran out on waits. Two leases were stopped by the 2 GiB runtime guard while another project (stillkit) compiled. Then, from 17:00, Islam's one-build rule required no foreign cargo/rustc before any Renderlab build or lease, and stillkit's nightly build and test ran continuously (12-18 compilers) until the box ended. |
| Fresh main (`ee361f5`, real fire and Fox, frames 8-47) | RT crate tops **0.079**, sides 0.044. Non-RT crate tops **0.075**, sides 0.024: P-NSQ's fix took non-RT from P-FLAT's 0.278. |
| Candidate (swing restart, budget 0.004, measured build) | RT crate tops **0.057**, sides 0.044, ground 0.008, arch 0.011, wall 0.026. Non-RT crate tops **0.054**, sides 0.024, ground 0.012, arch 0.013, wall 0.015. Every crop is equal to or better than main's. |
| Flicker slope (gate: wall, ground) | RT: wall 0.984 vs main 0.985, ground 0.995 vs 0.996. Non-RT: wall 0.982 vs 0.982, ground 0.994 vs 0.995. Paired RT difference: wall -0.0004 ± 0.0010, ground -0.0004 ± 0.0014 (standard errors), so not significant. |
| What the remaining difference was (M) | Inside the restart zone, only silhouette pixels differed from main. Coverage history (geometry) continued where main restarted it. The branch now restarts coverage and still-camera silhouettes in the zone as well, so the zone matches main's restart exactly (section 2). **That build was never compiled or run.** |
| Arch (recorded, not gated) | Slope 0.990 RT and non-RT, against main's 1.001 and 1.002. The arch is 4.7 m or more from the fire, mostly outside the 0.004 zone, and keeps the fire's flicker in its history (bound: under 0.004 of the fire's 1 m peak). A smaller budget widens the zone. The 0.002 and 0.001 runs were planned but never ran. |
| Static identity, SK1, MQ1, MV1, object motion, lib | **Not run** on any candidate build. Owed: plan `D:/Coding/renderlab-work/p-flat2/plans/w2.json`. The naga reconstruction-module validation passed on the measured build. |
| Patent reasoning | Light data only (position, range, candela history) and the receiver's position decide restarts; no pixel colour, moment or lighting difference (section 2). |

## 1. Why B' lagged (C, from p-flat-tr1.md)

B' capped a continued history near a changed light at `1 + budget / (change * falloff)` and restarted where the cap fell below 2. `change` was the light's relative change since the previous frame. That bounds the error the last frame adds, but a continued history also holds every earlier intensity since it began. On frames where the fire's two sines sit near a peak, `change` is small, the cap rises above 2, and the wall and ground keep history from intensities several frames old. That history is the lag.

The same holds for any cap of that form. Its error bound saturates at the budget in a steady flicker: with error `e` before the frame, the next is at most `(1 - 1/n)(e + change * falloff)`, whose fixed point at `n = 1 + budget / (change * falloff)` is exactly `budget`. Wherever the light's own swing at a receiver is much larger than the budget, the history carries that fixed error behind the flicker.

## 2. The change: restart by the light's swing since the histories began

P-FLAT's first direction, made exact. Instead of capping the window, decide restart or continue from everything a history can hold.

- **The swing (CPU, `widen_swings`).** `Previous` keeps each local light's candela range per channel, `[lo, hi]`, since every pixel last restarted. On a frame that restarts every pixel (cut, key change, first frame, sun turn, a zero budget), the range starts again from that frame's candela. Every pixel's history began at or after that restart: restarted pixels, disocclusions and caster-shadow rejections only begin later. So every intensity any history holds lies inside the range.
- **What a relit frame uploads (`relit_lights`).** For each light whose candela changed, it uploads the position and range, and its **swing**: the largest channel distance from the current candela to either end of the range, relative to that channel's peak (`hi`). The frame words, flags, `RELIT_SLOTS` and `FRAME_BYTES` are unchanged; only the meaning of `relit_change` changes, from the last-frame change to the swing.
- **The decision (`tr1_relit`).** A pixel within a changed light's range restarts, from this frame's spatial start as any restart, where `swing * falloff >= light_budget`. `falloff` is the light's range-windowed inverse square relative to 1 m, the shading's attenuation shape, as in B'. Elsewhere the history continues under MQ1's 3x3 box (`TR1_BOX`), with no window cap.

- **Silhouettes restart too.** Inside the zone a silhouette pixel restarts exactly as on a full restart: its other surface starts from this frame's samples (`tr1_history` rejects it), its occluder coverage restarts (`tr1_coverage` is skipped), and a still camera's kept two-surface state is not carried (the kept branch is skipped). That kept branch carried both stored colours at the light's old intensity, P-FLAT-TR1's known limit 2.5, so its lag is now gone too. Coverage is geometry, not light, but the first RT screen showed that carrying it in the zone made the wall and ground differ from main at the brazier's silhouettes. The difference was -0.0004 ± 0.0010 in wall slope, below the noise but not "equal". So the zone is now exactly main's restart.

**Why it is exact (C).** The light's contribution at a receiver is its candela times an attenuation and shading factor. The attenuation is at most `falloff` relative to 1 m. A continued history is a weighted mean of earlier samples, each lit at an intensity inside `[lo, hi]`. So its error from this frame's intensity is at most `swing * falloff` in units of the light's peak at 1 m, times the same shading factor the current sample carries. Below the budget, a continued pixel's error is below the budget whatever its window. A cap adds nothing there, because the bound already holds for an unbounded window. Where the bound could reach the budget, the pixel restarts every frame the light changes and follows it exactly, as main does. Frames where the light does not change add no error, so they need no decision.

**What this costs in history, by design.** The restart radius no longer collapses when the flicker passes a peak. It follows the swing, which settles at the flicker's range within a few frames: for the courtyard fire (0.70-1.00 of peak), about 0.15-0.22 after frame 4. At budget 0.004 the radius is 5.4-6.3 m in flight. The wall and the ground near the brazier (at most 2.9 m from the fire) restart on every flicker. The crates (7.3 m or more) keep history. Their bound is below 0.003 of the fire's 1 m peak, about 70 lux of the fire's red channel against the spot's ~2,900 lux on them (I).

**Patent reasoning.** Only engine light data decide it: each light's position, range and candela history, plus the receiver's world position. No pixel colour, luminance, moment or lighting difference enters the decision, so it stays inside the resolve's patent notes:
- US 12,437,371 and pending 2026/0162231 claim history control from a lighting difference or a pixel's own moments; neither is used.
- US 10,116,916 and US 10,362,289 claim a neighbourhood colour box or variance as a history control. The only clamp is MQ1's plain min/max of all nine 3x3 samples, which P0 cleared ([m7-p0-claims.md](m7-p0-claims.md)).
- The candela range is the light's own state, not a per-pixel history statistic.

**The other direction.** P-FLAT's second direction, restarting within a fixed fraction of the light's range, is the special case of a constant swing. It would restart the crates whenever the range fraction reached them. It also ignores how much a light actually changes, so a light dimmed by 1 % would restart as widely as one switched off. Not built.

**Unchanged.**
- A zero budget, or more than four changed lights in one frame, restarts every pixel, as main does.
- A light's position, range, cone, radius or kind still changes the key.
- Swept caster shadows, the restart budget (relit frames trace RTDEF2's 64 restart rays, as main's every-frame restarts did) and every other cap are as merged in P-FLAT-TR1.
- Static frames never set `TR1_RELIT`.

## 3. Verification

Setup:
- RTX 4070 SUPER, release builds, 1280x720.
- `game_scene --fox Fox.glb --frames 48 --every 1`, the dusk flight with the real flickering fire and walking Fox.
- Main is `ee361f5` (`bin/base`). The candidate is this branch's swing restart *before* the silhouette change (`bin/cand`, `build.json`), run with `--diag tr-light-budget=0.004`.
- The no-history control is main with `--no-temporal`.

All of it comes from lease `w1`/`w1b`/`w1c`, in `D:/Coding/renderlab-work/p-flat2/runs/w1`.

### 3.1 Flat-surface speckle (`scripts/p-flat-score.py`, frames 8, 16, 24, 32, 40 and 47)

| Run | crate top | crate side | ground | arch | wall |
|---|---|---|---|---|---|
| RT main | 0.079 | 0.044 | 0.012 | 0.015 | 0.026 |
| RT candidate 0.004 | **0.057** | 0.044 | **0.008** | **0.011** | 0.026 |
| non-RT main | 0.075 | 0.024 | 0.016 | 0.016 | 0.016 |
| non-RT candidate 0.004 | **0.054** | 0.024 | **0.012** | **0.013** | 0.015 |

- RT crate tops match B' at 0.002-0.004 (0.058-0.059).
- The crate sides stay in the Fox's conservative swept-shadow volume (p-flat-tr1.md section 3.2).

### 3.2 Flicker (`flicker.py`, P-FLAT-TR1's slope against the no-history control, frames 8-47)

| Run | wall | ground | crate top* | crate side* | arch* |
|---|---|---|---|---|---|
| RT main | 0.985 | 0.996 | 0.881 | 0.995 | 1.001 |
| RT candidate 0.004 | 0.984 | 0.995 | 0.901 | 0.992 | 0.991 |
| non-RT main | 0.982 | 0.995 | 0.830 | 1.016 | 1.002 |
| non-RT candidate 0.004 | 0.982 | 0.994 | 0.886 | 1.013 | 0.990 |

\* Recorded only. The fire is a small part of these surfaces' light, so their slopes are noisy (crate tops) or partly outside the zone (arch).

- For comparison, B' at 0.002 read RT 0.972 / 0.986 and non-RT 0.974 / 0.988 on wall / ground (p-flat-tr1.md). The swing restart removes almost all of B's lag.
- **Paired test (`maskwhere.py`), RT:** regressing the candidate-minus-main mask mean on the control gives wall -0.0004 ± 0.0010 and ground -0.0004 ± 0.0014. Each slope's own standard error is 0.02 (wall) and 0.012 (ground).
- **Where the two differ (`maskdiff.py`, `crops/diff-rt-b004-f24*.png`):** 0.3-2.4 % of wall and 1-3 % of ground mask pixels differ, by a mean 1e-5 to 2e-4 of the mask's luminance. The diff crop shows the wall and the ground by the brazier identical to main except at the brazier box's silhouettes. There, the candidate continued occluder coverage, geometry rather than light, where main's full restart reset it. Outside the zone (the gold sphere, far ground, foliage and the Fox decal) history continues, as intended.

### 3.3 Not run

The plan for the candidate with the silhouette change is `plans/w2.json`:
- the 0.004, 0.002 and 0.001 flights on both paths;
- SK1's fox fixture, main and candidate;
- RF1-RF10 after 32 TR1 frames and the Q-walk, main and candidate, on both paths;
- `gpu_mq1_moving_placement`, `gpu_mv1_mq1_static_identity` (manifest set), `gpu_skinned_streams_history_and_paths`, and `gpu_object_motion` on both paths, all with the validation layer available;
- naga validation and the lib suite.

Both builds' binaries are in `bin/`. The silhouette build has to be compiled first (one build at a time, under `build-slot.lock`).

Static frames never set `TR1_RELIT`, and every new branch is behind it. With `light_budget` 0 the CPU path matches main's (`light_restart`), so static identity is expected by construction (C). It is not measured, and a shader edit can change the bits of unrelated code in the same module, so it must be.

## 4. Cost

- **CPU:** six floats per local light a frame (the swing) and one comparison per changed light.
- **GPU:** none on frames without a changed local light. On relit frames, each pixel loops over at most four lights (one distance and a falloff each) and runs one more `tr1_relit` call at silhouettes. That is less arithmetic than B's cap, which also ran on every continued pixel.
- **Rays:** relit frames keep RTDEF2's 64-ray restart budget, as main's every-frame restarts do. No change in the courtyard.

## 5. Process

- **Time:** 15:00-17:50 UTC.
  - About 50 min of design, code, scripts and analysis.
  - About 12 min of GPU in leases w1 (naga only, stopped by the RAM guard), w1b (two flights, stopped by the RAM guard) and w1c (four flights, then out of lease time: the first non-RT candidate run took 395 s compiling pipelines under CPU contention).
  - The rest went to builds and waits:
    - p-retime's and p-lts3's compiler-quiet windows;
    - a 15:10 kill of every cargo.exe by another unit (p-nrt3, recorded on the schedule);
    - cold release builds of 3.5-7 min each under contention;
    - from 17:00, stillkit's continuous compiles under the one-build rule.
- **Builds:** `D:/Coding/pflat2-target/{cand,base}`, both outside renderlab path segments. Main's tree is a detached worktree at `D:/Coding/pflat2-base`.
  - `build_gate.py` waits out compiler-quiet holds (both the `[X QUIET active]` and `X ... admitted` forms) and takes `build-slot.lock` atomically.
  - It also waits for every foreign compiler to exit, and kills its own cargo tree if a hold appears.
  - `scripts/p-flat-run.py` now takes `RENDERLAB_PFLAT_OWNER` / `RENDERLAB_PFLAT_OUT` and waits for no compiler on the machine before a lease.
- **Evidence** in `D:/Coding/renderlab-work/p-flat2/`:
  - `runs/w1`;
  - `scores-w1.json` / `.txt`;
  - `crops/`;
  - `lease/`;
  - `plans/`;
  - `bin/*/build.json`;
  - the scripts: `flicker.py`, `maskdiff.py`, `maskwhere.py`, `analyze.py`, `plan_w.py`, `build_gate.py`.

```powershell
py -3 -B D:/Coding/renderlab-work/p-flat2/flicker.py rt CONTROL RUN...
py -3 -B scripts/p-flat-score.py OUT.json RUN... --frames 8,16,24,32,40,47
py -3 -B D:/Coding/renderlab-work/p-flat2/maskwhere.py CONTROL MAIN CAND 24 OUT.png
```
