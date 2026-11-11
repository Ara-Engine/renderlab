# Milestone 8: beat Ara on performance, zero quality loss

Islam, 2026-11-09, after reviewing the ARA1 comparison ([m7-ara1-report.md](m7-ara1-report.md)): "Okay mark this as a version of the renderer if it can be used alone, and let's start on optimizing and beating ara in performance without compromising even a bit in quality".

## Quality bar

Islam clarified the bar on 2026-11-10: "Look all I care about is visual fidelity that looks better than unreal's and solves all unreal's issues like ghosting and all of that, at the same time it has at least 3x the performance or so, all of these numbers are like we are working towards and commit as we go, it does not mean we ignore something that makes us 1.5x faster just because it does not achieve 3x we use 1.5x until we figure out the next one and so on". The rules below replace the 2026-11-09 version, which had required byte identity and equal-or-better scores on every crop. He never asked for those.

- The gate is the visible image, still, moving and on cuts, on both paths, judged against Unreal with its full feature set. The path-traced references guide that judgement; they are not a per-crop pass/fail. A visible regression blocks a change.
- Unreal's visible flaws, such as ghosting, are targets to beat and need explicit test cases.
- Speed targets (about 3x Unreal; the ARA1 targets below) are directions, not floors. Keep any change that is faster and not visibly worse, then keep going.
- Byte identity is still the cheapest proof that nothing changed, so use it when a change is meant to be exact. When it isn't, inspect the images and record ARA1 scores.
- No quality setting is lowered or capped to buy time. The 16-ray glossy budget and the 128-light shadow budget stay unless an alternative looks as good.

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

## Non-RT architecture change (2026-11-11)

Islam, about 10:00 UTC: "use unreal's architecture then, honestly Idc if we use it or no, as long as we can beat their performance by a mile, and beat their quality by a good mile too".

**Why.** The 2026-11-06 design ([non-rt-path.md](non-rt-path.md), decisions 1 and 3) made non-RT trace the same rays as RT in software and rejected shadow maps and distance fields. About thirty rounds then tuned a per-pixel software triangle walk. Unreal reaches about 4 ms on the matched quarry by doing much less work; Renderlab non-RT is 2.1x slower still, 2.6x slower moving and 10.5x slower on a cut.

**What changes.**
- Unreal's non-RT architecture is the starting point. The final image against Unreal decides, still, in motion and after cuts.
- Screen-space steps, shadow maps, distance fields and reuse are all allowed if the image passes.
- Superseded: non-rt-path.md decisions 1 and 3, and the AGENTS.md rule against screen-space techniques for core lighting and shadows.
- Aim: far faster than Unreal (3x stays the direction) and visibly better, with no ghosting and Unreal's failure reel fixed.

**Order.**
1. Finish P-RELIT. Its saving is mostly non-RT.
2. Profile Unreal pass by pass: software Lumen at Epic and VSM on the matched quarry, still, moving and cut, from the p-yard UE package. The result is a per-pass budget table next to Renderlab's, so each later unit has a target pass and a number.
3. Non-RT sun shadows: cached virtual shadow maps as the default, building on [design-nonrt-sun-shadows.md](design-nonrt-sun-shadows.md), with geometry rescue only where maps would leak or lose contact. Target: Unreal's VSM, 0.50 ms moving on the quarry, or better. This replaces S4 and the march on still, moving and cut frames.
4. Non-RT GI and glossy, Lumen-class: a surface cache or cards traced at low rate, screen traces with a world fallback, rough glossy read from the radiance cache, traced rays only for smooth reflections. Resampled reuse is allowed. P-GLOSSY folds into this and does not restart in its old form.
5. Cuts: mostly expected to fall out of persistent caches (Unreal's cut costs nothing extra). Fix what is left after 3 and 4.

**Rules for the track.**
- Non-RT is the only Renderlab feature track until it beats software Lumen at Epic on the quarry (still, moving and cut) and on Showcase. RT gets fixes only.
- It gets the PC GPU first; Ara captures run between its leases.
- Each unit is gated on side-by-side images against Unreal from the failure reel, plus timing. Keep every faster change that is not visibly worse.
- Unreal's source may be studied for the method, but no Epic code goes into this repository. Godot, AMD Capsaicin, Bevy and O3DE code may be copied with attribution after checking each licence; record what was taken in THIRD_PARTY_NOTICES.md.
- The non-RT path stays first-class and shading stays programmable.
