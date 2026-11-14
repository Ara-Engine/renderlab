# Glint GI settles differently between runs

Read/measure audit, 14 November 2026. No GI cache files changed. Follow-up belongs
to U-GIPASS through root; this is not a demonstrated insertion race or a fix.

## Picture size and priority

At Glint's held goal, 100 frames / 99 consecutive pairs give median pixels with
max RGB change strictly over 8: RT GI-on 17 / GI-ablated 18; non-RT 16 / 17.
All four rows have zero such changes inside the 28,688-pixel hand-drawn flat-face
mask. No GI-specific on-screen flicker was reproduced in this fixture.

Two natural GI-on runs, matched by held frame index, settle differently:

| Path | Median pixels over 8, across 100 equal-frame comparisons | Maximum RGB code difference |
| --- | ---: | ---: |
| RT | 120 | 49 |
| non-RT | 79 | 83 |

These are small settled differences, not temporal instability of that size.
The three hand-drawn interior rectangles have zero over-8 pixels in the first
and last equal-frame comparisons; they do not cover all visible speckle.

## What differs first

Two separate RT diagnostics use the existing blocking `RENDERLAB_GI_LOG`.
Both contain 1,930 logged frames, matching the pass-log frame count. Engine
frames below are one-based (counter log index + 1), and occur during the
initial view's hold, before the goal. Readback may change scheduling; these
diagnostics are separate from the natural picture rows.

| First differing engine frame | Counter | Run A | Run B |
| --- | --- | ---: | ---: |
| 5 | gather-generated request entries | 19,719 | 19,718 |
| 5 | total listed misses | 24,768 | 24,767 |
| 21 | actual gathered cells | 22,554 | 22,555 |
| 21 | actual gather rays | 645,913 | 645,923 |
| 192 | created cells / cell traces | 1,698 | 1,697 |

At frame 5, created cells, traces, gathers and gather rays are still identical.
The configured budgets are identical (64 directions, three gather iterations,
default gather-cell limit, time-based gather budget disabled). Actual work later
differs; this is not evidence that the configured ray budget changed.
`claim_deferred`, request overflow and local overflow are zero in both runs.

The earliest raw-list difference can be benign: `gi_request_list` uses
`atomicMax` on inverted key B. Each intermediate winner appends a miss;
`gi_miss_chosen` later accepts only the final winner. A collision processed in
ascending rank can list more intermediate winners than the opposite order,
while selecting the same final cell. Therefore differing request counts alone
do not identify different selected keys or an incorrect claim.

Candidate for a later investigation: make the request list deterministic before
the budget selects work. No record-level comparison was taken, so the audit
does not establish whether the first semantic difference is a selected cell,
canonical point/normal, a closest-hit tie, gathered values or a later filter.
No one-line cause was demonstrated; stop here rather than editing the cache.

## Control rule

Until this is fixed, every RT picture gate on Glint's goal uses a **same-state
control**. Separate fresh runs are not a byte-identity oracle for that view.
Record the run's cache history and use an explicit same-state comparison; do not
attribute this measured run-to-run envelope to a candidate feature. Non-RT also
has a smaller measured settled difference and should retain that caveat.

## Identity and evidence

Every row manifest includes these exact identities:

- Renderlab `f484d8153fe33af9764f6ab43a8ca6551f827a3a`, Ara `eb8cc4570db362ad643130ba5ee76ba875129154`.
- Scene `D:/Coding/renderlab-work/u-sunrt/gi-speckle/tree/samples/glint/content/levels/glint.scene`.
- Scene SHA-256 `1cc06fd0aa857c303189c7c0982b8b160deff01a9c9ddb2ad5e6712732abb2f6`.
- Player SHA-256 `935241a9c1a96b51e46e1e4ce79939c15fce5e5738a86081e723600a6cd6c8b7`.
- Replay SHA-256 `9ad874ff29df17609606f4bdfa5cbf14e0587e3d90ebb3b4b7c543f772afad44`.

Original replay input, seed and every preceding 100-frame capture hold are
preserved, including the original goal hold, before adding the 100-frame video.
LVIS is explicitly 0. Expected finish result is validated in every player.

- Natural four rows: `D:/Coding/renderlab-work/u-sunrt/gi-speckle/runs/u-sunrt-gi-held-history-1114-025632/held-summary.json`.
- Repeats/counters: `D:/Coding/renderlab-work/u-sunrt/gi-speckle/runs/u-sunrt-gi-repeat-counts-1114-030257/followup-summary.json`.
- Crop pairs: each natural-repeat path directory, `*-repeat-pair.png`.
- Ordinary leases released in 207.84 and 222.20 seconds, exit 0. No timing claim.
- The earlier goal-only lease `u-sunrt-gi-held-1114-023849` is excluded; images remain.

All PNGs, crops and sheets are retained. No more GI probes are queued.
