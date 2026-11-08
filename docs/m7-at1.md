# M7-AT1: atmosphere and fog

Overnight complete-base unit (Islam, 2026-11-08; rules in AGENTS.md and [milestone-7.md](milestone-7.md)). Branch `t3/m7-at1`.

## What shipped

| Part | Default | Both paths | Notes |
|---|---|---|---|
| Physical sky (Hillaire 2020 transmittance, multiple-scattering and sky-view LUTs) | on | yes | written from the paper; Earth medium by default |
| Sky backdrop on primary misses | on | raster, traced, visibility buffer | TR1b's RLE2 visible option still replaces it when on |
| Sun disk on the backdrop | on | yes | true angular radius (0.00465 rad), limb darkened, through transmittance |
| Aerial perspective on every camera-path surface | on | yes | camera volume slices 64..96 (to 32 km) |
| Analytic exponential height fog | on | yes | closed-form optical depth; sun (HG) and sky-mean in-scattering |
| Sun colour and illuminance from transmittance | on while no captured map is selected | yes | directional lux reads as outside-atmosphere illuminance |
| Procedural environment source (IBL and GI cache take their sky from it) | on while no captured map is selected | yes | GPU-generated legacy-layout environment |
| Volumetric fog (froxels, temporal reprojection) | **off** | RT shadowed; non-RT unshadowed | see gaps |

A captured (RLE2) sky stays the scene's lighting whenever one is selected: its lights stay as given and the procedural environment steps aside.

## API

```rust
renderer.set_atmosphere(Atmosphere::default())?;       // the product default
renderer.set_atmosphere(Atmosphere::off())?;           // a pre-AT1 frame
let a = renderer.atmosphere();
let t = a.transmittance(altitude_m, toward_sun);       // CPU sun colour, per channel
```

- `Atmosphere { sky, sun_disk, aerial_perspective, sun_transmittance, environment, medium, height_fog, volumetric_fog }`.
- `AtmosphereMedium`: planet radii, ground height (world y), Rayleigh/Mie/ozone coefficients per km, ground albedo, sun radius.
- `HeightFog { density, falloff, base_height, start_distance, albedo, anisotropy }`: extinction `density * exp(-falloff * (y - base))` per metre. Default: 2e-4 /m at y = 0 with a 50 m scale height, so about 2 % extinction per 100 m near the ground.
- `VolumetricFog { distance, extra_density, history_weight }`: froxels over the first `distance` metres (default 128). The analytic height fog continues beyond that range.
- The sun is the first directional light. Its `lux` is the illuminance outside the atmosphere while `sun_transmittance` applies.

Lib tests construct renderers with `Atmosphere::off()`, since they predate AT1 and score shading against atmosphere-free references. AT1's own tests opt in. Every other build gets `Atmosphere::default()`.

## Design

- **LUTs** (`shaders/atmosphere.wgsl`, `src/atmosphere_gpu.rs`):
  - transmittance 256x64 (Bruneton's parameterization), with an extra row holding the sky's mean radiance (the fog ambient);
  - multiple scattering 32x32, from 64 directions per texel, Psi = L2 / (1 - f_ms);
  - sky view 192x108, with the paper's horizon-compressed latitude and the sun-relative longitude.

  All are rgba16f, per unit sun illuminance outside the atmosphere. The transmittance and multiple-scattering LUTs rebuild only when the medium changes. The sky view, ambient mean and aerial volume run every lit frame.
- **Camera volume:** one 3D texture with one column per 16x16 pixels, 64 froxel slices (exponential distribution to the volumetric range), then 32 aerial-perspective slices (quadratic to 32 km). One texture keeps the lighting group at three new sampled textures and one uniform. The Virtual shadow lookup already uses all 11 storage buffers per stage, so AT1 adds none.
- **Application:** the generated entries apply `atmosphere_surface` to each program's exposed output, in both `program_raster_entry.wgsl` and `program_traced_entry.wgsl`. Every program, built-in or user, gets the same fog, so shading stays programmable and no program changes. MSAA, TR1 and transparent surfaces each apply it per surface, which a post pass reading depth could not do. TP1 can call the same `atmosphere_surface(c, world)` from its transparent entry. The world radiance cache's cell shading isn't composed with it: bounce light carries no camera fog.
- **Misses:**
  - Traced and visibility-buffer frames: TR1b's `primary_miss` is merged verbatim, with one change: its default branch returns `atmosphere_backdrop`. Its seams in `program_traced_entry.wgsl`, `film_accumulation.rs`, `environment*.rs` and `primary_ray_gpu.rs` are applied byte-identically from `origin/t3/m7-tr1b`, so merging both branches conflicts only on that one line.
  - Raster frames: a full-screen triangle at the reversed-Z far plane is drawn last in the lit mesh pass with depth `GreaterEqual`, so it lands only on uncovered samples, MSAA included.
- **Procedural environment:** while no captured map is selected, the renderer generates a 64-texel, 7-level GGX-prefiltered specular chain and a 16-texel irradiance cube from the sky-view LUT, without the sun disk. They're in the legacy environment layout, so image-based light, ES1 and the radiance cache read them unchanged. The renderer regenerates them when the sun, the medium or the yaw changes. Each regeneration is a new map, which the GI cache sees as a new sky.
- **Volumetric fog:**
  1. Injection jitters each froxel's sample per frame (R2 sequence).
  2. The medium is the height fog plus `extra_density`; the sources are the sun (HG phase, atmosphere transmittance, visibility) and the sky mean.
  3. Each froxel blends with its trilinear reprojected history (weight 0.9).
  4. Integration runs front to back per column.

  Sun visibility uses one TLAS ray per froxel per frame on hardware renderers; non-RT renderers have no visibility source yet (gap).
- **Patents:** Hillaire 2020 is a published paper with no known patent claims; froxel volumetric fog (Wronski 2014) is long-published practice. No flagged technique (docs/milestone-7.md) is used.

## Verification

All runs were headless, inside m7-at1's GPU leases, with an optimized test binary and evidence on `D:/Coding/renderlab-work/at1/`.
- Host: RTX 4070 SUPER, Vulkan.
- Scene: the quarry (G1c's manifest), pose 0, 1920x1080, 1x.
- Contention: other overnight units' leases interleaved with ours; the wallpaper load was not controlled.
- Paths:
  - `raster`: the RT device's code default, raster primaries with Exact shadows;
  - `nonrt`: the visibility buffer with Virtual shadows;
  - `traced`: traced primaries with Interactive shadows.

**Lib tests** (inside the lease): `cargo test --lib`, 155 passed, 0 failed (146 GPU tests ignored), exit 0. AT1's own CPU tests cover:
- the CPU transmittance against the medium's closed form;
- validation;
- directional-only scaling;
- naga validation of the kernels, the raster sky module and the composed standard program.

**Images** (`gpu_at1_quarry`, `runs-out/<run>/<path>/<path>-<config>.png` plus raw exposed HDR `.f32`). Configurations:
- `pre-at1`: `Atmosphere::off()` with the quarry's captured RLE2 sky and 1000 lux sun, the old default image;
- `default`: `Atmosphere::default()` with the same captured sky;
- `noon` and `low`: the procedural sky and environment, a 120 klux sun at 65 and 4 degrees;
- `low-volumetric`: `low` plus 0.01/m volumetric fog.

Inspected (run b, sheet `runs-out/b/sheet.png`):
- **The default doesn't change the quarry's captured-sky protocol.** On every path, surfaces are 0.2 % darker than `pre-at1` (median ratio 0.998, p1 0.997): aerial perspective and height fog over the scene's 10 m. Misses go from black to the procedural sky, which is dim (at most 0.005 exposed) because that protocol's sun is only 1000 lux. Not visibly worse.
- **Noon:** a hazy blue sky with the planet's ground below the horizon. Quarry surfaces carry sky light from the procedural environment and a white transmitted sun.
- **Low sun (4 degrees):** an orange sky graded to blue-grey overhead, a sun disk at the right elevation, and a backlit quarry silhouette lit by the sky.
- **Paths agree.** Raster misses match the deferred paths to 0.2 % of the sky's peak, except 117 sun-disk rim pixels: the disk edge isn't anti-aliased, and the two direction computations land differently there. Traced and non-RT skies are identical.
- **Volumetric fog (0.01/m):** a warm haze toward the low sun. Mean surface radiance is 0.064 R on traced, where froxels are shadowed by TLAS rays, and 0.075 R on non-RT, which is unshadowed (gap). Shadowed froxels under the rocks darken on the RT paths.

Run a found one bug, fixed before run b: the raster sky, drawn last with a depth `GreaterEqual` test at the far plane, never landed. The sky is now drawn first in the pass without depth, and the surfaces cover it.

**Timing** (quarry, static camera, 60 frames per configuration, two interleaved rounds; ms per frame wall clock, and shading as the timed mesh/shading pass). Contended by other units' work; development host; not a quiet-window claim.

| Path | Atmosphere off | Default (captured sky) | Volumetric vs procedural-low | Shading, off to default |
|---|---:|---:|---:|---:|
| non-RT (VB + Virtual) | 1.63-1.67 | 1.70-1.71 (+0.04-0.08) | +0.05-0.11 | 0.70 to 0.69 |
| traced + Interactive | 1.35-1.36 | 1.44 (+0.08-0.09) | +0.01-0.04 | 0.65 to 0.67 |
| raster + Exact (RT code default) | 15.9-16.4 | 16.3-16.7 (about +0.3, noisy) | noisy | 4.0-4.1 to 3.9 |

The sky, aerial perspective and height fog cost under 0.1 ms on the deferred paths. On raster, the full-screen sky draw adds about 0.3 ms inside 16 ms frames that are dominated by Exact shadows.

**The procedural environment's cost, found and fixed:**
- **Run b, the first form** (a 64-texel specular chain with 7 levels, down to 1x1): +0.47 ms of quarry shading on every path against the captured map at the same sun. Shading went 0.69 to 1.16 ms on non-RT, 0.67 to 1.12 on traced, and 3.9 to 5.2-5.5 on raster. The low sun alone costs nothing (`off-low` equals `off`). Rough lookups into 1-4 texel faces are mostly the environment's cross-face corner taps.
- **Run c, the shipped form** (128 texels, 5 levels, smallest face 8): shading is within run noise of the captured map. Non-RT measured 0.76-0.83 against 0.70-0.82, traced 0.81-0.91 against 0.70-0.77, raster 4.20-4.27 against 4.24-4.37. Run c's wall clock was noisier (non-RT 2.0-4.3 ms on unchanged configurations), so only the shading-pass deltas are cited from it.
- **The images barely moved:** surface median ratio c/b 0.999 at noon and 0.995 at the low sun, with p1-p99 of 0.97-1.02 and 0.93-1.12. Roughness steps 0.25 instead of 1/6.

Evidence: `D:/Coding/renderlab-work/at1/runs/{a,b,c}` (lease logs), `runs-out/{a,b,c}/<path>/` (PNGs and `<path>-at1.json` with per-configuration means and timing), and the sheets `runs-out/{b,c}/sheet.png`. Raw `.f32` dumps were deleted after these summaries (D: bulk rule).

Not run: no Unreal or reference comparison (overnight rules), no motion sequence, no MSAA raster image (the sky pipeline takes the renderer's sample count, but only 1x was rendered), and no visible-environment (TR1b option) image.

## Gaps

See [m7-gaps.md](m7-gaps.md), "Atmosphere".
