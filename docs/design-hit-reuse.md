# Reflection hits from the previous frame (S4a, design, 2026-11-13)

Status: draft for the root's review. Owner: the Mac GPU thread (hit shading). Switch:
`RENDERLAB_RL_HIT_REUSE=1`, off by default. Off, every module is byte-identical.

## Why
On the M5's Showcase split (port RT path, shading-pass ms, 2026-11-13):
- Glossy reflections cost 5.70 ms.
- Shading their hits costs 3.51 ms of that: lights at hits 1.88 (shadow rays 0.97), material
  and environment about 1.6.
- One shaded ray per glossy pixel still spends 0.86 ms on hit shading.

Most of Showcase's glossy hits land on surfaces the camera already shaded last frame: the
floor's reflections of props and mannequins, and the props' reflections of the floor.
Reading that shaded colour costs one or two fetches, against `surface` + `shade` + lights +
shadow rays. This is Lumen's screen trace applied after our own ray, so the ray decides
visibility and the screen only supplies lighting.

## Where
One function, `rl_hit_reuse(record, origin, d) -> vec4<f32>` (rgb exposed radiance, a = 1
when reused), in a new `shaders/reflection_hit_reuse.wgsl`. Two callers:
- `reflection_hits.wgsl`, the compacted pass, before `rl_hit_input`/`surface`/`shade`.
- The inline hit loop in `reflections.wgsl`, for mirrors and pixels without history. This is
  also the port's path today: the port does not run the compacted pass yet (S1 on the port
  follows), so the M5 rows price reuse there first.
Composed only when the switch is on. A program sees nothing new: its hits are shaded as
before, or not shaded at all because a reused colour stands in.

## What the record gives, and what the module must bind
The compacted pass's H record (t3/rtvis 13bdee0) carries everything per hit:
- placement + 1, primitive, barycentrics: the hit point in object space, then this frame's
  world point (`rl_hits_distance` already rebuilds it);
- the receiver's origin and the ray direction: the incoming direction -d, for the view test;
- pixel, slot, stratum and lobe alpha.

Nothing is missing from the record. The hit module additionally binds the previous frame's
data, which already exists:
- `tr1` (binding 40): the previous unjittered view-projection, the previous eye, the
  exposure carry factor (`jitter.z`) and the restart flag (`flags.x`);
- `tr1_colour_in` (42): the previous frame's resolved, exposed colour;
- `tr1_anchor_in` (43): per pixel, placement + 1, side, and view distance;
- `object_motion.wgsl`: `motion_previous_model` (rigid motion) and
  `motion_previous_vertex` (skinned, SK1).

One addition is asked of TR1's owner: keep MQ2's view hint in the anchor's free bits 24-31
(log2-quantised), instead of dropping it when TR1 overwrites the HDR alpha. Without it, the
view test falls back to the hit material's base factors (below).

## The test, in order (any failure shades the hit as today)
1. **No restart:** `tr1.flags.x == 0`. A cut, the first frame and reset events never reuse;
   that is the hard reset at cuts.
2. **Not in a restart zone:** the hit point lies outside this frame's `Tr1Zones` (P-RELIT's
   changed-light spheres, SK1's swept moving-caster shadows). A lamp that changed, or a
   shadow that moved over the hit, means last frame's colour is stale.
3. **The hit's own previous position:** from the record's object-space point through
   `motion_previous_model` (rigid) or the previous vertices (skinned). This is the reflected
   object's motion, not the receiver's: a reflected Fox that moved is looked up where it was,
   which is the same surface point last frame.
4. **In view last frame:** the previous position projects inside the previous image (with a
   one-pixel margin for the bilinear footprint) and in front of the camera.
5. **Same surface:** the anchor's placement and side at the four bilinear texels equal the
   hit's, and the anchor's view distance matches |previous eye - previous position| within
   1% + 2 cm. Texels that fail drop out of the bilinear weights. Fewer than two valid texels
   means shade. This handles disocclusion, an occluder in front last frame, and silhouettes.
6. **Small view dependence:** the hit is seen along -d now and was shaded along
   (previous eye - point) last frame. Reuse when MQ2's hint at the texel is 1
   (view-independent), or when the angle between the two directions, times (hint - 1), stays
   under half a pixel of the hint's own image. Without the hint (the TR1 addition missing):
   only hits whose material is non-metal with roughness at least 0.5 reuse. Metal hits always
   shade.
7. **Enough history:** the colour's weight (TR1's sample count) is at least 4, so a young,
   still-noisy history does not stand in for an exact shade.

## Units, jitter and atmosphere
- **Exposure:** `tr1_colour_in` holds radiance exposed at the previous frame's exposure.
  Multiplying by `tr1.jitter.z` (TR1's own carry factor) puts it in this frame's exposure,
  which is the unit hits are written in.
- **Jitter:** TR1's history is resolved onto pixel centres with the unjittered
  view-projections, so reprojecting through `tr1.previous` (unjittered) and sampling bilinear
  needs no jitter term. The raw HDR is never read, because it is jittered, aliased and holds
  the view hint in alpha.
- **Atmosphere:** the history includes the camera's aerial perspective and fog toward that
  surface, and a hit needs the bare surface radiance. `atmosphere_surface(c, p)` is affine in
  c, so with S = f(0, p) and T = f(1, p) - S, both evaluated now, c = (C - S) / T. Reuse is
  rejected where any channel of T falls under 0.5. Showcase has none of this; Glint's sky does.
- **Second bounce:** the reused colour carries the hit surface's own reflections. A shaded hit
  uses the environment there (one bounce). This is more correct, but it means pictures change
  on reflective hit surfaces by design. The gate must tell that from lag.

## Lag, and the gate
A reused hit shows the reflected surface's lighting from last frame, through TR1's history,
at the right place (test 3). What can trail:
- Lighting moving over a reflected object: a moving shadow, or a lamp turning on. Zones (2)
  and TR1's own motion clamp cover the known events. The gate looks for the rest.
- A history that lags its own object under motion. TR1 resets history on disocclusion and
  clamps it under motion, and test 7 rejects young histories.

Gate, as strict as S2's:
- The game flight's Fox, Showcase's walking figure, and Glint's ball and sliders, as moving
  reflected objects: crops on the frames after each starts moving, and after a cut.
- No default if any trail shows.

The rows also report the reused share of hits, per scene.

## Cost
- A reused hit pays: rebuilding the hit point (already paid for the distance), one previous
  transform, four anchor loads, four colour loads, and the atmosphere affine pair when it
  is on.
- A rejected hit pays that and then the full shade, so the switch loses where few hits pass.
  The reused share decides it.
- On the port today, rows run the inline path at full strata and at `strata=1`. On wgpu they
  run through the compacted pass once S1 is the default.
