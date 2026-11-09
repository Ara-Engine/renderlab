# Milestone 8: beat Ara on performance, zero quality loss

Islam, 2026-11-09, after reviewing the ARA1 comparison ([m7-ara1-report.md](m7-ara1-report.md)): "Okay mark this as a version of the renderer if it can be used alone, and let's start on optimizing and beating ara in performance without compromising even a bit in quality".

## Quality bar

These rules are hard:

- Every measured aspect keeps its current image quality against the path-traced references, on both paths: still, moving and on cuts.
- A pure optimisation must be byte-identical.
- Any other change must be equal or better on every crop and population in the ARA1 harness and the existing fixtures.
- No quality setting is lowered or capped to buy time. The 16-ray glossy budget and the 128-light shadow budget stay unless an alternative matches them.
- An idea that needs a quality trade is recorded and dropped.

## Targets

From ARA1: matched scenes at 1080p, RTX 4070 SUPER, Renderlab ae85750 against Ara main 1d2d647.

| Measure | Today | Target |
|---|---|---|
| Non-RT frame | 2.2-5.8x Ara raster (game 10.8 vs 1.9 ms) | below Ara raster |
| RT frame, small scenes | 50-79% slower than Ara hybrid | below Ara hybrid |
| Moving camera | rises over still (RT game 4.0 to 6.0 ms) | flat, like Ara |
| First cut | 12-31 ms | Ara's 2-4 ms |
| 64 lights | 12.0 ms RT, 27.9 ms non-RT | at most about 3.5 ms |
| GPU memory | about 2x Ara | at or under Ara |
| First new shader edit | 23-24.5 s | about 1-2 s |

Beating Ara is the first step. The standing goal is still Unreal's quality at 2x its speed, with film quality as the aim.

## Method

- Profile first with Nsight GPU Trace, then attack the largest measured cost on each path.
- Study how Ara, Unreal and others get their speed. Adopt an approach only where it keeps our image. Ara stays read-only, and no Epic code is copied.
- The ARA1 harness (`tools/ara1/`, [m7-ara1-plan.md](m7-ara1-plan.md)) is the scoreboard. Quality and time are measured in one run, in short leases.
- Unchanged rules: non-RT is first-class, shading stays programmable, the patent policy applies, and bulk output goes on D:.

## First units (2026-11-09)

| Unit | Area |
|---|---|
| P-RTS | RT sun-shadow rays: first-cut spike (256 cold rays), moving-camera cost, small-scene RT |
| P-NRT | Non-RT frame excluding GI and lights: visibility buffer, virtual/surface sun shadows, fog/atmosphere, TR1, R1n, scheduling |
| P-GI | GI cache cost on both paths: shading lookup, cut spikes, memory |
| P-LTS | 64 local lights: light loop, clustering, local shadow maps and atlas, RT local shadows |
| P-SHC | First-new-shader hitch and cold startup |

GPU memory gets its own unit once the profiles show where it goes.
