# MV1: per-object motion

2026-11-08 overnight unit, branch `t3/m7-mv1`, based on main `fcb1630`
(PP1 merged). Scope: submitted instance-transform history and the motion source read
by TR1 and PP1. MQ1 retains ownership of TR1's filtering/history policy.

## Implementation

The baseline did **not** reproject rigid motion. SK1 supplied previous mesh-local
vertices, but TR1 transformed them with the current instance placement. Transform
uploads also restarted all TR1 history, hiding the missing rigid reprojection by
discarding accumulation. PP1 reconstructed both clip positions from the current
world point, so its blur covered camera motion only and missed SK1 deformation too.

`Instances` now keeps its last successfully submitted transforms. Pending edits,
coalesced edits, failed admission and CPU buffer uploads do not advance that history.
The first submitted pose initializes both transforms identically. A successful
frame commits the current transforms; stopping therefore produces zero object
velocity on the next frame. Replacing a placement collection changes geometry-table
identity even when the mesh and instance count match, invalidating old surface IDs.

The placement-ordered model buffer stores current transforms followed by previous
transforms. Its ordinary model indices and programmable material/vertex interfaces
are preserved. This adds 64 CPU bytes and 64 GPU bytes per instance, plus 64 bytes
in the CPU publication shadow; moving collections copy their submitted transform
snapshot. Deferred transform publication grows from 128 to 192 bytes per placement
(current/previous models and normals). Static frames retain the existing no-upload
comparison. The storage-binding size is checked before allocation. No new binding
or full-resolution texture is required.

`shaders/object_motion.wgsl` is the shared geometry source. Both hardware closest-hit
primaries and the non-RT visibility buffer already publish placement, original
triangle and barycentrics. TR1 and PP1 interpolate the same point on that triangle
in the previous submitted vertex stream, then apply the previous instance transform.
Static meshes alias their current vertex stream; SK1 supplies the previous pose.
Translation, rotation, scale, deformation and their combination therefore use the
same material-independent convention.

TR1 projects the previous point through the previous unjittered camera, and the
current point through the current camera, retaining its footprint offset and
existing history validation. Its previous tangent plane also uses the previous
placement. Rigid movers enter SK1's moving-silhouette source classification.
Transform/derived-normal uploads no longer restart every history. A moving rigid
caster extends SK1's existing swept world-space shadow-refresh region so newly
changed lighting on static receivers does not accumulate stale shadows. Dynamic GI
retains SK1's conservative scene-wide refresh. These edits are marked MV1; resolve
weights, windows, clamping, coverage storage and filtering policy are unchanged.

PP1 writes `current_pixel - previous_pixel` into its existing depth/velocity texture,
in **output pixels**, including when its input is reconstructed/upscaled. Its shutter
fraction and maximum extent still bound the filter. The first frame and explicit
post-camera cuts suppress motion. TR1 reads the same previous-surface source directly;
PP1 does not read or modify TR1's private history.

## Verification

The first screen (`screen-a-3`) stopped after the lib suite: 161 passed and ten
shader-composition tests failed on a duplicate `query` declaration inherited from
main's PP1/SK1 merge. Both units had hoisted R1's query to avoid Naga's invalid opaque
null initialization, leaving declarations at function entry and immediately before
the loop. MV1 removes the latter and reuses the entry declaration. No reflection
algorithm, settings or history behavior is changed by that integration repair.

Pending the retained guarded screen. All test binaries and GPU commands run through
`scripts/n8-run.py` with `D:/Coding/renderlab-work/gpu-slot.lock`; the MV1 wrapper
checks shared QUIET/game-hold state. Compiles use two jobs and an owned D: target.

The focused GPU fixture has a stationary camera and a translating, bobbing, rotating
striped object. It checks per-pixel velocity against CPU barycentric projection,
zero first-frame/stopped velocity, continued TR1 history after transform edits, and
the cleared HDR region behind the moving object. Both paths retain blur-off/on
frames. A moving/rotating cube is added to the quarry, and the unmodified
`examples/game_scene.rs` supplies its bobbing hero/spinning crate/camera flight.
Each path loads the quarry once; the final eight frames provide diagnostic GPU
timestamps. The existing SK1 stream/history GPU regression checks source integration.

Evidence root: `D:/Coding/renderlab-work/m7-mv1/`. No private asset or generated bulk
output is committed.

## Gaps

- Forward raster frames still use PP1's camera-only depth unprojection. MV1 covers
  the shipping traced and visibility-buffer paths; it adds no raster velocity pass.
- Transparency and particle-layer velocities are not represented by the opaque hit.
  Hidden surfaces and shutter visibility changes remain PP1's single-layer limits.
- Removing whole-scene transform resets exposes ordinary changing-lighting/material
  history problems to TR1's existing validation. The swept-caster refresh is
  conservative, and scene-wide with dynamic GI; it can reduce static accumulation.
- No ground-truth shutter, Unreal comparison, film-quality certification or speed
  superiority is claimed by this bounded overnight unit.
