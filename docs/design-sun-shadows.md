# Sun shadows without ray hardware: design record (owner `clipmap`, 2026-11-16 to 17)

Two replacements for main's Disk lookup were built and judged against main and the ray-hardware frame, and neither was adopted. Both are parked on pushed branches. What they showed is that the defects still visible on main come from what the map holds, not from how the lookup reads it. The work now fixes the map, on main's lookup (branch `t3/shadow-map`).

## Main's lookup

The Disk lookup (shaders/virtual_shadow_disk.wgsl) projects map texels back onto 64 fixed sun-disk directions:

- Each texel is a square column: the top layer, the peel layer and a least thickness.
- A pixel ORs the columns' direction masks over a ladder of bands. An occluder at height t is read at the level whose texel is slope*t/3 to slope*t/6.
- The answer is exact for those columns and has no noise. The 64 directions already integrate the penumbra.

These defects remain on main:

- Steps along Glint's rails and in the pole's penumbra.
- A thin band missing behind Outpost's robot. Ray hardware shows it.
- Specks on the spinning bar in play, from answer reuse; reuse is now off on main (#184).

## Tried: the Disk design (`t3/shadow-design`, b4c3b1fc)

It replaced the Disk lookup's special cases with four properties on every receiver:

- **(a) Density:** the level comes from the output pixel's footprint on its worst axis.
- **(b) Filtering:** columns read at sub-texel offsets, one per sample group.
- **(c) Thin casters:** conservative raster for triangles narrower than a texel.
- **(d) Exact edges:** silhouette edges cut the outline texels.

It also made the sparse windows the default (64 pages a side).

Results:

- **Pictures:** behind main.
  - The tarp was grainy.
  - The rails stayed stepped.
  - The pole strip regressed: the peel gap is relative to the texel, so it crumbles at fine density.
  - The thin band stayed missing.
- **Cost:** clip `sun_shadow` was 3.8 / 6.7 / 3.6 ms (Showcase / Outpost / Glint), against main's 0.45 / 1.54 / 0.84.
  - On the sparse pool the page work dominates: requests ~0.9 ms and raster up to 2.1 ms.
  - The lookup was 1.0 / 4.7 / 2.0 ms.
- **Measured correction.** A one-texel box filter does not remove a staircase whose steps are several texels long; Glint's rails step about once every four texels in their band. Filters of 0, 1 and 4 texels on one frame: 0 and 1 show the same stairs, and 4 removes them but blurs and opens gaps (`main-thread/clipmap/design/zd-filter.png`). (d) did not straighten them either (`zd-rails.png`).
- **Track B, rejected.** It used the map as a conservative classifier and gave uncertain pixels software rays. Too many pixels came out uncertain at a 2-texel margin, which works out to 14-55 ms of rays per moving frame on Outpost.
  - Showcase: 11% (78k pixels).
  - Outpost: 53% (768k).
  - Glint: 17% (205k).

**Why not adopted:** the pictures are behind main on most subjects and it is 3-8x the cost.

## Tried: PCSS on the clipmap (`t3/shadow-pcss`, 3028a44e, `RENDERLAB_VD_LOOKUP=pcss`)

This is the industry lookup, as HDRP ships it, on our map:

- a plain depth map of the top layer;
- a blocker search;
- a penumbra from the sun's angular radius;
- 32 bilinear percentage-closer taps over max(penumbra, 1.5 texels), at the level whose texel is about half that radius;
- receiver-plane bias.

What it showed:

- **Lit holes, first version.** The tarp's shadow over the puddle band and the pole band went missing.
  - Diagnostic counts on Outpost, moving: 2.02 M receivers. No read of a page that was not current, and no exit at the ceiling. 35% found no blocker.
  - Cause: the map keeps its light basis while the sun turns up to 0.01 rad, so the sun's cone leans by up to twice its radius. A search disk centred at half the ceiling height misses blockers well below the highest caster nearby.
- **Unrequested reads made loud.** The lookup reads only pages current for the window: resident on the sparse pool, rasterized at this epoch on the dense windows. Where one is missing it moves coarser, and any other read is counted. None occurred.
- **Weak contacts.** Sized for the highest caster nearby, the far taps often missed a small near blocker (a foot). That frame was then lit, and TR1 averaged it into grey.
  - The fix is a two-scale search: a three-texel disk around the receiver decides wherever it finds blockers; otherwise the leaning capsule from the receiver to the ceiling.
  - This restored the tarp's region and the robot's shadow.
- **Still behind main on five subjects** (sheets `main-thread/clipmap/design/judge/s15-*.png`, columns main | Disk design | PCSS | ray hardware):
  - a dotted line along the tarp's inner edge, and a lit crescent;
  - white leak lines under the robot's feet;
  - lit blobs in place of the pole's lit band;
  - the rail post's long shadow missing in Glint, with a dark wedge;
  - the thin band missing.

  Its contact edges are visibly softer than ray hardware's at 1:1.
- **Cost, clip `sun_shadow` (requests / raster / lookup, ms):**

  | Windows | Showcase | Outpost | Glint |
  |---|---|---|---|
  | dense (v1) | 0.16 / 0.04 / 0.28 | 0.23 / 0.04 / 0.70 | 0.20 / 0.04 / 0.42 |
  | sparse | 0.92 / 2.09 / 0.50 | 0.92 / 1.14 / 0.99 | 1.07 / 0.14 / 0.68 |

**Why not adopted:** it is behind main on five subjects. The lookup itself is cheap, but the steps and the missing band look the same through all three lookups (main, Disk design, PCSS). Those two are properties of the map.

## What the detour established about the map

- **The level-of-detail cut does not cause the steps.** The shadow raster draws original geometry at every level: `caster_error_texels` defaults to -1 and nothing overrides it.
- **Casters are not lost at the raster.** The census (`RENDERLAB_VSM_CENSUS=<file>`) lists, per level, the placements the cull sends against those that cover a texel. Lost at a level, with the hardware raster's thin-triangle cover on or off:
  - Outpost: 0-4 placements, all sub-texel parts at coarse levels.
  - Showcase: 0-2.
  - Glint: 3-12, plus 21 placements flagged as casting no shadow.
- **Alpha-blended materials never enter the shadow map.** They go to the transparent pass, and transparent draws are not in the caster table. Outpost's chain-link fence wire is one (`alphaMode: BLEND`); it is the suspect for the thin band.

## Now: fix the map (`t3/shadow-map`, main's lookup)

- **A. Density.** First the test: every receiver one and two levels finer (`RENDERLAB_VSM_FINER=n`, dense and sparse windows). If the steps go, the fix is (a) on main's lookup, with its cost in pages, memory and ms next to main's.
- **B. Cut-out and blended casters.** They cast through the shadow raster with their alpha, as every engine's shadow maps do: an alpha test for cut-outs, and coverage by threshold or a stable dither for blended wire. Both paths follow the same rule.
- **C. Movers.** Per-mover invalidation and answer reuse, off by default until live frames next to the spinning bar and the robot are clean.

Unreal VSM for scale: 0.6 ms on a simple scene, 1.3-3.3 ms on game scenes.
