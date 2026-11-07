# N8h: walk records compressed, and GPU memory against Unreal

Follow-up unit to [N8g](non-rt-n8g.md), 2026-11-06/07 UTC, base `616e8c4` (N8g on `79e8ba2`), branch `t3/n8h-record-compress`. Brief (Islam, 2026-11-06): "compress as much as possible, and if we are using less memory than Unreal, then merge". N8g's 48-byte per-triangle walk records made the non-RT shadow pass 12-18% faster, at about 1.2 GiB of VRAM on the no-RT quarry. Labels: **M** measured here, **I** inference.

## Result

- **Records: 1,193 → 448 MiB on the no-RT quarry (−62%), 597 → 448 MiB traced (−25%), output byte-identical.**
  - Records now hold only the three positions of each *source* triangle: 36 bytes, packed across batches.
  - Edge neighbours stay in the index buffer's adjacency suffix.
  - Appended LOD triangles (the no-RT receivers' own cut) read the chains.
- **Most of the speed gain survives.** No-RT quarry, records against no records in the same lease:
  - static 0.66-0.70 against 0.78-0.83 ms (−15%);
  - camera moving 2.41-2.44 against 2.65-2.67 (−9%);
  - caster moving 2.08-2.15 against 2.21-2.29 (−6%).

  N8g's full records gave −18% and −12%. The traced quarry keeps all of it: −18% static, −12% camera.
- **Ours is below Unreal 5.7.4 on both matched paths, once our devices use MemoryUsage allocator blocks.** This branch makes them our default (`renderlab::MEMORY_HINTS`).
  - **No RT:** 3,511 MiB (allocator) / 3,662 (OS counter) against Unreal VSM's 3,912.
  - **With RT:** 3,175 / 3,241 against Unreal's hardware-RT shadows at 5,410.

  With wgpu's default 128-256 MiB blocks, the no-RT total with records is 4,160 / 4,247, above Unreal. N8g's records were above it by 1.3 GiB.
- **Mergeable under the memory rule** for the no-RT and RT paths, which are the matched Unreal configurations. One hybrid is roughly equal to Unreal: traced primaries with these non-RT shadows (3,927 / 4,001 against VSM-with-RT-enabled's 3,946). Its 870 MiB of primary-ray BLAS has no Unreal counterpart in that mode. See the table.
- **Exact compression below 36 bytes does not work on this content**, and a paged record pool cost the whole gain. Both were built, measured and removed (below).

## Record layout (M)

| | N8g | N8h |
|---|---|---|
| Per record | 3 × vec4<u32>: corner position bits + edge neighbour word | 9 × u32: corner position bits |
| Bytes | 48 | 36 |
| Triangles recorded (default) | every addressed triangle, source and appended LOD (26.05 M on the no-RT quarry) | source triangles only (`WalkRecords::Source`, 13.03 M) |
| Neighbours | in the record | the index buffer's adjacency suffix (as before N8g); its address depends only on the primitive, so its load issues beside the record's |
| Segments | per batch, 96 MiB, each batch from a segment boundary | one numbering across batches, 72 MiB; on the quarry six whole 72 MiB buffers and one of 15.5 MiB |
| Addressing | slot in bits 25-31 of `VsmFrame::horizon` | `VsmFrame::records`: per batch (first record, recorded count); `FRAME_BYTES` 1168 → 1680 |
| No-RT quarry | 1,193.4 MiB | 447.5 MiB |
| Traced quarry | 596.7 MiB | 447.5 MiB |

- **Coverage setting.** `VirtualShadowSettings::walk_records` (`WalkRecords::Off | Source | All`, default `Source`) chooses which triangles get records. Records change cost and VRAM, never output: `Off` reads the index, vertex and adjacency buffers, as N8f did. Records are still built once per draw list and are camera-, sun- and instance-independent.
- **Allocator blocks.** `renderlab::MEMORY_HINTS` (`wgpu::MemoryHints::MemoryUsage`, 8-64 MiB blocks) is the recommended `DeviceDescriptor::memory_hints`. The viewer and the test devices now use it; `RENDERLAB_MEMORY_HINTS=performance` restores wgpu's default in tests. Packing the segments matters here: every whole segment exceeds the 64 MiB block and gets its own allocation. N8h's first 36-byte layout, per batch, saved only 22 MiB of reserved memory over 48 bytes, because 37 MiB buffers left block tails.
- **Code.**
  - `shaders/virtual_shadow.wgsl`: `vsm_record_at`, `vsm_tri`, `vsm_nb`.
  - `shaders/virtual_shadow_records.wgsl`.
  - `src/virtual_shadow_gpu.rs`, and `src/lib.rs` (`MEMORY_HINTS`).
  - `viewer/src/main.rs`.
  - The harness: `src/visibility_buffer_tests.rs` (memory reports by allocator label, `soft_shadows: "interactive"`) and `src/fresh_shadow_tests.rs`.
- **Programmability is unchanged.** Records are geometry only, and no lighting-model data entered hit records, clusters, maps or the lookup.

## Speed (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off, 1920x1080, no-RT quarry at LOD 0.35 px. The records-on and records-off variants run in one process, interleaved twice, poses 0 and 1. Fresh single-variant processes for each sit later in the same lease. A visit is clean when its primary pass reads at most 0.45 ms (wallpaper contention). Ranges are clean visits' medians; contended visits (primaries 0.47-0.77 ms) are excluded. Final lease, `renderlab::MEMORY_HINTS` blocks:

**No-RT quarry, shadow pass, ms**

| Records | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| Off (N8f's lookup) | 0.781-0.833 | 0.799-0.850 | 2.647-2.673 | 2.213-2.292 | 1.50-1.56 / 3.37-3.44 / 2.94-3.06 |
| **Source, 36 B (default)** | **0.658-0.702** | **0.679-0.706** | **2.407-2.425** | **2.075-2.152** | **1.40-1.44 / 3.16-3.22 / 2.83-2.91** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

- **Camera-moving visits.** Three of the six source camera-moving visits were contended. The clean one read 2.407, and the fresh-process one 2.425 (primaries 0.468, at the threshold).
- **Default allocator blocks** (`final-tperf`, same lease, separate process): off 0.846-0.896 / 0.861-0.906 / 2.707-2.766 / 2.384-2.456; source 0.709-0.743 / 0.721-0.742 / 2.435 / 2.113-2.182. That is −17% static, −11% camera and −11% caster.
- **MemoryUsage blocks cost no speed.** Every stage, primaries included, read 3-8% faster under them, records off a little more than on, which narrows the records' relative gain. The two settings ran as separate processes, not interleaved. The cause, placement or run order, is not isolated (I).

Screening leases of the same lookup (p3, wgpu default blocks) agree: off 0.869-0.887 / 0.894-0.909 / 2.778-2.800 / 2.412-2.467, source 0.700-0.733 / 0.699-0.719 / 2.451-2.452 / 2.115-2.241. In p3, 48-byte source records read 0.665-0.703 static and 2.38 camera, within noise of 36 bytes. All coverage (895 MiB) read 0.692-0.723 static and 2.403-2.426 camera, no faster than source.

**Traced quarry screen** (`gpu_virtual_quarry_screen`, pose 0, 16 camera frames of 1 mm, MemoryUsage blocks, clean visits): records off read static 0.764-0.769, turning 0.752-0.760 and camera 2.756 ms. Source records read 0.622-0.636, 0.637-0.640 and 2.439 (−18%, −16%, −12%). Three later visits in that process were contended, camera 5.0-6.8 ms, and are excluded.

**S2 and S3** (validation on, as scored): shadow p50 against N8g's run is mixed, within the small-scene run-to-run spread (identical S2/S3 variants have differed by up to about 0.3 ms between runs).

| Bench / phase | N8g shadow p50 (ms) | N8h shadow p50 (ms) | VSM (ms) |
|---|---:|---:|---:|
| S2 camera, out / hold / return | 0.62 / 0.20 / 0.62 | 0.68 / 0.22 / 0.68 | 0.35 |
| S2 shadow step, out / hold / return | 0.44 / 0.15 / 0.43 | 0.40 / 0.17 / 0.39 | 0.35-0.36 |
| S3, six phases | 0.24-0.25 | 0.26-0.27 | 0.37 static, 0.59 rotating |

## Memory against Unreal, like for like (M)

Same quarry (12 scans, 13,034,903 source triangles, twelve 4K base-color textures uncompressed with mips), 1920x1080, static pose 0, measured after warm-up, one process per configuration.

- **Unreal 5.7.4.** The matched e2e command lines: packaged Development `UnrealGame.exe`, D3D12 SM6, `-RenderOffscreen`, hidden process, no window. Memory dumps at CSV frames 100-122: `memreport -full`, `rhi.DumpMemory`, `rhi.DumpResourceMemory`, `r.DumpRenderTargetPoolMemory`, `D3D12.DumpRayTracingGeometries`.
  - Its total is the engine's own `GPUMem/LocalUsedMB`, DXGI's per-process local usage.
  - `VSM no-RT` adds only `r.RayTracing=False` to the matched VSM line.
  - The matched config has no Lumen, GI, SSR or fog. Nanite runs with full residency and a 1,024 MiB streaming pool.
- **Renderlab.** The no-RT quarry harness and the traced quarry. The figures are the allocator's reserved bytes (`generate_allocator_report`, what wgpu committed) and the WDDM per-process counter `\GPU Process Memory(*)\Dedicated Usage`, the same OS quantity as DXGI's. That counter is sampled about every 1.5 s; the table gives the maximum, which is the steady state at the end of the run.

MiB (2^20 bytes; Unreal's "MB" is MiB). Ours: allocator reserved / WDDM.

| Path | Renderlab, `MEMORY_HINTS` blocks (default now) | Renderlab, wgpu default blocks | Unreal: DXGI / RHI dump / WDDM | Ours lower? |
|---|---|---|---|---|
| **No RT:** no-RT device, rasterized LOD primaries, Virtual shadows, **source records** | **3,511 / 3,662** | 4,160 / 4,247 | **VSM, `r.RayTracing=0`: 3,912** / 3,898 / 3,835 | **yes, by 250-400** (no with default blocks) |
| No RT, records off | 3,079 / 3,230 | 3,648 / 3,628 | as above | yes |
| No RT, records all (36 B) | 3,943 / 3,987 | — | as above | no |
| No RT, N8g's records (48 B, all) | — | 5,184 / 5,297 | as above | no |
| **RT:** traced primaries + Interactive RT shadows | **3,175 / 3,241** | 3,648 / 3,651 | **HRT: 5,410** / 5,476 / 5,335 | **yes, by about 2,200** |
| Hybrid: RT device, traced primaries + Virtual shadows, source records | 3,927 / 4,001 | 4,416 / 4,427 | VSM, RT enabled: 3,946 / 3,902 / 4,208 | about equal (−19 / +55) |
| Hybrid, records off | 3,431 / 3,579 | 3,904 / 4,036 | as above | yes |
| Hybrid, N8g's records | — | 4,672 / 4,683 | as above | no |

- **Unreal's RHI dump** counts 800 MiB of reserved but uncommitted buffer address space.
- **Unreal's WDDM samples are few** (its runs last 7-12 s), so DXGI's figure is its total.
- **The hybrid row has no exact Unreal counterpart.** Ours keeps 870 MiB of BLAS for traced primary visibility, while Unreal's VSM mode rasterizes Nanite and builds no BLAS (4 MiB of RT geometry). Against Unreal's RT configuration (HRT, 5,410) the hybrid is 1.4 GiB lower.
- **Run-to-run spread.** Our WDDM maxima vary by about ±70 MiB between identical runs (records off, MemoryUsage blocks: 3,127, 3,230, 3,256). The allocator figures are exact.

Breakdown, no RT. Renderlab with source records and `MEMORY_HINTS` blocks; Unreal VSM no-RT:

| Renderlab | MiB | Unreal | MiB |
|---|---:|---|---:|
| Geometry: source + LOD indices and adjacency 596, cluster tables 347, vertices 304, instances 26, groups 1 | 1,301 | Nanite streaming pool + roots (`ClusterPageData`) | 1,248 |
| Textures (twelve 4K base colors, three small) | 1,024 | Textures (twelve 4K base colors) | 1,025 |
| Walk records | 448 | — | |
| Clipmap: two 64-bit layers, max-mips, raster lists | 238 | VSM physical page pool 512, HZB pool 85, page tables | 642 |
| Visibility buffer, hit records, fresh targets | 228 | Render-target pool | 641 |
| Ring horizons | 99 | — | |
| Environment cubes | 98 | Transient RDG heaps in use | 640 |
| Allocator block slack (wgpu default blocks: 658) | 74 | RHI reserved, uncommitted | 800 |
| Driver and other (WDDM − allocator) | 151 | | |

## Tried and rejected (M)

- **Paged record pool** (`E:/renderlab-work/n8h/variants/pool.diff`).
  - **Design.** N8g's records for 256-triangle pages, in a fixed pool of slots. A lookup that missed read the chains and marked the page requested in its page-table entry. Each frame a scan, a FIFO slot assignment and a build made requested pages resident before the next lookup.
  - **Output.** Byte-identical: traced hashes equal at every pool size.
  - **Working set.** Lookups touch 48,000 of the quarry's 101,773 pages (47%, 563 MiB of records), because source triangle order is not spatially coherent.
  - **Speed.** A whole-scene pool read 2.77 ms on camera-moving frames against 2.80 with no records: the page-table load and the request write in the walk loops cost the records' gain. A 192 MiB pool thrashed at 3.4-5.8 ms.
- **Exact 32-byte records** (the p2 binary).
  - **Design.** Corner 0's position bits, corners 1 and 2 as 20-bit two's-complement differences of their bit patterns, neighbours as 13-bit primitive deltas, escapes to the chains. All integer, so exact; traced hashes equal.
  - **Escapes.** 4.48 M of 13.03 M source triangles (34%) escaped on positions, and 29.7 M of 39.1 M source edges (76%) on neighbours.
    - Positions: in object space many triangles span more than 2^19 ulps.
    - Neighbours: scan triangle order puts most of them more than 4,096 primitives away.
    - Over all triangles, 44% escaped.
  - **Speed and memory.** Escapes plus the decode's per-visit ALU made it slower than records off everywhere: no-RT camera 2.78-2.90 against 2.59-2.66 ms, traced camera 2.96 against 2.70. Memory was 398 MiB for source triangles.
- **48-byte records, source only:** as fast as 36 bytes within noise, 149 MiB more.
- **All coverage:** no faster than source (the walks' cost is in source casters), 447 MiB more.
- **Not built, reasons measured:**
  - Quantization against cluster bounds: positions are raw scan floats, so lossy quantization would move receivers off their own triangles and change output. Even exact deltas need more than 20 bits for a third of the triangles.
  - Vertex sharing within clusters: source clusters keep non-contiguous primitive IDs, so it needs a per-primitive map, the same extra dependent load that cost the pool its gain.

## Quality (M)

Every retained change is byte-identical to N8g's output, which is N8f's.

- **Traced quarry:** static and turned hashes `2d6e4fa6ca32…` and `88888216c1d9…`, dense turned `222153a86f18…`. They equal N8g's in every screen variant of every lease: records off, source or all, under both allocator settings, and in the pool, 32-, 48- and 36-byte binaries.
- **No-RT quarry at 0.35 px:** fresh single-variant processes give N8g's counts SHA-256 `05e50a1d8ad17426…` (pose 0) and `bed48bf7099343bb…` (pose 1), both with source records and with records off. Every hash is in `counts-hashes.txt`.
- **S1, S2, S3:** rescored on the final binary with N8f's scorers. Every non-timing field equals N8g's: 792 S1 score fields, and 72 each in S2's and S3's `comparison.json`. Those include partial RMS, HDR RMS, lost contacts, leaks and flips.
- **Caster-moving frames:** N8f's identity gate (rolled counts against a dense lookup) reports 0 differing pixels in every variant of every lease.

So the quality table is N8g's, against Unreal VSM:

| Bench / mode | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges / contacts | Unreal VSM |
|---|---:|---:|---:|---:|---:|---|
| Quarry pose 0, no-RT 0.35 px | .3529 | 2,434 | 3,288 | 60 | 2,376 | .483 / 12,390 / 14,935 / 87 / 11,884 |
| Quarry pose 1, no-RT 0.35 px | .3476 | 2,474 | 3,366 | 55 | 2,424 | lit darkened 88 |
| Quarry pose 0 / 1, traced | .2863 / .2896 | 340 / 356 | 1,201 / 1,213 | 0 / 0 | 339 / 356 | as above |
| S1 pose 0 / 1 | .1085 / .1065 | 0 / 0 | 0 / **2** | — | contacts lost 0/419, 0/335 | .1642 / .1628; flips 0; leaks 0 / 0; lost 5-6 / 2-3 |
| S2 camera, out / hold / return | .0833 / .0865 / .0809 | 0 | 0 | — | 0 | .1752 / .1758 / .1739; leaks 57-62; flips 152-167 |
| S2 shadow step, out / hold / return | .0759 / .0766 / .0759 | 0 | 0 | — | 0 | .1697 / .1675 / .1692; leaks 65-67; flips 119-120 |
| S3, all six phases | .1159-.1266 | 0 | 0-1 | — | 0 | .1811-.1878; leaks 103-114; flips 187-226 |

HDR RMS is .0099-.0115 on S2 and .0146-.0152 on S3, against VSM's .0145-.0154 and .0194-.0219. Not film quality, and never presented as meeting the film gate.

### The 513-pixel order effect: not the records (M)

- **What happens.** Inside one no-RT process, some configurations that follow certain others give a second counts image (`db9e1a6068e1721f…` pose 0, `c7ad0451679f59cc…` pose 1). It differs from N8g's in 513 of 2,073,600 pixels, scattered over the receivers.
- **Seen in every binary.** It appeared in all four binaries: pool, 32-, 48- and 36-byte. The image was identical in each, so record contents do not produce it.
- **Decisive cases:**
  - In the isolation process (`final-order`), a **records-off** configuration following a source one produced the same image.
  - The fresh source and off processes give N8g's hashes.
  - First configurations and most others match N8g's hash.
- **Verdict.** The records do not change output. The effect is a history dependence of the no-RT path: the same settings and camera give either image, depending on the configurations run before them in the process. It survives the cache reset that a settings change does.
- **Not found:** its cause, and whether it predates N8h. Earlier units scored one configuration per process, and their raw counts are deleted. The S1-S3 scores and every result above come from fresh processes or first configurations. **Open for a follow-up**, since it means a renderer state escapes the settings-change reset.

## Misses, honestly

- **Memory needs MemoryUsage blocks.** With wgpu's default 128-256 MiB blocks, the no-RT total with source records is 4,160 / 4,247 MiB, above Unreal's 3,912; records off is below it either way. This branch makes MemoryUsage blocks Renderlab's default (`MEMORY_HINTS`, viewer and test devices). An application that creates its own device with wgpu's default gives up that margin.
- **The hybrid path is about equal to Unreal**, not clearly lower: 3,927 / 4,001 against 3,946 (`WalkRecords::Off` saves 447 MiB there).
- **Part of N8g's gain is lost** under MemoryUsage blocks: camera motion −9% against N8g's −12%, caster motion −6% against −7 to −11%. With default blocks the gain is −11% camera and −17% static.
- **Camera motion still misses VSM's estimated 0.5 ms by about 5x** (2.41 ms; no-RT frame 3.16 against 1.61). Static is 0.66-0.70 against 0.50. Turning is met (0.68-0.71 against about 0.95). The no-RT static frame is 1.40-1.44 ms, under 1.61.
- **Compression below 36 bytes failed** on this content (above).
- **The order effect** is open.
- **Carried from N8e-N8g:** S1 pose 1 has 2 umbra leaks against VSM's 0. The quarry's lit-darkened margin stays thin (55-60 against 87-88). S2's camera phases (0.68 ms) stay above VSM's 0.35.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8h/`):
  - `final-*`: the final lease's screens, timing, fresh and order processes, memory runs, and S1-S3 (`final/S1/scores.json`, `final/S2|S3/comparison.json`, `n8-score.log`, `S1-score.log`).
  - `ue/`: Unreal memreports, CSVs, logs and WDDM samples.
  - `m1`-`m3-*`: screening memory runs. `p1-*`-`p3-*`: screening leases.
  - `variants/`: the pool diff and shaders, the 48-byte diff. `edits/`: the scripted edits.
  - `scripts/`: `ue-mem.py`, `mem-runs.py`, `gpumem.ps1`, `mem-summary.py`, `timing2.py`, `uecsv.py`.
  - `counts-hashes.txt`: every counts and screen hash. `final-binaries.sha256`, `final-source.diff`, `runs/`.

  Raw dumps over 5 MB (counts, hits, HDR and world arrays, S1-S3 frames) were deleted after hashing and scoring. They are listed in `deleted-raw.txt`; hardlinked S2/S3 references and PNGs are kept.
- **Build and environment.**
  - Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, Windows.
  - Validation was off for timing and memory, and on for S1-S3.
  - Unreal 5.7.4 CL51494982, packaged Development, D3D12 SM6.
- **Gate.**
  - `cargo test -p renderlab --lib -j 2` exited 0 inside the final lease: 106 passed, 0 failed, 107 ignored.
  - Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
  - The viewer's `MEMORY_HINTS` change is compile-checked only: no window was opened.
- **Contention.** `mypaper-wallpaper` ran intermittently. Contended visits (primaries 0.47-0.80 ms) are excluded from the ranges. VALORANT ran between the screening leases and the final one, and no GPU work ran during it. No window, focus change or capture was used.
- **Leases.** Eight under the shared lock with the 4/2 GiB guard, 1,503 s in total: ue-mem 29, p1 7 and p1b 4 (both failed in the smoke step, on bind-group validation), p1c 109, m1 169, p2 320, p3 313, final 552. Available memory at every start was at least 9.9 GiB.
- **Time (UTC).**
  - 2026-11-06 19:40-19:50: reading and the Unreal memory runs.
  - 19:50-20:10: pool implementation and screens.
  - 20:10-20:25: memory runs, the 32-byte format and its lease.
  - 20:25-20:33: 48- and 36-byte screens.
  - 20:33-20:45: final layout, then a PC restart and a deferral while a game ran.
  - 2026-11-06 23:49 to 11-07 00:01: final lease.
  - Then scoring, analysis and this record.

  GPU leases total 25 minutes; implementation and analysis took the rest.
