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

## Transparency seed and fallback presentation (entries 42–43)

`transparency_seed::Seed` uses the original TP_AA_SEED section. A backed
`MsaaColor` keeps four RGBA16Float coverage samples for subsequent pane draws;
`resolve_to` preserves those samples and validates its single-level destination.
The 7×5 replay copies nonuniform HDR and alpha exactly in two queued frames,
including a second resolve from the same multisample contents.

`FallbackPresent` uses the original scene.wgsl vs_present/fs_present pair. It maps
RGB as value/(1+value), writes alpha one, and selects the target's actual mip/layer
through Ara's portable raster-attachment API. Its half-float target differs from
the CPU reference by at most 0.00048828125. This is separate from post tone and
the explicit display-transfer shader. No shader body changes were needed.

All 36 focused tests pass with API/shader validation and API-only validation;
57 standard SPIR-V stages plus two authored/override variants validate. Native
Vulkan is pending, and backend library/integration clippy has no diagnostics.
The latest scoped AS review had no must-fix findings. Its small negative-test
follow-up now checks the specific caller-material diagnostic.

## Programmable transparent panes (entry 44)

`transparency_glass::program` composes the original non-RT glass source with
caller-authored material WGSL. `Glass` draws into the retained four-sample HDR
attachment with depth/blending disabled, matching the source renderer. Resolve
that attachment into the packet's behind-color image between overlapping panes.
`Program` and the native cache now support raster families with explicit render
state; the compute convenience path rejects a raster program.

The 8×6 replay runs two sorted panes with nonuniform HDR behind color, opaque-depth
rejection, distinct material/parameter/base-texture records and a mirrored model.
It checks both intermediate resolves against scalar composition, then repeats
with a single-sided backface and with zero packed opacity. Untouched pixels stay
exact; changed channels are within two half ULPs. The authored shade differs
observably from the conformance material, so this proves caller code is compiled.
All resources are initialized and both pane submissions precede the first wait.
Fractional MSAA edges, tangent streams, refraction thickness, active atmosphere,
and declared reflection/transmittance hooks remain unproven by this fixture.

No new per-pass naga repair was needed: this reuses the shared packet adapter,
Metal4 per-vertex input support and vertex locations11+N. Both default and authored
raster pairs pass `spirv-val` for Vulkan1.3; native Vulkan needs fragment barycentric
capability negotiation and remains pending. The source factory gains only crate
visibility, with no edits to the original shading algorithm.

All **37** focused tests pass with Metal API+shader validation and API-only
validation. **59** standard SPIR-V stages plus four authored/override stages
validate. The independent scoped review found no must-fix issues. Clippy adds no
warnings in the backend library or new Glass files; existing fixture warnings
remain. Ara is now pinned to **0b4308e18**, including both reviewed Vulkan
queue-family fixes and main d8015f178.

## Fresh receiver raster and current-main integration (entry 45)

`receiver_raster::Receivers` uses the original `vs`/`fs_receiver` pair from the
fresh-shadow source factory. It clears the world/epsilon, raw geometric normal/thin
sign and reversed-depth attachments once, then retains depth winners over ordered
mesh draws. Color attachments select their actual mip/layer. This prepass executes
no ray queries; visibility tracing remains a separate pass.

The scalar fixture uses perspective, mirrored/sheared models and reordered instance
slots, translated to X≈384 with a compensated camera. Three separately recorded
mesh draws check attachment preservation. Four queued sidedness/thinness variants
match 338 covered pixels each, with exact misses, epsilon1/2048 and thin sign;
maximum world error is 0.000091552734 (three f32 ULPs at384), normal/depth error
5.96e-8. Tangent streams, clipping and multisample receivers remain unproven.
The independent review found a missing packet-contract check; a same-family,
zero-binding program was accepted before the fix and is now rejected before graph
or command mutation. The fixture records that failing-before/passing-after probe.

Renderlab main **f820af9** is merged. Its original source factories now supply the
256-entry draw constant to visibility and program binning. The replay arrays/list
records use the public `TRACED_DRAWS` value; original software/hardware visibility
and authored primary shading actually fetch logical owner173, beyond the old limit.
No shader repair was required, and the lazy packet frontend needed no capacity change.

Main also extends `TpEmitter` to512 bytes and changes birth motion and texture UVs.
The sort record and replay inputs now use that ABI. The CPU birth oracle accounts
for the extra random draws, analytically integrates acceleration/drag, and preserves
noise phase; it differs by at most1.20e-6. Billboard sampling uses the new top-left
UV convention. Extended shapes, forces, curves and flipbooks are compiled but are
not CPU-reference proven by these default-extension fixtures. All original wgpu
changes on this branch remain visibility seams only.

All **38** focused tests pass in both Metal validation modes; **61** standard
SPIR-V stages plus four variants validate for Vulkan1.3. Backend library and
integration clippy report no diagnostics; existing warnings outside this module
remain. Vulkan execution of every migrated pass is pending for the PC.

The Ara0b4308e18 CPU barrier benchmark was rerun after the native hardening:
200 alternating AB/BA pairs after20 warmups, 256 barriers, timestamps disabled,
with GPU waits excluded from CPU time. Individual/batched total p50 is
**131.416/15.208µs**, p95 **165.667/35.125µs**; record p50 **3.208/0.500µs**, submit
p50 **128.333/14.708µs**. This is a microbenchmark, not a renderer-frame speed claim.
The unchanged `ara-dev gpu` guard accepted it: foreign CPU mean5.82%, peak11.33%
without a sustained breach; GPU peak0.64%. Evidence stays in Ara's ignored
`target/ara-gpu/barrier-timing-review.{json,log}` and
`target/ara-dev/gpu-load/90104-1794339026238.json`. The numbers are also on Ara PR#5;
its head remains0b4308e18, so this measurement does not change the Vulkan gate target.

## Fresh shadow visibility queries (entry 46)

`fresh_visibility::Visibility` runs the original `fresh_compute` over world/normal
receiver images and a retained TLAS. Exact mode traces256 directions with eight
lanes per pixel in2×2 tiles. Interactive mode specializes only the existing
`FRESH_DIRECTIONS` override to8, using one lane per pixel in8×4 tiles; both retain
the0..256 count contract. Dispatch dimensions are derived from the selected tile
shape, rather than treating the32-thread workgroup as32 adjacent pixels.

The7×5 fixture compares all35 counts exactly in six queued variants: zero-angle
and finite-angle light, both budgets, and disabled shadowing. The finite-angle
case includes14 penumbra receivers in each mode. Uncovered/out-of-cascade/out-of-box
pixels overwrite sentinels with256. A receiver pair close to the blocker observes
the original opaque/thin origin bias as0 versus256. A built decoy TLAS occupies
the first publication, making a literal-zero AS lookup observable.

The oracle preserves f32 hash/Kronecker coordinates, then uses an f64 cone and
independent two-sided triangle-segment intersections. Sampled rays remain away
from the blocker's external silhouette; no approximate count tolerance is needed.
Multiple cascades, arbitrary projection/instance transforms and tangent rays are
outside this bounded fixture. No new naga repair or original shader edit was
needed; the shared ray-query packet path serves both modes.

All **39** focused tests pass with Metal API+shader validation and API-only
validation. **62** standard SPIR-V stages plus five authored/override variants
validate for Vulkan1.3, including the eight-direction specialization. The scoped
independent review has no must-fix findings, and backend library/integration clippy
has no diagnostics. Native Vulkan remains pending.

## Shadow-map raster and atlas selection (entry 47)

`shadow_raster::ShadowRaster` runs the unchanged scene `vs_shadow`/`fs_shadow`
pair with the original depth-only Greater state and zero clear. Its vertex stage
reads builtin instance indices, unlike the regular mesh stage's location4 slot
stream. `ShadowDraw` therefore carries a checked native model range. The encoder
binds only the48-byte vertex stream and index buffer; a range starting at1 really
reads model rows1 and2. Source-sided winding/discard stays in the original shader.

`Texture::depth_atlas` publishes a parent and narrow2D views ordered by mip/layer.
The replay writes mip1 layers0 and1 of a two-mip, three-layer Depth32Float atlas,
with a range1..3 followed by row0, mirrored/sheared models and two sidedness modes.
All four queued depth snapshots match the scalar raster reference:338 covered
pixels, exact cleared misses and maximum error5.96e-8. Every mip0 layer and mip1
layer2 preserves its initialized canary. An oversized model range is rejected
before drawing. Contracts, table bounds and selected view dimensions are checked.

No new shader rewrite was needed; the shared adapter maps the explicit position
attribute to Ara's native location11, leaving the builtin instance index intact.
Clipping, dynamic bias, deformation updates and multisample shadows are outside
this fixture. The independent review has no must-fix findings.

All **40** focused tests pass with Metal API+shader validation and API-only
validation. **64** standard SPIR-V stages plus five variants validate for Vulkan1.3;
backend library/integration clippy has no diagnostics. Native Vulkan remains pending.

## Vertex-only transparency sun shadow (entry 48)

The original TP_SHADOW source contains only `tp_shadow_vs`. The frontend now keeps
that shape through selection, MSL/SPIR-V and cached metadata. `MetalEntries`
represents compute and raster with an optional fragment; the cache writes exactly
one vertex stage. No fragment stub is inserted. This required one Ara Metal loader
addition, now pinned at **b822fb343**, with the same portable cooked stage shape and
no Vulkan-file or heap-ABI changes. Ara's native depth regression failed before
admission support and now passes; full GPU/replay gates pass109 in both Metal modes.

`ShadowRaster::transparency_sun` preserves Renderlab's original Less comparison,
clear1, constant bias2 and slope bias2. It validates only the model table needed by
this source; source-sided shadow maps retain their normal/material validation.
Two native model rows, including a mirror, produce35 covered pixels each with exact
misses. Flat-plane depth agrees with the CPU value plus a positive native precision
bias bounded by2e-6; observed offsets are5.96e-8 and2.98e-8. Sloped/general casters
and clipping remain unproven here. The test also reads the cached JSON to require
one vertex stage and no fragment artifact.

All **41** focused tests pass with Metal API+shader validation and API-only
validation. **65** standard SPIR-V stages plus five variants validate for Vulkan1.3;
backend library/integration clippy has no diagnostics. The scoped shape review has
no must-fix findings. Native Vulkan remains pending. No decision override was added.

## Coloured sun transmission layers (entry 49)

The original `tx_vs`/`tx_fs` pair from `transparency_shadow.rs` runs unchanged on
`scene.wgsl`. `Transmission::record_layer` keeps Renderlab's scheduling: every
transparent caster clears and draws its own `Rgba16Float` array layer (white, depth
one, forward Less, no blend, no culling), and lighting combines layers afterwards.
`Texture::view_2d` publishes one mip/slice of a parent and rejects a view outside
the parent's interval; `depth_atlas` now builds its views through it.

Four fixtures replay against an independent CPU reference: pane order, reversed
layer placement with an oblique sun (Schlick's angular term), a mirrored pane with
reversed triangles, and zero packed opacity. Each layer is read back after its own
draw: clears, coverage and alpha are exact, RGB is within two half-float ULPs, depth
within 2e-6. An HDR base texel makes the RGB clamp observable. A whole-array readback
at the end equals the per-layer snapshots, so a later caster does not disturb an
earlier layer. Sloped casters, clipping and textured (non-constant) bases remain
unproven here.

All **42** focused tests pass with Metal API and shader validation from a clean
build, and the SPIR-V cohort including `TransmissionShadow` passes `spirv-val` for
Vulkan 1.3. Native Vulkan remains pending.

## First assembled frame: forward lit

`backend_ara::forward::render` is the first code that assembles a frame from the
ported passes instead of replaying one alone. It covers what
`Renderer::try_render_view(.., ViewMode::Lit)` encodes on an untraced frame with the
atmosphere, shadows, GI, decals and transmission off: the LC1 cluster build, then the
lit mesh pass with the **standard** program (`scene.wgsl` + `programs/standard.wgsl`,
composed by `program::compose` exactly as the wgpu lit raster variant), into the
exposed `Rgba16Float` HDR target, read back as `try_capture_hdr` returns it.

Inputs come from Renderlab's own writers: `PackedLighting` records and uniform,
`light_cluster::header`, `MaterialDescriptor::bytes`, `mesh::matrix_bytes`, the
prepared DFG. Every other reflected input of the lit interface is a feature that is
off; it is bound with what the wgpu renderer binds there (zero-filled records and
zero-initialised 1x1 views, the three 1x1 default material textures). The clear is
`Renderer::lit_clear`. `ToonPass::program` builds the lit raster pipeline for any
caller-composed program; `MeshToon` names that family, not the toon program.

`forward_lit_frame_matches_closed_form_photometry` holds the frame to the oracle and
tolerance of the wgpu renderer's own `gpu_lit_photometry_1x_4x`: a floor facing the
camera, six metal/roughness materials, a directional, three point and one spot light
one at a time (unbounded, ranged, out of range, cone falloff), the BRDF and
attenuation in f64, and the uniform environment against an independent DFG
quadrature. All **4,338** radiance samples are within `0.002 + 1%`; the worst uses
7.0% of that allowance. Clustered local lights therefore reach shading through the
GPU-built lists. Not covered: 4x, tilted and mirrored transforms (the oracle's last
part), textures, more than one object per packet.

`examples/ara_frame.rs` renders the authored **M5-A** lighting fixture (six spheres,
floor, mirrored and double-sided panels; directional, point and spot light) on either
layer with the same features off, and compares two outputs:

```sh
cargo run --example ara_frame -- --backend wgpu --output out/wgpu       # Linux/Windows
cargo +1.99.1 run --features backend-ara --example ara_frame -- --backend ara --output out/ara
cargo run --example ara_frame -- --compare out/wgpu out/ara
```

Each run writes the raw `frame.rgba16f`, the displayed `display.rgba8`, PNGs for looking
at, and `frame.json` with 24x16 grids of mean linear RGB and mean display bytes that
can travel as text. wgpu cannot produce its side on this Mac: it opens Vulkan only,
and over MoltenVK 1.4.1 its device is lost during `Renderer::new_with_options`.

### Cross-layer result: wgpu on Vulkan against Ara on Metal

Reference: wgpu at e5be1b3 on an AMD Radeon 760M (RADV PHOENIX, Mesa 26.0.8, Vulkan
1.4.335), dev profile. Ara: this Mac's M5 Pro on Metal 4 under API and shader
validation, at eff8d75 (the frame code is unchanged since e5be1b3). M5-A, pose 0,
960x640, 614,400 pixels. HDR error is relative to the brighter of the two values,
floored at 0.001; display error is in 8-bit levels.

| Level | HDR max | HDR mean | HDR pixels over 1% | Display identical | Display worst |
|---|---:|---:|---:|---:|---:|
| Forward lit, default post | 0.29% | 0.00028% | 0 | 92.6% | 1 level |
| + physical atmosphere | 0.30% | 0.050% | 0 | 91.2% | 1 level |
| + ray-traced sun shadows | 1.58% | 0.051% | 1 | 91.0% | 1 level |

No displayed pixel differs by more than one level at any level, and no HDR pixel by
more than 5%. The single pixel over 1% is at (425, 331), on a shadow edge. With the
atmosphere on, the mean rises to 0.05% and one grid cell's display mean differs by a
full level: a small systematic difference in the sky and its lighting between the two
GPUs' arithmetic, far below a display step per pixel. The two layers share no shader
compiler, no API and no GPU vendor here, so this is the first evidence that the
WGSL-through-naga path on Ara draws what wgpu draws for whole frames, not only for
single passes. It covers the levels above and nothing else: one scene, one pose, one
frame after a cut.

### The post tail on the same frame

With `ForwardScene::post` set, the frame continues through `PostGpu::encode` and the
tone stage in the renderer's order: raster motion surface, tile maximum, motion blur,
the bloom pyramid, tone, then the display transfer to RGBA8. `PostSettings::record`
now packs the 80-byte `Settings` record for both layers; `PostGpu::prepare` calls it
instead of packing inline (the arithmetic is unchanged, the wgpu side compiles here
but has not been run since the move). The frame is always the first after a cut:
serial one, no previous camera matrix, the 2x2x2 identity LUT at strength zero. Depth
of field is rejected rather than skipped.

`forward_frame_display_matches_cpu_post_replay_of_its_hdr` reads back the frame's own
HDR and motion surface, pushes them through the scalar post replay used by the stage
tests (tile maximum, motion blur, six bloom levels, tone, sRGB transfer) and requires
every displayed byte inside that replay's bounds: the tone stage's 0.003 linear
allowance carried through the transfer, plus UNORM rounding. All 6,144 pixels pass, on
a glossy metal floor whose highlight peaks at 27.7 with 572 distinct display colours.
What this does not show: motion (the camera has no history), depth of field, a custom
LUT, grain.

The M5-A `display.png` is the first Renderlab image shown through Ara end to end, from
scene data to display bytes, with no wgpu device and no CPU tone mapping.

### The atmosphere on the same frame

With an active `ForwardScene::atmosphere` the frame adds `encode_atmosphere` in its
order: transmittance, multiscatter, sky-view and ambient LUTs, the five-level
procedural specular cube and the diffuse cube, and the aerial slices of the camera
volume. The sky backdrop then replaces the clear under the mesh pass, the lit pass
reads the LUTs, the volume and the cubes in place of the "off" views, the lighting
uniform carries `EnvironmentState::uniform` for the procedural chain, and
`Atmosphere::transmittance` scales the directional records as `sun_scale` does.
Volumetric fog is rejected: its inject and temporal kernels are not ported.

`forward_frame_atmosphere_scales_the_sun_and_lights_from_the_sky` holds the assembly,
not the sky's radiance. A low sun's transmittance (0.72, 0.49, 0.26) reaches the
floor exactly once, within the photometry tolerance; the backdrop replaces a black
clear; and a floor facing away from the sun is black without the procedural
environment and blue-weighted with it. The sky's own values rest on the stage replays
and on the cross-layer comparison, which is the only whole-image check of this level:
`ara_frame --atmosphere` renders M5-A under the physical sky on either layer.

### Ray-traced sun shadows on the same frame

With `ForwardScene::sun_shadows` the frame is the one
`Renderer::new_with_hardware_shadows` draws for a sun with a disk on an untraced
frame: one bottom-level structure per object and a top-level scene with one instance
per placement, the receiver prepass (entry 45) into world and normal targets with the
frame's depth, 256-ray fresh visibility (entry 46) into an `R32Uint` image, and the
lit pass composed on the fresh prelude, reading that image through group 0. It needs
a ray-query device.

The CPU half is shared with the wgpu renderer rather than copied.
`shadow_gpu::plan_frame` is `ShadowState::plan` as a function of the settings and the
previous orientations; the method now calls it. The backend plans the sun's views
through it, runs `prepare_hardware_bias`, binds the packed view records, and writes
each light's view range into its record as `upload_shadows` does. As with
`PostSettings::record`, the wgpu side compiles here but has not run since the move.
Casters must be double-sided, the renderer's own admission rule.

`forward_frame_sun_shadow_has_the_predicted_umbra_and_penumbra` holds the result to
geometry: a square occluder one unit above the floor under a sun 56 degrees off the
normal with a 0.05 rad disk. Along the centre row 61 unshadowed pixels match the
closed-form radiance within 2%, 13 umbra pixels are black, and the penumbra is a
falling ramp that crosses one half at x = -2.003 where the occluder's edge projects
to -2, spanning 0.260 floor units between 5% and 95% against a full geometric width
of 0.327. Not covered: more than one cascade, a moving sun (no previous orientation),
single-sided or thin casters, local-light shadows.

`ara_frame --atmosphere --sun-shadows` renders M5-A with the sky and these shadows on
either layer. On this Mac it runs at 960x640 under Metal API and shader validation;
the picture has contact shadows under every sphere and both panels, tinted by the
sky.

### The traced frame: the engine's reference path, minus post

`ForwardScene::primary = Primary::Traced` draws the same scene the way `HardwareRt`
does at `Quality::Reference` with GI, temporal reconstruction and reflections off:
the camera trace with receivers (entry 41), the fresh estimator in its traced form
(coverage gated on the primary tiles and hit image), then the standard program on the
hardware soft head, which reads the visible-ray counts and the hit records and writes
the HDR target from compute. The frame assembles what `Renderer::prepare_primary`
publishes: the 144-byte frame record, the tile-aligned region, the placement table,
current and previous placement matrices and normal transforms, and the per-draw
vertex, index, tangent and material arrays padded to `TRACED_DRAWS`. It shares the
scene's acceleration structures, the shadow frame, the lighting group and the
atmosphere inputs with the mesh-pass family.

`sun_shadow_has_the_predicted_umbra_and_penumbra_on_both_frame_families` runs the
shadow fixture through both families. The traced frame, rendered first on a device
that has drawn nothing, meets the same geometry on its own: 61 unshadowed pixels
within 2% of closed form, 13 umbra pixels black, the penumbra crossing one half at
x = -2.003 over the predicted width. Compared with the mesh pass it is identical in
all 36,864 channels of the 128x96 frame. Both unproven variants named below have
therefore executed and agree with a frame that is itself checked against wgpu.

`traced_frame_agrees_with_the_mesh_pass_over_misses_sky_and_post` adds a frame the
geometry only partly covers, with the uniform environment and with the physical sky,
through bloom and tone to display bytes (motion blur off: its surface on a traced
frame comes from hit records, entry 35, not chained yet). Misses show the identical
background in both families, sky included; 11 and 25 of 12,288 pixels differ at all,
by at most 1.4%, all in the penumbra; displays agree within one level.

`ara_frame --traced` renders M5-A this way on either layer: on wgpu it is the
renderer's traced primary visibility with `SoftShadows::Exact` and GI, temporal
reconstruction and reflections off. On this Mac the traced M5-A frame with the sky
differs from the mesh-pass frame in 22,588 of 614,400 HDR pixels, 84 by more than 1%
and 4 by more than 5%, the worst at the floor's left silhouette where the mesh pass
shadows a pixel the traced frame does not; 613,421 displayed pixels are identical.
The two families are different algorithms at silhouettes in the wgpu renderer too
(a rasterised receiver against a ray hit), so the reference for those pixels is the
wgpu traced frame, which has not been produced yet.

Limits of this first traced frame: no motion blur, one draw per object with default
textures, no tangents, no previous-frame motion, no local-light shadow rays exercised
(no local shadows are requested), and no wgpu traced reference yet.

## Order for the engine's default paths

`Engine` selects traced primary visibility on `HardwareRt` and the visibility buffer
on `NonRt`, with GI, temporal reconstruction and reflections on. Read against
`Renderer::submit_frame`, the families split into what is already ported and only
needs assembling, and what the default image needs in addition.

**Smallest engine-path frame: built** (the traced frame above). `HardwareRt` at
`Quality::Reference` with GI, temporal reconstruction and reflections off encodes:
TLAS build, camera trace with receivers, fresh exact sun visibility, programmable
primary shading, then atmosphere, transparency and post. `Quality::Reference` selects
`SoftShadows::Exact`, which takes the fresh-visibility branch rather than the
Interactive trace. It needed no new family, only two variants of ported entries, and
both now execute:

- **Fresh visibility in its traced form** (`fresh_visibility::traced_program`, from
  `shader_sources`): the estimator of entry 46 with its coverage seam gated on the
  primary tiles and hit image. Entries 45 and 46 as first ported are the forward
  path's variant, a receiver raster prepass feeding `fresh_compute`.
- **Shading on the hardware soft head** (`primary_shade::hardware_soft_program`,
  `heads()[1]`): it reads the fresh counts and declares the scene for local-light
  shadow rays. Entries 38-39 were replayed on the non-RT hard head, where
  `hard_shadow` returns one and no scene is declared. With the standard program it
  reflects 39 inputs: the lit interface, the 256-element geometry and material
  arrays, the scene and the fresh counts.

The second one's SPIR-V emission failed at first (`resource global is shadowed
locally: "camera"`): ray-query modules are rewritten in their original text, and
`reflections.wgsl` declares `let camera = primary_ray(..)` under the `camera` global.
`shader::local_shadows` now renames such locals within their WGSL scope (parameters
over their body; `let`, `var` and `const` from the end of their statement to the end
of their block) before the rewrite, and rejects a loop-scoped shadow rather than
guess. Both variants emit SPIR-V that passes `spirv-val` for Vulkan 1.3 and the
NonUniform audit. Neither has run on a Vulkan device.

What the assembly reproduces from `Renderer::prepare_primary`:

- **Frame record, 144 bytes** (`primary_frame`): inverse view-projection at 0, the
  exposed clear RGBA (`lit_clear`, alpha one) at 64, a word at 80 set when any
  placement is single-sided, direction rotation at 84 and the visibility-filter flag
  at 88 (both zero here), visible-environment flag at 92, the scene's pixel rectangle
  from `scene_pixels` at 96, sampling `(8, 8, 0, 1.0 as bits)` at 112, and the
  reflection bands at 128 (`ReflectionSettings::off().frame_words()`).
- **Region record, 16 bytes**: that rectangle grown to whole `TRACE_TILE` (8x4) tiles.
- **Placement table**: `placement_table`, one word per instance: the draw index with
  tangent, single-sided and thin flags in bits 31, 30 and 29.
- **Geometry arrays**: one buffer of current then previous model matrices (two
  matrices per placement), one of normal transforms, and per-draw vertex, index and
  tangent buffers padded to `TRACED_DRAWS` with a 48-byte zero buffer.
- **Dispatches**: the trace over the hit image in `TRACE_TILE` groups; the estimator
  in 2x2 receiver tiles; shading over `max(region groups, clear blocks)` by
  `region rows + clear rows`.

What stands between it and a picture comparable with an engine render: post on the
traced frame (the hit-based motion surface, entry 35), miss shading with the sky, and
a wgpu traced reference to compare with.

**Then, by how much of the default image each one changes:**

1. **GI cache, hardware-ray variant** (`gi_cache.wgsl`, `gi_cells.wgsl`, `gi_api.wgsl`,
   about 2,800 lines): sky occlusion and bounce light. Without it every surface reads
   the unoccluded environment, the largest visible difference from the default image.
2. **Temporal reconstruction** (`temporal_resolve.wgsl`, 789 lines): the default
   antialiasing and the history the Interactive shadows and GI lean on. Exact shadows
   do not need it, so the picture is comparable before it, only aliased.
3. **Traced reflections** (`reflections.wgsl` and the program reflection lobe): replace
   sky specular on glossy and mirror surfaces; visible wherever a scene has them.
4. **Interactive trace and film** (`primary_interactive`, the penumbra list and trace,
   `film_accumulation`): the realtime-quality sun shadows and their repair. They
   approximate what Exact already draws, so they change cost more than the picture.
5. **The non-RT default: virtual shadow maps, the surface cache and S4**
   (`virtual_shadow*.wgsl`, `sunvis_s4.wgsl`, about 3,800 lines), then software GI and
   reflection rays. The largest family, and it only matters on devices without ray
   queries. Its unshadowed base is already ported: the visibility buffer's cull,
   rasters, HZB and receiver resolve (through entry 34) and primary shading (38-39).
   A `NonRt` frame without sun shadows is therefore also assembly work.

Decals and streamed textures are tables and uploads rather than passes; they follow
whichever scene needs them. `Engine` itself takes wgpu textures, so "an engine frame
through Ara" means a second entry point over the same scene description, as
`forward::render` is for the forward path, until the two are unified.

## Frame sequences

`forward::Frames` renders a sequence and keeps between frames what the wgpu renderer
keeps: post's frame serial and the previous unjittered camera (valid only from the
frame directly before, as in `PostGpu::prepare`), the shadow planner's light
orientations (`ShadowState::bases`), and the placement transforms a traced frame
publishes as previous matrices. `Frames::cut` forgets them. It also owns the radiance
cache (below), the first family with data on the GPU between frames; `Frames::retire`
releases it, and a cut keeps its buffers and empties the table on the next frame, as
`Renderer::reset_indirect` does. Every other GPU resource is still built per frame.
`forward::render` is a one-frame sequence.

`frames_carry_the_previous_camera_into_motion_blur` renders two frames with the
camera stepping 0.3 units sideways. The first has a still motion surface. The second
carries serial two and a shutter velocity of 1.15 pixels, which is the step's 4.61
pixels times the 180 degree shutter's quarter, and its blurred display sits inside
the scalar post replay of its own HDR and surface. After a cut the same camera gives
the same HDR, no velocity and a different display.

`ara_frame --frames N` is the comparison tool for every family with history: poses 0
to N-1 of the fixture's camera path through one renderer, on either layer, one
summary per frame in `frame.json` and the last frame's raw files and pictures.
`--compare` reports every frame's grids, then the last frame per pixel. The summary
is about 20 KB of compact JSON per frame, so a sequence travels as a file, not as a
PR comment.

```sh
cargo run --example ara_frame -- --backend wgpu --atmosphere --sun-shadows --frames 8 --output out/wgpu-seq
cargo +1.99.1 run --features backend-ara --example ara_frame -- --backend ara --atmosphere --sun-shadows --frames 8 --output out/ara-seq
cargo run --example ara_frame -- --compare out/wgpu-seq out/ara-seq
```

On this Mac a four-frame sequence with the sky and sun shadows is byte-identical
across two runs.

### Cross-layer result: the traced frame and an eight-frame sequence

Reference: wgpu at 65119b7 on the Radeon 760M (RADV). Ara: the M5 Pro on Metal under
API and shader validation at df365bc (frame code unchanged since 65119b7). M5-A at
960x640 with the physical sky.

| Run | HDR max | HDR mean | HDR pixels over 1% / 5% | Display identical | Display worst |
|---|---:|---:|---:|---:|---:|
| Traced frame (`--traced`) | 3.43% | 0.050% | 2 / 0 | 91.0% | 1 level |
| Mesh pass with sun shadows, frame 8 of 8 | 54% | 0.051% | 16 / 1 | 90.3% | 24 levels |

The traced frames agree across the two layers to within a ray or two of 256 in the
penumbra: the worst pixel, (150, 368), is 0.0542 against 0.0561 with identical
neighbours. So the handful of silhouette pixels where the traced frame and the mesh
pass differ on one layer are the two algorithms' difference, not the layer's.

In the sequence every one of the eight frames' grid means agrees within 0.1% in HDR
and one display level, which covers motion blur with a real previous camera for the
first time. The last frame has one pixel far out, (152, 332): it lies on the edge
between a panel and the lit floor, and the two GPUs' rasterisers put that pixel
centre on different sides (0.527 on Vulkan, 0.293 on Metal; the neighbours above and
below match). Three displayed pixels differ by more than one level, all at that
edge. That is a rasterisation difference between Metal and Vulkan at a triangle
edge, which the mesh pass will always have and the traced frame does not.

## GI cache: scope before porting

The world radiance cache (`gi_cache.rs`, 3,227 lines; `gi_cache.wgsl`, `gi_cells.wgsl`,
`gi_api.wgsl` and `gi_shade_entry.wgsl`, 2,948 lines) is the first family the default
image needs that had no ported entry. This section is the scope as read from the
source before porting; what is built since is in the two subsections after it.

**Thirteen compute entries, in `GiState::encode` order:** `gi_probe` (visible pixels
request cells), `gi_gather` (bounce), three rounds of `gi_claim` and `gi_resolve` with
`gi_resolve_last` closing the third (inserts are `atomicMax` claims the winner
writes), `gi_scan` (the fixed refresh rotation), `gi_setup`, `gi_rays` and `gi_finish`
(up to 2^21 visibility rays a frame, traced in parallel), `gi_integrate` (the occluded
sky as L2 SH), `gi_filter` (cell-space filter), then `gi_light` and `gi_shade_cells`
(bounce: light visibility, and the material program shading cells through its own
entry, so this family also goes through the program composer). Shading reads the
result through `indirect_irradiance` and lighting binding 20, which the frames above
already bind as the header-only "off" table.

**What makes it larger than a pass count suggests:**

- **It is stateful across frames.** Cells persist, refresh on a rotation and evict;
  after a cut the cache fills over several frames. A first frame alone shows partly
  filled GI, so the comparison with wgpu has to be a frame sequence with the same
  per-frame plan on both layers. `forward::render` is one-shot; this family needs the
  frame code to keep resources and history between frames first.
- **The CPU plan is large.** `prepare_gi` (from line 2125) decides each frame's
  groups, scan, clear, integrate, bounce and filter, publishes a 1,104-byte frame
  record with up to 32 boxes, and maintains request, counter and miss words. That
  plan has to be shared with the wgpu renderer as a function, as the shadow planner
  now is, not re-derived.
- **Two ray sources.** Hardware rays are the variant to port first; the software
  variant (`gi_rays_software`, `gi_gather_software`, the CPU BVH in `gi_bvh.rs`)
  belongs with the non-RT default.

**Smallest comparable configuration:** `IndirectSettings` with `bounce` off and the
filter bits clear is sky visibility only (the original G1): probe, claim, resolve,
scan, setup, rays, finish and integrate, nine entries, with no program-composed
shading entry. That is the first target: stage replays for those nine, then a
persistent frame, then a fixed-length sequence compared with wgpu. Bounce (gather,
light, cell shading) and the filter follow.

### The nine sky-visibility kernels through the adapter

`gi_cache::Entry::program` composes `gi_cache::kernel_source` for hardware rays and
selects one entry. All nine reflect, emit MSL and SPIR-V that passes `spirv-val` for
Vulkan 1.3 with the NonUniform audit, and become native Metal pipelines on a
ray-query device (`sky_visibility_kernels_reflect_validate_and_compile`). Every one
takes the table, the work words and the frame record; the probe adds the primary hit
image, tile flags and the placement geometry, setup and rays add the scene, finish
and integrate add the lighting uniform and the environment cubes. Two adapter gaps
closed on the way:

- **`@builtin(num_workgroups)` on Metal.** Seven of the entries take it; the MSL
  entry wrapper now forwards `[[threadgroups_per_grid]]`.
- **Hit vertex positions.** The hardware ray source declares its scene and queries
  with `vertex_return` and reads the committed hit's vertex positions once, in
  `gi_closest`. As in the camera trace, the qualifier is dropped and that one
  intrinsic becomes a fetch from the module's own per-draw vertex and index arrays
  through the hit's placement; seam counts are checked so a source change fails here.

That test replays nothing; the replays are in the next subsection.

### Sky visibility in frame sequences

**The plan is Renderlab's own.** `Renderer::prepare_gi` now plans through
`gi_cache::Planner` (`begin` gives the table header, `finish` the frame record and
the plan, `advance` counts the frame), and sizes its buffers and packs its draw table
through `Sizes::of` and `draw_table`. The backend calls the same functions, so the
header, the 1,104-byte frame record, the idle shortcut and the pass list cannot drift.
Those two extractions are their own PRs on Renderlab main and are merged into this
branch; the dispatch grids and clears in `Cache::record` follow `GiState::encode` by
hand.

**The cache.** `gi_cache::Cache` owns the table, the work words, the outgoing radiance
and the filter sources (the last two allocated but unused), device-local and zeroed on
their first frame, plus the planner and the nine kernels. `ForwardScene::indirect`
asks for it on a traced frame; `forward::Frames` allocates it on the first such frame
and keeps it. Each frame writes the header, uploads the frame record, the draw table
and the placement matrices, runs the plan's passes between the fresh estimator and
shading (probe, three claim and resolve rounds, scan, setup, rays, finish, integrate,
each only when the plan has it), and binds the table to shading at lighting binding
20. Refused for now: software rays, and settings or a light count that change within
a sequence. Bounce and the cell filter are described below.

**Replays** (`tests/ara_backend/cases/gi_cache.rs`, reference in
`gi_cache_reference.rs`), on a 12 m floor with a 12 m wall under a uniform sky:

| Test | What it holds the cache to |
|---|---|
| `sky_visibility_cells_hold_exact_ray_bits_and_their_sh` | All 668 cells traced from their own query point: each of their 42,749 testable rays (64 a cell, less those within 2 mm of an edge) against the scene in f64, bit for bit. Every valid cell's stored SH against the sum over its open directions: scale within 1.4e-7, halves within 2.4e-4. |
| `sky_visibility_darkens_the_floor_by_the_walls_share_of_the_sky` | 132 floor pixels: frame with the cache over frame without against one minus the wall's form factor in closed form, from 0.52 at the wall's foot up. Worst error 0.033, bound 0.06 (64 directions, 4 px cells, an L2 fit of a hard edge). |
| `cache_sequences_repeat_idle_when_still_and_follow_the_camera` | Two runs of an 11-frame still sequence give the same table words. Still frames change only the frame stamp and seen stamps, the ninth only the header stamp, and from then on nothing (the planner's idle shortcut). Six frames of a moving camera claim cells every frame and keep exact bits. |

Shifting the reference's rotation by 0.05 rad or one SH constant by 2% fails the first
test, so it is not vacuous. The suite is 55 tests under Metal API and shader
validation.

**The comparison tool.** `ara_frame --gi` (implies `--traced`) turns sky visibility
on for both layers with a fixed 2^17-slot table, writes each frame's whole table as
`table-NNN.u32` (25 MB a frame, so they stay local) and puts the slot count, counts
and digests of 256 chunks in `frame.json`. `--still` holds pose 0. `--compare` reports
the slot count, then per frame the first differing word and what it holds, and the
cells by key: present on one side only, differing in direction bits, differing in
stored SH. A still sequence idles from its ninth frame, so the moving sequence is the
one that keeps comparing.

**The acceptance rule.** Two layers' tables are not expected to be bit-identical: hit
positions and ray directions differ by rounding between shader compilers and drivers,
which now and then moves a pixel across a cell or level boundary or flips a ray that
grazes an edge. `--compare` ends with one line, `cache tables: PASS` or `FAIL`, and
exits nonzero on a fail. A frame passes when all six hold; a comparison passes when
every frame does.

| | Criterion | Bound | What a miss would mean |
|---|---|---|---|
| 1 | Header: integer words equal (the frame stamp among them), float words within 2 ulp | exact | The two layers planned different frames |
| 2 | Cells present, or valid, on one side only, of the larger side's cells | 0.5% | A stage is missing cells, not boundary pixels |
| 3 | Differing direction bits over cells valid on both sides, of their rays | 0.1% | A wrong ray origin or basis, not edge grazers |
| 4 | Every SH coefficient of cells valid on both sides with equal bits, of the cell's scale | 1% | A wrong weight, not f16 storage or a sampled environment |
| 5 | Of those cells, the ones with a bounce coefficient further than 2% of its scale apart | 1% of them | A wrong gather or cell shade, not one boundary cell or grazing ray moving a neighbour by a ray's share |
| 6 | Of those cells, the ones whose split flag differs | 10% of the larger side's split cells, or 2 | A refine that does not fire, not a cell at its threshold |

The bounds were chosen before any cross-layer run existed, from what each criterion
should absorb; every frame's line prints the measured figure beside its bound, so the
first gate run shows how much room there is and the bounds can be tightened to it.
Without both raw tables equal digests pass and anything else is `NO VERDICT`, also a
nonzero exit. Perturbing one frame of a Metal run confirms each path: one flipped bit
passes and is reported, 200 dropped cells, a changed header word and a 5% SH scale
each fail with their reason.

```sh
cargo run --example ara_frame -- --backend wgpu --gi --atmosphere --frames 12 --output out/wgpu-gi
cargo +1.99.1 run --features backend-ara --example ara_frame -- --backend ara --gi --atmosphere --frames 12 --output out/ara-gi
cargo run --example ara_frame -- --compare out/wgpu-gi out/ara-gi
```

On Metal, M5-A at 960x640 with the physical sky: two runs of the 12-frame moving
sequence give identical tables and pixels in every frame (14,516 cells after the
first frame, 15,642 after the twelfth); the still sequence holds 14,516 cells and its
stamp stops at 8. **Not compared with wgpu yet**: the reference has to come from the
Linux gate, which can also run both layers on the same GPU, where the raw tables do
not have to travel.

**Bounce.** With `IndirectSettings::bounce` the frame adds `GiState::encode`'s bounce
passes: the gather after the probe and before the claim rounds (it reads last frame's
outgoing radiance and may request the cells its rays hit), then light visibility and
cell shading after the integration. Cell shading is the standard program composed at
the cell stage on the hardware soft head, as `gi_shade_pipeline` composes it; its
group 0 is the cache's (work words, outgoing radiance, draw table) and its other
groups are the traced shading packet's. The planner gets the lighting key (lights,
environment, program) and so schedules relights as on wgpu.

`bounce_cells_shade_and_gather_what_the_scene_predicts` runs the yard with a 1,000 lux
sun straight down, a black sky, a 0.8 floor and a 0.2 wall, and judges frame 8 of a
still sequence, by which the values have stopped changing:

| Stage | Held to | Result |
|---|---|---|
| Cell shading | Outgoing radiance times pi over incident light (the sun where the cell faces it, plus its own gathered bounce) is one number per material | 0.81231 on all floor cells and 0.21231 on all wall cells, each constant to 3e-7: the base colour plus the rough dielectric's 0.0123 |
| Gather | Each wall cell's four L1 coefficients equal the floor's radiance times the kernel's quadrature over exactly that cell's blocked directions | 1,413 cells, worst at 0.82 of a bound set by the floor's own 3.5% spread; a 5% weight error fails |
| Shading | Wall pixels are black without bounce (the sun grazes the wall) and show their cell's radiance with it | 108 pixels at 0.98 to 1.16 of the nearest cell's radiance |

The closed-form irradiance `pi L F` matches only near the floor: high on the wall the
stored value is up to 50% above it, because an L1 fit cannot tell grazing light from
light along the normal. That is the representation Renderlab chose, not a port
defect, which is why the gather is held to its own quadrature instead.

`ara_frame --bounce` (implies `--gi`) is that level on either layer; on Metal two
runs of the 12-frame moving sequence are identical in every table word and pixel
(14,516 cells after the first frame, 23,613 after the twelfth, the gathers' hit cells
included).

**The cell filter.** With bounce on and filter bits set, `gi_filter` runs once every
value of the frame is written and before cell shading reads them; the sources buffer
(22 words a slot) is then table-sized. Renderlab's default settings (bounce, filter
bits 15) are now a configuration the backend runs.
`filtered_cells_are_the_weighted_mean_of_their_unfiltered_neighbourhood` reads the
sources back and recomputes `gi_filter_cell` on the CPU for every valid cell in each
of six frames: the tent-, normal- and plane-weighted mean of the cell's own value and
its tangent neighbours', found by level, bin and lattice coordinates from the cold
records, for the sky SH and for the bounce SH. In the last frame all 2,495 valid cells
agree to f16 precision, with 6.9 neighbours a cell; the filter moves 2,465 of them,
so agreeing with it is not agreeing with the unfiltered value.

**Contrast refine** (filter bit 2) does not fire on the yard or, as far as the cell
counts show, on M5-A, so it has its own scene:
`contrast_refine_splits_the_cells_of_a_slot_as_its_rule_predicts` stands two 1.5 m
walls 7 cm apart on the floor and looks into the slot from its open end. A floor cell
whose query point lies in the slot sees almost no sky, the cells beside it outside the
walls see most of it, and the filter marks it split so that its pixels read the next
finer level. In each of five frames the split flag of every valid cell equals the
rule recomputed on the CPU from the unfiltered sources (under a quarter of an
agreeing neighbour's openness, that neighbour above a quarter): 12 cells in the first
frame, 36 once settled, told apart from the 21 flags the probe sets itself on the two
facing walls. The finer cells the refine asks for appear on the slot's floor from the
second frame. Lowering the reference's threshold to a tenth gives mismatches, so the
agreement is not vacuous. **Still not modelled by the reference:** with an
environment map the openness is measured against the diffuse cube; the test's sky is
uniform.

`ara_frame --crevice` adds such a slot to M5-A (105 split cells by the twelfth frame
on Metal), and the verdict has a sixth criterion: split flags may differ on at most
10% of the larger side's split cells, or two cells. A copy of a Metal run with its
split flags cleared fails it.

`ara_frame --filter` (implies `--bounce`) is the cache at Renderlab's defaults but for
the 2^17-slot table; on Metal two runs of the 12-frame moving sequence are identical
in every table word and pixel.

One adapter defect surfaced with these kernels: the SPIR-V path's textual rewrite of
resource names decided "struct member" by looking back over comments, so a table
write that followed a comment ending in a period kept its original binding. The
heap-binding check refused the module, so it was loud, never silent; both textual
passes now look back over code only, and the rename regression covers it.

## Temporal reconstruction: scope before porting

Temporal reconstruction (TR1; `temporal_reconstruction.rs`, 1,693 lines, and
`temporal_resolve.wgsl`, 792) is the next family the default image needs. Read from
the source at 3cc9384; only the last paragraph is built.

**What it adds to a traced frame.** The camera is jittered by a fixed 256-entry
sub-pixel sequence, through the projection only, and every pass of the frame sees the
jittered camera except post and the cache's still key. After shading, the shaded HDR
is copied aside and one kernel, `tr1_resolve` (8x8 groups over the output), reprojects
three history images, blends, and writes the reconstructed image back into the HDR
target in place. Shading already writes a view hint into HDR alpha for it, on or off.
On Interactive frames the fresh shadow kernels are also swapped for a variant whose
disk directions rotate with the sample index and whose ray count is a per-frame
budget (256 cold, 64 after a reset, 32 moving, 8 once still for 16 frames); that
variant is two string edits on the production module, not a new file.

**State across frames.** Two each of history colour (RGBA32F: exposed mean and
weight), anchor (RG32U: placement, side, distance) and silhouette layer (RGBA32U), at
the output size, ping-ponged by a parity bit; the previous frame's camera, eye,
exposure, sun directions, light and scene key, skin revisions; the sample index; a
still counter. History continues only when the previous reconstructed frame is the
one directly before.

**The CPU plan.** `prepare_reconstruction` is about 370 lines, of which about 300 are
arithmetic on the camera, the lights, the draws' bounds and motion, the settings and
the previous frame: it produces the 448-byte `Tr1Frame` record, sixteen bytes of the
primary frame record (the ray budget) and the next history. The rest allocates,
writes and binds. That pure part is the function both layers must share, as the
cache's planner is. Its one awkward input is the reset key, a digest of renderer
internals (generations, upload counters, programs); the backend needs only that it
changes when the same things change.

**What makes it harder than one kernel suggests:**

- **Stillness is decided by bit equality.** `still` compares the unjittered camera
  matrix's bits with the previous frame's, and rigid motion compares model matrices
  the same way. A layer whose matrices differ in the last bit never settles.
- **It reads four other families' outputs**: the trace's hit records and tiles, the
  primary frame record with the jittered inverse, the placement geometry with
  previous models, and shading's alpha.
- **Its sample index leaks.** The rotation word also seeds glossy reflection rays and
  the light clusters' source sampling.
- **Render scale.** Below native scale every pass renders at an internal size and
  the resolve writes the caller's size; transparency is then skipped.

**Order.** (1) The resolve on the frame the backend has, `HardwareRt` at
`Quality::Reference` with the default settings at native scale: the jitter, the copy
and `tr1_resolve`, with the history in `forward::Frames` and the plan shared. The ray
budget is computed and nothing consumes it. (2) Interactive shadows with the rotated
kernels and the budget. (3) Render scale.

**Step 1 is built.** `tr1_resolve` composed as `resolve_source` composes it reflects
its 17 inputs, emits MSL and SPIR-V that validates, and compiles natively
(`temporal_resolve_reflects_validates_and_compiles`); it needed no adapter change.

The plan is Renderlab's own: `prepare_reconstruction`'s device-free part is
`temporal_reconstruction::Planner::prepare` (its own PR on Renderlab main, merged
into this branch), which the backend calls with the camera, lights, scene motion and
settings. `temporal::History` owns the three history image pairs, the planner and the
sample index, and lives in `forward::Frames` beside the cache. With
`ForwardScene::reconstruction` a traced frame is drawn through the camera jittered by
its sample of the sequence (post and the cache's still key keep it unjittered), the
primary frame record carries the rotation word and the planner's sampling words, and
after shading the HDR target is copied aside and resolved in place. A cut restarts the
history and keeps the sample index, as `cut_reconstruction` does. Refused: settings
or a size that change within a sequence. The ray
budget is planned but nothing consumes it until Interactive shadows are assembled.

Three tests on a bright card over a dark floor, seen from straight above:

| Test | What it holds the resolve to |
|---|---|
| `reconstruction_accumulates_jittered_samples_into_edge_coverage` | The first frame restarts at weight one and equals the unreconstructed frame. Each later frame adds one sample to the mean weight, within 1%, and leaves smooth regions at the plain frame's value within 0.1%. After 48 frames the pixel the card's left edge crosses holds 0.290 of the card's radiance where the card covers 0.290 of its area, and the top-edge pixel 0.749 where it covers 0.753: 48 jittered point samples have become coverage. Bound 0.03. |
| `reconstruction_history_survives_a_pan_and_restarts_on_cuts_and_sun_turns` | A camera panning half a pixel a frame never restarts and the floor's weight keeps growing through reprojection. A cut and a 0.2 rad sun turn each restart every pixel, with weights back within the spatial start's window, and the frame after a restart adds one sample again. Two runs of the pan give identical images and history. |
| `reconstruction_keeps_history_under_a_climbing_camera_and_a_slow_sun` | Under the physical sky a camera climbing half a pixel a frame from 2 m never restarts, though every frame changes what the procedural sky's content is keyed on. A 0.01 rad sun turn, under the 0.064 rad turn angle, does not restart either and holds every weight to `turn_angle / turn` = 6.4 samples. Fails on the key the port had before (restart on climb frame 1; restart on the turn). |

**The restart key, and what the first comparison with wgpu found.** History restarts
when `reconstruction_key` changes. The renderer's key holds what its planner does not
follow: a sun by its lux alone (a turn caps the window), a local light without its
candela, the environment by its uniform and whether the sky shows, the programs,
geometry and materials. The port's first key was written from a summary of that and got
two things wrong: it hashed the suns' directions, so any sun turn restarted, and it
hashed the digest the cache relights on, which includes the eye's altitude under the
physical sky. On M5-A's camera path the eye only moves in x, but the eye recovered
from the inverse view matrix changes in its last bits from pose 3, so from pose 3 the
port restarted every pixel on every frame while the wgpu renderer kept accumulating.
The gate's first wgpu-against-ara run showed it as poses 0 to 2 within 0.07% and pose
3 on up to 33% apart in one HDR cell (the mirror sphere's highlight; 7% of pixels over
1%, mostly one-pixel silhouette edges), only with reconstruction on, and not on a
still camera. The cache tables passed in every frame throughout. The key now mirrors
the renderer's field by field, with the atmosphere's settings standing for
`set_atmosphere`'s cut; the rotation word honours `rotate_directions`.

`ara_frame` now records, per frame, how each layer made it: on ara the planner's
restart flag and shadow ray budget, on wgpu `reconstruction_shadow_rays`, the frame's
uploads by the categories the key hashes, and the revision. `--compare` prints both
beside each pose, so a restart on one side reads directly (256 rays cold, 64 on a
restart, 32 moving, 8 held).

**Step 2 is built: Interactive shadows.** The fused Interactive trace (the camera hit
and eight sun directions per pixel in one dispatch, near-edge pixels listed for an
indirectly dispatched repair) has three entries, `primary_trace_interactive_deferred`,
`primary_trace_interactive` and `primary_repair_interactive`. Composed from
`shader_sources`' soft module with the portable trace and the renderer's
eight-direction override, and again through `trace_source`'s two edits (rotated
directions, a ray count read from the frame record), all six reflect, emit valid
SPIR-V and compile natively (`interactive_trace_entries_reflect_validate_and_compile`).

`Primary::TracedInteractive` is the traced frame with them: the deferred trace and its
repair, dispatched from the queue's own counts, replace the camera trace and the
256-ray estimator, as `HardwareRt` draws at its realtime quality. With reconstruction
the rotated kernels run and read the planner's budget from the frame record.
`interactive_frame_is_the_exact_frame_outside_the_penumbra_and_on_a_cold_start` holds
it to the exact frame under a slanted sun with a wide disk:

- Without reconstruction, 12,101 of 12,288 pixels are bit-identical to the exact
  frame. The 187 that differ all lie where the exact frame is partly shadowed, and
  each holds a whole number of eighths of the sun's light (within 0.04 of an eighth).
  That also holds the fused trace's hits, repaired near triangle edges, to the exact
  trace's.
- With reconstruction, the first frame is the exact frame in every pixel: the cold
  budget is 256 rays at rotation zero, so the rotated kernels, the budget words and
  the repair reproduce the trace and gated estimator they replace. Later frames, on
  64, then 32, then 8 rotated rays, differ from the exact sequence on at most 244
  penumbra pixels by at most 0.05 of the shadow's contrast.

`ara_frame --interactive` is the level; with `--reconstruction --filter` it is the
closest frame so far to `HardwareRt` at its defaults. Not assembled: the path for a
repair queue above device limits (`Full` compiles and is never chosen).

`ara_frame --reconstruction` is the level on either layer, combinable with the cache
levels. On Metal, M5-A with the cache at its defaults and reconstruction on is
identical across two 16-frame moving runs. The first comparison with wgpu found the
restart key above. At the corrected key (520b2bb, both layers on the Radeon 760M, 16
moving frames) the two agree: worst HDR cell mean 0.06% at poses 0 to 4 and 0.03% at
pose 15, worst display channel 6 levels on 6 pixels, at this level and with
Interactive shadows, reflections and the textured floor added; both show 256 shadow
rays at pose 0 and 32 after, and neither restarts after pose 0. What neither the
tests nor that comparison sees is the resolve's rejection rules under object motion:
the path moves only the camera.

**Render scale below native.** `Reconstruction::scale` at `Quality` or `Performance`
draws the frame at two thirds or half the target in each axis. As
`Renderer::upscale_target` does it, every pass before the resolve (the trace, the
estimator, the cache's shading, the atmosphere) runs at the internal size, the copy of
the shaded samples is the internal size, and the three history pairs and the resolve's
output are the target's: a separate target-size HDR image that post and the caller
then see. The planner takes its matrices at the internal size and one output pixel's
angle; post's camera record is at the internal size and every post target at the
caller's, with the hit-based surface pass reading the internal hit image as its shader
already does. The resolve shader needed nothing: it derives the ratio from the two
image sizes.

`reconstruction_below_native_scale_fills_the_target_from_fewer_samples`, on the card
under a still camera, at both scales: the frame, its history and the displayed image
are the target's size; away from the edges every frame equals the native frame; once
the history is no longer young the mean weight grows each frame by the share of target
pixels a frame samples (0.248 a frame for 0.250 at half scale, 0.440 for 0.443 at two
thirds); after 64 frames the pixel the card's edge crosses holds 0.324 and 0.343 of the
card where it covers 0.290 (bound 0.1), and pixels two away are pure.

`ara_frame --scale quality|performance` is the level on either layer. Below native
scale the wgpu renderer's `try_capture_hdr` returns its internal-size shaded samples,
not the resolved image, and exposes no capture of the latter; the tool therefore
writes those samples as the HDR on both layers (`ForwardImages::samples` on this one)
and the displayed image is the comparison of the resolved frame. Compared once with
wgpu on the Radeon at 80acbfd, with the gate upscaling wgpu's HDR by hand: cache PASS at
both scales, worst display channel 11 levels at two thirds and 6 at half, at most 10
pixels over one level. **The HDR samples have not been compared.** What the test cannot see: anything under motion (the rejection tolerance
uses the output pixel's angle, and the interpolation of young history), and the
quality of the upscaled edge beyond that one bound. As in the renderer, an upscaled
frame would skip transparent draws; the frame has none yet.

## Traced reflections

On a hardware-ray traced frame reflections are not a pass. `program::compose` appends
`reflections.wgsl` to the traced shading module of any program that declares
`reflection_lobe` and reads `specular_incident`, on a head that declares a scene; the
standard program on the hardware soft head is one. So the shading kernel the backend
already built and ran contains the reflection rays (one mirror ray below roughness
0.05, four to sixteen strata over the declared lobe above it, a hit shaded by the same
program with its own shadow ray, a miss reading the sky), and they were inert only
because the primary frame record carried `ReflectionSettings::off()`'s words. There is
no reflection state between frames on this path; under reconstruction the strata turn
with the rotation word the frame record already carries.

`ForwardScene::reflections` writes the settings' words into the frame record. Three
checks, on a metal floor:

| Test | What it holds reflections to |
|---|---|
| `a_mirror_floor_shows_the_wall_at_the_metals_reflectance` | In front of a sunlit matte wall under a black sky: with reflections off the floor is black. With them on, 224 floor pixels whose mirror ray meets the wall show the wall's radiance (read from the pixels that see it directly) times the metal's reflectance toward the camera, within 0.003 for a white metal and for a grey one against Schlick's fraction; 64 pixels whose ray passes the wall's top see the black sky; the wall, too rough to trace, is unchanged. At roughness 0.2 the same pixels, aimed at the wall's middle, hold 0.90 to 1.00 of its radiance. |
| `a_uniform_sky_reflects_as_the_untraced_estimate_in_every_band` | A lone floor under a uniform sky looks the same with reflections on as off, exactly, at roughness 0.02, 0.2 and 0.45: every ray misses and returns what the untraced estimate assumes. |

`ara_frame --reflections` is the level. With `--interactive --reconstruction --filter`
it is `HardwareRt` at its defaults but for the items below; on Metal two 16-frame
moving runs of that are identical in every table word and pixel.

**Not compared with wgpu**, like the levels above it.

**Motion blur on the traced frame.** Post's motion surface on a traced frame is the
hit-based entry (35), already replayed alone: it takes each pixel's hit record, the
placement's current and previous model matrix and the post frame record, and needs no
depth target. The frame now chains it before the tile maximum and the blur, with the
same record the mesh pass's surface takes (the frame's camera: unjittered for the
current matrix, the frame's own for the inverse).
`traced_frames_carry_camera_and_placement_motion_into_motion_blur` gets 1.153 px of
shutter velocity for a sideways camera step, which is the closed form the mesh-pass
test uses, and the same 1.153 px for a still camera over a floor that slides by that
distance, which a depth target cannot know; still, settled and cut frames have none,
and each blurred display sits inside the scalar post replay of its own HDR and
surface. `ara_frame`'s traced levels now run post at its defaults.

**Material textures, on both frame families.** `ForwardScene::textures` gives each
object a base-colour and a metallic-roughness slot as asset data with every mip level
and an asset sampler, the terms `try_upload_texture` and `TextureBinding` take. The
frame uploads each level, maps the sampler as `material::sampler_descriptor` does
(wrap, filters, anisotropy on fully linear samplers), and binds the object's own
image and sampler: per object on the mesh pass, per draw in the traced frame's
256-element image and sampler arrays, where a draw without a texture keeps the 1x1
default. Material bytes need no change: only normal maps carry a flag there.

| Test | What it holds textures to |
|---|---|
| `a_textured_floor_is_quadrant_by_quadrant_the_untextured_floor_of_that_texel` | A 2x2 base-colour and a 2x2 metallic-roughness texture, sampled nearest: each quadrant of the floor equals the untextured floor whose factors are that texel's values (colour decoded from sRGB, roughness from green, metalness from blue), within 0.15%, on the mesh pass and on the traced frame. |
| `a_mip_chain_shows_the_level_the_pixel_footprint_selects` | A chain of flat, distinct levels tiled to about four texels a pixel from 4 m and two from 2 m, with nearest mip selection: the floor is the untextured floor of level two's colour from 4 m and level one's from 2 m, within 0.15%. The mesh pass selects from screen derivatives, the traced frame from each pixel's ray footprint; both land on the same level. |

`ara_frame --textures` gives M5-A's floor a 64-texel checker with a box-filtered mip
chain, trilinear at anisotropy 16, on either layer.

**Not assembled:** normal maps (the frame's meshes carry no tangents) and
`MipmapFilter::None` on a mip chain, which the renderer implements with a sampler LOD
clamp that ara-gpu's samplers do not have; the frame refuses both. Streamed
(partially resident) textures are the renderer's own residency system and are out of
scope here.

**What still separates this frame from `HardwareRt`'s default image** on an opaque
scene: only the smaller table the tool uses for the cache. Depth of field was listed
here and is not part of the default (`PostSettings::default()` has it off). Local-light
shadows, normal maps, render scale below native and depth of field are built since
(below, and under temporal reconstruction). Not assembled: transparent draws and
particles on the frame, and on the mesh pass local-light shadow maps and the tangent
vertex stream.

**Depth of field on the frame.** `PostSettings::depth_of_field` adds the ported
`DepthOfField` kernel to the post tail in `PostGpu::encode`'s place, after motion
blur and before bloom, reading the surface pass's depth (which now runs for either
setting) and what motion blur left, or the frame when motion blur is off.
`depth_of_field_blurs_by_the_thin_lens_radius_and_keeps_the_focused_edge` holds it to
the lens: a card 3 m away over a floor 4 m away, through a 100 mm f/0.5 lens. Focused
on the floor, pixels further from the card's edge than the thin-lens radius for 3 m
(3.42 px) are the sharp frame's bytes and pixels well inside it lie between the floor
and the card; focused on the card, every card pixel is the sharp frame's and the flat
floor does not change away from the edge. The mesh pass (surface from the depth
target) and the traced frame (surface from the hit records) display identical images
in every case. `ara_frame --depth-of-field` is the level on either layer and either
family. What the test cannot see: the kernel's weights inside the blur, and depth of
field together with motion blur or an upscaled frame. Out of the blur's reach the test
allows one display level, with nine pixels in ten exact: a pixel there is still a
weighted mean of up to 48 taps of its own flat surface, and an RTX 4070 SUPER showed
192 for 193 at one pixel 5.8 px inside the card, where no floor tap can reach. A leak
would move many pixels, and by more.

## Local-light shadows on the traced frame

On ray-traced shading modules the shadow rays of point and spot lights are code the
shading program already carries (`local_light_ray.wgsl`, `rl_local_shadow`): one ray
toward a light with no source radius, four over a disk for one with a radius. The
renderer switches them on per light. With `LocalShadows::count` above zero on a frame
whose shading traces them, `prepare_light_clusters` writes each local light's own index
plus one into its record's last word, and shading traces where that word is not zero;
no map slot is involved. `ForwardScene::local_shadows` does the same on traced frames,
and writes the cluster frame's sample word (the reconstruction's sample index plus
one, zero without it), which turns the four disk samples from frame to frame. The mesh
pass refuses the setting: its shadow maps (atlas, slots, caching) are not assembled.

`a_lamp_behind_a_card_shadows_the_floor_where_the_card_blocks_it` holds it to one
occluder: a card 1 m over a floor, seen from above, with a lamp up and to one side.

- No source radius: each floor pixel is decided by whether the segment from its floor
  point to the lamp passes through the card. All 836 pixels it does are the sun-only
  frame within 0.1%; all 10,428 it does not, and the 900 on the card's own top, are
  the unshadowed frame bit for bit.
- A 15 cm source: beyond the source's reach of the edge nothing changes, and within it
  every pixel holds a whole number of quarters of the lamp's light (264 hold one,
  two or three).
- Under reconstruction the cold first frame is the plain frame outside the penumbra,
  and inside it 246 pixels hold a different number of quarters: other points of the
  disk were asked. With the sample word left at zero none differ, and the test fails.

`ara_frame --local-shadows` is the level on either layer. On Metal it darkens 20,699
of M5-A's 614,400 pixels, by up to 15%, and brightens none. **Not compared with wgpu.**
What the test cannot see: a spot light's cone against its shadow, a light with a
range, and more lights than one.

## Normal maps on the traced frame

Traced shading already carries the tangent path: `program_traced_entry.wgsl` builds a
hit's tangent frame through the prelude's own `vs_tangent` when the placement word's
bit 31 says the draw has a tangent stream, reading one vec4 per vertex from the
tangent buffer array at (1, 4). What a frame has to supply is the stream, the bit,
the map and the material's flag. `ObjectTextures::normal` takes a `NormalMap`: the
texture (`TextureRole::NormalLinear`, full mip chain, its own sampler) and the
`TangentStream` it is authored against, checked by Renderlab's own
`validate_tangents`. The frame uploads the stream as the draw's tangent buffer, sets
bit 31 through `placement_table`, publishes the map as a one-layer array view (the
`normal_map` binding is a `texture_2d_array`) at (2, 15) with its sampler at (2, 16),
and sets `normal_mapped` in the material record, which `MaterialDescriptor::bytes`
only sets from a wgpu texture binding. `normal_scale` is the material's.

`a_normal_mapped_floor_is_the_floor_tilted_to_that_normal` holds it to geometry. A
flat floor with a one-texel map has the same shading normal at every pixel: the
texel's normal carried through the tangent, the bitangent `handedness * cross(n, t)`
and the surface normal. The same floor with no map, turned about the origin until its
real normal is that vector, has the same normal, the same view ray per pixel and the
same sun, so the two images must agree. Five cases: a tilt along the tangent, along
the bitangent at either handedness, with the tangent stream along y, and at half
`normal_scale`. Three agree with their tilted floor to under 0.001% at every pixel
and two within 0.08% (bound 0.5%); each is 4.7% to 12% from the flat floor on average, and
the two handednesses are 16% apart. With the tangent buffers left unbound the
bitangent case is 21% off and the test fails. The mesh pass refuses a normal map, and
a tangent that is not perpendicular to its normal is refused.

`ara_frame --normal-map` gives M5-A's floor a 64-texel stud map with its mip chain
and a tangent stream, on either layer; on Metal it changes 135,814 of 614,400 pixels,
99,712 by more than 5%. **Not compared with wgpu.** Not assembled: the mesh pass's
tangent vertex stream (`vs_tangent` and a third vertex buffer), and the variance and
slope-moment normal roles (two-layer maps and specular antialiasing from them). What
the test cannot see: mip selection and filtering of the map (one texel), and a
tangent frame that varies across a mesh.

## Cube-array environment lookups (Renderlab main's P-SKY)

Renderlab main turned the three environment maps into `texture_cube_array<f32>`
sampled with a filtering sampler (lighting binding 11), in the prelude every lit
program includes. ara-gpu had no cube views, so this needed
[ara#45](https://github.com/Ara-Engine/ara/pull/45) (`ViewDimension::CubeArray`);
it is merged, and this branch is pinned to Ara main.

In the backend: reflection maps an arrayed cube image to that view dimension; the MSL
path reads it through a `texturecube_array` table entry and the SPIR-V path declares
`texture_cube_array<f32>` in its heap arrays; the procedural environment chain is
published as one cube of a cube array while its levels are still written through
2D-array storage views of the same layers; an absent environment binds one black
cube; and the environment sampler is bilinear with nearest mips.

With main merged, all 59 tests pass under Metal validation, the atmosphere frame's
sky-lit checks included, and M5-A at the cache's default level differs from the same
frame before the merge by at most 0.10% in HDR (mean 0.0003%) and one display level:
hardware cube filtering against the manual bilinear lookup it replaced. **Not run:**
the Vulkan side of cube arrays, in ara-gpu or here; its first run is this suite on
the gate.

## First Vulkan run (Radeon 760M, RADV, validation on)

At 65119b7 the Linux gate ran the suite on Vulkan for the first time: 38 passed and
12 failed under Khronos and synchronization validation; with validation off, 48
passed and the forward frame matched the wgpu reference within one display level.
What the twelve were, and where each stands:

| Failures | Cause | State |
|---:|---|---|
| 9, and `ara_frame --backend ara` | The per-vertex fragment stages declare `FragmentBarycentricKHR`; ara-gpu never enabled `VK_KHR_fragment_shader_barycentric`. | Fixed in ara-gpu (Ara PR #34), which this branch now pins. The cache refuses those families where `DeviceInfo::fragment_barycentrics` is false. Not rerun. |
| 2 | The traced fresh module carried the camera trace text, whose vertex-return types put `SPV_KHR_ray_tracing_position_fetch` in its SPIR-V. | Fixed here: it takes the portable trace. All 94 stages the suite emits are scanned; none declares the extension. Not rerun. |
| 1 | Rendering to layer 0 of an array attached the image's whole view, so layer 1 needed the attachment layout too (coloured transmission layers). | Fixed in ara-gpu (same PR), with a regression test that passes on Metal. Not rerun on Vulkan. |
| 2 wrong results, validation or not | `particle_depth_sort` differs at 2,048 slots (1 and 9 are exact); `primary_geometry_check_retry_queue` differs at words 30 and 39 (GPU 0, CPU 1). | Not diagnosed. Both kernels are unchanged Renderlab WGSL; whether wgpu gives the right answer for them on the same driver is not known yet, which would separate the adapter from the driver. |

At 33f1ff6, pinned to the ara-gpu fixes, the gate reran the suite under validation:
49 passed and 2 failed with no validation message, and all ten validation-only
failures are gone. The gate then diagnosed the two wrong results; neither is a broken
kernel.

- **Particle depth sort.** The fixture's depths were multiples of 0.25, whose low
  eleven mantissa bits are zero, exactly on the boundary of the key's `>> 11`
  bucket. RADV's dot product landed one ulp under for a few of them, so those keys
  moved one bucket down and the sort then ordered them correctly. The fixture now
  keeps every positive depth at least 768 ulps inside its bucket. A one-bucket move
  of a boundary depth changes no picture; the kernel is untouched.
- **Primary geometry check.** Two of 85 words differed, both for items on exact
  dyadic edge coordinates. The SPIR-V carried no `NoContraction`, so the driver was
  free to fuse a multiply and an add where Metal, compiled with
  `-fno-fast-math -ffp-contract=off`, keeps them apart. `shader::spv_precise` now
  decorates every float arithmetic result of every emitted stage `NoContraction`
  (4,520 results in the standard cohort; explicit `fma` calls stay fused, as the
  source asks), which gives Vulkan the float policy Metal already has. The fixture
  is unchanged. Whether this makes the two words agree on RADV, and what it costs
  in GPU time, are both unmeasured: it has only run through `spirv-val` here. If it
  costs measurably it can be narrowed to the geometry kernels, and if the words
  still differ the fixture moves off the dyadic edges instead.

Neither change has run on a Vulkan device.

## Second and third Vulkan runs (8a8ead7, 520b2bb)

At 8a8ead7: 62 passed, 2 failed, no validation message. At 520b2bb: 66 passed, the
same 2 failed. The particle sort passes.

**The geometry check, and the length of a heap-array element.** The check failed on
the same two words in every run: the fallback flags of the two hits on an edge of a
mesh with no adjacency (GPU 0 where 1 is expected). `NoContraction` did not change it
and neither did moving the hits 2^-22 inside their edge, so it was never arithmetic.
The gate found the cause by running the buffer-array probe on Vulkan (it was gated to
macOS, the only test in the suite that was): `arrayLength(&reads[i].data)` returned
one element's length for all four invocations.

The SPIR-V explains it. For `heap[i].data[j]` Naga decorates the access chain
`NonUniform`; for `arrayLength(&heap[i].data)` it emits the element pointer and
`OpArrayLength` with no decoration (`write_runtime_array_length` never adds one).
Without it the index must be the same for every invocation of a workgroup, and where
it is not the result is undefined. RADV then reads one element for the whole
workgroup; NVIDIA and Metal (where lengths come from a table, not a query) happen to
give each invocation its own. The renderer's `primary_edge_neighbour` tells an index
buffer with adjacency from one without by `arrayLength % 3`, per pixel, so on RADV
every mesh in a workgroup was judged by one mesh's length: in the fixture the mesh
without adjacency took the other's 13 words for its own 3.

`shader::spv_nonuniform::repair` now decorates the element pointer of every
`OpArrayLength` that reads through an indexed access chain, next to the
sampled-image repair it already made. Every emitted stage is held to it statically
(`assert_nonuniform_resources`, which failed on three tests before the fix), and the
buffer-array probe now runs on every backend, with buffers that are not multiples of
16 bytes and, off Metal, with every read in bounds: the emitted SPIR-V has no bounds
clamp, where Naga's Metal output clamps, so a read past the end is the device's to
define.

The same omission is in Naga's output for the wgpu renderer: its SPIR-V for
`primary_edge_neighbour` has the same undecorated `OpArrayLength`. Whether the wgpu
renderer shows it on RADV is not measured here; the frames compared so far would not
show it, because every mesh in them has the same kind of index buffer.

**The cold reconstructed frame** was held bit-equal to the plain frame, where RADV
gives weights of 1 + 2^-16 and 48 pixels one half-float step apart; the test allows
one step. With that it runs on and fails at frame 16 on the mean weight: 16.826
against 17, 1.02% short where the bound is 1%. On Metal the shortfall is 92 pixels
(the card's outline) and 809 samples at that frame, 0.39%; RADV's is 2,138 samples
with a least weight of 3.56 against Metal's 6.66. At 29d8e7b the gate's line says
659 pixels are short on RADV where 92 are on Metal, so it is more pixels, not the
same pixels losing more. On Metal all 92 lie within a pixel of the card's outline.
Not explained: the bound stays, and the failure now also counts the short pixels by
their distance from that outline, which says whether RADV's are a wider ring around
the card or somewhere else.

At 29d8e7b the buffer-array probe and the geometry check both pass on RADV (69 of 70,
no validation message), so the `NonUniform` repair is confirmed there. The same
question put to the wgpu renderer's own path (Renderlab #30) fails on RADV with
lengths 3, 3, 3, 3 for buffers of 3, 13, 6 and 22 words.

**Where RADV's short pixels are, and out-of-bounds image loads.** At 937e439 the gate's
line placed them: 215 within a pixel of the card's outline and 444 three or more pixels
from it. A 128x96 image has exactly 444 border pixels, so RADV loses weight along the
image's border, which Metal and the RTX card do not. The resolve reads its 3x3
neighbourhoods and history taps beside tests for being inside the image, and its
shader, like the renderer's others, is written against a defined result for a load
outside one. The emitted SPIR-V left image loads unchecked, and ara-gpu enables no
robust image access, so on Vulkan such a load was undefined: each driver's own
behaviour. On Metal the emission clamped it. The wgpu renderer on Vulkan leaves loads
unchecked under `robustImageAccess`, which returns zero.

Both emissions now give a load outside the image zero (Naga's `ReadZeroSkipWrite` for
image loads): one defined behaviour, the renderer's on the devices it is compared on.
All 73 tests pass on Metal with it and all 67 stages pass `spirv-val`. **Whether it
removes RADV's border shortfall is not known until the weights test reruns there**;
it is the likely cause, not a measured one. Index and buffer accesses in the SPIR-V
are still unchecked (enabling those checks trips Naga bugs), where the wgpu renderer
has `robustBufferAccess2`: a read past a buffer's end is still each driver's own on
Vulkan. The GPU-time cost of the image checks is unmeasured.

## Renderlab main at 75c5639: restart zones and batch words

Main gained two changes the frame mirrors, merged here together.

**P-RELIT.** A changed local light now restarts only its own sphere and the restart
shadow budget is per pixel. The decisions are in the shared planner, which the backend
already calls; what moved is where they travel. `PrimaryFrame` grew from 144 bytes to
336: after the reflection bands it carries TR1's restart zones (`Tr1Zones`: the moving
caster's swept shadow volume, eight relit spheres, the counts), which the trace
kernels now read. The frame record here is that size, zero past byte 144 unless the
frame is reconstructed, when `Planned::zones()` is copied in as `prepare_reconstruction`
writes it. Nothing else in the backend changed for it. **No test here changes a local
light or moves a skinned caster**, so the zones' effect is exercised only through the
shared planner's own tests on main.

**Batch words.** After PR #30 showed the wgpu renderer reading one element's
`arrayLength` for a whole workgroup on RADV, main stopped deciding anything from the
length of a buffer-array element. Each draw's triangle count, adjacency flag and
skinned flag now sit in a word per draw slot after the placement words
(`primary_ray::placement_words`, `batch_word`), read by `primary_batch_info`. The frame
uploads its placements through those two functions. The stand-alone replay fixtures
(the geometry check, the trace, the hit-based post surface, programmable shading, the
visibility resolve) build the same buffer from its description in the test harness, not
through the crate, so the layout is checked from both sides. The adapter's `NonUniform`
repair stays: other kernels still take lengths of heap elements, and an undecorated one
is undefined whoever reads it.

## Transparent panes on the traced frame

`ForwardScene::transparent` draws panes as `Renderer::encode_transparency` does on a
ray-traced renderer, after opaque shading and the resolve and before post. Opaque view
depth comes from the hit records (`tp_depth_hits`, already ported). Then each pane, far
to near by the centre of its world bounds along the view's forward axis: the HDR target
is copied behind it, a four-sample target is seeded from that copy, the pane draws
into it, and it resolves back into the HDR target. The renderer reseeds for every pane
and so does the frame; the earlier stand-alone glass replay seeds once and keeps the
samples, which is the same picture for panes with nothing drawn between them.

The pane's program is the standard program composed by the renderer's own
`glass_source` on the hardware head (`transparency_glass::traced_program`). Its
fragment stage traces the pane's own sun ray against the frame's scene, the first
fragment-stage ray query the backend has compiled or run; it took no adapter change.
The frame block and the sun-map fit are the renderer's own functions (Renderlab #32).
The camera is the unjittered one. A frame below native scale skips the panes, as the
renderer does.

`a_pane_passes_what_its_coverage_fresnel_and_tint_leave` measures what a pane passes
without modelling its lighting: a pane's light is its own shading plus `T` times the
colour behind it, so two frames with different floors behind the same pane differ by
`T` times the two bare floors' difference.

- Inside a pane, on 1,764 pixels, `T` is within 0.002 of (1 - coverage) x (1 -
  Schlick's Fresnel reflectance at the pixel's view angle) x tint, in each channel.
- Outside every pane the frame is the bare floor's, bit for bit.
- On 176 edge pixels the four coverage samples leave whole quarters of the pane.
- With a second, different pane under half of the first: the floor's light passes
  both (544 pixels), and the frame there is the near pane alone plus its `T` times what
  the far pane alone adds to the floor, within 0.0005, in either submission order. The
  other draw order would be 0.21 away, and with the sort removed the test fails by
  exactly that.
- The mesh pass refuses panes.

**Short of the renderer, by design of this step:** on a ray-traced frame the renderer
also traces a reflection for panes whose program declares a lobe (the standard one
does), with a three-attachment lobe pass and `tp_reflect`; neither is ported, and the
frame block tells the shader so. Coloured sun transmission (a pane's tinted shadow on
opaque surfaces) was not assembled at this step; it is now (below, under the first
game-scene comparison).
Particles and emitters are not assembled. **Not compared with wgpu**, and with the
reflection missing a comparison of a pane is expected to differ.

## Shading programs and their parameters on the traced frame

Until here the frame shaded with the standard program alone. That was a shortcut of
the port, not a limit of what was ported: a renderer material names its program by a
wgpu `Program` handle, which the backend cannot take, so the frame's description had no
way to name one and `frame()` composed `STANDARD_PROGRAM` as a constant. The pieces
behind it (`program::compose` and `compose_list`, the binning kernel, the list entry)
were ported and replayed long ago and never chained.

`ForwardScene::programs` now gives each object a program by its WGSL source and its
parameter values by name, which is also how the engine-level entry will identify them
(docs/design-engine-backend.md on main's #35). The frame does what
`Renderer::create_program` and the traced frame do with them:

- Each distinct source is composed at the raster stage and analysed, which validates it
  and reflects its `Parameters` layout. Values are checked against that layout by name
  and type and packed into a 256-byte block per draw (`ParameterLayout::pack`), bound
  as `rl_parameters` at (2, 17). A program without `Parameters` gets a zero block when
  another program of the frame declares some.
- The program is composed into every stage the renderer composes it into: camera hits,
  reflection hits (same pipeline) and, with bounce, the radiance cache's cells
  (`Cache::new` takes the source). The restart key covers the sources, each object's
  slot and its parameter bytes, as `reconstruction_key` does.
- **One program:** the whole region is shaded by one dispatch, as before.
- **Several programs** (MP1, `ProgramLists::prepare` and `encode`): the distinct
  sources in first-draw order are the frame's slots, at most 32. After the trace and
  the fresh visibility pass, the binning kernel lists each program's 4x4 tiles, then
  each program's list entry (`primary_shade::hardware_soft_list_program`) runs as one
  indirect dispatch from the arguments the binning pass wrote, with its own `RlList`
  record (slot, list base, capacity, then every batch's slot) at (0, 38) and the lists
  at (0, 39). Slot 0, the first draw's program, also writes misses. The cache's cells
  are shaded by slot 0's source.

A list program reads only what its own source reads, so the frame gathers its inputs
from every list program and not from slot 0's alone: with a program that samples no
texture in slot 0, the next program's texture arrays were at first not supplied.

`a_program_shades_each_object_with_its_own_parameters` (a card over a floor, traced):

- A program of the test's own that emits `colour * gain` and reads no light puts
  exactly that times the exposure in every pixel of each object, the floor at the
  program's defaults and the card at its own values: 11,232 floor and 812 card pixels,
  bit for bit.
- The game scene's program at its defaults shades as the standard program (0 apart).
- `glow` set on the card alone adds the exposed glow to the card (within 0.0002) and
  changes no floor pixel.
- Two programs in one frame, in three arrangements (game then flat, flat then game
  with glow, standard then flat): the flat object's pixels are the flat value and every
  other pixel is bit for bit the frame where its program shaded alone, 36,132 pixels
  in all, down to the tiles both objects share.
- Refused: a parameter the program does not declare, and any program on the mesh pass.

`programs_emit_valid_spirv_at_the_region_and_list_entries` emits the region and list
entries of that program and of the game scene's two as SPIR-V and holds them to
`spirv-val`, the `NonUniform` check and exact arithmetic. **None of this has run on
Vulkan, and none is compared with wgpu.**

### The game scene's opaque objects

Superseded by the engine-level entry below, which builds the same scene by the
example's own calls. The numbers it first gave (183 pixels changed by the hero's
program, 18 by the glow, 17,280 exact sky pixels, the arch bulb keeping 0.9955 of its
glow) are the ones the engine's frames give.

## Renderlab main at 6adfaee

Merged: #33 (the glass reflection source and head, crate-visible; nothing uses them
yet) and the TR1 native-scale fix. `tr1_native()` compared a float quotient of the
sample and output sizes with exactly 1, and RADV's division of equal sizes is not
exactly 1, so its native frames took the upscaling path and lost history at the image
border and beside silhouettes: the 659 short pixels of
`reconstruction_accumulates_jittered_samples_into_edge_coverage` on the Radeon, which Renderlab's own test
(#34) showed on its wgpu path too. The resolve now compares the two integer sizes. The
port needed no change, and neither did its test, which should now pass on the Radeon;
that has not been rerun. 78 tests pass on Metal in both validation modes.

**On the Radeon (RADV) at 322372d:** 78 of 78 pass under validation and sync
validation with no validation message, programs and the game-scene frame included, and
`reconstruction_accumulates_jittered_samples_into_edge_coverage` passes with main's
fix. The wgpu renderer on main gives 92 pixels short and none on the border there, the
RTX's line exactly. The weights question is closed on both sides.

## The engine-level entry

`backend_ara::engine::Engine` takes the same scene-building and frame calls as
`engine::Engine`, through the `engine::Scene` trait (Renderlab #36, design in #35). It
records them into the engine's own device-free `State`, with its own payloads: mesh
vertices and indices, texture data, a program's WGSL and reflected parameter layout, a
material's factors with the program and textures it keeps alive. `render` turns the
state into a `ForwardScene` and runs it through `forward::Frames`; `capture` returns
the displayed bytes; `images` gives the frame's HDR. The sun, lights, sky and settings
go through the engine's own `settings::lighting` and `settings::atmosphere`, so both
layers translate them with the same code. `Quality::Realtime` is the Interactive
traced frame and `Reference` the exact one. A change of settings or size starts a new
frame sequence, since the cache and the history are sized by them.

**Refusals.** A call or setting the layer cannot honour is recorded once as
`Refusal { call, what }` and the scene goes on without that piece:

| Call | What |
|---|---|
| `set_settings` | `volumetric_fog`; `sky_light` other than 1; `post.enabled` false; any field added to `Settings` later, while it differs from its default |
| `create_emitter` | particle emitters |
| `set_decals` | decals |
| `create_skinned_mesh` | skinned meshes (objects spawned with one are not drawn) |
| `create_material` | `normal_texture` (the trait has no mesh with tangents yet) |
| `create_environment`, `set_sky` | environment maps and `Sky::Map` (the previous sky stays) |
| `add_asset` | cooked assets |
| `render` | textures on a transparent material |

What the frame itself rejects is an error from `render`, not a refusal: a scene
without a sun, and more than `TRACED_DRAWS` (256) objects.

**Short of the wgpu engine in how it works, not in what it draws:**

- (Removed since: a mesh-material batch is one draw of its objects as instances, as
  in the wgpu engine; see "One draw, many placements" below.)
- Little is resident between frames yet (see "What a frame keeps" below): frame time
  here is not yet the layer's.
- The design note's strict mode (the first refusal as an error) is not built.

`the_game_scene_builds_and_renders_through_the_engine_calls` builds the example's scene
at its defaults by `Scene` calls (two programs, twelve materials, four meshes, 30
objects with the glass pane, the sun, two emitters, two decals) and animates it as the
example does.

- The engine's refusals are exactly the scene's volumetric fog, its emitters and its
  decals.
- With direct light only, against a second engine whose hero has the props' program:
  183 pixels differ, all at the hero, and all 17,280 sky pixels are lit and identical.
- `set_material_parameters` setting the bulbs' glow to zero changes 18 pixels, all at
  the two bulbs, and the arch bulb's centre loses 0.9955 of its exposed glow.
- At the scene's own settings, six frames of the animation run (hero, spinner,
  lanterns and camera moving), finite and lit, and the last is written to `target/`.

The scene's description is `support/ara_game_scene.rs`, **a temporary second copy** of
the example's construction, animation and geometry builders, with flat stand-ins for
the two decal textures. It goes when the example's own are functions both layers call
(the PR on main after N-VSM1).

`ara_frame --scene game` renders it on either layer, the wgpu `Engine` on `HardwareRt`
or the backend's, and `--compare` reads the two as it reads the fixture's levels.
`--supported` leaves out on wgpu what the backend refuses. On Metal two backend runs of
three frames are identical in every byte. **The wgpu side has only been compiled**: it
cannot run on the Mac.

```sh
# what the picture is missing: the full scene on wgpu against the backend
ara_frame --backend wgpu --scene game --frames 12 --output out/game-wgpu
ara_frame --backend ara  --scene game --frames 12 --output out/game-ara
ara_frame --compare out/game-wgpu out/game-ara
# what both draw: the same with --supported on each
```

## What a frame keeps, and its time on Metal

`forward::frame` was written as a test harness: it allocates, uploads, compiles and
releases everything a frame needs, every frame. Measured on the game scene
(`ara_frame --backend ara --scene game --supported --frames 12 --size 1280x720`, release
build, M5 Pro, no validation layer; frames 2 to 12, which agree within 3%):

| | wall time a frame | building the frame | of it, meshes | GPU until done | readback and release |
|---|---|---|---|---|---|
| Everything rebuilt | 196 ms | 111 ms | 3.0 ms | 83 ms | 2 ms |
| Meshes and bottom-level structures kept | 186 ms | 106 ms | 0.03 ms | 78 ms | 1.8 ms |
| and shader reflection kept | 86 ms | 14 ms | 0.02 ms | 70 ms | 1.8 ms |

The last frame's HDR is byte for byte the same in all three. `ForwardImages::times`
carries the four numbers and the tool prints them per frame.

**Meshes.** `ForwardScene::meshes` gives each object an identity. `Frames` keeps a named
mesh's vertex and index buffers and its bottom-level structure for as long as frames
name it; objects of one identity share one structure, each with its own instance. The
top-level structure is still built every frame, as the renderer rebuilds its own when
instances move. The engine names its meshes, so the game scene builds four structures
once in place of thirty a frame. `named_meshes_stay_on_the_gpu_and_draw_the_same_frames`
holds a named sequence to the unnamed one byte for byte over five frames in which two
cards of one identity move, leave and return. Panes and normal-map tangent streams are
still uploaded every frame.

**Shader reflection.** That was not where the time went. A sampling profile of the
steady state showed the frame's CPU time in naga: every frame parsed and validated the
whole composed WGSL of each of its programs (the scene cooks 42 pipelines) to
reflect their inputs and workgroup sizes, validated each scene program again for its
parameter layout, and hashed a quarter of a megabyte of adapter source for each
pipeline's cache key.
Reflection results are now kept for the life of the process by pass and source digest
(`shader::memo`), parameter layouts by source, the key's constant part is digested
once per `ShaderCache`, and pipeline manifests are read once. The cache key's format
changed with it, so a cache directory cooks its shaders once more.

**Pipelines.** The shader cache now owns each pipeline: it makes one per WGSL, entry
and state and hands the same handle out every frame, and `ShaderCache::retire` releases
them. The passes that draw with a pipeline no longer release it, and the `retire`
methods that did nothing else are gone with their call sites. No pipeline is created
or destroyed per frame.
On Metal this moved the frame little, since Metal's own cache hid the cost; on Vulkan
it is every pipeline of the frame.

**Fixed passes and digests.** A pass's own source is composed once and kept, its
inputs and workgroup size are kept by pass, and its pipeline is found without hashing.
A caller-authored program takes its source's digest once, for the reflection tables
and the pipeline key.

**The atmosphere's tables.** `Frames` keeps the transmittance, multiple-scattering and
sky-view tables and the procedural environment's two cubes, and remakes each only
when the renderer would: the first two when the medium changes
(`a.luts != Some(medium)`), the environment when `ProceduralKey` (sun, medium,
altitude) changes, the sky view every frame.
`kept_atmosphere_tables_give_every_frame_what_fresh_tables_give` holds a five-frame
sequence (a repeated view, the camera 40 m higher, a thicker medium, a lower sun) to
the frame a fresh sequence renders, byte for byte; it fails at the camera's rise if
the environment is made only once and at the thicker medium if the first two tables
are. One exception it states: after the sun turns, a few penumbra pixels differ,
because the shadow planner carries the sun's shadow orientation from frame to frame
and a fresh sequence starts it anew. That is the planner's state and not a table's.

**Readback.** `Frames::without_images` (the engine's `set_readback(false)`) renders
frames and leaves their images on the GPU, for timing.

The frame over those steps, same scene and machine as the table above (frames 2 to
12; the last frame is byte-identical from step to step):

| | wall time a frame | building the frame | GPU until done | readback and release |
|---|---|---|---|---|
| Reflection kept (as above) | 86 ms | 14 ms | 70 ms | 1.8 ms |
| Pipelines owned by the cache | 84 ms | 12 ms | 70 ms | 2 ms |
| Fixed passes by pass, digests carried | 79 ms | 7 ms | 70 ms | 1.8 ms |
| Atmosphere tables kept | 81 ms | 8 ms | 71 ms | 2 ms |

The last two rows are within the run-to-run spread. A sampling profile of the last
state puts about 90% of the main thread in the wait for the GPU.

**What the 70 ms on the GPU is.** With one feature left out at a time
(`--without`): the radiance cache 24 ms, the lanterns 13 ms, reconstruction 8 ms,
local-light shadows 8 ms, reflections 5 ms, the pane 2 ms; with all six out the frame's
GPU time is 19 ms. Those are the renderer's own passes running through this layer, not
the harness. Whether they cost more here than through wgpu is what the comparison on
one machine is for; it cannot be made on the Mac, where wgpu's traced path does not
run.

**`ara_frame --scene game --timing`** is that comparison's script for both layers: it
captures and reads back nothing and prints the median, least and greatest time of a
frame after the first eight, each from the call to `render` until the GPU has finished
the frame (the wgpu engine returns at submission, so the tool then waits on its
device). On the Mac the backend's line is `median 80.03 ms` at 1280x720.

**What a frame still makes every time:** its targets and record buffers (allocated
and released), the top-level structure, the aerial volume, and each program's composed
source (the text, to look its kept reflection up). On Metal none of these shows in the
profile. On Vulkan the allocations may; the first timing run there will say, since the
tool prints the split between building the frame and the GPU.

## The first game-scene comparison (Radeon, c9a0053) and what it found

`ara_frame --scene game --supported` on the Radeon, wgpu against the backend, 12 moving
frames: whole-frame means agreed (146.48 against 146.51) but the worst HDR cell was
16% to 18% off, far from the fixture levels' 0.06%. Three findings, each read from the
code:

**The bright band on the ground: the pane's coloured sun shadow, the port's.** The
renderer draws each transparent draw's nearest sheet into its own layer of a 1024²
sun map over the panes' footprint (`prepare_transmission`, `encode_transmission`: the
light the sheet passes in rgb, its depth in alpha), and `light_sample` in the program
API multiplies a shadowed sun by every layer that lies between the receiver and the
sun. Every program reads the record at (3, 26) and the map at (3, 27). The port had
the layer pipeline as a stand-alone replay (entry 49) and the frame bound zeros there,
which the shader reads as no panes: full light. At dusk the game scene's pane throws a
long shadow, the band. The frame now builds the record with the renderer's own
`shadow_fit`, draws each pane into its layer before anything shades, and binds both;
the restart key takes the record and each pane's geometry, material and transform, as
the renderer cuts reconstruction when the map changes. A frame below native scale
keeps the shadows though it skips the panes, as the renderer does.

`a_pane_passes_what_its_coverage_fresnel_and_tint_leave` now holds the shadow to its
closed form with the view's passage: under a lower sun, the difference of two frames
with different floors over the bare floors' difference is every `T` the view ray
crosses times every `S` the floor point's sun ray crosses, `S` being the pane's
coverage, Schlick's Fresnel at the sun's angle and its tint. 1,593 pixels inside a
pane and 225 in a pane's shadow alone are within 0.0013; outside every pane and shadow
the frame is the bare floor's bit for bit. Under two panes the far pane's own light,
dimmed where the near pane shadows it, passes the near one within 0.0005, and the
other draw order would be 0.17 away.

**The pane's own picture: its traced reflection, not built.** On a ray-traced frame
the renderer traces a reflection for a pane whose program declares `reflection_lobe`.
The port does not. `--supported` now leaves it out on wgpu by giving the pane the
standard program with that one function renamed, which the renderer then shades
without tracing; `ScenePrograms::transparent` lets a pane name its program in the
port, and there the two programs draw the same image byte for byte (checked on
Metal). A pane program that declares `Parameters` is refused: a pane's parameter block
is not assembled.

**The 12 triangles: the pane, a matter of counting.** The renderer's `FrameReport`
counts its opaque draws; the pane's cube is 12 triangles and the port counted it. It
no longer does (37,598 on both). Draws stay different by construction: the renderer
reports 11 mesh-material batches, the port 29 objects.

**The bulb's rim: not explained from the code.** The worst HDR pixel (38%) is on the
rim of the bulb under the arch. That bulb sits one source radius above its own point
light, whose light on it is about ten times its glow, and every lantern has a source
radius, which no fixture light has: the soft-source path had never been compared with
wgpu. The cluster header's sampling word, the light records and the shading code read
the same on both sides. `ara_frame --scene game --without NAME[,NAME]` leaves one
feature out on both layers (`gi`, `reconstruction`, `reflections`, `local-shadows`,
`glass`, `lanterns`, `glow`) so the gate can find which one the rim belongs to.

## Depth of field on NVIDIA: one level low on the flat card

Twice on the RTX 4070 SUPER the depth-of-field test failed on the card's pixels out of
the blur's reach: 192 where the sharp frame shows 193. The two runs are one symptom
(the first stopped at the first such pixel; the second counted them: all 9 card
pixels one level low, all 23 floor pixels exact).

From the kernel: a card pixel out of the edge's reach takes 48 bilinear taps of one
value, divides their weighted sum by the weights' sum, mixes that with the centre and
stores a half float, where the sharp frame stores the value once. Nothing in it can
reach the floor from there.

From the numbers. The displayed level is the device's own: its tone stage, its sRGB
encode in a fragment shader and its UNORM store. No CPU code converts it. A CPU
conversion of the same half floats (the scalar tone and sRGB replay) gives the card,
`0x3c86`, 192.58, and each half-float step below it 0.035 of a level less; the
rounding boundary 192.5 falls between `0x3c84` and `0x3c83`, which is exactly where
Metal changes level. The RTX shows 193 for `0x3c86` and already 192 for `0x3c85`
(192.55 on the CPU): its own encode lands about 0.06 of a level lower than the CPU's,
a few parts in ten thousand, which is what two different GPU encodes are allowed. So
on the RTX the card sits one half-float step above its boundary. And the blur does
lose a step on a flat region, on Metal too: in half floats 4 of 37 out-of-reach pixels
are one step low when focused on the card, and one of 16 brightness steps. One step
is invisible in levels on Metal here and a whole level on the RTX.

The test assumed more exactness than the pass gives, and measured it in the wrong
unit. It now compares, out of the blur's reach, the half floats depth of field leaves
(`ForwardImages::focused`) with the sharp frame's, one step allowed, which holds a
leak to 0.1% whatever a device's rounding. `depth_of_field_keeps_a_flat_region_within_a_level_at_every_brightness`
prints, per brightness step, the sharp half float, the
CPU's level for it, the device's level, the blurred half float, the steps lost and
its level, and requires at most one step lost and every displayed level within one of
the CPU's. If the RTX's line shows more than one step lost, that is a real loss and
both tests fail there.

## The one-pixel difference at the arch lamp: where it is, not yet why

In the supported pair on the Radeon one pixel, (588, 178) at 960x640, differs by 38%
under reconstruction and by nothing without it. What is established, on Metal:

- **It is not the bulb.** `ara_frame --probe 588,178` prints the pixel and its
  neighbours every frame. Through pose 10 it is plain beam, (0.3210, 0.1620, 0.0527).
  In the last frame the camera's motion brings the edge between the beam and the lit
  surface below it into the pixel's row, and it becomes a silhouette pixel.
- **Both values are one mix of the same two surfaces.** wgpu on the Radeon,
  (0.3843, 0.2228, 0.1062), is the beam plus 20.6% of the pixel below it,
  (0.6279, 0.4585, 0.3132), in all three channels; ara on the Radeon,
  (0.4619, 0.2976, 0.1720), is the beam plus 45.9% of it. So the two runs differ in
  one number: the coverage the resolve carries for a moving silhouette edge
  (`tr1_coverage`), at one pixel, in one frame.
- **The port on Metal gives wgpu's value**, (0.3843, 0.2228, 0.1063). The port's
  resolve is not different from the renderer's here; the port on the Radeon is the odd
  one of three.
- **It is not contraction, on Metal.** Letting the resolve's arithmetic contract
  (`-ffp-contract=fast` for that shader alone) changes no pixel by more than 0.1%, and
  for every shader no pixel by more than 0.5%; this pixel does not move at all.
- **Forcing the resolve's decisions the other way, for every pixel and frame, does not
  give the Radeon's value**: the occluder chosen by the other side of
  `s.distance<=u.distance` gives 0.343; the carried straight edge switched off
  (`gg>=0.0025`) gives 0.381; no coverage history gives the plain beam, 0.321, so the
  20.6% is carried coverage and this frame's samples alone would show none.

What is not established: which expression fell differently on the Radeon. The
candidates in `tr1_coverage` are its thresholds on stored coverages
(`c<=0.001 || c>=0.999`), on the gradient (`gg>=0.0025`) and the tap chosen by
`floor(q)`; each is a decision on a float, in the renderer's own shader. The next
datum is the port's per-pose probe on the Radeon and on the RTX: whether pose 10
already differs from Metal's line, and whether the RTX sides with Metal.

The second pixel, (504, 377) without reconstruction, is a different thing: there the
port agrees with itself on Metal and on the Radeon, (0.3369, 0.1519, 0.0365), and
wgpu differs, (0.3716, 0.1664, 0.0392). The difference has the dusk sun's colour on
the ground and is an eighth of the sunlit ground beside it: one of the eight shadow
rays of the unreconstructed Interactive estimator at the edge of the gold sphere's
shadow.

## One draw, many placements

A traced frame held one placement a draw, so 256 objects; the renderer holds 256 draws
of any number of instances. `ForwardScene::instances` gives each object a list of
transforms: it is one draw placed at each. The frame's per-placement tables (transforms
and previous transforms, world bounds, top-level instances, the placement table) run
over every instance in draw then instance order, and its per-draw tables (geometry,
material, parameters, program slot, batch word) stay one a draw; the renderer's own
`placement_table` already took instance counts. The engine now draws each
mesh-material batch as one draw, and its report's counts are the renderer's: the game
scene is 11 draws and 37,598 triangles on both layers.

`one_draw_placed_many_times_is_the_frame_of_as_many_objects`: three cards as three
objects and as one draw of three instances are the same frame byte for byte; 300
cards are one draw where 300 objects are refused; the mesh pass refuses instances.

**A defect this found in the port's radiance cache.** The cache's draw table gives
each draw its stable id, first placement and instance count, and the port wrote one
instance a draw, which was true only while a draw was a placement. With batches the
game scene's frames went 20% to 32% wrong in their worst cell from the second frame,
all of it through the cache (identical with it off). The table now takes each draw's
count; the batched scene is byte for byte the unbatched one over twelve frames, and
the test above repeats its comparison over four frames with the cache on and a card
moving, where the old table fails at the third frame. A one-frame test without the
cache had passed.

## Each draw's cells by its own program

Until this, a frame of several programs shaded every cell of the cache with the first
program's cell shading, so a surface's bounce light was another program's. The frame
now does what `Renderer::ensure_gi_shade` does (MP1): one cell program for each of the
frame's programs that reflects, each rewritten by the renderer's own
`gi_cache::mixed_cell_entry` to shade only the cells whose draw selects it, and each
dispatched over the queue in turn; the draw table's fourth word is the draw's
`cell_selector`, zero for a program that reflects nothing. A frame of one program is
unchanged (the plain entry, zero in every row). The selector is the program's slot in
the frame, where the renderer uses its program id; the two only have to agree between
the table and the entry.

When the frame's programs or their order change inside a sequence, the cache
composes its cell programs again and keeps its table (`Cache::shade_with`), as the
renderer remakes its cell pipelines and keeps its cache. The cache's lighting key now
also takes every program, which draw has which, and the panes' sun map, as
`prepare_gi`'s does; the sun map was missing from it since the panes' shadow was
added, and nothing else tells the cache about a pane (it is no caster in the cache's
world, and the default settings have no rotation), so cells under a pane that moved
kept their old light for as long as the view stood still.

`a_pane_carried_away_relights_the_cells_it_shaded`: a sunlit floor, a card the sun
does not reach, a red pane overhead. Under the pane the card is [0.641, 0.128, 0.128].
The pane is carried away after ten frames; three frames later the card has settled at
[0.673, 0.671, 0.671], against [0.668, 0.668, 0.668] in a sequence whose pane was
never overhead (the test asks for 2% at six frames). The 0.7% that stays, with red
0.2% over green, does not decay over the three frames after and I have not explained
it. With the key as it was the card is still [0.641, 0.128, 0.128] six frames later
and the test fails.

`two_programs_in_one_frame_each_light_their_own_cells`: two sunlit floors with a
black wall between them and an upright card on each that the sun does not reach,
under a black sky, so a card shows only what its own floor sends up. One floor is the
standard program at 0.8, the other a program that sends on red alone. After eight
frames the card over the white floor is [0.602, 0.602, 0.602] and the card over the
red floor [0.759, 1e-7, 1e-7], with either floor drawn first; the ratio 1.26 is the
red program's whole reflectance against 0.8. With every cell shaded by the first
program (the old behaviour) the test fails in both orders: the card over the red floor
comes out grey with white first, the card over the white floor red with red first.
When the floors change places in the draw order after eight frames, the next frame is
still the settled one (a cache that had started again shows nothing on its first
frame) and four frames later the colours hold; without `shade_with` the two cards
swap colours there. (These cases name their draws; see the next section.)

The cell entry of one program among several emits valid SPIR-V for the game scene's
two programs and the test's (`programs_emit_valid_spirv_at_the_region_and_list_entries`).
It has not run on a Vulkan device yet.

## Draws named in the cache as the renderer names them

A cell remembers the draw it lies on by an id (written when the cell is probed, never
on a relight) and reads that draw's material and program through the draw table. The
port numbered its draws by their place in the frame; the renderer keeps an id for each
(vertex buffer, material) pair it has seen (`Renderer::gi_draw_ids`). The port now does
the same: `ForwardScene::materials` gives each object a material identity beside its
mesh identity, the cache keeps one id per pair for the sequence (and per repeat, for a
frame that draws a pair more than once), and its lighting key takes the names as the
renderer's takes the ids. The engine entry names every material it creates. A scene
that names nothing is numbered by place as before, and a sequence must do one or the
other throughout.

What numbering by place gets wrong is narrower than I first reported. I had read the
0.7% rise four frames after the floors changed places as cells read through a stale
id. It is not: the named sequence shows the same numbers to the last bit, and so does a
sequence where the order changed and nothing was misread. It is the relight that any
change of the lighting key starts, which runs the cache's gathers again on top of the
light already there; a sequence that never changes stays at 0.602. The same residue is
in the pane test above. Whether the renderer shows it is not measured; the planner
that schedules it is shared.

What protects a sequence numbered by place is the cache's own planner (shared code):
it compares the placements' bounds place by place, and where they differ it probes the
cells in both boxes again, which writes their ids afresh in the frame of the change.
So a draw put ahead of the others, which moves every draw one place on, comes out
right under either numbering. Numbering by place fails when a draw's place changes
and the bounds at each place do not, and then it fails for good: with the test's two
floors given equal bounds and traded in the draw order, the card over the white floor
is [0.755, 0, 0] and the card over the red floor [0.615, 0.613, 0.613] four frames
later and after. With names both cases hold on the next frame and four frames later
(`two_programs_in_one_frame_each_light_their_own_cells`).

Not done: the renderer's software path also maps ids on the CPU (`draw_identity`);
the port has no software rays. An id the map cannot hold (past 65,534 pairs) reflects
nothing, as in the renderer; no test reaches it.

## The first timing against wgpu (Radeon, d5f9cf8) and what it found

`ara_frame --scene game --supported --timing`, 1280x720, 60 frames, three interleaved
pairs on the Radeon 760M: wgpu's median frame 86 to 89 ms, the backend's 119 to 120 ms.
The port was 33 ms, 38%, slower a frame, with 17.4 ms building the frame and 100.9 ms
on the GPU (medians of poses 8 to 59), and 1.2 s for the first frame of every process.
That run had one draw a placement (29 draws); every backend timing line from it is
superseded by the batched frame.

**Two costs it exposed that were the harness's.** An Interactive traced frame made its
repair queue anew each frame, zero-filled and uploaded: a 16-byte header and 36 bytes a
pixel, 33 MB at 1280x720, where the renderer clears the header alone. It also
zero-filled and uploaded the camera volume and the fog's scatter volume, 18 MB, though
fog is not assembled and the aerial pass rewrites only its own slices. 51 MB of fill,
copy and upload a frame. The sequence now keeps all three (`Kept::repairs`,
`SkyTables::volumes`): a frame clears the queue's header, and the volumes keep the
zeros they were made with. On Metal building the frame goes from 7 ms to 3.4 ms and
the last frame is byte-identical. The Radeon's number is not known yet.

**The split.** A timed sequence (`Frames::without_images`) wraps its submission in one
ara-gpu completion scope and returns the device's timestamp for every labelled pass
(`FrameTimes::passes`). `--timing` prints `timing split, medians in ms` on either
layer: the backend's passes with `record_ms` and `gpu_ms`, or the wgpu renderer's own
GPU samples from `Engine::poll` (render span, shading, camera rays, visibility,
reconstruction, present, shadow generation). What `gpu_ms` holds beyond the passes' sum
is unlabelled: structure builds, uploads, copies. The renderer's samples have no line
for the cache, so a feature's cost on both layers comes from `--timing --without NAME`
by difference. On Metal the per-pass attribution is not usable (the device reports a
46 ms cache stage beside a 0.01 ms trace: its timestamps do not follow the command
stream); on Vulkan they do.

**The first frame of every process.** The cooked shader code is reused across
processes and ara-gpu persists its Vulkan pipeline cache in the run's device directory.
What repeats in each process is reflecting every program's WGSL once (kept in memory
only) and creating each pipeline object: 230 ms of the first frame on the Mac, about a
second on the Radeon. Persisting the reflection beside the cooked code is not done.
