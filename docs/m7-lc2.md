# M7-LC2: soft local shadows and every light in bounce GI

Implementation follows LC1, starting from main `fa3b74b` (2026-11-08). Branch `t3/m7-lc2`. Main was merged through `e5c4623` (local merge `4fd55fd`), preserving the concurrent G2c shading, PP1 post, transparency and fog work.

## Implementation

`Light::Point` and `Light::Spot` add `source_radius: f32`, a finite nonnegative emitter radius in metres. Zero retains LC1's hard visibility, including the map path's 3x3 antialiasing PCF. Positive radii enable soft shadows automatically for selected local lights; LC1's default four shadowed lights, ranking, range, candela and cone falloff stay the same. The engine facade re-exports the same light type and preserves the radius. Tool JSON accepts an optional `source_radius`, default zero. Existing Rust light literals need the new field.

- **RT heads:** four uniform samples of a disk facing the receiver (the apparent-area approximation of a spherical source). TR1 rotates the sample sequence across frames. Without TR1 the sequence is deterministic per pixel. Radius zero traces exactly one centre ray. This changes visibility only: light intensity and incidence use the centre, preserving the existing programmable light API.
- **Map heads:** PCSS-style twelve-tap blocker search, averaging linear light-view blocker distance, followed by twenty-four bilinear PCF taps (96 depth loads). Filter size scales with emitter radius and receiver/blocker separation. Search and filter support are capped at 96 texels per axis. Receiver-plane depth comparison and slope bias remain; cube taps crossing a face reproject into the adjacent face and compare on the same world receiver plane. Maps stay cached when only radius changes.
- **Every light in GI:** replace the single per-cell visibility word with a dynamically sized bitset. `gi_light` tests the full vector, rejects zero-energy lights and those outside the query point's range, cone or receiving hemisphere, and traces the remaining lights. Cell shading reads every visibility word, sums incident irradiance and preserves G2c's effective hemispherical albedo, energy clamp and emission calculation. Moved-caster invalidation also considers all lights. No camera cluster is used for off-camera cache points, and no first-N lights are dropped.

GI-side edits are marked LC2 and limited to light-list layout, allocation, invalidation's light enumeration and the cell shader's light enumeration. Scheduling, lookup, hit/material shading and gather policy remain owned by GC1/G2c.

The GPU light record grows from 64 to 80 bytes; its old shadow-index offsets remain intact, with source geometry appended. All duplicated GPU declarations, the tool boundary and light-buffer capacity/accounting follow that ABI. This introduces no new bindings or shading-model restriction.

## Cost bounds

A GI cell holds `round_up_to_4(31 + max(1, ceil(light_count/32)))` cold words, plus its existing 24 hot words. At 261 lights this is 256 bytes per slot, versus 224 before LC2: **16 MiB extra** for the default 524,288 slots. Allocation grows when needed and retires old buffers safely. Growth exceeding the device buffer/storage-binding limit returns an explicit error; it never silently drops lights.

Light-list work is bounded by **queued shade cells x light count** cheap tests and at most that many visibility rays, usually far fewer after spatial rejection. Existing GI gather/claim and event scheduling budgets are unchanged. There is no fixed per-cell ray cap: dense overlapping lights remain a scaling gap. Local direct shadows cost at most four RT rays per selected light, or 108 map depth loads per selected light (12 search + 96 filter), with an early return when no blocker is found. The 96-texel map support cap is a disclosed quality limit, not a light-count limit.

## Verification

Retained evidence is under `D:/Coding/renderlab-work/m7-lc2/`. All renderlab test binaries ran inside `scripts/n8-run.py` leases, with QUIET/game-hold checks and no viewer window. Bulk output and the two-job Cargo target stayed on D:. No new Unreal/reference campaign or superiority claim.

- `runs/lc2-w4`: **181 lib tests passed, zero failed**; point/spot hard/soft fixtures passed on traced RT, non-RT VB and forward raster; traced quarry passed. Binary SHA-256 `5f0fad3299a5a51f188c42032cf5479191eb6080fa3d93a1269b9b77cfe7ffd2`.
- `runs/lc2-x1`: non-RT VB and forward raster quarry passed. Binary SHA-256 `03524b3a323e1eb4527c564366a75342043203c3b3dc5ea93f357f4896d90756`. The only change from w4 was test setup registering the existing cooked software near field on non-RT. LC1's low-level night harness had omitted that registration, so its nominal non-RT GI-on case used fallback lighting. The new per-light GI diagnostic caught this; the retained x1 capture has real cache lighting.
- Each quarry path loads the asset once and checks 256 coloured lights plus four bright locals and the moon (261 total), radius 0 versus 0.08 m, no local shadows, cluster bypass, moved lights and sun day/dusk. CPU/GPU cluster lists match with zero overflow; longest list 99. With GI and TR1 disabled, cluster bypass changes **zero image channels** on all three paths.
- At settled cache sampling, 195 lights reach at least one visible cell on each deferred path, including **171 after the first 32**. The last four light counts are RT `[19263,15059,6709,13254]` and non-RT `[18085,14248,6633,12842]`. The table is 134,217,984 bytes. The explicit forward-raster control has no GI-cache pass; that existing pipeline limitation is not presented as all-light GI coverage.
- The point/spot fixture uses 0 versus 0.25 m, 24 frames per configuration. It changes 10,610/10,588 pixels on RT and 9,909/11,363 on non-RT (absolute linear HDR difference above 0.03). Inspected complete fixtures show a clear wider penumbra while the foreground blocker silhouette stays sharp. Maps retain a slightly boxy/banded filter boundary; RT retains fine grain.

Inspected final night, hard control and sun day/dusk images for all three paths, plus the point and spot fixtures. The quarry's silhouettes, coloured light pools and shadowed cavities remain coherent; positive radius softens the local shadows without a new broad image regression in these captures. The post-processed images have broad stepped bloom halos against the black background in both hard and soft controls; this is a retained post/output quantization gap. The quarry is opaque, so this unit does not certify transparency or animated material behavior. Kept PNG contact sheets: `quarry-inspected-sheet.png` and `runs/lc2-w4/fixture-inspected-sheet.png`; individual captures remain beside their JSON. Raw HDR dumps over 5 MB and this unit's Cargo target are removed after summarizing.

### Diagnostic timing

RTX 4070 SUPER, driver 617.14, Vulkan, release profile, 1920x1080, LC1 quarry camera, default TR1/GI, four selected local shadows, 1024-square pooled maps on map paths. Eight drained submission-to-idle frames per configuration after four warmups. These include CPU submission and GPU work, are not isolated GPU-pass times, and were **not a QUIET measurement**; other CPU builds and desktop GPU activity were not excluded. Exact records and ranges are in each `lc2-*.json` and `summary.json`.

| Path | Radius 0 median (min-max) ms | Radius 0.08 m median (min-max) ms | No local shadows median ms | Moved lights median ms |
|---|---:|---:|---:|---:|
| Traced RT | 11.93 (10.73-14.73) | 12.27 (10.90-14.48) | 6.97 | 16.18 |
| Non-RT VB | 17.55 (14.59-18.44) | 18.08 (15.61-19.78) | 17.22 | 25.71 |
| Forward raster control | 43.94 (42.82-46.42) | 67.45 (54.56-67.85) | 41.61 | 72.98 |

The moved configuration moves the last four **authored** lights; only two of those are selected by LC1's ranking (selection `[257,258,196,187]`, all spot lights). It does not measure four moving selected lights. The point fixture separately covers cube maps. The forward raster soft-shadow cost is a material miss; deferred primaries remain the default RT/non-RT paths. No 2x performance or Unreal fidelity gate is claimed.

### Development and verification time

Implementation/integration was about 45 minutes of active work. Verification, warm compilation and GPU-queue coordination took roughly another 90 minutes of wall time, partly overlapping documentation. Final retained GPU batches used about 141 seconds combined, including the failed non-RT setup diagnostic. Start to final captures was about 2 h 15 min: **the two-hour time box was exceeded**, chiefly while waiting for shared GPU slots and correcting harness/integration failures. No broader reference or repeat campaign was started. Earlier rejects found a reserved WGSL identifier, CRLF-sensitive shader composition, an eight-byte frame seed copied into a four-byte slot, and a single-sided fixture material; all were corrected before the retained passing evidence. The build helper now rejects CRLF shader includes. Merging current main also required PP1's primary-frame binding to use the current 128-byte frame size.

## Gaps

Known limits: disk visibility with centre-incidence BRDF; four-ray noise when TR1 is disabled; sparse blocker-search misses; map support capped at 96 texels; lights beyond LC1's shadowed pool remain unshadowed in direct lighting; GI local visibility still uses centre rays (hard bounce shadows); GI light-list cost grows with dense overlap. A fully queued default table with 261 overlapping lights has a worst-case upper bound of 136,839,168 visibility queries, with no separate fixed ray budget; range/cone/hemisphere rejection and existing cell scheduling reduce actual work. New bitset allocation is bounded by the device buffer limit, not an arbitrary light-count truncation. No new ReSTIR, variance clipping or denoiser is introduced; TR1's existing policy is unchanged. No local-light temporal/reference campaign was run, so moving-shadow quality is inspected only through the existing TR1 policy and static captures, not certified against a reference. Radius zero retains the legacy cube-edge-clamped hard PCF; positive-radius PCSS reprojects across cube faces. Explicit user-owned shadow maps retain their existing hard-shadow contract; LC2 softness applies to LC1's selected automatic local-shadow pool.
