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

## Non-RT goal (2026-11-11)

Islam, about 10:00 UTC: "use unreal's architecture then, honestly Idc if we use it or no, as long as we can beat their performance by a mile, and beat their quality by a good mile too". Then, correcting how that was passed on: "stop treating what I say as rules, I need the fast path that generates the best possible results idc what you do architecture wise".

**Goal.** A non-RT path much faster than Unreal and visibly better, still, in motion and after cuts. Nothing else here is a rule from him.

**What no longer binds.** The 2026-11-06 design ([non-rt-path.md](non-rt-path.md), decisions 1 and 3: the same rays as RT traced in software; shadow maps and distance fields rejected) and the AGENTS.md rule against screen-space techniques for core lighting and shadows. The architecture is our engineering call, pass by pass: Unreal's design, our own, or a mix.

**One legal limit.** No Epic code or text in this repository. Code from other engines needs a licence check and attribution in THIRD_PARTY_NOTICES.md, as for any third-party code.

**Why the old design is set aside.** About thirty rounds tuned a per-pixel software triangle walk. Renderlab non-RT is still 2.1x slower than UE Epic on the matched quarry when still, 2.6x slower moving and 10.5x slower on a cut, while Unreal reaches about 4 ms by doing much less work.

**Current plan (root's and the coordinator's judgement, open to change).**
1. Land P-RELIT.
2. Fix the RADV `arrayLength` defect (m7-gaps.md).
3. Profile Unreal pass by pass on the matched quarry (software Lumen at Epic and VSM; still, moving and cut) into a per-pass budget table next to Renderlab's. Knowing where Unreal's 4 ms goes is the cheapest way to choose well.
4. Then choose an approach for each of sun shadows, GI, glossy and cuts, on that table and on what existing work (virtual shadow maps, the sun-visibility surface cache, the world radiance cache, resampled reuse) already does better.

The non-RT gap to Unreal is the largest problem, so most of the PC's GPU time goes there. RT work is not frozen; it is done when it is the best use of a unit.
