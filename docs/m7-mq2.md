# M7-MQ2: reflections that keep up with the camera, and skinned history with GI on

2026-11-08, branch `t3/m7-mq2`: MQ1 (`f753cfa`) and MP1 (`b276863`) merged onto main `ddd823c`, then main `4c8ef7d` (R1n, GC1, PC1, LC2, AT2, TP2) merged. Rules: AGENTS.md's overnight push (ship when clearly better and not visibly worse in inspected images; gaps go to [m7-gaps.md](m7-gaps.md)), [P0](m7-p0-claims.md). Labels: **M** measured here, **I** inference.

**Status: shipped as the default on both paths.**

**Problem (MQ1's gap 1).** TR1 keeps a pixel's history by surface identity. A mirror or glossy reflection moves across its surface when the view direction turns, so that history trails it. MQ1 measured it: the chrome sphere's error against a 2×2 supersampled control doubled and its flicker more than doubled, and the reflected arch and gold sphere smeared by about a pixel. The same lag softens the gold sphere's reflections. It is part of the "mushy" glossy look Islam reported; RQ1 owns the rest (ray count, the fade to the cache, demodulation).

**Fix.**
- A program may declare how much of its output moves with the view: `fn view_dependence(s, input) -> ViewDependence { alpha, share }`.
- The shading entry turns that into a per-pixel sensitivity and stores it in the HDR sample's alpha: share ÷ the reflected direction's turn across one pixel.
- TR1 caps that pixel's history window while its view direction turns, so the reflection's share-weighted lag stays within a quarter pixel (`Reconstruction::view_budget = 0.25`).
- Still views keep full history. Programs that declare nothing are unchanged.
- The standard program and the example's photoreal program declare their specular share.

**Skinned characters (MQ1's gap 3).** SK1 rejected every pixel's history scene-wide on each animated frame while GI was on. MQ2 drops that widening; skinned frames clamp continued history to MQ1's 3×3 min/max box, as rigid motion does. SK1's swept volume for direct shadows is kept.

**Found and fixed on the way: non-RT TR1 restarted every pixel on every animated frame.** Since R1n, any frame where a placement moves makes R1n rewrite its software-reflection instance records. That rewrite was counted as a geometry upload, which TR1's restart key hashes. The result was MQ1's original bug, back on non-RT: the example's non-RT flight on main has no anti-aliasing, and MQ1's own GPU test fails on main ("nonrt frame 1: transform edits must not restart"). The rewrite now counts as a transform edit, which TR1 reprojects (one line in `primary_ray_gpu.rs`, marked MQ2). The test passes.

**Effect** (the example's 48-frame flight against a 2×2 supersampled TR1-off control, display RMS codes, lower is better):
- **RT:** chrome sphere 5.68 → 4.19 and its flicker 8.11 → 6.21; gold 4.40 → 3.48, flicker 3.89 → 3.47. Every other population is better or level.
- **Non-RT:** chrome 4.35 → 4.19 against main's every-frame restart. Flicker falls everywhere (geometric edges 12.45 → 11.29). Edges are anti-aliased again where main stair-steps.
- **Static scenes are byte-identical to main** on both paths: RF1–RF10 after 32 TR1 frames, and the quarry Q-walk.

## 1. Changes

### 1.1 The program hook (`src/program.rs`, `shaders/temporal_view.wgsl`)

```wgsl
struct ViewDependence { alpha:f32, share:f32 };   // composed for programs that declare the hook
fn view_dependence(s: Surface, input: SurfaceInput) -> ViewDependence
```

- `share` (0–1) is the part of `shade`'s output that moves with the view like a GGX reflection of `alpha`. Any style can declare it. A view-independent style declares nothing.
- The traced shading entry (RT traced primaries and the non-RT visibility buffer, single- and mixed-program dispatch alike) stores `1 + share / width` in the HDR sample's alpha. `width` is the reflected direction's turn across one pixel: the pixel angle, plus twice the shading normal's turn per pixel (curvature, as R1's ray cone), plus twice `alpha`. A view turn of θ radians then slides the view-dependent part by about θ·(hint − 1) pixels of its own image, share-weighted. Programs that declare nothing store exactly 1, through a stub.
- TR1 reads the alpha and overwrites it with 1. Nothing else reads HDR alpha: post and present read rgb, and TP1's premultiplied blend does not read destination alpha for colour.
- **Standard and example programs:** share = specular directional albedo ÷ (specular + diffuse albedo) under uniform light, from `environment_dfg`, alpha = roughness². Chrome and gold (metals) get share 1. The stone ground gets about 0.08 at roughness 0.9, so its cap never binds. The toon programs declare nothing.
- Film accumulation's seam on the entry's store line and TP1's pre-TP1 identity test were updated to the new text. R0b's frozen reflection probe composes the struct.

### 1.2 TR1's view cap (`shaders/temporal_resolve.wgsl`, `src/temporal_reconstruction.rs`)

- `Tr1Surface` carries the sample's hint (`view`). `tr1_view_turn` measures the angle the reflected view direction turned since the previous frame: the view direction's change from the eye to the surface's current and previous points, plus twice the surface plane's turn (moved placements, skinned vertices). It is exactly 0 for a still camera and a still surface.
- In `tr1_history`, after every other window cap: `window = min(window, 1 + view_budget / (view × turn))`. This is the same rule shape as TR1's bilinear blur budget. Both of a silhouette's surfaces get it.
- `Reconstruction::view_budget` (default 0.25 px, frame word `starts.w`, validated finite and ≥ 0; 0 turns it off).
- **Screened and dropped:** using the spatial start (the 3×3 samples interpolated to the pixel centre) as the capped pixels' observation. It was worse than the raw sample on the chrome sphere (4.74 vs 4.42 at budget 0.5).

### 1.3 Skinned history with GI on

- `prepare_reconstruction` no longer widens SK1's swept shadow volume to the whole scene when GI is enabled.
- Frames where a skinned pose changed (`posed`) set `TR1_BOX`, as MQ1's moved placements do: every continued history is clamped to the plain min/max of this frame's nine 3×3 samples. That holds sky occlusion and bounce changed by the character to this frame's range.
- The swept volume for directional shadows, and SK1's scene-wide fallback for shadowed point and spot lights, are unchanged.

### 1.4 R1n's instance records (`src/primary_ray_gpu.rs`)

When the meshes and slot count are unchanged, R1n's per-frame rewrite of instance records only carries placement transforms. It now counts as `uploads.models`, which TR1 has reprojected since MQ1, not `uploads.geometry`. A full repack (new meshes) still counts as geometry and restarts.

### 1.5 Tooling

- `scripts/m7-mq2-run.py`: MQ1's lease runner that also obeys `[root order]` lines. `--after UNIT[:N],…` waits for those release lines after the latest order. It exits 5 if a newer order appears, waits for the guard's RAM threshold before reading the order, and re-reads the order inside the lease before running anything.
- `scripts/m7-mq2-score.py`: MQ1's scorer plus the puddle decal's ground footprint as a region (`puddle`) and in sheets.

## 2. Claim-avoidance notes (engineering reading, not clearance)

TR1/TR1b/MQ1's readings hold ([m7-tr1.md](m7-tr1.md) §2, [m7-tr1b.md](m7-tr1b.md) §5, [m7-mq1.md](m7-mq1.md) §3). MQ2 adds:

| Claim | MQ2 has | Why it stays outside |
|---|---|---|
| US 10,116,916 / 10,362,289 (colour boxes) | MQ1's plain min/max of all nine 3×3 samples, now also on skinned frames | no mean, variance or deviation; no sample outside the box (P0 §B) |
| US 12,437,371 / pending 2026/0162231 (temporal gradients) | a history window cap | the cap comes from geometry (eye and surface motion) and a program declaration, never from a lighting difference, colour or moments |
| History-clamp family (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) | a shorter window | one history per surface per pixel; no fast or second history |
| US 10,600,167 (W5) | a shading output (the hint) read by TR1, like the hit record | TR1's own per-pixel data is still read only by TR1, next frame |

Not used: variance clipping, SVGF, temporal gradients, own-pixel moments, fast histories, colour-based rejection or weights, reflection-specific reprojection.

## 3. Results (M)

**Protocol.** `examples/game_scene.rs` unchanged: 48 frames at 1280×720, every frame written. RT: traced primaries, Interactive shadows, R1 reflections. Non-RT: visibility buffer, Virtual shadows, R1n reflections. Development build, RTX 4070 SUPER, NVIDIA 617.14. The control is the same flight with TR1 off at 2560×1440, box-filtered in linear light to 1280×720: four pixel samples per output pixel, each with its own reflection rays. It is a lighting-matched control, not a path-traced reference. Regions follow the example's camera path per frame (MQ1's: sphere interiors at 0.9 radius, crate and arch boxes, edges, geometric edges outside the spheres), plus `puddle`, the puddle decal's ground quad outside the spheres. Flicker is the motion-compensated temporal error.

### 3.1 The budget screen (RT, branch before main `4c8ef7d` was merged; one binary, budget via a screen hook since removed)

| RT, budget | full | edges | geo edges | gold | chrome | puddle | flicker gold | flicker chrome | flicker geo edges | grain gold | grain chrome |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 (MQ1 + MP1) | 1.55 | 10.90 | 10.09 | 4.40 | 5.68 | 0.86 | 3.89 | 8.11 | 9.67 | 1.12 | 2.21 |
| 1 | 1.52 | 10.49 | 10.03 | 4.34 | 4.86 | 0.77 | 3.85 | 7.00 | 9.60 | 1.14 | 2.45 |
| 0.5 | 1.48 | 10.18 | 9.97 | 3.96 | 4.42 | 0.76 | 3.63 | 6.45 | 9.51 | 1.27 | 2.62 |
| 0.5, spatial-start observation | 1.49 | 10.31 | 10.00 | 3.96 | 4.74 | 0.75 | 3.56 | 6.74 | 9.55 | – | – |
| **0.25 (shipped)** | **1.45** | **9.92** | **9.90** | **3.48** | **4.19** | 0.76 | **3.47** | **6.21** | **9.44** | 1.60 | 2.79 |
| control | | | | | | | | | | 2.32 | 2.93 |

Grain is the mean |Laplacian| inside the spheres at 0.8 radius, frames 8–47. **MQ1's history over-smoothed the reflections relative to the control** (gold 1.12 vs 2.32, chrome 2.21 vs 2.93), and the view cap restores their detail toward it. Every step down to 0.25 improved every reflective population. 0.125 and 0.0625 were queued in `mq2-a6`, which was released unused (it had been taken against a root order), and later leases went to verifying main's merge (gap 2).

### 3.2 Final code against main `4c8ef7d`

| RT | full | edges | geo edges | gold | chrome | crates | arch | puddle | flicker: full | gold | chrome | crates | puddle | geo edges |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| main | 1.54 | 10.82 | 9.95 | 4.40 | 5.68 | 2.00 | 2.06 | 0.86 | 1.64 | 3.89 | 8.11 | 1.93 | 0.94 | 9.64 |
| **MQ2** | **1.43** | **9.83** | **9.77** | **3.48** | **4.19** | 2.00 | **2.05** | **0.76** | **1.51** | **3.47** | **6.21** | 1.93 | **0.90** | **9.41** |

| Non-RT | full | edges | geo edges | gold | chrome | crates | arch | puddle | flicker: full | gold | chrome | crates | puddle | geo edges |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| main (restarts every animated frame) | 1.84 | **10.87** | **11.36** | **3.47** | 4.35 | **2.59** | 3.58 | **1.13** | 1.91 | 4.45 | 6.60 | 2.65 | **1.28** | 12.45 |
| MQ2 without the cap (history restored only) | 1.91 | 12.22 | 11.94 | 4.40 | 5.68 | 2.68 | 3.32 | 1.21 | 1.86 | 3.89 | 8.11 | 2.53 | 1.35 | 11.48 |
| **MQ2** | **1.83** | 11.40 | 11.79 | 3.48 | **4.19** | 2.67 | **3.32** | 1.15 | **1.76** | **3.47** | **6.21** | **2.53** | 1.30 | **11.29** |

Reading:
- **RT: better or level on every population.** The chrome sphere's error falls 26 % and its flicker 23 %; the gold sphere's error 21 % and flicker 11 %.
- **Inspected.** In `deliver/zoom-rt-chrome-0{24,40}.png` and `deliver/strip-rt-chrome-036.png` (control | main | MQ2), main's reflected arch and gold sphere are thick and smeared on the chrome sphere. MQ2's are thin and sharp, like the control's. In `zoom-rt-gold-*.png` the reflected hero on the gold sphere is more defined (with the control's kind of grain). RQ1's glossy blur remains.
- **Non-RT: TR1 works again in motion.** Main's flight restarts every pixel on every frame. `deliver/zoom-nonrt-crates-040.png` (main | MQ2) shows main's stair-stepped crate, rim and shadow edges, and MQ2's anti-aliased ones. Chrome beats main's no-history image (4.19 vs 4.35), gold ties it (3.48 vs 3.47), and every flicker population falls.
- **The non-RT costs.** Static edge RMS rises 10.87 → 11.40 (geometric 11.36 → 11.79), crates 2.59 → 2.67, puddle 1.13 → 1.15. This is MQ1's known trade on RT: moving edges carry a little history lag against a control that has none. It is not visible as a regression in the inspected crops, which are anti-aliased where main aliases.
- **Without the cap, history alone would double down on lag**: the non-RT no-cap row reproduces MQ1's RT numbers (chrome 5.68, gold 4.40), because R1n's non-RT reflections match R1's.
- **Against RQ1's 16,384-sample path-traced frame 47** (`D:/Coding/renderlab-work/rq1/runs/d1/ref16k-47`, aerial perspective and fog off, unlike these flights), the spheres' error is dominated by the shared lighting and fog difference: the 2×2 controls score 18.81 (gold) and 25.25 (chrome). Main and MQ2 differ by at most 0.2 codes there (RT chrome 25.97 → 25.79; non-RT 25.63 → 25.79). It does not discriminate motion lag, so §3.2's controlled comparison is the evidence.

### 3.3 Static identity, tests and the skinned walk

| Check | Result |
|---|---|
| R0a RF1–RF10, 32 TR1 frames per pose (`gpu_r0a_current`, `RENDERLAB_R1_TR1=1`), main vs MQ2, RT | **13/13 poses byte-identical** (RF10 renders since MP1) |
| The same, non-RT | **13/13 byte-identical** |
| TR1's quarry Q-walk (`gpu_tr1_sequence`, native TR1, frames 8/40/64/96), main vs MQ2 (cap on, the default) | **byte-identical, 4/4 on each path**: the cap never binds on the quarry walk. A second in-process variant with the cap off differs on RT and not on non-RT; it runs after the first in the same process (the known run-order effect, I) |
| `gpu_mq1_moving_placement` (no restart after frame 1, both paths) | **main fails** on non-RT ("transform edits must not restart"); **MQ2 passes** both paths |
| Library suite, final code | **192 passed, 0 failed, 158 ignored; exit 0** (with and without the R1n fix) |
| CPU composition checks (naga validation of every program composition, both heads, the resolve; film seams; `view_dependence_composition`) | pass |
| `cargo fmt --check`; Clippy `--lib --tests` | clean; no warnings in MQ2's files |
| The example flight, RT, final code with and without the R1n fix | byte-identical, 48/48 (R1n's records exist only on non-RT) |
| SK1's fox walk (`examples/skinned-mesh.rs`, fixture, default GI and TR1), main vs MQ2 | non-RT half runs on both; **the RT half fails on main's own binary and on MQ2's** with a wgpu validation error ("BLAS … is newer than the containing TLAS"). Main regression; reported to root |
| `gpu_skinned_streams_history_and_paths`, MQ2 | its oracles pass (deformed vs baked: max byte error 0 on both paths; TR1 continues after frame 0), but it **fails on 8 Vulkan validation errors**, the same BLAS/TLAS ordering as the fox walk (not run with main's binary) |

**Skinned history.** In the non-RT fox fixture the static camera sees a flat, untextured ground, where a restarted pixel and a continued one look alike. So that fixture cannot show the restored history; it shows only where MQ2 differs from main. `deliver/fox-nonrt-024.png` (main | MQ2 | 16× difference): the differences are on the fox itself, whose own history MQ2 now clamps to the box as MQ1 does for moving rigid objects, plus faint contours on the ground. There are no trails at the moving shadow. The RT fixture could not run (above).

### 3.4 Timing (not informative)

GPU render span, frames 8–47, development build, shared GPU. RT main p50 47.6 ms vs MQ2 44.6 ms; non-RT main 51.9 ms vs MQ2 53.0 ms (non-RT with the R1n fix: 107.4 ms, cap off 121.7 ms). The same byte-identical RT flight measured p50 44.6 ms in one lease and 100.5 ms in another, and minima range 8–36 ms: contention dominates. MQ2's added work per pixel is the program's `view_dependence` call (one DFG lookup and a few ALU) in shading, plus two normalizations and a square root in TR1. Not isolated tonight (gap 6).

## 4. Programmability

- **The hook is optional and style-agnostic.** Any program can declare what share of its output is view-dependent and how wide its lobe is, whatever its lighting model. Undeclared means view-independent: unchanged history.
- **Constraint added: the HDR sample's alpha now belongs to the engine** (the hint, consumed by TR1). Programs never wrote alpha; `shade` returns rgb.
- **Constraint: the lobe width is GGX-shaped** (alpha), as R1's `reflection_lobe` is. A non-GGX style gives its nearest width.
- **Constraint: the share is the program's estimate.** The standard program's is "under uniform light". On a dielectric that reflects a much brighter scene than its diffuse lighting, the true view-dependent share is larger, so some lag remains there.
- **MV1 compatibility (root's note).** The cap reads only each surface's current and previous world point and plane normal (`Tr1Surface.point`, `previous_point`, `normal`, `previous_normal`), however they were obtained. Any shared motion-vector path that keeps providing a previous point and plane per surface keeps the cap working; `tr1_view_turn` is the only consumer.

## 5. Gaps

1. **Reflections are still reprojected by their surface, not their reflected image.** The cap removes the lag by giving up history, so a mirror under fast camera motion gets little temporal anti-aliasing of its reflected content: chrome 4.19 codes, against a lag-free AA'd ideal not measured. Virtual-image reprojection would keep the history. It needs a per-pixel reflection hit distance (or virtual depth) from shading, a second channel beside this hint, and curvature handling for convex mirrors.
2. **Budgets below 0.25 are unscreened.** RMS still improved from 0.5 to 0.25 while grain rose toward the control's. 0.125 might be better or noisier.
3. **Non-RT static edge RMS +0.5 codes** against the lag-free control (MQ1's moving-edge history trade, now on non-RT too since TR1 works there again). Not visible in inspected crops.
4. **The standard share is a uniform-light estimate** (§4).
5. **Skinned history evidence is thin.** The RT fox walk and SK1's GPU test fail on main's BLAS/TLAS validation error. The non-RT fixture's flat ground cannot show restored history. Shadowed point and spot lights still widen SK1's rejection scene-wide.
6. **No quiet timing.** The hint's shading cost and the cap's TR1 cost are not isolated.
7. **Upload counters as restart keys are fragile.** A later per-frame buffer rewrite counted as `geometry` would restart TR1 again on that path. `gpu_mq1_moving_placement` now catches it on non-RT; a broader guard would be an explicit "restart reason" rather than byte counters.
8. **RQ1's path-traced frame-47 reference** was used only as a check (§3.2); its fog-off settings make it non-discriminating for these flights.

## 6. Evidence, reproduction and time

**Evidence** (`D:/Coding/renderlab-work/mq2/`; nothing new on E: except schedule lines):
- `deliver/`:
  - `sheet-rt-0{24,40}.png` and `flight-rt.gif` (control | main | MQ2);
  - `zoom-rt-{chrome,gold}-0{24,40}.png` and `strip-rt-chrome-036.png`;
  - `sheet-nonrt-040.png` and `strip-nonrt-crates-036.png` (control | main | no cap | MQ2);
  - `zoom-nonrt-crates-040.png`;
  - `fox-nonrt-024.png`.
- `runs/a/score-rt.json` (budget screen; frames deleted), `runs/b/` (main and MQ2 flights, the 2×2 controls, `score-*.json`, lib log), `runs/c/` (Q-walk `tr1.json`, `rf-identity-*.json`, fox frames, SK1 log), `runs/d/` (R1n-fix flights, MQ1 test logs, lib log), `runs/e/score-nonrt.json`.
- `lease/`: process records of every lease. Raw RF radiance dumps and the 2× control PNGs were deleted after hashing and downsampling.
- `bin/base` (main `4c8ef7d`) and `bin/final2` (the delivered code); the Cargo target is deleted.

**Reproduce:**

```powershell
cargo build --example game_scene --target-dir D:/Coding/renderlab-work/mq2/target
py -3 -B scripts/m7-mq2-run.py LABEL 15 240 --after "UNIT" -- powershell -NoProfile -File D:/Coding/renderlab-work/mq2/b.ps1
py -3 -B scripts/m7-mq2-score.py reference RUN/ref2-raw-rt RUN/ref2-rt --factor 2 --delete-raw
py -3 -B scripts/m7-mq2-score.py score RUN --reference RUN/ref2-rt --variants base,final --path rt --out score-rt.json
py -3 -B scripts/m7-mq2-score.py identity RUN/rf-base-rt RUN/rf-final-rt --out rf-identity-rt.json --delete-raw
```

**Time (UTC, 2026-11-08).**
- Reading, design and first implementation: 20:30–20:50. Builds took 20 s – 2.7 min each (two jobs).
- Waiting for the GPU: about 20:50–21:20, 21:27–21:54 and 22:01–22:06, across root orders, two holds and two foreign-renderer stops (ARA1's `ara1-ara.exe` rendered during `mq2-a5` and `mq2-b2`).
- GPU held in all: about 22 minutes over leases `mq2-a2` (stopped at the root's request after its first step), `a5`, `a6` (taken against a root order and released after 16 s), `b2`, `b3`, `c1` and `d1`.
- Two leases broke root orders: `mq2-a2` and `mq2-a6`. MQ1's runner did not read them, and then a memory wait inside lease acquisition outlived a new order. Both were released at once; `m7-mq2-run.py` now prevents both.
- Merging main (R1n) and finding the R1n restart: 21:40–22:10.
- Scoring, inspection and documentation: interleaved, and 22:20–22:45.

No window, viewer, focus change or input automation. No Unreal launch.
