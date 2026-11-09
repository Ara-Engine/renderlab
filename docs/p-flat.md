# P-FLAT: speckle on the courtyard's flat crates

Milestone 8 quality defect ([milestone-8.md](milestone-8.md)). Islam, 2026-11-09, on the v0.1.0 courtyard frames: "in both quality defect images you showed I can see noise on the boxes". Branch `t3/p-flat` from origin/main `f430c8e`. Labels: **M** measured here, **I** inference, **C** read from code.

## Status

| Item | Result |
|---|---|
| Defect | **Reproduced on both paths (M).** The crates (`wood`: constant colour, roughness 0.75, no texture) carry grain on RT and orange sparkle on non-RT. The ground, arch and wall are 10-40x cleaner by the speckle metric. |
| Cause, RT | **Three things together (M, C).** (1) The courtyard's spot lamp sat inside its own iron post: 74-82 % of its 0.1 m source disk is blocked for every receiver, so every spot-lit pixel is a permanent penumbra. (2) LC2 samples that penumbra with four rays per pixel per frame. (3) TR1 never accumulates history during the flight: the flickering fire changes its history key every frame, and the walking Fox's sun-shadow rejection box covers the crates. |
| Cause, non-RT | **The non-RT sun lookup (M).** On a still camera the crate grain is a frozen per-receiver pattern (residual correlation 0.995 between frames 40, 47 and 191) that the sun alone produces (0.733 → 0.025 with the sun off). It is the open P-NSQ gap. TR1's resets (cause 3) make it worse in motion. |
| Fix on this branch | The example's lamps clear their own fixtures (`examples/game_scene.rs`). RT crate tops in flight: **0.759 → 0.079**, sides 0.270 → 0.044, arch 0.059 → 0.015. Non-RT: 0.339 → 0.278. `--diag lamp=post` reproduces main byte for byte on both paths. |
| Handoff | TR1's two history resets: P-NRT (`src/temporal_reconstruction.rs`, section 6). Non-RT sun pattern: P-NSQ. LC2 needs no change: with the lamps fixed and both resets removed, RT crate tops reach 0.041 against 0.034 with the local lights off. |
| Reference | Matched courtyard (sun at hour 17.5, uniform sky, no local lights, no Fox) against a 16,384-spp path trace: RT crate tops 0.021-0.045 against the reference's own 0.080-0.096, RMS 1.3 codes. Non-RT 0.056-0.060 in flight but 0.509 on a still camera, RMS 4.0-4.6 codes. |

## 1. Method

### 1.1 Harness

- `examples/game_scene.rs` and `examples/reflection_matched.rs` take `--diag KEY=VALUE` (`support/estimator_diag.rs`). The keys set estimator settings the public `Settings` does not expose (`IndirectSettings`, `Reconstruction`, `VirtualShadowSettings`, soft-shadow mode, ray budget, light mask and source-radius scale), applied after the engine sets its defaults. `game_scene` adds scene keys: `flicker=0` (steady fire), `lamp=post` (v0.1.0's lamp placement) and `sun=0`. Both examples now record each frame's camera eye and target (and `t`) in `run.json`.
- `reflection_matched --hour H` puts the frozen RQ1 scene under the courtyard's dusk sun, so `--matched` and `--reference SPP` compare it against the path tracer.
- `scripts/p-flat-run.py`: RD1's lease runner under owner `p-flat`, gated on owner lines for the M8 owners (and on an exact line through `RENDERLAB_PFLAT_AFTER_LINE`). `scripts/p-flat-batch.py` runs one plan of prebuilt binaries inside a lease.

### 1.2 Metric (`scripts/p-flat-score.py`)

Masks come from the scene, not the image. Each pixel's centre ray is cast against the courtyard's analytic boxes, spheres and ground at the frame's time and recorded camera. Pixels are labelled crate top, crate side, ground, arch or wall. Occluders (spheres, spinning crate, glass, bush hulls, post and lamp, fire and smoke, hero) and both ground decals are excluded, and every face is eroded by 3 px, so silhouettes and TR1 jitter never enter. Inspected overlays: `D:/Coding/renderlab-work/p-flat/crops/mask-rt-16.png`, `mask-rt-47.png`.

Per class, in display codes of Rec.709 luma:
- **speckle** = mean |Y − median3x3(Y)|: isolated grain. Edges and smooth gradients score near zero.
- **lap** = mean |4-neighbour Laplacian| (in `scores-*.json`).
- With a reference: RGB RMS against it, and the reference's own speckle (its floor: penumbrae, gradients and split-half noise).

Flight values below average frames 8-47; frame 0 is a cut. Still values are the last frame.

## 2. Bisect (lease `a1`, main `f430c8e` plus the harness)

`game_scene --fox Fox.glb`, 1280x720, 48 frames, every 8th written, release build, RTX 4070 SUPER. One process per row. The two TR1 switches turned out to be no-ops in this scene (byte-identical to the default, 7 of 7 frames); section 3 explains why.

| Variant | RT crate top | RT crate side | RT ground | RT arch | RT wall | non-RT crate top | non-RT crate side | non-RT ground | non-RT arch | non-RT wall |
|---|---|---|---|---|---|---|---|---|---|---|
| default | **0.759** | 0.270 | 0.018 | 0.059 | 0.048 | **0.339** | 0.077 | 0.038 | 0.051 | 0.015 |
| local lights off | **0.058** | 0.013 | 0.006 | 0.011 | 0.012 | 0.200 | 0.077 | 0.022 | 0.017 | 0.016 |
| spot lamp only | 0.758 | 0.267 | 0.012 | 0.011 | 0.012 | 0.199 | 0.077 | 0.022 | 0.017 | 0.016 |
| local shadows off | 0.067 | 0.010 | 0.008 | 0.014 | 0.011 | 0.283 | 0.057 | 0.037 | 0.051 | 0.014 |
| source_radius 0 | 0.071 | 0.013 | 0.009 | 0.014 | 0.014 | 0.339 | 0.077 | 0.039 | 0.051 | 0.016 |
| GI off | 0.696 | 0.246 | 0.015 | 0.061 | 0.048 | 0.320 | 0.047 | 0.034 | 0.044 | 0.006 |
| G3 filter off | 0.762 | 0.283 | 0.030 | 0.081 | 0.094 | 0.379 | 0.120 | 0.053 | 0.087 | 0.094 |
| reflections off | 0.759 | 0.270 | 0.018 | 0.059 | 0.047 | 0.339 | 0.077 | 0.039 | 0.051 | 0.015 |
| rough specular traced (max_roughness 1) | 0.819 | 0.301 | 0.022 | 0.065 | 0.321 | 0.458 | 0.134 | 0.043 | 0.061 | 0.324 |
| exact sun (`Quality::Reference`) | 0.728 | 0.270 | 0.015 | 0.056 | 0.048 | 0.339 | 0.077 | 0.039 | 0.054 | 0.015 |
| fog off (volumetric, height, aerial) | 0.797 | 0.294 | 0.020 | 0.061 | 0.063 | 0.354 | 0.082 | 0.041 | 0.054 | 0.016 |
| TR1 off | 1.665 | 0.495 | 0.077 | 0.162 | 0.083 | 0.614 | 0.123 | 0.062 | 0.093 | 0.018 |
| still, 48 frames | 0.175 | 0.042 | 0.013 | 0.020 | 0.013 | 0.733 | 0.211 | 0.138 | 0.104 | 0.011 |
| still, 192 frames | 0.090 | 0.015 | 0.011 | 0.016 | 0.009 | 0.733 | 0.202 | 0.137 | 0.101 | 0.009 |
| still, local lights off | 0.061 | 0.009 | 0.011 | 0.016 | 0.010 | 0.799 | 0.217 | 0.139 | 0.113 | 0.013 |

Readings (M):
- **RT:** the grain is LC2's soft local-light shadow rays. Lights off, local shadows off and radius 0 each remove it, and the spot lamp alone reproduces it. GI, G3's filter, reflections, the exact sun and fog leave it unchanged. Tracing the rough specular (roughness 0.75 above R1's 0.5 band) adds noise of its own (wall 0.048 → 0.321), so the GI-cache fallback is not the source.
- **RT still** converges toward the lights-off floor (0.175 at 48 frames, 0.090 at 192, floor 0.061), but in flight it never does: TR1 gives no accumulation in motion, only its spatial start (TR1 off: 1.665).
- **Non-RT:** a third of the grain in flight is local lights (0.339 → 0.200), the arch or fire cube maps rather than the spot. On a still camera the grain is frozen and remains with the lights off (0.799). Section 3 isolates it.
- The ground, arch and wall stay at 0.006-0.06 on RT. Non-RT's still ground (0.138) is the same sun pattern on the ground's penumbrae (P-NSQ's recorded gap).

## 3. Isolation (lease `a3`)

| Variant (flight unless still) | RT crate top | non-RT crate top |
|---|---|---|
| default (byte-identical to `a1`'s, 7 of 7 frames, both paths) | 0.759 | 0.339 |
| fire steady (`flicker=0`) | 0.722 | 0.193 |
| no Fox (two rigid spheres) | 0.766 | 0.348 |
| no Fox + fire steady | **0.172** | **0.079** |
| no Fox + fire steady + local lights off (floor) | 0.034 | 0.071 |
| spot lamp beside its post (`lamp=offset` in `a3`, this branch's default) | **0.088** | 0.276 |
| lamp + fire steady | 0.067 | 0.132 |
| lamp + fire steady + no Fox | **0.040** | **0.065** |
| arch light only | 0.084 | 0.193 |
| fire light only | 0.073 | 0.357 |
| still, sun off | 0.400 | **0.025** |
| still, exact sun lookup | 0.164 | 0.733 |
| still, GI off | 0.163 | 0.704 |
| still, `vsm-refresh=1`, `vsm-walk-cap=0` | 0.175 | 0.847 |
| still, GI directions unrotated | 0.181 | 0.726 |
| still, lamp beside post | 0.065 | 0.630 |

RT's still with the sun off rises because the crates are darker there, and sRGB spends more codes per unit of radiance in the dark (I).

### 3.1 The three RT mechanisms

1. **The spot source is inside its post (M, C).** `post_lamp - 0.15 m` = (3.0, 3.05, 3.0) lies inside the post box (x, z ±0.06 m, y 0-3.2 m). A Monte Carlo of LC2's disk (radius 0.1 m, facing the receiver, as `shaders/local_light_ray.wgsl`) against the post, cap and bulb gives visible fractions of 0.215, 0.262 and 0.184 from the three lower crate tops and 0.249 at the spot's ground target. The centre ray is blocked. Both paths agree on the loss: crate tops gain about 21 codes with local shadows off and only 8 with them on. The arch light also clips its bulb by 3-8 % from the pillar faces.
2. **Four rays in a permanent penumbra (C).** `rl_local_shadow` traces four disk samples per light per pixel per frame and rotates them with TR1's frame index (`LcFrame.info.z = reconstruction_frame + 1`, `src/light_cluster_gpu.rs`). At visibility ~0.2 the per-frame estimate is 0, 0.25 or 0.5 almost everywhere the spot lights.
3. **TR1 history never continues in this scene (M, C).** Either trigger alone suffices (`flicker=0`: 0.722; no Fox: 0.766; both: 0.172):
   - **The history key hashes light intensity.** `Renderer::reconstruction_key` (`src/temporal_reconstruction.rs:692`) hashes every local light's full `Debug` text, candela included. The courtyard's fire flickers its candela every frame, so `continued` is `None`, `state.reset` is true and every pixel restarts every frame. The `motion_box` and `view_budget` switches therefore never applied (both need `!state.reset`), which is why their rows are byte-identical.
   - **The skinned-pose rejection box covers the crates.** In `Renderer::prepare_reconstruction` (`src/temporal_reconstruction.rs` ~905-949), a changed skin pose sets `shadow_bounds` to the swept skin bounds unioned with the same bounds pushed `2 x scene size` along each directional light's travel. Static pixels inside lose their history in `tr1_history` (`shaders/temporal_resolve.wgsl`). Under the 5.7° dusk sun that box runs from the Fox far east, below the Fox's height, and contains the crates. The tall arch pillars are outside it, which is why the Fox matters for the crates and not the arch (arch 0.059 → 0.024 with a steady fire, 0.053 with no Fox).

### 3.2 Non-RT

- The still-camera grain is the sun: 0.733 → 0.025 with `sun=0`. The exact lookup, GI off, a full refresh and unrotated GI directions leave it, and its residual is frame-invariant (correlation 0.995 between frames 40, 47 and 191). The pattern is fixed per receiver: `VirtualShadowSettings::directions` evaluates "the first of each receiver's R2 sequence" (C), which TR1 cannot average. This is P-NSQ's open gap, here measured on the crates.
- In flight, TR1's resets (section 3.1) add the arch and fire cube maps' per-frame aliasing: 0.339 → 0.193 with a steady fire.

## 4. Reference (lease `a3`)

`reflection_matched --hour 17.5 --matched` (uniform sky, no fog, decals, local lights or post; photoreal sphere hero) on both paths, against `--reference 16384` at frames 0, 16 and 47 (11-12 s a frame, split-half noise 1.21-1.25 codes RMS over the whole image, no invalid pixels).

| Crate tops | speckle | reference speckle | RMS vs reference | mean Y vs reference |
|---|---|---|---|---|
| RT flight, frames 16 and 47 | 0.026, 0.021 | 0.093, 0.080 | 1.31, 1.28 | 79.8, 79.7 vs 80.5, 80.1 |
| non-RT flight, frames 16 and 47 | 0.056, 0.060 | 0.093, 0.080 | 4.23, 4.01 | 78.6, 78.8 vs 80.5, 80.1 |
| RT still, frame 191 vs reference frame 0 | 0.040 | 0.096 | 1.26 | |
| non-RT still, frame 191 | **0.509** | 0.096 | 4.51 | |

Without local lights, the Fox or the flicker, the flat material renders at or below the path tracer's own grain on RT. Non-RT is clean in motion but grainy and about 2 codes dark on a still camera: the sun pattern again. The path tracer has no point or spot lights, so the local-light part of the defect has no path-traced reference. Its floor here is the renderer's own lights-off run (0.034 RT, 0.071 non-RT).

## 5. Fix (this branch)

`examples/game_scene.rs` (lease `a4`, same harness):
- **The post lamp's head hangs beside its post.** Bulb and spot move 0.22 m east and 0.22 m north of the post axis, under a cap widened to span post and head. The spot keeps its target. Monte Carlo of the new placement: its disk is 0.999-1.000 visible from the crates and its ground target.
- **The arch light sits 0.25 m below its bulb** (was 0.2 m), one source radius plus one bulb radius: 1.000 visible from the pillar faces, ground and crates. The lintel underside's 0.96 is the bulb genuinely between them.
- `--diag lamp=post` restores v0.1.0's placement: **byte-identical to main on both paths (7 of 7 frames each)**.

No renderer code, setting, ray count or sample budget changes.

| Flight (frames 8-47) unless still | RT crate top | RT crate side | RT arch | non-RT crate top | non-RT crate side |
|---|---|---|---|---|---|
| main (v0.1.0 lamps) | 0.759 | 0.270 | 0.059 | 0.339 | 0.077 |
| **this branch** | **0.079** | **0.044** | **0.015** | **0.278** | **0.059** |
| this branch, still (frame 47) | 0.058 | 0.012 | 0.016 | 0.631 | 0.177 |
| this branch + both TR1 triggers removed (proxy: no Fox, steady fire) | 0.041 | 0.017 | 0.007 | 0.065 | 0.024 |

The crates are brighter now that the spot reaches them: crate-top mean luma 125.2 → 139.5 (RT) and 118.4 → 137.9 (non-RT). The two paths now agree within 1.6 codes, where main differed by 6.8.

**Crops (inspected):**
- `D:/Coding/renderlab-work/p-flat/crops/p-flat-final-crates-f016.png` and `p-flat-final-crates-f047.png`: RT before, RT after, non-RT before, non-RT after.
- `p-flat-final-full-f016.png`: full frames, same order.
- `p-flat-crates-f016.png` and `p-flat-crates-f047.png`: main, the lamp fix, and the lamp fix with the TR1 triggers removed, both paths.
- `p-flat-lamp-head.png`: the lamp before and after.

RT's crate grain is gone at both frames. Non-RT keeps orange sun-penumbra grain on the right crate's west face and the upper crate's top (section 6). Nothing else in the frame changes.

**Cost:** none designed in. LC2 still traces four rays per light per pixel. Rays toward the spot now reach the source instead of stopping at the post, so they traverse somewhat further (I). The drained timing pair was cut short: the shared 2 GiB runtime guard stopped the lease on its legacy half (another project's builds held the machine's memory). The fixed half (RT 16.1 ms, non-RT 25.6 ms a frame, 120 frames) ran under that contention and is not a cost claim.

## 6. Handoff

Root decides who implements these. P-FLAT edited none of these files.

**P-NRT, TR1 (`src/temporal_reconstruction.rs`, `shaders/temporal_resolve.wgsl`).** These two triggers stop TR1 from accumulating in any scene with a flickering or animated local light, or an animated character under a low sun. Every stochastic estimator then shows its raw per-frame noise.
1. `Renderer::reconstruction_key` (line 692): `format!("{lights:?}")` hashes each local light's position, intensity and radius, so any intensity change restarts every pixel's history. Light identity and topology belong in the key. Intensity and position edits want an event that continues history and bounds it (MQ1's 3x3 box, as rigid motion already does), without trailing a flicker.
2. `Renderer::prepare_reconstruction`, the MQ2 skinned-pose block (about lines 905-949): `shadow_bounds` is a world AABB of the swept skin bounds and the same bounds pushed 2x the scene diagonal along the sun's travel. `tr1_history` rejects every static pixel inside it. Under a 5.7° sun that box spans much of the courtyard below the character's height. Testing receivers against the swept bounds' footprint along the light, a sheared prism, would keep unaffected history. Clamping with TR1_BOX instead of rejecting there is the other option.

Expected effect, by proxy: RT crate tops 0.079 → 0.041 and non-RT 0.278 → 0.065 in flight.

**P-NSQ, non-RT sun (`virtual_shadow*`).** On a still camera the crates show the sun lookup's fixed per-receiver pattern (0.631; 0.025 with the sun off). Its residual does not change between frames, so TR1 cannot average it. RT rotates its disk directions per frame (`Reconstruction::rotate_directions`); the virtual lookup evaluates the first `directions` of each receiver's sequence every frame. This is the open "grainy, harder penumbrae on non-RT" gap, measured here on a flat material.

**P-LTS, local lights.** No change needed: with the lamps fixed and both TR1 triggers removed, RT crate tops reach 0.041 against 0.034 with the local lights off. The defect showed one authoring hazard, though: a soft source placed inside or touching geometry gives a permanent penumbra, visible as grain wherever TR1 cannot settle, and nothing warns about it.

## 7. Process

- **Leases** (`scripts/p-flat-run.py`, all headless, prebuilt binaries):
  - `a1`: the bisect, 34 runs, about 2.5 min.
  - `a3`: isolation plus the matched scene and the reference, 37 runs, 4 min.
  - `a4`: verification, 1 min. The 2 GiB runtime guard stopped it on run 11 of 12, the timing pair's legacy half; every quality run had completed.
  - The `a3` waiter was withdrawn once, before taking a lock, when a newer hold arrived. The `a4` waiter was requeued at root's 7 GiB short-lease admission.
- **Builds** in `D:/Coding/pflat-target`, outside renderlab path segments: cold 2 min 45 s, then 5-12 s incremental.
- **Time:**
  - about 1 h 20 min of harness, scorer and analysis;
  - about 8 min of GPU leases;
  - about 1 h of holds and queueing;
  - 52 min lost to the API outage (09:10-10:02).
- **Evidence** in `D:/Coding/renderlab-work/p-flat/`:
  - `runs/a1`, `runs/a3`, `runs/a4`: frames and each run's `run.json`;
  - `scores-*.json` and `table-*.txt`;
  - `crops/`;
  - `lease/` and `logs/`;
  - `bin/{a,b,c}/build.json`, with each binary's base commit.

```powershell
py -3 -B scripts/p-flat-run.py LABEL 15 30 -- py -3 -B scripts/p-flat-batch.py PLAN.json OUT 13.5
py -3 -B scripts/p-flat-score.py OUT.json RUN_DIR... [--reference REF_DIR]
```
