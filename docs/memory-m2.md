# M2: lossless geometry storage

2026-11-07 UTC. Base `b183023`, branch `t3/m2-memory`.

Shared immutable cluster-instance/group tables remove **13.466 MiB of live no-RT
allocations**. **Reserved memory does not fall**: the space becomes allocator
slack. RT and traced Virtual have only one consumer and save nothing. The larger
lossless position-palette experiment was rejected after its reserved footprint
grew. Textures, source/LOD indices, vertices and walk records remain unchanged.
M1's retained allocator policy remains the default.

## Measure first

The baseline is [M1's per-label inventory](memory-m1-after.csv), backed by individual
allocation/block CSVs in `E:/renderlab-work/m1-memory/retained/`. Reserved memory is
3,439.676 / 2,641.247 / 3,258.055 MiB (no-RT / RT / traced Virtual). This unit reuses
`scripts/memory-m1.py`, `scripts/memory-wddm.ps1`, and the guarded `scripts/n8-run.py`.
New measurements and intermediate output live in `E:/renderlab-work/m2-memory/`.

## Before and after

MiB = 2^20 bytes. Allocator reservation includes unused block tails; WDDM is the
sampled process-local dedicated usage and includes driver costs. They are not
interchangeable. Warm figures are the same at both poses.

| Path | Live before → after | Reserved before → after | WDDM before → after | Recorded Unreal DXGI |
|---|---:|---:|---:|---:|
| No-RT, raster LOD + Virtual | 3,357.763 → **3,344.297** | 3,439.676 → **3,439.676** | 3,512.3 → 3,512.3 | VSM no-RT: 3,912 |
| Traced + Interactive RT | 2,509.556 → 2,509.556 | 2,641.247 → 2,641.247 | 2,716.3 → 2,716.3 | HRT: 5,410 |
| Traced + Virtual | 3,164.990 → 3,164.990 | 3,258.055 → 3,258.055 | 3,333.1 → 3,383.0 | VSM RT enabled: 3,946 |

Thus **reserved savings are 0 / 0 / 0 MiB**. No-RT slack grows from 81.913 to
95.380 MiB. The roughly 50 MiB hybrid WDDM shift is within M1/N8h's observed
approximately ±70 MiB process variation; its allocator footprint is identical.
Sampling is about every 1.5 seconds, not an instantaneous peak certificate.

The Unreal 5.7.4 CL51494982 measurements are reused from [N8h](non-rt-n8h.md).
Same twelve scans, 13,034,903 source triangles, uncompressed 4K base colors with
mips, 1920×1080. This is the recorded direct-light subset, excluding
Lumen/GI/SSR/fog. Traced Virtual has no exact Unreal VSM counterpart because it
also owns 335.6 MiB of primary-visibility BLAS. This unit establishes no new
whole-engine quality or speed superiority.

Allocated breakdown; unchanged groups show one number. All values are MiB.
The [after CSV](memory-m2-after.csv) preserves every allocator label; the harness
also records individual allocations, offsets, block membership, usage and lifetime.

| Resource | No-RT before → after | RT | Traced Virtual |
|---|---:|---:|---:|
| Material textures/mips | 1,024.023 | 1,024.023 | 1,024.023 |
| Source/LOD indices and adjacency | 596.291 | 298.347 | 298.347 |
| Source vertices | 303.673 | 303.674 | 303.674 |
| Cluster geometry words | 346.811 | 226.911 | 226.911 |
| Cluster-instance/group tables | 26.932 → **13.466** | 0 | 6.397 |
| Walk records/build data | 447.549 | 0 | 447.549 |
| Ring horizons | 99.397 | 0 | 49.740 |
| Other Virtual data | 237.518 | 0 | 230.850 |
| BLAS/TLAS | 0 | 335.608 | 335.608 |
| Environment cubes | 97.527 | 97.527 | 97.527 |
| Other allocations | 178.042 | 223.465 | 144.364 |

Consumer audit:

| Representation | Live consumers | Safe to free after preparation? |
|---|---|---|
| Source vertices, 48 bytes each | Forward/shadow raster, BLAS build, primary/film receiver reconstruction, programmable shading, virtual-shadow fallback and walk-record construction | No. BLAS compaction does not remove the shading/fallback reads. |
| Source indices | The same consumers, plus primitive identity | No. |
| Appended LOD indices | Primary hit reconstruction, programmable shading, receiver edge repair, virtual-shadow receiver handling | No. Cluster existence does not make this suffix build-only. |
| Adjacency suffix | Primary repair and virtual-shadow exact 1-ring handling; appended triangles have cut-dependent within-cluster adjacency | No. Omitting it invokes a different, potentially expensive fallback. |
| Cluster position words | Primary software/hardware raster and resolve; virtual-shadow caster software/hardware raster | Every referenced position remains necessary, but bit-identical palettes can overlap. |
| Cluster packed local triangles / primitive tables | Those raster paths and exact primitive-number reconstruction | No. Local indices are already u8; quarry global vertex counts exceed u16. |
| Upload staging / BLAS build scratch | Submission preparation | Already absent from M1's warm inventory. |

## Rejected: overlapping position palettes

Neighboring clusters may share exact three-word object-space positions. Pack each
pair as A-exclusive, shared, B-exclusive. A and B keep individual contiguous
palettes, overlapping only in the shared words; remap their packed u8 corners.
Weld only identical bits, preserving signed zero. Keep cluster/triangle/primitive
order, bounds, LOD links, maximum vertex count and shader ABI. No shader change,
extra projected vertex, retained CPU cache, or lighting-model assumption is needed.

A CPU quarry replay passed every decoded triangle corner and primitive ID for all
twelve source meshes and their LOD hierarchies. Source geometry words fell from
237,933,576 to 222,554,280 bytes (**14.667 MiB**), and LOD geometry from 363,657,832
to 353,982,856 bytes (**9.227 MiB**). Packing the twelve source/LOD sets took
2.68 / 3.78 seconds in the initial two-thread CPU diagnostic, separately from
loading and hierarchy construction. The subsequent binary used binary-search
remapping to reduce that preparation cost.

The first no-RT GPU screen allocated 3,348.541 MiB live, but reserved
**3,601.112 MiB**, versus M1's 3,439.676: 161.436 MiB worse despite the small live
saving. Slack grew from 81.913 to 252.571 MiB. This is measured allocation/packing
behavior, not evidence of a leak or a proven allocator-internal cause. Reject the
candidate on this short screen; do not spend broad identity/timing checks on it.
The following RT process was interrupted by the shared guard when a foreign
Cargo/rustc build started (12,593 MiB available, no game); it provides no RT result.

The implementation, CPU audit/log, binary identity and reproduction patch remain
under `E:/renderlab-work/m2-memory/rejected-palette/` and the evidence root. The
rejected packer and its diagnostic tests are absent from the retained source.

## Shared cluster-instance tables

The no-RT baseline owns two identical cluster-instance and LOD-group tables, one
for primary visibility and one for virtual-shadow casters. Both call
`ClusterInstances::new` with the same immutable geometry and placement counts.
Share the two buffers with `Arc`, keyed by the ordered cluster-data generations
and instance counts. Transforms and materials are separate bindings and do not
change these records. A weak lookup owns no GPU allocation; replacing geometry or
counts creates a new generation while old consumers/submissions keep theirs.

The focused GPU test checks sharing across transform edits, invalidation by
instance count and geometry generation, and release with no cache-owned reference.
The headless mode-round-trip test exercises actual bound consumers.

This one retained step saves exactly **14,120,000 live bytes** on no-RT:
13,699,968 cluster-instance bytes and 420,032 group bytes. Reserved memory remains
3,439.676 MiB; RT/hybrid retain their original live/reserved totals. No geometry
is moved into a CPU cache, and no material/shading ABI or shader changes.

## Texture residency: deferred

All material textures and source mips remain resident. A visibility-only feedback
pass cannot certify residency for arbitrary user shader UV/gradient choices:
`sample_base_color_grad` accepts authored coordinates and derivatives. Trilinear
and anisotropic filtering also need more than a rounded scalar mip estimate.
Previous-frame feedback plus a margin cannot prove that a disoccluded material or
camera cut never needs an absent mip in the current frame. This unit does not
ship an unproven texture-residency policy or claim a texture saving.

A future exact implementation should collect conservative requests at the actual
programmable sampling boundary (including explicit gradients), include both
trilinear levels and the anisotropic footprint, service requests before shading,
and expose a full-resident fallback for unsupported sampling/programs. It needs
missing-mip counters and complete hashes through S2/S3, not just static screenshots.

## Walk-record proposal for N8k/N8l

No `shaders/virtual_shadow*.wgsl` or `src/virtual_shadow_gpu.rs` is edited here.
The 36-byte records duplicate nine position words for each source triangle:
13,034,903 triangles occupy about 447.5 MiB. Source indices already provide the
three vertex IDs, and the 48-byte vertex stream retains their exact position bits.

The simplest coordinated experiment removes the copied records and reads the
existing index/vertex chain through the already-present `vsm_triangle` fallback.
That can save the full record allocation, but N8h established that scattered
loads can cost frame time; it must be an A/B measurement, not a free-memory claim.
The concrete next experiment is a tightly packed, exact XYZ stream indexed by the
existing source index buffer. Twelve bytes per source vertex cost about 75.9 MiB,
instead of 447.5 MiB per-triangle records: about **371.6 MiB less live data** before
allocator effects. This still needs dependent gathers, but its position working
set is one quarter of the 48-byte vertex stream. Unlike N8h's failed clustered
pool, it adds no primitive-to-page table, residency write or escape/decode branch.
Unlike its failed 32-byte records, it has no delta-width limit. A copied
12-byte-per-triangle vertex-ID record would just duplicate the existing indices
and is not a useful intermediate representation.

If that screen passes timing, coordinate a second change to make the packed XYZ
stream the source position representation for all consumers, separating the
remaining attributes behind the existing logical vertex/shading interface. That
could remove the remaining position duplicate too, but touches raster bindings,
primary and virtual shaders and must be owned together. Preserve exact batch and
primitive identity, appended LOD indices, corner order and generic programmable
attribute fetches. Do not cache BRDF parameters or other lighting-specific data in
geometry records. Those belong to programmable shading.

Owners should first replay saved query/triangle distributions, then measure one
paired quarry process before broad tests. Require all paths/both poses' complete
hashes, identical S1-S3 non-timing fields, motion/disocclusion, and clean-visit
timing ranges. Keep original records if the saved memory trades for a regression.

## Identity and checks

All **24 quarry SHA-256 fields** match M1 at both poses across all three paths:
complete hit/count/HDR/world buffers. No-RT first changed hits equal settled hits
at both poses, and every caster rolled-versus-dense check is zero. S1's **904**
non-timing fields and S2/S3's **72 each** match M1 exactly. Evidence:
`quarry-identity.json`, `identity.json`, `scores/`, and `after/*/changed-frames.json`.
These are complete-buffer checks, not region averages. Tone mapping and all WGSL
are unchanged. Existing quality misses, including S1 pose 1's two umbra leaks,
remain; this is image preservation, not a new quality-floor acceptance.

`cargo test -p renderlab --lib -j 2` exits **0: 106 passed, 110 ignored**. Fmt passes.
All-targets Renderlab Clippy passes with `-D warnings` and the same baseline
allowances as M1: `-A clippy::nonminimal_bool -A clippy::items_after_test_module`.
The explicit validation-enabled `gpu_primary_memory_modes` and
`gpu_shared_cluster_tables` pass with no validation errors. `gpu_virtual_fixture`
also exits 0. Its validation-disabled shader compilation, and the S1-S3 benchmark
processes, emit the existing `VUID-StandaloneSpirv-None-10684` SPIR-V layout
messages under the forced Vulkan layer; those also occur in M1's retained log.
This unit does not claim those pre-existing messages are fixed.

No `src/virtual_shadow_gpu.rs`, virtual-shadow WGSL or
`docs/milestones-5-6-decisions.tsv` changes. No window, input automation or desktop
capture is used. The shared runner owns the atomic GPU lock and reservation/
release lines, checks games, requires 4 GiB free at start and aborts below 2 GiB.

## Frame timing and verification cost

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu 30.0.1, rustc 1.96.0.
Development/test profile (Renderlab opt-level 1), not release-performance data.
1920×1080, 1x, anisotropy 16; no-RT LOD 0.35 px, lift 1, software extent 32.
Timing/memory runs disable validation. Eight visits per process with one asset
load per path; each visit has 32 warm frames, eight measured static frames,
16 sun-turn frames, 48 camera steps and 48 caster steps. Frozen binaries run
baseline then candidate for all three paths. No compiler ran during this lease.

Wallpaper contention remains. M1's clean-visit primary thresholds are unchanged:
≤0.45 ms no-RT, ≤0.65 ms fused Interactive RT, ≤0.30 ms traced Virtual. Rejected
visits stay in `timing-summary.json` and the source logs. These thresholds do not
detect every possible preemption. Baseline/candidate frame render-span ms follow:
median of clean visit medians, with their min–max range.

| Path | Phase | Before median [range] | After median [range] | Clean before / after |
|---|---|---:|---:|---:|
| nort | static | 1.407 [1.388–1.427] | 1.426 [1.391–1.435] | 8 / 8 |
| nort | turn | 1.428 [1.384–1.442] | 1.429 [1.403–1.443] | 8 / 8 |
| nort | move | 3.145 [3.106–3.150] | 3.159 [3.113–3.163] | 3 / 3 |
| nort | caster | 2.859 [2.830–2.919] | 2.861 [2.835–2.887] | 8 / 8 |
| rt | static | 0.904 [0.874–1.100] | 0.879 [0.872–0.922] | 4 / 5 |
| rt | turn | 0.909 [0.877–1.101] | 0.885 [0.874–0.924] | 4 / 5 |
| rt | move | 0.879 [0.867–1.094] | 0.890 [0.864–0.939] | 3 / 6 |
| rt | caster | 0.884 [0.873–1.101] | 0.897 [0.871–0.951] | 3 / 6 |
| traced-virtual | static | 1.295 [1.276–1.463] | 1.308 [1.280–1.394] | 8 / 7 |
| traced-virtual | turn | 1.311 [1.292–1.494] | 1.326 [1.292–1.405] | 8 / 7 |
| traced-virtual | move | 3.093 [3.066–3.103] | 3.108 [3.074–3.402] | 6 / 7 |
| traced-virtual | caster | 2.893 [2.852–2.946] | 2.898 [2.853–3.114] | 7 / 8 |

All phase ranges overlap. Median shifts are small and mixed; no frame-time
regression is resolved at this precision, and no speedup is claimed. No-RT
camera-shadow visit medians span 2.391–2.403 ms before and 2.387–2.409 after;
caster shadows span 2.111–2.192 before and 2.112–2.146 after.

Keep development latency separate: the fresh paired load records sum to
20.523 → 20.046 s for no-RT, 11.766 → 11.340 s for RT, and 11.905 → 11.080 s for
traced Virtual (decode + hierarchy preparation + upload + attachment). These
are single-process preparation milestones, not measured displayed startup-to-image
or a claimed startup speedup. Shader code is unchanged; no display-latency test
was run because this unit permits no windows.

Implementation/inventory/harness work was approximately 20 active minutes
(estimate), interleaved with verification and shared-slot waits. Verification is
recorded separately: five GPU/build leases total **717.2 s (12.0 min)**, including
the short aborted screen. Initial cold test compilation took 136.3 s; retained
test compilation plus CPU suite took 53.9 s (suite 0.53 s), Clippy 6.2 s. The
CPU triangle audit took 64.82 s; S1-S3 CPU scoring took about 132.4 s outside the
GPU leases. Shared-slot waiting is excluded from lease time.

## Reproduction and artifacts

```powershell
$env:N8_OWNER='m2-memory'
$env:N8_OUT='E:/renderlab-work/m2-memory'
py -3 -B scripts/n8-run.py --label memory --minutes 8 --fast-check -- py -3 -B scripts/memory-m1.py <test-binary> E:/renderlab-work/m2-memory/replay
```

Use `--poses 0 --visits 8` for the timing batch. `scripts/n8-batch.py` runs the
`score-s1 score-s2 score-s3` motion/quality replays; score with the unchanged
`bench-contact-fold-score.py` and `n8-score.py`, then compare non-timing fields.
Evidence root: `E:/renderlab-work/m2-memory/`. `after/` contains both-pose
per-allocation inventories, `timing-before/` and `timing-after/` the frozen pair,
`memory-summary.json` / `breakdown.json` the exact resource totals, and
`gpu-windows.json` the guarded durations. `retained-source.diff` and its SHA-256
record the source change; `build-identities.json` records full executable hashes.
Baseline SHA-256 starts `30da11d607f2`; retained starts `0c90786aefe4`.

After hashing/scoring, 939 raw/binary or uninspected-image files were removed
(2.85 GiB logical bytes, including hardlinks). Summaries, source patches and the
inspected S1 crop remain. `cargo clean --target-dir E:/renderlab-work/m2-memory/target`
removed 2,956 build files (1.4 GiB); the target is gone. No owned GPU/build job or
lease remains.
