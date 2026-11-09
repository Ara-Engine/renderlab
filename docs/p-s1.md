# P-S1: the umbra certificate (design S1), rejected at its gate

Unit P-S1, 2026-11-09, branch `t3/p-s1` from main `9d38de7`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It builds design S1 of [design-nonrt-sunvis.md](design-nonrt-sunvis.md) and runs S1's own gate. That
gate decides whether S3 follows. Labels: **M** measured here, **I** inference.

## Result

**DON'T MERGE. S1 fails its R0 gate and its cost gate, and it is not merged. The branch carries
this record, the CPU oracle and the gaps entry. The certificate code is kept in the
evidence, not in the shader.**

- **R0 gate (M).** On the 1080p game cut, pixels whose count ends at 0 carry **2.8%** of the
  lookup's work (ring tests plus walk visits). The design's floor was 15%, so S1 is dropped. The
  game cut's cost lies in penumbra and lit-but-walked pixels, not in umbrae. The quarry cut is 20%,
  but its cut is already 9.7 ms.
- **Cost (M).** S1 as built is slower everywhere:
  - game cut: 26.74 → 28.35 ms;
  - quarry still: 1.29 → 2.04 ms. Still frames run no certificate test, so this is codegen, not
    work.
  The certificate fired on 9,981 game cut pixels (66% of the count-0 pixels) and removed 1.4% of
  the walk work.
- **Exactness (M).** The CPU oracle found no false certificate in 175,322 cases. On the GPU, the
  game timing frames that changed all have a cut or the cold first frame in their TR1 history. The quarry's changed pixels all got darker
  on the cut frames.
- **Quality bar (M).** On the quarry quality scene (10 frames), every crop pixel that changed moved
  toward the path-traced reference (4 of 4), but 2 of 26 changed pixels in the whole image moved
  away, through TR1 history. The crop RMS is unchanged to six digits (.015045). That misses the
  strict bar "every changed pixel toward the reference", independently of the cost.
- **RT** is untouched (no RT file changed). The non-RT default is main's.
- **S3** (the design's next step when R0 fails) **is dropped too**, by its own cheap rejection.
  Turning the shared walks off makes the game cut slower (26.75 → 27.66-28.19 ms). The visits a
  replay could skip are 1.2% of the game cut's walk work (quarry: 34%, a separate lead). See
  [S3](#s3-the-cost-screen).
- **Quiet before/after (M), the sun-visibility pass, ms (still / moving / cut):**
  - game: main 2.85-2.86 / 5.61-5.64 / 26.70-26.75; S1 2.95 / 5.83 / 28.35;
  - quarry: main 1.26-1.32 / 1.55-1.57 / 9.42-9.70; S1 2.04 / 2.07 / 11.87.
  Nothing changes on the branch, so the shipped numbers are main's.

## The gate, R0 (M)

The 1080p ARA1 timing scenes, Surface lookup, P-NSQ3 lane split. A counting variant of main's
`vsc_finish` adds each queued pixel's summed lane work (`vsm_batch_work`, ring tests plus walk
visits, over its batches) into free words of the queue tail, and `vsc_claim` copies them to the
cache counters for readback (`P_S1_STATS`). The image is unchanged.

| Frame | Pixels walked | Count 0 | Their share of the work |
|---|---:|---:|---:|
| game 41 (moving, 4 batches) | 952,252 | 12,177 | 3.9% |
| game 72 (cut, 32 batches) | 1,039,584 | 15,150 | **2.8%** |
| quarry 41 | 60,945 | 18,274 | 23.5% |
| quarry 72 (cut) | 102,960 | 22,455 | 20.1% |

A perfect umbra skip could remove at most 2.8% of the game cut's walk work, against the design's
3-10 ms. Every pixel of the game cut walks (1.04 M), averaging about 41 visits a pixel. The cost is
spread over penumbrae and lit pixels whose marches meet candidates. That matches P-NSQ2's C1 result
from the other side: lit pixels were already cheap, umbrae are rare, and the march in between holds
the cost.

## What was built (kept in the evidence only)

The patch is `p-s1/s1-umbra.patch`, the function `vsm_umbra` plus two hooks.

**The test (I, by construction).** Every direction of a pixel's sequence, in any batch or
rotation, is `normalize(l + w)` with `w` in the sun disk (`vsm_direction`). In the lookup's permuted
frame (z the sun's dominant axis), each direction's shear lies within a radius `R` of the central
shear:
`R = (|w|max |l| + |w_along| (|l.z| + |l.xy|)) / ((|l.z| - |w|max) |l.z|)`. Here `w_along` is the
disk's part along `l`, nonzero only if the shadow view's rows are not exactly perpendicular to the
sun. Vertex k of a triangle projects along the central ray to `P_k` at height `h_k`. A direction
with shear offset `d` moves that projection to `P_k - d h_k`. So each of `vsm_hits_sheared`'s three
edge functions is exactly `e0 + cross(d, G)` with `G = h_j P_i - h_i P_j`, linear in `d`, and its
least value over the disk is `e0 - R |G|`. The certificate holds when:
1. all three keep one strict sign over the disk, each above a rounding margin. The margin covers
   the vertex transform rounding differently at another call site (16 ulps of the largest world
   coordinate or translation), the origin, the directions and their shears (`|w|` and `R` inflated
   by 1/1024 plus absolute terms), and the test's own arithmetic;
2. the hit's `t` clears 4 epsilon, for one of two reasons. Either every vertex lies that far toward
   the light: `t` is at least the least height, because the barycentric weights are non-negative
   and `|d.z| <= 1`. Or the plane faces the whole cone one way and lies that far from the origin
   on the light's side, so `t` is at least the plane distance; the plane's tilt from rounded edges
   is bounded;
3. the receiver is not thin.

Then every sequence direction hits that triangle under the watertight test, so the pixel's
exact count is 0.

**Use.**
- The candidate is the triangle of the batch's last walk to stop blocked. It is one private `u32`,
  written at the walk's stop.
- The test runs after the march, outside the walk loops. It runs only after a batch the current
  code already blocked fully, and only when the lane has batches left. The lane then skips the
  rest, counting them 0.
- That batch's count is unchanged by construction. A later batch can differ only where its march
  would have missed the proven occluder, and there the count becomes the exact 0 (darker).
- An earlier version (`s1a`) tested only shared-walk candidates, and after every batch. It was
  byte-identical on the game timing frames, saved 0.13 ms of the game cut, and cost the quarry
  +0.75 ms on still frames.

**CPU oracle (M, before any GPU run).** `tools/p_s1_oracle.rs` is standalone (`rustc -O`). It ports
`vsm_umbra` and `vsm_hits_sheared` line for line to f32. Every certified case must pass two checks:
- the watertight test hits for all 256 sequence directions at four pixel shifts, and for 512 rim
  and disk directions. Each direction is tested unperturbed, with ±6e-7 per component (GPU
  `cos`/`sin`/`sqrt`/`normalize` rounding), and with a 2.5e-7 relative shear error (approximate
  division). Each runs against vertices from two transform rounding orders;
- an exact f64 cone-in-triangle test.

Seeded fixtures use four sun slopes (0.00465-0.08), suns at 10-85 degrees of elevation or anywhere,
world offsets up to 800 m, and some skewed shadow-view bases:

| Fixture | Cases | Exactly contained | Certified | False certificates |
|---|---:|---:|---:|---:|
| boxes on ground, every crossed box triangle | 55,322 | 39,192 | 36,188 (92% of contained) | 0 |
| scan-sized triangles (1-30 mm) at far offsets | 40,000 | 658 | 2 | 0 |
| adversarial rims (edge at the cone's radius × (1 + 1e-7 … 3)) | 40,000 | 31,030 | 3,939 | 0 |
| adversarial heights (plane 0.5-12 epsilon away, tilted up to 80°) | 40,000 | 35,671 | 14,264 | 0 |

The margins are generous: millimetre triangles hundreds of metres from the origin rarely certify,
because the vertex transform bound (16 ulps of 800 m, about 0.8 mm) dominates there.

## Timing (M)

1080p ARA1 timing scenes, release build, RTX 4070 SUPER (NVIDIA 617.14), synchronous GPU spans, one
process per variant (shader files through the screen-only override). Leases l1 and l2 ran under
the shared lock with no compiler running (admission 14.6-15.5 GiB available). Medians of the
sun-visibility pass, ms:

| Scene / variant | still | moving | cut | recovered |
|---|---:|---:|---:|---:|
| game, main (l1 ×2, l2) | 2.850-2.858 | 5.613-5.636 | 26.70-26.75 | 5.570-5.575 |
| game, S1 (l2) | 2.945 | 5.827 | 28.35 | 5.806 |
| game, S1a (l1 ×2) | 2.853-2.860 | 5.588-5.594 | 26.57-26.59 | 5.556-5.575 |
| quarry, main (l1, l2) | 1.264-1.292 | 1.545-1.569 | 9.68-9.70 | 1.931-1.943 |
| quarry, S1 (l2) | 2.043 | 2.069 | 11.87 | 2.424 |
| quarry, S1a (l1) | 2.016 | 2.014 | 11.43 | 2.462 |

## Quality (M)

- **Identity.** Game timing frames: S1a was identical (8/8); S1 matched 1/4. Quarry timing: 0/4
  for both. All the changes are on frames whose history includes a cut or a cold start. Quarry
  cut frame 72 under S1a: 36 pixels changed, all darker (at most 16 codes).
- **Quarry quality scene against the path-traced reference** (`shadow_contact`, 10 frames).
  - The crop RMS is .015045 for both.
  - 4 crop pixels changed, all toward the reference.
  - In the whole image, 26 pixels changed: 24 toward the reference and 2 away (TR1 history). That
    fails the strict per-pixel bar.
- **Original-sun game quality scene** (`sun_shadow`, 12 frames, still to recovered): S1 is
  byte-identical to main (12/12), crop .023813.
- **Not run.** The wide-sun and S2/S2G penumbra fixtures, and the courtyard. Those steps were
  scheduled after the gate, and the lease deadline cut the wide-sun pair. The unit stopped there
  because S1 was rejected on its gate and cost.

## S3: the cost screen

R0 sends the unit to S3, replaying the shared steps' candidate walks across batches. The design
gives S3 its own cheap rejection: turn the shared walks off on the current binary, and drop S3 if
that removes under 3 ms of the game cut. Two screen variants of main (image changes, cost only;
l3, same harness as above). Medians, ms:

| Variant | game still / moving / cut | quarry still / moving / cut |
|---|---|---|
| main | 2.861 / 5.614 / 26.75 | 1.318 / 1.560 / 9.42 |
| shared walks off in every batch | 2.939 / 5.811 / 28.19 | |
| shared walks off but in each lane's first batch | 2.892 / 5.679 / 27.66 | 1.295 / 1.601 / 6.93 |
| S1's file with `VSM_UMBRA` off (codegen control) | 2.853 / 5.593 / 26.78 | 1.283 / 1.545 / 9.25 |

On the game, skipping the shared walks makes the cut slower. The candidates they would decide
stay open, and each open direction then marches and walks on its own. That screen therefore
mixes the walks' cost with their effect. A replay would keep the effect. So the bound that
matters is the walks' own share. A counting variant (l4) sums the visits that shared walks make
in each lane's batches after its first, which are exactly what a replay could skip:

| Cut frame | Walk visits, all | Shared-walk visits after each lane's first batch | Share |
|---|---:|---:|---:|
| game 72 | 43.1 M | 0.50 M | **1.2%** |
| quarry 72 | 29.2 M | 9.99 M | 34% |

Moving frames have none: four batches split over four lanes leave each lane one batch. **S3 is
dropped for the game.** A replay could remove at most about 1% of the game cut's walk work, far
from the 3 ms floor. The game cut's work is per-direction walks in penumbrae and lit pixels: the
shared walks hold little of it, and umbrae hold little of it.

The quarry is different. A third of its cut's walk work is replayable, and the screen above
removed 2.5 ms of its 9.4 ms cut. That is a lead for the quarry's cut, not for the game's: see
[Next](#next).

## Next

- **The game cut's cost.** The work is per-direction walks in penumbra and lit-but-walked pixels:
  about 41 visits a pixel over 1.04 M pixels. Neither umbra certificates (2.8%) nor shared-walk
  replay (1.2%) reaches it. What remains is design S4 (occluder-centric soft visibility: one
  triangle fetch serves all 256 directions), or a cheaper exact per-direction walk. S4 needs the
  patent mapping recorded above extended to its own claims first.
- **Quarry cut, S3.** 34% of the quarry cut's walk work is replayable, and the cost screen bounds
  the gain at about 2.5 ms of 9.4. Building it exactly needs the design's (level, cell, step) keyed
  record with walk fallback, in workgroup memory at 12 warps per SM. Check occupancy in Nsight
  first.
- **The certificate.** If S4 lands, its per-triangle disk coverage could reuse the S1 test (the
  oracle is kept). S1 itself costs about 1.6 ms of game cut from codegen, so any reuse must sit
  outside the lookup kernel's register-critical path.

## Patents (I; engineering reading, not clearance)

Claims were read on Google Patents.
- **US 10,055,883** (NVIDIA, active to 2036): independent claims 1, 11, 12, 14, 18 and 20. Each
  needs three things: a frustum projected from a light-space grid cell, a light-space visibility
  buffer of pixel-fragment footprints, and a per-footprint decision. On top of those:
  - claim 1 needs per-footprint sample counts that follow a tangent plane;
  - claim 11 needs culling at empty cells;
  - claims 12, 14 and 18 need half-plane coverage of the footprint's samples.
  S1 tests one receiver's own direction cone against one triangle. It has no light-space grid of
  receivers, no visibility buffer of footprints and no per-footprint samples, so it does not read
  on these claims.
- **HFTS** (US 10,748,332, active to 2038; US 2018/0268604; continuation US 11,348,308): claim 1
  needs dynamic eye-to-light re-projection driven by counting screen pixels per light pixel, and
  claim 17 adds frustum tracing. S1 has neither.
- **US 2011/0141112** (Intel, abandoned): needs a light-view stencil from the camera depth buffer
  and culling of proxy geometry. Not used.
- The design flagged all three for S4, not S1. Constraint C8 holds: no filter, history or averaging.


## Evidence

`D:/Coding/renderlab-work/p-s1/` (external):
- `s1-umbra.patch`: the S1 shader change, against main `9d38de7`.
- `variants/`: main's lookup and cache files as `base*`, plus these variants:
  - `s1` (S1), `s1a` (the first S1), `nocert`;
  - the counting variants `base-stats`, `s1-stats`, `stats.vsc`, `s3count`;
  - the S3 screens `s3all` and `s3later`.
- `plans/`: lease plans and step logs, l1-l4.
- `runs/`: captures, `run.json` timing and counters per step, `analysis.json`, and the runner
  logs in `runs/runs/`.
- `oracle.txt`.
- `tools/`: `guard.py` (turn, stillkit mutex, build slot, admission), `build.py`, `variants.py`,
  `mkplan.py`, `plan.py`, `analyze.py`, `screen_patch.py`.

`screen_patch.py` is the screen-only change, never committed. It lets the release examples load
`RENDERLAB_VSM_SHADER` and `RENDERLAB_VSC_SHADER` and read the cache counters (`P_S1_STATS`).

Build and leases:
- **One build (build-1, 4.5 min, -j 2).** It held the build slot and the stillkit mutex, with no
  other compiler running, into `D:/Coding/ps1-target`, deleted at the end. It made:
  - the oracle;
  - the screen-patched release `p_nsq_screen` and a naga checker;
  - the release lib test binary.
- **Leases.** Under `scripts/n8-run.py` as owner p-s1, alternating with p-nrt3b per root order:
  - l1: 12.5 min, cut at its deadline after the timing steps;
  - l2: 12.5 min, cut at its deadline before the wide-sun pair;
  - l3: 10.6 min;
  - l4: 2.4 min.
  Admission was at 14.6-15.5 GiB available, with no compiler running.

Not run:
- **The lib suite.** The branch changes only docs and a standalone tool outside the crate, so its
  binaries are main's.
- **The wide-sun pair, S2/S2G and the courtyard.** The gate had already rejected the candidate.
- **RT identity.** RT runs no file the unit touched. The planned RT pair fell after l1's deadline.

Time:
- implementation: oracle, certificate and harness, about 1 h;
- verification: about 1.5 h, most of it waiting for lease turns.
