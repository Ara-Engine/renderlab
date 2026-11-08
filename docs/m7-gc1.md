# M7-GC1: GI cost on the quarry

Unit GC1, branch `t3/m7-gc1` (from main `fa3b74b`; main merged at `9f98869`, PP1). It continues [M7-G2b](m7-g2b.md)'s cost work on the world radiance cache, which has been on by default since G2b. Rules: Islam's overnight push (AGENTS.md, 2026-11-08). A change ships when it is clearly better and not visibly worse. Verification is lib tests inside a lease, inspected images on both paths and one timing run. Claim constraints are in [m7-p0-claims.md](m7-p0-claims.md) (W1-W3: no counter averaging, no ranked updates, no jitter, a fixed query point). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference. Gaps are listed under "GI" in [m7-gaps.md](m7-gaps.md).

## 1. Status

FINAL_STATUS

**Kept (all quality-neutral on the quarry, section 4):**
1. **2^20 table slots** (was 2^19). On the quarry, buckets had filled. About 280 claims a moving frame failed on RT (1,100 on non-RT), those cells were requested again every frame, and lookups walked crowded buckets, which was the shading pass's largest hotspot in G2b's trace. The table is 224 MiB, under the non-RT device's 256 MiB buffer limit.
2. **Gathers stop once every hit value they read is final** (`gstop2`). A cell's next gather would re-trace the same fixed rays against the same values and replace its value with itself.
3. **Binned-SAH runtime software BVHs** (the non-RT merged world and full-detail meshes): the same hits, fewer node and triangle tests (section 3.3).

**Screened and recorded:** gather stride 2 (half the gather rays; same quarry image; section 3.4), parallel bucket-key loads and a probe load-before-compare-exchange (section 3.5).

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
- An Nsight trace in motion was attempted in lease `a2`. The shared guard stopped it at 1.36 GiB of available RAM: Nsight's capture buffer plus the quarry, under other units' load. TRACE_RESULT

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

GS2_RESULT

### 3.5 Lookup and probe micro-changes (screened)

- **`lkw`, parallel bucket keys.** When a lookup's start slot holds another key, the bucket's other seven keys load in one round trip instead of up to seven dependent ones, in shading (`gi_hot_settle`) and in the kernels' `gi_find`. It finds the same slot.
- **`plc`, probe load before compare-exchange.** The tile table is checked with a plain load first, since most inserts meet a key already listed.

Lease `f1` (M, binary `b3`, the variant's shaders in their own process; cache-on and cache-off visits interleaved):

| | RT shading on / off | Non-RT shading on / off | Probe p50 |
|---|---:|---:|---:|
| Final code | 1.603-1.608 / 0.589 ms | 1.320-1.426 / 0.544 ms | 431 / 445 µs |
| `lkw` | **1.501-1.514** / 0.655 ms | — | 430 µs |
| `lkw` + `plc` | 1.501-1.509 / 0.654 ms | 1.409-1.437 / 0.540 ms | 435 / 448 µs |

- **`lkw`: rejected.** It cuts RT's cache-on shading by 0.1 ms but makes the cache-off shading 0.066 ms slower (the same code is composed either way). On non-RT it reads 0.05-0.09 ms slower on three visits of four. Not clearly better on both paths: a codegen effect of the kind [shader-codegen hazards](m7-r1.md) records.
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

FINAL_QUALITY

## 5. Memory

MEMORY_RESULT

## 6. Claim notes

- **W1.** Values are still replaced by each gather's fixed-direction quadrature. The early stop only skips gathers that would replace a value with itself. The changing/final mark is a schedule state; no field counts samples.
- **W2.** No cell is compared with another and no budget picks cells. The stop rule is the same kind of per-cell rule as G2's retry: a cell's own gather decides its own next gather from the states of the cells its fixed rays met. Query points are unchanged.
- **W3.** Lookup weights are unchanged; `lkw` finds the same slot.
- **W8.** The SAH world BVH is still one global structure traversed from the top per ray.

## 7. Changes to shared lines

- `shaders/gi_shade_entry.wgsl`: one marked line in `gi_shade_cells`'s loop (the w mark). G2c's `gs_shade` body is untouched.
- `src/program.rs`: the cell entry is read through `gi_shader`, so a lease can screen it. This is the same two-line change G2c made.
- `scripts/m7-g2-run.py`: a deferred lease attempt (exit 3) now retries into its existing run directory instead of crashing, and `RENDERLAB_G2_TRACE_MS` sets the Nsight window.
- `IndirectSettings::default().slots_log2` is 20. Harnesses that name G2's settings (`g2_variant("g2")`) keep 19.

## Evidence

All under `D:/Coding/renderlab-work/m7-gc1`: `runs/a4` (baseline and variant cost, cut profiles, static captures, `score.log`), `runs/f1` (final code), `runs-sv/f1` (SV), `analysis/` (crops), `variants/` (screened shaders), `tools/` (`cost.py`, `compare.py`, `crop.py`, trace scripts). Raw captures over 5 MB are deleted once scored; hashes in `deleted-raw.txt`.
