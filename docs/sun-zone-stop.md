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
256-query budget is requested on this moving frame. The requested 16/32 classes
are 65.82%/33.82% of receivers; the latter accounts for 62.78% of actual queries.
302,410 receivers are fully lit and 304,871 fully occluded: 92.80% of valid
sun receivers have unanimous visibility. Of the 32-query receivers,
80,281 pass held-history validation and 80,230 have an agreeing eight-query
prefix. Stopping those saves exactly 80,230 × 24 = 1,925,520 queries (17.006%).

## Picture and validation

- Picture/census base: main `c5d16feb` plus this change (`a3144255`).
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

## Paired timing

Frozen release binaries: main `91b4527b` plus this change, built before merging
`fdf2fb05`. Control has `RENDERLAB_SUN_PASS=1`, `RENDERLAB_SUN_ZONE_STOP=0`;
candidate changes only `RENDERLAB_SUN_ZONE_STOP=1`. These rows isolate the stop
from the measurement split's overhead; they do not compare the fused default
against the split. No probe or detailed pass-log ablation is enabled.

RTX 4070 SUPER, Vulkan, driver 617.14, 2026-11-13, exclusive timing lease
`u-sunrt-matrix-timing-42`: exit 0, 146.85 seconds, no watchdog failures.
Preflight reports no game, other GPU process or build. GPU clocks start at
210/405 MHz (P8) and finish at 2805/10501 MHz (P0). Medians use pass-log frames
40 onward: 121 samples per 1280x720 moving Fox flight visit and 313 per
1920x1080 registered quarry visit. The quarry follows the unchanged p-yard
manifest, including still, motion, hold and cut/recovery states.

Two interleaved rounds, control then candidate in round 1 and reversed in
round 2. Each cell is control → candidate, in milliseconds:

| Scene/path | Round | Camera visibility | Sun | GPU frame span |
|---|---:|---:|---:|---:|
| Flight RT | 1 | 0.1249 → 0.1249 | 1.1141 → 0.9861 | 11.1882 → 11.1135 |
| Flight RT | 2 | 0.1260 → 0.1260 | 1.1192 → 0.9892 | 11.2220 → 11.1493 |
| Flight non-RT | 1 | 0.1987 → 0.1987 | 1.3947 → 1.3937 | 18.9737 → 19.0433 |
| Flight non-RT | 2 | 0.1997 → 0.1997 | 1.3957 → 1.3957 | 19.0710 → 19.0648 |
| Quarry RT | 1 | 0.2949 → 0.2970 | 0.4987 → 0.4997 | 3.3587 → 3.3649 |
| Quarry RT | 2 | 0.2970 → 0.2959 | 0.5018 → 0.5018 | 3.3843 → 3.3567 |
| Quarry non-RT | 1 | 0.6390 → 0.6554 | 0.2304 → 0.2202 | 4.4339 → 4.3858 |
| Quarry non-RT | 2 | 0.6482 → 0.6267 | 0.2191 → 0.2202 | 4.3684 → 4.3745 |

RT flight saves 0.1280/0.1300 ms in sun visibility (11.49%/11.62%) and
0.0747/0.0727 ms in frame span (0.67%/0.65%). Quarry has no repeatable sun
improvement. Non-RT executes the same algorithm: its flight frame is slower
in round 1 and slightly faster in round 2; those rows establish no speedup.
The option remains default off. Outpost's CPU-held eligibility is unchanged,
so its prior 3.01 ms visibility cost does not imply a benefit from this option.
PC integration must measure that workload separately.

Full per-state data and raw pass-log summaries remain in
`D:/Coding/renderlab-work/u-sunrt/timing-matrix-summary.json`.

After merging main `fdf2fb05`, Linux validation passed 294 lib tests, zero
failed, 180 adapter-dependent tests ignored (5.03 seconds). All 34 switch
composition sets passed seven focused tests each, including the eight sets
containing `sun-zone`. `tools/rtvis/switch_pairs.py` now registers the option.

Final GPU starts use the `fdf2fb05`-based binary (release build 29.77 seconds).
All eight new switch sets draw on both paths: alone and paired with GI,
material, sun, light visibility, deferred reflections, reflection diagnostics
and the combined reflection diagnostics. Ten additional standard-program
starts cover each other merged pass and all passes together on both paths.
All 26 starts exit 0, with no panic or validation error. Switch-off flight
also matches the accepted reference on all 30 PNGs per path. Ordinary lease
`u-sunrt-final-starts-01` exits 0 in 136.29 seconds with no watchdog failures;
it makes no performance claim. Evidence: `pass-starts-summary.json`,
`final-off-identity.json` and the lease log under the unit evidence directory.
Later main merges through `d786f5a6` contain port/runner changes only and do
not change the tested wgpu renderer.
