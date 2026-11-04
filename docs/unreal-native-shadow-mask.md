# Unreal quarry: native main-view shadow masks and receiver isolations

2026-11-04; test branch based on `7120789`. This executes the next-run plan in [the mismatch diagnosis](unreal-shadow-mismatch.md), using the same existing cooked **Development main-view pipeline** for capture and fresh timing. **No tested configuration establishes matched fidelity.** The native mask resolves the zero-direct observability problem, but 20/28/20 source-only silhouette receivers, including one partial per pose, remain absent. Complete source-domain Level B remains undefined; these are nearly complete native-receiver profiles against the unchanged Renderlab reference.

Only scripts and this report enter Git. No engine build, cook, private asset change, engine-source copy, push, foreground focus change or desktop capture was performed.

## Evidence and capture contract

Private root `U = D:/Coding/renderlab-work/ue-m5/Quarry/`; reference `R = D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry/`. Final consolidated evidence is `U/native-shadow/final-report-02/`: `summary.json`, all-population `population-scores.csv`, and hashes of the locally inspected engine files. Every retained case has raw `Base/Passes.json`, `ResourceDescs.json`, effective `ConsoleVariables.csv`, shader parameter layouts/binaries and versioned texture binaries. Its `decoded-v2/` contains `native.npz`, readbacks, confirmed controls, scores, G6 coordinates, whole masks/error maps, selected crops and a paired main-view RGB cross-check. No reference was regenerated or rewritten.

- RTX 4070 SUPER 12 GB, Ryzen 7 7800X3D, NVIDIA 617.14; UE **5.7.4 CL51494982**, D3D12 SM6, 1920x1080, no AA/reconstruction, screen percentage 100.
- Source RLA SHA256 `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`; unchanged twelve meshes and placements. Every retained HRT capture/timing memory dump contains **13,034,903 native noncompacted Nanite RT triangles**. VSM retains its native view-selected Nanite representation.
- Existing packaged `UnrealGame.exe` SHA256 `69d6f88a9a51809d1454e3c99f4f1d56c039e5eaeb3f4399b2f4988c57d5a6e5`. Content was not recooked; the [previous package identities](unreal-quarry-full-detail-comparison.md) remain the content binding.
- Atomic GPU lease acquired **18:21:50.749743 UTC**, released **18:40:05.726135 UTC** (18 min 14.976 s). Acquisition/release records are in `U/slot-history/` and `U/../gpu-released.json`. All 34 launches passed game/renderer/RAM preflight; minimum free RAM **8.745 GiB**, no game/foreign-renderer monitor sample, and identical foreground handle/PID before and after every launch. The lock was removed on release.
- All launches used `-RenderOffscreen`, hidden/no-console process startup, native profiler UI off, and bounded execution. DumpGPU used **Upload=0, Explore=0, Stream=0, FrameCount=1, PassParameters=1, Texture=2, Buffer=1**, local directory only. Initial descriptor-only full-tree discovery established exact pass/resource names. Retained binary dumps filter **Root=Lights**; dump at CSV frame 64 captures engine frame 66 after warmup. Native camera-cache readback occurs at CSV frame 63. Textures are diagnostic readbacks, not timing samples.

Retained capture labels are `native-vsm-{baseline,screen-zero,normal-zero}-p{0,1,2}-01`, `native-hrt-{256,denoised,epsilon-zero}-p{0,1,2}-01`; the final baseline pose 0, HRT-256 pose 0 and denoised HRT pose 0 use suffix **02**. Initial discovery and the earlier pose-0 files remain as superseded diagnostics. `decoded/` was superseded by `decoded-v2/` for corrected world-position reconstruction and history export; visibility scores were unaffected by that reconstruction correction.

## Native mask, receivers and light readback

VSM's `VirtualShadowMapProjection(Input:GBuffer)` writes the projection factor; `CompositeVirtualShadowMapMask` composites into **ShadowMaskTexture, PF_B8G8R8A8**, consumed by `Light::StandardDeferred: M6Quarry_Sun`. HRT without denoising consumes **RayTracingOcclusion, PF_FloatRGBA**; denoised HRT consumes **Shadow.Denoiser.DenoiserOutput0, PF_FloatRGBA**. The decoder resolves the last writer **before the consuming pass**, rather than sorting opaque pointer/version names.

At this deferred directional-light input, attenuation is sqrt encoded. Decode channel R by squaring; channel B is the separately multiplying per-object factor, also squared. Here the directional surface scalar is `R² * B²` (B is one). Raw BGRA is reordered explicitly. The retained main-light RGB is before `ReflectionEnvironmentAndSky`, with indirect/AO/emission absent; native environment and exposure are retained for the subsequent complete main image. No gamma transform, fitted gain, shifting, resampling or replacement of absent geometry enters scoring.

Depth is dumped from **SceneDepthZ, PF_DepthStencil**, preprocessed by DumpGPU to R32 float device depth. **GBufferA, PF_A2B10G10R10**, decodes native world normal from its 10-bit RGB using `2*RGB-1`, without inventing geometric normals or normalizing away native packing. World positions are reconstructed in the registered source basis from device depth and the runtime camera cache; raw depth and decoded normals are retained with the mask.

Two readback corrections matter:

1. **Actual packaged near plane is 1 cm.** `r.SetNearClipPlane 0.5` clamps to 1 cm in the installed engine command, and the runtime camera inherits the global near plane (`PerspectiveNearClipPlane=-1`). The historical packaged claim of .5 cm was inaccurate. Device depth times independently retained source view depth verifies .01 m. This changes depth reconstruction, not these visible masks; all quarry geometry is well beyond either near plane. Native/source receiver displacement at pose 1 is median **.0989 mm**, p90 **.4545 mm**, p99 **1.6691 mm**, with large outliers at differing depth/triangle boundaries. Referenced View uniform layouts are dumped, but DumpGPU does not serialize the referenced uniform's contents; camera-cache and inline shader readbacks plus the depth check supply the reconstruction evidence.
2. **A light-property readback alone is insufficient.** Development's console `set` changed LightSourceAngle on the UObject while its existing proxy retained the old radius. After setting the exact angle and factor, temporarily toggling `r.Shadow.CacheWPOPrimitives` 1 then back to its native 0 rebuilds render states before warmup. Retained HRT shader readback is **SourceRadius .03998933732509613**, consistent with float32 `sin(.04)`; stale initial dumps read .03999227657914162 and are superseded. The scorer rejects stale radii. Requested diameter **4.583662361 degrees**, UObject readback **4.583663**, source-angle factor **1**. VSM's native control and both VSM isolations retain the same original diameter **4.584000**.

The main camera readback is source eye `[5.2,3.4,6.2]`, mapped UE `[-620,520,340]` cm, yaw/pitch **-39.313966/-18.837218** degrees, roll zero; horizontal FOV **77.639771** degrees, aspect **1.777778**, unconstrained, off-center offset zero. Pose 1 translates source eye/target by 1 cm in X; pose 2 independently reloads pose 0. Inline light direction is UE `[-.317999363,.423999161,.847998321]`, mapping to the registered source direction; RGB lux **[1000,947.306519,846.873230]**, soft radius zero, contact length zero.

## Effective controls

Every CVar explicitly named in either comparison/diagnosis document was read back. Complete per-capture values and `SetBy` provenance are in `Base/ConsoleVariables.csv` and `decoded-v2/confirmed-controls.json`; none of the requested controls was missing. Values below are native baseline values unless a configuration column specifies its sole receiver/sampling intervention. HRT light-angle correction described above is common to all HRT cases.

| CVar | Effective baseline / intervention |
|---|---|
| `r.ShadowQuality` | 5 |
| `r.ContactShadows` | 0 |
| `r.Shadow.Virtual.Enable` | 1 |
| `r.Shadow.Virtual.NormalBias` | .5; **0 only in vsm-normal-zero**, native .2-mm minimum remains |
| `r.Shadow.Virtual.ScreenRayLength` | .015; **0 only in vsm-screen-zero** |
| `r.Shadow.Virtual.SMRT.TexelDitherScaleDirectional` | 2 |
| `r.Shadow.Virtual.SMRT.RayLengthScaleDirectional` | 1.5 |
| `r.Shadow.Virtual.SMRT.ExtrapolateMaxSlopeDirectional` | 5 |
| `r.Shadow.Virtual.SMRT.RayCountDirectional` | 8 |
| `r.Shadow.Virtual.SMRT.SamplesPerRayDirectional` | 8 |
| `r.Shadow.Virtual.SMRT.AdaptiveRayCount` | 1 |
| `r.Shadow.Virtual.ResolutionLodBiasDirectional` | -1.5 |
| `r.Shadow.Virtual.MaxPhysicalPages` | 4096 |
| `r.Shadow.Virtual.Cache` | 1 |
| `r.Shadow.Virtual.OnePassProjection.SkipScreenShadowMask` | 1; directional sun still has the recorded surface-mask path |
| `r.Shadow.CacheWPOPrimitives` | 0; restored after the HRT startup refresh |
| `r.RayTracing.Shadows` | 0 VSM; 1 HRT |
| `r.RayTracing.Shadows.SamplesPerPixel` | 1; **256 in hrt-256 and hrt-epsilon-zero**; shader parameter independently confirms |
| `r.Shadow.Denoiser` | 1; **0 in the two 256-spp diagnostics** |
| `r.Shadow.Denoiser.TemporalAccumulation` | 1; inactive when denoiser=0 |
| `r.Shadow.Denoiser.HistoryConvolutionSamples` | 1 |
| `r.Shadow.Denoiser.MaxBatchSize` | 4; only first batched signal is the sun |
| `r.RayTracing.NormalBias` | .1 cm; inline shader .10000000149 |
| `r.RayTracing.Shadows.AvoidSelfIntersectionTraceDistance` | 1 cm; **0 only in hrt-epsilon-zero**, inline shader confirms |
| `r.RayTracing.Shadows.EnableTwoSidedGeometry` | 1; shader `bTwoSidedGeometry=1`; quarry material remains double-sided |
| `r.RayTracing.Culling` | 3 |
| `r.RayTracing.Geometry.InstancedStaticMeshes` / `.Culling` | 1 / 1 |
| `r.RayTracing.Nanite.Mode` | 1, source-count native BLASes confirmed |
| `r.Nanite.MaxPixelsPerEdge` | .25 |
| `r.GBufferFormat` | 1 |
| `r.ProfileGPU.ShowUI` | false |
| `r.DumpGPU.Upload` / `.Explore` / `.Root` | 0 / 0 / Lights |

Other explicitly retained SMRT values: local rays 8, local samples/ray 4, hair samples/ray 1, local texel dither 2, local extrapolation .05, local max slope bias 50, max ray angle from light .03. Unchanged original full-residency/streaming, 16x anisotropy, no-AA and exposure settings are recorded in each launch command and full CVar snapshot. VSM projection reports **bCullBackfacingPixels=1**; a native factor at a culled receiver is the renderer's actual lighting attenuation policy, not proof that a physical visibility ray was traced there.

## Unchanged Level B metrics

The exact `fresh_gate.py` facade is imported without calling `prepare()`. Metrics SHA256 remains **0ffacfc6621145b30e5b919e89d471539898be32e9766fbe1464f1b2c7e57322**. Each of the eighteen cases reproduces the frozen seed-0 production record exactly before scoring. Reference is still independent seeds 1-64 (16,384 rays); sigma and Q are unchanged. Full-reference populations and contours are formed before applying native availability; absent receivers remain missing. No definition or threshold changed.

| Pose | Source covered | Native covered | Evaluated common domain | Source-only / native-only | Partial evaluated / reference |
|---|---:|---:|---:|---:|---:|
| 0 | 346,520 | 346,518 | 346,500 | 20 / 18 | 8,765 / 8,766 |
| 1 | 345,883 | 345,871 | 345,855 | 28 / 16 | 8,775 / 8,776 |
| 2 | 346,520 | 346,518 | 346,500 | 20 / 18 | 8,765 / 8,766 |

All native zero-direct pixels have a mask value. The new paired main RGB still leaves **8,868/8,893/8,868 native receivers** unusable for division, which does not affect native-mask availability. It is no longer necessary to drop hundreds of reference partials for zero light. The remaining one missing partial per pose is genuinely absent native geometry, not a denominator failure. G4 has **zero unavailable contour outputs**, evaluated counts 4,416/4,426/4,416. Complete-domain gate output stays explicitly null because source-only geometry remains missing.

| Configuration | Pose | Partial RMS | p99 abs | Signed mean | G1 violations | G4 RMS | G6 endpoints |
|---|---:|---:|---:|---:|---:|---:|---:|
| VSM native | 0 | .369477 | .975979 | +.232113 | 7,829 | .232509 | 5,584 |
| VSM native | 1 | .368880 | .977067 | +.230592 | 7,865 | .233260 | 5,631 |
| VSM native | 2 | .369477 | .975979 | +.232113 | 7,829 | .232509 | 5,584 |
| VSM screen=0 | 0 | .342989 | .944226 | +.199854 | 7,809 | .223664 | 5,083 |
| VSM screen=0 | 1 | .344085 | .949152 | +.200438 | 7,873 | .225492 | 5,209 |
| VSM screen=0 | 2 | .342989 | .944226 | +.199854 | 7,809 | .223664 | 5,083 |
| VSM normal=0 | 0 | .287303 | .923318 | +.018686 | 7,600 | .119449 | 3,957 |
| VSM normal=0 | 1 | .284755 | .940699 | +.016997 | 7,634 | .119486 | 3,919 |
| VSM normal=0 | 2 | .287303 | .923318 | +.018686 | 7,600 | .119449 | 3,957 |
| HRT 256, raw | 0 | .198386 | .845144 | +.024234 | 4,817 | .067384 | 1,215 |
| HRT 256, raw | 1 | .193480 | .823040 | +.025374 | 4,843 | .068201 | 1,231 |
| HRT 256, raw | 2 | .198386 | .845144 | +.024234 | 4,817 | .067384 | 1,215 |
| HRT 1, denoised | 0 | .390556 | .947342 | +.077088 | 7,888 | .159592 | 3,629 |
| HRT 1, denoised | 1 | .393720 | .954545 | +.080135 | 7,918 | .166879 | 3,585 |
| HRT 1, denoised | 2 | .390556 | .947342 | +.077088 | 7,888 | .159592 | 3,629 |
| HRT 256, epsilon=0 | 0 | .198386 | .845144 | +.024234 | 4,817 | .067384 | 1,215 |
| HRT 256, epsilon=0 | 1 | .193480 | .823040 | +.025374 | 4,843 | .068201 | 1,231 |
| HRT 256, epsilon=0 | 2 | .198386 | .845144 | +.024234 | 4,817 | .067384 | 1,215 |

Production on these same partials has RMS **.006532/.006552/.006532**. G2 maximum error is **1 for every native case**. G6 coordinates are fully enumerated; crop enumeration is not a claim to have inspected every endpoint.

Per-population RMS, with membership fixed by the full reference:

| Pose / configuration | Partial | Narrow | Faint | Isolated | Silhouette | Lit | Occluded |
|---|---:|---:|---:|---:|---:|---:|---:|
| 0 VSM native | .369477 | .404517 | .077877 | .509016 | .354547 | .006588 | .665309 |
| 0 VSM screen=0 | .342989 | .361778 | .110373 | .445957 | .323168 | .019690 | .634158 |
| 0 VSM normal=0 | .287303 | .332141 | .218370 | .450862 | .276336 | .024748 | .591515 |
| 0 HRT 256 raw / epsilon=0 | .198386 | .229401 | .218308 | .344842 | .160101 | .063539 | .291607 |
| 0 HRT 1 denoised | .390556 | .403083 | .139539 | .446899 | .377100 | .048225 | .570062 |
| 1 VSM native | .368880 | .404222 | .088673 | .492655 | .336835 | .007409 | .667326 |
| 1 VSM screen=0 | .344085 | .363983 | .120391 | .436739 | .303763 | .019875 | .637120 |
| 1 VSM normal=0 | .284755 | .326466 | .199486 | .433177 | .307040 | .025370 | .593849 |
| 1 HRT 256 raw / epsilon=0 | .193480 | .223402 | .195523 | .326874 | .134317 | .064324 | .292800 |
| 1 HRT 1 denoised | .393720 | .402841 | .161674 | .450853 | .374955 | .048730 | .566797 |
| 2 VSM native | .369477 | .404517 | .077877 | .509016 | .354547 | .006588 | .665309 |
| 2 VSM screen=0 | .342989 | .361778 | .110373 | .445957 | .323168 | .019695 | .634158 |
| 2 VSM normal=0 | .287303 | .332141 | .218370 | .450862 | .276336 | .024748 | .591515 |
| 2 HRT 256 raw / epsilon=0 | .198386 | .229401 | .218308 | .344842 | .160101 | .063539 | .291607 |
| 2 HRT 1 denoised | .390556 | .403083 | .139539 | .446899 | .377100 | .048225 | .570062 |

Evaluated partial/narrow/faint/isolated/silhouette/lit/occluded counts are **8765/4724/1495/835/97/311340/26395** at poses 0 and 2; **8775/4676/1515/859/98/310677/26403** at pose 1. `population-scores.csv` preserves reference/evaluated/missing counts, RMS, mean, p99 and maximum for every case/population, including both epsilon variants separately.

## What each intervention removes

**ScreenRayLength=0** removes only VSM's screen-space start advance. It recovers part of the contact at `(928,325)` (native 1 to **.751111**) but leaves `(1203,669)` and `(1326,607)` at 1. The shading-normal offset remains. Mean and RMS improve modestly; faint/lit populations worsen. It does not establish receiver parity.

**NormalBias=0** leaves the screen correction active and the native .2-mm floor. At `(1203,669)` native 1 becomes **.248043**; `(928,325)` becomes **.124567**. Reference values there remain .000122/.000183. Contacts recover, but other contacts still disappear, additional dark features occur, and faint/lit error increases. The large positive partial mean is largely reduced; the full residual is not.

**HRT 256 with denoiser off** removes most one-ray variance but retains the native receiver/culling policy. At `(1203,669)` visibility is **.343323**, and at `(928,325)` **.078007**. At `(1388,529)` and `(945,276)` it remains **zero**; the first has reference **.998962**. The smooth/native receiver and geometric reference remain different. More samples do not fix that persistent self-shadowing.

**HRT self-intersection distance=0** removes the separate near backface-culled trace, with normal bias, two-sided setting, exact source angle, SPP and all other settings retained. **The full native visibility array is bit-identical to the 1-cm control at each of the three poses.** This isolation removes no measured visibility mismatch in these views; it does change measured cost below.

**Native 1-spp denoised main view** really runs reconstruction, preconvolution, temporal accumulation and final spatial accumulation. `bCameraCut[0]=0`, first-signal previous/current history and frequency are dumped. `denoiser-history.npz` uses the installed PENUMBRA_HISTORY layout: R normalized miss ratio, G accumulated count, B world blur radius in cm, A transmission distance. Pose-0 previous/current median counts are **1/1**, p90 **54.669/55.269**. Current count is exactly one at **4,418/8,765 partials**; 4,953 have count <=1, including invalid/rejected zero-count signals. Pose 1 has **4,397/8,775** exactly-one partials. This is heterogeneous accumulation, not globally absent history. At inspected contact/self-shadow probes, count remains one despite valid history. Blur-frequency buffers include native no-hit sentinel/saturated values, so large values must not all be read as finite physical penumbrae.

**RGB cross-check:** all paired main-view outputs use the same native pipeline, with shadows disabled only in the denominator process. Across cases, restricting this diagnostic to exactly equal dumped depth and normal gives maximum RMS **.00020447**, maximum absolute disagreement **.00451973**, and **zero errors above .01**. Every larger paired RGB disagreement occurs at receivers whose native depth/normal differs across processes; those outliers remain in the raw cross-check rather than being fitted or silently discarded. This restriction is never applied to the native-mask gate. The main-pass SceneColor v0 alias is not subtracted: unused SceneTextures references can cause its dump readback to occur after the writer. Versioned light outputs and full-tree pass ordering are used instead.

Verified visually: full packaged VSM main image; pose-0 native mask/error maps and crop sheets for native VSM, each VSM isolation, HRT-256 and denoised HRT; HRT-256 pose-1 crop sheet. Each crop sheet's first seven 33x33 source-pixel crops are centered at **(1203,669), (928,325), (1388,529), (1326,607), (945,276), (1577,595), (1254,549)**, followed by four highest-error partials. Rows are reference, seed-0 production, native candidate and signed error x8. Nearest-neighbor enlargement is display only. Inspected images show shortened VSM contacts, recovered but still displaced dark details with bias zero, binary denoised HRT structure and persistent dark HRT self-shadowing after convergence. Other retained crops/G6 endpoints are available but not all individually inspected.

## Fresh packaged timing, pose 0

All six configurations were retimed independently in the existing packaged Development build after their image configuration was established. No old timing is reused. Labels `fresh-time-{configuration}-p0-01` retain raw CSV, **native GPU trees**, command/readback log, native RT memory dump, screenshot, pre/postflight and contention samples. DumpGPU is not invoked. ProfileGPU at requested CSV frame 64 and screenshot at 72 precede the fixed measured **96-159 block, n=64**. These are single-process static pose-0 cost diagnostics, not replicated speed claims or motion timings.

| Configuration | GPU median [min,max], ms | GPU p95 | Frame wall median |
|---|---:|---:|---:|
| VSM native | 1.59650 [1.48620,2.61440] | 2.19139 | 2.60880 |
| VSM screen=0 | 1.64355 [1.52710,3.52790] | 2.39597 | 2.65000 |
| VSM normal=0 | 1.81405 [1.51330,2.35250] | 2.24323 | 2.59705 |
| HRT 256 raw, near trace=1 cm | 26.25730 [22.52140,38.46610] | 33.32218 | 26.59800 |
| HRT 1 denoised | 2.14975 [1.66750,3.45180] | 2.98554 | 2.63500 |
| HRT 256 raw, near trace=0 | 16.58935 [14.32620,21.14420] | 18.79813 | 16.98665 |

No configuration is proposed as matched fidelity. The cheaper epsilon-zero diagnostic has equal masks in these poses, but still fails the unchanged reference comparisons. GPUTime is the complete native frame field; tree parents and async queues overlap. GT/RT/RHI fields, ranges/p95/standard deviations and every parsed tree are retained in `summary.json`; they are not added or presented as a dependency-aware CPU trace. No new Renderlab timing was run, so these numbers establish neither renderer's 2x acceptance.

## Remaining blockers and reproducibility

1. Source-only geometry prevents a strictly complete-domain score: one missing partial and 19/27/19 other receivers. Native depth/normal/position are now available to diagnose the boundary and receiver disagreements; a controlled native-receiver reference would be a separately labeled diagnostic, not a replacement for this frozen gate.
2. Receiver offset, shading-normal culling, finite map filtering and native histories remain different from Renderlab's geometric-normal origin/projected interval. The 256-spp mask makes the persistent disagreement measurable. These data do not identify a physically unbiased winner.
3. These are three independently warmed static poses. **No new contiguous camera-motion/return G5 or moving-light gate was run.** Pose 2 must not be called a temporal return-history result. End-to-end radiance/environment filtering parity and complete dependency-aware CPU comparison remain unresolved from the prior report.
4. All tested partial errors remain far above production on the unchanged reference. Better native-mask observability is not fidelity acceptance. No setting is promoted and no performance/quality superiority is claimed.

Scripts: [guarded launch recipes](../scripts/ue-shadow-native.py), [RDG decode/scoring](../scripts/ue-shadow-dump.py), [consolidation](../scripts/ue-shadow-native-report.py), [focused decoder tests](../scripts/ue-shadow-native-test.py). The existing [runner](../scripts/ue-quarry-run.py) gained optional extra console commands and an optional CSV-event schedule; its existing preflight, lease bounds, process monitor and owned-process termination are reused. Future capture recipes also hash these scripts. Recorded commands and immutable dump inputs bind this run; initial launch recipes predate that additional recipe-hash field.

```powershell
# Requires an owned GPU lease and a successful per-launch preflight; no build/cook.
py -3 -B scripts/ue-shadow-native.py vsm-native native-vsm-next --pose 0
py -3 -B scripts/ue-shadow-native.py hrt-256 native-hrt-next --pose 0
# CPU only; decoder requires a fresh decoded-v2 directory.
$env:OPENBLAS_NUM_THREADS='2'
$env:OMP_NUM_THREADS='2'
py -3 -B scripts/ue-shadow-dump.py D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow/native-vsm-next --score
py -3 -B scripts/ue-shadow-native-test.py
py -3 -B scripts/ue-quarry-test.py
py -3 -B scripts/ue-shadow-native-report.py --output D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow/new-summary
```

The four focused decoder tests cover adjacent JSON objects, predecessor resource-version selection, packed BGRA/10-bit normal decoding, and nested/padded shader layouts. All four plus eleven existing quarry CPU tests pass. Implementation was not separately stopwatched. Verification used **388.709 seconds of total Unreal process time across 34 launches** (startup, warmup, reads/exports and profiling included, never substituted for frame time); the shared-GPU lease lasted 18 min 14.976 s while tooling/analysis overlapped it. Consolidation took about 1.26 s; CPU decode/scoring and cross-checks were separate short runs with two math-library threads. No heavy measurement build was needed.
