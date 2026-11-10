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

## Proven passes: 3

| Pass | Metal CPU-reference replay | Vulkan |
|---|---|---|
| Bloom downsample | 17×11 → 9×6 → 5×3, max error 0.001953125 | Pending PC run |
| Bloom upsample | Coarsest self-tent and reverse level, max error 0.001953125 | Pending PC run |
| Post tone | Filmic, white balance, contrast, saturation, vignette and 3D LUT, max error 0.000534 | Pending PC run |

Run `MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 cargo +1.99.1 test -p renderlab --features backend-ara --test ara_backend -- --nocapture --test-threads=1`.
This executes actual cached native metallibs through Ara's normal loader and
compares all five outputs with independent scalar references (tolerance 0.003;
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
