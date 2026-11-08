# M7-GC1: GI cost on the quarry

Unit GC1, branch `t3/m7-gc1` (from main `fa3b74b`; main merged at `9f98869` (PP1), `e5c4623` (G2c, TP1, VF1, RTDEF2) and `2d1c084` (MP1, MQ1)). It continues [M7-G2b](m7-g2b.md)'s cost work on the world radiance cache, which has been on by default since G2b. Rules: Islam's overnight push (AGENTS.md, 2026-11-08). A change ships when it is clearly better and not visibly worse. Verification is lib tests inside a lease, inspected images on both paths and one timing run. Claim constraints are in [m7-p0-claims.md](m7-p0-claims.md) (W1-W3: no counter averaging, no ranked updates, no jitter, a fixed query point). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference. Gaps are listed under "GI" in [m7-gaps.md](m7-gaps.md).

## 1. Status

Final code: binary `b3` (lease `f1`, timing; `f2`, images and memory), then `b5`: `b3` plus the slot clamp of section 3.1, merged with main at `e5c4623` (G2c's cell shading and lookup fill): lease `f5`, the lib suite. Then `b6`: main at `2d1c084` (MP1's mixed-program cell shading, MQ1) merged, with the mark moved after MP1's store seam. Lease `f6` ran the lib suite and naga checks on `b6`. Its image step, and the retry `f7` (images, a timing check, Nsight traces), were stopped by the shared guard: another unit's build script, then available RAM under 2 GiB. **So the timing and images in this document are `b3`'s**, which holds all of GC1's changes except the slot clamp (a no-op on the quarry's devices) and the move of the mark after MP1's seam (the same value). The merged code's own GI changes (G2c, MP1) were measured by their units. Main's code is lease `a4` (binary `b1` with main's shaders). Functional leases on the shared GPU, not quiet windows: ranges, not certified timings. The cache-off frame in `f1` is 0.5-1.1 ms heavier than in `a4` because main gained PP1's post-processing between them, so the increments are what compare.

| Camera moving, quarry pose 0, over the cache-off frame of the same visit | Main (`a4`) | **GC1** (`f1`) |
|---|---:|---:|
| **RT increment** (p50 of 4 visits) | +1.98-2.22 ms (2.08) | **+1.60-2.01 ms (1.68)** |
| … shading pass on / off | 1.84 / 0.59 ms | **1.60** / 0.59 ms |
| … cache passes p50 | 831 µs | **645 µs** |
| **Non-RT increment** (p50 of 4 visits) | +2.75-2.95 ms (2.84) | **+1.90-2.98 ms (2.51)** |
| … shading pass on / off | 1.47-1.66 / 0.54 ms | **1.32-1.43** / 0.54 ms |
| … cache passes p50 | 1,534 µs | **1,332 µs** |
| Cut: gather queue empty at frame (RT / non-RT) | 29 / 115 | **23 / 93** |
| Cut: gathers in 160 frames (RT / non-RT) | 671k / 652k | **540k / 528k** |
| Quarry TR1x64, all-covered RMS at −14 (RT / non-RT, pose 0) | 7.388 / 7.242 | 7.390 / 7.241 |

**Against Lumen** (A, [M7-U0](m7-u0.md): Lumen Epic's GI pass on the quarry is 1.24 ms HW and 1.21 ms SW). The 2x target needs 0.62 / 0.61 ms. GC1 still misses by about 2.6-3.2x on RT and 3.1-4.9x on non-RT (G2b's reading: 2.2-2.6x and 4.3-4.7x, on frames that still carried the cut's drain). Section 2 explains why main's moving cost read higher than G2b's.

**Verdict.** About −0.4 ms (RT) and −0.3 ms (non-RT) a moving frame, and cuts settle about a fifth sooner. The quarry image is the same (one 55 x 40 px RT population moves 0.05 codes) and the RT SV image is smooth; non-RT SV is unmeasured on the final code (section 4). **The GI budget is still missed.** The largest remaining stage is the shading pass's +1.0 ms (RT) / +0.8 ms (non-RT) increment.

**Kept (section 4 has the images):**
1. **2^20 table slots** (was 2^19). On the quarry, buckets had filled. About 280 claims a moving frame failed on RT (1,100 on non-RT), those cells were requested again every frame, and lookups walked crowded buckets, which was the shading pass's largest hotspot in G2b's trace. The table is 224 MiB. A device whose storage-binding limit is wgpu's default 128 MiB gets 2^19 (section 3.1).
2. **Gathers stop once every hit value they read is final** (`gstop2`). A cell's next gather would re-trace the same fixed rays against the same values and replace its value with itself.
3. **Binned-SAH runtime software BVHs** (the non-RT merged world and full-detail meshes): the same hits, fewer node and triangle tests (section 3.3).

**Screened and rejected:** gather stride 2 as the default (half the gather rays and the same quarry image, but a visibly finer mottle on SV's flat panels; section 3.4); parallel bucket-key loads and a probe load-before-compare-exchange (not clearly better on both paths; section 3.5).

**Not done (recorded in the gaps):** a half-resolution lookup with geometry-aware upsampling (a new pass, buffer and shading binding, over this time box), level hysteresis, and an on-screen-first cut gather order.

## 2. Where the moving frame goes (M, before)

Lease `a4`, binary `b1` with main's shaders (`variants/base`): main `fa3b74b`'s code. Quarry pose 0, 1920x1080, development build, RTX 4070 SUPER. Per visit: a cut, 160 still frames so the cut's capped gathers drain, then 24 frames moving 1 cm a frame. Four unprofiled visits give the frame span and the shading pass (p50 per visit; increments are over the cache-off frame of the same visit). A fifth, profiled visit gives per-pass p50s and the kernels' counters. The lease is functional, not a quiet window: ranges, not certified timings.

| Stage, camera moving | RT | Non-RT |
|---|---:|---:|
| Frame span, cache off | 1.54-1.57 ms | 2.75-2.97 ms |
| **Increment, default cache** | **+1.98-2.22 ms** | **+2.75-2.95 ms** |
| … shading pass (lookup, ES1, bounce reads), on minus off | **+1.25 ms** (0.59 → 1.84) | **+0.92-1.12 ms** (0.54 → 1.47-1.66) |
| … cache passes p50 | 831 µs | 1,534 µs |
| … probe | 466 µs | 448 µs |
| … setup (query points of new cells) | 44 µs | 444 µs |
| … rays (their visibility) | 29 µs | 173 µs |
| … gather | 74 µs | 280 µs |
| … light / finish / claims / cell shading | 11 / 59 / 32 / 15 µs | 94 / 55 / 32 / 11 µs |
| Counters a moving frame: new cells, gathers, gather rays | 178, 469, 23.4k | 180, 353, 16.7k |
| Bucket-full claims / listed misses a frame | 276 / 270 | 1,131 / 557 |

- **The shading pass is now the largest stage on both paths.** G2b measured it at +0.61 ms (RT) and +0.60 ms (non-RT) on older main. Main's shading grew since then (cache-off shading 0.43 → 0.59 ms with LC1, DC1 and R1), and the cache's increment grew with it. G2b's Nsight trace put the lookup's bucket walk at the top of that pass. The walk's length is what 2^20 slots cut (section 3.1).
- **G2b's moving frames still carried the cut's drain.** With 160 settle frames, moving frames create about 180 cells and run 350-470 gathers, against G2b's 690-755 cells and 1,100-1,355 gathers. So the non-RT gather is 280 µs here, not 736.
- An Nsight trace in motion was attempted in lease `a2`, at 200 ms with the window starting 45 s in. The shared guard stopped it at 1.36 GiB of available RAM: Nsight's capture buffer plus the quarry, under other units' load. The partial capture had also started during pipeline admission, before the camera moved. The retry in `f7` (100 ms window from 120 s, 80,000 motion frames; `RENDERLAB_G2_TRACE_MS`) never reached its trace steps. **No new Nsight profile exists for GC1.** Stage costs here come from the cache's per-pass timestamps and the shading pass's GPU span; the hotspot attribution is G2b's trace (A, `m7-g2b.md` section 5).

## 3. Changes

### 3.1 2^20 slots (kept)

Same lease, same process, interleaved visits (M, `a4`):

| | RT 2^19 | RT 2^20 | Non-RT 2^19 | Non-RT 2^20 |
|---|---:|---:|---:|---:|
| Increment | +1.98-2.22 ms | **+1.69-1.74 ms** | +2.75-2.95 ms | **+2.45-2.78 ms** |
| Shading pass increment | +1.25 ms | **+1.01 ms** | +0.92-1.12 ms | **+0.79 ms** |
| Cache passes p50 | 831 µs | 663 µs | 1,534 µs | 1,545 µs |
| Bucket-full claims / misses a frame | 276 / 270 | 3 / 162 | 1,131 / 557 | 24 / 169 |

- **Why.** A cut fills the table with every visible cell, its parents, and two generations of gather hit cells. At 2^19 slots, 8-slot buckets filled: claims failed, the same cells were listed as misses every frame, and a lookup for a missing key walked to the bucket's end. Every lookup that isn't at its key's start slot pays one dependent load per step, in shading, the probe and the gathers.
- **Cost.** Memory: +112 MiB of table, +12 MiB of queues, +8 MiB of outgoing radiance (section 5). A cut also clears a larger table.
- **Image.** Same as 2^19 on every quarry population (section 4).
- **Device limits.** Shading binds the whole table, so a device must allow a 224 MiB storage binding. The SV rig's non-RT device keeps wgpu's default 128 MiB limit, and the first SV run failed to create the lighting bind group (lease `f2`). The quarry harness's devices request higher limits. `gi_slots` now clamps the slot count to the largest power of two whose table fits the device's binding and buffer limits, so such a device gets 2^19, G2b's table. That also covers the opt-in 2^21, which G2b found failing on the non-RT device.

### 3.2 Gathers stop on final hits (kept)

G2's schedule gives every cell an event reaches G = 3 gathers, plus a retry when a gather met hit cells without a value. A gather re-traces the cell's fixed directions and replaces its value (W1 form 1). So once every hit value it read was final, another gather writes the same value again.
- **Mark.** Cell shading marks a value as changing (`gi_outgoing` w 0.75) while its cell still has gathers scheduled, and final (w 1) once it has none. This is one marked line in `gi_shade_cells`'s dispatch loop, outside G2c's `gs_shade` body.
- **Hit status.** `gi_hit_radiance` reports what another gather could change. A hit can be final, changing, missing (a coarser stand-in while the hit level's cell is absent or not shaded yet), or settled-none. Settled-none means no value can come: the hit level's cell is an orphan, or a tertiary cell's gather met an absent cell, which it never requests, and no coarser level has a value. For an orphan, the coarser value it reads is final too.
- **Rule.** A gather whose every hit was final or settled-none ends its cell's schedule. Missing hits keep G2's retry. Changing hits keep the G gathers.
- **What reschedules.** Events that change hit values (new cells, lighting, environment and caster steps) schedule their cells again, as before.
- **Determinism.** Gathers read only the previous frame's outgoing radiance, which cell shading writes after the gather pass.

A first form (`gstop`) counted orphan hits as missing. Most moving gathers meet one, so it removed only about 10% of gathers.

| (M, `a4`) | RT main | RT `gstop2` | Non-RT main | Non-RT `gstop2` |
|---|---:|---:|---:|---:|
| Gathers / gather rays a moving frame | 469 / 23.4k | **296 / 14.7k** | 353 / 16.7k | **265 / 13.6k** |
| Gather pass, moving | 74 µs | 66 µs | 280 µs | 275 µs |
| Cut: gathers over 160 frames | 671k | **544k** | 652k | **529k** |
| Cut: gather queue empty at frame | 29 | **23** | 115 | **93** |
| Cut: frames over 10 ms | 3 | 3 | 45 | **35** |

The moving gather pass barely moves (−8 µs RT, −5 µs non-RT). It is bound by its slowest workgroups' ray latency, not by the number of gathers. The cut is where it pays: a fifth fewer gathers, and on non-RT bounce settles about 22 frames (0.4 s at 60 fps) sooner.

### 3.3 SAH runtime software BVHs (kept)

`near_field::build_bvh_sah` splits at the binned surface-area-heuristic plane: 16 bins per axis over the centroids, ties to the lower axis and bin, deterministic. It keeps R0c's record layout and eight-triangle leaves. It falls back to the median build if the tree is deeper than 15 node levels, which is what the software traversal's 48-entry stack holds. `gi_bvh::build` (the merged world and full-detail meshes) uses it. Cooked packages keep the median `build_bvh`, because their bytes are cache keys. The rays find the same hits. A unit test checks that every triangle lands in exactly one leaf of at most eight, within the depth bound, on an uneven scene.

**Effect (M, non-RT, moving; lease `f1` binary `b3` against lease `a4`'s 2^20 run on the median build, both with 2^20 slots):**

| Pass, p50 | Median BVH | SAH BVH |
|---|---:|---:|
| Setup (probe lines of new cells) | 418 µs | **335 µs** |
| Rays (visibility of new cells) | 174 µs | **142 µs** |
| Gather | 285 µs | **223 µs** (with `gstop2`) |
| Light | 89 µs | **74 µs** |

That is about 18-22% off each software-ray pass. The comparison crosses two leases, so it is indicative. Only the software-ray passes moved; the probe (443 → 445 µs) and finish (53 → 54 µs) stayed put. The quarry's world build takes about 1-2 s at `set_indirect_near_field`, as before (I: binned SAH is O(n) per level against the median build's sort).

### 3.4 Gather stride 2 (screened; not the default)

`gather_stride: 2` (G2b's opt-in) traces the blocked directions k % 2 == 0 at twice the weight: a fixed subset, no ranking.

| (M) | Default | Stride 2 |
|---|---:|---:|
| Gather pass, moving (RT / non-RT, `f1`) | 52 / 223 µs | 39 / 183 µs |
| Gather rays a moving frame (RT / non-RT) | 12.7k / 11.6k | 6.4k / 5.0k |
| Cut worst frame (RT / non-RT, `f1`, `f2`) | 12.4 / 13.4 ms | 10.7 / 12.7 ms |
| Quarry TR1x64 all-covered, pose 0 (RT / non-RT, `a4`) | 7.388 / 7.242 | 7.393 / 7.227 |
| SV frame RMS at frame 48, gaps / overview pose (RT, `f2`) | 2.86 / 1.54% | 2.97 / 1.66% |

**Not the default.** The quarry can't tell the two apart, but SV's flat panels show a finer, slightly more visible mottle at stride 2 (`analysis/f2-sv-gaps-gs2.png`: default | stride 2). Each cell's fixed-direction quadrature error grows with fewer directions, as G2b found for finer cells. It stays an opt-in setting.

### 3.5 Lookup and probe micro-changes (screened)

- **`lkw`, parallel bucket keys.** When a lookup's start slot holds another key, the bucket's other seven keys load in one round trip instead of up to seven dependent ones, in shading (`gi_hot_settle`) and in the kernels' `gi_find`. It finds the same slot.
- **`plc`, probe load before compare-exchange.** The tile table is checked with a plain load first, since most inserts meet a key already listed.

Lease `f1` (M, binary `b3`, the variant's shaders in their own process; cache-on and cache-off visits interleaved):

| | RT shading on / off | Non-RT shading on / off | Probe p50 |
|---|---:|---:|---:|
| Final code | 1.603-1.608 / 0.589 ms | 1.320-1.426 / 0.544 ms | 431 / 445 µs |
| `lkw` | **1.501-1.514** / 0.655 ms | — | 430 µs |
| `lkw` + `plc` | 1.501-1.509 / 0.654 ms | 1.409-1.437 / 0.540 ms | 435 / 448 µs |

- **`lkw`: rejected.** It cuts RT's cache-on shading by 0.1 ms but makes the cache-off shading 0.066 ms slower (the same code is composed either way). On non-RT it reads 0.05-0.09 ms slower on three visits of four. Not clearly better on both paths: a code-generation effect like the one R1 recorded for its inlined hit shading ([m7-r1.md](m7-r1.md) section 4).
- **`plc`: rejected.** No measurable probe change.

## 4. Quality (M)

**Quarry, scored against U0b's 4,096-spp references** (`scripts/m7-g2-score.py`, every registered population plus `crop` and `hollow`, −14 / −10 stops). TR1 over 64 still frames after a reset and 12 frames: the shipping image. Lease `a4`.

| TR1x64, all-covered RMS / pinholes | RT pose 0 | RT pose 1 | Non-RT pose 0 | Non-RT pose 1 |
|---|---:|---:|---:|---:|
| Main | 7.388 / 142 | 7.387 / 126 | 7.242 / 128 | 7.232 / 125 |
| `gstop2` | 7.388 / 141 | 7.389 / 127 | 7.241 / 124 | 7.232 / 125 |
| 2^20 slots | 7.388 / 142 | 7.387 / 127 | 7.241 / 127 | 7.231 / 125 |
| Gather stride 2 | 7.393 / 143 | 7.389 / 125 | 7.227 / 129 | 7.217 / 123 |
| Cache off | 10.008 / 74 | 10.018 / 71 | 10.007 / 74 | 10.018 / 71 |

- No population is worse than main's by more than 0.05 codes at either exposure for any of them, except `gstop2`'s RT pose-1 hollow (+0.05 / +0.19). That is a 55 x 40 px region, within the cache's run-to-run spread there (`tools/compare.py`).
- **Inspected** (`analysis/a4-{rt,nonrt}-crop.png`: reference | main | `gstop2` | 2^20 | stride 2, Islam's crop at −14 stops): no visible difference on either path. The cave above the dark pyramid rock keeps G2's blotchy mottle in all of them.
- **1x captures.** The first static capture of each process is taken while TR1 is still on (TR1b's harness leaves TR1 off only after its first TR1 capture, as G2b recorded). Its 209 / 263 pinholes against 1,459 / 1,900 are that artifact, not a variant's. The later 1x captures match main within 0.02 codes.

**Final code** (lease `f2`, binary `b3`, TR1x64; `tools/compare.py` against main in `a4`):

| All-covered RMS −14 / −10, pinholes | RT pose 0 | RT pose 1 | Non-RT pose 0 | Non-RT pose 1 |
|---|---:|---:|---:|---:|
| Main | 7.388 / 142 | 7.387 / 126 | 7.242 / 128 | 7.232 / 125 |
| GC1 | 7.390 / 142 | 7.389 / 127 | 7.241 / 127 | 7.231 / 125 |

- **Non-RT:** no population worse than main's by more than 0.05 codes.
- **RT:** the same everywhere except the hollow under the central slab, +0.04 / +0.14 codes (pose 0) and +0.05 / +0.19 (pose 1), against main's 13.53 / 26.7 and cache-off's 22.84 / 31.08.
  - It is systematic, and comes from the early stop's settled-none rule. A tertiary cell no longer retries a hit whose cell another request creates a frame later, so a little multi-bounce goes missing in the enclosed hollow.
  - Not visible: `analysis/f2-rt-hollow.png` shows reference | main | GC1 at −10 stops, 5x.
- **Inspected:** `analysis/f2-{rt,nonrt}-crop.png` (reference | main | GC1 | cache off, Islam's crop at −14 stops) are indistinguishable from main.

**SV test scene** (G1's harness, `bounce` section, frame 48 after a reset, frame-relative RMS against G2's multi-bounce 4,096-spp references; lease `f2`, binary `b3`):

| RT, frame RMS gaps / overview pose | Cache off | Sky only | **GC1 default** | Gather stride 2 |
|---|---:|---:|---:|---:|
| Frame 48 | 98.12 / 46.19% | 20.61 / 11.93% | **2.86 / 1.54%** | 2.97 / 1.66% |

G2 read 2.62 / 1.59% without a gather cap, and G2b's cap made SV settle more slowly. The default here is smooth on both poses (`runs-sv/f2/rt-bounce-bounce-*.png`).
- **No main-code SV number to compare in this window.** The base-shader SV step never ran: `f3` was stopped by another unit's build script and `f4` was withdrawn for the merge.
- **Non-RT SV is not measured on the final code.**
  - In `f2` it found the 2^20 table over the SV rig's 128 MiB binding limit. That found the bug `gi_slots` now handles: such a device gets 2^19 slots, G2b's table and SV image (A: G2c's non-RT gaps-pose RMS at frame 48 is 6.10%).
  - In `f5`, on main merged at `e5c4623`, the SV rig's non-RT device failed earlier, at main's `post primary surface` pipeline: a group-0 binding-10 struct of 128 bytes against a 112-byte `min_binding_size`. That is outside GC1's files and was reported to root.
  - The slot clamp is therefore exercised on a default-limit device only by construction (I), not by a run.

## 5. Memory

Lease `f2` (M), `memory` section: the allocator after a settled pose 0, with the cache's buffers live.

| | RT off | RT GC1 | Non-RT off | Non-RT GC1 |
|---|---:|---:|---:|---:|
| Allocator reserved (allocated) | 2,734 (2,554) MiB | **3,022 (2,850) MiB** | 3,370 (3,218) MiB | **3,722 (3,553) MiB** |
| Cache table / work / bounce buffers | — | 224.0 / 56.2 / 28.0 MiB | — | 224.0 / 56.2 / 28.0 MiB |
| Software ray geometry (merged world, SAH) | — | — | — | 38.7 MiB |
| Unreal, quarry, DXGI LocalUsedMB (N8h, A) | | VSM RT on 3,946 MiB | | VSM no-RT 3,912 MiB |

**Below Unreal's on both paths,** by 924 MiB (RT) and 190 MiB (non-RT), compared as G2 did (our allocator against Unreal's DXGI total, which was measured without Lumen, so the margin is conservative, I). The 2^20 table adds about 132 MiB over G2b's cache. The non-RT margin is now thin: a device or scene that needs the room can set `slots_log2: 19` at the 2^19 cost of section 3.1.

## 6. Claim notes

- **W1.** Values are still replaced by each gather's fixed-direction quadrature. The early stop only skips gathers that would replace a value with itself. The changing/final mark is a schedule state; no field counts samples.
- **W2.** No cell is compared with another and no budget picks cells. The stop rule is the same kind of per-cell rule as G2's retry: a cell's own gather decides its own next gather from the states of the cells its fixed rays met. Query points are unchanged.
- **W3.** Lookup weights are unchanged; `lkw` finds the same slot.
- **W8.** The SAH world BVH is still one global structure traversed from the top per ray.

## 7. Changes to shared lines

- `shaders/gi_shade_entry.wgsl`: two marked lines in `gi_shade_cells`'s loop, after the store (the w mark). G2c's `gs_shade` body and MP1's store seam (`gi_cache.rs` `CELL_STORE_SEAM`, which MP1's mixed-program entry rewrites) are unchanged. The mark depends only on the cell's state, so in a mixed frame every program's dispatch marks alike, whichever stored the value.
- `src/program.rs`: the cell entry is read through `gi_shader`, so a lease can screen it. This is the same two-line change G2c made.
- `scripts/m7-g2-run.py`: a deferred lease attempt (exit 3) now retries into its existing run directory instead of crashing, and `RENDERLAB_G2_TRACE_MS` sets the Nsight window.
- `IndirectSettings::default().slots_log2` is 20. Harnesses that name G2's settings (`g2_variant("g2")`) keep 19.

## 8. Housekeeping (M)

- **Lib suite**, `cargo test -p renderlab --lib` inside GC1 leases, exit 0. `f5` (main `e5c4623` + GC1): 182 passed, 0 failed, 153 ignored. `f6` (main `2d1c084` + GC1, the final code): **185 passed, 0 failed, 155 ignored**. Both include the new `sah_build_partitions_every_triangle` and MP1's seam assertion in `cell_shading_composes_for_reflecting_programs`.
- **Naga:** `kernels_validate_on_both_ray_sources` and the cell-shading composition pass on the final code and on every screened variant.
- **fmt** is clean. **clippy** is clean in GC1's files; the warnings that remain are in other units' files.
- **Lease discipline.** `f1` was cut at its root-ordered 15-minute cap (step 9 stopped at its start, lock released with its line). `f3` and `f6` were stopped by other units' `build-script-build.exe`, which the shared guard counts as foreign renderers. Later leases ran under `tools/order_guard.py`, which withdraws a waiting launcher when a newer `[root order]` appears.

## Evidence

All under `D:/Coding/renderlab-work/m7-gc1`: `runs/a4` (main and variants: cost, cut profiles, scores, `score.log`), `runs/f1` (final code: cost, RT cut profile), `runs/f2` (non-RT cut profile, images, memory, scores), `runs/f5` (merged code), `runs-sv/f2` and `runs-sv/f5` (SV), `analysis/` (crops), `variants/` (screened shaders), `tools/` (`cost.py`, `compare.py`, `crop.py`, trace scripts). Raw captures over 5 MB are deleted once scored; hashes in `deleted-raw.txt`.
