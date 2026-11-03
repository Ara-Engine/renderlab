# Milestone 4: bounded virtual geometry

In progress. Unit A is accepted after root reproduction. Unit B streaming and GPU lifetime implementation passes root's correctness suite, but a reproduced normal-budget silhouette failure blocks M4 acceptance. Live interaction, full quality/capacity experiments and performance/development measurements remain pending. Follow [the execution plan](milestone-4-plan.md); stop after M4. There is no demonstrated advantage over Nanite or Unreal.

## Registered content

The original registration is `measurements/m4-cook/registration-initial/registration.json`, SHA-256 `532393aff2ff8c885c014024f3a600bd601a55bdfbc2dccacf57d468a1b5fb7b`. It records the pre-selector fixture executable (`7cbed886edba90d8049e5df0319a4deceb47af47c7376f6bf26dd3f640c4a894`) and generator source (`36f7a734fa6b28fc977c96f8d2df03713eae76d79e05efe1f9a292c478551187`). Later generator formatting, lockfile and executable changes retain this registration; regeneration must reproduce the content hashes below. Cross-platform trigonometric bit identity is not promised.

The explicit 631-pose, 1920x1080 camera JSON has SHA-256 `0988e3cb375a0040a433b66edd1c445fae183a179a1c37cd5e34cedf92229bd1`. Its phases are overview (60), approach (90), sweep (120), abrupt cut (1), return (60), and settled view (300). Each pose records eye, target, up, FOV and near/far planes.

| Identity | Closed sphere | Material/thin fixture |
| --- | --- | --- |
| Geometry | Six cube faces projected onto a sphere, 288 subdivisions | Two-material wavy sheet, 704 subdivisions, 48 narrow fins |
| Prepared source SHA-256 | `ecbc974b446e2b7fd4a924be9ba07e0e2bd44261bf269fee3e362f6ce8e832be` | `7d23ec5a78a48f1e2f249e00780c87352f064d0c81b524940048ce9ba578031e` |
| Materials/textures SHA-256 | `9c4da66cc374ea93f1f7f6964732343103f5f53c0049f504ba92048c96a46524` | `1098cec660e52c4fbf18dadc42c2a643397ccf36e8b2293f26f4817fe2c9f0f2` |
| Virtual package SHA-256 | `63e6f0d39d2d0b01c1a858c3ba4a5c5704f280268ba482cd28a6fffae349da1d` | `7c9da3f0eaa6f5f477eeb690908eba639def21948fe9aeafcad7cc4cd0e13eea` |

The self-authored full-detail RLA files are independent image references. No imported artwork, Ara or Unreal source is used. The sheet is intentionally open; closed-manifold checks apply only to the sphere.

## Unit A boundary and counts

The dependency-free asset crate distinguishes replacement groups, clusters and immutable IO pages. The isolated cooker uses pinned meshopt 0.7.0 / meshoptimizer 1.3, with copied MIT notices and archive identities in [third-party notices](../THIRD_PARTY_NOTICES.md). See [format](virtual-format.md) and [cooking](virtual-cooking.md) for exact layouts, attribute weights, seam protection, numeric domain, cache publication and commands.

| Recorded quantity | Sphere | Material/thin |
| --- | ---: | ---: |
| Full-detail vertices / indices | 501,126 / 2,985,984 | 497,874 / 2,973,840 |
| Full-detail geometry bytes | 35,997,984 | 35,793,312 |
| Groups / clusters | 990 / 16,272 | 1,006 / 16,220 |
| 64 KiB pages | 1,243 | 1,236 |
| Page payload bytes | 78,808,896 | 78,336,384 |
| Padded disk page bytes | 81,461,248 | 81,002,496 |
| Metadata file / host element bytes | 802,500 / 867,544 | 800,452 / 865,252 |
| Minimum cover clusters / pinned pages | 1 / 1 | 13 / 2 |
| Normal geometry pool bytes | 8,388,608 | 8,388,608 |
| Selector scratch element envelope | 812,736 | 810,752 |

Both source geometries exceed four times the normal pool. Packed vertex records are 1,144,433 and 1,136,752; extra records include different LOD representations and page duplication, not just seams. Disk padding and host element storage are not GPU allocations or process RSS.

Selection starts from every terminal group, including shallow stuck groups. A replacement activates its complete dependency closure and publishes only when the final-cover pages are available. It does not demand intermediate pages eliminated by that transaction. Missing pages and candidate/output/page-capacity exhaustion preserve a legal complete cover and report quality debt; capacities below minimum cover reject admission. The current scalar selector scans metadata and sorts page sets per candidate. Its runtime cost is unmeasured. Its page-union budget does not yet charge GPU staging, retired resources or other instances/assets; Unit B must do so.

## CPU verification

Implementation evidence is `measurements/m4-cook/unit-a-evidence-v3/`. Root independently reviewed the selector, validation/codec, callback ownership, page packing, cache and proof boundaries and reran the complete `scripts/m4-cook-check.ps1` into `measurements/m4-cook/root-unit-a/`: exit 0, `result.txt` passed. The root run reproduced every package/content identity and the checks below, including corruption repairs on both dense fixtures. Source hashes matched before/after and a fresh post-run verification; copied license hashes match the audit. Root executable SHA-256 is `7c26c01b7cf9ca3b4c29e76afa3ad4f86a8b39ab7868d91c2ce9110d22b42b2e`, distinct from the worker binary; both runs' identical source-hash record is identified below.

- Formatting, locked workspace/all-target checks, Clippy with warnings denied, and 28 CPU tests. Ten existing GPU tests are intentionally ignored by this CPU helper.
- 58,040 small DAG/availability/capacity cases, with independent upstream predicates and exact-once replacement replay. Chain, shared-owner, shallow terminal and diamond dependencies are covered, including zero candidates, unavailable roots and below-minimum rejection.
- The small sphere enumerates all 24 legal activation sets. Dense sphere tests check 12 scalar/camera cuts and 40 constrained selections: every tested positional edge appears twice with opposite orientation. Exact float-bit position welding canonicalizes signed zero only. Dense tests are sampled, not exhaustive over all possible cuts or camera trajectories.
- Independent Python parsing checks metadata/page hashes, local indices, index partitions and upstream scalar-cut counts. Cache tests cover cold/warm identity, settings and source invalidation, actual corrupt-page rejection/repair and partial-manifest recovery.
- Regenerated fixtures/material resources/cameras must exactly match the original registration. Runtime/viewer/asset normal and build dependency trees exclude meshopt, C++ tools, importers, image codecs and SHA-256 at this checkpoint.

These checks do not prove projected one-pixel error, arbitrary affine transforms, visual seam quality, GPU lifetimes or full M4 acceptance. Projection tests cover positive uniform scales and near/interior cases; runtime needs a declared conservative transform/near policy.

## Preparation and build measurements

Worker measurements below are single debug executions per state on Windows 11 Pro 10.0.26300, Ryzen 7 7800X3D (8C/16T), Rust 1.96.0 MSVC, two Cargo jobs. Available memory was approximately 12.3-12.6 GiB. Desktop processes and OS caches were uncontrolled; the helper rejected active gameplay and quiet-window ownership. No GPU was used. These are descriptive preparation costs, not optimized renderer timings or attributed regressions.

| Internal CLI seconds | Sphere | Material/thin |
| --- | ---: | ---: |
| Cold total | 21.746689 | 29.886401 |
| Clustering/packing preparation | 15.320871 | 15.207828 |
| Final verification | 3.537828 | 3.483688 |
| Warm total, no clustering | 4.226888 | 4.234911 |
| Settings64 recook total | 21.862380 | 23.048621 |
| Roughness-only source recook | 19.593675 | 19.545395 |
| Corrupt-page repair | 23.250535 | 22.884719 |

Warm reuse still hashes source, rereads/decodes every page and republishes a manifest. It is neither renderer startup nor a no-op. Cold sampled peak working sets were 267,460,608 and 270,196,736 bytes. These are observations, not strict memory caps; upstream scratch and proof edge maps are outside the runtime pool.

Root reproduction used the same host/toolchain/profile, approximately 13.0 GiB free memory, no gameplay/quiet-window owner, and uncontrolled background/OS-cache conditions. Its single internal cold totals were 21.641839 / 21.460043 s (sphere / material-thin), preparation 15.289954 / 15.289117 s, verification 3.580636 / 3.507071 s; warm totals 4.294690 / 4.310105 s with zero clustering. Settings64 recook totals were 21.544805 / 21.087326 s, source-edit recooks 19.769138 / 19.921362 s and corruption repairs 23.258849 / 23.183224 s. These reproduce functionality and artifact identity, not a controlled speed comparison with the worker run. Root's format/check/Clippy/test commands took 0.462 / 0.418 / 0.690 / 3.759 s respectively; the complete helper additionally regenerates, cooks, verifies and repairs dense caches.

`measurements/m4-cook/build-impact/` contains isolated snapshot builds: fresh Cargo target with existing sccache wrapper 19.005342 s, five no-op builds median 0.334581 s (0.323151-0.345856), algorithm-identity edit 1.431280 s. A separate fresh target with process-local wrapper disabled took 10.834895 s and compiled Rust/C++ directly. Different runs/cache methods do not establish a cache benefit or M3-to-M4 build regression. Existing viewer evidence remains intact; runtime development-loop measurements follow integration.

The implementation run's source-hash record SHA-256 is `19bbd30166de5369979d09446d513df4616a29a13be4ee1fb1840acdda7c9fe2`, executable `128cb0b6561a5d28667eaa3308a35190385afd9140b0270fa4794d924de75906`. Failed earlier helper runs are preserved: one PowerShell exited-process status bug and one Clippy complaint were corrected before the passing run. No observed proof failure was relabeled as success.

## Remaining acceptance

Unit B must implement bounded off-thread page IO, digest validation before publication, shared per-asset residency, per-instance cuts, page-local GPU ranges, last-use retirement and complete geometry accounting. Unit C must run the registered capacity/failure/delay/settled-view experiments, inspect actual images and interaction, measure quality and optimized/development costs, obtain independent review, and push final acceptance. No M5 work is authorized here.

## Unit B implementation checkpoint: quality failure remains

The Unit B implementation now supplies the runtime work listed above. [Runtime contracts](virtual-runtime.md) describe caller-owned assets/scenes, explicit page states, bounded IO, SHA-256 verification, shared page ranges and submission-based staging/retirement. It reuses the ordinary renderer's pass, depth, materials, MSAA, timestamps and reload. Runtime adds sha2, while the asset crate stays dependency-free and importer/meshopt/C++ dependencies remain cooker-only. Different assets have explicitly separate pools; callers budget their sum.

The geometry pool reserves 64 KiB per resident/retiring page plus actual explicit staging bytes until completion. Root admission includes one staging page: sphere minimum 131,072 bytes, sheet minimum 196,608. The normal pool remains 8,388,608 bytes. IO admits four outstanding reads, two uploads per streaming tick, and a declared 327,681-byte payload element envelope. Metadata/selection reservations total at most 8 MiB per asset. CPU resource bootstrap can temporarily retain 128 MiB, separate from streaming payloads and the 64 MiB GPU texture policy. These are logical/element bounds, not driver or process-RSS certificates.

Root reproduction: `scripts/m4-runtime-check.ps1 -Gpu -Output measurements/m4-stream/root-unit-b` passed formatting, locked workspace/all-target checks, Clippy, 37 CPU tests and 13 explicitly run GPU tests. The GPU suite took 44.12 s; it is a validation-on correctness run, not a frame-time benchmark. Active Khronos validation reported zero errors/missing layers. Host GPU snapshot: RTX 4070 SUPER, driver 617.14, 2% load, 1,710 MiB used; shared desktop contention remains uncontrolled. Before/after source record SHA-256 `1bdf3dea1088a061a7c516ef4aa83c50e94a26090efee0caf2b34288d89f0081` matches the implementation run.

New boundaries include actual delayed/missing/corrupt filesystem reads and explicit recovery, shared-scene admission, mirrored/nonuniform transforms, page-local ranges versus independently uploaded raw spans, color/depth/normal and 1x/4x equivalence, failed/corrected reload, 300 settled frames without reads/uploads, and incomplete-last-use retirement. Root reviewed the selection/protection/publication and allocation/retirement seams. Comprehensive independent acceptance review is still pending.

Worker evidence `measurements/m4-stream/unit-b-final/` includes both original 631-pose replays and preserved executable identities; root has not yet repeated the complete moving trajectory. Sphere/sheet peak logical geometry was 5,497,060 / 8,388,592 bytes. The final 300 settled poses had zero read/upload/selection deltas. Retirement stress recorded 192 incomplete observations, seven evictions, one observed incomplete-last-use retirement and 65,536 retiring-byte peak. The 4096x4096 4x target serves overlap evidence, not representative performance.

Two necessary selector changes preserve the independent oracle: package-global protected page unions, and refinement from an already published legal cut. Earlier root-only flashes and a stationary 3,000-frame reread loop are preserved in worker diagnostic directories; current-cut refinement resolved the tested reread loop. Current selection still uses group-ID priority and can retain a capacity-limited cut. It has not established view/history-independent quality.

**Blocking failure:** a cold start at registered material/thin pose 270 settles with a visibly flattened orange silhouette. Root independently ran the preserved replay executable (`2ea014a6ffc8bfc92ccb8a51bacf248062d447970d325b9e3287d8e70246334e`) with warmup 300 + held 300 frames into `measurements/m4-stream/root-held-cut/`. Against the full original RLA reference, bidirectional pixel-boundary distance is p99 136.9452 px / maximum 137.6118 px, with 110,038 foreground-mask disagreements. Root inspected both complete images. The required normal-budget target remains p99 <=1 px / maximum <=2 px. The warmed trajectory's sampled near image passes; that does not excuse the cold-start failure. Group-order starvation, retained-cut rebalancing and near-view error ranking are investigation hypotheses, not established causes.

The next work fixes this failure without changing registered content, camera, normal pool or thresholds, then completes Unit C's matrix, moving image/UV/normal/material/depth checks, live viewer interaction, optimized timings, development loops and consolidated independent acceptance. This checkpoint records working streaming infrastructure and an explicit failed quality gate, not completed M4.
