# Unreal failure catalogue and comparison-shot checklist

Reading-only study at Renderlab main **`7bd8b9abc403d3144c940f09a94112189b785035`**, after **v0.1.3**. The owner's bar is “visual fidelity that looks better than unreal's and solves all unreal's issues like ghosting”. This catalogue makes that a testable image-and-sequence checklist, not a claim that every listed artifact appears in every Unreal project or that Renderlab already avoids it. No scenes, builds, captures or GPU measurements were produced here.

**Evidence labels:** **R** is reported by the linked primary source, including code constants; **M** is measured/observed in the linked repository report; **E** is this study's estimate, forecast or proposed shot parameter. All new scene dimensions, trajectories, durations and rankings below are **E**, not measurements. Versions, issue numbers, shot IDs, hashes and line numbers are identifiers. **Documented limitation**, **configuration/content fix**, **mitigation** and **version-specific bug** have different meanings; a workaround that removes an effect does not preserve its quality.

**Current comparison evidence is narrower than this list.** [P-YARD2](p-yard.md) compares quarry, S4 door and S5 mirror. **M:** Renderlab's quarry motion residual is **4.39 codes RT / 4.44 non-RT**, versus UE Epic **3.68 HW / 3.69 SW**; Renderlab has visibly grainier dim walls. **M:** both Renderlab paths avoid the S4 thin-wall leak and broken S5 mirror seen with SW Lumen; HW Lumen also avoids those failures. Neither engine visibly trails the sliding door in that capture. These are the starting witnesses, not an engine-wide superiority result.

## Rules for a fair failure reel

- Use matched **UE 5.7.4 Lumen + TSR Epic** and the registered Renderlab settings as the first baseline. Record HW-Lumen/RT and SW-Lumen/non-RT separately. Use the best applicable documented UE fix as a second labelled control; if it cures the problem, compare against that corrected image and report its cost. MegaLights, forward-lit translucency and Nanite tessellation are explicitly separate feature configurations, not silently attributed to the existing P-YARD preset.
- Match camera matrices, world units, animation times, light radiometry and extent, exposure, output and internal resolution, shutter, tone curve and texture filtering. Keep film/exposure fixed while diagnosing lighting lag. Native-output comparison and equal-output upscaling comparison are separate tests. Proper motion vectors, valid caster bounds and adequate residency pools belong in the baseline; intentionally broken inputs are diagnostic controls.
- Every proposed shot below lasts **E 8 seconds** unless stated otherwise: **E 2 s** held start, **E 4 s** action, **E 2 s** held end, at a proposed **E 60 output frames/s**. Stated actions happen during that middle interval; “step” happens at **E t=2 s**. Keep deterministic motion and seeds. Capture the whole sequence plus the first changed frame and following **E frames 1,2,4,8,16,32**. Longer convergence is measured by extending the end hold, not by hiding it before recording.
- Hold lights, geometry and camera fixed except for the motion or light event named in each recipe. This makes a camera-only shot, a moving-object shot and a changing-light shot distinct controls, even when they reuse the same room.
- Build the fixtures from cuboids, quads, spheres, deterministic procedural textures and authored leaf/ribbon geometry. Their recipes need no private quarry scan, marketplace pack or non-redistributable capture asset. Reuse the repo's primitive generators and manifest format; verify redistribution rights before adding any external asset. A new primitive recreation is still a new comparison until run in both engines.
- Inspect normal-speed, display-scale motion first, then frame-step and zoom. Preserve direct lighting, indirect-only lighting, reflections and final-image crops separately when the harness supports them. Debug pink, cache pages and motion-vector overlays explain a failure; they are not the player-facing failure image. Score latency, flicker, lost detail, ghosts, leaks and frame-time peaks separately. A missing feature is a failed capability check, not a clean artifact score.

**Coverage tags:** **Existing matched** means a registered P-YARD shot/evidence exists, not that every extension below has run. **Partial** means a Renderlab-only fixture or related matched shot can seed it. **New** means the actual comparison still needs authoring. The [P-YARD manifest](../tools/p-yard/manifest.py#L1-L18) and its [scene list](../tools/p-yard/manifest.py#L115-L118) contain only quarry, S4 door and S5 cube.

## Lighting and reflected-light failures

### G1 — Indirect light follows a light change late

**R appearance/trigger:** global-light changes can take seconds to propagate through Lumen. **Lumen Scene Lighting Update Speed** and **Final Gather Lighting Update Speed** reduce delay at GPU cost: a latency mitigation, not a zero-lag guarantee. [Epic, “Lumen Lighting Update Speed”][l-overview]

**E shot:** a matte room with a red wall and white floor, lit by a spot aimed at the red wall; keep direct light off the inspected floor patch. Translate the spot sideways **E 2 m**, then step its intensity off on a second replay. Inspect the floor's red bounce after the spot passes and after switch-off, against a lit wall/direct-light control. Duration follows the common shot; extend the final hold if light has not settled.

**Coverage/forecast:** **Partial; shared risk on both paths.** S4's room and the game/local-light fixtures are reusable, but this moving-light comparison is new. Renderlab [relights fixed GI slot groups and schedules later gathers](../src/gi_cache.rs#L2369-L2418); local-light changes can [restart TR1](../src/temporal_reconstruction.rs#L1139-L1151). That can substitute fresh noise for screen-history lag while world bounce still catches up. Do not predict an automatic win from world-space storage.

### G2 — GI streaks around a moving character or blocker

**R appearance/trigger:** Lumen's temporal help explicitly describes character-near-wall ghosting and foot streaks. `r.Lumen.ScreenProbeGather.Temporal.DistanceThreshold`, `.RejectBasedOnNormal` and `.MaxFramesAccumulated` trade rejection/response against flicker and cost; they are residual-quality mitigations. [Epic UE 5.7 CVar help][cvars]

**E shot:** slide a tall grey cuboid **E 2 m** along a white wall over a red floor, then stop. Keep the camera fixed and put the inspected wall patch outside the blocker's direct sun shadow, so stale bounce/occlusion is distinguishable from silhouette transport. Repeat with a rigid pair of hinged cuboids as a minimal animated figure. Inspect the vacated wall/floor, feet and the stopped object's wake; common duration.

**Coverage/forecast:** **Partial; similar risk, possible Renderlab advantage only at the silhouette.** S4's moving door, [MV1](m7-mv1.md) and [MQ1](m7-mq1.md) supply motion plumbing, not this GI certification. World-cache invalidation and gathers still lag. A clean direct silhouette says nothing about a stale indirect shadow. See T1 for the separate reconstruction test.

### G3 — Light leaks through thin walls

**R appearance/trigger:** software Lumen's distance-field representation can leak through thin or unsuitable one-sided geometry. Epic recommends modular closed meshes and **R 10 cm** walls; **Distance Field Resolution Scale** and **Distance Field Voxel Density** improve representation at cost. This is a representation/content limitation, not a universal hardware-Lumen defect. [Epic, software tracing limitations][l-tech]

**E shot:** reuse S4's closed box and its **R code 1 mm / 5 mm / 100 mm** wall controls ([generator](../scripts/m7-u0-scenes.py#L140-L176)). Pan along the unlit inner wall while an exterior sun illuminates the other side; keep a thick wall in the same exposure. Inspect bright patches behind each wall and floor contacts; common duration.

**Coverage/forecast:** **Existing matched; Renderlab better than SW Lumen in this witness, approximately the same as HW Lumen.** [P-YARD](p-yard.md) records that result on both Renderlab paths. Current triangle-based rays/visibility avoid that particular distance-field thickness requirement. Coarse GI cells, filters and software visibility can still leak elsewhere; this test does not clear all thin geometry.

### G4 — Grain, boiling or blotchy lighting in dim rooms

**R appearance/trigger:** small/bright emissive sources produce Lumen noise; **Final Gather Quality** reduces GI noise at additional cost. These support a difficult sampling regime, not “every dark room boils”. [Epic emissive guidance][emissive], [Lumen settings][l-overview]

**E shot:** a matte room with a narrow skylight and a bright small emissive square, plus a non-emissive version lit by a regular area/spot-light approximation. Dolly **E 1 m** past a doorway and stop; inspect a uniformly painted wall, a dark corner and newly revealed ground. Keep exposure fixed rather than amplifying darkness through auto-exposure; common duration. The non-emissive control remains usable where Renderlab's authored-emission transport differs.

**Coverage/forecast:** **Existing matched for S4 wall grain; new emissive extension. Renderlab currently worse on the wall witness.** [P-YARD](p-yard.md) and [P-FLAT-TR1](p-flat-tr1.md) already show the stability problem. Final-colour reconstruction does not remove noisy incident illumination. Treat absent emissive transport or an artificially brighter room as a failed match, not reduced boiling.

### G5 — Surface Cache holes remove bounce or reflected detail

**R appearance/trigger:** uncovered cards show pink in the cache debugger, fail to bounce light and appear black in reflections. **Max Lumen Mesh Cards** or splitting complex meshes can repair coverage; extra cards do not always suffice. [Epic, “Surface Cache”][l-tech]

**E shot:** assemble a concave shelf/corridor from cuboids, first imported as one combined mesh, then as modular pieces with identical visible triangles. Orbit slowly around a doorway while viewing the shelf both directly and in a mirror. Inspect missing red bounce and black reflected shelf faces, not just the debug colours; common duration.

**Coverage/forecast:** **New; Renderlab avoids card coverage, but can fail differently.** Its [GI lookup can fall through coarse levels to open environment](../shaders/gi_api.wgsl#L248-L266), lifting occlusion; missing bounce can be dark. World-space cache availability is not complete surface coverage. Judge both bright leakage and missing indirect energy during first exposure.

### G6 — Hardware Lumen / Nanite proxy mismatch darkens otherwise covered surfaces

**R appearance/trigger:** [Epic UE-316311][proxy-issue] lists dark, unstable Surface Cache lighting from a Nanite proxy/position mismatch, affecting **5.7** and marked **Won't Fix**. Disabling Nanite removes the sample defect; **Max Edge Length Factor** and `r.LumenScene.DirectLighting.HardwareRayTracing.ShadowRayBias`, `r.LumenScene.Radiosity.HardwareRayTracing.SurfaceBias`, `r.LumenScene.Radiosity.HardwareRayTracing.SlopeSurfaceBias` can mitigate it, with bias consequences. Screen traces can conceal it.

**E shot:** a grey tessellated sphere above a floor under angled sun; rotate the camera while watching the lit sphere and its reflected image. Compare Nanite on/off with the same triangles and a diagnostic screen-traces-off replay. Inspect lit-view dark spots and temporal movement; common duration. A primitive sphere is also Epic's suggested substitute for the issue's attachment.

**Coverage/forecast:** **New; uncertain.** Renderlab does not use Lumen cards, but primary LOD geometry and secondary representations can still disagree. Its non-RT reflection geometry has [admission limits](../src/reflection_software.rs#L14-L16), and software sun uses its own record hierarchy. An absence of this exact proxy combination does not prove representation consistency.

### R1 — Rough reflections become noisy, blurred or temporally sticky

**R appearance/trigger:** rough specular switches from dedicated reflection rays toward GI-derived estimates. **Max Roughness To Trace** / `r.Lumen.Reflections.MaxRoughnessToTraceClamp` change that split. `r.Lumen.Reflections.Temporal.DistanceThreshold` / `.MaxFramesAccumulated` trade ghosting for noise; `r.Lumen.Reflections.ScreenSpaceReconstruction.RoughnessScale` deliberately blurs. These are quality/cost compromises. [Epic performance guide][l-perf], [reflection CVar help][cvars]

**E shot:** sphere and flat metal panels at proposed roughnesses **E 0.05, 0.15, 0.35, 0.5**, reflecting a bright moving cube and a narrow light highlight. Orbit **E 20°**, let the cube pass behind an occluder, then hold. Inspect highlight tails, lost sharp reflected objects, grain and energy changes across the roughness transition; common duration.

**Coverage/forecast:** **Partial; shared risk, Renderlab may be worse in motion.** [RF fixtures](m7-rq1.md) and [reflection_matched](../examples/reflection_matched.rs) provide material/motion content; S5 alone is not a rough-reflection test. Renderlab's [per-pixel ray estimator](../shaders/reflections.wgsl#L7-L35) has no independent reflection denoiser; TR1 integrates the result. [MQ2](m7-mq2.md) shortens view-dependent history, which can reveal noise rather than solve it. Keep programmable hit shading active.

### R2 — Mirrors omit small objects, highlights or secondary mirror content

**R appearance/trigger:** culling and cache lighting can lose small reflected objects/highlights; **Lumen Scene Detail**, lighting quality and HW **Hit Lighting for Reflections** are remedies with cost. Secondary mirror regions can remain black with insufficient **Max Reflection Bounces**; more bounces require the supported HW hit-lighting path. [Technical troubleshooting][l-tech], [Lumen reflection settings][l-overview]

**E shot:** reuse S5's offscreen moving cube and glossy neighbour. Add a small sphere reflected beside the cube; run a second, explicitly extended variant with mirrors facing each other. Track the cube **E 1 m** while the camera is fixed; inspect disappearance, repeated-room distortion, highlight absence and black nested reflection; common duration.

**Coverage/forecast:** **Existing matched for S5; extensions new.** Renderlab already beats SW Lumen's broken S5 mirror; HW Lumen is also clean there. Renderlab is **not** a multi-bounce mirror solution: [reflection hits use an untraced estimate](../shaders/reflections.wgsl#L7-L13), [skinned software reflection geometry is omitted](../src/primary_ray_gpu.rs#L2747-L2749), and cross-program hit response remains constrained. The nested-mirror/character variants may expose Renderlab losses.

### S1 — Screen-edge or disocclusion changes reveal a different transport estimate

**R appearance/trigger:** screen-first rays cover only the current view and hide differences from fallback geometry/lighting. Epic documents view-dependent GI with large **Indirect Lighting Scale**; the Lumen talk explains the edge boundary. Toggle `r.Lumen.ScreenProbeGather.ScreenTraces` / `r.Lumen.Reflections.ScreenTraces` to attribute the transition, not to assume disabling traces fixes every image. [Epic technical details][l-tech], [Epic SIGGRAPH Lumen, “Screen Traces”][l-talk], [CVar help][cvars]

**E shot:** put a red bounce panel and moving blue cube just behind the camera, with a matte floor and mirror visible ahead. Yaw so the contributors cross the image edge while keeping the receiver/mirror in frame; then reveal them from behind a foreground pillar. Inspect view-linked colour/reflection changes, edge fading and reveal holes; common duration.

**Coverage/forecast:** **Partial; plausible Renderlab advantage on both paths.** S4 authors an [offscreen red panel/population](../scripts/m7-u0-scenes.py#L142-L175); S5 supplies the reflected mover. New edge-crossing motion is needed. World GI/rays do not require an onscreen contributor, but new cell levels, refills and limited secondary geometry can still pop. This forecast excludes glass refraction, which remains screen-space (X1).

## Reconstruction and geometric detail

### T1 — Moving opaque edges leave trails or smear background detail

**R appearance/trigger:** TSR's history reuse can lose sharpness under motion; bad WPO previous-frame inputs can ghost. **Previous Frame Switch** repairs those inputs; `r.TSR.History.ScreenPercentage` and `r.TSR.Velocity.WeightClampingSampleCount` tune reconstruction. Correct vectors are the baseline, not a cure for all disocclusion. [Epic TSR, motion and troubleshooting sections][tsr]

**E shot:** slide a thin white cuboid and a rotating hinged cuboid across a black-and-white striped wall, then stop. Repeat with diagonal edges and a camera pan, preserving the same object trajectories. Inspect the vacated background, double contours and stopped-edge settling; common duration. Run native and matched lower internal resolution as separately labelled captures.

**Coverage/forecast:** **Partial; similar on the observed door, possible advantage at simple edges.** [MV1's moving fixture](../src/object_motion_tests.rs#L157-L195) and S4 are reusable; that MV1 fixture disables GI. Renderlab [reprojects rigid/skinned motion](../src/temporal_reconstruction.rs#L1028-L1041), stores surface-specific histories and [transports coverage as a straight edge](../shaders/temporal_resolve.wgsl#L493-L510). It still loses fresh samples at reveals, and its [two-surface limit](m7-tr1b.md#1-what-changed-from-tr1) cannot represent arbitrary layered edges. A clean S4 door is not proof of ghost-free thin geometry.

### T2 — Fine detail, wires and animated shader detail flicker

**R appearance/trigger:** temporal moiré detection needs time and its luminance analysis can miss chromatic flicker. `r.TSR.ShadingRejection.Flickering` and `.Period` mitigate it; **Has Pixel Animation** prevents the flicker heuristic from wrongly stabilizing animated opaque shading. [Epic TSR, flickering and pixel animation][tsr]

**E shot:** procedural fence wires over a checker wall, with a coloured fine stripe panel and a scrolling texture control. Dolly **E 1 m** and yaw **E 10°**; repeat at the same velocity in reverse. Inspect wire disappearance, crawling colours, moiré and an animated texture that sticks to old values; common duration.

**Coverage/forecast:** **Partial/new matched; Renderlab can be worse.** [TR1b/U1b](m7-tr1b.md) cover specular stripes and some motion, not this full fence. Straight-edge coverage helps isolated boundaries but not unresolved multiple wires; Renderlab's measured quarry flicker is already worse. Static geometry can therefore look stable while animated shading still smears or aliases. Test the user's shader output, not only the built-in BRDF.

### T3 — Particles and translucent movers leave ghosts or unstable detail

**R appearance/trigger:** translucent layers can lack a usable shared motion vector. **Output Depth and Velocity**, **Opacity Mask Clip Value** and pass placement change TSR's treatment; overwriting background velocity introduces another conflict. This is partly authoring and partly layered-reconstruction limitation. [Epic TSR, translucency][tsr] [UE-227677][widget-issue] separately documents widget trails in **5.4/5.5**; its persistence in **5.7** is unverified.

**E shot:** overlapping procedural smoke quads and bright small sparks crossing a striped background while the camera pans **E 1 m**. Include an opaque moving object behind them and a second pass with fixed camera. Inspect trails, popping spark visibility and background smear through smoke; common duration. Use identical deterministic particle trajectories, not independently simulated emitters.

**Coverage/forecast:** **Partial; Renderlab has a different quality loss.** [TP fixtures](../src/transparency_tests.rs#L491) and game fire/smoke exist, but no matched Niagara sequence. Particles draw [after TR1 and have no temporal AA/motion blur](m7-gaps.md#particles), so avoiding that history ghost can expose aliasing, coarse lighting and sorting errors. Renderlab cannot count an unlit particle or missing light response as a stability win.

### V1 — Shadow page demand spikes; exhausted pools corrupt shadows

**R appearance/trigger:** cuts and disocclusion demand new VSM pages. Pool overflow can yield checkerboards/missing shadows; `r.Shadow.Virtual.MaxPhysicalPages` addresses capacity. Ordinary page demand is a redraw/cost risk, **not** documented proof of normal delayed shadow pop-in. **Cached Page** visualization attributes it. [Epic VSM, “Caching” and “Overflow of Physical Page Pool”][vsm]

**E shot:** a grid of cuboids casting sun shadows across a floor; strafe **E 4 m**, whip to a new view and return. Inspect first visible contact shadows and frame-time spikes. Keep the normal pool adequate; a deliberately reduced-pool replay is a labelled capacity diagnostic, not the headline UE comparison. Common duration.

**Coverage/forecast:** **Partial; RT avoids the page mechanism, non-RT shares demand risks.** Renderlab RT uses fresh triangle shadow rays. Non-RT [requests/rasterizes stale pages](../src/virtual_shadow_gpu.rs#L2750-L2825) and retains [world-cell epochs](../src/virtual_shadow_gpu.rs#L2286-L2306). Only eligible large-budget frames use [occluder-centric S4](../src/sunvis_s4.rs#L24-L46); it does not replace every sun lookup. New-view cost remains real on both paths.

### V2 — Wind-driven foliage shadows shimmer, soften incorrectly or become stale

**R appearance/trigger:** low SMRT samples can make penumbrae noisy; deformation invalidates cached pages. `r.Shadow.Virtual.SMRT.RayCountDirectional` / `.SamplesPerRayDirectional` trade quality for work; tight bounds and **World Position Offset Disable Distance** reduce unnecessary updates. Disabling required invalidation risks stale shadows. Hidden occluders also limit depth-based soft-shadow reconstruction. [Epic VSM, penumbra quality/deformation sections][vsm]

**E shot:** several layers of procedural leaf/ribbon meshes above a matte plane, swaying on deterministic hinges under a finite sun. Track sideways **E 2 m**, then stop the camera while foliage continues. Inspect crawling leaf contacts, detached shadows and penumbra shape; common duration. Use equivalent transformed/skinned geometry for the matched shot; WPO-specific authoring controls are separate diagnostics.

**Coverage/forecast:** **Partial/new matched; mixed.** Renderlab's opaque cutouts avoid runtime alpha-mask holes, but [static contour/no cutout LOD constraints](m7-gaps.md#transparency) remain. RT can trade shimmer for sampling noise; non-RT has [known near-field contact speckles and self-layer omissions](m7-gaps.md#non-rt-sun-shadows). S4's boolean geometry tests are promising where selected, not an all-frame/all-content guarantee.

### V3 — Coarse shadow coverage changes fog/translucency quality or cost

**R appearance/trigger:** VSM coarse pages provide coverage for fog and forward translucency. `r.Shadow.Virtual.MarkCoarsePagesLocal` / `MarkCoarsePagesDirectional` change that coverage/work. They are not a universal fallback that automatically substitutes perfect shadows for absent fine pages. [Epic VSM, “Coarse Pages”][vsm]

**E shot:** a slatted room with fog and a transparent pane; move from close slats to a wide exterior view while sunlight remains fixed. Inspect beam continuity and pane shading as geometry leaves primary coverage. Compare normal coverage against a diagnostic coarse-page toggle; common duration. The toggle's lost lighting is not a performance win.

**Coverage/forecast:** **Partial/new matched; Renderlab non-RT may be worse.** VF2's slatted-window fixture is reusable. Its [fog reads shadow state without requesting pages](../src/atmosphere_shadow.rs#L12-L23), and [missing shadow pages return unoccluded sun](../shaders/atmosphere_shadow_virtual.wgsl#L9-L25). The visibility buffer supplying primary receivers does not establish offscreen froxel coverage. RT avoids that page hole but keeps finite froxel/sample limits.

### N1 — Nanite streaming pop-in, over-simplification or blinking geometry

**R appearance/trigger:** Epic names streaming pop-in and pool thrashing, plus missing/blinking geometry on fixed cluster-buffer overflow. **Minimum Residency (Root Geometry)**, `r.Nanite.Streaming.StreamingPoolSize`, `r.Nanite.MaxCandidateClusters` / `r.Nanite.MaxVisibleClusters` address them; **Max Edge Length Factor** helps deforming-mesh over-simplification. This does not establish that normal cluster transitions inherently pop. [Epic Nanite technical details][n-tech]

**E shot:** a procedurally tessellated corrugated wall and a bent ribbon, viewed from far to near with a rapid return. Keep a silhouette ridge and a tiny hole visible. Inspect delayed detail, silhouette steps and disappearing patches; common duration. Run an adequate-pool baseline first; pressure and overflow controls remain separately labelled diagnostics.

**Coverage/forecast:** **Partial/new matched; shared streaming/LOD risk.** Renderlab's [virtual geometry](milestone-4.md) preserves a complete legal cover when pages are unavailable, which can avoid holes while leaving coarse detail/quality debt. Its [visibility buffer](../src/visibility_buffer.rs#L1-L35) resolves chosen geometry; it cannot restore absent detail. Recorded quota/progress tests are not a Nanite motion-quality comparison. Keep streaming warm and cold passes separate.

### N2 — Foliage thins with LOD; aggregate geometry defeats cheap occlusion

**R appearance/trigger:** disjoint leaves resist simplification, layered holes cause overdraw, and distant canopies can lose area. **Preserve Area**, suitable geometry foliage and **Nanite Overdraw** inspection mitigate those content-dependent problems. [Epic, working with Nanite content][n-content]

**E shot:** a hedge made from repeated authored leaf meshes with several depth layers; dolly toward it and back while hinges sway. Put a high-contrast wall behind the hedge. Inspect canopy silhouette/opacity, leaf dropout and frame-time changes, not just an overdraw debug colour; common duration. A masked-card version is an authoring/cost control with matched silhouette, not presumed equivalent geometry.

**Coverage/forecast:** **New matched; uncertain quality, Renderlab could be slower.** Its [cutout conversion](../src/cutout.rs#L1-L12) replaces static alpha contours with triangles, avoiding masked-out pixel work. Dense triangles still incur visibility, secondary-ray and memory cost, and the contour has no dedicated LOD. Do not equate “no runtime alpha test” with solved foliage overdraw or stable tiny-leaf coverage.

## Transparency, fog, many lights and cuts

### X1 — Translucent surfaces receive coarse or incomplete lighting

**R appearance/trigger:** the conventional translucency-volume path has coarse, range-limited lighting and lacks detailed direct specular; **Surface ForwardShading** is Epic's per-pixel alternative. `r.TranslucencyLightingVolumeDim` and inner/outer distances trade resolution, range and cost. [Epic lit translucency][translucency] Lumen's **High Quality Translucency Reflections** sharpens the front layer; deeper layers remain glossy. [Lumen settings][l-overview]

**E shot:** overlapping tinted panes, one glossy and one nearly clear, over a checkerboard floor; move a compact spot along the panes and dolly **E 1 m** sideways. Inspect specular light shape, lighting gradients, deeper-layer reflection and offscreen refraction holes; common duration. Compare UE's appropriate forward-lit material, not only the inferior volume mode.

**Coverage/forecast:** **Partial/new matched; Renderlab likely shares or loses several features.** [TP2](m7-tp2.md) provides panes and programmable shading, but [screen-space thin-slab refraction](../shaders/transparency.wgsl#L129-L161), per-object sorting, absent rough/thick refraction and [non-RT glass reflection fallback](m7-gaps.md#transparency) remain. Glass is after TR1: fewer temporal ghosts do not compensate for wrong layering or missing reflected content.

### F1 — Volumetric lighting trails a moving/flashing light

**R appearance/trigger:** heavy fog reprojection can trail flashlights and muzzle flashes. Epic's **Volumetric Scattering Intensity = 0** workaround removes those lights from fog; it does not preserve the intended beam. Greater view distance undersamples the grid; `r.VolumetricFog.GridSizeZ` increases resolution/cost. [Epic volumetric fog][fog]

**E shot:** a fog-filled room with window slats; sweep the supported directional sun **E 15°** and step its intensity on a separate replay. Add a moving spot/muzzle-flash version as a feature check. Inspect old beam locations after movement/off, fresh noise and beam thickness; common duration.

**Coverage/forecast:** **Partial/new matched; shared temporal risk.** [VF2](m7-vf2.md) keeps current-view phase but reuses visibility/extinction. Renderlab's fog currently supports the first sun, so the local-light variant is a **feature gap**, not a trail-free win. [Fog reset rejection](../src/atmosphere_gpu.rs#L600-L619) and [history clamping](../shaders/atmosphere.wgsl#L257-L296) may reduce lag while increasing noise. Reset injection also takes more samples; dispatch dimensions alone do not describe its cost.

### F2 — Fog light leaks through occluding geometry

**R appearance/trigger:** [Epic UE-381266][fog-issue] reports rotating-directional-light fog shadow leakage with **VSM + Nanite tessellation**, affects **5.7.4**, **Unresolved**, target **6.0**. The issue names `r.Shadow.Virtual.UseReceiverMaskDirectional 0` as a seeming workaround, not a universal fix. This is specific evidence, not proof all UE fog leaks.

**E shot:** the common fog/slat room, with a subdivided displaced wall and a non-displaced geometric control. Rotate sunlight **E 15°** while the camera is held; inspect the fully blocked air behind the wall. For an actual reproduction, match the ticket's Nanite tessellation/receiver-mask combination; Renderlab's corresponding comparison uses explicitly tessellated triangles with the same visible surface. Common duration.

**Coverage/forecast:** **Partial/new matched; non-RT Renderlab already shares leakage risk.** [VF2](m7-vf2.md) records transient moving-sun leakage. Coarse froxels and missing software shadow-page coverage can light blocked air. A simpler non-tessellated shot that stays dark does not clear the UE ticket, nor certify Renderlab's own moving-sun case.

### M1 — Crowded lights cost time, or bounded sampling blurs/ghosts/noises

**R appearance/trigger:** conventional deferred-light cost grows with relevant lights. **MegaLights** bounds sampling but dense influential lights can produce denoiser blur, noise and ghosts. Tighter bounds, fewer overlapping sources, `r.MegaLights.NumSamplesPerPixel` and `.DownsampleMode` trade cost/content/quality. Its simplified/cull-limited RT scene can also lose shadows. MegaLights is labelled **Beta** in the pinned docs. [Epic MegaLights][many]

**E shot:** a room containing proposed **E 32**, then **E 128** overlapping coloured point/spot lights and thin moving cuboid occluders. Slowly pan **E 1 m**, then move a group of lights. Inspect dim lights turning off, contact-shadow noise and delayed direct-light changes, alongside frame time. Common duration per density. Compare conventional UE and a separately labelled MegaLights configuration.

**Coverage/forecast:** **Partial/new matched; Renderlab can be worse.** [LC3](m7-lc3.md#remaining-gaps) records finite non-RT shadow-map capacity, lower-resolution tails and redraw cost; [light clusters](../src/light_cluster.rs#L21-L59) have finite contributor capacity. RT tests contributing lights but still pays traversal/shading and samples soft lights. Neither “traced” nor “world space” guarantees every light is present, stable or cheap.

### C1 — First frames after a cut lose detail or await lighting coverage

**R appearance/trigger:** TSR can have only current-frame data after a cut; spatial AA hides aliasing but cannot restore unobserved detail. [Epic TSR, “Spatial Anti-Aliaser”][tsr] Lumen probe/update budgets can cause lighting pop during rapid new-view exposure. [Epic performance guide][l-perf] A warm same-region cut is not documented to turn universally dark; cold/distant views are separate cases.

**E shot:** cut between a previously visited room and an unseen room containing a glossy object, fine railing and dark crevice, then begin a lateral camera move immediately. Include a warm return and repeat cuts on a separate replay. Inspect detail/noise in the cut and early recovery, brightness ramps, edge dots and shifted frame-time peaks; common duration.

**Coverage/forecast:** **Existing matched warm cut; new distant/cold extension.** [P-YARD's schedule](../tools/p-yard/manifest.py#L1-L18) and [cut study](design-cuts.md) provide the baseline. Renderlab [resets TR1/fog while retaining world GI](../src/engine/mod.rs#L1033-L1040), so it avoids old-view contamination but can expose young noisy history and refill gaps. The measured cut cost is substantially worse; this is not yet a Renderlab win.

## Priority: player visibility and expected Renderlab position

This is **E judgement**, not a measured probability or artifact score. “Better” is restricted to the named mechanism/witness; “shared” includes a different failure with comparable player impact. Read **RT / non-RT** separately. Priority emphasizes conspicuous moving-image defects and a useful comparison, not only likely wins.

| E priority | Shot | Player impact | Expected RT / non-RT position and reason |
|---:|---|---|---|
| 1 | G3 thin walls | High: a room lights through a wall. | Similar to HW Lumen / better than SW Lumen in the witness. Triangle visibility helps; coarser GI remains a risk. |
| 2 | T1 opaque motion | High: object outlines trail during ordinary play. | Similar or conditionally better / similar or conditionally better. Surface identity plus straight-edge coverage helps; reveal noise and multi-surface limits remain. |
| 3 | G1 light change | High: switched-off light remains in the room. | Shared / shared. World GI still relights and gathers over time; TR1 restart can add noise. |
| 4 | G4 dim-wall noise | High: moving grain covers large flat surfaces. | Currently worse / currently worse. Existing matched evidence. |
| 5 | R2 mirror | High when a mirror is present. | Similar to HW Lumen / better than SW Lumen in S5. More bounces/characters can reverse the result. |
| 6 | S1 screen edge | High: light/reflection follows the viewport. | Plausibly better / plausibly better. World queries avoid screen visibility, not cache/refill/admission errors. |
| 7 | R1 rough reflections | High: sparkle, blur and tails on common materials. | Shared or worse / shared or worse. Finite strata plus final-image history; software coverage/cost adds risk. |
| 8 | F1/F2 fog | High in strong beams; content-specific. | Shared / known shared leak risk. First-sun-only support limits local-light comparison. |
| 9 | V2/N2 foliage | High outdoors. | Mixed / mixed or worse. Fresh rays or opaque contours remove some mechanisms; tiny geometry, shadow noise and triangle cost remain. |
| 10 | C1 cuts | High: player notices both image restart and hitch. | Image uncertain, cost worse / image uncertain, cost worse. Existing history reset and world-cache retention do not solve budgets. |
| 11 | X1 glass | High in windows and layered effects. | Shared feature losses / likely worse for reflected content. Screen-space refraction and non-RT glass fallback. |
| 12 | T2 fine detail | High during pans, lower on simple content. | Shared or worse / shared or worse. Reconstruction bandwidth/sampling and layered edges remain. |
| 13 | G2 moving GI | Medium–high near characters. | Shared / shared. Correct geometric motion does not instantly refresh bounce. |
| 14 | T3 particles | Medium–high during effects. | Different tradeoff / different tradeoff. Post-TR1 particles avoid that history but lack temporal AA and some lighting. |
| 15 | M1 crowded lights | High in dense authored lighting, less universal. | Cost/noise risk / stronger capacity/redraw risk. Current clusters/maps have explicit limits. |
| 16 | G5 card holes | High locally; content-dependent. | Different failure / different failure. No cards, but open-sky/coarse fallback and missing bounce remain. |
| 17 | G6 proxy mismatch | Medium–high on affected geometry. | Uncertain / different representation risks. Different data structures do not prove consistent transport geometry. |
| 18 | V1 pages | High if corruption occurs; otherwise mainly a hitch. | Page-specific advantage / shared cache/page risk. Adequate-pool baseline is essential. |
| 19 | V3 coarse coverage | Medium–high in fog/translucency. | Different finite-sampling risk / potentially worse uncovered froxels. |
| 20 | N1 streaming | High if geometry disappears; conditional on streaming/limits. | Shared LOD/residency risk / shared LOD/residency risk. Legal cover preserves geometry, not full detail. |

## Ten-shot first reel

Use **E ten shots × 8 s = 80 s** of comparison content, with separately labelled engine/path panels. Proposed order follows the priority above. The reel deliberately includes known Renderlab losses; it is a checklist for what must improve.

| Reel shot | Recipe and exact acceptance target | Work required |
|---:|---|---|
| 1 | **G3 — S4 thin walls.** Interior stays dark behind each wall while camera moves; preserve legitimate skylight/bounce. | Existing matched scene/manifest; reuse registered geometry and light. |
| 2 | **T1 — Moving edge over stripes.** No trailing outline, no erased background detail, no stop-time smear. | Extend MV1/S4 into a matched simple/diagonal-edge shot. |
| 3 | **G1 — Moving light and switch-off.** Direct and bounced regions respond without lingering light, new grain or brightness steps. | New matched room/light track using existing primitive/GI fixtures. |
| 4 | **G4 — Dim S4 wall/reveal.** Flat wall and exposed ground remain stable during motion and after stopping. | Existing matched witness; emissive variant is a separate extension. |
| 5 | **R2 — S5 mirror.** Offscreen cube and small details remain geometrically correct and temporally clean. | Existing matched cube; add the small-detail control. Nested mirrors belong in the extended suite. |
| 6 | **S1 — Offscreen contributor crossing.** Receiver lighting/reflections do not follow contributor screen visibility. | Add a yaw/occluder track to S4/S5; keep world content unchanged. |
| 7 | **R1 — Rough metal highlight.** Moving bright reflection has neither stray points nor broad temporal smear. | Adapt reflection_matched/RF into the shared manifest. |
| 8 | **F1/F2 — Moving-sun window fog.** Vacated beams disappear and blocked air stays dark. | Port VF2 room to UE; keep local-light/tessellation-specific variants labelled as capability/bug diagnostics. |
| 9 | **V2/N2 — Wind and hedge dolly.** Leaf opacity/silhouette and shadow contacts stay stable at the measured cost. | New procedural common-motion foliage fixture; this checks the combined final image, then splits geometry/shadow attribution in diagnostics. |
| 10 | **C1 — Warm cut, unseen destination and immediate move.** No old-view ghost, brightness ramp or later shifted hitch; early detail remains competitive. | Reuse cut manifest; author an unseen primitive-room destination and return track. |

Second reel: T2/T3 fine detail and particles, X1 glass, M1 crowded lights, G2/G5/G6 indirect/cache stress, V1/V3 page coverage and N1 streaming pressure. These remain required checklist items; omitting them from the first reel is not a pass. Verify candidate fixes independently before combining them into a scene where attribution becomes ambiguous.

For every shot retain a record: engine build and preset, backend, actual material/motion-vector mode, dimensions, all changed quality controls, warm/cold status, normal-speed artifact verdict, direct/indirect/reflection crop verdicts, first-change/recovery frames and frame-time distribution. The headline is the correctly configured final moving image. If correcting UE content or changing Renderlab's feature coverage changes the result, rewrite the claim around that narrower scope.

## Source and interpretation notes

The links below are Epic documentation, Epic-authored course material or Epic's public issue tracker; no community speculation is used as confirmation. Most documentation is pinned to **5.7**. The VSM pinned page was intermittently fetchable; some section text was cross-checked against Epic's [current VSM page][vsm-current], then labelled **5.9**. Do not inherit its newer defaults as verified **5.7.4** settings. Record actual console values in the future capture. Issue states are those read for this study, not promises that a target fix shipped.

The SIGGRAPH Lumen talk is historical algorithm evidence: screen-bound traces and temporal-filter latency were discussed together with mitigations. This catalogue uses current CVar help for residual GI/reflection ghosting tradeoffs; it does not present every historical talk problem as an unfixed modern bug. Likewise, [UE-312483][tsr-film-issue] describes a film-slope-sensitive ghost case in **5.5/5.6**, marked **By Design** with target **5.7**; it is an optional diagnostic, not proof of a current baseline regression. A fixed-exposure film-matched reel avoids confusing display amplification with transport error.

This is a failure/shot catalogue, not an implementation proposal or patent clearance. The renderer's patent design-around policy remains applicable to subsequent fixes; no Epic algorithm is authorized for copying by its presence here. Programmable shading remains part of every Renderlab forecast: stable built-in GGX output does not certify arbitrary user shader animation or cross-program reflection hits.

Reading alone cannot determine which artifacts the actual matched UE build reproduces, their recovery times or visibility thresholds, a statistically repeatable performance delta, or whether a proposed Renderlab advantage survives the whole shot. Existing captures cover only the stated witnesses. The new shots and controls must be run and inspected before any “solves Unreal's issue” claim.

## Five-line summary

1. Prioritize opaque motion, lighting changes, dim interiors, reflections and cuts while retaining thin-wall/mirror witnesses already won against SW Lumen.
2. Separate GI history, reflection history, TSR coverage, fog history and geometry/page coverage before naming a ghost or leak.
3. Use Epic's documented fixes and adequate resources in the baseline; label remaining limits and version-specific bugs honestly.
4. Expect shared Renderlab failures in noise, cache response, foliage, translucency, fog and many-light capacity; missing effects never count as wins.
5. Start the proposed ten-shot reel, judge whole moving sequences and retain only improvements with measured cost and no visible regression.

[l-tech]: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine?application_version=5.7
[l-overview]: https://dev.epicgames.com/documentation/unreal-engine/lumen-global-illumination-and-reflections-in-unreal-engine?application_version=5.7
[l-perf]: https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine?application_version=5.7
[l-talk]: https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf
[cvars]: https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-console-variables-reference?application_version=5.7
[emissive]: https://dev.epicgames.com/documentation/en-us/unreal-engine/using-the-emissive-material-input-in-unreal-engine?application_version=5.7
[proxy-issue]: https://issues.unrealengine.com/issue/UE-316311
[tsr]: https://dev.epicgames.com/documentation/en-us/unreal-engine/temporal-super-resolution-in-unreal-engine?application_version=5.7
[widget-issue]: https://issues.unrealengine.com/issue/UE-227677
[tsr-film-issue]: https://issues.unrealengine.com/issue/UE-312483
[vsm]: https://dev.epicgames.com/documentation/unreal-engine/virtual-shadow-maps-in-unreal-engine?application_version=5.7
[vsm-current]: https://dev.epicgames.com/documentation/ru-ru/unreal-engine/virtual-shadow-maps-in-unreal-engine
[n-tech]: https://dev.epicgames.com/documentation/unreal-engine/nanite-technical-details?application_version=5.7
[n-content]: https://dev.epicgames.com/documentation/unreal-engine/working-with-naniteenabled-content?application_version=5.7
[translucency]: https://dev.epicgames.com/documentation/en-us/unreal-engine/lit-translucency-in-unreal-engine?application_version=5.7
[fog]: https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine?application_version=5.7
[fog-issue]: https://issues.unrealengine.com/issue/UE-381266
[many]: https://dev.epicgames.com/documentation/en-us/unreal-engine/megalights-in-unreal-engine?application_version=5.7
