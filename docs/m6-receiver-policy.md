# M6 R1: receiver policy and radiance references

2026-11-05. Test-only, base `eff3553`, branch `t3/m6-receiver-policy-tests`. No production files/defaults changed. Committed locally; no push. Binding scope and stop rules: [film gap §§6–7](m6-film-gap.md). [F0](m6-film-reference.md) and [the exact-quality design](m6-exact-quality-fast-shadows.md) remain frozen evidence.

**Magnitude verdict: passes the frozen per-population difference gate.** This authorizes no production policy change. The viewed-side variant is an estimand change presented for Islam’s decision and is **not adopted**. The production default remains Exact with its existing receiver policy.

**Convergence: the replay does not trigger the frozen revision rule.** The tables below give the measured N=1–16 fits and recalculated ideal accumulation predictions. No AA implementation or temporal/motion acceptance is claimed.

## What was measured

R1a (`magnitude`) reconstructs the shadow receiver from hit-triangle barycentrics, retains the light-side geometric normal, and uses ε = max(10 µm, 16 ulp(maximum absolute world-space hit-triangle vertex coordinate)) for both normal offset and ray-distance t_min. R1b (`viewed`) changes only the normal’s sign to the viewed side. Every tested material is treated as opaque. A production adoption would require an explicit material thin/opaque contract; no lighting-model data or sidedness contract was added to production.

Each film reference uses F0’s 256 randomized 16×16 footprint strata, independently salted 256-ray Exact visibility per sample. R_F accepts the centre placement and depth within 2%; R_L and R_D include all primary samples and silhouettes. R_D uses the identical material/BRDF, visibility and exposure, with the environment contribution and background set to black. This is Renderlab direct radiance, not a path-traced GI reference.

The first 16 saved samples form a Latin permutation across the 16×16 footprint, rather than F0’s first-row prefix. Every original stratum and its seed are retained in the complete 256-sample reference. Visibility is saved as u16 count/256, with 65535 indicating a rejected surface sample; full and sun-only HDR are RGBA16F. Saved samples total about 597 MB per 1080p pose/policy (including sun-only HDR). R_F uses F0’s weighted interleaved split-half σ_F. Radiance sums change accumulation order; their floating-point roundoff is reported by the minimal replay.

A fresh fixed-10 µm replay supplies R_D, which F0 did not retain. Its visibility, accepted counts and R_L are checked against the immutable F0 before using it. Centre Exact captures stay under the production receiver policy, so the comparison uses the frozen original Exact rather than quietly re-baselining it.

Hardware/workload: RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan with active Khronos validation. Registered quarry: twelve draws, 13,034,903 triangles, 1920×1080, 1x, 16x anisotropy, RLE2 environment, −14-stop exposure, 0.04-rad sun disk. Pose 1 is +1 cm X; pose 2 is an independent pose-0 process. These are reference-generation runs with readbacks and validation, not performance measurements.

## Analytic fixtures

All 24 acne cases pass: flat and convex faceted roofs, translated along all axes by 1 km / 10 km, at 80°, 85° and 89° light incidence, for both variants. Every case has 35,328 covered pixels, zero false shadows and zero visibility error. A 0.001-rad disk keeps every direction on the illuminated side at 89°; convex roof facets slope ±0.002. The complete images were inspected.

| Fixture | Policy | Partial n | RMS | σ_F RMS | Max error | Count mismatches | Verdict |
|---|---|---|---|---|---|---|---|
| sawtooth | magnitude | 13632 | 0.000265 | 0.022934 | 0.001678 | 0 | PASS |
| sawtooth | viewed | 13632 | 0.000267 | 0.022610 | 0.001678 | 0 | PASS |
| thin | magnitude | 933 | 0.000408 | 0.004714 | 0.001456 | 0 | PASS |
| thin | viewed | 933 | 0.000408 | 0.004714 | 0.001456 | 0 | PASS |
| stress | magnitude | 2296 | 0.000310 | 0.001394 | 0.001757 | 16 | PASS |
| stress | viewed | 2296 | 0.000310 | 0.001394 | 0.001757 | 16 | PASS |

The concave fixture has eight teeth at each of three sizes: approximately 2, 4 and 8 mm facet lengths, 85° incidence and a 0.04-rad disk. Its independent analytic oracle intersects the extruded piecewise-planar profile, unions projected slope intervals and integrates their continuous disk CDF. The profile oracle also passes 262,144 independent f64 rays at five probes, including three partials (maximum difference 1.775e-5). The continuous oracle starts at the offset origin with positive t; its comparison includes the effect of GPU ε t_min rather than adding that numerical guard to the analytic geometry. The thin/stress oracle reuses F0’s polygon/disk quadrature and independent primary intersections, with the actual vertex-scaled ground ε.

The 1 mm **magnitude alone**, even before adding production’s projected-depth t_min, differs from the small-offset continuous sawtooth reference by RMS 0.354465 / 0.354491 (magnitude/viewed), signed mean +0.174451 / +0.170805. Its image is retained in the sawtooth oracle sheet. This is an analytic magnitude control, not a GPU film capture of the full production 1 mm policy.

Thin accepted counts match. Stress retains F0’s 16 boundary pixels differing by one accepted primary sample; none is discarded. All-covered and partial RMS both pass the unchanged σ_F gate. No fixture tolerance was changed. The initial GPU attempt exposed F0’s stale test entry name (`primary_shade` versus `primary_shade_tiles`); the alias was fixed in composed test source. A Numba scalar-conversion error was fixed before oracle results were used.

## Frozen per-population comparison

RMS differences below use populations from the original F0 10 µm R_F, including point-aliased and micro. E is original centre Exact’s error against that same reference. R_D uses the verified fixed-10 µm replay. `occluded` is umbra; `solid` combines lit and umbra. Complete linear HDR, display at registered/+4 stops, mean, p99, maximum, counts and verdicts are in `scores/population-scores.csv`. Image/background/multi-surface populations are included there; visibility excludes background because R_F has no background receiver.

### Pose 0

Fixed-10 µm replay: maximum R_F difference 0; accepted-count mismatches 0; maximum R_L linear-channel difference 1.78813934e-07.

| Population | Pixels | R1a R_F | R1b R_F | E R_F |
|---|---|---|---|---|
| partial | 49697 | 0.000173 | 0.025598 | 0.250461 |
| lit | 280600 | 0.000013 | 0.001499 | 0.006833 |
| occluded | 16220 | 0.000000 | 0.007830 | 0.022273 |
| narrow | 8777 | 0.000148 | 0.025136 | 0.258420 |
| faint | 18797 | 0.000155 | 0.008936 | 0.078717 |
| isolated | 867 | 0.000133 | 0.013256 | 0.143243 |
| silhouette | 606 | 0.000146 | 0.034973 | 0.234609 |
| wide | 40920 | 0.000178 | 0.025697 | 0.248721 |
| wide6 | 27293 | 0.000177 | 0.024782 | 0.245608 |
| contact | 15057 | 0.000117 | 0.030837 | 0.271350 |
| solid | 296820 | 0.000013 | 0.002340 | 0.008441 |
| point-aliased | 41088 | 0.000185 | 0.027153 | 0.244288 |
| micro | 35549 | 0.000189 | 0.029646 | 0.289623 |
| backlit | 11824 | 0.000124 | 0.040229 | 0.271631 |
| all-covered | 346517 | 0.000067 | 0.009933 | 0.095172 |
| multi-surface | 6911 | 0.000129 | 0.017501 | 0.134309 |

| Population | R1a R_L codes | R1b R_L codes | E R_L | R1a R_D +4 codes | R1b R_D +4 codes | E R_D +4 |
|---|---|---|---|---|---|---|
| partial | 0.000046 | 0.002803 | 9.349567 | 0.001883 | 0.178942 | 7.869805 |
| faint | 0.000047 | 0.001767 | 8.588591 | 0.001861 | 0.086038 | 6.703549 |
| narrow | 0.000038 | 0.001818 | 10.864207 | 0.001514 | 0.129420 | 8.214422 |
| wide | 0.000048 | 0.002972 | 8.991519 | 0.001953 | 0.187871 | 7.793904 |
| isolated | 0.000072 | 0.002150 | 12.145498 | 0.003059 | 0.099369 | 12.008141 |
| silhouette | 0.000039 | 0.002185 | 13.104383 | 0.001406 | 0.141947 | 8.159824 |
| contact | 0.000034 | 0.002284 | 10.340062 | 0.001711 | 0.173421 | 7.665733 |
| lit | 0.000006 | 0.000026 | 4.784443 | 0.000213 | 0.001052 | 5.111735 |
| occluded | 0.000002 | 0.000222 | 7.807876 | 0.000124 | 0.019463 | 2.048069 |
| point-aliased | 0.000048 | 0.003046 | 9.872617 | 0.001996 | 0.193190 | 8.225870 |
| micro | 0.000049 | 0.003302 | 9.999357 | 0.002025 | 0.210849 | 8.761930 |
| backlit | 0.000031 | 0.003337 | 12.458928 | 0.001719 | 0.237557 | 8.785567 |
| multi-surface | 0.000044 | 0.000843 | 13.659986 | 0.001532 | 0.053252 | 9.603083 |
| background-centre | 0.000000 | 0.000024 | 1.092799 | 0.000010 | 0.001139 | 0.583076 |
| image | 0.000007 | 0.000435 | 2.581497 | 0.000302 | 0.027778 | 2.310024 |

### Pose 1

Fixed-10 µm replay: maximum R_F difference 0; accepted-count mismatches 0; maximum R_L linear-channel difference 1.78813934e-07.

| Population | Pixels | R1a R_F | R1b R_F | E R_F |
|---|---|---|---|---|
| partial | 49701 | 0.000166 | 0.025589 | 0.250319 |
| lit | 280046 | 0.000002 | 0.001488 | 0.006085 |
| occluded | 16135 | 0.000000 | 0.005939 | 0.024599 |
| narrow | 8530 | 0.000123 | 0.023508 | 0.253117 |
| faint | 18831 | 0.000141 | 0.008756 | 0.080567 |
| isolated | 862 | 0.000145 | 0.015337 | 0.164238 |
| silhouette | 607 | 0.000165 | 0.032049 | 0.237999 |
| wide | 41171 | 0.000174 | 0.025999 | 0.249736 |
| wide6 | 27414 | 0.000174 | 0.024859 | 0.248299 |
| contact | 15012 | 0.000118 | 0.030872 | 0.272239 |
| solid | 296181 | 0.000002 | 0.002004 | 0.008245 |
| point-aliased | 41086 | 0.000177 | 0.027070 | 0.244071 |
| micro | 35524 | 0.000182 | 0.029559 | 0.289540 |
| backlit | 11824 | 0.000128 | 0.039462 | 0.274656 |
| all-covered | 345882 | 0.000063 | 0.009876 | 0.095195 |
| multi-surface | 6922 | 0.000076 | 0.015549 | 0.136107 |

| Population | R1a R_L codes | R1b R_L codes | E R_L | R1a R_D +4 codes | R1b R_D +4 codes | E R_D +4 |
|---|---|---|---|---|---|---|
| partial | 0.000045 | 0.002814 | 9.660197 | 0.001914 | 0.178311 | 7.787547 |
| faint | 0.000047 | 0.001730 | 9.003775 | 0.001887 | 0.085944 | 6.490246 |
| narrow | 0.000036 | 0.001834 | 10.537683 | 0.001360 | 0.131396 | 8.132413 |
| wide | 0.000047 | 0.002977 | 9.468231 | 0.002010 | 0.186561 | 7.714168 |
| isolated | 0.000088 | 0.002635 | 12.261449 | 0.003241 | 0.130025 | 11.432381 |
| silhouette | 0.000037 | 0.002424 | 14.120547 | 0.001828 | 0.173304 | 8.291445 |
| contact | 0.000033 | 0.002251 | 10.621643 | 0.001618 | 0.168846 | 7.654767 |
| lit | 0.000002 | 0.000023 | 4.897059 | 0.000053 | 0.001163 | 5.093291 |
| occluded | 0.000002 | 0.000108 | 7.835889 | 0.000079 | 0.009517 | 1.934638 |
| point-aliased | 0.000047 | 0.003052 | 10.220653 | 0.002018 | 0.192127 | 8.138900 |
| micro | 0.000047 | 0.003313 | 10.321797 | 0.001987 | 0.210032 | 8.674297 |
| backlit | 0.000032 | 0.003399 | 12.777544 | 0.001863 | 0.238246 | 8.661407 |
| multi-surface | 0.000029 | 0.000971 | 14.170306 | 0.000947 | 0.055986 | 9.312926 |
| background-centre | 0.000001 | 0.000024 | 1.061199 | 0.000024 | 0.001283 | 0.575476 |
| image | 0.000007 | 0.000436 | 2.625172 | 0.000298 | 0.027647 | 2.293935 |

## Viewed-side decision

| Pose | Backlit channel | Pixels | R1b−R1a RMS | Signed mean | Frozen E RMS | Above E |
|---|---|---|---|---|---|---|
| 0 | R_F | 11824 | 0.040256 | 0.014423 | 0.271631 | False |
| 0 | R_L | 11824 | 0.000007 | 0.000002 | 0.021380 | False |
| 0 | R_D | 11824 | 0.000007 | 0.000002 | 0.000295 | False |
| 1 | R_F | 11824 | 0.039487 | 0.014231 | 0.274656 | False |
| 1 | R_L | 11824 | 0.000007 | 0.000002 | 0.021846 | False |
| 1 | R_D | 11824 | 0.000007 | 0.000002 | 0.000281 | False |

The side rule changes the opaque surface’s estimand even if a particular aggregate stays below E. It is not adopted. Full R_F/R_L/R_D images at both exposures and five separated maximum-sun-change backlit crops per pose are retained, alongside all seven frozen contact probes. Crop coordinates are recorded in the summary JSON.

The observed side change is not uniformly darker: centre-backlit R_F increases by mean +0.014423 / +0.014231 at poses 0/1. R_F RMS changes are 0.040256 / 0.039487, below original Exact’s 0.271631 / 0.274656 error on that population. R_L/R_D side differences also remain below E. Film footprints can include multiple facet orientations; the centre-backlit population is a diagnostic, not a per-sample sidedness partition. These results do not establish a visual improvement or settle the material policy.

- Pose 0: [decision crops](D:/Coding/renderlab-work/m6-exact-quality/r1/images/pose-0-viewed-side-decision-crops.png), [frozen contacts](D:/Coding/renderlab-work/m6-exact-quality/r1/images/pose-0-frozen-contact-crops.png).
- Pose 1: [decision crops](D:/Coding/renderlab-work/m6-exact-quality/r1/images/pose-1-viewed-side-decision-crops.png), [frozen contacts](D:/Coding/renderlab-work/m6-exact-quality/r1/images/pose-1-frozen-contact-crops.png).

## Saved-sample convergence and stop rules

| Pose | Policy | Population | β visibility | β radiance | β sun | V model N=8 | V model N=32 | R_L first ≤ E | Revise |
|---|---|---|---|---|---|---|---|---|---|
| 0 | magnitude | partial | 0.799379 | 0.843198 | 0.861151 | 0.096534 | 0.031872 | 4 | False |
| 0 | magnitude | all-covered | 0.798921 | 0.843793 | 0.859239 | 0.036557 | 0.012077 | 5 | False |
| 0 | magnitude | image | 0.798921 | 0.849758 | 0.866662 | 0.036557 | 0.012077 | 5 | False |
| 0 | viewed | partial | 0.811239 | 0.843187 | 0.863117 | 0.092933 | 0.030182 | 4 | False |
| 0 | viewed | all-covered | 0.810592 | 0.843790 | 0.860045 | 0.035215 | 0.011447 | 5 | False |
| 0 | viewed | image | 0.810592 | 0.849755 | 0.867455 | 0.035215 | 0.011447 | 5 | False |
| 1 | magnitude | partial | 0.799596 | 0.835941 | 0.857068 | 0.096086 | 0.031714 | 4 | False |
| 1 | magnitude | all-covered | 0.799186 | 0.836725 | 0.856791 | 0.036424 | 0.012029 | 5 | False |
| 1 | magnitude | image | 0.799186 | 0.841311 | 0.862627 | 0.036424 | 0.012029 | 5 | False |
| 1 | viewed | partial | 0.810590 | 0.835936 | 0.859010 | 0.092724 | 0.030142 | 4 | False |
| 1 | viewed | all-covered | 0.810008 | 0.836724 | 0.857599 | 0.035166 | 0.011441 | 5 | False |
| 1 | viewed | image | 0.810008 | 0.841309 | 0.863407 | 0.035166 | 0.011441 | 5 | False |

β is an unweighted log-RMS fit at all N=1–16, not the F0 two-point extrapolation. Curves, missing counts, fitted amplitudes and modelled frames-to-Exact are in `scores/summary.json`. These fits describe one deterministic, well-spread sequence of Exact-shaded samples, with the finite 256-spp reference’s own noise and sample inclusion. They do not include 8-ray AA shadow noise, reprojection, motion or history validation. N=32 is an extrapolation. The recalculated values replace the prior ideal static prediction where the revision rule triggers; they do not select Islam’s perceptual T_q or certify F1-AA.

**Earlier prediction miss:** ideal R_L replay on partials first reaches point Exact at N=4 at both poses, for both policies; all-covered and whole-image radiance first reach it at N=5. N=2 is above Exact. Thus the earlier two-frame radiance crossing is unsupported by this saved sequence even though β_L > β_V. The literal R1 β/radiance-rate predicate does not fire; before AA, use the measured 4–5-sample crossing as the new ideal static baseline and measure the actual 8-ray/history cost. The F1-AA two-frame acceptance floor remains frozen: this observation does not relax it. T_q remains Islam’s decision.

Stop-rule verdicts: all fixtures pass unchanged σ_F and zero-acne requirements; magnitude verdict is stated above; viewed-side remains a separate decision; the replay revision predicate is reported without relaxing β=0.45 or the radiance-rate test. No production policy or default changes, and no F1-AA work.

CPU scoring correction: the first replay incorrectly conditioned radiance error on visibility-sample acceptance. The retained replay scores R_L/R_D over the complete requested population, including background silhouettes for the image row; R_F alone uses accepted covered samples. Run 01 is retained as superseded. A final reduction corrected the image row’s visibility baseline and missing count to the identical all-covered visibility domain; radiance curves were unchanged. No GPU capture or tolerance changed.

Independent pose-2 replay identity (byte equality against pose 0):

| Policy | R_F/R_L raw film | R_D raw film | Exact counts | Exact HDR | Exact sun HDR |
|---|---|---|---|---|---|
| minimal | True | True | True | True | True |
| magnitude | True | True | True | True | True |
| viewed | True | True | True | True | True |

## Inspection, evidence and reproduction

Verified locally: all full acne fixture panels; all six sawtooth/thin/stress oracle sheets; both quarry poses under both policies, complete R_F/R_L/R_D at registered and +4 stops, signed visibility differences, viewed-side decision crops and frozen contact crops. Acne panels have no dark defects. Analytic/GPU fixture boundaries agree; the 1 mm sawtooth control loses grooves. Quarry magnitude differences are sparse and small; viewed-side differences add fine visibility structure around rough faces, ridges and rubble. Full R_L looks substantially alike, while sun-only +4 crops expose small local changes. No blanket darkening or quality improvement is claimed. The complete silhouettes and cast-shadow regions remain present. Static image inspection does not establish motion quality.

Evidence: `D:/Coding/renderlab-work/m6-exact-quality/r1/`, physically on E: through a directory junction after D: filled during build-02. Only this task’s own target/evidence was relocated; old evidence and other workers’ files were not moved. `provenance.json` and `identities.sha256` bind source/scripts, binary, raw buffers, scores and images. `quarry-{minimal,magnitude,viewed}-p{0,1,2}/` contains references and the 16 saved frames. Fixture subdirectories contain geometry, raw buffers, oracles, validation JSON and images.

The shared GPU lock was acquired by atomic mkdir plus token-bearing owner.json before every build/GPU run; all runs were hidden/offscreen. Preflight checked games, Unreal, other Renderlab jobs and ≥6144 MiB available physical memory; the monitor also aborted on contention or insufficient available memory. The first quarry attempt was stopped when another compilation plus asset upload lowered availability to 4739 MiB, then retried. No foreign application was closed.

All 46 owned windows were released: total 501.198 s, longest 125.777 s, below the 45-minute limit. Complete UTC acquire/release times:

| Label | Acquired UTC | Released UTC | Seconds |
|---|---|---|---|
| build-01 | 2026-11-05T00:35:32.416905+00:00 | 2026-11-05T00:37:38.207401+00:00 | 125.777 |
| flat-1000-80-magnitude-01 | 2026-11-05T00:38:44.805047+00:00 | 2026-11-05T00:38:57.786418+00:00 | 12.981 |
| build-02 | 2026-11-05T00:39:26.331213+00:00 | 2026-11-05T00:39:48.971984+00:00 | 22.638 |
| build-03 | 2026-11-05T00:40:40.856949+00:00 | 2026-11-05T00:40:46.896498+00:00 | 6.039 |
| build-04 | 2026-11-05T00:41:03.357288+00:00 | 2026-11-05T00:41:40.643485+00:00 | 37.286 |
| flat-1000-80-magnitude-02 | 2026-11-05T00:42:43.743961+00:00 | 2026-11-05T00:42:53.778587+00:00 | 10.034 |
| flat-1000-85-magnitude-03 | 2026-11-05T00:43:06.835654+00:00 | 2026-11-05T00:43:10.264274+00:00 | 3.428 |
| flat-1000-89-magnitude-03 | 2026-11-05T00:43:11.826898+00:00 | 2026-11-05T00:43:15.249079+00:00 | 3.422 |
| flat-10000-80-magnitude-03 | 2026-11-05T00:43:16.863276+00:00 | 2026-11-05T00:43:20.022664+00:00 | 3.159 |
| flat-10000-85-magnitude-03 | 2026-11-05T00:43:21.294321+00:00 | 2026-11-05T00:43:24.750779+00:00 | 3.456 |
| flat-10000-89-magnitude-03 | 2026-11-05T00:43:25.992511+00:00 | 2026-11-05T00:43:28.752337+00:00 | 2.760 |
| convex-1000-80-magnitude-03 | 2026-11-05T00:43:29.920924+00:00 | 2026-11-05T00:43:32.775110+00:00 | 2.854 |
| convex-1000-85-magnitude-03 | 2026-11-05T00:43:33.949790+00:00 | 2026-11-05T00:43:36.842528+00:00 | 2.892 |
| convex-1000-89-magnitude-03 | 2026-11-05T00:43:38.026658+00:00 | 2026-11-05T00:43:40.933790+00:00 | 2.907 |
| convex-10000-80-magnitude-03 | 2026-11-05T00:43:42.141730+00:00 | 2026-11-05T00:43:44.983708+00:00 | 2.842 |
| convex-10000-85-magnitude-03 | 2026-11-05T00:43:46.193366+00:00 | 2026-11-05T00:43:49.089802+00:00 | 2.896 |
| convex-10000-89-magnitude-03 | 2026-11-05T00:43:50.338139+00:00 | 2026-11-05T00:43:53.134722+00:00 | 2.796 |
| sawtooth-magnitude-03 | 2026-11-05T00:43:54.306509+00:00 | 2026-11-05T00:43:57.213652+00:00 | 2.907 |
| thin-magnitude-04 | 2026-11-05T00:44:32.904196+00:00 | 2026-11-05T00:44:36.193612+00:00 | 3.289 |
| stress-magnitude-04 | 2026-11-05T00:44:43.624822+00:00 | 2026-11-05T00:44:46.489517+00:00 | 2.864 |
| flat-1000-80-viewed-04 | 2026-11-05T00:45:05.829714+00:00 | 2026-11-05T00:45:14.354794+00:00 | 8.525 |
| flat-1000-85-viewed-04 | 2026-11-05T00:45:15.888766+00:00 | 2026-11-05T00:45:18.757714+00:00 | 2.869 |
| flat-1000-89-viewed-04 | 2026-11-05T00:45:19.966269+00:00 | 2026-11-05T00:45:22.874501+00:00 | 2.908 |
| flat-10000-80-viewed-04 | 2026-11-05T00:45:24.154661+00:00 | 2026-11-05T00:45:27.125815+00:00 | 2.971 |
| flat-10000-85-viewed-04 | 2026-11-05T00:45:28.318323+00:00 | 2026-11-05T00:45:31.124654+00:00 | 2.806 |
| flat-10000-89-viewed-04 | 2026-11-05T00:45:32.479258+00:00 | 2026-11-05T00:45:35.391583+00:00 | 2.912 |
| convex-1000-80-viewed-04 | 2026-11-05T00:45:36.579423+00:00 | 2026-11-05T00:45:39.398187+00:00 | 2.818 |
| convex-1000-85-viewed-04 | 2026-11-05T00:45:40.564556+00:00 | 2026-11-05T00:45:43.371897+00:00 | 2.807 |
| convex-1000-89-viewed-04 | 2026-11-05T00:45:44.573052+00:00 | 2026-11-05T00:45:47.437211+00:00 | 2.864 |
| convex-10000-80-viewed-04 | 2026-11-05T00:45:48.657524+00:00 | 2026-11-05T00:45:51.513486+00:00 | 2.856 |
| convex-10000-85-viewed-04 | 2026-11-05T00:45:52.725477+00:00 | 2026-11-05T00:45:55.533833+00:00 | 2.808 |
| convex-10000-89-viewed-04 | 2026-11-05T00:45:56.738725+00:00 | 2026-11-05T00:45:59.617442+00:00 | 2.878 |
| sawtooth-viewed-04 | 2026-11-05T00:46:00.878687+00:00 | 2026-11-05T00:46:03.904948+00:00 | 3.026 |
| thin-viewed-04 | 2026-11-05T00:46:16.580351+00:00 | 2026-11-05T00:46:19.715791+00:00 | 3.135 |
| stress-viewed-04 | 2026-11-05T00:46:29.716787+00:00 | 2026-11-05T00:46:32.608595+00:00 | 2.892 |
| quarry-minimal-p0-01 | 2026-11-05T00:47:04.261294+00:00 | 2026-11-05T00:47:11.795689+00:00 | 7.534 |
| build-05 | 2026-11-05T00:51:59.587809+00:00 | 2026-11-05T00:52:46.091320+00:00 | 46.503 |
| quarry-minimal-p0-05 | 2026-11-05T00:56:02.395755+00:00 | 2026-11-05T00:56:33.578136+00:00 | 31.182 |
| quarry-minimal-p1-05 | 2026-11-05T00:56:33.705445+00:00 | 2026-11-05T00:56:52.406763+00:00 | 18.701 |
| quarry-minimal-p2-05 | 2026-11-05T00:56:52.536628+00:00 | 2026-11-05T00:57:07.707323+00:00 | 15.170 |
| quarry-magnitude-p0-05 | 2026-11-05T00:57:07.834022+00:00 | 2026-11-05T00:57:23.103742+00:00 | 15.269 |
| quarry-magnitude-p1-05 | 2026-11-05T00:57:23.230324+00:00 | 2026-11-05T00:57:34.944780+00:00 | 11.714 |
| quarry-magnitude-p2-05 | 2026-11-05T00:57:35.073647+00:00 | 2026-11-05T00:57:46.863995+00:00 | 11.790 |
| quarry-viewed-p0-05 | 2026-11-05T00:57:46.991282+00:00 | 2026-11-05T00:57:58.194851+00:00 | 11.203 |
| quarry-viewed-p1-05 | 2026-11-05T00:57:58.327395+00:00 | 2026-11-05T00:58:10.148639+00:00 | 11.821 |
| quarry-viewed-p2-05 | 2026-11-05T00:58:10.277318+00:00 | 2026-11-05T00:58:24.284931+00:00 | 14.007 |

Implementation/analysis/reporting overlapped verification and were not separately stopwatched. Verification: builds and GPU windows are timed separately in `runs/*.json`; fixture CPU elapsed times are in `oracle-run-*.log`; final population scoring/replay took 77.852 s. All 77 nonignored library CPU tests pass (60 GPU tests remain explicitly ignored), plus composed WGSL validation, independent oracle checks, formatting and Python parsing. Accepted GPU runs report active Vulkan validation with zero errors. No frame-time or engine-quality superiority claim is made.

```powershell
$env:OPENBLAS_NUM_THREADS='2'; $env:OMP_NUM_THREADS='2'; $env:NUMBA_NUM_THREADS='2'
py -3 -B scripts/m6-receiver-run.py --label fresh-build --minutes 15 -- cargo test -p renderlab --lib --release --no-run -j 2 --target-dir D:/Coding/renderlab-work/m6-exact-quality/r1/target
py -3 -B scripts/m6-receiver-fixtures.py --check-oracle
py -3 -B scripts/m6-receiver-collect.py --phase fixtures --attempt fresh
py -3 -B scripts/m6-receiver-collect.py --phase quarry --attempt fresh
py -3 -B scripts/m6-receiver-score.py
py -3 -B scripts/m6-receiver-report.py
```

Collection reuses complete existing captures and passed validations. To regenerate retained raw captures, choose a fresh evidence location in the guarded runner and collector constants; logs refuse overwrites. Python requires NumPy, SciPy, Numba and Pillow. Source: `src/{film_reference_tests,receiver_policy_tests}.rs`, `support/receiver_policy.wgsl`, F0’s `support/film_accumulate.wgsl`, and `scripts/m6-receiver-*.py`. The focused `m6-receiver-score-test.py` checks that rejected foreground and background samples remain in radiance while visibility baseline/missing counts use covered receivers. Affected release Clippy passes with warnings denied. All edits are test/support/tooling/documentation only.
