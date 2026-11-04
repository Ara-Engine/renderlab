# Why the full-detail Unreal quarry shadows disagree

2026-11-04, CPU-only investigation of main `76d920e`. No GPU use, Unreal/viewer launch, desktop access or new capture. The saved reference and captures were not changed. This report supersedes speculation that the roughly 55–59x partial RMS gap is principally an EXR extraction error or a half-angle mistake.

**The main causes are unmatched shadow receiver rules and sampling.** VSM deliberately moves the lookup away from contact geometry much farther than Renderlab does. HRT retains approximately single-ray variance on these predominantly narrow penumbrae, despite its denoiser and 64 warmup ticks. Its different receiver offset also changes actual visibility, including persistent self-shadowing. Full source triangle counts and a matching light diameter do not match these estimands.

**The paired RGB ratio is working. There is no justified extraction-only correction or improved CPU rescore.** Missing direct-light receivers remain undefined. Native UE behavior explains the large aggregate discrepancy, but the existing files do not identify the cause of every individual flip or establish that SceneCapture and the timed main view have identical shadow histories.

## Inputs and reproducibility

Read the [full-detail comparison](unreal-quarry-full-detail-comparison.md), [reduced-ray gate](m6-reduced-ray-gate.md), [2x plan](m6-path-to-2x.md), extraction/scoring/capture code and frozen reference support. Engine source observations below refer to the **installed UE 5.7.4 CL51494982**, read locally. No Epic source is copied into this repository.

Private input root `U = D:/Coding/renderlab-work/ue-m5/Quarry/`:

- `runs/full-{vsm,hrt}-capture-01/`: actual paired EXRs and capture manifests; retained launcher/script copies and logs are in the corresponding `launches/` directories.
- `extracted/full-{vsm,hrt}-capture-01/`: raw RGB, unclipped ratio and validity arrays.
- `level-b-full/`: existing per-case scores, G6 coordinates/atlases, reference and production images.
- `R = D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry/`: immutable `pose-*-reference.npz`, seed-0 production, saved world positions and derivative geometric normals.

New, ignored evidence in this checkout's `measurements/`:

- `ue-shadow-diagnosis-final/diagnosis.json`, `audit.json`, native-resolution maps and selected 65x65 crops enlarged with nearest-neighbor sampling.
- `ue-shadow-rays/receivers.json`, `rays.json` and cone-culled source triangles. These are private source-asset derivatives and must remain ignored.

Scripts: [array/map diagnostics](../scripts/ue-shadow-diagnose.py), [EXR/source audit](../scripts/ue-shadow-audit.py), [source ray probes](../scripts/ue-shadow-ray-probe.py). All write evidence locally, without invoking a renderer. Use new output directories:

```powershell
$env:OPENBLAS_NUM_THREADS='2'
$env:OMP_NUM_THREADS='2'
$env:NUMBA_NUM_THREADS='2'
py -3 -B scripts/ue-shadow-diagnose.py --output measurements/ue-shadow-repeat
py -3 -B scripts/ue-shadow-audit.py --output measurements/ue-shadow-repeat/audit.json
py -3 -B scripts/ue-shadow-ray-probe.py --prepare --output measurements/ue-shadow-ray-repeat
```

The audit hashes four pose-0 EXRs against their manifests and verifies exact equality to the NPZ RGB; diagnostics hash all six steady NPZs and all three references. `audit.json` also hashes the relevant installed engine files. The source-ray probe reads the original RLA, not imported/fallback triangles. Its twelve selected receivers reproduce the saved reference to within 0.001 using an independent 1,024-direction disk; reconstructed primary intersections differ from saved positions by at most **0.000514 mm**. This validates the probe's geometry/coordinate convention, not UE's compressed GBuffer or denoiser.

## Spatial and statistical signature

These are recomputed original scores on the same observable reference-partial domains; no new validity rule or fitted transform:

| Pose | Observable / missing partials | Production RMS | VSM RMS / signed mean | HRT RMS / signed mean |
|---|---:|---:|---:|---:|
| 0 | 8,392 / 374 | .006535 | .360949 / +.224627 | .387781 / +.070541 |
| 1 | 8,424 / 352 | .006554 | .359810 / +.221785 | .389487 / +.066098 |
| return | 8,392 / 374 | .006535 | .360949 / +.224627 | .387802 / +.068956 |

I inspected both full pose-0 error maps, the existing VSM/HRT overviews, and enlarged worst-error crops at `(1203,669)`, `(928,325)`, `(1388,529)` and `(1326,607)`. Red/blue in the new maps mean positive/negative error on a fixed +/-1 scale; green means unavailable. Reference contacts are often shortened or absent in VSM. HRT has both lost contacts and additional dark pixels. Registration agrees outside these structures. The maps do not look like a uniform gain error or a common displacement.

Numerical controls support that interpretation:

- Removing the mean error still leaves RMS **.282537 VSM / .381311 HRT**. Even an unconstrained best affine prediction of UE from reference leaves **.263476 / .380036**. Those fits are falsification probes, never corrected scores.
- Exhaustive integer shifts of +/-3 pixels favor **zero shift** for both modes at all three poses. The fixed domain valid under every shift contains 2,197 pose-0 partials: unshifted RMS .297812/.370873, versus .339816/.437324 for the next best VSM/HRT shift. Local bias-induced shrinkage is not a global image translation.
- Errors are not confined to grazing light. Orienting saved derivative normals toward the camera, pose-0 partial RMS at geometric `N·L > .8` is **.297227 VSM / .391025 HRT**, on 1,144 receivers. At `0 < N·L < .04` it is .401698/.392681. The stored derivative normal's raw sign is opposite the camera-facing sign; interpreting it directly would give a false light-direction diagnosis.
- Large positive solid errors are real flips. Pose 0 has **4,620 VSM / 3,288 HRT** umbra receivers with error >.95, and **6 / 704** solid-lit receivers with error <−.95. HRT's 84,369 G2 failures above .002 are not all black/white flips: 80,802 are in the lit population, but only 911 lit errors exceed .01. A small native response/quantization discrepancy near white contributes heavily to that tight threshold; it cannot explain .388 partial RMS.
- Missing partials favor the terminator: their pose-0 camera-facing geometric `N·L` median is .0553; 337/374 fall below .2. This explains observability loss, not the large errors in the remaining bright receivers.

## Extraction hypotheses: tested and rejected as the main cause

For the opaque default-lit material and direct-only capture, the native light accumulation applies one surface-shadow scalar to both diffuse and specular direct terms. Schematically, `A_c = exposure * BRDF_c * shadow`, `B_c = exposure * BRDF_c`. Albedo, central-light cosine, roughness/specular response, source color and common exposure cancel within the pair. A different Renderlab BRDF affects radiance parity, but does not insert an extra `f(N·L)` into this UE ratio. Subsurface/transmission, emissive/ambient contamination, changed materials or unequal exposures would break that argument; they are not the configured quarry direct-light experiment. Installed `DeferredLightingCommon.ush:398` and `ShadingModels.ush:203` establish the relevant default-lit path.

Evidence against a hidden denominator/gain problem:

- The VSM and HRT pose-0 unshadowed EXRs are **byte-identical**, SHA256 `cfa9fdb7c11f770d9c9b569874221b50e139fc946349157d54c7d0e8159d521d`.
- The four audited RGB arrays contain no negatives, nonfinites or values at/above 65,504. Maxima are **261.5** for unshadowed/VSM and **261.0** for HRT shadowed. Although EXR channels are float32, all values round-trip through float16 exactly: the upstream precision is limited, but HDR saturation is not the problem.
- Across observable receivers, RGB channel-ratio disagreement has RMS **.0000309 VSM / .0003896 HRT**, maximum .001015/.005412. This is consistent with a shared shadow scalar plus finite precision, not large uncancelled material terms. The launcher requests pre-exposure override 1; fully lit VSM pairs also directly rule out a large common exposure ratio.
- Dropping dark denominators does not fix the score. At luminance >.1, partial RMS is **.359925/.386796**; at >5, **.341983/.384153**. Those restricted domains are diagnostics, not replacements for the registered population.
- Taking sqrt of the ratio gives .396841/.390655; squaring it gives .347356/.387068. Neither repairs the discrepancy. The extraction is not accidentally scoring encoded sqrt attenuation as linear visibility.
- A model `UE = V * f(N·L)` cannot explain thousands of `V=0, UE≈1` receivers for any finite multiplicative function.

At pose 0, **8,902 covered source pixels have an exactly zero unshadowed denominator and six have a positive denominator <=1e-6**. The missing total is 8,908, including 21 source-only silhouette pixels. At pose 1 the total is 8,922, including 28 source-only pixels. These categories overlap; they must not be added. Most missing receivers are shaded away from the light, although source geometric and native smooth-normal terminators need not coincide. The ratio contains no visibility information where both direct values are zero. Neither a neutral white material nor an exposure change recovers a zero cosine. Filling those pixels with 0 or 1 would invent evidence.

**Measurement mismatch:** this is conditional native surface-shadow attenuation where direct radiance is observable, compared with Renderlab's binary-ray visibility integrated over its own biased receivers. It is not a complete unbiased physical oracle for either engine, or a complete-domain UE quality score.

## VSM: the missing shadows follow the actual receiver policy

The capture scripts never set or record the following defaults. No override was found in the saved project/config files or installed engine config. They must nevertheless be queried on the next run before claiming their historical effective values are independently measured.

| Installed UE control | Default and effect |
|---|---|
| `r.Shadow.Virtual.NormalBias` | .5, scaled by camera distance and horizontal FOV; along the **GBuffer shading normal**, not the source triangle normal |
| `r.Shadow.Virtual.ScreenRayLength` | .015, scaled by view depth and vertical FOV; a four-sample depth trace chooses how far to advance the VSM/SMRT start |
| `r.Shadow.Virtual.SMRT.TexelDitherScaleDirectional` | 2; combines with a slope-dependent bias |
| `r.Shadow.Virtual.SMRT.RayLengthScaleDirectional` | 1.5; finite map-ray length with eight samples |
| `r.Shadow.Virtual.SMRT.ExtrapolateMaxSlopeDirectional` | 5; map-depth extrapolation, not exact triangle intersections |

The normal offset in metres is `max(.0002, NormalBias/1000 * distance_m * tan(horizontal_fov/2))`. The screen-trace maximum is `.015 * view_depth_m * tan(vertical_fov/2)`. **Disabling the light's contact shadows does not disable this VSM screen-space start correction.** It is a separate receiver policy. Relevant source: `VirtualShadowMapArray.cpp:568`, `:575`, `:773`; `VirtualShadowMapProjectionCommon.ush:184`; `VirtualShadowMapProjection.usf:244`, `:809`; `VirtualShadowMapScreenRayTrace.ush`; `VirtualShadowMapProjectionDirectional.ush:112`, `:249`.

CPU intervention on five widely separated worst-error pixels:

| Pixel | Saved reference | Saved VSM | Default normal offset, mm | Source blocked-ray median distance, mm | CPU default normal + screen-start visibility |
|---|---:|---:|---:|---:|---:|
| 1203,669 | .000122 | 1 | 3.066 | 9.47 | 1 |
| 928,325 | .000183 | 1 | 4.945 | 43.13 | 1 |
| 1326,607 | .000244 | 1 | 3.001 | 9.79 | 1 |
| 1269,328 | .000305 | 1 | 4.014 | 25.64 | 1 |
| 987,477 | .000488 | 1 | 3.774 | 8.25 | 1 |

The independent source-ray baseline is 0/0/0/0/.000977. With just the native default normal offset along interpolated source normals it becomes **1/.9141/1/.5938/1**. The source-depth screen-start replay raises the remaining two to 1. Its computed advances are **50.4/82.1/48.3/64.6/63.6 mm**. A simple 5 mm geometric-normal offset also makes all five lit; doubling disk radius only raises their visibility to **.191–.218**, nowhere near 1.

This is a causal intervention on the source geometry using specified native defaults, not a fitted offset. The screen replay uses saved **Renderlab** depth and midpoint dither, not dumped UE depth/noise; it does not emulate map sampling, native normals, compression or slope extrapolation. It establishes why the configured policies erase this class of contact, without allocating percentages of the whole-image error to each policy. VSM's positive mean and predominantly umbra-to-lit flips are consistent with that mechanism. More rays alone do not restore skipped geometry.

## HRT: one-ray variance plus a different biased surface

On pose 0, **7,792/8,392 partials (92.85%)** are within .01 of zero or one, including 3,141 exact zeros. An ideal independent one-ray estimate with this exact reference would have RMS `sqrt(mean(V*(1-V))) = .382302`; observed HRT is **.387781**. This is the scale of a Bernoulli visibility sample, not the .0065 scale of 256-ray randomized QMC. It persists through cut/1/7/31/63: RMS **.387842/.388056/.387446/.388083/.387781**. A retrospective mean of the five sparse frames lowers RMS to .235261, exposing both sampling noise and residual bias; it is not a corrected steady-state score or a performance-matched candidate.

This does **not** establish that the denoiser was disabled or that SceneCapture cannot denoise. Both CVars read back enabled. The installed denoiser deliberately bounds temporal sample count using penumbra radius, with a floor of one. Directional blur radius derives from `tan(half_angle) * closest_hit_distance`; the temporal target shrinks with its projected area. Millimetre/subpixel penumbrae can retain a one-sample target even with valid history. See `ScreenSpaceDenoise.cpp:1627`, `SSDSignalFramework.ush:613`, `SSDTemporalAccumulation.usf:1073–1106`. Native per-pixel frequency/history buffers were not saved, so this source policy plus the observed endpoint/variance signature is strong evidence, not a readback of every pixel's target count.

HRT also uses a different ray origin and culling rule:

- `r.RayTracing.NormalBias` defaults to **.1 cm**. Its effective maximum is at least .01 cm and also scales with camera distance (`distance * 1e-4`). A view-direction correction accompanies a cosine-dependent offset along the **shading** normal.
- Default-lit shadow rays whose sampled direction is behind the shading normal are culled. Renderlab instead flips its geometric normal toward the central light and traces every disk direction.
- `r.RayTracing.Shadows.AvoidSelfIntersectionTraceDistance` defaults to **1 cm**, with a backface-culled near trace followed by two-sided tracing. This is distinct from Renderlab's projected positive `tmin` (about .247 mm in these probes).

Sources: `RayTracingShadows.cpp:28–51`, `:310`, `:482`; `RayTracingCommon.ush:393–420`; `RayTracingOcclusionRGS.usf:435–483`, `:750`.

The CPU UE-origin intervention changes `(1388,529)` from **.9990 to 0**, agreeing with HRT's persistent zero despite `V_R=.998962`; no angle or gain correction can recover that pixel. The source shading normal and camera correction can push an otherwise exposed point into nearby geometry. At the five VSM contacts above, this origin instead predicts visibility **.3457/.0957/.2520/0/.1123**, versus almost zero under Renderlab's rule: sparse native samples can now be lit.

Not every flip is explained by this source-only replay. At `(945,276)`, both source-origin models predict lit, while all five saved HRT frames are black. At `(1577,595)` and `(1254,549)`, the source interpolated normal is behind the central light, the denominator is tiny, and native HRT remains near lit; the simplified source normal/culling replay does not reproduce it. Actual Nanite receiver position, decoded shading normal, winding/epsilon traversal and native mask are needed to separate those effects. These remain explicit per-pixel risks, not evidence for a denominator gain fit.

## Light direction and disk width

The saved UE sun forward vector maps back to normalized source travel with vector error **4.61e-16**. Actual camera registration already passes the <.01-pixel check. A global light-direction mismatch is not supported; +/-1-degree source-ray perturbations also fail to erase the five contacts (visibility stays <=.168).

UE's Source Angle is an angular **diameter**: `DirectionalLightComponent.cpp:395` converts it to `sin(angle/2)` for shader parameters; `RayTracingDirectionalLight.ush:20` samples the corresponding cone. Renderlab's slope disk radius is `tan(.04)`. The exact matching nominal diameter is **4.583662361 degrees**. The recorded 4.584 differs in slope by only **0.00737%**, not a factor of two. The separate shadow source-angle factor was not recorded; the interpretation assumes its installed default of 1 and that property must be queried next time. Uniform solid-angle cone versus uniform slope-disk density is a small remaining distribution difference at this angle.

VSM's directional ray basis in `VirtualShadowMapProjectionDirectional.ush:149–162` is not normalized after the cross products. For this mapped light, that introduces an additional roughly **5.2% reduction in slope radius**, separate from half/full-angle semantics. It cannot explain the observed near-zero-to-one changes. On a deliberately simplified straight-edge disk model, half/same/double radius leave VSM residual RMS **.397602/.360949/.358396** and HRT **.412796/.387781/.405644**. Complex quarry contacts do not obey that edge model; the triangle probes above are the stronger rejection of an angle-only explanation. Do not tune Source Angle to absorb receiver bias.

## Concrete correction and next Unreal run

1. **Fix measurement coverage by exporting the actual native surface-shadow mask**, before material lighting, together with device depth/world position, decoded GBuffer normal and relevant view/light parameters. Capture the same view/pipeline whose timing is reported. UE's installed RDG `DumpGPU` facility can dump pass inputs/outputs and parameter/CVar metadata; use a development build, local output, `r.DumpGPU.Upload=0` and `r.DumpGPU.Explore=0`. Filter the appropriate projection/light pass (`r.DumpGPU.Root`) after confirming its event/resource name. Decode the native attenuation encoding and directional-light channel according to the recorded resource version. A debug readback of the final surface-shadow factor is another concrete implementation. Keep paired RGB as a cross-check where the denominator is positive. This recovers zero-direct receivers without inventing their visibility. It still requires receiver/culling/domain reconciliation before a complete-domain comparison.
2. **Isolate the VSM receiver correction:** rerun the native baseline with explicit readbacks, then change only `r.Shadow.Virtual.ScreenRayLength=0`. Next test `r.Shadow.Virtual.NormalBias=0` (its native floor remains **.2 mm**), separately. Record slope/dither/extrapolation/resolution controls. A fixed 1 mm geometric-normal offset cannot be matched everywhere by the camera-scaled scalar CVar: the equivalent NormalBias would be `1 / (distance_m * tan(horizontal_fov/2))`, approximately **.10–.17** at the five contacts. Setting one fitted scalar is not an exact match. Matching the receiver requires a controlled debug path or a separately labeled reference evaluated at dumped native receivers; keep the registered Renderlab gate unchanged.
3. **Separate HRT sampling from receiver behavior:** request `r.RayTracing.Shadows.SamplesPerPixel=256`, `r.Shadow.Denoiser=0`, Source Angle **4.583662361**, shadow source-angle factor **1**, and explicit readbacks of `r.RayTracing.NormalBias`, `r.RayTracing.Shadows.AvoidSelfIntersectionTraceDistance` and two-sided tracing. This is a diagnostic convergence capture, not the former performance candidate. Compare it with the 1-spp native denoised main view and dump denoiser frequency/history. Test epsilon distance 0 separately. There is no single CVar that makes UE's camera/shading-normal bias equal Renderlab's fixed geometric offset and projected interval.
4. Preserve the production-native configuration as a separate quality/performance candidate. If more native samples or different bias are retained, **remeasure timing in that configuration**. Do not claim the old packaged timings apply to a 256-spp diagnostic, or call the biased Renderlab reference a universal engine-quality oracle.

**Before/after:** recomputing the unchanged extraction gives exactly the original .360949/.359810/.360949 VSM and .387781/.389487/.387802 HRT RMS. There is no valid improved after-score from the saved bytes. Complete-domain scores remain undefined. The missing-mask capture, native receiver/normal readbacks, default-CVar confirmation and main-view denoiser check require an Unreal/GPU rerun; none occurred here.

Verification was CPU-only: array reproduction, EXR/hash audit, twelve source-geometry intervention probes, analytic ray-kernel checks, Python AST/whitespace checks and the focused existing quarry CPU tests. The final array analysis took **7.76 s**, EXR/source audit about **2.35 s**, and source cone preparation **42.40 s**. Implementation and the successive ray investigations were not separately stopwatched; an initially slow allocation-heavy ray loop was replaced with scalar arithmetic. These are research/verification latencies, not renderer frame costs. No build, push or engine setting change was performed.
