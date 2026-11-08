# M7-LC2: soft local shadows and every light in bounce GI

Implementation follows LC1, starting from main `fa3b74b` (2026-11-08). Branch `t3/m7-lc2`.

## Implementation

`Light::Point` and `Light::Spot` add `source_radius: f32`, a finite nonnegative emitter radius in metres. Zero retains LC1's hard visibility, including the map path's 3x3 antialiasing PCF. Positive radii enable soft shadows automatically for selected local lights; LC1's default four shadowed lights, ranking, range, candela and cone falloff stay the same. The engine facade re-exports the same light type and preserves the radius. Tool JSON accepts an optional `source_radius`, default zero. Existing Rust light literals need the new field.

- **RT heads:** four uniform samples of a disk facing the receiver (the apparent-area approximation of a spherical source). TR1 rotates the sample sequence across frames. Without TR1 the sequence is deterministic per pixel. Radius zero traces exactly one centre ray. This changes visibility only: light intensity and incidence use the centre, preserving the existing programmable light API.
- **Map heads:** PCSS-style twelve-tap blocker search, averaging linear light-view blocker distance, followed by twenty-four bilinear PCF taps (96 depth loads). Filter size scales with emitter radius and receiver/blocker separation. Search and filter support are capped at 96 texels per axis. Receiver-plane depth comparison and slope bias remain; cube taps crossing a face reproject into the adjacent face and compare on the same world receiver plane. Maps stay cached when only radius changes.
- **Every light in GI:** replace the single per-cell visibility word with a dynamically sized bitset. `gi_light` tests the full vector, rejects zero-energy lights and those outside the query point's range, cone or receiving hemisphere, and traces the remaining lights. Cell shading reads every visibility word and applies the program's existing `scatter` for every visible light. Moved-caster invalidation also considers all lights. No camera cluster is used for off-camera cache points, and no first-N lights are dropped.

GI-side edits are marked LC2 and limited to light-list layout, allocation, invalidation's light enumeration and the cell shader's light enumeration. Scheduling, lookup, hit/material shading and gather policy remain owned by GC1/G2c.

The GPU light record grows from 64 to 80 bytes; its old shadow-index offsets remain intact, with source geometry appended. All duplicated GPU declarations, the tool boundary and light-buffer capacity/accounting follow that ABI. This introduces no new bindings or shading-model restriction.

## Cost bounds

A GI cell holds `round_up_to_4(31 + max(1, ceil(light_count/32)))` cold words, plus its existing 24 hot words. At 261 lights this is 256 bytes per slot, versus 224 before LC2: **16 MiB extra** for the default 524,288 slots. Allocation grows when needed and retires old buffers safely. Growth exceeding the device buffer/storage-binding limit returns an explicit error; it never silently drops lights.

Light-list work is bounded by **queued shade cells x light count** cheap tests and at most that many visibility rays, usually far fewer after spatial rejection. Existing GI gather/claim and event scheduling budgets are unchanged. There is no fixed per-cell ray cap: dense overlapping lights remain a scaling gap. Local direct shadows cost at most four RT rays per selected light, or 108 map depth loads per selected light (12 search + 96 filter), with an early return when no blocker is found. The 96-texel map support cap is a disclosed quality limit, not a light-count limit.

## Verification

Pending: lib tests inside the lease, point/spot fixture captures and LC1's night quarry, sun day/dusk on raster, traced RT and visibility-buffer non-RT, plus one diagnostic timing run. No new Unreal/reference campaign or superiority claim.

## Gaps

Pending image inspection. Known approximation limits: disk visibility with centre-incidence BRDF; four-ray noise when TR1 is disabled; sparse blocker-search misses; map support capped at 96 texels; lights beyond LC1's shadowed pool remain unshadowed in direct lighting; GI local visibility still uses centre rays (hard bounce shadows); GI light-list cost grows with dense overlap. No new ReSTIR, variance clipping or denoiser is introduced; TR1's existing policy is unchanged.
