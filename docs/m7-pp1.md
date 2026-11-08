# PP1: post processing after reconstruction

2026-11-08 overnight unit. Branch `t3/m7-pp1`, initially based on `984a18b`; main through `ae95d3a` (TR1b/DC1) merged at `ba3496f`, then Islam's requested `f159fcd` (GI, R1 and AT1 defaults) at `3cbd578`, and `f870cb4` (engine API) at `6d2b7ce`.

## Implementation and API

The order is atmosphere LUT/volume preparation → programmable HDR shading with fog and sky → TR1 (when enabled) → motion blur → optional depth of field → bloom pyramid → white balance/tone/grade/LUT/lens finish. Post never writes the reconstructed HDR texture or temporal history. `try_capture_hdr` intentionally retains its pre-post meaning. Exposure and viewer auto-exposure remain unchanged. Raster, traced and visibility-buffer lit frames share the post filters and tone stage; diagnostic views retain their existing output.

AT1 integration was checked against `f159fcd`: raster sky is drawn first into the lit HDR mesh pass; deferred misses use `atmosphere_backdrop`; generated material shading entries apply `atmosphere_surface`. All precede reconstruction. PP1 runs in `encode_hdr_present`, after `encode_reconstruction`, and applies no additional fog. Lib-test constructors deliberately disable atmosphere, so PP1's final GPU checks explicitly select `Atmosphere::default()` and `IndirectSettings::default()`. The bright-sky fixture uses AT1's real procedural sky/environment with a 120 klux sun, exposure -10 and a 1.5 m camera height.

`PostSettings` is exported with `DepthOfField`, `ColourLut` and `Tonemapper`. Defaults: bloom redistributes 3.5% of HDR energy, 180° shutter motion blur with a 16 px maximum half extent, 3.5% edge vignette, Reinhard tone, neutral grading, no DOF/grain/aberration. DOF is requested by supplying camera lens settings in `depth_of_field`. `Renderer::set_post_settings` validates a whole replacement atomically. `PostSettings::disabled()` selects exact legacy presentation. `cut_post_motion()` suppresses motion across an explicit camera cut.

```rust
let mut post = renderer.post_settings();
post.depth_of_field = Some(renderlab::DepthOfField {
    focus_distance_m: 3.0,
    f_number: 2.8,
    ..Default::default()
});
post.tonemapper = renderlab::Tonemapper::Filmic;
renderer.set_post_settings(post)?;
renderer.set_colour_lut(Some(renderlab::ColourLut::identity(16)?))?;
// On a camera teleport:
renderer.cut_post_motion();
```

Bloom is a threshold-free six-level normalized tent downsample/upsample pyramid. Every level combines by convex weights; final output mixes the direct image with the scattered image rather than adding energy. Constant radiance is invariant before tone mapping, apart from half-float rounding. The finite image uses clamped edges. This is a simple optical scattering approximation, not a calibrated lens PSF. The hierarchy follows the approach described in [Jimenez's 2014 post-processing talk](https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/); no filter source was copied.

Motion blur uses pixel velocity, a 16×16 tile maximum and a neighbouring tile maximum, followed by 16 depth-aware taps with foreground/background cone and cylinder weights. It is an original implementation of the reconstruction structure in [McGuire et al., 2012](https://casual-effects.com/research/McGuire2012Blur/index.html). Shutter angle integrates a fraction of the interval between submitted frames; the maximum half extent bounds cost. Motion is derived with the same hit geometry and unjittered projection convention as TR1. TR1 does not export a velocity texture in this baseline; post generates its own auxiliary depth/velocity without reading private history/ages/weights. Raster frames unproject the reversed-Z depth attachment instead. Explicit cuts and the first frame have zero motion.

DOF uses signed thin-lens circles of confusion in output pixels and a 48-tap disc gather with separate near/far accumulation and foreground coverage. It remains off unless requested by the camera. The display stage keeps Reinhard and adds [Narkowicz's ACES-like fit](https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/) (CC0 selected; attribution in `THIRD_PARTY_NOTICES.md`). The latter is not full ACES colour management. RGB white balance, a contrast power curve, luminance-preserving saturation control and a trilinear 3D display-linear Rec.709 LUT follow. LUT red varies fastest, then green and blue; its texture is RGBA8. Vignette is radial, grain is zero-mean changing noise, and optional chromatic aberration offsets red/blue radially by an output-pixel setting.

Resources are owned and reused by size. Target replacements, disable and LUT replacements retire tracked allocations through their last submitted frame. Changing post settings or the LUT does not change the renderer revision or TR1 reset key. The post/tone interval is included in `GpuSample::present_pass`; the complete render span still ends after tone. Settings uploads are independent of scene/program/material semantics. No light-model-specific receiver ABI was introduced. The scratch textures are about 74 MiB at 1080p (including the currently reserved DOF scratch when DOF is off); this excludes driver/pipeline overhead.

TR1b integration: post reads the target-size reconstructed HDR, including `RenderScale::Quality` and `Performance`, and writes the caller's target. Depth and velocity are generated at output size from nearest current internal hits, using the internal projection and output-pixel motion units. Lens radii remain output-pixel settings. The only production TR1-file change is restricted access to its output HDR image; post does not read its history, ages, weights or coverage layers. Disabling post still presents TR1b's upscaled output. The camera's focal length/sensor height controls do not change `Camera::fov_y`; callers should keep those optical settings consistent.

## Verification

Evidence is under `D:/Coding/renderlab-work/m7-pp1/`. `scripts/m7-pp1-run.py` uses the shared schedule/game-hold guards and `scripts/n8-run.py` for every test binary. `scripts/m7-pp1-batch.py` runs the lib suite, then loads the quarry once per path and sweeps all configurations. The screen includes 1920×1080 quarry and a 1280×720 bright-sky fixture with dark near silhouettes, a far surface and a small HDR highlight. Each configuration has a still capture, a moving capture and eight moving-frame timestamp samples. CPU shader validation covers all four modules and the primary composition.

The first retained sweep (`retained-a`) passed 149 lib tests and both full path sweeps with zero Vulkan validation errors. The hardware-filter/grain sweep (`retained-c`) also passed 149 tests and both paths. Every post variant produced the same reconstructed-HDR SHA-256 within each of its four scene/path sets. Inspection found and corrected diagonal grain correlation, then a hard inner edge in near bokeh. The corrected near-field fixture (`near-d`) passed on both paths with clean validation; its inspected images soften both sides of the silhouette.

The TR1b/DC1 integration sweep (`integrated-h`, before the `f159fcd` merge) passed 153 lib tests and all seven GPU cases: native RT/non-RT quarry plus fixture; native raster fixture; Quality and Performance fixture on RT and non-RT. All had zero validation errors. Each of its nine scene/path/scale sets had one HDR hash across all thirteen post variants. Its inspected native and upscaled images are in `D:/Coding/renderlab-work/m7-pp1/integrated-h/`. The final `f159fcd` atmosphere-enabled evidence is recorded below.

Atmosphere integration screen `atmosphere-i` (`3cbd578`, lease `atmosphere-i-24`, 158.79 seconds): 159 lib tests passed. The RT quarry and procedural-sky fixture rendered all thirteen variants, with one reconstructed-HDR hash per scene and matching off/neutral HDR. Their still/moving contact sheets were inspected. The test then failed its strict validation summary: four `VUID-VkShaderModuleCreateInfo-pCode-08737` errors, invalid `OpConstantNull` types 34/43. The first two occur during the first frame with post **disabled**, before PP1 is instantiated; the second pair occurs on the fixture's material-program reload. This is being isolated in the shared shader path; the atmosphere-enabled final run is not accepted as clean, and the batch stopped before non-RT. Logs: `atmosphere-i/native-rt.log` and `atmosphere-i/lib.log`.

The minimized `spirv-j-19` CPU screen and `spirv-val` identify R1's loop-local `ray_query` as the invalid null's type. Naga's WGSL lowering resets that opaque local at each glossy-ray iteration with `OpConstantNull`, which Vulkan prohibits. The PP1 integration fix hoists only its declaration before the loop; every ray still calls `rayQueryInitialize`. `integrated_reflection_spirv_has_no_ray_query_null` checks both composed standard shading variants after override processing. No validation or safety check is disabled. A separate retained fixture transition checks raster → traced at fixed target size, rebuilding post's primary-surface pipeline when it becomes necessary.

`final-k-3` passed all 163 lib tests, including that regression, on the merged API source plus the two integration fixes. The live resource guard then stopped the first non-RT quarry load below its unchanged 2 GiB available-RAM floor while other units compiled. This is a resource-aborted run, not a GPU correctness result. Its preflight had 5.6 GiB free. Smaller atmosphere fixtures and a quarry retry with more headroom follow.

### Retained integration timings

`integrated-h`: RTX 4070 SUPER (NVIDIA 617.14), Vulkan, Ryzen 7 7800X3D; test profile with validation, runtime commit `ba3496f`. Binary SHA-256 `B0BCBC6BC099070EF6D46E10C9785F1B925CFF7F739AE75C1D74BCD64309032F`. Lease `integrated-h-1` held the shared lock for 303.68 seconds. Timestamp medians below include post auxiliaries, enabled filters and tone; each row enables only its named effect above neutral settings, except Default. Eight moving frames per cell. Quarry is 1920×1080; fixture is 1280×720. These are diagnostic costs under shared CPU/build contention, not an optimized or quiet performance claim. Small tone-only differences and negative deltas are clock/sample noise, not savings.

| Configuration | RT quarry ms | Non-RT quarry ms | RT sky ms | Non-RT sky ms | Raster sky ms |
|---|---:|---:|---:|---:|---:|
| Legacy off | 0.013 | 0.012 | 0.056 | 0.035 | 0.007 |
| Neutral | 0.092 | 0.080 | 0.089 | 0.066 | 0.013 |
| Default | 1.427 | 1.050 | 1.261 | 1.259 | 0.433 |
| Bloom | 0.571 | 0.496 | 0.468 | 0.493 | 0.221 |
| Motion blur | 0.578 | 0.523 | 0.992 | 0.993 | 0.382 |
| Far DOF | 2.435 | 1.896 | 2.764 | 2.267 | 1.273 |
| Near DOF | 1.246 | 2.006 | 1.497 | 2.110 | 1.457 |
| White balance/grade | 0.026 | 0.058 | 0.068 | 0.073 | 0.013 |
| Filmic tone | 0.087 | 0.059 | 0.039 | 0.078 | 0.013 |
| 3D LUT | 0.185 | 0.076 | 0.089 | 0.016 | 0.065 |
| Vignette | 0.105 | 0.068 | 0.059 | 0.065 | 0.013 |
| Grain | 0.120 | 0.077 | 0.065 | 0.015 | 0.032 |
| Chromatic aberration | 0.224 | 0.090 | 0.095 | 0.018 | 0.061 |

### Inspected images

Images and JSON remain ignored on D:. Contact sheets compare all thirteen settings. Native quarry inspection retained rock detail, texture and silhouettes with a subtle default finish. The bright-sky silhouettes expose bloom, near/far bokeh and edge colour shifts. Grain was checked in a flat crop: adjacent/diagonal noise correlations below 0.01 after replacing the correlated sine hash. Quality and Performance output dimensions and post radii are correct; nearest internal depth makes reduced-resolution depth edges coarser.

- [RT quarry still contact sheet](D:/Coding/renderlab-work/m7-pp1/integrated-h/native/rt/quarry/inspect-still.png)
- [Non-RT quarry moving contact sheet](D:/Coding/renderlab-work/m7-pp1/integrated-h/native/nonrt/quarry/inspect-moving.png)
- [RT bright-sky moving contact sheet](D:/Coding/renderlab-work/m7-pp1/integrated-h/native/rt/bright-sky/inspect-moving.png)
- [Raster bright-sky still contact sheet](D:/Coding/renderlab-work/m7-pp1/integrated-h/native/raster/bright-sky/inspect-still.png)
- [Quality RT default, moving](D:/Coding/renderlab-work/m7-pp1/integrated-h/quality/rt/bright-sky/default-moving.png)
- [Performance non-RT default, moving](D:/Coding/renderlab-work/m7-pp1/integrated-h/performance/nonrt/bright-sky/default-moving.png)
- [Quality non-RT near bokeh](D:/Coding/renderlab-work/m7-pp1/integrated-h/quality/nonrt/bright-sky/dof-near-still.png)
- [Performance RT all effects, moving](D:/Coding/renderlab-work/m7-pp1/integrated-h/performance/rt/bright-sky/inspect-moving.png)

These local evidence links require this machine; no private quarry asset or generated bulk output is committed.

## Gaps and limits

- Object, skinned and particle velocities are not available in the baseline TR1 interface. Motion blur currently follows camera motion over static geometry; current transforms cannot reconstruct prior object positions. Both paths need a shared exported velocity attachment once those units land.
- One depth/velocity per pixel cannot recover hidden surfaces, transparent-layer velocities or newly exposed background. Motion blur can leave trails or underblur at disocclusions. No ground-truth shutter/reference or Unreal comparison was run tonight.
- DOF is a first-pass gather, without layered hidden-background reconstruction, shaped aperture, temporal stabilization, highlight scatter or physically exact occlusion. Large near bokeh can show sampling and coverage errors. MSAA raster auxiliaries use sample zero.
- White balance is a bounded RGB camera adaptation, not a full chromatic-adaptation transform; the optional filmic fit can oversaturate highlights. The LUT is SDR/display-linear RGBA8, without HDR/log shapers or `.cube` file import.
- Bloom is a normalized pyramidal optical approximation with a fixed six-level shape. Lens dirt, diffraction, anamorphic profiles and lens calibration are absent. Grain is lightweight noise, not a scanned film model.
- Post shaders are compiled library stages, not part of the existing editable prelude reload transaction. Upscaled post depth comes from nearest current internal hits; there is no separate reconstructed depth or multi-layer attachment, so reduced-resolution depth edges can remain coarse.
- No claim of Unreal-equivalent post quality, speed superiority, or film-accurate DOF/motion is established by this overnight unit.
