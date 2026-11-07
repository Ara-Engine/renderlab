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

S2G uses U0b's separate `E:/renderlab-work/m7-u0b/ue-package`, `/Game/M7U0b/S2G/Bench`; U0's original package is untouched. The other S2G camera speeds and continuous-sun track, additional High/HRT runs, Q-zoom and Q-sun-slow remain outstanding. Smoke captures are not quality samples.

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

Initial CPU scoring uses the 4096-spp static p0 and 1024-spp frame 001/047 archives, 32 visibility rays per accepted primary. Canonical f32 camera inputs allow exact return-pose reuse; archive hashes, projection and black-primary metadata are checked. The current six scored output frames are 0, 1, 47, 113, 159 and 160; **155 frames still lack references**, so this cannot certify the motion gate. Full JSON retains raw/noise-subtracted RMS, signed mean and SE, maxima, reference noise, background distance bands and reference-subtracted consecutive-frame flicker. Age-32 comparisons and age-0–7 pools use the restored fixed T1b masks for both methods.

Illustrative **frame 160**, same T1b age ≥32 pixels, reference-noise-subtracted RMS in F0 codes (Epic/HRT):

| Population / exposure | T1b black-primary replay | TSR Epic/HRT |
|---|---:|---:|
| All-covered L0 | 10.2990 | 10.6116 |
| All-covered L4 | 16.1329 | 15.2495 |
| Edge L0 | 22.5929 | 16.7471 |
| Edge L4 | 47.9815 | 28.9363 |
| Background-centre L0 | 2.6889 | 0.3228 |
| Background-centre L4 | 8.2257 | 0.9652 |

Frame 159→160 reference-subtracted covered/same-placement, age ≥8 residual is **4.5054 versus 3.3345 L0**, and **6.3084 versus 3.9708 L4** (T1b black versus TSR). These sparse rows show mixed combined-radiance behavior; they establish neither universal superiority nor the original frozen gate. The complete pending witness set and held-pose reuse will supply the measured settle curve and fixed-population hold flicker.

## Cost method and initial screen

Capture and timing use different processes. The timing process alternates **no AA and TSR on the same full input track**, with a reset per replay and eight leading track frames excluded from each measured block. Two adjacent A/B pairs fit per process; four processes provide eight pairs. One scene upload is reused within each process. UE's installed `FCommandLine` limit is **16 KiB**; a failed over-limit attempt is excluded, rather than treated as a timing sample.

The native CSV `GPUTime` is the whole GPU-frame measure. `GPU/TemporalSuperResolution` is a pass stat, not a substitute for the frame's end-to-end increment. Both and the per-frame A/B deltas are retained. CPU/frame wall time is not substituted for GPU time. No Cargo runs inside a TR0 lease.

Current **desktop screens only**, RTX 4070 SUPER 12 GB / NVIDIA 617.14 / Ryzen 7 7800X3D. All completed visits are pooled without favourable-visit filtering in `cost-summary.json`. Capture processes and a block aborted below 2 GiB are excluded:

| Q-walk path / preset | A/B pairs; frames/method | No-AA frame p50 | TSR frame p50 / p95 | Paired frame increment p50 / p95 | TSR pass p50 / p95 |
|---|---:|---:|---:|---:|---:|
| VSM / Epic | 4; 612 | 1.7015 | 3.2094 / 4.2713 | 1.4966 / 2.4654 | 1.2083 / 1.8011 |
| VSM / High | 2; 306 | 1.6657 | 2.7438 / 2.7967 | 1.0723 / 1.1380 | 0.8171 / 0.8285 |
| HRT / Epic | 2; 306 | 1.7813 | 3.2662 / 3.3296 | 1.4837 / 1.5567 | 1.2175 / 1.2357 |
| VSM, globally no RT / Epic | 2; 306 | 1.6623 | 3.2332 / 3.2997 | 1.5648 / 1.6487 | 1.2031 / 1.2091 |

This is not the final quiet denominator and certifies no 2× claim. The original U0 screen (TSR pass 1.41–1.59 ms) is another workload/state screen, not silently averaged into this table. T1b's recorded total span was 1.108–1.128 ms, +0.203–0.223 ms over Interactive, on its separate Vulkan run; those are historical measurements, not a matched TSR A/B.

## Settle diagnostics and remaining work

`hold-diagnostic.json` reports full-image consecutive-frame changes and residuals to the **last held frame** after the F0 transform. That is useful for capture diagnostics but uses neither an independent reference nor T1b's covered/changed populations. Its settle threshold (mean <0.5 code and p99 <4, sustained to the end) is labelled as a self-residual proxy and cannot pass the eight-frame film gate.

On Q-walk, High/VSM's full-image median hold step is **0.1349 code**, with the proxy threshold first sustained 3 frames after the hold starts. Epic/HRT's is **0.1770 code**, already below that proxy threshold at the first held frame. These include the large black background and compare with the final hold; they are **not** directly comparable to T1b's covered-pixel 0.2769 / 0.3297 values, and are not film-settle results.

U0b now supplies quarry textures/RLE2 illumination and black-primary references. T1b's age masks have been recovered and verified above. U0b is prioritizing Q-walk 48/49, 63/64, 56 and return witnesses around 120/128; pose 64 also supplies every held frame 65–96. Original frozen raw references and Exact images remain deleted. The new physical target and old recorded bars stay separate. The full capture/reference matrix, post-TSR direct/visibility estimands, every-event film settle, and final quiet costs remain incomplete.

## Safety, provenance and retention

- Shared atomic directory lock: `D:/Coding/renderlab-work/gpu-slot.lock`, with token-checked release and reservation/release lines in `E:/renderlab-work/gpu-schedule.md`. Each own run is blocked to completion. A busy lock defers the run; it never removes another worker's lock.
- Normal preflight requires ≥4 GiB available memory and no game/foreign renderer; live guards abort below 2 GiB. During the explicit **GAME HOLD RELAXED**, TR0 allows only headless work, leases ≤3 minutes, child timeouts ≤120 seconds and a higher **6 GiB launch headroom**. Unreal is limited to two CPU cores. All timings during play are contended and ineligible for speed claims; a quiet rerun is queued. No windows/viewers, focus/input automation, desktop/game captures, heavy build or Nsight work occurs. The latest anchored schedule control is checked before acquiring a lease; a strict hold defers before reservation. The one `t1b-black-a` preflight caught `cs2.exe`, launched no GPU child and released immediately; the schedule hold arrived during that attempt.
- Other foreign renderers remain conflicts. Preflight records free/available memory and GPU/process inventory. `build-script-build.exe` is recognized as a CPU build helper after a false smoke abort. No Cargo was run by TR0, during or outside a lease.
- All output is beneath `E:/renderlab-work/m7-tr0`. No private asset or capture enters Git. The original package remains available for the other units.
- No patented reconstruction/denoising method is implemented or adopted. TSR is measured as the installed Unreal binary; no Epic implementation is copied. Renderlab's default and its patent-policy gates remain unchanged.
- `clean` removes owned raw `.bin/.npz/.npy/.exr` files larger than 5 MB only after a capture summary exists, recording sizes and SHA256s. Pending scoring inputs are retained until scored. No TR0 Cargo target exists.
- `prune` removes unused large native resource slices/outputs after readback, preserving the exact Tonemap-consumed HDR input and small exposure/parameter buffers for reference scoring. After the black control, original colored-replay full HDR is also summarized and deleted. Current ledgers total **18.86 GB** including the new no-RT capture's unused resources; every deletion is hashed. Remaining large inputs are pending scoring and will be deleted after their summaries are complete.
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
# Only after all scoring is summarized:
py -3 -B scripts/m7-tr0.py clean <fresh-label>
```
