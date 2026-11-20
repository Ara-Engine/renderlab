# Covered fraction for the non-RT sun lookup

Branch `t3/shadow-fraction` (2026-11-17 to 2026-11-20). `RENDERLAB_VSM_FRACTION=1` turns it on; it is off by default (see [Open](#open)). Labels: **M** measured, **C** read from code, **I** inference.

## What it does

The clipmap holds whichever triangle covers each texel centre, so a shadow edge reads as a staircase one texel high. Main hides that with a per-pixel receiver jitter (`VD_JITTER`), which TR1 averages over frames. The covered fraction draws the edge from the map instead, and the jitter goes off with it.

- **Silhouette pass** (`vsm_silhouette`, levels 0-3). It runs after both raster layers, over a 16x16 tile with a one-texel apron. A texel is marked when its 3x3 neighbourhood disagrees: occupancy, placement, or a pair of opposite neighbours off the texel's plane by more than the peel gap.
- **Candidate triangles.** The neighbours' top ids plus the texel's own top and peel words name them. The index buffer's adjacency ring is added when samples stay open.
- **Coverage.** The candidates are tested at the 8x sample positions. A sample counts as covered when:
  - its highest surface is in the near layer, or belongs to the top's own mesh;
  - or, with no candidate covering it, the texel toward it holds that mesh or a near surface.
- **What the pass writes.** The count of eighths goes into the top word's low height bits. A 16-bit side word holds the covered side's angle and a centre-covered flag.
- **Promotions.** A covered part standing above the centre's surface becomes the top. Within the centre's own mesh only the height rises; the id stays the centre's triangle. The open samples' own surface goes to the peel word.
- **Resolved geometry only.** A texel whose covered samples lie on a triangle smaller than a texel keeps the map's own answer.
- **Disk lookup** (`vd_side`). A partly covered column blocks the k/8 of its directions furthest along its covered side (rank shares) where the sun's cone at the column's top is 2 texels wide or more. Under 1 texel it is binary, by the centre flag, as the map holds it without the fraction. Between 1 and 2 the two blend. The exact hard edges read a partly covered texel by its centre too.

## Measured (M)

- **Rail (Glint, play frame 650, jitter off):** wobble 0.1196 mean, single frames 0.130-0.141. Main with its jitter: 0.40.
- **Contacts (Outpost tarp and robot's feet, Showcase lantern), pole wedge, tower corner:** no texel stairs, matching main. Sheets: `judge/full-2edd1763-*`, `judge/b12-showcase-lantern-4x.png`, `judge/prepr-goal-tower.png`.
- **Lit and grey pixels inside ray hardware's deep shadow** (lit: visibility over 0.5; grey: over 0.1; ray hardware's shadow eroded by 3 px; two runs gave identical counts):

  | Scene | Main lit / grey | Fraction lit / grey |
  |---|---|---|
  | Outpost still | 1415 / 3756 | 1403 / 3717 |
  | Showcase still | 502 / 642 | 505 / 650 |
  | Glint goal view | 138 / 361 | 138 / 361 |
  | Glint goal, tower | 0 / 0 | 0 / 0 |

- **Mark test against the 8x raster** (play frame 650, jitter off, levels 1-3; levels 4 and up hold no partial texels by design):

  | Scene | 8x partial | Missed by the fraction | Extra in the fraction |
  |---|---|---|---|
  | Glint | 10,386 | 887 (8.5%) | 6,593 |
  | Outpost | 27,087 | 13,069 (48%) | 13,006 |

  The resolved-geometry rule hands texels on sub-texel triangles back to the map's own answer, and Outpost's detailed meshes have many of them at levels 2-3.

- **Cost, sun shadow raster in play (ms; two interleaved rounds; Riot client running, contended):**

  | | Main | Fraction |
  |---|---|---|
  | Glint, walking | 1.08-1.09 | 1.50 |
  | Glint, standing | 0.43-0.44 | 0.46 |
  | Outpost, walking | 0.58-0.59 | 0.78 |
  | Outpost, standing | 0.38 | 0.73-0.75 |

  These miss the targets (Glint walking 1.08, Outpost 0.61, within about 0.2). The cost items are not done: folding the neighbourhood scan into the first mip pass, and moving partial columns out of the lookup's inner walk.

## Open

- **Showcase keeps 3 white pixels and 8 grey ones in shadows that main does not have.** The cause: where a caster's outline crosses a texel whose centre holds another, lower object, the promotion makes the caster the top. The exact hard edges then no longer name that lower object, which is itself an occluder for the receiver under it.
  - **Tried and reverted:** keeping the centre's surface in the peel word and reading it there cleared Showcase (503 / 645). It cost Outpost 124 grey pixels near the robot, because the peel word also carries the second layer.
  - **What the fix needs:** a slot for the centre's own surface that is not the second layer, such as a third word for partly covered texels only, or the exact hard edges taking the lower object from the raster's first pass. Until then the fraction stays off by default.
- **Levels 4 and up keep main's answer.** A coarse texel holds more caster detail than 8 samples resolve. On Glint's goal view an overhead structure 90 m up read 5 of 8 covered where it covers all of them. So the tower's edge, read at level 5, is not improved.
- **The two-layer limit.** The map holds two surfaces a texel. A caster hidden under another (the thin shadow beside Outpost's barrel, the band behind its robot) is missing in every map form, this one included.
- **Parked forms, behind switches.**
  - `RENDERLAB_VSM_FRACTION=line` stores each outline edge's line. It failed the rail (0.135, a one-texel sawtooth), leaked at contacts and cost 3 ms walking on Glint.
  - `=msaa` is the 8x multisampled raster, kept as the mark test's reference.
- **Measurement switches.** `VSM_SIL_*` and `VD_FR_*` split the parts; `RENDERLAB_VD` overrides now apply after the fraction's own.
