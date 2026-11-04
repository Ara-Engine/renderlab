# Independent roughness environments

RLE2 separates specular roughness from spatial resolution. The production recipe uses 65 quadratic knots `r=(i/64)^2`, 128-square faces at every knot, and linear RGB32F. Positive knots retain the accepted normalized N=V GGX kernel, alpha floor, 8,192 GGX plus 8,192 square-root residual-HDR samples, constant control and exact Z. Knot zero remains the unblurred source. Source reconstruction retains longitude wrap, bilinear interpolation and polar row means. No radiance clipping, source blur, stochastic runtime sampling or sample reduction is introduced.

`PreparedEnvironment::new` still constructs a legacy complete mip chain. `new_independent` constructs equal-sized spatial levels and validates the canonical knot table. Smaller power-of-two faces are permitted for bounded fixtures; the selected production recipe always uses 128. `specular_layout()` distinguishes the two conventions. RLE1 encoding and decoding retain their magic, version, 96-byte header, complete mip dimensions and linear roughness mapping. Changing a magic value cannot convert between formats.

## RLE2 boundary

The little-endian header is 388 bytes. Offsets 8 through 88 retain the corresponding RLE1 field types. Version at 4 is 2, roughness convention at 20 is 2, and level count at 72 is 65. The orientation and colour identifiers remain 1. Reserved offset 76 remains zero.

| Offset | Value |
| --- | --- |
| 96 | u32 independent spatial layout = 1 |
| 100 | u32 normalized N=V GGX with delta at zero = 1 |
| 104 | u32 r^4 interpolation = 1 |
| 108 | u32 f64 host / compensated f32 GPU algorithm v1 = 1 |
| 112 | u32 knot count = 65 |
| 116 | reserved zero |
| 120 | f64 unchanged alpha floor = 1e-4 |
| 128 | 65 exact f32 quadratic knots |
| 388 | diffuse faces followed by specular levels, face-major RGB32F |

Decode checks conventions, metadata, exact knots, dimensions, checked counts, exact length, address-space and caller policy before allocating pixel arrays. It scans the complete external payload for nonfinite and negative values before decoded pixel allocation. Generator identity is provenance, rather than a payload checksum.

## Offline GPU boundary

The cooker feature `gpu-environment` opts into the already installed wgpu 30 and pollster versions. Default cooking remains CPU-only. Renderer and viewer dependencies still exclude the importer, image codecs and cooker. The evidence example additionally uses the existing cached log version to observe Vulkan validation. Production preparation has no Python dependency.

`environment::gpu::GpuPreparer` builds the accepted f64 proposal, represented PDFs, deterministic sample tables, basis and exact Z in Rust, then rounds the explicit buffer ABI to f32. Its kernel is byte-identical to the accepted prototype, SHA256 `96865924434439bafd5b82ea33f4eb1f28071ac4bcc61f60bcb13f43d88571f1`. It uses at most 1,024 output workgroups per drained submission, with bounded tile input/output/readback. Full-field CPU output accumulates owned RGB levels; there is no full-field GPU output buffer. Host query construction uses scalar vector arithmetic instead of per-texel Python calls.

The versioned recipe includes the representation, interpolation, sample budget, host/device precision, kernel hash, source pixels/dimensions, knots, calibration, units and yaw. Errors report failure without starting a CPU fallback. Wait, tile and total wall limits are explicit. Adapter/backend and measured work counts are reported. GPU timestamps are optional; absent timestamps use the drained wall span for the tile limit and are explicitly identified.

`estimate_field_cost` retains the full work count. A 1024x512 source requires 103,105,552,384 declared evaluations. Default cooker policy rejects that operation. Selecting the example's `full` profile admits 104 billion by default, with every provided user policy taking precedence. Other cooker defaults do not change.

```text
cargo run -p renderlab-cook --features gpu-environment --example environment_field --offline --locked -- probe INPUT.hdr EARLY_ORACLE.json EARLY_REPORT.json
cargo run -p renderlab-cook --features gpu-environment --example environment_field --offline --locked -- full INPUT.hdr EARLY_REPORT.json NEW_CACHE.rle2 [POLICY.json]
```

The probe oracle contains keyed `[level,face,y,x]` texels with f64 MIS values and independently converged `reference_rgb` values. Full preparation requires the same binary/source/recipe's passing ten-texel independent report. Outputs are published without replacing an existing path. Use a new cache name; do not rewrite the legacy map. Policy JSON exposes sample/working/source/prepared/dimension caps and finite wait/tile/wall limits. Set `RENDERLAB_REQUIRE_VALIDATION=1` for acceptance runs with explicit clean Khronos layers.

## Runtime

Lighting group 3 bindings 6, 7 and 9 are unfilterable 2D arrays. The independent specular field splits into 42 and 23 cubes, or 252 and 138 layers, within the default 256-layer device limit. Binding 8 remains available to M6. No fifth bind group or optional float filtering feature is required. The 48-byte lighting uniform is unchanged in size. Its map-count lane is positive for RLE1 and negative for RLE2.

The shared WGSL implements seam-aware bilinear `textureLoad`. A support tap outside both face axes averages the three adjacent corner supports, matching the accepted probe. Independent queries bracket the quadratic knots and blend in r^4. Legacy queries retain mip dimensions and linear roughness weights. Diffuse uses the same manual spatial filtering. Both regular and virtual paths share these functions.

Upload still packs one level at a time. Busy admission performs no pixel packing, allocation or queue write. The immutable map owns both banks through pending uploads, selected/retired bind groups and caller handles. Parameter edits retain finite yaw/intensity checks and publish with admitted frames. Constant fallback and transactional shader reload remain available.

The full layout uses 76,695,552 prepared bytes and 102,260,736 GPU bytes. Each fits its existing separate 128 MiB policy. Their accepted combined residency is 170.666016 MiB before packing/overhead. The 384 MiB live GPU policy still includes pending and retired generations. No existing policy is raised.

This remains partial M5. Device precision, complete images, motion and measured costs must be reported from actual runs. It establishes no local bounce GI, M7 work, Unreal parity or combined 2x performance claim.

## Verified unit and measured limits

The retained production host passes ten difficult texels against independent converged references and 3,836 sparse texels against the accepted f64 MIS values. The complete cooked field then passes all 538 distinct reference queries through the actual runtime WGSL on a device without FLOAT32_FILTERABLE enabled. The maximum per-channel fraction of the unchanged 4% + .001 allowance is 0.648956. The 289-direction delta-sun check measures 0.835258% peak-relative error under the unchanged 3% gate, with zero sampled peak shift under the one-degree gate. Probe spacing is about two degrees, which limits the angular precision of that observation. These references measure the N=V prefiltered radiance lookup; they do not reference the split-sum BRDF composition, Fresnel, material response or its N!=V error. These sampled directions do not certify every texel or arbitrary HDR input.

The former r=1 dark leak now reads RGB `(0.01953125, 0.03955078, 0.09960938)` on the device against `(0.01953271, 0.03955192, 0.09961005)`. The corrected bright-tail case at r=.05 reads `(0.10095215, 0.10461426, 0.14013672)` against `(0.10093229, 0.10462839, 0.14019378)`. Readback includes the existing exposed RGBA16F target. The field's diffuse payload is byte-identical to the accepted corrected RLE1 diffuse payload.

Six format tests, seven focused preparation/admission tests, the renderer CPU packing test, affected all-target Clippy and formatting pass. Legacy environment gates retain their original numerical/image tolerances. New device checks exercise both banks, r^4 interpolation, featureless devices and replacement retirement. Actual-field regular/virtual HDR and display images are equal at 1x/4x with corrected normals and direct shadows. Targeted retained direct-light photometry, shadow reload and geometric-normal tests pass. Every successful GPU process records positive Khronos activation and zero validation errors.

The full material fixture, corrected 1,001,334-triangle boulder and 13,034,903-triangle quarry were captured at three nearby poses in 1x/4x. All eighteen resolved frames have zero nonfinite and saturated texels. Full images and motion sheets were inspected. The ten-second background viewer accepts seven yaw edits, rejects a bad shader and invalid settings, restores mapped lighting and exits with zero flights; the foreground window remains unchanged. Content is opaque. Per-sample 4x saturation, transparency and an independent temporal quality reference remain outside these checks.

One complete development-profile cook on RTX 4070 SUPER / NVIDIA 617.14 / Vulkan and Ryzen 7 7800X3D takes 18.005 seconds end to end. It performs all 103,105,552,384 declared evaluations once, including 6,291,456 positive-level outputs. CPU query construction takes 6.201 seconds, shared CPU tables .349 seconds, and diffuse/delta .646 seconds. The drained dispatch/readback wall span is 8.406 seconds; GPU timestamps total 2.980 seconds **within** that span. CPU encoding/submission, completion/map waits and host readback copying account for 1.728, 5.073 and 1.602 seconds respectively. Final RLE2 encoding, cache publication and cache load take .041, 1.435 and .080 seconds. These overlapping categories must not be added twice. Logical preparer GPU buffers total 17,121,632 bytes; admitted CPU working buffers total 106,973,184 bytes, excluding driver/allocator bookkeeping.

This is one contended observation, not a CPU/GPU speed comparison. CPU load before cooking is 82%; recorded GPU utilization is 14% before work. The retained artifact is 76,695,940 bytes with SHA256 `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764` and generator identity `6d85ffb481a3829e4fc31fd292b7328bc287e14296d36cbda8ee87ee06729ea4`. No second full cook ran.

A short paced boulder run records six observations per constant/mapped state at 960x640, 4x and 4096 direct shadows. Constant mesh GPU median is .728 ms, range .709–2.747; independent-field median is 2.077 ms, range 1.376–4.239. All measured frames upload zero environment/geometry bytes. The stable runtime increment is inconclusive because of substantial contention and an unprofiled limiter. This does not establish pipelined throughput or a performance advantage over RLE1.

Three warm-cache process launches reach first observed completion in 1.988–2.321 seconds. One incremental Rust edit builds in 6.311 seconds and reaches first observed completion in 13.377 seconds including restart. Native shader write-to-completion is 1.985 seconds for the first changed shader and .110 seconds for restoration. The first shader compilation and Rust restart remain material development latency. These are completion/present-call proxies, not display scanout. Implementation active time was not instrumented; verification process timings and exact source/binary identities are retained separately in the ignored handoff.

The first 538-wide probe failed because its test quad did not cover the extreme-aspect target. Widening that test geometry made the unchanged field and shader pass. Two retained test setup assumptions were corrected: the normal-fixture path and the old expectation that featureless mapped upload rejects. No quality allowance, mathematical contract, production kernel or prepared field changed during those corrections.

Independent Opus review found no format, runtime-addressing or estimator blocker. It required the numerical scope above and a documented single-session `GpuPreparer` contract. Any preparation error ends the session; callers must discard it and construct a new preparer to retry. Root's combined hardware-shadow tests and actual quarry captures pass in [the integration record](m5-independent-integration.md).
