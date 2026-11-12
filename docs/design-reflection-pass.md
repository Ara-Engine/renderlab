# Reflection pass: trace, hit shading and resolve (design, 2026-11-12)

Status: agreed (the Mac GPU thread's review, 2026-11-12, with the changes under "Review
outcome"); both sides build against it. Ruling (root, 2026-11-12):
- U-RTVIS owns strata selection, the trace, the lobe side output, and the reduced-rate pass's
  accumulator and resolve.
- The Mac thread owns hit shading as its own compacted pass on both backends
  (shaders/reflection_hits.wgsl, src/reflection_hits.rs), including lights at hits and
  GI-cache lighting for rough hits.

## Why
On Showcase, reflections are 17-19 ms of the RTX frame (shading 24.4 ms RT, 6.3 with
reflections off) and about 14 ms of the M5's 31 ms of shading. The cost has two sources:
- **Hit count:** about 2 strata per lit pixel on average, 16 at p90, so roughly 4M hit shades
  a frame at 1080p.
- **Divergence:** inline hit shading runs inside the camera shading groups, so each SIMD
  group pays for its worst pixel.

This design cuts glossy hits to at most one per 2x2 quad a frame and shades them in a
compacted list. An accumulator averages them over frames and space.

## Which pixels go where
The traced shading entry decides per pixel, after `surface` and `reflection_lobe`:

| pixel | `specular_incident` this frame | rays this frame |
|---|---|---|
| no traced lobe (rough above the cutoff, weight 0) | untraced, as today | none |
| mirror (roughness < RL_MIRROR, 0.05) | inline: one ray, hit shaded inline, exact | inline only |
| glossy with valid history | the accumulator, reprojected (see Resolve) | its lobe goes to the pass |
| glossy without history: disocclusion, cut, `rl_restarts`, first frame | inline: today's full strata (`rl_reflection_prepare`) | inline, and its result seeds the accumulator |

- **Mirrors stay inline and exact**, so the mirror sphere does not change.
- **Lag:** glossy pixels with history see the previous frame's resolved value, reprojected by
  the reflected point's motion. Root accepted this one-frame lag on 2026-11-12. If it shows
  anywhere in motion (a reflection trailing its surface), this falls back to a lobe pre-pass.

## Frame order
All passes are compute, in one encoder, both backends. A(N) is the accumulator written in
frame N, and passes 4-6 are new:

1. **Visibility and camera trace** (unchanged).
2. **Traced shading entry** (U-RTVIS seam in the entry; the program is unchanged):
   - reads A(N-1) for glossy pixels with history;
   - writes the lobe side output L(N);
   - traces inline for mirrors and pixels without history, and writes those pixels' seed
     S(N).
3. **TR1 and the rest of the frame** (unchanged), in parallel order.
4. **`rl_trace`** (U-RTVIS):
   - reads L(N) at slot rate;
   - traces one ray per active slot, on hardware ray queries or the software BVH;
   - writes each miss's sky radiance straight into the sample image R(N);
   - appends one hit record per hit to the list H;
   - writes every ray's hit distance into D(N).
5. **`rl_hit_args`** (U-RTVIS): one invocation turns H's count into indirect dispatch
   arguments.
6. **`reflection_hits`** (Mac): an indirect dispatch over H that shades each record and writes
   its radiance into R(N) at the record's slot.
7. **`rl_resolve`** (U-RTVIS): combines R(N), D(N), S(N) and A(N-1), reprojected, into A(N),
   with variance clamping and a spatial filter.

Passes 4-7 can run after TR1 resolve of frame N, since nothing in frame N reads A(N). The
camera image of frame N+1 shows A(N), which carries frame N's rays.

## Slots and rate
A slot is one 2x2 quad of the image: slot (x, y) covers pixels (2x..2x+1, 2y..2y+1). It
traces at most one ray a frame, from one glossy pixel of its quad.
- **Which pixel:** frame N tries quad position (N + hash(slot)) mod 4 first, then the next
  positions in order, until it finds a glossy pixel with history. A slot with none traces
  nothing.
- **Which direction:** that pixel's VNDF stratum. Stratum k = (N / 4 + hash(pixel)) mod
  count, of today's fixed per-pixel set: count = clamp(ceil(48 roughness), 4, 16) and
  `rl_vndf` with today's u. So over 4 x count frames a pixel's set is covered at main's
  directions and cone widths.
- **Rate switch:** `ReflectionSettings::pass` is Off, Quad or Pixel, with an env switch
  `RENDERLAB_RL_PASS=0|quad|pixel`. Pixel rate is one ray per glossy pixel a frame, for
  measurement and quality checks. Off is today's inline path.
- **Sizes** (quad rate, 1080p): 960 x 540 = 518,400 slots. Pixel rate: 2,073,600.

## Data layouts
All new resources are owned by src/reflections.rs (U-RTVIS) and sized from the view. Flagged
for the Ara port: new textures and buffers, new bindings in the traced shading entry, and a
new pass order.

### L(N): lobe side output (shading entry writes, `rl_trace` and `rl_resolve` read)
One `rgba32uint` at full resolution, 16 B a pixel:
- **x:** octahedral lobe normal, snorm16 x2.
- **y:** `pack2x16unorm(alpha, traced fraction)`. alpha 0 means no ray from this pixel this
  frame (mirror, untraced or inline).
- **z:** `pack2x16float(cone width at the receiver in metres, cone spread per metre)`, as the
  inline loop computes them, before the stratum's sub-cone.
- **w:** bit 0 glossy-with-history (eligible for the pass), bit 1 inline seed written,
  bits 8-31 the stratum count.

The ray origin is rebuilt from the primary targets the shading entry reads: world (binding 16),
geometric normal (17) and epsilon (23). It uses the same `fma(side, epsilon, receiver.world)`
as the inline loop, so the origin is the same as inline.

### H: hit records (`rl_trace` appends, `reflection_hits` reads)
One storage buffer:
- **Header, 16 B:** `count` (atomic), then the dispatch arguments x, y, z.
- **Records, 48 B each (three `vec4<u32>`):**

| word | field |
|---|---|
| 0 | pixel: x in the low 16 bits, y in the high 16 |
| 1 | slot index (y x slot width + x) |
| 2 | `stratum` (low 16 bits), `strata` count (high 16): the share for `rl_hit_samples` / `rl_hit_offset` |
| 3 | hit placement + 1 (as `instance_custom_data + 1`, the primary record's x) |
| 4 | hit primitive index |
| 5, 6 | hit barycentrics, f32 bits (the inline `bitcast<vec2<u32>>(hit.barycentrics)`) |
| 7, 8, 9 | ray origin, f32 bits (world) |
| 10 | ray direction, octahedral snorm16 x2: `rl_oct_encode` / `rl_oct_decode` in reflections.wgsl, the one pair both sides use (decode: unpack2x16snorm, fold z < 0 as (1 - abs(v.yx)) x sign, normalize) |
| 11 | `pack2x16float(cone width at the origin, total spread per metre)`: `rl_hit_input`'s `width` and `spread`, the sub-cone included |

- **Hit point and distance:** `reflection_hits` rebuilds the hit input exactly as the inline
  path does. `rl_hit_input(record, origin, d, width, spread, pixel)` stays in reflections.wgsl
  as the shared helper. Path regularization is `rl_hit_variance = 8 spread^2`, as inline. The
  hit distance is in D(N), so the record does not carry it.
- **Capacity:** one record per slot: 518,400 x 48 B = 24.9 MB at quad rate, 99.5 MB at pixel
  rate. Pixel rate is under the 128 MiB binding limit at 1080p. Above 1080p it falls back to
  quad rate.
- **Order:** `rl_trace` aggregates its appends per workgroup (one atomic per 64 rays), so the
  list order follows trace tiles. Re-binning by batch or program for coherence is the Mac
  pass's call, with its own prefix pass if it wants one. The list is valid only for frame N.

### R(N): samples (`rl_trace` writes misses, `reflection_hits` writes hits)
One `rgba16float` at slot resolution, 4.1 MB at quad rate:
- **rgb:** the exposed radiance arriving along the ray: `rl_finite(shade(...))` for a hit,
  `rl_sky(d, cone alpha)` for a miss.
- **a:** 1 written this frame, 0 no ray.

`rl_trace` clears every active slot's a to 0 first. A hit's record writes rgb and a = 1;
`reflection_hits` writes only at its record's slot.

### D(N): ray distance (`rl_trace` writes)
`r32float` at slot resolution: the hit distance, or -1 for a miss (the sky is at infinity).

### S(N): inline seed (the shading entry writes)
`rgba16float` at full resolution: an inline glossy pixel's mean radiance (rgb) and mean hit
distance (a). Only pixels without history write it, and L.w bit 1 marks them. Full resolution,
so the resolve can take a pixel's exact first value.

### A: accumulator (`rl_resolve` writes A(N), the shading entry and `rl_resolve` read A(N-1))
Two ping-pong pairs at slot resolution:
- `rgba16float`: radiance (rgb) and the reflected-point distance (a);
- `rgba32uint`:
  - **x:** octahedral lobe normal, snorm16 x2;
  - **y:** view depth, f32 bits;
  - **z:** history length (low 8 bits) and roughness (unorm8);
  - **w:** luminance first and second moments, f16 x2.

4.1 + 8.3 MB each, 25 MB both pairs.

Total new memory at quad rate, 1080p: about 33 (L) + 25 (H) + 4 (R) + 2 (D) + 17 (S) + 25 (A)
= 106 MB. S can drop to half precision rgb plus distance packed into L's spare bits if memory
matters (Ara budget: compress before merge if over).

## Reading the accumulator in the shading entry (`rl_history`, reflections.wgsl)
For a glossy pixel at frame N:
1. Take the accumulator's reflected distance from the slot that held this surface last frame,
   found by surface motion from the TR1 frame's previous projection.
2. Form the virtual point: receiver + reflect(-v, lobe normal) x distance.
3. Project the virtual point with the previous view-projection to get its previous slot
   position.
4. Read A(N-1) bilinearly over the 2x2 slots there. A slot counts when its normal is within
   about 25 degrees, its depth is within the receiver plane's tolerance and its roughness is
   within 0.1.
5. If no slot counts, the pixel has no history and traces inline.

For misses (the sky), the distance is infinite and the virtual point is a direction, so
reprojection uses the rotation only.

The read upsamples slot resolution to the pixel, weighted by normal, depth and roughness:
- no blur across edges;
- under a pixel of blur at roughness 0.1;
- detail at roughness 0.05-0.1 is the risk. A narrow lobe may need pixel rate (the `pixel`
  switch), or a full-resolution accumulator for narrow lobes only. The pictures decide.

## Resolve (`rl_resolve`)
Per slot:
- **New sample:** R(N) when a = 1. Seeded slots average their quad's S(N).
- **History:** A(N-1), reprojected by the reflected point as in `rl_history`. Rejected on
  normal, depth or roughness, or on a cut (the TR1 frame's restart flag), which restarts it
  from the new sample.
- **Variance clamp:** the history is clamped to the new sample's spatial neighbourhood (mean
  plus or minus k sigma over the 3x3 slots with matching normal, depth and roughness). k
  narrows as roughness falls. A disoccluded or moved reflected object then leaves no trail.
- **Bright samples:** a sample more than 8 times the neighbourhood mean luminance, standing
  alone, is limited to that. This replaces the inline lone-stratum bound, which needs several
  strata in one frame.
- **Blend:** weight 1/(history length + 1), length capped by roughness (about 8 frames at
  0.1, 32 at 0.35). Motion shortens it in proportion to the virtual point's screen motion,
  above about 1 slot a frame.
- **Spatial filter:** after the blend, a 5x5 a-trous pass over slots. Radius = lobe width x
  distance / slot footprint, clamped to 1-3 slots, with weights by normal, depth, roughness
  and luminance distance. It runs once; history is stored before the filter, so blur does not
  accumulate.
- **Moments:** stored for the next frame's clamp.

The patent policy is suspended (AGENTS.md, 2026-11-10), so variance clamping is allowed.
Patent numbers for flagged methods go in this doc when known.

## Hit shading interface (`reflection_hits`, the Mac's)
`reflection_hits` binds:
- H (read);
- R(N) (write, `rgba16float` storage);
- the frame, lighting, GI, materials and geometry groups, as the traced shading entry binds
  them.

For each record it:
1. rebuilds the hit with `rl_hit_input`;
2. calls `rl_load_parameters(batch)`, then `surface` and `shade` under `rl_reflection_hit =
   true`, with:
   - `rl_hit_samples = 1` and `rl_hit_offset` = (frame + hash(pixel, stratum)) mod the
     light's sample count (local shadows in `rl_local_shadow`);
   - `rl_hit_variance` from the record's spread;
   - `rl_hit_footprint` = width + spread x distance;
3. writes `vec4(radiance, 1)` at the record's slot.

How it orders, groups or sorts the records, and how it lights rough hits (GI cache), is the
Mac thread's design. Hits read the open environment state, as inline hits do.

Inline hits (mirrors and pixels without history) keep running inside the shading entry with
today's code. They read the same `rl_hit_samples` and `rl_hit_offset`, set to today's share
across the pixel's strata. The Mac's shared shadow decision then reads only those values,
and the `RL_SPLIT` text seams in src/program.rs go away.

## Costs to expect (RTX, Showcase)
Today's ~4M hit shades become:
- **Glossy:** at most one per glossy quad, about 1/8 to 1/16 of today's glossy hits.
- **Mirrors and inline pixels:** unchanged in count, but only in a few groups.

New passes:
- `rl_trace`: about 0.3-0.6 ms RT, more on non-RT software rays;
- `rl_hit_args`: under 0.01 ms;
- `rl_resolve`: about 0.3-0.5 ms.

Target: reflections at about 3 ms in all.

## Judging
- **Scenes:**
  - reflection_matched with its lamp: the gold sphere (0.25), the spinning crate (0.35), the
    copper ball (0.3), the 0.42 ball, the turntable (0.35), the hero walking through the arch
    and the mirror sphere. Path-traced references at frames 16/32/47 and the held pose.
  - game flight and night lanterns;
  - Showcase on the integration thread.
- **States:** still, moving, and the four frames after a cut, on both paths.
- **Failures to look for:**
  - a reflection trailing its surface (lag);
  - swimming of reflected detail under camera motion;
  - ghosts of moving reflected objects;
  - grain after a cut;
  - dulling against the path tracer.

  Any of these blocks a default change.
- **Timing:** same binary, interleaved twice, with the reference run.

## Review outcome (the Mac GPU thread, 2026-11-12)
1. **Records stay 48 B.** `reflection_hits` rebuilds each hit with `rl_hit_input` from words
   3-11, and takes the hit distance from the rebuilt point, so it does not read D(N). D(N)
   stays for the resolve.
2. **`rl_trace` clears R(N).** `reflection_hits` writes rgb and a = 1 only at its records'
   slots, so record order never changes a result.
3. **No records for misses.**
4. **No per-batch header from `rl_trace`.** `reflection_hits` bins by program inside its own
   pass: placement, then batch, then program slot through MP1's slot table, then prefix,
   scatter, and one indirect dispatch per program. Per-batch counts from `rl_trace` would only
   be a nice-to-have, never a dependency.

Notes:
- **Mixed-program frames:** list frames now shade inline hits with the camera pixel's list
  program (`LIST_HIT_PARAMETERS`). Binning by program shades each hit with its own program.
  That is correct, but it changes the picture on mixed frames only, and the Mac flags it with
  crops. Single-program frames (Showcase on wgpu) are unchanged.
- **One hit function:** inline hits and `reflection_hits` run the same code. `rl_hit_input`
  and the `rl_hit_*` privates stay in reflections.wgsl with this doc's signatures, and the Mac
  composes its pass from them.
- **`rl_hit_offset`:** `reflection_hits` computes it from record word 2 and the frame; inline
  hits get it from the U-RTVIS seam. The Mac's shared shadow decision then replaces the
  `RL_SPLIT` seams.
- **Ara port:** `reflection_hits` binds H (read) and R(N) as `rgba16float` storage (write),
  plus the shading entry's groups.
- **Parity first:** the Mac builds `reflection_hits` to match inline hits first, with a
  byte-matching harness against them at pixel rate.

## Bindings and hit state (agreed with the Mac GPU thread, 2026-11-12)
- **Group 0, U-RTVIS's shading-entry resources:** 54-63. Taken so far: 54 the reflection history
  texture and 55 its frame record (shaders/reflections_history.wgsl); L, S and A take the next
  ones.
- **Group 0, the Mac's `reflection_hits`:** 64 H (storage, read), 65 R (`rgba16float` storage,
  write), 66 its bin uniform, 67 the binned order.
- **Hit state:** `reflection_hits` sets `rl_reflection_hit`, sets `rl_hit_stratum` and
  `rl_hit_strata` from word 2, and sets `rl_hit_variance` = 8 spread^2 from word 11's spread.
  So `rl_trace` keeps word 2 as (stratum, strata) and word 11 as (cone width at the origin,
  total spread per metre), exactly as inline. The `RL_SPLIT` seams keep working until both
  sides move to `rl_hit_samples` / `rl_hit_offset`.
- **Parity:** a hit from H matches the inline hit to within the direction's snorm16 error,
  which the Mac measures at pixel rate. It is acceptable unless a crop shows more than one
  output code.
