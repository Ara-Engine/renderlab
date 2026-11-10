# Design S4: occluder-centric non-RT sun visibility (P-S4)

Unit P-S4, 2026-11-10, branch `t3/p-s4` from main `76f07a1`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It takes design S4 of [design-nonrt-sunvis.md](design-nonrt-sunvis.md) through three steps: the patent
mapping, an exact design, and the CPU pair-count replay that serves as its rejection gate. S1 and S3 were
dropped at their gates ([p-s1.md](p-s1.md)), which left S4 as the only route found toward Ara's 2-4 ms cut
that keeps the 256-direction budget.

This is design and a CPU check only: no renderer build, lease, test binary or renderer change. Labels:
**M** measured here (CPU replay), **A** reported by another unit, **I** inference. The patent readings
are engineering readings, not legal clearance.

## Result

**GO, for one first build: S4 on cut-class frames of game-class scenes. The quarry is deferred on
memory, not on work.**

- **Gate (M).** On the 1080p game cut, S4 needs **3.4 M loads**, against today's **43.1 M walk visits** (A,
  P-S1), which is 13x less. On the quarry cut, the queued-size subset needs at most **14.8 M**, against
  29.2 M, about 2x less.
- **Patents.** Every independent claim of US 10,055,883, the HFTS family (US 10,748,332 and
  US 11,348,308) and US 2011/0141112 (abandoned) was mapped. Three more were found and mapped:
  US 9,639,976, US 8,471,853 and US 9,117,306.
  - The **gather** form below misses at least two elements of every claim.
  - The original **scatter** form, with receivers binned in light space, reads on all four common
    elements of US 10,055,883. Its design-around there would be thin, so it is not used.
- **Exactness.** S4 is not byte-identical to today's march, and no occluder-centric order can be,
  because the march's verdicts carry its heuristics. Instead, each direction's verdict becomes the
  watertight test over every triangle in the direction's cone. That is the hit the RT path's ray and
  the path-traced reference compute for the same ray. So S4's counts can be checked against the RT
  path's counts for the same pixel and the same 256 directions.
- **Expected saving (I).**
  - Game cut: 26.7 ms down to about 4-8 ms.
  - Moving and still frames: unchanged, because they keep the march.
  - Quarry cut, in the second build: 9.4 ms down to about 3-6 ms. Until then it is unchanged.

## 1. Patent mapping (primary claim text)

The claims were read from Google Patents pages, not from summaries. They were fetched on 2026-11-10 and
split with `E:/renderlab-work/p0-patents/scripts/extract_claims.py`. The raw pages and claim text are in
`D:/Coding/renderlab-work/p-s4/patents/` (`raw/`, `claims/`). Family rows were followed one level out.

Two S4 forms are mapped:
- **S4-T (gather, recommended).** A screen tile's receivers query a caster BVH with the union of their
  sun cones. One cooperative traversal per 8x4 trace tile collects the candidate triangles. Each lane then
  tests its own receiver's directions.
- **S4-S (scatter, the original sketch).** Receivers are binned into light-space tiles, and each caster
  triangle is scattered over the tiles its dilated footprint covers.

S4-R, a per-receiver cone traversal of the same BVH and S4-T's per-tile fallback, maps like S4-T.

### US 10,055,883 "Frustum tests for sub-pixel shadows" (NVIDIA)

**Status.** Active; adjusted expiry 2036-09-02. Google lists no continuation. The published application
US 2016/0203635 had broad claims (1, 16 and 19: frustum, primitive, light-space buffer, identify); they
were narrowed at grant.

**The four elements every independent claim (1, 11, 12, 14, 18, 20) shares:**
- (a) a frustum projected from a grid cell corresponding to a light source in light space;
- (b) determining that a primitive intersects the frustum;
- (c) a light-space visibility buffer accessed for a set of pixel fragment footprints corresponding to the
  frustum;
- (d) identifying whether each footprint is shadowed by the primitive, then rendering.

**Each claim's extra element:**
- claims 1 and 20: the number of samples in a footprint follows the tangent-plane orientation of a second
  primitive;
- claim 11: a grid cell whose footprint set is empty, and culling the primitive fragments of its frustum;
- claims 12, 14 and 18: a footprint is shadowed when the half-planes of the primitive's projected edges
  show at least one sample in the footprint covered.

| Element | S4-T (gather) | S4-S (scatter) |
|---|---|---|
| (a) frustum from a light-space grid cell | **Absent.** The volume is the union of one *screen* tile's receiver cones. No light-space grid exists. | Present in substance: a light-space tile dilated toward the light. |
| (b) primitive intersects the volume | Analogue present: BVH nodes and triangles are culled against the tile volume. | Present |
| (c) light-space buffer of pixel fragment footprints | **Absent.** Receivers are read from the screen-space visibility buffer in screen-tile order. | Present in substance: per-light-tile receiver lists. |
| (d) per-footprint shadowed decision | Different: a count of 256 directions toward the light from one receiver *point*. | Same as S4-T. |
| claims 1 and 20: sample count from a tangent plane | **Absent.** The direction count is TR1's frame budget (256/64/32/8). | Absent |
| claim 11: empty cells cull fragments | **Absent.** No light-space cells exist. Sky tiles are skipped in screen space. | Risky: skipping empty light-space bins is the natural implementation. |
| claims 12, 14, 18: half-planes of projected edges over footprint samples | **Absent.** No footprint samples. Each direction runs a ray/triangle test; there is no "any sample covered" rule. | Arguable: per-direction edge functions are half-planes in a sheared projection. |

**Verdict.**
- S4-T misses (a) and (c) in all six claims, plus each claim's extra element. That is clear of the
  combination.
- S4-S keeps (a)-(d), so it would rest on the extra elements alone. Claim 11 is the hazard. Not used.

### HFTS: US 10,748,332 and its continuation US 11,348,308 (NVIDIA)

Both are active, with anticipated expiry 2038-03-15. US 2018/0268604 is the published application of
10,748,332. No further child is listed.

| Claim | Required combination | S4 |
|---|---|---|
| 10,748,332 claim 1 | A compute shader doing dynamic eye-to-light re-projection: a pass counting screen pixels per light pixel into a list, a pass selecting sub-areas against a threshold, new light frusta | No re-projection, no per-light-pixel counts. **Clear.** |
| 10,748,332 claim 9 | An initial projection list, cells over a pixel-count threshold, re-projection of their pixels | Absent. **Clear.** |
| 10,748,332 claim 17 | Re-projection when the screen pixels per light pixel exceed a threshold, plus frustum tracing | Absent. **Clear.** |
| 11,348,308 claim 1 | A frustum from a light-space cell; light-space intersections; hard shadow information; the primitive-to-screen-position distance; soft shadow *from the hard* via that distance | No light-space cell. Soft visibility is an exact per-direction count, not hard shadow softened by distance. **Clear.** |
| 11,348,308 claim 8 | Compute-shader projection, a pixel shader for light-space visibility and distances, soft from hard, frustum tracing | As claim 1. **Clear.** |
| 11,348,308 claim 15 | Light-space visibility information, frustum tracing to hard shadow, light-to-eye conversion, soft from hard using deltas | As claim 1. **Clear.** |

### US 2011/0141112 "Image processing techniques" (Intel)

**Status: confirmed abandoned** (Google: US12/653,296 "not_active Abandoned"). The DE counterpart
(DE102010048486A1) is withdrawn. GB2476140B, CN102096907B and TWI434226B were granted. Their claims were
not read here, because only the US is in this record's scope; their titles and family match the US
application.

Claims 1, 8 and 14 need:
- a camera depth buffer *transformed into a light-view stencil buffer* of visible regions;
- proxy geometry or bounding volumes tested against that stencil and selectively excluded;
- rendering the remaining objects' shadows.

S4 has no light-view stencil. Its culling is of BVH nodes against screen-tile cone volumes, read from the
screen-space buffer. **Clear** in the US, and the foreign counterparts would need the same stencil.

### Patents found in the citation search

The search was the citations and cited-by rows of the three pages above, plus Google Patents keyword
queries.

**US 9,639,976 "Efficient computation of shadows for circular light sources"** (Google; active to
2035-06-05).
- Claims 1, 11 and 20 need four elements:
  - each occluder vertex is projected through the light's *centre point onto the receiver surface*;
  - those projections form an outline polygon;
  - a projection *circle* is drawn around each projected vertex;
  - the penumbra comes from exterior tangents and the umbra from interior tangents.
- S4's disk bounds (S1's `e0 - R|G|` per edge) live in the receiver's direction space. They have no
  projection onto the receiver surface, no outline polygon and no circles or tangents. **Clear.**

**US 8,471,853 "Reconstructable geometry shadow mapping"** (VIA; active to 2032-04-25).
- Claim 1 needs four elements:
  - per-pixel front-face occluder geometry from the light's view;
  - a consistency test solving barycentric weights from the receiver's light-space (x, y);
  - reconstructing the occluder's depth at that point;
  - a depth comparison.
- S4 uses no light-view geometry map and no reconstructed depth. **Clear.**
- **Side finding, outside S4's scope.** Today's march stores a triangle per light texel and walks to
  "the triangle under the aim's light-space xy", then compares the aim height with its plane
  (`vsm_top_hits`). That is near this claim's language. It should get a P0-style mapping in its own
  record. S4 would remove that mechanism from the frames it serves.

**US 9,117,306 "Method of stencil mapped shadowing"** (Adshir, now Snap; active to 2032-12-26).
- Claim 1 needs a ray-tracing system that divides the scene into a uniform grid of cells and registers
  each light's occluders per cell in a volumetric stencil map of raster fragments. At runtime, hit points
  are tested against the surrounding stencil fragments, with intersection tests when uncertain.
- S4-T and S4-R use a BVH: no uniform grid and no stencil map. **Clear.**
- S4-G, the light-space caster grid replayed in section 3, would come closer. It is rejected on cost
  anyway.

**US 6,903,741** (Crytek soft shadows, cited by both NVIDIA patents) expired on 2022-07-18.

**Also checked.** The academic sources are Johnson et al. 2005/2009 (IZB and soft IZB), Sintorn et al.
2008, Lehtinen et al. 2008 and Wyman et al. 2015. Keyword queries ("irregular z-buffer", "alias-free
shadow map", "penumbra casting") returned no other granted US claim on irregular-z-buffer shadows.

**Policy (AGENTS.md).** S4-T is a design-around: it keeps the occluder-centric loop order and drops the
light-space receiver buffer that the only live IZB claims need. Each part's avoided elements are listed
above. No patented method is used, so no named setting is needed.

## 2. Exact design (S4-T with the S4-R fallback)

### What each frame computes

For each receiver of the frame the shared state is unchanged:
- the origin (`vsm_origin`; per direction for thin receivers);
- the 256-direction R2 sequence and TR1 rotation, of which the frame uses K (256/64/32/8);
- the test `vsm_hits_sheared` with `tmin = epsilon`.

For each active direction j, S4's verdict is blocked exactly when **some** scene triangle passes that
watertight test.

S4 gives the same answer whatever order or grouping the triangles are presented in. Every triangle a
direction could hit lies in that direction's ray, inside the receiver's cone
`{x : h > eps, |q(x)| <= slope * h}`. The culls below are all conservative supersets of the cone, so no
hitting triangle is dropped. The count is `popcount(active & ~blocked)`, as now.

### Passes (non-RT, Surface lookup's queued pixels, frames with batches >= 8)

1. **Tile volume (per trace tile, 8x4, one warp).**
   - Read the tile's queued receivers. Compute their origins in the sun frame (q across the sun, h toward
     it).
   - The volume is the q bounding box, the lowest h, the smallest epsilon and the frame's slope. The
     slope already includes `sun_hysteresis` and the 1.001 factor of `prepare_virtual`.
2. **Gather.**
   - The warp traverses the caster BVH cooperatively. A node survives if its box has a point above
     `h0 + eps` whose q lies within `slope * (h - h0)` of the tile's q box; the replay's `cone_box` is that
     test.
   - Surviving leaf triangles are appended to a workgroup candidate ring, in chunks of 32. That needs no
     new global memory: the lookup's `vsm_wg_dirs` and `vsm_wg_shear` slots are freed in this mode.
   - **Fallback (S4-R).** If a tile's candidate count passes 32 x its receiver count (the quarry's
     depth-edge tiles), each lane instead traverses alone with its own cone. The per-tile choice is the
     replay's `hybrid`.
3. **Per receiver and candidate (registers).**
   - Each lane transforms the candidate to its origin's permuted frame, as `vsm_test` already does, and
     classifies it with the disk bounds of P-S1's certificate. Each edge function is linear in the shear
     offset: `e = e0 + cross(d, G)`.
   - **Reject** when some edge's *largest* value over the disk is below `-margin`, or the triangle lies
     wholly below `eps`. No direction can hit.
   - **Certify** when all three edges' *smallest* values exceed `margin` and the plane clears `4 eps`
     (S1's test). Every direction hits, so `blocked = active`.
   - **Otherwise** run `vsm_hits_sheared` for each still-open active direction: the same function, text
     and operands as today.
   - A lane stops when `blocked == active`.
4. **Write the count** to the queue word, as `vsc_finish` does, and the cache proceeds unchanged.

Moving and still frames (32 and 8 directions) keep today's march. The replay (section 3) shows S4's cost
does not depend on K, so it would not pay on one to four batches.

### Why the verdicts are exact

- Steps 1-2 cull only boxes and triangles that lie outside every receiver cone of the tile. A triangle
  outside a receiver's cone cannot meet any of its rays (I, by construction).
- The replay checks this from the other side. Two independent searches, the BVH cone traversal and a
  light-space grid, found **identical pair sets for every receiver** (`g_mismatch` 0, M).
- Rejection and certification must be conservative against the f32 test.
  - Certification is P-S1's oracle-checked test: 0 false certificates in 175,322 cases (A).
  - Rejection is the mirrored bound. It needs the same oracle (`tools/p_s1_oracle.rs`, with the sign
    flipped) **before** any GPU work. That is build step 0.
- Directions left undecided run the shared watertight function. As in S1, one module must compile it for
  both paths, and its output is hashed against an in-module per-direction reference (memory note "same
  program in two modules can differ in bits").

### How it compares with today's verdicts

The march's verdicts differ from exact wherever its rules decide instead of a hit:
- `truncated_walks_block` blocks directions whose walk ran out;
- the continuity rule (bit 8) assumes a surface continues;
- near-field occluders under the receiver's own top layer go unpresented (P-NSQ2's speckle);
- the peel layer skips own-placement surfaces.

S4 replaces each of those with the exact answer. So S4 is **byte-identical wherever today's verdict is
right, and exact elsewhere**. "Exact" here means the same per-direction answer the RT path's ray and the
path-traced reference compute.

**The identity check is stronger than "toward the reference".**
- The RT path traces the same sequence, rotation and origins ([p-nsq.md](p-nsq.md) "RT's sampling"). So
  non-RT S4 counts should equal RT counts pixel by pixel on the same frame.
- Differences are allowed only where the hardware and software watertight tests split a tie. Each must be
  confirmed by an f64 re-test.
- The ARA1 crop battery still decides acceptance, because TR1 history mixes S4 cut frames with march
  frames after the cut. P-S1 failed its strict per-pixel bar through exactly that mixing. That bar applies
  here too, and S4 may miss it the same way.
- The open P-NSQ2 caveat (exact own-peel hits darkened about 50 game pixels away from the reference by
  .00001-.00002) must be traced first with VSM_DEBUG. If S4 reproduces it, the cause is the reference's
  geometry or origin, not S4.

### Constraints

- **Programmable shading.** None affected: programs read the same 0-256 count. Nothing is added to hit
  records, material inputs or the program ABI.
- **Memory.**
  - The game needs nothing new: the non-RT GI already keeps a full-detail BVH4 over the scene
    (`src/gi_bvh.rs`, G1b), which S4 shares. Its cone test runs in object space per instance against the
    CPU top level. Similarity transforms keep the cone a cone; any other transform is bounded by an
    enclosing cone.
  - Candidate lists live in freed workgroup slots. There is no receiver binning and no per-pixel mask
    buffer, because the mask stays in registers as today.
  - **The quarry is the exception.** Its GI uses the cooked 1 cm cut (38.8 MB), which is not exact for
    shadows. S4 needs the full-detail BVH (401 MB, A, [m7-g1c.md](m7-g1c.md)).
  - That passes the budget only if the march's own structures retire with it: the walk records
    (447.5 MiB, P-MEM), the adjacency, and the map pages the march alone reads. That in turn needs S4 to
    serve still frames too, which the replay says it does not do cheaply. So the quarry is a second step.

## 3. CPU pair-count replay (the gate)

### Method

`tools/p_s4_pairs.rs` is standalone. It was built once in the build slot (`rustc -O`) and run one
process at a time, with peak memory about 0.8 GiB (I) and at least 8.9 GiB available.

1. It reads the RLA geometry of the ARA1 timing scenes:
   - game: `ara1/scenes-dry/game.rla`, 9,452 triangles;
   - quarry: `quarry-quality/detail/full.rla`, 13,034,903 triangles.
2. It places every mesh and moves it to the sun frame, then builds a median-split BVH (4 triangles a leaf)
   there.
3. It casts frame 72's primary rays, the cut, at 1080p from `phase2-scenes/*-timing.json`.
4. It takes every receiver of every 4th trace tile in x and y (1/16 of the tiles, whole tiles), and
   scales the totals by 16.
5. For each receiver it generates the 256 sequence directions (`vsm_direction`'s formula). Each cone-cull
   survivor is tested with f64 Moller-Trumbore from the origin, `t >= eps`.

The replay's coverage was checked against the GPU's captured cut frame:
`p-s4/runs/quarry72-cov.png` against `p-s1/runs/l2/t-quarry-base-0/fin/frame-0072.png`.

Its umbra count is close to the GPU's: game umbra receivers 16,944 against R0's 15,150 count-0 pixels (A).
The remaining gap comes from approximations listed under Limits below.

"Loads" counts dependent global fetches: a BVH node visit, a leaf triangle fetched for its cull, or a
tile candidate loaded once and broadcast. Today's unit is P-S1's walk visit: one dependent triangle fetch
with adjacency, up to 8 direction tests, and the march steps around it.

### Numbers (M; cut frame 72, 1080p, scaled to the full frame)

| Quantity | Game | Quarry |
|---|---:|---:|
| Receivers (visible pixels) | 1,458,144 | 405,920 |
| GPU queued pixels on this cut (A, P-S1 R0) | 1,039,584 | 102,960 |
| **Today: walk visits on the cut (A, P-S1 l4)** | **43.1 M** | **29.2 M** |
| Cone-cull pairs (triangles in a receiver's cone box) | 3.86 M | 1.74 M |
| of which: no direction hits / all 256 hit / some hit | 3.81 M / 28.7 k / 16.7 k | 1.21 M / 36.9 k / 492 k |
| Pairs per receiver p50 / p99 / max | 2 / 10 / 20 | 2 / 67 / 310 |
| Receivers: umbra / penumbra / lit (of 256) | 16,944 / 1,312 / 1,439,888 | 34,624 / 1,744 / 369,552 |
| **S4-T:** tile node visits + candidate loads | **3.05 M + 0.18 M** | 34.3 M + 47.4 M |
| S4-T: per-lane register culls | 5.6 M | 1,514 M |
| Candidates per tile p50 / p99 / max | 2 / 18 / 708 | 0 / 4,549 / 529,714 |
| **S4-R:** per-receiver node visits + leaf culls (binary BVH) | 93.6 M + 18.8 M | **29.1 M + 4.6 M** |
| S4-R, 4-wide estimate (node fetches + leaf culls) | 23.2 M + 18.8 M | 6.9 M + 4.6 M |
| **Hybrid** (per tile, the cheaper of T and R) | **3.40 M** | 20.8 M |
| S4-R loads of the heaviest 25% of receivers (about the queued count) | | **14.8 M** |
| S4-G, light-space caster grid (0.2 m game, 1 cm quarry): cells + candidates | 7.6 M + 11.0 M | 2.1 M + 65.6 M |
| Direction tests after reject and certify (K = 256) | 4.3 M | 126 M (all receivers) |
| Today's direction tests, at most (visits x 8) | 345 M | 234 M |

### Reading

- **Game.** S4-T's 3.2-3.4 M loads are **13x below** today's 43.1 M walk visits.
  - 98.8% of cone pairs hit nothing: they are neighbouring ground and box faces whose boxes touch the
    cone. The disk reject removes them in registers, which leaves 4.3 M direction tests against today's
    up to 345 M.
  - The game's cut cost was never occluders: 99% of receivers are lit. The cost is the march re-proving
    "lit" 32 times through candidate texels. S4 proves it once per tile.
- **Quarry.** S4-T fails at depth edges: a tile spanning rock and far ground has a huge union cone
  (p99.9: 174 k candidates). S4-R is what fits there.
  - Over all 406 k receivers, S4-R costs about as much as today's 29.2 M visits over 103 k queued pixels.
  - Over a queued-size subset, it is at most 14.8 M (binary BVH): **2x below**.
  - With a 4-wide SAH BVH, such as GI's BVH4 with 8 triangles a leaf, it is about 5 M (I, scaled by the
    4-wide-to-binary ratio above).
- **S4-G** (a light-space grid of caster lists) loses on both: per-cell lists hold every scan layer of a
  column. It is also nearest to US 9,117,306. **Rejected.**
- **K-independence.**
  - Today's work grows with batches: P-S1 gives 1.3 walk visits a pixel per batch on the game cut, and
    fewer on moving frames.
  - S4's traversal does not. Game S4-T costs about 2.3 loads a receiver, so it breaks even near two
    batches. Moving frames (4 batches) have little walk work left (P-S1 R0: about 0.7 M visits on game
    frame 41), and their 5.6 ms is mostly march texel steps and the cache (A, P-NSQ2).
  - So S4 runs only on frames with at least 8 batches: cuts and history restarts.

**Gate verdict: GO.**
- The game's pair work is clearly below today's: 13x.
- The quarry's is 2x below for the queued subset, but blocked on memory (section 2).

### Limits of the replay (I)

- Light-aligned boxes are tighter than GI's world-aligned BVH4. On world boxes, slanted cones may visit
  1.5-3x more nodes. That still leaves the game at 5-10 M, below 43 M.
- These are not modelled:
  - thin receivers (per-direction origins);
  - alpha-tested and skinned casters;
  - the renderer's exact receiver epsilon (the replay uses 2e-4 m + 2e-5 x distance);
  - the per-pixel sequence shift (a different hash).
- None of these changes pair counts by more than a few percent on these opaque scenes.
- The replay counts all visible pixels. The GPU queue is a subset: 71% on the game and 25% on the quarry.
  The quarry's queued subset is bounded by the heaviest receivers, which is pessimistic.

## 4. Recommendation: first build and expected saving

**Build 1: `VirtualLookup::Surface` with S4-T and the S4-R fallback on frames of at least 8 batches,
where a full-detail BVH is already resident.** That covers game-class scenes; the quarry stays on the
march.

| Step | Gate |
|---|---|
| 0. CPU: extend `tools/p_s1_oracle.rs` with the mirrored reject bound | 0 false rejects. A false reject would add light, so the bar matches the certificate's. |
| 1. Shader: tile volume, cooperative BVH4 gather over GI's records, register classify, per-direction fallback | naga validation. Workgroup memory stays at today's 8,448 B a warp: check occupancy in Nsight first, since 12 warps per SM is the lookup's operating point (N8m). |
| 2. Identity | 1080p game timing cut and restart frames, non-RT S4 against **RT** counts pixel by pixel. Only f64-confirmed ties may differ. Against main, every difference must be one of the four march rules above. |
| 3. Quality | The ARA1 battery: wide-sun and original-sun game crops, still to recovered, both paths. Every crop and population must be equal or better. Trace the P-NSQ2 50-pixel caveat first. |
| 4. Timing | Game cut pair at 1080p. Reject below 10 ms saved. |

**Expected (I).** The cost is fixed setup plus loads times today's per-visit cost:
- game: 26.7 ms / 43.1 M = 0.62 ns, an upper bound, since a visit also carries direction tests and march
  steps;
- quarry: 0.32 ns.

| Frame | Today (M, A) | Build 1 (I) | Saving |
|---|---:|---:|---:|
| Game cut (256 dirs) | 26.7 ms | 1.9 ms setup (directions-only still screen, A) + 3.4-10 M x 0.62 ns = **4-8 ms** | **19-23 ms** |
| Game restart (64 dirs) | not measured | same 4-8 ms | |
| Game moving / still | 5.6 / 2.85 ms | unchanged (march) | 0 |
| Quarry, all frames | 9.4 / 1.55 / 1.3 ms | unchanged (no resident full-detail BVH) | 0 |
| Quarry cut, step 2 (needs the memory trade) | 9.4 ms | 0.3 ms + 5-15 M x 0.32 ns + cache = **about 3-6 ms** | about 3.5-6.5 ms |

This does not reach Ara's 2-4 ms *frame* on a cut. It does bring the sun pass from the largest item
of the game cut to a few ms, at RT-equal per-direction verdicts.

**Step 2 (quarry), only after build 1 holds.** S4 would serve every frame of the Surface lookup's queued
pixels. That retires the march, the walk records and the adjacency, and keeps memory at or below today's.
Its gate is the quarry's still frame: queued pixels × S4-R loads must stay under today's 1.3 ms.

## Evidence

`D:/Coding/renderlab-work/p-s4/` (external, 8.2 MB, no file over 5 MB):
- `bin/p_s4_pairs.exe`;
- `runs/game72.json` and `runs/quarry72.json` (the table above);
- the grid-size runs `runs/*-c*.json`;
- coverage images `runs/*-cov.png`;
- `patents/raw/` (Google Patents pages) and `patents/claims/` (parsed claims, `all.txt`);
- `tools/patch1.py`, an edit helper.

Process notes:
- **Builds.** Three single-file `rustc -O` compiles, about 10 s each, in the build slot.
- **Slot incident.** One cleanup ran while another unit's cargo build (PID 8024) held the slot, and
  removed that lock. It was restored with a note at once, and the lock was gone after that build ended.
  Every later compile took the slot only when it was free and no cargo/rustc was running.
- **GPU.** No renderer build, lease or test binary ran.

Time:
- design and patent reading: about 1.5 h;
- tool and replay: about 1 h (each replay runs in under 15 s).
