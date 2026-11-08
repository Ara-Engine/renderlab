# DC1: projected box decals

Overnight unit, 2026-11-08. Base `984a18b`; branch `t3/m7-dc1`. Decals are active whenever the caller places them, on traced, visibility-buffer and forward lit shading. `set_decals(&[])` removes them.

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

Pending the owned GPU lease: the lib suite; a focused planar fixture on actual RT and no-RT devices; inspected quarry images on both paths; eight paired timing blocks. All generated output and the private Cargo target are under `D:/Coding/renderlab-work/m7-dc1`.

Reproduction:

```powershell
py -3 -B scripts/m7-dc1.py build dc1
py -3 -B scripts/m7-dc1.py gpu retained --binary dc1
```

The build helper uses two jobs and stops for QUIET windows. The run helper waits for the shared schedule and game holds, then executes every test binary through `scripts/n8-run.py` under `D:/Coding/renderlab-work/gpu-slot.lock`. It blocks until its owned run exits. The quarry manifest names the existing private local scans; no scan data is committed. The generated moss, painted mark and puddle textures are authored procedurally in the test scene.

## Remaining gaps

- No matched Unreal or path-traced decal campaign in this time-box. The overnight first pass does not establish the Unreal quality/performance floor.
- Manual trilinear filtering has no anisotropic decal filtering, compressed formats or streaming. Linear float texels prioritize a simple single-binding implementation over memory efficiency.
- One global grid may produce long candidate lists when decals are spread over a large world. There is no silently reduced quality or list overflow; partitioning and a GPU builder are future work.
- Decal normal-map variance is not included in specular AA's variance estimate.
- Opaque coverage/shadows, transparency integration, secondary reflection hits, animated/attached decal transforms and motion-heavy temporal reconstruction require integration verification.
