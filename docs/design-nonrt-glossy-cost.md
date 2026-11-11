# Non-RT glossy reflections: three changes to screen

Reading-only study, 2026-11-11, on main `48962fa` (patent policy suspended, `AGENTS.md`). No build, no GPU, no new measurement. It extends [design-glossy-rate.md](design-glossy-rate.md) (rate and reconstruction, both paths) and [design-nonrt-reflections.md](design-nonrt-reflections.md) (software traversal) with what the code at this baseline allows now that resampled reuse is open. PR #16 (sky-map lookups) and PR #17 are not merged and are not assumed.

Labels: **C** read from code (file and line on `48962fa`); **M** measured in a repo document (named); **S** reported by a source; **E** this study's estimate or proposed parameter. Every arithmetic forecast is **E**, not a measurement.

**Result.** Run one cheap timing split first (section 3), because the three changes depend on a number nobody has measured on Showcase: how much of the 18.6 ms is traversal. Then screen, in this order of expected value: (1) four strata plus reconstruction, about 6-10 ms; (2) replace the reflection BVH2 with the GI/S4 BVH4, about 1-5 ms and byte-comparable hits; (3) take hit light visibility from GI cells instead of tracing it, about 1-5 ms and visibly riskiest per millisecond. The 2.2-2.6x gap to Unreal in the brief was measured on the quarry, which traces no reflection rays (section 1), so none of these closes it; they target Showcase-class content.

## 1. What the 18.6 ms is made of

**M, [p-show.md](p-show.md) per-pass table** (RTX 4070 SUPER, 1080p, Ara perf profile, medians): non-RT shading 20.78 ms, with reflections off 2.18 ms, so glossy reflections are `20.78 − 2.18 = 18.60 ms` (RT: `7.76 − 2.24 = 5.52`). The brief's 21 ms / 6 ms are the same quantity rounded up. The table does not say which sky it used; [p-show2.md](p-show2.md) measures a map sky adding about 16.6 ms of RT shading (25.1 against 8.5, M), so the map-lookup cost (PR #16's subject) sits inside figures taken under a map and must be removed before any screen below is read.

**What one glossy pixel does (C).**

| Step | Where |
|---|---|
| Strata count `clamp(ceil(48 × roughness), 4, 16)`, 1 below roughness 0.05; cone `alpha / sqrt(count)` | `shaders/reflections.wgsl:156-158`, constants `:28-35` |
| Per stratum: GGX visible-normal sample, `rs_closest` (software ray), `rl_sky` on a miss | `reflections.wgsl:182-202`; seam `src/program.rs:129-141` |
| Hit reconstruction: three vertex fetches, decals, ray-cone gradients | `reflections.wgsl:95-128` |
| `surface` and `shade` of the hit; per light on the hard-shadow path (the sun) a shadow ray, `rs_blocked`, any-hit; shadowed local lights take `rl_local_shadow` (maps) instead | seam `program.rs:79-81` inside `light_visibility` (`shaders/program_api.wgsl:96-100`), `reflections.wgsl:76-89`, `reflections_software.wgsl:106` |
| Mean over strata into `rl_specular`, mixed into the program's specular | `reflections.wgsl:203-207`, `shaders/programs/standard.wgsl:128-134` |

So a fully hit pixel with the sun on the hard path is 16 closest-hit walks, 16 any-hit walks and 16 hit shadings. **Walks:** every ray loops over a flat instance list testing each world box (`reflections_software.wgsl:41-45`), transforms into object space for each box it enters, then walks that mesh's median-split BVH2 (at most 4 triangles a leaf, `src/reflection_software.rs:18`; 64-byte nodes, 48-byte triangles, `:20,:24`) with a 32-entry private stack (`:11`, `:38`). Meshes over 2^19 triangles and skinned meshes get no software geometry (`reflection_software.rs:16`).

**Measured shares are for another scene.** Traversal was about 31% of the 2.03 ms game-base shading pass in P-NRT2 and about 54% of a 3.0 ms pass before the vec4 loads in P-NRT (M, quoted in [design-nonrt-reflections.md §1](design-nonrt-reflections.md)). Showcase's split between closest-hit walks, shadow walks and hit shading is **not measured**; below, the traversal share `t` of the 18.6 ms is taken as 0.3-0.6 (**E**, bracketing those two).

**Closed already, do not redo.** Workgroup ray compaction (P-R2): loop-count max/mean 1.08 and 1.14 against a 1.4 gate (M, [p-r2.md](p-r2.md)). Share-scaled glossy rays: −12 ms of non-RT shading but brightened the white glossy specks (M, p-show.md, "Rejected"); the speck source was fixed afterwards by hit regularization (P-DEFECT, M: held-frame bright outliers 24 → 2), so a lower count has not been re-screened on the fixed baseline ([m7-gaps.md](m7-gaps.md) P-DEFECT, "glossy method change").

**What this cannot fix.** The 2.2-2.6x gap in the brief is the quarry against Unreal; the quarry traces no reflection rays because every scan exceeds 2^19 triangles and the coverage rule zeroes the bands (M, m7-gaps.md Reflections item 1). Showcase non-RT is 33.1 ms after the relight scaffolding was deleted (M, p-show2.md) with 20.8 ms in shading; removing all 18.6 ms of glossy cost would still leave about 14.5 ms.

## 2. The three changes

Shared formula (**E**): `C = 18.6 ms`, saving `= C × f × (1 − r) − O` with `f` the affected fraction, `r` the remaining work ratio and `O` the added overhead. Savings are not additive: change 1 removes rays that 2 and 3 would have made cheaper.

### Change 1: four strata at the receiver, current-frame normalized reuse, reflection-aware history

**Idea.** Cap eligible glossy lobes at four strata (`RL_GLOSSY_MAX`, `reflections.wgsl:34`) and restore the lost samples by a fixed-tap gather of neighbours' incident light, normalized by each tap's lobe weight over its proposal density (the Frostbite and Heitz-Hill-McGuire form in design-glossy-rate.md §2), followed by a temporal stage keyed on reflected-content motion. If that is not enough, use the open family: one new ray per pixel plus reservoir resampling of neighbour and previous-frame hit samples, re-shading each reused hit from the new receiver (the hit's sun visibility is receiver-independent, so it travels with the sample and no shadow ray is re-traced).

**Arithmetic (E).** Tier A (4 of 16, fixed taps): `r = 4/16`, `f = 0.6-0.8`, `O = 1-2 ms` (Frostbite reported 0.81 ms resolve plus 0.30 ms temporal on a PS4 at 1600×900, S, from design-glossy-rate.md): `18.6 × 0.6 × 0.75 − 2 = 6.4 ms` to `18.6 × 0.8 × 0.75 − 1 = 10.2 ms`. Tier B (one new ray, two reused hits re-shaded): per-ray cost normalized to 1 for a new ray and `1 − t` for a re-shaded reuse (no walk, no shadow ray): `1 + 2(1 − t) = 2.1` units at `t = 0.45` (range 2.0-2.4 for `t` 0.3-0.6) against 16, so `r ≈ 0.13-0.15`: `18.6 × 0.8 × 0.86 − 3 ≈ 9.8 ms` with an assumed `O = 3 ms` for candidate selection, MIS weights and history. Tier B has no measured basis for `O`; treat it as an upper bound.

**What could look wrong.**
- The reflected sun's glow and thin bright rails get thinner, or the white specks return: fewer rays make each rare bright hit weigh more, and the cone (`cone_alpha = alpha / sqrt(count)`, `:157`) widens with the count drop, so a lower count can look smoother only because the footprint grew. Freeze the old footprint while screening (design-glossy-rate.md §1).
- Neighbour reuse crosses an object boundary in the reflection: a bright foreground hit smeared into a dark receiver. A receiver-side check on hit distance and program id is needed; mirror (`count = 1`) pixels must stay un-reused, since a neighbour's mirror hit is the wrong direction for this receiver.
- Motion: RQ1 already showed that four rays leave blocky smears on moving glossy metals (m7-rq1.md, M) and the history is receiver-keyed (TR1, `temporal_resolve.wgsl:427-466`); the open item is reflection-aware reprojection (m7-gaps.md item 14). This is also where Unreal is ahead on flicker (brief). A reservoir tier adds correlation noise that looks like slow-moving blotches.
- Custom programs that bypass `brdf` get no regularization (P-DEFECT constraint) and gain the most variance from a lower count.
- Mixed-program hits are still shaded by the receiver's program (m7-gaps.md item 3); reuse would hide that gap, not fix it.

**Cheapest screen.**
1. One-line: set `RL_GLOSSY_MAX` to 4 and compare, on `examples/reflection_matched.rs --stage --metal-balls --glass-balls` (still and the 48-frame flight) against its 16,384-spp reference, both paths, with the old cone held fixed (replace `count` by 16 in line 157 only). A diagnostic variant with the cone recomputed shows how much of any "pass" is footprint. This needs one GPU lease; it is the P-SHOW2 screen that was built on a local branch (`RENDERLAB_GLOSSY_MAX`, `t3/p-show2-screens`, not on `origin`) and never run.
2. Without a build of the new pipeline: dump `rl_specular` and the hit distance per pixel at 16 strata for the still frame, and replay on the CPU: take 4 of the 16 strata by pixel, run the fixed-tap gather, and compare to the 16-stratum frame and the reference by crop (glow, rails, gold fine band). This needs a small diagnostic shader write, not the new pipeline. Reject if the gather cannot get within P-DEFECT's outlier count (2 held, reference 3) on the turntable.

**Patent flag (note only).** Reservoir resampling is the family P0 maps to US11315310B2 claim 1; spatial-before-temporal filtering with auxiliary data to US10600167B2 claim 1 ([design-glossy-rate.md §3](design-glossy-rate.md)). Policy is suspended; recorded here as the habit asks.

### Change 2: trace reflection rays on the GI/S4 BVH4

**Idea.** The non-RT path already uploads a second, better tree: per-mesh BVH4s from a binned SAH build, eight triangles a leaf, byte-quantized child boxes, built from the drawn mesh at full detail when `Blas::exact` is set (`src/gi_bvh.rs:49`, `:43-70`; traversal `shaders/gi_rays_software.wgsl:65-160`; also serving S4 sun visibility, `src/sunvis_s4.rs`). Reflection rays use a separate median-split BVH2 packed each frame (`reflection_software.rs`). Move reflection rays (closest and the hit shadow rays) onto the BVH4, add the drawn primitive index to its leaves, and keep today's shading unchanged.

**Arithmetic (E).** `t` 0.3-0.6, traversal speedup `s` 1.3-2 (Ylitie et al. report about 1.5-2x for compressed wide trees over binary on incoherent rays, S, from design-nonrt-reflections.md; GC1 found fewer node and triangle tests for SAH over median splits with the same hits, M, m7-gc1.md, no ratio quoted): `18.6 × t × (1 − 1/s)` = `18.6 × 0.3 × (1 − 1/1.3) = 1.3 ms` to `18.6 × 0.6 × (1 − 1/2) = 5.6 ms`; `O ≈ 0.3 ms` for the extra primitive-index load: net about **1.0-5.3 ms**. Also removes one geometry copy and the per-frame reflection packing (C: `reflection_software.rs` re-packs the instance list each frame), and halves the work in a mixed scene only where both trees were built; memory effect not measured.

**What could look wrong (visible only at seams and ties).**
- **Identity.** `gi_traverse` returns the triangle's vertex indices, not the drawn primitive (`gi_rays_software.wgsl:179-192`, `gi_hit_primitive` returns none). A reflection hit is `(placement + 1, primitive, u, v)` (`reflections_software.wgsl:8-9`); a leaf-triangle to primitive table (4 bytes a triangle) is needed, or the build must keep primitive order.
- **Edge slack.** The reflection triangle test has a 4e-6 barycentric slack because Möller-Trumbore leaked sky through shared edges (`reflections_software.wgsl:15-18`); the GI test has none (`gi_rays_software.wgsl:42-56`). Porting the slack changes hits only at edges. Exact-tie order changes with any new tree: use a `(t, placement, primitive)` tie rule before comparing images (design-nonrt-reflections.md R1, step 1).
- **Coverage.** The reflection BVH2 skips meshes over 2^19 triangles; S4's game-class gate excludes skinned meshes and caps the scene at 2^21 triangles (`sunvis_s4.rs`). The two coverage rules must be reconciled or reflections gain or lose geometry silently; the coverage rule exists so no ray leaks sky through a missing mesh (m7-gaps.md Reflections 1).
- Quantized boxes must be conservative (round lower down, upper up) or hits near box faces are lost.

**Cheapest screen.** CPU replay, no GPU: dump one still frame's reflection rays (origin, direction, tmin) and the two packed buffers; run both trees on the CPU with the same tie rule; compare hit records (must be identical away from edges), nodes visited, triangles tested and bytes fetched per ray. Reject if bytes fetched fall less than 25% (the threshold design-nonrt-reflections.md used) or if any hit differs away from an edge. Then one GPU lease on Showcase: shading span with the BVH4 against the BVH2, hash-compared frames.

### Change 3: take a reflection hit's light visibility from GI cells

**Idea.** Sharing hits themselves between GI and reflections is not available: GI rays start at cell centres in fixed quadrature directions (`gi_rays_software.wgsl`, `gi_cache.wgsl`), reflection rays start at a receiver along a view-dependent lobe, so their hit points do not coincide. What GI does hold that hits recompute is per-cell light visibility: the `gi_light` pass stores one bit per light per cell (`shaders/gi_cache.wgsl:1605-1626`, `gi_visible_within` from the cell's origin). Let a glossy-stratum hit read the bit of the cell at its point and normal (`gi_find`, `shaders/gi_cells.wgsl:312`) instead of tracing `rs_blocked`, falling back to the traced ray on a cell miss. Programs still run `surface` and `shade` on the hit; only the engine-supplied visibility input changes, so programmability is unchanged (`light_visibility` is already engine-owned, `shaders/program_api.wgsl:96-100`). The same lookup could supply the hit's bounce light, which would also fix RQ1's gap 18 (hit lighting, still-image error 4.78 → 4.35, M, m7-gaps.md) at a cost.

**Arithmetic (E).** A shadow walk costs `a` of a closest-hit walk, `a` 0.5-1.0 (an unoccluded light ray must finish its walk; a closest-hit ray narrows `best.t`), so the shadow share of traversal is `a / (1 + a)` = 0.33-0.50 for one light. Saving `= 18.6 × t × a/(1+a) − O`: `18.6 × 0.3 × 0.33 = 1.8 ms` to `18.6 × 0.6 × 0.50 = 5.6 ms`, minus a lookup cost: a per-hit cache lookup measured **+0.62 ms of quarry shading with nothing traced** on RT (M, m7-gaps.md item 8, a code-generation effect), and G2b's profile put the cache bucket walk at the top of its shading hotspots (M, m7-g2b.md as cited in m7-gc1.md). Net **about 1-5 ms**, more with several hard-shadowed lights; shadowed local lights already use maps at hits and are not affected.

**What could look wrong.** Cells are coarse and visibility is one bit at the cell origin, so reflected shadow edges become blocky or shift, thin occluders leak, and contact shadows near a hit's own receiver bias (the cell may sit across the occluder). The lobe blurs that for rough surfaces and not for mirrors, so keep mirror and low-roughness strata on the ray. A cell miss in a part of the scene GI has not visited needs the traced fallback, which also pins the worst-case cost. Cell visibility is for the lights the cache shades; check it covers the sun and shadowed local lights that `hard_shadow` and `rl_local_shadow` serve.

**Cheapest screen.** Two steps, the first needing no new code beyond one constant. (a) Cost-only: force the hit visibility to 1.0 (the `REFLECTION_VISIBILITY` seam, `program.rs:79-81`) on Showcase and the matched scene, both paths; if the shading span falls by under 1.5 ms, reject without looking at an image. This is the same kind of cost-only screen P-SKY used (PR #16 table). (b) If it passes, a CPU check of cell resolution: for the matched scene's hit points, compare cell-bit visibility to the traced bit (needs a dump of hit position and the cell table; fraction of disagreeing hits weighted by lobe weight).

## 3. Recommendation

**First, one lease, no new pipeline (the numbers every estimate above waits on).** On Showcase non-RT, after PR #16 or with `--sky constant`, run three ablations in one process and read the shading pass: (i) hit shadow forced to 1 (change 3, step a); (ii) hit shading replaced by the sky (`surface`/`shade` skipped, rays still traced); (iii) both. The differences give the any-hit walk share, the closest-hit walk plus reconstruction share and the shading share, replacing the 0.3-0.6 bracket for `t`. Pair with change 1's constant-4 screen in the same lease (section 2, step 1), since a faster, visibly equal four-stratum image would shrink the value of every per-ray saving below it.

**Build first, by expected value:**
1. **Change 1, tier A**, if the constant-4 screen holds on the matched scene. It is the only one that can take most of the 18.6 ms, and the 4-stratum variant is a few lines. Switch to change 2 if the screen shows lost reflected sun, returning specks, or moving-metal smear (RQ1's failure), because those are reasons to keep the rays, and cheaper rays are the answer then.
2. **Change 2** unconditionally in order if `t` from the split is 0.4 or more: it is the only lossless step (hits are the same away from edges), software-only (the RT path gains nothing), and it multiplies with change 1.
3. **Change 3** last; it is the only one that trades visible shadow detail for time, and its cost-only screen is cheap enough to run first and drop.

**Switch from change 1 to change 2** when: the four-stratum screen fails any of glow, outlier count (P-DEFECT: 2 held against 3 in the reference) or motion smear on the turntable and gold sphere flight; or when tier A passes but its reconstruction overhead `O` exceeds half its saving. **Do not** reopen P-R2 count compaction (closed, 1.08-1.14).

## 4. What reading cannot determine

- Showcase's split of the 18.6 ms between closest-hit walks, shadow walks and hit shading: measured only on game-base. The `t` bracket of 0.3-0.6 is a guess between two game-base figures from older code.
- Any image result: no variant was built, rendered or compared. Nothing here predicts how the four-stratum image looks on the fixed regularized baseline.
- Hit-point agreement between GI cells and traced visibility (change 3), and the per-ray bytes saved by the BVH4 (change 2): both need the CPU replays described, which need ray and buffer dumps that exist only on the Windows machine.
- How the map-sky lookup cost (PR #16) shifts the split on Showcase: its reported savings are on a Radeon 760M, not the RTX 4070 SUPER.
- Whether reservoir reuse (tier B) leaves a visible correlation pattern; that is an image question with no estimate behind it beyond the formula above.
