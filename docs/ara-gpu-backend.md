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

## Procedural environment: two more entry points

`at_environment_specular` and `at_environment_diffuse` now use the original WGSL
hemisphere integrations and cube-face convention. The specular output is one
128×128, six-layer, five-mip RGBA32Float image; narrow array views select each
128/64/32/16/8 output level. All views share their parent resource identity and
retirement. Diffuse uses a six-layer cube. The shared adapter adds the corresponding
writable array-texture entry type; no per-pass native bindings were added.

A fixed nonuniform 33×19 half-float HDR sky checks all 130,944 specular texels and
1,536 diffuse texels against independent GGX/cosine CPU integration. Max errors by
specular mip: 0.00538504, 0.000394434, 0.000199467, 0.000162095, 0.000244439;
diffuse 0.000145912. The unsmoothed base exposes half-texture interpolation precision:
at its worst texel, ideal CPU is 1.9272321, GPU 1.9326172, and an 8-bit filter-fraction
CPU probe is 1.9330912. The base tolerance is therefore 0.01; integrated outputs
retain 0.004. This is a sampling-precision allowance, not a shader or quality setting
change. Vulkan execution, including writable array-view descriptors, remains pending.

There are now 13 Renderlab pass entry points plus the new display transfer. Shadowed
fog and temporal history are still reserved for the later shadow/temporal phase.

## Skinning: two more entry points, using an indirect record

`skin_vertices` and `skin_clusters` now run unchanged through the same frontend.
Ten resource bindings exceed eight direct roots, so one padded storage record holds
ten heap indices. Root words 0/1 carry the record buffer and byte offset; the test
uses a nonzero offset. The typed semantic table is shared by the host packet and
both shader adapters. Runtime-array sizes still come from the selected buffer's
metadata on Metal, preserving bounds checks through the indirection. Vulkan uses
typed aliases of the same storage heap, scoped by generated variable identity.

The native replay uses an independently written scalar oracle for nine vertices,
six affine joints, two poses, reflected/nonuniform scales, four-joint weights,
rank-two and zero-linear blends, tangent handedness, and padded/out-of-order cluster
mappings. Maximum vertex/tangent differences were 1.20e-7 / 5.97e-8 by pose. UV/color,
previous pose, mapped cluster position bits and untouched sentinels are exact.
Both poses and their GPU snapshots are submitted before the first host wait;
palette/flag changes use ordered UpdateBuffer commands, not host overwrites.
BLAS refit/visibility integration remains later work; optional AS usage requires
an RT-enabled device. The leaf uses coherent records initially; geometry page
consolidation/private uploads remain a scene-upload integration task.

Skinning exposed field names equal to global names (`Params.vertices`, for example).
The shared SPIR-V rewrite now takes its resource names from naga's parsed globals,
leaves members and field declarations intact, and rejects local shadows rather than
rewriting them silently. Descriptor alias repairs are scoped to generated variable
IDs, not arbitrary matching binding numbers. All 18 exported stages validate.
There are now 15 Renderlab entries plus display transfer; every Vulkan native run
remains pending.

## Raster post surface: sample-one and sample-zero MSAA variants

The unchanged `surface_raster` entry now reconstructs linear depth and camera
velocity from depth attachments. The shared adapters support depth2D/depth2DMS
heap views; the original 4x variant reads sample zero. A 17×11 depth fixture
contains three depth rectangles and sky, rendered on the GPU before compute,
without a host wait between those submissions. Moving-camera and camera-cut
references pass for both variants: max error 1.20e-7, cut velocities exactly zero.
This is camera motion; object motion still belongs to the later primary-surface
port. There are 16 original Renderlab entry points plus display transfer, and
20 exported SPIR-V stages validate. Vulkan execution remains pending.

## Camera-volume aerial and fog integrators

Two more unchanged WGSL entries produce the packed 96-slice camera volume:
`at_aerial_volume` writes slices64..96 and `at_fog_integrate` writes0..64 from
resolved visibility/extinction. The frontend adds typed writable3D heap views;
SPIR-V aliases that view onto storage binding3 by generated variable identity.
The camera record retains the original432-byte layout, flags, fog parameters,
matrices and integer grid. No sun-visibility stub or temporal replacement is linked.

A translated camera and fixed3×2×64 visibility/extinction stream feed independent
scalar references. Both native stages differ by at most0.000488282, below0.002;
the aerial write preserves the initialized fog region, and the subsequent fog
write preserves all aerial slices. The full9-test replay suite passes with Metal
API/shader validation; all22 SPIR-V stages validate. This brings the port to18
original Renderlab entries plus display transfer. Visibility injection and fog
history remain for the later shadow/temporal phase; Vulkan execution is pending.

## Raster sky backdrop

`vs_sky`/`fs_sky` now use the same heap-only fullscreen path. Source composition
calls Renderlab's existing `atmosphere::api_source`, including its DFG sampler
alias; no shader arithmetic changes. The shared frontend selects the actual
vertex/fragment entry names instead of assuming `vs`/`fs`.

The replay renders all eight sun-disk/height-fog/volume-fog combinations against
an independent scalar oracle, using a translated low-altitude camera, an offset
sun at the centre, a nonuniform sky LUT and packed3D fog. All pixels pass
`abs(gpu-reference)/max(1,abs(reference)) <=0.0005`; observed maximum0.000210286.
The peak sun texel's absolute difference is0.518 in much larger linear radiance,
so an absolute-only display-range tolerance would be inappropriate. All10 replay
tests pass with Metal API/shader validation and all24 SPIR-V stages validate.
There are19 original Renderlab entry points plus display transfer. This is a
backdrop leaf, not complete opaque scene/lighting integration; Vulkan is pending.

## Particle depth sorting

The unchanged `tp_particles_sort` workgroup kernel now consumes four heap buffer
indices: original frame/emitter records, particle state, and output keys. Source
composition takes the original frame/particle definitions and sort section;
lighting-dependent simulation and billboard drawing remain separate ports.
The shared buffer-interface adapters retain runtime lengths and threadgroup
scratch, with typed storage aliases on Vulkan. The 2,048-slot limit comes from
the shader's fixed scratch and 11-bit slot encoding, not a new backend policy.

A scalar total-order oracle checks every depth/slot key for capacities 1, 9 and
2,048, including dead and nonpositive-depth slots, ties and power-of-two padding.
All keys are exact on Metal. The oracle reconstructs the eye from the stored
camera view, matching the actual packet input rather than assuming look-at
construction preserved the requested eye bit-for-bit. All 11 replay tests pass;
all 25 SPIR-V stages validate. There are 20 original entries plus display.
Particle simulation/lighting/draw and all native Vulkan runs remain pending.

## Runtime post settings and complete persistent bloom

The post settings record now supports ordered GPU updates, with transfer usage
and transactional allocation inherited from the shared storage buffer helper.
An eight-submission replay updates the same record, draws tone and snapshots
outputs before the first host wait. The scalar oracle covers both tone curves,
LUT on/off, chromatic aberration and the exact wrapping integer grain hash with
changing frame indices. Max tone error is 0.000879109 (tolerance 0.003).
The same fixture exercises the public persistent six-level bloom chain at 65×33,
including its coarsest self-tent: max error 0.002929688 (tolerance 0.004).
All 12 replay tests pass with Metal API/shader validation. Native Vulkan remains
pending. Strict native float flags apply to every pass; no performance claim is
made for this development configuration.
