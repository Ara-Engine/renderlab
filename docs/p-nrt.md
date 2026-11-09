# P-NRT: non-RT frame cost (beat Ara raster, zero quality loss)

Unit P-NRT, 2026-11-09, branch `t3/p-nrt` from v0.1.0 (`3a71de0`). Islam: "let's start on
optimizing and beating ara in performance without compromising even a bit in quality".
Scope: the non-RT frame except GI cache internals (P-GI) and local lights (P-LTS). That covers
the visibility-buffer primary, the Virtual/surface-cache sun shadows, fog/atmosphere, TR1,
post, R1n reflections, and frame scheduling.

**Quality bar:** a pure optimisation must be byte-identical; anything else must be equal or
better on every crop and population. Nothing here trades quality.

## 1. Attributed profile (baseline v0.1.0)

Nsight Graphics 2026.4.0 GPU Trace (Top-Level Triage plus the real-time shader profiler) of
ARA1's frozen `rl-capture.exe`, whose renderer source equals v0.1.0. Each trace covers four
consecutive frames at about frame 1100 of a long single-phase scene (`tools/p-nrt/scenes.py`):
ARA1's 1080p camera, still / moving (ARA1's pan, wrapped) / every frame a cut. Every wgpu pass
is a debug marker, so `tools/p-nrt/attribute.py` assigns each microsecond of a frame to a pass,
an in-frame gap or the CPU gap before the next frame. Medians, ms. RTX 4070 SUPER, NVIDIA
617.14, Ryzen 7 7800X3D; GPU shared with other units' leases, never concurrently.

| Scene / phase | GPU frame | in-frame gaps | CPU gap | visibility buffer | sun shadow pages | sun shadow visibility | GI cache | deferred shading | TR1 | post |
|---|---|---|---|---|---|---|---|---|---|---|
| game-base still | 7.59 | 0.19 | 1.55 | 1.21 | 0.19 | 2.53 | 0.26 | 3.00 | 0.51 | 0.03 |
| game-base move | 11.53 | 0.19 | 2.13 | 1.55 | 0.20 | 3.20 | 0.26 | 4.63 | 0.80 | 0.03 |
| game-base cut | 9.32 | 0.19 | 2.66 | 1.11 | 0.19 | 3.37 | 0.26 | 4.35 | 0.59 | 0.03 |
| ara-pbr still | 6.35 | 0.19 | 1.68 | 1.45 | 0.13 | 1.18 | 0.17 | 2.80 | 0.39 | 0.03 |
| ara-pbr move | 8.68 | 0.20 | 1.97 | 1.46 | 0.11 | 1.28 | 0.17 | 3.40 | 0.75 | 0.03 |
| ara-pbr cut | 8.06 | 0.29 | 2.19 | 1.60 | 0.11 | 1.33 | 0.18 | 3.60 | 0.33 | 0.03 |
| quarry still | 3.68 | 0.18 | 1.59 | 0.68 | 0.11 | 0.57 | 0.89 | 0.69 | 0.51 | 0.03 |
| quarry move | 3.63 | 0.18 | 1.74 | 0.66 | 0.11 | 0.60 | 0.76 | 0.65 | 0.48 | 0.03 |
| quarry cut | 4.00 | 0.19 | 2.32 | 0.75 | 0.12 | 0.70 | 0.51 | 0.70 | 0.45 | 0.03 |

Light clusters, transmission and wgpu's indirect validation passes are each under 0.02 ms.
Atmosphere and fog are inactive in these matched scenes (uniform sky, no medium).

ARA1's per-pass timestamps missed most of the span. Its GPU span starts at the
visibility-buffer cull and ends at present, but only four passes carried timestamps. The
untimed ones were the sun-shadow page work, GI's 14 passes, transparency, and (on the
quarry) most of the frame. The CPU gap is the synchronous harness's time between one
frame's GPU end and the next frame's GPU start: driver completion, `poll`, `render()`'s CPU
(about 0.8-1.0 ms) and submit latency. Ara raster's whole 1.7-1.9 ms service fits inside our
CPU gap alone.

### Where each large pass spends its time (shader profiler)

- **Deferred shading** (game-base still, 3.0 ms): R1n software reflection traversal
  (`rs_trace`, `rs_f`, `rs_box`) about 54% of samples, `exposed_product` 14%, GI cache
  lookups 14% (P-GI's code), `unit_normal` 7%. The traversal is latency-bound on
  word-by-word geometry loads (long-scoreboard stalls) and a local-memory stack.
- **Sun shadow visibility** (N8n surface cache, 2.2-3.4 ms on game-base): `vsc_missed`
  (exact walks of queued pixels) 43% and `vsc_claim` (3x3 consensus plus nine fills'
  table probes) 42%; probe and resolve about 7% each. All global-memory latency.
- **Visibility buffer** (0.4-1.6 ms on game-base, frame-to-frame, still camera): the exact-hit
  resolve. Its cost swings with the TR1 jitter. A pixel near an edge without an adjacency
  neighbour, on a mesh with ambiguous edges, scans every triangle of the mesh for shared
  endpoints. About 35% of resolve samples are that scan.
- **TR1** (0.33-0.80 ms): history fetch and validation (`tr1_history`, `tr1_tap` about 52%),
  `motion_rigid` 15% (two mat4 loads per call, 11 calls per pixel), `tr1_sample_id` 9%.

### Sun-shadow passes, for P-NSQ

Root moved the non-RT sun-shadow files (shadows*, vsm*, virtual_shadow*, surface cache) to
P-NSQ on 2026-11-09. P-NRT changed none of them. Baseline numbers from the traces above
(medians of four frames, ms):

| Scene / phase | Pages (requests, cull, page rasters, max-mips) | Visibility (N8n surface cache) |
|---|---|---|
| game-base still / move / cut | 0.19 / 0.20 / 0.19 | 2.53 / 3.20 / 3.37 (range 2.2-3.8) |
| ara-pbr still / move / cut | 0.13 / 0.11 / 0.11 | 1.18 / 1.28 / 1.33 |
| quarry still / move / cut | 0.11 / 0.11 / 0.12 | 0.57 / 0.60 / 0.70 |

Shader-profiler sample share inside the visibility pass:

| Trace | vsc_missed (exact walks) | vsc_claim | vsc_probe | vsc_resolve | vsc_sort |
|---|---|---|---|---|---|
| game-base still | 43.1% | 41.6% | 7.7% | 7.3% | 0.3% |
| game-base move | 38.8% | 44.7% | 7.3% | 7.7% | 1.4% |
| ara-pbr still | 41.3% | 42.7% | 7.6% | 6.9% | 1.5% |
| quarry still | 74.0% | 11.8% | 12.6% | 1.1% | 0.6% |

- Every hot line is global-memory latency (long-scoreboard stalls of 90-99%).
- `vsc_claim`'s top lines are `vsc_target`'s slot fingerprint compare (9.6% of the pass), the
  epoch check in `vsc_current` (8.5%), `vsc_side` (4.2%), the fills' 3x3 loop (3.9%) and
  `vsc_verdict`'s neighbourhood distance and counts loads (3-5%).
- `vsc_missed`'s top lines are the walk-cap test (3.9%), the max-mip reads, the ring loop
  and the triangle fetches.
- On game and pbr, still frames re-walk as much as moving ones. TR1 jitters every frame, so
  receiver points move every frame and the cache's still-frame shortcut does not apply to
  the ARA1 cameras.

Two items for P-NSQ's files, not applied by P-NRT:
- A CPU find. `prepare_virtual` (src/virtual_shadow_gpu.rs, the moved-casters loop) rebuilds
  `draws.iter().flat_map(repeat_n(skin.is_some(), instances.len())).nth(p)` for every
  placement p. That is O(placements²) per frame. Collecting the flags once before the loop
  gives identical values (P-GI fixed the same pattern in `prepare_gi`).
- The frame-16 penumbra grain (docs/m7-gaps.md, "Non-RT sun shadows"). N8e's 8 directions
  per receiver per frame (the setting's maximum) quantize per-frame visibility to eighths.
  The 16-triangle walk cap counts undecided directions as blocked, giving narrower, darker
  penumbrae.

Traces (per-frame `attribution.json`, hotspot JSON):
`D:/Coding/renderlab-work/p-nrt/traces/base-{game-base,ara-pbr-oriented,quarry-daylight}-{still,move,cut}/`.
Readers: `py -3 -I tools/p-nrt/attribute.py DIR` and `py -3 -I tools/p-nrt/hotspots.py DIR "N8n surface"`.

## 2. Changes

(in progress)

## 3. Quality proof

(in progress)

## 4. Timings

(in progress)

## 5. Gaps

(in progress)
