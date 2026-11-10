# Optional ara-gpu backend

Enable `backend-ara` to build the candidate backend. The default renderer remains
wgpu; its pass files are not edited. Ara is pinned to its GPU-gap branch until PR #5
merges. This is incremental migration work, not whole-renderer parity.

The first slice uses the existing `shaders/post.wgsl` and `post_tone.wgsl` directly.
`src/backend_ara/shader` centralizes naga heap adaptation and checked SPIR-V repairs
proven in Ara's replay. Reusable typed textures, settings records, post encoders and a persistent mip/layer
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

## Initial post replay (six Renderlab entries plus display transfer)

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
compares the initial bloom/tone/display outputs with independent scalar references (tolerance 0.003;
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
same-state storage writes. It schedules whole buffers and whole or mip/layer image views on
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

A translated camera and fixed 3×2×64 visibility/extinction stream feed independent
scalar references. Both native stages differ by at most0.000488282, below0.002;
the aerial write preserves the initialized fog region, and the subsequent fog
write preserves all aerial slices. The full9-test replay suite passes with Metal
API/shader validation; all 22 SPIR-V stages validate. This brings the port to 18
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
The peak sun texel's absolute difference is 0.518 in much larger linear radiance,
so an absolute-only display-range tolerance would be inappropriate. All 10 replay
tests pass with Metal API/shader validation and all 24 SPIR-V stages validate.
There are 19 original Renderlab entry points plus display transfer. This is a
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

## Consolidated review checkpoint

Opus5.5 reviewed fa4b1a7 read-only and found no blocking defect. Follow-up checks
now reject missing LUT storage views, incorrect LUT extents/formats, non-array
environment outputs and specular dimensions that would alter roughness steps.
The final SPIR-V exporter verifies every descriptor maps to Ara's actual heap;
missing a future name-based alias repair can no longer pass silently. Entry
selection retains exactly the chosen vertex and fragment. The particle replay
also reloads a cached pipeline without changing its artifact. The export test
invokes spirv-val automatically when VULKAN_SDK is set; manual validation remains
available otherwise. All 12 native replays pass after these changes.

Remaining proof limits include jittered camera reconstruction, per-sample-different
4x depth and optional skinning branches. The next diagnostic mesh port preserves
the original unlit/diagnostic vs/fs. Complete lit forward needs native Metal 4
per-vertex interpolation and Vulkan fragment-shader-barycentric feature negotiation;
the latter is a pending PC-owned Ara backend change, not a root-constant model gap.

## Indexed, instanced diagnostic mesh draw

The unchanged `scene.wgsl::vs`/`fs` now run through Ara, including transformed
normals, texture sampling, source-winding fragment rejection, diagnostic modes
and reversed-Z depth. Explicit streams retain Renderlab's 48-byte vertex layout
and 4-byte instance-slot layout. The Metal adapter wraps both stages and maps
WGSL locations to Ara's reserved native attributes11+location. Stage input and
front-facing builtins pass through unchanged. Camera/models/normals/material,
two textures and two regular samplers fit the eight roots directly. Normal and
material packing reuse Renderlab's existing writers.

The SPIR-V frontend now selects and compacts original IR before interface
adaptation, re-emitting WGSL through naga with intended equivalent semantics.
This SPIR-V path is structurally validated only: the Metal path translates the
original IR directly. The PC must replay the complete Vulkan cohort after this
change, including single-sided mesh outputs, before claiming native equivalence. This removes unreachable
program/shadow resources rather than supplying fake bindings. Typed heap
preambles include only the records still required after compaction. The entire
27-stage cohort passes spirv-val after this shared change.

The independent scalar raster oracle covers an indexed mesh, three deliberately
reordered instances (including a mirror and nonuniform affine shear), varying
clip-W, per-vertex color/UV/normal, two sampled textures, all four view modes and
both sidedness settings. All eight outputs have exact pixel coverage. Maximum
color error is 0.000699282 (tolerance 0.001); normal mode1.20e-7; depth5.97e-8
(tolerance 1e-6). The fixture keeps every vertex inside the clip volume and its
edges away from sample centres; clipping and 4x mesh raster remain unproven.
An optional ARA_BACKEND_CAPTURE=1 writes inspected PNGs under Cargo target;
only these inspection artifacts receive CPU display conversion. Numeric checks
compare actual GPU linear outputs and depth. All 13 native replay tests pass.

There are 21 original Renderlab entries plus display transfer. This is the original
unlit/diagnostic path; lit forward has not been replaced by a simplified shader.
Its Metal 4 per-vertex and pending Vulkan barycentric requirements remain above.
All native Vulkan runs remain pending. Shared disk space recovered to 161GiB;
the backend still uses one3.6GiB build tree and 11MiB of replay artifacts.

## Transparency raster-depth conversion

`tp_depth_raster` now runs unchanged from the original TP_FRAME and
TP_DEPTH_RASTER sections. It reuses the depth-input heap adapter with a scalar
R32Float storage output. A rendered 17×11 depth fixture is consumed in a later
graphics submission without an intervening CPU wait. View distances differ from
an independent reversed-Z projection formula by at most5.67e-7 (tolerance 1e-5),
and all nine background pixels retain the exact1e30 miss sentinel. All 14 native
replay tests pass; all 28 SPIR-V stages validate. There are 22 original entries
plus display. The original conversion is single-sample; transparency composition,
particle simulation/billboards and native Vulkan remain pending.

When Ara activates this external backend, its workspace must patch the pinned
Ara git dependency to its local ara-gpu package. Patch the optional ara-sdl3-src package to the local workspace too when using its native window feature. This unifies Rust package identity
so Renderlab borrows the existing device; compiling git and path copies together
would produce distinct device types. This is an engine-integration dependency
step, not another GPU device or descriptor-set model.

## Moving models and exposed sky records

Mesh model/normal ranges now update through ordered GPU commands while geometry,
slots and heap identities stay stable. A nonzero-row update changes a mirrored
instance into a nearer positive-scale instance. Two submissions, 16 mode/sided
snapshots and both poses precede the first CPU wait; coverage stays exact, maximum
color error0.000699282 and depth5.97e-8. Camera/material retirement now removes graph
states as well as GPU records.

AtmosphereSun supplies actual unexposed RGB illuminance and camera exposure,
using the original f64 product/clamp when packing. The eight sky flag cases now
exercise illuminance[2,1,0.5] and exposure0.75; maximum normalized error0.000272602
against the scalar reference, within 0.0005. Unit-illuminance LUT constructors retain
their existing behavior. The second Opus review found no blocking code defect;
documentation now explicitly separates Metal proof from structural-only validation
of compacted/re-emitted SPIR-V. The complete native Vulkan cohort remains required.


## Programmable lit toon and a reflected input packet

The production composer now builds the original `toon.wgsl` program on
`scene.wgsl` and executes `vs`/`fs_lit` unchanged on Metal 4. Its 22 reachable
inputs become one persistent storage packet, sorted by original group/binding.
Two roots name the packet and byte offset; no native per-pass sets or uniform
buffers are introduced. `HeapBindings` validates every buffer size, image sample
class and view dimension, and sampler comparison class before publication, and
retains input borrows. `ToonPass` declares the union before one raster scope and
can retain an initialized sky/depth attachment. One-layer array views remain
explicit arrays, including depth arrays.

The independent CPU raster/shading replay covers three reordered instances,
mirroring, affine normals, varying clip-W, textured vertex color, sidedness,
smooth/flat normals, banded direct light, ambient fill and rim lighting. All four
outputs have exact coverage; maximum absolute color error is 0.000164450 against
a 0.003 allowance. Optional shadows, clustered lighting, decals and atmosphere
remain real source branches with valid initialized descriptors but are disabled
in this fixture. Per-vertex interpolation is compiled/linked; this fixture does
not independently observe the shadow receiver's three-vertex payload values.

Shared adaptations added for this interface:

- MSL uses reflected ordinal slots for buffers, images and both sampler heaps;
  runtime-array lengths still come from descriptor byte metadata.
- Naga30's Metal 4 per-vertex input field is declared as `vertices` but read using
  the containing `triangle` argument name. The adapter checks and repairs exactly
  three `metal::vertex_index` extractions to the declared field, preserving order.
- SPIR-V's compacted WGSL re-emission uses inferred local types because explicit
  `frexp` result annotations use an internal type name that WGSL cannot parse.
  Module directives are hoisted ahead of the heap declarations.
- Typed buffer wrappers retain source layouts and direct field reads. The initial
  field-by-field material loader was replaced by the shared physical/logical
  SPIR-V type split described below, which also handles mutable particle records.

There are now 23 original Renderlab entry points plus display transfer. All 15
replay tests pass with Metal API/shader validation, and all 30 exported stages
pass spirv-val for Vulkan 1.3. SPIR-V execution remains pending for the complete
cohort. In addition, this lit raster path needs PC-owned Ara feature negotiation
for `VK_KHR_fragment_shader_barycentric`; no Vulkan backend files were edited here.


## Particle simulation and shared physical/value type separation

`tp_particles_update` runs from the same production prelude, transmission,
atmosphere and particle sections as the wgpu compositor. Its ten selected inputs
use the reflected packet and `Kernel` encoder, including a real comparison sampler
and writable particle storage. Shader group sizes are reflected once at pipeline
creation. The graph sees the particle record as a read/write storage dependency.

A fixed 17-slot emitter uses 20 allocated records: wrapped spawn counter0xfffffffe,
five births across the capacity wrap, random position/velocity/lifetime, drag and
gravity, already dead/never alive particles, exact-boundary expiry, and three tail
sentinels. Two directional lights, an ignored point light, nonunit camera exposure
and distinct frame sun illuminance prove that incident particle lighting stays
unexposed and comes from the light table. Maximum component error is 5.97e-8
(tolerance 2e-5); untouched records and just-expired light words match exactly.
Shadow/transmission and prepared-environment branches are initialized but disabled.

Mutable local `TpParticle` exposed naga 30's reuse of physical buffer structs for
Function values. One shared SPIR-V stage now clones physical composite types only
where internal memory shares their explicit layout, preserves offsets/strides on
those physical types and bridges whole loads/stores using SPIR-V1.4
`OpCopyLogical`. Internal values retain the original logical types without layout
metadata. This also replaces the initial WGSL material-value loader; field reads
and shader math remain unchanged. Unexpected pointer/value origins are rejected.

There are 24 original entries plus display. All 16 native replays pass with Metal
API/shader validation; all 31 SPIR-V stages validate for Vulkan 1.3. Native Vulkan
execution, including these type-copy bridges, remains pending on the PC.


## Sorted particle billboard composition

The original `tp_particle_vs`/`tp_particle_fs` stages now read a twelve-input heap
packet and emit six procedural vertices per sorted rank. Both stages use the same
entry shim; the vertex stage receives vertex/instance IDs and synthetic runtime
buffer lengths. `ParticleBillboard` declares all emitter inputs before one ordered
raster batch and uses the original premultiplied source-over blend.

A fixed 24×20 orthographic replay covers three overlapping live billboards, dead
and padded ranks, birth/death color and size fades, emission/exposure, hard opaque
depth rejection, soft intersections, and procedural radial versus textured shapes.
The CPU reference rounds each blend store to RGBA16Float. Both variants differ by
at most0.000976563 (allowance0.003). One procedural pixel is exactly the clear word
on Metal while the reference changes its red channel by one half ULP; clear-boundary
differences are separately bounded to one half ULP. This is numeric image proof,
not a claim of exact fragment coverage. Atmosphere inputs are valid but disabled;
perspective billboards and active fog remain unproven.

The packet review found no must-fix defect. Both shader backends now assert their
reflected keys and kinds match the original input contract before emission. Sample
classes use an exhaustive format match; the native toon replay rejects short buffers,
wrong dimensions/sample classes/sampler heaps and duplicate keys. Float filterability
is still a caller/device-format requirement; WGSL reflection does not encode it.
Multi-material toon batch/load behavior is implemented but not independently replayed.

There are 25 original entries plus display transfer,17 native replay tests, and 33
validated SPIR-V stages. Native Vulkan remains pending for the entire cohort.


## Conservative HZB base and mip reduction

The unchanged `visibility_buffer_hzb.wgsl::base` and `reduce` entries rebuild one
R32Float image with a complete mip chain. Base extent preserves Renderlab's
power-of-two half-resolution dimensions. The first kernel reads the visibility
buffer's second word as reversed-Z depth; image padding stays zero and cannot
occlude. Reduction clamps neighbours when an axis reaches one texel.

Storage-image inputs now use the same reflected packet, with exact format and
single-mip validation. MSL selects a typed write view from Ara's storage-image
heap; SPIR-V aliases its typed runtime arrays to binding3. Currently supported
write formats are R32Float, RGBA16Float, RGBA32Float and R32Uint; unsupported
storage accesses/formats fail explicitly. No new descriptor-set model is introduced.

The graph registers allocation bounds when an image view is declared, inheriting
its prior whole-resource state. It then tracks mip/layer cells, permits disjoint
read/write views in one node, and emits one batched list of exact range barriers.
Later whole-resource uses expand to those cells; attachment loads require every
cell to be initialized. Buffer and old whole-image call sites retain their API.
Invalid bounds, overlapping writes and undefined reads are rejected before graph
or recorder mutation. `Texture::view` now carries dimension and subresource bounds
as one native view description.

Two 17×11 visibility frames produce 171 exact float texels across five mips each.
Both complete rebuilds and their full-pyramid readbacks are submitted before the
first CPU wait, covering read-after-write, overwrite-after-read and reinitialization
of the same allocation. The native replay also checks invalid read/alias/range
rejection. All 18 replays pass with Metal API/shader validation; all 35 exported
SPIR-V stages validate. There are 27 original entries plus display transfer.
Native Vulkan, including mixed mip layouts, remains pending.

The shared SPIR-V repair's independent review found no must-fix issue. Unknown
store-pointer origins now fail explicitly, matching load handling; header and
unsupported grouped-decoration checks also fail before rewriting.


## Mixed-program tile bins and GPU counter reset

The original `program_bins` kernel now produces per-program tile lists and indirect
arguments through seven reflected inputs. Runtime arrays of atomic 32-bit values
use the common buffer adapter; the existing workgroup scratch path retains the
source's shared atomic mask and barriers. `ProgramBins` resets argument words and
only each list's count header with ordered Ara fill commands, preserving unused
list storage, then dispatches the original 4×4 kernel. Dispatch metadata must match
the supplied source RlList/region records, as documented by BinDispatch.

The independent23×19 fixture spans five program slots, offset/widened region
[8,4,28,24], mixed programs in one tile, different placements sharing a program,
misses, uncovered stale hit records, out-of-image region pixels and four clear
blocks. Slot4 exists only in stale or outside-region records and stays empty.
Two submissions reuse the same counter storage before the first CPU wait. Both
produce exact counts[20,12,12,12,0], encoded tile sets, indirect arguments and
untouched trailing sentinels. Concurrent atomic append order is intentionally
unspecified, so the reference checks sets plus duplicate absence rather than byte
order. The65535-workgroup argument-row rollover is outside this fixture.

There are 28 original entries plus display transfer. All 19 replays pass with Metal
API/shader validation; all 36 SPIR-V stages validate. Native Vulkan execution and
consumption of these arguments by deferred shading remain pending.


## Two-phase cluster cull, argument generation and reset

Three unchanged visibility stages now use reflected packets: `cull`, `finish` and
`reset`. The first classifies the full cluster set; the second pass consumes the
first pass's GPU-written next-dispatch record at byte 48, with separate output
argument buffers. There is no CPU count readback between stages. Finish generates
software/hardware and pyramid arguments; reset preserves all sixteen working words
in diagnostic history, clears them, and clears the shared reject header for pass 0.

The independent f64 reference uses 16 clusters, five LOD groups, three affine models,
a 32×32 viewport and two constant HZB pyramids. It covers owner/refined cut decisions,
frustum rejection, near/eye-plane hardware fallback, small software/large hardware
routing, mirrors, and a shear case that requires conservative affine LOD scaling.
Five previous-pyramid rejects are retested; four recover and one remains occluded.
Both cull/finish snapshots match unordered lists, exact counters, indirect/pyramid
arguments and unused canaries. Reset runs pass 1 then pass 0 to prove header retention
and clearing, and exact history copies. Decisions stay away from floating-point
classification boundaries; rasterization and boundary precision are separate work.

Intermediate snapshots exposed a Metal cross-encoder dependency bug in Ara.
Its fix is pinned at 168ae55e1: late dependencies in an open compute encoder now
also wait on producers in earlier encoders. The Ara native regression failed before
the fix and passes afterward. The complete Renderlab replay retains all intermediate
snapshots and ordinary pass timing; it uses no bypass or intermediate CPU wait.

The image-range graph review found no must-fix defect and highlighted that Metal
conservatively synchronizes stages rather than checking Vulkan layouts. A new pure
`Graph::plan` contract test now verifies exact per-cell before/after accesses,
whole-state inheritance/expansion, layer separation, import/forget and rejection
without state mutation. Actual mixed-mip Vulkan execution remains pending.

There are 31 original entries plus display transfer. All 21 focused tests pass,
including Metal API/shader-validated replays and the pure barrier-plan check; all 39
SPIR-V stages validate. Native Vulkan and full visibility raster/shading remain pending.

## Fixed buffer arrays and software/hardware visibility raster

The unchanged `visibility_buffer_sw.wgsl::raster` and
`visibility_buffer_raster.wgsl::{vs,fs}` now run through Ara on Metal. Both use
64 logical cluster-data buffers. `Input::Buffers` validates the source count and
minimum element layout, retains every owner, and writes arbitrary heap indices
into a tail of the existing persistent storage packet. Its input head word is an
absolute word offset to that tail. Root constants remain 32 bytes. Repeated owners
are allowed; identical shader-storage declarations deduplicate in the graph.
Incompatible access aliases still fail.

Naga's MSL buffer-array parameter becomes a lazy `AraBufferArray<Ptr>` value;
its `operator[]` resolves one packet slot to a buffer pointer. The generated
`.inner`, member accesses and local pointers stay intact. `AraBufferSizes` obtains
per-element byte lengths from heap metadata on demand. No 64-pointer argument array
is reconstructed. SPIR-V rewrites only the first indexed access rooted at each
original array global, preserving downstream members, pointers and `arrayLength`.
Typed runtime descriptor aliases still end at set0/binding1. Three pure tests cover
nested/dynamic/constant indices, shadowed names, parentheses and mismatched spans.

Metal index, data-buffer and image-load policies are now explicitly `Restrict`,
matching wgpu's Metal checks. Earlier comments intended to preserve checks but
Naga's defaults were actually unchecked. The Metal-only conformance probe proves
unequal buffer lengths, a 16-byte prefix, scattered and repeated slots, restricted
out-of-range data reads and shared writable atomic sums. SPIR-V retains its previous
unchecked policy: enabling Naga30 checks fails descriptor-wrapper inference and
emits an invalid signed constant in an unsigned-vector composite. Consequently this
probe is not Vulkan evidence; native robustness and the complete Vulkan cohort
remain pending on the PC.

The raster replay queues direct and indirect draws of both paths before the first host wait. Its independent
f64/i64 oracle checks all 256 pixel words and four allocation guards exactly:
12 clusters, two actual data owners at logical slots3/37, mirror/translation,
front/back rejection, depth and triangle-ID ties, degenerate triangles, partial and
fully offscreen geometry, and three padded software workgroups. Hardware emits
384 procedural vertices per cluster with GreaterEqual depth writes so equal-depth
fragments reach the unsigned 64-bit maximum. Both constructors reject devices
without the required unsigned buffer min/max capability. The fixed fixture has
constant dyadic depth and no pixel centres on edges; varying depth and near-plane
clipping remain unproven.

There are 33 original renderer entries plus display transfer. All 27 focused tests
pass with Metal API/shader validation; all 43 exported SPIR-V stages (including the
adapter probe) validate for Vulkan1.3. Native Vulkan runs remain pending. The
repository's unrelated lib-test target currently fails E0275 in
`receiver_policy_tests.rs:396` on this Mac; the pure frontend tests also compile
and pass inside the focused integration target.

The consolidated buffer-array review found no must-fix defect. Its GPU-driven wiring
findings are now covered: `ShaderRead` plus `Indirect` on one buffer merges to Ara's
existing combined `Indirect` access, and hardware draws consume the original args
record at byte16. The fixture first copies args on the GPU, then queues direct SW/HW
and indirect SW/HW snapshots. The final hardware draw loads the initialized depth
attachment; attempting that load before a producer is rejected. Short array counts
and short element layouts are also rejected. A pure graph test checks both read
orderings and transactional rejection of write/read aliases.

Ara is now pinned to `c040cc1d5`, including both native backends and the complete
joint test cohort. The Linux gate exposed Ara's requirement that SPIR-V exports native entry
`main`. The adapter now renames the selected IR entry without changing WGSL.
Every emitted stage's metadata records `main`; the export test reads `OpEntryPoint` from each binary
and checks it against its metadata before structural validation. Native Vulkan
execution of this Renderlab cohort still remains pending. No new Ara API or decision
override was needed for the buffer-array or indirect-raster integration.

## Visibility receiver resolve (entry 34)

`visibility_buffer_resolve.wgsl::resolve` now uses the original ray-free source
composer, shared through a crate-visibility change on that existing factory. The
selected WGSL body stays unchanged. Shared selection now evaluates WGSL override
defaults with naga before either native backend; it resolves workgroup sizes through
the same step. The buffer-array probe uses an overridden default group size to
exercise that path. This preserves defaults `WRITE_RECEIVERS=true` and `LOD_LIFT=0`;
non-default specialization is not claimed by this checkpoint.

An independent f64 plane/barycentric oracle checks a 10×9 image and 2×3 coverage
image, with four hits, two logical owners at 3/37, linear and TABLE primitive IDs,
a mirrored thin placement, scene bounds, covered misses, wholly uncovered tiles,
partial edge tiles and allocation/image canaries. The encoded visibility depth is
deliberately unrelated to the source triangle plane. Both queued submissions rebuild
receivers from source positions, consume only in-scene hit words, and preserve stale
outputs where the original kernel skips stores. Maximum barycentric/world errors
are 1.94e-7/1.79e-7; normals, IDs, epsilon bits, thin sign, coverage and untouched
regions match exactly. Adjacency/edge repair and nonzero LOD lift remain unproven.

After merging Renderlab main 7bd8b9a, all 28 focused tests pass under Metal API/shader
validation, and all 44 exported SPIR-V stages validate with matching native entry
metadata. Native Vulkan remains pending. There are 34 original entries plus display.

## Hit-based post surface (entry 35)

The unchanged `post_surface.wgsl::post_primary_surface` now runs from the original
ray-free head plus `object_motion.wgsl`. Two crate-visibility changes expose the
existing head composer; the wgpu pass bodies are unchanged. Current/previous models
and three fixed buffer arrays preserve the original rigid/skinned stream ABI. The
rigid fixture shares its current and previous buffer owner, exercising readonly
aliases across logical inputs.

Four queued submissions, with GPU-ordered updates to one parameter buffer, compare
all 117 RGBA32Float pixels against independent f64 reconstruction/projection:
7×5 internal hits to 13×9 output, two scattered owners, reordered corners, rigid
model motion, skinned vertex history, sky camera motion, stale uncovered hits,
velocity-radius clamping, history off, previous clip behind the camera and minimum
view depth. Maximum errors are 9.54e-7 for motion and 2.39e-7 in the other variants.
Sky depth 1e8, alpha 1 and the history-off/behind velocities match exactly.

All 29 focused tests and 45 exported SPIR-V stages pass on Metal with API/shader
validation and structural/native-entry audits. Vulkan execution is pending. There
are 35 original entries plus display; temporal reconstruction remains a later phase.

## Transparency hit depth (entry 36)

The original `transparency.wgsl::tp_depth_hits` runs from its existing standalone
composer, exposed by one crate-visibility change. It consumes primary hit/tile
textures, models/placements and two fixed buffer arrays. The source keeps signed
linear distance; it does not apply post surface's minimum-depth clamp.

Four queued frame updates cover forward/reverse directions, an oblique cropped
scene and an empty scene. The independent f64 geometry oracle shares raw primary
fixture data, not its post-render results. All 35 depth texels match within 2.39e-7;
the 12/12/23/35 miss values are exactly 1e30. Tiny positive and negative distances
remain near ±0.0005 rather than clamping to 0.001. All 30 focused Metal tests and 46
SPIR-V stages pass; native Vulkan remains pending. There are 36 original entries
plus display transfer.

## Primary geometry check (entry 37)

The original arithmetic-only `primary_check_interactive` composition runs on the
heap packet, including its two 64-element mesh buffer arrays. `PrimaryCheck`
resets only the retry queue header, then records direct or GPU-indirect work
using the production 256-column grid. It does not read queue counts on the CPU.

The independent fixture checks six active repairs, two mesh owners and padded
invocations. Coplanar overlap canonicalizes the primitive and barycentrics exactly;
missing adjacency sets fallback flags and appends the exact semantic retry set.
Two submissions precede the first CPU wait. Queue headers, untouched repairs and
canaries compare word-for-word on Metal with API/shader validation. Concave-fold
and inset-probe branches remain unproven. Native Vulkan is pending.

Checkpoint: all 31 focused tests pass with API/shader validation and API-only;
47 exported SPIR-V stages validate for Vulkan 1.3. No backend clippy diagnostics.

## Fixed sampled-image and sampler arrays

`Input::Images` and `Input::Samplers` use the same packet-tail scheme as buffer
arrays: a head word points at buffer-absolute words containing arbitrary heap
indices. Reflection checks exact source counts, image dimensions/sample classes,
and regular versus comparison sampler heaps. Packets borrow every image owner
and declare each view to the graph. Samplers remain immutable device-cached state.

Naga MSL prints handle arrays as `constant NagaArgumentBufferWrapper<T>*` and
element access as `[i].inner`, including hidden helper-function parameters. The
shared adapter replaces only that parameter type with a lazy `AraImageArray` or
`AraSamplerArray`; its `operator[]` resolves one heap slot. Auxiliary image entry
structs precede generated helper bodies. Naga binding-array targets remain buffer
arguments; texture/sampler binding attributes would be rejected by its writer.
WGSL algorithms remain unchanged. SPIR-V uses the same AST first-index rewrite
as buffer arrays and aliases typed image/sampler arrays onto bindings 5, 2 or 6.

The conformance replay uses noncontiguous/duplicated 2D color/depth slots, nearest
and linear filtering, clamp/repeat/mirror addressing and four comparison modes.
Dynamic indices, constant indices, texture dimensions and mip counts match the
CPU reference within 1e-6; tail canaries are exact. Two differently mapped packets
are submitted before the first CPU wait. Wrong count, sample type and comparison
heap are rejected. All 32 focused tests and 48 SPIR-V stages pass with Metal API
and shader validation. Native Vulkan is pending. Array/3D/uint image proxy forms
are supported by reflection but not yet claimed as native-proven by this probe;
storage-image binding arrays are explicitly unsupported. Binding-array indices
retain Naga's unchecked policy and must be within their logical source count.

## Programmable primary shading (entries 38–39)

`primary_shade::Entry::{Tiles,List}.program(material_wgsl)` uses the original
non-RT shading heads and `program::compose` / `compose_list`. The material owns
its Surface/Parameters definitions and surface/shade hooks. `shader::Program`
reflects that caller-owned source; `Kernel::from_program` and
`HeapBindings::for_program` retain matching packet identities after the source
is dropped. The native cache, workgroup reflection and SPIR-V export all consume
that same source. All kernels and packets retain an ABI identity derived from source keys and kinds.
Compatible default/custom packets interoperate; incompatible layouts fail before recording.
The array conformance replay also changes an authored workgroup size from 4 to 2
and proves its native dispatch and exported SPIR-V.

The original tiled and tile-list entries run against a 19×9 hit image, partial
region, uncovered stale records, misses and two batch owners 3/37. Five 64-element
geometry/material/parameter arrays and six image/sampler arrays stay in packet
tails, including two-layer normal maps sampled at layer 1. The independent f64
reference evaluates barycentric UV/color interpolation, transforms, material
texture lookups and per-owner parameters. All 171 pixels compare within
0.00048828125. Program-zero and program-one lists reproduce the full tiled image;
the first list preserves exactly 36 other-program sentinel pixels. GPU-resident
indirect buffers drive the lists; all three snapshots are queued before waiting.

The shader algorithms were unchanged. Defaults disable atmosphere, decals and
local lighting in this fixture; their original controls and valid fallback inputs
remain. Analytic derivative values, filtered/mip material sampling and moving
GI/reflection/shadow/temporal algorithms are not proven here. Native Vulkan is
pending. The production non-RT source factory gains only crate visibility.

Review found Naga 30 omits NonUniform on combined OpSampledImage results although
its divergent image/sampler inputs are decorated. A shared word-level pass now
propagates that resource identity, with an independent audit of every exported
stage in addition to spirv-val. See [Khronos descriptor-indexing guidance](https://docs.vulkan.org/samples/latest/samples/extensions/descriptor_indexing/README.html).
The query probe now uses four distinct extents and differing mip counts, so the
CPU checks detect a wrong heap slot. Storage-image array rejection also runs in
the integration target. The blocked legacy lib-test target is not relied upon.

Checkpoint: 39 original entries plus display are proven. All 33 focused tests pass
in both Metal validation modes. Fifty standard stages plus the authored-workgroup
stage pass Vulkan 1.3 validation, native-entry and NonUniform binary audits.
Focused backend library/integration clippy reports no new diagnostics.

## Ray-query scene packets (diagnostic probe)

`acceleration::Scene` owns a published TLAS and `Input::Scene` retains it in a
packet with an AsRead graph use. Metal reads the converter-compatible TLAS header
from the acceleration table's GPU pointer. The packet head stores its normal
heap index; roots remain `[packet_slot,16,0,0,0,0,0,0]`.

The original diagnostic `primary_probe` runs in both default and WRITE_WORLD
modes on Metal. A shared two-triangle BLAS is instanced twice with translation,
reflection and shear, distinct custom IDs 3/37 and masks. Across 187 pixels the
independent f64 reference checks exact custom/primitive IDs, 123 exact misses,
and barycentrics/depth/world within 2e-5. A built decoy TLAS occupies the first
heap slot so an accidental constant-zero descriptor cannot pass. Both snapshots
are submitted before the first wait; disabled world writes preserve raw canaries.
This diagnostic does not increase the 39-production-entry count.

Naga 30 cannot re-emit ray-query WGSL, and its analyzer panics on dynamic AS
binding-array indices. The shared SPIR-V route preserves original ray-query
source, relocates old binding groups until IR compaction removes them, and emits
constant descriptor-index markers. A checked binary pass replaces only those
markers with actual packet-head loads. It validates naga's root/packet type
shapes and leaves the same heap packet and 32-byte root API on both backends.
SPIR-V validates; native Vulkan execution remains pending.

The authored-shading review found a proof gap: its material initially matched
the conformance default. They now have observably different MR coefficients;
runtime shading rejects implicit conformance materials and requires caller WGSL.
The authored probe's exported LocalSize is audited as 2×1×1. Contract identities
now omit source names and origins, with positive default/authored interoperability
and negative tests using a genuinely different source key. Normal/tangent values,
filtered material mips and the moving lighting algorithms remain unproven.

Checkpoint: 34 focused tests pass in both Metal validation modes; 51 standard
SPIR-V stages plus authored-workgroup and world-output variants validate. Backend
library and integration clippy is clean; legacy wgpu diagnostics remain.

## Production primary tracing (entries 40–41)

`primary_trace::Entry::{Trace,Receivers}` composes the original hardware camera
trace source. A checked compatibility step replaces exactly two candidate-position
intrinsics and one committed-position intrinsic with the existing
`primary_triangle_vertices` helper. It resolves the instance custom ID through
`primary_placements` to the batch, then gathers the three indexed object-space
positions from that batch's vertex buffer. Two query types and one AS declaration
lose the vertex-return qualifier; traversal, confirmation, culling, repair and
receiver arithmetic remain unchanged. AS builds and fetches consume the same
retained 48-byte vertex records and primitive ordering.

Two genuinely different BLAS owners (3/37), sparse placement IDs (4/9), shuffled
TLAS order, reordered corners, mirror/shear, and a camera translated to X=64 make
wrong-owner or cached-position fetches observable. Both original entries pass
forced-opaque and single-sided candidate-confirmation modes. Across four queued
submissions, IDs, crop misses, coverage and untouched records are exact; receiver
world/normals/barycentrics match the f64 reference within 2e-5 and nonfloor epsilon
is 1/8192 within 1e-9. Trace leaves world/normal images untouched; receivers leave
the epsilon image untouched. Empty tiles preserve stale hit records as designed.

This fixture keeps hits outside the edge-repair strip. Direct near-edge repair
traversal, AS refit paired with deformation, ties and degenerate geometry remain
unproven here. The earlier arithmetic-only geometric-check replay covers its own
canonicalization/fallback queue cases. The existing wgpu source factory gains only
crate visibility; source algorithms are not edited.

Checkpoint: 41 original entries plus display, 35 focused tests in both Metal
validation modes, 53 standard SPIR-V stages plus two authored/override variants
validated. Native Vulkan remains pending. No new backend clippy diagnostics.
The quiet CPU timing guard again refused the run at 17.98% foreign CPU load
(threshold 10%); no timing sample was accepted or added.
