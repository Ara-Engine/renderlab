# P-SKY: what a sky map costs in shading, and half of it back

Follow-through on [design-sky-map-cost.md](design-sky-map-cost.md), off the PC. AMD Radeon 760M (RADV PHOENIX, Mesa), Linux. Release build, 1920×1080, Renderlab alone (no embedder), from main `ac7f46a`. Every timing is the median of the warmed half (frames 24–47) of one 48-frame run, from `RENDERLAB_PASS_LOG`. Repeats of the same binary agreed within 0.05 ms. The GPU was shared only through the machine's GPU lock, so no other GPU work ran during a run. **These are Radeon numbers. The RTX expectation is at the end.**

## Result

| Change | Verdict | Showcase-like RT shading, map sky |
|---|---|---|
| Environment lookups sample cube-array views with hardware filtering (`FLOAT32_FILTERABLE`) | **KEEP** | 65.5 → 58.2 ms |
| Reflection rays look up the sky on misses only | **KEEP** | 58.2 → 57.0 ms |
| Both, as shipped | | **65.5 → 57.4 ms. The map's premium over a constant sky drops from 16.5 to 8.5 ms on RT, and from 17.4 to 9.4 ms on non-RT.** |

Images match main to within the hardware's filter-weight rounding (section 4). The constant sky is not slower. The physical atmosphere also lights through these lookups, so its frames get faster too (4.3–4.9 ms less shading in the game scene).

## 1. The reproducer

`examples/reflection_matched.rs --stage --metal-balls --glass-balls` is the Showcase-like scene (P-DEFECT, P-SHOW2). `--sky map` now lights it with Showcase's gradient sky as a prepared `Sky::Map`, built in-process by `support/gradient_sky.rs`. Its layout is the one Ara's bridge uploads: the independent-roughness field, 65 levels of 64-texel faces, and a 16-texel irradiance cube. `--sky map-flat` keeps that layout and every texture fetch but stores one constant value. `examples/game_scene.rs` takes the same options.

| RT, still | Shading, ms |
|---|---|
| `--sky uniform` (constant) | 49.1 |
| `--sky map` (gradient) | 65.5 |
| `--sky map-flat` (same layout, constant value) | 65.5 |

- **The map costs 16.4 ms of shading here, against about 17 ms on the RTX 4070 SUPER (P-SHOW2).** On the Radeon the whole frame is slower and the premium is about the same.
- **The sky's content plays no part.** The flat map costs exactly what the gradient costs.
- **Nothing is constant-folded.** The sky type is a runtime uniform branch (`lighting.map.w`) inside a single shading pipeline. The constant sky runs the same compiled kernel and skips the lookups at run time. The comparison with the constant sky is therefore fair: the premium is the lookups the map actually executes.
- **The bridge's earlier face-size test (64, 32, 16: no change) already showed this is not bandwidth or working set.**

## 2. The suspects, one change at a time

These are screen-only edits to the composed WGSL, applied before compilation from one binary (the substitution hook stayed on a local branch). RT, still, map sky. Rows marked *cost-only* alter the image and only attribute time.

| Screen | Shading, ms | Δ | Reading |
|---|---:|---:|---|
| Baseline map | 65.5 | | |
| *cost-only*: escaped rays' sky lookup constant (`rl_sky`) | 58.6 | −6.9 | Misses pay for their lookups. |
| *cost-only*: hit programs' environment reads constant (`indirect_radiance`, `gi_open_irradiance` at reflection hits) | 59.1 | −6.5 | Hits pay about as much. |
| *cost-only*: primary receivers' environment read constant | 63.7 | −1.9 | The primary's own lookup matters least. |
| Sky looked up on misses only (explicit `else`; image-identical) | 63.2 | −2.4 | **The compiler does not sink the discarded lookup.** Every hit also paid for a miss lookup. |
| *cost-only*: one roughness level per lookup instead of two | 59.0 | −6.5 | Two levels cost about double. |
| *cost-only*: one texel load per level instead of four | 59.3 | −6.2 | The load count matters. |
| *cost-only*: cross-face seam path removed (still four in-face loads) | 61.6 | −3.9 | **The seam path costs 3.9 ms, though few lookups take it.** A wave pays for it if any of its lanes does: 4–16 glossy rays per pixel spread across faces. |
| Importance sampling or CDF search per pixel | — | — | Not in the runtime: the CDF belongs to the CPU cooker (design doc). Nothing to test. |
| A sky-type branch that turns off a cache or adds rays | — | — | None in the source (design doc). The flat map matching the gradient rules out content-driven ray counts. |
| Texture format: RGBA16F copy of the field (screen hack) | −1.5 on top of the kept changes | | Measured, **not taken.** Half precision needs range scaling for sun maps (design doc, format quality gate). |
| *cost-only*: fitted environment lobes off | −0.4 on top | | Small. |
| Guard the primary's untraced specular when traced light replaces it fully (`traced.w == 1`) | ±0.0 on top | | Rejected. Nothing to gain. |

**Cause.** The cost lies in the manual cube filter that every lookup runs: face selection, four `textureLoad`s per level, a seam path that diverges a whole wave, and two roughness levels. It runs at every glossy-ray miss, every reflection hit's diffuse and specular inputs, and the primary. On top of that, every hit also ran a miss lookup and threw the result away.

## 3. The fix

1. **Filtered cube-array lookups.** The field already stores each roughness level as six consecutive array layers, which is a cube array.
   - The lighting group's diffuse cube, specular field and second bank (bindings 6, 7 and 9) are bound as cube-array views of the same textures, with a bilinear sampler and nearest levels (binding 11). The group's texture count is unchanged.
   - `environment_level` and `environment_diffuse_at` (shaders/scene.wgsl) now take one `textureSampleLevel` per level, in place of face selection, four loads and the seam path.
   - The texture unit's seamless cube filtering samples across face edges and corners. These are the same texels and the same bilinear blend that `environment_tap` computed, with the same r⁴ blend between knots.
   - Legacy mip chains, their banked narrow levels and the atmosphere's procedural chain take the same path.
   - RGBA32F needs `FLOAT32_FILTERABLE` to filter, so the feature is now part of `RECEIVER_FEATURES`. Desktop Vulkan drivers (NVIDIA, AMD, Intel, Mesa) and Metal all expose it.
   - **A device without it is refused at start, never drawn wrong.**
     - `Capabilities::query` reports neither path as supported, because both feature sets include `RECEIVER_FEATURES`. The test `both_paths_require_filterable_f32_environments` pins this.
     - `Engine::new` then returns `InvalidInput("adapter lacks this render path's features or limits")`.
     - A caller that builds a `Renderer` itself gets `InvalidInput("receiver policy requires caller-enabled RECEIVER_FEATURES")` before any pipeline is made.
     - Ara's bridge reports `ARA_RENDERLAB_UNAVAILABLE` and keeps its own renderer.
2. **The sky is looked up on misses only.** This is in `rl_reflection_prepare` and in its software traversal template (src/program.rs). It returns the same values.

The manual filter (`environment_bilinear`, `environment_tap`, `environment_remap`, the face helpers) had no caller left and is removed. The CPU bank for a legacy chain's narrow levels (`bank_legacy`) is unchanged.

## 4. Images

The renderer is bit-deterministic run to run on this GPU: main against main is 0 codes on every frame, still and moving. Main against the branch, 1080p, 8-bit output. "Moving" is the examples' 48-frame camera path, frames 0, 8, …, 40 and 47.

| Capture | Max difference | Pixels over 2 codes | Pixels over 8 |
|---|---:|---:|---:|
| Showcase-like, RT, map, still | 1 | 0 | 0 |
| Showcase-like, non-RT, map, still | 2 | 0 | 0 |
| Showcase-like, RT, map, moving | 1 | 0 | 0 |
| Showcase-like, non-RT, map, moving | 7 | ≤ 4 | 0 |
| Game scene, RT, atmosphere, moving | 1 | 0 | 0 |
| Game scene, RT, map, moving | 1 | 0 | 0 |
| Game scene, non-RT, atmosphere, moving | 48 | ≤ 1695 | ≤ 108 |
| Game scene, non-RT, map, moving | 77 | ≤ 6831 | ≤ 403 |

- **Inspected:** each scene's last frame as main | branch | difference ×8, plus 2× crops of every area with differences over 8 codes. The sheets stay on the local branch.
- **Showcase-like scene and the RT game scene:** no visible difference.
  - The chrome, gold, aluminium and copper balls' sky reflections are unchanged, as are the glossy turntable and the horizon.
  - The difference panes are black, still and moving.
- **The non-RT game scene's differences are one band.** It runs along the ground plane's far edge, where software reflection rays leave at grazing angles toward the horizon.
  - Main renders that band as a stippled line, and so does the branch.
  - The change only moves which of its noisy pixels light up, so the band looks the same in both.
  - No new seams, no lost sky detail, no white pixels and no sparkle elsewhere.
  - That band's noise on main is a separate, existing defect.
- **Cube face edges and corners** are covered by `gpu_environment_axes_yaw_poles_seams_and_mips`, which passes on the filtered path. It tests 12 edges, the poles, yaw rotation and mip boundaries, and checks seam continuity under 0.002.

**Seen while capturing (not this change):** the non-RT *moving* game scene hangs RADV's gfx ring. The kernel charges the timeouts to the example's process, on main's binary and on the branch alike.
- The first run of each succeeded.
- After the first ring reset, every later run lost the device.
- Still captures and the Showcase-like scene never did.
- This is now in the gap list.

## 5. Before and after (Radeon 760M)

Shading pass in ms, 1080p, still, main `ac7f46a` against this branch, in one sitting.

| Scene, path, sky | main | branch | Δ |
|---|---:|---:|---:|
| Showcase-like, RT, map | 65.53 | 57.43 | −8.10 |
| Showcase-like, RT, constant | 49.07 | 48.93 | −0.14 |
| Showcase-like, non-RT, map | 113.35 | 105.62 | −7.73 |
| Showcase-like, non-RT, constant | 96.00 | 96.19 | +0.19 |
| Game scene, RT, atmosphere | 69.48 | 64.58 | −4.90 |
| Game scene, RT, map | 69.59 | 65.05 | −4.54 |
| Game scene, RT, constant | 62.83 | 61.71* | −1.12 |
| Game scene, non-RT, atmosphere | 129.79 | 125.49 | −4.30 |
| Game scene, non-RT, map | 129.88 | 126.00 | −3.88 |
| Game scene, non-RT, constant | 122.06 | 122.09* | +0.03 |

\* From the previous build of the same change. That build differed only in binding layout: three extra views instead of the replaced ones.

The map premium falls from 16.5 to 8.5 ms on RT and from 17.4 to 9.4 ms on non-RT. The constant sky's −1.1 ms in the game scene on RT is the miss-only change: the constant `rl_sky` also used to run on hits. Other passes are unchanged.

**What is left, by site on the new path (cost-only screens):** misses 2.1 ms, reflection hits 3.8, primary 0.9. These are the lookups' own sampling now, not the filter's overhead. The next measured step would be a half-precision field (−1.5 ms on this GPU), and it needs the range and quality gate first.

## 6. Expected on the RTX 4070 SUPER (for the build thread)

P-SHOW2's Showcase RT shading is 25.1 ms with the map against 8.5 with the constant sky. Ara's integration thread measured about 30 against 12 ms for the frame.
- The two changes remove what the Radeon says is the manual filter's overhead and the discarded miss lookups: about half the premium here, on both paths.
- **I expect roughly 7–9 ms back on Showcase RT**, so the frame lands near 21–23 ms against 12. Non-RT should drop by about the same fraction of its premium.
- The atmosphere sky should get faster too (game scene, Ara's outdoor levels). A saving much smaller than this on NVIDIA would mean NVIDIA already hid the manual filter's divergence and RGBA32F filtering costs more there. The cost-only screens in section 2 would separate those.

## 7. P-SKY2: the other half, as far as the image allows

The premium left after P-SKY was 8.5 ms (RT) and 9.4 ms (non-RT) on the Showcase-like scene. Same method: cost-only screens on the filtered path, then image-preserving candidates one at a time.
- The machine was shared with another thread's builds, which slow this iGPU. So each row interleaves baseline and screen, and only differences inside a pair (or the fastest of alternating runs) are read.

**Where the remaining time is (cost-only, RT, shading ms):**

| Screen | Δ |
|---|---:|
| Reflection hits' specular lookup made constant | −4.1 |
| One roughness level instead of two (every site) | −3.3 |
| Escaped rays' sky lookup made constant | −3.0 |
| Primary receivers' lookup made constant | −0.8 |
| Hits' diffuse irradiance made constant | −0.5 |

**What the suggested image-preserving cuts are worth:**
- **One level where roughness sits on a knot.** Lookups only land exactly on a knot as mirrors (r = 0) and at r ≥ 1, and both already take one level. Glossy cones and material roughnesses fall between knots, so dropping the second level there changes the r⁴ blend. The −3.3 ms is cost-only.
- **A prefiltered diffuse term stored once per hit, or a fetch shared between a hit's diffuse and specular inputs.** The hits' diffuse lookup is worth at most 0.5 ms even when replaced by a constant. The two inputs read different textures in different directions (normal against reflection), so there is no common fetch to share.
- **One trilinear sample in place of two bilinear ones.** Measured on the legacy chain, where the hardware can do it: −0.45 ms. The texture unit pays for the eight texels either way. So a 3D-texture layout, the only way to blend independent knots in hardware, would gain little, and would bring back manual face selection. Not built.
- **Fewer bytes per texel** is what moves the cost:

  | Field format, Showcase-like RT map | Shading ms (fastest of 4) | Image |
  |---|---:|---|
  | RGBA32F (P-SKY) | 57.12 | |
  | RGBA16F | 55.73 | max 1 code |
  | RGB9E5 | 54.70 | max 1 code in this scene, but rejected |

  RGB9E5's shared exponent quantizes the r⁴ interpolation test's values (1 + k⁴ alongside 2.0) to 1/128. That fails `gpu_environment_independent_banks_filtered`'s 0.0011 tolerance, a quality reduction by the repository's own gate. RGBA16F passes it.

**Kept:**
1. **Half floats for maps that fit them as given** (`Encoding`, src/environment.rs).
   - RGBA16F when every nonzero texel already lies in half precision's normal range, 2⁻¹⁴ to 65504. Each value then moves by at most 2⁻¹¹ of itself, and none is flushed or clipped.
   - Anything wider stays RGBA32F, for example a captured sun over dark ground, or the photometry test's 1e30 map.
   - There is deliberately no range scale. A power-of-two scale had to divide the lighting intensity, and the range test (1e30 map at intensity 1e20) overflowed it to infinity. With no scale, the intensity, lobe table, GI and every program see exactly what they did.
   - Ara's gradient and studio skies (hundreds of cd/m²) qualify.
2. **One trilinear sample for an unbanked legacy chain.** This is the atmosphere's procedural sky: `textureSampleLevel` at the fractional level, through the environment sampler, whose mip filter is now linear. Integer levels elsewhere stay exact.

**Before and after (P-SKY → P-SKY2, Radeon 760M, 1080p, still, shading ms):**

| Scene, path, sky | P-SKY | P-SKY2 |
|---|---:|---:|
| Showcase-like, RT, map | 57.30 | **55.74** |
| Showcase-like, non-RT, map | 104.73 | **102.73** |
| Game scene, RT, atmosphere | 64.6 | **64.15** |
| Game scene, RT, map | 65.1 | 65.1 (no measurable change: the map is a small part of this scene) |

- The Showcase-like rows are the fastest of 4 alternating runs. The game-scene rows are quiet interleaved pairs.
- **The map premium falls from 8.5 to about 6.8 ms on RT, and from 9.4 to about 6.5 ms on non-RT.**
- Against main, the premium is down from 16.5 to 6.8 ms on RT and from 17.4 to 6.5 ms on non-RT.

**Images, P-SKY → P-SKY2** (8-bit, 1080p; moving is the 48-frame camera path, 7 frames each):

| Capture | Max difference | Worst frame, pixels over 2 codes | Pixels over 8 codes |
|---|---:|---:|---:|
| Showcase-like, RT, still / moving | 1 / 1 | 0 / 0 | 0 |
| Showcase-like, non-RT, still / moving | 3 / 4 | 1 / 6 | 0 |
| Game scene, RT, atmosphere, moving | 1 | 0 | 0 |
| Game scene, RT, map, moving | 1 | 0 | 0 |

- Inspected as main | branch | difference ×8: the difference panes are black, and the reflections, horizon and sky are the same.
- On the GPU tests, `axes_yaw_poles_seams_and_mips` (a map that takes half floats), `constant_photometry_and_compatibility` (including the 1e30 range test, which stays f32) and `independent_banks_filtered` all pass.

**What remains is the lookups themselves.** Each glossy-ray miss and each reflection hit makes two filtered half-float samples, and the primary makes two more. Taking any of them away changes what the image integrates. The next step would be fewer rays (design-glossy-rate.md), not cheaper lookups.

## Process

- Implementation, about 1.5 h. Verification, about 1.5 h, including the GPU lock held about 25 minutes by another thread's compiler server.
- `cargo test --release --lib`: 207 passed (167 ignored GPU tests).
- GPU, under the machine's lock, `gpu_environment_*`. Three pass:
  - `axes_yaw_poles_seams_and_mips`
  - `constant_photometry_and_compatibility`
  - `independent_banks_filtered`, formerly `…_without_float_filter_feature`
- Four need PC-only inputs (`RENDERLAB_ENVIRONMENT_FIELD`, `_QUALITY`, `_NORMAL_FIXTURE_DIR`, `RENDERLAB_MAT1_MANIFEST`) and are for the build thread's batch.
- `ownership_admission_and_reload` fails at its live-bytes bound (environment_tests.rs:949). It fails on main identically on this GPU.
- Clippy: no warnings in the changed lines. The touched files have existing warnings on main.
