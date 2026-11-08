# M7-G2b: the GI cache on by default, cost, cut bounding, determinism

Unit G2b continues [M7-G2](m7-g2.md) on branch `t3/m7-g2b` (from main `5e3b963`, main merged at `aef74bd` and `71094d1`). It started under the strict G2 ruling ([milestone-7.md](milestone-7.md)) and finished under Islam's overnight complete-base push (2026-11-08, AGENTS.md), which superseded the strict default gate: **the world radiance cache is now on by default on both paths**, with bounce light and ES1's blocked sky specular. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference. Gaps are listed under "GI" in [m7-gaps.md](m7-gaps.md).

## 1. Status

| Item | Result | Verdict |
|---|---|---|
| Default | `IndirectSettings::default()` is on: G2's measured configuration (4 px cells, 2^19 slots, bounce over every blocked direction, three gathers an event) plus a per-frame gather cap. `IndirectSettings::off()` is the named switch. The cache allocates lazily on the first frame whose program calls an indirect input, so non-callers still hold none of it | **On by default** (push rule: beats cache-off on 20 of 22 quarry populations and Lumen on overall quarry error, G2; inspected images below) |
| Determinism | Both cross-process table differences G2 recorded are removed (section 3): tables word-identical across two repetitions in two processes on both paths, images identical | **Fixed** |
| Camera cuts | The 12-64 ms gather frames of a cut are bounded by a per-frame gather cap (32,768 cells hardware, 8,192 software): section 4 | **Bounded**; the first cut frame (cell creation) is not, and bounce settles in more than 8 frames after a cut (recorded) |
| Moving cost | Gather restructured to one lane per direction (non-RT gather 1.5 ms → 0.5 ms a moving frame); lookups stop at an empty slot (exact). Section 5 | **Improved; still over the ~0.45 ms budget** |
| Crevice mouths | Diagnosed: cell size alone explains them. 2 px cells bring TR1x64 pinholes to the cache-off level (RT 103 → 41 against 40; non-RT 97 → 55) and beat G2, es1open and cache-off on every quarry population but the hollow. Not the default: 2 px speckles the flat SV test scene, settles it worse, and its 448 MiB table exceeds the non-RT device's 256 MiB buffer limit. Section 2 | **Opt-in fix recorded; default keeps G2's 4 px** |
| Lib suite | `cargo test -p renderlab --lib` inside G2b leases: 147 passed, 0 failed (binaries `g2b-g`, `g2b-h`, `g2b-j`) | **Pass** |

## 2. Crevice mouths (item 1 of the G2 ruling)

**How other engines handle near-field occlusion at cell boundaries** (docs/research/gi.md, papers only):
- **Lumen** splits by distance: screen probes and per-pixel screen traces (and screen-space short-range AO) carry the near field at pixel resolution, and the world radiance cache is traced with a minimum distance so it carries only the far field. The near field is screen-space, which this project rules out for core lighting.
- **DDGI** handles probes behind thin walls: a Chebyshev visibility test on per-probe depth moments, a backface-ratio test that disables or relocates probes inside geometry, and normal/self-shadow biases. The quarry's dark cells are not behind a wall from the pixel: G2 found their query points on the same visible surface, 2-3 cm across it, under a roof 20-28 cm up.
- **Irradiance caching (Ward 1988)** adapts record density to the distance of occluders: finer records near crevices.
- **Hash-grid caches** (SHaRC, Solari, GI-1.0) pick a fixed cell scale by distance and continue paths past the cache at short range when terminating would leak.

**Measurement (M, runs `a1`, `c1`).** The same default candidate with cells of 3 and 2 px of footprint instead of 4 (2^21 slots), TR1x64:

| | RT pinholes p0 / p1 | non-RT pinholes p0 / p1 | RT all-covered RMS (-14/-10) | Losses to cache-off or es1open |
|---|---|---|---|---|
| Cache off | 40 / 45 | 40 / 45 | 9.98 / 15.76 | — |
| es1open (ES1's unoccluded diagnostic) | 22 / 25 | 19 / 21 | 7.36 / 10.05 | — |
| G2 (4 px) | 103 / 101 | 97 / 104 | 7.35 / 9.76 | 11-16 populations |
| 3 px | 76 / 77 | — | — | 5-6 (hollow, isolated, lit, solid-lit, background-centre) |
| **2 px** | **41 / 39** | **55 / 47** | **5.88 / 8.04** | **hollow** (and isolated at RT p1) |

2 px cells beat every comparator on every other population (Lumen HW Epic all-covered 8.41 / 11.65). So the mouths are a resolution problem, fixed by finer cells, with no per-pixel ray and no screen-space term: the measured fix the ruling preferred. Gather stride 2 and 4 and 1-7 gathers change nothing measurable at 2 px (section 5).

**Why 2 px is not the default tonight (M, runs `g1`, `e1`):**
- On the SV test scene (flat grey panels, at 1x) 2 px shows a visible fine speckle that 4 px does not, and settles worse even uncapped: gaps-pose frame RMS 3.74% / overview 2.88% against 4 px's 2.74% / 1.58%. Finer cells make each cell's fixed-direction quadrature error visible.
- The 2^21-slot table is 448 MiB: the non-RT test device's maximum buffer size is 256 MiB (the SV step failed to allocate it).
- It costs about +0.4 ms (RT) and +1.0-1.4 ms (non-RT) more per moving frame and about three times the cut work (section 5).

It stays an opt-in setting (`cell_pixels: 2.0, slots_log2: 21`) where the device allows it. A per-pixel short-ray check was not built: the cell-size measurement showed the cause, and non-RT shading has no ray source.

## 3. Determinism (M, runs `a1`, `b1`)

G2's tables differed across processes in a few slots. Two causes, both removed:
1. **Claim ties.** A gather's requests all carried one priority (the gathering cell's), and a pixel's cells share its priority, so two keys of one bucket could claim a slot with equal values and both "win". The claim value is now the generation band (pixels' over gathers') above the bits of key a that do not pick the bucket: unique within a bucket, fixed by the key (W2: it orders inserts, never which rays are cast). The probe's per-tile priority table went with it.
2. **Key-a collisions.** Different cells share a 31-bit key a about twice per cut frame (`63fbe195`: a level-5 and a level-4 cell); the request table kept whichever listed first. Request entries now also keep the highest inverted key b, only the request that raises it lists a miss, and only the miss matching the final value claims.

After both, four tables per path (two repetitions, two processes) are word-identical and the images equal (`runs/b1/determinism.txt`; before: `runs/a1`).

## 4. Camera cuts bounded in time (M)

**Gather cap.** A frame gathers at most `gather_cells` cells (default 32,768 with hardware rays, 8,192 with software rays). When more are queued, it gathers those with `slot % P == frame % P` (P the smallest power of two up to 64 that brings the queue under the cap); the rest wait with their schedule unchanged. A fixed rotation by slot (W2), deterministic. Still frames keep running until a cut's spread can have drained.

**Opt-in claim cap and coarse-first refill** (`claim_cells`): a frame creates at most N cells, by fixed key phase, and the first four frames after a cut read cells one level coarser. On SV the coarse-to-fine handover made frames 8-12 worse again and frame 1 reached 58% gaps-pose error: off by default.

CUTTABLE

## 5. Cost (M)

**Where the moving frame goes** (Nsight GPU Trace, real-time shader profiler, camera moving over existing cells, 2 px stride 4 candidate, `runs/d2`): RT per frame, shading 1.36 ms (0.43 without the cache), probe 0.66 ms, every other cache pass under 0.15 ms together; non-RT shading 1.08 ms and probe 0.62 ms. The shading pass's largest hotspot was the cache lookup's bucket walk (10% of the trace on one line, 15% with its neighbours: dependent loads stalling on memory).

| Change | Effect | Exact? |
|---|---|---|
| Gather: one workgroup per cell, one lane per fixed direction (G2: eight lanes, each tracing about six rays in a row) | Non-RT moving gather 1,503 → 522-536 µs p50; increment over cache-off +3.2-3.7 → +2.1-2.4 ms (functional runs) | Image within 0.034 codes of G2 (summation order) |
| Lookups stop at the first empty slot (a key is inserted in the first free or reusable slot from its start; slots are never emptied except by a whole-table clear) | Fewer loads per missing key in shading, the probe and gathers | Exact |
| Gather stride S (`gather_stride`, opt-in) | Gathers trace 1/S of the blocked directions: no measurable quality change at 2 px on the quarry; untested at 4 px on SV, so the default keeps S = 1 | — |

COSTTABLE

## 6. Inspected images (M)

INSPECT

## 7. Claim notes

Unchanged from G2 (W1-W3, C1-C5, C8). New pieces: the claim value and key-b choice are fixed by the key (W2: they decide which cell exists at a full bucket, never which rays an existing cell casts); the gather and claim caps defer work by fixed slot or key phases, never by any characteristic of a cell (W2); no jitter, counters or averages anywhere.

## 8. Changes to other units' assumptions

- `IndirectSettings::default()` is now on. Code that used it to mean "cache off" should use `IndirectSettings::off()`. G2b changed its own harness (`src/gi_cache_tests.rs`). Not changed (other owners' files): `src/reference_r0a_tests.rs` asserts the renderer starts with the cache off, and `src/iq1_tests.rs` restores `default()` after its cache-on captures.
- `scripts/m7-g2-run.py` takes a unit name (G2b's wrapper is `scripts/m7-g2b-run.py`), per-step settings (a fifth step field), and an Nsight `trace` step.

## Evidence

All under `D:/Coding/renderlab-work/m7-g2b`: `runs/a1`-`h1` (scores, counters, cut profiles, determinism summaries and hashes), `runs/*-sv` (SV images and scores), `analysis/` (inspected crops, trace hotspots), `tools/` (screen, crop, cut, SV and trace scripts). Raw captures and table dumps over 5 MB are deleted once summarized; references are G2's and U0b's, kept in place.
