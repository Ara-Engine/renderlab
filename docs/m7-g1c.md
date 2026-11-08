# M7-G1c: non-RT near field, quarry cavity, cost, rotating map and the quarry gate

Unit G1c continues [M7-G1b](m7-g1b.md) on branch `t3/m7-g1c` (from main `e024146`; main merged at `7971991` and `dc2e3c8`). Contract: [milestone-7.md](milestone-7.md). Claim constraints: [m7-p0-claims.md](m7-p0-claims.md) W1-W3, W8, C1-C5 and C8. Unreal comparators: [M7-U0b](m7-u0b.md) on main. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

The cache stays **off by default** (`IndirectSettings::enabled`), as at G1 and G1b.

## Status

| Item | Result | Verdict |
|---|---|---|
| 1. Non-RT near field | A cooked 1 cm far-field cut (new `m7-near` cook, RNF v1, own cache namespace): **38.8 MB** on the GPU against 401 MB of full detail. Quarry quality equals full detail within 0.2 points (18.93% against 18.74% / 18.44% RT). Per-vertex masks are built and opt-in, but never beat the cut. | **Pass on quality and memory.** The cut cost is bounded in frames, not in time on non-RT (8-13 ms a frame for four frames): **miss** |
| 2. Quarry cavity | The inside test rejected floors under open scan sheets. A side-aware enclosure test, a second candidate per line and split cells with one finer level fix it: quarry **35.8% → 18.4%** (RT), with SV unchanged. | **Pass** |
| 3. Non-RT cost | SV camera increment **+0.53-0.62 ms** against 0.46, 0.04-0.09 ms over G1b's code (item 2's per-cell probe work). Three reduction attempts failed. RT reads +0.42-0.50 against 0.45. On the quarry the increments are +1.8-2.4 ms (non-RT) and +1.3-1.6 ms (RT). | **Miss** (RT narrowly) |
| 4. Rotating map | Fixed slot-group re-integration (1/8 a frame, W2-safe), scan of that group only, and environment-only frames: **RT +1.70-1.85 → +0.41 ms; non-RT +1.63-1.66 → +0.38-0.55 ms**. Steps still equal a fresh cache; the image settles in 7 frames after a stop. | **Pass** |
| 5. Quarry gate against Unreal | **SV sky visibility:** beats Lumen HW and SW Epic on every population, static and in motion, both paths. **Quarry, G1's estimand under the real RLE2 map:** 30.9% / 31.2% (RT / non-RT). A map quadrature bias was found and partly fixed (bias −11% → −7%); no Lumen comparator exists for it. **Quarry whole image against Lumen's floors:** miss on both paths, mostly upstream: the cache-off image already misses, and the standard material reads 11-27% dark on open ground. | **SV pass; quarry miss** (attribution in 5.4) |

**New findings.**
- **Quarry cost.** The cache costs 3-4x more on the quarry than on SV: +1.3-1.6 ms (RT) and +1.8-2.4 ms (non-RT) camera-moving, over every Lumen budget reading (section 3.2). Item 2's three-level split refinement had added ~3 ms of probe a frame on the quarry; one level keeps its quality at a third of the cost (section 5.5).
- **Map SH bias.** The cache read 11% dark on open ground under a sun-dominated map, from its 64-direction quadrature. A per-cell open-sky scale against the prepared irradiance fixes most of it at no storage or lookup cost (section 5.3).

**Gates (M, x40, final code):** functional, determinism, non-caller, caster roll, environment steps and the SV pan pass on both paths. The lib suite passes (137); cook and asset tests pass (96); fmt and clippy are clean in G1c's files. **Memory:** 122.3 MiB on SV and 159.3 MiB on the non-RT quarry, under Lumen's totals.

## 1. Non-RT near field (item 1)

### 1.1 What was built

**Cook (`renderlab-cook m7-near`, `crates/cook/src/near_field.rs`).** A new versioned cook with its own cache namespace, `near-field-v1/rla<S>-rvg<S>/{keys,packages}`, beside R0c's BVH v4 and U1b's moments v5/v6. Its asset format is RNF v1 (`crates/asset/src/near_field.rs`).
- **Far field:** the cluster DAG's legal cut at `lod_error` (metres), rebuilt per mesh as an R0c-layout BVH4 with eight-triangle leaves.
- **Optional near-field masks:** per vertex and side, `bins`² octahedral bits of whether a cook ray from the vertex is blocked within `reach` (exact any-hit BVH2 over the full-detail placed scene). The runtime's software rays consult them for their first `reach` of travel. Masks are additive and vote all-agree across the hit triangle's three vertices.
- **Settings:** `lod_error`, `reach`, `skip` (< reach), `bins` ∈ {0, 8, 16, 32}; a 160 MB cap (`Limits::max_bytes`).
- **Runtime:** `Renderer::set_indirect_near_field(mesh, field, index)` uploads a mesh's cut BVH and, for placements with masks, the masks; instance word 23 carries the mask base. Hardware rays ignore it.

**The default is the cut alone** (`lod_error` 1 cm, `bins` 0): masks never beat it (section 1.3).

| Cook (M, debug cook binary, 8 threads) | Package | Triangles | Wall | Peak private |
|---|---:|---:|---:|---:|
| Cut 1 cm, no masks (default) | 39.47 MB | 1,172,108 | 7.4 s (6.7 s in the cook) | 179 MB |
| Cut 1 cm, 16-bin masks, 3 cm reach | 85.09 MB | 1,172,108 + 45.6 MB masks | 48.2 s | 962 MB |
| Full-detail scene (G1b's non-RT input, runtime build) | 401 MB on the GPU | 13,034,903 | 8-19 s at load | — |

Cook tests (`crates/cook/tests/near_field.rs`): masks follow occluders under rotated, scaled placements; the cut alone has no masks; the cook is deterministic across thread counts and cached. Asset tests cover the BVH4 build, bin round trips, and encode/validate/corruption rejection.

### 1.2 Quarry results (G1's estimand, M)

The reference tracer's depth-2 image (camera, surface, sky) at quarry poses 0 and 1, 1,024 spp, albedo 0.5 Lambert, constant sky 16,384 cd/m², scored with G1's floored relative-luminance metric over covered non-edge pixels.

Final code `g1c-m` (x40), poses 0 / 1:

| Path and geometry | GPU geometry | Against the reference | Dark pixels | Bias | Against RT's image |
|---|---:|---:|---:|---:|---:|
| RT (hardware rays) | — | **18.44 / 17.61%** | 29.68 / 28.13% | −3.8% | — |
| Non-RT, full-detail software BLAS (G1b's input) | 401 MB | 18.74 / 17.61% | 30.23 / 28.13% | −3.8% | 3.60 / 1.20% |
| **Non-RT, cooked 1 cm cut (default)** | **38.8 MB** | **18.93 / 17.73%** | 31.00 / 28.82% | −2.2% | 28.27 / 26.47% |
| G1b, RT | — | 35.8% (pose 0) | 61% | | |

**Other cuts (M, earlier code `g1c-c`, same metric).** A 0.5 cm cut is 84 MB and reads 19.29% / 17.53%; 0.7 cm is 64 MB. At 0.5 cm, 16-bin masks exceed the 160 MB cap.

### 1.3 The "28% gap to RT" is decorrelation, not error

Against RT's image, the cut reads 27-29% RMS (pose 0/1), while full-detail software rays read 1-4%. Against the reference, all three are within 0.5 points of each other. The cut's surfaces are up to 1 cm from the scan's, so cells take different query points and different fixed rays: a per-pixel decorrelation of two equally good estimates. Its bias is smaller than RT's (−2.2% against −3.8%).

**Masks (M).** They were meant to restore near-surface occlusion that the cut loses. Every mask variant tried read the same as the cut or worse against the reference, at 2.1-2.8x its memory: 8 and 16 bins; near distances of 3-25 cm; skips of 0.5-2 cm; 0.5, 0.7 and 1 cm cuts; majority and all-agree votes. The best, 16 bins at 3 cm, read 18.90% at 85 MB against the cut's 18.90% at the time. The cut already places surfaces within 1 cm of the scan, and the cache's cells are 4 px wide, so the masks' extra centimetre of occlusion is below what the cells resolve. They stay an opt-in cook setting (`bins` > 0), off by default.

### 1.4 Cut cost on the quarry (M, warm, functional)

A reset at quarry pose 0, then nine still frames, with the default settings (refill S = 4 on software rays, none on hardware), G1's gate program under the RLE2 map. Three warm cuts; medians; per-pass timestamps on. Frame 9 is idle. Functional lease x40, not a quiet window.

| Frame after the reset | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 (idle) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| RT frame, ms | 5.94 | 3.75 | 2.61 | 2.17 | 2.04 | 2.25 | 1.73 | 2.21 | 0.66 |
| RT setup + rays, ms | 2.47 | 1.19 | 0.36 | 0.04 | 0.03 | 0.03 | 0.02 | 0.02 | — |
| Non-RT frame, ms | 16.07 | 17.26 | 14.34 | 13.17 | 6.86 | 2.63 | 2.92 | 2.59 | 1.08 |
| Non-RT setup + rays, ms | 10.84 | 12.84 | 9.12 | 8.06 | 3.23 | 0.41 | 0.31 | 0.02 | — |

- **It is bounded in frames:** ray work falls below 0.05 ms by frame 4 on RT and frame 8 on non-RT, within the 8-frame settle rule, and the cache idles at frame 9. A few hundred orphan-parent and refinement cells are still created on frames 2-8, so the image is not exactly a settled cache's before then.
- **It is not bounded in time on non-RT.** A quarry cut creates about 90,000 cells. Their software line walks and rays cost 8-13 ms a frame for four frames. G1b's SV cut was 3.2 ms. The refill spreads rays only; setup (the five probe lines per new cell, through the cut BVH) is not spread. **Miss**, section 9.
- **The probe holds about 1 ms on still quarry frames** until the cache idles (section 5.5).

## 2. Quarry cavity (item 2)

**Cause (M).** G1b's remaining error was one cavity under a slab, holding 76% of the squared error. The probe's inside test rejected the floor beneath open scan sheets: a scan sheet has no closed back, so a ray from the floor met the sheet's back face and the floor point read as "inside a solid". The cell then took a point on the sheet above, and the floor read the sheet's sky.

**Fix, within P0's constraints (fixed query point, no voting, no jitter).**
- **Side-aware inside test.** A candidate is inside only when the ray along its normal hits a back face within one cell edge, or, when the back face is farther, all four lateral rays at 80° hit back faces too (an enclosure). One ray-query call site serves all five rays (more inlined sites raised setup's register count).
- **Second candidate per line.** When a probe line's first qualifying hit is inside, its next hit past an open gap (> 1% of an edge) is tried: the floor beneath the slab.
- **Split cells.** A line that meets two surface layers more than an eighth of an edge apart marks its cell split (hot record word 15, bit 16). Shading then reads one finer level there (`GI_REFINE`), and the probe requests it.

**Result (M).** Quarry RMS against the reference: G1b 35.8% → **18.44% / 17.61%** (RT, poses 0 / 1), 18.74% / 17.61% (non-RT, full detail) and 18.93% / 17.73% (non-RT, the cooked cut); dark-pixel RMS 61% → 29.7%. SV is unchanged (section 6).

**Split refinement depth (M).** Three finer levels cost about 3 ms a frame in the probe on the quarry, for 0.09 points of RMS over one level (section 5.5). One level is the default. Against no refinement it is worth about 0.6 points (map sky, 31.42% → 30.85%) and 0.5-1 point under a constant sky.

## 3. Non-RT cost (item 3)

### 3.1 Quiet windows `q10-1` and `q10-2` (M)

**Conditions.**
- 2026-11-08, `q10-1` 08:28:11-08:30:37Z and `q10-2` 08:40:22-08:42:38Z.
- Each window opened after 120 s with no foreign `stillkit` compiler; stillkit was sampled every 3 s.
- stillkit restarted inside both windows, so each step is classified by the samples inside its own interval:
  - `q10-1`: samples only at 08:29:36-39, inside the RT quarry-motion step. Every SV, map and non-RT quarry step is **clean**.
  - `q10-2`: samples at 08:40:57-41:00 (inside `t-m2`) and from 08:41:48 (both quarry steps). `t-m`, `t-g1b`, `md-m` and `md-f` are **clean**.
- **The RT quarry-motion row is contended in both windows** and is reported as such.

**Binaries.** `g1c-n` (the pushed code) beside G1b's shaders on the G1c host (`g1c-e` + `variants/g1b`, as in x28) and the pre-item-4 code (`g1c-f`). SV overview pose, 1920x1080, Lambert gate program, no per-pass timestamps, two visits each.

**SV camera-moving increment over the cache-off frame of the same visit (ms, clean steps only):**

| Path | G1b code, same windows | G1c final | Root's stricter budget |
|---|---:|---:|---:|
| RT | +0.40-0.48 | +0.42-0.50 | 0.45: **missed narrowly**, over on five of six visits (by up to 0.05) |
| Non-RT | +0.48-0.54 | **+0.53-0.62** | 0.46: **missed by 0.07-0.16** |

Other SV phases (clean steps of both windows; G1c against G1b, ms over cache-off):
- **Caster:** RT +1.18-1.39 against +1.19-1.40; non-RT +1.77-2.13 against +1.63-1.93.
- **Cut, no refill:** RT +2.27-2.32 against +2.21-2.29; non-RT +4.9-5.7 against +4.1-4.6.
- **Cut, refill S = 4 (the non-RT default):** non-RT +3.27-3.68.
- **Still:** +0.15-0.21 on both codes and paths. The exception is `q10-1`'s first non-RT process (+0.39 / 0.73), a cold start.

**Why non-RT got slower than G1b.** Item 2's probe (five lanes walking every line, side-aware inside tests, second candidates, split detection) costs more per new cell on software rays: setup on SV camera frames rose from 55-61 µs (G1b) to 74-84 µs. Caster and cut frames, which create many cells, show it most. Item 2 halved the quarry error, so the cost is kept and recorded.

**Attempts at item 3 (M, functional lease x28, same lease as a G1b row):**

| Attempt | Non-RT camera increment | Verdict |
|---|---:|---|
| G1c at the time | +0.523 / 0.565 | — |
| Probe tile table without per-pixel priorities (one atomic per insert fewer) | +0.520 / 0.561 | No gain |
| Lazy setup resolution (line 0's candidate first; lines 1-4 only when it gives no point) | +0.565 / 0.605 | Worse |
| G1b's shaders on the same host | +0.520 / 0.524 | Reference |
| Two pixels per probe lane (32x16 tiles) | probe pass unchanged (profiled) | No gain; reverted |
| One ray-query call site in setup (inside tests and lines through one loop) | setup 110-116 → 74-84 µs | **Kept**: recovered a setup regression from inlined ray-query sites |

**Profile (Nsight, earlier G1c code).** The probe (about 170 µs on SV) waits on the latency of its table finds; shading with the cache is about 0.34 ms.

### 3.2 On the quarry (M)

Section 5.5 has the probe cost. The camera-moving increment on the quarry, Lumen's bench, is 3-4x SV's:

| Quarry, pose 0 moving 1 cm a frame (24 frames, two visits, p50) | Cache off | Cache on | Increment | Conditions |
|---|---:|---:|---:|---|
| RT, gate program, map sky | 0.567 ms | 1.87-1.96 ms | **+1.30-1.39** | functional x40 |
| Non-RT, gate program, map sky | 1.02 ms | 3.08-3.40 ms | **+2.06-2.38** | functional x40 |
| RT, whole image (standard program, sun shadows) | 0.88-1.21 ms | 2.33-2.58 ms | +1.37-1.63 | contended (`q10-1`, `q10-2`), x40 +1.55-1.62 |
| Non-RT, whole image | 2.35-2.53 ms | 4.28-4.32 ms | **+1.77-1.97** | **clean (`q10-1`)**; x40 +1.69-1.72 |

**Against Lumen's L** (quarry, static; [m7-g1.md](m7-g1.md) section 4):

| Reading of L | RT budget (0.3 L_hw) | G1c RT | Non-RT line (L_sw / 2) | G1c non-RT |
|---|---:|---:|---:|---:|
| Lumen plus TSR (design 8.1) | 0.88 ms | +1.30-1.63: **miss** (1.5-1.9x) | 1.17 ms | +1.77-2.38: **miss** (1.5-2.0x) |
| Lumen over TSR | 0.45 ms | **miss** (2.9-3.6x) | 0.46 ms | **miss** (3.8-5.2x) |
| Lumen's GI pass | 0.37 ms | **miss** | 0.61 ms | **miss** |

Lumen's numbers are static frames and G1c's are camera-moving, so this flatters Lumen slightly (I). The verdict holds at every reading.

## 4. Rotating environment map (item 4)

**Cause (M, G1b).** A continuously rotating map re-integrated every cell every frame: about 1.0 ms of a 1.9-2.0 ms frame on both paths.

**Change.**
- **A single step after a still frame re-integrates every cell**, so the next frame equals a fresh cache (G1b's behaviour).
- **A map changing on consecutive frames re-integrates one fixed slot group a frame:** slots with `(slot + phase) % 8 == 0`, the phase advancing by one each frame (`ENVIRONMENT_PERIOD` = 8). This is a fixed rotation by slot index, never a ranking (W2). It continues for eight frames after the last change, so every cell is exact again within the 8-frame settle limit.
- **The scan visits only that group** (strided by 8), not every slot.
- **Environment-only frames** (the view and scene unchanged for `IDLE_AFTER` frames, only the map changing) run only the scan and integrate passes, and do not advance the cache frame, so seen stamps don't age while nothing is probed.

**Quality (M, x40, both paths, SV).**
- **Steps:** each of six 15° yaws after a still frame equals a fresh cache on the next frame (0 pixels differ; 90,447 integrates, no rays).
- **Lag while rotating** (48 frames at 15° a frame, against a fresh cache at the same yaw): relative RMS **1.20%**, maximum 23.2% (the newest phase's cells are one to seven frames old).
- **Settle:** after the rotation stops, the image equals a fresh cache after **7 still frames**, within the 8-frame rule.

**Cost (M, quiet windows `q10-1` and `q10-2`, clean steps; SV, a map rotating every frame; frame p50 in ms):**

| Path | Cache off | Before (`g1c-f`, G1b's scheme) | **Final** | Map still, cache on |
|---|---:|---:|---:|---:|
| RT | 0.408-0.446 | 2.11-2.26 (+1.70-1.85) | **0.854-0.855 (+0.41)** | 0.627-0.630 |
| Non-RT | 0.504-0.514 | 2.14-2.17 (+1.63-1.66) | **0.88-1.05 (+0.38-0.55)** | 0.677-0.683 |

- A rotating map now costs about a quarter of G1b's increment.
- Profiled (functional x33): integrate 133-145 µs and scan 16 µs a frame, with no probe, setup or rays once the view idles.
- The non-RT final row varied between windows (0.881 and 1.05 ms); both are reported.

## 5. Quarry gate against Unreal (item 5)

### 5.1 What Unreal offers to compare against

- **Sky visibility, SV only.** U0b's G1 comparator (`/Game/M7U0b/SVSky/Bench`, `ue-svsky-package`): Lumen sky-only, constant sky 4,000 cd/m², albedo 0.5, radiosity 0, scored with G1's metric against regenerated 2,048-spp depth-2 references. Section 5.2.
- **The quarry: full radiance only.** U0b's 19 static, 4 Q-walk and 15 alternation floors score Lumen's whole image (materials, sun, shadows, sky, bounce, specular, TSR) in F0 codes against 4,096-spp full-radiance references. No Lumen sky-visibility capture of the quarry exists. A matched one would need a new cooked package with albedo-0.5 material overrides; none was made (section 9).
- So the quarry gate has two parts: G1's estimand under the real RLE2 map against the reference tracer (5.3), and Renderlab's whole quarry image against Lumen's full-radiance floors (5.4).

### 5.2 SV against Lumen sky visibility (M, `g1c-m`, x40; relative RMS %, overview / gaps)

| Population | G1c RT | G1c non-RT | Lumen HW Epic | Lumen SW Epic, mesh SDF |
|---|---:|---:|---:|---:|
| frame | **1.95 / 2.75** | **1.95 / 2.75** | 10.71 / 11.16 | 38.91 / 87.18 |
| open | 1.77 / 2.02 | 1.77 / 2.02 | 7.95 / 6.27 | 34.14 / 30.42 |
| crease-90 | 2.71 / 2.74 | 2.71 / 2.74 | 18.51 / 5.40 | 33.89 / 61.07 |
| wedge-30 | 3.19 / — | 3.19 / — | 24.83 / — | 45.77 / — |
| underside | 2.62 / 3.43 | 2.62 / 3.43 | 15.71 / 11.07 | 34.38 / 88.98 |
| gaps | 3.73 / 4.37 | 3.73 / 4.37 | 30.52 / 20.81 | 111.14 / 175.94 |
| contact | 1.87 / — | 1.86 / — | 14.40 / — | 32.93 / — |
| ground near crease-90 | 1.87 / 2.17 | 1.87 / 2.17 | 6.08 / 7.43 | 10.00 / 29.63 |
| ground near wedge-30 | 3.62 / 1.25 | 3.62 / 1.25 | 17.14 / 6.24 | 93.41 / 6.93 |
| ground near underside | 2.22 / 2.24 | 2.22 / 2.24 | 10.78 / 9.80 | 50.48 / 69.62 |
| ground near gaps | 2.03 / 2.22 | 2.03 / 2.22 | 6.19 / 5.67 | 25.13 / 29.74 |
| ground near contact | 1.29 / 1.54 | 1.29 / 1.54 | 4.66 / 4.84 | 5.08 / 6.02 |
| crevice pixels (frame) | 4.21 / 3.93 | 4.21 / 3.93 | 23.78 / 16.10 | 96.93 / 137.23 |
| p99 (frame) | 6.38 / 8.99 | 6.38 / 8.99 | 31.51 / 41.44 | 123.43 / 172.99 |
| over-occluded open pixels | **0 / 0** | **0 / 0** | 407,581 / 226,107 | 112,027 / 139,633 |
| edges | **15.26 / 17.76** | **15.26 / 17.76** | 26.85 / 18.39 | 61.13 / 91.94 |
| first frame after a cut | 1.95 / 2.75 (no refill) | 6.82 / 9.98 (refill S = 4) | 10.59 / 9.51 | 49.56 / 101.05 |
| pan residual change, mean / max | **6.14 / 6.45** | **6.14 / 6.45** (refill: 8.40 / 15.85) | 20.18 / 45.84 | 166.17 / 191.52 |

Lumen columns are U0b's (`docs/m7-u0b.md`, G1's SV comparator).
- **Every lighting population matches G1b's to 0.01 points.** The determinism hashes match the earlier G1c builds (`db3f7832535ddb9f` RT, `ba208a2b204c6a04` non-RT).
- **The pan residual improves** from G1b's 8.55 / 10.43 to 6.14 / 6.45 (cold start; the warm row is identical).
- **Edges now beat Lumen HW on both poses** (15.26 / 17.76 against 26.85 / 18.39). G1b recorded 45.83 / 34.42 against its own references. The change appeared with the merged reference tracer and its regenerated SV reference; this unit did not trace it (I). The edge population still scores single-sample anti-aliasing (TR1's measure), so no G1c claim rests on it.
- **Verdict (M):** G1c beats Lumen HW Epic and Lumen SW Epic (mesh SDF) on every SV population, static and in motion, on both paths.

### 5.3 The quarry under its real RLE2 map: G1's estimand (M)

**Reference.** The reference tracer now loads the RLE2 map (U0b). `quarry_reference` renders depth 2 (camera, surface, sky), albedo 0.5 Lambert on every placement, the registered `sun128.rle2` as the only light, 2,048 spp. The test passes `RENDERLAB_M7_G1_QUARRY_SKY=map`. The cache runs under the same map and program.

| Code | RT, poses 0 / 1 | Bias | Non-RT cut | Bias |
|---|---:|---:|---:|---:|
| Before the open-sky scale (x35) | 30.43 / 30.50% | −11.3% | 31.06 / 30.75% | −9.8% |
| **Final `g1c-m` (x40)** | **30.85 / 30.82%** | **−7.2%** | **31.21 / 30.91%** | **−5.1%** |
| Same code, constant sky (5.2's G1 estimand on the quarry) | 18.44 / 17.61% | −3.8% | 18.93 / 17.73% | −2.2% |

**The map is harder than a constant sky** because its radiance is concentrated: on open, up-facing ground the reference's sky term is about 98% of the full image, and the analytic sun about 2%.

**A map quadrature bias, found and partly fixed.**
- Cache-off (the prepared diffuse irradiance) against the reference on open, up-facing ground: median **0.99**. The environment's own response is right.
- Cache-on there read **0.866**: 13% dark on ground that is essentially unoccluded. The spread was narrow (5th-95th percentile 0.62-0.93, equal per channel), so it was not aliasing noise.
- **Diagnostic, every direction forced open:** still 0.891. So it was not visibility. It was the 64-direction hemisphere quadrature of the map's prefiltered radiance, reconstructed in L2 SH.
- **Prefilter roughness screen** (default 0.35): 0 aliases (median 0.83, spread 0.49-1.29); 0.7 and 1.0 lose more energy (0.776, 0.734), because the hemisphere-only directions and the clamped cosine both favour the bright upper sky.
- **Fix: the open-sky scale.** At integration (`gi_store_sh`), each map cell's SH is scaled per channel so that its unoccluded L2 value at the cell normal equals the prepared diffuse irradiance there. The unoccluded value is one extra accumulator: every direction adds its radiance times the band-2 clamped-cosine kernel 3/32 + z/2 + 15z²/32 at z = d·n. Storage and lookup are unchanged.
- Under a constant sky the scale would be 1 to within 0.002% (the 64 directions integrate that kernel to π), so it is skipped there; SV's image hashes are unchanged.
- **Effect:** open-ground median 0.866 → **0.927**, bias −11.3% → −7.2% (RT) and −9.8% → −5.1% (non-RT). Frame RMS is unchanged within 0.4 points, because dark and occluded regions dominate it (dark-pixel RMS 44.8%). Roughness 0 or 0.7 with the scale read 32.7% and 34.5%, so 0.35 stays.

### 5.4 The whole quarry image against Lumen's floors (M)

**Capture.** `RENDERLAB_M7_G1_QUARRY_MODE=full` renders U0b's protocol:
- the quarry's own materials and textures;
- the sun from `quarry-hdr-lighting.json` with a 0.04 rad disk and the 4/12/40 m splits;
- the RLE2 map bound with a zero constant (a black primary background, as U0b's);
- exposure −14;
- each path's shipping shadows: traced Interactive on RT, Virtual (Surface lookup, selected at creation) on non-RT, through FIX1's 11-buffer limits on main;
- cache off, then on (8 settle frames).

`fullscore.py` converts the HDR to F0 codes with U0b's own `codes` transform and statistics, against the 4,096-spp `quarry-p0.npz`, with its registered populations. The raw RMS columns compare directly with U0b's static floor table.

Final code `g1c-m` (x40), pose p0 against U0b's static floors (raw RMS, F0 codes at −14 / −10 stops). Both paths use the default near field (non-RT: the 1 cm cut):

| Population | RT off | RT on | Non-RT off | Non-RT on | Depth-2 reference | Lumen HW Epic | Lumen SW Epic |
|---|---:|---:|---:|---:|---:|---:|---:|
| all-covered | 11.63 / 18.40 | 14.22 / 22.03 | 11.64 / 18.43 | 13.35 / 21.07 | 1.90 / 3.40 | **8.41 / 11.65** | **9.32 / 13.99** |
| lit | 8.89 / 12.97 | 12.36 / 17.28 | 8.90 / 13.01 | 11.27 / 16.08 | 1.17 / 1.59 | 7.39 / 9.40 | 7.49 / 9.78 |
| solid-lit | 8.77 / 12.78 | 12.20 / 17.00 | 8.78 / 12.82 | 11.10 / 15.80 | 1.17 / 1.56 | 7.35 / 9.33 | 7.43 / 9.66 |
| occluded | 28.26 / 48.75 | 20.71 / 42.08 | 28.27 / 48.78 | **20.58 / 41.28** | 4.61 / 9.88 | 14.53 / 26.40 | 22.38 / 41.75 |
| solid-occluded | 27.69 / 47.99 | 19.86 / 40.40 | 27.70 / 48.01 | **19.83 / 39.70** | 4.58 / 9.82 | 14.25 / 25.84 | 22.18 / 41.36 |
| silhouette | 27.02 / 48.06 | 20.20 / 42.00 | 27.03 / 48.10 | **19.98 / 41.63** | 1.96 / 4.33 | 13.21 / 26.80 | 20.72 / 40.36 |
| contact | 19.02 / 34.08 | 21.90 / 42.25 | 19.03 / 34.12 | 21.85 / 42.01 | 4.24 / 9.47 | 13.42 / 22.17 | 14.02 / 23.59 |
| backlit | 18.46 / 36.25 | 22.79 / 46.33 | 18.45 / 36.27 | 22.25 / 45.23 | 5.59 / 11.62 | 9.93 / 17.08 | 11.40 / 20.62 |
| faint | 13.68 / 20.42 | 19.31 / 28.41 | 13.69 / 20.43 | 18.42 / 27.63 | 2.51 / 4.40 | 9.90 / 12.29 | 10.39 / 13.65 |
| partial | 15.16 / 24.82 | 19.65 / 32.50 | 15.17 / 24.85 | 19.09 / 31.87 | 3.13 / 5.71 | 10.59 / 14.66 | 11.14 / 16.14 |
| multi-surface | 20.10 / 32.00 | 24.79 / 42.70 | 20.11 / 32.02 | 24.23 / 41.85 | 2.84 / 5.26 | 11.95 / 18.55 | 14.24 / 23.67 |
| narrow | 16.66 / 29.82 | 20.72 / 38.31 | 16.72 / 29.98 | 20.70 / 37.91 | 3.02 / 5.91 | 12.09 / 17.39 | 12.66 / 18.87 |
| edge | 22.60 / 50.57 | 24.32 / 53.82 | 22.55 / 50.46 | 23.81 / 53.29 | 2.69 / 5.19 | 11.28 / 20.20 | 13.83 / 25.54 |
| background-centre | 1.28 / 3.84 | 1.28 / 3.84 | 1.28 / 3.83 | 1.28 / 3.83 | 0.04 / 0.13 | 0.33 / 1.01 | 0.33 / 1.03 |
| image | 4.90 / 8.30 | 5.93 / 9.66 | 4.90 / 8.30 | 5.58 / 9.30 | 0.78 / 1.40 | 3.45 / 4.85 | 3.82 / 5.79 |

Pose p1 reads the same within 0.05 codes (all-covered RT on 14.26, non-RT on 13.36). The other populations (isolated, single-surface, solid, wide, wide6) are in `analysis/x40/full-score.json` and follow the same pattern.

**Verdict (M): miss on both paths.** Renderlab's whole quarry image is worse than Lumen's static floor on every population, with three exceptions on non-RT against Lumen SW Epic: occluded and solid-occluded at both exposures (20.58 / 19.83 against 22.38 / 22.18 at −14), and silhouette at −14 only. No population beats Lumen HW Epic.

**Attribution (M).**
- **Bounce is not the gap.** A depth-2 reference (no bounce, same materials) is only 1.90 codes from the full reference over covered pixels.
- **The image without the cache already misses Lumen**, before any cache error: 11.63 against Lumen HW Epic's 8.41. On open, up-facing, sunlit ground the unoccluded-sky image is 0.73-0.89x the reference, varying by placement. Under identical albedo-0.5 Lambert materials (5.3) the same comparison reads 0.99, so the environment response is right and the gap is in the standard material's response or texture sampling. Mips are built in linear space and TINT is 1, so those are not the cause. This is outside G1c's files and is recorded for the material/standard-program owner (section 9).
- **The cache's own effect** on the whole image (RT, −14): occluded pixels 28.26 → 20.71 and silhouettes 27.02 → 20.20 improve. Lit pixels 8.89 → 12.36 worsen: the cache correctly removes sky light from an image that is already too dark, and adds its remaining −7% open-ground bias (5.3). Non-RT's cut reads slightly better than RT (13.35 against 14.22 all-covered) because its bias is smaller (−5% against −7%).
- **Edges** (22.6-24.3 against Lumen's 11.3-13.8) are single-sample anti-aliasing against a pixel-integrated reference, without temporal reconstruction: TR1's measure.

### 5.5 Quarry probe cost and split refinement (M)

**Finding.** On the quarry, every non-idle frame paid a probe of about 3 ms (RT, x35, still frames after a cut), against about 0.18 ms on SV. A cut frame's probe was 0.6 ms, before any cell was traced and marked split. Item 2's split refinement made the difference. Pixels whose cells are split walk to finer levels each round, through shared-memory hash probes of a nearly full 512-entry tile table, then insert the finer cells. SV has few split cells, so its probe never showed it.

**Screen (M, x38-x39, RT, map sky, three warm cuts each; rows marked * ran beside foreign GPU load, so compare within a row's own idle frame).**

| Refinement | RMS, poses 0 / 1 | Still-frame probe | Idle frame |
|---|---:|---:|---:|
| Three finer levels, every bilinear corner (as built for item 2) | 30.76 / 30.78% | about 3 ms (x35, uncontended); 5-9 ms * | 0.65 ms; 1.0-1.3 * |
| **One finer level (final)** | **30.85 / 30.82%** | **0.91-1.44 ms** | 0.65 ms |
| One level, the pixel's own cell only | 31.17 / 30.99% | 0.64-0.83 ms | 0.67 ms |
| Three levels, own cell only | 31.06 / 30.82% | 1.05-1.44 ms | 0.68 ms |
| None | 31.42 / 31.41% | about 0.6 ms (cut-frame probe) | 0.93-1.6 * |
| Three levels, no orphan-parent rounds | 30.77 / 31.03% | 2.5-3.6 ms | 0.67 ms |

**Decision.** One finer level (`GI_REFINE` 1) keeps all but 0.09 points of three levels' quality, at about a third of the probe cost. The remaining probe on still quarry frames is about 1 ms, five times SV's. The quarry's scan normals spread each tile over several bins, so tiles list hundreds of unique cells and orphans' parents.

**The quarry's camera-moving increment** with the final code is +1.3-1.6 ms (RT) and +1.8-2.4 ms (non-RT), against +0.4-0.6 ms on SV (section 3.2). The probe's ~1 ms on scans is most of RT's; non-RT adds software setup and rays for new cells.

## 6. Gates (M, final code `g1c-m`)

Functional lease x40, both paths, final binary `g1c-m` (main `dc2e3c8` merged):

| Gate | Result |
|---|---|
| Functional (`gates-a`: quality, gates, caster; 10 configurations) | **Pass**, no failures |
| Determinism (`gates-b`: repeat, serial and table hashes) | **Pass**: image and table repeats equal, serial equal, on both paths |
| Non-caller | **Pass**: no cache frames added, toon image equal, both paths |
| Caster roll | Rolled equals fresh, 0 pixels differ, both paths |
| Environment steps (six 15° yaws) | Every step equals a fresh cache on the next frame, both paths |
| Motion (9-frame SV pan) | 6.14 / 6.45% mean / max, both paths |
| Quarry, constant sky / map / full radiance, both paths | Ran; scores in 1.2, 5.3, 5.4 |
| `cargo test -p renderlab --lib` (inside the lease) | **137 passed, 0 failed**, exit 0 |
| Cook and asset tests, fmt, clippy | **Pass**: 15 cook and asset test binaries, 96 passed and 0 failed (CPU guard, light mode); `rustfmt --check` clean on every changed file; `cargo clippy` clean in G1c's files (renderlab `--lib --tests`, renderlab-asset and renderlab-cook `--all-targets`). The renderlab lib suite passed again on the pushed code (`g1c-n`, inside `q10-1`: 137 passed) |

## 7. Memory (M)

| Item | G1b | G1c |
|---|---:|---:|
| Table (2^19 slots: 16-word hot, 28-word cold records) | — | 92,274,944 B (88.0 MiB) |
| Work buffers | — | 35,913,792 B (34.3 MiB) |
| **Cache on SV** | 114.4 MiB | **122.3 MiB** (the cold record gains 4 words for the near field) |
| Non-RT quarry geometry | 382.4 MiB (full-detail software BLAS) | **37.0 MiB** (cooked 1 cm cut); 81.2 MiB with 16-bin masks |
| **Non-RT quarry total** | 496.8 MiB | **159.3 MiB** |

**Against Unreal** (buffer sums, as G1b's section 7):
- RT: 2,641 + 122.3 = 2,763 MiB, against 6,444 MiB for Lumen HW Epic.
- Non-RT: 3,503.7 + 122.3 + 37.0 = 3,663 MiB, against 5,142 MiB for Lumen SW Epic.
- Both stay under Lumen's totals.

## 8. Claim-avoidance notes (engineering reading, not clearance)

- **W1 (US 10,614,613):** every SH store replaces the cell's value; the spread re-integration and the open-sky scale replace too. No count, no blend.
- **W2 (US 11,790,594):** the environment's slot groups are a fixed rotation by slot index; no cell is ranked by state or age. Cut refills and split refinement are fixed per cell. Query points stay fixed per cell definition (no voting).
- **W3 (US 12,423,782):** no jitter anywhere; refinement reads deterministic finer levels.
- **Near field (P0 §4.1, G-N1):** the cut is a standard LOD cut; masks store cook-time visibility per vertex, every vertex traced, no per-triangle grid, no ranking. Clear per P0.
- **C1-C5, C8:** no reservoirs or resampling; keys stay unprojected 3D cubes; inserts keep atomicMax and winner writes; no per-pixel source switching. The open-sky scale is a per-cell normalization by the prepared irradiance, not a light-sampling or resampling step.

## 9. Misses and open items

1. **Quarry cost, both paths (the largest miss).** The quarry camera-moving increment is +1.30-1.63 ms on RT and +1.77-2.38 ms on non-RT (section 3.2). This is against root's design reading of 0.3 × L_hw = 0.88 ms (RT) and L_sw / 2 = 1.17 ms (non-RT), and far over the stricter readings. Every earlier cost claim, G1b's included, was measured on SV, where the same code costs +0.4-0.6 ms.
   - **Where it goes:** the probe's tiles list hundreds of unique cells on scans (normals spread across bins, orphans' parents, refinement), about 1 ms on still frames; the lookup; and new cells' setup and rays (non-RT software traversal).
   - **Next steps (proposals, not measured):** one bin per pixel unless near a tie; cap the tile's unique cells by merging bins on scans; per-cell rather than per-pixel refinement requests; and spreading new cells' setup over fixed key-hash phases, as the environment groups do (a fixed rotation, W2-safe).
2. **Non-RT quarry cut:** 8-13 ms of setup and rays per frame for four frames. It is spread over frames but not bounded in time (section 1.4). The same fixed setup phases would bound it.
3. **SV non-RT camera cost (item 3).** The non-RT SV camera increment is +0.53-0.62 ms against the 0.46 ms budget (clean quiet steps), 0.04-0.09 ms above G1b's code in the same windows; item 2's per-cell probe work is the increase (section 3.1). RT misses its 0.45 ms line narrowly (+0.42-0.50, against G1b's code at +0.40-0.48 in the same windows). Attempts that did not help (functional lease x28, same window): a probe tile table without per-pixel priorities (no gain), lazy setup resolution (worse), two pixels per probe lane (no gain). One ray-query call site in setup recovered a 30-40 µs setup regression and is kept.
4. **The whole quarry image against Lumen (item 5):** a miss on both paths (section 5.4). Most of the gap is upstream of G1: Renderlab's unoccluded image is already 11.6 codes against Lumen HW's 8.4, and on open ground the standard material reads 0.73-0.89x the reference where identical Lambert materials read 0.99x. **For the material/standard-program owner:** find why the textured quarry materials respond 11-27% darker to the same sky than the reference tracer's (the specular split, vertex colours, texture LOD or metallic/roughness channels are the candidates; none was verified here).
5. **The cache's remaining map bias, −7% (RT) / −5% (non-RT)** under the real RLE2 map (section 5.3). With every direction open the scaled cache is exact at the cell normal, so the rest is visibility at the cell's query point against the pixel's.
6. **No Lumen sky-visibility capture of the quarry.** It would need a cooked albedo-0.5 override package like U0b's SV one (`ue-svsky-package`). G1's estimand on the quarry is therefore scored against the reference tracer only.
7. **The SV edge population's change since G1b** (45.83 / 34.42 → 15.26 / 17.76) was not traced (section 5.2).

## 10. Evidence and process

**Retained under `E:/renderlab-work/m7-g1c`.**
- `runs/x1`-`x40`, `q8-1`, `q10-1`, `q10-2`: per-step plans, process records, contention notes, JSON results and inspected PNGs.
- `analysis/`: full-radiance and map scores (`x34`-`x40`), map checks (`x35`-`x40`), crops.
- `cpu/`: cook reports and guarded CPU test records.
- `plans/`, `scripts/`: the runner plans and the analysis tools (`quarry.py`, `timing.py`, `fullscore.py`, `mapcheck.py`, `svtable.py`, `variant.py`, `naga.sh`, `lease-poll.py`, `quiet-poll.py`).
- `near-cache/`: the cooked near-field packages (default cut `743a2fce…`, 16-bin masks `8093d835…`).
- Deleted after summary, hashes in `deleted-raw.txt`: raw image dumps, the reference tracer's cached quarry references (`refs/`, regenerated by the test on a cache miss), RT exact images and staged test binaries. The Cargo target was deleted at close.

**Code.** `crates/asset/src/near_field.rs` and `crates/cook/src/near_field.rs` (with `tests/near_field.rs`), the `m7-near` command, and `src/gi_bvh.rs`, `src/gi_cache.rs`, `src/gi_cache_tests.rs`, `shaders/gi_*.wgsl`. Runner: `scripts/m7-g1c-run.py` (UNIT `m7-g1c` over `m7-g1-run.py`).

**Reproduce.**
- `renderlab-cook m7-near <source cache> <source manifest> <cache> <manifest> 1 0.25 0 0 8` cooks the default cut.
- `gpu_gi_quarry` takes:
  - `RENDERLAB_M7_G1_QUARRY_SKY=16384` (constant sky) or `map`;
  - `RENDERLAB_M7_G1_QUARRY_MODE=full`;
  - `RENDERLAB_M7_G1_NEAR=<package>`;
  - `RENDERLAB_M7_G1_QUARRY_CUTS=3`, `RENDERLAB_M7_G1_QUARRY_MOTION=24`;
  - `RENDERLAB_M7_G1_QUARRY_DUMP=<dir>`, then `fullscore.py` / `mapcheck.py` on the dumps.

**Process.**
- Every GPU executable ran inside a g1c lease holding `gpu-slot.lock`; CPU-only tools were announced in `gpu-schedule.md`.
- Variant naga checks ran on a renamed binary copy outside `renderlab-work`.
- Functional leases overlapped foreign stillkit builds; the cost claims are quiet window `q10-1` and `q10-2`, per step (section 3.1) only.
- Quiet attempt `q8-1` deferred on the 5 GiB output-disk guard (E: at 4.96 GiB). G1c's superseded dumps (3.1 GB) were hashed and deleted before `q10`.

AA state (IQ1, 2026-11-08; [iq1-quarry-image.md](iq1-quarry-image.md)): this unit's gates rendered 1x with TR1 off and anisotropy 1, the shipping default, so every population, edges included, is an un-anti-aliased image scored against pixel-integrated references. Islam's pinholes and grain in this unit's quarry image come from that missing pixel integration, not from the cache (IQ1 §2).
