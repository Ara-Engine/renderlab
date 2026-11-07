# M7-TR0: native TSR motion baseline

2026-11-07, base `708e87c`, branch `t3/m7-tr0`. Measurement tooling only; no renderer or default changes. Contract: [M7](milestone-7.md), [design §6–8](m7-design.md), [U0](m7-u0.md) and [the recorded T1b experiment](m6-temporal-gpu.md#s2-t1b-camera-motion).

**Status: capture/timing checkpoint, not an accepted TR1 default bar.** Matched-reference scoring remains incomplete. Missing scores are unknown, not zero or passes. TR1 must beat **TSR Epic on every measure, on both paths, at no more than half its measured cost** before it becomes the default. A cheap temporal pass or a good hold image alone cannot establish this.

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
| `g-e-v-c1` | S2G camera 1 px/frame, Epic/VSM | 160 | 160 pass | 160 pass (`2^-12`) | 160 pass |
| `gz-e-n-c2` | S2G sun 0.25°/frame, Epic/VSM globally no RT | 160 | 160 pass | 160 pass (`2^-12`) | 160 pass; sun diameter pass |
| `g25-e-n-c1` | S2G camera 0.25 px/frame, Epic/VSM globally no RT | 160 | 160 pass | 160 pass (`2^-12`) | 160 pass; sun diameter pass |
| `g50-e-r-c2` | S2G camera 0.5 px/frame, Epic/HRT | 160 | 160 pass | 160 pass (`2^-12`) | 160 pass; sun diameter pass |

S2G uses U0b's separate `E:/renderlab-work/m7-u0b/ue-package`, `/Game/M7U0b/S2G/Bench`; U0's original package is untouched. Every S2G camera speed and the continuous-sun track now has a native capture; the complete paired RT/non-RT matrix, additional High rows, Q-zoom and Q-sun-slow remain outstanding. Smoke captures are not quality samples.

**Shared primary-background protocol (Islam, 2026-11-07): black everywhere.** Black is applied to primary misses **before temporal accumulation/integration**, with the environment still illuminating surfaces. Unreal already clears black. The reference uses per-sample black-primary compositing, including silhouette coverage; no post-composite or sky exclusion mask is used. The black-primary T1b control below passed, so the staged colored-clear Unreal fallback was cancelled. **Background-sky silhouette behavior is untested under this protocol.**

The earlier S2G reference's visible sky and T1b's colored quarry clear were real mismatches, found by inspection and HDR readback. Their old scores do not establish reconstruction dominance over black-primary Unreal. U0b now supplies black-primary references. For any future S2G T1b run, its constant sky must remain bound as an environment map: setting `lighting.environment=0` alone would also remove the old constant-sky fallback's foreground illumination. TR0 has launched no T1b S2G replay.

The registered S2G sun diameter is **0.532850749 degrees**. The runner restores it after U0's HRT preset, whose quarry override is 4.583662361 degrees. New runs read back `LightSourceAngle` and reject a mismatch. Earlier quarry VSM captures inherit the cooked 4.584-degree value; HRT explicitly uses 4.583662361 degrees. New captures explicitly set the exact 0.08-radian diameter on both paths; the small earlier rounding discrepancy remains disclosed.

## Recovered T1b data and black-primary control

No Cargo build was needed. The original retained b15 test executable, SHA256 `4e1a90bfa2d6482dc5338184809af918739f606911dff89a8d751c17e739e20a`, replayed the original 161-frame input with its original 60 scored frames. `t1b-restore-a/history-readback.json` verifies **180/180 image hashes, all three final histories, all 161 counter rows and all scored frame indices** against the retained original record. Original age-file hashes were not retained; their compatibility follows that deterministic replay evidence. Its process exited 101 because four Vulkan loader diagnostics named missing PRISMLiveStudio layer JSON files; it is not reported as a clean test pass.

`t1b-black-b` changes only the primary clear to black and keeps the RLE2 environment bound. Its audit passes: **all 60 age files byte-identical, all 161 counter rows identical, sun/visibility image and history controls unchanged, 346,517 cold covered HDR pixels byte-identical, and primary-miss corner RGB exactly zero**. Original quarry clear was `[30,35,45] * 2^-14`, quantized to f16. The black replay wrote every scored output and `motion.json`, then reached the 120-second process guard during teardown; process exit 1 is retained separately from the passing quality-data control. Validation was disabled for this controlled replay; it supplies no timing or clean-process claim.

Validated data: `E:/renderlab-work/m7-tr0/runs/t1b-black-b/data/`; audit: sibling `history-readback.json`. The original colored replay is `t1b-restore-a`. Neither variant is relabelled as the other. Unused sun/visibility raw files were deleted after hash verification; full HDR and age masks remain pending reference scoring.

## TSR versus the recorded T1b scores

The T1b column below is carried evidence from `b8fa1f3`, not a new render. `baseline` extracts all **3,285 converged population/channel/frame rows** and the frozen gate results into `E:/renderlab-work/m7-tr0/t1b-recorded.json`, with the original score and static-spread hashes. The Exact bar is preserved as `E × (1 + spread_2sd)`.

| Measure | Recorded T1b, Q-walk | TSR Epic | TSR High |
|---|---|---|---|
| F1 all-covered, moving frames 48/64, L0 / L4 / D4, × Exact bar | 0.65–0.69 / 0.60–0.64 / 0.74–0.79 | Pending matched reference/masks | Pending matched reference/masks |
| F1 partial L0 / D4, same frames | 0.61–0.65 / 0.64–0.68 | Pending | Pending |
| F1 occluded L0 / D4 | 0.71–0.72 / **1.57–1.69** | Pending | Pending |
| F1 edge L0 / L4 | **1.06–1.08 / 1.01–1.04** | Pending | Pending |
| F1 background-centre L0 / L4 / D4 | **1.37–1.42 / 1.46–1.49 / 1.44–1.46** | Pending | Pending |
| F1 hold frame 96, all-covered L0 / L4 / D4 | 0.12 / 0.13 / 0.14 | Pending | Pending |
| F1 return frame 120, background-centre L0 / L4 / D4 | **2.21 / 2.13 / 1.81** | Pending | Pending |
| R_F solid maximum, bound 0.002 | **0.40–0.61** moving; 0.10–0.28 hold | Pending visibility estimand | Pending visibility estimand |
| Silhouette trails | **5–8 px**, cliff-coloured trails into sky, reverses side on return | Pending reference/edge mask | Pending reference/edge mask |
| Background L0, 1.5–3 px / 3–6 px from silhouette, frame 64 | **9.80 / 7.09 codes**, Exact 0 / 0 | Pending | Pending |
| F2 disocclusion, all-covered L0 at age 3 / 7 | **18.0 / 13.4**, Interactive 11.4 / 8.3 | Pending event masks | Pending event masks |
| F2 background-centre L0 at age 7 | **39.4**, Interactive 11.3 | Pending | Pending |
| F3 bias | **996 failing cases** | Pending | Pending |
| F4 D4 pixels >16 codes, frame 128 | **3,934**, Exact 3,356 | Pending R_D reference | Pending R_D reference |
| F5 changed-pixel L0, frame 72 (+8 after stop) | 2.7519, bar 7.9715; ghost bias fails on 30 frame/channel cases | Pending changed mask/reference | Pending changed mask/reference |
| F6 moving residual, L0 | 3.1–3.8 codes, Exact 7.0–7.2 | Pending reference subtraction | Pending reference subtraction |
| F6 hold steps L0 / L4 (covered pixels) | 0.2769 / 0.3297 codes | Pending same population | Pending same population |
| Settle to the film bar after each event | Hold RMS passes at +8; F2 revealed age-7 **fails**; no universal passing settle count | Pending film-reference curve | Pending film-reference curve |
| Q-disocclusion, Q-sun and S2-glossy | **Not run on T1b** | Capture/score work in progress | Cost curve work in progress |

TSR's post-TSR colour must not be presented as film visibility `R_F`. A native shadow mask before TSR does not include TSR's reconstruction leakage and is not a substitute for it. Separate matched full, sun-only and visibility-output runs (or another validated equivalent) are needed for the original three estimands.

## New black-primary physical-reference rows

These are a **separate estimand**, not replacements for the frozen-bar ratios above. U0b's lossless `quarry-final` archives contain unexposed cd/m² full-radiance split halves, finite-disk physical direct radiance, conditioned visibility and geometric populations. Direct radiance differs from the original delta-BRDF D estimand; combined error also includes foreground material/lighting differences, not just reconstruction. Zero shutter matches Unreal's disabled motion blur. No fitted gain, exposure or geometry alignment is used.

CPU scoring uses the 4096-spp static p0 and 1024-spp moving/held-pose archives, 32 visibility rays per accepted primary. Canonical f32 camera inputs allow exact return-pose reuse; archive hashes, projection and black-primary metadata are checked. Currently **45/161 output frames, including 16/60 original T1b scored checkpoints**, have references; **116 output frames and 44 original checkpoints remain missing**. This cannot certify the motion gate. Full JSON retains raw/noise-subtracted RMS, signed mean and SE, maxima, reference noise, background distance bands and reference-subtracted consecutive-frame flicker. Age-32 comparisons and age-0–7 pools use the restored fixed T1b masks for both methods.

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
| Hold 80→81 covered/same-placement, age ≥8 step RMS | 0.2774 / 0.3314 | 0.4320 / 0.4776 | 0.4325 / 0.4782 | 0.2785 / 0.3089 |

Silhouette and recovery measurements on that same new target:

| Measure | T1b black replay | TSR Epic/HRT | TSR Epic/VSM, no RT | TSR High/VSM |
|---|---:|---:|---:|---:|
| Frame 64 background L0 RMS, 1.5–3 px from edge | 11.2921 | 0.00406 | 0.00407 | 0.02128 |
| Frame 64 background L0 RMS, 3–6 px from edge | 8.1884 | 0.00641 | 0.00641 | 0.01010 |
| Revealed age 3, all-covered L0 | 20.7580 | 5.8874 | 5.8922 | 7.9817 |
| Revealed age 7, all-covered L0 | 18.8087 | 9.1064 | 9.1073 | 10.6791 |
| Revealed age 7, background-centre L0 | 42.1877 | 4.1748 | 4.1761 | 7.4430 |

Distance bands use raw RMS (the reference is nearly black there); recovery rows subtract split-half reference noise. The recovery pools currently contain only **87, 109 and 499 pixels**, respectively, from available original checkpoints, so their coverage is explicitly partial. They do not replace T1b's full historical pooled scores. The 0–1.5 px band retains large true silhouette-integration error and is also recorded; a small wider-band RMS is not a zero-pixel trail-width claim. Film settle and frame-72 changed-mask/ghost-bias certification still await frame 56 and the remaining event references. The measured fixed-population hold step is now available above; the earlier self-residual diagnostic remains separately labelled.

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

These ceilings use the paired **end-to-end increment**, not half the standalone TSR marker. No TR1 implementation has been timed or qualified here. `cost-per-frame.csv` retains **1,220 per-input-frame rows** with eight paired samples each, median/p95 frame and pass costs; the first eight cold-repeat frames are excluded as registered and remain in raw CSV. `cost-summary.json` keeps `quiet` and earlier `screen` rows separate. Capture processes and the memory-aborted earlier block are excluded. All game-time timing is ineligible; none supplies this table.

Cost rows are keyed to the registered **CSV input chronology**. Installed `LaunchEngineLoop.cpp` records `GPUTime` from `RHIGetGPUFrameCycles()`, which reads the latest completed GPU-frame value. Its asynchronous reporting lag has not been independently calibrated to each camera pose; the qualified denominator is the pooled paired full-track measurement. Native HDR camera/light alignment is independently verified and does not inherit this timing-attribution limit.

T1b's recorded total span was 1.108–1.128 ms, +0.203–0.223 ms over Interactive, on its separate Vulkan run. Those historical measurements and U0's earlier 1.41–1.59 ms TSR-pass screen are not pooled into the quiet denominator.

## Settle diagnostics and remaining work

`hold-diagnostic.json` reports full-image consecutive-frame changes and residuals to the **last held frame** after the F0 transform. That is useful for capture diagnostics but uses neither an independent reference nor T1b's covered/changed populations. Its settle threshold (mean <0.5 code and p99 <4, sustained to the end) is labelled as a self-residual proxy and cannot pass the eight-frame film gate.

On Q-walk, High/VSM's full-image median hold step is **0.1349 code**, with the proxy threshold first sustained 3 frames after the hold starts. Epic/HRT's is **0.1770 code**, already below that proxy threshold at the first held frame. These include the large black background and compare with the final hold; they are **not** directly comparable to T1b's covered-pixel 0.2769 / 0.3297 values, and are not film-settle results.

U0b now supplies quarry textures/RLE2 illumination and black-primary references. T1b's age masks have been recovered and verified above. U0b is prioritizing Q-walk 48/49, 63/64, 56 and return witnesses around 120/128; pose 64 also supplies every held frame 65–96. Original frozen raw references and Exact images remain deleted. The new physical target and old recorded bars stay separate. The full capture/reference matrix, post-TSR direct/visibility estimands, every-event film settle, and final quiet costs remain incomplete.

**S2G reference probe rejected:** `gr-1p-64-a` used U0b's retained tracer with unchanged S2G geometry, illumination and black primary background at the registered 1 px/frame track's frame 64. The first 1024-spp image had zero invalid pixels; the same-seed replay then differed and the test exited 101. Its 5.06-second render and 11.65-second process times are game-contended diagnostics, not performance evidence. The process is stopped and its lease was released at 15:14:39Z. No reference from that probe is accepted or scored. Raw output and generated source remain for U0b's diagnosis. Subsequent probes pin the shared executable by copy and hash, and request geometry/D/V export explicitly.

U0b narrowed this to independent acceleration-structure rebuilds: three 1024-spp repetitions on a fixed AS/pipeline are bit-identical, as are fixed-AS runs that recreate the shader module/pipeline. Independent AS rebuilds differ by at most two f32 ULPs in RGB; **all 80-byte geometry records, centre points/normals/IDs, accepted counts, D and visibility sums remain byte-identical**, with zero invalid pixels. The latest reported maximum absolute RGB difference is 0.00390625 cd/m², full-image RMS 3.32e-6. This is a bounded diagnostic, **not reference acceptance**; U0b's AS-build-preference check is pending. Native TSR data is unaffected.

TR0 paused new launches after U0b's quarry frame took 52.41 seconds, exceeding its 45-second contention cutoff with only 3.117 GiB available RAM, and continued CPU scoring. Short native captures resumed after headroom improved. The **game hold was released at 15:36:21Z**. TR0 explicitly reserved the second quiet window after rt1c, before dl3 and U0b's long timings. The queued `tq1` cost matrix uses one exclusive lease; old screens and any game-time timings remain separate and cannot supply the final speed denominator.

## Safety, provenance and retention

- Shared atomic directory lock: `D:/Coding/renderlab-work/gpu-slot.lock`, with token-checked release and reservation/release lines in `E:/renderlab-work/gpu-schedule.md`. Each own run is blocked to completion. A busy lock defers the run; it never removes another worker's lock.
- Normal preflight requires ≥4 GiB available memory and no game/foreign renderer; live guards abort below 2 GiB. During the explicit **GAME HOLD RELAXED**, TR0 allows only headless work, leases ≤3 minutes, child timeouts ≤120 seconds and a higher **6 GiB launch headroom**. Unreal is limited to two CPU cores. All timings during play are contended and ineligible for speed claims; a quiet rerun is queued. No windows/viewers, focus/input automation, desktop/game captures, heavy build or Nsight work occurs. The latest anchored schedule control is checked before acquiring a lease; a strict hold defers before reservation. The one `t1b-black-a` preflight caught `cs2.exe`, launched no GPU child and released immediately; the schedule hold arrived during that attempt.
- Other foreign renderers remain conflicts. Preflight records free/available memory and GPU/process inventory. `build-script-build.exe` is recognized as a CPU build helper after a false smoke abort. No Cargo was run by TR0, during or outside a lease.
- All output is beneath `E:/renderlab-work/m7-tr0`. No private asset or capture enters Git. The original package remains available for the other units.
- No patented reconstruction/denoising method is implemented or adopted. TSR is measured as the installed Unreal binary; no Epic implementation is copied. Renderlab's default and its patent-policy gates remain unchanged.
- `clean` removes owned raw `.bin/.npz/.npy/.exr` files larger than 5 MB only after a capture summary exists, recording sizes and SHA256s. Pending scoring inputs are retained until scored. No TR0 Cargo target exists.
- `prune` removes unused large native resource slices/outputs after readback, preserving the exact Tonemap-consumed HDR input and small exposure/parameter buffers for reference scoring. After the black control, original colored-replay full HDR is also summarized and deleted. Current ledgers total **22.84 GB** including the new no-RT and S2G captures' unused resources; every deletion is hashed. Remaining large inputs are pending scoring and will be deleted after their summaries are complete.
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
# S2G references require U0b's qualified retained binary; rejected replay output is never accepted:
py -3 -B scripts/m7-tr0.py reference <reference-label> --track s2g-camera-1px --frames 64 --compact-tool <checkout>/scripts/m7-u0b-score.py
# Only after all scoring is summarized:
py -3 -B scripts/m7-tr0.py clean <fresh-label>
```
