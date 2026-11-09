# Design: non-RT sun visibility cost (P-DESIGN)

Unit P-DESIGN, 2026-11-09, branch `t3/p-design` from main `9fd0a9e` (after the P-NSQ3 merge).
Milestone 8 ([milestone-8.md](milestone-8.md)): beat Ara on performance with zero quality loss.
This is a study and design note only. No build, lease or test ran for it.

Labels: **M** measured (cited), **A** reported by another unit, **I** inference made here.

Scope: the default non-RT sun lookup, `VirtualLookup::Surface`. Its files are
`shaders/virtual_shadow.wgsl`, `shaders/virtual_shadow_cache.wgsl` and
`src/virtual_shadow_gpu.rs`.

## 1. Measured cost

1080p, RTX 4070 SUPER (NVIDIA 617.14), ARA1 timing scenes, sun-visibility pass in ms. Values
come from shared-machine leases, so they are ranges.

| Scene | Still | Moving | Cut | Recovered | Source |
|---|---|---|---|---|---|
| game, before P-NSQ (8 fixed directions, walk cap 16) | 2.81-2.98 | 2.90-2.91 | 3.38-3.43 | 2.80-2.88 | p-nsq.md (M) |
| game, P-NSQ (rotated directions, 8/32/256 a frame) | 2.84-2.90 | 5.60-5.67 | 31.67-32.04 | 5.50-5.70 | p-nsq2.md (M) |
| game, P-NSQ3 (batches spread over 8 lanes, merged) | unchanged | span about +0.14 | **26.72** (was 31.41) | span about +0.14 | decisions TSV, P-VERIFY q1 (M) |
| quarry, P-NSQ | 1.34-1.44 | 3.81-3.98 | 32.79-34.20 | 4.29-4.46 | p-nsq2.md (M) |
| quarry, P-NSQ3 | unchanged | | **9.28** (was 30.89) | | decisions TSV (M) |

Where the cost goes:

- **Cut (P-NSQ2, game 31.7 ms before P-NSQ3).**
  - The map march takes about 24 ms (quarry 28 ms).
  - The receiver's 1-ring walk takes 4.1-4.7 ms.
  - Setup and direction generation take under 3.4 ms. A directions-only screen gives game
    3.39 ms and quarry 0.48 ms.
  - Turning off any single part of the march (coarser levels, peel, refinements, shorter
    walks) saves at most 2.1 ms each.
  - The cost is the march itself, repeated for each of the 32 batches of eight directions.
    Every batch re-runs setup, the ring and the march from the origin.
- **Where the cut does not go (P-NSQ2 C1, M).** A whole-disk clear proof that skipped every
  batch's march saved nothing: lit pixels already end in a few max-mip tests. The cost sits
  in pixels whose marches meet candidates and walk them, which are umbra and penumbra
  receivers.
- **What P-NSQ3 shows (M).** Spreading a pixel's batches over 8 lanes took the quarry cut from
  30.9 to 9.3 ms, because the quarry's cost was imbalance within warps. The game cut only fell
  from 31.4 to 26.7 ms. The game's remaining cost is total work, not imbalance.
- **Still frames (P-NRT v0.1.0 profile, game-base, before P-NSQ).**
  - `vsc_missed` (exact walks of queued pixels) takes 43% of the pass.
  - `vsc_claim` (3x3 consensus, nine fills' table probes) takes 42%.
  - Probe and resolve take about 7% each.
  - All of these are global-memory latency, with long-scoreboard stalls of 90-99%.
  - TR1's jitter moves every receiver point every frame, so still frames re-walk like moving
    ones.
- **P-NRT2 frame share at frame ~1200 (M):** game-base 2.54 of 6.08 ms GPU, the largest single
  pass. Game-atmosphere 2.55 of 8.56.
- **Comparators.**
  - Unreal VSM's camera-moving shadow share is about 0.50 ms (UE 5.7.4, no RT, measured by
    the shadow hillclimb; also non-rt-n5.md).
  - Ara raster's whole frame is 1.86 ms on game-base (ARA1).

Quality to keep (M):
- Wide-sun game `sun_shadow` crop .02398 against the path-traced reference (RT .02374).
- Original-sun crop .02394.
- Quarry `shadow_contact` .01505, which carries the open near-field speckle item; main before
  P-NSQ was .01496.
- Courtyard penumbra width equal to RT's.

Any candidate must hold every one of these, still, moving, on the cut and recovered, and on
both paths. RT is untouched by everything below.

## 2. What the best engines do

- **Unreal Engine 5, Virtual Shadow Maps with SMRT** ([Epic: Virtual Shadow Maps](https://dev.epicgames.com/documentation/unreal-engine/virtual-shadow-maps-in-unreal-engine)).
  - 16k virtual clipmap pages are cached across frames. Static and dynamic geometry are
    cached separately, so a static page survives camera motion.
  - Soft shadows come from shadow-map ray tracing: a few rays toward the light's disk
    (`r.Shadow.Virtual.SMRT.RayCountDirectional`), each sampled at a fixed number of points
    against the page depths (`SamplesPerRayDirectional`). An adaptive ray count fires fewer
    rays where the first agree.
  - Costs stay low because a sample is one depth compare, the ray count is single digits, and
    TSR integrates the noise.
  - The near field relies on a short screen-space trace (`r.Shadow.Virtual.ScreenRayLength`)
    and contact shadows. **Screen-space is excluded here** for core shadows.
  - Depth compares and adaptive counts are approximations. Renderlab already beats VSM's
    image (ARA1, N8o), so these knobs are not borrowable without a quality trade.
- **Unity HDRP** ([HDRP shadows](https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Shadows-in-HDRP.html)).
  - Cascades with PCF or PCSS filtering, cached shadow maps for static lights, and
    screen-space contact shadows (excluded).
  - Optional ray-traced shadows go through a spatio-temporal denoiser.
  - None of these is exact.
- **Frostbite and EA SEED.**
  - Frostbite ships cached cascades and filtered maps.
  - SEED's PICA PICA (Andersson and Barré-Brisebois, GDC 2018, "Shiny Pixels and Beyond") used
    one RT shadow ray per pixel plus denoising for area-light penumbrae.
  - Neither is a non-RT exact method.
- **id Tech 7** (Geffroy, Gneiting and Wang, "Rendering the Hellscape of DOOM Eternal",
  SIGGRAPH 2020 Advances).
  - A shadow-map atlas with static caching, so unchanged lights are not redrawn.
  - The sun uses cascades with filtering.
- **Exact, occluder-centric research.** These are the only published non-RT methods that give
  per-sample exact area-light visibility:
  - Aila and Laine, "Alias-Free Shadow Maps" (EGSR 2004), and Johnson et al., "The Irregular
    Z-Buffer" (ACM TOG 2005): per-receiver exact hard shadows.
  - Sintorn, Eisemann and Assarsson, "Sample Based Visibility for Soft Shadows using
    Alias-free Shadow Maps" (EGSR 2008), and Johnson, Hunt, Mark, Burns and Junkins, "Soft
    Irregular Shadow Mapping" (I3D 2009). Each caster triangle is scattered over the receivers
    in its penumbra footprint, and each receiver's light-sample bitmask is cleared by the
    samples the triangle covers.
  - Lehtinen, Laine and Aila, "Hierarchical Penumbra Casting" (Eurographics 2008): the same
    idea over a receiver hierarchy.
  - Wyman, Hoetzlein and Lefohn, "Frustum-Traced Raster Shadows: Revisiting Irregular
    Z-Buffers" (I3D 2015), and NVIDIA's HFTS: exact hard shadows at game rates.
  - The common point: the cost scales with caster-receiver pairs, and every light sample of
    a pair is tested in registers. There is no per-direction march.

**Ara** (read-only, `D:/Coding/renderlab-work/ara1/ara-main` at `1d2d647`, the ARA1 revision;
the requested `snengine/t3-9c4a1f92` worktree is empty):
- `crates/ara-render/src/look/cascades.rs`: one D32 map per cascade, redrawn only when its
  matrix or the casters in its volume change. A still camera over a still scene draws none.
  Moved casters patch only the texels they reach.
- `look/vsm.rs` (optional): Unreal-style clipmap pages with a PCSS-style blocker search one
  level coarser (`vsmSoftVisibility`).
- `look/horizon.rs`: a heightfield horizon for distant terrain.
- Each pixel costs a handful of filtered depth taps. There are no triangles and no per-direction
  rays.
- That is why it is fast, and also why its shadows score 70.4% worse than Renderlab non-RT
  against the reference (ARA1 sun-shadow crop .07317 against .02169). PCSS approximates
  penumbra width from an averaged blocker depth, and PCF leaks at contacts.

**Conclusion.** No engine has a cheaper exact per-direction test to copy. Every exact saving
has to come from one of three places:
- deciding many directions at once (certificates);
- sharing work across the 32 batches;
- changing the loop order to caster-centric, so that one triangle fetch serves all 256
  directions.

## 3. Candidates, ranked by expected saving

### S4 (largest, longest): occluder-centric exact soft visibility

**Idea.** Replace the per-direction march with Johnson et al.'s or Sintorn et al.'s loop
order:
1. Bin each frame's receivers (the VB's pixels) into light-space tiles, with each tile's
   height range.
2. For each caster cluster, then each triangle, take the light-space footprint dilated by
   `disk slope x height above the tile's lowest receiver`. Cull it against the tile bounds.
3. For every receiver in the footprint, run the existing watertight test (`vsm_hits_sheared`)
   for each of the frame's directions, in registers, and clear the hit bits of the
   receiver's 8/32/256-bit mask (atomicAnd).
4. The count is popcount(mask).

**Expected saving (I).**
- The cost becomes one triangle fetch per pair, plus K ALU tests per pair.
- The march's per-direction memory walks disappear, and so do repeated batches. A 256-direction
  cut would cost about K/8 times a still frame's ALU, not 32 times its memory chain.
- Estimate: cut 26.7 to about 4-8 ms, moving 5.6 to about 2-3 ms on game.
- **Quarry risk:** 12 scans of about 1-2 M triangles self-shadow everywhere. Pair counts could
  exceed the march's work unless the culling is hierarchical (cluster boxes against tile boxes,
  as Sintorn et al. 2014 do for hard shadows).

**Quality argument.**
- Every caster triangle whose footprint covers a receiver is tested against every direction,
  with the same watertight function. Nothing is truncated, and no candidate is missed.
- Directions the current march decides correctly get the same verdict, so the image is
  byte-identical there. The exception is the cross-module bit risk; the test function is
  shared text, but it must still be hashed.
- The only directions that change are those the current lookup gets wrong:
  - truncated walks blocked by `truncated_walks_block`;
  - the near-field speckle occluders P-NSQ2 located (own-placement surfaces under the top
    layer).
  - Both move toward exact visibility, which is what the path-traced reference computes.
- **Caveat (P-NSQ2):** exact own-peel hits darkened about 50 game pixels slightly *away* from
  the reference, by .00001-.00002 RMS. The cause is unknown; shared ground-level caster
  geometry is suspected.
- Under M8 this is equal-or-better only after the full crop battery. Find that cause first
  (a VSM_DEBUG trace of those pixels).

**Patent check.**
- Claims to read before any build:
  - [US 10,055,883 "Frustum tests for sub-pixel shadows"](https://patents.google.com/patent/US20160203635)
    (IZB with sub-pixel sample masks);
  - the HFTS family, [US 2018/0268604](https://patents.google.com/patent/US20180268604A1/en),
    US 10,748,332 and US 11,348,308;
  - [US 2011/0141112 "Image processing techniques"](https://patents.google.com/patent/US20110141112)
    (IZB light-view stencil).
- The 2004-2009 academic methods predate these, but filings near them may still be in force.
- **Not cleared.** Map the claims element by element, as P0 did, before work starts.
- Constraint C8 still holds: verdicts, no averaging.

**Programmable shading.** None. Programs read the same 0-256 count.

**Memory.**
- Receiver lists plus masks: about 2.07 M x (8 + 4K/32) bytes, which is about 24 MiB at
  K = 32 and about 80 MiB at K = 256, at 1080p (I).
- Offsets: walks and their records would no longer be needed. Quarry walk records are
  447.5 MiB (P-MEM), and the adjacency suffix goes too. So the net could fall, but only if
  Surface's cache and the walk records are retired together.
- Must be counted with P-MEM's method. Non-RT game is already 1,340 MiB against Ara raster's
  619 MiB.

**Cheap rejection (CPU, no lease).**
- Dump one cut frame's VB receivers (world, normal, id) and the caster clusters once, using
  the ARA1 game and quarry timing scenes at frame 72.
- Count footprint pairs per receiver with cluster-then-triangle culling.
- Reject if the quarry's pair count times K/32 exceeds the march's measured work estimate.
  The lookup scratch's per-tile work words are already written for the walk order.
- The same replay checks verdict identity against the GPU output counts.

### S1 (recommended first build): umbra certificate, one exact test for the whole disk

**Idea.**
1. The disk's directions all lie in the cone of half-angle `atan(slope)` about `l`.
   `vsm_direction` draws every one of the pixel's 256 sequence directions from it, whatever
   the batch or rotation.
2. Batch 0's shared near-step walk locates a triangle T under the central aim, the same in
   every batch, since the aim `q = (p.xy, p.z + h)` and the step heights do not depend on the
   batch.
3. If the whole cone from the shared origin passes strictly inside T, then every direction in
   the sequence hits T. "Strictly inside" means each of T's three edge planes through the
   origin has the cone on its inner side by an angular margin above the watertight test's
   floating-point error, and T's plane is in front at `t > 4 x epsilon`.
4. The pixel's count is then 0. Every later batch, on this lane or the P-NSQ3 helper lanes,
   is skipped.
5. Extension: T's coplanar edge neighbours (box faces are two-triangle quads). Test the cone
   against the union's outer edges.
6. Not for thin receivers, whose rays have per-direction origins.

**Expected saving (I, from P-NSQ2's split).**
- Umbra receivers pay setup, the ring (4-5 ms over 32 batches in total) and a march prefix to
  the first candidate, in every batch.
- If umbra pixels carry a third to a half of the game cut's work, a skip of 31/32 (or 3/4 of
  each helper lane's batches under P-NSQ3) saves about **3-10 ms of the 26.7 ms game cut** and
  **0.3-1.2 ms of the 5.6 ms moving frame**.
- Still frames have one batch, so about 0.
- The quarry gains less: its triangles are small, though at 0.00465 rad the cone's
  cross-section is about 2.3 mm per metre of height, so centimetre triangles still certify.

**Quality argument.**
- For every direction, the proof implies the same watertight test the march runs would report
  a hit on T. So wherever the current lookup decides those directions correctly (blocked), the
  count is identical: byte-identical.
- The count differs only where the current lookup leaves open a direction that T provably
  blocks (a missed occluder). There it becomes exact.
- Identity check: every differing pixel must be darker, and each must be a direction the
  certificate proves.
- **Patent:** a geometric containment test. No claim in P0's inventory covers it, and it adds
  no filter, history or averaging (C8). Engineering reading, not clearance.
- **Programmable shading:** none.
- **Memory:** none. One private triangle id and a flag; watch registers, as C3's 40% loss
  warns.

**Cheap rejection, before building.**
1. **R0 (CPU readback, no shader change).** On the game cut frame, read back the lookup
   scratch's per-tile work words and the output counts. Sum the work in tiles whose covered
   pixels end at count 0.
   - Under 15% of the cut's work: drop S1 and go to S3.
2. **After the build.**
   - P-VERIFY's q1 frame set (46 frames: wide-sun, original-sun and quarry, still to
     recovered). Accept only identical frames, or differences that are all darker and all
     certified.
   - Game cut pair at 1080p. Reject below a 1 ms saving.

### S3: replay the shared-step walks across batches

**Idea.** P-NSQ2's lead:
- A shared step's candidate walk follows the same triangle path in every batch: same cell,
  same aim.
- Record its visited triangles once per pixel, in the P-NSQ3 lane's workgroup slot. Later
  batches test their eight directions against the recorded list, with no pointer-chasing
  walk.
- The list's loads are independent, so they issue together. A walk's loads are serialized,
  because each step needs the previous triangle.

**Expected saving (I).**
- The share of the march spent in shared-step walks, times 31/32, times the latency saved per
  step.
- Perhaps 15-30% of the march: **2-5 ms of the game cut, 0.3-0.8 ms moving**.

**Quality argument.**
- Byte-identical only if each batch replays exactly what its own walk would visit and stops
  where it would stop.
- Two details in `vsm_march` break the naive form:
  - The shared cell range depends on the batch's own cone radius (`drift` and `offset` come
    from that batch's active directions).
  - The repeat filter `shared_last` evolves with the batch's open set.
- So the replay must be keyed by (level, cell, step) and fall back to walking when a batch
  asks for a cell or step not recorded.
- `vsm_top_hits` stops early once all of `mask` is blocked. A replay of the full list tests
  more triangles, but cannot unblock anything, and a stopped walk had no open direction left,
  so identical verdicts follow.
- Each truncation and continuity bit (8, 9, `vsm_undecided`) must be stored with the record.

**Patent.** As now (C4, C8; no accumulation).

**Programmable shading.** None.

**Memory.** Workgroup only. The lookup is at 12 warps per SM with shared memory full
(8,448 B a warp, N8m), so any table costs a warp. **Check occupancy in Nsight before timing.**

**Cheap rejection.**
- A cost screen on the current binary: turn the shared steps' walks off (image changes, cost
  only). If that removes under 3 ms of the cut, drop S3.

### S2: exact lookup instead of the surface cache, once certificates exist (quality and still-frame cost)

**Idea.**
- On still frames, `vsc_claim` (consensus and fills) costs about as much as the walks: 42%
  against 43% (P-NRT).
- N8o measured Surface *below* Exact on four of five quarry film-reference measures, because
  cells reuse verdicts from other camera positions. Surface became the default by decision
  (N8p).
- If S1 and the existing lit exits make solid pixels cheap without the cache, the Exact lookup
  could be both better in quality and cheaper on still and moving frames.

**Expected saving (I).**
- Up to the claim, probe and resolve share of a still frame (about 1.4 ms of game's 2.5).
- Minus the walks the cache saved. N8o measured Exact's camera-moving cost at 1.49-1.50 ms
  against Surface's 1.04-1.05, before P-NSQ. So it can lose without S1.

**Quality.**
- The image changes from Surface's reuse to Exact's per-pixel verdicts, which are better on
  the quarry (N8o).
- It still needs the whole crop battery, because Surface wins lit-darkened (37 against 60).

**Cheap rejection.**
- `VirtualLookup::Exact` against Surface on the ARA1 game and quarry timing scenes, once S1
  lands. That is an existing setting, so no new code is needed.

## 4. Recommendation

**First build: S1, the umbra certificate, gated by R0.**

| | |
|---|---|
| Expected saving | Game cut 26.7 to about 17-24 ms; moving 5.6 to about 4.4-5.3 ms; still unchanged (I) |
| Quality | Byte-identical wherever the current lookup is right; elsewhere exact and darker only where a proven occluder was missed |
| Patent | Plain geometric containment; no filter, history or averaging |
| Shading | Same 0-256 count; programs unaffected |
| Memory | None |
| Kill switch | R0's umbra work share under 15%, or a game cut pair below 1 ms saved |

**Next steps.**
- S3 if R0 shows the cost lies in penumbrae.
- Start S4's patent mapping and CPU pair-count replay in parallel. S4 is the only candidate
  that could bring a cut near Ara's 2-4 ms without lowering the 256-direction budget.
