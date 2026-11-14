# Non-RT options: sun shadows, diffuse GI, glossy reflections, camera cuts

Study on main `f7e6e3b`, 2026-11-11. Reading and reasoning only: no build, no GPU, no new measurement. Islam's direction (relayed by the coordinator): "I need the fast path that generates the best possible results idc what you do architecture wise." This is therefore a comparison of options per problem with a recommendation each, not a plan to adopt Unreal's design. It replaces the earlier framing in [non-rt-path.md](non-rt-path.md) (same rays as the RT path in software; VSM and distance fields rejected as core), which assumed a different goal.

Labels: **C** read from code or a repository doc at a file and line; **M** measured, in a repository document or by the coordinator's brief (named); **S** reported by a public source (named); **E** this study's estimate or proposed parameter. Every forecast is **E**, not a measurement. Numbers from other hardware are marked, and none is compared with a 4070 SUPER number as if equal.

## Result

1. **The quarry gap is three different problems, and nobody has split the 8.65 ms yet.** Still 8.65 ms against Unreal software Lumen 4.02 (2.15x), moving 10.57 against 4.12 (2.6x), cut 43.11 against 4.11 (10.5x) (M, [p-show2.md](p-show2.md#quarry-rows-root-steer-ratio-against-unreal-epic), [design-cuts.md](design-cuts.md)). The quarry traces no reflection rays, because every scan exceeds 2^19 triangles and the coverage rule zeroes the bands (M, [m7-gaps.md](m7-gaps.md)), so **glossy reflections are not part of the quarry gap**; they are the Showcase problem (18.6 ms of 33.1, M, [design-nonrt-glossy-cost.md](design-nonrt-glossy-cost.md)). Step 0 below (a per-stage pass log of the three quarry states) must come before any option is built, and takes under an hour on existing tooling.
2. **Sun shadows: keep Renderlab's cached virtual clipmap, change the receiver side.** The structure that makes Unreal's VSM cheap (page-driven, cached, invalidated by moved casters) is already Renderlab's (C). What differs is the per-pixel cost: 32 active sun directions per receiver (C, [temporal_reconstruction.rs:296-326](../src/temporal_reconstruction.rs)) against Unreal's 8 SMRT rays. The one measurement is a ray-budget sweep of the `sun_shadow` stage.
3. **Diffuse GI: keep the world radiance cache as the store; do not add Lumen's screen probes or screen traces.** The WRC survives a cut, runs the user's program per cell, and avoids screen-space failure at the source. Take from Unreal the *scheduling* (fixed refill budget with a coarse fallback) and from nobody's design the *invalidation* (reactive, by change stamp, instead of slow temporal blending). The one measurement is the quarry cut's per-stage split plus a gather-cap sweep.
4. **Glossy: Unreal's roughness banding is the idea worth copying; its screen-probe carrier is not.** The cells hold low-order SH (C, [gi_cache.rs:1-12](../src/gi_cache.rs), `:3255`), which cannot carry a glossy lobe, so rough reflections need a new directional store. Before building one, force one ray on every pixel with roughness ≥ 0.4 on Showcase and read what that is worth: it bounds the saving.
5. **Camera cuts are a budget decision, not a cache decision.** Unreal's cut costs 4.11 against 4.02 still (M); Renderlab's cut does not flush its world caches (C, design-cuts.md) but quintuples the cold sun budget (4 → 32 batches of 8 directions, C). Decide the cut by a fixed budget plus visible-quality check, not by trying to avoid a cache flush that does not happen.
6. **Two policy conflicts need Islam's decision** (section 0.2): Lumen's design is screen-trace-first and AGENTS.md says "avoid screen-space techniques for core lighting and shadows". This document recommends not using them as the core and says what that costs.

## 0. Starting point

### 0.1 Measured

| Quantity | Value | Source |
|---|---|---|
| Quarry non-RT, still / moving / cut | 8.65 / 10.57 / 43.11 ms | M, [design-cuts.md:13-18](design-cuts.md), P-YARD2 (4070 SUPER, 1080p, UE 5.7.4, Lumen+TSR Epic) |
| Unreal software Lumen, same | 4.02 / 4.12 / 4.11 ms | M, same |
| Re-measured today | 2.1x / 2.6x / 10.5x slower | M, coordinator brief; matches [p-show2.md:154-156](p-show2.md) (8.66 / 10.74 / 43.34 on head) |
| Showcase non-RT frame, glossy part | 30.8 ms (brief) and 33.1 ms ([p-show2.md](p-show2.md)); glossy 18.6 ms (shading 20.78 − 2.18 with reflections off) | M, brief; [design-nonrt-glossy-cost.md §1](design-nonrt-glossy-cost.md). The brief's "19 of 30.8" is this same quantity on a newer head; not reconciled here |
| Unreal VSM, moving, quarry | 0.50 ms | M, brief only (I found no repository table for it). An earlier matched figure is **1.61 ms static** ([non-rt-path.md:37](non-rt-path.md)); the two are not reconciled, so treat 0.50-1.61 as the range |
| Renderlab non-RT sun visibility, game scene | march 2.8 ms still / 5.6 moving; S4 BVH gather 8.1 / 10.5; cut 26.5 → 8.4 with S4 | M, [m7-gaps.md:133](m7-gaps.md) (P-S4C). Game scene, not the quarry |
| Quarry non-RT sun cut | 32.8 ms, march about 28 ms | M, [design-cuts.md:49](design-cuts.md) (P-NSQ2); a different head from P-YARD2 |
| WRC costs | lookup about 0.80 ms; probe about 0.45; frame increment over cache-off +2.26 ms moving; worst non-RT cut frame 18.2 ms; 40-frame cut sum 383.7 vs 142-148 ms cache-off; 8,192-cell gather cap drains in about 100 frames | M, [design-nonrt-gi.md §1](design-nonrt-gi.md) |
| Unreal Lumen GI pass, quarry | 1.21 ms (SW), 1.24 (HW) | M, same file, GC1 |
| Software N0 8-ray sun shadows through the scene BVH | 9.43 ms, 21.9x hardware | M, [non-rt-path.md:72](non-rt-path.md) |

Two things follow by subtraction, both **E**. GI's moving-frame increment (2.26 ms) exceeds Unreal's whole GI pass (1.21) by about 1 ms, so GI explains only a fraction of the 6.4 ms moving gap. The rest is unattributed on the quarry: sun visibility and everything else. A per-stage split does not exist in the documents I read for the quarry moving state; Step 0 produces it.

### 0.2 Policy that constrains the options

- AGENTS.md: "Avoid screen-space techniques for core lighting and shadows: their off-screen and disocclusion failures conflict with this bar", and [milestone-7.md](milestone-7.md) says no SSGI/SSR as the core answer. Lumen traces the screen first and its final gather is built from screen-space probes (S, section 2). Using them as an accelerator with a world fallback would be a policy change; this document does not assume it, and says where it would matter.
- Shading stays programmable (AGENTS.md): any cache that stores *lit* results must be lit through the user's `shade` or accept that custom shading does not reach GI. Each option is scored on that.
- Patents: suspended 2026-11-10; one line only. Flags recorded in [m7-p0-claims.md](m7-p0-claims.md): US 11,315,310 (resampled reuse and world-space cache, relevant to Capsaicin GI-1.x and ReSTIR-family options), US 10,116,916 (variance clipping).

### 0.3 Step 0: split the quarry frame (about 1 hour, no new code)

`RENDERLAB_PASS_LOG` already logs named stages (`skinning`, `light_clusters`, `tlas`, `sun_shadow`, `transmission`, `atmosphere`, `visibility`, `shading`, `mesh`, `transparency`, `post`; [pass_timeline.rs:3](../src/pass_timeline.rs)). Run the P-YARD quarry capture (`p_yard_capture`, command line in [p-show2.md:147](p-show2.md)) on the non-RT path for still, moving and cut, and tabulate per-stage medians next to the 4.02 / 4.12 / 4.11 totals. Everything below is ranked on the assumption that `sun_shadow` and `shading` (which contains the GI lookup) dominate the excess; Step 0 can overturn that and the ranking with it. Run `--no-gi` and `--no-reflections` as differences if the stages are not enough.

## 1. Sun shadows

### 1.1 Candidates

| | A. Unreal VSM | B. Renderlab clipmap + Surface lookup (today) | C. Per-receiver software rays on the scene BVH | D. Cascaded or atlas shadow maps (Godot, O3DE, Bevy) |
|---|---|---|---|---|
| Per-frame work, scales with | Mark needed pages from the depth buffer; render only pages that are new or invalid; project with SMRT: 8 rays, 4-8 samples per ray (S: Epic VSM docs). Scales with page churn, lights, rays, samples per pixel | Page requests from receiver cells; software raster of visibility-buffer clusters into two 64-bit layers (height, triangle id) in 128-texel pages; lookup by direction batches of 8: 4 active batches, 32 cold (C: [virtual_shadow_gpu.rs:1-24](../src/virtual_shadow_gpu.rs), [temporal_reconstruction.rs:296-326](../src/temporal_reconstruction.rs), [design-cuts.md:38](design-cuts.md)). Scales with pixels × directions | Pixels × rays × BVH depth. N0: 8 rays 9.43 ms | Fixed cascade renders each frame or when the light or caster moves; Godot's atlas re-renders only what changed (S: Godot lights-and-shadows docs) |
| Cache and invalidation | Static pages cached across frames; invalidated by moved caster bounds; WPO, PDO and skeletal casters invalidate every frame (S: Epic VSM docs). 16k×16k virtual map in 128² pages; sun clipmap levels 6-22 | Cached until the sun turns, the window moves or a moved caster's bounds touch (C: virtual_shadow_gpu.rs header; [virtual_shadow_cache.rs](../src/virtual_shadow_cache.rs)). Surface lookup also caches per receiver cell with epochs (C, [non-rt-n8p.md](non-rt-n8p.md)) | None | Cascades re-render on camera movement unless cached; atlas rule above |
| Cost | Epic gives no milliseconds. Brief: 0.50 ms moving on the quarry; 1.61 ms static earlier (M, section 0.1) | Game scene: 2.8 ms still, 5.6 moving, cut 8.4 with S4 (M). Quarry cut sun 32.8 ms, march 28 (M, P-NSQ2) | 9.43 ms for 8 rays, quarry class (M) | No figures read |
| Visible failure | Box-like shadows when the page budget is exhausted, pool overflow corruption, terminator and bias artefacts, new pages on a cut, SMRT penumbra clamp and first-layer-only leaks, expensive moving foliage (S: Epic VSM docs; non-Nanite meshes cost much more to render into pages) | Sampling noise until TR1 converges; cut cost; umbra-only shortcuts missed penumbra fringes (P-RTS, M: 122 pixels, 0.004-0.047) | None structural; cost only | Resolution steps between cascades, cascade shifts, PCSS cost (S: Godot docs) |
| Fit with programmable shading | Fine: a scalar visibility at the `light_visibility` seam ([program_api.wgsl:96-100](../shaders/program_api.wgsl)) | Same seam, already integrated | Same | Same |

I did not read Bevy's or O3DE's shadow code (O3DE has a `Shadows` directory in `Gems/Atom/Feature/Common/Code/Source`; Bevy has a `bevy_light` crate), so column D rests on Godot's documentation alone. Nothing below takes shadow code from either.

### 1.2 What copying Unreal's structure would copy

- **Page pops and box-like shadows when the pool runs out.** Renderlab's page set is requested by receiver cells, so exhaustion shows as missing pages, not corruption, but no one has measured the failure threshold. Mitigation: a hard page budget with a coarser-level fallback, checked by a stress view.
- **Animated casters invalidate every frame.** Renderlab's invalidation by moved caster bounds is the same structure, so it has the same cost. Mitigation, if it matters: a static layer cached once and a small dynamic layer re-rasterized each frame, merged by min at lookup. Not recommended before a scene shows the cost.
- **Penumbra quality is bounded by 8 rays.** Renderlab's temporal reconstruction already accumulates beyond that over frames, which is where it can beat SMRT in a still image and lose in motion.

### 1.3 Recommendation

**Keep B; shrink the lookup budget; make convergence reactive.** The structural pieces that make VSM cheap are present. The open cost is the per-receiver direction budget: with the march at about 28 ms for 256 cold directions (M), **E** 0.11 ms per direction, so 32 active directions are about 3.5 ms and 8 are about 0.9 ms. That arithmetic assumes linear scaling in direction count, which [design-cuts.md:73](design-cuts.md) assumes too and nobody has verified on the quarry. The fix for the visible cost of fewer directions is not a larger blur but history reset only where visibility actually changed (moved caster bounds, sun change), which Renderlab's page epochs already know.

**Measurement (one two-hour unit).** On the quarry, non-RT, moving, sweep `active_rays` 32, 16, 8, 4 (the TR1 config at temporal_reconstruction.rs:296-326; add a one-line environment override if no flag exists), log the `sun_shadow` stage, and also run with visibility forced to 1 to get the fixed intercept. Compare the `shadow_contact` crop and the whole frame against the 10-frame path-traced reference used by P-NSQ2 ([p-nsq2.md:101](p-nsq2.md)).
- **Confirm** if the slope gives at least 2 ms between 32 and 8, and the 8-ray crop is not visibly worse in a flipbook of the moving frames.
- **Reject** if the fixed intercept is above 1 ms: the cost is page work or lookup setup, and the ray budget is not the lever; then profile page raster separately.

## 2. Diffuse GI

### 2.1 What Unreal's software Lumen does (S: Wright et al., SIGGRAPH 2022 course notes; Epic Lumen docs)

- **Per frame.** Update the surface cache within fixed texel budgets (capture 512², direct lighting 1024², indirect 512²), trace the radiance caches, place and trace screen probes, then filter spatially and temporally. Screen probes sit at 1/16 resolution per dimension, 8×8 octahedral radiance each, on a 16/8/4 hierarchical grid, jittered; the final gather integrates SH3 for diffuse. The world radiance cache (32×32 octahedral, persistent, clipmapped) serves rays beyond about 2 m.
- **Tracing order.** Screen traces (HZB) first; on a miss, mesh SDFs within about 2 m (sphere trace, 64 iterations); beyond that the global SDF (4 clipmaps of 256³); then the skylight. SDF hits read the surface cache, or voxel lighting clipmaps (4 × 64³) for global SDF hits. The surface cache is axis-aligned mesh cards in a virtual atlas (128×128 pages in a 4k×4k atlas), residency by GPU feedback.
- **Cache lifetimes.** Surface cache pages are updated by priority (last used minus last updated) under the budgets above, with accumulation capped at a few frames. Probes in the world cache persist and are relit through a GPU priority queue.
- **Cost (S, PS5, 1080p, 2022).** "Lumen in the Land of Nanite", global SDF only: surface cache 0.93, final gather 3.46, reflections 0.24, total 4.63 ms (Epic); Lyra: 0.66 / 3.30 / 4.74 = 8.70 ms. On the quarry, 4070 SUPER, UE 5.7.4: GI pass 1.21 ms (M, GC1). These are different hardware and years; do not subtract them.
- **Visible failures** (S: Epic technical-details docs; course notes): lag after lighting changes (sun-change propagation takes seconds), fast cameras outrun cache updates, walls under 10 cm leak, small emissives are culled, splotches and ghosting from temporal filtering of sparse probes, screen-trace disocclusion.
- **Fit with programmable shading.** The surface cache stores material attributes and is lit by Lumen's own pass (S: course notes, "Surface Caching shares material evaluation and lighting work between indirect rays"). A user's custom `shade` does not reach GI unless the card lighting calls it. That is a cost for Renderlab specifically.

### 2.2 Candidates

| | A. Lumen (above) | B. Renderlab WRC (today) | C. Godot SDFGI | D. AMD Capsaicin GI-1.0 | E. Bevy Solari | F. Resampled reuse (ReSTIR GI) |
|---|---|---|---|---|---|---|
| Per-frame work, scales with | Pixels at 1/16 probes plus budgeted cache refresh | Probe per 8×8 tile; setup, rays, gather and shading for new, invalid or rotating cells; scales with visible new cells, not pixels (C: [gi_cache.rs:2124-2865](../src/gi_cache.rs)) | Cascades of SDF with light probes, converging over frames | Screen probes plus a hash-grid radiance cache plus world-space resampling (S: README) | World cache update/query and resampling passes (file names only) | Per-pixel path samples reused across neighbours and frames |
| Cached, invalidated | Surface cache by priority budget; probe cache persistent | Hashed world cells keyed by distance level and normal bin; values replaced not averaged; new and invalidated cells always traced, rest on a fixed rotation (C: gi_cache.rs header); not flushed on a cut (C: design-cuts.md) | Cascades shift with the camera; moved static objects wrong until the camera moves (S: Godot docs) | Hash-grid cache with temporal updates | Unread | History reservoirs; reset on disocclusion |
| Cost | Table above | Lookup 0.80 + probe 0.45 ms; +2.26 ms moving; cut worst frame 18.2 ms (M) | No figures read | GI-1.0 report gives its own numbers (not read) | None read | Several ms at 1080p on RT GPUs, per the papers (not read) |
| Visible failure | Section 2.1 | Gather cap 8,192 caused long-lived cut blotches; 32,768 drains cleanly (M, [design-cuts.md:26](design-cuts.md)); refill drains for about 100 frames at the cap | Slow convergence (default "Frames To Converge" 25), cascade shift popping (S: Godot docs) | Not read | Not read | Correlation blotches, lag on lighting change |
| Fit with programmable shading | Poor (own lighting) | **Good**: cell shading runs the program (C: design-cuts.md row on GI scan) | Unverified | Fixed hit shading | Unverified | Needs hit shading per sample |
| Licence | Epic: read, no code or text copied | Ours | MIT | MIT; requires D3D12 DXR; patent US 11,315,310 | MIT OR Apache-2.0; hardware RT requirement not checked | Papers; same patent number |

O3DE: its Atom gem lists `ReflectionProbe`, `ReflectionScreenSpace`, `ScreenSpace`, `Shadows` and `RayTracing` directories (checked by listing); I did not read the code and found no diffuse-probe directory by that listing, so no claim about its GI is made. Licence Apache-2.0 with an MIT option.

### 2.3 Where copying Unreal would copy its failures, and what avoids it

- **Slow temporal blending produces ghosting and lag.** Lumen filters temporally with depth and normal rejection and amortizes relighting over frames. Avoid with *reactive invalidation*: each WRC cell already carries an epoch; when a light, sun direction or emitter changes, bump the affected cells' epoch and give them priority in the refill queue, and tell TR1 per pixel that its cell epoch changed so it rejects history there (instead of averaging toward the new value). Renderlab has relight groups and a reset path (C: design-cuts.md rows on GI and TR1); what is untested is a per-pixel epoch-change signal into the resolve. Not in any document I read.
- **Screen probes and screen traces fail off-screen and at disocclusions.** The WRC is world-space: no screen-space carrier, no disocclusion failure at the source. It costs the 0.45 ms probe and 0.80 ms lookup every frame (M), which is where Unreal's 1/16 resolution screen probes are cheaper per pixel.
- **Thin-wall leaks and small emitters.** Any distance-field or coarse-cell method leaks through walls thinner than its cell. The WRC's cells are keyed by normal bin as well as position, which already separates the two sides of a wall for lookups; the residual leak is at cell level near distance-level boundaries (E, not measured).
- **Cut refill.** Unreal's cut cost is flat because the caches persist and budgets are fixed (S). The WRC persists too (C). The excess is the refill of new cells and the 8,192/32,768 cap trade-off (M).

### 2.4 Recommendation

**Keep B as the store and the consumer. Borrow from Unreal the fixed per-frame budget with a coarse fallback for unfilled cells, and add reactive invalidation.** Specifically: lower the gather cap only together with a parent-level lookup so a cell that is not yet filled reads a coarser filled cell, not a hole. The known blotch at 8,192 (M) is exactly the missing-fallback failure.

**Measurement (one unit).** Quarry, non-RT, 40-frame cut sequence with the pass log. (1) Tabulate per-stage cost of the cut frame and the following 39 and subtract the recovery window to attribute the 39 ms excess between sun, GI and shading (design-cuts.md calls this attribution an estimate, not a measurement). (2) Sweep the planner's gather cap (32,768, 16,384, 8,192, 4,096) and record the worst frame and the frame at which the blotch crop returns to the settled value. If the cache has no parent levels to fall back to, the proposal costs more than two hours; report that as the finding.
- **Confirm** if GI stages are at least 8 ms of the cut excess and the cap sweep shows the worst frame falls with the cap.
- **Reject** the GI route for the cut if GI is under 5 ms of the excess: then the cut is the sun budget (section 5), and the GI cap is the wrong lever.

## 3. Glossy reflections

### 3.1 Candidates

| | A. Unreal | B. Renderlab R1n (today) | C. Reduce what we trace (our studies) | D. Others |
|---|---|---|---|---|
| Per-frame work | Roughness bands: ≥ 0.4 reuse the screen probes (importance-sample GGX, interpolate radiance; cuts reflection cost 50-70%); 0.3-0.4 shortened rays with world cache on miss (−16% on Matrix road); < 0.3 traced; then spatial reuse, temporal, bilateral (S: course notes) | `clamp(ceil(48 × roughness), 4, 16)` strata per pixel, each a closest-hit walk, an any-hit sun walk and a hit shading (C: [reflections.wgsl:156-207](../shaders/reflections.wgsl)); software BVH2 over a flat instance list (C: [reflection_software.rs](../src/reflection_software.rs)) | Four strata plus neighbour reuse (E 6-10 ms saved); BVH4 shared with GI/S4 (E 1-5); hit light from GI cells (E 1-5) (E: [design-nonrt-glossy-cost.md](design-nonrt-glossy-cost.md)) | Capsaicin GI-1.1 reflection paper (not read); Bevy `ssr`, O3DE `ReflectionScreenSpace` and `ReflectionProbe` exist (listed, unread); Godot not fetched |
| Cache | Probes and world cache; no reflection history of its own beyond temporal | None of its own; TR1 accumulates the final shading (C: design-cuts.md) | New history keyed on reflected-content motion | |
| Cost | PS5, 2022: Lyra Epic 4.74 ms, Land of Nanite 0.24 (S) | Showcase 18.6 ms (M); quarry none (M) | E, as listed | |
| Visible failure | Probe splotches and lag on rough surfaces, smear on moving glossy | White specks (fixed by hit regularization, M); blocky smears at 4 rays on moving metals (M, m7-rq1.md); mixed-program hits shaded by the receiver's program (C) | Neighbour reuse smears bright hits across object edges | Screen-space methods conflict with policy |
| Fit with programmable shading | Hit lighting from Lumen's cache | Hits shaded by the user's program (the seam at [program.rs:79-141](../src/program.rs)) | Reuse hides, not fixes, the mixed-program gap | |

### 3.2 Copying Unreal's structure, and what to change

Unreal's cheapest idea is not tracing rough pixels at all. Its carrier is the 8×8 screen probe; Renderlab's cells hold visibility bits and fixed-weight SH (C: gi_cache.rs header, `:3255`), which is irradiance, not a lobe. So the idea transfers only with a **new directional store**: either per-cell octahedral radiance (Lumen's world cache is 32×32; storage at 1M cells would be large; E, size not worked out) or reusing the probe tile's own hit results. Both are bigger than one unit, which is why the measurement below is an upper bound first. Screen probes are the route Epic took and the one our policy rules out as a core.

### 3.3 Recommendation

**Mix: Unreal's banding with our carrier, after the bound is known.** Rank for Showcase: (1) strata reduction with reconstruction, which is cheapest and already specified in design-nonrt-glossy-cost.md; (2) a rough-band reuse path for roughness ≥ 0.4 if the bound below says it is worth a new store.

**Measurement (one unit, no new pipeline).** On Showcase non-RT: add a diagnostic constant that forces `count = 1` for pixels with roughness ≥ 0.4 (one line at reflections.wgsl:156), log the `shading` stage, and in the same run report the fraction of glossy pixels at or above 0.4. The saving is an upper bound on what the rough band can ever give; compare with a pass-log split of walks and hit shading (design-nonrt-glossy-cost.md §3 asks for the same split).
- **Confirm** the rough-band route if the bound is at least 6 ms and the crop shows no visible difference on rough surfaces (they average many rays anyway).
- **Reject** it if the bound is under 2 ms: the cost lives in the < 0.4 band, and only strata reduction and BVH4 matter.

## 4. Sun and GI reuse of each other's structure, briefly

One structural overlap worth a note: Renderlab's WRC key is "copied from the shadow surface cache, not shared with it" (C: gi_cache.rs header). A shared cell key would let sun visibility and GI invalidate from one change signal. Not recommended for this unit; recorded because reactive invalidation (section 2.3) needs one place where "something changed here" is known.

## 5. Camera cuts

### 5.1 What a cut costs, system by system

| | Unreal | Renderlab non-RT |
|---|---|---|
| VSM / sun | Measured whole-frame cut 4.11 vs still 4.02 ms (M). Epic documents no VSM cut cost. Sun clipmaps are centred on the camera, so a far cut needs new pages; on this quarry the whole frame stayed flat, so the pages needed were presumably cached or cheap (E, unverified) | The cut multiplies the sun budget: 4 → 32 batches of 8 directions (cold 256, reset 64, active 32, held 8; C: [temporal_reconstruction.rs:296-338](../src/temporal_reconstruction.rs), [virtual_shadow_gpu.rs:1402-1436](../src/virtual_shadow_gpu.rs)). Cut sun about 32.8 ms, march 28 (M). New pages for the new view are requested but caches are not flushed (C) |
| GI | Persistent world cache and surface cache; fixed per-frame budgets for captures and relighting; cache misses fall back to lower resolution (S: Wright 2021 and 2022) | Not cleared on a cut (C). New cells refill: worst frame 18.2 ms; 100-frame drain at the 8,192 cap (M) |
| Reconstruction | TSR on a cut uses current-frame data only and a spatial anti-aliaser (S: Epic TSR docs): softer first frames | TR1 goes cold: raw noise until history builds (C). No separate reflection history to refill |
| Whole frame | 4.11 ms | 43.11 ms |

### 5.2 Why Unreal's is cheap

By construction, from the sources: nothing in Unreal's cut path scales with a *sample count* that the cut raises. Lumen's work is budgeted in texels and probes per frame (S), and TSR accepts a softer first image. Renderlab's cut raises the cold sun sample budget eightfold over active (32 → 256) because the policy was to give the first frame full quality. That is a choice, and it is the choice the earlier cuts study already proposes to screen ([design-cuts.md:5](design-cuts.md)).

### 5.3 Recommendation

**A fixed cut budget with visible-quality gating, not cache engineering.** Set the cold sun budget to the reset value (64) or active (32) and let reconstruction converge over the following frames; keep the world caches (as now); add the GI fallback of section 2.4. Do not copy TSR's soft first frame blindly: Islam's bar is "visibly better than Unreal", so the first frame must be inspected, not assumed acceptable.

**Measurement (same unit as 1.3, one extra row).** Quarry, cut frame: `cold_rays` 256 vs 64 vs 32 (config at temporal_reconstruction.rs:296-326). Log the cut frame ms and compare the first-frame crop against the reference.
- **Confirm** if the cut falls by at least 15 ms (E: 28 ms march × (1 − 64/256) = 21 ms) with the first-frame crop acceptable.
- **Reject** if the cut stays above 25 ms: the cut is GI refill or first-use cost, not sun budget (design-cuts.md warns that first-use pipeline warm-up can swing cut timings 5-13x; take warmed medians).

## 6. What may be taken from other projects

Licences were read from each repository's licence files and manifests on 2026-11-11.

| Project | Licence | What could be taken | Condition |
|---|---|---|---|
| **Godot** | MIT | The rule that a shadow map is re-rendered only when something it depends on changed (S: Godot lights-and-shadows docs); SDFGI's cascade structure as a design reference (S: SDFGI docs). No code recommended: the sun and GI recommendations above do not use SDF cascades | Keep the MIT notice and copyright with any copied file; Godot's contributors as author attribution |
| **AMD Capsaicin** (GPUOpen-LibrariesAndSDKs/Capsaicin) | MIT | Two-level cache structure (screen probes plus hash-grid radiance cache, `screen_probes.hlsl`, `hash_grid_cache.hlsl`) as a reference for the WRC's directional store; `world_space_restir.hlsl` for resampled reuse | MIT notice; requires D3D12 DXR on their side, so shaders are references, not drop-in; ReSTIR parts carry US 11,315,310 (number only) |
| **AMD FidelityFX SDK** (Brixelizer) | Not permissive (docs/license.md: binary-only redistribution, no reverse engineering; only samples MIT) | Nothing | Do not take |
| **Bevy** | MIT OR Apache-2.0 | Solari's world cache passes (`world_cache_update`, `world_cache_query`, `world_cache_compact`) and meshlet two-pass culling (`cull_clusters.wesl`, `visibility_buffer_software_raster.wesl`) as references: same licence family as ours, WGSL-adjacent (`.wesl`) | Preserve the notice; hardware-RT requirement of Solari unchecked; Solari and meshlet features were read by file listing only |
| **O3DE** | Apache-2.0 with MIT option | Nothing recommended; Atom has `Shadows`, `ReflectionProbe`, `ReflectionScreenSpace` directories (unread) | Elect MIT or Apache-2.0 per file; contributions under both |
| **Unreal / Epic** | Not licensed for reuse | Nothing: ideas only, from public documentation and course notes | No Epic code or text copied into this repository; the section 2.1 and 3.1 descriptions are paraphrase |

The brief names "HDDAGI" for Godot. I found no source for that name: a code search of `godotengine/godot` for the string returned nothing, and the issue search returned only SDFGI threads. It is not assessed.

## 7. Order of work

1. Step 0 (pass-log split of the quarry): 1 hour; decides what follows.
2. Sun budget sweep + cold budget sweep (sections 1.3 and 5.3): one two-hour unit; they share a capture.
3. GI cut attribution and gather-cap sweep (2.4): one unit.
4. Showcase rough-band bound (3.3): one short unit, independent of the quarry.

Only the first gives a measurement every other step depends on. No option is built before its measurement.

## 8. What reading cannot determine

- Where the quarry's 8.65 ms goes (Step 0).
- Whether the per-direction sun cost is linear down to 8 directions.
- Whether the WRC has a coarser level to read when a fine cell is empty.
- What Unreal's VSM costs on a cut; Epic's documentation gives no milliseconds and I could not read Epic's Fortnite VSM article (Cloudflare challenge, HTTP 403; not circumvented).
- Whether Bevy's Solari and O3DE's Atom GI are usable without hardware ray tracing.
- Unreal's source: not accessible to me, not read.

## Sources

Read in full: Wright et al., "Lumen: Real-time Global Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances course notes (https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf), text with speaker notes; Wright, "Radiance Caching for Real-time Global Illumination", SIGGRAPH 2021 Advances (https://advances.realtimerendering.com/s2021/). Renderlab docs and code cited above, at `f7e6e3b`.

Read through a fetch tool (extracted text, so quotations may differ from the page): Epic documentation pages for Lumen technical details, Lumen GI and reflections, Virtual Shadow Maps and Shadowing in Unreal Engine (https://dev.epicgames.com/documentation/en-us/unreal-engine/); the UE 5.7 TSR page as cited by [design-cuts.md](design-cuts.md); Godot documentation on SDFGI and on lights and shadows (https://docs.godotengine.org/en/stable/tutorials/3d/).

Licence and structure checks (GitHub API, file and directory listings only, no code read): godotengine/godot, GPUOpen-LibrariesAndSDKs/Capsaicin (README, GI-1.2; reports GPUOpen2022_GI1_0.pdf and SA2023_RealTimeReflection.pdf named, not read), GPUOpen-LibrariesAndSDKs/FidelityFX-SDK (docs/license.md), bevyengine/bevy (`crates/bevy_solari`, `crates/bevy_pbr/src/meshlet`), o3de/o3de (`LICENSE.txt`, `Gems/Atom/Feature/Common/Code/Source`).

Named but not read in this study: Bitterli et al., "Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting" (SIGGRAPH 2020); Ouyang et al., "ReSTIR GI: Path Resampling for Real-Time Path Tracing" (HPG 2021). Cited for what the family is, not for numbers.

Not used: Epic's Fortnite VSM technical article (blocked); the 2021 Nanite deck (contains no shadow material).
