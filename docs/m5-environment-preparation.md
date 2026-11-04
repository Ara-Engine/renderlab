# M5-C CPU environment preparation

This unit adds an owned prepared distant-environment asset and an offline cooker, based on `87d1003a79a74c62296d2cd985506d13b708c18b`. It does not integrate GPU resources, shaders, the viewer, or a new lighting API. Distant illumination supplies no local bounce, visibility, occlusion or local reflections. The existing constant environment, RLA/RLV formats and normal-map cache recipes are unchanged.

## Public surface

- `renderlab_asset::environment`: validated `CubeLevel`, `PreparedEnvironment`, `Metadata`, configurable `Limits`, and separate RLE1 `encode`/`decode`. No new asset dependencies.
- `renderlab_cook::environment`: caller-owned `LinearEnvironment`, `PreparationSettings`, `CookLimits`, preflight `estimate_cost`, pure `convolve_diffuse_irradiance`, `prefilter_ggx_radiance` and `prepare_environment`.
- `renderlab_cook::environment_hdr`: `decode_rec709_hdr` and bounded `read_rec709_hdr`. The existing pinned image 0.25.10 dependency enables `hdr = []`; no new dependency or lockfile change. Decoder and convolution dependencies stay in the cooker.
- `renderlab_cook::environment_fixture` and its `environment_fixture` example: owned analytic sun/sky and actual Radiance RGBE writing, followed by production HDR read/decode, preparation, deterministic RLE1 roundtrip and cost reporting.

Example library call (HDR decoding and convolution happen only during preparation):

```rust,ignore
use renderlab_asset::environment::{Metadata, RadianceUnits};
use renderlab_cook::{environment::*, environment_hdr::read_rec709_hdr};

let limits = CookLimits::default();
let input = read_rec709_hdr(path, limits)?;
let prepared = prepare_environment(&input, PreparationSettings::default(), Metadata {
    units: RadianceUnits::CandelasPerSquareMetre,
    rotation_y_radians: 0.0,
    source_radiance_scale: 1.0,
    generator_hash: [0; 32], // replaced by the production cooker
}, limits)?;
```

## Coordinates, units and range

Everything is linear Rec.709/D65 RGB32F. Luminance is `.2126 R + .7152 G + .0722 B`. Absolute input radiance uses cd/m²; diffuse output is irradiance in lux. Relative input remains explicitly relative. Diffuse stores **E**, including the cosine integral and pi: a constant L produces pi L. Lambert shading must apply albedo/pi once. Specular stores normalized GGX-prefiltered **radiance**, in the input radiance units, for later combination with the matching DFG.

Input equirectangular texel centres use `phi = 2pi ((x+.5)/width-.5)`, `theta = pi (y+.5)/height`, and direction `(sin(theta) cos(phi), cos(theta), sin(theta) sin(phi))`. +Y is the north pole; +X is u=.5, +Z is u=.75, and -X is the longitude wrap. Rows run top to bottom. Bilinear sampling wraps longitude. At each pole a half-texel cap blends to that latitude row's mean, so the exact pole is longitude-independent. This reconstruction cannot restore source features below the input angular resolution.

Cube layers follow WebGPU +X, -X, +Y, -Y, +Z, -Z, with row-major texels inside each face. For `u,v` in [-1,1], v downward, normalize these vectors:

| Face | Source-space vector |
|---|---|
| +X | (1, -v, -u) |
| -X | (-1, -v, u) |
| +Y | (u, 1, v) |
| -Y | (u, -1, -v) |
| +Z | (u, -v, 1) |
| -Z | (-u, -v, -1) |

`rotation_y_radians` is **unapplied**, right-handed source-to-world yaw around +Y. Positive pi/2 moves source +X to world -Z. Both diffuse and specular world-space lookup vectors need the inverse yaw. CPU preparation leaves pixels unchanged when this metadata changes; its generator identity changes. Runtime rotation remains a root integration gate.

`source_radiance_scale` is a positive finite input calibration multiplier **already baked once** during preparation. Its serialized value is provenance, not another runtime multiplier and not a proposed GPU unpack scale. All calculations accumulate in f64 and convert to f32 at the prepared boundary. Negative, nonfinite or unrepresentable results reject. No maximum-channel scaling, mip energy normalization or 65504 clipping is applied to radiance. Numerical cosine/GGX kernel normalization preserves constant environments; it does not normalize an image's intensity or dynamic range.

HDR files must use -Y +X layout and RGBE, standard scanline RLE or uncompressed pixels. When primaries are absent, the caller of `*_rec709_hdr` asserts Rec.709; explicit primaries must match Rec.709/D65. Nonidentity EXPOSURE/COLORCORR, GAMMA and other explicit primaries are rejected rather than silently ignored or guessed. XYZE/EXR and colour-space conversion are outside this small input adapter. Legacy chained repeat markers are rejected before reaching image 0.25.10's unchecked legacy run multiplier. File/header/scanline framing and exact lengths are validated before image allocation/decoding.

RGBE fixture quantization is explicit: shared exponent `e=1+floor(log2(max(L)))`, channel `floor(L*256/2^e)`. Per-channel error is below `2^e/256`; weak channels in a bright pixel can quantize to zero. That source-format loss is separate from the f32 preparation contract. The fixture checks the actual encoded file against this bound and decodes it through the production adapter.

## Convolution and quality controls

Diffuse uses deterministic equal-area Fibonacci sphere quadrature. Sampled source radiance/directions are reused across output texels; each texel integrates the positive cosine kernel and normalizes its quadrature weight to pi.

Specular uses two deterministic importance proposals with N=V=R. The total sample count is split between Hammersley GGX half-vector samples and source samples, with the extra sample assigned to GGX for odd counts. A 16,384-sample setting uses 8,192 samples from each proposal. The source CDF weights each spherical texel rectangle by `sqrt(residual Rec.709 luminance) * solid angle`. Residual radiance subtracts the per-channel source minimum C. Samples are uniform in solid angle inside the selected rectangle. Sampling and PDF queries use the represented CDF's actual bin mass divided by its area, including rounding. The square root changes the importance distribution only. Source interpolation and physical radiance remain unchanged.

For cosine c, `q_g = D(H)/4` and `K = q_g * max(c,0)`. The estimator is `C + sum((L-C) K / (n_g q_g + n_h q_h)) / Z` over both sample sets. Rejected lower-hemisphere samples remain in the counts. The constant contribution is exact; constant environments have no residual integration. With `a = max(r*r,1e-4)^2`, the exact kernel integral is `Z = (1-a + 2*a*ln(2*a/(1+a))) / (1-a)^2`. A convergent series avoids cancellation near a=1, where Z=1/4. This preserves the conventional normalized split-sum radiance kernel `integral(L K dOmega) / integral(K dOmega)`. It is not an exact BRDF integral for nonconstant input, even with an exact DFG, or a finite-sample accuracy guarantee for arbitrary HDR sources.

The complete power-of-two specular chain uses perceptual roughness `r=i/(mip_count-1)` and `alpha=max(r*r,1e-4)`, matching the existing shading/DFG convention. Mip zero is an unblurred directional lookup (delta convention), not a finite alpha-floor convolution. A base size of one contains only that delta mip. For larger bases, runtime LOD is `r*(mip_count-1)`. Finite source/cube resolution, mip interpolation, source bilinear reconstruction and sample counts all remain approximations. No PDF-based input-mip blur is introduced.

Defaults are diffuse 4² per face, specular 16² with a full chain, and 1024 samples for each convolution. These are deliberately quick iteration settings. The minimum diffuse sample count is four to span a 3D sphere; specular needs at least one. Sizes and sample counts are caller-selected. File bytes, source texels, prepared dimensions/texels/bytes, working buffers and total work each have configurable policies; `unbounded()` removes policy caps while retaining dimensions, checked arithmetic, address-space, finite-value and chain validation. Aggregate output/work admission occurs before convolution allocations. Memory accounting covers owned pixel/sample buffers, excluding allocator/decoder bookkeeping and process RSS.

The numerical fixture uses a dark sky (0.02/0.04/0.1) and a cosine-power-128 sun with peak 2e6 towards (0.36,0.48,0.8). Its known lobe integral is `2pi/129`. Independent f64 reference integration uses global equal-solid-angle z/azimuth midpoint cells, not the production Fibonacci or half-vector samples. The 256x512 and 512x1024 references must converge within 0.2% + .01. On a 1024x512 source and diffuse2/specular4 output, 16384 samples pass the nine selected diffuse/r=.5/r=1 cases (27 channels). The original preparer observed maximum relative error **0.7082%**; the square-root MIS correction observes **0.7608%** under the unchanged 4% + .0005 allowance. Delta-mip texels retain the 3% + .005 gate. These are selected numerical cases, not an exhaustive quality certification.

The original matching 1024-sample preview reached **45.52%** relative error among those channels. The square-root MIS preview reaches **28.20%** and remains unaccepted for this high-contrast scene. An earlier cosine-weighted diffuse attempt missed a grazing tail by 19.97% and was replaced without loosening its reference gate. A 512x256 source's unblurred sun tail also missed the 3% test (3.094% at one texel); increasing source resolution to 1024x512 resolved the retained test. Raising convolution samples alone cannot correct source reconstruction loss. Default/source-resolution misses remain relevant to runtime quality selection.

## Focused MIS correction

The original GGX-only sampler missed a selected r=.05 sun tail in the actual RGBE HDR. A rejected linear-residual-luminance MIS proposal passed 490 actual-HDR cases but failed the inherited f32-source grazing gate. Only one source sample reached that direction's positive hemisphere. Root's numerical preflight selected square-root residual luminance to broaden faint-tail coverage at the same sample count. The linear proposal and a rejected mixture remain historical failures; their passing subsets do not certify them.

Production Rust passes all 490 preserved independently converged actual-HDR cases, or 1,470 channels, with maximum error 0.1481 of the unchanged 4% + .001 allowance. The previously failing f32-source grazing direction `normalize((.5,.5,-1))` at r=.5 passes its original 4% + .0005 gate. Five directions at offsets -1, -.5, 0, .5 and 1 degree towards the sun pass for each source. These use independent angular references, with the actual-HDR references reused and five f32-source references checked at 512x1024 and 1024x2048. Neighborhood maximum allowance fractions are 0.07987 for actual HDR and 0.20045 for f32. This evidence covers the measured directions only.

All five original environment preparation tests pass unchanged. Focused tests also cover the square-root proposal, represented-CDF PDF rounding, extreme finite source arithmetic, linear radiance scaling, constant controls, analytical normalization and byte/work admission. Affected all-target Clippy and formatting pass. The recipe identity is `renderlab-environment-fibonacci-diffuse-ggx-sqrt-hdr-mis-exact-norm-control-v3`. RLE1 layout, units, source scale, roughness mapping and runtime behavior retain their existing contracts.

The source CDF, shared source samples and GGX table are charged before allocation. For diffuse16/specular64 with 16,384 samples and the 1024x512 source, admission is 160,448,512 bounded source, kernel and preprocessing evaluations and 11,618,280 owned working-buffer bytes. Prepared RGB pixels remain 411,624 bytes. These are accounting bounds, not process RSS or runtime GPU costs.

One completed development-profile cook on the Ryzen 7 7800X3D took 37.682 seconds for `prepare_environment`, including identity hashing and both convolutions. Decode, encode and file IO are outside that timer. Contention was uncontrolled; this single observation establishes no speed comparison. The new 411,720-byte RLE1 has SHA256 `ff1271201e628a40624800283112531ba3ee2187226dced44ef668529cf7fcb3`. Its complete payload is finite and nonnegative. Its diffuse and delta-mip payloads are byte-identical to the preserved historical sun64 artifact. The generator identity was independently recomputed from the new recipe and decoded source. No GPU or native verification ran for this cooker correction.

Finite-sample MIS is not a convex average of the source. At tiny sample budgets it can overshoot the source maximum; a sufficiently extreme finite source can therefore fail the existing f32 output-range check. The extreme arithmetic tests exercise the f64 estimator, not that final f32 rejection path. No output clamping was added.

The separate roughness and spatial mip reconstruction miss remains open. This correction establishes no whole-image, Unreal-quality or renderer-throughput result.

## RLE1 interchange layout

Little-endian, 96-byte header followed by tightly packed RGB32F. No per-level headers or alpha; level dimensions/counts are derived and validated before any pixel allocation. This is **f32 CPU interchange**, not a commitment to RGBA16F GPU storage.

| Offset | Field |
|---|---|
| 0 | `RLE1` magic |
| 4 | u32 version = 1 |
| 8 | u32 cube orientation = 1 (table above) |
| 12 | u32 colour convention = 1 (linear Rec.709) |
| 16 | u32 units: 1 cd/m², 2 relative |
| 20 | u32 roughness/kernel convention = 1 (above) |
| 24 / 28 | f32 unapplied yaw / already-baked source scale |
| 32 | 32-byte SHA256 generator identity |
| 64 / 68 / 72 / 76 | u32 diffuse size / specular base size / mip count / reserved zero |
| 80 / 88 | u64 total texels / pixel bytes (12*texels) |
| 96 | diffuse faces, then specular mips fine to coarse, each face-major RGB32F |

Exact file length is 96 + pixel bytes. Unknown versions, conventions, reserved bits, invalid counts, truncation, trailing bytes, nonfinite/negative components and insufficient budgets reject. The generator hash includes the versioned algorithm identifier, decoded source pixels/dimensions, resolutions/sample counts, calibration, units and rotation. It is provenance, not a cryptographic checksum of the serialized payload; arbitrary changes between otherwise valid floats are not detected as corruption.

## CPU evidence and remaining integration

Ignored evidence/artifacts: `D:/Coding/renderlab-work/m5-environment-preparation/`. Tests generate files there using `RENDERLAB_ENVIRONMENT_TEST_OUTPUT`; other hosts can use the temporary-directory fallback. No captures or GPU/window activity occurred.

On Windows / Ryzen 7 7800X3D (16 logical processors), the cached development profile produced one final default warm-cook observation of **216.621 ms**, separately **43.593 ms** decode. This times a completed pure cook including identity hashing, after one discarded warmup; encode/file IO are outside the cook timer. It is a single uncontrolled-contention observation, not a performance comparison or optimized measurement. A nearby CPU load sample was 65%; root retained the concurrent GPU/native/heavy-build slot. The default cook performs 623104 source/kernel evaluations and accounts for 1647720 owned working-buffer bytes, of which 25704 are prepared pixels; the RLE1 file is 25800 bytes. Input maximum 1990656 and mip-zero maximum 1903934.3 demonstrate values above 16f range remain present. Decode additionally admits encoded bytes, two source-sized buffers and one RGBE scanline. The final `--refined` artifact uses a 1024x512 RGBE source and 16384 samples on the same diffuse4/specular16 layout: one warm cook 3963.077 ms, decode 154.261 ms, 9946624 evaluations and 7103592 owned working-buffer bytes. Its complete cube/roughness quality is not certified by the selected smaller-layout tests. See `default/cost.txt`, `refined/cost.txt` and the local handoff for exact commands and source/artifact hashes.

Both affected crates' 56 CPU tests plus one compile-fail doc test pass, including existing cache compatibility, normal preparation and virtual-selection/cooking boundaries. Focused new tests: 4 asset and 5 cooker tests. Affected all-target Clippy passes with warnings denied. No heavy fresh dependency compilation, runtime crate check, GPU test, native viewer launch or Unreal comparison was requested/run by this unit.

Root still owns upload admission, owned-resource lifetime/replacement, shader reload, the environment API and existing constant fallback. Before using these maps, settle an explicit GPU radiance packing/pre-exposure/unpack-scale contract for **both** irradiance and radiance. The supplied sunny values cannot be directly stored in 16f without range loss. A GPU unpack scale must be distinct from the already-baked source calibration. Any underflow, clipping, unfilterable-format requirement or packing loss needs measured bounds; runtime 16f remains a proposal.

Remaining gates: actual cube sampling/face-edge filtering at all roughness levels, inverse yaw and poles, narrow sun retention, roughness/DFG matching (including delta versus alpha floor), diffuse Lambert division, metals/dielectrics/grazing views, packing dynamic range, parameter edits and failure recovery. Verify full rendered fixture/quarry images, moving views, resource budgets, and matched Unreal quality/performance under root's GPU window. No CPU result here establishes M5 completion, visual parity or 2x renderer performance.
