# M7-LC3: many local shadows

Overnight unit, 2026-11-09, branch `t3/m7-lc3`, based on main `622db4d`. Extends [LC1](m7-lc1.md) and [LC2](m7-lc2.md). Verification is in progress; the evidence and timings below are filled after the retained leased batch.

## Approach

- **RT: every contributing clustered local light gets visibility.** Remove the camera-selected pool from ray heads. Keep LC2's four receiver-facing disk rays for positive `source_radius`, one centre ray at zero, and TR1's existing rotating sequence. No stochastic light omission, new reservoir or new reconstruction algorithm. This spends rays on actual contributing cluster lights and avoids the extra motion noise of sampling only some lights. `count = 0` still disables automatic local shadows; any positive count enables all-light RT visibility.
- **Map heads: 128 lights by default, maximum 128.** One six-layer Depth32Float atlas, with the same rectangle on each cube face. At the default 1024 maximum face resolution it is 4096 x 4096 x 6, **384 MiB** of depth, independent of light count. LC2's four full cube slots took 96 MiB; LC3 trades 4x depth memory for up to 32x as many shadowed lights. Spots occupy one face of their rectangle; five reserved face regions are wasted, a remaining packing opportunity.
- **Coverage and allocation.** Rank frustum-intersecting lights by camera illuminance times clipped projected influence-sphere area, with the existing 25% hysteresis. Pick a 1024/512/256 requested face size from projected influence; fall back by halves to 128 at the default maximum. Reserve minimum-size space for remaining slots. Held tiles never move or change resolution with camera motion. The influence sphere is conservative; it does not measure visible receiver pixels or occlusion, and narrow spot cones are not tightly fitted.
- **Transitions.** Keep an outgoing slot and tile while its shadow fades by 1/8 per admitted frame. Reassign only at zero, then fade the incoming shadow up by 1/8. A full replacement takes up to sixteen frames. Initial maps also fade in. Explicitly disabling shadows, shrinking the configured budget, or removing/reindexing lights is not a smooth transition contract. Light identity remains its position in the supplied light vector.
- **Cache.** Each slot hashes its light placement/cone, near plane, tile and intersecting casters. Each caster hashes mesh/material identity, skin revision and model transform. A distant caster change does not invalidate unrelated lights. A light or intersecting caster moving still redraws that light's complete regular-geometry map; this is not virtual page caching or a static/dynamic-caster split. Radius and intensity changes do not redraw depth.
- **Filtering.** Retain LC2's blocker search and variable PCF for positive radii. Radius-zero 3x3 bilinear PCF now also reprojects taps over cube edges. All loads clamp inside the assigned tile; cube-face reprojection uses the neighbouring face of the same light. The 96-texel PCSS support cap remains.

The existing cluster is bounded to 128 light indices per receiver; ray work there is at most 512 visibility rays with positive radii. Receivers outside the cluster grid and unprepared stages still walk all lights, as before. Direct-light evaluation and material incidence remain programmable. No new storage binding, material hook, GI layout, sun-shadow change or reflection implementation is introduced.

The group-3 frame uniform grows to **51,312 bytes** (768 matrices plus 128 tile/fade records). The maximum map count follows that uniform's 64 KiB structural bound. Devices must expose that binding size; renderer construction now reports an explicit ABI error below it. `LocalShadows::resolution` is the maximum tile face size and must be <= one quarter of the device's 2D texture limit. Stats expose tile resolutions, strengths, atlas bytes and redraw counts.

## Candidate decision and patent design-around

The retained candidates are exhaustive clustered RT visibility versus the four-light limit, and the cached tiled atlas versus the four-light map limit. Their paired measurements are recorded below. Stochastic light selection is deferred unless exhaustive visibility's measured cost requires it; it would add a second temporal-noise source beyond LC2's emitter sampling. Per-pixel irradiance reservoirs are not required to remove the direct-light pool limit.

Following the engineering mapping in [P0](m7-p0-claims.md), LC3 adds no reservoir/RIS (US 11,315,310 claim 1's resampling element is absent), no accumulated path-contribution hash table or sample counter (US 10,614,613 claim 1(e)-(g)), and no ranked irradiance-cache ray updates (US 11,790,594 claim 1). Its cached data is raster depth keyed by light/caster changes. It adds neither variance clipping nor any denoiser, so the flagged temporal/SVGF combinations are not extended. Existing TR1 and GI are unchanged. This records the design choices against the known flags, rather than asserting general patent clearance.

## Verification and timing

All executables run through `scripts/n8-run.py`, owner `m7-lc3`, leases <=15 minutes, after rereading the latest anchored `[root order]` and its release dependencies. The LC3 queue stops for a changed directive and checks it again before each batch step/path. Tests run only in those leases. `scripts/m7-lc3-run.py` builds with two Cargo jobs into `D:/Coding/renderlab-work/lc3/target` and stages binaries beside it. No new Unreal or reference campaign.

`runs/lc3-c`, lease `lc3-c-1`: **186 lib tests passed, zero failed**, including every shading head's naga validation, cluster coverage, screen-weighted selection, disjoint atlas allocation and eight-frame fade/reuse boundaries. The point/spot soft-shadow fixture and the 64-light leakage fixture both passed on traced RT and non-RT VB. The test executable's SHA-256 is `3345de09511c029de812f1b3d50b91f8103b1d37c099c41b7c3aa88c508bb775`.

| Visibility-only fixture | Traced RT | Non-RT VB |
|---|---:|---:|
| LC2-limit shadowed lights | 4 | 4 |
| LC2-limit measured leaking lights in umbra | 60 | 60 |
| LC3 shadowed lights | 64 | 64 |
| LC3 measured leaking lights in umbra | **0** | **0** |
| Point hard/soft changed pixels (>0.03 HDR) | 10,634 | 16,669 |
| Spot hard/soft changed pixels (>0.03 HDR) | 10,586 | 11,269 |

Inspected `D:/Coding/renderlab-work/lc3/runs/lc3-c/fixture-inspected-sheet.png` and the full non-RT point-soft image. Both point and spot penumbrae remain soft, the blocker silhouette remains sharp, and the 64-light diagnostic has a coherent dark umbra where the four-light control is nearly fully lit. The map filter retains LC2's boxy/banded edge, while rays retain fine grain. Individual hard/soft and leakage PNGs are beside each fixture JSON.

The first lib attempt (`lc3-b-1`) passed 185 tests and failed an old exact-string assertion for the pre-fade map wrapper; the assertion was updated, and the complete lib suite above passes. The combined `lc3-c-1` batch was stopped safely by the 2 GiB available-memory guard during quarry upload, after its lib and fixture steps completed. Those completed checks are retained; quarry and example checks are retried separately with >=6 GiB available at startup. No guard was lowered.

The quarry test retains the LC1/LC2 261-light night scene, registered full-detail quarry and software near field on non-RT. One process loads each path's asset once. The test-only LC2 control reinstates the old camera-illuminance ranking and four-light ray/map limit. A short camera walk, moving authored bright lights, hard/soft controls and the existing day/dusk full images are captured. Timing uses eight drained submission-to-idle samples after four warmups, including CPU submission and GPU execution, rather than isolated pass times. No quiet-window performance claim is made.

The 64-light fixture uses coincident sources and a known fully occluded receiver point. A visibility-only program sums each source's visibility. The old four-light limit predicts 60 leaking lights; LC3 must report zero there on both paths. This is a geometry-proven leakage count. Quarry statistics separately report the number of local lights without any direct-shadow coverage; they do not prove every such light visibly leaks in that camera image.

Small edits outside the light/local-shadow files: `src/renderer.rs` checks the larger uniform ABI; `examples/game_scene.rs` adds 24 warm local lights and `--night-lanterns` to expose them under dim sky/moon lighting. The existing sun rendering, GI internals, reflection and transparency files are untouched. The example still/moving runs exercise the engine API and moving regular casters.

## Remaining gaps

Non-RT still has a finite 128-light pool, lower-resolution tail maps, full mesh redraws for changed lights, no virtual-geometry local casters, the existing near-plane omission and capped PCSS blocker support. Atlas allocation has no repack/compaction or resolution promotion for held lights. Dense overlapping lights remain costly on both paths. Reflection-hit local visibility is still R1's separate limitation. No Unreal-quality, 2x-performance, reference-motion or transparency acceptance is certified by this unit.
