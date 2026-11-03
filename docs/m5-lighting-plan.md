# M5-A: direct lighting and HDR

Implementation contract selected on 2026-11-03 at `84a7ac1`. This is the first executable unit of [M5-M6](milestones-5-6.md), not completion of either milestone. It must produce inspected images of the owned lighting fixture and the actual twelve-scan quarry.

## Decision

The two independent designs converged on forward shading into HDR followed by a presentation pass. Root selected Sol, 12/15 versus Astra 11/15; the independent Opus judge also selected Sol, 11/15 versus 10/15. The differences concern interface/development cost and the cost of shared virtual cuts, not a measured rendering advantage. Both rejected deferred material buffers for this first unit because they introduce material reconstruction and extra attachments before any lit image exists.

Simplify the selected design: use a whole validated lighting set rather than handles, and keep one WGSL file and the existing `reload(&str)` API. The diagnostic, lit and presentation pipelines compile and install atomically from that file. There is no generic ABI generator, source assembly framework or public pass scheduler. Take exposure-before-half-float storage, one coherent virtual cut and separate shadow-view uniform ranges from Astra. Shadow implementation follows in its own unit.

Ignored design/review records are under `.refs/m5-m6/design/`. Their statements are proposals; this contract resolves the remaining corrections below.

## Caller and ownership

Expose `Light` variants for directional, point and spot parameters, and a `Lighting` value containing a light vector, constant environment RGB and exposure stops. `Renderer::set_lighting(&Lighting)` validates and stages a replacement; `max_lights()` reports the device-derived limit. Failed validation preserves the previous set. No arbitrary product light-count ceiling or generational ID machinery is needed.

The renderer owns normalized/packed data, dirty detection, GPU capacity and publication. Edits and camera-dependent lighting uniforms upload only after frame admission; busy frames retain edits without staging writes. Upload spans respect wgpu alignment, and replacements with work in flight follow existing owned-resource/ledger rules. Unchanged lights upload zero bytes. Exposure edits do not repack every light. The same admitted submission owns camera/mode/light uploads for regular and virtual rendering.

Add `ViewMode::Lit = 4`; modes 0-3 and default unlit rendering retain their diagnostic meaning and image expectations. Require valid affine normal transforms for every lit collection, including double-sided geometry. Preserve mirrored source winding, face-oriented double-sided shading and finite fallback normals.

## Shading and numeric contract

- Metres; directional/spot `travel` points away from the source. Linear Rec.709 triples carry photometric luminance `Y = .2126 R + .7152 G + .0722 B`: lux for directional, candela for local lights, cd/m² for constant environment.
- Point/spot inverse square uses a documented 1 cm distance regularization. Optional range multiplies by `saturate(1-(d/r)^4)^2`; spot attenuation is squared inner/outer cosine interpolation. Validate finite values, cone order and representable packed parameters.
- GGX, height-correlated Smith visibility, Schlick Fresnel, dielectric F0 .04 and metallic interpolation. Perceptual roughness is squared with a declared alpha floor. Plain nonmetallic Lambert is the initial diffuse model. It is not an assertion that all Unreal energy-conservation configurations match.
- Constant environment uses the same BRDF with a prepared DFG LUT and diffuse irradiance πL. It has no occlusion or local bounce transport. Finite LUT resolution, integration and half-float interpolation are approximations, not an exact numerical evaluation.
- Base colour samples decode sRGB once; factors, vertex colours and MR are linear. The quarry initially uses its existing dielectric, roughness .85 material and mesh normals. Normal/roughness/AO scan-map integration is a later M5 unit, not silently claimed here.
- Exposure is a positive, finite normal f32 multiplier. Evaluate lighting/exposure in f32, store nonnegative exposed RGB in RGBA16F, resolve 4x MSAA in that linear space, then apply per-channel Reinhard in the present pass. Preserve the existing output contract: display-linear shader output, hardware encoding for sRGB attachments, linear storage for UNORM targets.
- Clamp exposed values to 65504 before half-float storage. For the fixed nonnegative Reinhard-to-8-bit path at 1x/4x, this saturation cannot change the quantized displayed channel: one saturated 4x sample alone remains well above the white quantization threshold. This does **not** certify HDR radiance preservation, arbitrary hot-reloaded tone curves, float output or future reconstruction. F32 BRDF evaluation itself must avoid division/cancellation producing NaNs; the half-float clamp cannot repair invalid intermediate arithmetic.
- HDR captures record pre-exposure and nonfinite/saturated stored-texel counts. At 4x, a resolved texel below 65504 can contain a saturated sample; do not call resolved saturation counts a complete per-sample clipping detector. Numeric photometry fixtures stay demonstrably below saturation. Any clipped HDR evidence is excluded from radiance-parity conclusions and labeled accordingly.

Root corrections to the review: retain these saturation qualifications; do not call a regenerated finite-sample LUT an independent integration proof; and do not classify a wrong quarry image as necessarily an asset issue merely because patch tests pass. Content, coordinate conventions, integration and shading remain competing causes until localized.

## Implementation boundaries

Keep camera/instance/material groups 0-2 unchanged for diagnostics. Lighting adds group 3 with a small eye/count/environment/exposure uniform, runtime light storage, DFG texture and sampler. Presentation has its own layout, consuming the resolved HDR texture. One WGSL module shares functions and supports all entry points; reject the complete candidate if any required pipeline is invalid.

HDR targets are allocated on demand and replaced only with extent/sample changes. Charge target/light/LUT resources and retained generations. Preserve mesh-pass timing and add presentation timing with correctly labeled boundaries. Do not call the sum a whole-frame measurement; retain CPU preparation, uploads, submission and acquisition/presentation observations.

Viewer adds `--view lit`, key 5 and an editable lighting JSON file. Bad settings keep the last valid set and diagnose the failure. Existing shader polling, `TINT`/`CLEAR_COLOR` measurement anchors and single shader-path tooling remain. Defaults stay unlit; the quarry launcher gets an explicit lit preset. HDR capture/readback stays window-independent, with inspection PNGs and metadata beside raw data.

No shadow API, tangent format, HDR environment decoder, render graph or light-cache framework enters this first unit. Keep the runtime independent of cooker and window dependencies.

## Focused acceptance

1. CPU boundary/packing tests cover invalid data, atomic rejection, directions, count/device limits, aligned dirty spans and identical updates. Check the prepared DFG LUT against an independently more converged f64 integration at representative and difficult inputs, with explicit finite-resolution tolerance; byte regeneration alone checks reproducibility only.
2. GPU photometry at 1x/4x compares directional/point/spot/environment patches against independent f64 references, including falloff, range/cone boundaries, mirrored/double-sided normals and finite extreme cases. Interior exposed HDR error target is 1% relative + .002 absolute, below saturation. Check low-roughness/grazing formulas for f32 cancellation, not just half-float storage overflow.
3. A sample-index probe writes 0/0/0/16: resolved HDR must be 4 before presentation, giving .8 on a linear UNORM output. This tests resolve order without assuming raster sample coverage.
4. Exercise lighting edits under full-flight backpressure and failed reloads in diagnostic, lit and present entry points. Assert last-good revision/image, recovery, bounded resources, no busy writes, and no repeated mesh/pipeline creation for parameter changes. Check lit regular/virtual equivalence.
5. Run affected checks and existing diagnostic/material/AA/reload/replacement gates once for the retained integrated change. Inspect actual complete fixture/quarry images at 1x and 4x and a short moving camera sequence. Use active Vulkan validation and record source/binary identities.
6. Run a short lit/base-colour paired cost sample with identical content, then measure shader, incremental Rust and startup latency. Report implementation and verification time separately. Expand repetitions only if variance affects the claim. This measures feature cost, not Unreal parity.

## M6 and comparison follow-through

Shadow casters cannot come from camera-visible IDs. Begin with a coherent complete published cut for virtual geometry and full submitted instance coverage for regular geometry. Before M6 acceptance, aggregate camera/light-view demand into that same cut and verify offscreen/thin/grazing caster quality against full source geometry. Selector ownership has returned; there is no longer a concurrent M4 writer. Each shadow view needs a disjoint uniform range. First maps redraw each admitted frame; cache design follows measured need.

The next comparison uses installed UE 5.7.4 CL 51494982, calibrated physical lights/exposure, matched materials/cameras and linear HDR plus full display/motion inspection. Use forward MSAA only if Unreal actually supports the matched configuration; otherwise record the mismatch. Missing GI, reflections, atmosphere and reconstruction are excluded on both sides for this explicitly limited comparison. M5-M6 acceptance still requires the stated quality and simultaneous performance floors, not just correct patches or an attractive quarry capture.
