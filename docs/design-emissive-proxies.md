# Emissive proxies: small bright emitters as lights (GI night blotches, option c)

U-RTVIS design note, 2026-11-13. Status: **design only**, approved direction (root/coordinator,
2026-11-13). No code yet. It is queued after S1, the RT speck lease, the inline stub and U-FIXED's
`Engine::set_casts_shadows`; the `glow=gi` reproduction comes first. The Mac GPU thread needs the
same proxy path on ara-gpu: the CPU side (decision, fit, records) is meant to be one shared planner
(memory: shared planners for ara-gpu), the reduction one WGSL kernel.

## The defect and its cause

Atelier's night scene (Ara #142 at 42104e12, `levels/atelier_night.scene`; stills
`coord-batch/atelier/night.png`, `nightdolly_a/b/c.png`) shows bright mottled blotches tens of
pixels across on the courtyard floor and walls. With every lamp blocked by its own fitting, the
glowing lantern glass is the only light, and it reaches the floor only through the GI cache. The
blotches are world-fixed (dolly frames correlate 0.980 at the floor's own shift), do not flicker
(mean change 1.5/255) and do not settle (one frame against 60 held frames: 0.02/255).

From the code (`shaders/gi_cache.wgsl`, `shaders/gi_shade_entry.wgsl`, `src/gi_cache.rs`):

1. **A cell's estimate is deterministic.** `gi_gather` casts one ray along each of 64 fixed
   directions (`GI_DIRECTIONS`, stride 1). `gi_direction` is a golden-angle spiral rotated only
   by the cell's key (`key.y>>8`): there is no frame or gather term. Each event gathers a cell
   `gathers` = 3 times (W2), and each gather re-traces the same rays and replaces the value (W1).
   Nothing accumulates, so nothing converges, and idling (16 identical frames; the idle key holds
   no radiance) only freezes values that would not change anyway.
2. **A small emitter is hit by 0 or 1-2 of a cell's 64 rays**, each weighted 2π/64 ≈ 0.1 sr
   (`gi_gather`, the `6.283185307179586*f32(stride)/f32(GI_DIRECTIONS)` line). Neighbouring
   cells differ by that whole term, permanently.
3. **The hit reads a cell, not the emitter.** `gs_shade` returns `rl_emission(s,input)+...` at
   the hit cell's single query point, so a cell whose query point lies on the glass radiates the
   glass's full radiance over the cell, and far hits read coarser cells (`gi_hit_level`).

The pixels' interpolated lookup spreads each cell's value over the pixels that read it: blotches.
Small emitters never get `gi_near_gather`'s aimed rays, which exist only for light-list entries.

## The method (c): proxies in the light list

A placement whose material's program declares `emission`, and whose measured emission passes the
bound below, gets a **proxy light**: an ordinary `Light::Point` with `source_radius` in the light
list. Its emission then leaves the gather; pixels, cells (`gi_light`, `gi_near_gather`) and
reflection hits light it like any local light, with its shadows, at pixel resolution on pixels.
The camera and reflections still see the glass through its program as today.

### Power and colour come from the program (programmability)

The proxy's power, colour and fit come from evaluating the program's own `emission` hook on the
GPU, never from a CPU copy of a material constant:

- **The reduction.** One compute dispatch per changed candidate group (an object and one of its
  emissive materials, below). Each triangle is sampled at stratified barycentric points, as many
  as its texel footprint needs: its UV area times the bound emissive texture's size, between 4
  and 64 points, and 4 when the material has no texture. Each point runs the program's `surface`
  and `emission` with the interpolated vertex inputs (UVs, normal, tangent) and the material's
  parameters and textures, as cell shading composes them (`Stage::Cell`). The dispatch writes
  each triangle's reduced emission (mean radiance times area, rgb) to a small buffer.
- **The fit is weighted by that per-triangle emission**, not by the mesh. Triangles are sorted by
  emitted power, and the fit keeps the smallest set holding 95% of the group's power, dropping
  any triangle under 2% of the brightest triangle's radiance. That way a third-party lamp whose
  whole material emits but whose texture glows only in the bulb is fitted to the bulb's
  triangles. From the kept set, in object space: emitted power Φ = Σ L·A·π (Lambertian), the
  power-weighted centroid and mean colour, the radius around the centroid that holds every kept
  triangle, the power-weighted mean normal and its spread, and the principal-axis ratio. A group
  with no triangle above the threshold has no emitter and stays as it is.
- **When it runs:** on the first frame a candidate is placed, and again whenever the material's
  parameters, a bound texture's revision, the program's source or the mesh change. Programs get
  no time input (`program_api.wgsl`: a program may declare `struct Parameters`, read as
  `params`, and declares no bindings), so time-varying emission is always a material-parameter
  revision through `Engine::set_material_parameters`, which the CPU sees. The re-reduction keys
  on those revisions; there is no periodic path. **Constraint:** animated glow is CPU-driven
  through parameters today; if programs ever gain a time input, this design needs a periodic
  reduction.
- **An emissive program with no texture** runs the same reduction; its result is uniform over
  the mesh, so the dispatch is tiny. No special case.
- **The first reduction blocks:** at scene load and at material creation it runs and is read
  back before the first frame that draws the placement, so there is never a blotchy gather frame
  followed by a pop. Its load cost (ms per emitter: the dispatch plus one readback wait, batched
  over all of a load's emitters into one submit and one wait) is measured with the gate.
- **Later changes are asynchronous:** a revision queues a reduction, its record is read back
  double-buffered, and the light follows the glass **2 frames** after the change (encode, then
  map). Until then the light holds its last power and colour. The cost per changed material is
  one dispatch over its mesh's samples, measured with the gate.
- **Bounded per frame:** at most 4 reductions a frame, oldest pending revision first; a newer
  revision of a placement already queued replaces its entry. A lamp flickered through
  `set_material_parameters` every frame therefore costs at most its share of the 4 per frame and
  follows at the reductions' pace, holding its last power in between; it never re-reduces
  without bound.

### Grouping: one emitter per object, not per connected piece

Kits are often unwelded boxes: Glint's route chevrons are 532 pieces, the hangar's festoon bulbs
39, Outpost's campfire coals 34, each hoop band 48 (the Content thread's lamp audit,
`coord-batch/lamp-audit/glow_*.txt`). A proxy per piece would mean hundreds of lights. So:

- **The unit is the object**: one renderer (placement), and within it one emissive material's
  triangles, reduced and fitted together, whatever their connectivity.
- **One proxy per object when it fits**: the kept triangles pass the shape tests (compact, below)
  and the bound. Outpost's coals (34 pieces in about a 0.5 m pile) are one sphere.
- **Otherwise up to 4 proxies per object**: power-weighted k-means on the kept triangles'
  centroids, k = 2, 3, 4 in turn, until every cluster passes the shape tests and the bound.
  **Cap: 4 lights from one object.** Past the cap, the whole object stays with the gather, so
  an object is never split between the two. The festoon string (39 bulbs along a line), Glint's
  chevrons along the course and its hoop bands (rings) stay with the gather; that is recorded
  as their limit, for (a) and (d).

### Shape and placement

- **Sphere** (`Light::Point`, `source_radius` = the fitted radius, at the fitted centre) for
  closed or curved emitters: lantern glass, bulbs, globes. Transformed by the placement each
  frame, so moving emitters move their proxy with no new reduction (skinned emitters re-reduce on
  their skin revision).
- **Disc** (a one-sided cosine lobe) for flat, one-sided panels: the light model has no
  cosine-lobe light today. Until one exists, flat one-sided emitters stay with the gather rather
  than radiating backwards as a sphere. That is not a minor limit: see "The hangar" below, where
  it decides whether the disc light is a requirement.
- **Long thin emitters** (principal-axis ratio above 4:1: strip lights, neon) fit neither and
  stay with the gather. They are a known limit for option (a) (sampling emissive triangles), and
  the proxy is not bent to fit them.

### The hangar: flat panels are its main light

Outpost's hangar is lit mainly by twelve ceiling light panels (0.99 m, flat, one-sided
`diffuser`) and a bench fluorescent tube (0.88 m, flat and long). Under the shape limits above,
all of them stay with the gather. What the hangar then shows:

- **No direct shadows from its main light.** Gather light is cell-resolution indirect light, so
  nothing under the panels casts a contact shadow; objects sit in an even wash.
- **The same deterministic per-cell sampling as the night blotches.** From the floor about 5 m
  below, a 0.99 m panel subtends about 0.12 sr, near one gather direction. Each cell's 64 fixed
  rays hit each panel 0 to 2 times, so cells differ by whole panels' worth of light. Twelve
  overlapping panels average some of that out, but a mottled floor at cell scale is the expected
  result.

**Ranking:** the hangar goes into the `glow=gi` reproduction's lease as a still (both paths, by
PC integration's capture if the wgpu harness cannot load it). If it blotches or shows no
shadows under the panels, **the disc light becomes a requirement of (c), built before the gate**,
not a later item:

- a one-sided cosine-lobe light (centre, normal, radius);
- RT samples points on the disc itself;
- non-RT maps it with the five cube faces of its hemisphere;
- the cluster bounds it by its hemisphere;
- its cost per receiver is about a spot light with a source radius.

The bench tube (long) waits for (a) either way.

### 1. The bound, and the hand-over to the gather

The decision is per emitter (placement) and binary, so a surface is never counted twice:

- **Influence range** R: the distance at which the proxy's illuminance falls to the clustered
  lights' cutoff (the same rule that sets an authored light's range). Below a minimum R (the
  emitter is too dim to matter at any receiver) it stays with the gather.
- **Upper end: the gather resolves it.** A receiver at distance d sees the emitter under about
  π r²/d². The gather resolves it when that is at least one direction, 2π/64 sr, i.e. for
  d ≤ √32·r ≈ 5.7 r. The receivers that matter lie within R. If the gather resolves the emitter
  for at least the inner half of them (5.7 r ≥ R/2, i.e. **r ≥ R/11.3**), it stays with the
  gather; otherwise it becomes a proxy. A 20% hysteresis band on r/R stops flipping when the
  emission is animated.
- **No surface counts twice.** The decision writes one bit per placement, "proxied", read by
  both sides: a proxied placement's surfaces return no `rl_emission` in cell shading (the
  gather sees them dark), and only proxied placements have a light record. Not proxied: no
  record, cell shading unchanged.

### 2. An authored light already inside the glowing mesh

Atelier's lanterns have an authored point light inside the glowing glass. Under the rule above
each would become two shadowed lights (25 to 50 in the night scene). The bound plus the emission
value are not enough to say "the glass is only seen": a small, bright glass passes the bound
whatever its brightness, and dimming the emission to fail it dims the glass the camera sees.

So, without a flag: **an authored local light inside the fixture's own bounds claims it.** The
fixture is the object after grouping: the claim test is the object's world box (all its
emissive groups, expanded by 5 cm), or its fitted sphere, whichever holds the light. It is not
the distance to one piece's centre: Outpost's floodlight spot sits 0.41 m from each of its four
glass pieces (radius 0.25 m, so outside 1.5 r) but inside the head's box, so it claims the
glass and the bulb together.

Only an object that would otherwise become a proxy can be claimed. An object that stays with
the gather (too large for the bound, long, flat, or past the 4-light cap) keeps its glow
bouncing whatever light is inside its bounds. Glint's goal, 364 pieces up to 10.8 m with its
goal light inside the box, is one. A claimed object gets no proxy and leaves the gather (its
surfaces are dark to cell shading); the authored light is its light. An author then:

- keeps the authored light, for "the glass is only seen, the lamp inside is the light"; its
  intensity is the light's, the glass's emission only what the camera sees;
- removes the authored light (or sets its intensity to 0), for "the glass is the light": the
  proxy's power then comes from the glass's emission.

What the author sees when a claim happens: the glass's own emission leaves the GI gather, so its
glow no longer bounces; the claiming light provides that light instead. That is intended, and it
matters to whoever balances the two intensities (the Content thread for Atelier).

**Claims change today's pictures** wherever a glowing mesh sits near an authored light: Atelier
day and night, Glint's lamps and ring lights, Showcase's lantern. Their glass's bounce disappears.
So:

- the gate's crop pairs include one claimed emitter per scene, before and after, besides the
  proxied repro;
- the reproduction measures how much light the claimed glass was adding (the bounce it gave
  its surroundings, with the authored light off and on), and the note records the figure;
- a light with zero intensity, or one disabled, does not claim. A light switched off at runtime
  releases its claim: the glass becomes a proxy again (after its reduction, at the same 2-frame
  latency as any change), and the reverse when it comes back on;
- the Content thread's audit (`coord-batch/lamp-audit/glow_*.txt`) lists every glowing mesh and
  its nearest authored light. Under the bounds rule its compact fixtures with a light inside
  are claimed:
  - Outpost's floodlights (glass and bulb), security light and mast beacon;
  - the hangar's hanging lamps.

  Showcase's emissive ball and water bottle have no light within 3 m, so they are proxied
  wherever they pass the bound. Outpost's coals are proxied too, unless the fire light lies
  inside the pile's bounds.

No program and no per-material flag. Atelier stays at 25 shadowed lights either way; which one
it should be, and the energy balance, is the Content thread's call. The fitting blocking the
authored light is `set_casts_shadows`' fix, not this note's.

### Shadow rays never start inside their own glass

The proxy sphere is fitted around the emitting triangles, so the glass lies inside it. On both
paths, **geometry inside a proxy's sphere does not shadow that proxy**:

- **RT:** LC2's four receiver-facing disk samples, with each ray's extent ending at the sphere's
  surface (tmax = distance to the sample minus 1.01 r). The rays never reach the glass.
- **Non-RT:** the proxy's cube faces in the local atlas use a near plane of 1.01 r, so the glass
  is clipped out of its own map.
- Over the atlas limit (below), the software visibility rays use the same tmax rule.

This needs no casts-shadows bit and no offset tuning. It is independent of
`Engine::set_casts_shadows`: that per-object choice still decides whether the fitting shadows the
authored lights (and every other light); the sphere rule only keeps a proxy's own glass out of
the proxy's shadow. Its cost: an occluder that is inside the
sphere and outside the glass (a fitting's arm) does not shadow the proxy. That is recorded,
small, and the same at every distance.

### Budget and switches: like any local light

A proxy is an ordinary local light from the light list's point of view: it is clustered, ranked
for the atlas with LC3's 25% hysteresis and 1/8 fades, counted against the shadow budget, and
obeys the per-light `cast_shadows` (a proxy's comes from its placement's casts-shadows choice)
and the scene's local-shadow switches. It reaches GI through the per-cell light bitset like any
light (LC2), and reflection hits light it through their own cluster walk.

### 3. Non-RT

- **Method:** the local atlas, exactly as authored point lights: one cube (six faces) in the
  4096² x 6 atlas, PCSS filtering since `source_radius` > 0. The cache's cells light it with one
  software visibility ray per cell per proxy in range (`gi_light`'s existing path).
- **Cost per proxy:**
  - the map: drawn once while its casters and the emitter are still (LC3's per-slot cache), then
    no per-frame depth cost;
  - each receiver pixel in range: the PCSS blocker search and filter (up to 96 depth loads);
  - each cache cell in range: one software visibility ray when it is shaded.
  Measured numbers go with the gate.
- **At the atlas limit (128 maps):** today an authored light that ranks out of the atlas
  "returns lit" (m7-gaps, Lights: finite map coverage). For proxies that would be silent light
  without shadow. Instead, **a proxy that ranks out of the atlas is shadowed with software
  visibility rays** (the same software BVH walk non-RT reflections and the cache already use),
  with the same tmax rule. It costs more per receiver than a map but is exact. Every such frame
  counts the proxies on rays in the renderer's stats, so it is never silent.
- **Atelier:** 25 lanterns under the claim rule stay 25 shadowed lights; without it, 50. Either
  fits the 128 maps.
- **RT** has no map budget: every contributing clustered light, proxies included, gets
  visibility rays (LC3).

### 4. Zero cost with no qualifying emitters

- The candidate set is built on the CPU from what the frame already knows: placements whose
  program declares `emission` (the composition's token scan). An empty set encodes no reduction
  pass and adds nothing to the light list.
- A candidate is reduced only when it is placed or changes (a material-parameter, texture,
  program or mesh revision),
  never per frame. A candidate that fails the bound adds no light record.
- The cell-shading exclusion is composed into the cell module only while at least one proxy or
  claim is active. Without one, every composed module is byte-identical to today's.
- Tests: a scene with no `emission` program records no reduction pass in the pass log, keeps
  its light count, and composes byte-identical modules; a below-bound emitter adds no record
  after its one reduction.

## The gate

Before any default: a crop pair for Islam of the `glow=gi` repro (both paths) and Atelier's night
still, before and after, plus three consecutive moving frames; the hangar under its panels, before
and after (with the disc light if the ranking above made it a requirement); one claimed emitter per scene
(Atelier, Glint, Showcase), before and after; with the measured cost per proxy (RT rays, non-RT
map draw and filter, cache rays), the first reduction's load cost per emitter, the reduction's
cost per changed material, and the claimed glass's former bounce alongside.

## Related, not here

- **(d) rotate the gather directions per gather, with a running mean** that resets on events
  (DDGI and Lumen-style probe rotation with hysteresis). It removes the fixed per-cell bias for
  every indirect light, but it changes every GI picture and the idle rule (which would need a
  converged count). It gets its own design note, crop pairs and gate after (c).
- **(a) sampling emissive triangles in the gather**, later, for large emitters and strip lights.
- **(b) a per-update clamp** is dropped: with replace-not-accumulate values there is no history
  to return clamped energy to.
