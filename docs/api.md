# Renderlab engine API

The engine-facing API is `renderlab::engine`. A game owns one `Engine`. It creates resources and objects through typed handles, sets lights, sky and settings, and calls `render` once per frame. Everything else in the crate root is the low-level renderer the engine wraps. That renderer's tuning surface changes as features are measured; the engine module is the part that stays stable (see [Stability](#stability-and-freeze-notes)).

Status: unit API1 (2026-11-08). Phase 1 covered main `984a18b`; phase 2 has wired DC1, TR1b, R1, G2b and AT1 (main `f159fcd`) and wires the other overnight feature units as they merge (see [Planned extension points](#module-layout-and-planned-extension-points)).

## Principles

- **One owner.** The `Engine` owns the device, the renderer, every resource and every object. Handles (`MeshId`, `MaterialId`, `ObjectId`, ...) are `Copy` and safe to store in a game's own components. A removed handle is rejected with `Error::InvalidInput`; it never reaches a reused slot.
- **Two first-class paths.** `RenderPath::HardwareRt` and `RenderPath::NonRt` render the same features against the same quality bar. Game code is identical on both. Only creation names the path.
- **Blocking only for backpressure.** Uploads and frames are bounded on the GPU. When all frames in flight are busy, a call waits (polling, not sleeping) and keeps the frame events for `poll`. A call that cannot be admitted within 20 s returns `Error::InvalidInput("... timed out")`.
- **Coarse settings.** `Settings` names choices a game makes (GI on, temporal reconstruction on, a quality preset, exposure). Each path maps them to its measured configuration. Estimator tuning stays on `engine.renderer_mut()`.
- **Programmable shading.** Every material runs a user WGSL program. The standard photoreal program is just the default one.

## Quick start

```rust
use renderlab::engine::*;

let adapter = headless_adapter()?; // or your windowed adapter
let caps = Capabilities::query(&adapter);
let path = caps.preferred().ok_or("no supported render path")?;
let mut engine = Engine::new(&adapter, EngineDescriptor {
    path,
    // HardwareRt only: the caller accepts wgpu's experimental ray-query contract.
    experimental_ray_query: (path == RenderPath::HardwareRt)
        .then(|| unsafe { ExperimentalRayQuery::acknowledge() }),
    // GI, temporal reconstruction, the physical sky, aerial perspective and height fog
    // are on by default.
    settings: Settings {
        exposure: Exposure::Manual { stops: -14.5 },
        ..Default::default()
    },
    ..Default::default()
})?;
engine.resize(1280, 720)?;

let cube = engine.create_mesh(&MeshData { vertices: &vertices, indices: &indices })?;
let stone = engine.create_material(&MaterialDesc {
    base_color: [0.6, 0.58, 0.55, 1.0],
    roughness: 0.9,
    ..Default::default()
})?;
let crate_ = engine.spawn(cube, stone, Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)))?;

// The default sky is the physical atmosphere, which colours this sun through its air.
engine.set_sun(Some(Sun { travel: Vec3::new(-0.45, -0.75, -0.5), ..Default::default() }))?;

loop {
    engine.set_transform(crate_, model_for_this_frame)?;
    let camera = Camera::look_at(eye, target, Vec3::Y, 55f32.to_radians(), 0.05, 500.0)?;
    engine.render(&camera)?;
    let events = engine.poll()?; // completed frames and GPU pass timings
}
```

`examples/game_scene.rs` is the complete version: procedural props, a toon-styled hero, mirror and glossy spheres, decals, sun, the physical sky and fog, GI, reflections, temporal reconstruction and a camera fly path, offscreen, with PNG frames.

## Creating an engine

`Capabilities::query(&adapter)` reports, from the adapter's features and limits alone, whether it can run each path:

| Path | Needs | Primary visibility | Sun visibility | GI rays |
|---|---|---|---|---|
| `HardwareRt` | ray queries with hit-vertex return, descriptor indexing (`TRACED_FEATURES`, `traced_limits`) | one traced ray per pixel | traced: 8 disk directions per pixel (`Quality::Realtime`) | hardware |
| `NonRt` | 64-bit storage atomics, descriptor indexing (`VISIBILITY_*`, `VIRTUAL_FEATURES`, `visibility_limits`) | visibility buffer (software and hardware raster of cluster cuts) | cached sun clipmap with exact triangle confirmation (`SoftShadows::Virtual`) | software BVH |

`Capabilities::preferred()` picks hardware RT where available. `Engine::new(&adapter, descriptor)` creates the device itself, with exactly the path's features and limits, the adapter's buffer-size limits (large scans need one big buffer) and, where available, timestamp queries and the pipeline cache. So the game never negotiates Renderlab's feature list.

`HardwareRt` needs `ExperimentalRayQuery`, made by the `unsafe fn acknowledge()`. wgpu's ray queries are experimental, and the caller accepts that risk explicitly. Renderlab never enables experimental features on its own.

`EngineDescriptor` fields (construct with `..Default::default()`):

| Field | Default | Meaning |
|---|---|---|
| `path` | `NonRt` | the render path |
| `experimental_ray_query` | `None` | required for `HardwareRt` |
| `output_format` | `Rgba8UnormSrgb` | format of every colour target; `capture` needs RGBA8 |
| `limits` | 1024 instances per batch, 128 MiB meshes and textures | development policy limits; device limits always apply |
| `settings` | `Settings::default()` | initial frame settings |
| `texture_anisotropy` | 1 | anisotropic filtering, 1-16, fixed for the engine's lifetime |
| `pipeline_cache` | `None` | directory for the persistent program pipeline cache |

**Windowed games.** Create the surface on your own `wgpu::Instance`, request an adapter compatible with it, create the engine on that adapter, and configure the surface with `engine.device()` and `engine.output_format()`. Each frame, call `engine.render_to(&surface_texture.texture, &camera)` and then `engine.present(surface_texture)`. Resizing the window only means reconfiguring the surface: the engine follows the target's extent, and the camera's aspect follows the target.

## Resources

| Call | Returns | Notes |
|---|---|---|
| `create_mesh(&MeshData)` | `MeshId` | 48-byte `Vertex` (position, normal, uv, colour), u32 triangle list. On `NonRt` it also builds the mesh's GI ray geometry (CPU time proportional to triangle count). |
| `create_mesh_with_tangents(&MeshData, &TangentStream)` | `MeshId` | needed by normal-mapped materials |
| `create_texture(TextureData)` | `TextureId` | a prepared mip chain (`renderlab_asset::Texture`), for example from the cooker |
| `create_program(label, wgsl)` | `ProgramId` | composed and checked for both paths now; see [Programs](#shading-programs) |
| `reload_program(id, wgsl)` | `ReloadReport` | transactional: a rejected edit keeps the last working pipelines |
| `standard_program()` | `ProgramId` | the built-in photoreal program |
| `create_material(&MaterialDesc)` | `MaterialId` | factors, textures (`TextureRef`), program and parameter values |
| `set_material_parameters(id, &[(name, Value)])` | | a buffer write, no shader compile |
| `create_environment(&PreparedEnvironment)` | `EnvironmentId` | an RLE1/RLE2 HDR map for `Sky::Map` |
| `add_asset(asset::Asset)` | `AssetHandles` | a cooked `.rla`: textures, materials, one mesh per primitive, one object per placed primitive |
| `remove_*(id)` | | meshes and materials only once no object uses them; materials keep their own textures and program alive |

`MaterialDesc::default()` is an untextured, double-sided, white dielectric (metallic 0, roughness 0.5) on the standard program. Keep `double_sided: true` for now: both paths' shadowed primaries currently need double-sided materials.

## Objects

`spawn(mesh, material, transform) -> ObjectId`, `set_transform(object, Mat4)`, `transform(object)`, `despawn(object)`.

Objects that share a mesh and a material are drawn together as one instanced draw. Moving an object is a transform write on the next frame. Spawning or despawning re-uploads that group's transforms on the next frame. One group holds at most `limits.max_instances` objects (raise it in the descriptor for large crowds or forests).

## Decals

`set_decals(&[Decal])` replaces the scene's projected box decals atomically; an empty slice removes them (DC1, [m7-dc1.md](m7-dc1.md)). A `Decal`'s `transform` maps the local cube [-0.5, 0.5]³ to world space. Local XY carries its UVs, local +Z is the receiving surface's preferred normal, and the Z scale is the projection depth. A ground decal therefore rotates local +Z to world +Y:

```rust
let emblem = DecalTexture::from_rgba8(DecalTextureKind::BaseColorSrgb, [256, 256], &rgba8)?;
engine.set_decals(&[Decal {
    transform: Mat4::from_translation(position)
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::new(2.0, 2.0, 0.6)),
    textures: DecalTextures { base_color: Some(emblem), ..Default::default() },
    channels: [1.0, 0.0, 0.0, 0.0], // base colour, roughness, metallic, normal strengths
    ..Default::default()
}])?;
```

`DecalBlend::{Alpha, Multiply, Add}` composites base colour; roughness, metallic and normal blend by their own `channels` strengths. Decals edit material inputs before a program's `surface` reads them, on both paths, and GI bounce sees them. A program opts in by calling `decal_base_color(input, base)` and `decal_metallic_roughness(input, vec2(rough, metal))` on its resolved inputs (`material_normal` already includes the normal overlay), as standard.wgsl and the example's program do; a program that ignores them is unaffected. Textures are copied when `set_decals` is called, so the caller may drop its handles. The decal grid's resolution stays on `renderer_mut().set_decals_with_settings`.

## Lighting and exposure

- `set_sun(Option<Sun>)`. The sun is the scene's one shadowed directional light. `Sun::travel` is the direction its light travels. `illuminance` is lux per channel: under `Sky::Atmosphere` it is the sun outside the atmosphere (default 120,000), which the atmosphere dims and warms; under the other skies it arrives as given (clear midday is about 100,000). `angular_radius` (radians, 0-0.1) sets penumbra width. Its default, 0.04, is the measured shipping configuration on both paths; 0 gives hard shadows. The visible sun disk keeps the real sun's size (the medium's `sun_radius`), independent of the penumbra.
- `set_lights(&[Light])`: local point and spot lights, in candela with an optional range. They are unshadowed at this revision, and a soft sun must currently be the only light (see [Current limits](#current-limits)); many-light sampling and shadows arrive with LC1. Directional lights go in the sun.
- `Settings::exposure`: `Exposure::Manual { stops }` scales scene luminance by 2^stops before tone mapping. With sunlight in lux, -14 to -16 is typical.

## Sky, atmosphere and fog

`set_sky(Sky)` chooses what lights the scene from far away and what camera rays that miss everything show. The view-path media (aerial perspective and fog) are `Settings` fields, independent of the sky. Everything here is AT1 ([m7-at1.md](m7-at1.md)) and renders on both paths.

| `Sky` | Lights the scene with | Background | Sun |
|---|---|---|---|
| `Atmosphere(AtmosphereMedium)`, **the default** (Earth) | a procedural environment generated from the physical sky (Hillaire 2020), so image-based light and the GI cache see it | the physical sky with a limb-darkened sun disk | `illuminance` is outside the atmosphere; transmittance at the camera's altitude dims and colours it, so low suns turn orange |
| `Uniform(radiance)` | a constant radiance in cd/m² | the same constant | arrives as given |
| `Map { map, rotation_y, intensity, background }` | a captured HDR environment (`create_environment`) | `Background::Color(radiance)`, or `Background::Sky` for the map along each pixel's ray (TR1b's visible sky) | arrives as given |

`AtmosphereMedium` holds the planet and air: radii, `ground_height` (world y of the planet surface), Rayleigh, Mie and ozone coefficients per km, ground albedo and the visible sun radius. The procedural environment regenerates when the sun, the medium or the yaw changes, and the GI cache treats each regeneration as a new sky.

| `Settings` field | Default | What it adds |
|---|---|---|
| `aerial_perspective: bool` | on | scattering along every camera ray to a surface: distant surfaces fade toward the sky colour (camera volume to 32 km) |
| `height_fog: Option<HeightFog>` | `Some(HeightFog::default())`: about 2 % extinction per 100 m near y = 0, 50 m scale height | analytic exponential fog, `density * exp(-falloff * (y - base_height))` per metre, lit by the sun (Henyey-Greenstein `anisotropy`) and the sky mean |
| `volumetric_fog: Option<VolumetricFog>` | `Some(VolumetricFog::default())`: 128 m range, 0.001/m extra density, history weight 0.9 | shadowed froxel fog: the height fog's medium plus `extra_density`; None disables it (VF1) |

Aerial perspective and fog use the physical medium's sky colour under every `Sky`. Every program's output gets the same fog in the generated entry, so custom programs need no changes. Volumetric fog's sun visibility uses rays on `HardwareRt` and a depth test of resident Virtual clipmap pages on `NonRt` (conventional raster frames read their light-space depth maps). Missing Virtual pages fall back to coarser resident levels, then to sunlight. Reprojection rejects changed lighting, media and uploaded casters, clamps old scattering to the medium's lighting range and reduces history during visibility changes. See [VF1](m7-vf1.md).

## Settings and quality presets

```rust
pub struct Settings {
    pub quality: Quality,                 // Realtime (default) | Reference
    pub global_illumination: bool,        // world radiance cache: sky visibility and bounce
    pub temporal_reconstruction: bool,    // TR1 jittered accumulation and antialiasing
    pub render_scale: RenderScale,        // Native | Quality (2/3 per axis) | Performance (1/2)
    pub exposure: Exposure,
    pub aerial_perspective: bool,         // see Sky, atmosphere and fog
    pub height_fog: Option<HeightFog>,
    pub volumetric_fog: Option<VolumetricFog>,
}
```

- `Quality::Realtime` is each path's measured real-time configuration (the table above).
- `Quality::Reference` integrates sun visibility exactly: all 256 disk directions per pixel on RT, and the exact triangle-tested map lookup without RT. It is for stills, captures and A/B checks, not real-time play.
- `render_scale` is TR1b's upscaling preset: with temporal reconstruction on, every pass renders at the scale's internal size and TR1 reconstructs the target. It needs `temporal_reconstruction`; a scale below `Native` without it is rejected.
- `set_settings` applies a change only when something changed, and validates before touching the renderer. It overwrites any overlapping tuning made through `renderer_mut()`.

Defaults follow each feature unit's default ruling, not the facade's opinion. With G2b and AT1 merged:

- GI is **on** (G2b; `global_illumination: false` maps to `IndirectSettings::off()`).
- Temporal reconstruction is **on** at native scale (TR1b; its remaining losses to no-AA and TSR are in [m7-gaps.md](m7-gaps.md)).
- The physical atmosphere sky, aerial perspective and height fog are **on**; volumetric fog is off (AT1).
- Traced reflections are on wherever a program asks for them (R1, see [Reflections](#reflections)).

Gaps against Unreal and the reference for each are in [m7-gaps.md](m7-gaps.md).

Rendering is single-sample on both quality paths. Temporal reconstruction is the antialiasing; MSAA belongs to the low-level raster path only.

## Reflections

Reflections are a property of the shading program, not a setting. A program asks for them with the optional hook

```wgsl
fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe  // SpecularLobe { normal, alpha, weight }
```

and reads the result with the engine input `specular_incident(input) -> vec4<f32>`. Its rgb is the mean exposed radiance over the lobe, and w is the traced fraction, so the program blends it over its own untraced estimate: `mix(untraced, traced.rgb * reflectance, traced.w)`. `alpha` is perceptual roughness squared, and `weight > 0` requests rays.

| Path | What a requesting program gets (M7-R1, [m7-r1.md](m7-r1.md)) |
|---|---|
| `HardwareRt` | One mirror ray below roughness 0.05, and four fixed GGX strata up to 0.35, fading to the untraced estimate by 0.5 (w falls from 1 to 0 over 0.35-0.5). Hits are shaded by the frame's program, with the hit's own material, decals and a hard shadow ray. TR1 integrates the glossy strata over frames. One bounce. |
| `NonRt` | w = 0 everywhere, so the program's untraced estimate stands: `indirect_radiance`, the environment, occluded and with bounce when GI runs. Software-traced non-RT reflections (R1n) are in progress and need no API change. |

The standard program requests them for every material. The example's program shows per-material control: its `reflect` parameter is the lobe weight, so a material with `reflect: 0` asks for no rays. The renderer has no global reflection switch at this revision.

R1 records three constraints. The hook is GGX-shaped, so a non-GGX style gives its nearest GGX lobe or weight 0. Hits read the open environment, not the GI cache. A program that declares the hook but not the input, or the reverse, traces nothing.

## Frames

| Call | Meaning |
|---|---|
| `resize(w, h)` | creates or resizes the engine's own output target (offscreen use and `capture`) |
| `render(&Camera)` | renders into the engine's output |
| `render_to(&wgpu::Texture, &Camera)` | renders into a caller target, such as a surface texture, in the output format |
| `present(SurfaceTexture)` | presents a `render_to` surface frame |
| `cut()` | the next frame starts without history: a camera cut or teleport |
| `poll()` | completed frames and GPU pass timings since the last poll (nonblocking) |
| `capture()` | blocking RGBA8 readback of the engine's output |

`Camera::look_at(eye, target, up, fov_y, near, far)` is right-handed with reversed finite depth. Its aspect follows the target. `render` returns a `FrameReport`: diagnostic counters for the frame, including the path that resolved it.

## Shading programs

A program is WGSL without bindings, composed with the engine's prelude and API (shaders/program_api.wgsl and the raster and traced stage APIs):

```wgsl
struct Surface { ... };                                  // anything the program needs
fn surface(input: SurfaceInput) -> Surface { ... }       // material evaluation
fn shade(s: Surface, input: SurfaceInput) -> vec3<f32> { ... } // exposed linear HDR
// optional
struct Parameters { ... };                               // f32, vecN<f32>, u32, i32, fixed arrays
const DEFAULT_PARAMETERS: Parameters = Parameters(...);
fn scatter(s: Surface, input: SurfaceInput, wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32> { ... } // bounce light
fn emission(s: Surface, input: SurfaceInput) -> vec3<f32> { ... }
```

Engine inputs a program may call include `sample_base_color`, `sample_metallic_roughness`, `material_normal`, `light_count`, `light_sample`, `light_visibility` (world-space, never screen-space), `indirect_irradiance` and `indirect_radiance` (GI-occluded sky and bounce), `environment_lobe*`, `specular_incident` (traced reflections), `decal_base_color` and `decal_metallic_roughness`, `brdf`, `expose` and `exposed_product`. The optional `reflection_lobe` hook joins `scatter` and `emission` (see [Reflections](#reflections) and [Decals](#decals)). shaders/programs/standard.wgsl and toon.wgsl are complete references. A program without `scatter`/`emission` still blocks light but reflects no bounce light.

**One program per frame (current limit).** Both quality paths shade every pixel of a frame with one program: traced RT primaries and the non-RT visibility buffer reject frames whose materials use different programs. Per-program dispatch is planned (M7 design, R1). Until it lands, give per-object styles to materials of one program through a `Parameters` member, as the example does: examples/game_scene.wgsl is the standard program plus a toon branch selected by `style`, so the hero is toon-shaded and the props photoreal in the same frame. When per-program dispatch lands, separate programs per material will work with no API change.

## Current limits

- One shading program per frame on both quality paths (above). MP1 lifts it, with no API change.
- At most 64 mesh-material groups per frame (`TRACED_DRAWS`), on both paths. Objects sharing a mesh and material count once.
- Materials must be double-sided while the sun casts shadows.
- Local lights are unshadowed, and a soft sun (`angular_radius > 0`) must be the only light: the renderer accepts soft sun visibility only for a light list of exactly one directional light. `set_lights` with a soft sun returns `InvalidInput`. Use `angular_radius: 0.0` (hard sun) to combine the sun with local lights. Many-light shading (LC1) is expected to lift this.
- On `NonRt`, removing a mesh does not yet release its GI ray geometry, which the renderer keeps until it drops. Avoid heavy mesh churn on that path.
- On `NonRt`, reflections are the environment estimate only (R1n adds software-traced reflections), and volumetric fog is unshadowed (no sun shafts).
- The GI cache shows bright speckles near metallic objects and faceted streaks on some flat faces, on both paths. G2c owns the fix.
- Virtual geometry (streamed cluster meshes) and the diagnostic view modes stay on the low-level `Renderer`.

## Module layout and planned extension points

```
renderlab::engine            Engine, EngineDescriptor, Capabilities, RenderPath, ExperimentalRayQuery,
                             MaterialDesc, TextureRef, AssetHandles, Image, headless_adapter
renderlab::engine (settings) Settings, Quality, Exposure, Sky, Background, Sun
renderlab::engine (ids)      Id<T>, MeshId, TextureId, ProgramId, MaterialId, EnvironmentId, ObjectId
renderlab::engine re-exports Camera, Light, Mat4, Vec3, MeshData, Vertex, Value, Limits,
                             RenderScale, AtmosphereMedium, HeightFog, VolumetricFog,
                             Decal, DecalBlend, DecalTexture, DecalTextureKind, DecalTextures,
                             Sampler, TextureData, TangentStream, PreparedEnvironment,
                             Error, ReloadError, ReloadReport, FrameReport, Events, FrameId, Renderer
renderlab::*                 the low-level renderer (expert API, not frozen)
```

Each overnight unit lands in a named slot. Wired rows are on this branch; the others are the intended shape, which phase 2 confirms against what each unit merges.

| Unit | Feature | Planned engine API |
|---|---|---|
| TR1b | TR1 default, upscaling presets | **Wired**: `Settings::temporal_reconstruction` on by default; `Settings::render_scale` (`RenderScale::{Native, Quality, Performance}`); `Background::Sky` for the visible sky |
| G2b | GI default | **Wired**: `Settings::global_illumination` on by default, off maps to `IndirectSettings::off()`; no new calls |
| R1 | reflections | **Wired**: the `reflection_lobe` hook and the `specular_incident` input (see [Reflections](#reflections)). No setting, because the renderer has no switch. R1n's non-RT traced reflections need no API change. |
| TP1 | transparency, particles | `MaterialDesc` blend and transmission fields for glass; `create_particles(ParticleDesc) -> ParticlesId`, `set_particles`, `remove_particles`. The example gets a glass pane and a particle emitter. |
| AT1 | atmosphere, fog | **Wired**: `Sky::Atmosphere(AtmosphereMedium)` (the default sky); `Settings::{aerial_perspective, height_fog, volumetric_fog}`. See [Sky, atmosphere and fog](#sky-atmosphere-and-fog). |
| SK1 | skinned meshes | `create_skinned_mesh(..) -> MeshId` with joints and weights; `set_pose(object, &[Mat4])`. The example's hero becomes a skinned character. |
| LC1 | many lights | `set_lights` keeps its shape; per-light shadows and the light budget arrive as `Light` fields or a `LocalLight` wrapper. LC1 also lifts the soft-sun-only limit, and the example's lantern returns. |
| DC1 | decals | **Wired** (phase 2): `set_decals(&[Decal])`, with `Decal`, `DecalBlend`, `DecalTextures`, `DecalTexture` and `DecalTextureKind` re-exported. See [Decals](#decals). |
| PP1 | post-processing | `Settings::post: PostSettings` (bloom, motion blur, depth of field, tone mapper, white balance, grading, vignette, grain), with `PostSettings`, `Tonemapper`, `DepthOfField` and `ColourLut` re-exported, following PP1's branch; an `Exposure::Auto { .. }` variant if PP1 adds auto exposure |
| R1n | non-RT traced reflections, reflection switch | `Settings::reflections: ReflectionSettings` (`enabled`, `max_roughness`), re-exported; the program hooks are unchanged |
| MP1 | per-program dispatch | no API change: materials of different programs share a frame. The example's hero moves to its own toon program, and the `style` workaround goes. |
| G2c | GI speckles and streaks | no API change |

New handles follow the `Id<T>` pattern, new settings are new fields with defaults, and new sky and exposure kinds are new enum variants. None of these change an existing call.

## Stability and freeze notes

These notes are final for everything wired above. Rows still marked planned become final when their unit merges and the row is updated.

**Stable** (changes only with a recorded migration):

- `Engine` method names and signatures above; `EngineDescriptor`, `Capabilities`, `RenderPath`, `ExperimentalRayQuery`.
- Handle semantics: `Copy` typed `Id<T>`, stale handles rejected, the batching of objects by mesh and material.
- `MaterialDesc`, `TextureRef`, `Sun`, `Sky::Atmosphere`/`Uniform`/`Map`, `Background::Color`/`Sky`, `AtmosphereMedium`, `HeightFog`, `VolumetricFog`, `Exposure::Manual`, `Quality::Realtime`/`Reference`, `RenderScale`, and the meaning of each `Settings` field.
- `set_decals(&[Decal])` and the re-exported decal types.
- The program contract: `surface`, `shade`, optional `Parameters`, `DEFAULT_PARAMETERS`, `scatter`, `emission`, `reflection_lobe`, and the documented engine inputs.

**May grow** (additive only): new `Settings` fields with defaults, new variants of the `#[non_exhaustive]` enums (`Sky`, `Background`, `Exposure`, `Quality`), new engine calls and handle kinds, new engine inputs for programs.

**May change**: `FrameReport`, `Events` and the other diagnostic counters; defaults that a unit's ruling flips; every low-level crate-root type and `renderer_mut()` tuning; the current limits listed above, which should only loosen.

**Re-exported renderer types** (`AtmosphereMedium`, `HeightFog`, `VolumetricFog`, the decal types, `RenderScale`) are plain parameter structs and enums owned by their units. They are part of the stable surface. A field added to one of them is a breaking change for struct literals that spell out every field, so construct them with `..Default::default()`.

### Migrations since phase 1

| Change | When | Migration |
|---|---|---|
| `Sky::Map { backdrop }` became `Sky::Map { background: Background }` | TR1b | `backdrop: c` becomes `background: Background::Color(c)`; `Background::Sky` shows the map |
| `Sky::default()` is `Sky::Atmosphere(AtmosphereMedium::default())`, not `Uniform([0; 3])` | AT1 | set `Sky::Uniform(..)` explicitly for the old look |
| `Sun::default().illuminance` is 120,000 lux (outside the atmosphere) | AT1 | under `Uniform`/`Map` skies, set the illuminance that reaches the ground |
| `Settings::default()`: GI and TR1 on, aerial perspective, height fog and volumetric fog on | G2b, TR1b, AT1, VF1 | set the fields off explicitly to reproduce older frames |
| `Settings` gained `render_scale`, `aerial_perspective`, `height_fog`, `volumetric_fog` | TR1b, AT1 | none for `..Default::default()` constructions |
| Soft sun plus local lights is rejected | phase 1 | hard sun (`angular_radius: 0.0`) until LC1 |

## Example

```text
cargo run --example game_scene -- --path nonrt --out D:/Coding/renderlab-work/api1/nonrt
RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
    cargo run --example game_scene -- --path rt --out D:/Coding/renderlab-work/api1/rt
```

Options: `--path rt|nonrt|auto`, `--frames N` (48), `--every K` (8; PNG every K-th frame and the last), `--size WxH` (1280x720), `--exposure STOPS`, `--no-gi`, `--no-temporal`, `--render-scale native|quality|performance`, `--sky atmosphere|uniform`, `--volumetric-fog`, and `--quarry MANIFEST` to load the registered quarry scans and RLE2 sky in place of the procedural ground, shown as a visible sky. `run.json` beside the frames records the path, settings, per-frame CPU time and GPU render spans. These are development-build timings on a shared GPU, not performance claims.
