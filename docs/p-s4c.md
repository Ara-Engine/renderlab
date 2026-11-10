# P-S4C: per-frame choice between S4 and the march, and the plan to retire the march

Unit P-S4C, 2026-11-10, branch `t3/p-s4c`: [P-S4B](p-s4b.md) (`2fe2d1b`) rebased onto main `17bb249`.
Milestone 8 ([milestone-8.md](milestone-8.md)). Design: [design-nonrt-sunvis-s4.md](design-nonrt-sunvis-s4.md).
Labels: **M** measured here, **A** reported by another unit, **I** inference.

## Result

**MERGE.** S4 is on by default where it wins, the game cut is 3.2x faster, nothing measured is
slower, and nothing is visibly worse. The march still serves every other frame, so the patent flag
on it stays open; [Retiring the march](#retiring-the-march) has the plan and the measured gap.

- **Per-frame choice (M).** S4 now runs only on frames where it was measured to beat the march.
  `sunvis_s4::chosen` decides before dispatch from the frame's direction batches, the screen's
  pixel count and the sun disk's angular radius. The caller also requires the scene's S4 geometry.
  - S4 runs from 8 batches up (cuts and restarts) when the screen has at least
    `0.75 M x sqrt(disk / 0.00465)` pixels.
  - That is 0.75 M pixels at the original sun and 2.2 M at the wide sun (0.04 rad).
  - So the 1080p game cut (2.07 M pixels, original sun) runs S4. The 960x540 cuts and the wide sun's
    1080p cut run the march.
- **1080p game cut (M):** the sun-visibility pass goes from **26.54 to 8.41 ms**, and the frame
  span from 31.74 to 13.62 ms. Still, moving and recovered frames are unchanged:
  2.83 / 5.58 / 5.52 ms on main against 2.82 / 5.56 / 5.46 ms here.
- **Other scenes (M):** they run the march, so they cost what main costs, and their frames are byte-identical to main on every captured frame (2 x 12
  at 960x540, 10 at wide-sun 1080p). Cut, ms, main / here:
  - 960x540 original sun: 3.60 / 3.62;
  - 960x540 wide sun: 5.58 / 5.21;
  - 1080p wide sun: 30.19 / 30.16.
- **Image (M, inspected).** At 1080p the cut and the frames after it (72-87) look the same as main.
  The 93 pixels that differ by more than 8 codes on the cut are single pixels on shadow contacts
  and silhouettes: no stray bright pixels, grain or edge crawl. Five frames later, 9 such pixels
  remain.
  - Against RT, on the 1080p `sun_shadow` crop (linear RMS, mean of frames 41-87):
    S4 .00265, main .00306.
  - On the cut: S4 .00078 against main's .00242. S4's error stays below main's through frame 87.
  - Per-pixel RT counts on the cut (frame 72): S4 equals RT on **99.994%** of pixels, 132 differ,
    all darker; the march equals RT on 99.988%, 250 differ, both darker and lighter.
  - The gold sphere's glossy grain on the cut is the same on main (reflections and GI, outside
    this unit).
- **Quality scores.** S4's per-direction test, records and gather are byte-for-byte P-S4B's, and
  P-S4B's ARA1 `sun_shadow` scores still hold: original sun .02378 against main's .02381, wide sun
  .02397 against .02398 (RT .02343 / .02374). Under the selector those 960x540 scenes run the
  march, so their images are main's.
- **Tests.** The full lib suite (the final release test binary) ran inside lease l2r:
  **205 passed, 0 failed**, 164 ignored (the GPU tests, ignored by default). It includes
  `chosen_follows_the_measured_break_even` and P-S4B's four `sunvis_s4` tests.
  - The CPU oracle passed with 0 false rejects and 0 false certificates, run on the measurement
    build's math, which adds the reverted plane reject to P-S4B's; that math is P-S4B's again
    here.

## Per-frame choice

### Break-even measurements

These are cut-frame sun-visibility medians in ms, from one process per scene with the asset loaded
once. Hardware: RTX 4070 SUPER, NVIDIA 617.14. The S4 column is P-S4B's gather.

| Scene | Pixels | Disk (rad) | March | S4 | Faster |
|---|---:|---:|---:|---:|---|
| game 1080p (ARA1 timing) | 2.07 M | 0.00465 | 26.54 | 8.41-9.07 | S4, 3x |
| game 960x540 (ARA1 quality) | 0.52 M | 0.00465 | 3.53 | 5.07 | march |
| wide sun 960x540 (P-NSQ) | 0.52 M | 0.04 | 5.12 | 17.46 | march, 3.4x |
| wide sun 1080p (this unit) | 2.07 M | 0.04 | 30.16 | 30.57 (with the drain below) | tie |

- **The march** grows faster than the pixel count: 7.5x for 4x the pixels at the original sun,
  and 5.9x at the wide sun. Its walks step finer map texels at higher resolution.
- **S4** grows slowly: 1.8x for 4x the pixels. But its cost rises steeply with the sun cone:
  3.4x from the original to the wide sun at 960x540.
- **The fitted rule.** Power laws through each sun's two points give break-evens of about
  0.74 M pixels at the original sun and 1.8-2.1 M at the wide sun.
  - The rule rounds these to 0.75 M, scaled by the square root of the disk, which gives 2.2 M at
    0.04 rad.
  - That keeps the measured tie, the wide sun at 1080p, on the march.
- **Restart frames (8 batches)** follow the same rule. S4's cost does not depend on the batch
  count (next section), while the march's falls with it, so a restart is less favourable than a cut.
  - Restarts were not timed separately. The 1080p game's 64-direction restarts would run S4, which
    is estimated close to a tie ([Not done](#not-done)).
- **Inputs.** All of them are known on the CPU before dispatch: TR1's budget, the
  virtual-shadow target size, `directional_disk_angle`, and whether the scene's records are
  resident.
  - The queued pixel count is known only on the GPU, and it did not separate the measured cases:
    the cuts queue the whole view.
  - A per-tile choice would need the march and S4 in one kernel. That is the codegen hazard of
    P-S1, where a dead certificate cost the march +1.6 ms.
- **The unit test** `chosen_follows_the_measured_break_even` pins the measured choices.

### Where S4's cut cost goes (M)

These measurement-only shader switches have been removed:
- **traversal**: the cooperative BVH gather alone, with no candidate processed;
- **classify**: the gather plus every lane's reject/certify bound, with no per-direction test;
- **full**: S4 as shipped.

| Cut, ms | Traversal | + classify | Full | March |
|---|---:|---:|---:|---:|
| game 1080p | 2.64 | 5.08 | 9.07 | 26.54 |
| game 960x540 | | 3.76 | 5.07 | 3.53 |
| wide sun 960x540 | 0.86 | 4.48 | 17.46 | 5.12 |

- **Traversal is cheap.** Two things cost more:
  - every lane classifying every candidate of its 32-pixel chunk (2.4-3.6 ms);
  - the per-direction tests of the candidates the bound leaves open (4.0 ms at 1080p, 13 ms on the
    wide sun).
- **Wide penumbrae** leave many candidates open. Each open candidate costs up to 256 direction
  generations and tests.

### Two attempts that lost (M, reverted)

1. **Plane reject.** This is a conservative test that a receiver's whole cone starts on one side
   of a candidate's supporting plane and leaves it. It removes the receiver's own triangle and its
   coplanar neighbours, which the edge bound cannot reject.
   - The CPU oracle passed with two new fixtures, receivers on and near the planes, thin
     receivers included: 0 false rejects in 12,000 cases.
   - On the GPU it did not help: the 1080p cut took 9.07 ms with it, against P-S4B's 8.26-8.84 ms.
     Its sqrt adds cost to the classify step, which is already 2.4 ms.
2. **A deferred, direction-outer drain.** Each lane queued its open candidates in workgroup memory
   and tested them together after each flush, generating each direction once.
   - The aim was to stop one lane's 256-direction loop from stalling the other 31 lanes.
   - It was slower: the 1080p cut went from 9.07 to 10.10 ms, and the wide sun's 960x540 cut from
     17.46 to 20.91 ms.
   - The likely causes are another 2 KB of workgroup memory per warp and a plane-record reload for
     every direction and candidate pair (I).

Both are kept outside the repo (`tools/sunvis_s4-drain.wgsl`, `sunvis_s4_math-beside.rs`,
`p_s4b_oracle-planes.rs`). The shader that ships is P-S4B's, plus one fix: a single-batch frame
now writes the batch's count as its verdict code, as `vsc_finish` does. Merged frames never run S4
on one batch, but retirement will need it.

## Retiring the march

**Islam's rule:** no legacy fallbacks. Once S4 can replace the march everywhere, the march is
deleted, together with `occluder_gather`, `chosen`, the walk records, the adjacency and the map
pages only the march reads.

**Patent position.**
- US 8,471,853 and divisional US 8,605,088 stay **AT RISK** for every frame the march still decides
  ([patent-via-8471853.md](patent-via-8471853.md)). With this merge, that is every frame except
  S4-chosen cuts.
- S4's own path stays weight-free:
  - plane records, not vertex triplets;
  - boolean half-space tests: no coordinate weights, no depth reconstruction, nothing from
    `vsm_hits_sheared`.
- P-VIA2's clearance holds only for a complete replacement. This partial deployment does not
  clear the family.

**What S4 costs on every frame today (M).** The S4-on-every-frame switch was measurement-only and
has been removed.

| Scene | Still | Moving | Cut | Recovered |
|---|---:|---:|---:|---:|
| game 1080p, march | 2.83 | 5.57 | 26.54 | 5.33 |
| game 1080p, S4 everywhere | 8.08 | 10.55 | 10.20 | 10.24 |
| wide sun 960x540, march | 0.80 | 1.02 | 5.12 | 1.17 |
| wide sun 960x540, S4 everywhere | 6.14 | 6.26 | 22.88 | 6.65 |

S4 costs about the same on a still frame as on a cut: 8.1 against 10.2 ms. Two things explain it:
- its traversal and classification do not depend on the direction budget;
- its verdicts reach the surface cache like the march's, yet still frames kept re-queuing
  (I: S4 writes solid codes and a fixed work estimate, not the march's walk work; not traced).

**What S4 needs before it can replace the march (I, from the split above).**

1. **Still frames: from 8.1 to under 2.8 ms (game 1080p).**
   - First trace why the queue stays full under S4. A still frame should queue almost nothing.
     If the queue matches the march's, the K-independent part (traversal + classify, 5.1 ms on
     the full cut queue) scales with the queue and may already fit.
   - Then cut classify: a per-lane cull of each candidate leaf against the lane's own cone,
     before the 96-byte record loads. Spatial 8x4 trace-tile chunks instead of probe-order ones
     shrink each union cone.
2. **Moving frames: from 10.5 to under 5.6 ms.** The same steps apply, plus the direction tests
   below.
3. **Wide suns (0.04 rad): direction tests from 13 ms to a few.** Per candidate, a partial
   certificate by sub-disk would decide groups of directions at once, where today's test is all or
   nothing. Directions could then be tested as 2-D sign tests on the disk point: the hit
   condition is linear in the disk coordinates, and stays weight-free.
4. **The quarry.** S4 refuses it today: 13 M triangles, and its GI keeps only a 1 cm cut. A
   full-detail BVH is +401 MB (A, [m7-g1c.md](m7-g1c.md)), which pays only once the march's walk
   records (447.5 MiB, A) and adjacency go with it.
   - Gate: the quarry's still frame, queued pixels times S4-R loads, under today's 1.3 ms.
5. **Then delete:** the march (`vsm_march`, `vsm_walk_batches` on the Surface path), the
   `occluder_gather` setting, `chosen`, and the walk-record and adjacency builds.

None of this fit the time box. The 3 h went to the selector, the two failed speed-ups above and
the queue for the build slot and GPU. The queue waits took about 1.5 h behind the Ara PR gate, the
Ara integration thread and P-FREE.

## Not done

- **Restart frames** (8 batches) were not timed: the ARA1 timing path has none. The rule sends the
  1080p game's restarts to S4. An estimate from the split above (I):
  - S4: about 5.1 ms of traversal and classify, plus a quarter of 4.0 ms of tests, so about 6-6.5 ms;
  - the march: about 26.5 / 4 = 6.6 ms, plus its fixed passes.
  So a restart is close to a tie, probably in S4's favour. Next: time one restart, and raise
  `MIN_BATCHES` to 32 if it loses.
- No per-pixel f64 re-test of the GPU ties (as in P-S4B).
- No Unreal capture of these scenes exists.
- The validation-layer messages in the lease logs (`VUID-StandaloneSpirv-None-10684`) are on main
  too.

## Evidence

`D:/Coding/renderlab-work/p-s4c/` (external):
- `runs/l1q` (march vs S4 cut timings, 4 scenes, S4 on every frame);
- `runs/l1b` (cost split);
- `runs/l2` and `runs/l2r` (final binary: selector timings, RT counts, frames);
- `sheets/` (inspected crops: `g1080-hot-72.png`, `g1080-region-72.png`) and `score/` (the
  `sun_shadow` scores against RT and their sheets);
- `oracle.txt`;
- `tools/`: guard, build, plans, summary, the scorer from P-NSQ, and the reverted attempts.

Build and lease process:
- **Builds:** A (measurement build: oracle, probes, drain, plane reject) and B (final).
- **Leases:** l1q, l1b, l2 (aborted after its first step by the runner's guard when an Ara cargo
  test started) and l2r.
- The count dumps (8 MB each) and the target were deleted after summarizing.

Time:
- implementation: about 1 h;
- verification: about 0.5 h of leases and builds;
- queue: about 1.5 h.
