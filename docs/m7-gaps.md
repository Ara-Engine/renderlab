# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## TR1 (temporal reconstruction, default on; TR1b)

TR1 is the default on traced and visibility-buffer lit frames, both paths, from TR1b's overnight pass; `Reconstruction::off()` switches it off ([m7-tr1b.md](m7-tr1b.md)). It is never presented as meeting the Unreal bar.

**Reach.** The RT device's code default is still raster primaries (RT1c/RT1d), which TR1 does not reconstruct: RT users get TR1 only with traced primaries. Temporal soft-shadow frames keep their own film history and are not reconstructed.

**Against no-AA, judged on each path's own 64-spp pixel integral** (Islam's ruling; the visible-sky quarry walk, 40 frames, every row pooled per segment). Before tonight TR1b lost 31 of 256 pooled rows on both paths. With the overnight changes (radius-1 spatial start with a radius-2 fallback, point-symmetric jitter pairs, exact single-layer output at whole coverage, sub-1e-3-px fetch snap) it loses 26 on both paths; the cold first frame now equals no-AA bit for bit (all start-age-0 rows tie). What still loses, by kind (RT; non-RT within a few rows of it):

| Kind | Rows (RT: still start / moving / still end / aged pools) | Example | Cause |
|---|---|---|---|
| Single-pixel maxima | 2 / 12 / 3 / – | single-surface L0 216 vs 108 (moving) | a rock pixel under the sun-bright sky takes 4–5 % sky through a transported coverage estimate; at 10⁴ contrast a 4 % coverage error is 200 codes. Also a lit pixel on a thin dark crease (lit L4 212 vs 81) |
| Signed means of small near-edge populations (lit, faint, silhouette) | 1 / 5 / 5 / – | silhouette L0 0.61 vs 0.22 (moving) | the same sky bleed, as a bias |
| Sky (background-centre, distance bands) | 1 / 1 / 9 / – | 0.009 vs 0.004 codes RMS far from edges (L0) | f16 output rounding of a mean against a centre sample on a smooth glow; paired jitter halved it, pending pairs (rejected: +6 aged rows) brought it to 0.0074 |
| Still-hold step | – / – / 4 / – | 0.66 codes vs 0 (L0) | any accumulation changes still frames while converging; no-AA is exactly constant. Structural |
| Revealed pockets | – / – / – / 37 | revealed ages 0–7 (4, 4, 3, 9, 5, 4, 4, 4 rows) | young history on revealed pixels, mostly next to bright-sky silhouettes |

Non-RT: 8 / 16 / 23 / 37 rows by segment, the same kinds.

Not fixed tonight; each needs either an exact coverage model at very high background contrast or a decision on the step row (no accumulating method can tie a constant image there).

**Against the path-traced reference** (kept for the record; re-scored once G2's cache is the default): TR1b is worse than no-AA in 149 of 256 pooled rows on both paths, almost all from the lighting error both images share in that backlit view (shadowed rock 3,600× brighter than the reference), which pixel integration moves further from a near-black reference.

**Against TSR Epic** (TR1b's settings before tonight; not re-scored with tonight's defaults):
- Q-walk: revealed pixels (age 3, all-covered L0 9.64 vs 7.70), background-centre next to edges (0.419 vs 0.371 at frame 64), edges at L0 by under 1 % (17.98 vs 17.81); +4-stop populations where the converged still image is itself above TSR's.
- U1b's 776-comparison specular gate fails (458–462 failed comparisons): view-dependent shading lags under camera motion; light-track turns restart the history.
- S2G sun 0.25°/frame on non-RT loses (15,476 wins, 26,772 losses): Virtual's shadowed shading under a turning sun.
- Cost: p95/p99 increments over the half-TSR ceilings (TR1's quiet run: RT increment p95 0.815 / p99 1.009 ms vs 0.769 / 0.816); the quiet re-time with TR1b's binary is outstanding.
- No TSR capture on the visible-sky walk from TR1b (a separate unit builds the sky-visible Unreal level).

**Reference instrument.** The visible-sky references were traced by a release-profile test binary; on fixed resources its first renders differ from later ones by ≤2 ULP, which the bounded-AS policy's late fixed-resource control does not see. A dev-profile build of the same source (U0b's profile) reproduces quarry-final byte for byte on 8 independent and 4 fixed replays. The sky references stay accepted under the policy; a dev-profile re-trace was stopped by the overnight no-new-references rule.
