# Fresh compute shadow verification

The bounded fresh-compute unit passes its ordered quality checks. All 14 previously ignored shadow GPU tests pass with active Khronos validation and zero errors. No rendering fix or threshold change was required. One validation-off actual-quarry frame measures **25.598 ms GPU**, including receiver rasterization, compute visibility, shading and tone mapping. This remains above the historical hard-4x cost and does not complete M6 or the matched Unreal contract.

## Integration and identity

Source `c768a1d47a5f6635b207393b348ebdc6dc2ddd26`, based on `611109b`, was cherry-picked onto `de73c13` as `60e7af897d30de2e29c2bd7b6c277f4a14f03053` on `t3/m6-shadow-gpu-verification`. The sole conflict was a `Flight` initializer: retain main's `submission` and add `fresh: false`. The complete diff against `de73c13` in `src/virtual_gpu.rs` is exactly two `fresh: false` initializers; `src/virtual_geometry/` is unchanged. Page pooling semantics are preserved.

Subsequent changes affect only the fresh-shadow test harness: explicit validation-off timing mode, startup/reload observations, separate scene-draw CPU timing, and exact comparison of timing/reloaded HDR against the already validated quarry reference. Quality thresholds and production renderer/shader inputs remain identical. Compiler-input manifests confirm `src/fresh_shadow_tests.rs` is the only input difference between the two binaries.

Evidence: `D:/Coding/renderlab-work/m6-fresh-verify-t3-4d214b85/`. Quality binary SHA-256: `5b532c13e63111376d998b76ac9c382ccbdc3975172117d79dd437b9125158f7`. Timing binary: `da246013574b31e2efea9383c008bab8721467e4674d5944e45345b451b0a9e4`. The final commit and complete patch identity are recorded in `handoff.json`; `compiled-source/`, `compiled-source-timing/`, logs and copied executables retain exact inputs.

## Ordered quality gates

The thin/contact test ran first: 256x192, five views at X offsets 0, 0.01, 0.02, 0, -0.01. Every pose covers 49,152 receivers. Partial-shadow populations are 897, 752, 897, 897, 752. Every covered visibility count matches the fresh fragment reference exactly; every HDR pixel and depth winner matches. Uncovered visibility outputs are checked against 256, and changed-view visibility is nonvacuous. The return image is byte-identical to the first image. Removed-blocker ownership, disabled-target release and rejected reload boundaries pass.

The other 12 previously skipped tests then passed individually: two environment/normal regular-virtual suites; five hardware shadow orientation, MSAA, geometric-origin, unsupported/featureless and queued-replacement tests; the mapped-normal shadow-plane test; and four raster shadow admission, offscreen/reload, planar-motion and virtual-cut tests. Every process positively observed active Khronos validation with zero errors and no missing layer.

Actual quarry ran last at 1920x1080, 1x, 0.04-radian disk, 256 unchanged closest-hit directions, regular opaque double-sided geometry, twelve placements, 13,034,903 triangles, original 4K texture chains, 16x anisotropy, exposure -14 and the unchanged RLE2 environment. Asset, cameras, lighting, limits and environment SHA-256 identities match the recorded probe inputs.

| Quarry pose | Covered receivers | Partial shadows | HDR/count/depth differences |
|---|---:|---:|---:|
| Original | 346,520 | 8,547 | 0 / 0 / 0 |
| Forward 1 cm | 345,883 | 8,603 | 0 / 0 / 0 |
| Return | 346,520 | 8,547 | 0 / 0 / 0 |

Validation reports `errors=0 missing_layer=0 active_layer=1`. The original/return HDR images are byte-identical. Independent CPU analysis rechecks all eight complete HDR images and all covered visibility counts, including finite values.

Verified by opening the five-pose fixture sheet and all three complete quarry images, both as HDR visualizations and using the renderer's Reinhard-plus-sRGB display mapping. Files: `fixture/display-motion-sheet.png` and `quarry/pose-{0,1,2}-display.png`; raw `.rgba16f`, `.f32`, `.u32` and equivalence JSON files remain adjacent. Inspected outer silhouettes, thin projected blockers, contact/partial-shadow transitions, slab contacts, rubble and wall crevices. No compute-specific missing shadow, detached halo, silhouette change or stale return-view result was observed. Fine 1x aliasing and scan detail remain. These are finite-reference equivalence checks, not continuous ground truth, transparency coverage or Unreal acceptance.

## One complete frame and development latency

The separate optimized timing process disables Vulkan validation, installs no reference pipeline, renders one cold preparation frame, then measures exactly one forward quarry frame. No sample campaign or hard-4x rerun occurred. It subsequently checks full HDR equality to the validation reference, reloads unchanged source, and checks equality again. Both checks pass. The validation process also retains its single 23.740 ms frame observation separately; it is not the performance denominator.

| Validation-off observation | Milliseconds |
|---|---:|
| Receiver raster | 3.207168 |
| Fresh closest-hit compute | 18.623168 |
| Ordinary shading | 3.696640 |
| HDR tone mapping | 0.053248 |
| Directly timestamped render span | 25.597952 |
| CPU scene-draw preparation | 0.000600 |
| CPU draw preparation through admission | 0.190100 |
| Start through observed GPU completion | 26.274100 |

GPU span includes intervening GPU work/barriers and excludes uploads, initial acceleration-structure builds, readbacks, surface acquisition and OS presentation. The headless completion interval includes required submission work; there is no window or display-latency claim. The frame reports twelve draws and all 13,034,903 triangles. CPU encoding is 0.1128 ms, submission 0.0439 ms, shadow planning/upload 0.0118 ms; the raw `FrameReport` preserves the other disjoint fields. Steady uploads are 336 shadow bytes, 12 lighting bytes and 64 camera bytes in three writes; no mesh, texture, material, environment or instance upload occurs. No persistent-resource counter or target-creation counter changes during this frame; the ordinary temporary attachment view is created/dropped.

The recorded [fragment probe](m6-shadow-compute-probe.md) medians are 139.055 / 130.834 ms for fragment mesh/shading alone. The [history cost screen](m6-history-quarry-cost.md) records hard-4x mesh medians 9.509 / 9.839 ms. The new complete span is substantially below those historical fragment intervals and above the hard-4x intervals. These are different sessions and interval definitions, and hard-4x has different shadow/sampling quality. One shared-desktop sample establishes a baseline observation, not a robust paired speedup or milestone pass.

Startup to prepared-ready: 7,957.7821 ms. Startup to first completed offscreen image: 7,988.8632 ms, including instance/device setup 322.2311 ms and all renderer/pipeline setup 4,584.5608 ms. Cold frame start to completion: 30.8446 ms; its GPU render span is 26.532864 ms. Unchanged-source reload compiles all renderer pipelines, including fresh compute, in 11.3172 ms and reaches a completed image in 38.4832 ms. These are one-process/one-reload observations and completion proxies, not scanout, isolated fresh-pipeline compilation, Rust edit-to-running latency or a latency improvement claim. The multi-second startup setup remains a development-latency concern.

## Owned resources and contention

Fresh attachments occupy 74,649,600 logical bytes; total renderer HDR/depth/fresh targets occupy 99,532,800 bytes. The caller's 8,294,400-byte output target is excluded from renderer ownership. Live texture payload is 1,073,741,820 bytes, environment 102,260,784, materials 624, lights 112, shadow records 452 and BRDF LUT 65,536. Geometry upload payload is 474,842,772 bytes. AS admission estimate is 3,336,942,336 bytes, positively attributed AS allocation is 911,788,544 bytes; scratch estimate is zero live and 383,873,664 peak. These are distinct counters, not physical residency. `owned-resources.json` retains named before/after counts: 82 buffers, 26 textures/views, 31 bind groups, 9 bind layouts, 8 pipeline layouts, 11 render pipelines, 1 compute pipeline, 1 query set, 40 samplers, 12 BLAS and 1 TLAS live. The bound compute group is reused on the measured frame.

Windows/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14. All GPU work was headless. Every native preflight checked processes/foreground state; foreground was Helium and no running game was found. User applications remained open. Two preflights deferred work for transient Cargo/rustc processes; no foreign process was stopped. Initial VRAM was about 8,112 MiB for quality and 7,232 MiB for timing. Quality-quarry samples span 2-100% utilization, 8,112-10,073 MiB total VRAM, 810-2,820 MHz and 42-52 C. Timing samples span 1-58%, 7,229-10,011 MiB, 540-2,535 MHz and 43-45 C, including startup and exit. The short measured frame is not resolved by the 0.5-second contention sampler. This is not an idle-machine certificate.

## Reproduction, failures and time

Both test binaries use `cargo test -p renderlab --release --lib --no-run --offline --locked -j 2`, the warm target `D:/Coding/renderlab-work/m4-stream-followup/target`, `CARGO_BUILD_JOBS=2`, and empty `RUSTC_WRAPPER`. The experimental acknowledgment and Vulkan SDK layer environment are retained in each run JSON. `run.py` stages `fixture`, `regression`, `quarry`, and `timing` retain exact commands and manifests. Tests use `--ignored --exact --nocapture --test-threads=1`. The changed-package release Clippy check with `--lib --tests -D warnings`, formatting, `git diff --check`, and 29 CPU shadow tests pass.

No quality/validation failure occurred. One 0.064-second regression attempt failed before GPU creation because the fixture/environment paths were unset. It is preserved as `regression-0`; the corrected run uses the existing corrected normal fixture and RLE2 field. Existing unused vertex-attribute performance warnings remain. There were no device errors or TDRs.

Verification time: successful native processes **50.191 s** (15 processes: 14 quality tests plus one timing process); release builds **51.688 s** (26.770 + 24.917); Clippy **22.937 s**; CPU shadow tests **0.02 s**; CPU image/resource analysis **0.940 s**. Preflights, image inspection and packaging are additional. Implementation is the minimal cherry-pick resolution and test-only instrumentation, with no production rendering fixes; isolated editing time was not clocked. It overlaps reading/verification and must not be inferred by subtracting checks from wall time. Source freezes are 11:47:28 UTC for quality and 11:53:19 UTC for instrumentation. The originating implementation worker's 1,123-second overlapping preparation span remains separate in its original handoff.

GPU, heavy-build and shared-target ownership was **released at 2026-11-04T11:55:37.2804701Z**, with no owned native/Cargo process remaining. Subsequent inspection and packaging were CPU-only. The next shared-target owner must rebuild from their own source. This branch is a local checkpoint for root review; main was not modified or pushed.
