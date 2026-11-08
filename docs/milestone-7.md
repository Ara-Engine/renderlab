# M7: global illumination, reflections and temporal reconstruction

Authorized by Islam on 2026-11-07, at main `cf001a5`. M7 runs in the main checkout. The M5-M6 shadow hillclimb continues alongside it in an isolated worktree (see [milestones-5-6.md](milestones-5-6.md)).

## Scope

- Indirect diffuse light (bounce GI) and sky/environment visibility.
- Glossy and specular reflections.
- Temporal reconstruction of the final image under motion.

Two M5 items fold in here, because GI and reflections replace the mechanisms they were going to use: environment visibility and glossy-motion quality. Transparency, volumes and atmosphere are out of scope unless a later update adds them.

## Acceptance

- **Quality floor:** Unreal 5.7 quality on every measure, static and in motion. That includes disocclusion, camera cuts, and moving lights and casters. Film quality is the target. References are path-traced and pixel-integrated, and include motion-integrated sequences.
- **Unreal comparators:** Lumen software and hardware GI, Lumen reflections and TSR, on matched scenes, cameras, lights and materials. Unreal 5.7.4 CL51494982 runs offscreen with no visible window.
- **Performance floor:** at least 2x Unreal at matched quality, measured end to end on both paths. Every speed claim gets a final quiet-window run.
- **Both paths are first-class.** Every gate is measured with hardware RT and without. Non-RT GI should rival other engines' RT GI.
- **No screen-space core lighting:** no SSGI or SSR as the core answer. Caching, temporal reuse, denoising and approximations are allowed when the final image passes.
- **Shading stays user-programmable.** GI and reflection inputs reach user programs as clean values. Record any constraint a design imposes.
- **Memory:** total VRAM stays below Unreal's on the matched scene.
- **Licences:** copy no Epic, Unity or NVIDIA RTX SDK code. ReSTIR- and SVGF-family methods need a patent review first.

## Execution

1. Design first, on CPU only. Write competing sketches and a measured plan against Unreal 5.7. Ground the design in docs/research/gi.md, reflections.md and reuse-temporal-denoise.md once they land, plus the existing non-RT machinery: the surface-keyed cache, cluster LOD and the virtual clipmap.
2. Implement in the order the design records, as separately measurable units, each priced per stage.
3. Check once per retained change, in batched runs on the shared GPU lease.
4. Push validated checkpoints. Record decisions and misses in the decisions trail.

## Ownership

- **Main checkout (M7):** lighting, environment, shading, and the new GI, reflection and temporal files.
- **Hillclimb worktree:** shadow files only. It doesn't edit M7's files.

## Decisions (2026-11-07)

Answered by Islam:

- **Resolution:** compare at native 1920x1080 with Unreal TSR at 100%. 4K upscaled output comes later, in its own unit, once our temporal reconstruction exists.
- **Lumen floor:** Lumen Epic is the floor on both paths. Lumen High is measured for the cost curve.
- **Bounce light from user shaders:** user programs may supply an optional `scatter()`/`emission()`. A program that omits them still blocks light but reflects none.
- **Cook-time occlusion:** short-range occlusion for static geometry may be precomputed at cook time for the non-RT path. Budget about 50-150 MB, which counts toward the VRAM-below-Unreal gate.
- **If non-RT GI misses 2x speed:** record the measured miss, and make non-RT GI the default anyway if it beats Unreal's quality on every measure. Quality is never lowered to reach 2x.

Decided by root:

- **Settle limit:** the 8-frame limit for soft shadows also applies to GI and reflections.
- **Path tracer:** an in-process GPU path tracer is the quarry reference. It is cross-checked against pbrt or Mitsuba on small scenes.
- **Temporal file ownership:** `src/film_accumulation.rs` and M7's temporal reconstruction belong to M7. The shadow hillclimb doesn't edit them.

Open, needs a human: a patent review of the world-space cache against US 9,754,407, and of any ReSTIR- or SVGF-family method, before such code ships. Agents won't treat a design as cleared on patents. Superseded 2026-11-07 by the patent policy below: Islam accepts design-arounds, unclaimed parts and, as a last resort, patented methods. So P0's recorded claim mapping is enough for a design-around or unclaimed-part tier to become a default. No separate human review is needed first. P0's mapping is an engineering reading, not legal clearance; Islam can still commission a legal review at any time.

Patent exposure flagged by the research notes (2026-11-07), each Islam's decision before adoption. No exposed technique goes into a default until he rules:

- NVIDIA US 11,315,310: ReSTIR combined with a world-space cache. This is the Solari / GI-1.x shape and close to our surface-cache GI sketch. Don't commit to that exact combination.
- NVIDIA US 10,116,916: variance clipping for temporal reuse, in force to 2037. Also the SVGF / gradient-estimation patents.
- Adshir/Snap and NVIDIA ReSTIR patents, for reflections.
- US 9,754,407: world-space cache. Its review is still open, as noted above.

Code licences: RTXGI 2.x, SHaRC, NRC, NRD and DLSS / DLSS RR are reimplement-only, so none of their code may be copied. DDGI papers are CC BY-ND, so write our own code from the text. Falcor's SVGF pass is BSD-3 and may be copied with attribution, but the SVGF patent question still applies.

Islam's decisions on design draft 2 (2026-11-07):
- **GI cache:** it may become a default on the same basis as the surface shadow cache while the US 9,754,407 review is pending. If the review finds a problem, both are revisited.
- **Adshir/Snap:** record the non-RT, non-planar reflection miss for now. Ship planar and RT reflections, and review the patent later.
- **Denoisers:** if the unflagged denoisers in D0 fail their gates, record the miss and spend on more samples and better caches first. Ask for a review of SVGF-type filters only if that also fails.
- **`reflection_lobe()`:** the hook is GGX-shaped. A custom style that isn't GGX gets its nearest GGX lobe, or no traced reflections. This is accepted and recorded as a constraint on programmable shading.

Root decisions:
- **Cache code:** the GI cache code stays in its own M7 files, copied from the shadow cache. Once the shadow hillclimb ends, the two may be merged into a shared module.
- **RT comparison memory baseline:** M7 measurements use the non-default fast setup, traced primaries with hardware Interactive shadows: 2,641 MiB reserved, against Unreal HRT's 5,410. The RT-device default remains raster plus Exact; [RT1c](rt1c-rt-default.md) and [RT1d](rt1d-shadow-filter.md) record the failed promotion gates.
- **DLAA preset K:** if it's available in the installed Unreal, it may appear in the Unreal runs as an informative row only. It is never the pass bar, and DLSS is not linked into Renderlab.

## Patent policy (Islam, 2026-11-07)

"Design around them, and consider taking parts if needed and more, and if all fails then we can just use them." This replaces "hold until Islam rules" for the flags above. Apply it in this order:

1. **Design around first.** Prefer methods with no patent exposure. Examples:
   - GI without the ReSTIR-plus-world-space-cache combination of US 11,315,310;
   - history rejection other than variance clipping (US 10,116,916);
   - denoisers outside the SVGF and gradient patents;
   - reflections outside Adshir/Snap and ReSTIR.
2. **Unclaimed parts are allowed** where the claim needs the full combination. Examples: resampled reuse alone, or a world-space cache alone. The design doc notes which claim each part avoids, and why.
3. **Fall back only on evidence.** Use a patented method only if every design-around candidate measurably fails the quality gates (the Unreal floor on every measure, static and in motion, on both paths), or is clearly worse in the final image. Record:
   - the failed alternatives and their numbers;
   - the patent number.

   Keep the patented method behind a named setting where practical, so it can be swapped out later.

The quality bar is unchanged. Avoiding a patent never justifies shipping below the floor. Record any design-around's measured cost in quality or speed.

This updates the draft-2 rulings above. Non-RT non-planar reflections are no longer just a recorded miss. Design around Adshir/Snap first: the LOD-BVH and SDF-brick tiers may proceed where their parts avoid the claims, and the patented form remains the evidence-based fallback. Likewise for denoisers, try more samples and caches and unflagged filters first; SVGF-type filters are the evidence-based fallback.

## Unreal comparison content on E: (cleanup record, 2026-11-07)

`E:/renderlab-work/m7-u0/` holds the recooked Unreal 5.7.4 project, which has mesh distance fields. The following were deleted because they can be regenerated: the staged build under `Saved/StagedBuilds` (a byte-for-byte duplicate of `ue-package/`), `Saved/Shaders`, `Saved/ShaderDebugInfo`, `Saved/Temp`, `Saved/Logs`, `Intermediate/` and the package's crash-report configs. That freed 1.66 GB.

The rest is kept, about 6.0 GB:

| Kept | Size | Why | Needed by |
|---|---:|---|---|
| `ue-package/` | 1.45 GB | packaged build the runs use | TR0, M7-U0b, R0a, R0b, TR1, G1/G2 and R1 comparisons, integrated acceptance |
| `ue-project/Content` | 0.98 GB | source content for importing benches | M7-U0b (bench import), R0a (reflection scenes) |
| `ue-project/DerivedDataCache` | 1.08 GB | avoids a slow distance-field recook | M7-U0b, R0a recooks |
| `ue-project/Saved/Cooked` | 1.81 GB | incremental cook base | M7-U0b, R0a recooks |
| `study/`, `scenes/`, `reference/`, `fixture/`, `mitsuba*/`, `tools/` | about 0.7 GB | summaries, scenes, references, Mitsuba tooling | all M7 comparisons |

Final delete: when the last Unreal-comparison unit (integrated M7 acceptance) finishes, remove `ue-package/` and `ue-project/`. Keep only the summaries, the inspected PNGs, and the scripts and configs that rebuild the project (`scripts/m7-u0-*.py`, plus the config deltas recorded in docs/m7-u0.md). The DDC and `Saved/Cooked` may be deleted earlier, once no more recooks are planned, after R0a.

RT shadow floor (Islam, 2026-11-07): the RT path's shadow floor is Unreal's matched RT technique, hardware ray-traced shadows (HRT), not VSM. Measured against HRT, the fast RT setup (traced primaries plus Interactive, 8 rays) wins on every recorded measure:

| Measure | Fast RT setup | Unreal HRT |
|---|---:|---:|
| False shadows | 27 | 743 |
| Lit darkened | 0-2 | 724 |
| Leaks | 0 | 2,718 |
| Missed film shadows | 9,245 | 23,931 |

So it becomes the RT-device default once its motion gates against HRT are confirmed. The fast setup's gap to VSM on false shadows (27 against 11) stays recorded, but it doesn't block. VSM's low count comes from blurring toward lit: it misses 27,071 film shadows and leaks 5,537.

Unreal packages, update of 2026-11-08 (from M7-U0b). Never mix packages within one measurement.

| Package | Identity | Use |
|---|---|---|
| `E:/renderlab-work/m7-u0b/ue-package` | cook/b | SV, S4, S5 and S2G comparisons (R0a, TR1, G1c) |
| `E:/renderlab-work/m7-u0b/ue-svsky-package` | cook/sv-sky | G1c's diffuse-only SV sky comparator |
| `E:/renderlab-work/m7-u0/ue-package` | original U0/TR0 | U0 and TR0 measurements |

`m7-u0/ue-project/Saved/StagedBuilds` was a byte-identical copy of ue-svsky-package (proof: `m7-u0b/package-retention-proof.json`) and is deleted, freeing 1.42 GB.

Retention correction (root, 2026-11-08): path-traced reference archives, plus the Unreal native captures that later units score against, are not "raw dumps". Keep them until integrated M7 acceptance. That covers the S2G, Q-disocclusion, Q-sun and U1b specular references, and TSR's Q-disocclusion capture. The 5 MB delete rule applies to intermediate buffers, rejected attempts and duplicates. TR1 found that deleted S2G, Q-disocclusion and U1b references blocked its remaining gates. Regenerate those once, compactly (half-precision where the scorer allows), into `E:/renderlab-work/m7-refs/`, and keep them there.

Package update (2026-11-08): R0a's reflection fixtures RF1-RF10 use `E:/renderlab-work/m7-r0a/ue-package-c` (package C). Keep package C and the source `ue-project/Content`. R0a needed no further recooks, so the Unreal project's DerivedDataCache and Saved/Cooked are deleted, freeing 2.88 GB.

ES1 occlusion ruling (Islam, delegated, 2026-11-08): keep sky specular occlusion by direct sky visibility, the physically correct form, which can't double-count bounce light after G2.
- ES1's record keeps the unoccluded diagnostic variant's quarry numbers as a comparison point.
- The cache, with bounce light and ES1, may become the default only if it equals or beats both the cache-off image and that unoccluded variant on every quarry population, on both paths, static and in motion.
- If it doesn't, record the miss and bring the trade-off back to Islam. Never switch to unoccluded quietly.

Shipped anti-aliasing (IQ1, 2026-11-08, [iq1-quarry-image.md](iq1-quarry-image.md)): **the default has no anti-aliasing on either path.**
- Both paths ship at 1x with TR1 off and anisotropy 1. The RT device's code default is raster primaries with Exact shadows; non-RT selects the visibility buffer with Virtual shadows.
- Every M7 gate except TR1's own therefore scores single-sample, un-anti-aliased images against pixel-integrated references. That covers the edge, silhouette and interior populations alike.
- On the quarry (13 M triangles, no covered pixel's 3x3 block on one triangle), the missing pixel integration causes Islam's black pinholes (about 1,300 per frame) and the grain.
- TR1 removes them: 1,290 → 40 pinholes still, 10–34 per Q-walk frame, every population better at both exposures on both paths.
- IQ1 proposes TR1 as the default on traced and visibility-buffer frames. The switch is TR1b's gate.

Default anti-aliasing ruling (Islam, delegated, 2026-11-08). TR1 becomes the default on both paths (traced and visibility-buffer frames), because the no-AA default it replaces is worse on every measure (IQ1, docs/iq1-quarry-image.md). Conditions:
1. TR1 still misses the Unreal TSR Epic floor on moving edges and newly revealed pixels. The default is never presented as meeting the Unreal bar until it does; TR1b keeps closing those gaps.
2. A named setting switches TR1 off.
3. Before the default flips, the claim that TR1 is better everywhere must also hold on the quarry walk with the real sky visible behind silhouettes, on both paths. If TR1 is worse than no-AA anywhere there, the default doesn't flip, and it's reported instead.
4. Cost tails above the half-TSR ceiling stay recorded as a miss.
5. Earlier quality gates are re-scored on the new default image, noting which numbers moved.
IQ1 also found the RT-device code default is still raster primaries, which TR1 doesn't reconstruct. A TR1 default helps RT users only once traced primaries are the RT default (RT1d).

TR1 visible-sky gate result and ruling (Islam, delegated, 2026-11-08). TR1b's 40-frame visible-sky quarry walk failed condition 3, so TR1 stays opt-in.
- Against the path-traced reference, TR1 is worse than no-AA in 149 of 256 rows on both paths. Most of that comes from the lighting error both images share (unoccluded sky in shadow), not from AA.
- Against each path's own 64-spp integrated image, TR1 wins 219 of 256 rows and loses 31. The losses are the cold first frame, revealed-pixel pockets at ages 2-4, rare single-pixel maxima while moving, and a still-frame settle step.

Ruling:
- Judge the AA change against each path's own 64-spp integral, because the shared lighting error isn't TR1's.
- Flip only when no row is worse than no-AA, on both paths, static and moving.
- Keep the path-traced comparison in the record, and re-score it once G2's sky occlusion and bounce make the cache the default, so the lighting gap shows separately.
- Fix the losses in order of size, recording each fix's effect. If a row can't be fixed without hurting others, report it back; never relax the gate.
- Build an Unreal quarry level with the sky visible as a separate short unit, with its package and captures on D:, then delete Unreal's derived data and build cache once the cook is verified.

G2 ruling (Islam, delegated, 2026-11-08): keep the blocked (direct-visibility) sky specular, and keep the cache opt-in. The next unit, G2b:
1. Fixes crevice mouths on both paths, preferring a short per-pixel visibility check or finer cells over any screen-space core-lighting trick. Study how Lumen and other engines handle near-field occlusion at cell boundaries before building.
2. Cuts cost toward the ~0.45 ms budget, with camera cuts spread over frames and bounded in time, not just in frame count. Everything is timed on the quarry on both paths.
3. Makes the cache the default only when it equals or beats both cache-off and the unblocked variant on every quarry population, both paths, static and moving, scored on TR1 frames. Any remaining miss is recorded and brought back; the gate is never relaxed.

## Overnight complete-base push (Islam, 2026-11-08, about 19:00)

"I want global illumination that's better than Lumen. I want every single feature that we talked about done… within 3 to 10 hours, by the time I wake up tomorrow… an engine that is ready to render games on the quality of Unreal… We can improve further after that, but everything should have a good base."

Target by about 05:00 local: every feature built and on by default, on both paths.

| Feature | Unit |
|---|---|
| TR1, plus an upscaling preset | TR1b |
| GI: sky cache with bounce and ES1 | G2b |
| Reflections | R1 |
| Transparency, plus particles | TP1 |
| Atmosphere and fog | AT1 |
| Skinned meshes | SK1 |
| Many lights | LC1 |
| Decals | DC1 |
| Post-processing | PP1 |
| Engine-facing API freeze and example scene | API1 |

The default rule, time-box and verification for this push are in AGENTS.md. They supersede the stricter default gates above for this push. Every gap to Unreal and to the reference is recorded in `docs/m7-gaps.md`.
