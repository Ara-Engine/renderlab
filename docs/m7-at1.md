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
  - Traced and visibility-buffer frames: TR1b's `primary_miss` is merged verbatim, with one change: its default branch returns `atmosphere_backdrop`.
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

RESULTS-PLACEHOLDER

## Gaps

See [m7-gaps.md](m7-gaps.md), "Atmosphere".
