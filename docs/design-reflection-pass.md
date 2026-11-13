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
| 2 | `stratum` (bits 0-7), `strata` count (bits 8-15; at most RL_GLOSSY_MAX, 16), the lobe's GGX alpha as f16 (bits 16-31: `pack2x16float(vec2(alpha, 0)) << 16`; the Mac's cache-terminated hits gate on its roughness). Changed 2026-11-12 from stratum and strata in 16 bits each |
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
  `rl_hit_strata` from word 2 (bits 0-7 and 8-15; bits 16-31 carry the lobe's alpha as f16),
  and sets `rl_hit_variance` = 8 spread^2 from word 11's spread. Word 11 is (cone width at the
  origin, total spread per metre), exactly as inline. The `RL_SPLIT` seams keep working until both
  sides move to `rl_hit_samples` / `rl_hit_offset`.
- **Parity:** a hit from H matches the inline hit to within the direction's snorm16 error,
  which the Mac measures at pixel rate. It is acceptable unless a crop shows more than one
  output code.

## First stage on t3/rtvis (2026-11-12, `RENDERLAB_RL_PASS=1`, off by default)
`rl_trace` runs and writes H, R and D, but the picture still comes from the inline trace: the
accumulator and resolve come later. Code: src/reflection_pass.rs, shaders/reflections_lobe.wgsl,
shaders/reflections_trace.wgsl, and `rl_cast` in reflections.wgsl.
- **Lobe output L, written by primary shading for glossy receivers (mirrors stay inline):**
  - binding 56, `rgba32float`: ray origin, and `pack2x16float`(cone width at the origin, spread
    per metre before the stratum's sub-cone) as f32 bits;
  - binding 57, `rgba32uint`: octahedral lobe normal, view direction and geometric side, then
    `pack2x16float`(GGX alpha, frame tag);
  - binding 58: the pass uniform (frame counter, slot width, on, 0). The tag is the frame counter
    mod 2048; `rl_trace` ignores texels with another tag (stale or not glossy).
  - This replaces the doc's earlier single-texture L: the origin is stored, not rebuilt.
- **`rl_trace`** (8x8 slots a group) picks a glossy pixel of the quad: frame + hash(slot) mod 4
  first, then the next positions. It takes stratum (frame + hash(pixel)) mod count of today's
  set, with today's VNDF direction, radial and rotation. It casts with `rl_cast` (RT ray query;
  the software BVH on non-RT), and its `tmin` is 1e-4 x the origin's distance from the world
  origin, not the receiver's own epsilon.
  - It clears R and D at every slot, writes a miss's sky into R (a = 1), and writes the hit or
    miss distance into D (binding 59; -1 a miss, -2 no ray).
  - It appends a hit's 48-byte record to H (binding 64) with one atomic per workgroup.
- **`rl_hit_args`** (one invocation, after `rl_trace`) writes H's words 1-3: `reflection_hits`'
  dispatch arguments for 64-record groups, rows of at most 65535.
- **Order:** the frame's shading, then `rl_trace`, then `rl_hit_args`. The pass log stage is
  `reflection.trace`. `reflection_hits` and `rl_resolve` go after it.

## Next: a deferred reflection composite (plan, 2026-11-13)
The resolve's pre-pass runs `surface` a second time for every pixel, and that is most of what is
left of its cost: Showcase-like RT reflections take 3.2 ms against today's 3.8. The known
structure shades once.
- **The shading pass**, run once, writes its colour as today: `shade` sees the untraced estimate
  (`specular_incident` with w = 0). It also writes two side outputs per glossy pixel:
  - the untraced specular it used (rgb);
  - the lobe's reflectance: rgb, or the luminance `weight` the Mac is making the standard
    program declare.
  It also writes L (the lobe) as rl_trace already reads it.
- **After shading:** rl_trace (or the inline one-stratum sample), the Mac's `reflection_hits`, and
  rl_resolve with its a-trous passes.
- **A composite pass** adds w x (reflectance x resolved - untraced specular) per pixel, w being
  today's traced fraction (1 inside the traced band, fading to 0 at max_roughness).
- **Programmability:** the program still decides the reflectance it reports and the untraced
  estimate. The composite assumes `shade` is linear in `specular_incident` about that estimate,
  which the standard program is (`mix(specular, traced x reflectance, w)`). A program that wants
  something else keeps the inline path.
- **Interface to settle with the Mac:** the reflectance output's form (rgb or luminance weight)
  and its binding, in U-RTVIS's 54-63 range.

## S2: one ray per glossy pixel, resolved (design, 2026-11-13)
Target: Showcase glossy reflections at no more than 1.5 ms on both paths, with no visible loss
still or moving. The Mac's split (M5 port, 720p, 99f5d395): one shaded stratum a glossy pixel
costs 1.43 ms over glossy-off (0.57 trace and fixed, 0.86 hit shading), against 5.70 for
today's 16. So the resolve has to stay lean, about 0.1-0.3 ms.

**Where it runs.** In the compacted deferred path only (RL_PASS=1, RL_DEFERRED=1), and only
for programs that report their reflectance (`RL_DEFERRED_PROGRAM`). Every other program, and
mirrors (roughness under RL_MIRROR), keep their inline path. The shading kernel stays as it is:
the resolve is its own pass, which is why it can't bloat shading the way the history did on
Glint.

**The sample.** Each glossy pixel traces one VNDF stratum a frame. The stratum index is
`(frame + hash(pixel)) mod count`, where count is today's faded and thinned strata count, so
`count` frames visit every stratum once. The trace writes one sample a pixel, screen-sized:
- **incident radiance, rgb:** demodulated by construction, because the composite multiplies by
  w x reflectance (binding 62);
- **hit distance** t (a miss marks itself);
- **direction**, as an octahedral word (the VNDF pdf is recomputed from it and the pixel's lobe);
- **the hit's motion:** the hit's previous world position from its H record (placement,
  primitive, barycentrics). It is rebuilt with object_motion.wgsl's `motion_previous_world` and,
  for skinned meshes, `motion_previous_vertex`: the same helpers TR1 uses, so the Fox counts.

The resolve never asks how the hit got its colour. That keeps it independent of
`reflection_hits`, of the Mac's S4a (last frame's lit HDR at on-screen hits), and of any later
hit cache.

**Temporal (anti-ghosting first).**
1. **Reproject by the reflected virtual point.** The point is the receiver plus the reflected
   direction times t; its previous position uses the hit's previous world position, not just
   the receiver's. The receiver must also pass TR1's surface tests (same placement, depth and
   normal).
2. **Reject on the hit's own motion.** If the hit moved more than its footprint (the cone width
   at t, about pixel width plus 2 alpha t), the pixel restarts. So a moving reflected object
   never drags last frame's reflection along the floor. A receiver that moved over a pixel
   restarts too.
3. **Hard reset** on the planner's cut and reset (temporal_reconstruction `cut`/`reset`), at
   respawn, and in `rl_restarts` zones. Today's 2x restart strata become a reset.
4. **Variance clamp.** Before blending, history is clamped to the current frame's neighbourhood
   mean plus or minus k sigma (YCoCg), with the neighbourhood sized to the resolve's footprint.
   This replaces the lone-stratum bound: a lone bright stratum is clamped against its
   neighbours instead of within its own set.
5. **History length:** capped by roughness. Near-mirrors (0.05-0.1) keep a short history (their
   trails show most); rough lobes keep up to about 16 frames. Any rejection resets the length to
   1.

**Spatial (one pass).** About 8 rotated taps inside the lobe's screen footprint. The footprint
is 2 alpha t over the pixel's world width, clamped to a maximum radius.
- **Each neighbour's sample is reweighted to this pixel's lobe:** this pixel's GGX value at the
  neighbour's direction over the neighbour's VNDF pdf. It is rejected on placement, depth,
  normal, hit-distance ratio, or lobes that differ too much.
- **This is not resampled reuse** (no reservoirs), just a ratio estimator over neighbours.
- **One pass, not three a-trous passes.** The a-trous passes only return if the gate shows
  grain.

**Composite.** As today: w x reflectance x resolved, added before TR1. TR1 then sees a stable
reflection, so its motion box stays narrow. That was RQ1's failure mode with four strata.

**Bytes at 1080p.**
- **Current sample:** 16 B a pixel (radiance, t, direction, motion).
- **History:** 12 B a pixel (radiance, length, the previous virtual depth).
- **Lobe:** 32 B a pixel today; storing it per glossy pixel is the next cut.
- **Gone:** H shrinks to one record a glossy pixel, and R's room goes away.
- **Total:** about 60 B a pixel, roughly 125 MB at 1080p, against today's 235 MB. Compacting
  the lobe per glossy pixel takes it lower.

**Gate (root, 2026-11-13).**
- **Scenes:** a moving camera, a moving reflected object (Glint's sliders and ball, Showcase's
  figure, the game flight's Fox in a glossy floor via `--diag floor=0.2`), and a cut.
- **Crops:** along the moving object's reflected edge over the frames after it moves, against
  today's inline result.
- **Rule:** no default if any trail is visible.

**Ray budget inside S2 (approved 2026-11-13).** After shading, the trace kernel reads the
pixel's own shaded radiance (the HDR target before the composite) and its reflectance (binding
62). It skips the pixel's ray when even a bound on what the reflection could add moves the
displayed value by less than half a code at this frame's exposure. The bound is the larger of two
terms:
- the prefiltered environment in the reflected direction, at the lobe's roughness;
- a per-frame scalar: the brightest emitter radiance, the sun's radiance reflected off the
  brightest sunlit surface, and the previous frame's on-screen maximum.

The test uses the current frame's exposure, so a cut into a bright scene can't skip a highlight
on its first frame. A skipped pixel resets its history. It lands as a would-skip counter first,
then a switch gated on crops of reflected highlights, still and moving.
