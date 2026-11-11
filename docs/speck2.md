# U-SPECK2: specks and grain from reflections and the fire's restart sphere

Unit U-SPECK2, branch `t3/speck2` from main `d1ea969f`, 2026-11-11. The second half of Islam's report on the game scene's flight ("almost as you go all frames has noise on ground alot of points of white... also on all of the objects"); U-SPECK ([speck.md](speck.md)) has the GI half. Labels: **M** measured here, **C** read from code, **I** inference.

Flight: `game_scene --fox Fox.glb --size 1920x1080 --frames 180 --every 7 --diag cut=1`, both paths, RTX 4070 SUPER, NVIDIA 617.14. Scores are `tools/speck2/grain.py`: grain (RMS of luminance minus its 5x5 mean, 8-bit codes) / specks (pixels more than 6 codes above their 7x7 median).

## Result

| Symptom | Cause | Fix | Before | After | Clean level |
|---|---|---|---|---|---|
| (b) white specks on the cube's right face, frame 14 | one glossy ray in tens of thousands strikes a lamp bulb | lone-stratum bound | non-RT 3.81 / 11, RT 1.17 / 2 | 0.87 / 0, 0.92 / 0 | bulbs removed: 0.82 / 0, 0.85 / 0 |
| (a4) dots on the cube's left face, frame 14 | lone glossy rays on fire-lit surfaces, shown raw inside the fire's restart sphere | the same bound | 2.65 / 35, 2.64 / 35 | 2.28 / 3, 2.22 / 0 | steady fire: 2.17 / 6, 2.11 / 1 |
| (a3) grain on the ground by the brazier, frame 98, RT | four disk rays per soft local light, shown raw inside the restart sphere | 16 disk rays for restarted receivers | RT 2.63 / 73 | 1.04 / 11 | steady fire: 1.02 / 8 |
| RT reflections of surfaces facing away from the camera were unlit by local lights | hit shadow rays started on the camera's side of the surface | offset toward the light | | | |

Before is main without these commits; after is `609a2b55` (the three fix commits). Section 5 has the merged head against current main.

**Known loss, accepted by root for the 2026-11-12 preview:** the bound removes bulb glints from rough metal (section 2.3). Cost on frame time is **not measured** (section 4).

## 1. (b): a lamp bulb, struck by one ray in tens of thousands (M)

- The specks sit at the same two pixels of frame 14, (733-734, 551) and (730-731, 555), in every switch flight of U-SPECK's lease, and faintly with all local lights off (luminance 113 and 109 against 103 and 97 around them). Which light is on only scales them: arch lamp only 132 / 153, fire only 126 / 123, all lights 149 / 176.
- **One switch:** `--diag bulb=off` (U-SPECK's, the two lamp bulbs' spheres left out) removes both on both paths: 149 / 176 → 113 / 112.
- **What it is (C, M).** The cube is paint at roughness 0.35: 16 strata per pixel (`shaders/reflections.wgsl`). The arch lamp's bulb is a 12 cm sphere glowing at 60,000 nits, 6.8 m away. A replay of that pixel's lobe (200,000 rays) puts 0.001 % of its rays on each bulb: about one strike per frame over the whole face. A struck pixel takes 1/16 of 60,000 nits or more where its neighbours take none.
- **The non-RT-only part.** The bulb's surface is 0.19 m from its own lamp. On non-RT the hit is lit through the pooled shadow map and reads up to four times its glow; `--diag bulb=dark` (glow kept, no reflectance) drops non-RT to exactly RT's level (149 → 130, 176 → 136). RT missed that light by a bug (section 3).
- Not a history effect, not the fire, not GI: steady fire, GI off and particles off all keep the specks. Temporal reconstruction off moves them (its frame word turns the strata).

## 2. The lone-stratum bound

### 2.1 Why a bound

A glossy pixel averages at most 16 strata and temporal reconstruction about 16 frames: some 256 samples. A target that one ray in tens of thousands strikes cannot average out at that count. The true mean of the bulb in such a pixel is a few nits; a strike is thousands.

Root asked whether the lamp's analytic specular already draws the bulb, so that rays reaching it count it twice and multiple importance sampling would lose nothing. Arithmetic: the arch lamp is 30,000 cd; the bulb's glow is 60,000 nits x π 0.06² = 678 cd, 2.3 % of it. That 2.3 % is the true double count. The rest of what a ray sees there is lamp light bounced off the bulb, which is real. MIS needs the engine to know that this mesh is that light's emitter and to tell glow from bounce: `shade` returns one radiance, the lamp sits 0.25 m from its bulb, and the API has no link between them. Weighting whole hits by distance to a light would also drop the brazier top's bounce, which is not a double count. Root chose the bound.

### 2.2 The rule

The brightest stratum counts for at most the luminance of all the others together (`primary_frame.reflection.z`, default 1; 0 switches it off). So no single stratum carries more than half a pixel's estimate.

- Two strata on one feature bound nothing: a feature the lobe resolves keeps its energy.
- Mirror rays (one ray, perceptual roughness below 0.05) are never bound: the chrome sphere and the puddle are untouched.
- No state, no neighbours, same code on both paths.
- 0.5 instead of 1 was screened: the same specks, slightly more energy removed (still: -1.10 against -0.99 codes). 1 stays.

### 2.3 What it costs in the settled frame (M)

`--still`, 96 frames, bound on against off on one binary (`RENDERLAB_REFLECTION_LONE=0`), 576 boxes of 60 px, both paths (`tools/speck2/energy.py`):

| | Largest box change, luminance | Worst channel | Boxes above 1 code | Whole frame |
|---|---:|---:|---:|---:|
| Non-RT | -0.99 | 1.50 | 1 | -0.004 |
| RT | -0.99 | 1.50 | 1 | -0.005 |

That one box is the post lamp's bulb reflected in the gold sphere (roughness 0.25). **With the bound the glint is gone**: the box reads as it does with the bulbs removed (-1.13). This fails the test root set for a clamp (the reflection must still show at the right size and brightness), and root accepted it for the preview after looking at the crops, because main does not draw that glint correctly either:

- On main the glint is a cluster of sand-like specks that has not settled after 96 still frames, and in motion stray white dots cover the whole sphere (frame 98). About one ray in 50 from those pixels reaches the bulb: four hits per pixel over the history.
- With the bound the stray dots go; the glint shrinks (frame 98) or vanishes (frame 21, the still).
- The right picture is a smooth glow at the mean. No per-frame rule on 12 strata gives it: keeping a hit whole is sand, cutting it is dark, and averaging extra rays only when a ray hits loses 65 % of the energy (the estimate is conditioned on its own first sample).

Fixes that give the glow (gap list): an emitter link so the light's sample covers its bulb (MIS), or carrying the bounded energy in its own long-window history.

## 3. Reflection hits offset toward the light (C, M)

`shadow_origin` puts a receiver's shadow origin on the side of the surface the camera sees. A reflection hit's viewed side is the reflection ray's. For every reflected surface that faces away from the camera the hit's local-light rays (RT) started inside the surface, so closed meshes shadowed their own reflected faces from point and spot lights. The hit's receiver now takes the light's side (`thin`), as `rl_hit_shadow` already does for the sun.

- RT, frame 14, gold sphere: the crates' spot-lit faces go from dark to lit in the reflection and now match non-RT (`offset-fix-rt-f014.png`).
- Alone it raises RT's cube-right grain 1.17 → 1.53: RT now lights the bulb as non-RT does, as it should; the bound then removes the speck.
- RT settled still against before: largest box change 0.64 codes. Non-RT's map lookup already read lit there (I: its origin was inside by one epsilon, under the map's bias); both paths now take the same origin.
- Changes any reflected surface lit by a shadowed local light whose reflected face points away from the camera. Camera receivers are untouched.

## 4. Restarted receivers: more disk rays and strata

Inside a changed light's restart sphere ([p-relit.md](p-relit.md)) a receiver starts its history again on every frame the light changes, so it shows one frame's estimate. The sun already gets its restart count there (64 rays). Two estimators did not:

- **Local-light disk rays, RT.** A camera receiver the resolve restarts traces `LcFrame.info.w` (16) disk rays per soft local light instead of four. Reflection hits keep four. Brazier ground, frame 98: 2.63 / 73 → 1.04 / 11; 32 rays reach 0.81 / 9. Non-RT filters its maps and was already at 0.63 / 5.
- **Glossy strata, both paths.** Twice the strata (`primary_frame.reflection.w`). Small effect on this scene once the bound is on (cube left: 3 → 3 specks non-RT, 3 → 0 RT); kept because a restarted glossy receiver has nothing else.
- `rl_restarts` asks the resolve's own tests (`shaders/temporal_zones.wgsl`) on the same record, so the three agree on who restarts.
- **The fire still flickers at full amplitude.** Nothing is held or blended: restarted pixels restart as before, with more rays. A settled frame is unchanged (still, all on against the bound alone: 0.00 codes in every box).
- **Not fixed: history inside the sphere.** The restart is for the light's intensity, not its shadow, so the right fix lets restarted receivers keep their history and rescales the light's share. That needs each pixel's share of the changed light (a second shade, or bookkeeping in the light services and a new per-pixel channel); a regression of the pixel on the light's intensity was worked through and is too noisy at a 10 % flicker. Recorded in the gap list.

**Cost: not measured.** Root ruled that timing waits for a quiet machine. Expected from the code: the bound is a dot product per stratum and one branch; the disk rays are 12 more per soft local light per restarted receiver on RT only (the quarry has no local lights and no restart spheres, so nothing there); the strata are 12-16 more rays and hit shades per restarted glossy receiver on both paths.

## 5. Merged head against current main (M)

Lease `speck2-k1r` (86 s, picture only, no game running): the merged head `ecd364b3` (`bin/s5`) against main `53d29e4f` (`bin/m5`, its own tree and Cargo target), the same flight on both paths, then the lib suite on the merged head.

| Box (grain / specks) | Non-RT main | Non-RT merged | RT main | RT merged |
|---|---:|---:|---:|---:|
| Cube right face, f14 (b) | 3.81 / 10 | 0.88 / 0 | 1.17 / 2 | 0.92 / 0 |
| Cube left face, f14 (a4) | 2.68 / 34 | 2.30 / 2 | 2.66 / 36 | 2.25 / 0 |
| Cube, f21 | 0.90 / 8 | 0.92 / 5 | 0.79 / 4 | 0.82 / 3 |
| Brazier ground, f98 (a3) | 0.46 / 0 | 0.46 / 0 | 2.59 / 61 | 0.91 / 0 |
| Bush shadow, f98 | 0.42 / 0 | 0.42 / 0 | 0.50 / 0 | 0.41 / 0 |
| Wall f21, cube shade f98, open ground f98 | 0.19, 0.74, 0.12 | 0.19, 0.73, 0.12 | 0.21, 0.82, 0.12 | 0.20, 0.75, 0.12 |

- **Whole frame, `tools/blobs.py`** (blob pixels over the 27 frames; it counts soft blobs, which are the GI half's): non-RT median 10,215 → 10,215, worst frame 13,453 → 12,989; RT median 9,710 → 9,600, worst 13,428 → 13,376. No frame is worse.
- **Settled still against main** (96 frames, 576 boxes): non-RT one box -0.99 codes luminance (1.50 worst channel), RT one box -0.83 (1.29); the same box on both, the gold sphere's bulb glint (section 2.3). Every other box is under 1 code.
- **Looked at** (2x nearest, both paths, main beside merged): frames 14, 21, 28 (four frames after the cut) and 98. The cube's specks and dots, the stray dots on the gold sphere and RT's grain by the brazier are gone. The gold sphere's bulb glint is smaller (frame 98) or gone (frame 21). The cube's dark blotches at frames 21 and 28 are in both builds: they are U-SPECK's (GI after the cut). Nothing new appears.
- **Lib suite on `ecd364b3`:** `test result: ok. 220 passed; 0 failed; 172 ignored; 0 measured; 0 filtered out; finished in 4.78s`, including `light_cluster_kernel_and_every_shading_head_validate` (every shading head with the new composition, parsed and validated) and `reflection_settings_bands`. The 172 ignored are the GPU tests; none was run.
- Sheets: `final-f014.png`, `final-f021.png`, `final-f028.png`, `final-f098.png`, `final-f098-rt.png`.

## 6. Records and switches (for the ara-gpu port)

- **No new buffer, texture or binding.**
- `ReflectionSettings::frame_words` word 2 (was 0): the lone-stratum limit; word 3 (was 0): the strata multiple of restarted receivers. Both backends already copy the four words.
- `LcFrame.info.w` (was 0): restart disk rays, written by `light_cluster::header` (word 23). A backend that leaves it 0 keeps four rays.
- Every traced shading module now composes `rl_restarts` and the zone tests (`temporal_zones.wgsl` with `rl_zone_` names) through `light_cluster::shading_api`, reading `primary_frame.zones`.
- No planner signature or record changed (`temporal_reconstruction::Planner`, `gi_cache::Planner`).
- Measurement switches, one process each: `RENDERLAB_REFLECTION_LONE`, `RENDERLAB_REFLECTION_RESTART_STRATA`, `RENDERLAB_LOCAL_RESTART_RAYS`.

## 7. Process

- Leases, headless, prebuilt binaries, `scripts/n8-run.py` with its watchdog, no game running: `speck2-j1r` (265 s: `609a2b55` with each fix switched, U-SPECK's `bin/d1` as before), and section 5's.
- Not run: the GPU tests (`gpu_r1_mirror_identity`, `gpu_r1n_mirror_identity`, `gpu_lc2_penumbra`), the quarry and Showcase sequences, any timing, and the path-traced reference.
- "Before" in the Result table is U-SPECK's `bin/d1`: main `30e7e26c` plus its tooling. Its scores in these boxes equal the fixes-off run of this branch (3.80 / 11 against 3.81 / 11), so the boxes are valid; it is not a whole-frame non-RT baseline, which section 5 supplies.
- Sheets (inspected, 3-8x nearest) in `D:/Coding/renderlab-work/unit-speck2/sheets/`: `b-f014-cube-right.png`, `a4-f014-cube-left.png`, `a3-f098-brazier-ground.png`, `offset-fix-rt-f014.png`, `gold-f021-bound.png`, `gold-f098-bound.png`, `still-bound-960-540.png`.
- No path-traced reference was used: the settled still on the same binary with the bound off is the working reference for the bound's energy.

```powershell
py -3 -B tools/speck2/host.py build2 NAME [TREE]
py -3 -B tools/speck2/host.py lease LABEL 14 -- py -3 -B tools/speck2/flights.py OUT name=EXE[,ENV=VALUE]...
py -3 -B tools/speck2/grain.py RUN_DIR...
py -3 -B tools/speck2/energy.py BEFORE.png AFTER.png
```
