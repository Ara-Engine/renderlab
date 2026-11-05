# M6 film gap: whole-image aliasing and the receiver policy

Design decision, 2026-11-05, base `b0ba363`. It answers the two questions [F0](m6-film-reference.md) raised. CPU only: saved F0 buffers, frozen captures, decoded native Unreal masks, the read-only source asset and the installed UE 5.7.4 shader source. There was no GPU run, build, window or renderer change, and nothing was pushed. Labels: **M** is measured here from saved evidence, **I** is an inference or estimate.

## Decisions

1. **The F0 gap is pixel-integration aliasing, not a shadow-estimator defect (M).** The noise-free 16,384-ray point reference has the same film error as Exact: 0.2316 against 0.2316 RMS on film partials. That is 99.98% of Exact's squared error, and Exact differs from it by only 0.0028. Of Exact's 9,199 missed film-visible shadows, the point reference misses 9,187. More rays at the pixel centre cannot close this gap.
2. **The gap is whole-image, and visibility is a minor part of the visible error (M).** Against the film radiance R_L, single-surface pixels whose film visibility is fully lit carry 43% of the image's squared display error at the registered exposure. Visibility-varying pixels carry 28%, multi-surface footprints 9%, and silhouettes seen from background-centre pixels 15%. Inside the visibility-varying population, any correction that is a function of the visibility gap alone could explain at most **7–9%** of the squared radiance error. A linear one explains under 1%. Both errors live where the source triangles (about 6 mm, near the 8.2 mm footprint) turn by 20° or more between pixels.
3. **Fix class: whole-image film integration by temporally jittered primary samples with surface-anchored accumulation.** It is not a shadow-specific cache. Each frame traces one jittered primary per pixel with its own Interactive shadow rays and full shading. Pixel identity is carried by the exact hit record. One mechanism integrates geometry, normals, texture and visibility, including their product inside the footprint. The estimate is 0.89–0.94 ms per frame. Modelled visibility error beats Exact after about 2 static frames and reaches 0.067 at 16 (I). The surface visibility cache (SVC) as specified is **rejected as the fix class**. Its parts stay available for a per-sample visibility cache if the AA fails moving-sun or time-to-quality checks ([§4](#4-fix-classes-priced)).
4. **Level F scores the final image.** The primary reference becomes R_L, full film radiance, plus R_D, a direct-light-only film radiance so shadows are scored at full contrast. R_F visibility stays as a diagnostic for missing and false shadows and contacts ([§5](#5-level-f-against-radiance)).
5. **Receiver policy: an error-scaled, view-side geometric origin.** The origin lies on the hit triangle and is offset along the geometric normal on the viewed side by `max(10 µm, 16 ulp(max |p_i|))`, with t_min equal to that ε. The 1 mm constant and the projected-depth t_min go. At quarry scale this is F0's 10 µm magnitude. The view-side rule changes the 3.4% of covered pixels whose viewed facet faces away from the sun ([§6](#6-receiver-policy)).
6. **Next unit: R1, the receiver-policy unit F0's frozen rule requires**, extended to produce the radiance references. F1-AA follows only after R1 settles the estimand. The SVC F1/F2 plan in [the exact-quality design](m6-exact-quality-fast-shadows.md#7-first-unit-and-stop-rules) is superseded ([§7](#7-next-unit-and-stop-rules)).

## 1. Estimator or sampling position

All scores are against the F0 policy (1 mm) reference on film partials (n = 46,514 at pose 0), unless marked minimal.

| Pose / reference | Exact vs R_F | 16,384-ray point vs R_F | Exact − point | Point share of Exact MSE | Exact missed / also missed by point |
|---|---:|---:|---:|---:|---:|
| 0 / policy | .2316 | .2316 | .0028 | 99.98% | 9,199 / 9,187 |
| 1 / policy | .2320 | .2320 | .0029 | 99.996% | 9,118 / 9,107 |
| 0 / minimal | .2505 | .2505 | .0027 | 99.99% | 12,615 / 12,597 |
| 1 / minimal | .2503 | .2503 | .0027 | 99.995% | 12,473 / 12,460 |

The film buffers also give the footprint convergence rate (M). Per-footprint visibility SD over one sample is σ₁ = 0.285 RMS. The 256-sample stratified estimate has split-half σ = 0.0138. That gives σ(N) ∝ N^−β with **β = 0.543–0.546** at every pose and policy. This is slower than the N^−0.75 of a straight edge and close to random sampling, as expected for micro-relief with many small occluders. β comes from two points (N = 1 and 256); R1 checks it at N = 1–16 ([§7](#7-next-unit-and-stop-rules)).

**Consequence for references.** At 256 samples, R_F's own error (0.014) is no longer negligible once a candidate approaches 0.03. Level F references for converged candidates need about 1,024 samples, or noise-subtracted statistics as the deciding measure.

## 2. Radiance against the film image

R_L is F0's film radiance: the mean of all 256 jittered primary samples of the renderer's own HDR shading, each with its own Exact visibility. The point image is the frozen centre Exact capture, byte-identical to `accept-b7`. Errors are in display codes after F0's inspection transform (exposure, Reinhard, sRGB), RMS over channels. Pose 0, policy reference; the other three cases agree to within about 10%.

| Population (a partition of the image) | n | Codes RMS, registered / +4 stops | Share of image squared error, registered / +4 | Pixels > 4 codes, registered / +4 |
|---|---:|---:|---:|---:|
| Film partial, single surface (accepted n = 256) | 44,715 | 9.38 / 16.67 | 28.5% / 31.0% | 31.8% / 37.6% |
| Film lit, single surface | 281,292 | 4.58 / 6.31 | **42.6% / 28.0%** | 14.8% / 16.8% |
| Film umbra, single surface | 13,599 | 6.87 / 13.28 | 4.7% / 6.0% | 22.8% / 29.6% |
| Multi-surface footprint (n < 256) | 6,911 | 13.66 / 21.96 | 9.3% / 8.3% | 60.7% / 64.6% |
| Background at centre (silhouettes) | 1,727,083 | 1.09 / 2.49 | 14.9% / 26.7% | 0.08% / 0.09% |

Error on visibility-constant lit pixels is pure shading and geometry aliasing, since nothing about the shadow varies in their footprint. It is the largest share of the image's error.

**How much of the radiance error follows the visibility gap (M).** On single-surface pixels the signed display error is regressed on δ = V_centre − R_F:

| δ bin | n | Mean codes, registered | RMS codes, registered | Mean codes, +4 | RMS codes, +4 |
|---|---:|---:|---:|---:|---:|
| ≤ −0.4 | 3,151 | −5.9 | 14.4 | −11.0 | 28.2 |
| −0.4 to −0.1 | 4,643 | −3.5 | 11.7 | −7.9 | 25.1 |
| −0.1 to −0.02 | 3,166 | −0.9 | 6.9 | −2.6 | 13.7 |
| within ±0.02 | 307,719 | −0.34 | 4.8 | −0.79 | 7.0 |
| 0.02 to 0.1 | 10,022 | −0.39 | 8.0 | −1.4 | 12.8 |
| 0.1 to 0.4 | 9,624 | −0.39 | 8.0 | −1.6 | 13.1 |
| ≥ 0.4 | 1,281 | −3.4 | 12.6 | −7.4 | 25.0 |

- **Positive δ is the missed-film-shadow side.** The centre is lit while the footprint is partly shadowed. Its mean radiance error equals the no-gap bin, about −0.4 codes. The film image is not darker there. The footprint's shadowed microfacets are grazing or backlit, so they had little direct light to lose: direct light and visibility covary inside the footprint.
- **Negative δ is visible** (centre in shadow, footprint partly lit): 1–6 codes darker on average at the registered exposure. Even there, the bin means carry a small fraction of the bins' squared error.
- **Explained share.** Linear R² over film partials is ≤ 0.009. The binned (any-function-of-δ) share over the 31.9–35.4 k pixels with |δ| > 0.02 is **7.1–7.8% registered and 8.6–9.3% at +4 stops**, across both poses and policies. That bounds what a visibility-only correction keyed to the gap can remove.
- **Direct share.** Where direct light is identifiable, it is small. On 8,187 point-partial pixels the Exact and four Interactive captures differ in visibility. There, L = A + D·V fits with p99 relative residual 3.9e-4, and D is a median 0.57% of the lit radiance (p90 1.7%). Applying D·(R_F − V) there changes the display error from 5.973 to 5.967 codes (registered) and 9.752 to 9.731 (+4).
- **Cast-shadow contrast.** At cast-shadow boundaries, umbra/lit film radiance on the same placement with normals within 25° has median 0.873 (IQR 0.64–1.13, 2,612 pairs). The sun is about 13% of film radiance there.

**Where the error sits (M).** Neighbour geometric-normal spread is the maximum angle to same-placement 8-neighbours. It is a centre-sampled proxy for normal variation inside the footprint.

| Spread | Covered pixels | Film partials | Exact partial squared error | Lit-pixel radiance RMS, +4 stops |
|---|---:|---:|---:|---:|
| < 5° | 1.6% | 0.4% | 0.09% | 1.0 codes |
| 5–20° | 24.1% | 13.9% | 8.7% | 1.9 codes |
| ≥ 20° | 74.2% | 85.7% | 91.2% | 7.3 codes |

Visibility varies inside the footprint almost only where the geometry does. Shading aliasing on lit pixels follows the same geometry. Texture is not the driver: flat lit pixels, mip-filtered at 16x anisotropy, err by only 1.0 code. A 3×3 block holds a median of 9 distinct triangles, and only 3% of horizontal neighbours share one. The inspected crops (`film-gap/out/inspect-*.png`) agree. Exact's visibility error follows cracks and ridges. The radiance error is fine two-signed speckle over every rough face, plus crack and silhouette outlines. It is not concentrated where the visibility errors are.

**Lighting dependence.** The registered lighting has an unoccluded uniform environment and no environment occlusion or GI. That is why shadows carry so little radiance here. Under sun-dominant or occluded-sky lighting, visibility errors weigh more. The aliasing mechanism does not change, and neither does the covariance argument against shading the centre with a box-filtered visibility. The gate must not rely on this lighting hiding shadows ([§5](#5-level-f-against-radiance)).

## 3. Unreal with TSR off

Both native modes are 1x, unjittered, with one receiver per pixel centre ([comparison setup](quarry-end-to-end-comparison.md), "no TSR or temporal AA"). Structurally, neither can represent sub-pixel visibility, so R_F charges them for the same missing footprint integration. Their measured error is not mostly Exact's aliasing, however (M, pose 0; pose 1 within 1%):

| Reference | Mode | vs R_F | vs point reference | Point vs R_F | Cross term / MSE | Correlation with Exact's film error |
|---|---|---:|---:|---:|---:|---:|
| policy | VSM | .371 | .453 | .232 | −0.88 | −0.11 |
| policy | HRT | .346 | .412 | .232 | −0.87 | +0.02 |
| minimal | VSM | .403 | .416 | .250 | −0.45 | +0.13 |
| minimal | HRT | .375 | .383 | .250 | −0.48 | +0.21 |

Unreal is farther from the point value than from the film value. Its deviation from the point is strongly anti-correlated with the point's aliasing, with a cross term of −0.45 to −0.88 of its MSE. Shadow-map texel filtering and the denoiser act as a crude spatial integrator, while its receiver offsets remove contacts ([mismatch analysis](unreal-shadow-mismatch.md)). Its film score therefore mixes the same missing integration with its own biases.

**The 1.60 / 2.15 ms denominators exclude TSR.** AGENTS sets the minimum at Unreal's high-fidelity game rendering, which ships with TSR. TSR would add to Unreal's frame time and move its image toward the film integral; both are unmeasured. Treat the TSR-off numbers as a 1x-to-1x comparison. The next Unreal capture should add TSR-on image and time at the same poses.

## 4. Fix classes priced

The model uses the [Interactive passes](m6-interactive-climb.md) (M): fused trace plus 8 rays 0.40 ms (0.07 fixed), shading 0.396, tone 0.049, span 0.861. A supersampled pixel repeats about 0.73 ms of trace, rays and shading per sample.

Visibility error is modelled as `σ(N) = sqrt((σ₁ N^−β)² + σ₈²/N)`, with Interactive's 8-ray point noise σ₈ = 0.094 (measured on the thin fixture). In static frames this gives **N = 2: 0.207; 4: 0.142; 8: 0.098; 16: 0.067; 32: 0.046; 64: 0.032**, against Exact's 0.232 (I). If radiance converges at the same rate, the lit-pixel error of 4.6 codes falls to about 1.0 code at N = 16 (I).

| Fix class | Integrates | Frame cost | Film quality reached | Motion and light change | Verdict |
|---|---|---|---|---|---|
| Exact 1x | Disk at the centre | 8.39 ms (M) | .232 (M) | Stateless | Not film quality; 256 rays buy nothing against R_F |
| **Temporal jitter, surface-anchored accumulation** | Footprint, disk, shading and their product | **0.89–0.94 ms** (I: Interactive plus a 0.03–0.08 ms accumulate pass) | Beats Exact after about 2 frames; 0.067 at 16 (I) | Cold pixels fall to today's 1x Interactive image. History must be invalidated by exact signals (surface identity, light and scene change). Fast sun needs a reset or a visibility history | **Recommended** |
| Spatial supersampling, N per frame | Same, stateless | 0.12 + 0.73N: N = 2 1.58, N = 4 3.0, N = 16 11.8 ms (I) | N = 2 0.207, N = 16 0.067 (I) | Exact under motion | Offline film mode and reference generator only |
| Adaptive per-pixel counts | Same, where triggered | +0.73 ms per 100% of pixels boosted (I) | — | — | Allocator only. 74% of covered pixels exceed 20° spread and each pixel holds about one triangle, so geometry triggers select nearly every scan pixel. Variance triggers repeat the faint-miss failure. Use exact triggers (cold history) to speed up revealed pixels |
| SVC as specified (shadow only) | Footprint visibility × centre shading | 0.85–0.93 ms ([prior estimate](m6-exact-quality-fast-shadows.md#4-recommended-surface-visibility-cache), I) | Removes at most 7–9% of partial-pixel radiance error (M bound). Leaves the 43% lit, 9% multi-surface and 15% silhouette shares | World-keyed; sliding sun window | **Rejected as the fix class** |
| AA plus a per-sample visibility cache | AA, with low-noise visibility at each jittered sample | ≈ 0.95–1.05 ms (I) | — | Lag-free moving sun | Deferred until AA evidence demands it |

**Budget.** Every option misses the 2x VSM floor of 0.80 ms; Interactive alone already does. Against the stored 2.15 ms HRT anchor the floor is 1.075 ms, and the recommended class fits. Against the fresh matched HRT GPU medians of 1.74–1.78 ms the floor is 0.87–0.89 ms, so 0.89–0.94 ms is a likely miss. Record misses; do not reduce quality to pass.

**Why reprojection here does not count as "screen-space core lighting".** Every sample's lighting is computed in world space by ray tracing. The history only integrates the pixel footprint. It is validated by exact surface identity and object-space distance, never by colour clamping. Its failure mode is the unbiased 1x sample, not missing off-screen information. It does lose off-screen history: surfaces returning to view start cold, and F2 must measure that. A world- or texture-space radiance cache was rejected because it bakes view-dependent and user-authored shading.

**Programmable shading constraint (recorded).** Accumulation requires shading to be a function of the sample's surface and of declared time-varying inputs. User stages that animate must declare it, so history is invalidated rather than ghosted.

## 5. Level F against radiance

Level F should compare against full radiance, with visibility kept as a diagnostic:

- **R_L, primary.** Film radiance of the renderer's shading, scored in linear HDR and in display at the registered and +4-stop exposures. Report the five-way partition of §2 and normal-spread bins. The F1 floor becomes "≤ point-sampled Exact 1x against R_L"; the target is R_L within its own noise.
- **R_D, shadows at full contrast.** Film radiance of direct light only (environment black). Under the registered lighting the sun is only about 13% of radiance at cast boundaries, so R_L alone would hide shadow defects. Shadows must hold at any light balance. F3 bias, F4 missing and false shadows, and contact checks run on R_D and R_F.
- **R_F, diagnostic.** It stays for population definitions, enumeration and inspection. Re-derive the 0.002 solid bound on R_D: F0 showed that Exact is not exact on film-defined solids.
- **Scope.** R_L certifies the pixel integral of Renderlab's shading model, not physical lighting. A path-traced reference would also occlude the environment in crevices; that remains a separate open gap.
- Reference sample count rises to about 1,024 for converged candidates ([§1](#1-estimator-or-sampling-position)).

## 6. Receiver policy

**Geometry (M, source asset read in place; the hit-record-to-triangle mapping reconstructs world positions within 1 µm):**

| Quantity | p10 | Median | p90 | p99 |
|---|---:|---:|---:|---:|
| Visible triangle, longest edge | 3.7 mm | 8.5 mm | 18.4 mm | 29.1 mm |
| Shortest edge | 2.3 mm | 4.9 mm | 10.5 mm | 16.5 mm |
| Equal-area edge | 2.8 mm | 6.2 mm | 12.8 mm | 19.7 mm |
| Pixel footprint (view distance × 1.028 mrad) | 7.3 mm | 8.2 mm | 10.1 mm | — |
| Neighbour normal spread | 10.8° | 36.3° | 76.8° | — |
| Vertex-normal deviation from face | 1.6° | 6.3° | 19.9° | 38.3° |
| Smooth-surface offset (Cycles' vertex-normal pseudo-surface, computed per pixel) | 0 | 0.094 mm | 0.40 mm | 0.84 mm |

Coordinates satisfy |x| ≤ 3.9 m, so the f32 ulp is at most 0.24 µm. The stored receiver (the ray point) lies at most 2.8 µm off its triangle plane ([primary-ray probe](m6-primary-ray-probe.md)).

**What the 1 mm policy removes (M, pose 0; pose 1 agrees).** The 1 mm reference is brighter than the 10 µm one by more than 0.05 on 17,903 pixels, and darker on 135. Of the brighter pixels:

- 43.7% are grazing (0 < camera-side geometric N·L < 0.26).
- 20.1% are backlit (N·L ≤ 0), against 3.4% of all covered pixels.
- 0.08% are flat (spread < 5°) and strongly lit (N·L > 0.5), where self-intersection acne would show.
- Their normal spread has median 42°, against 36° overall. They form 3,354 components (median 2 pixels, p99 50).

That is geometric occlusion by neighbouring mm-scale triangles at grazing light, not a precision artifact of the 10 µm policy. At quarry scale, 10 µm is 3.6 times the receiver's plane error and 1/490 of the median shortest edge. 1 mm is a fifth of that edge. It exceeds the vertex-normal smooth offset on 99.5% of pixels and is about half the triangle-scale relief (I: 6 mm edges turning a median 36°), so it acts as a geometry filter, not a precision guard. The projected-depth t_min (about 0.25 mm) depends on the cascade depth range, so visibility also changes with camera-dependent cascade fitting.

**The light-side flip.** Renderlab orients the origin toward the light. A viewed facet that faces away from the sun therefore has its visibility traced from behind the surface. Such facets have mean film visibility 0.203 at 1 mm and 0.151 at 10 µm (11,824 pixels, M), where an opaque solid would give about 0. This is thin two-sided semantics applied to scanned solids. On 28% of those pixels the smooth shading normal faces the sun, so the leak reaches the image (I).

**What production renderers do:**

- **UE 5.7 path tracer.** Origin offset along the geometric normal by `max(2^-8 cm ≈ 39 µm, 16 ulp(max(|translated origin|, HitT)))`, with TMin 0 (`RayTracingCommon.ush:424`, `PathTracingCore.ush:2017–2023`). The normal takes the two-sided sign, i.e. the viewed side (`PathTracingMaterialHitShader.usf:349`). It flips toward the light only for transmissive materials. At quarry scale that is about 39 µm.
- **UE real-time HRT.** 1 mm along the shading normal plus a 1 cm back-face-culled near trace ([mismatch](unreal-shadow-mismatch.md)). That measurably erases contacts.
- **Cycles.** Removed positional ray offsetting and now explicitly skips the originating primitive ([remove ray offsetting](https://git.blender.org/gitweb/gitweb.cgi/blender.git/commit/74afc86d4bf1a559e5b7a0cf5cea29cb508a3e99), [explicitly skip self-intersection](https://projects.blender.org/blender/blender/commit/ae440703411486c9219fa0ff54e471eea64afb58)). Separately, a per-object shadow-terminator Geometry Offset (default 0.1 since 3.0) moves shadow origins toward the vertex-normal pseudo-surface ([T68920](https://developer.blender.org/T68920), [T94205](https://developer.blender.org/T94205)). On this scan that offset is 0.09 mm at the median.
- **Wächter and Binder** ([Ray Tracing Gems ch. 6](https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_6.pdf)) offset by integer ulps of the coordinates. Fixed-length offsets are not scale-invariant; primitive skipping still self-intersects at shared edges and grazing angles.

**Recommendation: error-scaled view-side origin.**

- **Origin:** the point on the hit triangle, from its barycentrics and returned world-space vertices.
- **Offset:** along the geometric normal, oriented toward the viewer for opaque materials. Toward the light only for materials that declare thin or transmissive two-sided behaviour.
- **Size:** ε = max(10 µm, 16 ulp(max |p_i|)) over the triangle's world vertex coordinates; t_min = ε, which keeps first-hit termination exact. There is no projected-depth t_min and no constant normal bias.
- **At quarry scale** ε is 10 µm (16 ulp is at most 7.6 µm below 8 m). F0's minimal reference is therefore the magnitude proxy; the side rule is the new part.
- **Constraint (recorded):** visibility queries need each material's thin/opaque declaration, a material-stage property that programmable shading must expose.

Scan noise is not the receiver's business. If sub-mm relief is reconstruction noise, smooth it in asset preparation or move it to normal maps. Terminator softening may be an explicit, declared shading-stage option. Neither belongs in a hidden bias that also erases real contacts.

**Evidence this policy needs (R1):**

1. Analytic acne fixtures: planar and convex, translated 1 km and 10 km, with light at 80–89° incidence. Zero false shadows.
2. An analytic concave micro-relief fixture: sawtooth / V-groove, 2–8 mm facets, grazing light. It matches the oracle within σ_F; the 1 mm policy is shown diverging.
3. The existing thin and stress fixtures, within σ_F.
4. Quarry R_F, R_L and R_D under the policy, scored against the F0 10 µm reference. Every population's difference must be ≤ Exact's error under the frozen F0 rule.
5. The side rule measured separately on the backlit population.

## 7. Next unit and stop rules

**R1: receiver policy and radiance references.** GPU work is test-only and runs only when Islam is not playing. Under the foreground-game rule, the CPU parts can proceed now: fixture oracles, shader edit, WGSL validation and CPU tests.

- **Test-only shader edits, in the F0 style:**
  - R1a: error-scaled magnitude with the light side retained.
  - R1b: R1a plus the view side for opaque materials.
- **Fixtures:** the four in §6 at both variants, against analytic oracles.
- **Quarry poses 0 and 1** (pose 2 as the identity check), each variant:
  - R_F, R_L and R_D at 256 samples;
  - the first 16 jittered subsample HDR and visibility frames saved, about 400 MB per pose, so N = 1–16 convergence is replayed on the CPU, checking β and the radiance rate before AA is built.
- **Expected verification:** one test build, about 12 fixture windows and 6 quarry windows of about 10–12 s each. CPU scoring is about 2 minutes.
- **Frozen stop rules:**
  - Any fixture outside σ_F of its oracle, or any acne-fixture false shadow: fix the implementation, never the tolerance, and stop until fixed.
  - A quarry population where R1a differs from the F0 10 µm reference by more than Exact's error against it: the magnitude is unsettled. Record it, inspect it, and do not start F1-AA.
  - R1b changes the backlit population by more than Exact's error: present the side rule to Islam as an estimand change with R_L and R_D crops; it is not adopted silently.
  - Saved-subsample replay showing β < 0.45 or radiance converging slower than visibility: recompute F1-AA's frame and T_q predictions before building.
  - The production default is unchanged in R1. Adopting the policy is a separate change that re-baselines Level A, because it changes Exact's output.

**F1-AA, frozen now and started only after R1.** Opt-in `PrimaryVisibility::Traced` plus temporal film accumulation. Exact, Interactive and hard output stay byte-identical. File ownership: new `src/film_accumulation.rs` and `shaders/film_accumulation.wgsl`; jitter in `shaders/primary_trace.wgsl` and `src/primary_ray_gpu.rs` only.

- **Static quality:** against references at 1,024 samples. Stop if static visibility partial RMS exceeds 1.25× the prediction (0.12 at frame 8, 0.058 at frame 32), or if radiance against R_L is not ≤ Exact 1x by frame 2: the convergence model is wrong.
- **Sequences:** Level F with R_L, R_D and R_F on Q-static, Q-walk, Q-cut, Q-disocclusion and Q-blocker, plus fixtures. Any F4 excess on R_D over Exact 1x stops the candidate. T_q stays Islam's decision.
- **Q-sun:** 0.25°/s must pass F5. A 10°/s failure is recorded; it reopens the per-sample visibility cache, not a quality trade.
- **Cost:** stop if p50 exceeds 1.075 ms or any frame exceeds 1.5× steady. Report against the 0.80 and 0.87–0.89 ms floors as misses if missed.

## 8. Rejected options

| Option | Reason |
|---|---|
| More shadow rays at the pixel centre (Exact as the visible bar) | The noise-free point reference has the same film error (M) |
| SVC as the fix class | At most 7–9% of partial-pixel radiance error; ignores the 43% lit-pixel shading aliasing and the silhouette shares; centre shading × box visibility ignores direct-light/visibility covariance (M bound, I) |
| Spatial supersampling as the real-time default | 1.58 ms at N = 2 for 0.207; about 11.8 ms at N = 16 (I) |
| Adaptive counts as a standalone fix | Scan pixels are almost all complex; variance triggers miss faint features |
| MSAA or coverage-only AA | Leaves the largest share, lit-pixel shading aliasing |
| TAA or TSR-style colour clamping | Clamp boxes from an aliased 1x neighbourhood on rough scans ghost or flicker; heuristic rather than exact rejection |
| World- or texture-space radiance cache | Bakes view-dependent and user-authored shading |
| Cross-pixel filters | Bias across pixels (previously measured worse) |
| Visibility-only Level F | Overstates invisible micro-shadowing; blind to shading and silhouette aliasing |
| 1 mm normal bias plus projected-depth t_min | Geometry filter at mm relief; light leaks through backlit thin relief; camera-dependent t_min (M) |
| Fixed 10 µm alone | Not scale-robust (f32 ulp at 1 km is 61 µm) |
| Wächter–Binder 256-ulp integer offset | About 0.12 mm at 4 m and mm-scale far from the origin, against mm triangles |
| Primitive-ID self-skip (Cycles) | Needs non-opaque candidate processing under `FORCE_OPAQUE` ray queries, cost unmeasured; still self-intersects at shared edges. Revisit only if R1's acne fixture fails |
| UE HRT-style shading-normal bias plus 1 cm near trace | Measured contact erasure |
| Smooth-surface terminator offset as the reference | Changes the shadowing geometry; a declared shading option at most |
| Tuning the bias to Unreal or to hide scan noise | Asset and shading matters, not receiver bias |

## 9. Risks

- **The radiance findings depend on the registered lighting** (bright unoccluded environment, no GI). R_D is added for that reason.
- **R_L is not a physical reference.** It omits environment occlusion and GI.
- **Convergence predictions are a two-point power law**, and the AA cost is unmeasured: accumulate pass, history bandwidth, jitter effect on traversal coherence.
- **Temporal risks:** reprojection on rough micro-geometry, disocclusion and off-screen return, view-dependent specular, fast-sun lag, resampling blur, and invalidation declarations for animated user shading.
- **Budget:** the fresh HRT floor (0.87–0.89 ms) is likely missed. VSM's floor is missed by every option. TSR-on Unreal is unmeasured in both time and image.
- **The policy change re-baselines** Exact, Level A and B and F0, all of which were measured under 1 mm.
- **The view-side rule needs per-material sidedness**, which is a new material-stage contract.
- **Static scope.** One quarry view and two poses, with no motion data in this unit.

## Evidence, reproduction and time

Evidence root: `D:/Coding/renderlab-work/m6-exact-quality/film-gap/`. `identities.sha256` hashes the script copy, `tri_probe.py` (hit-to-triangle mapping check), `out/film-gap.json`, `out/run.log` and the two inspected crops. Inputs, read in place and unchanged: `f0/quarry-{policy,minimal}-p{0,1}/`, `m6-level-b-probe/gate-final/reference/`, `m6-level-b-probe/runs/accept-b7/` (Exact and Interactive seeds 0–3 HDR and visibility), the decoded native masks, and the private source asset (statistics only; nothing copied). The UE source citations are from the installed 5.7.4 engine.

```powershell
$env:OPENBLAS_NUM_THREADS="2"; $env:OMP_NUM_THREADS="2"
py -3 -B scripts/m6-film-gap.py --f0 D:/Coding/renderlab-work/m6-exact-quality/f0 --out D:/Coding/renderlab-work/m6-exact-quality/film-gap/out
```

Measured CPU verification: three analysis runs of 25.8 s, 46.3 s and 44.8 s, the last being the retained output. The mapping probe ran once, under a minute, and was not stopwatched. The first run lacked the asset, regression and contrast sections; nothing was retracted. Design, reading and writing overlapped with the analysis and were not timed separately. There was no GPU, window, build or Unreal launch.
