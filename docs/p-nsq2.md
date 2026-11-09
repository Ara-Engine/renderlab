# P-NSQ2: non-RT sun-penumbra cost and the quarry near-field speckle

Unit P-NSQ2, 2026-11-09, branch `t3/p-nsq2` on main `3910082`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It follows P-NSQ ([p-nsq.md](p-nsq.md)), which gave non-RT RT's penumbra sampling (256/64/32/8
directions a frame under TR1, rotated). Two items were left open in [m7-gaps.md](m7-gaps.md)
("Non-RT sun shadows"): the cost of those directions on non-RT, and about 100 single-pixel
speckles a frame in the quarry's contacts. Labels: **M** measured here, **I** inference.

## Result

**No change merged: neither item is closed. The cut's cost is now located, and so is the
speckle's source; every candidate either saved nothing or failed the bar.** The branch adds this
record and the gap entries only. Code, images and timings are main's.

- **Cost.** Of the 31.7 ms game cut (32.8 ms quarry) at 1080p, the map march is about 24 ms
  (28 ms), the 1-ring walk 4-5 ms, and setup with direction generation under 3.4 ms. Two exact,
  byte-identical candidates were built and timed: a whole-sun-disk clear proof that skips every
  batch's march (no gain) and a ring replay across batches (40% slower). Turning the march's parts
  off one at a time (coarser levels, peel layer, refinements, shorter walks) moves the game cut by
  at most 2.1 ms each. The cost is the per-direction march over candidate texels, spread over all
  of its parts. An exact saving needs work shared across batches inside the march, which the
  current batch-of-eight structure does not expose (see [Next](#next)).
- **Speckle.** Located (M): the occluders the march misses are surfaces under the receiver
  mesh's own top layer, within two texels of the ray origin (scan overhangs over gravel and
  rock contacts). The peel layer skips them unless `peel_own` is on. Testing them only there,
  and letting them block only by an exact hit, takes the quarry contact crop from .01505 to
  .01493, better than main before P-NSQ (.01496; RT .01485). It moves 38 pixels a frame
  darker, 34 of them toward the reference. But it costs both game crops a little (.02394 to
  .02395 original sun, .02398 to .02400 wide sun) and the quarry 3.4 ms moving and 24 ms on cuts.
  It fails the equal-or-better rule and the cost goal, so it is recorded, not merged.
- **RT** is untouched (no RT file changed). **Non-RT timings** are main's (table below).

## Where the cut goes (M)

1080p ARA1 timing scenes, release build, RTX 4070 SUPER (NVIDIA 617.14), one frame in flight,
synchronous GPU spans, shared machine (other units idle). One process per variant: the lookup
module is replaced by a variant file of `shaders/virtual_shadow.wgsl` (`RENDERLAB_VSM_SHADER`,
enabled in release examples for the screens only). Medians of the sun-visibility pass, ms:

| Variant (what runs per batch) | game still / moving / cut / recovered | quarry still / moving / cut / recovered |
|---|---|---|
| main (setup, directions, ring, march) | 2.84-2.90 / 5.60-5.61 / 31.72-31.75 / 5.50-5.66 | 1.39 / 3.98 / 32.79 / 4.29 |
| march off (`VSM_FAR` false) | 2.02 / 2.63 / 8.06 / 2.57 | 0.50 / 0.71 / 4.58 / 0.85 |
| ring and march off (directions only) | 1.87 / 2.02 / 3.39 / 1.93 | 0.28 / 0.27 / 0.48 / 0.33 |

On the cut (32 batches of eight directions per walking pixel) the map march is about 24 ms of
the game's 31.7 ms and 28 ms of the quarry's 32.8 ms. The 1-ring walk is 4.1-4.7 ms, and setup
plus direction generation is under 3.4 ms. The march is where any saving has to come from.

Cost screens on the same binary (each turns one part of the march off, so the image changes;
they bound each part's share, they are not candidates), sun-visibility pass, ms:

| Game timing scene | still | moving | cut | recovered |
|---|---:|---:|---:|---:|
| main | 2.85 | 5.67 | 31.67 | 5.70 |
| no coarser levels (`VSM_ASCEND` off) | 2.82 | 5.28 | 29.76 | 5.40 |
| no peel layer (`VSM_PEEL` off) | 2.90 | 5.29 | 29.54 | 5.24 |
| no crossing refinements (`VSM_REFINE` 0) | 2.82 | 5.42 | 30.68 | 5.43 |
| 8-step walks (`walk_steps` 8, was 32) | 2.85 | 5.54 | 31.65 | 5.62 |

On the quarry, no coarser levels gives 32.07 against 33.06 ms on the cut. No single part
holds the cut: each is 1-2 ms of it. The remaining ~20 ms is the receiver-level march itself:
its near shared steps, the per-direction steps where the max-mip test fails, and the candidate
walks there, repeated for each batch of eight directions.

Main across four visits (l1 twice, l3, l5), sun-visibility pass, ms:

| Scene (1080p) | still | moving | cut | recovered |
|---|---|---|---|---|
| game | 2.84-2.90 | 5.60-5.67 | 31.67-32.04 | 5.50-5.70 |
| quarry | 1.34-1.44 | 3.81-3.98 | 32.79-34.20 | 4.29-4.46 |
| before P-NSQ (p-nsq.md): game / quarry | 2.81-2.98 / 0.79 | 2.90-2.91 / 0.76 | 3.38-3.43 / 1.07 | 2.80-2.88 / 0.93 |

## Byte-identical candidates (M)

Both are exact: they skip only work whose outcome is already decided, and every captured frame
(game and quarry timing scenes at 1080p, frames 41, 72, 73, 87; the original-sun and wide-sun
quality scenes at 960x540, 12 frames each, still to recovered) is byte-identical to main.

| Candidate | game cut | quarry cut | Verdict |
|---|---:|---:|---|
| main | 31.72-31.75 | 32.79 | |
| C1 whole-disk clear proof | 31.53-31.77 | 32.24 | no gain; dropped |
| C3 ring replay | 44.51-44.65 | 33.84 | slower; dropped |

- **C1, whole-disk clear proof.** Every direction of a pixel's 256-direction sequence lies in the
  sun disk's cone, and the march's height schedule does not depend on the batch. So one pass of
  the max-mip over the disk's footprint at each step decides whether *any* batch's march can meet
  a candidate texel. Where none can, every batch skips its march with the same result. It is
  exact but saves nothing: the pixels it proves clear were already cheap, because their marches
  end in a few mip tests. The cost is in pixels whose marches meet candidates and walk them.
- **C3, ring replay.** The ring walk visits the same triangles in every batch. The first
  complete walk recorded the triangles that the disk's cone cull keeps (at most 16, in
  workgroup memory), and later batches tested only those. It is exact but 40% slower on the
  game cut. The extra private state cost the whole lookup kernel registers (the compiler-
  sensitivity noted in N8m), and the ring is only 15% of the cut anyway.

## The quarry speckle (M)

960x540 ARA1 quality scenes; linear display RMS of the crop against the 16,384-sample
path-traced references (quarry `shadow_contact`, 10 frames; game `sun_shadow`, original sun 10
frames, wide sun 11), still, moving, cut and recovered frames. "Speckles" counts crop pixels a
frame that differ from main by more than 0.03 linear, and how many of those moved toward the
reference.

| Variant (from main) | Quarry | Original sun | Wide sun | Verdict |
|---|---:|---:|---:|---|
| main (P-NSQ) | .01505 | .02394 | .02398 | |
| main before P-NSQ / RT | .01496 / .01485 | | | |
| fold stops block (walks stopped at a fold count as undecided) | .01504 | .02394 | .02398 | no effect |
| also degenerate projections | .01504 | .02394 | .02398 | no effect |
| repeat filter off | .01505 | .02394 | .02399 | no effect |
| repeat filter kept only after a walk that ended on the ring | .01505 | .02394 | .02398 | no effect |
| `peel_own` (all own-placement peel candidates) | .01495 | .02398 | .02404 | game worse |
| `peel_own` within the ring's reach, within 2 texels, or at the first level only | .01495 | .02398 | .02404 | identical to `peel_own` |
| near own peel, blocks only by an exact hit (truncation and continuity ignored) | **.01493** | .02395 | .02400 | game worse, cost |

- **Where they are.** All the speckle signal is in own-placement peel candidates within two
  texels of the ray origin: restricting `peel_own` there changes no score. A scan's surface hidden
  under its own top layer is skipped by the march (taken as a fold the top walk decides). Near a
  receiver in gravel or at a rock contact, it is the overhang that blocks.
- **Why the plain `peel_own` darkens the game.** Under `peel_own`, those candidates' walks start
  under the mesh's own top surface (a box's floor under its lid) and often cannot reach the aim.
  With P-NSQ's `truncated_walks_block` and the continuity rule, such a walk blocks its
  direction. Letting own-peel candidates block only by an exact watertight hit removes most of
  that: quarry speckles 64.5 a frame (46 toward the reference) become 37.7 (33.7 toward).
- **Why it still fails.** On the game crops, the exact-hit variant still darkens about 50 pixels
  a frame by small amounts (above 0.005), 40 of them away from the reference. That is
  .00001-.00002 RMS, but it is worse. Its cost is also against this milestone: on the quarry,
  still 1.34 -> 2.22 ms, moving 3.81 -> 7.21 ms and cut 34.2 -> 58.5 ms, from walks under every
  near own-placement peel. On the game, cut +3-4 ms.

The variant is kept as a patch in the evidence (`variants/peel-own-near.patch`, setting
`peel_own_near`, off by default).

## Next

- **Cost.** The only exact lever left is sharing work across batches inside the march. A shared
  step (the cone's footprint within half a texel) samples the central ray, so its candidate walk
  follows the same path in every batch; only the directions tested against each visited triangle
  differ. A per-pixel record of those walks' triangles would let later batches test them without
  walking. C3's lesson applies: the record must not raise the kernel's register count, so it
  belongs in workgroup memory, with an Nsight occupancy check (P-NRT's recipe) before timing. A
  wider batch (16 or 32 directions per march, sharing walks) is cheaper still but changes which
  candidates are presented. It would need the full quality battery on every crop.
- **Speckle.** The exact-hit own-peel rule finds the right occluders. What remains: (1) why its
  exact hits darken the game's penumbrae against the reference (likely shared caster
  geometry such as a box's floor at ground level; check with a VSM_DEBUG trace of the ~50
  pixels), and (2) its cost, which comes from walking every near own peel; a cheaper exact
  presentation would test only the peel triangle and its edge neighbours, not walk.

## How Unreal handles the same two problems

Unreal's virtual shadow maps use shadow-map ray tracing (SMRT): a few rays per pixel toward the
light's disk, each tested at samples against the virtual shadow map's depth, with TSR integrating
the noise ([Epic: Virtual Shadow Maps](https://dev.epicgames.com/documentation/unreal-engine/virtual-shadow-maps-in-unreal-engine)).
Two of its public settings answer the two items here:

- **Cost.** SMRT's ray and sample counts are the cost knobs (`r.Shadow.Virtual.SMRT.RayCountDirectional`,
  `SamplesPerRayDirectional`), and its adaptive ray count fires fewer rays where the first ones
  agree. That is a lower budget wherever the estimate looks settled, so it changes the image.
  Epic's forums tie moving-shadow artifacts to it. Under this unit's bar it is not allowed unless
  it reaches the same converged image and per-frame error on cuts. Renderlab's per-direction
  test is exact rather than a depth comparison, so its cost per direction is higher. Unreal's
  answer is fewer and cheaper rays; it has no exact equivalent to borrow.
- **Near field.** Unreal does not trace the contact region from the shadow map alone: a depth
  bias and ray start offset skip it, and screen-space contact shadows fill it in. Screen-space
  methods are excluded for core shadows here (off-screen and disocclusion failures). Renderlab's
  near field comes from the receiver's exact 1-ring and the march's first texels instead.

Patent policy: nothing in this unit adds a filter, reservoir or history; the variants only change
which walks run and how an undecided walk is counted.

## Evidence

`D:/Coding/renderlab-work/p-nsq2/` (external): `variants/` (shader variant files and the patch
scripts that made them, `tools/patch_*.py`), `plans/` (lease plans and step logs), `runs/l1`,
`runs/l2` (screens: `t-*` timing at 1080p, `q-*`, `qq`, `go`, `gw` quality at 960x540),
`runs/*.json` (scores), `runs/*-waiter.log`, `tools/` (lease waiter, plan runner, `timing.py`,
`identity.py`, `score.py` from P-NSQ). References: ARA1's path-traced quarry
(`phase2-runs/references-corrected/quarry-daylight-quality`), game
(`references-hq-final/game-base-quality`) and P-NSQ's wide-sun references (`p-nsq/runs/ref-wide`).

Leases (scripts/n8-run.py under the shared lock, owner p-nsq2): l1 (cost split and C1/C3 identity
and timing, 2.8 min), l3 (speckle variants and march screens, 5.1 min), l4 and l5 (near-field
own-peel variants and their cost, 1.4 and 2.1 min), l6 (lib suite: 199 passed, 0 failed, 164
ignored; the branch changes docs only, so the binary is main's source). A mis-gated l2 waiter
took one out-of-turn window at 14:18 UTC and stopped on the RAM guard after two screens; its
output was discarded and the schedule corrected. Binaries: release builds of main `3910082` in
`D:/Coding/pnsq2-target`, with the variant-file override enabled for the examples during the
screens only (not on the branch), deleted at the end.
