# RT1c: the HRT default gate

2026-11-07 UTC. Branch `t3/rt1c-rt-default`, from main `dedc068`.
Labels: **M** measured/replayed here; **R** retained evidence, audited here.

Follow-up: [RT1d's fixed visibility filter](rt1d-shadow-filter.md) clears the
disk-source display miss but fails quarry false-shadow/lit-darkened gates,
S2 point-source phases and strict-solid gates. It stays opt-in. The RT-device
default is still raster + Exact; RT1c's fast timing labels remain non-default.

**Do not switch the RT default.** The [revised RT shadow floor](milestone-7.md)
is HRT1, not VSM. Interactive passes the retained shadow-mask and visibility
flicker comparisons against HRT1. However, S2's previously registered **+4-stop
final-image display RMS** loses to HRT1 in all 24 recorded case/phase pairs. A
current optimized-source witness reproduces that miss with a fresh film reference.
The instruction to pass every measure therefore remains unmet.

This is a final-image counterexample, not a newly discovered shadow-mask loss.
At registered exposure the same witness passes. The report keeps those facts
separate; it neither discards the +4 diagnostic nor calls it a registered-exposure
failure. Its cause is not established as an error in the eight shadow directions.

The library and viewer retain raster primaries plus 256-direction Exact on RT
devices. Traced primaries, Interactive and raster/Exact remain settings. No
production code, shader or gate setting changed. Every historical RT speed claim
remains **measured on the non-default fast setup (traced + Interactive)**.
VSM's **27 versus 11** false-shadow gap is recorded and **does not block** this
decision under Islam's new ruling.

## The confirmed counterexample (M)

S2, `camera-0.25px-r0.04`, frame 0 of the outward-motion phase. Native production
size 1920x1080; original scored crop `[704,480,1216,768]`, 512x288 native pixels.
Black environment/background, 0.04-radian sun disk, 1,000 lux, exposure -8,
metallic zero, original roughness and double-sided/thin material declarations.
The retained HRT1 image is the matched 1-spp denoised, post-TSR pre-tonemap image.
No new Unreal capture is needed for this already recorded frame.

| Common image metric, lower is better | Current traced + Interactive | Recorded Unreal HRT1 | Decision |
|---|---:|---:|---|
| Display RMS at registered exposure, 8-bit codes | 5.518361 | 7.346502 | pass |
| Display RMS at +4 stops, 8-bit codes | **14.083180** | **13.665473** | **miss** |

Current HDR RMS against the fresh film reference is .009817005. Native HDR arrays
were previously cleaned, so no new native linear-HDR score is reconstructed from
an 8-bit PNG. The full recorded phase HDR scores below remain available.

The fresh current-source film and Interactive display crops each have **zero
differing RGB channels** from their retained counterparts, at both exposures.
An independent PNG subtraction also reproduces both old frame-0 +4 RMS values
exactly. This is stronger than merely finding a similar error on an older build.
The reference crop also passes the existing byte-equality check against the full
S1 film dispatcher in the GPU test.

The metric is the unchanged `radiance()` in `scripts/bench-contact-fold-score.py`:
multiply linear RGB by `2^stops`, apply Reinhard then sRGB, round to 8-bit codes,
and compute RGB RMS over the complete registered crop. The same transform and
reference apply to both engines. +4 was already part of
[the S2/S3 final-image metrics](bench-motion.md#reference-and-metrics), not added
after seeing this result. No exposure fitting, new crop, background replacement
or receiver exclusion is used.

Verified: `E:/renderlab-work/rt1c/witness/camera-0.25px-r0.04/witness-plus4.png`,
showing fresh film, current Interactive, recorded HRT1 and both absolute-error
images at native crop resolution. Complete current production HDR was captured
before the diagnostic shading pass. This crop proves a counterexample; it does
not certify the rest of the full image.

### Where the display loss lies (M, diagnostic)

| Film visibility acceptance at the witness | Pixels | Interactive +4 RMS | HRT1 +4 RMS |
|---|---:|---:|---:|
| Fewer than all 256 footprint primaries accepted | 21,827 | **32.675122** | **25.664434** |
| All 256 accepted | 125,629 | 6.877316 | 10.234906 |

The net loss concentrates in mixed-footprint/boundary pixels. Interactive is
better on the fully accepted population, on film-lit pixels (.439713 versus
5.297863 codes), and on the partial-visibility population (30.050327 versus
35.337419 codes). These diagnostic subdivisions do not replace the full-crop gate.
They are consistent with primary/coverage aliasing in the complete image and
explain why a passing visibility aggregate can coexist with this display loss.
They do not isolate every contributing shading or reconstruction difference.
Native/Renderlab BRDF parity remains the limitation documented by the original
bench; no M7 shading or temporal code was edited to hide or fix it.

## Retained motion audit (R, CPU replay M)

`scripts/rt1c-hrt-audit.py` reads the original `per-frame.json` files and S1's
`scores.json`, preserving source SHA-256 identities. It checks **1,280 Interactive
and 1,280 HRT1 S2 rows**, **385 + 385 S3 rows**, and four S1 static configurations.
It pools squared errors by each original population count, keeps defect sums and
per-frame maxima, and reports visibility flicker as the original per-frame
residual-error change. Frame sets and flicker availability must match.

All 24 S2 phases, all six S3 phases and all four S1 configurations are retained.
The only failed phase-aggregate measure in this audit is S2 +4 display RMS.
Partial, lit, umbra, narrow, faint, silhouette, wide, contact, solid and contour
RMS; contact loss; leaks; flips; strict-solid defect counts; missing receivers;
false and missed shadows; linear HDR; registered display; and visibility flicker
all pass their paired HRT1 comparisons where available. This statement concerns
the audited measures, not every possible per-pixel maximum or a new full-frame gate.

The exact per-measure table, including defect sums/maxima and every population,
is `E:/renderlab-work/rt1c/recorded-audit/comparison.csv`; `misses.csv` contains all
24 failures. `audit.json` keeps phase values and `identities.json` pins the inputs.

### Every S2/S3 phase

Cells are **Interactive / HRT1**. Flicker is mean visibility residual change,
not final-HDR flicker. Lower is better. S2 point-source labels all invoke the
same production hard-source path, as registered in the original bench.

| Case / phase | Partial RMS | Linear HDR RMS | +4 display RMS, codes | Mean visibility flicker |
|---|---:|---:|---:|---:|
| S2 camera-0.25px-r0 / hold | 0.271445 / 0.377261 | 0.012279 / 0.017785 | 17.067548 / 15.284476 | 0.000970 / 0.076278 |
| S2 camera-0.25px-r0 / out | 0.273299 / 0.378899 | 0.011408 / 0.017013 | 16.573076 / 14.281831 | 0.029989 / 0.073703 |
| S2 camera-0.25px-r0 / return | 0.272201 / 0.373656 | 0.011014 / 0.016590 | 16.071155 / 13.965439 | 0.028957 / 0.067321 |
| S2 camera-0.25px-r0.04 / hold | 0.074709 / 0.149742 | 0.011147 / 0.015294 | 14.721532 / 13.665040 | 0.000362 / 0.059657 |
| S2 camera-0.25px-r0.04 / out | 0.074881 / 0.144318 | 0.010452 / 0.014849 | 14.377052 / 13.522232 | 0.013213 / 0.059435 |
| S2 camera-0.25px-r0.04 / return | 0.075124 / 0.143782 | 0.010145 / 0.014666 | 14.156851 / 13.742022 | 0.012741 / 0.057379 |
| S2 camera-0.5px-r0 / hold | 0.274478 / 0.370748 | 0.012076 / 0.017960 | 16.760670 / 15.242260 | 0.001200 / 0.074714 |
| S2 camera-0.5px-r0 / out | 0.273213 / 0.377448 | 0.011388 / 0.017573 | 16.517855 / 14.910526 | 0.037068 / 0.072033 |
| S2 camera-0.5px-r0 / return | 0.272063 / 0.373011 | 0.010974 / 0.017349 | 16.022568 / 14.474786 | 0.035788 / 0.068194 |
| S2 camera-0.5px-r0.04 / hold | 0.073333 / 0.147767 | 0.011042 / 0.015486 | 14.378476 / 13.648584 | 0.000474 / 0.059243 |
| S2 camera-0.5px-r0.04 / out | 0.075079 / 0.143633 | 0.010460 / 0.015196 | 14.372408 / 13.745360 | 0.016309 / 0.058982 |
| S2 camera-0.5px-r0.04 / return | 0.075016 / 0.143625 | 0.010134 / 0.015131 | 14.130478 / 13.830126 | 0.015707 / 0.057769 |
| S2 camera-1px-r0 / hold | 0.271570 / 0.373153 | 0.011506 / 0.017733 | 16.937956 / 15.182396 | 0.001312 / 0.071253 |
| S2 camera-1px-r0 / out | 0.273026 / 0.376604 | 0.011374 / 0.017515 | 16.344098 / 14.631687 | 0.040354 / 0.071049 |
| S2 camera-1px-r0 / return | 0.271435 / 0.372779 | 0.010889 / 0.017056 | 15.917939 / 14.066482 | 0.038611 / 0.065930 |
| S2 camera-1px-r0.04 / hold | 0.075561 / 0.149912 | 0.010717 / 0.015287 | 14.848481 / 13.811416 | 0.000588 / 0.058410 |
| S2 camera-1px-r0.04 / out | 0.075172 / 0.144475 | 0.010529 / 0.015250 | 14.361932 / 13.630753 | 0.019083 / 0.060335 |
| S2 camera-1px-r0.04 / return | 0.075391 / 0.144569 | 0.010134 / 0.015099 | 14.152182 / 13.803187 | 0.018253 / 0.058253 |
| S2 shadow-step-r0 / hold | 0.271821 / 0.367331 | 0.010602 / 0.018042 | 15.579837 / 14.451436 | 0.000297 / 0.062029 |
| S2 shadow-step-r0 / shadow-out | 0.273075 / 0.372720 | 0.010597 / 0.018290 | 15.600543 / 14.649103 | 0.009761 / 0.063717 |
| S2 shadow-step-r0 / shadow-return | 0.273075 / 0.369469 | 0.010597 / 0.018190 | 15.600543 / 14.587549 | 0.009757 / 0.063069 |
| S2 shadow-step-r0.04 / hold | 0.073883 / 0.137602 | 0.009896 / 0.015387 | 14.018696 / 13.769081 | 0.000125 / 0.053559 |
| S2 shadow-step-r0.04 / shadow-out | 0.074235 / 0.137306 | 0.009881 / 0.015398 | 14.077559 / 13.717891 | 0.003976 / 0.054174 |
| S2 shadow-step-r0.04 / shadow-return | 0.074235 / 0.136968 | 0.009881 / 0.015527 | 14.077559 / 13.863624 | 0.003976 / 0.054018 |
| S3 sun-r0.04 / hold-fast | 0.077448 / 0.254630 | 0.014063 / 0.027458 | 13.989372 / 24.288857 | 0.000000 / 0.124875 |
| S3 sun-r0.04 / hold-slow | 0.078006 / 0.253876 | 0.014159 / 0.026834 | 13.641695 / 24.118333 | 0.000000 / 0.123311 |
| S3 sun-r0.04 / jump-recovery | 0.076084 / 0.250671 | 0.013822 / 0.027876 | 14.204706 / 23.935954 | 0.001058 / 0.127352 |
| S3 sun-r0.04 / rotate-0.25 | 0.078030 / 0.254967 | 0.014156 / 0.026790 | 13.671750 / 24.124745 | 0.002525 / 0.123382 |
| S3 sun-r0.04 / rotate-1 | 0.077901 / 0.253865 | 0.014138 / 0.027117 | 13.851984 / 24.123926 | 0.005041 / 0.123724 |
| S3 sun-r0.04 / static | 0.077495 / 0.255519 | 0.014154 / 0.026736 | 13.676308 / 24.103285 | 0.000000 / 0.123648 |

Defect entries below are also Interactive / HRT1. Contacts, false and missed are pixel-frame sums; other defects are individual-frame maxima.

| Case / phase | Lost contacts | Leaks | Flips | False | Missed | Strict-solid defects | Missing |
|---|---:|---:|---:|---:|---:|---:|---:|
| S2 camera-0.25px-r0 / hold | 288 / 4497 | 0 / 73 | 0 / 283 | 0 / 3712 | 16384 / 19844 | 0 / 283 | 0 / 0 |
| S2 camera-0.25px-r0 / out | 343 / 8223 | 0 / 123 | 0 / 301 | 0 / 5870 | 31727 / 37208 | 0 / 301 | 0 / 0 |
| S2 camera-0.25px-r0 / return | 333 / 7476 | 0 / 74 | 0 / 241 | 0 / 5132 | 30470 / 34929 | 0 / 241 | 0 / 0 |
| S2 camera-0.25px-r0.04 / hold | 0 / 1248 | 0 / 93 | 0 / 175 | 800 / 4840 | 4416 / 10123 | 0 / 2847 | 0 / 0 |
| S2 camera-0.25px-r0.04 / out | 0 / 2498 | 0 / 126 | 0 / 195 | 1752 / 7261 | 9472 / 18440 | 0 / 3077 | 0 / 0 |
| S2 camera-0.25px-r0.04 / return | 0 / 2344 | 0 / 199 | 0 / 159 | 1767 / 6584 | 9562 / 17322 | 0 / 2846 | 0 / 0 |
| S2 camera-0.5px-r0 / hold | 288 / 4089 | 0 / 74 | 0 / 333 | 0 / 3664 | 16480 / 19794 | 0 / 333 | 0 / 0 |
| S2 camera-0.5px-r0 / out | 426 / 8194 | 0 / 103 | 0 / 325 | 0 / 5835 | 31619 / 37095 | 0 / 325 | 0 / 0 |
| S2 camera-0.5px-r0 / return | 460 / 7407 | 0 / 119 | 0 / 329 | 0 / 5327 | 30306 / 34759 | 0 / 329 | 0 / 0 |
| S2 camera-0.5px-r0.04 / hold | 0 / 1271 | 0 / 69 | 0 / 211 | 864 / 4032 | 4256 / 10015 | 0 / 2325 | 0 / 0 |
| S2 camera-0.5px-r0.04 / out | 0 / 2474 | 0 / 136 | 0 / 193 | 1767 / 6980 | 9533 / 18468 | 0 / 3093 | 0 / 0 |
| S2 camera-0.5px-r0.04 / return | 0 / 2375 | 0 / 174 | 0 / 198 | 1755 / 6764 | 9870 / 17328 | 0 / 3065 | 0 / 0 |
| S2 camera-1px-r0 / hold | 224 / 4353 | 0 / 76 | 0 / 234 | 0 / 2901 | 15680 / 19404 | 0 / 234 | 0 / 0 |
| S2 camera-1px-r0 / out | 530 / 8164 | 0 / 77 | 0 / 308 | 0 / 5926 | 31129 / 36582 | 0 / 308 | 0 / 0 |
| S2 camera-1px-r0 / return | 503 / 7446 | 0 / 77 | 0 / 288 | 0 / 5226 | 29767 / 34238 | 0 / 288 | 0 / 0 |
| S2 camera-1px-r0.04 / hold | 0 / 1378 | 0 / 97 | 0 / 149 | 736 / 3916 | 4512 / 10221 | 0 / 2697 | 0 / 0 |
| S2 camera-1px-r0.04 / out | 0 / 2461 | 0 / 108 | 0 / 190 | 1703 / 6965 | 9892 / 18228 | 0 / 2876 | 0 / 0 |
| S2 camera-1px-r0.04 / return | 0 / 2298 | 0 / 165 | 0 / 196 | 1612 / 6362 | 9793 / 17105 | 0 / 2853 | 0 / 0 |
| S2 shadow-step-r0 / hold | 64 / 3243 | 0 / 65 | 0 / 195 | 0 / 2166 | 13600 / 15812 | 0 / 195 | 0 / 0 |
| S2 shadow-step-r0 / shadow-out | 258 / 6746 | 0 / 68 | 0 / 195 | 0 / 4524 | 27788 / 31778 | 0 / 195 | 0 / 0 |
| S2 shadow-step-r0 / shadow-return | 258 / 6681 | 0 / 68 | 0 / 197 | 0 / 4506 | 27788 / 31747 | 0 / 197 | 0 / 0 |
| S2 shadow-step-r0.04 / hold | 0 / 1223 | 0 / 102 | 0 / 135 | 768 / 2118 | 4832 / 9099 | 0 / 1987 | 0 / 0 |
| S2 shadow-step-r0.04 / shadow-out | 0 / 2445 | 0 / 100 | 0 / 135 | 1482 / 4257 | 9611 / 17456 | 0 / 2048 | 0 / 0 |
| S2 shadow-step-r0.04 / shadow-return | 0 / 2437 | 0 / 105 | 0 / 136 | 1482 / 4336 | 9611 / 17408 | 0 / 2030 | 0 / 0 |
| S3 sun-r0.04 / hold-fast | 0 / 6077 | 0 / 123 | 0 / 198 | 640 / 10127 | 5504 / 25054 | 0 / 1122 | 0 / 0 |
| S3 sun-r0.04 / hold-slow | 0 / 5506 | 0 / 128 | 0 / 196 | 224 / 10245 | 5152 / 26271 | 0 / 1131 | 0 / 0 |
| S3 sun-r0.04 / jump-recovery | 0 / 6086 | 0 / 142 | 0 / 231 | 231 / 9753 | 5511 / 25228 | 0 / 1050 | 0 / 0 |
| S3 sun-r0.04 / rotate-0.25 | 0 / 22183 | 0 / 127 | 0 / 198 | 1043 / 41515 | 20495 / 105532 | 0 / 1144 | 0 / 0 |
| S3 sun-r0.04 / rotate-1 | 0 / 23005 | 0 / 126 | 0 / 200 | 1196 / 40785 | 20626 / 102916 | 0 / 1168 | 0 / 0 |
| S3 sun-r0.04 / static | 0 / 5499 | 0 / 130 | 0 / 194 | 288 / 10355 | 5120 / 26120 | 0 / 1157 | 0 / 0 |

For a typical moving disk phase (S2, .25 px/frame out), partial RMS and linear-HDR
RMS pass, yet +4 display RMS is 14.377052 versus 13.522232. Nonlinear display
weighting can rank different spatial distributions of error differently from
linear HDR. Neither measure is substituted for the other.

S3's visibility and final-HDR jump projection both retain their original age
0-32 curves. Interactive's recorded lag90 is zero for both; HRT1 is zero for
visibility and three frames for final HDR. This is a diagnostic projection,
not a claim of zero total image error or a universal ghost-free result.

### S1 static configurations

| Pose / environment | Partial RMS | Lost contacts | Leaks | Flips | +0 display RMS | +4 display RMS |
|---|---:|---:|---:|---:|---:|---:|
| p0-e0 | 0.087840 / 0.165304 | 0 / 34 | 0 / 431 | 0 / 16 | 2.084255 / 2.735305 | 5.471364 / 6.741099 |
| p0-e1 | 0.087840 / 0.161410 | 0 / 29 | 0 / 448 | 0 / 15 | 1.836986 / 12.336992 | 3.599625 / 64.231604 |
| p1-e0 | 0.086842 / 0.177903 | 0 / 26 | 0 / 348 | 0 / 6 | 1.855495 / 2.474870 | 4.854999 / 6.010041 |
| p1-e1 | 0.086842 / 0.175632 | 0 / 26 | 0 / 352 | 0 / 6 | 1.631093 / 12.566350 | 3.180769 / 65.672037 |

S1 has no recorded motion sequence in this evidence set. Its environment rows
also retain the documented native black-background versus visible-environment
mismatch, so their HDR/display numbers are not a clean isolated BRDF comparison.
No missing S1 motion is counted as a pass.

## Quarry: the new comparator is applied (R)

The two current-policy static poses from [RT1](rt1-interactive.md) and
[RT1b](rt1b-footprint.md) beat HRT1 on every principal shadow measure below.
No previously deleted RT1/RT1b raw capture is needed or restored for this table;
it reuses their retained scores and the unchanged R1/native reference identities.

| Measure, pose 0 / 1 | Previous/current fast setup | Unreal HRT1 |
|---|---:|---:|
| Partial RMS | .207157 / .208537 | .370873 / .371257 |
| False shadows | 27 / 26 | 743 / 758 |
| Missed film shadows | 9,245 / 9,166 | 23,931 / 23,789 |
| Solid flips | 0 / 1 | 3,306 / 3,215 |
| Umbra leaks | 0 / 0 | 2,718 / 2,616 |
| Lit darkened | 0 / 2 | 724 / 736 |
| Lost umbra edges | 0 / 0 | 2,565 / 2,461 |
| Missing covered receivers | 0 / 0 | 20 / 26 |

The VSM false counts remain 11 / 9. They are informative and non-blocking for the
RT HRT floor. This unit does not call that difference a reason to retain Exact.

## Cost, memory and missing work

The retained optimized M6-close/M5-DL runs report:

| Recorded workload | Fast RT rendered span, ms | HRT whole GPU frame, ms |
|---|---:|---:|
| Static | .867 | 1.743-1.753 |
| Camera | .868 | 1.764-1.766 |
| Sun | .867 | 1.738-1.751 |
| Caster | .865 | 1.776-1.778 |

These are **measured on the non-default fast setup (traced + Interactive)**.
[M6-close](m6-acceptance.md) preserves the
scope and clean-visit selection limits: unfiltered visits reached 1.4-2.8 ms,
and the Vulkan rendered-span and Unreal D3D12 whole-frame boundaries differ.
The isolated shadow increment was <=.276 ms versus HRT's .626-.655 ms.
These retained numbers are not a new current-source quiet-window certificate.

[M1's retained inventory](memory-m1.md) reports allocator reserved / WDDM usage
**2,641.2 / 2,716.3 MiB**, against Unreal HRT **5,410 MiB**, at both quarry poses.
[M7-U0](m7-u0.md) also retains HRT static/camera/sun timing and memory with effective
configuration readback; its alternating camera/sun motion differs from M6-close's
continuous tracks. It cannot silently fill a missing quality/flicker phase.

No cost from the game-time witness is used. A fresh optimized cost/memory check
would need a quiet GPU after Islam finishes playing. That work and missing S1 /
quarry motion-quality captures do not change the rejection established above, so
the broad missing-phase Unreal batch was not launched. This is the requested
failure outcome, not confirmation of all remaining motion gates.

| Requested gate | RT1c status |
|---|---|
| Quarry static, both poses, principal shadow measures against HRT | retained pass |
| S1 static principal measures | retained pass; environment-radiance mismatch disclosed |
| S2/S3 all recorded motion/hold/return/jump phases | mask, HDR and visibility-flicker pass; **S2 +4 final-image RMS miss** |
| Current optimized-source counterexample | **confirmed**, fresh film and current display images match the retained witness |
| S1 motion, quarry motion quality/flicker, final-HDR flicker | not newly certified; missing data are not passes |
| Current-source optimized cost and memory against HRT | quiet rerun not performed; no game-time speed claim |
| RT-device default | unchanged: raster + Exact |

## Checks and game handling

Before the game started, `cargo test -p renderlab --lib --release -j 2` exited 0:
**123 passed, 0 failed, 118 ignored**, test execution .54 s. The cold optimized
build/check took **181.994 s**. `cargo fmt --all -- --check` exits 0 (1.765 s).
Release Clippy exits 0 (28.660 s) with `-D warnings` and the two existing baseline
allowances `clippy::nonminimal_bool` and `clippy::items_after_test_module`.
This is not a claim of an allowance-free main. All three new Python scripts parse;
the recorded audit took .476 s, and the independent PNG replay matches exactly.
No Rust implementation or shader changes need a second measurement build.
CPU refusal checks also confirm that the relaxed-hold launcher rejects a lease
longer than two minutes and rejects an unrelated command before taking a lease.

The initial current-source witness was stopped when the guard saw `cs2.exe`:
13,442 MiB free at preflight, minimum 12,437 MiB; 7.506 s process duration.
It produced no captures and supplies no usable timing. Its lease was released.
The hard hold was acknowledged and no new GPU work started during that hold.

After Islam explicitly relaxed the hold, the quality-only launcher preserved the
shared lock, foreign-renderer check, 4/2 GiB guard and hidden offscreen process.
It recorded the specifically authorized CS2 process separately, marked timing
contended, and limited leases to two minutes. The successful single-frame witness
took **29.713 s**, test 26.15 s, with **7,639 MiB minimum free host RAM**. It did
not crawl. Eight production timing replays were internal to the existing harness;
their timings are **invalid for speed claims** during play.

Successful quality lease: **2026-11-07T14:41:21.004799+00:00 to 2026-11-07T14:41:50.718493+00:00** (see the exact launcher
record for subsecond endpoints); optimized test executable SHA-256
`03bc1fa4077ac24f36ce4c7842e332639e77c5770224ee3a34c566d15e8270fa`.
`E:/renderlab-work/rt1c/runs/` and `gpu-windows.json` retain actual ownership,
resource samples, contention and command identities. No owned Cargo process ran
in either lease. No window, viewer, desktop/game capture, focus/input automation,
Nsight or persistent background job was used. Coding/analysis was not separately
stopwatched; the measured verification durations above are not frame costs.

## Evidence and cleanup

Keep the E:-local manifests, `recorded-audit/`, `png-replay.json`,
`witness-score.json`, `witness-populations.json`, inspected PNGs and all guard/check
logs. `scripts/rt1c-hrt-audit.py` reproduces the phase audit; `rt1c-witness.py`
scores the fresh witness using the retained native PNG and unchanged display
transform. Neither script starts GPU work. `rt1c-quality-run.py` is only the
explicitly relaxed-hold quality launcher, not a performance launcher.

After all work, `cargo clean --target-dir E:/renderlab-work/rt1c/target` exits 0,
removing **1,588 files / 674.3 MiB**. The owned target directory is absent.
No Rust or shader source changed after the successful release checks. The prior
64 RT1/RT1b raw captures remain deleted, as Islam reported.

Automatic approval review rejected the new raw-file deletion command as
**"blocked by policy"**. No alternate deletion mechanism was used. These three
captures remain for Islam to remove after reviewing the summaries:

| Exact path | Bytes |
|---|---:|
| `E:/renderlab-work/rt1c/witness/camera-0.25px-r0.04/film-full.rgba32f` | 33,177,600 |
| `E:/renderlab-work/rt1c/witness/camera-0.25px-r0.04/Interactive/0000-full.rgba16f` | 16,588,800 |
| `E:/renderlab-work/rt1c/witness/camera-0.25px-r0.04/reference/pose-0.film.bin` | 7,077,888 |

Total **56,844,288 bytes**. `cleanup-raw-inventory.json` records each SHA-256.
The small cropped candidate, receiver and visibility buffers also remain for CPU
replay. Removing the film array requires a reference recapture before the fresh
witness scorer can run again. Original S1-S3 and native assets are untouched.

## Proposed decisions rows

`docs/milestones-5-6-decisions.tsv` stays untouched. Proposed text:

- **RT1c comparator:** apply Islam's RT HRT floor; the quarry's 27/26 false shadows
  beat HRT1 743/758. The VSM 11/9 gap remains recorded and non-blocking.
- **RT1c default held:** audited S1-S3 principal shadow and visibility-flicker
  measures pass HRT1, but S2's registered +4-stop final-image RMS loses in every
  phase; current optimized witness 14.0832 versus HRT1 13.6655, with exact display
  reference identity. Retain raster + Exact default; see this report. All RT speed
  claims remain measured on the non-default fast setup (traced + Interactive).
- **RT1c performance evidence:** quality-only capture during explicitly relaxed
  game hold; timing contended and invalid for speed claims. No new quiet optimized
  cost/memory certificate or missing motion pass is inferred.

## Quiet cost and memory follow-up (M, 2026-11-07)

Branch `t3/rt1c-quiet`, main base `79a1966`. Measurement only: **no default switch**.
This section supersedes the earlier "quiet rerun not performed" cost/memory status;
it changes neither the S2 image miss nor any missing quality/flicker gate.
The prior three raw dumps were deleted by Islam before this unit.

**All fast RT results below are measured on the non-default fast setup
(traced + Interactive).** The comparator is the actual RT-device default,
**raster + Exact (256 directions)**, asserted and explicitly pinned in the test.
Both use full-resident original quarry geometry, both registered poses, native
1920x1080, 1x, full texture residency/16x anisotropy, identical lighting/environment,
RTX 4070 SUPER, NVIDIA 617.14, Vulkan/wgpu 30.0.1, optimized release test binary,
validation off. Production renderer and shader code are unchanged.

The M1 harness starts a fresh process and uploads the asset once per configuration.
Each pose has eight visits, each with 32 warm frames then 8 static, 16 sun,
48 camera and 48 caster frames. Sun moves 0.5 mrad/frame; camera and placement 0
move 1 mm/frame, as in the M1/M6-close tracks. Actual frame IDs join timestamps
to phases. There are no image/readback hashes or diagnostic passes in the timed
block. Each configuration retains **128 / 256 / 768 / 768** static/sun/camera/caster
samples, plus all M1 allocation/block/usage/lifetime reports and WDDM observations.

### Quiet render spans and visit ranges

Values are medians of the **16 pose/visit medians**; brackets retain the full
visit range. These spans run from first rendered pass through tone mapping,
excluding presentation, CPU preparation and external uploads. Pass medians are
not summed to obtain the span. HRT costs are the recorded M6-close whole-GPU-frame
process-median ranges, not a new paired Unreal run. Their D3D12 boundary and
sampled frame/pose intervals differ; the motion rates and scene are the registered
comparison workload.

Any uploads or AS preparation before the first rendered timestamp are outside
these spans. The ratios are descriptive interval comparisons, not complete
application-throughput measurements.

| Phase | Traced Interactive span [range], ms | Raster Exact span [range], ms | Recorded HRT frame, ms | HRT / fast median |
|---|---:|---:|---:|---:|
| Static | .877056 [.862208, .924160] | 14.485248 [14.462368, 14.507920] | 1.743-1.753 | **1.987-1.999x** |
| Camera | .870656 [.866304, .913408] | 14.454016 [14.431744, 19.151760] | 1.764-1.766 | 2.026-2.028x |
| Sun | .878336 [.864768, .923136] | 14.486784 [14.465408, 14.505472] | 1.738-1.751 | **1.979-1.994x** |
| Caster | .877312 [.868352, .921600] | 14.481408 [14.470144, 14.496768] | 1.776-1.778 | 2.024-2.027x |

All **16/16 traced visits in every phase** meet the carried M5-DL RT clean screen:
median primary <=.60 ms and shade <=.50 ms. Therefore its clean and unfiltered
results are identical; no slow traced visit was excluded. The raster comparator
also retains every visit. Its full triangle raster costs are intrinsically above
the traced-only primary threshold, so that threshold is not misused to reject
its observations as contamination. The raster camera outlier remains visible.

The fast medians are 16.49-16.60x below the current default's spans on this workload.
They **do not establish a robust 2x win against HRT**: static and sun medians
slightly miss 2x, and the slowest retained fast visit in every phase is below 2x
(HRT / fast visit bounds overall 1.883-2.048x). No threshold or sample was changed
to turn these narrow misses into passes. This is cost evidence, not a new combined
quality/performance acceptance or an engine-wide throughput certificate.

| Phase | Fast fused primary + shadow, ms | Fast shade, ms | Default raster receiver, ms | Default Exact compute, ms | Default shade, ms |
|---|---:|---:|---:|---:|---:|
| Static | .500736 | .346224 | 2.646784 | 8.517336 | 3.286784 |
| Camera | .498688 | .345464 | 2.647040 | 8.477720 | 3.292152 |
| Sun | .500480 | .345680 | 2.647256 | 8.517144 | 3.286272 |
| Caster | .501760 | .346000 | 2.648064 | 8.512088 | 3.287040 |

Tone mapping is .013312 ms in both configurations. Interactive has no separate
shadow pass; its fused interval is not the historical .276 ms shadow increment.
This run does not remeasure that isolated increment.

### M1 memory, both poses

MiB = 2^20 bytes. Allocator values agree exactly across all 16 warmed reports
per configuration. WDDM is sampled per-process Dedicated Usage, comparable to
the recorded Unreal DXGI local usage, rather than allocator slack or total
desktop VRAM. Warm and whole-process observed peaks agree.

| Configuration | Allocated MiB | Reserved MiB | WDDM warm/process peak MiB | WDDM warm / total samples | Recorded HRT MiB |
|---|---:|---:|---:|---:|---:|
| Non-default traced + Interactive | 2,509.556 | 2,641.247 | 2,716.297 | 3 / 11 | 5,407-5,410 |
| Default raster + Exact | 2,396.351 | 2,506.055 | 2,572.980 | 29 / 32 | 5,407-5,410 |

The fast setup uses **143.316 MiB more WDDM memory** than the current default
(135.191 MiB more allocator reservation). Both remain below recorded HRT usage.
The short fast process supplies only three warm WDDM samples; these observations
are not instantaneous peak bounds. Requested cadence was 250 ms, with the normal
PowerShell/PDH sampling overhead retained.

### Reservation, verification and handoff

Explicit quiet reservation: 15:46:37-15:51:37Z. Actual shared-lock run:
**15:46:53.409429-15:47:54.364870Z**, about **61 seconds**, released immediately
after the two GPU processes. The line `rt1c quiet-cost-memory window released`
handed the window to TR0; its `tq1` acquired the next lease at 15:47:56Z.
There were **zero compiler, game or foreign-GPU-process samples** throughout;
minimum free host RAM was 11,436.543 MiB. The 4 GiB start / 2 GiB abort guard and
`--no-build-contention` were active. No owned Cargo ran inside the lease, and no
window, viewer or desktop capture was opened. No further GPU work is required.

Both GPU tests exit 0 (13.75 s traced, 40.26 s raster). The wrapper exits 1 because
CPU inventory processing initially rejected the previously unlisted raster
`depth` allocation **after both GPU processes had completed**. It is the renderer's
Depth32Float target, now catalogued from its creation/retirement site. All CSVs
and the raster WDDM summary were recovered from the saved reports, CPU-only;
the original wrapper failure is retained. No renderer rerun or new quiet slot
was used to repair this catalogue omission.

Release library check: **123 passed, 0 failed, 121 ignored**, 149.429 s including
the cold build, test execution .50 s. Formatting and release Clippy pass with
the previously documented two baseline allowances (25.325 s combined). All
measurement Rust was checked before the lease; later edits affect only Python
cataloguing/scoring and this report. Binary SHA-256:
`3d21dd4cf06de85881c8fd5e3a543f681a5a40e58f9fc8692bb4561d93649c42`.

Evidence: `E:/renderlab-work/rt1c-quiet/summary.json`, `visits.csv`,
`m1/{rt,rt-raster-exact}/` (all raw timestamp rows, allocation reports/CSVs,
WDDM samples, manifests and stdout), and `runs/quiet-cost-memory.json`.
`scripts/rt1c-quiet-score.py` recomputes the summary without GPU work. No large
image/raw dump was produced; the prior deleted captures remain deleted.

After measurement and CPU summary recovery, `cargo clean --target-dir
E:/renderlab-work/rt1c-quiet/target` exits 0: **1,588 files / 674.8 MiB** removed,
owned target absent. Cleanup ran after TR0's quiet release, during the published
G1 functional gap; it did not run inside a quiet timing lease.

Proposed decisions-row text (TSV unchanged): **RT1c quiet M1 measurement** —
traced Interactive spans .871-.878 ms versus raster Exact 14.454-14.487 ms;
all 16 traced visits qualify. Recorded HRT ratios are 1.987-1.999 static,
1.979-1.994 sun, 2.026-2.028 camera and 2.024-2.027 caster; no robust all-phase 2x
certificate. WDDM 2,716.3 versus default 2,573.0 and HRT 5,407-5,410 MiB.
Measurement only; no default change and no quality miss waived.
