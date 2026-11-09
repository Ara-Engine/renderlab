# P-GI: world radiance cache cost, zero quality loss

Unit P-GI, branch `t3/p-gi` (from main `dc93b7e`). Phase: beat Ara on performance with zero quality loss (Islam, 2026-11-09: "let's start on optimizing and beating ara in performance without compromising even a bit in quality"). Scope: the world radiance cache GI on both paths (`src/gi_cache*`, its shaders, the probe, gathers, query points, cell shading and the shading-pass lookup). Quality bar: a pure optimisation is byte-identical; anything else must be equal or better on every crop and population. Claim constraints: [m7-p0-claims.md](m7-p0-claims.md) W1-W3. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## 1. Status

**Six byte-identical optimisations ship on both paths; the moving-frame GI target is missed.** Every quarry capture (two poses, 1x and TR1x64, cache on and off), U0b's TR1 pan and the game scene's fly path hash the same as main's on both paths (section 4). The changes shorten the shading lookup's bucket walk, skip work for gather-deferred cells, rewrite each filter neighbour once, speed gather and filter finds, and halve the dependent loads of software-ray leaves (section 3).

| Lease `a5` (M): main's shaders against P-GI's on one binary | RT main | **RT P-GI** | Non-RT main | **Non-RT P-GI** |
|---|---:|---:|---:|---:|
| Moving: shading pass (cache off 0.564-0.577 ms) | 1.400-1.408 ms | **1.358-1.375 ms** | 1.450-1.462 ms | **1.435-1.442 ms** |
| Moving: cache passes p50 | 774-779 µs | **775 µs** | 1,471 µs | **1,377 µs** |
| Moving: frame increment over cache off, p50 of visit p50s | +1.62 / +1.74 ms | **+1.65 / +1.65 ms** | +2.45 ms | **+2.26 ms** |
| Cut: 40 frames, sum (cache off: 105-109 / 142-148 ms) | 295.8 / 296.7 ms | **287.8 / 285.9 ms** | 434.8 ms | **383.7 ms** |
| Cut: worst frame | 13.7 / 13.8 ms | **13.8 ms** | 18.9 ms | **18.2 ms** |
| Cut: gather kernels, sum of 40 frames | 81.5 / 83.2 ms | **76.0 / 75.4 ms** | 128.8 ms | **85.4 ms** |
| Cut: filter kernels, sum of 40 frames | 26.2 / 26.1 ms | **23.6 / 23.4 ms** | 14.6 ms | **15.4 ms** |

- **Moving.** RT GI cost falls about 0.04 ms (shading); non-RT about 0.1-0.2 ms (software-ray passes: setup 347 to 317, gather 243 to 206, rays 146 to 133, light 70 to 62 µs). The target was GI under about 0.6 ms moving on both paths. **Missed**: the increment is still about +1.6 ms (RT) and +2.3 ms (non-RT).
- **Cuts.** Non-RT's capped gather drain, which P-NRT found dominates ARA1's first 88 quarry frames, is about a third cheaper per frame (gather kernels −34%), and the 40-frame cut costs 12% less. RT cuts cost about 3% less; the worst RT frame (13.8 ms, frame 2) is unchanged, dominated by new cells' rays, query points and finish. **Cut spikes stay far above Ara's 2-4 ms frame.**
- **Game scene** (API1's example, 48-frame fly path from start, development build, 1280x720, one run each, lease `a4b`): mean GPU span over frames 8-47, RT 20.6 to 19.2 ms, non-RT 27.7 to 27.4 ms. Images identical.
- **Memory:** unchanged. No buffer was added or resized (the filter's marks reuse the per-slot claim words), and the cache's buffer sizes match main's (section 6).
- **Quality: byte-identical**, so equal on every crop and population by construction. SV and the ARA1 harness were not rerun (section 4).
- Timings are functional leases on the shared machine with other units' builds running: ranges, not certified.

## 2. Profile of the base (M)

Binary `base` (main `dc93b7e`), lease `a1b`, `gpu_g2_quarry` cost and cut-profile sections, pose 0, 1920x1080, development build, RTX 4070 SUPER, functional leases on the shared machine (other units built on the CPU during them): ranges, not certified timings. Each of five visits per variant is a cut, 160 still frames, then 24 frames moving 1 cm a frame; the fifth visit is profiled.

| Camera moving | RT | Non-RT |
|---|---:|---:|
| Frame span, cache off / on (p50 per visit) | 3.22-4.49 / 5.06-5.69 ms | 3.51-3.62 / 6.05-6.38 ms |
| Shading pass, cache off / on | 0.573-0.586 / 1.409-1.683 ms | 0.571-0.573 / 1.447-1.469 ms |
| Cache passes p50 | 793 µs | 1,500 µs |
| … probe | 445 | 459 |
| … filter | 103 | 104 |
| … gather / setup / rays / light | 56 / 40 / 28 / 10 | 241 / 356 / 153 / 74 |
| … finish / claims / cell shading | 56 / 30 / 12 | 55 / 31 / 11 |

**Cuts** (cut profile, 100 frames): RT frames 1-6 12.9, 15.0, 16.9, 10.6, 13.0, 9.0 ms (cache off 3.2-5.1); non-RT 15.0, 16.7, 15.6, 15.6, 19.8, 16.9 ms (off 3.5-3.9), and non-RT's gather queue is still 36k cells deep at frame 96.

**Shading-pass attribution** (lease `a2`, RT, cost section, one process per shader variant; the ablations change the image and never ship):

| RT shading pass, camera moving, p50 per visit | ms |
|---|---:|
| Base (cache on) | 1.409-1.411 (two clean visits) |
| Lookup removed (`gi_prepare` returns at once) | 0.605-0.607 |
| Contrast-refine reads removed | 1.148-1.162 |
| Bucket walk removed (a key off its start slot reads as absent) | 1.225-1.233 |
| One bilinear corner instead of four | 1.442-1.461 |
| Cache off (same processes) | 0.536-0.566 |

- The lookup is about 0.80 ms of the RT shading pass. Contrast refine's finer reads are about 0.25 ms of it and the bucket walk about 0.18 ms; the rest (hashes, four start records, accumulation, ES1's visibility) about 0.37 ms.
- Reading one corner instead of four saves nothing: the four start-slot records already load in parallel. The cost is the dependent chains (refine's second lookup, the walk) at the pass's low occupancy, not bandwidth.

**Nsight.** The RT trace of `a1b` (`runs/a1b/trace-5-rt`) started 45 s after launch, before the trace section's camera motion, and its real-time shader profiler attributed samples only to the traced-primary pipeline (shader 1, the Interactive soft-shadow direction sampling, P-RTS's area): no GI shader has samples. The non-RT trace was stopped by the 2 GiB RAM guard after its file was written. Attribution here is by ablation instead.

## 3. Changes

Every kept change is a pure optimisation: the same cells, slots, rays, hits, visit order and arithmetic, so the images are byte-identical (section 4).

### 3.1 Kept

1. **Shading lookup, next slot with the start record** (`gi_hot_find`, [gi_api.wgsl](../shaders/gi_api.wgsl)). The start slot's record and the next slot's key word load in one round trip. A key whose start slot was taken usually sits one slot on, and it then costs two round trips instead of the walk's three. The same slot as the walk. `gi_hot_settle` is folded in.
2. **Gather fast path for deferred cells** (`gi_gather`, [gi_cache.wgsl](../shaders/gi_cache.wgsl)). Every lane of a gather workgroup reads the same queue entry, so whether the cell gathers this frame is the workgroup's. It is now broadcast with `workgroupUniformLoad`, and a cell outside this frame's slot group (the gather cap) or not queued skips the zeroed sums and their six-round tree reduction (about eight barriers). Those gathered zeros for it. A capped cut defers 140-200k cells a frame.
3. **Filter: each neighbour rewritten once a frame** (`gi_filter`). A cell next to several queued cells was rewritten once per queued neighbour, with the same words each time. The first item to mark the cell's per-slot claim word with this frame's mark does the rewrite and the others skip it. Marks are 1 to 0x3fffffff, below every claim value (whose band fills the top two bits, `gi_miss_claim`), so a later claim still wins its slot and resolve clears it; consecutive frames' marks differ. No new memory.
4. **Two-slot finds for gather hits and filter neighbours** (`gi_find_pair`, [gi_cells.wgsl](../shaders/gi_cells.wgsl)). As 1, in the kernels. The probe keeps `gi_find`'s one-word walk: there the extra load cost about 10-15 µs a frame (`lk1fk1` in lease `a3b`).
5. **Software rays: leaf triangles two at a time** ([gi_rays_software.wgsl](../shaders/gi_rays_software.wgsl), non-RT). Both triangles' indices, then both triangles' positions, load before either is tested, so a leaf costs half the dependent round trips. They are tested in leaf order, the second against the first's `best.t`.
6. **CPU: `prepare_gi`'s moved-caster loop** ([gi_cache.rs](../src/gi_cache.rs)) collected the per-placement skin flags by searching the draws once per placement, quadratic in placements (found by P-NRT). They are now collected once. Same values.

### 3.2 Screened and rejected (M)

| Variant | Identity | Why rejected |
|---|---|---|
| Nearest child kept in a register instead of a stack push and pop (`swtc`, software rays) | identical | Any-hit visibility rays (`rays`, `light`) 2-3x slower on non-RT (cut frame 2: 3.4 → 8.7-11.4 ms; moving 178 → 483-567 µs), though closest-hit setup and gathers were faster. Likely register pressure in the any-hit kernels (I). |
| Two-slot find in the probe too (`fk1`) | identical | Probe +10-15 µs a frame on RT (section 3.1 item 4). |
| GC1's parallel bucket keys on current code (`lkw`) | not run | GC1 measured it mixed (RT −0.1 ms, cache-off +0.07 ms, non-RT slower); the two-slot form keeps one extra load. |

## 4. Quality

**Byte identity** (M; `tools/ident.py`, sha256 of the raw f32 radiance captures; the game scene's RGBA8 hashes):

| Check | RT | Non-RT |
|---|---|---|
| Lease `a4b`, binary `final`: quarry static, poses 0 and 1, 1x and TR1x64, main's shaders (`variants/base`) against P-GI's | 4 / 4 identical | 4 / 4 identical |
| Lease `a4b`: U0b's TR1 pan, witnesses 97, 112, 128, 159 | 4 / 4 identical | 4 / 4 identical |
| Lease `a4b`: game scene fly path, frames 0, 8, ..., 40, 47 (`game-base` is main's build, `game-final` P-GI's) | 7 / 7 identical | 7 / 7 identical |
| Lease `a3b`: every candidate (`all`, `lk1fk1`, `swp2tc`) against a base control with the same plan, cache on and off | 8 / 8 identical each | 8 / 8 identical each |
| Main's shaders on the final binary against the base binary (`a4b` against `a1b`, same capture order) | identical | — |

- The last row shows the one Rust change (the CPU loop in `prepare_gi`) is neutral too.
- Cross-process identity holds for the base itself (RT, `a1b`: two processes, 4 / 4 identical). A capture plan that renders the cache-off variant first leaves different process state, so identity is always compared between processes with the same plan (`a2` first compared across plans and saw large differences that were the plan's, not the shaders').
- **Not rerun:** SV (G1's harness) and the ARA1 harness. Every change keeps the same cells, slots, rays, hits, visit order and arithmetic, and every image measured is byte-identical, so their images are expected identical too (I).

## 5. Timings

Lease `a5` (M), binary `final`, `gpu_g2_quarry` cost (five visits: a cut, 160 still frames, 24 moving 1 cm a frame) and a 40-frame cut profile (every frame in the profile ring), one process per shader set, interleaved: RT main, RT P-GI, non-RT main, non-RT P-GI, RT P-GI again, RT main again. Section 1 has the summary table.

**Cut frames 1-6** (ms): RT main 9.8, 13.7, 13.1, 9.4, 9.2, 9.1; RT P-GI 9.9, 13.8, 13.1, 8.9, 8.8, 8.7. Non-RT main 14.2, 16.0, 14.7, 14.7, 18.9, 14.1; non-RT P-GI 13.7, 15.8, 14.6, 14.6, 18.2, 13.6.

**Cut attribution, RT** (lease `a3b`, base control, kernel time per frame): frame 1: probe 0.76, setup 1.26, rays 1.00, finish 1.26, filter 0.97 ms; frame 2: setup 2.45, finish 1.83, gather 2.66, filter 2.05, rays 1.02 ms; frames 3-16: the capped gather 2.8-4.0 ms and the filter 0.5-1.3 ms. New cells' visibility rays, their query points and finish dominate frames 1-3; the gather drain the rest.

**Earlier screens** (lease `a3b`, same plan, one process each; process order made the base control's cut slower, so `a5`'s interleaved runs are the result): RT 40-frame cut 274.7 ms (base) against 239.5 (`lk1fk1`) and 236.0 (`all`).

Context: development build, RTX 4070 SUPER, 1920x1080, functional leases on the shared machine with other units building (P-NSQ, P-RTS, P-NRT); not quiet-classified. Implementation about 1 h 10 min; verification about 1 h of leases and analysis, plus about 1 h 30 min queued behind other owners, root orders and two compiler-quiet windows.

## 6. Memory

GI buffers are unchanged (M, `IndirectStats` in every run of `a1b` and `a5`): table 224 MiB (2^20 slots), work 56 MiB, bounce 28 MiB, filter sources 88 MiB; software ray geometry 38.7 MiB on non-RT. No buffer was added or resized: the filter's per-frame marks reuse the work buffer's per-slot claim words.

Allocator after a settled pose 0 (`memory` section): `a5`, binary `final`, RT reserved 3,103 MiB (allocated 2,938), non-RT 3,803 MiB (3,641). Lease `a1b` read 2,975 (2,752) and 3,611 (3,456) MiB for the base binary, with the section run after `static` in the same process; the difference is the process's history, not the cache (the same buffers).

## 7. Not done (recorded in [m7-gaps.md](m7-gaps.md), GI)

- **The moving-frame target (GI under about 0.6 ms on both paths) is missed.** Exact changes leave the lookup's dependent chains (refine's second lookup, about 0.25 ms; the walk, now shorter; the rest, about 0.37 ms) and the probe (about 0.45 ms). Reaching the target needs non-exact or architectural work, each with its own quality proof:
  - a lookup that shares records between pixels of a tile (the probe already finds every tile's cells; shading would need their slots, or a cooperative entry, which is the shading owners' file);
  - a probe that skips tiles whose cells it found last frame (needs a per-tile record and an exact invalidation rule);
  - a half-resolution lookup with an exact-where-linear upsample. The lookup is bilinear in lattice coordinates, not screen coordinates, so a screen-space upsample is not exact even on a plane; the fallback would carry most pixels.
- **Cut spikes stay well above Ara's 2-4 ms frame.** RT cut frames still run 8-12 ms (worst 12.3 ms, 14.3 before), dominated by new cells' visibility rays, query points, finish and the gather drain. Non-RT's drain runs about 100 frames at the gather cap. Faster drains without a visible settle difference need the cap's schedule changed, which W2 constrains (fixed rotation only) and which needs a settle-quality proof.
- **Still frames under TR1 never idle** (section 2 of the gaps entry): an exact idle needs a 256-frame zero-work confirmation.
- **Software traversal:** keeping the nearest child in a register made any-hit rays 2-3x slower (likely registers, I). A short-stack or shared-memory stack, measured per kernel, is untried.
- **Memory:** the cold record's 8 unused words (32 MiB) cross P-LTS's test file.

## 8. Housekeeping

- **Leases** (`scripts/p-gi-run.py`: the G2 runner under P-GI's name, on the shared lock and guards that `scripts/n8-run.py` uses): `a1` stopped seconds in by P-NRT's build script; `a1b` profile (both paths), its non-RT Nsight step stopped by the 2 GiB guard after the trace was written; `a2` ablations and traversal screens; `a3b` screens with base controls; `a4` withdrawn by its order guard for the 09:01 root order; `a4b` identity, game scene and lib; `a5` timing and memory. All 15 min or less, each after the latest root order (gated on owner release lines, `tools/hold.py`).
- **Lock handoffs.** The shared queued lease (`m7-u0b.py`) polls the lock every 0.5 s and reruns a process inventory before taking it; it lost every handoff to faster owners for about half an hour. `p-gi-run.py` shortens only its own waits to 50 ms; every check is unchanged.
- **Lib suite**, `cargo test -p renderlab --lib` on binary `final` inside lease `a4b`: **197 passed, 0 failed, 163 ignored**, exit 0. Naga: `kernels_validate_on_both_ray_sources` and `only_callers_compose_the_indirect_inputs` passed for every screened variant (`a2`, `a3b`).
- **Evidence:** `D:/Coding/renderlab-work/p-gi`: `runs/` (JSON, logs), `analysis/` (trace summaries), `variants/` (screened shaders), `tools/` (`cost.py`, `ident.py`, `a3cmp.py`, `final.py`, `trace.py`, `hold.py`, lease scripts). Raw captures and traces over 5 MB are deleted once summarized (`deleted-raw.txt`, with sizes and sha256).
