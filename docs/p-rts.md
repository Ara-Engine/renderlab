# P-RTS: RT sun-shadow ray cost

2026-11-09, branch `t3/p-rts`, merged with main `bdec7b9` (P-GI exact GI cuts, v0.1.0).
Phase: beat Ara on performance with zero quality loss (docs/milestone-8.md). This unit
owns the RT path's sun-shadow rays: RTDEF2's adaptive budget, the Interactive shadow
kernels and their TR1 integration.

**Result: implemented, measured, and off by default.** Penumbra listing spends the
RTDEF2 budget only near shadow edges on TR1 cut, reset and moving frames. Turned on, it
cuts the game-base first cut from 21.6 to 5.6 ms CPU service, and the cut's shadow pass
from 17.5 to 1.7 ms. On the frozen ARA1 scenes, all eight non-quarry scenes stay
byte-identical, and the quarry has one pixel off by one code, which scores slightly
better against the reference. It fails RTDEF2's S2 large-light fixture, though: some
fringe pixels near a 0.04 rad light are not listed, so they lose up to 0.125 of their
shadowing on moving frames. Under the zero-loss rule (root's ruling, 2026-11-09)
`PENUMBRA_LISTING` is therefore `false`. The shipped default is byte-identical to main
(§4.1), and the mechanism stays one constant away for the follow-up in §6.

## 1. What the cut and moving frames spend

ARA1's own per-pass GPU timestamps isolate the cost (frozen final runs, 1920×1080).
`primary_ms` is the fused camera-ray + sun-visibility pass:

| Scene | still (8 rays) | moving (32) | first cut (256) | ms per ray per pixel |
|---|---:|---:|---:|---:|
| game-base | 0.74 | 2.33 | 17.45 | 0.067 |
| ara-pbr-oriented | 0.43 | 1.28 | 9.71 | 0.036 |
| quarry-daylight | 0.54 | 1.30–1.41 | 11.60 | 0.044 |

The cost is linear in the ray count: same camera rays, receivers and launch, and on
game-base (2.33 − 0.74)/24 and (17.45 − 0.74)/248 both come to 0.066–0.067 ms per ray
per pixel (about 31 G shadow rays/s). The other passes stay within noise across the
three states, so the cut's +17 ms and the moving +1.6 ms are shadow rays. Per-ray
traversal is already lean: `FORCE_OPAQUE | TERMINATE_ON_FIRST_HIT`, tmax clipped to the
light-space box, `PREFER_FAST_TRACE` BLAS/TLAS. A traversal tweak cannot close a ~10×
gap; the ray count has to fall. The planned Nsight GPU Trace was not taken: the GPU queue
was saturated by five units, and the linear per-ray decomposition already answers the
question a trace would.

Where the rays go: the matched scenes' sun is a 0.00465 rad disk. Almost every receiver
is fully lit or fully in umbra, so all of its 256 (or 32) rays agree, and its count is
exactly 0 or 256 whatever the budget. Only penumbra receivers, a thin band at shadow
edges, have budget-dependent counts.

How Ara and Unreal avoid the spike: Ara hybrid's sun shadows are filtered shadow maps
(moment pyramid, contact-hardening kernels; `shaders/look-shadow.slangh` in the
read-only Ara checkout), so a cut costs the same as a still frame. That flat cost comes
with Ara's 57% higher sun-shadow error in ARA1. Unreal's RT shadows trace about one ray
per pixel and denoise. Renderlab keeps its exact per-pixel estimator and stops spending
it where every ray would agree.

## 2. Change: penumbra-listed full budget

With `PENUMBRA_LISTING` on (off by default, §4), on TR1 frames whose budget exceeds 16 (cut 256, reset 64, moving/young 32):

1. **Probe** (the existing fused trace `primary_trace_interactive_deferred` and its edge
   repair): every receiver traces 16 probe rays. Eight are the budget's own first eight
   directions, and eight reuse that sequence over a cone twice the sun's. The probes
   only decide which pixels trace the budget.
2. **List** (`shaders/primary_penumbra_list.wgsl`): a receiver is listed when its 3×3
   window holds both a blocked and a visible probe. That covers a mixed pixel, or
   unanimous-0 and unanimous-256 pixels side by side (a sub-pixel edge).
3. **Budget** (`shaders/primary_penumbra.wgsl`, indirect dispatch, four pixels × eight
   lanes per group): each listed pixel re-derives its receiver with the probe pass's own
   arithmetic (raw camera ray, near-edge test, the repair kernel's correction), then
   traces all budget directions and writes production's count. The estimator body is
   `fresh_estimate`'s own text with a lane stride (`trace_source`), so directions,
   flags, tmax and the integer sum are production's.

A non-listed receiver takes the unanimous 0 or 256. That is production's value unless a
later budget ray would have disagreed: a "hidden fringe" whose whole window saw no
disagreement. §4 counts these against the unchanged main binary.

Held frames (budget 8) are untouched: they keep j and an exact ×1.0 slope, and no
extra pass runs. Programmable shading is untouched too: materials still read one 0..256
visibility per pixel, and no hit record, material input or program ABI changes. Non-RT
is unaffected; the passes exist only in TR1's RT trace module. The legacy Exact and
non-reconstructed Interactive pipelines keep their own estimator. The non-deferred
fallback (edge repair buffer over the device limit) keeps the full budget for every
pixel. Frame words: `PrimaryFrame.sampling.y` holds the probe count, `.z` the in-disk
probes (0 when not listing), `.w` the wide-probe cone scale (f32 bits). New memory:
one 4-byte-per-pixel list buffer (8.3 MB at 1080p, deferred-edge targets only).

Patent note: adaptive refinement of sampled soft-shadow visibility where samples
disagree is classic prior art (adaptive supersampling, Whitted 1980; Cook 1986; lazy
visibility evaluation, Hart et al. 1999). The pass uses no reservoir, no reuse across
pixels or frames, no denoiser and no variance-guided filter. The shadow-filter claims
mapped in [m7-p0-claims.md](m7-p0-claims.md) §3 (US 11,688,042; US 12,423,782; the
history-count filter-size claim) each recite a spatial filter applied to ray-traced
values. Here the window only decides which pixels trace more rays; every output is that
pixel's own ray count, unfiltered.

## 3. Variants screened

Same frozen ARA1 scenes, 960×540 quality captures against the unchanged binary, 1080p
streams for cost. "Diff px" is the most pixels differing in any of the 19 registered
frames, and "codes" the largest 8-bit change. Crop verdicts are ARA1's linear display
RMS against the path-traced references, candidate versus baseline.

| Variant | game-base | quarry | quarry crops worse | game cut / move primary ms | Verdict |
|---|---|---|---:|---|---|
| list every receiver | identical | identical | 0 | 19.6 / 4.0 | exact, slower: proves the budget pass reproduces production bit for bit |
| 8 in-disk probes, 5×5 | ≤3 px, 3 codes | ≤7 px, 1 code | 16 (≤2.3e-5 rel.) | 1.40 / 1.06 | hidden fringes |
| 8 probes, 7×7 / 9×9 | ≤2 px | ≤4 / 3 px | 12 / 11 | 1.74 / 1.29, 2.13 / 1.57 | fewer misses, costlier |
| 16 in-disk probes, 5×5 / 7×7 | ≤1 / 0 px | ≤4 / 3 px | 8 / 8 | 2.4 / 1.6, 2.3 / 1.8 | costlier, still misses |
| listed pixels trace 2× budget | ≤480 px, 6 codes | ≤408 px | 27 | 1.75 / 1.10 | **rejected**: sun-shadow crops worse |
| 8 probes on a 1.5× cone, 5×5 | identical | ≤4 px | 0 | 1.41 / 1.07 | better |
| 8 probes on a 2× cone, 3×3 | identical | 1 px, 1 code | 0 | 1.14 / 0.92 | best ARA1, **fails S2** (below) |
| **8 in-disk + 8 on a 2× cone, 3×3** | identical | 1 px, 1 code | 0 | 1.67 / 1.44 | final design; **fails S2** (§4.2), so off by default |

The 2×-cone-only probe failed RTDEF2's S2 fixture, which uses a 0.04 rad light (nine
times the matched sun). There, 280 fringe pixels on the cold frame that production gives
0.934–0.965 came out fully lit, with up to 6.6% visibility error; moving frames erred by
up to 0.16. When a blocker's shadow is no wider than the probe cone, widening spreads
the probes thinly over the real disk. The final design keeps the budget's own eight
in-disk directions for that regime and adds the eight wide probes for thin rim slivers
of a small sun.

## 4. Quality proof

### 4.1 Shipped default (listing off)

With `PENUMBRA_LISTING = false`, `trace_source` returns main's exact TR1 module text. No
penumbra pipelines are compiled, and the list buffer is a 20-byte placeholder. Lease
`quarry-ab` compared the default build with main `bdec7b9` (same ARA1 adapter):

| Scene (960×540, 19 registered frames) | Identical frames | Crops equal / better / worse |
|---|---:|---|
| quarry-daylight | 19/19 | 56 / 0 / 0 |
| game-base | 19/19 | 126 / 0 / 0 |
| ara-pbr-oriented | 19/19 | 56 / 0 / 0 |

Its quarry first-cut shadow pass is 11.66 ms against main's 11.68, the same cost. The
library suite passed 197, failed 0, ignored 163; the naga check covers the off and on
modules and the list pass.

### 4.2 Listing on (final design, not default)

Lease `final-2b` and `quarry-ab`, against main `bdec7b9`:

| Fixture | Result |
|---|---|
| ARA1 game-base, no-GI, no-reflections, alpha panel, 64 lights, fog, atmosphere, PBR | byte-identical, 19/19 frames each |
| ARA1 quarry-daylight | 18/19 identical; frame 75 has 1 pixel off by 1 code; its whole-image and surface-detail crops are 2e-7 *better* against the path-traced reference; 54 equal, 0 worse |
| RTDEF2 S2, 0.04 rad light, `TR1-Auto32`, three tracks × 16 checkpoints | **fails**: 45 of 48 visibility captures differ. Cold frame: 122 pixels lighter by 0.004–0.047. Moving frames: up to 69 pixels lighter by up to 0.125 (base counts 28/32 where the candidate's window was unanimous) |

S2's fringe is the failure root's rule names ("lost visibility"). The 2×-cone-only
probe lost up to 0.066 on the cold frame there. Adding the eight in-disk probes
narrowed the loss but did not remove it.

## 5. Timings

Windows 11, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14, Vulkan, release
builds. 1920×1080, the unchanged ARA1 adapter (`examples/ara1_capture.rs`), synchronous
single frame in flight. CPU service is submit plus GPU drain, excluding readback.
`primary` is the GPU timestamp span of the camera-ray and sun-visibility passes. The
values are medians of each process's segment medians. Still is frames 32–39, moving
64–71, first cut 72 (one per process), and the cut stream is 24 repeated cuts at frame
72's pose (`tools/p-rts/scenes.py`). Game and PBR come from two alternating processes
per binary; the quarry is one process per binary.

| Scene | Build | Still ms | Moving ms | First cut ms | Cut stream ms | primary still / moving / cut |
|---|---|---:|---:|---:|---:|---|
| game-base | main = default | 4.08 | 6.05 | 21.61 | 20.96 | 0.74 / 2.33 / 17.53 |
| game-base | listing on | 4.16 | 5.15 | 5.64 | 5.03 | 0.74 / 1.44 / 1.67 |
| game-base | Ara hybrid (ARA1) | 2.24 | 2.35 | 2.38 | — | — |
| ara-pbr-oriented | main = default | 3.01 | 3.87 | 12.83 | 12.23 | 0.46 / 1.25 / 9.73 |
| ara-pbr-oriented | listing on | 2.84 | 3.45 | 4.15 | 3.56 | 0.43 / 0.83 / 1.13 |
| ara-pbr-oriented | Ara hybrid (ARA1) | 1.95 | 2.00 | 2.18 | — | — |
| quarry-daylight | main | 3.27 | 4.22 | 15.89 | 15.27 | 0.50 / 1.32 / 11.68 |
| quarry-daylight | default (off) | 3.90 | 4.05 | 15.69 | 15.33 | 0.50 / 1.32 / 11.66 |
| quarry-daylight | listing on | 3.23 | 3.73 | 7.50 | 5.85 | 0.50 / 1.04 / 3.43 |
| quarry-daylight | Ara hybrid (ARA1) | 5.75 | 8.90 | 9.13 | — | — |

The shipped default does not change any of these costs; the main rows are its prices.
With listing on, the first cut is 3.8×, 3.1× and 2.1× cheaper, and moving frames lose
0.4–0.9 ms. The quarry beats Ara hybrid in every state, and game/PBR cuts land within
2× of Ara's. The small scenes' still frames stay above Ara's. Their sun-ray share is
0.43–0.74 ms; shading, GI and TR1 resolve make up the rest, outside this unit.

Contention: the GPU was leased exclusively. Other units' compilers ran during some
leases (`quarry-ab`'s preflight shows four), so CPU service carries that noise; the
`primary` GPU spans repeat to ±0.02 ms across processes. Builds: main adapter
`cb58e81…`, listing-on adapter `2bfa8c0…`, default adapter `4bd32b6…`; test binaries
`5f33906…` (S2 baseline, main `3a71de0`, GI-only differences from `bdec7b9`; S2 runs
with GI off), `314f81f…` (on), `a18a194…` (default).

## 6. Follow-up

The listing machinery (probe split, list pass, budget pass, frame words) is complete and
naga-validated. Two directions could make it pass S2 without losing ARA1's identity:

- An exact certificate for "fully visible" (a conservative cone/occluder bound, for
  example per-tile occluder depth ranges from the TLAS instance bounds) would make
  listing exact rather than sampled.
- A temporal re-check: a pixel that keeps the unanimous value on a moving frame could
  be listed on the next frame with the rotated directions. This narrows the miss rate
  but still does not certify the cut frame.

Until one of these passes S2 with zero changed fringe pixels, `PENUMBRA_LISTING` stays
false. [m7-gaps.md](m7-gaps.md) records the gap.

## 7. Process and evidence

Implementation took about 75 minutes. Verification was dominated by the shared GPU
queue: 14 leases or attempts, several stood down by root-order changes, compiler-quiet
holds and one 2.6 GiB preflight; a 09:10–09:38 API outage paused the unit. Every lease
was ≤15 minutes, read the latest root order, gated on owner release lines, and admitted
at the RAM floor in force. One lease (`final-1b`) ran its S2 and lib steps about five
minutes past a root order. That is recorded in the schedule, and every driver step now
re-reads the order.

Harness (`tools/p-rts/`): `lease.py` (n8 lease, owner-anchored gates, queue following,
RAM-gated admission), `ab.py` (paired ARA1 captures), `final.py` / `identity.py` (lease
plans), `score.py` (byte identity plus ARA1 crop scores via `tools/ara1/images.py`),
`scenes.py` (cut-stream scenes), `build.py` (cargo under a root-order watcher) and
`hold.py`. Evidence lives under `D:/Coding/renderlab-work/p-rts/`: `analysis/*.json`
(scores and timings for every screen), `runs/*/ab/*/engine.json`, and the lease records
in `lease/runs/`.
