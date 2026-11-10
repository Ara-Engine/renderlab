# P-S4B: occluder-centric sun visibility on cut frames (S4 build 1), parked

Unit P-S4B, 2026-11-10, branch `t3/p-s4b` from main `8d9ce93`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It builds "build 1" of [design-nonrt-sunvis-s4.md](design-nonrt-sunvis-s4.md): the S4 gather on the
non-RT Surface lookup's queued pixels, on frames of 8 or more direction batches (cuts and history
restarts), for game-class scenes. Labels: **M** measured here, **I** inference.

## Result

**DON'T MERGE (parked for the v0.1.1 release freeze).** S4 is correct and faster on the ARA1 game
timing cut, but slower on the cuts of the two game quality scenes. Shipping it on by default would
slow those cuts, so it needs a per-frame cost choice first (see [Next step](#stopped-at--next-step)).

- **Game timing cut, 1080p (M).** The sun-visibility pass drops from **26.70-27.25 ms to
  8.26-8.84 ms**, about 18.5 ms saved. Four runs each, ABBA over two leases.
  - The cold first frame drops from 200 ms to 54 ms.
  - Still, moving and recovered frames are unchanged within noise.
- **Game quality cuts, 960x540 (M).** S4 is **slower**:
  - original sun (0.00465 rad): 3.40 → 5.93 ms;
  - wide sun (0.04 rad): 5.31 → 16.69 ms.
  Other frames are unchanged.
- **Quarry (M).** The scene is refused (13 M triangles; its GI is not full detail), so the gather
  never runs. Its timing pair is unchanged: cut 9.19 / 9.22 ms, still 1.30 / 1.31 ms.
- **Per-pixel check against RT (M).** 1080p game cut frame 72:
  - S4 matches RT's 0-256 counts on **99.994%** of pixels: 132 differ, 81 of them by 1-2 codes,
    all darker;
  - main's march matches on 99.988%: 250 differ, both darker and lighter.
  - The cold frame 0 behaves the same way: 99.995% against 99.991%.
- **Image (M, inspected).** The cut and the frames after it (72, 73, 76) look the same on both paths.
  In the `sun_shadow` crops, against the path-traced references:
  - S4 is equal to or better than main on every frame, apart from three frames at +.00001;
  - S4 lands on RT's values on the cut. Original sun, frame 72: main .02737, S4 .02724, RT .02725.
- **Patents.** S4 uses none of the claim-1 elements of US 8,471,853 (see [Patent contract](#patent-contract)).
  The gather form avoids US 10,055,883.
- **Tests.** Every unit test passes (M):
  - `sunvis_s4`: 4 tests;
  - `surface_passes_bind_only_their_layouts`;
  - the CPU oracle: 0 false rejects and 0 false certificates.

## What was built

- **`src/sunvis_s4_math.rs`** (dependency-free; the shader's CPU twin) holds three pieces:
  - the plane records;
  - the per-direction test (`hits`);
  - the reject/certify bound (`classify`).
- **`shaders/sunvis_s4.wgsl`**: the `s4_missed` pass, which replaces `vsc_missed` on gather frames.
  - It takes chunks of 32 queued pixels, one pixel per lane.
  - The chunk's origins in the sun frame form the tile cone.
  - The warp traverses the BVH2 cooperatively, with a workgroup stack and a candidate list that is
    flushed to all lanes.
  - Each lane classifies each candidate, then tests its still-open directions (up to 256) one by one.
  - If the stack overflows, each lane traverses again alone (S4-R).
  - The count and queue word are written as `vsc_finish` writes them, so the cache inserts the results
    unchanged.
- **`src/sunvis_s4.rs`**: the geometry, module, pipeline and per-frame decision.
  - Triangles come from GI's full-detail software geometry (`Blas::exact`: built from the drawn mesh's
    own positions and indices), placed with each draw's instance transforms in f64.
  - A scene qualifies when every draw has such geometry over its whole index range, none is skinned,
    and the total is at most 2 M triangles.
  - The light-space BVH2 uses median splits with 4 triangles per leaf, depth at most 31. Records are
    96 bytes. The game uses about 0.9 MB.
  - The records are rebuilt only when the draws, the transforms or the sun change.
  - The pass runs when `occluder_gather` is on (it defaults to on on this branch) and TR1's budget
    gives at least 8 batches.
- **Hooks into existing code:**
  - one optional pipeline and group in `SurfaceCache::encode` (`virtual_shadow_cache.rs`);
  - the setting, state and per-frame flag (`virtual_shadow_gpu.rs`);
  - `Blas::exact` and its accessor (`gi_bvh.rs`, `gi_cache.rs`).
  - `virtual_shadow.wgsl` and `virtual_shadow_cache.wgsl` are untouched. Moving and still frames run
    the same `vsc_missed` pipeline from the same module text.
- **Evaluation key.** `examples/p_nsq_screen.rs` gets a `gather=0|1` key and records which frames ran
  the gather.

### Patent contract

US 8,471,853 claim 1 ([patent-via-8471853.md](patent-via-8471853.md)) requires three steps:
reading the selected triangle's three vertices, solving coordinate weights, and deciding consistency
from those weights. It then reconstructs the occluder's depth. S4 omits all of them:

- **No vertex triplets.** A candidate is a record of planes: its supporting plane and three edge
  planes, built on the CPU. The receiver query never reads a triangle's three vertices.
- **No weights and no depth.** `s4_hits` is a boolean half-line / half-space predicate. The ray from
  `a = o + eps d` crosses the supporting plane (a sign test of `p(a)` against `p·d`). The crossing
  lies inside each edge plane, tested by the sign of the 2x2 determinant
  `e(a)|p·d| - sign(p·d) p(a) e·d`.
  - No coordinate weight is solved, and no consistency is decided from weights.
  - No occluder depth or hit distance is reconstructed. No division happens at all.
- **Nothing reused from `vsm_hits_sheared`.**
- **The certificate.** It is a containment proof for the prism along L (edge values at the origin
  against a reach bound). It is not a depth comparison.

US 10,055,883 does not apply either. The volume is a screen tile's union of receiver cones, with no
light-space grid cell and no light-space receiver buffer, as the design chart says.

## Evidence

**Hardware and build.**
- RTX 4070 SUPER, NVIDIA 617.14, 1080p game timing scene.
- The 960x540 ARA1 quality scenes: original sun, and P-NSQ's wide sun.
- One release build in `D:/Coding/renderlab-work/p-s4b/target`, of `8d9ce93` plus this branch's
  changes, with build-2's plane fix. Build-3 rebuilt only the test binary after a test-fixture fix.
- The screen binary was patched (uncommitted) to read back the visibility counts.
- Synchronous GPU spans, one process per variant. Base is the same binary with `gather=0`, which
  creates no S4 module.

**Sun-visibility pass, medians, ms** (still / move / cut / recovered):

| Run | Base | S4 |
|---|---|---|
| game timing l1 | 2.84 / 5.62 / 27.20 / 5.59 and 3.11 / 5.60 / 26.70 / 5.58 | first build, with its bug: 8.26 cut |
| game timing l2/l2b | 3.09 / 5.75 / 27.25 / 5.58 and 2.88 / 5.68 / 27.23 / 5.58 | 2.90 / 5.69 / 8.84 / 5.55 and 2.86 / 5.88 / 8.53 / 5.74 |
| game quality, original sun | 0.64 / 0.74 / 3.40 / 0.78 | 0.64 / 0.74 / 5.93 / 0.80 |
| game quality, wide sun | 0.87 / 1.07 / 5.31 / 1.16 | 0.87 / 1.03 / 16.69 / 1.11 |
| quarry timing | 1.30 / 1.61 / 9.19 / 1.95 | 1.31 / 1.58 / 9.22 / 1.96 (gather never runs) |

**Crop RMS, mean over captured frames** (`sun_shadow`, linear display, against the 16k-sample
path-traced references):

| Scene | Main | S4 | RT |
|---|---:|---:|---:|
| Original sun | .02381 | .02378 | .02343 |
| Wide sun | .02398 | .02397 | .02374 |

**Bug found and fixed on the way (M).** The first build's count check showed 18k false umbra pixels.
- The cause was a degenerate sliver in the game's ground: two vertices 3e-18 m apart.
- Its third vertex lay on its opposite edge, so the edge-plane orientation was noise and its "prism"
  a half-space, which certified distant ground receivers.
- Fix: such triangles get no record. A third vertex within 4 f32 ulps of an edge plane can never be
  hit by an f32 test.
- A CPU replay over the game geometry (`p_s4b_replay`) went from 816 to 1 of 1600 band receivers
  mismatching the f64 exact test. The remaining one is 2 directions at a shared-edge tie.

**Not done.**
- No per-pixel f64 re-test of GPU ties. They are consistent with ties: 1-2 codes, always darker,
  from the edge-overlap tolerance.
- No Unreal comparison: no Unreal capture of this scene exists.
- The validation-layer messages in the lease logs (`VUID-StandaloneSpirv-None-10684`) are identical
  with the gather off and on, so they predate this unit.

**Files.**
- `D:/Coding/renderlab-work/p-s4b/` holds:
  - `runs/` (l1, l2, l2b, timing `run.json`, frames, score JSONs);
  - the inspected sheets `runs/sheet-crops.png`, `runs/sheet-orig-72.png`;
  - the diff maps `runs/diff-s4-rt-72.png` (first build) and `runs/diff-s4fix-rt-72.png`;
  - `oracle.txt`;
  - the tools: guard, build, plans, counts, diffmap, replay, screen patch.
- The count dumps (8 MB each) were deleted after summarizing.
- The target was deleted.

**Time.**
- Implementation: about 1.3 h.
- Verification: about 1 h, including one lease cut short by my own guard. It read "not stopped" in
  a root order as a stop order; that is fixed.

## Stopped at / Next step

Stopped at a correct, tested build that wins 18.5 ms on the 1080p game cut and loses 2.5-11 ms on
the 960x540 quality-scene cuts.

Why it loses there (I):
- The march's cut cost scales with pixels × walk visits. It is cheap at 960x540 and on scenes where
  few pixels walk.
- S4's per-chunk traversal and per-direction tests in penumbrae grow with the cone. The wide sun's
  0.04 rad cone makes tiles' union cones and penumbrae about 9x larger.

Next step, in order:
1. **A per-frame choice.** Run S4 only where it is cheaper: from the queue size and last cut's
   measured cost on each path, or simply from the sun's angular radius and the queued pixel count.
   Measure the break-even on the three scenes above.
2. **Cut S4's own cost on wide cones.**
   - Smaller chunks (8x4 trace tiles instead of probe order).
   - Certify by sub-disk instead of all-or-nothing, so wide penumbrae resolve more directions per
     candidate.
   - A per-candidate early skip of directions already blocked.
3. Then re-run this record's timing set. Keep S4 where it is faster at all and not visibly worse
   (Islam's 2026-11-10 bar).

The records do not depend on the direction budget, so the same pass can later serve moving and still
frames when the march is retired (the patent flag on the march).
