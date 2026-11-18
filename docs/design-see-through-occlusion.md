# See-through surfaces in every occlusion query (gicut, 2026-11-18; in progress)

## The rule

Every consumer of occlusion sees the same casters with the same transmission. That covers the
camera pixel's sun shadow, a cache cell's light visibility, cache and probe rays, and the
reference. A surface that blocks light for one of them blocks or attenuates it for all of them.
A see-through surface passes `1 - coverage`. Glass (`ior > 1`) also passes its Fresnel
transmission, tinted by its base colour.

## Why

Today see-through draws leave every opaque path (`transparency_gpu::split`):

- The pixels' sun shadow takes TP2's sun-space transmission map.
- Cache rays and probe rays pass panes untouched.
- The live reference treats panes as thin films and slabs (#191, #197).

On Outpost's hangar the cache's rays passed the clerestory glass at full strength and found
sunlit outdoor surfaces. The engine's walls and ceiling are then 30-60 codes brighter than the
live reference with the sun on, and match it within a few codes with the sun off
(docs/design-gi-accumulate.md, term table, 18 Nov). Keeping the panes as blockers
(`RENDERLAB_TRANSPARENT_OPAQUE=1`) brings the back wall to the reference's level (109 against
108). Glint's views do not move.

## Steps

1. **Masks (#194, merged).** Camera, reflection and bounce rays cull with `VISIBLE_MASK`
   (0x02), never 0xFF. A placement under a bit of its own is invisible to them.
2. **See-through placements in the ray scenes.**
   - TLAS: the frame's see-through placements follow the opaque ones under
     `SEE_THROUGH_MASK` (0x04) only. Their custom index points into a see-through table, not
     the traced placement table, so the frame's program lists, materials and shading slots are
     unchanged and no kernel compiles.
   - Software BVH (the cache's non-RT rays): the same placements as instances with a
     see-through flag.
   - Nothing reads the bit yet, so pictures are unchanged.
3. **The shared query.** `transmitted_within(origin, direction, tmax) -> vec3<f32>`, lifted
   from shaders/reference_live.wgsl into one file the tracer and the engine both include.
   - It culls with `SHADOW_CASTER_MASK | SEE_THROUGH_MASK` and `FORCE_NO_OPAQUE`, and visits
     every candidate before the first opaque hit.
   - It returns zero at an opaque hit and multiplies a see-through one in.
   - Both faces count.
   - It is used by the cache's visibility and update rays, the final gather's probe rays, and
     the ray-hardware sun shadow. The software traversal gets the same rule through the
     instance flag.

## Decisions to make in step 2

- **Coverage from texture alpha.** A chain-link fence is mostly holes. Material factors alone
  (`opacity * base alpha`) would make it block fully. The options are:
  - (a) carry the base-colour texture into the see-through table and sample its alpha at the
    hit, which needs a texture binding in every consumer's group, and group 3 is at its
    storage cap (see the memory note on it);
  - (b) take the mean alpha over the texture once at material creation and use it as a
    constant coverage. That is exact for diffuse transport through a fine pattern and wrong
    only for the sharp shadow pattern of a coarse one.

  Proposed: (b) for cache and probe rays, which carry diffuse light. The pixels' sun shadow
  keeps TP2's map until (a) is possible there.
- **What the table holds per see-through placement:** base colour (rgb), coverage, IOR. That
  is 2 words (rgb9e5 colour, coverage and IOR as f16x2), bound read-only in the cache's own
  group, with the final gather's in its own group 2.
- **`Material` keeps its base colour factor and its texture's mean alpha** (it does not today).
