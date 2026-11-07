# N8k: a map-only sun visibility lookup, and a map/exact hybrid

Follow-up unit to [N8i](non-rt-n8i.md), [N8j](non-rt-n8j.md) and [N8l](non-rt-n8l.md), 2026-11-07 UTC, base `d798200`, main `a6d773e` (N8l) merged before the final runs, branch `t3/n8k-map-lookup`. Brief (Islam, 2026-11-07): a non-RT shadow lookup with **no per-pixel geometry walks**, reading only light-space map data at VSM-like cost, tuned to stay above Unreal VSM on every quality measure, in its own shader and module beside the exact walk. Update during the unit: if the first prototype misses, measure why per stage and per measure and try the next variant the data points to; report once a variant beats VSM on every measure at ≤ 1 ms camera-moving, or once the directions are exhausted with data. Labels: **M** measured here, **I** inference.

## Result

**No variant beats VSM on every quality measure at ≤ 1 ms camera-moving. Each direction below was built and measured, and its data is in this record.** At this checkpoint Exact (main's N8l walk) stayed the default; Map and Hybrid were opt-in. [N8p](non-rt-n8p.md) subsequently makes Surface the default and retains Exact as the reference.

- **Map** (`VirtualLookup::Map`, shaders/virtual_shadow_lookup.wgsl) reads only light-space map data per pixel.
  - **Cost:** 0.94-0.96 ms camera-moving on the no-RT quarry. Exact costs 1.95 on the same binary; VSM's is estimated at about 0.5.
  - **Above VSM:** every S2 and S3 measure in motion, and the quarry's partial RMS, flips, leaks and lost umbra edges.
  - **Misses on the quarry:** lit-darkened, 263 / 242 against VSM's 87 / 88.
  - **Misses on S1:** solid flips, 48 / 38 against VSM's 0-1.
  - Both misses are lit receivers darkened by their own surface below a texel, which the map cannot resolve.
- **Hybrid with 6 march samples** (`VirtualLookup::Hybrid`, `march_samples: 6`, Exact's walk cap). Map runs first, then main's exact walk re-tests the 4.3-4.8% of quarry receivers whose map decision used an ambiguous block.
  - **Quality:** above VSM on every measure of every bench (quarry no-RT and traced, S1, S2 and S3 in motion).
  - **Cost: 1.51-1.53 ms camera-moving**, which misses the 1 ms bar by about 0.5 ms (Exact: 1.95).
  - The walk cap of 4 measured earlier saves only 0.02-0.05 ms. It misses VSM's partial RMS on S2 and S3, because it collapses queued penumbrae to umbra.
- **The map-only frontier ends at VSM's corner, not beyond it.** Every lever that removes lit-darkened errors also frees real penumbra blocks. The best combination ties VSM within noise on the two binding measures:
  - pose 0: partial RMS .4841 against .483, lit-darkened 86 against 87;
  - pose 1: partial RMS .4808 against .483, lit-darkened 97 against 88.
- **Cost to close the gap** (I, from the data below):
  - The hybrid's queued exact pass has a latency floor of about 0.5-0.9 ms, whatever the queue's size, order or direction count.
  - Getting under 1 ms needs either a map lookup near 0.5 ms plus a queued pass near 0.4 ms, or a map-only decision that tells a receiver's own near relief from its own surface error.
  - That near-field distinction is exactly what the map cannot see below a texel (§ Why the map misses lit-darkened).
- **Further misses:**
  - **Caster motion:** Map is no faster than Exact (1.92-1.96 against 1.87-1.94 ms). Its per-texel plane resolve over re-rendered caster pages costs 0.53 ms more than Exact's page work (P23).
  - **Small scenes:** Map's lookup costs about twice Exact's (S3 0.47 against 0.235 ms, validation on).

| No-RT quarry, pose 0 (pose 1) | Camera-moving ms | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|---:|
| Unreal VSM native (recorded) | about 0.5 (I) | .483 | 12,390 | 14,935 | 87 (88) | 11,884 |
| Exact (main, N8l) | 1.95 ✗ | **.353** (.348) | **2,434** | **3,288** | **60** (55) | **2,376** |
| **Map** | **0.94-0.96** | .446 (.444) | 5,537 | 8,923 | 263 (242) ✗ | 5,297 |
| **Hybrid, 6 samples** | 1.51-1.53 ✗ | .445 (.443) | 4,931 | 5,538 | 83 (58) | 4,824 |
| Hybrid, walk cap 4 + 6 samples (misses S2/S3) | 1.47-1.50 ✗ | .458 (.456) | 4,802 | 4,837 | 83 (58) | 4,694 |
| Map, best tie (soft .4, facing 9, lone 2, foreign cover .65) | (I: about Map's) | .4841 ✗ (.4808) | 8,090 | 11,503 | 86 (97 ✗) | 7,984 |

### Timing against VSM and Exact (M)

The final lease ran one process on the final binary for the no-RT quarry: 1920×1080, LOD 0.35 px, lift 1, poses 0 and 1, 16 turning frames, 48 camera-moving and 48 caster-moving frames. Nine configurations ran interleaved (Exact, Map, Hybrid, their dense `refresh: 1` forms, then the three again). Ranges are the shadow-pass medians of clean visits (primaries ≤ 0.45 ms). Contended visits (primaries 0.46-0.65 ms) are excluded, and so are Map's first camera visit (1.46 ms, primaries 0.65) and Exact's first moving visits (1.96-1.97, primaries 0.47). Unreal's rows are the recorded native ones (camera motion is N8f's estimate).

| No-RT quarry, ms | Unreal VSM | Exact (main N8l) | **Map** | Hybrid, cap 4 + 6 samples | **Hybrid, 6 samples** (P23) |
|---|---:|---:|---:|---:|---:|
| Static (rolling refresh) | 0.50 | 0.553-0.583 | **0.372-0.382** | 0.781-0.824 | 0.791-0.827 |
| Turning | about 0.95 | 0.578-0.589 | **0.385-0.396** | 0.793-0.818 | 0.819-0.828 |
| **Camera moving** | about 0.5 (I) | 1.947-1.954 | **0.940-0.957** | 1.471-1.499 | 1.506-1.531 |
| Caster moving | no recorded row | 1.873-1.939 | 1.923-1.958 | 2.355-2.364 | 2.369-2.400 |
| Dense static (`refresh: 1`, the lookup alone) | — | 1.834-1.908 | 0.854-0.884 | 1.376-1.433 | — |
| No-RT frame, camera moving | 1.61 | 2.754-2.769 | 1.760-1.815 | 2.203-2.357 | 2.316-2.608 |
| No-RT frame, static | 1.61 | 1.265-1.418 | 1.111-1.204 | 1.500-1.674 | 1.560-1.599 |

- **Map halves Exact's camera-moving shadow pass** (0.95 against 1.95 ms) and is a third faster static and turning. It meets 1 ms but not VSM's estimated 0.5. Its camera-moving frame (1.76-1.82 ms) is above VSM's 1.61; its static frame is below.
- **Hybrid with 6 samples** was timed in P23, the next lease on the same binary. Its cap-4 form read 1.485 ms there, as it did here (1.47-1.50). Exact's walk cap therefore costs it 0.02-0.05 ms. Hybrid at Exact's settings read 1.66 ms in P23.
- **Caster motion is not faster** (1.92-1.96 against Exact's 1.87-1.94). See § Caster motion and small scenes.
- **Exact's counts equal N8l's** at both poses (`05e50a1d…`, `bed48bf7…`), with the mip skip in place.

### Caster motion and small scenes (M)

P23 skips each lookup at its entry: `VMAP_SKIP` 3 writes lit before any work, `VSM_SKIP` 1 after the tile checks. Page work, texel planes (Map), horizons and records (Exact) and mips still run. A skipped variant's time is therefore the page-side cost, and the full variant minus it is the lookup. Clean visits:

| No-RT quarry, ms | Exact full | Exact page side | Exact lookup | Map full | Map page side | Map lookup |
|---|---:|---:|---:|---:|---:|---:|
| Camera moving | 1.93 | 0.115-0.131 | about 1.81 | 0.94-0.96 (final lease) | 0.119-0.139 | about 0.82 |
| Caster moving | 1.867-1.930 | 0.820-0.842 | about 1.06 | 1.92-1.96 | 1.363 | about 0.58 |
| Static, turning | 0.57-0.59 | 0.004 | — | 0.37-0.40 | 0.004 | — |

- **On caster frames Map's lookup is about half of Exact's, but its page side costs 0.53 ms more.**
  - Each moving caster re-rasterizes its pages at every level on every frame.
  - `vmap_resolve` then rebuilds each of those texels' planes and planarity errors from their triangles: three vertices per layer, plus four neighbour planes.
- (I) **Cost to close it:** write the plane where the raster pass already holds the triangle in workgroup memory. It is constant per triangle, so only the planarity error would need a pass. That edits main's `vsm_raster` in virtual_shadow.wgsl, which this unit was asked to leave alone.
- **Small scenes** (validation on, as scored; shadow-pass p50):
  - **S3** has no page work in its scored frames. Map's lookup costs 0.47 ms there against Exact's 0.235 and VSM's 0.37 static / 0.59 rotating.
  - **S2** has page work on every frame. Map costs 0.95-1.01 against Exact's 0.35-0.48 and VSM's 0.35.
  - **Hybrid with 6 samples:** S3 0.56, S2 0.89-1.06.
  - (I) Why: Exact's walk is cheap on simple geometry. Map's march costs about the same per pixel whatever the geometry, and S2 pays its plane resolve on every frame. Map wins only on dense scenes like the quarry.

## What was built (M)

- **Setting.** `VirtualShadowSettings::lookup: VirtualLookup` (`Exact` reference, `Map`, `Hybrid`; Surface is the default since [N8p](non-rt-n8p.md)). Shared edits are small: the enum and its export, one parser line in the test settings, dispatch selection in `encode_virtual`, the constants list for screens.
- **Per-texel data at page-draw time** (`vmap_resolve`, my module; only for pages rasterized this frame). For the top and peel triangles, it stores the light-space height plane (f16 gradient, with the winding's facing toward the sun in bit 0) and the plane's planarity error: its largest prediction error at a same-placement 4-neighbour inside the page. These are 8 + 4 bytes per texel. A min-mip pyramid of the top layer sits beside main's max-mips.
- **Map lookup** (`vmap_lookup`, shaders/virtual_shadow_lookup.wgsl). Per pixel it reads the receiver record (world point, geometric normal, placement and primitive) and light-space map words only. No triangle, vertex or adjacency is fetched. Its steps:
  - **Facing.** A direction below the receiver's facet plane is blocked. Mode 6 frees one that rises above a sun-facing own plane, for front-viewed, sun-facing receivers.
  - **Own-surface floor** (receiver-plane depth bias, from exact receiver data). Under the facet's tangent plane, and under the envelope of the plausible own planes the 2×2 centres around the receiver rebuild, within 2 texels, no surface of the receiver's placement can block. Plausible: within a quarter texel plus the slope difference over the distance, plus the planarity error.
  - **Certain umbra** (min-mips, as in min-max hierarchical shadow maps). If the lowest top-layer centre over the cone's footprint, out to the height where the rays would meet it, lies above every ray and the tangent plane, all directions are blocked.
  - **March.** A contact sample, a half-texel sample, then geometric heights at ratio 1.75 up to a **local caster ceiling**: the max-mip over the full cone's footprint at the receiver, two refinements. Max-mip skips run per step, and a **clean-cell skip** applies when the receiver's 2×2 centres hold only its own surface.
  - **Candidate test** (SMRT-like, against the map's surfaces). The 2×2 bilinear coverage (PCF) of plane-rebuilt surfaces sits over the sample. The ray rises to the lowest covering surface (top or peel) until it is at a surface that still covers it (blocked), or nothing covers it (it passed beside or under it).
- **Hybrid.** The map queues receivers with an ambiguous block, one contiguous block per trace tile. An ambiguous block is a facing block within 0.05 of the facet plane, a block by an own surface within 1.5 texels, or one at partial coverage. Then `vsm_lookup_queued` (shaders/virtual_shadow_hybrid.wgsl, appended to main's virtual_shadow.wgsl so it calls that module's `vsm_prepare`, `vsm_ring` and `vsm_march`) runs the exact walk for them over an indirect dispatch. It shares Exact's walk settings; use it with `march_samples: 6`, Exact's walk cap.
- **Mip rebuilds only on frames that rendered a page** (both lookups). Mips are now dispatched indirectly from `vsm_compact`'s cull arguments, so camera motion inside the windows rebuilds none. Output is byte-identical (Exact's hashes unchanged on every screen), and it saves 44 µs per camera-moving frame (`vsm_mip` 47.6 → 3.6 µs, Nsight).
- **Programmability.** Planes, errors and min-mips are geometry only; the passes output visibility counts that user shading programs consume. One constraint: the facing bit and the own-surface rules assume consistently wound casters, as N8's peel already does. Mirrored placements flip my resolve's facing bit (none in the benches; recorded, not handled).
- **Diagnostics** (screen constants, never shipped on): `VMAP_TRACE` 1/2/3 (per-direction cause; why open directions stayed open; ambiguity flags), `VMAP_COST` (cover and max-mip tests per pixel), `VMAP_SKIP` 1/2/3 (stage isolation). Scripts are in E:/renderlab-work/n8k/scripts: `causes.py`, `nort_causes.py`, `timing2.py`, and the trace exporters.

## How the cost moved, stage by stage (M)

No-RT quarry 1920×1080, LOD 0.35 px, lift 1. "Dense" is a static frame with `refresh: 1` (no page work; GPU-bound, so clean). Map frames are light enough that the next frame's primaries read slow under unaltered clocks, which made N8f's "primaries ≤ 0.45 ms" clean rule mark them contended (§ Measurement notes). Nsight GPU Trace (Top-Level Triage, real-time shader profiler, N8g's `move_wrap` recipe) gives per-dispatch time inside camera motion.

| Change | Dense lookup ms | Camera-moving ms | What the data said next |
|---|---:|---:|---|
| P1 first map lookup (planes, 2×2 coverage, tangent filter) | — | 1.8-4.9 (contended screen) | setup + facing only: 0.32 incl. page work, so the march is the cost |
| P2-P3 own-surface envelope, texel-aware schedule (contact + half-texel) | — | 1.6-2.0 (contended) | lit receivers: 8.4 cover and 10 max-mip tests each, all at the half-texel step |
| P4 trace | **1.03-1.11** per dispatch | — | SM 34%, L2 30% (97.8% hit), VRAM 5%: not bandwidth. 12.4 warps/SM, 13 of 32 lanes. 9-11% of samples on a dynamically indexed private array (local memory) |
| P5 own planes in registers, clean-cell skip (byte-identical) | — | — | lit cover tests 8.4 → 0.78, yet cost barely moved |
| P6 stage isolation | dispatch 0.032; + setup 0.037; + own planes and facing 0.16-0.18; full **1.00-1.02**; no max-mip skip 2.95-3.28 | 1.13 | the march is 0.83 ms, about 9 max-mip tests per lit receiver |
| P7 trace | 0.86-0.90 per dispatch | — | 9.4 warps/SM, 30% lanes; umbra and partial covers in divergent waves |
| P8 local caster ceiling, range-proportional steps | **0.68-0.74** | **0.82** | max-mip tests per lit receiver 9.1 → 3.1 |
| P9 mip skip (both lookups), certain-umbra test | 0.66-0.70 | 0.76-0.83 | certain umbra decides 5,108 receivers, 8 of them wrong |
| P10-P20 facing mode 6 (default), umbra test, all defaults | 0.86-0.90 | **0.96-0.97** | as P9 plus mode 6's second march and the per-tile queue's code |

Hybrid's queued exact pass (Nsight, P10): 0.74-0.79 ms per frame for about 17k receivers. 4.6 warps/SM and 4.1 of 32 lanes active: incoherent heavy walks.
- Re-testing only the flagged directions: no cheaper, worse quality (P12).
- Appending one contiguous block per trace tile: identical output, no cheaper (P13).
- On main's N8l walk (camera-moving):
  - 1.84 ms at Exact's settings (1.66 in P23);
  - about 1.7 at walk cap 8;
  - 1.475 at cap 4 with 6 march samples;
  - 1.36-1.52 with 4 walk steps and 1 refinement added (P21);
  - 1.51-1.53 with 6 samples at Exact's cap (P23). This is the kept setting, because cap 4 fails S2 and S3.
- Ring-only exact pass: 1.05-1.06 ms, at a large quality loss (P19).
- In the final lease, Hybrid's static pass reads 0.78-0.83 ms against Map's 0.37-0.38: the queued pass costs about 0.4 ms even under the rolling refresh.

## How the quality moved, change by change (M)

Two instruments:
- the traced quarry screen against hardware Interactive on the same eight directions (346,517 receivers): lit receivers shadowed / umbra receivers lit / flips ≥ ½; Exact 1 / 1,438 / 2,157;
- from P7 on, the 64-seed film reference with n8e-score, the scorer and pixels the Unreal VSM row uses. Exact reproduces N8h's .3529 / 2,434 / 3,288 / 60 / 2,376 in every run.

Causes come from `VMAP_TRACE` dumps.

| Step | Lit shadowed | Umbra lit | Finding |
|---|---:|---:|---|
| P1 first map lookup | 4,714 | 5,853 | 312 from the facing test alone; planarity ×3 cuts to 1,109 |
| P2 own-surface filter plane (closest own plane) | 4,035 | 7,047 | causes: own placement 3,685, facing 231, other placements 38. Silhouettes are not the problem |
| P3 envelope of all plausible own planes | **915** | 8,531 | own 565, facing 183, other 38 |
| P4 own coverage 1.0 near the receiver | 474 | 13,638 | about 10 umbra receivers lost per lit one saved |
| P6-P7 envelope also for terminator receivers, own reach 2 texels | 452 | 9,754 | facing 311, own 103, other 37; facing-own modes 1-3 fail (frees shadow-side directions) |
| P9 facing mode 6 (sun-facing, front-viewed receivers only) | **240** | 9,497 | facing 105, own 99, other 36 |

Film-reference scores, no-RT quarry pose 0 (pose 1 within about 10% unless given):

| Variant | pRMS | Flips | Leaks | Lit dark | Lost edges | Finding |
|---|---:|---:|---:|---:|---:|---|
| P7 Map (mode 0) | .431 | 5,432 | 9,160 | 417 | 5,193 | lit-darkened the only miss |
| P9 Map (mode 6, default) | .446 | 5,537 | 8,923 | 263 | 5,297 | |
| P13 causes of those 263 | | | | | | **facing 199** (148 with all 8 directions blocked), own 37, other 27: LOD receivers whose simplified facet faces away while the traced source surface is lit |
| P14 mode 7 (closest own plane decides) | .488 ✗ | 8,048 | 11,363 | 101 | 7,970 | frees umbra receivers too |
| P15 mode 7, margin .02 / .05 / .10 | .487 / .486 / .481 | ~8,000 | ~11,360 | 102 / 106 / 116 | ~7,900 | margin does not discriminate |
| P15 mode 7 + foreign coverage .75 | .505 ✗ | 9,003 | 12,652 | 51 | 8,927 | |
| P16 mode 8 (only when other directions open) | .488 | 8,043 | 11,362 | 101 | 7,965 | vacuous: all-8-facing receivers dominate |
| P16 + lone release 1 / 2 | .495 / .506 | 8,043 | 11,362 | 69 / 63 | 7,965 | releases real penumbra blocks |
| P17 mode 8 non-vacuous + lone (near-own) 2 + cover .65 | .495 | 6,239 | 9,693 | 173 | 6,051 | 139 of 173 all-8-facing |
| P18 mode 9 (strict own plane), plausibility .25 / .5 / 1 texel | .443 / .451 / .462 | 5,884-6,810 | 9,427-10,315 | 245 / 193 / 139 | 5,740-6,718 | a smooth trade, no corner |
| P22 soft decisions ±.15 / ±.25 / ±.40 | .448 / .450 / .452 | 5,816-6,067 | 8,920-9,700 | 245 / 241 / 237 | 5,571-5,820 | soft does not lower partial RMS |
| P22 soft .4 + mode 9 (1 texel) + lone 2 + cover .65 | .4841 ✗ (.4808) | 8,090 | 11,503 | 86 (97 ✗) | 7,984 | **ties VSM's corner** |
| P10 Hybrid (full exact queue) | .444 | 4,850 | 5,250 | 83 (57) | 4,748 | beats VSM everywhere |
| P12 Hybrid, flagged directions only | .454 | 5,902 | 9,778 | 163 | 5,759 | worse and no cheaper |
| P19 Hybrid, ring-only exact pass | .531 ✗ | 12,137 | 14,043 | 154 | 11,446 | releases real penumbra and near-own blocks |
| P20 ring + far-field march, all facing blocks to the ring | .494 ✗ | 11,752 | 14,443 | 42 (47) | 11,404 | |
| P21 Hybrid, walk cap 8 / cap 4 + 6 samples | .452 / .458 | 4,770 / 4,802 | 4,943 / 4,837 | 83 / 83 | 4,667 / 4,694 | beats VSM everywhere |

### Why the map misses lit-darkened (M, with the I marked)

- Map's partial error is a lit bias: its mean signed partial error is +.29 (Exact +.05, VSM +.35). The why-trace shows umbra receivers' open directions mostly *never covered* (P5: 33,432 never covered, 10,481 passed a candidate).
- The rules that suppress false self-shadow also suppress real self-shadow (the own envelope, plausibility, near-own coverage). They act within the texel or two where the map's height planes cannot tell a receiver's own relief from its own reconstruction error.
- On no-RT receivers the largest single class is LOD facets that face away from the sun over a sunlit source surface. Exact frees them through the lifted origin and the LOD triangle's finite extent. The map only sees the source surface, and its closest plane is unreliable at the creases where this happens.
- (I) A per-texel record of the receiver's own 1-ring horizon would resolve the near field. But it is receiver geometry the brief excludes, and N8b's per-triangle horizons cost 99 MiB.

## Quality tables (M)

All on the final binary (merged main plus N8k), with the unchanged scorers: n8e-score for the quarry, `bench-contact-fold-score.py` for S1, `n8-score.py` (the bench-motion scorer) for S2 and S3. Unreal rows are the recorded native captures. Exact's rows are N8l's final scores on main (identical output). Hybrid is `march_samples: 6` at Exact's walk cap (16) unless named. ✗ marks a measure below VSM; a tie at 0 is not a miss.

**Quarry, no-RT film reference** (LOD 0.35 px, lift 1). Map's and cap-4 Hybrid's counts on the final binary hash equal to P21's (`e99a3992…`, `5865d402…`, pose 0), so those are P21's scores. Hybrid with 6 samples was scored in P23:

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|
| Unreal VSM | .483 / .483 | 12,390 | 14,935 | 87 / 88 | 11,884 |
| Exact | .353 / .348 | 2,434 / 2,474 | 3,288 / 3,366 | 60 / 55 | 2,376 / 2,424 |
| Map | .446 / .444 | 5,537 | 8,923 | **263 / 242 ✗** | 5,297 |
| **Hybrid, 6 samples** | .445 / .443 | 4,931 / 4,825 | 5,538 / 5,490 | 83 / 58 | 4,824 / 4,751 |
| Hybrid, cap 4 + 6 samples | .458 / .456 | 4,802 / 4,701 | 4,837 / 4,762 | 83 / 58 | 4,694 / 4,624 |
| Hybrid, Exact's settings | .444 / .443 | 4,850 / 4,766 | 5,250 / 5,219 | 83 / 57 | 4,748 / 4,697 |

With traced primaries (the RT path's receivers, same shadows), Hybrid with 6 samples scores .417 / .417 partial RMS, 3,873 / 3,812 flips, 4,554 / 4,539 leaks, 25 / 16 lit-darkened and 3,832 / 3,779 lost edges.

**Quarry, traced screen** against hardware Interactive on the same directions (346,517 receivers, pose 0, static / turned; Hybrid at cap 4 + 6 samples):

| | Lit shadowed | Umbra lit | Flips ≥ ½ | Hashes |
|---|---:|---:|---:|---|
| Exact | 1 / 12 | 1,438 / 1,505 | 2,157 / 2,071 | `2d6e4fa6ca32…` / `88888216c1d9…`, N8l's |
| Map | 240 / 232 | 9,497 / 9,626 | 9,350 / 9,503 | `b9cbc1d4fb5f…` / `0ff5c320b345…` |
| Hybrid, cap 4 + 6 samples | 25 / 25 | 4,407 / 4,473 | 7,580 / 7,612 | `cb3b883a794e…` / `c827959c289b…` |

**S1, contact fold** (environment 0; environment 1 equal unless given):

| Pose 0 / pose 1 | Partial RMS | Solid flips | Umbra leaks | Contacts lost |
|---|---:|---:|---:|---:|
| Unreal VSM | .1642 / .1628 (env 1: .1644 / .1620) | 0 (env 1: 1) / 0 | 0 / 0 | 5-6 / 2-3 |
| Exact | .1085 / .1065 | 0 / 0 | 0 / **2 ✗** | 0 / 0 |
| Map | .1260 / .1274 | **48 / 38 ✗** | 0 / 0 | 0 / 0 |
| **Hybrid, 6 samples** | **.0916 / .0889** | 0 / 0 | 0 / 0 | 0 / 0 |
| Hybrid, cap 4 + 6 samples | .1628 / .1590 | 0 / 0 | 0 / 0 | 0 / 0 |

Map's S1 flips are all lit pixels rendered dark (reference 1.000, Map 0.01-0.03). They sit in a dotted grid on the 1-20 mm strips (double-sided thin receivers, below a texel) and in a dotted line along the convex control's lit top edge. Those are the same sub-texel own-surface limit as the quarry's lit-darkened errors. Hybrid's queue catches all of them.

**S2 and S3 in motion** (validation on, as scored; per-phase maxima for leaks and flips):

| Bench, phase | Exact pRMS | Map pRMS | Hybrid pRMS | VSM pRMS | Map / Hybrid HDR RMS (VSM) | Map / Hybrid contacts lost of n (VSM) | Map / Hybrid leaks (VSM) | Map / Hybrid flips (VSM) |
|---|---:|---:|---:|---:|---|---|---|---|
| S2 camera out | .0833 | .0881 | .0835 | .1752 | .0111 / .0107 (.0151) | 56 / 32 of 19,396 (2,101) | 1 / 1 (62) | 39 / 0 (167) |
| S2 camera hold | .0865 | .0883 | .0837 | .1758 | .0119 / .0114 (.0154) | 32 / 32 of 11,264 (1,039) | 1 / 0 (57) | 32 / 0 (152) |
| S2 camera return | .0809 | .0881 | .0829 | .1739 | .0107 / .0103 (.0145) | 52 / 34 of 17,990 (1,972) | 1 / 1 (62) | 39 / 0 (155) |
| S2 shadow out | .0759 | .0845 | .0788 | .1697 | .0103 / .0100 (.0152) | 3 / 3 of 16,687 (2,070) | 1 / 1 (65) | 16 / 0 (120) |
| S2 shadow hold | .0766 | .0849 | .0771 | .1675 | .0104 / .0100 (.0150) | 0 / 0 of 8,832 (1,059) | 0 / 0 (66) | 16 / 0 (119) |
| S2 shadow return | .0759 | .0845 | .0788 | .1692 | .0103 / .0100 (.0152) | 3 / 3 of 16,687 (2,057) | 1 / 1 (67) | 16 / 0 (120) |
| S3 static | .1186 | .0891 | .1005 | .1878 | .0144 / .0145 (.0194) | 32 / 32 of 48,512 (2,929) | 0 / 0 (111) | 1 / 0 (187) |
| S3 rotate .25 | .1217 | .0890 | .1016 | .1875 | .0143 / .0145 (.0194) | 128 / 128 of 194,291 (11,630) | 0 / 0 (111) | 1 / 0 (190) |
| S3 hold slow | .1266 | .0883 | .1040 | .1873 | .0143 / .0146 (.0194) | 32 / 32 of 48,640 (3,006) | 0 / 0 (109) | 1 / 0 (191) |
| S3 rotate 1 | .1218 | .0913 | .1031 | .1863 | .0143 / .0146 (.0200) | 189 / 157 of 196,290 (12,289) | 0 / 1 (106) | 1 / 0 (192) |
| S3 hold fast | .1210 | .0909 | .1035 | .1875 | .0143 / .0145 (.0205) | 64 / 64 of 49,728 (3,378) | 0 / 0 (103) | 1 / 0 (188) |
| S3 jump recovery | .1159 | .0891 | .1015 | .1811 | .0142 / .0143 (.0219) | 33 / 0 of 51,777 (3,153) | 0 / 0 (114) | 3 / 0 (226) |

- **Map and Hybrid beat VSM on every S2 and S3 measure, in every phase.**
  - On S3, both have lower partial RMS than Exact: Map .088-.091, Hybrid .100-.104, Exact .116-.127. Map's penumbrae are smooth where the exact walk's per-pixel directions leave grain.
  - Exact loses no contacts. Map and Hybrid lose 0-189, at most 0.1%; VSM loses 6-9%.
- **Hybrid at walk cap 4 misses VSM's partial RMS on S2's camera out and hold phases and on all six S3 phases.** The cap, not the 6 march samples, causes it. The final lease rescored S1-S3 with the two settings apart (below). N8e's walk cap takes every undecided direction as blocked once a receiver's walks have visited that many triangles and one direction is proven blocked. At 4, a queued penumbra receiver collapses to umbra: S3's penumbrae turn to dots of 0 and 1 (`final-hybrid/S3/images/*-details.png`). The queue holds most penumbra receivers on these small scenes, unlike the quarry's 4.3-4.8%, so the cap's partial-RMS cost there (.444 → .458) becomes a miss here.

| Partial RMS | S1 pose 0 / 1 | S2, six phases | S3, six phases | Other measures |
|---|---:|---:|---:|---|
| Unreal VSM | .1642 / .1628 | .1675-.1758 | .1811-.1878 | |
| Exact | .1085 / .1065 | .0759-.0865 | .1159-.1266 | |
| Map | .1260 / .1274 | .0845-.0883 | .0883-.0913 | |
| Hybrid, cap 4 + 6 samples | .1628 / .1590 | .1565-.1880 ✗ | .2173-.2328 ✗ | |
| Hybrid, cap 4 | .1541 / .1517 | .1589-.1871 ✗ | .2144-.2241 ✗ | |
| **Hybrid, 6 samples** (cap 16) | **.0916 / .0889** | **.0771-.0837** | .1005-.1040 | S1: 0 flips, leaks, contacts lost. S2/S3: flips 0, leaks 0-1, contacts lost as Map's (0-157) |
| Hybrid, Exact's settings (cap 16, 10 samples) | .0930 / .0907 | .0782-.0886 | .1033-.1078 | as 6 samples |

Hybrid with 6 samples and Exact's cap beats VSM on every S1-S3 measure, and beats Exact's own partial RMS on S1 and S3 (S2 within .003 of it). On the quarry it scores at Exact's-settings level (.445 / .443 partial RMS, lit-darkened 83 / 58). Its camera-moving cost is 1.51-1.53 ms, only 0.02-0.05 above the cap-4 form, so the cap bought almost no speed there.

Not film quality in any mode, and never presented as meeting the film gate.

## Memory against Unreal (M)

One process per path, on the no-RT quarry (pose 0, LOD 0.35 px, `MEMORY_HINTS` blocks). Measured with N8h's method: the allocator's reserved bytes and the WDDM per-process dedicated-usage maximum, against Unreal VSM's DXGI `LocalUsedMB` on the matched scene.

| No-RT quarry, MiB | Allocator reserved | WDDM max | Against Unreal VSM's 3,912 |
|---|---:|---:|---|
| Exact | 3,439.7 | 3,497.9 | 472 below (M2's figure) |
| **Map** | **3,068.3** | **3,126.4** | **844 below** |
| Hybrid | 3,599.7 | 3,644.7 | 312 below |

- **Map** drops the walk records (447.5 MiB) and ring horizons (99.4). It adds texel planes (96.0), planarity errors (48.0) and min-mips (16.0): 160 MiB for three levels of 2048² texels.
- **Hybrid** keeps both lookups' data plus its queue (15.8 MiB, 8 bytes per pixel).
- The mip dispatch arguments are 192 bytes.

## Measurement notes (M)

- **Clean visits.** N8f's rule marks a visit clean when its primaries read ≤ 0.45 ms. Map and skip-variant camera-moving frames read primaries of 0.47-0.81 ms in the *same* process where Exact's read 0.39. That was consistent per variant (P5): the lighter the shadow pass, the slower the next primaries, i.e. GPU idle or clock behaviour between CPU-bound frames, not random contention. Map's camera-moving numbers are the visits within the rule plus the closest above it, and are cross-checked by Nsight per-dispatch times (P4, P7, P10, P12) and by dense static frames.
- **Contention.**
  - Another worker's `retained.exe` (m1-memory) used 12-30% SM during its own lease, and wallpaper contention was intermittent.
  - The guard's foreign-process check aborted two of this unit's leases when a cargo build script started beside them: P21, whose timing half was rerun as P21b, and the first final lease. The second was this unit's own viewer clippy, started during its lease. Its S1-S3, memory and screen steps had finished; part 2 reran the timing.
  - Free memory never fell below 11 GiB in any lease.
- **Builds.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, validation off for timing, quality and traces. Separate target E:/renderlab-work/n8k/target, -j 2.

## Misses, honestly

- **No variant meets the bar** (every quality measure above VSM at ≤ 1 ms camera-moving).
  - **Map:** 0.94-0.96 ms. It misses the quarry's lit-darkened (263 / 242 against 87 / 88, three times VSM's) and S1's solid flips (48 / 38 against 0-1).
  - **Hybrid with 6 samples:** every measure of every bench above VSM, at 1.51-1.53 ms, about 0.5 ms over.
  - **Exact:** 1.95 ms.
- **Map's camera-moving cost is about twice VSM's estimate** (0.95 against about 0.5 ms). Its static (0.37-0.38) and turning (0.39-0.40) passes are below VSM's 0.50 and 0.95.
- **Caster motion is no faster with Map** (1.92-1.96 ms against Exact's 1.87-1.94). Its page side costs 0.53 ms more (§ Caster motion and small scenes). Hybrid's is 2.37-2.40.
- **On the small benches Map costs more than Exact:** S2 0.95-1.01 ms against 0.35-0.48, S3 0.47 against 0.235 (validation on). Both are above VSM's 0.35 and 0.37 static.
- **Hybrid at walk cap 4 is rejected:** it misses VSM's partial RMS on S2 and S3, and saves only 0.02-0.05 ms on the quarry.
- **Not film quality**; never presented as meeting the film gate. Map and Hybrid are not defaults.
- **The quality bars shift with the receivers.** Lit-darkened on no-RT receivers includes LOD-receiver errors that Exact shares (its own 60/55).
- **Mirrored placements** would flip the resolve's facing bit (none in the benches; not handled).

## Evidence and time

- **Code.**
  - New: shaders/virtual_shadow_lookup.wgsl (the map module: resolve, mips, lookup, queue) and shaders/virtual_shadow_hybrid.wgsl (the queued exact pass).
  - Shared, kept small: `VirtualLookup` and its resources and dispatch in src/virtual_shadow_gpu.rs; the export in src/lib.rs and src/renderer.rs; the `lookup` setting in src/film_accumulation_tests.rs; screen constants in src/virtual_shadow_tests.rs.
  - main's shaders/virtual_shadow.wgsl is unchanged. Its `vsm_mip` pass is now dispatched indirectly from Rust, which changes no output.
- **Evidence** (`E:/renderlab-work/n8k/`):
  - `p1`-`p23`: each lease's script, its `-t` (timing, hashes, memory reports) and `-q` (quality counts and `n8e-scores.json`) roots.
  - `final-*`: the final binary's runs.
    - `final-map`, `final-hybrid` (cap 4 + 6 samples), `final-hybrid-{full,cap4,samples6}`: S1-S3 scores (`S1/scores.json`, `S2|S3/comparison.json`, `n8-comparison.json`, `n8-score.log`, `S1-score.log`, images).
    - `final-mem`: memory reports and WDDM samples per path. `final-screen`: traced screens. `final-t`: interleaved timing.
  - `runs/`: every lease's log and guard samples. `edits/`: the scripted shader edits (e1-e18).
  - `scripts/`: causes, why and cost-counter decoders, `timing2.py`, `memsum.py`, `s1flips.py`, `s23split.py`, the Nsight exporters.
  - `bin/`: the measured binaries and `final.sha256`. The committed sources hash equal to the final binary's.
  - Deleted after scoring and export, and listed in `deleted-raw.txt`: raw dumps over 5 MB (Nsight traces, fixture hit buffers, S1 buffers) and the scored S2/S3 per-frame visibility, 12 GiB in the final cleanup. The S2/S3 references hardlinked from N7a, JSON/CSV summaries and PNGs are kept.
- **Gate.** `cargo test -p renderlab --lib -j 2` exited 0 inside the final lease, on the committed source: 106 passed, 0 failed, 110 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
- **Build and environment.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Validation was off for timing, quality and traces, and on for the traced screen and S1-S3. No game ran, and no window, focus change or capture was used.
- **Leases.** 26 under the shared lock with the 4/2 GiB guard, 3,933 s of GPU time in total, each taken after the other workers' release lines; at least 11 GiB was available at every start. P10's first lease failed on a timestamp validation error (fixed). P21 and the first final lease were aborted by the guard (above).
- **Time (UTC, 2026-11-07).** The work folder was created at 01:29, and the last lease (P23) was released at 05:07. Verification was the 26 leases (66 min of GPU) plus about 25 min of CPU scoring for the final S1-S3 runs. The rest went to implementation, the scripted edits and analysis between leases.
