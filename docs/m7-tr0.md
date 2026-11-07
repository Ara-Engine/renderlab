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
- U0's matched direct-light/environment subset, no Lumen GI/reflections; VSM 8×8 and HRT 1-spp/denoised paths are kept separate. The purpose is reconstruction of T1b's subset, not a new Lumen quality comparison.
- **Native main-view HDR**, read from the resource actually consumed by the Tonemap pass. Epic consumes `TSR.Output`; High at native resolution consumes a slice of `TSR.History.Color`. Resource name, description, writer version and SHA256 are recorded for every frame. No SceneCapture2D or HighResShot temporal substitute.
- `DumpGPU` says it starts **next frame**. It is armed one CSV frame before motion begins. The next tick's camera cache is read before applying the next pose, and every captured pose is checked numerically for source-space position, direction and horizontal FOV. Earlier smoke runs that lacked this correction are excluded.
- UE clamps the inherited global near clip to **1 cm**, while T1b registers 0.5 cm. This inherited U0 mismatch remains disclosed; it is not silently called projection identity.

The HDR is **pre-exposed**. A dump and camera match do not alone establish the physical radiance conversion. Per-frame exposure proof, matching reference transport, geometric populations and event/age masks are required before publishing T1b-unit error scores.

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

## Cost method and initial screen

Capture and timing use different processes. The timing process alternates **no AA and TSR on the same full input track**, with a reset per replay and eight leading track frames excluded from each measured block. Two adjacent A/B pairs fit per process; four processes provide eight pairs. One scene upload is reused within each process. UE's installed `FCommandLine` limit is **16 KiB**; a failed over-limit attempt is excluded, rather than treated as a timing sample.

The native CSV `GPUTime` is the whole GPU-frame measure. `GPU/TemporalSuperResolution` is a pass stat, not a substitute for the frame's end-to-end increment. Both and the per-frame A/B deltas are retained. CPU/frame wall time is not substituted for GPU time. No Cargo runs inside a TR0 lease.

Initial **contended screen only**, RTX 4070 SUPER 12 GB / NVIDIA 617.14 / Ryzen 7 7800X3D, Q-walk Epic/VSM, two A/B pairs, 306 measured frames per method:

| Measure | p50 ms | p95 ms |
|---|---:|---:|
| Whole GPU frame, no AA | 1.7037 | 2.3782 |
| Whole GPU frame, TSR Epic | 3.1784 | 4.2726 |
| TSR pass | 1.2032 | 1.7963 |

This is not the final quiet denominator and certifies no 2× claim. The original U0 screen (TSR pass 1.41–1.59 ms) is another workload/state screen, not silently averaged into this table. T1b's recorded total span was 1.108–1.128 ms, +0.203–0.223 ms over Interactive, on its separate Vulkan run; those are historical measurements, not a matched TSR A/B.

## Settle diagnostics and remaining work

`hold-diagnostic.json` reports full-image consecutive-frame changes and residuals to the **last held frame** after the F0 transform. That is useful for capture diagnostics but uses neither an independent reference nor T1b's covered/changed populations. Its settle threshold (mean <0.5 code and p99 <4, sustained to the end) is labelled as a self-residual proxy and cannot pass the eight-frame film gate.

The dependency is being closed in U0b: quarry texture/normal/RLE2-sky support, reference sequences and S2G import/cook. T1b's original raw references, compact populations and age buffers were deleted under retention policy; its recorded score JSON remains. Rebuilding those masks and the relevant full/sun/visibility reference estimands is required for the exact side-by-side gate. A new multi-bounce reference must not silently replace T1b's original direct-plus-environment reference while retaining its old numerical bar.

## Safety, provenance and retention

- Shared atomic directory lock: `D:/Coding/renderlab-work/gpu-slot.lock`, with token-checked release and reservation/release lines in `E:/renderlab-work/gpu-schedule.md`. Each own run is blocked to completion. A busy lock defers the run; it never removes another worker's lock.
- U0's preflight requires ≥4 GiB available memory and no game/foreign renderer. Its live guards abort below 2 GiB or on a game/foreign renderer. Preflight logs record both free and available memory, GPU inventory and foreign compiler contention. `build-script-build.exe` is explicitly recognized as a CPU build helper after a false smoke abort; renderer and game checks remain active.
- All output is beneath `E:/renderlab-work/m7-tr0`. No private asset or capture enters Git. The original package remains available for the other units.
- No patented reconstruction/denoising method is implemented or adopted. TSR is measured as the installed Unreal binary; no Epic implementation is copied. Renderlab's default and its patent-policy gates remain unchanged.
- `clean` removes owned raw `.bin/.npz/.npy/.exr` files larger than 5 MB only after a capture summary exists, recording sizes and SHA256s. Pending scoring inputs are retained until scored. No TR0 Cargo target exists.
- `docs/milestones-5-6-decisions.tsv` is untouched.

## Reproduce

```powershell
py -3 -B scripts/m7-tr0.py plan
py -3 -B scripts/m7-tr0.py baseline
py -3 -B scripts/m7-tr0.py run <fresh-label> --track q-walk --quality 3 --path vsm --capture
py -3 -B scripts/m7-tr0.py summarize <fresh-label>
# Timing: omit --capture; use four fresh labels for eight A/B pairs.
# High: --quality 2. Hardware-shadow path: --path hrt.
# Only after all scoring is summarized:
py -3 -B scripts/m7-tr0.py clean <fresh-label>
```
