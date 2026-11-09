# P-FLAT-TR1: TR1 history under flickering lights and walking characters

Milestone 8 ([milestone-8.md](milestone-8.md)), the TR1 half of P-FLAT ([p-flat.md](p-flat.md)). Branch `t3/p-flat-tr1` from main `04bd98e`. Root assigned it after P-NRT finished, limited to `src/temporal_reconstruction.rs` and `shaders/temporal_resolve.wgsl`. Labels: **M** measured, **C** read from code, **I** inference.

## Status

| Item | Result |
|---|---|
| Shipped | **Fallback C, by root's rule.** A local light's intensity change still restarts every pixel (`Reconstruction::light_budget` defaults to 0, main's behaviour), plus the swept-shadow test and the restart budget. In the courtyard flight, with the real fire and Fox, C is **byte-identical to main: 48/48 frames on each path**. The fire flickers every frame, so every frame restarts and the other two fixes never engage there. |
| Option B and B' (measured, not default) | Better on every crop, but below main's flicker slope (section 3.1). Root's acceptance requires flicker at least as good as main's on every measured surface, so they ship only as an opt-in `light_budget`. |
| Swept-shadow test | Replaces the stretched AABB for one shadowed sun. SK1's fixture shows no trailing shadow on either path. All of SK1's, MQ1's, MV1's and the object-motion tests pass. |
| Restart budget | Frames that drop history somewhere get the 64-ray restart budget; in the courtyard every frame already did. Estimated cost in section 4. |
| Static identity | **Byte-identical to main:** RF1-RF10 after 32 TR1 frames 13/13 poses on each path; the quarry Q-walk 4/4 frames on each path; MV1's static-identity test passes. |
| Tests | Lib suite 199 passed, 0 failed, including the new key-contract test. MQ1's moving-placement test, SK1's skinned streams (deformed vs baked max byte error 0 on both paths) and the object-motion tests on both paths all pass, with the validation layer available and MV1's manifest set. Clippy: no warnings in the changed files. |
| Courtyard crates, real fire and Fox | Shipped (C) = main: RT crate tops **0.079**, sides 0.044; non-RT tops **0.278**, sides 0.059. B' at budget 0.002: RT 0.059 / 0.043, non-RT 0.206 / 0.059 (section 3.2). |

## 1. The two triggers (from P-FLAT, C)

1. **`Renderer::reconstruction_key` hashed every local light's `Debug` text**, candela included. Any intensity or colour change, such as the courtyard fire's flicker, restarted every pixel's history every frame.
2. **The skinned-pose block of `Renderer::prepare_reconstruction` rejected history in a world AABB.** The box was the caster's swept bounds unioned with the same bounds pushed twice the scene diagonal along each shadowed sun. `tr1_history` rejected every static receiver inside it. Under the courtyard's 5.7° dusk sun the box ran from the Fox across the crates, so they never kept history while the Fox walked.

## 2. Changes

### 2.1 Light intensity: restart by default, an opt-in capped history

- **The key.** `reconstruction_key` hashes each local light with its candela zeroed (`without_candela`). Position, direction, range, cone, source radius and kind still restart history, as do the sun's colour and every other input already in the key.
- **The relit event.** `Previous` keeps the local lights' candela bits (`local_intensities`). When they change, `relit_lights` lists each changed light's position, range and largest relative channel change.
- **The default (`light_budget` 0, fallback C):** the frame restarts every pixel, as main does (section 3).
- **With a positive `light_budget` (opt-in; options B and B'), the frame is relit instead.** A relit frame:
  - sets `TR1_BOX`, the existing MQ1 clamp of continued history to the plain min/max of this frame's 3x3 samples (P0-cleared, no mean or variance), as a moved placement or a changed skinned pose does;
  - sets `TR1_RELIT`, with up to four changed lights in `Tr1Frame::relit`.
- **The cap (`tr1_relit_cap`, `Reconstruction::light_budget`).** Within a changed light's range, a pixel's window is capped at `1 + light_budget / (change * falloff)`:
  - `falloff` is the light's range-windowed inverse square relative to 1 m, the shading's own attenuation shape (`program_api.wgsl`);
  - **where the cap falls below 2, the pixel restarts** from this frame's spatial start, exactly as every pixel did before. Below 2 the capped history would add less than one earlier sample, while still carrying the light's old intensity.
- **Patent reasoning.** Only engine data size the cap: the light's position, range and change, and the receiver's position. No pixel colour, luminance or lighting difference enters it, so the cap stays within the resolve's patent notes. Those notes rule out a lighting difference or own-pixel moments (US 12,437,371, pending 2026/0162231) and a neighbourhood colour box or variance (US 10,116,916 / 10,362,289) as history controls. The clamp is MQ1's plain 3x3 min/max of all nine samples, which P0 cleared (docs/m7-p0-claims.md).
- **Restarts.** More than four changed lights in one frame also restart. Options B (cap only) and B' (cap, restarting below 2) both lagged the flicker slightly near the light, so root's rule made C the default.

### 2.2 The walking character's sun shadow, tested as its swept shadow

With exactly one shadowed sun and no other shadow request (the engine's case), `prepare_reconstruction` uploads:
- the swept skin bounds (the previous and current pose's bounds) with their existing margin;
- the unit direction toward the sun, with no zero component;
- the sun disk's half-angle tangent (`Shadows::directional_disk_angle`), widened by 1 % plus 1e-4.

`tr1_caster_shadow` rejects a static receiver's history when it lies inside the bounds, or when the central ray toward the sun meets the bounds grown by `reach * tan(half-angle)`. Here `reach` is the distance to the bounds' farthest corner.

**Conservative (C):**
- Any disk direction within the half-angle θ of the central direction l satisfies |d − l| ≤ 2 sin(θ/2) ≤ tan θ.
- A disk ray that meets the bounds does so at some distance t ≤ reach, where its point lies within t·tan θ ≤ reach·tan θ of the central ray's point.
- So the central ray meets the bounds grown by that much along every axis. Every receiver whose sun visibility this frame's or the previous frame's pose can change is therefore rejected, on RT's disk rays and on the non-RT lookup's exact disk directions alike.
- Light the character changes elsewhere (sky occlusion, bounce, local-light shadows, reflections) stays under MQ2's existing `TR1_BOX` on posed frames.

Other configurations keep main's box exactly: several shadowed suns, a shadowed local light (the low-level API), or no shadow request. `tr1_caster_shadow` then performs main's comparison.

### 2.3 Restart budget

Frames that drop history somewhere use RTDEF2's restart budget (64 sun rays rather than 32), as frames that restart every pixel do: receivers in a moving caster's shadow, or pixels near a relit light. The restarted pixels keep main's per-pixel ray count. The budget is one frame-wide word, so the frame's continued pixels also get 64.

### 2.4 Frame layout

`Tr1Frame` gains `shadow_light` and the relit block (`FRAME_BYTES` 336 → 448). Both are all zero on frames without a posed caster or a relit light. MQ1's frozen static-control shader reads the old layout's tail as before, so its identity test runs unchanged.

### 2.5 Known limit

A still camera's three-surface silhouette carries its two stored colours without a new sample (TR1b's two-layer limit). On a relit frame those few corner pixels keep the light's old intensity until a sample returns, as on main's moving-placement frames.

## 3. Verification

All screens are on an RTX 4070 SUPER with release builds: main `04bd98e` (`bin/tr1-base`) against the candidate. Every courtyard run is `game_scene --fox` at 1280x720, 48 frames, with the real flickering fire and the walking Fox. Main and the no-history control (TR1 off) come from lease `v1`.

### 3.1 Flicker: history must follow the fire (`flicker_split2.py`, `scripts/p-flat-motion.py`)

The surfaces are the fire-lit red wall and the ground within 2.5 m of the brazier, masked analytically. For each run, the slope regresses its frame-to-frame change in mean linear luminance (frames 8-47) on the no-history control's: 1 follows the flicker, and below 1 lags it.

| Variant | RT wall | RT ground | non-RT wall | non-RT ground |
|---|---|---|---|---|
| main | 0.985 | 0.996 | 0.982 | 0.994 |
| clamp only, no cap (`v1`) | 0.861 | 0.911 | 0.867 | 0.929 |
| B, budget 0.0005 | 0.975 | 0.995 | 0.977 | 0.995 |
| B, budget 0.002 | 0.966 | 0.988 | 0.969 | 0.990 |
| B, budget 0.008 | 0.946 | 0.975 | 0.950 | 0.978 |
| B', budget 0.001 | 0.980 | 0.988 | 0.980 | 0.989 |
| B', budget 0.002 | 0.972 | 0.986 | 0.974 | 0.988 |
| B', budget 0.004 | 0.971 | 0.985 | 0.975 | 0.988 |
| **C (shipped)** | **0.985** | **0.996** | **0.982** | **0.994** |

- B caps the window only. B' also restarts where the cap falls below 2 (section 2.1).
- **Why the clamp alone lags:** the fire changes the wall by about 1.5 % a frame, less than the wall's own 1/d² gradient across a 3x3 neighbourhood. The old-intensity history stays inside the box.
- **Why B lags:** even a cap of 1.03 keeps about 3 % of the previous frame.
- **Why B' still lags slightly:** on frames where the flicker barely changes (its sines near a peak), the cap rises above 2 and history continues. That history holds earlier frames' intensities, which a one-frame change does not measure.
- **Follow-up:** sizing the cap by the change since the history's oldest sample, or restarting within a fixed fraction of the light's range, would remove that lag. Neither was built.

### 3.2 Crates and other flat surfaces (`scripts/p-flat-score.py`, frames 8-47)

| Variant | RT crate top | RT crate side | RT ground | RT arch | RT wall | non-RT crate top | non-RT crate side | non-RT ground | non-RT arch | non-RT wall |
|---|---|---|---|---|---|---|---|---|---|---|
| main = **C (shipped)** | 0.079 | 0.044 | 0.012 | 0.015 | 0.026 | 0.278 | 0.059 | 0.039 | 0.052 | 0.015 |
| clamp only, 32-ray budget (`v1`) | 0.090 | 0.044 | 0.008 | 0.011 | 0.016 | 0.197 | 0.059 | 0.020 | 0.018 | 0.015 |
| B, budget 0.0005 | 0.071 | 0.044 | 0.010 | 0.014 | 0.034 | 0.244 | 0.059 | 0.024 | 0.048 | 0.017 |
| B, budget 0.002 | 0.059 | 0.043 | 0.009 | 0.012 | 0.027 | 0.206 | 0.059 | 0.022 | 0.028 | 0.016 |
| B, budget 0.008 | 0.057 | 0.044 | 0.009 | 0.010 | 0.020 | 0.197 | 0.059 | 0.020 | 0.019 | 0.015 |
| B', budget 0.001 | 0.063 | 0.043 | 0.008 | 0.012 | 0.024 | 0.220 | 0.059 | 0.022 | 0.036 | 0.016 |
| B', budget 0.002 | 0.059 | 0.043 | 0.008 | 0.011 | 0.024 | 0.206 | 0.059 | 0.021 | 0.028 | 0.015 |
| B', budget 0.004 | 0.058 | 0.044 | 0.008 | 0.011 | 0.023 | 0.198 | 0.059 | 0.021 | 0.023 | 0.015 |

- **Clamp-only RT crate tops (0.090) were worse than main.** About 72-75 % of crate-top pixels lie inside the Fox's conservative swept-shadow volume under the 5.7° sun. The test cannot know that the wall already shades the Fox, so those pixels still restart. Main's flicker restarts gave them the 64-ray restart budget, while continued frames dropped to 32. Section 2.3's restart budget fixes this.
- **The crate sides never improve:** they stay inside the swept volume.
- **Non-RT's remaining crate-top grain** is the sun lookup's fixed per-receiver pattern ([p-flat.md](p-flat.md) section 3.2; P-NSQ).

### 3.3 Trailing shadows

- **SK1's fox walk fixture** (`examples/skinned-mesh.rs`: still camera, flat ground, one shadowed sun with a 0.025 rad disk, so the swept-shadow test applies), main against the final candidate (`D:/Coding/renderlab-work/p-flat/crops/tr1-final-fox-fixture.png`: main, candidate, 8x difference):
  - non-RT: at most 30 pixels over 2 codes;
  - RT: up to 300 pixels over 2 codes (maximum 15), all within the Fox's current shadow penumbra, where the restart budget now gives 64 rays.
  - Nothing trails behind the moving shadow on either path.
- **The courtyard noon flight** can't show it: the Fox's short noon shadow falls behind the Fox as the flight camera sees it.

### 3.4 Static identity and tests (leases `v1b`, `v2`)

| Check | Result |
|---|---|
| R0a RF1-RF10, 32 TR1 frames per pose (`gpu_r0a_current`, `RENDERLAB_R1_TR1=1`), main vs candidate | RT 13/13 and non-RT 13/13 poses byte-identical (`scripts/m7-mq2-score.py identity`) |
| TR1 quarry Q-walk (`gpu_tr1_sequence`, frames 8/40/64/96) | RT 4/4 and non-RT 4/4 frame hashes identical |
| `gpu_mv1_mq1_static_identity` | pass |
| `gpu_mq1_moving_placement` | pass |
| `gpu_skinned_streams_history_and_paths` | pass; deformed vs baked max byte error 0 on both paths |
| `gpu_object_motion`, both paths (fixture-only) | pass |
| Library suite | 199 passed, 0 failed, 164 ignored |
| `reconstruction_modules_validate_without_ray_queries` (naga) | pass |
| Courtyard flight, C vs main, every frame | 48/48 frames identical on each path |

- `v1`'s first runs of the SK1, object-motion and MV1-static tests failed only on my setup: the validation layer was disabled and MV1's manifest unset. Their oracles passed, and the `v1b` reruns with the setup fixed pass.
- The identity runs used the B' binary. The shipped code differs from it only in `LIGHT_BUDGET`'s value, which static frames never read.

## 4. Cost

- **Key and relit bookkeeping:** a few hashes and comparisons a frame on the CPU.
- **Swept-shadow test:** one slab test per static pixel, on frames with a changed skinned pose only.
- **Restart budget:** frames that drop history somewhere trace 64 sun rays instead of 32 on RT (RTDEF2's restart count). This is not measured. RTDEF2's quiet quarry rows (moving 32 rays 4.66 ms; cold 256 rays 14.85 ms, at 1080p) put it at about 0.045 ms per ray per pixel, so roughly **+1.5 ms** on such frames at that scale (I). It applies while a character animates in a scene without a flickering light; main traced 32 there while rejecting history across most of the scene. In the courtyard every frame was already a 64-ray restart, so nothing changes. A paired timing run is still owed.
- **Frame upload:** 112 bytes more (336 → 448).
- **Non-RT:** none (no ray budget).

## 5. Process

- **Leases**, all headless with prebuilt binaries:
  - `v1`, 12:44-12:52: flights, fixture, tests, lib.
  - `v1b`, 13:45-13:47: the B sweep and C, fixture, tests with the setup fixed, lib.
  - `v2`, 14:31-14:36: the B' sweep, fixture, RF1-RF10 and the Q-walk on main and candidate.
- **Builds:** in `D:/Coding/pflat-target` and `D:/Coding/pflat-base-target` (main in a detached worktree at `D:/Coding/pflat-base`), outside renderlab path segments. Incremental builds took 2-3 minutes.
- **Quiet-window breach:** a final Clippy check (14:47-14:48) started inside p-retime's compiler-quiet window. My build watcher recognised `[OWNER QUIET active]` lines but not root's `p-retime retime admitted` form; recorded on the schedule.
- **Time:** about 11:21-14:55 wall, mostly queue waits. Root placed P-FLAT in the queue at 12:42.
- **Evidence** in `D:/Coding/renderlab-work/p-flat/`:
  - `runs/v1`, `runs/v1b`, `runs/v2`;
  - `scores-v1*.json` and `scores-v2*.json`;
  - `motion-flicker-*.json`;
  - `runs/v2/rf-identity-*.json`;
  - `crops/tr1-*.png`;
  - `bin/tr1-*/build.json`;
  - `plans/`.

```powershell
py -3 -B D:/Coding/renderlab-work/p-flat/flicker_split2.py rt RUN...
py -3 -B scripts/p-flat-score.py OUT.json RUN... --frames 8,16,24,32,40,47
py -3 -B scripts/m7-mq2-score.py identity RF_BASE RF_CAND --out rf-identity.json
```
