# M7-TR2: TR1's gap to TSR, re-measured on current main

2026-11-08/09 overnight unit, branch `t3/m7-tr2` from main `577b4aa`, with main merged at MV1 (`778275c`) and RQ1 (`08aa1ba`). Rules: AGENTS.md's overnight push (2-3 h box; ship what is clearly better and not visibly worse; record misses), [P0](m7-p0-claims.md) and the patent policy, [TR1b](m7-tr1b.md), [MQ1](m7-mq1.md), [MQ2](m7-mq2.md), [UE-SKY](ue-sky.md). No new Unreal or reference campaign: every score below uses the retained path-traced references, TR0's TSR Epic bar rows and UE-SKY's TSR captures. Labels: **M** measured here, **R** retained evidence read here, **I** inference.

**Status: one change shipped (upscaling presets); the native default is unchanged.**

- **Most of the recorded TSR gap was lighting, and G2's default GI cache has since closed much of it on RT; TR2 did not.** Re-scored on current main with the same references, RT TR1 loses to TSR Epic in **35 of 256** pooled sky-walk rows (UE-SKY recorded 216 for TR1b `tr1b-k`) and **2,783 of 22,684** Q-walk rows (TR1b: 8,811). All 35 remaining RT sky losses are single-pixel maxima from bright GI-cache spots (§1.3). **Non-RT keeps the gap**: 218 of 256 sky rows (its GI settles in about 93 frames; the walk has 40) and 9,192 Q-walk rows.
- **TR2's native screen was rejected.** Catmull-Rom history next to edges (rejected outer taps take their nearest inner tap's value) lifts the window of near-edge pixels from 3-5 samples to 16. It helps background-centre next to edges (−0.4 to −0.6 %) and the hold step (0.222 → 0.207 codes), but costs moving interiors 1.0-1.7 % (isolated, narrow, lit): non-RT Q-walk goes from 12,334 wins / 9,192 losses to 11,984 / 9,542 against TSR. A 32-sample window is worse again (11,831 / 9,695). Short windows next to edges are a lag/noise trade, not an oversight (§2.2).
- **Shipped: interpolating young history in the upscaling presets** (`Reconstruction::upscale_interpolation`, default on). Each output pixel weighs every internal sample of its surface by a tent that shrinks from the internal spacing to one output pixel as its history fills, so short histories no longer repeat one internal sample over a 1.5-2 px block. It removes most of the staircase on S2G's moving highlight stripes (inspected, both paths). Native frames are unaffected: **Q-walk 93/93, sky walk 40/40 and the example flight 48/48 frames are byte-identical to main on both paths** (M).
- **Also tried and removed:** adding the sun's turn to MQ2's view drift. U1b's light track rendered byte-identical with and without it (12/12 frames, both paths), and U1b's camera track has not moved since TR1b either (216-218 failures vs 220). MQ2's cap does not appear to engage in U1b's fixture, so the change could not be shown to work (§2.3).

## 1. Where TR1 loses to TSR, and why

### 1.1 The visible-sky walk (R, M)

UE-SKY's rows (TR1b `tr1b-k` against TSR Epic): TR1b worse in 216 of 256 pooled rows on RT and 213 on non-RT. Of those, 214 (RT) and 211 (non-RT) are rows where no-AA (TR1 off, same binary) also loses to TSR. On single-surface pixels, where reconstruction has nothing to resolve, no-AA and TR1b are equal and both far above TSR (moving L0: 26.23 / 26.18 / TSR 15.46). That is TR1b §4.5's lighting error: shadowed rock lit by the unoccluded environment term. TR1's own losses there were the still-hold settling step (0.80 / 1.88 codes, TSR 0.32 / 0.80) and sky next to silhouettes at +4 stops.

**Current main (M).** The same walk on main (`a2`, byte-identical to the pre-merge base on all 40 frames, both paths), scored against the same references and pooled against UE-SKY's TSR Epic scores:

| Segment | RT: TR1 better / worse / tied | Non-RT: better / worse / tied | UE-SKY TR1b worse (RT / non-RT) |
|---|---:|---:|---:|
| All 40 frames (256 rows) | **218 / 35** / 3 | 35 / **218** / 3 | 216 / 213 |
| First still, 0-7 (166) | 115 / 45 / 6 | 14 / 146 / 6 | 132 / 132 |
| Moving, 8-31 (172) | 144 / 22 / 6 | 24 / 142 / 6 | 142 / 142 |
| Last still, 32-39 (256) | 235 / 18 / 3 | 46 / 207 / 3 | 207 / 207 |
| Aged start/revealed pools (626) | 542 / 84 / 0 | 37 / 589 / 0 | 586 / 586 |

The RT change is G2's world radiance cache (default on since G2b), not TR1: frame 0 is byte-for-byte the old no-AA image (single-surface L0 25.79 codes, the same rows as UE-SKY's no-AA), then the error falls as the cache warms (frame 5: 8.9, frame 10: 5.9, frame 35: 3.1). On non-RT the error stays at 25.8-26.6 for the whole walk: GC1 measured non-RT bounce settling in about 93 frames after a cut, and the walk has 40. This answers the GI gap "path-traced re-score not done" for RT; non-RT needs a longer walk or faster settling.

TR1b's lighting-matched control (each path's own 64-spp integral, traced without the cache) is no longer lighting-matched to main's images on RT, so its no-AA gate is not re-scored here (no new reference campaign).

### 1.2 The quarry walk (Q-walk; R, M)

Against TR0's TSR Epic bar (93 reference frames, 22,684 rows; frames byte-identical between main, TR2 and the pre-merge base):

| Path | TR1b (recorded) wins / losses | Main wins / losses / ties |
|---|---:|---:|
| RT | 12,716 / 8,811 | **18,746 / 2,783** / 1,155 |
| Non-RT | 12,468 / 9,056 | 12,334 / 9,192 / 1,158 |

Main's losses by row kind (`tools/attr.py`):

| Path | Still frames | Moving frames | Pooled revealed/start ages |
|---|---:|---:|---:|
| RT | 601 (L0 373, L4 228) | 1,510 (L0 863, L4 647) | 672 |
| Non-RT | 2,123 (L0 746, L4 1,377) | 6,023 (L0 2,500, L4 3,523) | 1,046 |

RT's largest remaining loss groups are background-centre (moving L0 268 rows, L4 239) and isolated pixels (moving L0 250). Non-RT's are the lighting rows TR1b §4.4 described (backlit, lit, L4).

Headline rows (L0 / L4 codes, lower is better; `scores/qwalk-*-table.md`):

| Row | TSR Epic | TR1b (recorded) RT | Main RT | TR1b non-RT | Main non-RT |
|---|---:|---:|---:|---:|---:|
| Edge, frame 64 | 17.81 / 31.88 | 17.98 / 31.77 | **12.45 / 22.11** | 17.96 / 31.82 | 17.98 / 31.98 |
| Background-centre, frame 64 | 0.371 / 1.168 | 0.419 / 1.195 | 0.389 / 1.208 | 0.424 / 1.208 | 0.389 / 1.209 |
| Revealed age 3, all-covered | 7.70 / 12.06 | 9.64 / 16.63 | 8.69 / 12.90 | 9.78 / 17.16 | 9.48 / 16.60 |
| Revealed age 7, all-covered | 7.76 / 12.40 | 9.66 / 17.06 | 8.34 / 12.88 | 9.74 / 17.38 | 9.77 / 17.73 |
| Hold 80→81 step | 0.432 / 0.478 | 0.222 / 0.262 | 0.209 / 0.258 | 0.221 / 0.262 | 0.222 / 0.263 |
| Return 127→128 flicker | 3.43 / 4.18 | 4.36 / 4.95 | **5.25 / 6.23** | 4.36 / 4.95 | **5.03 / 5.64** |
| Start age 0, all-covered | 10.61 / 15.20 | 10.54 / 16.01 | 11.56 / 18.48 | 10.59 / 16.09 | 11.64 / 18.64 |

Two rows are worse on main than in TR1b's recorded run, on both paths: the return-segment flicker (127→128) and start age 0. Start age 0 is the overnight radius-1 spatial start, which makes a cold frame equal no-AA bit for bit (TR1b §4.6, chosen on the visible-sky gate). The return flicker regression is not isolated (§4).

**Why revealed pixels plateau (I, from the resolve's rules).** A moving pixel resamples its history by Catmull-Rom only when all 16 taps show its surface. Within about 1.5 px of an edge some outer tap shows the other surface, so the pixel falls back to bilinear, and T1b's blur budget caps its window at `1 + 1/Σf(1-f)`: 3 to 5 samples while the view moves. Revealed pixels sit there (TR1b §2.4), which is why their age-7 error does not fall below their age-3 error. Non-RT has the same plateau (9.78 vs TSR 7.69 at age 3), so it is not Interactive's ray noise. §2.2 shows that lengthening those windows costs more than it gains.

### 1.3 RT's remaining sky-walk losses are GI-cache spots (M)

All 35 of RT's remaining all-frame losses to TSR are `maximum` rows (single-surface L0 maximum 253.7 vs TSR's 133.3). The worst pixels are 2-3-pixel warm-white spots (145-253 codes, reference about 0) on interior rock of instance 11, e.g. (1204-1206, 586) at frame 31, which move with the surface. At frame 0 (cold cache) the image has 2,696 interior pixels more than 100 codes off. By frame 7 only 93 remain, as isolated spots, which persist through the converged last still frames. They come from the GI cache as it converges, not from reconstruction (whose history would dilute a one-frame firefly). Recorded for the GI gaps.

### 1.4 The named gaps outside the walks (M)

- **U1b's specular gate** (TR1b §7.2; scorer unchanged, 776 comparisons per path, TR1 against the unfiltered "before" program). On MQ2's binary (`mq2/bin/final2`, main before MV1/RQ1) and on TR2, both paths: **408 (non-RT) / 418 (RT)** failures against the 4096-stratum reference, 396 / 405 against the 256-stratum one (TR1b: 458 / 460). By track (non-RT, 4096): static 5, camera 216, light 187. Light-track failures fell from TR1b's 233 (the 0.064-rad turn cap no longer restarts the 0.005-rad track). Camera-track failures (216 vs 220) did not move with MQ2's view cap.
- **Upscaling presets:** stepped aliasing on the finest moving specular stripes. Each output pixel took only its nearest internal sample, so a pixel with a short history (MQ2 caps glossy metal under motion) repeated one internal sample across a 1.5-2 px block (§2.1).

## 2. Changes

### 2.1 Shipped: interpolating young history at reduced internal resolution (`Reconstruction::upscale_interpolation`, default on)

`shaders/temporal_resolve.wgsl` `tr1_gather` / `tr1_continue`: on an upscaled frame, a continued history takes this frame's contribution from every internal sample of the pixel's surface among the 3×3 around the nearest one. Each is weighted by a separable tent of radius `r` output pixels at its jittered position. `r` runs from the internal spacing (1.5 px quality, 2 px performance: bilinear interpolation of the internal grid, weights summing to 1) at an empty history to one output pixel (TR1b's tent, widened to every sample inside it) once the history weight reaches `sharp_window`. The weight added to the history is the tents' sum, at most 1. Native frames never take the branch, and identity, acceptance and windows are unchanged.

Inspected on S2G's 0.5 px/frame track at frame 80 (moving metal r0.1 relief, both paths; `inspect/s2g-*-f080-zoom.png`, 4× nearest): TR1b's quality and performance presets show staircase edges on the bright highlight blocks; with interpolation the quality preset's blocks are smooth-edged, and the performance preset keeps some stepping on the lowest stripes. On the quarry (Q-walk frames 40 and 96, `inspect/qp-rt-f0*-zoom.png`) the presets look the same with and without it: no added softness or trails was visible. **No reference scores at reduced resolution** (the screen wrote display bytes only); this is inspection evidence, as TR1b's preset evidence was.

### 2.2 Rejected: Catmull-Rom next to edges (M)

Screened in one process per path (`s1-q`, `s1-sky`; non-RT is the clean comparison because RT's GI cache carries state between in-process variants: RT frames 0-7 differ between variants that differ only under motion). Non-RT, against TSR Epic and against the base, row by row:

| Variant | Q-walk vs TSR wins / losses | Q-walk vs base: better / worse rows | Hold 80→81 step L0 | Revealed age 7 L0 | Sky walk vs TSR (all) |
|---|---:|---:|---:|---:|---:|
| Base (main) | 12,334 / 9,192 | — | 0.222 | 9.77 | 35 / 218 |
| Edge Catmull-Rom, window 16 | 11,984 / 9,542 | 4,877 / 14,876 | 0.207 | 9.88 | 35 / 218 |
| … window 32 | 11,831 / 9,695 | | 0.182 | 10.03 | 35 / 218 |

Median RMS change against the base: background-centre −0.4 to −0.6 %, edges about −0.2 %, but moving isolated +1.7 %, narrow +1.5 %, backlit +1.3 %, lit/partial/faint +1.1 %, and still frames inherit it. Longer windows next to edges trail the view-dependent and slowly settling shading in those populations. On the sky walk the change moves revealed rows by under 2 % against 50-60-code lighting errors: no usable signal. The code was removed.

### 2.3 Removed: the sun's turn in MQ2's view drift

A highlight's reflected direction moves with the light as with the view, so the change added the frame's largest sun turn to `tr1_view_turn` for view-dependent pixels. U1b's light track (0.005 rad per index) rendered byte-identical with and without it on both paths (12/12 frames), so either U1b's program writes no view hint or the hint does not reach TR1 in that fixture. Camera-track failures unchanged since TR1b (216-218 vs 220) point the same way. The change is not shipped; checking why MQ2's cap is inert in U1b is recorded as a gap.

### 2.4 Files

| File | Change |
|---|---|
| `shaders/temporal_resolve.wgsl` | `tr1_sample_id` (an internal sample's identity), `tr1_gather`, the interpolation branch in `tr1_continue`, option bit 16 |
| `src/temporal_reconstruction.rs` | `Reconstruction::upscale_interpolation` (default on), option bit |
| `src/temporal_reconstruction_tests.rs` | manifest key `upscale_interpolation` |
| `scripts/m7-tr2-run.py` | MQ2's lease runner under owner `m7-tr2` |

The frame-word layout, bindings and every native-scale expression are unchanged. MV1's merge removed MQ1's `TR1_MOVED` bit; TR2 resolved the conflict by keeping main's options and adding its own bit.

## 3. Verification

| Check | Result |
|---|---|
| Q-walk (93 reference frames), TR2 (`b2`) vs main (`a2`), separate processes | **byte-identical, 93/93 on each path**; also identical to the pre-merge base (`s1` `a-base`), so §1.2's main scores are TR2's |
| Visible-sky walk (40 frames), same | **byte-identical, 40/40 on each path** |
| Example flight (`game_scene`, 48 frames, 1280×720), `b2` vs `a2` | **byte-identical, 48/48 on each path** (inspected in motion: §3.1) |
| RF1-RF10 after 32 TR1 frames per pose, final binary vs main | see §3.2 |
| U1b gate | 408 / 418 failures (non-RT / RT), unchanged by TR2 (§1.4) |
| Resolve naga validation (no ray queries) | pass (`reconstruction_modules_validate_without_ray_queries`) |
| Lib suite, final binary, inside a lease | see §3.2 |

`b2` carried the removed sun-turn term, which is inert without a sun turn; the final binary `b3` (the pushed code) differs from `b2` only by that term, and is the one RF, the timing run and the lib suite use.

### 3.1 The example flight, in motion

Frames 8, 24 and 40 on both paths (`inspect/flight-b2-crops.png`): the lamp post, crate and arch edges and the moving cube's silhouette are anti-aliased on both paths. What still reads as noise is lighting: sparkle specks on the gold sphere (both paths) and speckle on crate faces and the red wall on non-RT, the GI cache's non-RT settling speckle recorded in the GI gaps. TR2 changes none of these frames.

### 3.2 Final binary checks (lease `tr2-s4d`, binary `b3` = code at `c84db6e`)

| Check | Result |
|---|---|
| RF1-RF10 after 32 TR1 frames per pose (`gpu_r0a_current`, `RENDERLAB_R1_TR1=1`), `b3` vs `a2` | **13/13 poses byte-identical on RT and on non-RT** |
| S2G 0.5 px frame 80, quality and performance, `b3` vs the screened interpolation (`a`, before the MV1/RQ1 merges) | mean \|difference\| 0.01-0.03 codes over the inspected crop (RQ1 changed the relief's reflections); the smoothing is the same |
| Lib suite (`b3`, release test binary, inside the lease) | **194 passed, 0 failed, 160 ignored; exit 0** |
| `cargo fmt --check` on the changed Rust files | clean |

**Timing** (one run; Q-walk, 1920×1080 target, 2 visits × 160 frames after warm-up per variant, in one process per path; GPU frame span and the resolve pass, ms; functional lease right after ARA1's held window, not QUIET; RTX 4070 SUPER, NVIDIA 617.14, release test binary `b3`):

| Variant | RT span p50 / p95 | RT resolve p50 | Non-RT span p50 / p95 | Non-RT resolve p50 |
|---|---:|---:|---:|---:|
| TR1 off | 3.72 / 4.39 | — | 2.61 / 2.90 | — |
| TR1 native (default) | 4.14 / 4.49 | 0.491 | 3.11 / 3.44 | 0.448 |
| Quality, nearest sample (TR1b) | 2.83 / 2.94 | 0.427 | 2.31 / 2.51 | 0.413 |
| **Quality, interpolation (TR2 default)** | 2.78 / 2.89 | **0.455** | 2.34 / 2.53 | **0.442** |
| Performance, nearest sample (TR1b) | 2.09 / 2.25 | 0.397 | 2.02 / 2.22 | 0.391 |
| **Performance, interpolation (TR2 default)** | 2.08 / 2.21 | **0.421** | 2.00 / 2.19 | **0.415** |

The interpolation costs +0.024-0.029 ms in the resolve pass on both paths; the frame-span differences are within this run's noise. Spans are higher than TR1b's recorded run (RT native 1.47 ms then) because main's default frame now includes the GI cache and later features; the native resolve pass itself is 0.49 / 0.45 ms (TR1b: 0.405 RT), grown with MQ1/MQ2/MV1's work in it.

## 4. Misses and remaining gaps

1. **Non-RT keeps the lighting gap to TSR** on the visible-sky walk (218 / 256) and the Q-walk (9,192 losses): non-RT GI does not settle within the 40-frame walk. Lighting, not reconstruction; GI's.
2. **RT's 35 remaining sky losses are GI-cache bright spots** (maxima, §1.3). GI's.
3. **Revealed pixels next to edges** still lose on Q-walk (age 3: RT 8.69, non-RT 9.48 vs TSR 7.70). Longer near-edge windows trade it for interior lag (§2.2). What is left is a better estimate for a young near-edge pixel that does not lengthen its window (a better resampling kernel for partial footprints, or a start that reuses the neighbouring same-surface history); not built.
4. **Return-segment flicker regressed since TR1b's recorded run** (Q-walk 127→128: RT 4.36 → 5.25, non-RT 4.36 → 5.03; TSR 3.43). Both paths, so not GI alone; candidates are the overnight radius-1 start and paired jitter (TR1b §4.6). Not isolated.
5. **MQ2's view cap does not appear to engage in U1b's fixture** (camera failures 216-218 vs TR1b's 220; light-track frames unchanged by a drift term). U1b's gate still fails (396-418 of 776).
6. **Upscaling evidence is inspection only** (no reference scores at reduced resolution); the performance preset keeps some stepping on the finest moving stripes.
7. **TR1b's lighting-matched control is stale on RT** (traced before the GI cache was the default): the visible-sky no-AA gate was not re-scored against it.

## 5. Claim-avoidance notes (engineering reading, not clearance)

`upscale_interpolation` weights samples by their jittered position and surface identity only: no colour, luminance, variance, neighbourhood statistic or gradient decides anything. The tent radius comes from the history's own sample weight (a count), not from pixel values, so US 11,847,737's value-derived windows and the colour-box family (US 10,116,916 / 10,362,289) are not touched. TR1's histories are still read only by the resolve, next frame (W5). The rejected edge Catmull-Rom was history-only resampling with the existing inner-tap clamp (no current-frame statistic).

## 6. Programmability

Unchanged: acceptance, weights and the new gather read surface identity and sample positions, never colour, so any program converges to the footprint integral of its own output at any internal scale.

## 7. Evidence, timing and reproduction

Hardware: RTX 4070 SUPER, NVIDIA 617.14, Windows 11. Bulk evidence under `D:/Coding/renderlab-work/tr2` (116 MB kept: score JSON/logs, `scores/s4-summary.json`, `ana/`, inspected PNGs in `inspect/`; 24.15 GiB of raw frames, RF images and superseded binaries deleted after summarizing, listed in `deleted-raw.json`). Kept binaries (release test binary and dev `game_scene`): `a2` (merged main's behaviour: TR2 option off) and `b3` (this branch's code). The Cargo target was deleted.

- Screens: `plans/s1-q`, `plans/s1-sky` (base / edge / edge window 32), `plans/s2-s2g`, `plans/s2-qp` (presets with and without interpolation); scored with `tools/score.sh` (scripts/m7-tr1-score.py `score`, `compare`, `pooled` against `ue-sky/runs/*-tsr-epic/score.json`), `tools/attr.py`, `tools/rowdiff.py`; crops with `tools/crops.py`.
- Identity: `plans/s3-q-{a2,b2}`, `plans/s3-sky-{a2,b2}` (frame hashes, `--no-write`), `runs/ex`.
- U1b: `u1b/` with `tools/u1b-score.sh` (TR1b §7.2's procedure, scorer unchanged).
- Leases: `tr2-s1` (2.1 min), `tr2-s2` (4.8 min), `tr2-s3` (8.7 min), `tr2-s4d`; all headless, functional (not QUIET), ≤15 min, holding for root's b11/b12 merge gates (order of 2026-11-08 23:44Z) and ARA1's held window (00:37Z). **Schedule error:** a waiter for ARA1's held window matched the quoted done-text inside the root order itself and queued `tr2-s4c`, which took the lock at 00:37:51Z, inside the held window, before ARA1 started. It ran two RF steps and was stopped, and the lock was released with a release line at 00:39:05Z. The re-armed waiter ignores `[root order]` lines.
- Time: implementation about 40 min; verification (builds, leases, lease waits, scoring) about 1 h 45 min, most of it lease queueing and CPU scoring.
