# PP1: post processing after reconstruction

2026-11-08 overnight unit. Branch `t3/m7-pp1`, initially based on `984a18b`, with main through `ae95d3a` (TR1b and DC1) merged at `ba3496f`.

## Implementation and API

The order is programmable HDR shading → TR1 (when enabled) → motion blur → optional depth of field → bloom pyramid → white balance/tone/grade/LUT/lens finish. Post never writes the reconstructed HDR texture or temporal history. `try_capture_hdr` intentionally retains its pre-post meaning. Exposure and viewer auto-exposure remain unchanged. Raster, traced and visibility-buffer lit frames share the post filters and tone stage; diagnostic views retain their existing output.

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

The first retained sweep (`retained-a`) passed 149 lib tests and both full path sweeps with zero Vulkan validation errors. The hardware-filter/grain sweep (`retained-c`) also passed 149 tests and both paths. Every post variant produced the same reconstructed-HDR SHA-256 within each of its four scene/path sets. Inspection found and corrected diagonal grain correlation, then a hard inner edge in near bokeh. The corrected near-field fixture (`near-d`) passed on both paths with clean validation; its inspected images soften both sides of the silhouette. Final main-integrated evidence follows below.

## Gaps and limits

- Object, skinned and particle velocities are not available in the baseline TR1 interface. Motion blur currently follows camera motion over static geometry; current transforms cannot reconstruct prior object positions. Both paths need a shared exported velocity attachment once those units land.
- One depth/velocity per pixel cannot recover hidden surfaces, transparent-layer velocities or newly exposed background. Motion blur can leave trails or underblur at disocclusions. No ground-truth shutter/reference or Unreal comparison was run tonight.
- DOF is a first-pass gather, without layered hidden-background reconstruction, shaped aperture, temporal stabilization, highlight scatter or physically exact occlusion. Large near bokeh can show sampling and coverage errors. MSAA raster auxiliaries use sample zero.
- White balance is a bounded RGB camera adaptation, not a full chromatic-adaptation transform; the optional filmic fit can oversaturate highlights. The LUT is SDR/display-linear RGBA8, without HDR/log shapers or `.cube` file import.
- Bloom is a normalized pyramidal optical approximation with a fixed six-level shape. Lens dirt, diffraction, anamorphic profiles and lens calibration are absent. Grain is lightweight noise, not a scanned film model.
- Post shaders are compiled library stages, not part of the existing editable prelude reload transaction. Upscaled post depth comes from nearest current internal hits; there is no separate reconstructed depth or multi-layer attachment, so reduced-resolution depth edges can remain coarse.
- No claim of Unreal-equivalent post quality, speed superiority, or film-accurate DOF/motion is established by this overnight unit.
