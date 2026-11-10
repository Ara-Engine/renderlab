# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Skinning

- SK1 uses ALLOW_UPDATE/PreferUpdate for animated BLASes and the frame's TLAS;
  wgpu 30 currently rebuilds both. Actual incremental refit remains open.
- The non-RT path skins full-detail source clusters. Skinned cluster LOD is
  unavailable; bounds are conservative mesh-wide bounds. CPU software-GI BVHs
  rebuild for changed poses, and caster invalidation is conservative. Replacing
  animated geometry also repacks the static software scene each pose: the default
  GI quarry run is currently too expensive for game frame rates (**242 ms CPU,
  310 ms synchronized frame** in SK1's contended opt-level-1 diagnostic run).
  Persistent dynamic geometry ranges are needed; see SK1's measured frame cost.
- RLS v1 supports four influences, TRS nodes and one active sampled clip. Matrix
  nodes, additional influence sets and morph targets are explicitly rejected.
  Clip blending, events, dual-quaternion skinning and automatic root motion remain
  outside the overnight base.
- TR1 reprojects model-transform edits (MQ1) and deformed vertices (SK1); SK1's
  walk example puts translation in the palette.
- SK1 exposes typed renderer upload/palette APIs and a clip player. API1's planned
  `Engine` facade adapter (`MeshId`/`ObjectId` pose ownership) remains to be wired.
- Moving-caster shadow regions reject old receiver lighting history to avoid
  trailing shadows. Since MQ2 ([m7-mq2.md](m7-mq2.md)) GI no longer widens the
  region scene-wide: animated frames clamp history to MQ1's 3x3 box instead.
  Shadowed point and spot lights still widen it scene-wide; finer rejection and
  stable shadow sampling remain open.
- **Main regression (found by MQ2, 2026-11-08, main `4c8ef7d`):** the fox walk's
  RT half fails after frame 0 with wgpu validation "Blas … is newer than the
  containing Tlas" at `Queue::submit` (message and reproducer in
  [m7-mq2.md](m7-mq2.md) §3.4; API1b is fixing it). RT skinned evidence is blocked
  until then.
- Hardware shadows retain the existing opaque, double-sided caster contract. The
  example enables double-sided materials explicitly.
- No new Unreal or path-traced comparison was run for this overnight unit. Its
  visual and timing evidence, and their limits, are in [m7-sk1.md](m7-sk1.md).

## Post

PP1 provides bloom, camera motion blur, camera-requested near/far gather DOF, tone/white balance/contrast/saturation/3D LUT and optional lens effects on both paths. [Implementation and evidence](m7-pp1.md).

- Motion vectors currently reconstruct camera motion over static geometry from TR1's hit/projection inputs. Prior object/skinned/particle transforms and transparent-layer velocities still need a shared attachment. Hidden/disoccluded colour cannot be reconstructed from one surface.
- DOF needs better near-field coverage, hidden-background reconstruction, shaped apertures and temporal stability for large bokeh. MSAA raster depth/velocity uses sample zero.
- The filmic option is an ACES-like fit, not full ACES; RGB white balance is approximate. Display-linear RGBA8 LUTs have no HDR/log shaper or file importer.
- Bloom uses a fixed normalized six-level optical approximation. Grain is simple noise; lens dirt/diffraction/anamorphic calibration are absent. Post shader reload is not wired into the editable prelude transaction.
- No new Unreal/reference campaign tonight; full post quality parity remains unproven. Final effect timings and image findings are in the PP1 report.
- Optional DOF needs release profiling: the retained contended test-profile run reached 11.219 ms for non-RT quarry near DOF and 5.581 ms for raster far DOF. Camera-requested DOF remains off by default.
- API1's planned `Settings::post` and engine-module re-exports still need wiring; the typed crate-root API works through `Engine::renderer_mut()`. Native final checks include AT1/GI defaults; upscale-preset evidence predates that merge.

## Transparency

TP1 ([m7-tp1.md](m7-tp1.md)) and TP2 ([m7-tp2.md](m7-tp2.md)). Cutout foliage is load-time geometry; transparent materials draw in a forward pass after reconstruction. TP2 adds coloured sun transmission on both paths, R1 reflections on traced RT frames and four-sample geometric coverage.

- **Shadows from transparent surfaces.** A cached 1024-square layered map filters the first directional sun by tint, coverage and Fresnel on both paths. Each draw stores only its nearest sheet; overlapping sheets/instances in that draw do not compound. Other lights, environment visibility and GI traversal still omit transparent intersections. No volumetric absorption, caustics or finite-sun transmission penumbra. The map uses engine material inputs rather than an arbitrary program's transmission model.
- **Order.** Sorting is per object: one key per draw, from its visible instances' mean bounds centre. Instances inside a draw, and intersecting transparent objects, aren't sorted per pixel, and no OIT fallback exists. Unreal sorts per primitive with optional OIT.
- **Refraction** is screen-space and thin-slab only. Content off screen or behind a nearer occluder falls back to the unshifted pixel. No thick or volumetric refraction, no dispersion, no rough (blurred) refraction: the colour copy has no mip chain. Each transparent draw copies the full HDR target before it draws.
- **Anti-aliasing.** TP2's four-sample coverage resolves geometric edges after TR1. Texture/shading aliasing, intersections and particles still need broader treatment; glass has no temporal history. The seed/resolve and additional attachments have a cost.
- **Frames that skip the pass.**
  - 4x raster frames: there is no single-sample depth to read.
  - TR1b upscaled frames (`RenderScale` below native): hit records are at the internal size and presentation reads the output HDR.

  1x raster, traced and visibility-buffer frames at native scale run it.
- **Cost** (M, quarry, TP2 `eb0f4a2`, functional lease, not QUIET-certified).
  - Cache searches at glass receivers caused the regression: isolating an open-environment input reduced the glass increment from 1.123 to 0.173 ms. Retained RT sun rays and atmosphere.
  - With TP2 reflections, transmission shadows and AA enabled, the measured pane increment is **0.421 ms RT traced / 0.123 ms non-RT**. Cold pipelines and moving-sun map updates are not priced by these steady-state rows.
  - Each transparent draw still copies the full HDR target. Full data and contention limits are in the TP2 report.
- **Reflections on glass.** Traced RT frames now run R1 for the glass program's lobe. RT raster and non-RT retain the environment fallback (R1n was not merged at this checkpoint). Hits see only opaque geometry, with no recursive glass. Glossy strata are fixed without a history denoiser; mixed-program hits retain MP1's response fallback.
- **Lighting inputs.** Glass now uses the open environment: the radiance cache creates no cells for it, and receiver searches were expensive. Non-RT sun visibility remains a 2048² opaque map over the transparent receivers' footprint, not the Virtual clipmap: it's harder-edged than the sun's disk and its texel size scales with the footprint. Other lights' visibility on non-RT transparent surfaces is 1. Skinned transparent casters and palette-based invalidation of TP1's auxiliary opaque map remain unverified.
- **Programmability.** A program shades transparent surfaces through its own `surface` and `shade`, plus an optional `transmittance` hook. Without the hook, it blends by coverage. Refraction uses the engine's `material_normal`, not a program normal. Streamed (non-resident) textures on transparent materials bind their resident levels without feedback.
- **Cutout foliage.**
  - The cut is static: no alpha animation, no runtime change of cutoff or texture.
  - The contour is linear per grid cell, and the triangle cost is high: the fern card is 7,722 triangles at 96 cells. Measured cost of 18 cards: +0.20 ms non-RT, +0.39 ms raster, within noise on RT traced. There's no LOD of the cut.
  - Cuts must stay welded: an unwelded cut cost +6 ms on non-RT.
  - No tangent stream, so cut meshes take no normal map.
  - No alpha-to-coverage softening of the cut edge.
- **No Unreal or reference comparison** this push (rule 3).

## Particles

TP1 GPU emitters ([m7-tp1.md](m7-tp1.md) §1.3).

- **Lighting.**
  - Each particle carries one irradiance value per step, at its centre: directional lights with the sun map's visibility and TP2's coloured transmission, plus the environment's six-axis mean irradiance.
  - No point or spot lights, no radiance cache, and no self-shadowing or volumetric shadowing between particles.
  - Particles cast no shadows.
  - The phase function is a fixed isotropic half.
- **Sorting** is per emitter, in one workgroup: at most `MAX_PARTICLES` = 2048 per emitter. Emitters sort against each other and against transparent draws by their spawn centre, so interleaved emitters can order wrongly.
- **Simulation.**
  - No collision, curl noise, or vector fields.
  - Colour, size and emission are linear between two keys, not curves. No flipbook animation, no motion-vector blending, no ribbons or meshes.
  - Spawns per step are capped at the capacity.
- **Rendering.**
  - Billboards render at full resolution after TR1, with no temporal stability or motion blur.
  - Overdraw isn't bounded: no low-resolution particle buffer. Fire (192) and smoke (256) together add +0.44 to +1.21 ms (M, indicative), mostly smoke overdraw (I).
  - AT1 fog and aerial perspective apply at each fragment's billboard point. Particles neither receive nor cast volumetric fog lighting.
  - Soft fading uses opaque depth only: particles don't fade against glass.
- **No Unreal (Niagara) comparison** this push (rule 3).

## Flat-surface noise (both paths)

Islam (2026-11-09): "in both quality defect images you showed I can see noise on the boxes". P-FLAT ([p-flat.md](p-flat.md)) found three causes.

- **Current status (P-FLAT2's fresh measure on main ee361f5, real fire and Fox, frames 8-47).** Crate-top speckle is 0.079 on RT and 0.075 on non-RT. The path-traced reference's own grain is 0.080-0.096, so both paths are at or below it. Main started at 0.759 (RT) and 0.278 (non-RT); P-FLAT's lamp fix and P-NSQ's non-RT shadow fix brought them down. Crate sides are 0.044 on RT and 0.024 on non-RT.
- **OPEN, improvement beyond the reference grain:** P-FLAT2's exact light-range history ([p-flat2.md](p-flat2.md), branch `t3/p-flat2` 68a902b, NOT merged) cuts crate tops to 0.057 on RT and 0.054 on non-RT, with flicker slopes within noise of main's. Its final silhouette build was never compiled, and none of its acceptance tests ran (RF, Q-walk, SK1, MQ1, MV1, object motion, lib), because outside builds blocked the host. `light_budget` stays 0. The plan is in D:/Coding/renderlab-work/p-flat2/plans/w2.json.

- **Fixed (P-FLAT): the courtyard's spot lamp sat inside its own iron post.** 74-82 % of its source disk was blocked for every receiver, so every spot-lit pixel was a permanent penumbra that LC2's four rays sampled as grain. The arch light also clipped its bulb. Both lamps now clear their fixtures (`examples/game_scene.rs`; `--diag lamp=post` restores the old placement).
  - RT crate tops in flight: speckle 0.759 → 0.079, sides 0.270 → 0.044, arch 0.059 → 0.015.
  - The crates get the spot's light: both paths agree within 1.6 codes, where they differed by 6.8.
- **Partly fixed (P-FLAT-TR1, [p-flat-tr1.md](p-flat-tr1.md)): TR1's two history resets in this scene.**
  - **The walking character's rejection box is fixed.** With one shadowed sun, `tr1_caster_shadow` tests receivers against the swept bounds' own shadow, a conservative disk-wide test, instead of a world AABB stretched along the sun. Frames that drop history somewhere use the 64-ray restart budget. SK1's fixture shows no trailing shadow on either path. RF1-RF10 and the Q-walk stay byte-identical to main.
  - **OPEN, quality defect: a flickering local light still restarts every pixel every frame** (`Reconstruction::light_budget` 0, main's behaviour). In the courtyard that keeps every flight frame a restart, so the crates keep main's numbers (RT tops 0.079, non-RT 0.278).
  - **What the opt-in budget does and why it isn't the default.** A positive `light_budget` continues history under MQ1's 3x3 box, capped near the changed light from its distance, range and intensity change alone (B'). It improves every crop: RT crate tops 0.079 to 0.058-0.063, non-RT 0.278 to 0.198-0.220, and the wall, ground and arch on both paths. But it follows the fire's flicker slightly less than main does (wall slope 0.980 vs 0.985 RT, 0.980 vs 0.982 non-RT; ground 0.988 vs 0.996 and 0.989 vs 0.994), so root's bar keeps it off.
  - **Remaining lag source and fix directions.** On frames where the flicker barely changes, the cap allows history that still holds earlier intensities. Two directions:
    - size the cap from the light's change since the history's oldest sample, not since the last frame;
    - restart within a fixed fraction of the light's range.
- **OPEN (owner P-NSQ): the non-RT sun lookup leaves a fixed per-receiver pattern on flat surfaces.** It measures 0.631 on the crates on a still camera and 0.025 with the sun off. Its residual is frame-invariant (correlation 0.995 across 150 frames), so TR1 cannot average it. This is the non-RT sun-penumbra gap below, measured on a flat material.
- **Reference:** the matched courtyard (dusk sun, uniform sky, no local lights or character) against a 16,384-spp path trace. RT crate tops sit at or below the reference's own grain (0.021-0.045 against 0.080-0.096, RMS 1.3 codes). Non-RT is clean in flight but 0.509 on a still camera (RMS 4.5 codes). The path tracer has no point or spot lights, so local-light quality has no path-traced reference yet.
- The ground, arch and wall stay at 0.006-0.06 on RT.

## Post / HDR invariance

- **RESOLVED: PP1's neutral HDR assertion had stale replay setup** ([P-POST](p-post.md)). The first bad merge is VF1 `b388368`: it enabled temporal volumetric fog, while the sweep restarted GI/TR1 but retained fog history and its sample index. Even identical disabled-post variants failed on main. The sweep now calls public `Renderer::reset_atmosphere_history()` and completes each refill frame. All 13 variants match scene HDR, reconstructed output and TR1 colour history, still/moving, on RT/non-RT at native/Quality/Performance; 198 lib tests pass. The invariant is exact; no rendering quality setting was reduced.
- **OPEN: non-RT in-flight replay changes HDR under post workload.** With fog reset but unfenced static refill frames, motion/DOF variants can differ in scene HDR and TR1 history; the failing variant moved between runs. All 13 disabled-post controls pass, and completing each refill frame passes the full sweep. The responsible resource and whether this is a production cross-frame race remain unisolated. P-POST fixes comparison setup, not renderer synchronization; evidence and limits are in [p-post.md](p-post.md).

## Non-RT sun shadows

- **FIXED (P-NSQ, [p-nsq.md](p-nsq.md)): grainy, harder penumbrae on non-RT.** Two causes. The walk cap turned wide penumbrae into umbra, and each pixel's 8 directions were the same every frame, so TR1 could not integrate them. Virtual now takes RT's sampling under TR1: directions rotated every frame, 256/64/32/8 a frame by `ReconstructionShadowBudget`. It also drops the full-march walk cap (a truncated walk blocks only its own direction). Wide-sun game crop .02669 -> .02398 against the path-traced reference (RT .02374). The courtyard penumbrae now match RT's width, still and in flight. S2 held-frame penumbra grain 13.4-14.5 -> 1.4-2.3 (RT 1.3-1.7). RT is byte-identical.
  - **OPEN: quarry near-field speckle.** The quarry contact crop is .01505 against main's .01496 (RT .01485): about 100 single-pixel speckles a frame in gravel and along rock contacts, which the old cap hid. Occluders right at the receiver are not presented by the exact march. A near-field-only cap fixes the quarry (.01491) but costs the wide-sun crop 0.4%, so it was not taken. The next attempt should find those occluders exactly near the origin. P-NSQ2 ([p-nsq2.md](p-nsq2.md)) located the missed occluders: surfaces under the receiver mesh's own top layer within two texels of the ray origin (scan overhangs), which the peel layer skips unless `peel_own` is on. Testing them there and letting them block only by an exact hit gives .01493, but costs both game crops .00001-.00002 and the quarry +3.4 ms moving / +24 ms on cuts, so it was not merged. Next: find why its exact hits darken game penumbrae, and present those occluders without a walk.
  - **OPEN: cost.** Moving frames +2.6-3.3 ms (32 directions), cut frames +28-30 ms (256 directions, RT's cold budget), quarry held frames +0.7 ms, at 1080p. Paid back by P-NRT/P-RTS work, not by lowering the budget. P-NSQ2 measured where the cut goes (game 31.7 ms: map march about 24 ms, 1-ring walk 4-5 ms, setup and directions under 3.4 ms; quarry 32.8 ms: march 28 ms). No single part of the march is more than 2.1 ms of it. Two byte-identical candidates failed: a whole-disk clear proof that skips every batch's march (no gain; the pixels it proves were already cheap) and a ring replay across batches (40% slower, from registers). Next: share the shared steps' candidate walks across batches through workgroup memory, checked for occupancy in Nsight first. P-NSQ3 ([p-nsq3.md](p-nsq3.md)) built an exact launch-shape candidate, NOT VERIFIED: on several-batch frames the Surface walk splits a pixel's batches over up to 8 of its queue lanes and sums their counts after a barrier, with no added shared memory. Each batch's lookup is unchanged, so the result should be byte-identical. No timing or identity screen ran: every lease stopped on the 2 GiB RAM guard while other builds compiled, and the quiet lease never started. Next: one quiet ~3 min lease (`p-nsq3/plans/q1.json`).
    P-S1 ([p-s1.md](p-s1.md)) built design S1, the umbra certificate: one conservative test proves that the whole sun cone passes inside a triangle, and the pixel's remaining batches are skipped. A CPU oracle found no false certificate in 175k cases. **Rejected at its R0 gate:** on the game cut, count-0 pixels carry 2.8% of the lookup's work, against a 15% floor; the quarry is 20%. Built anyway for the record, it was slower (game cut 26.7 → 28.4 ms, quarry still 1.29 → 2.04 ms from codegen) and moved 2 of 26 changed quarry pixels away from the reference. The game cut's cost is penumbra and lit-but-walked pixels, not umbrae. S3 (replaying shared walks across batches) is dropped by its cost screen. Turning the shared walks off makes the game cut slower (27.7-28.2 ms), and the visits a replay could skip are 1.2% of the game cut's walk work. **OPEN leads:** the game cut needs a cheaper exact per-direction walk or S4, occluder-centric visibility (patent mapping first). On the quarry, 34% of the cut's walk work is replayable (screen bound about 2.5 of 9.4 ms). P-S3Q ([p-s3q.md](p-s3q.md)) built that replay exactly: the first batch of each lane records its shared walks' tests, keyed by (start, height, steps), in private memory, and later batches retest their own directions against them. It was byte-identical on all 40 captured frames, with occupancy unchanged (168 registers, 8,448 B, 12 warps), but **slower: the quarry cut went from 9.22 to 11.00 ms and the game cut from 26.73 to 28.66 ms**, and the still frames got slower too. Recording alone is free on the quarry. The replayed tests, read from L2-resident local memory, cost more than the cache-hot walk. Not merged. Replay across batches is closed for this kernel: registers, local memory and workgroup memory all fail. The 34% stays a lead only if the repeated 8-direction tests themselves are merged across batches.
    P-S4 ([design-nonrt-sunvis-s4.md](design-nonrt-sunvis-s4.md)), CPU only: **GO** for the occluder-centric gather on cut and restart frames (8 or more batches). The game cut drops from 43.1 M walk visits to about 3.4 M BVH loads, estimated 26.7 ms down to 4-8 ms. It is exact per direction (the RT/reference answer), not byte-identical to the march, so it must pass the ARA1 crops and the non-RT vs RT per-pixel check. The quarry is deferred: 2x less work, but +401 MB for a full-detail BVH. **OPEN patent flag — P-VIA/P-VIA2:** [claim mapping](patent-via-8471853.md#divisional-us-8605088) finds independent claim 1 of both VIA US 8,471,853 and divisional US 8,605,088 **AT RISK** for the shipping Surface march; both active with all maintenance installments recorded paid (expiry: parent 2032-04-25, divisional 2028-10-10); complete BVH gather plus weight-free/depth-free boolean half-space tests is **CLEAR of the divisional literal combination**, conditional on replacing the weighted helper and removing all old-march fallbacks; current cut-only S4 leaves exposure open.
    P-S4B ([p-s4b.md](p-s4b.md)) built it. Its per-direction test is a boolean half-space test over plane records (no vertex triplets, weights or depth, per US 8,471,853). **MERGED by P-S4C ([p-s4c.md](p-s4c.md))** behind a measured per-frame choice: S4 runs on cut and restart frames whose screen has at least 0.75 M x sqrt(disk / 0.00465) pixels (`sunvis_s4::chosen`), the march elsewhere. Game timing cut, 1080p: 26.54 → 8.41 ms; still, moving and recovered frames unchanged. Counts equal RT on 99.994% of pixels (main 99.988%); the `sun_shadow` crop against RT is .00078 on the cut (main .00242). The 960x540 game cuts and the wide sun's 1080p cut keep the march and are byte-identical to main. **OPEN:** S4 on every frame costs 8.1 ms still and 10.5 ms moving at 1080p (march 2.8 / 5.6). Its cut cost splits into traversal 2.6, classification 2.4 and direction tests 4.0 ms; on the wide sun the tests take 13 ms. Retiring the march (and so the P-VIA patent flag above) needs: the still-frame queue traced, per-lane candidate culls, sub-disk certificates for wide suns, and the quarry's full-detail BVH paid for by deleting the walk records. Restart frames were not timed (estimated close to a tie).
  - **Residual:** on S2's camera tracks, held-frame penumbra grain stays 0.4-0.6 codes above RT's.

## Non-RT frame cost (M8, P-NRT)

- **OPEN: P-NRT3b identity passes, DON'T MERGE: RT early-still median service is not cleared.** [P-NRT3b](p-nrt3b.md) merged current-main runtime and repaired the WGSL blocker, then dropped batched gather: non-RT gather windows improve (5.085/0.433 to 4.657/0.414 ms), but measured RT windows do not (0.0379/0.0361 to 0.0395/0.0380 ms). Split-only passes every registered identity comparison (426 outputs/hashes/buffers) and 200 lib tests. Moving game service improves 11.46 to 11.27 ms non-RT and 5.13 to 4.90 ms RT; isolated gaps improve 1.852 to 1.253 ms and 0.868 to 0.447 ms. Eight additional quiet RT quarry early-still pairs yield median 7.377 to 7.423 ms, 4/8 paired median wins; mean improves 7.828 to 7.526 ms. Preserve that distribution/median miss rather than declaring a universal win. Source retains the split as an experimental candidate; no promotion. All 1,061 resource samples were compiler-free; target deleted.
- **OPEN, performance: non-RT is still 3-5x Ara raster's service time on the matched scenes.**
  P-NRT's byte-identical changes (docs/p-nrt.md §2) cut ARA1 service by 13-18% on game-base
  (still 11.30 → 9.79 ms) and 17-21% on ara-pbr (still 9.43 → 7.79 ms). The quarry stays within
  noise. Non-RT remains 5.3x (game), 4.6x (PBR) and 2.2x (quarry) Ara raster's still service.
  The remaining cost, ranked by measured size (docs/p-nrt.md §1, 1080p, RTX 4070 SUPER):
  - **GI settling in ARA1's first ~90 frames (P-GI).** On the quarry, about 9 ms per frame of
    the 11-13 ms span is GI fill while a view is first seen; the steady state is 0.5-0.9 ms.
    ARA1's 88-frame windows measure the settling, not the steady state.
  - **Non-RT sun-shadow visibility (P-NSQ).** The N8n surface cache costs 1.2-3.4 ms on the
    game and PBR scenes. Its exact walks and claim/insert are about 40% each, all
    global-memory latency, and TR1's jitter makes still frames walk like moving ones.
    P-NSQ's penumbra fix may add cost.
  - **Deferred shading's software reflections (R1n).** The BVH traversal was about 1.6 ms of
    game-base's 3.0 ms shading before the vec4 loads (shading is 2.3-2.9 ms after P-NRT). The
    16-ray glossy budget stays. Next
    candidates: a short register stack, and a two-level instance tree with an
    index-ordered tie rule (identical closest hits).
  - **Environment lookups with a physical sky.** The atmosphere variant adds 3.4 ms of
    deferred shading. About half of shading is the exact two-level, four-tap cube filtering,
    run per glossy-ray miss and per hit's own `shade()`.
  - **The visibility-buffer resolve** keeps 0.4-1.5 ms where meshes have many
    missing-neighbour edges near pixels.
  - **CPU per synchronous frame.** `render()` costs 0.7-1.3 ms (encode dominates), plus about
    0.6 ms of completion and submit latency. That alone equals a third of Ara raster's whole
    frame. A CPU sampling profile of `submit_frame` is the next step; no profiler is
    installed.
  - **64 shadowed local lights (P-LTS):** +27 ms of deferred shading.
- **P-NRT2 update** ([p-nrt2.md](p-nrt2.md); profile of main `a5ae72a` at frame ~1200, 1080p).
  - Two more exact changes, both byte-identical and neither measurable in the frame:
    environment lookups load in-face texels in straight-line code (about 1-2% of shading under a
    physical sky), and the GI software rays read whole vec4 records (no measured change in the
    quarry span). The target is still missed.
  - What remains, ranked by size:
    - **GI fill in ARA1's frames 1-87.** ARA1's span is 1.7 ms (game-base) to about 7 ms
      (quarry) above the settled frame. The drain's per-frame cost is not the record loads.
      Profile a drain frame (a trace at about frame 20 of a cut) before the next change. Fewer
      drain frames need a W2-compatible schedule with a settle-quality proof.
    - **The synchronous CPU gap is 1.8-2.1 ms.** `render()` is about 0.85 ms (encode 0.67-0.89)
      and the rest is completion and submit latency. Splitting the frame into two submits would
      overlap encode with the first half's GPU work; it must check that no `write_buffer` lands
      between the submits.
    - **R1n traversal is about 31% of game-base shading** (box slabs, triangle tests, the local
      stack). A different BVH would change which of two equal-t triangles wins (shared edges),
      so an exact one needs a traversal-independent tie rule first.
    - **Under a physical sky, the environment texel loads are about 26% of shading**, now as
      straight-line loads. An exact further cut needs fewer lookups, not cheaper ones.
    - The GI cache cannot idle on still views under TR1 jitter without a 256-frame zero-work
      confirmation. That idle would never trigger inside ARA1's 39 still frames.
    - The GI light key Debug-formats every light each frame (`prepare_gi`). That is small, and
      matters only in many-light scenes; a memo must keep its exact equality semantics
      (`-0.0`, NaN).

- **P-NRT3 original is historical; P-NRT3b completed verification and recommends DON'T MERGE**
  ([p-nrt3b.md](p-nrt3b.md)). The original scalar collision was repaired and all composed
  modules CPU-validated. Batched gather was dropped for lack of an RT win. The split-only
  candidate is byte-identical on both paths and improves moving service/idle gaps, but its
  RT early-still median gate remains open after eight paired repeats. Offscreen service
  is not measured scanout latency; extra submit/internal-gap costs remain disclosed.
  Evidence and rebuilt binaries: `D:/Coding/renderlab-work/p-nrt3b/`; the old
  `D:/Coding/renderlab-work/p-nrt3/` binaries are historical, not the current baseline.

## RT shadows

[RTDEF2](rtdef2.md) selects traced primaries, Interactive and TR1 for capable
1x RT devices: 256 rays on a fresh target/cut, 64 on scene-history resets,
32 for camera/light motion or young history, and eight after 16 held frames.
The registered cold/moving disk witnesses no longer show RTDEF's extra grain.
The viewer negotiates the required RT capabilities; explicit Raster/Exact
settings and the old recorded gates remain available.

- Transform edits of existing placements no longer restart TR1 (MQ1,
  [m7-mq1.md](m7-mq1.md)): they reproject through the previous transforms and
  clamp to a 3x3 min/max box. Material changes and new placements still restart
  every TR1 pixel; the 64-ray restart budget improves their grain without fixing
  that coarse invalidation. Broader animated-game coverage remains work.
- The final disk grain diagnostic wins 47/48 sampled checkpoints. The early
  camera-1px frame 1 row is 4.857 versus Exact's 4.103 +4-stop codes, though
  inspected speckle is not visibly worse and whole-crop error is lower.
  Native quarry crops also retain TR1's fine-texture softness; this sampling
  change does not fix its resolve filter or certify film-exact detail.
- A first GI-cache initialization can dominate a cold frame (RTDEF2's
  quiet first traced quarry visit reached 89.53 ms; an earlier functional
  visit reached 120.9 ms). Warm-cache cold TR1 targets cost 14.85 ms median;
  held/camera frames cost 3.71/4.65 ms versus Exact's 17.04/17.05 ms with
  GI/AT1 enabled. These costs are reported separately; no startup/frame-time equivalence
  or engine-wide Unreal performance claim is made.
- The retained HRT comparison stays non-blocking. Disk cold grain is repaired;
  point-source first-frame geometric aliasing, TR1's revealed-pixel/sky/specular
  gaps and missing full-HDR flicker comparisons remain. RTDEF2's sparse
  checkpoints do not replace RTDEF's full 1,280-frame audit.
- RT1d's spatial visibility filter remains opt-in. Its false-shadow,
  lit-darkened and strict-solid losses are not repaired by this ray budget.
- Raster/4x, virtual geometry and RT devices without traced binding-array/
  hit-vertex capabilities retain the fallback path. The traced draw/geometry
  admission limits remain; this is not acceptance of every game workload.
- Historical eight-ray, TR1-off speed/memory rows are retained as historical
  evidence, not prices for the new GI/AT1/TR1 default. Unreal/film parity and
  a combined 2x advantage are not certified by this unit.
- **P-RTS (M8, [p-rts.md](p-rts.md)): RT first-cut and moving shadow cost is still open.**
  Penumbra listing (16 probes per receiver, then the budget only at listed edge pixels)
  cuts the game first cut from 21.6 to 5.6 ms. It keeps ARA1's eight non-quarry scenes
  byte-identical, and the quarry has one pixel off by one code that scores better. But it
  loses fringe shadowing on RTDEF2's S2 0.04 rad light (up to 0.125 on moving frames), so
  `PENUMBRA_LISTING` is false and the default is main's. Remaining gaps:
  - Listing is a sampled decision, not a certificate. A sliver of the light that no probe
    in a pixel's 3x3 window sees ("hidden fringe") keeps the unanimous 0 or 256 where
    production would give a near-unanimous count. Exact certification needs a conservative
    cone/occluder bound; hardware ray queries offer no cone query.
  - Held frames still trace eight rays for every receiver. The small-scene still-frame
    gap to Ara hybrid is mostly shading, GI and TR1 resolve, outside this unit; the
    sun-ray share of a held small-scene frame is about 0.4 ms.
  - With listing on, quarry cut frames are 5.8–7.5 ms CPU service (shadow share
    about 2.8–3.4 ms); the rest is GI's cut path and the resolve.
  - The non-deferred fallback (edge repair buffer over the device limit, e.g. some 4K
    targets) would keep the full budget for every receiver even with listing on.
  - No Nsight trace was taken (saturated GPU queue). The cost model is ARA1's per-pass
    timestamps across budgets, which are linear in the ray count.

## Atmosphere

- **OPEN: fog history survives camera cuts; possible ghosting after cuts.** `cut_reconstruction()` only marks TR1 (`src/primary_ray_gpu.rs:404`); `Engine::cut()` calls it plus texture-residency `mark_camera_cut()` (`src/engine/mod.rs:1015`, `src/texture_residency.rs:326`). Atmosphere has no camera-cut/teleport invalidator (`src/atmosphere_gpu.rs:55`, `:568`, `:1098`), so it can reuse fog from before a cut. `Renderer::reset_atmosphere_history()` is now available explicitly; automatic engine-cut integration and visual ghosting verification remain open. See [P-POST's audit](p-post.md#camera-cuts-code-audit).

AT1, [m7-at1.md](m7-at1.md), followed by [VF1](m7-vf1.md), [AT2](m7-at2.md) and [VF2](m7-vf2.md). The sky, aerial perspective, height fog, sun transmittance, procedural environment and shadowed volumetric fog are on by default on both paths.

- **Shafts improved; complete motion stability remains open.** VF2's two-window room shows distinct beams with dark air between them on actual non-RT and RT, especially at 0.01/m density. Eight-pixel columns, two decorrelated samples (four on history resets), visibility-only history and default HG g=0.8 replace the coarse single-sample RGB-history path. Density and height falloff are unchanged. Thin/distant shafts remain soft; repeating slat gaps can still merge into a broad volume. A non-RT moving-sun frame lights the wall beside the windows unexpectedly; resident shadow coverage, sun-basis changes and reconstruction are not isolated. No leak-free or scored temporal-equivalence claim. [VF2](m7-vf2.md).

- **Camera cuts reset volumetric fog history (P-NRT2, fixed).** `Engine::cut()` now calls `Renderer::reset_atmosphere_history()`. Before this, the first frames after a cut or teleport reprojected the old view's froxel history. On the pixels the fix changes, about 90% move toward a fresh render of the same view on both paths ([p-nrt2.md](p-nrt2.md)). Renderer-level callers that cut without `Engine` must call it themselves; `cut_reconstruction()` does not, because transmission changes also call it.
- **Non-RT volumetric coverage is bounded by resident shadow pages.** VF1 reads Virtual's top depth layer without triangle walks or new page requests. Missing or stale fine pages fall back through coarser valid levels, then to sunlight. Fog in a volume with no resident pages can still leak sunlight. Depth-map resolution, the caster LOD and sun-basis hysteresis can differ from the RT fog's exact centre-sun rays. Conventional raster fog has the sun cascades' coverage limit. Dedicated fog page demand and a measured soft sun disk remain open.
- **The analytic height fog's sun term is unshadowed** beyond the 128 m volumetric range, and everywhere while volumetric fog is off. Dense fog in those shadowed spaces can glow. VF2 fixes a separate backdrop error: fog now stops at the implicit planet-ground hit instead of integrating underground to an opaque unshadowed colour (from 1 m altitude, a 10-degree-down ray stops at 5.7588 m, not 128 m/100 km). The actual WGSL distance regression passes.
- **No matched Unreal comparison.** Neither Unreal's SkyAtmosphere nor its ExponentialHeightFog was measured tonight (no new Unreal campaigns). The medium is Hillaire 2020's Earth, the same model Unreal's SkyAtmosphere implements, but no image or timing comparison exists.
- **No path-traced reference** for the sky or the fog. The reference path tracer has no participating media.
- **A captured sky with a non-physical sun shows a dim backdrop.** The procedural sky scales with the first directional light's lux. The quarry's captured-sky protocol (a 1000 lux sun beside an RLE2 sky) therefore shows a near-black sky on misses, as it did before AT1 (a black clear colour). Scenes with a captured sky should turn on `set_visible_environment` until misses follow the selected map automatically.
- **Visible captured-environment and constant-clear backgrounds bypass fog.** VF1 composes near volumetrics onto the procedural sky, but TR1b's selected visible-map branch returns the environment directly; a sky-off miss returns the clear colour. Surface hits remain fogged. Fogging these backgrounds consistently remains an integration gap.
- **Only one sun.** Only the first directional light drives the sky, the sun disk, the fog and transmittance. A second directional light (a moon) gets none.
- **Continuous time of day relights progressively.** AT2 updates the procedural environment in place for sun, lux, medium and camera-altitude changes, without replacing bindings or clearing geometric GI visibility. The cache keeps its existing eight-group environment/bounce relight schedule and gather caps, so indirect light can lag a fast day cycle. Every changed frame still convolves the environment; there is no staggered cube update or altitude quantization. Changed environment yaw still replaces storage. Extreme sunrise/sunset, high-altitude travel and long day/night cycles remain unscored.
- **Moving-sun quarry cost remains high.** AT2's contended release/validation-enabled 32-frame pair measured 5.95 to 10.16 ms RT traced and 15.45 to 43.35 ms non-RT, including GI and shadow updates. The game fixture added about 0.52/0.58 ms RT/non-RT. Removing map churn did not establish cheap quarry time of day; stage attribution and a quiet measurement on the subsequently merged GI/lighting work remain open. No quality reduction was used. [AT2](m7-at2.md).
- **Fog temporal acceptance remains limited.** VF2 reprojects cell centres with coverage and density rejection, bounds old visibility by the plain min/max of the raw cell and six neighbors, and evaluates phase using the current camera. It stores no view-dependent RGB in history and uses no variance clipping. Lighting/media/sky/caster changes still reset reuse, spending four fresh samples; noisy starts can recur. The 0.9 m move/hold and moving-sun frames were inspected on both paths, but held-camera diagnostics do not prove a stability improvement; long disocclusion, caster motion and scored Unreal comparison remain open. Sky integration remains TR1's; startup speckles and an elongated sun in quarry motion are unisolated complete-image misses.
- **Validation is not clean.** VF2's renderer processes each retain ten Vulkan `VUID-StandaloneSpirv-None-10684` SPIR-V layout diagnostics, matching its frozen baseline and starting in pipeline creation. The 194-test lib suite after the main integration and isolated WGSL ground-distance regression log none; renders complete, but the layout issue still needs investigation. Conventional non-RT raster-map fog was shader-validated only; actual non-RT captures use VB + Virtual. See [VF2](m7-vf2.md).
- **Transparent surfaces:** `atmosphere_surface` is ready for TP1's transparent entry. It isn't wired there in this unit.
- **Film accumulation (`SoftShadows::Temporal`)** fogs its hits, but its misses keep `film_background()` (no sky).
- **Slice resolution and cost:** VF2 uses 8 px columns and 64 exponential fog slices over 128 m: first thickness 0.22976 m, last 6.96479 m. The earlier 7.8 m first-slice diagnosis was incorrect; the separate 32-slice/32 km aerial volume starts at 31.25 m in this source. Finer columns plus raw/two history volumes use 74.65 MB at 1080p versus 14.62 MB before, and steady shadow queries rise from 0.522 M to 4.147 M (8.294 M on resets). No edge-preserving full-resolution integration or isolated fog-cost timing exists; the one sequential timing batch is state/order sensitive.
- **Sun disk integration is approximate.** AT2 replaces the hard rim with analytic square-pixel coverage from explicit camera-ray footprints on raster and deferred misses, using stable chord distance. The rim is locally straight and limb intensity is sampled at the pixel centre; unusually small, subpixel custom suns need a full curved-disk/limb integral. The HDR ceiling, extreme exposure, horizon crossings and broad motion remain unscored; the old hard-rim result is superseded by AT2's measurements: deferred RT/non-RT crops match exactly across eight subpixel positions; raster retains 0.00276-0.00346 exposed-HDR crop RMS against deferred (maximum 0.0977). Disk energy varies by about 0.4% across those positions; no full curved-disk/limb reference or broad motion score was run.
- **The procedural environment uses the legacy mip-chain layout** (no ES1 lobes; the sun is the directional light, so no lobe is lost). Its smallest face is 8 texels: at 1 texel, rough lookups cost +0.47 ms of quarry shading. Roughness is quantized in 0.25 steps between levels.
- **The planet ground below the horizon** is a flat Lambertian albedo (0.3) lit only by the sun through the atmosphere. Misses below the horizon show that, not terrain.

## Decals

DC1: [projected box decals](m7-dc1.md) modify resolved surface inputs before programmable shading on both paths. The API includes typed textures, alpha/multiply/add colour blends, stable sort order, angle fade, opacity and channel strengths. Placement activates them without another feature toggle.

- No matched Unreal or path-traced decal comparison was run in the overnight time-box; Unreal-equivalent decal quality and a 2× performance advantage are unproven.
- The one contended development-build cost run has large timing tails. Quiet optimized cost, empty-table overhead, table-publication latency and high-overlap scaling remain unmeasured; the median paired shading deltas (+0.011 ms RT, +0.008 ms non-RT) are diagnostic only.
- Decal textures use trilinear filtering, with no anisotropic filtering, compression, texture streaming or normal-variance contribution to specular AA yet.
- The world-space grid never truncates overlapping lists, but widely scattered or heavily overlapping decals can increase candidate counts and publication cost. There is no GPU grid builder or world partitioning yet.
- Opacity controls overlay coverage and reaches user programs as material alpha. It does not cut opaque silhouettes or alter shadow coverage; consuming alpha in transparency is an integration task.
- Secondary reflection hits, moving/attached decal transforms and temporal reconstruction under substantial motion remain integration verification gaps. GI cell shading consumes decals and notices their changes, but a measured decal-to-bounce propagation test remains open.
- Forward lit GPU images and combined GI/TR1 configurations remain unverified. The retained traced and actual no-RT tests passed with zero Vulkan validation errors; quarry still and 1 cm moving images were inspected on both paths.

## Reflections

- **M8 P-R2 (2026-11-10): workgroup ray compaction rejected before implementation.**
  The 1080p 8x4 ray-loop count replay gives max/mean 1.0797 on game-base (both
  paths) and 1.1433 on ara-pbr (both paths), below the required 1.4 gate.
  No renderer change or speed/identity claim; the traversal-cost gap remains.
  WGSL's 16-byte vec3 array stride also makes 512 radiance results 8 KiB before
  queue/ray state, above the design's 6 KiB result estimate. Details and fixture
  counts: [P-R2](p-r2.md). Prefer per-ray traversal work over count compaction on
  these measured still workloads; variable traversal/hit cost remains unmeasured.

- **Closed (RD1, [m7-rd1.md](m7-rd1.md)): non-RT courtyard reflection blotches.** LC3's inspected sheet came from a binary built on main `622db4d`, before R1n merged, so its non-RT spheres showed the untraced estimate. On main `ff56b79` (LC3 and VF2 merged), the courtyard's non-RT chrome and gold match RT, still and moving with `--fox`, from frame 000 (`D:/Coding/renderlab-work/rd1/rd1-final-spheres.png`).

From M7-R1 ([m7-r1.md](m7-r1.md)). Scores are whole-reflector display RMS at -14 stops (8-bit F0 codes) against R0a's references, static, 1x, GI cache off; Unreal floors are R0a's.

1. **Non-RT reflections (R1n, [m7-r1n.md](m7-r1n.md)) trace in software, with limits.** R1's rays now run on non-RT visibility-buffer frames over per-mesh software BVHs, on by default. They beat UE SW Epic on RF1-RF7 and RF9 (RF1 4.15 vs 37.2, RF2 13.8 vs 15.9, RF6 6.7 vs 12.3, RF7 23.3 vs 35.6) and miss it on RF8 (7.96 vs 5.24, the untraced rough band, item 5). Against UE HW-Hit Epic they miss RF7 and RF8, as R1 does. They match R1's RT scores within 0.04 codes. What is still missing:
   - **Large and virtual meshes have no software geometry.** Meshes over 2^19 triangles and streamed clusterLOD meshes get none, and a frame drawing any such mesh traces nothing (the coverage rule, so no ray leaks sky through a missing mesh). The quarry (twelve ~1 M-triangle scans) therefore keeps the environment estimate. That untraced estimate is visibly blotchy on mirrors in motion (RD1, measured with the example's `--no-reflections`). The chrome sphere is near black at a cut and shows blocky dark patches for roughly the first 8-24 frames of motion; that these are the world radiance cache's cells is inferred, not isolated. The fix is R0c's cooked cluster-LOD cut chosen by ray-cone footprint, carrying drawn-triangle identity (or position-free leaves into the draw's own buffers), so hit shading keeps R1's contract.
   - **Skinned meshes (SK1) are absent from non-RT reflections.** Their vertices move on the GPU every frame, so they are left out of the software geometry: they neither appear in nor block non-RT reflection rays, and frames that draw them still trace the rest. The fix is a per-frame CPU skin or a GPU-built tree for skinned meshes.
   - **No LOD selection by ray cone.** Full detail is the only level; R0c's `select_cone_level` is not used yet.
   - **Cost on reflective content is high** (m7-r1n.md §3): +1.6 to +3.5 ms of shading at 1080p on the mirror and puddle fixtures, +6.7 ms on RF8's glossy rocks (functional, contended), against Lumen's 0.07/0.22 ms reflection pass (U0, A). A flat instance loop, a 32-entry stack in the shading kernel and hit shading inlined beside it are the likely causes (I); a separate trace pass, a two-level tree and wider nodes are the levers.
   - **Quarry with the cache on (the default): +0.30 to +0.52 ms of shading although nothing is traced** (m7-r1n.md §4), R1's codegen effect on RT now on the default path too. Images are bit-identical (cache on) with TR1 off. Fix as R1's item 11: a separate hit-shading pass or an open-only composition.
   - **Edge slack.** Möller–Trumbore is not watertight; a 4e-6 barycentric slack closes shared-edge seams (the identity quad leaked sky without it). Hits can lie up to that fraction outside their triangle.
   - **Transforms are rewritten whole** each frame they change; there is no refit and no moving-object special case.
2. **Raster primaries get no traced reflections.** The RT device's code default is still raster primaries (IQ1, RT1d), and only traced-primary frames trace reflections. Raster frames keep the environment estimate, as non-RT does.
3. **Hits on another program's object** (MP1, [m7-mp1.md](m7-mp1.md)): mixed-program frames now render on both paths, but such a hit is shaded by the reflecting surface's program with the hit's material inputs, not by the hit's own program (Programs, below). The toon program is still untested as a reflection-hit program in production; only R0b's probe has tested it.
4. **One bounce.** Hits take their own untraced specular. RF7 (facing glossy panels) misses at 23.3 vs UE HW-Hit Epic 20.2; its recursive panel-to-panel reflections are missing.
5. **Rough band (roughness ≥ 0.5) is untraced.** RF8's wet rocks miss at 7.9 vs 3.9 (traced 0.3/0.4 bands 6.8/7.5; untraced 0.5 band 9.3). RF2's 0.5/0.7/1 spheres miss at 15.9/13.1/10.4 vs 15.3/11.9/5.5, with no local colour bleed, which needs the cache's bounce in `indirect_radiance` (G2b).
6. **RF2's mirror spheres** miss at roughness 0 and 0.05: 14.1/15.0 vs 11.6/14.4. The reference split noise is 4.8 codes, so this ranking isn't certified.
7. **Glossy grain without TR1.** Four static rays left visible sparkle on roughness 0.3-0.4 metals that reflect lit and shadowed surfaces (RF2). TR1's rotated strata integrate it (m7-r1.md §5). Since RQ1 these bands trace 15-16 strata. The TR1-off RF images were not re-inspected; with TR1, the residual rim sparkle is reduced (m7-rq1.md §4.2).
8. **Hit lighting is simpler than camera-hit lighting.** At hits:
   - the sun is one hard shadow ray, with no disk penumbra;
   - the indirect inputs read the open environment, never the world radiance cache's occlusion or bounce. A cache lookup per hit cost +0.62 ms of quarry shading with the cache on (m7-r1.md §4);
   - footprints are an isotropic ray cone only;
   - decals (DC1) project onto hits through `decal_prepare`, as onto camera receivers.
9. **Motion against Unreal is unmeasured.** RF3/RF5 motion, trails and settle against R0a's continuous Unreal runs: no new campaign tonight. In the example flight, MQ2's view cap stops TR1's history from trailing reflections on both paths (chrome sphere 5.68 → 4.19 codes RMS against a 2×2 supersampled control, flicker 8.11 → 6.21; [m7-mq2.md](m7-mq2.md)). Mirrors under fast camera motion then get little temporal anti-aliasing of their reflected content; virtual-image reprojection (a per-pixel reflection depth from shading) is the open fix.
10. **Primary-miss background** (environment where the references are black) is R0a's registered renderer item, not a reflection error, but it dominates the full-image difference panels.
11. **Cost.**
   - On reflective content the cost is unmeasured; no comparison with Lumen's 0.07/0.22 ms reflection pass has been made.
   - The quarry traces no reflection rays (every material is roughness 0.85). It pays nothing measurable with the cache off (the default). With the cache on, the shading increment depends on code generation: +0.29 ms in one build, +0.04 ms after merging DC1, for the same R1 code (m7-r1.md §4). If G2b makes the cache the default, the increment can return with any compile. Candidate fixes are a separate hit-shading pass, or a second program composition with open-only indirect inputs.
12. **RT quarry with the cache on:** with no ray traced, the same 309 small pixel clusters differ from main's program in two runs, so the difference is deterministic. It is attributed (I) to codegen changes in the cache lookup's inputs, now prepared after `surface`. It is unscored against the reference; the cache is off by default (m7-r1.md §4).
13. **Programmability constraint:** in reflecting modules, `surface()` reads the open environment from the indirect inputs; `shade()` reads the prepared values. Reflection hits always read the open environment.

From M7-RQ1 ([m7-rq1.md](m7-rq1.md), Islam's "mushy" report). Glossy rays now scale with roughness, 4 to 16, on both paths. Measured on the matched example (`examples/reflection_matched.rs`) against a 16,384-spp path-traced reference: on the merged code, gold-sphere error falls 5.87 → 5.27 in motion (frame 47) on both paths, and RF2's 0.3/0.4 bands improve by 0.66/0.55 codes. Remaining:

14. **Glossy history in motion (TR1).** On the merged code, with MQ2's history cap, the moving gold sphere is still softer than the converged still: 5.27 vs 4.76 codes, both paths. MQ2's cap trades lag for per-frame grain, which these rays now carry. Reflection-aware reprojection (a per-pixel reflection hit distance or virtual depth) is still open. Once it lands, re-measure the ray curve: history may then carry what rays carry now.
15. **Ray cost (miss, recorded rather than reducing quality).**
   - At 1080p, shading rises up to +2.1 ms on RT and **+13.6 ms on non-RT** where glossy roughness 0.3–0.5 fills the screen (RF8: 5.6 → 19.2 ms non-RT). RF2 rises +0.5 / +4.8 ms.
   - The example at 720p rises +1.95 ms on RT and +3.0 ms on non-RT (GPU span p50, merged code: 9.12 → 11.07 and 13.17 → 16.15 ms).
   - An 8-ray cap would cost about half and keep about 60–80 % of the gain (RT gold 5.81, non-RT 5.41 against 5.68 / 5.08). It is a one-line change (`RL_GLOSSY_MAX`).
   - The cheaper routes are R1n's traversal levers (item 1) and item 14.
16. **Roughness 0.1 band moved +0.02 codes** on RF2 and RF4 (both paths), from five strata instead of four. RF2's band noise is 7.7 codes; RF4's is 0.39, so its change is measured but small.
17. **One bounce shows on metals in metals.** The gold sphere seen in the chrome sphere is olive, because the hit takes only its untraced sky specular. This is the largest single error on the chrome sphere (`rq1/scratch/chrome-err-still.png`) and the same cause as item 4.
18. **Hit lighting from the GI cache would help, but its cost is unmeasured.** Preparing the cache lookup at each hit (`hitgi`) cut the still gold-sphere error 4.78 → 4.35 and its low-frequency bias 4.44 → 3.97. It was not adopted: R1 measured +0.62 ms of quarry shading for a similar structure with nothing traced (item 8), and no quarry timing ran tonight.
19. **Tracing to roughness 1.0 helps little per ray.** Whole-frame rms 3.72 → 3.42 on the example flight, but the mean absolute gain is 0.015 codes, all on the 0.5-roughness hero, while every rough pixel (ground, stone) would trace. Rough metals (RF2 0.5/0.7/1) still miss UE (item 5).
20. **Shipped-image softness outside reflections.**
   - Post's 180° motion blur and bloom remove 15–27 % of the spheres' fine band (3–5 % of the mid band) in the moving example (PP1).
   - The quality upscaling preset adds error on the spheres (chrome 9.40 → 10.65); the example renders native.
   - Caustics (the gold sphere focusing the sun onto the hero and the ground) are in the path-traced reference but in no real-time path.

## TR1 (temporal reconstruction, default on; TR1b)

TR1 is the default on traced and visibility-buffer lit frames, both paths, from TR1b's overnight pass; `Reconstruction::off()` switches it off ([m7-tr1b.md](m7-tr1b.md)). It is never presented as meeting the Unreal bar.

**Reach.** RTDEF2 makes traced primaries and TR1 the capable 1x RT default; the viewer negotiates that path. Raster/4x and unsupported-capability fallbacks still do not reconstruct. Temporal soft-shadow frames keep their own film history and are not reconstructed.

**Animated scenes (MQ1, [m7-mq1.md](m7-mq1.md)).** Before MQ1, any transform edit restarted every pixel, so the example scene (and any game with a moving object) got no anti-aliasing and no glossy integration. Moved placements now reproject through their previous transforms, and frames where something moved clamp history to a plain 3x3 min/max box. Remaining gaps, measured on the example flight against a 2x2 supersampled TR1-off control:
- **Mirror and glossy reflections under camera motion (MQ2, [m7-mq2.md](m7-mq2.md)).** Programs declare their view-dependent share (`view_dependence`), and TR1 caps those pixels' history while the view turns (quarter-pixel budget). On the example flight against the 2×2 control: RT chrome 5.68 → 4.19, gold 4.40 → 3.48, every population better or level; non-RT chrome 4.19 against main's no-history 4.35. Remaining:
  - the cap trades history for no lag, so fast-moving mirrors get little anti-aliasing of their reflected content. Virtual-image reprojection needs a per-pixel reflection depth from shading;
  - budgets below 0.25 are unscreened;
  - the standard program's share is a uniform-light estimate, so dielectrics reflecting much brighter surroundings keep some lag;
  - a static scene under a moving camera now has its view-dependent lag capped too, but still no box (TR1b's U1b case).
  - Glossy grain: with four glossy rays, per-frame grain widened MQ1's box into blocky smears on moving glossy metals; RQ1's roughness-scaled rays remove the blocks ([m7-rq1.md](m7-rq1.md), Reflections items 14-15).
- **Non-RT animated scenes restarted every pixel on every frame on main `4c8ef7d`** (R1n's per-frame instance-record rewrite counted as a geometry upload, which TR1's key hashes). Found and measured by MQ2; the fix ships in RQ1. Since TR1 works there again, non-RT static edge RMS shows MQ1's moving-edge history lag: 10.87 → 11.40 codes against the lag-free control, with flicker lower everywhere and edges visibly anti-aliased where main stair-stepped.
- **Skinned characters with GI on** no longer restart every static pixel (MQ2): animated skinned frames use MQ1's box. Evidence is non-RT only (the fixture's ground is flat, so it shows no difference on static pixels); the RT fox walk is blocked by main's BLAS/TLAS validation regression (Skinning, above).
- Revealed pixels behind moving objects start from one jittered sample for about four frames; edges between two differently moving objects keep some stepping.
- The example's `--quarry` mode fails on both paths (mixed programs, MP1).

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

**Against TSR Epic on current main (TR2, [m7-tr2.md](m7-tr2.md)).** Re-scored with the same references and TSR rows on main `08aa1ba` (MV1, RQ1; TR2 leaves native frames byte-identical):
- **RT: the gap is mostly closed, by G2's default GI cache, not by reconstruction.** Visible-sky walk: TR1 worse than TSR Epic in **35 of 256** pooled rows (was 216), 84 of 626 aged pools (was 586). Q-walk: 18,746 wins / **2,783** losses (was 12,716 / 8,811); edge at frame 64 12.45 vs TSR 17.81 (L0). The 35 sky losses are all single-pixel maxima: 2-3-pixel bright spots on interior rock that appear as the GI cache converges (§GI).
- **Non-RT keeps the lighting gap:** sky walk worse in 218 of 256 rows, Q-walk 12,334 / 9,192. Its GI settles in about 93 frames after a cut and the sky walk has 40, so the shadowed-rock error stays at about 26 codes all walk.
- **Revealed pixels next to edges** still lose on Q-walk (age 3: RT 8.69, non-RT 9.48 vs 7.70). Moving pixels within about 1.5 px of an edge resample bilinearly and keep 3-5 samples. TR2's Catmull-Rom-next-to-edges screen (16- or 32-sample windows) helped background-centre and the hold step but cost moving interiors 1-1.7 %, and lost 350-500 more rows to TSR: rejected.
- **Return-segment flicker regressed since TR1b's recorded run** (Q-walk 127→128: RT 4.36 → 5.25, non-RT 4.36 → 5.03; TSR 3.43). Both paths, so not GI alone; not isolated (candidates: the overnight radius-1 start and paired jitter).
- **U1b's gate** now fails 396-418 of 776 comparisons (was 458-462): light-track failures fell (no restart under the 0.064-rad turn cap), camera-track failures did not (216-218 vs 220). MQ2's view cap does not appear to engage in U1b's fixture: a sun-turn term added to its drift left the light track byte-identical. Needs a check of the fixture's view hint.
- TR1b's lighting-matched 64-spp control predates the GI default and is stale on RT; the no-AA gate on the sky walk was not re-scored against it.

**Against TSR Epic, TR1b's recorded rows** (TR1b's settings before the overnight defaults):
- Q-walk: revealed pixels (age 3, all-covered L0 9.64 vs 7.70), background-centre next to edges (0.419 vs 0.371 at frame 64), edges at L0 by under 1 % (17.98 vs 17.81); +4-stop populations where the converged still image is itself above TSR's.
- U1b's 776-comparison specular gate fails (458–462 failed comparisons): view-dependent shading lags under camera motion; light-track turns restart the history.
- S2G sun 0.25°/frame on non-RT loses (15,476 wins, 26,772 losses): Virtual's shadowed shading under a turning sun.
- Cost: p95/p99 increments over the half-TSR ceilings (TR1's quiet run: RT increment p95 0.815 / p99 1.009 ms vs 0.769 / 0.816); the quiet re-time with TR1b's binary is outstanding.
- The visible-sky walk ([ue-sky.md](ue-sky.md), TSR Epic and High on the same 40 frames, scored against the same path-traced references; TR1b rows from its pre-tonight binary `tr1b-k`): TR1b is worse than TSR Epic in **216 of 256** pooled rows on RT (37 better, 3 tied) and 213 on non-RT (40 better), and in 586 of 626 aged pools. Selected rows, L0 / L4 codes (RT; non-RT within 0.07):

  | Row | No AA | TR1b | TSR Epic | TSR High |
  |---|---:|---:|---:|---:|
  | Moving covered RMS | 26.66 / 85.18 | 26.38 / 86.59 | 15.79 / 68.22 | 15.88 / 68.63 |
  | Moving edge RMS | 49.17 / 102.13 | 36.88 / 104.36 | 36.19 / 76.99 | 32.48 / 77.89 |
  | Moving background-centre RMS | 0.20 / 0.31 | 0.37 / 1.22 | 1.65 / 1.15 | 0.52 / 0.74 |
  | Revealed covered, age 0 | 51.07 / 128.20 | 40.30 / 124.25 | 27.31 / 96.16 | 26.98 / 95.73 |
  | Revealed covered, age 3 | 43.73 / 118.42 | 39.29 / 120.27 | 26.85 / 90.82 | 28.19 / 92.77 |
  | Moving flicker | 16.23 / 46.34 | 13.76 / 40.91 | 8.62 / 22.70 | 8.39 / 22.24 |
  | Final-hold step | 0 / 0 | 0.80 / 1.88 | 0.32 / 0.80 | 0.41 / 0.97 |

  These are complete-image rows: the covered-pixel gap also contains the lighting error in the backlit shadows, which no-AA shares, so it is not attributed to reconstruction alone. TR1b beats TSR Epic only on moving background-centre pixels at L0. TSR's cost on this walk: Epic total increment p50 1.38–1.39 ms (p99 1.95–2.53), High 0.97 ms.

**Upscaling presets** (quality 2/3, performance 1/2; opt-in, native is the default):
- Inspected only: no scores against references or TSR at reduced internal resolution, and no 4K output (a later unit).
- On S2G's moving glossy reliefs both presets aliased the finest specular stripes (stepped edges). TR2's `upscale_interpolation` (default on: young history interpolates the internal grid, narrowing to one output pixel as it fills) removes most of it on the quality preset; the performance preset keeps some stepping on the finest stripes. On the quarry they are slightly softer than native, unchanged by TR2.
- Timing is one functional (not QUIET) run: Q-walk RT p50 0.93 ms (quality) and 0.73 ms (performance) against TR1 native's 1.47 ms; non-RT 1.71 and 1.45 against 2.37.
- The internal size is rounded per axis, so targets whose size isn't divisible by 3 (quality) or 2 (performance) render at a very slightly different aspect.
- The capable 1x RT viewer now renders traced primaries with TR1 (RTDEF2). Upscaling presets remain library settings; the viewer has no preset flag.

**Reference instrument.** The visible-sky references were traced by a release-profile test binary; on fixed resources its first renders differ from later ones by ≤2 ULP, which the bounded-AS policy's late fixed-resource control does not see. A dev-profile build of the same source (U0b's profile) reproduces quarry-final byte for byte on 8 independent and 4 fixed replays. The sky references stay accepted under the policy; a dev-profile re-trace was stopped by the overnight no-new-references rule.

## GI

- **P-GIE / design G-E rejected at its cheap gate** ([p-gie.md](p-gie.md), 2026-11-09).
  Existing probe statistics at the actual 8x4 shading dimensions exceed the allowed
  40 unique keys per tile: game maxima RT 58 / non-RT 83; quarry 143 on both paths.
  These are conservative probe request unions, not exact shading-only sets. Averages
  (~11.3 game / ~25 quarry per 32 covered pixels) pass but do not satisfy the maximum
  rule. No production lookup change ships; the ~0.8 ms lookup gap remains. A future
  bounded cooperative cache needs an exact overflow path and a revised gate; the
  16 KiB device budget must also accommodate R2/R3. No identity or quiet speedup is
  claimed for this dropped design.

G2b ([m7-g2b.md](m7-g2b.md)): the world radiance cache is on by default on both paths, with bounce light and ES1's blocked sky specular, in G2's measured configuration (4 px cells) plus a per-frame gather cap. `IndirectSettings::off()` switches it off. It beats the cache-off image on 20 of 22 quarry populations after TR1 and Lumen HW/SW Epic on all-covered quarry error (G2's tables).

G3 ([m7-g3.md](m7-g3.md)) added a fixed-weight cell-space filter of the sky and bounce values, contrast refine (2 px cells only at visibility edges such as crevice mouths) and bounce fill, on by default (`filter` 15). Every quarry population is better than G2b-GC1's default on both paths. TR1x64 pinholes are at the cache-off level (RT 142 to 65, non-RT 122 to 68, against 74). The cave's mottle is mostly gone, and G3 now beats Lumen HW on edge, isolated and faint.

- **Cost against Lumen** (GC1, [m7-gc1.md](m7-gc1.md); functional runs, not quiet-classified). With the camera moving after a cut has drained, the default cache adds +1.60-2.01 ms on RT and +1.90-2.98 ms on non-RT (p50 1.68 / 2.51) over the cache-off frame, down from +1.98-2.22 / +2.75-2.95 on main before GC1. Lumen Epic's GI pass on the same quarry is 1.24 ms (HW) and 1.21 ms (SW), so the 2x target still misses by about 2.6-3.2x (RT) and 3.1-4.9x (non-RT).
  - **Largest stage: the shading pass's increment**, +1.01 ms (RT) and +0.78-0.88 ms (non-RT). It grew since G2b's +0.6 as main's shading grew, and is codegen-sensitive (a lookup variant that saved 0.1 ms on RT cost 0.07 ms with the cache off and lost on non-RT).
  - Next: the probe (0.43-0.45 ms on both paths), and on non-RT the query points of new cells (0.34 ms), gathers (0.22 ms) and their visibility rays (0.14 ms).
  - Not built: a half-resolution lookup with geometry-aware upsampling (a separate pass would take the lookup's registers out of the shading shader), level hysteresis, and an on-screen-first cut gather order.
  - **G3 adds +0.38 ms (RT) and +0.25 ms (non-RT)** to a moving frame ([m7-g3.md](m7-g3.md) section 5): the filter pass (about 0.10 ms) and, mostly, the finer reads at contrast-refined cells in shading (+0.22 ms). The half-resolution lookup is still not built: its blended values (13 RGB coefficients) read four times per pixel would move about as many bytes as the corner records do today. It needs its own unit, with a trace.
- **Camera cuts.** The 20-67 ms gather frames are gone (gather cap). The worst cut frame is now 11.3 ms (RT) and 18.9 ms (non-RT, cell creation in the first frame, unbounded by default). Bounce then settles over about 23 frames (RT) and 93 frames, about 1.5 s (non-RT), not 8 frames (GC1: gathers whose hits are all final stop early, a fifth fewer cut gathers; G2b read 28 frames and 2-4 s). While non-RT drains, flat surfaces speckle (SV test scene); `gather_cells: u32::MAX` restores fast settle with the big cut frames. An opt-in claim cap bounds cell creation but settled the test scene worse. Since G3, cells waiting for their first gather take their gathered neighbours' bounce, so non-RT's SV speckle settles much sooner (gaps-pose error at frame 8: 16.4% to 11.1%). But G3's refined cells add about a fifth more cells and gathers to a cut. The worst cut frame is now 14.2 ms (RT) and 18.5 ms (non-RT), and non-RT's gather queue drains after frame 100.
- **Crevice mouths** (G3). Pinholes are at the cache-off level (TR1x64 pose 0: RT 65, non-RT 68, cache-off 74), and the streak in the gap right of the central slab is lighter and narrower. Some darkening at mouths remains narrower than a 2 px cell. No per-pixel short-ray check was built.
- **Enclosed cavities** (G3). The cave's coarse mottle is mostly gone (fine-scale error 8.2 to 6.9 codes), but the cave stays about 4-5 codes too bright. The hollow's error fell from 13.6 to 10.1 (RT), near es1open's 9.9, but it is still about 5.2-5.5 codes too dark, and one darker band remains under the slab's lip. Likely cause (I): gathers read the hit cell's value at its query point, not at the hit, so rays leaving through a gap narrower than a cell may read a shadowed point. The fix would be direct light evaluated at gather hits, a transport change.
- **Other losses** (G3). Isolated rocks lose to cache-off at −14 stops by 0.06-0.34 codes and win at −10. They now beat Lumen HW and SW. Multi-surface is 0.09-0.25 codes behind Lumen HW at −14 stops and ahead at −10. In U0b's TR1 pan, silhouette loses to G2b-GC1's default at three witnesses (worst: non-RT frame 128, +0.37 / +0.93): the gaps between small gravel stones open slightly where the reference is darker. Against ES1's unoccluded diagnostic it was not re-scored.
- **Filter memory** (G3). The filter's sources take 88 bytes a slot (88 MiB at 2^20 slots); section "Memory" of m7-g3.md has the process totals.
- **Path-traced re-score (TR2, [m7-tr2.md](m7-tr2.md), retained references).** On TR1's visible-sky walk the cache removes most of the shared lighting error on RT: single-surface L0 error falls from 25.8 codes at the cold frame to 5.9 by frame 10 and 3.1 by frame 35, and TR1 now beats TSR Epic on 218 of 256 pooled rows. On non-RT it stays at 25.8-26.6 for all 40 frames (bounce settles in about 93 frames). On RT, 2-3-pixel bright spots (145-253 codes against a reference of about 0) appear on interior rock as the cache converges and persist through still frames; they are the walk's only remaining losses to TSR (maxima). G3's filter was merged after this walk; whether it changes these spots is unmeasured.
- **Still frames after a cut** run the cache passes for up to 16 + 64 x (gathers + 1) frames while capped gathers may still be pending, about 0.5-1 ms each. **With TR1 on (the default) the cache never idles** (P-GI, [p-gi.md](p-gi.md)): every frame's camera is jittered, so the idle key changes every frame and a still view runs the probe (about 0.45 ms) and the other passes forever. The jitter sequence has a 256-frame period (`sample_offset(k & 0xff)`), so an exact idle would need a full period of frames with no cache work, confirmed by GPU feedback.
- **P-GI cost status** ([p-gi.md](p-gi.md)). Byte-identical changes cut the gather and filter passes in cuts and the shading lookup's bucket walk; see that document for the moving and cut timings. What remains, measured by ablation on the RT shading pass (camera moving): the lookup is about 0.80 ms, of which contrast refine's finer reads are about 0.25 ms, the bucket walk about 0.18 ms and the rest about 0.37 ms; four corner records already load in parallel. The probe (about 0.45 ms on both paths) is unchanged.
  - **Non-RT's capped gather drain dominates ARA1's early frames** (P-NRT's tail runs: about 9 ms of extra span over frames 1-88 of the quarry, 2.9 ms on the game scene). Faster software closest-hit rays and the gather fast path shorten each drain frame. The drain's length (the cap, 8,192 cells a frame) is unchanged.
  - **Cold records carry 8 unused words** with the default 64 directions (sized for 128): 32 MiB at 2^20 slots. Packing them moves the light bits, and `src/light_cluster_tests.rs` hard-codes their offset (`cold+31`), so it needs P-LTS's agreement.
  - **Shading workgroups are 4x4** (`primary_shade_tiles`, 16 invocations, half a 32-lane warp on NVIDIA). The lookup is latency-bound at that occupancy. Not GI's file; recorded for the shading owners.
  - **Nsight attribution for GI failed** in P-GI's trace (the real-time shader profiler attributed samples only to the traced-primary pipeline); the ablation table in p-gi.md section 2 stands in for it.
- **Moving objects mottle.** In the courtyard (`examples/game_scene.rs`), the spinning crate shows blocky light and dark patches on both paths in frames 008-032, and it is clean with `--no-gi` (RD1, [m7-rd1.md](m7-rd1.md) section 3; not investigated further).
- **Other units' harnesses.** `src/reference_r0a_tests.rs` asserts the cache starts off, and `src/iq1_tests.rs` uses `IndirectSettings::default()` to mean off. Both need `IndirectSettings::off()`.

G2c ([m7-g2c.md](m7-g2c.md)) fixed API1's game-scene ground speckles (glossy and mirror cells sent their sun highlight in every direction) and arch-edge streaks (not-yet-gathered cells read as zero bounce at level switches). What it leaves:
- **Shiny metals bounce too little.** A cell reflects as a Lambertian surface with the program's directional albedo over 16 fixed directions, which cannot integrate lobes much narrower than their ~36° spacing. Gold at roughness 0.25 reflects about 0.4-0.5 of a true ~0.8, and chrome at 0.02 about 0. Near shiny metals the bounce is too dark (energy lost). Mirror caustics are not represented at all: no view-independent cache does that.
- **Light-direction response beyond the cosine does not reach bounce.** Lights reflect through the albedo times max(n·l, 0). A program's wrap lighting, transmission or retro-reflection shades its own pixels but bounces as Lambert.
- **Energy cap.** The albedo is capped at 1 per channel. It binds only for programs whose `scatter` is not energy-conserving, or for a normal-mapped mirror whose lobe meets a quadrature direction head-on.
- **Bounce latency at level switches** is now filled from the coarser level, which is blurrier than the settled finer level until the new cells' first gather (a few frames under the gather cap). Moving toward a surface shows a softer bounce band, not a dark one.

## Lights

P-LTS2's A checkpoint ([p-lts2.md](p-lts2.md)) uses 32-invocation single-program
shading groups and preserves mixed-program 4x4 lists. The initial contended 1080p
screen is faster but still misses 3.5 ms on all four shadowed/unshadowed RT/non-RT
cases. Lib tests, ARA1 HDR/PNG, LC1-LC3, mixed-program and courtyard image gates
pass byte identity, including all 28 float-HDR/56 PNG full-quarry witnesses on both
paths. Clean shadowed pairs measure 10.098 to 6.632 ms RT and 15.864 to 13.685 ms
non-RT. Latest unshadowed diagnostics are 8.841 to 4.652 ms RT and 10.018 to 8.955 ms
non-RT; they are not quiet certification because of a reported overlapping build.
The original final guarded retime never admitted before the unit time box.
[P-RETIME](p-retime.md) now clears the suspected soft quarry default regression:
quiet ABBA still service is RT 13.59 -> 13.03 ms and non-RT 20.22 -> 19.25 ms;
camera motion is 10.40 -> 9.60 / 22.95 -> 20.95 ms, with byte-identical captures.
The separate full-track ARA1 RT quarry also improves; its non-RT pair and quiet
unshadowed certification remain open after P-RETIME's three-hour wait expired.
The final queue audit had only 5536 MiB available, 17 Cargo and 22 rustc processes;
no unshadowed regression verdict is inferred. The uniform-light/packed-index-cache prototype demonstrated no
gain in its contended screen and is unretained. All light/shadow budgets remain intact.

LC1 ([m7-lc1.md](m7-lc1.md)), LC2 ([m7-lc2.md](m7-lc2.md)) and LC3 ([m7-lc3.md](m7-lc3.md)): clustered point/spot lights with soft local shadows. LC3 removes the selected-light limit on RT heads and raises the cached map pool from 4 to 128 lights by default. The soft sun renders with further lights. LC3 evidence is recorded in its unit note; no new Unreal or path-traced comparison was run tonight.

- **P-LTS performance target remains open** ([p-lts.md](p-lts.md)). Exact-zero rejection in the standard material and four depth gathers for interior hard PCF retain the 128-map budget, every contributing RT light and LC2 softness. In a paired, compiler-quiet 1080p ARA1 steady replay (RTX 4070 SUPER), shadowed RT improves 12.13 to 11.08 ms and non-RT 25.52 to 18.00 ms; unshadowed RT improves 10.16 to 9.25 ms and non-RT 13.14 to 11.98 ms. All four remain above the approximately 3.5 ms frame target. These are drained CPU+GPU service medians, not isolated light-pass costs. Clustering is about 0.010 ms; map sampling and shading dominate. Image identity and fixture evidence are in the unit note. Moving-map redraw, soft filtering and dense contributing-light work remain gaps; no quality budget was reduced.
- **P-LTS4 exact hard-map improvement** ([p-lts4.md](p-lts4.md)): current-main, compiler-quiet 1080p ABBA reduces shadowed non-RT steady service from 13.256 to 12.131 ms (8.49%) by replacing dynamic hard-PCF vector indexing with sixteen fixed comparisons in the same accumulation order. RT shadowed remains about 6.67 ms; unshadowed RT/non-RT remain about 4.35/8.66 ms. All four remain above 3.5 ms. All four 1080p HDR/PNG ABBA witness sets match; 200 lib tests pass. Original-resolution replay and final unit verdict are recorded in the unit note. The 128-map budget, every contributing light and LC2 softness stay intact.
- **Forward raster cost.** RTDEF2 now defaults capable RT frames to traced primaries with TR1. Explicit forward raster still shades every rasterized fragment; LC2's diagnostic 261-light quarry costs 43.94 ms with hard local maps and 67.45 ms with soft maps, versus 11.93/12.27 ms traced and 17.55/18.08 ms non-RT VB. This is one contended drained-frame run, not a comparative performance certification.
- **Finite map coverage (LC3).** Map heads shadow at most 128 local lights; 132 of the quarry's 260 locals still lack automatic direct shadows, versus 256 under the old four-light limit. RT heads shadow every contributing clustered local light when automatic local shadows are enabled. The controlled 64-light blocked-receiver fixture reduced measured leaks from 60 to zero on both paths; coverage counts in the quarry are not geometry-proven leak counts. This does not remove the cluster overlap limit or R1 reflection-hit visibility gap. Unreal VSM shadows every movable local light, with page caching.
- **Selection and transition limits (LC3).** Map ranking now weights camera illuminance by clipped projected influence-sphere area, with 25% hysteresis. This is conservative sphere coverage rather than visible receiver coverage or a tightly fitted spot cone. Outgoing shadows fade to zero over eight admitted frames before slot reuse; incoming shadows then fade in over eight. Held tiles do not resize with camera movement, so moving much closer can retain insufficient resolution. Explicit budget reductions and light-vector removal/reindexing are not smooth-transition guarantees.
- **Local penumbra limits (LC2).** Point/spot `source_radius` now enables four disk-sampled RT rays integrated by TR1, or PCSS blocker search and variable PCF. Radius zero preserves hard visibility. Maps cap support at 96 texels and use 12 blocker-search taps; sparse blockers can be missed. RT visibility uses centre incidence and retains four-ray grain without TR1. No new local-shadow reference or motion campaign certifies the Unreal floor.
- **Maps draw full-detail meshes** (no cluster-LOD raster) and only regular draws. Virtual-geometry casters are missing from local maps. Moving lights still redraw their maps; LC1 measured +5.7 ms for four spot faces on the quarry. LC3 caches unaffected maps by intersecting caster placements, conservatively culls bounds per cube face and reprojects both hard PCF and soft PCSS across cube faces, but has no virtual page cache or static/dynamic-caster split. The retained LC3 moving-light quarry diagnostic still costs 45.62 ms on non-RT. The 0.02 m near plane drops casters closer to the light (rays have no near plane).
- **Unclassified map/ray differences.** Fine speckle on the gravel pile with maps (pebble shadows or acne), and one region behind the large rock that rays shadow and maps don't. The lantern scene's non-RT glossy spheres also show a patchy response with both four-map and 128-map budgets. These causes were not isolated tonight.
- **GI light-list scaling (LC2).** Every light now reaches bounce through a dynamically sized per-cell bitset, with range/cone/hemisphere rejection. Both deferred paths record 171 contributing lights after the first 32 in the night quarry. At 261 lights the default table grows by 16 MiB to 128 MiB plus its header. Work is bounded by queued cells times light count, without a separate ray budget; dense overlap remains expensive. Bounce visibility still uses hard centre rays. The explicit forward-raster control has no GI-cache pass.
- **Unculled stages walk every light:** R1 reflection hits, which run before the cluster is prepared; film accumulation (Temporal soft mode, opt-in); and GI visibility selection. Cell shading walks the recorded visible-light bits. Their cost grows with the total light count on reflective content. Reflection hits shadow unselected local lights not at all (R1's hit shadow skips lights without shadow views).
- **Cluster capacity.** 128 lights per cluster (the test's longest was 99); beyond that, a cluster drops its highest-indexed lights, and the dropped count is recorded in the list. Sets above 65,535 lights walk every light.
- **Programmability constraints:** implicit-derivative sampling inside the light loop is rejected (per-receiver loop bound), and fixed-index light addressing now means "the cluster's i-th light".
- **Memory (LC3):** default atlas depth is 384 MiB (4096-square x 6 Depth32Float), allocated on the first map frame, plus 3.6 MB of lists and a 51,312-byte uniform. This covers up to 128 selected lights with 1024-to-128-square faces. Spots waste five corresponding face regions. The old four-light maps used 96 MiB; devices below the larger uniform ABI now get an explicit construction error. In-flight atlas replacements temporarily retain both allocations.
- **LC3 timing:** contended drained CPU+GPU medians, eight samples after warmup, RTX 4070 SUPER / Ryzen 7 7800X3D, release, 1080p quarry, default GI/TR1 and soft locals. Still: RT 14.43 ms / non-RT 22.75 ms; camera moving: 12.92 / 22.91 ms; four bright lights moving: 17.44 / 45.62 ms. This is a diagnostic run, not a quiet-window or Unreal performance certification; details and binary identities are in the unit note.

## Programs

From MP1 ([m7-mp1.md](m7-mp1.md)): per-object shading programs on both paths, by per-program tile lists.

1. **Reflection hits on another program's object** are shaded by the reflecting surface's program, with the hit's material inputs and the receiver's parameters (m7-mp1.md 1.4). RF10's reflected toon sphere is smooth, not banded: RT 22.92 on that population against UE HW-Hit Epic 10.42 and SW Epic 12.99 (whole reflector 12.34, better than both floors). The fix is a per-program hit pass: trace and record hits, shade them in per-program lists by the hit's program, then let receivers read the result. It would also take R1's inlined hit shading out of the receiver's kernel (R1's cost gap 11).
2. **Non-RT RF10** scores 62.27 against UE SW Epic 53.12: non-RT has no traced reflections yet (Reflections 1). R1n's software hits must use the same seam as R1's (`program::compose_list` replaces the hit's parameter load once); a second hit-shading site needs the same treatment.
3. **Temporal film frames** (`SoftShadows::Temporal`, not a default) still admit one program per frame.
4. **Limits.** 32 programs per frame (the binning mask is one u32); 64 draws per frame as before. Programs are per material, not per instance.
5. **Compile hitches.** The first mixed frame compiles one list pipeline per program (the frame's shading variant only), concurrently, and a reload's list pipelines recompile at the next mixed frame rather than in the reload transaction. Debug example build, first frame: 49.0 s RT and 24.8 s non-RT for everything, against API1's 11.6-43.8 s with one program. Not measured in a release build.
6. **Mixed tiles** run every present program's dispatch over the whole tile; the others' pixels return after one hit-record read. Overhead grows with the number of program boundaries on screen. Quarry, cache off: +0.06-0.11 ms of shading with two programs and +0.05-0.07 ms with eight (standard plus toon variants), above the design's 0.03-0.05 ms estimate. The pure dispatch overhead (identical-cost programs) was not isolated: every attempt was stopped by the lease guard (m7-mp1.md 3).
7. **GI cells.** Each reflecting program's cell pipeline scans the whole cell queue, so cell shading grows linearly with the number of reflecting programs. Unmeasured beyond the quarry rows; the queue is bounded (32 slots).
8. **Unverified:** mixed frames with streamed textures (`TextureResidency` other than `Full`; composed and naga-checked, never rendered), mixed frames with RT1d's visibility filter, motion and disocclusion across program boundaries under TR1 (only static frames and the example's fly path were inspected), and Hard (inline-ray) shading in mixed frames (the gates use Interactive and Virtual).
9. **Memory with many photoreal programs.** The eight-standard-program quarry run took available memory below the lease guard's 2 GiB twice (32 GB machine, other units running), where the toon-program run bottomed at 5.1 GiB. Not diagnosed: concurrent list-pipeline compiles, per-program GI cell pipelines and per-switch material churn are candidates.
10. **No Unreal comparison** of per-object materials: Unreal assigns materials per mesh natively; the 2x speed gate on matched workloads was not measured for mixed frames.


## Startup

P-SHC2 ([p-shc2.md](p-shc2.md)) has **no production structural split** and
does not meet the 1–2 s first-new-shader target. Its compile-only buffered-input
screen leaves 11.04 s RT / 16.93 s non-RT of native compilation after removing GI
preparation, reflection preparation and light visibility (single contended
samples, no producer passes, never rendered). Removing GI alone leaves
25.43 / 31.36 s. Other helper work and user shading remain; a broader split is
unproven. A staged, shading-model-independent user/engine request protocol still
needs implementation, complete-image identity and ARA1 frame-cost evidence.
The existing background fallback keeps the previous revision while compiling;
this is responsiveness, not a reduction in new-shader driver codegen.
Its extended watcher fixture passes exact old/updated-versus-sync/restored HDR
checks on both paths (286/313 old frames), but edit-to-readback remains
40.70/32.31 s in that contended fixture. The corrected game API probe is
26.18/24.08 s; these are distinct endpoints. Fresh-initial-program empty-cache
startup is 30.12/30.31 s, populated replay 1.16/0.89 s, with complete RGBA8/HDR
identity at matched frames 0/1. All six ARA1 game/PBR/quarry baseline replays
complete; no split frame-time A/B exists. Lib: 199 passed. Main runtime and
default shaders are unchanged; this unit verifies the existing fallback and
leaves the optimization open. **OPEN: frame hitches during background compile.**
While the new pipeline compiles in the background, old-revision frames still
stall: the worst gap between old-revision frames is 432 ms, and the worst
readback 97 ms (P-SHC2, contended). That is visible in a running game, so the
fallback isn't hitch-free yet.

P-SHC ([p-shc.md](p-shc.md)) fixes a mixed-program reload transaction gap: active
MP1 list pipelines and dependent glass hit shading now compile before publication,
and unused list variants stay lazy. The corrected game probe requires a real source
change; the old `return exposed;` seam had become stale after emission was added.
On the original-runtime/final release pair, complete game RGBA8 and HDR frames 0/47
are byte-identical on both paths; drained frame cost is within 0.3%. Eight complete
old-image readbacks during a new heavy background compile finish in at most
83.5/75.8 ms (RT/non-RT), without publishing the pending revision.

- **The latency target remains open.** Fresh edit/readback is 21.612 → 20.818 s RT
  and 23.770 → 21.143 s non-RT in the paired game screen, still far above 1–2 s.
  Native driver compilation accounts for over 99% of the profiled heavy pipeline
  API call. Loop hints, runtime bounds, opaque fixed counts and disabled optimization
  did not solve it; NoInline screens hit the 6 GiB guard. All are reverted.
- Cached repeats improve to 93.8/42.5 ms (RT/non-RT); the RT glass pipeline now uses
  the program cache. Cache-off first-image samples are 28.595 → 28.761 s RT and
  27.131 → 27.633 s non-RT, with an uncontrolled private cache. No cold-startup gain
  or engine-wide speedup is established. Full ARA1/quarry remeasurement remains open.
  The final compiler-quiet, fresh-initial-program / empty-application-cache pair is
  24.257 → 27.224 s RT and 30.023 → 24.183 s non-RT; populated replay is
  0.756 → 0.813 s / 0.779 → 0.811 s, with identical cached/uncached images.
  Those single samples likewise show no consistent cold-startup gain.
- Reload coverage is bounded: ordinary hot sets, the last mixed frame's active
  lists and the current dependent glass pipeline. Other lazy engine/cell/transparent
  variants and prelude transactions can still require first-use work. Separating
  unchanged engine work from programmable shading needs measured frame-cost and
  complete-image gates; it is not shipped here.

PC1 ([m7-pc1.md](m7-pc1.md)) persists engine and program Vulkan pipeline caches before
initialization. Full-image identity passes on game/quarry, RT/non-RT, dev/release.
The shared-machine release RT game sample falls from 48.771 s with an empty application
cache to 0.745 s with a populated one. These are single diagnostic samples with an
uncontrolled NVIDIA private cache, not a universal speedup or a QUIET-window claim.

- Cold startup remains expensive: release RT game was 48.771 s empty-cache versus
  37.497 s with persistence off. A real unseen program edit still took 31.335 s to
  readback; 95.7 ms of module work was followed by a 31.081 s first-use mixed-program
  list pipeline compile. Background/prewarmed creation of unseen variants remains open.
- CC1's [native stage screen](m7-cc1.md) attributes substantial driver work to GI
  lookup, reflection hit shading and non-RT traversal. Removing any one is not a
  shipping fix. Sharing the user surface/shade call sites lowered the paired native
  compile from 36.854/34.182 s to 23.582/15.883 s (RT/non-RT), with identical game
  pixels, but raised release frame time by 84%/16%; it was rejected and reverted.
  Splitting trace/hit shading into smaller independently compiled programmable
  stages and a correct background fallback remain open. Large cold-compile gains
  on both paths without a steady-state cost have not been established.
- CC1 retains a single-call-site GI refinement loop with unchanged sampling order.
  Completed game comparisons (both builds/paths, all cache modes) and quarry
  comparisons (dev both paths, release RT) are bit-identical, with no observed
  frame-time regression. Release game disabled-cache startup is 30.760 -> 21.356 s
  RT and 36.540 -> 22.370 s non-RT; populated-cache samples are 0.833 -> 0.630 s
  and 0.603 -> 0.571 s. The smaller pipeline was privately warm afterward, so the
  entire startup reduction cannot be credited to CC1. Cold compilation still
  takes tens of seconds. Release non-RT quarry, quarry empty/populated-cache runs
  and the complete new-edit matrix remain unverified after yielding to ARA1's
  held window; those gaps are not replaced with inferred numbers.
- A driver-private-cache hit can already make startup fast. The dev matrix shows no
  consistent extra benefit from persistence, and one non-RT warm sample is slower.
- PC1 measured warm quarry preparation at 9.479/27.306 s in dev and 4.415/15.236 s in release
  (RT/non-RT); resource/geometry/ray preparation and upload dominate non-RT. This unit
  changes no GI, acceleration-structure or asset-cooking algorithms.
- The timed quarry is the engine example with the complete registered scan asset and
  game props, using its game camera. Startup of the low-level viewer/quarry walk was
  not measured; its legacy program-only cache setter does not persist all startup kernels.
- PC1's program edit check uses the API, without GI/TR1, on RT. CC1 adds native
  driver/SPIR-V stage attribution on both paths; its fresh API edit/readback matrix
  was not completed. File-watch debounce, physical scanout and a separate non-RT
  reload timing remain unmeasured. Corrupt-file/key tests pass on CPU; GPU replay after
  corruption or an actual driver update remains untested. Cache files are bounded to
  64 MiB and concurrent writers retain the last complete snapshot.

## M8 GPU memory (P-MEM, 2026-11-09)

- [P-MEM](p-mem.md) retains exact cold-record/software-BVH packing, inactive post
  target allocation and unreachable spot cube-face removal. Complete HDR/PNG,
  RF, Q-walk and local-light transition hashes match. Logical GI capacity,
  directions, texture detail, 128-light capacity and shadow resolution stay fixed.
- **Ara's per-scene memory target remains open.** Warm process-local DXGI MiB:
  game RT 1131.047 -> 1035.047, non-RT 1435.797 -> 1339.797; PBR RT
  1199.047 -> 1103.047, non-RT 1375.797 -> 1279.797; 64 point lights RT
  1138.734 -> 1042.734, non-RT 1827.484 -> 1731.484; quarry daylight RT
  3375.547 -> 3343.547, non-RT 4177.422 -> 3944.234. Only quarry RT falls
  below the historical Ara comparator. Live allocation savings are different
  from reservation/local savings; 64 MiB block packing varies between replays.
- Remaining non-RT quarry live memory is 3699.730 MiB. Largest allocations are
  material mips (1024.022), walk records (447.518), packed software-GI geometry
  (about 311.934), source vertices/indices (303.673/298.345), raster cluster
  geometry (226.911), GI table (192), sun clipmap layers (192), GI sources (88)
  and TR1 histories (168.750). Their active consumers prevent simply deleting
  copies or reducing capacities.
- Next exact candidates: address-compatible geometry sharing/encoding; sparse
  physical GI backing with unchanged logical capacity; reuse of ancillary GI
  buffers across light-bitset growth; proven non-overlapping target aliasing;
  mixed point/spot physical atlas layout with cache and fade identity. Initial
  GI allocation cannot simply be deferred: lighting upload precedes GI prepare
  and would change the first frame's binding. Existing BLAS compaction and
  upload retirement already operate and are not new P-MEM savings.
- **CLOSED at the reported magnitude: packed-BVH quarry regression** ([P-BVHT](p-bvht.md)).
  A compiler-free, prebuilt packed-only pair on `dc0d4ea`, two ABBA blocks and
  16 tracks per binary gives service medians moving 8.7766 ? 8.8154 ms (+0.44%),
  cut 38.8012 ? 38.4471 ms (?0.91%), recovered 9.8808 ? 9.7777 ms (?1.04%).
  All three service means and GPU-span medians improve; 532/532 HDR and
  532/532 PNG cross-process witness comparisons match. P-VERIFY's +5.5%/+6.1%
  means do not reproduce; broad tails affect both binaries. Keep the ~95.140 MiB
  software-GI geometry saving. Small sub-percent costs and tail causality remain
  unproven; this is frozen-pair clearance, not a current-main speed claim.
  Ordinary PP1's baseline neutral-HDR assertion remains a separate P-POST gap;
  P-MEM's record-only sweep leaves that gate strict.


## M8 local lights follow-up (P-LTS3, 2026-11-09)

[P-LTS3](p-lts3.md) promotes no renderer change. Current-main profiling still
selects light_sample (22.9% sampled states), unit_normal (10.5%) and light-index
reads (7.5%); achieved occupancy is unavailable. BRDF hoisting, exact scaled-normal
reuse and tangent memoization showed no established benefit under contention.
A conservative froxel-plane culling prototype is archived, with 199 lib tests,
24 LC PNGs, 960p and odd-edge pairs passing identity; its remaining 1080p pair,
128/129 boundary, MP1, courtyard, full quarry and quiet timing are unresolved.
The 3.5 ms target on both paths remains open. No quiet before/after numbers or
complete quality promotion are claimed. Preserve the 128-shadow budget, every
contributing RT light and LC2 softness in the follow-up. Raw summaries and staged
baseline/C binaries live under D:/Coding/renderlab-work/p-lts3.

## M8 non-RT gather follow-up (P-GATHER, 2026-11-10)

[P-GATHER](p-gather.md) recommends retaining batched queue screening only on the
software path. Quiet 1080p moving medians: game 11.2180 -> 11.0572 ms, quarry
11.5544 -> 11.2573 ms. All 51 A/B HDR hashes and 43 PNGs match; inspected quarry
cut-to-settle has no missing or newly delayed bounce; 200 lib tests pass. RT
module text and gather entry stay unchanged. Uniform speed remains open: game
cut is +0.6122 ms, quarry moving mean has tails, and the unchanged RT early-still
phase remains variable (7.4641 -> 8.4665 ms pooled medians in this small cohort).
These are disclosed measurements, not a new visual defect or an Unreal victory.
The owned Cargo target cleanup is blocked by automatic approval policy; see the
unit report for the exact path and handoff.

## M8 Ara Showcase through the bridge (P-SHOW, 2026-11-10)

[P-SHOW](p-show.md): Showcase (Ara's sample scene, drawn through Ara's bridge) cost 22-28 ms RT and
43 ms non-RT at 1080p against Ara Epic's 3.0 ms. The GI cache and the non-RT virtual shadow cache
counted every skinned placement as a moved caster on every frame, even when its pose held, so a
scene with any skinned mesh never settled. Fixed by comparing pose revisions: RT 25.4-28.8 to
13.7-16.0 ms (still, moving and cut), non-RT shadow compute -1.2 to -1.6 ms; images within a few
codes. Open:

- **Non-RT GI still churns on skinned scenes (scaffolding).** Software-ray frames keep the old rule
  because the settled cache shows blocky GI cells on flat white faces, which the churn averaged
  away. Remove the switch in `gi_cache.rs` with the non-RT grain fix: about -12 to -16 ms on Showcase.
- **Animated characters relight one eighth of every GI cell per frame** (about 13 ms RT on
  Showcase while a pose changes). A relight bounded to cells whose gathers can see the moved
  caster's swept box is the next lever.
- **Non-RT glossy shading is 18 ms of Showcase's 21 ms shading pass.** Reflectance-weighted strata
  saved 12 ms but brightened the white glossy specks on the dark turntable; retry after the
  speck fix. Fade-band-only scaling saved nothing.
- **Standing scene:** `tools/showcase/showcase.py` drives Ara's player against a Renderlab
  checkout; the content is fetched by Ara's own sample tool, not committed here.
