# Unreal quarry shadow comparison: historical fallback-only captures

**Superseded for full-detail comparison by [the corrected Nanite/RT measurement](unreal-quarry-full-detail-comparison.md).** Review at `d8e3c38` established that these captures and all eight original profile processes used the 131,453-triangle fallback mesh. Asset enablement and CVar readbacks did not establish actual Nanite rendering. Their Level B values and timings are retained as historical fallback evidence only; they cannot establish a matched full-detail denominator. The corrected unit verifies native shading, source triangle counts and RT geometry allocations directly.

2026-11-04, based on `06bf45d`. The real quarry is imported into the existing private UE calibration project. Both requested shadow modes have registered 1920×1080 float32 direct-light pairs, and eight main-view profiler processes completed. **Level B is measured on the observable direct-light domain**, against the adaptive worker's subsequent 16,384-ray reference, with production scored alongside it. Undefined receivers remain explicit. **The combined fidelity/2× acceptance floor is not established.** Main-view screenshots expose an unresolved display/exposure mismatch; these timings are a preliminary direct-black screen, not the final matched denominator.

The exclusive GPU slot was released at **2026-11-04T15:11:04.8225397Z**, within the assigned 45-minute window. `D:/Coding/renderlab-work/ue-m5/gpu-released.json` records release, and `Quarry/gpu-release-preflight.json` found no game, Renderlab or Unreal processes. Initial work was CPU-only while VALORANT was running. Every Unreal launch rechecked processes, and live runs sampled game/foreign-renderer names and GPU state. No desktop/game capture, input automation, foreground activation or foreign application shutdown was performed. Images came from Unreal's EXR exporter and `HighResShot` output files.

## Inputs, import and matched settings

Private evidence root: `D:/Coding/renderlab-work/ue-m5/Quarry/`. The existing `M5Calibration.uproject`, its retained fixture, calibration scripts and [radiance/pre-exposure diagnosis](unreal-comparison-setup.md) were reused. The project file was not replaced. Public commits contain scripts and this report only; no scans, UE assets, generated measurements or Unreal implementation source.

The old composed GLB has lower-resolution textures. Instead, `ue-quarry-prepare.py` reads the **actual measured schema-1 4K RLA**, preserving its vertices, indices, UVs, linear vertex colors, material assignments and all twelve composed placement matrices. It writes private glTF/binary buffers and lossless mip-0 PNGs, not millions of JSON vertices. All original mip identities are retained. CPU affine baking uses inverse-transpose normals and the retained `UE_cm = 100*(-Z, X, Y)` calibration basis; the installed glTF importer's different basis is accounted for before import.

| Setting | Registered / observed result |
|---|---|
| Prepared quarry | SHA256 `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`; 13,034,903 source triangles, twelve placed meshes |
| Camera input | SHA256 `8a9c055339a625121360dbe93b512fcbb75afd583594a403ffe2d7d2c3606dd1`; original, +1 cm source-X translation, exact return |
| Poses | Original eye `[5.2,3.4,6.2]`, target `[0,.6,-.15]`; translation moves eye and target together; up `[0,1,0]` |
| Projection | 1920×1080; vertical FOV .85 rad; UE horizontal readback 77.6397705078°; capture near .5 cm; main-view near requested through `r.SetNearClipPlane .5` |
| Transform audit | All twelve imported world bounds equal staged expected bounds, maximum observed difference **0 cm**; this is not a render-buffer bit-identity proof |
| Lighting input | SHA256 `064d75ab6d0aa626aa498de33867a01ecefa37b36efe1383d23a3d5d2cf2ef25`; travel `[-.4,-.8,-.3]`, RGB lux `[1000,950,850]`, exposure −14 stops |
| Actual UE sun | Forward `[.317999364,-.423999152,-.847998304]`; reconstructed RGB lux `[1000,947.306514,846.873224]` |
| Source Angle | Requested full diameter **4.584°**, read back **4.5840001106°**; Source Soft Angle 0; contact shadows off |
| Materials | Twelve opaque, double-sided glTF materials; linear factor `[1,1,1,1]`, metallic 0, roughness .8500000238; Default Lit / dielectric Specular default .5 requested through the standard glTF material path; effective material-buffer calibration remains pending |
| Textures | Twelve 4096² sRGB base-color textures; runtime build log reports uncompressed BGRA8; `never_stream=True`, maximum size 4096, filter `TF_DEFAULT`, `r.MaxAnisotropy=16` |
| Environment | **Black**, no sky/GI/reflections/AO/fog, for the named direct-light subset on both sides. A matching Renderlab lighting override is staged. No quarry Renderlab rerun was performed in this slot |
| Sampling | Native 100% internal/output resolution, 1x/no AA (`r.AntiAliasingMethod=0`), no TSR/dynamic reconstruction |
| Exposure transport | Persistent SceneCapture HDR, RGBA32F EXR, manual exposure, physical camera off, no capture tonemapper/eye adaptation; supported `r.EyeAdaptation.PreExposureOverride=1` requested; no GPU pre-exposure-uniform readback |

Source identities, topology, world bounds, materials, per-mip RGBA hashes, converted buffer hashes and known mismatches are in `manifest.json`, SHA256 **`6b192241635dda9424fd7b523a50c62872fb6ae65f89e87df047f74cf6a1d842`**. Successful import/readback: `runs/assemble-02/import.json`; geometry, filtering and camera readbacks: `runs/views-02/`. All imported meshes have Nanite enabled, keep percentage 1 and trim relative error 0.

Two explicit candidates were captured:

| CVar | VSM + SMRT | Hardware shadows + default denoiser |
|---|---:|---:|
| `r.ShadowQuality` | 5 | 5 |
| `r.Shadow.Virtual.Enable` | 1 | 1 |
| `r.Shadow.Virtual.SMRT.RayCountDirectional` | 8 | 8, unused for RT sun |
| `r.Shadow.Virtual.SMRT.SamplesPerRayDirectional` | 8 | 8, unused for RT sun |
| `r.Shadow.Virtual.SMRT.AdaptiveRayCount` | 1 | 1, unused for RT sun |
| `r.RayTracing.Shadows` | 0 | 1 |
| `r.RayTracing.Shadows.SamplesPerPixel` | 1, unused | 1 |
| `r.Shadow.Denoiser` | 1, unused for VSM | 1 |
| `r.Shadow.Denoiser.TemporalAccumulation` | 1 | 1 |

Both use D3D12 **SM6**. Runtime logs verify Nanite=1, ray tracing enabled and supported tier 1.1, and console value/priority readbacks. SceneCapture's otherwise-disabled `use_ray_tracing_if_enabled` is explicitly true. The hardware profiler tree includes `DenoiseShadowVisibilityMasks`. Numeric zero getters alone are not used to establish CVar existence. The old `r.UsePreExposure=0` entry remains in the preserved calibration config; it is unsupported and is not evidence that pre-exposure is disabled.

## Known mismatches and limits

- **Main display/exposure:** saved main-view `main.png` files are heavily clipped and do not match the offline −14-stop display of the raw captures. The camera requested manual −14-stop bias, but its effective main-view exposure/display path has not been diagnosed. No gain was fitted, and no main-image fidelity pass is claimed.
- **Workload/environment:** the 14.65 ms Renderlab anchor includes RLE2 illumination and the registered nonblack environment. These Unreal runs are the direct-black subset from the [reduced-ray plan](m6-reduced-ray-gate.md). Neither a matched complete-environment denominator nor new direct-black Renderlab timings were measured.
- **Geometry:** Nanite raster detail is an approximation. Default hardware RT fallback readback totals **131,453 triangles**, versus 13,034,903 source triangles; fallback percentage is 1 and relative error is 1. Native Nanite RT mode was not selected or measured. This difference belongs in the error profile, not an exact-geometry claim.
- **Precision:** UE retained authored normals, recomputed tangents, and used its default packed tangent basis and half UVs. High-precision tangent basis/full-precision UVs are false. CPU f64 affine baking rounded to f32 differs from GPU f32 placement; no oriented render-buffer equality audit was performed.
- **Materials/filtering:** the retained sharp-specular disagreement is unresolved. Quarry effective GBuffer material values have not been captured. Mip 0 comes from exact prepared RGBA bytes, but UE rebuilds lower mips; runtime mip bytes, LOD choice and effective anisotropic sampler equivalence remain unverified.
- **Light/bias:** 8-bit UE light-color quantization reduces green/blue lux by approximately .284%/.368%. UE's angular light distribution and shadow bias/filter models differ from the source uniform slope disk and its bias. The exact disk diameter is 4.58366236°; 4.584° is rounded. The cone-versus-slope-disk difference is O(α²), about 0.2% at this radius.
- **Projection/output:** UE's infinite far plane differs from 100 m source far. Main-view near and postprocess effects still need image-level validation. Native UE main output and Renderlab Reinhard/sRGB output are different; EXRs are displayed offline with a common curve.
- **Timing/CPU:** UE is installed Development Editor in offscreen game mode, D3D12/SM6, core limit 8; Renderlab's historical anchor is a release Vulkan headless span. UE includes native viewport/Slate work. Engine CPU counters, Rust accepted-service time, GPU span and queued completion time are distinct boundaries. No dependency-level Unreal Insights CPU trace was collected.

## Raw visibility and Level B

Successful raw sets: `runs/vsm-capture-03/` and `runs/hrt-capture-01/`. Each contains **18 float32 RGBA EXRs**: shadowed frames 0, 1, 7, 31 and 63 at each pose, plus unshadowed frame 63 at each pose. These run on actual engine ticks, with a persistent capture view state. Shadowed original → translated → return histories stay contiguous; only the initial shadowed/unshadowed sequence starts with a cut. Thus translated/return first frames are motion observations, not independent cuts. Full per-pose cut replays remain unrun.

Each direct pair is retained in `extracted/<run>/pose-<n>-<state>-direct.npz`, including **original shadowed/unshadowed RGB**, unclipped visibility, validity, UE/source coverage and registered source depth. No alignment, resampling, fitted brightness or reference synthesis is applied. Visibility is the ratio of linear Rec.709 luminances; per-channel ratio disagreements are recorded. Unshadowed EXRs are byte-identical between the two modes on each pose. All raw EXR channels passed finite-value checks.

| Pose | Source receivers | UE coverage | Source-only / UE-only | Observable direct ratio | Dark denominator |
|---|---:|---:|---:|---:|---:|
| Original | 346,520 | 345,996 | 718 / 194 | 336,891 | 8,911 |
| +1 cm | 345,883 | 345,299 | 765 / 181 | 336,251 | 8,867 |
| Return | 346,520 | 345,996 | 718 / 194 | 336,891 | 8,911 |

These domains are identical for the two modes. Source-only receivers receive a deliberate NaN tag during extraction; the extraction JSON's `nonfinite` count refers to this masked input, **not corrupt/nonfinite raw EXRs**. Dark-denominator receivers are undefined, not assumed lit or shadowed. Magenta marks undefined source receivers in display derivatives. Small >1 ratios in some settling frames are retained, not clipped in measurements.

Root subsequently supplied `D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry/`. Its `pose-{0,1,2}-reference.npz` contains the mean of production seeds **1–64**, σ256, depth and coverage. Seed 0 is the independent shipping estimate; seeds 65–80 define q_prod. Nothing was regenerated here. q is **.018157958984375 / .01802825927734375 / .018157958984375**.

`ue-quarry-level-b.py` imports the supplied `support/fresh_gate.py` facade and its shared `fresh_gate_metrics` module in worktree `t3-fe465593`. The metric module SHA256 **`0ffacfc6621145b30e5b919e89d471539898be32e9766fbe1464f1b2c7e57322`** is identical to the original registered definitions; facade SHA256 is `3d220710e0dc43766556be5d84c4c5871189a5122c74359fc5dd17064461a810`. It never calls the facade's reference-generation/preparation function. Production's recomputed full-domain metrics exactly equal the adaptive worker's own seed-0 metric dictionaries on all three poses.

`level-b/input-audit.json` verifies matching asset, camera and lighting hashes, **zero source coverage/depth differences** against the registered Unreal pair inputs, and independent reprojection of about 4,100 saved source world points per pose: maximum **.00249 / .00262 / .00249 pixels**. Camera JSON poses match exactly; pose 1 is the registered 1 cm **source-X** translation of eye and target, not an invented along-view displacement. Resolution is 1920×1080. No image alignment or adjustment was used.

**These are observable-domain Level-B profiles, not full-domain acceptance.** Each row uses all measurable reference partial receivers; production is also evaluated on precisely those receivers for equal-population comparison. Full reference populations remain unchanged. Source-only/dark-denominator receivers are not filled with either perfect or zero visibility. The reference itself preserves Renderlab's geometry/bias/direction estimand; it is not an independent continuous physical oracle. UE geometry/normal/bias differences therefore contribute to these errors and must not be presented as a pure shadow-sampling comparison.

| Mode / pose, frame 63 | Partial evaluated / reference | RMS | p99 absolute error | Signed mean | G1 violations | G4 contour RMS | G6 extreme partials |
|---|---:|---:|---:|---:|---:|---:|---:|
| VSM original | 8,102 / 8,766 | .402768 | .983268 | +.228417 | 7,305 | .260725 | 5,562 |
| VSM +1 cm | 8,129 / 8,776 | .396733 | .984175 | +.221938 | 7,347 | .260188 | 5,632 |
| VSM return | 8,102 / 8,766 | .402768 | .983268 | +.228417 | 7,305 | .260725 | 5,562 |
| HRT original | 8,102 / 8,766 | .456366 | .989622 | +.113813 | 7,403 | .230588 | 3,040 |
| HRT +1 cm | 8,129 / 8,776 | .452587 | .988134 | +.108595 | 7,435 | .232662 | 3,006 |
| HRT return | 8,102 / 8,766 | .459018 | .989807 | +.116431 | 7,422 | .232362 | 2,998 |
| Production original, same observable subset | 8,102 / 8,766 | .006540 | .018127 | −.000060 | 0 | see full-domain below | — |
| Production +1 cm, same observable subset | 8,129 / 8,776 | .006546 | .018616 | +.000073 | 0 | see full-domain below | — |

Production **full-domain** partial RMS is **.00653178 / .00655136 / .00653178**, p99 **.0182312 / .0186310 / .0182312**, G1 violations **0** and G2 solid maximum **0** on every pose. Full production G4 contour RMS is **.00276864 / .00266024 / .00276864** over 4,416/4,426/4,416 contour samples; G6 extreme partial counts are **219 / 173 / 219**, including sub-quantum faint tails. UE has 3,840/3,854/3,840 measurable contour samples; the remaining 576/572/576 are explicitly missing. UE G2 observed maximum is **1** in every steady pose. Counts exceeding the .002 solid tolerance are VSM **10,080 / 10,034 / 10,080**, HRT **99,183 / 114,731 / 116,443**. These are profiles of Unreal error, not a requirement that Unreal itself pass Level A.

G3 sub-populations, original pose, same observable domain:

| Population | Observable / full | Production RMS | VSM RMS | HRT RMS |
|---|---:|---:|---:|---:|
| Lit | 309,342 / 311,355 | 0 | .034026 | .063627 |
| Umbra | 19,447 / 26,399 | 0 | .634154 | .587650 |
| Narrow | 4,416 / 4,724 | .006541 | .450493 | .493263 |
| Faint | 1,420 / 1,496 | .003980 | .159989 | .307839 |
| Isolated | 733 / 835 | .006611 | .525978 | .535280 |
| Silhouette-adjacent | 93 / 97 | .005060 | .401420 | .451662 |

All poses, signed means, p99/maxima and missing populations are retained in the per-case JSONs. The silhouette population is reported despite being below G3's n≥100 sub-population acceptance threshold. Missing partial counts are **664 / 647 / 664**; original-pose missing lit/umbra counts are 2,013/6,952. The missing domain is material to the result.

First captured shadowed-frame partial RMS is VSM **.402768 / .394813 / .402381**, HRT **.458230 / .445132 / .455549**. Only original is a fresh cut; translated and return are first motion frames. All thirty cut/settling/frame-63 states were scored. Frame 63 is a stationary sample after 64 engine ticks, not a proof of infinite-time denoiser convergence.

G5 uses the shared native `motion()` metric without reprojection. For each adjacent registered-pose pair, 3,572 partial pairs are observable out of 3,844; **272 remain missing**. Production's full-domain residual-change RMS/p99 are **.00815128 / .0216760**; on exactly the UE-observable pairs they are **.00815722 / .0217816**. Steady VSM is **.312659 / .928447** on both transitions. Steady HRT is **.546697 / 1.421437** for original→translated, **.530253 / 1.370370** for translated→return. Residual differences can reach ±2, so p99 above 1 is valid here. On the observable domain VSM's frame-63 return is bit-identical; HRT's is not (return RMS .081092, maximum 1). First-frame return is nonidentical for both modes. Full G5 results are in `level-b/G5.json` and `summary-final.json`.

G6 enumerates every extreme partial in per-case CSVs with V, σ256, visibility, signed error and crop bounds. **128,849 receiver crops across all thirty states** are packed into **511 indexed atlas pages**, with reference/production/UE rows and visibility/×8-error columns; worst-error enlarged crops supplement them. Complete shared-scale visibility/error sheets and motion overviews are retained. Grey is zero signed error, red/pink positive, blue negative, and pure magenta undefined. **No G7/full-fidelity pass is claimed:** inspected UE error maps show coherent contact/crevice differences absent from production's error map, and the unresolved coverage/material/exposure issues prevent full certification. No Level-A threshold was loosened to obtain these numbers.

## Preliminary complete-frame profiler screen

Hardware verified live: RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, Windows. UE **5.7.4 CL 51494982**, `5.7.4-51494982+++UE5+Release-5.7`. `engine-identities.json` hashes Build.version, editor executable, Renderer and D3D12RHI modules. Editor executable SHA256: **`ecce9996191f7f7e69b3d104eecdd0e678ae69176fdccd79828c3e75a0654d6d`**; Renderer DLL: **`577713eb0c65c2205471a91045c421e76bc4d71e6c508e2083384f24fadb0551`**. Frozen helper SHA, complete command line, CVar queries, preflight and contention samples accompany every process.

`launches/{vsm,hrt}-profile2-{p0,p1,p2,motion}/` holds **eight successful independent processes**, one per mode/pose plus alternating 0/1 motion. Static p0/p2 are independent repeats of the same physical view. Each requested 160 profiler frames; CSV retained 165 rows including trailing flush rows. **Rows 96–159, all 64 samples, are the predeclared measured block.** Earlier rows include warmup, `ProfileGPU` at 64 and `HighResShot` at 72. No sample within the block was discarded. CSV's final repeated header is authoritative when late stats append columns.

Native `GPUTime` is Unreal's GPU-frame counter in milliseconds; detailed per-queue `ProfileGPU` trees are also retained. For example, VSM frame 64 reports Graphics 2.00 ms and Compute .06 ms; overlapping queues must not be summed. These are frame/profiler measurements, not EXR export or process wall time. Capture actors are inactive during main-only profiling. No explicit GPU-validation flag was requested; this is not a validation-layer correctness certificate.

| Mode / workload | GPU median [min,max], ms | GPU p95 | Native GT critical median | RHI median | Frame wall median |
|---|---:|---:|---:|---:|---:|
| VSM original | 1.933 [1.581,3.156] | 2.622 | 2.252 | .120 | 4.058 |
| VSM +1 cm | 1.685 [1.592,3.107] | 2.647 | 2.198 | .121 | 4.108 |
| VSM return | 2.012 [1.575,3.045] | 2.584 | 2.153 | .118 | 4.034 |
| VSM alternating motion | 2.624 [1.585,4.909] | 4.461 | 2.891 | .148 | 5.497 |
| HRT original | 1.909 [1.647,3.690] | 2.922 | 2.321 | .119 | 4.467 |
| HRT +1 cm | 1.976 [1.674,3.407] | 2.944 | 2.737 | .144 | 5.523 |
| HRT return | 2.139 [1.660,3.520] | 2.955 | 2.268 | .125 | 4.181 |
| HRT alternating motion | 2.073 [1.663,5.231] | 4.518 | 2.600 | .139 | 4.818 |

GT/RT columns are Unreal's emitted `GameThreadTime_CriticalPath` / `RenderThreadTime_CriticalPath`, not sums of exclusive scopes. RT critical medians are .0007–.0008 ms; this small counter does not certify all parallel render-worker work. Full CPU p95/ranges, pass counters and standard deviations are in each **`summary-v2.json`**. VSM static shadow-depth/projection medians are about .31/.14 ms. HRT has a different lighting/denoising path; no selected-pass total substitutes for `GPUTime`.

Sampled profile runs found no games or foreign Renderlab jobs. Other user desktop applications stayed open; typical sampled graphics clock was 2805 MHz. Process-level GPU samples include loading and do not prove an idle desktop or isolate every measured frame. The wide ranges are retained. These short blocks provide a screen, not a broad variance campaign.

**2× verdict: not demonstrated; the preliminary numeric screen misses it by a large margin.** Using the historical 14.65 ms soft-1x anchor would require a matched Unreal frame of at least **29.3 ms**; the approximately 9.6 ms hard-4x anchor would require at least **19.2 ms**, while also matching softness and AA quality. These observed Unreal GPU medians are far below either number. The environment/exposure/geometry/AA and timing-boundary differences above prevent turning that observation into a final matched speed ratio or an engine-wide claim.

The current soft anchor's pipelined timing file has no paired CPU critical-path measurement. A separate validation-on diagnostic reports 1.8688 ms CPU admission and .0012 ms draw preparation alongside a **17.036 ms**, not 14.65 ms, GPU span. Historical hard-4x accepted render-service medians are .1667/.2037 ms, caller preparation separate, with 9.509/9.839 ms **mesh** intervals and 9.678/9.982 ms drained frame proxies. Those scopes cannot be equated to Unreal's GT critical-path counter. See [fresh compute costs](m6-fresh-compute-cost.md) and [hard-4x evidence](m6-history-quarry-cost.md).

## Inspection, failures and handoff

Inspected saved whole images: both modes' `extracted/*/pose-0-steady-visibility.png`, VSM registered-exposure `pose-0-steady-shadowed-display.png`, both `launches/*-profile2-p0/main.png`, `level-b/pose-0-reference-visibility.png`, `pose-0-production-error-x8.png`, both `*-pose-0-steady-error-x8.png` and `*-pose-0-lit-high-contrast.png`. The enlarged HRT crop at **(1105,641)** exposes the local reference/production/UE contact disagreement. Additional full translated/return sheets and indexed crop pages support review; the generated motion GIFs are reduced overview derivatives, with full PNGs authoritative. Images verify the intended quarry view and expose clipped main display, contact/crevice errors and undefined coverage; they do **not** establish G7. Prior calibration `ue-runtime/comparison.png` and retained Renderlab `m6-compute-cost/images/quarry-c2-pose-0.png` were inspected as context, not matched new outputs.

Preserved failures explain the retained path: initial 836.917 s NullRHI import saved all twelve mesh assets, then hit a native Slate assertion inside `ImportAssetTasks`; resume avoids reimport. One texture property was not Python-exposed; the public uncompressed preset replaced it. A missing level-subsystem save-as API was replaced with public `EditorLoadingAndSavingUtils.save_map`. An SM5 startup explicitly disabled RT and was stopped; SM6 was then verified. Runtime actor-spawn helpers are not Python-exposed, so inactive captures were preplaced. Four early CSV launches asserted before RHI initialization; successful profiling starts CSV via a runtime command after initialization. None of these failures supplies accepted image/timing evidence.

Successful assembly took 87.968 s whole-process, camera preparation 77.581 s, capture-actor preparation 24.659 s, VSM capture 23.039 s and HRT capture 43.089 s. Successful profiler processes took approximately 12.5–15.1 s each, including loading and shutdown. These are preparation/verification spans, **not GPU frame times**. Focused CPU test runs took about 1.1–2.5 s. CPU Level-B scoring/crop generation and editing/analysis were not separately stopwatched; no invented implementation-time total is supplied.

CPU-only checks: `py -3 -B scripts/ue-quarry-test.py` (seven focused tests), Python AST parsing and `git diff --check`. Tests cover affine/normal/winding conversion, sampler intent, truncated binary rejection, unclipped ratios/undefined coverage, unchanged reference populations and CSV late-header handling. No Rust/WGSL/viewer/cooker source changed, and no Cargo build or renderer GPU/reference run was performed.

CPU reference binding/scoring is complete. Reproduction is `py -3 -B scripts/ue-quarry-level-b.py` with the documented existing private inputs; outputs are new-only. The facade is imported read-only, and its reference preparation function is never run. Raw captures, native profiles, Level-B JSON/CSV/PNG/GIF artifacts and hashes remain private. Before further GPU work, root must assign a new slot. Remaining acceptance blockers are main exposure/display, full-environment/material/filter/fallback matching, undefined visibility receivers, and full G7/CPU-boundary verification. A fresh NullRHI import still has the documented native completion-UI assertion; the saved imported assets are the validated resume checkpoint, not a claim that a clean import command exits successfully.

Public API grounding: [SceneCapture2D](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/SceneCaptureComponent2D?application_version=5.7), [Interchange](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/InterchangeManager?application_version=5.7) and [Blueprint editor utilities](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/BlueprintEditorLibrary?application_version=5.7). Actual runtime logs/readbacks, rather than API descriptions alone, establish the observations above.
