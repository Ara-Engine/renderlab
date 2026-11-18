# Live reference: path tracing the engine's own scene (sunrt, 2026-11-17)

The judging references (`main-thread/ref`, `examples/scene_reference.rs` over `src/reference.rs`) are traced
from a scene dump: meshes, standard-program materials and RGBA8 textures written by the live engine, then
traced by a separate GPU path tracer. That tracer shades every material with the standard program's
parameters. It cannot judge most of Outpost:

- **Material programs.** Ara's terrain is its own program (four splat layers from 2x2 atlases), and Ara
  edits the standard surface in nearly every material. The tracer shows the terrain's raw atlas.
  `gicut`'s first rule, which masked out non-standard programs, masked almost everything.
- **See-through surfaces** are left out of the dump (Outpost: 120 instances in every view, the puddles among them).
- **Fog** and aerial perspective are not traced (the live reference traces the fog: see Fog below).
- **Local lights** are traced as points, without their radius.

The dump is also about 1.4 GB a view on Outpost.

## Proposals

1. **Compose the programs into the dump tracer.** Record each material's program source and parameter
   block in the dump. Compose every distinct program's `surface` and `material_out` into the tracer's
   kernel, with the sampling services bound to its own texture arrays. One megakernel holding dozens of
   programs would take minutes to compile; the engine's shading kernels already take 69-96 s each. It also
   duplicates the engine's material inputs (vertex streams, tangents, decals, parameter blocks) in a second
   format that has to track every engine change.
2. **Trace the live engine's scene (chosen).** The engine already evaluates any program at an arbitrary
   hit. The lighting cache's cell stage (`Stage::Cell`, `shaders/gi_shade_entry.wgsl`) rebuilds a hit's
   full `SurfaceInput` from the engine's own buffers and runs the program's `surface`, one small module per
   program. A reference stage composed the same way writes `material_out(surface(input), input)`, the pass
   split's program-agnostic `MaterialRecord`, for a queue of path-tracer hits. Every program runs exactly
   as the engine runs it: textures, parameters, decals, normal maps. There is no second material format
   and no dump.

## Design (proposal 2)

A measurement API on the renderer, never in the frame loop: trace the current scene and camera at N
samples per pixel into a linear image with a split-half noise estimate, like `reference::Image`. It needs
ray queries, and uses the RT path's acceleration structure and draw tables.

Wavefront, per bounce:

1. **Rays.** The camera rays (pixel and shutter jitter as `reference.rs`), or the previous bounce's
   continuations.
2. **Trace.** A ray query against the engine's TLAS writes a hit record per path: placement, triangle,
   barycentrics, as the traced primary hit record, or a miss.
3. **Materials.** For each program present, its reference module runs over the queue and fills the
   `MaterialRecord` of the hits whose draw uses it. The modules are composed by `program::compose` with
   a new `Stage::Reference` and a generated entry modelled on the cell entry.
4. **Shade.** One program-free kernel uses `src/reference.rs`'s estimator on each `MaterialRecord`:
   next-event estimation for the sun, sky and local lights, BSDF sampling of the standard model
   (Lambert plus GGX, height-correlated Smith) with the power heuristic, emission on hit, and
   Russian roulette. It accumulates radiance and writes the continuation rays.

Shadow rays use the same TLAS, with the engine's caster mask.

## Order of work (main thread, 17 Nov)

1. Material programs: the reference stage and the wavefront loop. Re-trace the Outpost views.
2. Local lights with their radius, sampled as spheres.
3. See-through surfaces: the engine's transparent materials (opacity, IOR) as a dielectric
   transmission lobe.
4. Fog and aerial perspective, from the engine's media.

Each step re-traces the affected views at 16384 samples, reruns the judging baseline and updates
`main-thread/ref/README.md`.

## Fog (step 4)

The renderer's fog settings (`Atmosphere::height_fog`, `volumetric_fog`) are traced as one physical
medium on every ray: camera, bounce and shadow rays alike.

- **Medium.**
  - The height fog's extinction, `density * exp(-falloff * (y - base_height))`, everywhere.
  - Plus the volumetric fog's `extra_density` inside a sphere of its range about the eye. The engine's
    froxels hold height fog plus that extra over the first `distance` metres of every camera ray.
  - Extinction is grey. The albedo tints what scatters, and the phase is Henyey-Greenstein with the
    fog's anisotropy.
  - Rays that leave the scene meet fog out to the planet's ground or 100 km, as the engine's backdrop
    fog does. The sky cube holds no fog.
- **Sampling.**
  - Free flights are exact, with no majorant: each part's optical depth is inverted in closed form,
    and the nearer of the two distances is the flight of their sum.
  - A flight shorter than the segment scatters in `rf_trace`: next-event estimation for the sun,
    local lights and sky, then a phase sample, with the power heuristic between them as at surfaces.
  - Shadow rays from surfaces and from the fog take the medium's closed-form transmittance.
- **Where the reference differs from the engine's model.**
  - The engine lights its fog with the unshadowed sun plus the sky's mean radiance (height fog) or
    the shadowed sun (froxels), as single scattering on camera rays only.
  - The reference scatters any number of times, on every ray. Its sunlight and lamp light also
    lose what the fog takes on the way to a surface; the engine's surfaces get the sun undimmed by fog.
  - These are physical effects the engine leaves out, so the judge should see them as engine gaps.
- **Not traced.**
  - Aerial perspective: the air's scattering between surfaces is about 1% blue extinction at 300 m.
    The sky cube already holds the air beyond the scene.
  - The height fog's start distance, recorded in `not_traced` when it is set.

`reference.json` records the traced medium under `fog`.

## What it closes off

Programs with their own `shade` (toon, hand-written looks) are traced through their `MaterialRecord`
with the standard model, as the pass split already assumes. A non-standard look's reference is its
material under physical lighting, not its stylised shading.
