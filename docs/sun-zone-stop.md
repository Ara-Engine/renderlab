# Unanimous sun queries in local-light restart zones

`RENDERLAB_SUN_ZONE_STOP=1` opts reconstructed RT frames into the separate sun
pass and permits the existing eight-query unanimous stop inside per-pixel
restart zones. Default off; non-RT is unchanged. No new GPU allocation is
needed for the optimization.

The existing held-history test already reprojects world-still receivers while
the camera moves. It requires a unanimous 3x3 history neighborhood for four
frames, rejects moving/skinned receivers and moving casters' swept shadows,
and is enabled only when the planner permits history (including no sun turn).
Eight fresh queries must agree with that history before stopping. Uncertain
receivers keep their full budget in the original direction order.

Previously, the stop also required the receiver's budget to equal the frame's
budget. A local light's relighting sphere can raise a receiver from 16 to 32
queries to restart the RGB history even when its sun visibility is unchanged.
That equality veto prevented the independent sun-history test from stopping.
The option removes only that veto. It keeps all eight probes, the estimator,
history validation and caster exclusion.

The CPU planner's eligibility is unchanged: fully settled eight-query frames
and turning-sun frames do not gain a new history path. Outpost needs its own
measurement; the game fixture does not establish its speedup. The forthcoming
decode-once material record can replace receiver preparation independently.
Binding 74 belongs to that record and is untouched here.

## Actual ray census

The optional `RENDERLAB_SUN_RAYS_PROBE=<json path>` replaces only frame 32's
split sun kernel with the same estimator plus counters at binding 80. It
counts actual query initializations, requested/effective budgets and uniform
visibility, with a 0..256 query histogram. Temporary counter and readback
buffers are 1,156 bytes each; no per-pixel storage is added. The probe is not
used for performance timings. It requires the split pass (either
`RENDERLAB_SUN_PASS=1` or this option).

2026-11-13, main `c5d16feb` plus this change, RTX 4070 SUPER/Vulkan,
`game_scene --path rt --fox ... --frames 34`, 1280x720, moving flight:

| Frame 32 census | Control | Zone stop |
|---|---:|---:|
| Screen pixels | 921,600 | 921,600 |
| Receiver pixels | 656,696 | 656,696 |
| Requested 16 queries | 432,253 | 432,253 |
| Requested 32 queries | 222,126 | 222,126 |
| Outside the sun estimator's coverage | 2,317 | 2,317 |
| Actual queries | 11,322,424 | 9,396,904 |
| Queries per receiver | 17.2415 | 14.3094 |

The control's actual per-receiver histogram is 337,707 at eight queries,
94,546 at sixteen, 222,126 at thirty-two, and 2,317 with no query. No cold
256-query budget is requested on this moving frame. Of the 32-query receivers,
80,281 pass held-history validation and 80,230 have an agreeing eight-query
prefix. Stopping those saves exactly 80,230 × 24 = 1,925,520 queries (17.006%).

## Picture and validation

- Switch off: all 34 moving frames match the accepted control's PNG hashes in
  flight and night lanterns on both paths.
- Switch on: RT flight has two differing frames, at most five pixels by one
  code. No pixels differ by more than eight codes; frame-to-frame residual is
  also at most one code. The worst crop/full frame was inspected: no visible
  shadow-edge change. RT lanterns and both non-RT clips are byte-identical for
  all 34 frames.
- Linux lib suite: 276 passed, zero failed, 180 adapter-dependent tests ignored.
  The focused test validates the modified module and preserved history/caster
  guards with Naga. Release build: 26.53 seconds.
- Evidence: `D:/Coding/renderlab-work/u-sunrt/zone-pictures.json`,
  `zone-rt-flight-{0,1}-probe.json`, `linux-lib-zone.log`, and
  `zone-rt-flight-worst-f11.png`.

The picture lease completed all eight picture launches successfully, then
the harness stopped because it expected one PNG from `--every 1000` instead
of the first and last frames. That assertion was fixed and only the two
census visits repeated (`u-sunrt-zone-census-01`, 6.86 seconds, exit 0).
Paired timing is recorded separately below.
