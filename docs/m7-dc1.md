# DC1: projected box decals

Overnight unit, 2026-11-08. Base `984a18b`, merged main `35d27e6`; implementation checkpoint `3ac86a0`, branch `t3/m7-dc1`. Decals are active whenever the caller places them, on traced, visibility-buffer and forward lit shading. `set_decals(&[])` removes them.

## Implementation and API

`Decal` owns its transform, optional shared `DecalTextures`, linear RGBA factor, roughness, metallic, opacity, per-channel blend strengths, blend mode, stable sort order, angle fade and box-edge fade. The transform maps the local cube `[-0.5,0.5]^3` to world space. Local XY supplies UVs; local +Z is the preferred receiver normal. Scale sets the width, height and projection depth. Finite invertible affine transforms, including mirrored transforms, are supported.

```rust
use renderlab::{Decal, DecalBlend, DecalTexture, DecalTextureKind, DecalTextures};

let mark = DecalTexture::from_rgba8(
    DecalTextureKind::BaseColorSrgb, [width, height], &rgba8,
)?;
renderer.set_decals(&[Decal {
    transform: decal_to_world,
    textures: DecalTextures { base_color: Some(mark), ..Default::default() },
    roughness: 0.06,
    channels: [1.0, 1.0, 0.0, 0.0], // base, roughness, metallic, normal
    blend: DecalBlend::Alpha,
    sort_order: 10,
    ..Default::default()
}])?;
```

`DecalBlend::{Alpha,Multiply,Add}` controls base-colour compositing. Roughness, metallic and normal use their own alpha-over strengths, independently of the colour blend. Base alpha, the texture's alpha, opacity, edge fade and angle fade form coverage. Higher sort orders composite later; equal orders retain input order. `DecalTextureKind::MetallicRoughness` uses green roughness and blue metallic; `Normal` uses projected tangent-space XYZ. Texture roles are checked before publication. Base-colour mips use linear, premultiplied colour; all textures use explicit world-footprint trilinear filtering with clamped addressing.

The CPU conservatively bins transformed-box AABBs into a small world-space grid. Shading looks up one cell and tests only its sorted candidate list against the exact box and receiving angle. Lists have no overlap truncation. `DecalSettings::grid_resolution` is configurable through `set_decals_with_settings`; the default is 16×16×16. A single read-only storage table contains grid headers, references, records and mip texels, at lighting group 3 binding 21. It avoids a dependency on LC1 and adds no render pass, ray query or texture-binding-array requirement. Deferred devices should request the updated `visibility_limits`/`traced_limits` (12 storage buffers per stage).

Scene updates validate and pack on CPU. Only an admitted lit frame publishes an immutable GPU table. Old tables and their allocation accounting retire behind completed submissions, so in-flight frames keep the old scene. Identical tables do not republish. `decal_stats()` reports table bytes, grid cells, reference count and maximum cell occupancy; device storage limits bound publication. Upload accounting includes the table under material bytes.

## Programmable shading

The generated entries evaluate `decal_prepare()` before the user's `surface()`. `SurfaceInput.decals` holds affine base-colour and roughness/metallic blends plus a world-normal blend. The input's world point, geometric normal, UVs, derivatives, receiver and material identity stay intact. Decals never alter visibility or final lighting after `shade()`.

The standard program applies `decal_base_color(input, base)` and `decal_metallic_roughness(input, rough_metal)` to its resolved samples; `material_normal(input)` includes the normal overlay. Custom programs can use those helpers or the complete `material_base_color(input)` / `material_metallic_roughness(input)` convenience functions. `material_opacity(input)` exposes the resolved alpha. A custom surface may also apply `input.decals` to its own inputs, or deliberately ignore any channel. Existing raw `sample_*` services retain their mesh-UV meanings. There is no required BRDF or user `Surface` layout. GI cell shading uses the same decal input evaluation; decal changes join its existing bounce-relighting key.

Opacity controls decal coverage and the resolved material alpha exposed to programs. It does not change opaque geometry coverage, silhouettes or shadow casting. The TP1 transparency integration must choose how to consume material opacity. The frozen R0b reflection experiment uses identity decal inputs; production reflection hit integration remains R1's responsibility.

## Verification and evidence

Passed under `scripts/n8-run.py`, lease `dc1-retained-c-2`, 2026-11-08 16:28:35–16:29:59 UTC:

- **Lib:** 150 passed, 145 ignored, 0 failed, single test thread. CPU boundary tests cover linear premultiplied mip generation, odd extents, stable order, 80 overlapping decals without truncation, texture deduplication, alignment and invalid-input/budget rejection. Existing raster, traced, streamed and GI shader compositions pass.
- **GPU fixture, each path:** alpha, half opacity, multiply, add, out-of-box rejection, grazing-angle rejection, roughness/metallic/opacity, projected normal map, sort order, exact empty-scene restoration and a custom shader that ignores decals. Both devices report **0 Vulkan validation errors**, with the validation layer observed active. The non-RT device explicitly disables experimental ray queries and hit-vertex return.
- **Quarry, each path:** 13,034,903 triangles, 1920×1080, native 1x, anisotropy 1, TR1 off, GI off, prepared RLE2 environment, directional sun and soft shadows (Interactive on traced RT, Virtual on non-RT). Three generated decals: moss, orange painted mark and a dark puddle with roughness 0.035. Inspected full-frame off/on images and a 1 cm camera move, including silhouettes and shadowed rocks. Decals remain attached; the surrounding image has no visible regression. Changed pixels: RT 18,896, non-RT 18,894. The table is **1,085,856 bytes**, 4,096 cells, maximum one candidate per cell for this scene.

The puddle's low roughness is numerically verified, but this still view has little reflected scene detail; R1's secondary-hit integration is not certified by it. The textures are diagnostic generated patterns, not a claim of finished moss or wet-rock art. Forward lit composition passes the lib shader checks; forward GPU image inspection, decals plus GI, decals plus TR1 and a large camera-motion sequence remain unverified.

Inspected local images (the private scan and its captures are not committed):

| Path | Baseline | Decals | Moved camera | Fixture |
|---|---|---|---|---|
| Traced RT | [off](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/rt/quarry-off.png) | [on](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/rt/quarry-on.png) | [moved](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/rt/quarry-moved.png) | [alpha](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/rt/fixture-alpha.png) |
| Non-RT | [off](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/quarry-off.png) | [on](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/quarry-on.png) | [moved](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/quarry-moved.png) | [normal](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/fixture-normal.png), [half opacity](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/fixture-half-opacity.png) |

![Decals on the quarry, traced RT](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/rt/quarry-on.png)

![Decals on the quarry, non-RT](D:/Coding/renderlab-work/m7-dc1/runs/retained-c/nonrt/quarry-on.png)

## Diagnostic timing

One campaign in that lease: eight paired on/off blocks per path, alternating order, four measured frames per block after a publication/warm-up frame (32 samples per state). GPU timestamps exclude table publication and the setup-only Exact receiver frame. The retained binary is a development/test build (`renderlab` opt-level 1, wgpu opt-level 2), on an RTX 4070 SUPER, driver 617.14. Validation was enabled and foreign Rust compilers were active; **these are contended diagnostic medians, not a quiet performance claim**.

| Path | Shading off / on (ms) | GPU span off / on (ms) | Median paired shading delta (ms) |
|---|---:|---:|---:|
| Traced RT | 0.4540 / 0.4648 | 1.0250 / 1.0363 | +0.0112 |
| Non-RT | 0.4724 / 0.4831 | 1.1761 / 1.1832 | +0.0082 |

Tails are large: paired shading deltas range −0.209 to +5.880 ms on RT and −6.858 to +0.011 ms on non-RT; paired span deltas reach +5.133 / −29.069 ms. Preserve these misses rather than treating the medians as a certified decal budget. No empty-table overhead against pre-DC1 main, texture-publication latency or high-overlap cost was measured. The current result establishes functionality and a first cost sample only.

Evidence: [aggregate summary](D:/Coding/renderlab-work/m7-dc1/verification-summary.json), per-path `summary.json` / `test.log` beside the images, and lease preflight/resource samples under `D:/Coding/renderlab-work/m7-dc1/leases/runs/`. Retained executable SHA-256: `b41ca6a66e833696a63d4e5fabe257e437abf4b6c8eb8d72c8daada48bacca5b`. The successful lease took **83.7 s** including the lib suite, both devices, fixture/program compilation, quarry upload, captures and timings. Approximate time-box use: 20 minutes implementation/documentation and 15 minutes compilation, lease waiting, checks and inspection. The worker's Cargo target is deleted after verification; the verified standalone binary and small summaries/images remain on D:.

Two rejected screens are retained as logs: the first found three old standard-program instrumentation seams, restored without changing their tests; the second found that Interactive traced visibility does not populate the optional receiver attachments. The final harness uses a setup-only Exact frame to choose world-space placements, then restores Interactive before the baseline, inspected frames and timing.

Reproduction:

```powershell
py -3 -B scripts/m7-dc1.py build dc1
py -3 -B scripts/m7-dc1.py gpu retained --binary dc1
```

The build helper uses two jobs and stops for QUIET windows. The run helper waits for the shared schedule and game holds, then executes every test binary through `scripts/n8-run.py` under `D:/Coding/renderlab-work/gpu-slot.lock`. It blocks until its owned run exits. The quarry manifest names the existing private local scans; no scan data is committed. The generated moss, painted mark and puddle textures are authored procedurally in the test scene.

## Remaining gaps

- No matched Unreal or path-traced decal campaign in this time-box. The overnight first pass does not establish the Unreal quality/performance floor.
- Diagnostic timing has substantial tails and compiler contention; a quiet optimized cost, empty-table overhead, publication cost and overlap scaling are unmeasured.
- Manual trilinear filtering has no anisotropic decal filtering, compressed formats or streaming. Linear float texels prioritize a simple single-binding implementation over memory efficiency.
- One global grid may produce long candidate lists when decals are spread over a large world. There is no silently reduced quality or list overflow; partitioning and a GPU builder are future work.
- Decal normal-map variance is not included in specular AA's variance estimate.
- Forward GPU images, decals with GI/TR1, transparency integration, secondary reflection hits, animated/attached decal transforms and motion-heavy temporal reconstruction require integration verification. Opaque geometry coverage and shadow casting intentionally stay unchanged.
