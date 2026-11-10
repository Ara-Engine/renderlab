# P-SKY: what a sky map costs in shading, and half of it back

Follow-through on [design-sky-map-cost.md](design-sky-map-cost.md), off the PC. AMD Radeon 760M (RADV PHOENIX, Mesa), Linux. Release build, 1920×1080, Renderlab alone (no embedder), from main `ac7f46a`. Every timing is the median of the warmed half (frames 24–47) of one 48-frame run, from `RENDERLAB_PASS_LOG`. Repeats of the same binary agreed within 0.05 ms. The GPU was shared only through the machine's GPU lock, so no other GPU work ran during a run. **These are Radeon numbers. The RTX expectation is at the end.**

## Result

| Change | Verdict | Showcase-like RT shading, map sky |
|---|---|---|
| Environment lookups sample cube-array views with hardware filtering (`FLOAT32_FILTERABLE`) | **KEEP** | 65.5 → 58.2 ms |
| Reflection rays look up the sky on misses only | **KEEP** | 58.2 → 57.0 ms |
| Both, as shipped | | **65.5 → 57.4 ms. The map's premium over a constant sky drops from 16.5 to 8.5 ms on RT, and from 17.4 to 9.4 ms on non-RT.** |

Images match main to within the hardware's filter-weight rounding (section 4). The constant sky is not slower. The physical atmosphere also lights through these lookups, so it gains as well.

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
   - The lighting group gains cube-array views of the specular field (binding 11), its second bank (12) and the diffuse cube (13), and a bilinear sampler with nearest mips (14).
   - `environment_level` and `environment_diffuse_at` (shaders/scene.wgsl) now take one `textureSampleLevel` per level, in place of face selection, four loads and the seam path.
   - The texture unit's seamless cube filtering samples across face edges and corners. These are the same texels and the same bilinear blend that `environment_tap` computed, with the same r⁴ blend between knots.
   - Legacy mip chains, their banked narrow levels and the atmosphere's procedural chain take the same path.
   - RGBA32F needs `FLOAT32_FILTERABLE` to filter, so the feature is now part of `RECEIVER_FEATURES`. The renderer already refuses a device without them. Desktop Vulkan drivers (NVIDIA, AMD, Intel, Mesa) and Metal all expose it.
2. **The sky is looked up on misses only.** This is in `rl_reflection_prepare` and in its software traversal template (src/program.rs). It returns the same values.

Bindings 6, 7 and 9 become cube-array views of the same textures, and binding 11 is the sampler, so the lighting group's texture count is unchanged. The manual filter (`environment_bilinear`, `environment_tap`, `environment_remap`, the face helpers) had no caller left and is removed. The CPU bank for a legacy chain's narrow levels (`bank_legacy`) is unchanged.

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
- **I expect roughly 7–9 ms back on Showcase RT**, so the frame lands near 21–23 ms against 12. Non-RT should gain by about the same fraction of its premium.
- The atmosphere sky should gain too (game scene, Ara's outdoor levels). A gain much smaller than this on NVIDIA would mean NVIDIA already hid the manual filter's divergence and RGBA32F filtering costs more there. The cost-only screens in section 2 would separate those.

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
