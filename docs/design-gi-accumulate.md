# Accumulating cache cells (gicut, 2026-11-17; in progress, `RENDERLAB_GI_ACCUMULATE=1`, default off)

## Why

Two faults with one cause (main thread, 17 Nov):

- Outpost's hangar is blotchy on both paths where its path-traced reference is smooth. Turning GI
  off clears it; sampling the glowing strips as lights does not. A cell's value is the sum of one
  fixed set of 64 directions, and a small bright source (the doorway: sky and the sunlit ground
  outside) is met by some cells' directions and missed by their neighbours': a fixed mottle.
- Non-RT walking spends 21 ms of a 28-29 ms frame in the cache (Glint and Outpost): every change
  re-gathers a cell's 64 rays at once, and a cap on that (the bounded refill) only queues it.

World-space radiance caches with constant cost (Lumen's radiance cache, DDGI) have no refill events:
a fixed number of cell updates a frame, each with a small rotated set of rays, blended into the
stored value with a hysteresis that drops when the new sample disagrees with the history.

## The cell

- **One stored quantity:** the cell's incident radiance, sky and bounce together, absolute, as an
  L2 SH in the words the sky used (scale and 27 halves). Readers are unchanged except that
  `gi_sky_scale` passes the value through.
- **Statistics** in the bounce's words, whose scale word stays 0 so no reader adds a bounce: a fast
  and a slow running mean and the slow second moment of an update's L0 luminance, and the updates
  blended.
- The local lights' per-light visibility bits stay: they are exact, not noisy.
- A new cell's first value is finish's 64-direction sky, made absolute; its first update traces all
  64 lanes.

## The update (shaders/gi_accumulate.wgsl)

K rays (16 by default) stratified over the hemisphere's solid angle in a rotation drawn per update.
A miss adds the sky's radiance, a hit the hit cell's outgoing radiance (which carries that cell's
own incident light, so bounces chain). The update's L2 SH blends into the history with weight
1 / updates (down to 1 / 32). When the fast mean (weight 1/2) leaves the slow mean's band (twice its
standard deviation, plus 2%), the history is pulled to the update (weight 1/2) and its count
restarts. That test stands for every change: lights, sky, movers, cuts; there are no reset rules.

## Schedule (first slice)

Cells without history every frame, visible cells every 4 frames, others every 64, at a phase their
slot fixes. Scan, integrate, the gathers and the cell filter do not run. The order by priority and a
budget in rays derived from the cost target come after the picture holds.

## Acceptance: cost while moving

Target: the cache's cost rolling stays within about 1.5x of standing, on both paths, at both sizes.
The cache's cost is gi_gather + gi_update + gi_integrate, in GPU ms a frame. Today's cache, on
Glint's play loop (sunrt, bin-smoke-d64a3d6a, `ara-player --game`, the bot rolling the course, vsync,
high quality, one exclusive lease, 15 s a row; the ball at a median 9.5 m/s):

| path | size | standing | rolling | rolling / standing |
|---|---|---:|---:|---:|
| non-RT | 2560x1440 | 5.95 | 42.90 | 7.2x |
| non-RT | 1920x1080 | 4.30 | 40.26 | 9.4x |
| RT | 2560x1440 | 2.13 | 7.96 | 3.7x |
| RT | 1920x1080 | 1.69 | 5.81 | 3.4x |

Today's cost grows with how far the view moves between frames. Accumulation spends a constant ray
budget instead. Re-measure a candidate with `u-sunrt/roll/breakdown.py <bin> <out> SECS=15
SIZES=2,1 ROUTES=stand,course PATHS=nort,rt ONE_LEASE=1` (per-row data of the table:
`u-sunrt/roll/fig2/`).

## Results

(to fill: hangar pair, walk cost against still, after a cut, movers, a lighting change)
