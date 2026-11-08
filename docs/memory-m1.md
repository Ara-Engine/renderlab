# M1: GPU memory without changing the image

RTDEF2 update (2026-11-08): the current traced-capable RT default is traced + Interactive + TR1 with the adaptive 256/64/32/8 budget; see [RTDEF2](rtdef2.md). Earlier decisions below are historical. Their fixed-eight-ray, TR1-off timings and memory are not measurements of the new profile.

2026-11-07 UTC, base `d798200`, branch `t3/m1-memory`.

**RT scope:** all Interactive RT timings, speed claims and memory rows here are
**measured on the historical eight-ray setup with TR1 off (traced + Interactive)**. The RT-device
default remains raster + 256-ray Exact; see [RT1](rt1-interactive.md). Traced + Virtual
rows describe a separate explicit opt-in. Allocator-default decisions below do not
change primary visibility or shadow sampling defaults.

Retained: right-sized unused repair queues, asynchronous immutable BLAS compaction,
and compaction during upload collection. All quarry image hashes and S1–S3
non-timing fields remain identical. **The 8–64 MiB MemoryUsage policy stays the
default.** The smaller-block experiment saved memory but regressed timings and
was rejected.

## Totals against Unreal

MiB = 2^20 bytes. Renderlab cells are **allocator reserved / WDDM Dedicated Usage**.
These are different quantities: the allocator reports committed blocks, including
free tails and host-visible allocations; WDDM reports local per-process usage,
including driver costs. Their difference is not a leak counter.

| Path | Current-base baseline | Retained | Reserved saved | Recorded Unreal DXGI |
|---|---:|---:|---:|---:|
| No-RT: raster LOD + Virtual | 3,510.9 / 3,583.6 | **3,439.7 / 3,512.3** | **71.2** | VSM no-RT: 3,912 |
| Traced + Interactive RT | 3,175.2 / 3,249.4 | **2,641.2 / 2,716.3** | **533.9** | HRT: 5,410 |
| Traced + Virtual | 3,927.2 / 4,051.3 | **3,258.1 / 3,333.1** | **669.1** | VSM RT enabled: 3,946 |

The figures are fresh, complete processes at both quarry poses. Final warm maxima
also equal their whole-process sampled maxima. N8h's original WDDM figures were
3,662 / 3,241 / 4,001 MiB; the fresh controls above are used for the actual delta.
WDDM varies by about +/-70 MiB between identical runs, as N8h documented. Sampling
is approximately every 1.5 seconds, not an instantaneous peak certificate.

The Unreal 5.7.4 CL51494982 DXGI totals are reused from [N8h](non-rt-n8h.md), not
rerun. Same twelve scans, 13,034,903 source triangles, twelve uncompressed 4K base
colors with mips, 1920x1080. This is the recorded direct-light subset without
Lumen/GI/SSR/fog. The hybrid comparator has no exact Unreal counterpart: Renderlab
traces primary visibility (336 MiB compacted BLAS); Unreal's VSM mode rasterizes
Nanite and has only about 4 MiB of RT geometry. No whole-engine superiority or
complete Unreal quality/performance-floor claim follows from this table.

## Measure-first resource inventory

The initial [baseline CSV](memory-m1-baseline.csv) lists **every allocator label**,
exact bytes in all three paths, usage and owning lifetime. It was committed before
production changes, using N8h's measured reports. The immutable current-base
replay reproduced all three live/reserved totals exactly. The [retained CSV](memory-m1-after.csv)
provides the same inventory after the changes. The reusable harness additionally
saves **individual allocations**, offsets, block membership and lifetime/usage
CSVs after upload and warm-up, not just these label aggregates.

Largest resource groups, allocated MiB; unchanged entries show one number:

| Resource | No-RT before → after | RT before → after | Hybrid before → after |
|---|---:|---:|---:|
| Material textures/mips | 1,024.0 | 1,024.0 | 1,024.0 |
| Mesh indices, adjacency, appended LOD | 596.3 | 298.3 | 298.3 |
| Mesh vertices | 303.7 | 303.7 | 303.7 |
| Cluster geometry words | 346.8 | 226.9 | 226.9 |
| BLAS | 0.0 | 869.5 → 335.6 | 869.5 → 335.6 |
| Walk records/build data | 447.5 | 0.0 | 447.5 |
| Ring horizons | 99.4 | 0.0 | 49.7 |
| Other Virtual clipmap/raster/cache data | 237.5 | 0.0 | 230.8 |
| Environment cubes | 97.5 | 97.5 | 97.5 |
| Deferred edge/retry queues | 79.1 → 0.0 | 79.1 | 79.1 → 0.0 |
| Other named allocations | 205.0 | 144.4 | 150.8 |

Initial allocator slack was 74.0 / 131.7 / 149.2 MiB. Retained slack is 81.9 /
131.7 / 93.1 MiB. Final block counts are 68 / 71 / 79, versus 69 / 71 / 79.
No storage was moved into a retained CPU geometry cache.

## Changes and individual savings

### Repair queues only when used

Only traced Interactive visibility dispatches deferred edge repair. Other modes
now bind the ABI's 52/20-byte placeholders instead of unused full-resolution
edge/retry queues. Switching to traced Interactive restores full worst-case
capacity. Replaced targets and bindings retire after their last submission.

At 1920x1080 this removes **79.102 MiB live** on both Virtual paths, initially
saving 71.191 MiB reserved no-RT and 135.191 MiB hybrid through different packing.
Interactive RT keeps exactly its original capacity. The epsilon image stays full
size: no-RT resolve writes it too. No worst-case queue capacity was reduced.

### Compact immutable BLAS

Builds allow compaction while retaining fast-trace and hit-vertex-return flags.
Preparation is asynchronous. Completed builds compact through normal collection,
with a frame-time fallback for builds not yet ready. Copies flush with ordinary
submissions; no blocking GPU wait was added.

A placement retains the exact BLAS generation used by its TLAS. Replacing that
generation changes scene identity, triggering a new TLAS while older flights
retain their original structures. Admission accounts for overlapping generations
and upcoming TLAS/scratch. A restrictive caller policy skips optional compaction
and continues rendering the original valid BLAS.

The twelve BLAS allocations fall from **869.544 to 335.603 MiB**, saving
**533.941 MiB live**. Geometry buffers, primitive numbering, vertex-return data,
receiver records and programmable material/shading interfaces stay unchanged.

### Compact completed uploads before the first frame

The first screen compacted all meshes at the first frame, overlapping originals
with their copies and leaving larger allocator tails. Collection now compacts
completed uploads incrementally. Later ordinary uploads flush their copies.
The final post-upload reports contain all twelve compacted BLASes and no original
BLAS allocations, before the first quarry frame. Warm reports have zero pending
builds or retired AS generations.

This saves another **48.397 MiB reserved RT / 112.397 MiB hybrid** versus the
first-frame version. The RT sampled startup peak falls from 3,363.2 to 2,716.3 MiB;
this is sampled evidence, not a bound on sub-second transients.

Measured reserved memory after each step, MiB:

| Step | No-RT | RT | Hybrid |
|---|---:|---:|---:|
| Baseline | 3,510.868 | 3,175.188 | 3,927.188 |
| Remove unused queues | 3,439.676 | 3,175.188 | 3,791.996 |
| First-frame BLAS compaction screen | 3,439.676 | 2,689.644 | 3,370.452 |
| **Compaction during collection, retained** | **3,439.676** | **2,641.247** | **3,258.055** |

## Rejected: 4–8 MiB allocator blocks

RT timings here are **measured on the historical eight-ray setup with TR1 off (traced + Interactive)**.

Reports exposed an empty 32 MiB block and partly occupied 32/64 MiB blocks.
Smaller blocks reduced reserved memory to 3,366.523 / 2,520.684 / 3,179.172 MiB,
with identical images. The first screen requested 1–8 MiB, which wgpu 30 Vulkan
clamps to the same effective 4–8 MiB device / 4 MiB host configuration.

But timing shifts survived reversing the process order. Static frame visit
medians (ms), two run orders, were:

| Path | Baseline, A/B and B/A | Small blocks, A/B and B/A | Same candidate, original 8–64 MiB |
|---|---:|---:|---:|
| No-RT | 1.417 / 1.413 | 1.449 / 1.453 | 1.422 |
| RT | 0.898 / 0.882 | 0.909 / 0.932 | 0.873 |

The frozen candidate under `RENDERLAB_MEMORY_HINTS=memory_usage` restores the
timings. Therefore the smaller policy is **not retained or made a default**.
The test-only `small` override remains for reproduction. Applications still
choose their own device descriptor; `MEMORY_HINTS` continues recommending
MemoryUsage. No resource-size limit or quality reduction was introduced.

## Identity and acceptance

Final source (`retained/`, not the rejected `final/` small-block replay) matches
all **24 hit/count/HDR/world SHA-256 fields** at both poses in all three paths.
S1 matches all 904 non-timing leaves (852 mode fields, 52 reference fields);
S2 and S3 each match all 72 non-timing comparison fields against N8h. Every
caster rolled-versus-dense check remains zero; no-RT first changed hits equal
settled hits at both poses. Evidence: `retained-identity.json`, `retained-scores/`.
These compare complete quarry buffers, not region averages. The identical HDR
images pass through the unchanged tone-map shader.

No WGSL or `src/virtual_shadow_gpu.rs` changed. Shading remains programmable;
no lighting-model data or shortcut entered geometry, receivers or dispatch.
Existing N8h quality misses remain, including S1 pose 1's two umbra leaks. This
unit preserves that image, rather than claiming to close the remaining gates.

Final `cargo test -p renderlab --lib -j 2`: **exit 0, 106 passed, 109 ignored**.
Fmt passes. Renderlab all-targets Clippy passes with `-D warnings` and the existing
baseline allowances `-A clippy::nonminimal_bool -A clippy::items_after_test_module`.
Three explicit, validation-enabled GPU tests also pass:

- `gpu_primary_memory_modes`: tight-policy fallback, exact uncompacted/compacted
  receivers and HDR, required repair capacity, and two mode round trips.
- `gpu_hardware_shadow_replacement_with_queued_work`: actual incomplete frames,
  bounded retirement and complete release.
- `gpu_hardware_shadow_rejects_unsupported_before_writes_and_preserves_featureless`.

They report zero validation errors; existing unused-vertex-attribute performance
warnings remain. No viewer/window behavior was exercised. No window or input
automation was used. `docs/milestones-5-6-decisions.tsv` is unchanged.

## Frame timing

RT timings and ratios here are **measured on the historical eight-ray setup with TR1 off (traced + Interactive)**.

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu 30.0.1, rustc 1.96.0.
Development/test profile (renderlab opt-level 1), not release-performance data.
1920x1080, 1x, anisotropy 16; no-RT LOD 0.35 px, lift 1, software extent 32.
Timing/memory validation is off; quality/GPU boundary tests have it on.

Eight independent visits per process, one asset load per path. Each visit uses
32 warm frames, eight measured static frames, 16 sun-turn frames, 48 camera steps
and 48 caster steps. Baseline/candidate processes run in both orders for no-RT/RT.
The retained 8–64 MiB control uses the same frozen implementation with the explicit
MemoryUsage override; final-source full replays confirm both poses. Hybrid has a
separate eight-visit final-source repeat. No builds ran during timing.

Wallpaper contention remains. A clean visit's median primary cost is at most
0.45 ms no-RT (N8h's criterion), 0.30 ms traced Virtual, or 0.65 ms fused RT
Interactive. Rejected visits remain in the logs. Frame render-span milliseconds:
median of clean visit medians, with their min–max range; final column is clean
baseline / retained visit counts.

| Path | Phase | Baseline median [range] | Retained median [range] | Clean visits |
|---|---|---:|---:|---:|
| nort | static | 1.415 [1.384–1.470] | 1.422 [1.406–1.475] | 16 / 8 |
| nort | turn | 1.433 [1.406–1.471] | 1.438 [1.409–1.488] | 16 / 8 |
| nort | move | 3.156 [3.103–3.169] | 3.120 [3.104–3.147] | 9 / 3 |
| nort | caster | 2.875 [2.835–3.032] | 2.851 [2.843–2.895] | 16 / 6 |
| rt | static | 0.882 [0.857–0.924] | 0.873 [0.870–0.927] | 10 / 6 |
| rt | turn | 0.888 [0.859–0.927] | 0.876 [0.873–0.931] | 10 / 6 |
| rt | move | 0.878 [0.852–1.099] | 0.866 [0.864–0.923] | 9 / 6 |
| rt | caster | 0.878 [0.859–0.935] | 0.877 [0.873–0.929] | 8 / 6 |
| traced-virtual | static | 1.293 [1.281–1.370] | 1.284 [1.280–1.299] | 7 / 8 |
| traced-virtual | turn | 1.309 [1.289–1.366] | 1.299 [1.285–1.307] | 7 / 8 |
| traced-virtual | move | 3.105 [3.085–3.310] | 3.077 [3.055–3.100] | 7 / 7 |
| traced-virtual | caster | 2.910 [2.891–2.926] | 2.876 [2.860–2.895] | 6 / 8 |

The retained ranges overlap controls, with mixed sub-percent/roughly one-percent
median shifts and no resolved frame-time regression at this precision. No speedup
claim is made. No-RT shadow camera medians range 2.381–2.414 before versus
2.375–2.401 ms after; caster ranges 2.105–2.275 versus 2.105–2.145 ms. The primary
criterion cannot identify every possible preemption, so all samples are preserved
in `retained-timing-summary.json` / `timing-summary-all.json` and their source logs.

## Remaining costs and proposals

- Vertex/index allocations already serve raster, storage and BLAS inputs. Warm
  upload staging and build scratch are absent; the empty allocator block was
  reserved capacity. No live scratch allocation remained to free.
- An RT mesh later given LOD can retain its old index buffer through BLAS build
  inputs. Neither measured traced path attaches LOD. That is a separate lifetime
  candidate, not a claimed saving here.
- Quarry meshes exceed 65,535 vertices; their global indices cannot simply become
  u16. Cluster-local triangles are already packed u8 indices.
- Positions and normalized shading normals are f32. No smaller exact source
  representation was established. All texture formats, mips and LOD levels stay
  resident: a level unused by today's cut may be needed by the next camera/light.
- Cluster geometry copies (347 MiB no-RT / 227 MiB traced) merit a coordinated
  geometry-format investigation. Sharing exact source positions needs primary
  and shadow-raster shader changes while preserving the public shading ABI.
- No-RT owns two immutable cluster-instance/group tables, about 13.5 MiB each.
  Sharing their generation is a proposal for the shadow allocation owner.
- Walk records (447.5 MiB), horizons (99.4 MiB), clipmaps and shadow lookup layouts
  are untouched. Further shadow allocation work must coordinate with N8k/N8l;
  N8h already measured failed smaller-record and paged-pool designs.

## Reproduction, evidence and time

`scripts/memory-m1.py` runs the headless quarry once per path/device and emits
individual allocator allocations/blocks after upload and warm-up, CSV usage and
lifetime annotations, complete-buffer hashes, and WDDM samples through
`scripts/memory-wddm.ps1`. Unknown allocation names fail the catalogue.
Lifetimes describe ownership, not proof that resources can alias.

```powershell
$env:N8_OWNER='m1-memory'
$env:N8_OUT='E:/renderlab-work/m1-memory'
py -3 -B scripts/n8-run.py --label memory --minutes 8 --fast-check -- py -3 -B scripts/memory-m1.py <test-binary> E:/renderlab-work/m1-memory/run
```

`--memory-only` records one warm pose; `--poses 0 --visits 8` replays eight
configurations with one asset load, hashing counts/hits only. Default single
visits hash hit/count/HDR/world buffers and remove raw snapshots. The shared
runner owns `D:/Coding/renderlab-work/gpu-slot.lock`, reservation/release lines
in `E:/renderlab-work/gpu-schedule.md`, game detection, and the 4 GiB start /
2 GiB abort guard. All starts had no game and at least 12,617 MiB available RAM.

Evidence root: `E:/renderlab-work/m1-memory/`. Retained artifacts are under
`retained/`, `retained-scores/`, `timing-retained/`; original controls and rejected
screens remain separately named. `build-identities.json` records binary SHA-256s,
`retained-source.diff`/`.sha256` the source delta, and `runs/` the guarded steps.
Baseline executable SHA starts `69cdfc8a201d`; retained starts `b2e1feedef5d`.

Implementation/harness work was roughly 20 active minutes, interleaved with
verification and slot waits from about 01:38–02:34 UTC; this is an estimate.
Final analysis/documentation followed. Verification is measured separately:
10 guarded GPU/build leases totaling 1506.7 s (25.1 min). Initial cold test compilation took 157.3 s; the final retained
test compile plus CPU suite took 36.8 s, Clippy 3.7 s. CPU scoring took about
278 s across the rejected-policy and retained replays. Shared-slot waiting is
excluded from lease time; lease time includes loading/checks, not just GPU busy time.

After hashing/scoring, 456 raw/binary or uninspected-image files were deleted (2.52 GiB logical bytes, including hardlinks). JSON/CSV/log summaries and the inspected S1 crop PNG remain. This worker's Cargo target and staged executables are removed. No owned GPU/build job or lease remains.
