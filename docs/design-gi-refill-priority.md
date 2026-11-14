# Ordered software GI refill experiment

Default off: `RENDERLAB_GI_REFILL_PRIORITY=1`. This follows the bounded-refill experiment;
it changes admission, not an admitted cell's sky directions, bounce rays or SH reduction.
The receiver record and the 48-byte GI pixel output are unchanged.

`RENDERLAB_GI_REFILL_BUDGET` selects `today`, an integer cell ceiling, or `ms`:

| Mode | New-cell claims/frame | Gathers/frame |
|---|---:|---:|
| `today` | Existing claim policy (normally uncapped) | 32,768 hard ceiling |
| `8192` | 8,192 | 8,192 |
| `12288` | 12,288 | 12,288 |
| `ms` | Delayed GPU-time feedback | Delayed GPU-time feedback |

The old default gather budget is an average and can overshoot; `today` retains its nominal
budget but enforces it. Parent stand-in remains the caller's setting in every mode, so this
comparison does not silently enable it. Hardware rays keep the original module and plan.

Claims run all visible rounds before background rounds. A resolve ticket bounds actual new
cells, including retries across rounds. New claims have no bounce of their own. Gathers run
visible cells without a bounce value, other visible cells, then background. A gathered zero
uses `GI_BOUNCE_NONE`; it is distinguished from a cell with no value yet. The visible stages
share 80% of a finite ceiling; background uses the remainder, including unused visible tickets.
Cells denied a gather keep their schedule and requeue. Deferred misses retain the existing
request-again behavior; a cell ceiling does not by itself guarantee fast image convergence.

Gather classes are snapshotted into the existing per-slot claim scratch before the first
stage changes any bounce value, then cleared before claim rounds. This prevents a cell that
just gained its first value being gathered again in the next visible stage. Counter 47 uses
one atomic add per candidate. Tiny phase-boundary kernels clamp the gather ticket to the
actual admitted visible count and reset claim tickets from the actual created count before
each round. Denied claims skip bucket walks. A global compare/exchange loop was rejected:
it serialized admission and made the first fixed-budget timing rows much slower than default.
No new per-cell allocation is needed.

The `ms` mode targets `RENDERLAB_GI_REFILL_MS` (3 ms by default) across gather and claim/setup/
sky-trace/finish work, initially split 60%/40%. Four nonblocking 40-byte readbacks provide
GPU times and actual cell counts. Conservative recent costs, 20% headroom and limited upward
steps adjust the ceilings. Feedback is delayed; this is not GPU preemption or a hard time
deadline. Probe, filtering and cell shading are outside that time target. Other frame passes
can still dominate a cut.

Measurement controls: `RENDERLAB_GI_REFILL_TRACE=<csv>` records frame ordinal, claim ceiling,
gather ceiling; `RENDERLAB_GI_REFILL_REPLAY=<csv>` replays that sequence during picture/counter
captures. Those blocking readbacks can lower GPU clocks; replay prevents them from changing
the measured time-targeted workload. Use a fresh single-view process and the same frame
sequence. Invalid, short or reordered replay files fail instead of silently changing budgets.

The retained picture gate remains full-image counts over two/eight display codes at +1,
+5, +20 and +70, plus the first frame below default's +70 over-eight count. Whole-frame
mean error alone did not expose the bounded experiment's dark pillar patches.
