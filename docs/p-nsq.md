# P-NSQ: non-RT sun penumbrae at RT's quality

Unit P-NSQ, 2026-11-09, branch `t3/p-nsq` on main `bdec7b9`. Milestone 8 ([milestone-8.md](milestone-8.md)):
beat Ara on performance with zero quality loss. This unit fixes a quality shortfall recorded in
[m7-gaps.md](m7-gaps.md) ("Non-RT sun shadows"): on non-RT, sun penumbrae were grainy and harder
than RT's. Labels: **M** measured here, **I** inference.

## Result

RESULT_TBD

## Cause (M)

The v0.1.0 courtyard (`game_scene`, 0.04 rad sun at dusk) shows it plainly. RT's penumbrae from
the gate and boxes are about as wide as the shadows. Non-RT's are a few centimetres wide,
grainy, and hard (crops `crops/v010-f16-{nonrt,rt}.png` in the evidence). Two causes, bisected
on the ARA1 game scene with the courtyard's 0.04 rad sun against a 16,384-sample path-traced
reference ([Evidence](#evidence)):

1. **The walk cap collapsed penumbrae (the larger cause).** N8e's cap (`walk_cap: 16`):
   once a receiver's walks had visited 16 triangles and one direction was proven blocked,
   every direction still open was taken as blocked. A penumbra receiver of a distant caster
   always passes 16 visits: its open rays pass near the caster's silhouette in the map, and
   each candidate there costs a walk. So most of the penumbra became umbra, and only receivers
   whose every direction stayed clear kept light. The result was a hard edge where the first
   direction was blocked.
2. **Fixed directions, so TR1 could not integrate them.** On RT, TR1 rotates each pixel's
   sun-disk directions every frame (`temporal_reconstruction::rotation`) and samples
   `ReconstructionShadowBudget`'s 256/64/32/8 directions. The non-RT lookup seeded its 8
   directions from the pixel alone, the same every frame. TR1's history then averaged one
   8-sample estimate per pixel forever: grain that never converged and crawled in motion.

| Wide-sun game, `sun_shadow` crop, 11 frames | RMS vs reference | Grain | RMS vs RT |
|---|---:|---:|---:|
| main (8 fixed directions, cap 16) | .02669 | .01375 | .01298 |
| cap 0 only | .02492 | .01224 | .00930 |
| rotation only (8 a frame) | .02601 | .01275 | .00984 |
| rotation + RT's budget, cap 16 | .02585 | .01245 | .00943 |
| rotation + budget + cap 0 | .02393 | .01059 | .00235 |
| RT (traced Interactive, TR1) | .02374 | .01058 | |

RMS is linear display RGB (ARA1's metric). Grain is the RMS of the residual minus its 5x5 box
mean, which isolates pixel-scale noise. Neither fix alone reaches RT; together they do.

The 16-triangle cap had one job: on the quarry's scans, receivers in umbra whose walks ran out
of steps (8 triangles, doubled per coarser level) before reaching the triangle under the ray
were left open. Those leaks are what the cap hid. Removing the cap alone on the quarry
(narrow 0.00465 rad sun, `shadow_contact` crop) raises error from .01496 to .01549, with a
brighter mean: umbra leaks.

### What decides an undecided direction (M)

The cap hid two kinds of miss on the quarry. Each was measured with RT's sampling and no
cap, on the quarry's `shadow_contact` crop against its path-traced reference (main .01496,
RT .01485):

| Variant (rotation + budget, cap 0) | Walk steps | Quarry RMS | Mean | Wide-sun RMS | Original-sun RMS |
|---|---:|---:|---:|---:|---:|
| truncated walks stay open (N8b) | 8 | .01549 | +.00006 | .02393 | |
| truncated walks stay open | 128 | .01508 | -.00004 | .02393 | .02387 |
| truncated walks block (`truncated_walks_block`) | 8 | .01512 | -.00039 | .02399 | |
| truncated walks block | 32 / 64 / 128 | .01505 / .01506 / .01506 | -.00011 | .02398 | .02394 |
| cap 16 kept, 128-step walks | 128 | .01486 | -.00018 | .02591 | .02395 |
| main | 8 | .01496 | -.00013 | .02669 | .02463 |
| RT | | .01485 | +.00001 | .02374 | .02358 |

- **Truncated walks.** A walk that runs out of steps before reaching the triangle under its
  aim left its direction open. That is the leak the cap was masking. Blocking exactly
  those directions (the map texel's verdict: its surface is above the ray) removes it,
  and 32 steps make truncation rare. Beyond 32 nothing changes.
- **Near-field contacts.** The rest of the quarry gap, about 70-120 pixels a frame against
  main, is single-pixel speckle in the gravel and along rock-ground contact lines, slightly
  brighter. The cap blocked those receivers' remaining directions after their first
  proof. The exact march does not present those occluders as candidates.
  NEAR_TBD

Keeping the cap with long walks is the quarry's best (.01486, RT's level). It still
collapses the wide-sun penumbrae (.02591), so it is not a fix: a receiver's visit count
cannot tell a crevice from a penumbra.

## Fix

FIX_TBD

## How others get soft, stable penumbrae without RT

Unreal's virtual shadow maps use shadow-map ray tracing (SMRT). Each pixel shoots a few rays
toward the light's disk and tests samples along each against the virtual shadow map. The rays
are random per pixel and per frame, and TSR/TAA integrates their noise
([Epic: Virtual Shadow Maps](https://dev.epicgames.com/documentation/unreal-engine/virtual-shadow-maps-in-unreal-engine)).
Its adaptive ray count spends fewer rays where the first ones agree. Epic's forums trace
moving-shadow pixel artifacts to that adaptivity
([forum](https://forums.unrealengine.com/t/virtual-shadow-maps-for-high-quality-rendering-cinematics/1922741)).
More rays is the documented quality lever.

Renderlab's non-RT lookup already decides each direction exactly, with watertight triangle
tests rather than depth samples. It lacked two things: per-frame variation for TR1 to
integrate, and a correct verdict for directions whose walks could not finish. This unit adds
both. Patent policy: no filter, reservoir or history count is added. The rotation and budget
are RTDEF2's existing sampling scheduler, covered by TR1's W5 design-around record
([m7-p0-claims.md](m7-p0-claims.md)). The surface cache still stores consensus-certified
verdicts and never averages walks (constraint C8).

## Evidence

EVIDENCE_TBD
