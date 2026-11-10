# Optional ara-gpu backend

Enable `backend-ara` to build the candidate backend. The default renderer remains
wgpu; its pass files are not edited. Ara is pinned to its GPU-gap branch until PR #5
merges. This is incremental migration work, not whole-renderer parity.

The first slice uses the existing `shaders/post.wgsl` and `post_tone.wgsl` directly.
`src/backend_ara/shader` centralizes naga heap adaptation and checked SPIR-V repairs
proven in Ara's replay. Reusable typed textures, settings records, post encoders and a persistent whole-image
state tracker are present. CPU-reference replays and a stored-HDR Metal example
are the next checkpoint. No passes are claimed Metal-proven in this first commit. Vulkan runs remain pending.

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
