# Sun shadows without ray hardware: design (2026-11-16, owner `clipmap`)

Replaces the run of special cases on the Disk lookup (edge rays, triangle masks, peel gap, band jitter, soft-edge attempts) with three properties that hold on every receiver at every sun angle. Base: main 5d4f94d5.

## What we have and what failed

The Disk lookup (shaders/virtual_shadow_disk.wgsl) back-projects map texels onto 64 fixed sun-disk directions. Each texel is a square column (top layer, peel layer, least thickness). A pixel ORs the columns' direction masks over a ladder of bands: an occluder at height t is read at the level whose texel is slope*t/3 to slope*t/6. The answer is exact for those columns and noiseless, and the 64 directions already integrate the penumbra (a PCSS blocker search and filter in one).

| Defect seen | Property of our design that allowed it | Industry property that prevents it |
|---|---|---|
| Tarp/puddle steps 5-10 px on the ground | Receiver level from distance x pixel angle, isotropic: texel <= 1 pixel across the view ray, but the shadow of a texel stretches 1/sin(sun elevation) along the ground (4x at 15 deg). | UE VSM marks pages per pixel from the receiver's projected footprint (and biases finer, ResolutionLodBiasDirectional -1.5). |
| Steps inside soft shadows (pole top, Glint rails) | Each band reads squares 1/3-1/6 of the penumbra wide with no reconstruction filter: one binary answer per texel per direction, so the penumbra is a sum of square steps. | UE SMRT and HDRP PCSS sample the map at receiver density with jittered taps over a kernel of at least one texel and TAA/TSR on top: the texel grid is box-filtered by construction. |
| Dotted line (puddle band) | The fix for steps (one jittered ray per pixel) put noise on a mirror-reflected receiver. | Filtering that converges within the frame, not one sample per frame. |
| Missing stretch of a thin shadow | Texel-centre (point) raster: a caster narrower than a texel at that level owns no texel centre. | UE VSM rasterizes Nanite clusters at page density (>= 1 texel per pixel); thin geometry can still drop there too. Conservative raster is the standard guarantee. |
| Tarp missing under the pole | Two layers per texel, the second being a closed caster's own underside. Fixed (peel gap): the peel layer skips surfaces within the least thickness. | (Depth-only maps keep only the nearest surface; SMRT extrapolates under it.) |

Published better-than-industry options considered: irregular z-buffer and frustum-traced shadows (Johnson 2005; Wyman et al. 2015/2016) give exact per-pixel hard shadows, but soft shadows need many frusta per pixel and the cost is several ms at 1080p. Sub-pixel shadow mapping (Lecocq et al. 2014) stores the triangle per texel and tests it exactly: that is our triangle masks; it fixes hard edges only and needs the full triangle set per texel to avoid holes (our soft attempt showed exactly those holes). Variance and moment shadow maps filter cheaply but leak light, which fails the bar.

## Design

**(a) Density from the output-pixel footprint, worst axis.** The receiver level is the coarsest level whose texel is no larger than the smallest light-space step between the pixel's point and its screen neighbours' points (the projected footprint at output resolution, min over both screen axes and both sides; a depth discontinuity takes the other side). A low sun shrinks the ground's light-space step along the sun's azimuth, so it asks for finer pages there. The page budget degrades by taking a coarser level, which (b) renders as a wider soft edge, never as a step. The page requests use the same rule.

**(b) One filtered lookup, the same for every pixel.** Every column is read at a sub-texel offset per disk-sample group: the 64 directions form 4 interleaved groups, each with its own offset of the texel grid, stratified over the texel (2x2 in one frame, rotating over a 4x4 lattice by TR1 frame). The visibility count is then the exact disk integral of the occluders box-filtered by one texel of the level they are read at, in every band. A staircase box-filtered by its step is a straight edge, so steps cannot form, and the frame converges by itself (no single-ray noise). The tangent-plane and bias tests keep the receiver's unshifted point, so the offsets cannot cause acne on any slope. At matched density (a) the receiver band's filter is at most one output pixel: hard contact edges are pixel-antialiased like the ray-traced frame. The triangle masks are not needed for that, and they cannot be made exact everywhere without the full triangle set per texel. They are removed.

**(c) No caster falls between texels.** Software raster: a triangle narrower than one texel along its short axis writes every texel its footprint touches (conservative), while wider triangles stay centre-sampled so wide casters keep their exact size. Hardware raster clusters are large by construction. With (b), a thin caster then casts a soft line of at most one texel at its true position.

## What survives and what goes

- **Removed:** the edge test and 2x2 triangle masks with edge neighbours (`VD_HARD_EDGES`, `VD_EDGE_*`, `vd_edge_test`, `vd_word_mask`, `vd_far`, `vd_replaced`), band jitter, the receiver-only jitter `VD_JITTER` (both replaced by (b)), and the `VD_EDGES` outline lines (off since N-VSM1).
- **Kept as parts of the design:** the band ladder (it is the blocker search), the peel gap (the definition of the layers), the ring skip (exact, now padded by the filter offset), answer reuse (a cache of the same answer; pixels in penumbrae are never reused).
- **Parked:** the play-mode mover work on `t3/shadow-play` (dirty rectangles, clustering, requests hold) returns after this.

## Cost and budget

(b) evaluates each contributing column's mask 4 times; expect +0.3-0.6 ms on Outpost's lookup. (a) adds levels near the camera on low-sun ground (more pages, more memory), within the existing ladder of 8 levels.

Current clip `sun_shadow`, without ray hardware: Showcase 0.40, Outpost 1.21 and Glint 0.64 ms. Unreal VSM: 0.6 ms on a simple scene, 1.3-3.3 ms on game scenes. Each step's cost and the pool or window bytes are measured and recorded per scene.

## Judging

Against the path-traced references in main-thread/ref and the ray-traced frame. Subjects: the tarp and puddle band, the pole, Glint's rails, the robot and its feet, the thin shadow behind the robot, and a spinning bar in play; held, moving and in play. Done means no steps, dots or missing shadows at 1:1 or at 4x.
