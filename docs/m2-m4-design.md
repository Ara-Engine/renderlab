# Selected M2 design and M3-M4 boundaries

Status: selected for implementation, not milestone acceptance. Base `96ae201796d31c35789ea7d656a9968a0113bdce`. Pstack how used separate runtime and current Nanite/tooling explorers, followed by Astra synthesis. Architect/arena compared independent Sol and Astra sketches. Root initially scored Astra 10/15 and Sol 11/15; Opus independently scored Astra 11/15 and Sol 9/15. After checking the objection against the viewer's ownership, root accepts **Astra**.

Sol's retained collection borrows meshes across frames. The viewer owns its meshes inside the same View that would hold the collection, creating a self-reference. Rebuilding each frame loses persistence; shared handles or a registry change the ownership design. Astra's transform-only collections avoid this and batch repeated multi-primitive assets without a placement-major draw explosion. Existing M1 meshes still belong to the caller, and the renderer owns its device/queue. No new mesh registry or scene graph is selected.

## M2 caller contract

```rust
let Some(mut instances) = renderer.try_upload_instances(&models)? else {
    return Ok(()); // Retry setup through the existing event/poll loop.
};
instances.set(37, changed_model)?;
let mut draws = [Draw::new(&mesh, &mut instances)];
if let Some(report) = renderer.try_render(&color, &camera, &mut draws)? {
    record(report);
}
```

`Instances` owns a fixed, nonempty set of finite matrices and persistent GPU storage. `set(slot, matrix)` validates before mutation and treats bit-identical changes as no-ops. `Draw` borrows one Mesh and mutable Instances for a frame, with `with_visibility(Visibility::Unculled)` available. Default visibility is Frustum. Renderer-created collections carry renderer identity; a mismatch fails before admission. Rename Limits.max_draws to max_instances, a per-collection limit checked against the device's buffer and storage-binding limits. Empty draw lists may still clear a target; empty collections are rejected.

One collection has one CPU matrix vector, a dirty bitset/count, cached visible bits and compact ascending u32 IDs. Delete duplicate uploaded-matrix shadows and dirty-slot lists. GPU resources are a 64-byte-per-slot read-only storage buffer, an instance-rate u32 ID buffer and its bind group. Group 0 remains camera; group 1 is instance storage, leaving group 2 for later materials. One nonempty collection emits one indexed instanced draw. No indirect/compute/mesh-shader feature is required.

Draw order, then ascending slot order defines equal-depth winners. M2 used Less depth, depth writes, and no face culling for color geometry. The later [main-camera correction](camera-depth.md) changes this to reversed-Z / Greater while preserving equal-depth ordering. Do not globally sort. Callers grouping formerly interleaved objects explicitly choose a new order. The reference oracle draws individual slots in the same declared order; it is test-only, not a second public rendering path.

Instance creation follows mesh-upload admission: nonblocking collection, fewer than two occupied flights, immediate submission of uploads. Rendering retains three flights and no ordinary completion waits. Busy creates/uploads nothing and preserves pending edits. Adjacent dirty spans coalesce; do not add the unmeasured 64-bucket policy. Native per-write staging is counted through bytes/calls and measured under 1% sparse updates. Post-admission GPU faults stop further work; do not pretend failed publication is recoverable by dropping a CPU dirty flag.

## Visibility and reload

Derive local bounds from validated CPU vertex positions. Use the exact uploaded camera matrix and actual target aspect. The predicate evaluates homogeneous clip planes without perspective division, so mirrored, sheared, singular and projective finite transforms remain accepted. Outside one plane means the entire box must be outside, with conservative arithmetic uncertainty. Unknown/overflow cases remain visible.

The initial predicate follows Opus's center/extent proposal: evaluate matrix composition and bounds in f64, use the absolute product envelope, relative allowance 2^-15, flush-to-zero allowance 2^-118 times B squared, and fail visible when 128 times B cubed reaches f32::MAX. Root corrected the proposed 2^-16 allowance: it equals 128u for u=2^-23 and is slightly smaller than gamma(128)=128u/(1-128u); 2^-15 leaves margin. B is at least one and bounds all input magnitudes. These constants are an implementation hypothesis to test, not a universal accuracy theorem. WGSL permits different evaluation/rounding behavior; see the [26 October 2026 floating-point specification](https://www.w3.org/TR/2026/CRD-WGSL-20261026/#floating-point-evaluation). CPU reference evaluation varies association, accumulation, rounding direction and subnormal handling; GPU full-image equality remains required.

Cache visibility by exact camera matrix, mesh bounds and visibility mode. Unchanged keys require re-testing only edited slots. Rebuild/upload the changed ID span only if visible bits change. Camera movement may update IDs but never model/geometry bytes. The initial implementation deliberately favors auditable arithmetic; measure moving-camera CPU cost and introduce cached transform-derived bounds only if needed.

The first 100k camera-motion pilot triggered that refinement: recomposing matrices dominated CPU time. The implementation now caches a conservative four-dimensional world box (including homogeneous w), plus the absolute-product envelope, per slot. Camera planes are built once per collection; edits or changed mesh bounds rebuild affected cached boxes. Switching back from Unculled rebuilds all boxes because edits may have been submitted while culling was disabled. This can retain extra objects under shear/projective transforms, but cannot tighten the enclosing box. The same 2^-15 rounding and flush-to-zero allowances and overflow fallback apply. Cache capacity costs 104 bytes per instance on this host and is reported separately as CPU memory. Independent f32 tests, exact full RGBA/depth oracle checks and cache-transition tests passed after this change. The before/after pilots are retained under measurements/m2-perf/20261103-000300 and 20261103-000849; final milestone measurements follow separately.

Frustum mode assumes the shader uses camera * model * original mesh position. Layout-compatible reload cannot enforce this. Document the contract in the shader and API, expose an unculled viewer toggle, and test position-changing reload there. Rejected syntax/layout retains the last valid pipeline. The new fixed ABI migrates the viewer, examples and tests together; remove the M1 renderer-wide model buffer and ordinal shadows.

## Verification units

1. Pure bounds/visibility and independent f32 reference tests. Initially compile this module only in tests; production integration is the next unit, not an unused-function lint suppression.
2. Instances and the new shader ABI, unculled batched rendering and exact RGBA/depth equality against per-slot draws, including coplanar ties and width 641. Migrate every caller.
3. Culling/cache/ID uploads: full/narrow/intersecting views, all matrix classes, near/w boundaries, movement/cuts, Busy preservation, replacement with in-flight work, shader error/correction, resize/minimize.
4. Paired dev/release measurements against preserved M1 executables: 1k/10k/100k, single/mixed/grouped/alternating geometry, static/camera/sparse/contiguous movement. Record whole caller and renderer CPU, cull/encode/submit/poll, pass GPU, counts, bytes/writes, churn and retained memory; inspect images and live behavior. Remeasure shader/Rust/startup latency. Positive Vulkan-layer activation and zero unexpected errors remain mandatory.

The optimized baseline already shows a concrete target: 100k narrow CPU median 23.406 ms, of which 22.402 ms is encoding. This predicts no speedup: an encoding win with a worse total or incorrect image fails. See m2-baseline.md for conditions and retained binaries. No M2 acceptance is claimed until the units and full verification pass.

## M3 boundary

Use a small shared prepared-format crate without GPU/importers, plus a separate cooker package with defaults-off gltf 1.4.1 and PNG/JPEG image decoding. Adopt dependencies only with notices and measured build impact. Runtime edits must not build the cooker. Start with the documented static embedded GLB subset, ordered primitives/material assignments, reusable meshes, UV0, colors/normals, base-color and metallic-roughness data, samplers and prepared mips. Two self-authored assets and malformed/unsupported inputs exercise the boundary. Reject unimplemented deformation, alpha modes and texture roles explicitly.

Render base color unlit; normal/MR diagnostic views verify interpretation without claiming physical lighting. Color RGB is sRGB; data channels are linear; filter color mips in linear light. Primitive-major, then instance order is explicit. Preserve mirrored material sidedness. Normal transforms need their own supported-domain tests: a 3x3 cofactor shortcut alone does not establish correct normals for arbitrary projective/singular transforms. Do not silently weaken M2's finite-transform contract or claim that an undefined normal is correct; specify material/diagnostic restrictions or fallback when implementing M3.

Prepared blobs are immutable/content-addressed with bounded validation; source/dependency content, schema, algorithm and settings define identity. Publish a complete manifest atomically. Transform-only edits reuse geometry. Measure cold cooking, warm lookup/load, invalidation, bytes, upload, CPU memory and both runtime/cooker edits separately.

## M4 boundary and experiment

Use cooker-only meshopt 0.7.0/meshoptimizer 1.3 as the first hierarchy candidate, preserving notices. A cluster, legal replacement group and IO page are distinct. Copy borrowed cooker output, preserve material/attribute boundaries and full-precision shared positions. Page-local vertices permit actual detail eviction; a pinned original vertex buffer is not streaming.

Caller-owned VirtualMesh shares residency/demand across its placements. Cuts are per instance. Start from all terminal groups, including shallow terminals; replace every coarse shard of a group together using dependency closures across owner groups. The spatial forest is not replacement ancestry. Build selection from roots over current published availability rather than maintaining mutable coarsening history. A legal refinement may publish once its required final cover is ready. CPU proof fixtures must resolve intermediate-closure page requirements and demand publication together: requesting only a final cut while requiring unrequested intermediate pages would deadlock refinement. Do not encode that inconsistency as an accepted algorithm.

One geometry total charges pinned roots, resident pages, retired pages until completion, and upload staging. Separate explicit host read/metadata caps and texture accounting remain. Require room for minimum cover and upload headroom, and minimum output capacity for all admitted instances, even offscreen. Shared asset pages must not be charged twice for placements. Failed IO preserves fallback, reports failure and avoids unbounded retry. Demand is deterministic at a fixed view and selected/demanded pages are not self-evicted.

Before runtime streaming, test chain, shared-owner, shallow-terminal and diamond DAGs plus a small closed meshopt fixture. Unlimited-budget selection must match the documented group-error rule. Constrained cuts must preserve exact-once structural coverage and closed boundaries, with bounded work. Position-edge tests require appropriate welding/seam interpretation and supplement, not replace, complete image/depth evidence.

Selected experiment: covered fallback under candidate/output/residency exhaustion, with reported quality debt (denied projected error and missing demanded pages). Register concrete asset/camera hashes before implementation. Start with 64 KiB pages, source geometry at least four times the pool, a total geometry budget max(8 MiB, roots plus four pages), host pending reads four pages, uploads at most two pages/frame, metadata/scratch 8 MiB and textures 64 MiB. Exercise candidate capacities 1/64/1024, output minimum plus 0/64/4096, below-minimum rejection, 0/100 ms IO delay, cuts, revisits and 300 settled frames. Every budget must hold, ordinary frames must not wait for pages, and settled selected pages must not reread.

Normal-budget quality targets are silhouette p99 at most one pixel, maximum two pixels, and interior material-ID disagreement at most 0.1%, plus no unexplained seam cracks/depth discontinuities and actual full-frame/motion inspection at 1080p. Forced-minimum quality loss is reported separately. Compare the same prepared DAG with a simple legal selector at equal total budgets and full-detail images. This demonstrates only the measured Renderlab contract; no Nanite superiority is established without matched current-version execution. Topology recooking, compute/software raster and arenas remain deferred unless evidence requires them. Stop after accepted M4.

## Synthesis and limits

Astra is the base. Grafts are Sol's broader arithmetic domain and incremental legal-refinement intent, simplified by Opus to selection over published pages. Root accepts Opus's ownership correction, single transform copy, bitset state, renderer identity, partial re-tests and unified geometry budget. Root explicitly preserves proof obligations for numerical safety, normals and intermediate DAG page dependencies. Local candidate, rubric, independent scores and review artifacts are under .refs/m2-m4-design. No second architecture round or new user approval is needed; implementation and measured falsification follow.
