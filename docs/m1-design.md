# Milestone 1 selected design

Status: selected for implementation, not yet verified. Pstack architect compared independent Sol and Astra sketches. Opus 5.5 independently scored Astra 12/15 and Sol 11/15 and selected Astra. The root agrees after reading both proposals and the review. Local proposal/review artifacts are under .refs/m1-design/.

## Caller and ownership

Keep the library independent of winit. The caller owns immutable Mesh values and passes borrowed Draw values with model matrices. Renderer owns the device/queue, active pipeline, camera uniform, model instance buffer, depth cache, and three submission records. Color is a borrowed wgpu::Texture, from the surface or a headless caller. The renderer validates color compatibility and derives depth dimensions from it.

Use ordinary drop for replacement. wgpu retains submitted resources until completion. Do not add a resource registry, generational IDs, manual retirement queue, public depth/target types or a borrowed Frame protocol. Raw wgpu is an intentional narrow setup/target boundary. External queue clones/submissions fall outside this renderer's accounting contract.

## API contract

Use the recommended signatures in Opus's review. Renderer has new, try_upload_mesh, try_render, reload, poll, stats, device, and present. try_upload_mesh and try_render return None only after a nonblocking collection confirms capacity remains full. They must not perform upload/allocation work on Busy. Limits bound mesh payload and draw count against real device limits. Mesh upload flushes immediately through an admitted empty submit, including mapped-at-creation uploads if that helper is used. Upload admission leaves one of three submission slots available for a draw.

Vertex stores position and linear RGB. MeshData borrows vertices/u32 indices. Draw borrows Mesh and contains a glam Mat4. Camera validates look-at and perspective parameters and derives aspect from the color target. Use right-handed view, 0..1 projection depth, Depth32Float, clear 1, Less with depth writes. Transform matrices use one persistent instance vertex buffer and ordinal first_instance. Byte-exact shadow comparisons produce no writes on static frames; coalesce adjacent dirty model records. One changed model is 64 bytes. Keep one camera binding and one fixed shader interface.

Add glam for matrix conventions and enable wgpu counters. No bytemuck, new crate boundary, asset importer, graph, ECS or async runtime. Shader reload remains synchronous and transactional initially. Scope validation/OOM/internal errors as one operation, not per object. Keep the last valid pipeline/revision on rejected syntax or layout; fault/device loss stops new work with a diagnostic. A reload worker is conditional on measured p95 event-thread stalls above 50 ms or visible input stalls.

## Bounded execution and measurements

Each of three flights owns completion metadata and optional timestamp readback. One query set has six entries; one resolve buffer uses aligned 256-byte regions; each flight has a 16-byte MAP_READ readback buffer. Register completion and map actions before submit and retain their cells across callbacks. Collect with PollType::Poll. Successful mapping requires drop-view then unmap before reuse. Failed maps must not unmap; disable timing and report the error. Optional unsupported timestamps report unavailable rather than zero time. No separate timing ring or RingFull state is needed.

Internal collection queues at most 16 undelivered observations, dropping oldest with an explicit count. poll drains them, including while minimized. Report CPU preparation/upload staging, encoding, submit and poll separately from GPU pass timestamps. Viewer reports configure/acquire/present wall durations separately; those may include backend waits. Frame/revision-tagged completion observations support startup/reload proxies without explicit ordinary-frame waits. Only setup/capture/final-drain tests may use finite labeled waits.

Observe three layers: Renderlab created/dropped/live/peak ownership; actual wgpu HAL object counts and allocator allocated/reserved bytes; separately labeled external process/driver memory. Do not present a decremented app counter as proof of backing-resource release. The accepted instrumentation must detect retained growth if callbacks/resource cleanup are broken.

## Verification contract

- Full-image draw-order equality for noncoplanar occluders plus known-front probes; camera/model movement and non-aligned width-641 capture; correct aspect/depth across target replacement.
- Bad syntax/binding/vertex reload preserves full image and active revision; correction and source restoration recover. Verify pipeline attempts, valid creations and publications distinctly.
- 300 warm static frames with no persistent creation, upload bytes or write calls. One moving model uploads exactly 64 bytes per accepted frame, with no geometry upload/pipeline creation. Camera-only movement uploads only camera data.
- At least 2,048 admitted offscreen stress frames with repeated meshes, target extents and pipelines, no per-frame wait/configure/capture. Show incomplete prior submissions and overlapping retained HAL resources at replacement points; otherwise overlap remains inconclusive. Perform at least three equal post-warmup waves, inspect HAL counts and allocator allocation plateaus, then a finite final drain and full-image capture. Counts and driver observations must be reported with their actual scopes.
- Busy calls have no upload/allocation side effects. Fault injection into a foreign-device texture must produce a diagnostic without an uncaptured panic; further calls do no work. Distinguish injected device/surface paths from real hardware loss.
- Real viewer movement, resize bursts, minimize/restore, rejected shader/correction and event responsiveness. Coalesce resize, suspend zero/occluded frames while polling, and handle suboptimal/outdated/lost surfaces explicitly.
- Focused checks and practical measured build/startup/shader loops with exact hardware/build/scene/contention labels. Preserve M0 as historical evidence with its old measurement definition.

## Synthesis and rejected machinery

Base is Astra's immediate gated upload and caller-owned meshes. Grafts from Sol are the instance-transform buffer, raw color boundary, failed-map handling, width-641 capture and stress details, and unique measurement anchors. Opus removes public targets, separate timing ring, speculative reload worker, owner stamps, arbitrary per-frame quotas and computed retention formulas. Internal depth and combined timing/submission slots further reduce caller coordination.

Principles that changed the design: Model the Domain makes flight state explicit; Separate Before Serializing Shared State isolates proposals and gives one implementation owner; Laziness Protocol removes the Frame-abandonment recovery path and unproven worker complexity; Prove It Works adds retained HAL/allocator evidence rather than trusting app counters. These decisions remain falsifiable during implementation. Fix small API gaps directly; revisit the architecture only if repeated implementation friction disproves its shape.
