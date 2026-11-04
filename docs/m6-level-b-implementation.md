# M6 Level-B implementation and verification

2026-11-05 local (2026-11-04 UTC), based on `d021306`. Islam expanded the initial test-only probe authorization to the approved [Level-B unit](m6-level-b.md), steps 1–5. Quality passes; the visibility and complete-frame time targets miss. This is a retained quality checkpoint, not a 2x acceptance. No push. The separate 8-ray layouts below were superseded by the fused pass of the [Interactive cost hillclimb](m6-interactive-climb.md), whose output is byte-identical.

## Change

`Renderer::set_soft_shadows(SoftShadows::Exact | Interactive)` selects sampling on traced lit 1x disk-shadow frames. `Exact` remains the default. `soft_shadows()` reads the preference; `FrameReport::soft_shadows` reports the frame’s estimator. Raster, hard, unshadowed and diagnostic frames report Exact, preserving the preference for later traced frames. Both traced pipelines compile in the existing reload transaction.

Interactive traces directions 0–7 of the unchanged R2/shift sequence, scales visible rays by 32 and preserves skipped/uncovered output 256. The retained layout is one lane × eight rays, 8×4 receivers per 32-lane workgroup. Exact retains its eight-lane, 2×2 tile. One pipeline override selects 8 or 256 directions. No filter, history, target or binding was added; workgroup counters grow from 16 to 128 bytes. The three-layout screen and mask diagnostics remain test-only.

```rust
renderer.set_primary_visibility(renderlab::PrimaryVisibility::Traced)?;
renderer.set_soft_shadows(renderlab::SoftShadows::Interactive);
```

## Cost probe and limiter

RTX 4070 SUPER 12 GB, Ryzen 7 7800X3D, NVIDIA 617.14, Windows/Vulkan, release test binary, validation off. Registered quarry: 12 full-resident draws, 13,034,903 triangles, 1920×1080 at 1x, 16x anisotropy, RLE2 environment, exposure −14, 0.04 rad disk. Each process warms 24 repetitions, then rotates dispatch order for 24 paired repetitions over the same traced pose-0 receivers. Medians [min, max], ms:

| Layout | Process 1 | Process 2 | Process 3 |
|---|---:|---:|---:|
| 8 lanes × 1 ray | 0.5295 [0.5266, 0.9108] | 0.5305 [0.5266, 1.1047] | 0.5292 [0.5272, 1.3194] |
| 4 lanes × 2 rays | 0.3959 [0.3886, 0.9776] | 0.3960 [0.3889, 0.9763] | 0.3952 [0.3887, 0.5833] |
| 1 lane × 8 rays | 0.3427 [0.3380, 0.3473] | 0.3429 [0.3386, 0.5345] | 0.3425 [0.3375, 0.9096] |
| Exact 256 rays | 7.8531 [7.5922, 10.4707] | 7.6449 [7.5932, 10.7374] | 8.0169 [7.6120, 9.5489] |

Three processes × 24 samples/layout = 72 timed dispatches/layout, plus a final 24-pair diagnostic screen on the integrated source. The best layout’s process medians are 0.3425–0.3429 ms: below the 0.45 ms stop threshold and above the 0.30 ms budget. **Miss recorded; no quality reduction or alternate ray count was attempted.**

**Named limiter: BVH traversal on short per-receiver ray batches, with substantial lane-scheduling/receiver-setup overhead.** In the final paired screen, omitting `rayQueryProceed` leaves 0.0724 ms for the 1-lane layout versus 0.3426 ms with traversal (79% removed). The corresponding 8-lane diagnostic is 0.3046 ms versus 0.5283 ms: receiver work and dispatch shape matter strongly at only eight rays. The traversal-memory-locality interpretation from the earlier exact hillclimb is consistent with this screen; there are no hardware counters here, so cache attribution remains an inference.

## Exactness and recovery

With active Khronos validation, all three layouts equal `256 − 32 × popcount(mask[0] & 255)` over the complete image. A mask bit of one means blocked. All 256 mask bits also reconstruct the diagnostic Exact count. These are real atomic hit masks using the unchanged diagnostic operation from `e1c7822`, regenerated on traced receivers; they are not synthesized from counts.

- Quarry: 3 poses × seeds 0–3; both fixtures: 5 poses × seeds 0–15 each. All 172 pose/seed cases, three layouts each, are bit-exact.
- The public Interactive complete frame equals that replay. Quarry static repeats and the return image/counts match for all four seeds. Fixture return counts match all sixteen seeds; complete seed-zero fixture HDR returns match.
- Exact counts and HDR match the frozen traced-primary acceptance byte for byte at all three quarry poses. World and normal files match too.
- Invalid WGSL and missing lit seams retain the working Interactive frame; tint reload applies and restore is byte-identical. The mode survives reload. Raster fallback reports Exact.
- Removed fixture blockers leave every visibility count at 256; disabling shadows releases fresh targets. Every accepted validation run reports zero errors, no missing layer and active_layer=1. Existing unused-vertex-attribute warnings remain.

## Traced Level-B gate

`support/fresh_gate.py`, `fresh_gate_metrics.py` and `fresh_png.py` are byte-identical to `e1c7822`. The Level-B `rho`, `bounds` and `check` functions are AST-identical to the approved design’s `gate_b.py`; population/statistic code is retained in `fresh_level_b_metrics.py`. The 16,384-ray reference and seeds 65–80 are reused unchanged from primary acceptance b12. Native masks are rescored on that reference without rewriting their evidence. The approved U-bound rule is recomputed from the native masks on the traced reference, exactly as the design specifies; S-bound formulas and constants are unchanged.

Domain is the frozen camera-facing geometric N·L > 0 rule. Poses 0/1/return have 346,517 / 345,882 / 346,517 covered receivers and 334,693 / 334,058 / 334,693 included receivers. Each excludes 11,824 covered receivers; terminator policy is not silently folded into error.

Every check passes every pose and seed. The table gives the largest result, its limit at that case, and passing case counts. Per-case limits and full native rescoring are in `gate-final/gate.json`.

| Check | Largest result | Limit at that case | Passes |
|---|---:|---:|---:|
| U1 partial | 0.0817509 | 0.3695290 | 12/12 |
| U2 narrow | 0.0811366 | 0.4024059 | 12/12 |
| U3 wide | 0.0833233 | 0.3242470 | 12/12 |
| U4 isolated | 0.0816930 | 0.4446006 | 12/12 |
| U5 silhouette | 0.0844185 | 0.3324355 | 12/12 |
| U6 faint | 0.0557827 | 0.0920938 | 12/12 |
| U7 lit | 0.0000000 | 0.0066497 | 12/12 |
| U8 occluded | 0.0000000 | 0.1996024 | 12/12 |
| U9 contact | 0.0430885 | 0.3323477 | 12/12 |
| S1 solid flips | 0.0000000 | 0.0000000 | 12/12 |
| S2 lost contacts | 0.0000000 | 0.0000000 | 12/12 |
| S3 mean partial | 0.0021766 | 0.0036403 | 12/12 |
| S3 mean narrow | 0.0020401 | 0.0047628 | 12/12 |
| S3 mean wide | 0.0026201 | 0.0050587 | 12/12 |
| S3 mean faint | 0.0034197 | 0.0054383 | 12/12 |
| S3 mean isolated | 0.0047483 | 0.0109102 | 12/12 |
| S4 contour ρ | 1.2669030 | 1.5043126 | 12/12 |
| U10 motion RMS | 0.0998440 | 0.2622251 | 8/8 |
| S5 static / return | bit-identical | bit-identical | all |

Native pose-0 light-facing partial RMS is VSM 0.369529 and HRT 0.390697 against this traced reference; the GPU candidate’s largest partial RMS is 0.081751. All nine native population statistics at all three poses and native motion are retained in the JSON, including receiver exclusions.

Fixture S1–S4 also pass all 160 cases. Their analytic disk oracle is unchanged from `e1c7822`, evaluated at the traced plane points. Production spread seeds 65–80 were regenerated there because raster/traced fixture counts differ slightly. No threshold was revised. Empty fixture populations remain explicitly n=0.

| Fixture | Max partial RMS | Max contour ρ | Max motion RMS | S1–S4 |
|---|---:|---:|---:|---|
| thin | 0.105059 | 1.156717 | 0.181239 | 80/80 |
| stress | 0.066261 | 1.208886 | 0.077466 | 80/80 |

## Inspection

Verified full images at registered and +4-stop exposure, visibility and signed errors ×8 on one scale, against reference, Exact, VSM and HRT. Viewed `images/pose-0-sheet.png`, `pose-1-sheet.png`, full `pose-0-interactive-1x.png`, `pose-2-interactive-16x.png`, candidate/native full visibility and error maps, `contacts-rubble-crevices-sheet.png`, `named-33px-error-crops.png` and `fixtures-sheet.png`. The 33×33 crops use the seven frozen coordinates. Sheets also cover outer silhouettes, rubble, wall crevices and isolated cliff-top partials. The original/move/return frames were inspected; `interactive-motion.gif` and `motion-sheet.png` retain the flip-book.

Penumbra grain is two-signed and confined to shadow transitions. Contacts, rubble shadows, crevices, wedge and silhouettes remain complete. Native masks show the existing one-signed losses and HRT dark points; no additional candidate-only defect class was found. Thin strip, contact sliver, specks and rotated wires remain present. G7-B passes for the registered 1 cm motion, not an unmeasured long path or moving sun.

## Complete-frame timing and misses

Three independent processes × 24 paired triplets after 24 warmup triplets, with rotating hard/Exact/Interactive order and alternating registered poses. Frames are pipelined with no per-frame drain; GPU timestamps are collected by frame ID. All samples are retained. Each cell is a median [min, max] in ms.

| Pass / mode | Process 1 | Process 2 | Process 3 |
|---|---:|---:|---:|
| Interactive trace | 0.3232 [0.2945, 0.3336] | 0.3177 [0.2949, 0.9120] | 0.3227 [0.3076, 0.3365] |
| Interactive visibility | 0.3593 [0.3541, 1.2024] | 0.3573 [0.3524, 1.0219] | 0.3596 [0.3536, 0.3670] |
| Interactive shading | 0.3976 [0.3909, 0.8076] | 0.3907 [0.3860, 1.0276] | 0.3964 [0.3876, 0.4030] |
| Interactive tone | 0.0496 [0.0470, 0.0532] | 0.0492 [0.0461, 0.0532] | 0.0491 [0.0461, 0.0532] |
| Interactive span | 1.1469 [1.1255, 2.2209] | 1.1308 [1.1049, 1.9984] | 1.1420 [1.1278, 1.1608] |
| Hard span | 0.9605 [0.9392, 1.7911] | 0.9530 [0.9403, 1.4860] | 0.9597 [0.9395, 1.8000] |
| Exact visibility | 8.5793 [7.6008, 10.2413] | 9.4376 [7.6123, 10.5242] | 7.6174 [7.5986, 9.7433] |
| Exact span | 9.5606 [8.3676, 11.2070] | 10.3047 [8.3828, 11.6106] | 8.8096 [8.3773, 10.5491] |

Median paired Interactive/Exact span ratios are 0.1208 / 0.1111 / 0.1296; Interactive/hard ratios are 1.1981 / 1.1827 / 1.1934. Each ratio has 24 paired observations per process.

The Exact control moves between the known fast (~7.6 ms) and slow (~9–10 ms) visibility levels, sometimes within a process. As a descriptive split only, Exact <8.5 ms marks fast-control pairs: processes have 10 / 1 / 17 fast and 14 / 23 / 7 slow pairs. In process 3, Interactive visibility medians are 0.3592 fast and 0.3600 slow, with spans 1.1422 and 1.1408. The single fast pair in process 2 contains an Interactive outlier (1.9186 ms span). No bin is discarded or used to claim a pass.

**Misses:** the integrated visibility medians 0.3573–0.3596 ms miss ≤0.30 ms; spans 1.1308–1.1469 ms miss ≤1.10 ms. Shading medians 0.3907–0.3976 ms meet ≤0.40 ms, and trace medians 0.3177–0.3232 ms meet the broader ≤0.40 ms target but exceed the design’s 0.315 ms estimate.

The stored [native Unreal timings](unreal-native-shadow-mask.md) are UE 5.7.4 CL51494982, D3D12, 1920×1080, 1x/no AA, same quarry/light subset: VSM 1.59650 [1.48620, 2.61440] ms and HRT 2.14975 [1.66750, 3.45180] ms, 64 samples each. Dividing their medians by these Interactive process medians gives 1.39–1.41x VSM and 1.87–1.90x HRT. **Both 2x floors miss.** Unreal was not relaunched or paired contemporaneously; only the three Renderlab modes are paired in-process. This is the bounded direct-light/shadow subset, not whole-engine parity.

## Evidence, checks and time

Local evidence root: `D:/Coding/renderlab-work/m6-level-b-probe/`. `run.py` uses atomic mkdir of the shared GPU lock and owner.json, preflights games/Unreal/foreign renderer jobs and ≥8 GiB RAM before every child, monitors contention, starts hidden offscreen children and verifies its token before releasing. All 27 lock windows (including eight builds) were released; final release **2026-11-04 21:04:12.119106 UTC**. Total held time 466.63 s. No game/Unreal/foreign renderer was detected; lowest accepted preflight RAM was 11.124 GiB. Background desktop applications remained open, so occasional outliers are retained. There was no window, input automation or desktop capture.

Builds use only `D:/Coding/renderlab-work/m6-level-b-probe/target`, `cargo -j 2`. Final binary `builds/b8/probe-tests.exe` SHA256 `ae77a450bc15214242c3aea6201c8f645ace050648b72833a5f454b4a49e9158`. b5 timings remain valid: b6–b8 changed only test harness coverage, and b7’s 36 quarry image/count/receiver files match b5 exactly. Runtime sources match across those builds.

- 35 affected CPU shadow tests pass (26 GPU tests remain explicitly ignored in that invocation).
- Renderer release Clippy `--lib --tests -D warnings`, formatting and diff checks pass.
- Frozen gate-source byte hashes and copied Level-B function ASTs were checked. All added Python modules parse.

**Renderer implementation/analysis:** 20:43:20–21:07:20 UTC, 24.0 minutes elapsed, interleaved with verification; hands-on editing time was not separately stopwatched. Reading/grounding preceded that interval; report preparation followed. **Measured verification:** eight release test build attempts total 289.52 s (115.68 s cold; one compile error); 19 GPU child processes total 147.98 s, including two failed fixture harness attempts with missing evidence directories; Clippy 21.53 s; quarry/native gate 17.94 s in the first run plus 18.19 s in the final run; fixture gate 11.60 s; final CPU tests 0.10 s, format 1.06 s, diff 0.08 s. Image generation/inspection was not separately timed. Overlapping work must not be subtracted from the implementation interval as if it were isolated editing time.

Reproduce the two gates with `py -3 -I support/fresh_level_b.py --help` and `fresh_level_b_fixtures.py --help`. Manifests, commands, raw samples, preflights, process SHA256 identities, source snapshots and every acquire/release timestamp are retained under the evidence root. Test entries are under `renderer::fresh_shadow_tests::primary_ray_gpu_tests::soft_shadow_probe_tests`: `gpu_soft_shadow_cost_probe`, `soft_shadow_replay_tests::gpu_soft_shadow_mask_replay`, and `soft_shadow_acceptance_tests::{gpu_soft_shadow_traced_acceptance,gpu_soft_shadow_complete_paired_timing}`. Use the lock runner for every GPU invocation; set math-library threads to two for Python scoring.

The approved scope ends here. No filter, history, six-ray fallback or further performance hillclimb was added. Moving lights/blockers, long camera paths, rotation, other scenes and 4x remain outside this checkpoint.
