# M7-LC3: many local shadows

Overnight unit, 2026-11-09, branch `t3/m7-lc3`, based on main `622db4d`. Extends [LC1](m7-lc1.md) and [LC2](m7-lc2.md). The retained variant passed lib tests, both paths' quarry and lantern checks, still and in motion. Remaining quality and cost gaps are recorded below.

## Approach

- **RT: every contributing clustered local light gets visibility.** Remove the camera-selected pool from ray heads. Keep LC2's four receiver-facing disk rays for positive `source_radius`, one centre ray at zero, and TR1's existing rotating sequence. No stochastic light omission, new reservoir or new reconstruction algorithm. This spends rays on actual contributing cluster lights and avoids the extra motion noise of sampling only some lights. `count = 0` still disables automatic local shadows; any positive count enables all-light RT visibility.
- **Exact influence rejection.** A conservative cluster includes false positives outside a receiver's exact light range/cone. `light_visibility` now returns lit without querying rays/maps when the supplied sample's attenuation is exactly zero. The lighting contribution is already zero there. This assumes no BRDF or receiver hemisphere and adds no material-model restriction. A program needing a visibility query must supply a contributing sample; this follows the existing cluster contract, which can omit lights outside their influence altogether.
- **Map heads: 128 lights by default, maximum 128.** One six-layer Depth32Float atlas, with the same rectangle on each cube face. At the default 1024 maximum face resolution it is 4096 x 4096 x 6, **384 MiB** of depth, independent of light count. LC2's four full cube slots took 96 MiB; LC3 trades 4x depth memory for up to 32x as many shadowed lights. Spots occupy one face of their rectangle; five reserved face regions are wasted, a remaining packing opportunity.
- **Coverage and allocation.** Rank frustum-intersecting lights by camera illuminance times clipped projected influence-sphere area, with the existing 25% hysteresis. Pick a 1024/512/256 requested face size from projected influence; fall back by halves to 128 at the default maximum. Reserve minimum-size space for remaining slots. Held tiles never move or change resolution with camera motion. The influence sphere is conservative; it does not measure visible receiver pixels or occlusion, and narrow spot cones are not tightly fitted.
  Non-power-of-two maxima quantize tiles onto a shared aligned grid, so thin stranded strips cannot invalidate that reservation. Very small face settings use at least a 16-square atlas. Missing tiles return lit rather than dividing by a zero extent. CPU allocation checks cover maxima 1, 2, 7, 768, 1001 and 1024.
- **Transitions.** Keep an outgoing slot and tile while its shadow fades by 1/8 per admitted frame. Reassign only at zero, then fade the incoming shadow up by 1/8. A full replacement takes up to sixteen frames. Initial maps also fade in. Explicitly disabling shadows, shrinking the configured budget, or removing/reindexing lights is not a smooth transition contract. Light identity remains its position in the supplied light vector.
- **Cache.** Each slot hashes its light placement/cone, near plane, tile and intersecting casters. Each caster hashes mesh/material identity, skin revision and model transform. A distant caster change does not invalidate unrelated lights. A light or intersecting caster moving still redraws that light's complete regular-geometry map; this is not virtual page caching or a static/dynamic-caster split. Radius and intensity changes do not redraw depth.
  Each face then rejects caster bounds outside its own clip volume using the existing M2 conservative rounding envelope. Geometry resolution and shadow distance stay unchanged; empty faces still clear their tile so moving lights cannot retain stale depth.
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

### First all-light cost screen

`runs/lc3-e`, lease `lc3-e-123`, test binary SHA-256 above, code checkpoint `1324c4a`: both quarry paths and both still/moving lantern examples passed. All 13,824 GPU cluster lists matched the CPU reference, with zero overflow (longest 99). Cluster bypass with GI/TR1 off changed zero image channels. RT enabled visibility for all 260 locals; maps selected 128, leaving 132 without automatic direct shadows, versus 256 with the four-light control. Map selection's strongest slots were `[257,258,260,259]`, replacing LC2's `[257,258,196,187]`; all four bright authored lights now rank first. Tile resolutions were 14 x 1024, 4 x 256 and 110 x 128. Settled static frames redrew zero faces.

AMD Ryzen 7 7800X3D (8 cores/16 threads), 32 GB installed RAM (31.12 GiB visible), RTX 4070 SUPER, driver 617.14, Vulkan, release build, 1920x1080 quarry, eight drained frames after four warmups. Default GI/TR1, 8 cm local emitters. This run was contended: up to two other rustc processes; desktop GPU activity was not excluded. Available memory started at 6465 MiB and bottomed at 2152 MiB. All six steps completed in 158.90 seconds; no guard failure.

| Initial unfiltered candidate, ms median | Traced RT | Non-RT VB |
|---|---:|---:|
| Four-light legacy-ranking control | 7.06 | 16.52 |
| LC3 still | 33.56 | 32.79 |
| LC3 camera moving | 32.57 | 33.48 |
| LC3 four bright lights moving | 36.83 | 73.51 |

This is the rejected cost screen, not the final timing claim. Inspection found unnecessary visibility queries for exact zero-attenuation cluster candidates; the retained variant adds that exact rejection. The map redraw cost with moving lights remains a separate material miss, rather than a reason to lower image quality.

Inspected `D:/Coding/renderlab-work/lc3/runs/lc3-e/quarry-inspected-sheet.png`, full `quarry-traced/traced-night-present.png` and `quarry-nonrt/nonrt-night-present.png`, and `lantern-inspected-sheet.png`. The additional RT shadows darken occluded rock cavities and contact regions without breaking the quarry silhouette. The maps retain more light than RT in the known cavity differences, but additional contacts remain coherent. Lantern pools and blocker silhouettes stay coherent during the camera/object motion. The non-RT glossy spheres show a patchy response, also present in the four-map control below; its cause remains unclassified. The stepped bloom halo around the quarry is present in LC2's inspected baseline too. The 0.45 m quarry walk kept the same selected pool, so it does not by itself verify slot replacement; the retained non-RT fixture forces a replacement and checks the fade on the GPU. Raw dumps larger than 5 MB were summarized and deleted, except the two GI-off buffers retained briefly for the final exact-rejection comparison. Quarry reports retain the harness's `lc2-<path>.json` names.

### Retained variant

`runs/lc3-h`, lease `lc3-h-1`, test SHA-256 `aa88eb6ccbb448ea0f8d06dd8d499c8611fbe2641ba6f7ea1c063fa31cb9b2ae`: **186 lib tests passed, zero failed**, including the nonaligned/small atlas settings. Both point/spot and 64-light fixtures passed with the same leakage counts as above. The forced non-RT slot replacement measured receiver visibility `[0,0.125,0.25,0.375,0.5,0.625,0.75,0.875,1]`, maximum step **0.125**, rather than a one-frame jump. Its complete fade PNGs and JSON are in `fixture-nonrt/`. The CPU boundary test also checks the incoming eight-step fade.

Both quarry paths passed on the retained exact-influence and per-face-culling variant. **All 28 HDR captures match the first candidate byte for byte**, including the camera walk, moving-light endpoint, hard/soft, GI-off and day/dusk controls. This is one paired image comparison, not a repeated determinism/reference campaign. Full image hashes are retained in `D:/Coding/renderlab-work/lc3/exact-rejection-hashes.json`; the compared raw buffers were then deleted. Cluster lists and cluster-bypass results remain exact, and shadow coverage and the default tile distribution are unchanged.

Same hardware, resolution, quality and timing method as the first screen. Still and moving-light timings use eight samples after four timing warmups; the camera walk samples its last eight of sixteen 3 cm camera steps. The four authored bright locals move in the moving-light row, and all four have automatic visibility in LC3. These drained CPU+GPU timings include lighting/cache updates and are not isolated shadow-pass times. The retained quarry lease started with 11,504 MiB available, bottomed at 6,859 MiB and observed one competing rustc; the final example lease started at 8,881 MiB, bottomed at 8,363 MiB and observed two. Desktop GPU activity was not excluded. Contention differs between leases; no quiet-window, Unreal or engine-wide speed claim is made.

| Retained diagnostic, ms median | Traced RT | Non-RT VB |
|---|---:|---:|
| Four-light legacy-ranking control | 6.29 | 15.25 |
| LC3 still (min-max) | **14.43** (14.18-17.22) | **22.75** (22.57-23.03) |
| LC3 camera moving | 12.92 | 22.91 |
| LC3 four bright lights moving | 17.44 | **45.62** |

The four-light row is a budget/ranking control under the current engine, including exact zero-attenuation rejection; it is not a timing of the historical LC2 executable. The remaining moving-map cost is substantial despite conservative face culling. Shader visibility still scales with contributing overlap, and changed lights still rasterize their affected full-detail meshes. Quality, source radius and ray/filter sample counts were retained.

The four-map lantern control (`runs/lc3-h/game-control-nonrt/frame-015.png`, 24 authored sources, map budget four) also shows the patchy non-RT glossy-sphere response seen with the larger budget. This establishes that it is not introduced solely by raising the pool; this unit does not isolate local filtering from the existing GI/reflection limitations.

The coordinator changed the root order after that control, so `lc3-h-1` stopped between steps and released in 98.33 seconds. Every completed step passed; the batch's exit 1 records the scheduling stop. After the new `m7-mv1 FINAL release`, `runs/lc3-i`, lease `lc3-i-290`, completed the four remaining lantern steps: 16 still frames and 32 camera/object-motion frames on both paths. All passed, lease exit 0 in 53.89 seconds. Final example binary SHA-256: `204a7bd1627d4f68974fc42c1800bce54f9ba3e6e0fc4dddbfcaed7f10fafe6c`. All sixteen retained lantern PNGs are byte-identical to the first candidate's corresponding images.

Inspected `D:/Coding/renderlab-work/lc3/final-inspected-sheet.png`: the four-map non-RT control, final RT/non-RT still images, motion frames 0/8/16/24/31 on both paths, and forced-fade frames 0/3/7/15. Also inspected full `runs/lc3-i/game-still-nonrt/frame-015.png`. Final images are under `runs/lc3-i/game-still-{traced,nonrt}/` and `game-{traced,nonrt}/`. Silhouettes, local pools and contact shadows stay coherent over the inspected motion. The glossy-sphere and quarry map/ray differences remain recorded misses; these checks do not certify the Unreal floor.

The 64-light fixture uses coincident sources and a known fully occluded receiver point. A visibility-only program sums each source's visibility. The old four-light limit predicts 60 leaking lights; LC3 must report zero there on both paths. This is a geometry-proven leakage count. Quarry statistics separately report the number of local lights without any direct-shadow coverage; they do not prove every such light visibly leaks in that camera image.

Small edits outside the light/local-shadow files: `src/renderer.rs` checks the larger uniform ABI; the light helper in `shaders/program_api.wgsl` rejects exact zero attenuation; `examples/game_scene.rs` adds 24 warm local lights, `--night-lanterns` to expose them under dim sky/moon lighting, and `--local-shadow-count` for a map-budget control. The existing sun rendering, GI internals, reflection and transparency files are untouched. The example still/moving runs exercise the engine API and moving regular casters.

## Development and evidence retention

The unit took about 2 hours 25 minutes elapsed, including coordinator holds. Active implementation and harness work was roughly 45 minutes; build, queue, inspection and verification occupied roughly two hours of elapsed time, overlapping CPU fixes and documentation. The five acquired GPU windows totalled 354.41 seconds; no lease exceeded 15 minutes. All lib runs were inside LC3's own leases. Release builds used two Cargo jobs and the unit's own target; the timings above identify the binaries separately from the first compilation and pipeline-startup costs. No quiet-window performance claim or cross-engine comparison is made.

Evidence is retained under `D:/Coding/renderlab-work/lc3`: `summary.json`, `exact-rejection-hashes.json`, per-run JSON/logs, inspected PNGs and staged binaries. Raw buffers over 5 MB were summarized and removed, including the temporarily retained GI-off comparison buffers; deletion manifests are `raw-cleanup-e.json` and `raw-cleanup-h.json`. The unit's `target` directory was removed (699,623,800 bytes); `target-cleanup.json` confirms no remaining raw buffers over 5 MB.

## Remaining gaps

Non-RT still has a finite 128-light pool, lower-resolution tail maps, full mesh redraws for changed lights, no virtual-geometry local casters, the existing near-plane omission and capped PCSS blocker support. Atlas allocation has no repack/compaction or resolution promotion for held lights. Dense overlapping lights remain costly on both paths. Reflection-hit local visibility is still R1's separate limitation. No Unreal-quality, 2x-performance, reference-motion or transparency acceptance is certified by this unit.
