# Optional ara-gpu backend

Enable `backend-ara` to build the candidate backend. The default renderer remains
wgpu; its pass files are not edited. Ara is pinned to its GPU-gap branch until PR #5
merges. This is incremental migration work, not whole-renderer parity.

The first slice uses the existing `shaders/post.wgsl` and `post_tone.wgsl` directly.
`src/backend_ara/shader` centralizes naga heap adaptation and checked SPIR-V repairs
proven in Ara's replay. Reusable typed textures, settings records, post encoders and a persistent whole-image
state tracker are present. CPU-reference replays and a stored-HDR Metal example
are described below. Vulkan runs remain pending.

No Slang or Metal Shader Converter is involved. Native Metal artifacts will use
Apple's optional Metal toolchain and Ara's existing hashed pipeline loader.

## Build boundary

Use Rust 1.99.1 for the Ara feature: `cargo +1.99.1 check -p renderlab --features backend-ara`.
The default feature tree excludes Ara. Cargo still resolves optional dependencies
into the lockfile. The cooker's exact serde_json pin was relaxed to its compatible
range because Ara requires a newer patch. The feature also enables
`wgpu/vulkan-portability`: the existing crate unconditionally compiles a Vulkan HAL
development adapter, which otherwise does not compile on macOS. This does not create
a wgpu device; the candidate owns only Ara's Metal device.

For native MSL artifacts on Mac, install Apple's optional toolchain with
`xcodebuild -downloadComponent MetalToolchain`. This installed without sign-in here.
ShaderCache invokes metal/metallib on cache misses and passes hashed native artifacts
to Ara's unchanged cooked loader. The caller supplies the cache path; no user-profile
writes, Slang or MSC. A failed compile does not replace an existing PostPasses object.

## Proven Renderlab pass families: 6 (plus display transfer)

| Pass | Metal CPU-reference replay | Vulkan |
|---|---|---|
| Bloom downsample | 17×11 → 9×6 → 5×3, max error 0.001953125 | Pending PC run |
| Bloom upsample | Coarsest self-tent and reverse level, max error 0.001953125 | Pending PC run |
| Velocity tile maximum | Partial edge tiles, exact output; eight submitted frames | Pending PC run |
| Motion blur | Foreground/background depth and neighbour velocity, max error 0.00390625; eight frames | Pending PC run |
| Depth of field | 48-tap near/far gather, max error 0.00390625; eight frames | Pending PC run |
| Display transfer (new) | GPU sRGB encoding plus UNORM quantization, max error 0.002237 | Pending PC run |
| Post tone | Filmic, white balance, contrast, saturation, vignette and 3D LUT, max error 0.000534 | Pending PC run |

Run `MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 cargo +1.99.1 test -p renderlab --features backend-ara --test ara_backend -- --nocapture --test-threads=1`.
This executes actual cached native metallibs through Ara's normal loader and
compares all six bloom/tone/display outputs with independent scalar references (tolerance 0.003;
half-precision bloom stores). The same public encoders and test run on Vulkan.

The shader adapters are shared in one module, not repeated in each pass:
- Uniform Settings becomes read-only storage, preserving its five vec4 offsets.
- MSL selected-entry shims read heap texture/sampler/buffer indices from eight roots
  at Metal buffer 2; typed descriptor aliases preserve Ara's 24-byte entry stride.
- SPIR-V uses runtime descriptor arrays, 32-byte push roots and a typed 3D sampled
  alias at binding 5. Naga 30 requires RuntimeDescriptorArray capability insertion
  and exact type-checked image argument pointer repair. These are centralized and
  their output is emitted for `spirv-val --target-env vulkan1.3`.
- Content keys include source, both adapter implementations, native compiler version,
  render state and the heap ABI. Artifact directories publish only after successful
  compilation; a cache hit loads the same hashes through Ara.

The graph generates dependencies from declared direct resource uses. It preserves
states across submissions, rejects undefined reads and write aliases, and orders
same-state storage writes. It currently schedules only whole images/buffers on
Graphics. Range splitting, async queues and transient aliasing remain later work.

## First visible Metal output

`cargo +1.99.1 run -p renderlab --features backend-ara-window --example ara_post`
loads `tests/fixtures/ara-post.pfm`, runs the full six-level bloom down/up chain,
tone and GPU display transfer, writes `target/ara-backend/first-metal.png`, and
presents it in a native SDL Metal window. `--input <RGB PFM>`, `--output <PNG>`,
`--headless` and `--seconds <0..300>` are supported. The PFM is a redistributable,
self-authored synthetic linear HDR frame, not a capture of a migrated scene renderer.
No wgpu device, CPU tone mapping, Slang or MSC runs in this example. The PNG is a
raw readback of the GPU's already display-encoded RGBA8 output. The example retains
resources and pipelines for the window lifetime and retires them before exit.
The output PNG was visually inspected on the Mac. A separate desktop screenshot
was unavailable: Computer Use returned `sky requires node_repl; configure
NODE_REPL_TRUSTED_SERVICES`. Native presentation is verified by its completed-frame
counter; this does not substitute for a desktop screenshot.

New-filter tolerance is 0.004 (two half-float ULPs around the brightest fixture
values); tile maximum is exact. Bloom/tone/display retain 0.003. No speed claim is
made from these small replay fixtures. Strict Rust 1.99 clippy is blocked by existing
warnings in the wgpu/asset files (including the new chunks_exact_to_as_chunks lint);
these files are PC-owned and were not changed to silence them.

## Clustered-light culling: seventh Renderlab family

The original LC1 source and constants/packing/header helpers are reused unchanged.
Mixed bounded/unbounded point/spot/directional lights, overflow (133 lights, 128
stored) and empty input each match every count and stored u16 index in all 13,824
clusters on Metal. Vulkan execution is pending; the emitted stage is validated on
Mac along with the post stages.

Three additional shared interface steps were required:
- MSL runtime-array bounds checks get naga's size record from the heap descriptor's
  byte-length metadata. This thread-local record replaces only the generated size
  parameter's address space; bounds checks stay enabled.
- MSL workgroup reference arguments become scratch declarations in the native
  wrapper. Naga's initialization and barriers remain in the generated algorithm.
- SPIR-V workgroup-only arrays lose naga's invalid ArrayStride decoration. A type
  shared with a storage/uniform ABI is rejected rather than changing that ABI.
The LC frame has a typed descriptor alias at buffer binding 1; it shares the raw
buffer heap with the light array. These are checked output-interface adaptations.

Compute group sizes now come from each WGSL entry, for reflection and dispatch.
Cache keys include adapter/cache source and Cargo.lock, so frontend/compiler/flag
changes invalidate artifacts too. The post checkpoint received an independent
Opus 5.5 read-only review. No blocking Metal algorithm bug was found; cache-key and
threadgroup-size findings were fixed before the next leaf checkpoint. Further
frontend generalization and concurrent frame submission are still migration work.

Cargo feature unification: enabling Ara also enables serde_json `preserve_order`
and `float_roundtrip` through Ara's dependencies. Ara-feature verification is scoped
to `--test ara_backend`; the cooker's feature-on JSON/report golden suite has not
been certified. Default builds exclude these Ara feature edges, although Cargo.lock
updates serde_json from 1.0.149 to 1.0.154. Two ash versions are linked only in the
opt-in backend build (existing wgpu HAL and Ara); do not exchange their native types.

This migration was explicitly authorized by Islam on 2026-11-10. Earlier statements
that Ara integration is deferred describe the previous phase, not this branch's scope.

## Atmosphere LUTs: four more entry points

The selected source is the unchanged common definitions and LUT section of
`shaders/atmosphere.wgsl`, ending before the aerial/fog kernels. No visibility stub
or history implementation is substituted. Native Metal replays cover the complete
Earth-default transmittance (256×64 plus reserved ambient row), multiscatter (32×32),
sky-view (192×108) and ambient mean, with an independent scalar integration chain:

| LUT entry | Maximum absolute component error |
|---|---:|
| `at_transmittance_lut` | 0.00048828125 |
| `at_multiscatter_lut` | 0.000061035156 |
| `at_skyview_lut` | 0.00012207031 |
| `at_ambient_mean` | 0.000061035156 |

The native compiler now uses `-fno-fast-math -ffp-contract=off`. The initial default
Metal optimization changed a sphere-root branch on the exact top LUT boundary,
producing zero instead of one transmittance. Strict settings removed that mismatch
without changing the WGSL or widening reference tolerances. Vulkan's handling of
this boundary is explicitly pending along with its native run.

The shared frontend selects/compacts only the requested entry (plus fullscreen
vertex where needed) before native output. This removes unused entry interfaces and
avoids recompiling unrelated kernels into each selected native library. Workgroup
sizes are still derived from the selected original WGSL entry. MSL parameters and
SPIR-V descriptor arrays follow one atmosphere-root convention: parameter buffer,
transmittance, multiscatter, sky-view, output, sampler. Shadowed fog/history and
procedural environment kernels remain later ports, not substitutes in this slice.

The sky integrator also exposed naga's explicit Offset decorations on function-local
`AtSegment`. The internal-layout repair now handles both workgroup arrays and
function/private structs, removing only layout decorations on types exclusive to
internal memory; any type shared with an external buffer ABI is rejected. All 14
exported stages pass spirv-val for Vulkan 1.3. Integration test artifacts now follow
Cargo's `CARGO_TARGET_TMPDIR`, including the configured bulk-output drive on PC.
