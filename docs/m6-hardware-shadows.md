# Optional hardware directional shadows

This partial M6 unit moves the verified ray-query experiment into ordinary regular-geometry rendering. The twelve-scan quarry clears the three registered false-shadow controls and preserves the contact control. The full image differs from the verified prototype by at most one display code. M6 remains incomplete; this checkpoint does not establish Unreal parity or rendering throughput.

## API and supported content

`Renderer::new_with_hardware_shadows` adds `HardwareShadowOptions` to the existing renderer construction arguments. Existing constructors retain raster shadows and do not require ray-query features. Device creation belongs to the caller, which must request `EXPERIMENTAL_RAY_QUERY`, negotiate acceleration-structure limits, and acknowledge wgpu's experimental safety contract. The library adds no unsafe operation or dependency.

The path supports regular immutable meshes, affine transforms, opaque double-sided materials and directional hard shadows. The original checkpoint uses one sample per pixel. The [4x follow-up](m6-hardware-msaa.md) adds per-sample receiver queries and records its combined verification and review status. Unsupported material or transform inputs fail before frame writes. Local shadow requests fail atomically; unshadowed point and spot lights remain usable. Shadowed virtual rendering fails before streaming advances. It never substitutes full source geometry for a virtual cut.

`examples/m6-shadows.rs --hardware-shadows --ack-experimental-wgpu-risk` exercises the path on the owned fixture or an existing prepared scan. `--hardware-max-estimated-bytes <bytes|none>` sets the estimate policy. In hardware mode the example removes the fixture's local lights and marks its objects double-sided. Its fixture images therefore cannot be compared directly with raster fixture images. Prepared scans pass through unchanged for library validation.

The viewer does not select this path automatically. Existing shader edits still use `reload(&str)`. The optional query helper is embedded in Rust and requires rebuilding when that helper changes. Changes to the marked affine vertex functions are rejected because the acceleration structures contain original vertex positions. Shader compilation and installation remain transactional. Hardware renderers report the hash of the composed shader, which differs from the on-disk scene file's hash.

## Ownership and frame work

Each regular mesh upload builds one immutable bottom-level acceleration structure, or BLAS, using the existing vertex and index buffers. Those buffers are shared through owned handles, without a second geometry copy. Upload and build submit together through the existing admission gate.

A top-level acceleration structure, or TLAS, contains every submitted instance, including camera-invisible casters. Exact mesh identities and transform bits determine reuse. Camera and light changes reuse the scene structure. Placement changes build a new TLAS over existing BLASes; wgpu 30 does not support TLAS refitting. Prior generations and build inputs remain owned until their last submission completes. Disabling all shadow requests retires the TLAS. A BLAS and its source buffers remain owned while either a caller's mesh or an active or retained TLAS references them.

Visibility queries run inside ordinary lit fragment shading. The production path has no diagnostic receiver textures, visibility-mask pass, hit readback or ordinary-frame completion wait. Ray origins use the light-facing derivative geometric normal, independent of normal maps. Bias magnitude and finite directional coverage retain the raster planner's settings. Hardware shadows use sampling records and a one-texel fallback texture instead of allocating the raster shadow maps.

The optional acceleration structure uses group 0 binding 2. The original hardware unit leaves environment bindings and the lighting uniform unchanged. The subsequent [RLE2 integration](m5-independent-integration.md) uses group 3 binding 9 for its second environment bank and passes combined device verification.

## Memory and observations

`HardwareShadowOptions::max_estimated_bytes` defaults to 2 GiB and accepts `None` to disable that policy. Admission includes owned acceleration generations and pending build scratch estimates. Structural and device checks remain mandatory. Safe wgpu exposes no preallocation size query, so this estimate is not a physical residency cap. At 256 estimated bytes per triangle, the default admits about 8.4 million steady-state BLAS triangles, fewer while build scratch is pending. The quarry explicitly uses an 8 GiB estimate policy.

`hardware_shadow_stats()` separates build counts, active placements, retained generations, estimated bytes and optional allocator observations. The quarry creates twelve BLASes. Named allocator observations total 911,788,544 bytes for the active BLASes and TLAS, while the retained BLAS estimate is 3,336,935,168 bytes. Scratch observations cover named wgpu scratch allocations after submission and can include other internal work. They do not establish peak total scratch residency. The Vulkan HAL acceleration-structure counter remains unreliable on this host.

GPU mesh timestamps include visibility queries and exclude the preceding acceleration build commands. This unit does not provide an isolated query cost or an acceleration-build timing claim. The synchronized capture tool is unsuitable for pipelined throughput claims.

## Verification and identity

The implementation patch is based on `50b5d086d389d21b4661236788af9191e9d64136`, SHA-256 `a664a2057868f117c4c6e8e6af02238284ad94fa090110c21c1a5e771ec7b52c`. Root applied it and verified all sixteen normalized source hashes and both retained executable hashes. Independent Opus review accepts this regular 1x directional subset with no code blocker. Root then corrected documentation and source comments only; a comparison against the worker source confirms unchanged executable expressions.

Affected all-target checking and Clippy pass. The worker's CPU run passes 56 tests. Four focused hardware GPU tests cover geometric-normal orientation, offscreen blockers, transform updates, static reuse, finite coverage, busy admission, unsupported inputs, normal maps and retirement. A final reload case rejects malformed syntax with composition markers intact, preserves unshadowed rendering and installs the corrected source. Occlusion after reload is not separately asserted. Four retained raster shadow GPU tests pass at their existing 1x and 4x settings. Vulkan validation is positively active with zero errors. Unused shadow vertex-attribute performance warnings remain in the logs.

Only the orientation/reload test ran on the final retained test binary. The CPU run, four raster shadow tests, and replacement, unsupported-input and normal-map hardware tests ran on earlier revisions. Those preceded the final test-only reload assertion and shader-hash/capture-metadata edits. Final Clippy covers the final worker source. The raster normal-map shadow test was not rerun after its body moved into the shared helper; the reviewer checked that extraction by source inspection.

The replacement test observes seven frames with an actually incomplete earlier submission. Retained scene generations and pending builds stay within the three-flight bound. All owned acceleration and pending-scratch charges return to zero after disabling, dropping meshes and draining. This is ownership and in-flight evidence, not a general driver leak certification.

Root inspected the complete fixture, quarry enabled and quarry disabled images. The worker also inspected the one-centimetre camera perturbation. All ten retained HDR captures are finite and unsaturated. The quarry uses the unchanged private asset `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`, containing twelve meshes and 13,034,903 triangles, at 1920x1080 with 1x samples, original 4K mip chains and 16x anisotropy. No scan recook or geometry simplification occurred.

The three clear control pixels equal the same-run disabled image exactly. The contact remains RGB `[62, 58, 59]` against disabled `[162, 147, 133]`. All 316,002 pixels classified clear by the earlier prototype match the new disabled image, and background is unchanged. Root independently recomputed the selected controls and the complete image's maximum one-code difference from the prototype. This reuses the prototype's source-ray evidence; the production path adds no diagnostic ray readback or whole-scene oracle.

The retained evidence is local and ignored under `C:/Users/snowy/.t3/worktrees/renderlab/t3-a4b60057/.refs/m6-hardware-production/`. It includes `handoff.txt`, `source-identities.json`, logs, `quarry-controls.json`, resource observations and binaries. Root's reproducible source/image audit is `.refs/m6-hardware-integration/audit.py` with `root-audit.json`.

The host is Windows/Vulkan on Ryzen 7 7800X3D and RTX 4070 SUPER with NVIDIA 617.14. GPU utilization was already 36% at admission and contention was uncontrolled. Final example build time was 14.538 s. Fixture and quarry evidence commands took 1.601 s and 10.001 s respectively, including setup and synchronized captures. Total recorded verification command time was 137.583 s, including superseded attempts. Authoring time was not separately measured. These are development and verification costs, not frame timings.

## Remaining M6 work

The raster path's scan undersampling remains a known quality failure. Hardware shadows provide a separate supported option for this bounded subset. Single-sided materials, virtual-cut integration, local ray shadows, softness and broad motion/contact acceptance remain open. The 4x follow-up is device-verified and independently accepted as partial M6. Native interaction and development-loop timings were not rerun for this unit. The final matched Unreal quality and performance gates remain unmet.

The 4x follow-up normalizes shader line endings in the protected affine comparison, gives allocator labels a renderer-specific nonce, and renames `actual_as_bytes` to `observed_as_bytes`. Empty-scene and re-enable device coverage remain unverified. TLAS preparation currently depends on every later frame failure being terminal; adding a recoverable failure there requires explicit pending-build state or installation after submission. These findings do not invalidate the tested checkpoint.
