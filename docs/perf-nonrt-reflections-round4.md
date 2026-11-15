# Non-RT reflections, round four: corrected screen march rejected

The low acceptance was a prototype bug. Correcting it produced useful screen hits and saved hit-shading work, but the depth march, fallback and colour-history work still cost more than the software tree alone. The experiment is stopped under the brief's conditional stop rule. No screen-space runtime change is retained.

## Revisions and reproduction

- Branch: `t3/swrefl-4`, pushed. Corrected measured prototype: `638c16379ce4b458ce2cc9caaa2870ab022ada2c`; runtime revert: `f14ecf562d6035516ec6a567efa6599c05dd1f7d`.
- Prototype `0d8fce79` was merged into main `9b41cedd3125d51c249def2a52277cdbbc9ecd93`. Origin/main was fetched and merged before the final run; it remained at that revision.
- Prototype off switch: `RENDERLAB_SW_REFLECTION_SCREEN=0`. Diagnostics: `RENDERLAB_SW_REFLECTION_SCREEN_DEBUG=<absolute JSONL path>`. Without this debug setting, all new diagnostic atomics, receiver checks and fallback-reference classification are omitted from shader generation. Both switches belong to the reverted prototype; existing separate software reflections remain main's default.
- Windows, RTX 4070 SUPER, 1920 x 1080, wgpu Engine through Ara, game's normal quality settings, game closed. Full final smoke used `--exclusive`, default yardstick `bin-smoke-28588cbb`, no `--baseline` and no debug/environment overrides. Candidate binary: `bin-smoke-638c1637`.
- Final invocation: `py -3 D:/Coding/renderlab-work/main-thread/swrefl/fourth/fast-smoke.py D:/Coding/renderlab-work/ara-pin/bin-smoke-638c1637 --owner swrefl --rebuild --exclusive`. The helper only shortens lock polling sleeps; smoke retains ownership of locks and the build queue.
- Full run: `D:/Coding/renderlab-work/ara-integration/smoke/20261115-052630-bin-smoke-638c1637`; summary beside it: `smoke-20261115-052630-bin-smoke-638c1637.txt`. All three scenes and both paths ran with zero errors. GPU lease 351 s, invocation 597 s including queue. Representative source rebuilds took 53–54 s. Implementation wall time was not separately instrumented; these are build and verification times, not an implementation-time estimate.

## Bugs found and corrected

1. A coarse colour-history footprint touching another primitive rejected the entire otherwise valid hit. Select a finer uncontaminated level or a validated nearest history texel instead.
2. HDR alpha's TR1 angular view-dependence hint was treated as confidence with a fixed 1.2 cutoff. Ordinary valid lit surfaces failed it. Remove that cutoff while retaining lighting validity, primitive, depth and facing checks.
3. The first approximate depth overlap immediately fell back if exact triangle refinement missed that sample's barycentrics, minimum ray distance or pixel coverage. A raster sample is not continuous geometry coverage; continue unresolved crossings through the overlap band.
4. The 40-step budget counted hierarchy descents and ascents as ray steps. Count advancing cells instead, with a separate 1024-visit safety budget. This does not increase the spatial stride.
5. At grazing angles the thickness test compared depth at the ray point with depth at a different sample-centre position. Test the actual world-space triangle plane and camera-ray coverage before declaring a thick occluder. Clip the projected endpoint against the actual camera near plane.

The projection uses reversed device depth and the current jittered projection consistently. History is the previous lit HDR image copied after reflection composition and before TR1, with previous jittered matrices and previous depth. Static hit positions are reprojected directly; moving/skinned or relighting-invalid hits fall back to the tree. No motion vector is applied. All history-ID matches in the diagnostic windows passed the history plane-depth test; the remaining depth/normal counter entries were facing rejections. The upstream ray already had a normal epsilon offset. Receiver wrong-side, off-plane and more-than-two-pixel displacement counters were all zero in the separate Showcase receiver check.

No shading-kernel edit was made. The only shared renderer edit was camera wiring, and it was reverted with the prototype. Full original rays remain the fallback for screen exits, unresolved/thick occlusion and invalid history; off-screen geometry is not dropped.

## Rejection counts before and after

Matched steady diagnostic windows: Showcase 40 samples; Glint course 164 samples; Glint goal last 14 samples. Before: `423edd2d`; after: `1b9ca258` (the measured `638c1637` only adds diagnostics to the same default algorithm). Debug runs are not timing evidence. Values below are terminal outcomes; intermediate progress counters are excluded.

| Terminal outcome | Showcase before | Showcase after | Glint goal before | Glint goal after |
|---|---:|---:|---:|---:|
| Attempted | 10,085,502 | 10,085,502 | 3,597,854 | 3,597,854 |
| Frame has no history | 0 | 0 | 0 | 0 |
| Starts off-screen / behind near | 0 | 0 | 0 | 0 |
| Leaves screen | 4,147,116 | 4,465,499 | 20,081 | 25,788 |
| No crossing within budget/range | 1,759,951 | 857,484 | 1,211,556 | 107,349 |
| Thickness / unresolved crossing | 3,394,123 | 3,541,085 | 1,939,341 | 2,881,015 |
| Hit pixel has no valid history | 45,912 | 84,053 | 22,880 | 28,358 |
| Reprojected hit off-screen | 40 | 451 | 148 | 301 |
| History depth / normal mismatch | 3,126 | 23,098 | 8,408 | 19,739 |
| Moving hit | 152,639 | 181,975 | 0 | 0 |
| Relighting zone | 0 | 0 | 0 | 0 |
| Colour footprint off-screen | 7,808 | 0 | 1,252 | 0 |
| Colour footprint primitive mismatch | 403,280 | 0 | 162,808 | 0 |
| View-hint cutoff | 170,845 | 0 | 122,183 | 0 |
| Fog invalid | 0 | 0 | 0 | 0 |
| Tiny projected ray | 0 | 0 | 0 | 0 |
| Accepted screen hit | 662 | 931,857 | 109,197 | 535,304 |

| Window | Accepted before | Accepted after | After, fraction of geometry hits |
|---|---:|---:|---:|
| Showcase | 0.0066% | 9.24% | 28.00% |
| Glint course | 0.9780% | 7.81% | 41.40% |
| Glint goal | 3.0351% | 14.88% | 56.02% |

These are actual all-ray rates, not a claim of over-half acceptance on Showcase. The tree reference classified 6,757,114 of 10,085,502 Showcase rays as sky/misses (67.0%); over-half of all rays cannot reuse a geometry colour on this workload. Glint goal similarly has many sky rays. Geometry-hit denominator = attempts minus the actual fallback tree's sky/miss count. Screen accepted hits do not run that reference tree.

The dominant original failures were unresolved crossings and colour-history rejection, not the ray origin or history depth convention. After fixing them, a longer 87-frame Showcase diagnostic classified 7,396,154 thickness fallbacks: 6,899,447 behind another object and 496,707 against the receiver object. Of visible exact-endpoint fallbacks behind a thickness obstruction, 1,710,806 were another object and 65,127 the receiver. Hierarchy safety exhaustion was zero. This supports actual foreground occlusion as the dominant remaining fallback, rather than a systematic step-one self-hit. It does not certify every remaining miss as optimal.

Full counters, including Glint course and the intermediate progress counters, are in `D:/Coding/renderlab-work/main-thread/swrefl/fourth/matched-rejections.json`. Raw debug JSONL and count scripts are in the same directory.

## Clean full exclusive smoke

Values are default main yardstick -> corrected prototype, frame wall time / FPS. Glint course is smoke's course median; goal is listed separately because it is the expensive target view. These figures describe the rejected prototype, not the reverted branch head.

| Scene | Path / view | Main ms / FPS | Prototype ms / FPS | Reflection GPU ms, main -> prototype |
|---|---|---:|---:|---:|
| Showcase | RT / first-view window | 6.21 / 161 | 6.04 / 165 | 0.1956 -> 0.1946 |
| Outpost | RT / first-view window | 12.50 / 80 | 12.06 / 83 | 0.1495 -> 0.1306 |
| Glint | RT / course median | 7.02 / 142 | 7.91 / 127 | 0.1177 -> 0.1188 |
| Glint | RT / goal | 8.26 / 121 | 9.71 / 103 | 0.2468 -> 0.2478 |
| Showcase | non-RT / first-view window | 7.13 / 140 | 7.96 / 126 | 1.1526 -> 1.7788 |
| Outpost | non-RT / first-view window | 14.07 / 71 | 13.74 / 73 | 0.8935 -> 1.4336 |
| Glint | non-RT / course median | 9.94 / 101 | 10.27 / 97 | 0.7603 -> 1.1612 |
| Glint | non-RT / goal | 13.68 / 73 | 14.50 / 69 | 1.9364 -> 2.5692 |

Smoke reported slower Showcase non-RT, Glint non-RT course and goal, and Glint RT goal. The RT path dispatches no screen-reflection work and its reflection GPU stages remained flat (goal 0.2468 -> 0.2478 ms); the RT goal frame outlier is not attributed to SSR. No follow-up retiming was needed to reject the candidate: non-RT reflection costs regress substantially in every scene independently of the frame outliers. Outpost wall time improves in this pair while its reflection stages regress; this is not evidence that SSR wins there.

Per-stage GPU medians, milliseconds:

| View, non-RT | Trace incl. depth/march, before -> after | Hit shading | Composite incl. history | Total reflection | Target |
|---|---:|---:|---:|---:|---:|
| Showcase | 0.4956 -> 1.0696 | 0.6318 -> 0.5304 | 0.0307 -> 0.1398 | 1.1526 -> 1.7788 | 0.6 |
| Outpost | 0.2893 -> 0.6856 | 0.5837 -> 0.5801 | 0.0087 -> 0.1664 | 0.8935 -> 1.4336 | 0.5 |
| Glint course | 0.4183 -> 0.6620 | 0.3103 -> 0.3226 | 0.0082 -> 0.1147 | 0.7603 -> 1.1612 | 0.5 |
| Glint goal | 1.4070 -> 2.0838 | 0.4905 -> 0.3103 | 0.0297 -> 0.1792 | 1.9364 -> 2.5692 | 1.0 |

Totals are medians of per-frame sums, not sums of independently calculated medians. Showcase/Outpost use 40 steady frames. Glint course pass summary uses 150 steady samples from the first ten non-goal views, while smoke's frame course median covers its course windows; goal uses its own last 15 steady samples. Hit shading saves 0.1014 ms on Showcase and 0.1802 ms at Glint goal. Depth/march/fallback plus colour-history overhead costs more than those savings. The candidate misses every reflection target. This is a result about this corrected implementation; it is not a claim that screen-space reflections in general cannot be fast.

## Inspected images and limits

All paths below are preserved under `D:/Coding/renderlab-work/main-thread/swrefl/fourth/` unless stated otherwise. Pair sheets put main on the left, prototype on the right, using the same native-pixel crop coordinates.

- `rt-full-pairs.png` and `nort-full-pairs.png`: actual final full smoke Showcase and Outpost frame40, and Glint goal, on both paths. Full source images are in the final run directory above (`*-frame40.png`, `glint-{rt,nort}-{base,cand}/goal.live.png`). No obvious new missing glowing-object reflections in these overviews. This is visual inspection, not a blanket quality pass.
- `showcase-motion-pairs.png`: reflective balls in moving frames 20 and 21. Their reflected shapes and edges look close; no obvious edge smear in this inspected pair.
- `glint-motion-pairs.png`: chrome ball's ring reflections and blue platform's reflected orange marks in consecutive moving frames 20 and 21. The rings remain present, but the small orange marks are visibly more speckled/fuzzy in the candidate. The motion-noise gate is not passed.
- `glint-goal-pad-pair.png`: still goal-pad crop. The candidate has a sharper vertical reflected light streak than main; still-image matching is not certified.
- Native moving images: `motion-cut/showcase-{base,cand}/showcase.video/00020.png` and `00021.png`; `motion-cut/glint-{base,cand}/sliders.video/00020.png` and `00021.png`.
- Frame32 in both sheets is labelled **cut requested**. The capture API received a cut request, but the saved Glint frame does not establish that an actual camera cut occurred. Post-cut quality remains unverified; it must not be reported as passed. No further GPU capture was run after the conditional performance stop.

The full software fallback remains in the rejected implementation; off-screen objects are not intentionally removed. Inspected ball/ring reflections remain visible. These fixtures do not isolate every off-screen disocclusion case, and quality is already rejected for the observed noise.

## Disposition

Runtime source at the reverted head matches main 9b41cedd; only this report is retained. Corrected implementation, diagnostics and measured binaries remain recoverable from pushed commit 638c1637 and earlier branch history. No owned Cargo target was created; shared smoke targets are untouched. Own summarized raw BIN dumps larger than 5 MB are removed; PNG evidence is preserved.

Not done: reflection targets, motion-noise acceptance and verified post-cut acceptance.
Next: retain main's software-tree path and close this screen-march attempt under the requested stop rule; no further hillclimb is proposed.
