# M6 F0: pixel-integrated film reference

2026-11-05 local (2026-11-04 UTC), branch `t3/m6-film-reference-f0`, base `0f38982`. Measurement only; no production renderer or shader behavior changes. No push.

**Verdict: the estimand finding holds, but the frozen receiver-policy rule requires a receiver-policy unit before F1.** Exact partial RMS against the policy film reference is **0.231586 / 0.232030 / 0.231586** at poses 0/1/2. None is below 0.02, and all exceed 0.05. The two references differ more than Exact’s error on the film-lit population at every pose. F1/F2 were not started. Exact remains the default. The prior 8.4–10.9 ms Exact cost is unchanged historical evidence, not a new timing or a faster-mode acceptance.

The binding decision and stop rules are [F0](m6-exact-quality-fast-shadows.md#7-first-unit-and-stop-rules). The old [Level A](m6-reduced-ray-gate.md), [Level B implementation](m6-level-b-implementation.md), and [native mask contract](unreal-native-shadow-mask.md) were read and retained as evidence. No threshold was loosened.

## Reference and identity

The test-only mode traces **256 footprint samples**, one independently randomized position in every cell of a 16×16 box-filter stratum grid. Each hit runs the existing **256-ray Exact slope-disk estimator**, independently salted with seeds 1–256. This is the 65,536-ray-per-covered-pixel construction specified in §6, not a new production shading method. R_F includes a primary sample only when it hits the centre placement and its physical view depth is within 2% of the centre. R_L averages every exposed linear HDR primary sample, including different surfaces and background; it is not conditioned by the R_F acceptance test.

The two policies are (1) unchanged geometric-normal offset **0.001 m** and unchanged projected-depth t_min, and (2) offset **1e-5 m** and ray-distance t_min **1e-5 m**. Both retain the renderer’s light-space coverage and clipping. All modes are rescored unchanged against both references: “minimal” changes the reference, not the candidate.

Visibility sums/counts, per-footprint second moment, and HDR sums are accumulated on the GPU. Even/odd interleaved subpixel halves estimate σ_F using the weighted half difference. Raw RMS and `sqrt(max(0, MSE−mean(σ_F²)))` are both reported; the **raw** statistic decides the frozen stop. Split-half uncertainty includes footprint integration variation and is not a pure estimate of direction-only ray noise. No extra samples, filtering, gain fit, alignment or candidate-dependent population selection were introduced.

Hardware: RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan; active Khronos validation. Quarry: the registered twelve full-resident draws / **13,034,903 triangles**, **1920×1080, 1x**, 16x anisotropy, RLE2 environment, registered −14-stop exposure, 0.04-rad disk. Pose 1 is the registered +1 cm X translation; pose 2 is a fresh independent process at pose 0. No Unreal launch was needed: native VSM and 1-spp denoised HRT are the saved UE **5.7.4 CL51494982**, D3D12 main-view masks from the native-mask report.

All six captured centre Exact count arrays match the frozen b6/b12 production counts. All six centre Exact HDR files match `m6-level-b-probe/runs/accept-b7/` byte for byte. Policy and minimal pose-2 film buffers each match their pose-0 buffer byte for byte. That supports these independent static replays; it is not a contiguous temporal-return or F8 sequence result.

## Fixture validation before quarry collection

Both existing fixtures were evaluated at their static original pose, at both offsets. Every pixel and all 256 footprint positions were checked against an independently intersected planar geometry / continuous slope-disk oracle. The interval union is integrated between exact polygon/circle breakpoints with 64-node Gauss–Legendre quadrature. Representative scalar oracle values also match the frozen adaptive SciPy oracle (maximum 1.29e-8 in the retained fixture checks; a separate 64-point random thin/contact cross-check differs by at most 6.23e-8). No third-party implementation was copied.

| Fixture / offset | Partial n | GPU–oracle RMS | σ_F RMS | Maximum error | Verdict |
|---|---|---|---|---|---|
| thin/policy | 933 | 0.000414 | 0.004800 | 0.002020 | within σ_F |
| thin/minimal | 933 | 0.000407 | 0.004719 | 0.001555 | within σ_F |
| stress/policy | 2284 | 0.000321 | 0.001461 | 0.001648 | within σ_F |
| stress/minimal | 2296 | 0.000310 | 0.001395 | 0.001758 | within σ_F |

Thin/contact accepted counts match exactly. Each stress case has **16 pixels differing by one accepted primary sample**, consistent with f32 primary/rectangle-boundary arithmetic versus the independent analytic construction; they remain in the error score. No receiver is discarded for the mismatch. Both all-covered and partial RMS are below σ_F in all four cases. This validates the prescribed combined film estimate; it does not claim each individual 256-ray subpixel is noiseless or establish moving-fixture acceptance.

## Static partial scores and shadow defects

These are the **complete source-covered population** scores. Native values use available native outputs and preserve their missing counts; they do not certify complete-domain native parity. A missed film-visible shadow is `R_F <= 0.9 && output >= 0.99`; a false shadow is `R_F >= 0.99 && output <= 0.9`. Every coordinate, reference/candidate value, error and σ_F is enumerated in `scores/pose-{p}-{policy}-{method}-defects.csv`. Native two-frame persistence is unknown from a saved single mask; the counts below are static endpoints, not a temporal F4 pass.

| Pose | Reference | Method | Partial n | Raw RMS | Noise-subtracted | Signed mean | p99 abs | Missed | False |
|---|---|---|---|---|---|---|---|---|---|
| 0 | policy | Exact | 46514 | 0.231586 | 0.231173 | +0.000191 | 0.824219 | 9199 | 9 |
| 0 | policy | Interactive | 46514 | 0.234188 | 0.233780 | +0.000072 | 0.824219 | 9768 | 9 |
| 0 | policy | Unreal-VSM | 46510 | 0.370607 | 0.370348 | +0.230600 | 0.992035 | 24049 | 9 |
| 0 | policy | Unreal-HRT | 46510 | 0.345569 | 0.345292 | +0.154750 | 0.980176 | 20940 | 757 |
| 0 | minimal | Exact | 49697 | 0.250461 | 0.250096 | +0.063047 | 0.815768 | 12615 | 20 |
| 0 | minimal | Interactive | 49697 | 0.252656 | 0.252294 | +0.062962 | 0.820774 | 13293 | 21 |
| 0 | minimal | Unreal-VSM | 49693 | 0.402986 | 0.402759 | +0.257008 | 0.996094 | 27739 | 11 |
| 0 | minimal | Unreal-HRT | 49693 | 0.375384 | 0.375140 | +0.194760 | 0.991089 | 24596 | 743 |
| 1 | policy | Exact | 46423 | 0.232030 | 0.231612 | -0.000386 | 0.826266 | 9118 | 11 |
| 1 | policy | Interactive | 46423 | 0.234588 | 0.234174 | -0.000392 | 0.826982 | 9682 | 11 |
| 1 | policy | Unreal-VSM | 46418 | 0.371869 | 0.371608 | +0.230974 | 0.991234 | 24059 | 10 |
| 1 | policy | Unreal-HRT | 46418 | 0.345214 | 0.344933 | +0.154168 | 0.978557 | 20817 | 770 |
| 1 | minimal | Exact | 49701 | 0.250319 | 0.249951 | +0.062273 | 0.820297 | 12473 | 17 |
| 1 | minimal | Interactive | 49701 | 0.252437 | 0.252072 | +0.062258 | 0.825348 | 13166 | 17 |
| 1 | minimal | Unreal-VSM | 49696 | 0.404545 | 0.404317 | +0.257633 | 0.996354 | 27753 | 9 |
| 1 | minimal | Unreal-HRT | 49696 | 0.376230 | 0.375985 | +0.194789 | 0.991746 | 24467 | 757 |
| 2 | policy | Exact | 46514 | 0.231586 | 0.231173 | +0.000191 | 0.824219 | 9199 | 9 |
| 2 | policy | Interactive | 46514 | 0.234188 | 0.233780 | +0.000072 | 0.824219 | 9768 | 9 |
| 2 | policy | Unreal-VSM | 46510 | 0.370607 | 0.370348 | +0.230600 | 0.992035 | 24049 | 9 |
| 2 | policy | Unreal-HRT | 46510 | 0.345569 | 0.345292 | +0.154750 | 0.980176 | 20940 | 757 |
| 2 | minimal | Exact | 49697 | 0.250461 | 0.250096 | +0.063047 | 0.815768 | 12615 | 20 |
| 2 | minimal | Interactive | 49697 | 0.252656 | 0.252294 | +0.062962 | 0.820774 | 13293 | 21 |
| 2 | minimal | Unreal-VSM | 49693 | 0.402986 | 0.402759 | +0.257008 | 0.996094 | 27739 | 11 |
| 2 | minimal | Unreal-HRT | 49693 | 0.375384 | 0.375140 | +0.194760 | 0.991089 | 24596 | 743 |

The old point reference has 8,766 / 8,782 / 8,766 partials; the policy film reference has 46,514 / 46,423 / 46,514. Spatial aliasing is therefore measured on a substantially different, reference-defined population. On these complete-domain film partials Interactive is only **0.85–1.12%** worse than Exact in RMS, rather than the proxy’s inferred 7–22%. This is a measurement of the retained estimators on this scene, not a promotion of Interactive, a perceptual-equivalence finding or a speed claim.

## Per-population RMS

Membership uses R_F only. The existing Level-B population and contour functions receive the original stored reversed-Z centre depth, preserving their definitions; physical view depth is used separately for the reference’s 2% acceptance test. `point-aliased` means the frozen 16,384-ray point reference is solid while R_F is partial. `micro` means the measured within-footprint standard deviation exceeds 0.1 (the 256-ray subpixel estimator contributes a small residual). The CSV retains all populations, counts, availability, mean, SE, p99, maximum, raw RMS and noise-subtracted RMS, on both all-covered and light-facing domains.

| Pose / reference | Method | partial | narrow | wide | faint | isolated | silhouette | contact | lit | occluded | point-aliased | micro |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0/policy | Exact | 0.231586 | 0.240261 | 0.229780 | 0.109296 | 0.182350 | 0.239050 | 0.208406 | 0.001871 | 0.000000 | 0.243598 | 0.264940 |
| 0/policy | Interactive | 0.234188 | 0.242694 | 0.232419 | 0.109655 | 0.182785 | 0.241111 | 0.210478 | 0.001871 | 0.000000 | 0.243598 | 0.267282 |
| 0/policy | Unreal-VSM | 0.370607 | 0.462287 | 0.349007 | 0.045647 | 0.048642 | 0.321396 | 0.626736 | 0.002656 | 0.482565 | 0.365819 | 0.383240 |
| 0/policy | Unreal-HRT | 0.345569 | 0.425607 | 0.326885 | 0.062844 | 0.084284 | 0.306602 | 0.554655 | 0.048664 | 0.379653 | 0.324349 | 0.365390 |
| 0/minimal | Exact | 0.250461 | 0.258420 | 0.248721 | 0.078717 | 0.143243 | 0.234609 | 0.271350 | 0.006833 | 0.022273 | 0.244288 | 0.289623 |
| 0/minimal | Interactive | 0.252656 | 0.260577 | 0.250925 | 0.079037 | 0.143833 | 0.236947 | 0.273403 | 0.006823 | 0.021991 | 0.244288 | 0.291578 |
| 0/minimal | Unreal-VSM | 0.402986 | 0.483417 | 0.383544 | 0.044773 | 0.056294 | 0.353276 | 0.663620 | 0.002188 | 0.516684 | 0.375650 | 0.416186 |
| 0/minimal | Unreal-HRT | 0.375384 | 0.442377 | 0.359392 | 0.053797 | 0.062524 | 0.328665 | 0.592263 | 0.049086 | 0.408294 | 0.341712 | 0.396997 |
| 1/policy | Exact | 0.232030 | 0.229979 | 0.232445 | 0.112816 | 0.199449 | 0.230952 | 0.207314 | 0.003244 | 0.000000 | 0.244198 | 0.265779 |
| 1/policy | Interactive | 0.234588 | 0.231979 | 0.235115 | 0.113349 | 0.199634 | 0.234893 | 0.209795 | 0.003244 | 0.000000 | 0.244198 | 0.268102 |
| 1/policy | Unreal-VSM | 0.371869 | 0.461787 | 0.350739 | 0.044388 | 0.059338 | 0.319711 | 0.626721 | 0.003746 | 0.484846 | 0.367103 | 0.386263 |
| 1/policy | Unreal-HRT | 0.345214 | 0.423490 | 0.326981 | 0.066488 | 0.085822 | 0.298131 | 0.552513 | 0.049373 | 0.373592 | 0.323064 | 0.366227 |
| 1/minimal | Exact | 0.250319 | 0.253117 | 0.249736 | 0.080567 | 0.164238 | 0.237999 | 0.272239 | 0.006085 | 0.024599 | 0.244071 | 0.289540 |
| 1/minimal | Interactive | 0.252437 | 0.254775 | 0.251950 | 0.080929 | 0.164388 | 0.241480 | 0.274674 | 0.006096 | 0.026533 | 0.244071 | 0.291405 |
| 1/minimal | Unreal-VSM | 0.404545 | 0.481282 | 0.386744 | 0.044054 | 0.063339 | 0.354547 | 0.663977 | 0.003348 | 0.517113 | 0.377737 | 0.416989 |
| 1/minimal | Unreal-HRT | 0.376230 | 0.440440 | 0.361501 | 0.058231 | 0.078669 | 0.330024 | 0.591508 | 0.049773 | 0.400113 | 0.342401 | 0.396403 |
| 2/policy | Exact | 0.231586 | 0.240261 | 0.229780 | 0.109296 | 0.182350 | 0.239050 | 0.208406 | 0.001871 | 0.000000 | 0.243598 | 0.264940 |
| 2/policy | Interactive | 0.234188 | 0.242694 | 0.232419 | 0.109655 | 0.182785 | 0.241111 | 0.210478 | 0.001871 | 0.000000 | 0.243598 | 0.267282 |
| 2/policy | Unreal-VSM | 0.370607 | 0.462287 | 0.349007 | 0.045647 | 0.048642 | 0.321396 | 0.626736 | 0.002656 | 0.482565 | 0.365819 | 0.383240 |
| 2/policy | Unreal-HRT | 0.345569 | 0.425607 | 0.326885 | 0.062844 | 0.084284 | 0.306602 | 0.554655 | 0.048664 | 0.379653 | 0.324349 | 0.365390 |
| 2/minimal | Exact | 0.250461 | 0.258420 | 0.248721 | 0.078717 | 0.143243 | 0.234609 | 0.271350 | 0.006833 | 0.022273 | 0.244288 | 0.289623 |
| 2/minimal | Interactive | 0.252656 | 0.260577 | 0.250925 | 0.079037 | 0.143833 | 0.236947 | 0.273403 | 0.006823 | 0.021991 | 0.244288 | 0.291578 |
| 2/minimal | Unreal-VSM | 0.402986 | 0.483417 | 0.383544 | 0.044773 | 0.056294 | 0.353276 | 0.663620 | 0.002188 | 0.516684 | 0.375650 | 0.416186 |
| 2/minimal | Unreal-HRT | 0.375384 | 0.442377 | 0.359392 | 0.053797 | 0.062524 | 0.328665 | 0.592263 | 0.049086 | 0.408294 | 0.341712 | 0.396997 |

For continuity with the earlier Level-B domain, camera-facing geometric N·L > 0 partial RMS is also shown below. This domain is a diagnostic, not an exclusion used to manufacture the F0 verdict. Both domains independently exceed the 0.05 confirmation threshold.

| Pose / reference | Exact | Interactive | Unreal-VSM | Unreal-HRT |
|---|---|---|---|---|
| 0/policy | 0.175151 | 0.179221 | 0.284335 | 0.261857 |
| 0/minimal | 0.228012 | 0.230783 | 0.330234 | 0.301515 |
| 1/policy | 0.173666 | 0.177716 | 0.284341 | 0.262719 |
| 1/minimal | 0.227074 | 0.229761 | 0.330128 | 0.302083 |
| 2/policy | 0.175151 | 0.179221 | 0.284335 | 0.261857 |
| 2/minimal | 0.228012 | 0.230783 | 0.330234 | 0.301515 |

## Receiver-policy stop rule

Apply “difference greater than Exact’s error on any population” to population RMS, the gate’s aggregate error measure. Policy-film **partial** reference differences are smaller than Exact’s partial error. The **film-lit** population nevertheless triggers the frozen rule at every pose:

| Pose | Policy film-lit n | Exact RMS | Policy-reference difference RMS | Difference maximum | Rule |
|---|---|---|---|---|---|
| 0 | 285559 | 0.001871 | 0.006194 | 0.588348 | receiver-policy unit first |
| 1 | 285070 | 0.003244 | 0.006430 | 0.693542 | receiver-policy unit first |
| 2 | 285559 | 0.001871 | 0.006194 | 0.588348 | receiver-policy unit first |

The reference difference on partials is 0.116422 / 0.116101 / 0.116422 RMS. The minimal-epsilon reference produces 12,615 / 12,473 / 12,615 Exact missed-shadow endpoints versus 9,199 / 9,118 / 9,199 with the policy reference. Full policy-difference maps are `images/pose-{p}-policy-minus-minimal-x8.png`. Red shows visibility that the 1 mm reference retains and the minimal reference shadows. The measured origin/t_min policy change is real; F0 does not prove the minimal policy is a sufficient final receiver policy.

A further measured limit matters for future interpretation: Exact is **not exact on film-defined solid populations**. Its maximum film-lit error is 1 at every pose (policy-film lit RMS is in the table), because a dark centre can lie inside an otherwise sampled-lit footprint. The Level-F prose’s assumption that E is zero on those solids is false for this reference. The frozen 0.002 solid bound has **not** been relaxed and no F1 acceptance is claimed.

## Frozen static calibration and unavailable checks

Independent frozen production draws 65–80 are scored against each new R_F. `s = 2 SD / mean` for population RMS; F1’s Exact seed-0 RMS × (1+s), F3’s Exact absolute mean baseline, and the unchanged S4 contour-ρ formula are retained in `scores/summary.json:static_calibration` for every population. The partial calibration is:

| Pose / reference | Exact median RMS | 2-SD relative spread | F1 seed-0 RMS limit | S4 contour-ρ limit |
|---|---|---|---|---|
| 0/policy | 0.231579 | 0.000076134 | 0.231604 | 1.502680 |
| 0/minimal | 0.250458 | 0.000111106 | 0.250489 | 1.861354 |
| 1/policy | 0.232045 | 0.000066056 | 0.232045 | 1.501198 |
| 1/minimal | 0.250331 | 0.000069501 | 0.250337 | 1.858223 |
| 2/policy | 0.231579 | 0.000076134 | 0.231604 | 1.502680 |
| 2/minimal | 0.250458 | 0.000111106 | 0.250489 | 1.861354 |

No new candidate is accepted. F2 time-to-quality, F5 lag/ghosting, F6 temporal stability, F8 complete-sequence determinism and F9 production costs remain unmeasured. The independently warmed three-pose flip-book is not Q-walk, Q-cut or any moving-light/blocker sequence. T_q remains Islam’s decision. No threshold is inferred from a desired speed.

## Coverage and inspection

There are **zero source-covered pixels without an accepted footprint sample** in every reference. Accepted-count [minimum, p1, median, maximum] is [11, 210, 256, 256] at poses 0/2 and [45, 208, 256, 256] at pose 1. Both policies share those counts. Native availability still lacks 20 / 26 / 20 source receivers, including **4 / 5 / 4 film partials** under either policy. Full-reference populations are constructed before this availability mask. Native complete-domain results remain explicitly false/incomplete, rather than silently dropping absent geometry from an acceptance claim. Contours touching missing native support are counted separately; the scorer conservatively excludes support within 9 pixels of a missing output.

Verified visually with local image inspection: full R_F at both offsets for poses 0, 1 and 2; full ×8 error maps for Exact, Interactive, native VSM and native HRT at both offsets at poses 0/1; pose-2 complete overview/error sheets; the registered and +4-stop Film/Exact/Interactive HDR sheets; frozen 33×33 contact/self-shadow crops plus rubble, crevices, cliff-top and outer-silhouette regions; both analytic fixture sheets at both offsets. Pose-2 numeric film buffers and derived maps match pose 0; its independent return images were checked as well.

Exact and Interactive show paired bright/dark errors at footprint-scale micro-contacts, rubble and cliff-top transitions. The film image has partial coverage where the point images show binary details. Native VSM has more one-sided bright loss around contacts/crevices; denoised HRT has extensive contact loss and extra dark self-shadow points. The near-zero reference adds fine dark structure, most legible in the signed difference and crops. Registered exposure hides much of the scalar difference; +4-stop and direct visibility inspections do not justify discarding it. R_L also exposes silhouette integration, which remains outside production 1x anti-aliasing acceptance.

Image naming: `images/pose-{0,1,2}-{policy,minimal}-{reference,overview,crops,lit}.png`, `...-{Exact,Interactive,Unreal-VSM,Unreal-HRT}-{visibility,error-x8}.png`, and `{thin,stress}-{policy,minimal}-oracle.png`. `registered-{policy,minimal}.gif` is an independently warmed static-pose flip-book only. Only selected crops were individually inspected; every enumerated endpoint was not individually cropped and reviewed.

## GPU windows and verification latency

Every acquisition used atomic mkdir of `D:/Coding/renderlab-work/gpu-slot.lock` and tokened `owner.json`. Every build/run passed a fresh game / Unreal Editor / foreign renderer check and the actual **`\Memory\Available MBytes`** counter. Minimum accepted preflight: **7,848 MiB**; minimum monitored availability: **6,888 MiB**, above the required 6,144 MiB. Accepted logs have no game or foreign-GPU-process sample, abort or validation error. Foreign CPU builds were observed during an early build window; this is disclosed and no production performance timing is inferred from these reference runs. Processes were hidden/offscreen, with no viewer, window, input automation or desktop capture.

All **12 owned windows** were released; total held time **231.970 s**, longest **109.985 s**. A foreign worker may own the shared lock subsequently. Queue deferrals and all acquire/release timestamps remain in `gpu-windows.json` and `runs/*-deferred.json`. UTC times:

| Window | Acquired UTC | Released UTC | Held seconds |
|---|---|---|---|
| build-01 | 2026-11-04T23:32:12.923395+00:00 | 2026-11-04T23:34:02.908267+00:00 | 109.985 |
| build-02 | 2026-11-04T23:35:42.671177+00:00 | 2026-11-04T23:36:17.849450+00:00 | 35.178 |
| thin-policy-05 | 2026-11-04T23:39:47.828279+00:00 | 2026-11-04T23:40:01.984483+00:00 | 14.156 |
| thin-minimal-01 | 2026-11-04T23:40:22.897557+00:00 | 2026-11-04T23:40:28.815173+00:00 | 5.917 |
| stress-policy-01 | 2026-11-04T23:40:28.947292+00:00 | 2026-11-04T23:40:31.705609+00:00 | 2.758 |
| stress-minimal-01 | 2026-11-04T23:40:31.843187+00:00 | 2026-11-04T23:40:34.810136+00:00 | 2.967 |
| quarry-policy-p0-03 | 2026-11-04T23:44:39.452286+00:00 | 2026-11-04T23:44:49.716503+00:00 | 10.264 |
| quarry-minimal-p0-03 | 2026-11-04T23:44:50.031330+00:00 | 2026-11-04T23:45:00.866355+00:00 | 10.835 |
| quarry-policy-p1-03 | 2026-11-04T23:45:01.017527+00:00 | 2026-11-04T23:45:11.155702+00:00 | 10.138 |
| quarry-minimal-p1-03 | 2026-11-04T23:45:11.299685+00:00 | 2026-11-04T23:45:21.757172+00:00 | 10.457 |
| quarry-policy-p2-03 | 2026-11-04T23:45:21.890982+00:00 | 2026-11-04T23:45:31.215172+00:00 | 9.324 |
| quarry-minimal-p2-03 | 2026-11-04T23:45:31.345934+00:00 | 2026-11-04T23:45:41.338655+00:00 | 9.992 |

Build target: `f0/target`, **cargo -j 2**, release lib-test binary SHA256 **f823efca52c8ae0dd7ca0054c2bf3d4e087ab74b8b0f65ebe3f5edb5e4984d66**. The final test-only harness adds an ignored GPU entry and CPU WGSL validation; the only accessor in the renderer module is behind `#[cfg(test)]`. All runtime shader files remain unchanged.

**Implementation/analysis/reporting:** not separately stopwatched; work began with grounding at about 23:26 UTC and overlapped verification and GPU-queue delays. **Verification:** initial cold build 109.982 s failed on a test-helper mutability signature; corrected incremental build 35.167 s passed. All 75 nonignored renderer CPU tests pass (60 GPU tests explicitly ignored); focused WGSL validation passes; formatting, Python parsing and diff checks pass. Ten accepted GPU launcher processes total 86.629 s, including preflight/startup/capture/IO; quarry integration sections take 3.293–3.752 s each with validation and readback, not production frame time. Retained complete analytic fixture validation took 30.582 s after reusing the valid thin references; an initial slow analytic stress attempt was stopped and replaced by conservative empty-disk rejection, with no geometry or tolerance change, and was not separately stopwatched. First thin-only validation took 7.615 s. The initial scorer failed after 6.64 s because spread seeds were looked up in the four-seed mask directory; it was corrected to frozen b12 seeds 65–80. The first complete scoring pass took 98.213 s. A final pass corrected the metrics’ depth input back to the frozen reversed-Z contract and avoided unused duplicate domain calibration, taking **60.310 s**. No GPU reference was regenerated for CPU scoring corrections.

## Evidence and reproduction

Evidence root: **`D:/Coding/renderlab-work/m6-exact-quality/f0/`**. `provenance.json` binds source, scripts, binary and immutable inputs; `identities.sha256` hashes raw reference buffers, derived scores/images, manifests and logs. Generated measurements and private source assets stay outside Git.

- `quarry-{policy,minimal}-p{0,1,2}/pose-{p}.{film.bin,json,world.f32,normal.f32,hit.u32,exact.u32,exact.rgba16f}`: raw sums, centre evidence and run metadata. Film buffer records are 48 bytes: u32 visibility sums/counts (two halves), f32 moments and f32 HDR sums.
- `scores/summary.json`, `population-scores.csv`, `pose-{p}-{policy}-reference.npz`, and all `*-defects.csv`: complete static metrics, calibration, policy differences and enumerations.
- `fixture-validation.json`, `{thin,stress}-{policy,minimal}-analytic.npz`, `images/`, `gpu-windows.json`, `runs/`, `cpu-tests.log`: validation, inspection and resource evidence.
- Immutable baselines: `m6-level-b-probe/runs/quarry-b6/` (seeds 0–3, real hit masks), `m6-level-b-probe/gate-final/reference/` (point population reference), `m6-primary-prod/runs/accept-b12/quarry/` (spread seeds 65–80), `m6-level-b-probe/runs/accept-b7/` (HDR), and the exact `decoded-v2/native.npz` cases selected by `support/fresh_level_b_metrics.py`.

Scripts: [guarded run/build](../scripts/m6-film-run.py), [analytic validation](../scripts/m6-film-fixtures.py), [static scoring and images](../scripts/m6-film-score.py), [test-only generator](../src/film_reference_tests.rs), [accumulation shader](../support/film_accumulate.wgsl). Python scoring requires NumPy, SciPy, Pillow and Numba; use two math-library threads.

```powershell
$env:OPENBLAS_NUM_THREADS="2"
$env:OMP_NUM_THREADS="2"
$env:NUMBA_NUM_THREADS="2"
# Reuse the saved F0 manifests (paths to immutable local assets remain private).
py -3 -B scripts/m6-film-run.py --label fresh-build --minutes 15 -- cargo test -p renderlab --lib --release --no-run -j 2 --target-dir D:/Coding/renderlab-work/m6-exact-quality/f0/target
# Under the guarded runner, invoke the resulting lib-test binary with:
# renderer::fresh_shadow_tests::primary_ray_gpu_tests::film_reference_tests::gpu_film_reference
# --ignored --exact --nocapture --test-threads=1
# Use --manifest f0/{thin,stress}-{policy,minimal}.json first; validate both fixtures.
py -3 -B scripts/m6-film-fixtures.py --root D:/Coding/renderlab-work/m6-exact-quality/f0
# Then capture f0/quarry-{policy,minimal}-p{0,1,2}.json and rescore:
py -3 -B scripts/m6-film-score.py --root D:/Coding/renderlab-work/m6-exact-quality/f0
```
