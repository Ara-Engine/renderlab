# Ordered GI refill

Enable with `RENDERLAB_GI_BOUNDED_REFILL=1`. It is default off and selects an ordered,
hard 12,288-cell ceiling for both creation and gathering on the software path. Hardware
RT is unchanged. This replaces #108's 3,000-cell/hash-admission experiment under the
same switch; no second enable switch or budget parameter is required. The optional
`RENDERLAB_GI_REFILL_BUDGET` is a measurement control, not another enable switch.

The measured 12k candidate below used the earlier experiment spelling
`RENDERLAB_GI_REFILL_PRIORITY=1;RENDERLAB_GI_REFILL_BUDGET=12288`, with the old bounded
mode disabled. The single-switch implementation selects exactly that policy and module.

The production switch is default off. This follows the bounded-refill experiment;
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

Claims run all visible rounds before background rounds. Candidate tickets before bucket
walks bound actual new cells; resetting from the created count accounts for round retries.
New claims have no bounce of their own. Gathers run
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
each round. Denied claims skip bucket walks. The variant with a global compare/exchange
retry loop was rejected: its first fixed-budget timing rows were much slower than default.
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

## Rejected admission variant

RTX 4070 SUPER, 1080p forest turn, 400 held frames, one interleaved visit, drained GPU
service times from a clean timing lease. Binary `2896ecbb` used the global CAS ticket and
checked claim tickets after bucket selection. These are rejection evidence, not the final
two-round matrix for the atomic-add replacement.

| Variant | Worst frame ms | p95 frame ms | Worst gather ms |
|---|---:|---:|---:|
| Default | 121.52 | 56.63 | 29.18 |
| Priority / nominal current budget | 120.89 | 108.34 | 85.70 |
| Priority / 8,192 | 133.74 | 99.32 | 56.04 |
| Priority / 12,288 | 219.25 | 187.72 | 161.62 |

The CAS retry loop is a plausible contention source; the replacement also moves claim
admission before bucket walks, so any combined improvement must not be attributed solely
to one of those changes. The initial time-targeted row was faster but shrank to roughly
64 claims and 250 gathers; that does not establish an acceptable picture.

## Second-arrival fixture and compact selection

`RENDERLAB_SCENE_EVENT_FRAMES=400,460,520` alternates the forest turn. Frames 520?559
are a warm revisit, so they must not rank refill policies. Retain 400?439 separately as
true first-arrival evidence with pipeline creation (including atmosphere). Timing runs
drain submissions and capture no PNGs.

Priority selection screens the incoming queue once in a small compute entry, then chooses
visible/zero-bounce, visible/other and background counts. A small deferral entry rebuilds
the waiting queue; the ray-heavy entry visits only admitted cells. Per-slot claim scratch
deduplicates incoming entries. Three capped candidate lists reuse the existing ray-item
arena before sky setup, adding no persistent GPU allocation. Budgets too large for that
arena retain the previous three-scan implementation. Individual gather rays, reduction
order and cell writes are shared verbatim by both implementations.

The time controller now floors both ceilings at 3,000 cells, lowers each by at most 25%
per feedback sample, and raises it by at most 12.5%. A target below the floor's cost can
be missed; the floor protects progress and must be judged in pictures.

`RENDERLAB_GI_REFILL_BUDGET=hold` enables parent stand-ins and defers all bounce gathers
for `RENDERLAB_GI_REFILL_HOLD` frames after a cut (default eight). Claims and sky work
continue; startup without a cut is unchanged. Waiting cells retain their previous values,
or use the existing parent fallback when available. This does not guarantee a filled or
accurate picture: the +1/+5/+20/+70 comparisons decide that.

The 520 revisit is warm-cache evidence only and must not rank refill policies. The
120-frame eviction age makes slots reusable on a later insertion; it does not delete
entries, and idle frames stop advancing cache age. An early B visit followed by an A
hold therefore cannot certify a cold B cache by elapsed frame count alone.

The cold-cache/pipeline-warm fixture visits B at 0?20, switches to A at 21, explicitly
empties only GI there with `RENDERLAB_SCENE_GI_RESET_FRAMES=21`, holds A through 441,
then measures B at 442. `RENDERLAB_SCENE_EVENT_FRAMES=0,21,442` selects those views.
This uses the existing reset API, does not recreate pipelines, and performs no reset at
the measured cut. A no-reset control verifies the residency issue; created-cell counts
at +0?+5 are compared with a true first arrival at 442 before ranking any policy.

On the true first-arrival portion of the compacted experiment (`7d42d462`, RTX 4070
SUPER, 1080p forest turn, two clean interleaved visits), the adaptive 3,000-cell-floor
row's maximum complete gather block was 2.078 / 1.880 ms. The previous three-scan
variant's block cost 6.7?7.7 ms even after its ceiling fell to 64 cells. These workloads
have different admission ceilings; this demonstrates removal of the old screening
floor, not a certified whole-frame or picture win.

Coldness control (`918c38fd`, ordinary diagnostic lease, non-RT 1080p forest):

| B arrival at 442 | Created cells at +0 through +5 | Maximum late atmosphere block |
|---|---:|---:|
| True first arrival | 391,846 | 76.520 ms |
| Early B, retain GI across A hold | 53,622 | 0.295 ms |
| Early B, reset only GI at A entry | 391,847 | 0.277 ms |

The reset fixture matches true first-arrival creation within one cell. All three report
zero new engine pipeline objects at the measured cut; the atmosphere timings expose a
first-use cost beyond that counter. Blocking counters make these diagnostic timings,
not policy performance rankings. The corrected timing matrix uses the reset fixture.
Excess GPU time is measured against the paired default's steady B median over explicitly
bounded 40- and 140-frame windows; an active tail beyond the window is censored rather
than presented as a complete settled total.
