# M7-TR0: native TSR motion baseline

2026-11-07, base `708e87c`, branch `t3/m7-tr0`. Measurement tooling only; no renderer or default changes. Contract: [M7](milestone-7.md), [design §6–8](m7-design.md), [U0](m7-u0.md) and [the recorded T1b experiment](m6-temporal-gpu.md#s2-t1b-camera-motion).

**Status: Q-walk scoring and quiet costs complete; S2G reference scoring in progress.** All 60 original T1b checkpoints have new black-primary physical-target comparisons, and all eight paired S2G native captures are complete. The 0.25-pixel S2G track is fully scored on both paths (320 native frames); the other tracks' reference campaign remains in progress. Missing scores are unknown, not zero or passes. TR1 must beat **TSR Epic on every measure, on both paths, at no more than half its measured cost** before it becomes the default. A cheap temporal pass or a good hold image alone cannot establish this.

## What is actually registered

“S2” in T1b's record is section S2, the **quarry Q-walk**, not the later generated shadow bench named S2. T1b was scored on that 161-frame sequence only. Its record explicitly says Q-zoom, Q-disocclusion and moving lights/objects were not run. There are no recorded T1b scores for those or for S2-glossy; their table entries must remain unavailable.

- **Q-walk:** 64 one-centimetre steps with 0.05° yaw per frame, hold at the last pose through frame 96, then the exact reverse path. Source SHA256 `db018bb07934ee9746e802d1a5216e5348f76b7ad0f93c17f31ab529361d57de`.
- **Q-disocclusion:** 48 frames, 2 cm/frame translation along the original camera's right vector. Source SHA256 `cbb40f2bddbea1a7bb037fa7dbd058b265dd702e3fcb47764fdea175fd6d4db2`.
- **Q-zoom:** 49 frames, registered dolly and hold. Source SHA256 `64752c24c73ebff9d75c9cc33fa8d4f706dc6e8a8cd46102d192b1f39ba88286`.
- **Q-sun:** TR0 registers the T1 design's 0.25°/s, 10°/s then hold, and 5° step about source Y, starting with the original lighting manifest's normalized sun vector. These are new TR0 registrations, not historical T1b measurements.
- **S2-glossy:** U0's generated S2G relief slabs, with its original 0.25/0.5/1 px camera tracks and 0.25°/frame sun track. Its Unreal import/cook and reference sequences are supplied by U0b; no replacement scene is substituted here.

All tracks use a nominal 60 Hz simulation at **1920×1080 internal and output, TSR at 100%**. Unreal motion blur is disabled, so the matched reference has a **zero shutter** at each moving camera pose. A one-frame box-shutter reference is a different estimand and is not used interchangeably.

## Capture and readback

`scripts/m7-tr0.py` runs the retained packaged Development Unreal **5.7.4 CL51494982**, **D3D12 SM6**, from `E:/renderlab-work/m7-u0/ue-package/`. It does not use the deleted staged-build duplicate or project scratch.

The later S2G capture campaign uses **capture-only render-thread synchronization** (`r.OneFrameThreadLag=0`). Two 160-frame attempts with the original same-tick arm-before-reset order crashed in Unreal's GPU buffer readback; a nine-frame run passed. Reset-before-arm on that same tick allowed the full capture, but unsynchronized nine-frame repeats exposed two image phases (frame-0 difference 0.110902 F0 RMS code). The synchronized nine-frame controls are **9/9 HDR-byte-identical**, with matching exposure and measured TSR jitter. New full captures dump TSR parameters on their first two frames, then switch back to the small Tonemap tree. No shader, material, AA quality, resolution or performance-run setting changes. The new synchronized matrix remains separate from earlier capture variants.

Native TSR readback starts at jitter `[0.25,-0.3888889]` and `bCameraCut=0`: resetting at CSV95 leaves one preparation sample before the first recorded frame at CSV96. Thus the start-age pools are recorded-timeline/geometric-age comparisons, **not equal empty-history sample counts** between TSR and the original T1b replay. No cold-start superiority claim follows from them. The independent film target does not depend on a renderer's jitter phase.

- Hidden process startup, `CREATE_NO_WINDOW`, `-RenderOffscreen`, unattended/no sound/no crash UI. No desktop input, desktop capture, viewer or visible window.
- `r.EyeAdaptation.PreExposureOverride=0`, fixed manual exposure, no dynamic resolution. Epic is `sg.AntiAliasingQuality=3`, High is `=2`. Every setting in the installed `AntiAliasingQuality@N` section is checked against runtime `DumpCVars`, including **200% Epic history and 100% High history**. High is not silently forced to Epic's history resolution.
- U0's matched direct-light/environment subset, no Lumen GI/reflections; VSM 8×8 and HRT 1-spp/denoised paths are kept separate. `vsm` retains U0's RT-capable device setup; **`vsm-nort` additionally disables `r.RayTracing` globally and checks its readback**. These are not silently combined. The purpose is reconstruction of T1b's subset, not a new Lumen quality comparison.
- **Native main-view HDR**, read from the resource actually consumed by the Tonemap pass. Epic consumes `TSR.Output`; High at native resolution consumes a slice of `TSR.History.Color`. Resource name, description, writer version and SHA256 are recorded for every frame. No SceneCapture2D or HighResShot temporal substitute.
- `DumpGPU` says it starts **next frame**. It is armed one CSV frame before motion begins. The next tick's camera cache is read before applying the next pose, and every captured pose is checked numerically for source-space position, direction and horizontal FOV. Earlier smoke runs that lacked this correction are excluded.
- UE clamps the inherited global near clip to **1 cm**, while T1b registers 0.5 cm. This inherited U0 mismatch remains disclosed; it is not silently called projection identity.

The HDR is **pre-exposed**. With `r.DumpGPU.Buffer=2`, Tonemap's 32-byte `EyeAdaptationBuffer` records global exposure in `[0].x` and average local exposure in `[1].x`; its scalar parameters record `ColorScale0`. On **all 161 High/VSM and all 161 Epic/HRT Q-walk frames**, global exposure is exactly `2^-14`, local exposure is 1 and tint is white. The installed `FViewInfo::UpdatePreExposure` uses those factors for a warmed manual-exposure view, establishing the radiance scale for these captures. A new physical reference is multiplied by `2^-14` before the frozen F0 transform. This conversion is not image fitting. Matching reference transport, geometric populations and event/age masks are still required before publishing T1b-unit error scores.

Completed capture inventory at this checkpoint:

| Run | Scene / track / preset | Native HDR frames | Per-frame camera checks | Exposure checks | Light-direction checks |
|---|---|---:|---|---|---|
| `walk-epic-vsm-a` | Quarry Q-walk, Epic/VSM | 161 | 161 pass | Buffer was not enabled; unavailable | Static U0 setup |
| `w-h-v-c1` | Quarry Q-walk, High/VSM | 161 | 161 pass | 161 pass | Static U0 setup |
| `w-e-r-c1` | Quarry Q-walk, Epic/HRT | 161 | 161 pass | 161 pass | Static U0 setup |
| `w-e-n-c2` | Quarry Q-walk, Epic/VSM globally no RT | 161 | 161 pass | 161 pass | Exact sun diameter read back |
| `d-e-v-c1` | Quarry Q-disocclusion, Epic/VSM | 48 | 48 pass | 48 pass | Static U0 setup |
| `j-e-v-c1` | Quarry Q-sun-jump, Epic/VSM | 33 | 33 pass | 33 pass | This earlier capture predates the light readback; provisional |
| `sf-e-v-c1` | Quarry Q-sun-fast, Epic/VSM | 64 | 64 pass | 64 pass | 64 pass |
| `g1-e-n-s1` / `g1-e-r-s1` | S2G camera 1 px/frame, Epic/no-RT and HRT | 160 each | 160 each pass | 160 each pass (`2^-12`) | 160 each pass; sun diameter pass |
| `g25-e-n-s1` / `g25-e-r-s1` | S2G camera 0.25 px/frame, Epic/no-RT and HRT | 160 each | 160 each pass | 160 each pass (`2^-12`) | 160 each pass; sun diameter pass |
| `g50-e-n-s1` / `g50-e-r-s1` | S2G camera 0.5 px/frame, Epic/no-RT and HRT | 160 each | 160 each pass | 160 each pass (`2^-12`) | 160 each pass; sun diameter pass |
| `gz-e-n-s1` / `gz-e-r-s1` | S2G sun 0.25°/frame, Epic/no-RT and HRT | 160 each | 160 each pass | 160 each pass (`2^-12`) | 160 each pass; sun diameter pass |

S2G uses U0b's separate `E:/renderlab-work/m7-u0b/ue-package`, `/Game/M7U0b/S2G/Bench`; U0's original package is untouched. The synchronized matrix contains **1,280 native frames**. Every run also verifies the first two actual TSR jitter/camera-cut records. `s2g/synchronized-candidates.json` pins the eight recipes. Earlier unsynchronized/legacy S2G captures are retired, individually identified in `s2g/superseded-captures.json`, and their summarized raw dumps are deleted. High supplies the requested cost curve; it is not substituted for the Epic quality floor. Q-zoom and Q-sun-slow remain unmeasured extras, with no historical T1b score. Smoke captures are not quality samples.

**Shared primary-background protocol (Islam, 2026-11-07): black everywhere.** Black is applied to primary misses **before temporal accumulation/integration**, with the environment still illuminating surfaces. Unreal already clears black. The reference uses per-sample black-primary compositing, including silhouette coverage; no post-composite or sky exclusion mask is used. The black-primary T1b control below passed, so the staged colored-clear Unreal fallback was cancelled. **Background-sky silhouette behavior is untested under this protocol.**

The earlier S2G reference's visible sky and T1b's colored quarry clear were real mismatches, found by inspection and HDR readback. Their old scores do not establish reconstruction dominance over black-primary Unreal. U0b now supplies black-primary references. For any future S2G T1b run, its constant sky must remain bound as an environment map: setting `lighting.environment=0` alone would also remove the old constant-sky fallback's foreground illumination. TR0 has launched no T1b S2G replay.

The registered S2G sun diameter is **0.532850749 degrees**. The runner restores it after U0's HRT preset, whose quarry override is 4.583662361 degrees. New runs read back `LightSourceAngle` and reject a mismatch. Earlier quarry VSM captures inherit the cooked 4.584-degree value; HRT explicitly uses 4.583662361 degrees. New captures explicitly set the exact 0.08-radian diameter on both paths; the small earlier rounding discrepancy remains disclosed.

## Recovered T1b data and black-primary control

No Cargo build was needed. The original retained b15 test executable, SHA256 `4e1a90bfa2d6482dc5338184809af918739f606911dff89a8d751c17e739e20a`, replayed the original 161-frame input with its original 60 scored frames. `t1b-restore-a/history-readback.json` verifies **180/180 image hashes, all three final histories, all 161 counter rows and all scored frame indices** against the retained original record. Original age-file hashes were not retained; their compatibility follows that deterministic replay evidence. Its process exited 101 because four Vulkan loader diagnostics named missing PRISMLiveStudio layer JSON files; it is not reported as a clean test pass.

`t1b-black-b` changes only the primary clear to black and keeps the RLE2 environment bound. Its audit passes: **all 60 age files byte-identical, all 161 counter rows identical, sun/visibility image and history controls unchanged, 346,517 cold covered HDR pixels byte-identical, and primary-miss corner RGB exactly zero**. Original quarry clear was `[30,35,45] * 2^-14`, quantized to f16. The black replay wrote every scored output and `motion.json`, then reached the 120-second process guard during teardown; process exit 1 is retained separately from the passing quality-data control. Validation was disabled for this controlled replay; it supplies no timing or clean-process claim.

Validated data record: `E:/renderlab-work/m7-tr0/runs/t1b-black-b/data/`; audit: sibling `history-readback.json`. The original colored replay is `t1b-restore-a`. Neither variant is relabelled as the other. After all 60 checkpoints were scored against all three native candidates, large replay HDR/native dumps were hashed and deleted; age masks, counters, hashes, score summaries and inspected images remain. Raw regeneration requires the pinned original b15 executable and retained recipe.

## TSR versus the recorded T1b scores

The T1b column below is carried evidence from `b8fa1f3`, not a new render. `baseline` extracts all **3,285 converged population/channel/frame rows** and the frozen gate results into `E:/renderlab-work/m7-tr0/t1b-recorded.json`, with the original score and static-spread hashes. The Exact bar is preserved as `E × (1 + spread_2sd)`.

| Measure | Recorded T1b, Q-walk | TSR Epic | TSR High |
|---|---|---|---|
| F1 all-covered, moving frames 48/64, L0 / L4 / D4, × Exact bar | 0.65–0.69 / 0.60–0.64 / 0.74–0.79 | Frozen target unavailable | Frozen target unavailable |
| F1 partial L0 / D4, same frames | 0.61–0.65 / 0.64–0.68 | Same limit | Same limit |
| F1 occluded L0 / D4 | 0.71–0.72 / **1.57–1.69** | Same limit | Same limit |
| F1 edge L0 / L4 | **1.06–1.08 / 1.01–1.04** | Same limit | Same limit |
| F1 background-centre L0 / L4 / D4 | **1.37–1.42 / 1.46–1.49 / 1.44–1.46** | Same limit | Same limit |
| F1 hold frame 96, all-covered L0 / L4 / D4 | 0.12 / 0.13 / 0.14 | Same limit | Same limit |
| F1 return frame 120, background-centre L0 / L4 / D4 | **2.21 / 2.13 / 1.81** | Same limit | Same limit |
| R_F solid maximum, bound 0.002 | **0.40–0.61** moving; 0.10–0.28 hold | Not exposed by final colour | Not exposed by final colour |
| Silhouette trails | **5–8 px**, cliff-coloured trails into sky, reverses side on return | Physical-target extent below | Physical-target extent below |
| Background L0, 1.5–3 px / 3–6 px from silhouette, frame 64 | **9.80 / 7.09 codes**, Exact 0 / 0 | Physical-target rows below | Physical-target rows below |
| F2 disocclusion, all-covered L0 at age 3 / 7 | **18.0 / 13.4**, Interactive 11.4 / 8.3 | Physical-target rows below | Physical-target rows below |
| F2 background-centre L0 at age 7 | **39.4**, Interactive 11.3 | Physical-target rows below | Physical-target rows below |
| F3 bias | **996 failing cases** | Means/SE retained; Exact-bar test unavailable | Same limit |
| F4 D4 pixels >16 codes, frame 128 | **3,934**, Exact 3,356 | Original R_D unavailable | Original R_D unavailable |
| F5 changed-pixel L0, frame 72 (+8 after stop) | 2.7519, bar 7.9715; ghost bias fails on 30 frame/channel cases | Physical-target +8 rows below | Physical-target scores retained |
| F6 moving residual, L0 | 3.1–3.8 codes, Exact 7.0–7.2 | Physical-target rows below | Physical-target rows below |
| F6 hold steps L0 / L4 (covered pixels) | 0.2769 / 0.3297 codes | Same-population rows below | Same-population rows below |
| Settle to the film bar after each event | Hold RMS passes at +8; F2 revealed age-7 **fails**; no universal passing settle count | Original Exact/reference gate unavailable | Same limit |
| Q-disocclusion, Q-sun and S2-glossy | **Not run on T1b** | S2G reference scoring in progress; quarry extras provisional | Quiet cost curve complete |

TSR's post-TSR colour must not be presented as film visibility `R_F`. A native shadow mask before TSR does not include TSR's reconstruction leakage and is not a substitute for it. Separate matched full, sun-only and visibility-output runs (or another validated equivalent) are needed for the original three estimands.

## New black-primary physical-reference rows

These are a **separate estimand**, not replacements for the frozen-bar ratios above. U0b's lossless `quarry-final` archives contain unexposed cd/m² full-radiance split halves, finite-disk physical direct radiance, conditioned visibility and geometric populations. Direct radiance differs from the original delta-BRDF D estimand; combined error also includes foreground material/lighting differences, not just reconstruction. Zero shutter matches Unreal's disabled motion blur. No fitted gain, exposure or geometry alignment is used.

Archive keys `L0/L1` name the two sample halves. Table channels `L0/L4` instead name the frozen F0 transform at registered exposure and four stops brighter, as in T1b.

CPU scoring uses the 4096-spp static p0 and 1024-spp moving/held-pose archives, 32 visibility rays per accepted primary. Canonical f32 camera inputs allow exact return-pose reuse; archive hashes, projection and black-primary metadata are checked. U0b has completed all 31 prioritized canonical camera references and 62 aliases. Epic HRT, Epic no-RT and High each cover **93/161 output frames, including all 60/60 original T1b scored checkpoints**; **68 intervening output frames remain without references**. Full JSON retains raw/noise-subtracted RMS, signed mean and SE, maxima, reference noise, background distance bands and reference-subtracted consecutive-frame flicker. Age-32 comparisons and age-0–7 pools use the restored fixed T1b masks for both methods. Coverage alone cannot certify every event gate.

The static p0/p1 black-primary metadata was added retrospectively by U0b after a fail-closed metadata check. `quarry-final/black-primary-provenance.json` records source checkpoint `aea9dd4`, its hard-coded black primary-miss branch, original process record and a CPU audit of the top 64 rows (all primary misses, split L/D exactly zero). Archive hashes are unchanged. No immutable per-run source snapshot was saved; this is explicitly retrospective source validation, not fabricated runtime readback.

Illustrative **frame 160**, same T1b age ≥32 pixels, reference-noise-subtracted RMS in F0 codes (Epic/HRT):

| Population / exposure | T1b black-primary replay | TSR Epic/HRT |
|---|---:|---:|
| All-covered L0 | 10.2990 | 10.6116 |
| All-covered L4 | 16.1329 | 15.2495 |
| Edge L0 | 22.5929 | 16.7471 |
| Edge L4 | 47.9815 | 28.9363 |
| Background-centre L0 | 2.6889 | 0.3228 |
| Background-centre L4 | 8.2257 | 0.9652 |

Frame 159→160 reference-subtracted covered/same-placement, age ≥8 residual is **4.5054 versus 3.3345 L0**, and **6.3084 versus 3.9708 L4** (T1b black versus TSR Epic/HRT). These sparse rows show mixed combined-radiance behavior; they establish neither universal superiority nor the original frozen gate. The remaining event witnesses are required for the complete film-settle curve.

The current physical-reference side by side at **frame 64** uses the same T1b age ≥32 masks. Values are L0 / L4 noise-subtracted RMS codes; this table is distinct from the historical ×bar table:

| Measure | T1b black replay | TSR Epic/HRT | TSR Epic/VSM, no RT | TSR High/VSM |
|---|---:|---:|---:|---:|
| All-covered | 10.9147 / 17.5928 | 11.2447 / 16.6660 | 11.2464 / 16.6678 | 11.5701 / 17.0904 |
| Partial | 13.9361 / 22.5536 | 13.4978 / 20.9950 | 13.5073 / 21.0046 | 14.1688 / 21.9287 |
| Occluded | 28.9862 / 49.8995 | 31.2899 / 51.9517 | 31.2912 / 51.9528 | 31.4286 / 52.3805 |
| Edge | 23.5702 / 50.4996 | 17.8147 / 31.8806 | 17.8157 / 31.8781 | 18.6064 / 35.7946 |
| Background-centre | 1.5840 / 5.4559 | 0.3714 / 1.1681 | 0.3711 / 1.1675 | 0.5970 / 1.8505 |
| Hold frame 96, all-covered | 10.4028 / 16.9126 | 11.1930 / 16.5618 | 11.1944 / 16.5635 | 11.2989 / 16.7540 |
| Return frame 120, all-covered | 10.5893 / 16.8491 | 10.9734 / 16.0398 | 10.9746 / 16.0412 | 11.3961 / 16.5640 |
| Return frame 128, all-covered | 10.5484 / 16.6399 | 10.9011 / 15.8781 | 10.9020 / 15.8794 | 11.3689 / 16.4376 |
| Hold 80→81 covered/same-placement, age ≥8 step RMS | 0.2774 / 0.3314 | 0.4320 / 0.4776 | 0.4325 / 0.4782 | 0.2785 / 0.3089 |

Silhouette and recovery measurements on that same new target:

| Measure | T1b black replay | TSR Epic/HRT | TSR Epic/VSM, no RT | TSR High/VSM |
|---|---:|---:|---:|---:|
| Frame 64 background L0 RMS, 1.5–3 px from edge | 11.2921 | 0.00406 | 0.00407 | 0.02128 |
| Frame 64 background L0 RMS, 3–6 px from edge | 8.1884 | 0.00641 | 0.00641 | 0.01010 |
| Revealed age 3, all-covered L0 | 22.2403 | 7.6954 | 7.6911 | 9.7815 |
| Revealed age 7, all-covered L0 | 15.9747 | 7.7603 | 7.7602 | 9.0360 |
| Revealed age 7, background-centre L0 | 43.5149 | 4.7542 | 4.7530 | 7.7562 |
| Start geometry age 3, all-covered L0 | 10.5635 | 10.6214 | 10.6222 | 10.8469 |
| Start geometry age 7, all-covered L0 | 10.4236 | 10.6692 | 10.6705 | 11.0103 |
| Return 127→128 reference-subtracted covered/same-placement, age ≥8 L0 flicker | 4.4915 | 3.4247 | 3.4248 | 4.0075 |

Distance bands use raw RMS (the reference is nearly black there); recovery rows subtract split-half reference noise. Revealed-age pools now contain **993, 729 and 3,738 pixels**, respectively, across all 60 original T1b checkpoints; start-geometry-age pools contain 346,557 / 346,488. These are the complete recovered checkpoint pools on the new physical target; they do not replace historical frozen-target scores. The 0–1.5 px band retains large true silhouette-integration error and is also recorded; a small wider-band RMS is not a zero-pixel trail-width claim. The return flicker comparison uses 346,275 shared pixels. Film-settle certification still cannot borrow the deleted Exact/reference bar. The measured fixed-population hold step is available above; the earlier self-residual diagnostic remains separately labelled.

**Frame-56 witness is now ready.** A separate targeted CPU score (`physical-witness-scores.json`, preserving the full score) evaluates frames 64/72/80/81/96. The 56→64 physical changed mask contains **113,553 pixels**. At +8 after stop (frame 72):

| Changed-pixel measure | T1b black replay | TSR Epic/HRT | TSR Epic/VSM, no RT |
|---|---:|---:|---:|
| L0 noise-subtracted RMS | 11.1835 | 11.6562 | 11.6602 |
| L4 noise-subtracted RMS | 17.5940 | 17.3420 | 17.3466 |
| L0 signed mean / SE | -3.1651 / 0.03116 | +3.7510 / 0.03216 | +3.7586 / 0.03217 |
| L4 signed mean / SE | -3.8642 / 0.04990 | +5.0558 / 0.04840 | +5.0648 / 0.04840 |

This is a new physical-mask comparison, not T1b's old frame-72 2.7519/bar-7.9715 gate. At frame 64 on the same converged all-covered mask, L0 mean/SE is **-3.5901/0.01715** for T1b, **+3.9394/0.01760** for Epic HRT and **+3.9443/0.01760** for Epic no-RT. The material/lighting bias on this physical target must not be described as reconstruction error alone.

The original F5 bias condition compares `abs(candidate mean)` with **`abs(Exact mean) + 2*candidate SE`**. A combined physical-target mean exceeding two SE alone is not that test. Neither those means nor a self-steady proxy supplies an original film-settle count. The +8 witness, all 33 held-frame error rows and their uncertainty remain available without inventing an Exact value.

The targeted check also records thresholded **background error extent**: centre misses, T1b age ≥32, at least 1.5 px from the reference edge; mean absolute RGB error above 1/4/8 F0 codes. At frame 64 L0, both Epic paths have **zero pixels above any of these thresholds** in that domain. T1b has **6,913 / 4,136 / 2,596 pixels**; their 95th-percentile nearest-edge distances are **19.0 / 15.0 / 13.15 px**, maxima **26.25 / 22.20 / 21.40 px**. This is explicitly a thresholded nearest-edge extent, not connected-trail length or the historical visual 5–8 px estimate. It does not certify the 0–1.5 px integration band or visible-sky silhouettes.

Full-frame physical reference, T1b black replay, Epic on both paths and High comparison/error sheets are retained and inspected at frames **64 and 128**, at L0 and L4, under `E:/renderlab-work/m7-tr0/inspect/`. They use one fixed 0–64-code absolute-error scale, with saturation explicitly labelled and split-half reference differences shown separately. Per-sheet JSON pins every input hash. These support the silhouette and whole-image audit; they do not turn sparse checkpoint scoring into a complete movie certification.

## S2G quarter-pixel track: complete physical-target scores

`g25-e-n-s1` and `g25-e-r-s1` each score **160/160 frames**, with no missing reference, against D8-qualified zero-shutter targets. These use current reference-centre populations at all ages; only the explicitly named F2/F6 rows apply the shared offline geometry ages. T1b was not run on this scene. Values below are F0 codes unless labelled otherwise.

| Measure | TSR Epic, no RT | TSR Epic, HRT |
|---|---:|---:|
| All-frame covered L0 RMS, p50 / p95 | 6.0192 / 6.3036 | 12.2724 / 12.6189 |
| Frame 64 covered L0 / L4 RMS | 5.9036 / 5.9474 | 12.1110 / 7.7598 |
| Frame 64 metal L0 / L4 RMS | 11.3447 / 3.6634 | 26.0397 / 9.0205 |
| Frame 64 dielectric L0 / L4 RMS | 2.9944 / 0.4063 | 4.3814 / 0.5842 |
| Frame 64 edge L0 / L4 RMS | 16.4307 / 31.4413 | 19.1317 / 31.5886 |
| Frame 64 background-centre L0 / L4 RMS | 1.3202 / 3.7130 | 1.3185 / 3.7091 |
| Revealed covered L0, age 3 / 7 | 11.1681 / 8.5865 | 19.7145 / 16.2605 |
| Revealed background-centre L0, age 7 | 38.8100 | 38.7895 |
| +8 changed-mask L0 / L4 RMS, frame 72 | 12.2912 / 6.7295 | 23.9935 / 11.0446 |
| +8 changed-mask L0 signed mean / SE | -6.5529 / 0.04016 | -12.9202 / 0.07790 |
| Moving F6 L0 residual, p50 / p95 | 0.6987 / 0.7804 | 1.1332 / 1.2177 |
| Hold 80→81 covered/same-instance, age ≥8 L0 / L4 step | 0.19470 / 0.09368 | 0.34653 / 0.14718 |
| Frame 64 metal / dielectric highlight energy ratio | 0.09408 / 0.77545 | 0.41280 / 0.77634 |
| Frame 95 metal / dielectric highlight energy ratio | 0.09034 / 0.79880 | 0.43647 / 0.79268 |
| Full-image self-final settle proxy, frames | 0 | 0 |

RMS rows subtract split-half reference noise; F6 subtracts consecutive physical-reference changes. F2 age-3 / age-7 covered pools contain **47,959 / 46,438 pixels**, and age-7 background contains **7,656**. The fixed 56→64 changed mask contains **66,727** pixels. Hold 80→81 uses **1,341,774** shared covered pixels. Highlight ratios use the current reference's top-5% material masks with no gain fit. All per-frame populations, raw errors, uncertainty, centroid/peak drift, background bands/extents and age pools remain in the JSON/CSV bar.

The self-final proxy is the earlier full-image diagnostic (mean <0.5 and p99 <4 relative to the last held frame), not a film-settle result or an equivalent of the original Exact gate. A stable image can retain a large physical error. Whole-frame/error sheets at 64 and 95, L0 and L4, have been inspected. These combined final-colour errors include lighting/material/shadow differences as well as reconstruction; the two paths' difference is retained without attributing it solely to TSR. After scoring and inspection, both native raw inputs were hash-recorded and deleted (**2,654,208,000 bytes**). Regeneration recipes, runtime/camera/exposure/jitter checks and all summaries remain.

`scripts/m7-tr0-report.py` exports `report/s2g-score-summary.json` and `.md` under the evidence root. These tables add L4 frame quantiles, material centroid/peak drift and thresholded background-error extent, with source-score and script hashes. Each missing full track stays unavailable. F6 includes actual camera changes (including the final step into the hold) or every continuously changing Sun input; a stationary camera does not exclude the Sun track. Centroid/peak displacement and nearest-edge error extent are diagnostics, not a connected silhouette-trail width.

## Cost method and quiet results

Capture and timing use different processes. The timing process alternates **no AA and TSR on the same full input track**, with a reset per replay and eight leading track frames excluded from each measured block. Two adjacent A/B pairs fit per process; four processes provide eight pairs. One scene upload is reused within each process. UE's installed `FCommandLine` limit is **16 KiB**; a failed over-limit attempt is excluded, rather than treated as a timing sample.

The native CSV `GPUTime` is the whole GPU-frame measure. `GPU/TemporalSuperResolution` is a pass stat, not a substitute for the frame's end-to-end increment. Both and the per-frame A/B deltas are retained. CPU/frame wall time is not substituted for GPU time. No Cargo runs inside a TR0 lease.

**Qualified quiet window `tq1`, 15:47:56–15:53:09Z**, after rt1c's quiet release and before dl3: one exclusive shared lease, no Cargo, no game or competing GPU work. Hardware: **RTX 4070 SUPER 12 GB / NVIDIA 617.14 / Ryzen 7 7800X3D**. The supported wallpaper pause was restored; saved quiet-start inventory reports **0% GPU use** and no game. The helper's actual timestamped proof is under `quiet/tq1/windows/20261107T154756494849Z/`.

All **32 processes** passed their runtime settings, SM6, sun-diameter, process-exit and memory checks. CPU `quiet-check` verified zero native TSR stat on every measured no-AA frame and a positive stat on every TSR frame. Four processes per configuration yield **eight A/B pairs**, **1,224 Q-walk frames/method** or **1,216 S2G frames/method**. Configuration order reverses on alternate visits. All visits are pooled without favorable filtering.

All values below are **milliseconds**. S2G uses its full registered 1 px/frame camera track, including hold and return. The lighting scope is the matched direct/environment subset above, not Lumen:

| Track / path | Preset | No-AA frame p50 | TSR frame p50 / p95 / p99 | Paired frame increment p50 / p95 / p99 | TSR pass p50 |
|---|---|---:|---:|---:|---:|
| Q-walk / globally no-RT VSM | Epic | 1.6667 | 3.2313 / 3.2911 / 3.3572 | 1.5615 / 1.6378 / 1.7061 | 1.2033 |
| Q-walk / globally no-RT VSM | High | 1.6652 | 2.8383 / 2.8936 / 2.9481 | 1.1694 / 1.2419 / 1.2833 | 0.8161 |
| Q-walk / HRT | Epic | 1.7776 | 3.2363 / 3.3014 / 3.4177 | 1.4546 / 1.5377 / 1.6324 | 1.2094 |
| Q-walk / HRT | High | 1.7793 | 2.8471 / 2.9038 / 2.9989 | 1.0610 / 1.1480 / 1.2102 | 0.8232 |
| S2G / globally no-RT VSM | Epic | 1.2441 | 2.6452 / 2.6851 / 2.7073 | 1.4011 / 1.4621 / 1.4860 | 1.2185 |
| S2G / globally no-RT VSM | High | 1.2417 | 2.2402 / 2.2795 / 2.3021 | 0.9967 / 1.0534 / 1.0835 | 0.8223 |
| S2G / HRT | Epic | 1.6436 | 2.9949 / 3.0247 / 3.0667 | 1.3499 / 1.4409 / 1.4697 | 1.2309 |
| S2G / HRT | High | 1.6369 | 2.5784 / 2.6178 / 2.6557 | 0.9424 / 1.0292 / 1.0757 | 0.8274 |

**Retained whole-frame outlier:** `tq1-g-r-e-1`, first pair, CSV input index 19: **22.4824 ms**, paired no-AA 1.6452 ms, TSR pass **1.2277 ms**. Other named GPU counters do not explain that whole-frame spike; no cause is asserted. It remains in every percentile and maximum. The other seven configurations' measured maxima are 2.79–3.77 ms. This record does not certify a no-spike claim.

Concrete **p50 ceilings for TR1**, in addition to beating Epic on every quality measure and the corresponding tail statistics:

| Epic comparator | TR1 reconstruction increment ≤ | TR1 total frame ≤ |
|---|---:|---:|
| Q-walk, no RT | 0.78075 ms | 1.61565 ms |
| Q-walk, HRT | 0.727325 ms | 1.618175 ms |
| S2G, no RT | 0.700575 ms | 1.322625 ms |
| S2G, HRT | 0.67495 ms | 1.497425 ms |

`scripts/m7-tr0-bar.py` exports the retained per-measure physical-target comparisons to `E:/renderlab-work/m7-tr0/bar/physical-measures.csv` and pins their sources in `bar/tr1-bar.json`. The JSON also contains all four Epic **p50/p95/p99 half-cost ceilings**. It preserves missing S2G cases explicitly, keeps the historical frozen-target file separate, and never declares TR1 accepted. Colour RMS, absolute bias, highlight energy-ratio error, spatial drift, disocclusion-age pools, background extents and reference-subtracted flicker remain individual measures; improvement in one does not cancel a loss in another.

These ceilings use the paired **end-to-end increment**, not half the standalone TSR marker. No TR1 implementation has been timed or qualified here. `cost-per-frame.csv` retains **1,220 per-input-frame rows** with eight paired samples each, median/p95 frame and pass costs; the first eight cold-repeat frames are excluded as registered and remain in raw CSV. `cost-summary.json` keeps `quiet` and earlier `screen` rows separate. Capture processes and the memory-aborted earlier block are excluded. All game-time timing is ineligible; none supplies this table.

Cost rows are keyed to the registered **CSV input chronology**. Installed `LaunchEngineLoop.cpp` records `GPUTime` from `RHIGetGPUFrameCycles()`, which reads the latest completed GPU-frame value. Its asynchronous reporting lag has not been independently calibrated to each camera pose; the qualified denominator is the pooled paired full-track measurement. Native HDR camera/light alignment is independently verified and does not inherit this timing-attribution limit.

T1b's recorded total span was 1.108–1.128 ms, +0.203–0.223 ms over Interactive, on its separate Vulkan run. Those historical measurements and U0's earlier 1.41–1.59 ms TSR-pass screen are not pooled into the quiet denominator.

## Settle diagnostics and remaining work

`hold-diagnostic.json` reports full-image consecutive-frame changes and residuals to the **last held frame** after the F0 transform. That is useful for capture diagnostics but uses neither an independent reference nor T1b's covered/changed populations. Its settle threshold (mean <0.5 code and p99 <4, sustained to the end) is labelled as a self-residual proxy and cannot pass the eight-frame film gate.

On Q-walk, High/VSM's full-image median hold step is **0.1349 code**, with the proxy threshold first sustained 3 frames after the hold starts. Epic/HRT's is **0.1770 code**, already below that proxy threshold at the first held frame. These include the large black background and compare with the final hold; they are **not** directly comparable to T1b's covered-pixel 0.2769 / 0.3297 values, and are not film-settle results.

U0b now supplies quarry textures/RLE2 illumination and black-primary references. T1b's age masks have been recovered and verified above. All original checkpoints, the frame-56 changed-mask dependency and adjacent return witnesses around 120/128 are now scored. Pose 64 supplies every held frame 65–96. Original frozen raw references and Exact images remain deleted. The new physical target and old recorded bars stay separate. Quiet costs and the eight-run S2G native matrix are complete; full S2G reference scoring, the post-TSR visibility estimand and every-event film settle remain incomplete.

**S2G reference qualification:** `gr-1p-64-a` failed its original strict replay and remains rejected. U0b traced the at-most-two-ULP secondary RGB variation to acceleration-structure rebuilds; fixed-AS runs were identical, including after pipeline recreation, and all geometry/D/V/count records were identical. Its qualified tracer now deterministically reconstructs barycentrics from the hardware-selected triangle and precisely encodes the small sun cone height. The latter removes the earlier 0.2108% S2G irradiance loss. Eight independent-AS/pipeline 1024-spp rebuilds of the original S2G frame-64 repro are bit-identical on mean, both halves and every geometry/D/V/count byte, with zero invalid pixels. Final Mitsuba checks pass: X3 bias +0.000443% / 0.107 sigma and finite-disk X2 +0.005112% / 1.212 sigma. Qualified executable SHA256 is `bf169ef7f8599bbff53bc8edcdb1875e8c117b09630e1c912e0f0f5d73335dfb`, pinned under TR0's `binaries/`. TR0 reuses U0b's qualified `s2g-camera-1px-064` archive (SHA256 `a283b188bd1057f12978a712f6a3423fbac3274b0c1ac55211026416941643a7`), rather than rendering it again. Rejected probe raw output/generated source have been hashed and deleted (154,723,570 bytes); its summaries/process/replay remain.

**S2G input and TSR diagnostic:** the registered camera-1px frame 64 is held for 64 warm frames in fresh sun-only no-AA and Epic TSR processes, globally no RT, with measured PE exactly `2^-12`, black primary background and actual sun diameter 0.532851 degrees. No material, asset or cooked package is changed. G-buffer A/B/C readback on unjittered geometric interiors reconstructs authored vertex-normal interpolation from reference hit primitive/world positions, including the source-back rule; it does not compare shading normals to face normals. Metal/dielectric shading-normal median errors are 0.04455/0.04451 degrees and maxima 0.08981/0.08872 degrees. Metal roughness reads 0.098039 (8-bit quantization of 0.1), dielectric 0.2; metallic, specular and linear base colours match source authoring within storage quantization. This rules out a gross import/material mismatch, not every shading-model approximation.

The same qualified physical-direct top-5% highlight masks give the following **linear energy/reference ratios**. Actual `TSR RejectShading` SceneColor input and the `TSR.Output` consumed by Tonemap are identified by resource version and SHA256; this is not an unrelated prepass buffer:

| Static sun-only stage | Metal roughness 0.1 | Dielectric roughness 0.2 |
|---|---:|---:|
| No-AA Tonemap input | 0.902249 | 0.908826 |
| Epic TSR actual RejectShading input | 0.835607 | 0.910870 |
| Epic TSR output consumed by Tonemap | 0.117872 | 0.799582 |
| Epic TSR whole-material input → output | 0.831121 → 0.123069 | 0.914201 → 0.836081 |
| Diagnostic −4-stop TSR output, unexposed then scored at registered −12 | 0.286557 | 0.821605 |

The metal loss therefore spans TSR and is not simply energy spread outside the highlight mask. This does not yet identify the internal cause. On the original full-reference highlight masks, physical direct/full is 0.998963 for metal and 0.998458 for dielectric, excluding omitted interreflection as the main explanation. The manual exposure diagnostic uses `r.ExposureOffset=-4`, PreExposureOverride 0 and measured PE exactly `2^-16`; native HDR is unexposed and scored/displayed at registered −12 using the same D reference and masks, without fitted gain. Multiplying the actual lower-exposure RejectShading RGB input by exactly 16, leaving alpha unchanged, **reconstructs the original entire f16 SceneColor buffer byte for byte** (SHA256 `2d5862f030696277a17ceb1910f5dba269b5d1f3fe2cd5e256903dae86cd759e`). Thus working-exposure dependence is demonstrated with identical physical input; most metal energy is still lost at −4 stops. Whole-material output ratios are 0.289562 metal / 0.855207 dielectric. This diagnostic never replaces the registered bar. Its one-minute lease lasted 39.311829 seconds and released at 17:40:30.853530Z before G1's queued quiet window; it provides no timing claim. Evidence is under `runs/gc-sun-aa0-a`, `runs/gc-sun-tsr-a` and `runs/gc-sun-tsr-m4-a`: `input-readback.json`, `colour-control-readback.json`, `pre-input-identity.json`, resource identities, physical scores and inspected `colour-control.png` images.

**Broader-pose reference correction:** the BF-qualified tracer still failed strict replay on the registered 0.25-pixel camera track's frame 0. Mean RGB changed in 2,072 pixels, maximum 2 ULPs / 0.001953125 cd/m², RMS 1.86491e-6; half 0 reached 3 ULPs. Geometry, D/V sums and accepted counts remained identical. U0b reproduced this with the AS and pipeline held fixed, ruling out AS rebuilding as the sole cause. Rounded barycentric-weight sums modulated constant-white vertex colours. Its sole shader correction uses affine-difference colour interpolation, preserving constants exactly. New binary SHA256 `d8bd9a8e1e1c7b5d39ee04d70c01ed4ee87bb654463db2b40daec1a96f35f139` passes eight independent-AS/pipeline rebuilds of that exact f0 repro with no RGB/half/geometry differences. There is no equality relaxation. The failed BF f0 is not scored. BF-qualified f64 results above remain separately identified.

TR0 pins D8 and reuses U0b's accepted `s2g-frame-zero` archive. The new physical corpus is `E:/renderlab-work/m7-tr0/s2g/physical/d8bd9a8e/`. U0b's D8 f64 qualification archive, `reference/colour-controls/S2G-f64-control-camera-1px-064.npz`, is also imported: eight independent AS/pipeline rebuilds are byte-identical, with actual HDR/D/V halves and geometry. The exact generated source SHA `4ae907…` was recovered byte-exact from the retained registered source and pose after scratch deletion; that recovery is recorded rather than presented as an original saved snapshot. New-shader X3 and finite-disk X2 Mitsuba controls both pass. The new/old f64 F0 difference has maximum 0.00003052 code and RMS 1.22e-6 / 8.09e-7 at L0 / L4. Old BF scores remain separately identified.

States 1 and 2 also pass their own strict replay. Every subsequent production state runs in its own test invocation so the first-case replay guard covers **every state**, with separate short leases. The camera campaign is running outside quiet windows, alternating with U0b's bounded work. A cooperative `gpu-defer.txt` gate can stop the next acquisition without interrupting or releasing an active state.

The imported f0 metadata serializes its f32 FOV (`0.7853981852531433` radians), while the registered recipe retains the original f64 literal (`0.7853981633974483`). Both are exactly **f32 bits 1061752795**. Scoring now checks those effective bits, consistent with the tracer's `View` input and the registration/deduplication keys. A two-frame CPU control passes and the adjacent representable f32 angle is rejected; no tolerance is widened or angle fitted. Per-frame score rows retain both literals and the effective bits. The inherited Unreal/T1b near-clip discrepancy remains separately disclosed.

The scorer checks SHA256 on the exact in-memory archive snapshot it decodes, rather than eagerly reading every unused reference and then reading selected files again. A full-resolution two-frame CPU replay preserves **all per-frame rows, F2 pools and flicker fields exactly** (`s2g/archive-snapshot-readback.json`). Its wall timings are explicitly ineligible for a speed claim; the retained change reduces bulk reads without changing score arithmetic or weakening input identity.

A three-state saved-replay batching experiment timed out at 120 seconds after its first image and was discarded. The validated single-state invocation then timed out at the same pose (camera-0.25px frame 55). Their first mean, both halves, SE and all geometry bytes are hash-identical; neither completed strict replay or is accepted. Concurrent foreign `stillkit` compilation is recorded, but no cause is assigned to the replay stall. Root authorized **600-second functional per-state guards**, with 11-minute shared reservations to fit the guard and setup. These are correctness runs, with timing ineligible; production stays one state per process. No foreign compiler is paused or killed.

The subsequent 600-second f55 attempt **passes strict replay and exits 0**, with every RGB/half/geometry byte identical. The harness reports 166.23 seconds; the owned process records 166.65 seconds and minimum available RAM 12.979 GiB. These wall durations do not supply GPU cost claims. Both earlier first outputs match this accepted output exactly; their rejected raw/staging copies (678,265,893 bytes) were hash-recorded and deleted. No shader, material or equality threshold changed. Subsequent production states continue through the same one-state guard.

Targeted scores of the synchronized 1-pixel captures against **D8 f64** retain the whole-image difference between paths. At frame 64, noise-subtracted all-covered L0 RMS is **5.3973 no-RT / 11.0541 HRT**. The fixed physical-reference highlight energy ratios are **0.15903 / 0.59444** on metal and **0.67460 / 0.62308** on dielectric. By frame 95 those ratios are **0.12012 / 0.58781** and **0.77321 / 0.67826**. Hold 80→81 covered/same-instance L0 step RMS is **0.18719 / 0.37629**, before external age filtering. These are combined final-colour errors, not an attribution to TSR alone. Full-frame/error sheets at 64 and 95 are retained for inspection; final age-filtered/full-track results remain pending.

`scripts/m7-tr0-s2g.py plan` registers all 640 S2G frames, deduplicated only by exact f32 effective camera/light inputs into 129 primary-centre geometries and 288 physical states. Shared offline age masks use same instance and 2% forward-depth acceptance, with explicit pure-translation infinity handling; exact static-view identity increments the same deterministic centre pixels directly. These are scoring masks, never renderer inputs. Analytic static, placement/depth rejection and newly revealed boundary controls pass, as does the qualified full-resolution static age control. Completed tracks can publish ages incrementally; the protocol lists every generated frame and explicitly remains incomplete until all 640 exist. Camera physical references also supply the centre geometry, avoiding a duplicate geometry render campaign. A controller handoff during incremental publication skipped one geometry copy at camera-0.25px frame 52. Its 1024-spp colour archive stayed hash-identical; a separate two-sample strict replay recovered only deterministic centre world/hit/depth. Its radiance is excluded from film scoring, and `geometry/frame-052-recovery.json` records the repair. The BF/D8 f64 world/hit/depth arrays used for ages also compare byte-identical. Lossless scoring archives omit redundant geometry copies and verify SHA256 for every retained NPY payload; the f64 CPU control retains all scored arrays exactly while reducing 82.85 MB to 53.00 MB.

TR0 paused new launches after U0b's quarry frame took 52.41 seconds, exceeding its 45-second contention cutoff with only 3.117 GiB available RAM, and continued CPU scoring. Short native captures resumed after headroom improved. The **game hold was released at 15:36:21Z**. TR0 explicitly reserved the second quiet window after rt1c, before dl3 and U0b's long timings. The queued `tq1` cost matrix uses one exclusive lease; old screens and any game-time timings remain separate and cannot supply the final speed denominator.

## Safety, provenance and retention

- Shared atomic directory lock: `D:/Coding/renderlab-work/gpu-slot.lock`, with token-checked release and reservation/release lines in `E:/renderlab-work/gpu-schedule.md`. Each own run is blocked to completion. A busy lock defers the run; it never removes another worker's lock.
- Normal preflight requires ≥4 GiB available memory and no game/foreign renderer; live guards abort below 2 GiB. During the explicit **GAME HOLD RELAXED**, TR0 allows only headless work, leases ≤3 minutes, child timeouts ≤120 seconds and a higher **6 GiB launch headroom**. Unreal is limited to two CPU cores. All timings during play are contended and ineligible for speed claims; a quiet rerun is queued. No windows/viewers, focus/input automation, desktop/game captures, heavy build or Nsight work occurs. The latest anchored schedule control is checked before acquiring a lease; a strict hold defers before reservation. The one `t1b-black-a` preflight caught `cs2.exe`, launched no GPU child and released immediately; the schedule hold arrived during that attempt.
- Other foreign renderers remain conflicts. Preflight records free/available memory and GPU/process inventory. `build-script-build.exe` is recognized as a CPU build helper after a false smoke abort. No Cargo was run by TR0, during or outside a lease.
- Under foreign compiler load, an inherited WMI memory query timed out and its conflict-monitor thread exited. TR0 now reads physical memory through `GlobalMemoryStatusEx`, retaining the PDH available-memory launch/live thresholds. Other inventory-query failures become an explicit stop condition for **only the owned child**; failed or nonfinite live-memory readings also fail closed. A CPU child-stop control passes. Initially `resource-pause.json` required 60 Cargo-free seconds after a resource/monitor failure. Islam's later T3-recovery instruction explicitly removes that wait for **correctness-only reference states**. Only the reference entry point bypasses it; the RAM, disk, game-policy, defer/foreign-lock, live-monitor and strict-replay guards remain active. The CPU queue control verifies this boundary without a GPU launch (`cpu-correctness-queue-readback.json`). Normal functional references may use the explicitly authorized 600-second guard; game-play limits remain 120 seconds/three minutes. No reference duration supplies a speed claim, and the earlier qualified quiet timing data is unchanged.
- Later, a ten-second `nvidia-smi` telemetry timeout stopped two functional attempts before acceptance. Hardware/GPU inventory remains mandatory in synchronous preflight. The live conflict worker now polls game/renderer processes and RAM without repeating optional GPU telemetry; mandatory inventory-query failures still stop the owned child. CPU routing controls verify both paths and fail-closed process-query handling (`cpu-live-inventory-readback.json`). Reference samples, shaders and strict replay checks are unchanged.
- All output is beneath `E:/renderlab-work/m7-tr0`. No private asset or capture enters Git. The original package remains available for the other units.
- No patented reconstruction/denoising method is implemented or adopted. TSR is measured as the installed Unreal binary; no Epic implementation is copied. Renderlab's default and its patent-policy gates remain unchanged.
- `clean` removes owned raw `.bin/.npz/.npy/.exr` files larger than 5 MB only after a capture summary exists, recording sizes and SHA256s. Pending scoring inputs are retained until scored. No TR0 Cargo target exists.
- `prune` removes unused large native resource slices/outputs after readback, preserving the exact Tonemap-consumed HDR input and small exposure/parameter buffers for pending reference scoring. All completed Q-walk/T1b replay, retired S2G variants and sun-only diagnostic raw dumps are summarized and deleted. `scripts/m7-tr0-retention.py` rebuilds `retention-summary.json` from unique owned path/content deletions, including compactor raw arrays, transferred archives, generated scenes and duplicate executables; imported provider deletion records are excluded. Every deletion is hashed. Both completed quarter-pixel S2G inputs are deleted; the other six native inputs remain until scoring and inspection finish.
- `docs/milestones-5-6-decisions.tsv` is untouched.

## Reproduce

```powershell
py -3 -B scripts/m7-tr0.py plan
py -3 -B scripts/m7-tr0.py baseline
py -3 -B scripts/m7-tr0.py cost
py -3 -B scripts/m7-tr0.py run <fresh-label> --track q-walk --quality 3 --path vsm --capture
py -3 -B scripts/m7-tr0.py summarize <fresh-label>
# Timing: omit --capture; use four fresh labels for eight A/B pairs.
# High: --quality 2. Hardware-shadow path: --path hrt.
# Globally disabled hardware RT: --path vsm-nort.
# S2G: --track s2g-camera-1px (automatically selects U0b's separate package).
# Physical-reference scoring, once U0b's lossless zero-shutter references are ready:
py -3 -B scripts/m7-tr0.py score <fresh-label> --reference <U0b-reference-directory>
# Restored original replay and controlled black variant, using the archived binary:
py -3 -B scripts/m7-tr0.py history <restore-label> --minutes 3
py -3 -B scripts/m7-tr0.py history-check <restore-label>
py -3 -B scripts/m7-tr0.py history <black-label> --black-background --minutes 3
py -3 -B scripts/m7-tr0.py black-check <black-label> --baseline <restore-label> --reference E:/renderlab-work/m7-u0b/reference/quarry-final/quarry-p0.npz
# Score both candidates on the new physical target, without reusing the old bar:
py -3 -B scripts/m7-tr0.py score w-e-r-c1 --reference E:/renderlab-work/m7-u0b/reference/quarry-final --history t1b-black-b
# Quiet cost matrix: only after the preceding rt1c quiet release, with the retained U0b quiet helper:
py -3 -B scripts/m7-tr0.py quiet <quiet-label> --quiet-tool <checkout>/scripts/m7-u0b.py
# S2G: import qualified controls instead of duplicating f0/f64, then generate missing states:
py -3 -B scripts/m7-tr0-s2g.py plan
py -3 -B scripts/m7-tr0-s2g.py import E:/renderlab-work/m7-u0b/reference/s2g-f0-affine-colour-retry/s2g-frame-zero.json
py -3 -B scripts/m7-tr0-s2g.py import E:/renderlab-work/m7-u0b/reference/colour-controls/S2G-f64-control-camera-1px-064.json
py -3 -B scripts/m7-tr0-s2g.py trace physical --phase camera --limit 129 --wait-seconds 3600 --timeout-seconds 600 --compact-tool <checkout>/scripts/m7-u0b-score.py
# Score completed tracks incrementally, before deleting any corresponding native HDR:
py -3 -B scripts/m7-tr0-s2g.py ages --tracks s2g-camera-0.25px
py -3 -B scripts/m7-tr0.py score g25-e-n-s1 --reference E:/renderlab-work/m7-tr0/s2g/physical/d8bd9a8e --ages E:/renderlab-work/m7-tr0/s2g/ages
py -3 -B scripts/m7-tr0-bar.py
py -3 -B scripts/m7-tr0-report.py
py -3 -B scripts/m7-tr0-retention.py
# Only after all scoring is summarized:
py -3 -B scripts/m7-tr0.py clean <fresh-label>
```
