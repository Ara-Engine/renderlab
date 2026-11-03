# M5-C environment runtime

Prepared RLE1 environments now illuminate opaque regular and virtual draws through owned, filtered float32 cubes. The constant environment remains available on devices without the optional float32 filtering feature. Five focused environment GPU gates and fifteen retained lighting, shadow, normal, material, AA and virtual-image gates pass on the RTX 4070 SUPER with active Vulkan validation. This is partial M5. It does not establish Unreal parity or the combined quality and performance target.

## Runtime contract

`Renderer::try_upload_environment(&asset::environment::PreparedEnvironment)` returns `Result<Option<EnvironmentMap>, Error>`. `None` means the shared submission admission is busy. Busy performs no pixel packing, resource allocation or queue write. Accepted uploads submit immediately. The renderer retains upload resources through completion even if the caller drops the returned handle.

`EnvironmentMap` owns diffuse and specular `Rgba32Float` cube textures, their views and logical allocation records. It is immutable and renderer-affine. `set_environment(Some(&map), EnvironmentParameters)` selects it and retains ownership. Foreign handles and invalid parameters reject atomically. `set_environment(None, parameters)` restores `Lighting::environment`. Parameter changes publish with the next admitted lit frame. They allocate no textures or bind groups and upload no environment pixels.

`EnvironmentParameters` contains additional source-to-world `rotation_y_radians` and nonnegative linear `intensity`. The shader applies the inverse of metadata yaw plus caller yaw to both lookup directions. Prepared calibration is already baked. `source_radiance_scale` never becomes a GPU unpack multiplier.

The two cube textures use lighting group 3 bindings 6 and 7. The existing filtering sampler at binding 3 also supplies trilinear mip filtering. Shadow bindings 4 and 5 and normal bindings in group 2 remain unchanged. The lighting uniform grows from 32 to 48 bytes. Its last lane stores cosine, sine, intensity and specular level count. All six pipelines still install as one transaction. An invalid environment binding or shader rejects the whole candidate and preserves the prior image.

Diffuse pixels store irradiance E, including pi. Lambert shading divides by pi exactly once. Specular pixels store normalized GGX distant radiance. Perceptual roughness maps to `roughness * (level_count - 1)`, including delta mip zero. The existing DFG table supplies Fresnel and visibility response. Split-sum filtering remains an approximation for nonconstant environments.

The exposed RGBA16F target and presentation curve retain their existing contract. Environment radiance, material response, intensity and exposure multiply in mantissa/exponent form before final storage saturation. This preserves the tested `1e-20` cube values and finite results whose unexposed intermediate products exceed f32 range. Explicit weighted F0 terms replace `mix(.04, base, metallic)`, which lost a `1e-20` metallic base through cancellation on the tested device. Ordinary constant-environment photometry remains within its unchanged image tolerances.

The background retains the constant environment. There is no environment background renderer, local reflection, occlusion, bounce GI or sky atmosphere in this unit. Distant environment illumination is unoccluded.

## Device capability and budgets

Mapped uploads require `wgpu::Features::FLOAT32_FILTERABLE` on the supplied device. The installed wgpu 30 API exposes the feature for `Rgba32Float` filtering. The RTX 4070 SUPER with NVIDIA 617.14 reports it and passes actual filtered cube probes. The viewer and new evidence example request the feature only when the adapter advertises it. Existing constant, direct and shadow callers need no new feature request. A filterable zero RGBA16F cube keeps their binding layout valid. Unsupported mapped uploads reject before writes instead of converting to half. The shared layout now requires seven sampled textures per shader stage and six texture array layers, within the existing default device limits.

`EnvironmentLimits` defaults to dimension 2048, 128 MiB per map and 384 MiB for all owned environment bytes. `set_environment_limits` changes these policies. `unbounded()` removes policy ceilings while preserving checked addressing and device dimensions. `environment_limits()` reports the effective dimension. The live byte budget includes caller handles, submitted generations and the 48-byte dummy cube. Retained bind groups hold their map references until their last submitted use completes.

`Stats::ownership.environment_bytes` records logical cube bytes. `UploadBytes::environment` records RGBA upload bytes, including the initial dummy. `texture_writes` includes cube mip writes. These counters exclude driver allocation granularity, queue-internal staging and allocator bookkeeping. Each upload packs one level at a time. The accepted quality artifact has 411,624 prepared RGB bytes, 548,832 uploaded GPU bytes and a maximum 393,216-byte host packing vector. Its settled logical environment total is 548,880 bytes including the dummy. Pixel hashing happens once at upload. Frames hash only the cached map identity and small parameter record.

## Viewer and evidence tools

The viewer accepts `--environment FILE.rle` and `--environment-settings FILE.json`. It loads and decodes the bounded RLE1 file once. Settings poll with the existing shader/settings loop. Invalid settings retain the previous valid selection. The JSON fields are `enabled`, `rotation_y_radians` and `intensity`. Disabling a map retains the uploaded owner for later activation. Exposure remains in the existing `--lighting` JSON.

```json
{"enabled": true, "rotation_y_radians": 0.7, "intensity": 1.0}
```

The shared `--limits` file accepts an `environment` section with `max_dimension`, `max_texels`, `max_payload_bytes`, `max_map_bytes` and `max_owned_bytes`. Individual fields or the section accept `"unlimited"`. Decode remains in the asset crate. HDR parsing and convolution remain in the cooker. The renderer and viewer gain no image-codec dependency.

`examples/m5-environment.rs` captures the immutable full M5 material fixture when no asset is supplied. `--asset`, `--cameras` and `--lighting` select prepared content. `--compare-constant` alternates constant/map/map/constant at each pose after a discarded warmup of each state. The example records decode, upload, CPU admission, GPU mesh/presentation/render spans, input identities and complete HDR/display frames. Switching a map can recreate a lighting binding and republish light records. Consecutive unchanged mapped frames upload neither environment pixels nor lighting records.

The `environment_quality` cooker example reads an existing owned HDR and writes RLE1 at explicit diffuse/specular dimensions with 16,384 samples. Its work cap is 250 million evaluations. It changes no convolution algorithm or normal policy.

## Focused verification

The CPU packing test preserves exact f32 bits and checks dimensions, byte admission and parameter validation. All 50 renderer CPU tests pass. Affected all-target Clippy passes with warnings denied. The inherited CPU asset/cooker preparation evidence remains separately recorded in [environment preparation](m5-environment-preparation.md) and [normal maps](m5-normal-maps.md).

Five ignored `gpu_environment_` gates cover the following behavior.

- Constant cubes match existing constant lighting over complete frames at 1x and 4x. Dielectric, metallic, rough and grazing responses match independent f64 BRDF integration. Extreme range and dim input cases remain finite with the expected response.
- Axis and pole directions, metadata plus caller yaw, all twelve cube edges, adjacent mip boundaries and trilinear interpolation match an analytic directional field. Edge pairs remain continuous within the fixed 0.002 exposed allowance.
- Actual refined HDR samples retain the high-contrast sun. A 289-direction neighborhood uses fixed limits of 3% of peak radiance and 1 degree of peak-direction error. Selected diffuse and GGX values at integer-mip texel centres pass independent global angular quadrature with an independently checked convergence bound. Between-texel and between-mip reconstruction of the sunny map is gated only at mip zero.
- Busy uploads and frame writes allocate nothing. Foreign handles reject before writes. Replacements overlap incomplete prior frames and return to the same settled ledger. Fallback and restore recover the entire prior HDR image. Bad shader and environment-binding reloads retain the prior bundle and corrected recovery restores it.
- Corrected normal-mapped regular and exact-resident virtual fixtures render identical complete HDR/display images with direct shadows and a constant-valued mapped cube (0.2, 0.3, 0.4) at 0.8 rad yaw, at both AA counts. Both paths use the same lighting bindings and shader; this gate does not test direction-varying map parity.

The original refined diffuse4/specular16 artifact fails the fixed sun interpolation gate at **18.3263%** peak-radiance error. More convolution samples alone do not repair its spatial resolution. A diffuse16/specular64 recook passes at **1.8399%**, with zero sampled peak-direction shift. The direction probes are about two degrees apart, so that peak-location observation resolves only about plus or minus one degree. The same 16,384-sample preparation uses 159,391,744 source/kernel evaluations and 7,489,512 owned working-buffer bytes. One development-profile cook takes 45.658 seconds on this contended host. These sampled checks do not certify every map direction or eliminate split-sum approximation error.

The complete material fixture, corrected 1,001,334-triangle normal-mapped boulder, and full twelve-scan quarry were inspected in three nearby camera poses at 1x and 4x. The quarry retains all 13,034,903 source triangles and twelve 4K texture chains. Full silhouettes, highlights and material detail remain visible. All 25 retained headless pose captures report zero nonfinite and zero saturated resolved texels. A 4x resolve cannot detect every saturated individual sample. The content is opaque. Transparency and an independent temporal reference are outside these images' evidence.

Native checks use only the owned viewer window. The steady mapped boulder and the mapped image after a rejected/corrected shader, yaw/intensity/exposure edits and fallback restoration were captured and inspected. The final six-second recovery run presents 1,161 frames, exits with zero flights and clean active validation, and leaves the foreground HWND unchanged. A first helper waited for the wrong log token. A subsequent early screenshot showed the fallback before the later settled capture. Both observations remain in the ignored evidence. Native source-edit latency, Rust edit-to-running latency and repeated startup latency are still root-owned measurements. No source-mutating development-loop helper ran.

## Cost observation and remaining limits

A bounded release/Vulkan sample alternates constant/map/map/constant over three boulder poses at 960x640, 4x MSAA, 16x anisotropy and 4096 direct shadows. Each state has six paced observations with one draw and 1,001,334 triangles. Every measured frame uploads zero geometry and environment bytes.

| State | Mesh GPU median and range, ms | GPU render span median and range, ms | CPU admission median and range, ms |
| --- | --- | --- | --- |
| Constant | 0.7045 [0.7012, 0.7096] | 0.9155 [0.9124, 0.9236] | 0.5404 [0.4502, 0.6132] |
| Mapped | 0.9805 [0.9728, 0.9839] | 1.1950 [1.1878, 1.2031] | 1.0977 [0.8420, 1.5131] |

The feature-cost conclusion is **inconclusive** as a stable delta. Background contention is uncontrolled and the hardware limiter was not profiled. These paced measurements include readback drains between observations and do not measure pipelined throughput. CPU admission includes binding changes when a state switches. GPU render spans include direct shadow work. Decode and texture upload are outside ordinary frame timing. In this run decode is 0.477 ms and admitted upload is 0.9342 ms, including a 0.0867 ms poll and 0.8452 ms upload/preparation scope. They are single warm observations.

Environment shadows, local occlusion and local reflections remain absent. Normal variance compensation, an independent gloss-motion reference, existing M6 quarry shadow failures and matched Unreal acceptance remain open. The v1 linear roughness-to-mip chain reaches 4x4, 2x2 and 1x1 faces at roughness at least 0.75. An independent CPU emulation of seamless trilinear lookup on sun64 reads the sun 5-15% low at roughness 0.67-0.9 and 36% low at roughness 1. At roughness 1 it leaks about 8% of the peak into a direction 119 degrees from the sun. This is an emulation, not a device measurement. Diffuse irradiance stays within 0.6% of peak in that probe. This is an explicit open M5 quality miss for rough reflections and Unreal acceptance. The selected numerical and image gates establish this implementation unit, not all-direction quality, arbitrary HDR quality, or engine-wide superiority.

Independent Opus review accepts this bounded partial-M5 checkpoint after the evidence wording above was corrected. Root integrated the exact tested source, verified preserved identities and inspected full images. [The integration record](m5-environment-integration.md) identifies the retained local roughness probe and review evidence.

Exact patch, source, binary, input, command and inspected-image identities are in the local `.refs/m5-environment-runtime/handoff.md` and `identities.json`. Captures and measurements remain ignored under `D:/Coding/renderlab-work/m5-environment-runtime`. Implementation and verification time are recorded separately there. No generated private scan content enters the public patch.
