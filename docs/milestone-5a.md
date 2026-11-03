# M5-A: direct lighting and HDR checkpoint

Implementation is complete and independently accepted as **partial M5**, with root's review corrections verified. The low-roughness numerical check and all six affected existing GPU gates passed on the final source with active validation. This record does not close M5, M6 or the Unreal comparison. The implementation contract is [m5-lighting-plan.md](m5-lighting-plan.md).

## Working behavior

The independent library now renders directional, point and spot lights with GGX/Smith/Schlick specular and declared nonmetallic Lambert diffuse. The whole `Lighting` set is validated atomically. Parameters, exposure and camera edits update admitted GPU data without rebuilding geometry or pipelines. There are no arbitrary light-count policy limits; `max_lights()` reports the device limit. The present fixtures have only a few lights: that is not many-light scaling evidence.

Lit mode adds constant distant-environment illumination, exposure before RGBA16F storage, linear 1x/4x resolve and a per-channel Reinhard presentation pass. A single WGSL file compiles the diagnostic, lit and presentation pipelines together and retains the complete last valid set on errors. Existing diagnostic modes and the default unlit viewer remain. The library has no new runtime dependency; PNG writing is confined to tools/viewer.

The viewer supports `--view lit`, key 5, editable `--lighting` JSON and asynchronous `--capture-hdr` output. Invalid lighting files keep the last valid settings. The quarry launcher selects the recorded preset with `scripts/quarry-demo.ps1 -Lit`. Normal maps, spatial HDR environments, direct shadows, AO, GI and transparency lighting are not implemented by this unit.

## Images and correctness evidence

The actual unchanged twelve-scan quarry contains 13,034,903 source triangles, all-4K textures, twelve full-resident draws and 16x anisotropic filtering. Its asset SHA-256 is `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`. Material inputs remain dielectric with constant roughness .85 and mesh normals; the scan normal/roughness/AO maps are not yet used. Dark photographed creases do not prove computed shadows.

Local artifacts are under `measurements/m5-a/fixture-retained-{1x,4x}` and `quarry-retained-{1x,4x}`: twelve poses each, complete PNGs, raw exposed RGBA16F, PFM and frame/settings identities. Root and the independent reviewer inspected complete fixture/quarry images; the reviewer also inspected both motion sheets and the native viewer capture. Highlights, source winding and mirrored/double-sided panels behave plausibly; 1x aliasing remains visible. This is a short sampled motion observation, not a temporal reconstruction certificate.

All 48 retained raw captures are finite and nonnegative. Independent decoding found maximum exposed RGB 2356 for the fixture and 1.216 for the quarry. Both are below 16376: with nonnegative 4x samples, one sample saturated to 65504 would alone yield at least 16376 after resolve. These particular 4x captures are therefore free of per-sample saturation. The general metadata caveat remains valid for other images, and saturated HDR cannot establish radiance parity.

Independent f64 patch photometry covers directional/point/spot/range/cone/environment response at 1x/4x. A sample-index probe verifies 0/0/0/16 resolves to HDR 4 before mapping to linear UNORM 204. Other focused checks cover invalid normal domains, admitted/coalesced lighting edits under backpressure, stable uploads, atomic lighting rejection and three-pipeline reload, resource retirement and exact lit regular/virtual equivalence. Positive Khronos validation is required in retained GPU/native evidence.

The DFG LUT is 128x128 RG16F, 64 KiB, generated from authored mathematical code. Independent midpoint product quadrature at two resolutions checks the Hammersley-prepared LUT. Convergence tolerance is .0015 and coefficient/interpolation tolerance .005 absolute. Regeneration checks reproducibility; it is not the independent accuracy proof.

## Measurements and limits

Hardware: Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan. Optimized feature-cost observations use the full quarry at 1920x1080, validation enabled, all-4K/16x and twelve identical poses in Base/Lit/Lit/Base order; first two poses are discarded. These are **contended observations** with VALORANT and other work present, not a stable feature-cost estimate or controlled performance gate.

| Mode | Median mesh-pass ms | Median HDR-present-pass ms | Median render/admission CPU ms |
| --- | ---: | ---: | ---: |
| 1x BaseColor | 12.715 | not applicable | 1.324 |
| 1x Lit | 13.133 | .059 | 1.324 |
| 4x BaseColor | 13.354 | not applicable | 1.028 |
| 4x Lit | 19.545 | .040 | 1.247 |

The raw ranges and commands are in `retained-cost-summary.json` and `retained-runs.json`; a local chart is `measurements/m5-a/cost-chart.svg`. These timestamps exclude uploads, acquisition, presentation and scanout. Evidence readback drains are not ordinary renderer behavior. The 4x lit sample is materially more costly; no quality reduction was made to conceal it. No whole-frame or Unreal speedup follows from this table.

Development observations use the lit quarry, a development build and two Cargo jobs. No-op build was .428 s, representative Rust incremental build 3.272 s, and edit-to-observed-log 10.275 s including preparation/launch. The incremental launch reached the present-call proxy in 6146.910 ms. Its overall helper record, `measurements/dev-loop/20261103-222706/results.json`, has **`success:false`**: a separate shader launch was minimized to zero size and produced no frame before its deadline. That sample is rejected. A bounded retry using a private shader copy succeeded: shader compilation 84.854 ms, write-to-present-call 113.374 ms, write-to-completion-observed 136.229 ms. These are proxies, not scanout timings. Exact edited source bytes were restored.

The older unlit anchor compiled a shader in 8.062 ms; the new reload creates three pipelines, and this sample is slower. Earlier M5-A observations varied from 16 to 81 ms. A quiet repeat is needed to attribute the regression, not a claim of improved latency. No fixed campaign was run to manufacture confidence under contention.

The implementation edit window was approximately 28 minutes, with targeted checks interleaved; final verification/captures/latency/handoff took approximately another 15 minutes. These are elapsed windows, not exclusive coding-effort measurements. Exact subprocess durations and failures remain in the saved JSON/logs.

## Review corrections and next work

The independent Opus review found no blocking functional defect in the bounded unit. Root corrected two corrupted comments and added an alpha-floor dielectric/metal highlight check against the f64 reference at the centre pixel and its four neighbours, at both sample counts. It passed the 1% + .002 exposed-HDR tolerance. All six requested existing GPU gates also passed: image/upload/reload/timing, sampler/normal/sidedness, asset/AA/reload/reuse, AA edge reference, material/target replacement overlap and replacement stress. Formatting and affected library/viewer all-target Clippy passed. Raw commands, logs, durations and unchanged source identity throughout the checks are in `measurements/m5-a/root-closeout`.

The retained capture shader hash `017b2e65060045187b9dba5b17e2eb98629b990d5e1db511699429ed1966ac42` differs from root's corrected working-file shader `d80c7acdff6250748a9349f29a4cc9350809e64dfa74782d7b332c8ce8937a73` only in a comment; photometry test additions do not change rendering. Normal viewer/example binaries from the implementation evidence remain valid for that behavior-equivalent shader. Root rebuilt the test executable for the closure rather than recapturing an unchanged scene or repeating latency measurements. Git line-ending normalization is separate from these working-file hashes.

Known follow-through: fix the rare exactly-zero interpolated-normal fallback orientation when normal mapping touches this path; it was not reached by the current normal tests. Avoid repeated record scans/hashing before many-light scaling claims. Use a local-lights-only fixture variant for Unreal matching because the sun dominates the current overview. M5-B adds scan material/normal inputs; M6 integrates the separately prepared shadow views and independent caster coverage. The Unreal tooling must match the actual exported fixture, not its earlier specular-free bootstrap. No Unreal quality parity or simultaneous 2x-performance result has been established.
