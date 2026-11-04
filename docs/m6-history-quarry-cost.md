# M6 history: attributed quarry cost and first full soft images

**Reject the current implementation floor (decision 3), in both repetitions.** At 1x, soft motion costs 139.990 / 138.361 ms; the diagnostic without narrow resets costs 138.364 / 138.263 ms. Hard 4x motion costs 9.509 / 9.839 ms. Disabling the narrow reset does not rescue this implementation. This does not reject every ray-based design, establish an isolated four-ray cost, or select a replacement architecture.

The failed [eight-query sparse forecast](m6-history-quarry-screen.md) remains failed. This is the newly declared direct measurement in [the public plan](m6-softness-history-plan.md), applying root's corrections to the private judgment. No thresholds, ray counts/radius, sample positions, geometry, materials, normals, bias, history policy or environment were tuned. No fixture or query-sampling campaign was repeated.

## Source, bounded implementation and disposition

Worktree: `C:/Users/snowy/.t3/worktrees/renderlab/t3-054d0511`, accepted base `414e337d57a4a3cdab96fbd946198ad448b1280f`. Main was not edited. Before application, the reconstruction `implementation.patch` and durable quarry `root-implementation.patch` both hash to `d8719bc140d28a2991c4c556ebbd232e2b50e7bb111eb78eae3d656b4ff8137a`. The private-probe `complete-implementation.patch` was not used. Existing 78/78 contact image/data gates, sixteen continuous motion steps at 1x/4x, reveal/light reset and cached-cost evidence are reused with their recorded limits. The candidate's Clippy-only redundant conversion removal is included.

The existing pipelined `m6-shadows --ordinary-cost` example now supports `--soft-disk <radians>` and the declared hard/soft/diagnostic controls. A `shadow-history-cost` Cargo feature confines the no-narrow-reset seam to explicit instrumentation. Its hidden, documented diagnostic method sets uniform bit 1 and invalidates history between controls. Only the narrow-reset predicate uses that bit; surface, contradiction and global resets remain enabled. Diagnostics atomics remain disabled. The candidate always uses the bit cleared; the diagnostic is never captured or presented as a candidate. `support/lighting_controls.rs` gets the missing explicit `directional_visibility_disk: None`. Other explicit `Shadows` literals were inspected; they use defaults or already include the field.

The additional tooling includes bounded isolated cold records, full validation-only captures, and a two-second cold completion guard. Those account for more code than the judgment's roughly forty-line estimate. No general benchmark framework or rendering optimization was added. Only one release example build ran, with no compile corrections or broader viewer/cooker build.

Because the result rejects the implementation, the accepted source is restored in this checkout. The exact rejected source, both patches, executable and images remain in ignored evidence. This report is the only retained working-tree addition. Nothing was committed, pushed or opened as a PR. The shared target's previous executables were preserved before rebuilding; root's `native.exe` was never written. The shared target currently contains the measured candidate executable, so its next owner must rebuild their own source with correct mtimes.

## Registered workload and timing

Windows/Vulkan, Ryzen 7 7800X3D (16 logical processors), RTX 4070 SUPER 12 GB, NVIDIA 617.14. All frames submit 13,034,903 triangles in twelve draws at 1920x1080, original 4K texture chains, 16x anisotropy, regular full-resident quarry geometry. Cameras are the registered static pose and alternating two-pose one-centimetre X translation. Lighting is the accepted exposure-only preset (-14 stops); RLE2 intensity/yaw remain the original defaults. Estimated AS allowance is explicitly 8 GiB. Soft radius is 0.04 radians throughout.

Timing validation is OFF, release optimization is ON, and there is no window, presentation, focus change, input automation or desktop capture. All Rust/WGSL/manifest mtimes were advanced before using the warm shared target. Existing three-flight admission remains pipelined: each measured block drains once, with no per-frame blocking GPU wait, diagnostic atomics or image readback. The mesh timestamp begins before the visibility-depth prepass and ends after mesh shading. HDR tone mapping/render span and CPU/completion observations remain separate in raw records. GPU AS building occurs before this interval.

The 1x process orders H1, S, Snr in repetition zero, then Snr, S, H1 in repetition one. Each control has 24 fixed static warmup frames outside measured blocks, all retained. Snr has no measured static block. Each S/Snr installation gets one separately recorded cold frame after an ordinary hard AS-startup frame. Each measured block has 24 frames. The same binary then runs H4 motion, two blocks, in a second process in the same session; it never enables soft 4x. The final validation process captures H1, S static after eight cached frames, and S after 24 two-pose motion frames. There are five soft cold frames in total, including validation. No repeat was needed.

## Measured costs

Every cell below is a per-block GPU mesh median in milliseconds, with raw minimum and maximum in brackets; every block retains all 24 samples. The full CPU, span, completion, upload and resource rows are in `summary.json` and individual `block-*.json` files. The ignored `motion-cost.svg` plots both repetitions separately.

| Configuration | Static, repetition 0 | Static, repetition 1 | Motion, repetition 0 | Motion, repetition 1 |
|---|---:|---:|---:|---:|
| H1 hard 1x | 4.205 [4.194, 4.219] | 4.214 [4.195, 4.236] | 4.208 [4.188, 4.226] | 4.211 [4.187, 4.225] |
| S soft 1x | 6.079 [6.063, 6.259] | 6.095 [6.078, 8.357] | 139.990 [6.079, 143.698] | 138.361 [6.107, 140.953] |
| Snr diagnostic 1x | skipped | skipped | 138.364 [6.090, 142.957] | 138.263 [5.905, 142.770] |
| H4 hard 4x | skipped | skipped | 9.509 [9.430, 9.981] | 9.839 [9.548, 11.289] |

The low motion minima include exact static reuse at the first pose of a block; the medians describe the alternating views. No sample was discarded. Warmup, cold and startup records are separate from these blocks.

| Declared difference | Repetition 0, ms | Repetition 1, ms |
|---|---:|---:|
| P = S static - H1 static | 1.874944 | 1.881120 |
| F = Snr motion - H1 motion | 134.156288 | 134.051952 |
| N = S motion - Snr motion | 1.625088 | 0.098816 |

P prices the combined prepass/cache-path increment, not the prepass alone. F includes reprojection, fresh queries, remaining resets and changed control flow; it is **not** proof of an isolated four-ray floor. N is only about 1.20% / 0.074% of the S-minus-H1 increment, much smaller than the forecast's 60% query-count share. Differences of whole-path medians are not additive kernel timings or evidence of warp divergence. This run did not count actual moving queries or reset reasons, so the much larger than forecast moving cost has no finer validated hardware explanation. No compaction or recursive tuning was attempted.

The measured limiter is the soft GPU prepass/mesh interval: its approximately 138-140 ms medians accompany 0.327-0.377 ms accepted CPU service for S and 100% sampled GPU utilization during sustained soft work. S motion's start-through-drain proxy is 131.746 / 130.422 ms per frame. Its submit-to-observed completion medians are 418.404 / 412.449 ms because three frames can queue; these are not individual GPU execution or display latencies. H4's corresponding drained proxy is 9.678 / 9.982 ms. CPU retry/poll/yield overhead is retained and is not scene-preparation cost. Exact traversal, branch scheduling and reset contributions remain unmeasured.

## Cold, startup and resources

Every soft cold result is below the two-second GPU/completion stop cap. The validation cold result is separated from validation-off timings.

| Cold frame | GPU prepass + mesh, ms | Accepted CPU render, ms | Submit-to-observed completion, ms |
|---|---:|---:|---:|
| S, repetition 0 | 130.011136 | 0.631800 | 130.518600 |
| Snr, repetition 0 | 140.129280 | 0.532000 | 140.543400 |
| Snr, repetition 1 | 130.153472 | 0.472000 | 130.404200 |
| S, repetition 1 | 137.826304 | 0.446800 | 138.148300 |
| S, validation on | 387.983360 | 2.262000 | 389.509900 |

The 1x process reaches prepared-ready in 8.479 seconds, and validation in 6.263 seconds. Those startup intervals include asset loading and are not soft cold-frame time. Twelve BLAS builds and one TLAS build precede ordinary soft work. Steady blocks have no AS rebuilds, geometry/material/environment/texture/instance uploads, target creations or pipeline installations. Soft blocks create one history bind group per accepted frame, also on cached frames; hard blocks do not. Their live bind-group count remains bounded in the recorded ownership snapshots. The history's direct 2,272-byte uniform write per soft frame is not included in the existing `UploadBytes` counter; do not interpret that counter as complete history traffic. The 1x history planes plus visibility depth occupy 74,649,600 bytes (71.19 MiB), separate from ordinary HDR/depth targets.

Live named AS attribution is 911,788,752 bytes. Owned AS admission estimates, driver allocator values and physical residency are separate quantities. Preflight observes 10.52 GiB free system RAM, 26.6 GiB free on C:, 163.3 GiB on D:, 34% reported CPU load, and 3,298 MiB pre-existing GPU memory. No competing renderer/Cargo process was found; user applications remained open. GPU samples during the 1x process range 0-100% utilization, 3,298-6,500 MiB total VRAM, 210-2,820 MHz graphics clock and 29-41 C, including startup/transitions. H4 peaks at 6,713 MiB VRAM; the entire run peaks at 46 C. Samples are timestamped in the process JSON, not attributed to individual frames. Early idle-clock drift is retained. These are short session measurements on a shared desktop, not an idle-machine certificate. Afterward, GPU memory returns to 3,298 MiB and approximately 10.30 GiB system RAM is free.

## Full-image verification and limits

The validation process positively activates Khronos: `errors=0 missing_layer=0 active_layer=1`. Both timing processes record `active_layer=0`. Existing unused vertex-attribute warnings remain; there is no validation/device failure or TDR. All three 1920x1080 RGBA16F images are finite, nonnegative and unsaturated (maximum stored channel 1.4765625, 1.4765625, 1.38671875 for H1, S static and S final motion). Full PNG, PFM, RGBA16F and metadata files are preserved under `validation/`.

The complete static soft, final-motion soft and H1 images were opened and inspected, including outer silhouettes, central slab contacts, rubble and wall crevices. No gross silhouette leak, detached contact halo, broad penumbra band or widespread soft-only speckle is apparent. Jagged 1x silhouettes and granular fine scan/shadow detail remain visible; scan boundaries are already present in the hard control. Gross inspection cannot distinguish every fine visibility defect from the scan's texture/geometry. There is no independent quarry soft reference or new temporal image-error certification here. Final motion is the second pose, so its whole-image difference from static is not a same-pose error score. Snr was never captured.

The H4 anchor is hard rendering at a different sampling frequency, not equal-quality softness or the Unreal denominator. No 4x soft cold start, quarry lifecycle suite, moving-light performance claim, Unreal comparison, architecture replacement or general M6 acceptance follows. The earlier 10.018 queries/receiver, 346,520 covered receivers and 31/1,298 sampled narrow population remain the prior sparse forecast; anticipated 88.7 M cold queries are not a measured query count or frame-time conversion. No 2.12 B-query 4x cold workload ran.

## Reproduction and exact identities

Durable ignored handoff: `D:/Coding/renderlab-work/m6-history-quarry-cost-t3-054d0511/`. Worktree evidence: `.refs/m6-history-quarry-cost/`. `candidate.patch` applies to the accepted base; `tooling-only.patch` applies after the unchanged reconstruction patch. `compiled-source/` stores and verifies every compiler input in `source-identities.json`. `compiled-wgsl/` reconstructs the exact shader compositions from those byte-preserved inputs; the hard composition is also checked against capture metadata. Metadata's existing shader hash covers the hard composition, so the separate soft hashes below are required.

| Artifact | SHA-256 |
|---|---|
| Measured executable | `6bbdc9a7fad5af9901867b827962e63a53e326af76780533a0bb21a91381246a` |
| Full rejected source patch | `e0601e2b9cde373fe1f23c09ba7591722cf6c02da9c5df18c69a07be42fb94a7` |
| Tooling-only patch | `b9a47d107cdcb90ed6175850516084070ca0322fcb0bf93bf2441edf3837d33b` |
| Compiler-input hash manifest | `f4aa79e1a3294a9957ffe555f6e1607e9bb816c7662741c353652b2bff1869bd` |
| History WGSL input | `aadaee2a8164ea1e53c918b2e3848c9fa45db8c631d542117794b8a9da2f6624` |
| Soft moving 1x WGSL composition | `da96d0401149186731b3c3fc3860af590a840a9a028c4f653b3c8a0ab66922c2` |
| Soft cached 1x WGSL composition | `a8683446e73f6a8541ed33c5498d94cbfc45fe77b9321c22863a5f22ae6e0613` |
| H1 full HDR | `979dd56a5c5c79de9985cbb80ade9602ed5bdce0e735d3352cef32bf310e9b09` |
| S static full HDR | `d4e375a1eb13b3d1bec7cd11d8316e2e5742e18fbee12dd4cc745d379d633784` |
| S final-motion full HDR | `b76cc364334dc9c0e2fe9b10b0859f5e84a9a8c43dff986421543964edc61f42` |

Content identities are unchanged from the combined-cost screen and verified before execution: quarry `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`, RLE2 `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764`, limits `6e417dc134313669a653383ed2b38a605119c9c0349ecf82999e1610ff4dd121`, cameras `8a9c055339a625121360dbe93b512fcbb75afd583594a403ffe2d7d2c3606dd1`, lighting `064d75ab6d0aa626aa498de33867a01ecefa37b36efe1383d23a3d5d2cf2ef25`. Assets were neither copied into the handoff nor published.

Build: `cargo build --release --example m6-shadows --features shadow-history-cost --offline --locked -j2`, with `CARGO_TARGET_DIR=D:/Coding/renderlab-work/m4-stream-followup/target`, `CARGO_BUILD_JOBS=2`, empty `RUSTC_WRAPPER`. `run.py` and each process JSON retain the full executable command, environment, output paths and hardware observations. The common arguments match the original profiler command in [combined-rendering-cost.md](combined-rendering-cost.md); process-specific suffixes are `--samples 1 --soft-disk 0.04 --ordinary-cost --frames 24`, then `--samples 4 --ordinary-cost --frames 24`, then `--samples 1 --soft-disk 0.04 --history-captures --validate`. `analyze.py` reruns the CPU-only audit and summary. The first analysis incorrectly asserted zero persistent-object creation on the new history path; its rejected script is retained. The corrected audit accounts for the observed per-frame history bind group. No measurement was rerun or rejected to fix that analysis assertion.

Implementation from first recorded mutation to build start: 134.13 seconds (initial reading/grounding excluded). Release build: 46.20 seconds. The three native verification processes: 23.54 + 7.96 + 10.93 = 42.43 seconds; build plus native verification totals 88.63 seconds. CPU analysis, inspection and packaging time is additional and not fully instrumented. Formatting and `git diff --check` pass. No broad tests or additional builds ran.

GPU and heavy-build/shared-target ownership were explicitly **RELEASED at 2026-11-04T06:08:16.927098Z**, before CPU-only inspection/analysis/packaging. Direct process checks find no owned native/Cargo process remaining. No user application was closed. Root owns review and any future promotion.
