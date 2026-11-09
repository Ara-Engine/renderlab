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

### ARA1's 88 frames are mostly GI settling on the quarry

The traces sit at about frame 1100. ARA1 times frames 1-87 after startup. ARA1's own
sequence was run with 1000 still frames appended at the frame-0 pose (`*-tail` scenes).
In the quarry, the named passes (VB, deferred shading, sun visibility, TR1) stay at about
2.7 ms throughout, while the GPU span (median ms) falls:

| Frames | quarry span | game-base span | ara-pbr span |
|---|---|---|---|
| 1-39 (ARA1 still) | 11.75 | 10.61 | 8.85 |
| 40-71 (ARA1 moving) | 11.69 | 10.92 | 7.06 |
| 73-87 (after ARA1's cut) | 13.44 | 9.22 | 7.23 |
| 88-149 (back at the frame-0 pose) | 4.79 | 8.26 | 6.64 |
| 300-599 | 3.76 | 8.32 | 6.54 |
| 600-1087 | 3.40 | 8.51 | 6.72 |

On the quarry about 9 ms per frame of ARA1's span is outside the named passes. That is the
GI cache's fill and settle while a view is first seen, which decays after the frame-0 view
returns. ARA1's no-GI variant agrees: game-base span is about 2.9 ms lower in the same
frames. Handed to P-GI. CPU times in these runs are not used: P-NRT's own candidate builds
overlapped them.

### The atmosphere variant

ARA1's game-atmosphere costs +3.8 ms on non-RT, and the trace shows where. The
atmosphere passes themselves take 0.06 ms. Deferred shading goes from 3.0 to 6.4 ms
(game-atmosphere still trace, same pose). About half of its samples are the cube-map
environment lookups (`environment_tap`, `environment_remap`, `environment_bilinear`,
`environment_face_uv`, `environment_radiance`): exact two-level, four-tap filtering per
lookup. A physical sky makes every glossy-ray miss, every reflection hit's own `shade()` and
the receiver's sky specular sample it. `exposed_product` is another 11.8%.

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

All pure optimisations: the same values reach every output.

1. **Visibility-buffer resolve: scan only listed candidates instead of the whole mesh**
   (`src/receiver_adjacency.rs` `repair_candidates`, `src/visibility_buffer.rs`
   `REPAIR_POLICY`). Some pixels lie near an edge with no unique adjacency neighbour, on a
   mesh with ambiguous edges. For those, the non-RT repair scanned every triangle of the
   mesh in ascending order and accepted only triangles with exactly two corners at the
   edge's endpoint positions. Mesh upload now lists, per halfedge, exactly those triangles
   by welding key, ascending. The welding key is a superset of the shader's float equality,
   and the shader keeps all its own checks. The resolve walks that list instead, so a true
   boundary scans nothing and an ambiguous edge (a UV-sphere pole, a duplicate face) visits
   only its few sharers, in the same order as the full scan. The per-draw policy buffer holds
   0 (never), u32::MAX (whole mesh: meshes without adjacency, virtual geometry, appended LOD
   triangles) or the offset of the mesh's halfedge count, list offsets and lists. The first
   screen (cand1, a skip-only version) proved boundary skips identical; cand4 replaces it
   with the lists.
2. **`exposed_product` fast path** (`shaders/scene.wgsl`). When `a*b` is a finite normal f32
   and the exposed product is normal, `(a*b)*exposure` rounds exactly as the mantissa/exponent
   form: power-of-two scaling preserves round-to-nearest inside the normal range. Both
   saturate to 65504 past 2^17, and exact zeros stay zero. Anything else takes the original
   `frexp`/`ldexp` path.
3. **R1n traversal reads whole vec4 words** (`shaders/reflections_software.wgsl`,
   `src/reflection_software.rs`, `src/program.rs`). Triangle records grow from 10 to 12
   words (two pad words), so every instance record, node and triangle starts on a 16-byte
   boundary. `rl_geometry` becomes `array<vec4<u32>>`, and its layout entry's minimum
   binding size becomes 16 bytes (the first screen failed pipeline validation at 4). A node
   is 4 loads instead of 14, an instance record 6 instead of 22 and a triangle 3 instead of
   10, with the same words in the same arithmetic.
5. **Environment fast paths** (`shaders/scene.wgsl`). `environment_product` gets the same
   guarded plain-product path as `exposed_product`: every partial product a finite normal,
   with exposure and intensity normal. In `environment_radiance`, a blend weight of exactly 0
   (every mirror lookup: mirror surfaces and mirror-ray sky misses) skips the second level's
   four taps. Its finite texels would contribute exactly zero.
4. **TR1 memo** (`shaders/temporal_resolve.wgsl`). `tr1_moving` (two mat4 loads) is a pure
   function of the placement, and `tr1_near_skin` only ORs. The 3x3 ring and the two
   history slots now skip a placement equal to the last one tested.

## 3. Quality proof

Every change is a pure optimisation, so the bar is byte identity. The final candidate
(**cand4**: all five changes) against the v0.1.0 baseline, each in its own process, from
separate per-tree targets:

| Check | Result |
|---|---|
| ARA1 quality witnesses, non-RT, 960x540 (game-base, ara-pbr, quarry; 19 frames each: still, moving, the cut and its recovery) | 57/57 PNGs byte-identical |
| ARA1 game-atmosphere quality witnesses (physical sky; the environment fast paths) | 19/19 byte-identical |
| ARA1 1080p timing captures (frames 39, 71, 87, every scene and repetition) | 21/21 byte-identical (97/97 frames in total) |
| Q-walk (quarry walk, 93 frames, TR1 on), non-RT and RT | 93/93 and 93/93 frame hashes identical |
| RF1-RF10 after 32 TR1 frames per pose (`gpu_r0a_current`), non-RT and RT | 13/13 and 13/13 poses byte-identical |
| Lib suite, release test binary of cand4 | 198 passed, 0 failed, 163 ignored. Includes `repair_candidates_equal_the_shaders_whole_mesh_scan`, which checks the lists against a brute-force float-equality scan with nonmanifold, duplicate, collapsed-corner and signed-zero cases. |

Screens on the way:
- cand1 (exposure fast path plus a boundary-only scan skip) was 75/75 ARA1 frames identical.
- cand2 (adding R1n vec4 and the TR1 memo) failed non-RT pipeline creation: the binding-36
  layout still declared a 4-byte minimum. Its RT Q-walk and RF were identical, and the
  layout was fixed in cand3.
- cand3 (adding the environment fast paths) was 57/57 identical.

No quality setting changed: ray counts, directions, walk caps, cache sizes and the 16-ray
glossy budget are untouched.

## 4. Timings

ARA1 timing scenes, 1920x1080, synchronous service (`render()` plus GPU drain, ARA1's
definition). Medians pooled over repetitions; the n column is frame samples, base / cand4.
The first cut is ARA1's frame 72 (one sample per run). GPU passes are ARA1's timestamps:
VB = `primary_ms`, shading = `shading_ms`, TR1 = `reconstruction_ms`.

| Scene | Phase | Base ms | cand4 ms | Change | GPU span | VB | Shading | TR1 | n |
|---|---|---|---|---|---|---|---|---|---|
| game-base | still | 11.30 | 9.79 | -13% | 10.33 → 8.78 | 1.19 → 0.40 | 3.36 → 2.93 | 0.51 → 0.44 | 117 / 78 |
| game-base | moving | 11.68 | 10.10 | -14% | 10.59 → 9.04 | 1.56 → 0.40 | 2.89 → 2.46 | 0.81 → 0.72 | 96 / 64 |
| game-base | cut | 13.14 | 11.07 | -16% | 11.51 → 9.84 | 1.56 → 0.38 | 3.04 → 2.68 | 0.43 → 0.36 | 3 / 2 |
| game-base | recovered | 9.58 | 7.85 | -18% | 8.50 → 6.84 | 1.56 → 0.41 | 2.99 → 2.55 | 0.51 → 0.44 | 45 / 30 |
| ara-pbr-oriented | still | 9.43 | 7.79 | -17% | 8.31 → 6.82 | 1.47 → 0.30 | 2.83 → 2.32 | 0.39 → 0.35 | 117 / 78 |
| ara-pbr-oriented | moving | 7.82 | 6.15 | -21% | 6.83 → 5.16 | 1.47 → 0.31 | 2.75 → 2.26 | 0.54 → 0.50 | 96 / 64 |
| ara-pbr-oriented | cut | 8.72 | 7.08 | -19% | 7.10 → 5.33 | 1.48 → 0.30 | 2.84 → 2.33 | 0.33 → 0.29 | 3 / 2 |
| ara-pbr-oriented | recovered | 7.74 | 6.08 | -21% | 6.73 → 5.14 | 1.48 → 0.31 | 2.80 → 2.34 | 0.39 → 0.35 | 45 / 30 |
| quarry-daylight | still | 12.80 | 12.49 | -2% | 11.44 → 11.33 | 0.71 → 0.71 | 0.70 → 0.67 | 0.51 → 0.51 | 117 / 78 |
| quarry-daylight | moving | 12.62 | 12.47 | -1% | 11.31 → 11.27 | 0.71 → 0.73 | 0.66 → 0.63 | 0.50 → 0.49 | 96 / 64 |
| quarry-daylight | cut | 18.62 | 19.03 | +2% | 16.87 → 17.05 | 0.74 → 0.77 | 0.78 → 0.75 | 0.49 → 0.49 | 3 / 2 |
| quarry-daylight | recovered | 13.93 | 13.92 | -0% | 12.73 → 12.75 | 0.74 → 0.78 | 0.79 → 0.75 | 0.54 → 0.54 | 45 / 30 |
| game-atmosphere | still | 15.25 | 13.16 | -14% | 14.09 → 12.10 | 1.19 → 0.40 | 6.76 → 5.87 | 0.55 → 0.48 | 39 / 39 |
| game-atmosphere | moving | 16.01 | 13.99 | -13% | 14.95 → 12.77 | 1.56 → 0.41 | 6.28 → 5.23 | 0.85 → 0.77 | 32 / 32 |
| game-atmosphere | cut | 17.44 | 15.17 | -13% | 16.10 → 13.62 | 1.53 → 0.38 | 6.89 → 5.67 | 0.47 → 0.40 | 1 / 1 |
| game-atmosphere | recovered | 15.51 | 13.73 | -11% | 14.47 → 12.15 | 1.56 → 0.41 | 6.76 → 5.64 | 0.55 → 0.48 | 15 / 15 |

- **Hardware:** RTX 4070 SUPER (12,282 MiB), NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11
  10.0.26300.
- **Build:** release profile, rustc 1.96.0, `examples/ara1_capture.rs` with the CPU-parts
  rows.
  - Base: v0.1.0 (3a71de0) in `tree-base`/`target-base`, `bin/base.exe` sha256 2e66d4f8...
  - cand4: `tree-cand4` in `D:/Coding/pnrt-target-cand`, `bin/cand4v.exe` bf55da0d...
  - Lib-test binaries: base-lib 7762be11..., cand4-lib 9340d310...
- **Contention:** each repetition ran inside a P-NRT GPU lease, the only GPU job. Leases
  lease-4b (base repetition 0) and lease-6 (base repetitions 1 and 2, cand4 repetitions 1
  and 2) ran at 08:43 and 10:07 UTC, interleaved per case. No P-NRT build overlapped a
  timing run. Other units' CPU-only builds and ordinary desktop activity (the wallpaper GPU
  client) may have.
- **Quarry:** within noise. Its ARA1 frames are dominated by GI settling (§1), which P-GI
  owns. ARA1's single first-cut sample swings by several ms.

**Against Ara raster** (ARA1: game 1.86/1.96/1.84, PBR 1.70/1.71/1.71, quarry 5.60/8.71/8.84 ms):
non-RT remains 5.3x (game still), 4.6x (PBR still; 3.6x moving) and 2.2x (quarry still) slower after these changes.
The target is not met; §5 ranks what remains.

## 5. Gaps

Recorded in docs/m7-gaps.md, "Non-RT frame cost". In order of measured size:
1. **GI settling** in ARA1's early frames (P-GI): about 9 ms per frame on the quarry, up to
   2.9 ms on game-base.
2. **Surface-cache sun visibility** (P-NSQ): 1.2-3.4 ms. P-NSQ's penumbra fix may add cost.
3. **CPU per synchronous frame:** `render()` costs 0.7-1.3 ms, encode dominating, plus about
   0.6 ms of completion and submit latency. Next step: a CPU sampling profile of
   `submit_frame`; no profiler was available.
4. **Deferred shading:** 2.3-2.9 ms on the small scenes (5.2-5.9 ms with a physical sky).
   - R1n traversal is still the largest part. Next: a short register stack, and a two-level
     instance tree with an index-ordered tie rule (identical closest hits).
   - Environment lookups dominate under a physical sky.
   - P-GI found the shading pass runs 16-pixel workgroups (half a warp). Retiling the
     shading dispatch is the next screen; its workgroup-shared state must stay
     byte-identical.
5. **TR1:** 0.3-0.8 ms. The memo saved about 0.07 ms; the history fetch's local arrays are
   next.
