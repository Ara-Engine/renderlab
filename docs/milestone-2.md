# Milestone 2: persistent instances and conservative visibility

Implementation and root verification checkpoint, 2026-11-03. Pstack comment/acceptance review remains pending; this is not yet the final milestone close. Base is `768508fe489991a5a56550eff917d586a4b49553`. M0/M1 remain complete; M3/M4 are authorized but not implemented. See [the selected design](m2-m4-design.md) and [decision trail](milestones-2-4-decisions.tsv).

## Behavior and ownership

Caller-owned `Instances` stores fixed transform slots independently of meshes. `Draw::new(&mesh, &mut instances)` preserves draw order and ascending slot order. One visible collection emits one indexed instanced draw. Static frames perform no model, geometry or visible-ID upload. Changes upload adjacent dirty matrix runs; camera changes only affect the camera and compact visible IDs. `Visibility::Unculled` supports custom shaders that move vertex positions beyond the declared bounds.

Visibility uses conservative homogeneous bounds, including finite mirrored, singular, sheared and projective matrices. Overflow uncertainty remains visible. Exact camera/bounds/mode keys reuse visibility; unchanged camera tests only edited slots. A measured camera-motion bottleneck justified a 104-byte-per-slot CPU transformed-bounds cache. This enclosing world box may retain extra objects. It is not an occlusion culler. The renderer keeps the three-flight admission gate, immediate upload submissions, asynchronous timestamps, last-valid shader reload and caller-owned color target with internal depth.

No dependency was added. Windowing remains outside the library. GPU model storage is 64 bytes per instance and ID capacity four bytes per instance, in addition to CPU models, bitsets, IDs and cached bounds. The reusable byte-packing scratch may retain 64 bytes per largest collection slot; it is not another last-uploaded matrix shadow. Each collection costs two buffers and one bind group. Very many small collections remain expensive; primitive-major grouping is the intended M3 path.

## Correctness evidence

`scripts/m2-check.ps1 -Gpu` passed formatting, workspace/all-target checks, Clippy with warnings denied, seven CPU tests and five serialized GPU tests. Final library GPU run: `measurements/m2-implementation/check-20261103-000806`, GPU suite 27.582 s. Source and exact test executable hashes are saved there. Subsequent changes affect benchmark setup notification draining and viewer rejection-title persistence; neither changes the library rendering path. Those changes passed workspace Clippy and the runs below.

The CPU suite independently evaluates floating-point clipping under varied association, rounding and subnormal treatment, and checks cache transitions, dirty runs and ID shrink/empty/re-expansion. GPU comparisons require byte-exact complete RGBA **and depth** against ordered per-slot draws: full/narrow/offscreen, near/w crossings, camera cuts, edits, changed mesh bounds, width 641, portrait aspect, coplanar winners and isolated mirror/singular/shear/projective fixtures. The isolated cases require rendered pixels so occlusion cannot hide a broken special case. Root inspected all 28 captures in `captures/m2/contact.png`, plus full-resolution special cases. These synthetic fixtures do not certify all possible shader arithmetic or production content.

The replacement test runs 2,176 frames (64 warmup plus three waves of 704), without per-frame waits, surface reconfiguration or captures. Mesh and instance replacements alternate; color/depth targets change each frame and pipelines every 32 frames. Each measured wave observed 704 prior draws still incomplete and 704 retained-overlap frames, including 352 mesh and 352 instance replacements. Peak flights: three. High-water values were identical in all three waves: 18 buffers, eight texture views, two pipelines, 602,832 buffer bytes, 3,194,880 texture bytes, 3,799,088 allocator allocated bytes and 268,435,456 reserved bytes. Draining returned all reliable counters/bytes exactly to baseline. Another 256 upload-only replacements passed.

The first stress attempt was **inconclusive**, not a lifetime failure: two uploads per iteration caused the upload gate to complete the prior draw before observation. Its failed log is retained in `check-20261102-235908`; alternating replacement restored meaningful overlap without lowering thresholds. Vulkan's known ordinary-texture object-counter bug remains: raw counts become negative. Buffer/view/byte and allocator observations support bounded retention; physical driver memory and counter-feature overhead remain unverified.

All GPU tests and both interactive sessions positively observed `VK_LAYER_KHRONOS_validation` activation and zero unexpected HAL errors. Host SDK is 1.4.363.0. A process-local empty `VK_IMPLICIT_LAYER_PATH` excludes broken implicit overlay registrations; global loader settings were unchanged. Expected invalid WGSL is caught by the scoped API validation and is not an unexpected HAL error.

## Actual viewer

`measurements/m2-window-validation` records a 10,000-cube grid plus triangle, near/overview views, culling toggle, held camera orbit, 24 intermediate resizes plus final 641x861 client extent, five minimize/restore cycles and shader failure/correction. Near-view culled/unculled client captures are byte-identical. Invalid-reload and baseline client captures are byte-identical; corrected tint visibly changes the full image. `measurements/m2-window-small` separately records visible cube rotation, correct occlusion and persistent rejection-title recovery. Both used viewer SHA-256 `14e25da3e333833b9b822e2d64821264e649aa7dc1d3ee6cb4abb2e95907b000` and exited with clean mandatory validation. No viewer remains running.

Root inspected actual window captures. The Win32 helper selects the unique owned PID/window and requires foreground agreement before input/capture. After minimize, one capture refused a foreground mismatch; a fresh lookup/capture succeeded. It did not send input to another app. Queued-resize merging was not established by this sequence. Four recorded grid input-to-present-call markers were 7.668-21.282 ms; these are event-handler proxies, not input-to-photon latency. The single-sample overview shows visible distant-grid aliasing and jagged edges. No antialiasing, lighting, shadows, transparency or production visual-quality claim is made. Basic AA remains a measured quality task for the asset/geometry work.

## Performance method

Windows, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14, Rust 1.96.0. Two Cargo jobs. Deterministic seed-zero cubes at spacing 1.5, 960x640 RGBA8/Depth32Float, one sample, no lighting or presentation. Wide camera distance is 1.8 times the grid side; narrow distance is 12. GPU pass timestamps, CPU phases, caller work, polling, Busy time, requested sleeps and stats accounting are separate. Counters are enabled; Vulkan validation is disabled in performance runs and enabled in correctness runs. No quiet-window owner or foreground game was active. Shared desktop contention was not controlled: per-launch paired snapshots ranged 0-35% GPU use and 1,744-1,759 MiB VRAM. Snapshot load can include preceding runs and does not identify its cause.

Paired sweep: `measurements/m2-perf/20261103-000917`; 72 launches, three paired repetitions per profile/count/view, alternating baseline/candidate order. Each launch uses 60 frames, excludes 20 warmup, retains 40 samples. Exact executable hashes, source hashes, commands, hardware snapshots, raw CSV and logs are retained. Baseline binaries and preparation are recorded in [m2-baseline.md](m2-baseline.md). The following values are medians of the three per-run medians. CPU is the whole accepted `try_render` call; GPU is the pass interval, not end-to-end frame time.

| Profile / instances / view | M1 CPU ms | M2 CPU ms | M1 GPU ms | M2 GPU ms | M2 visible |
| --- | ---: | ---: | ---: | ---: | ---: |
| Dev / 1k / wide | 3.064 | 0.214 | 0.01536 | 0.01126 | 1,000 |
| Dev / 10k / wide | 29.699 | 0.214 | 0.10342 | 0.02867 | 10,000 |
| Dev / 100k / wide | 296.271 | 0.240 | 3.48877 | 0.26264 | 100,000 |
| Dev / 1k / narrow | 3.105 | 0.214 | 0.01741 | 0.00915 | 96 |
| Dev / 10k / narrow | 29.527 | 0.216 | 0.10240 | 0.00819 | 96 |
| Dev / 100k / narrow | 294.324 | 0.215 | 3.45702 | 0.00819 | 117 |
| Release / 1k / wide | 0.217 | 0.032 | 0.01536 | 0.01126 | 1,000 |
| Release / 10k / wide | 2.225 | 0.046 | 0.10342 | 0.02765 | 10,000 |
| Release / 100k / wide | 22.949 | 0.042 | 1.04602 | 0.26213 | 100,000 |
| Release / 1k / narrow | 0.213 | 0.032 | 0.01741 | 0.00819 | 96 |
| Release / 10k / narrow | 2.242 | 0.032 | 0.10240 | 0.00819 | 96 |
| Release / 100k / narrow | 23.109 | 0.039 | 1.04448 | 0.00819 | 117 |

All measured static frames had zero uploads and one draw. Persistent creation and pipeline counts did not grow. Temporary encoders, attachment views and native write staging are not claimed allocation-free. The 100k M2 case reports 10.4 MB CPU bounds-cache capacity; wide final HAL buffer bytes 7,325,792, texture bytes 4,915,200, allocator allocated 12,241,968 and reserved 268,435,456. These are backend observations, not process RSS or physical GPU residency.

Very small timestamp intervals are quantized; do not infer precise microsecond improvements. CPU/GPU overlap and admission backpressure prevent interpreting accepted-call CPU as FPS. For the last 100k wide release pair, mean admission interval was 23.105 ms M1 versus 0.289 ms M2, including polling, caller work, measured accounting and Busy sleeps. M2's median interval was 0.084 ms: the mean exposes intermittent backpressure that the median hides. That run had 26 Busy render attempts and peak three flights. No presentation or scanout is included, and capture tests were separate from timed runs. This is a repeated-cube M1 comparison, not evidence of an Unreal/Nanite advantage.

## Motion and sparse updates

`measurements/m2-perf/20261103-001242` contains one 60-frame run for each profile/count/motion combination and grouped/alternating mixed geometry. Setup notification draining was corrected after the initial alternating case overflowed the bounded completion event queue; the failed `20261103-001206` run remains saved. This was a fixture bug, not dropped GPU work. The paired static run predates that setup-only fix; its saved executable/source identities remain authoritative.

| 100k narrow workload | Dev whole renderer ms | Release whole renderer ms | Release caller ms | Model bytes/frame |
| --- | ---: | ---: | ---: | ---: |
| Camera movement | 19.237 | 0.898 | 0.0003 | 0 |
| 1% strided translations | 8.463 | 1.430 | 0.0164 | 64,000 |
| 1% contiguous translations | 2.503 | 0.140 | 0.0152 | 64,000 |

Release camera visibility cost was 0.829 ms; visible-count median 108. The transformed-bounds pilot reduced repeated arithmetic while retaining exact image tests; pre-cache evidence remains under `20261103-000300`, post-cache under `20261103-000849`. Strided updates make 1,000 native write calls versus one for a contiguous run: this remains the main sparse-update cost and is not hidden by averaging. Each motion's GPU-pass median was 0.008192 ms. Development camera motion at 100k still exceeds a 60 Hz CPU budget; optimized performance and practical smaller-scene development remain distinct.

At 1k mixed placements, grouping the two primitive types uses two draws; preserving alternating order uses 1,000 collections/draws. Release CPU medians were 0.075 and 0.506 ms respectively; GPU 0.00904 and 0.01843 ms. Development CPU medians were 0.219 and 5.007 ms. Grouping explicitly changes draw order. This establishes a known limit for fragmented scenes, not a license to reorder arbitrary equal-depth geometry.

## Development loop

`measurements/dev-loop/20261103-001415/results.json`, two-cube/triangle default viewer, dev profile, two jobs, shared desktop, debug Vulkan validation enabled with the same implicit-layer isolation. Pre-run GPU snapshot: 19% utilization, 1,786 MiB VRAM. Five startup/shader runs, five no-op builds; one fresh-target and one representative library edit. Exact Rust/shader bytes were restored, hashes and launched executable identities recorded. Fresh target means downloads and OS/compiler caches remained warm.

| Measurement | Result |
| --- | --- |
| Fresh target build | 22.624 s |
| No-op build | Median 0.351 s; range 0.347-0.355 s |
| Library edit build | 1.693 s |
| Library edit to externally observed viewer log | 3.059 s |
| Startup to present-call / completion-observed | 562.662-581.183 / 565.573-583.509 ms |
| Shader compile | Median 1.872 ms; range 1.845-13.827 ms |
| Shader write to present-call / completion-observed | 28.826-74.949 / 30.488-76.803 ms |

The 50 ms file-poll phase and shared-desktop/compiler outlier contribute to shader latency. These samples do not establish a controlled regression versus M1. Startup includes adapter/device and surface setup; validation overhead is enabled. Completion-observed and presentation-call proxies are not scanout or GPU pass measurements. All six launches positively logged validation activation and zero errors. The helper now writes milestone-neutral `measurements/dev-loop` paths and records the full runtime source hash set.

## Reproduce and remaining limits

Run `scripts/m2-check.ps1 -Gpu` only with GPU ownership clear. Build `cargo build --locked -j2 --example scene-scale`, repeat with `--release`, then run `scene-scale 100000 60 wide` or `narrow`, adding `--motion=camera|sparse|contiguous` or `--geometry=grouped|alternating`. Add `--validate` for mandatory-layer diagnostics. Use `renderlab-viewer --instances 10000 --validate --log-frames --seconds 60` for interaction. `scripts/measure-dev-loop.ps1 -Cold -Runs 5` temporarily edits source: serialize it with all source work. Preserve comparison binaries before changing code.

Only Windows/Vulkan on this host is certified. Arbitrary shader displacement requires Unculled. Device loss remains restart/recreate-only. Physical driver memory, counter overhead, statistical confidence across machines/content and production quality remain unverified. License choice is still pending. M3 assets/materials and M4 actual geometry streaming remain separate unimplemented milestones. Final independent review and checkpoint closure follow; no acceptance failure is silently waived.

Pstack Comment Sicko recommended 27 comment-line deletions in 21 blocks and six naming/type/test-structure targets. Root accepted the deletions, introduced a named `VisibilityUpdate`, renamed the homogeneous world bounds and independent oracle buffers, extracted equal-depth/adjacent-write assertion helpers, and named oracle cases and the alternating replacement policy. Public API/shader contracts and the external WGSL arithmetic constraint remain. No comment was restored; no review rerun, architecture round, permission-dependent constraint encoding or open comment finding was needed. Numerical arithmetic, submission behavior and assertions remain unchanged. The recorded performance and development measurements predate these naming/type/helper changes; their saved hashes identify the actual measured code. Final verification at `measurements/m2-implementation/check-20261103-001801` passed formatting, workspace/all-target checks, Clippy, seven CPU and five GPU tests on the refactored source; source hashes and exact executable hashes are recorded. This supersedes the earlier GPU run as the checkpoint's correctness identity.
