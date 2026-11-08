# Renderlab engine API

The engine-facing API is `renderlab::engine`. A game owns one `Engine`. It creates resources and objects through typed handles, sets lights, sky and settings, and calls `render` once per frame. Everything else in the crate root is the low-level renderer the engine wraps. That renderer's tuning surface changes as features are measured; the engine module is the part that stays stable (see [Stability](#stability-and-freeze-notes)).

Status: unit API1 (2026-11-08). Phase 1 covered main `984a18b`; phase 2 has wired DC1 (main `976a952`) and wires the other overnight feature units as they merge (see [Planned extension points](#module-layout-and-planned-extension-points)).

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
    settings: Settings {
        global_illumination: true,
        temporal_reconstruction: true,
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

engine.set_sun(Some(Sun { travel: Vec3::new(-0.45, -0.75, -0.5), ..Default::default() }))?;
engine.set_sky(Sky::Uniform([3500.0, 5000.0, 7500.0]))?;

loop {
    engine.set_transform(crate_, model_for_this_frame)?;
    let camera = Camera::look_at(eye, target, Vec3::Y, 55f32.to_radians(), 0.05, 500.0)?;
    engine.render(&camera)?;
    let events = engine.poll()?; // completed frames and GPU pass timings
}
```

`examples/game_scene.rs` is the complete version: procedural props, a toon-styled hero, sun, sky, GI, temporal reconstruction and a camera fly path, offscreen, with PNG frames.

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

## Lighting, sky and exposure

- `set_sun(Option<Sun>)`. The sun is the scene's one shadowed directional light. `Sun::travel` is the direction its light travels; `illuminance` is lux per channel (clear midday is about 100,000); `angular_radius` (radians, 0-0.1) sets penumbra width. Its default, 0.04, is the measured shipping configuration on both paths. 0 gives hard shadows.
- `set_lights(&[Light])`: local point and spot lights, in candela with an optional range. They are unshadowed at this revision, and a soft sun must currently be the only light (see [Current limits](#current-limits)); many-light sampling and shadows arrive with LC1. Directional lights go in the sun.
- `set_sky(Sky)`: `Sky::Uniform(radiance)` in cd/m², which is also the background colour; or `Sky::Map { map, rotation_y, intensity, backdrop }`, where the map lights the scene and `backdrop` fills the background. The visible-sky background and the physical atmosphere arrive with TR1b and AT1.
- `Settings::exposure`: `Exposure::Manual { stops }` scales scene luminance by 2^stops before tone mapping. With sunlight in lux, -14 to -16 is typical.

## Settings and quality presets

```rust
pub struct Settings {
    pub quality: Quality,                 // Realtime (default) | Reference
    pub global_illumination: bool,        // world radiance cache: sky visibility and bounce
    pub temporal_reconstruction: bool,    // TR1 jittered accumulation and antialiasing
    pub exposure: Exposure,
}
```

- `Quality::Realtime` is each path's measured real-time configuration (the table above).
- `Quality::Reference` integrates sun visibility exactly: all 256 disk directions per pixel on RT, and the exact triangle-tested map lookup without RT. It is for stills, captures and A/B checks, not real-time play.
- `set_settings` applies a change atomically and only when something changed. It overwrites any overlapping tuning made through `renderer_mut()`.

Defaults follow each feature unit's default ruling, not the facade's opinion. At `984a18b`, GI is off by default (the M7-G2 ruling keeps the cache opt-in until G2b passes its gates) and temporal reconstruction is off (TR1b's visible-sky gate failed). The example turns both on. Phase 2 updates these defaults as G2b and TR1b rule.

Rendering is single-sample on both quality paths. Temporal reconstruction is the antialiasing; MSAA belongs to the low-level raster path only.

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

Engine inputs a program may call include `sample_base_color`, `sample_metallic_roughness`, `material_normal`, `light_count`, `light_sample`, `light_visibility` (world-space, never screen-space), `indirect_irradiance` and `indirect_radiance` (GI-occluded sky and bounce), `environment_lobe*`, `brdf`, `expose` and `exposed_product`. shaders/programs/standard.wgsl and toon.wgsl are complete references. A program without `scatter`/`emission` still blocks light but reflects no bounce light.

**One program per frame (current limit).** Both quality paths shade every pixel of a frame with one program: traced RT primaries and the non-RT visibility buffer reject frames whose materials use different programs. Per-program dispatch is planned (M7 design, R1). Until it lands, give per-object styles to materials of one program through a `Parameters` member, as the example does: examples/game_scene.wgsl is the standard program plus a toon branch selected by `style`, so the hero is toon-shaded and the props photoreal in the same frame. When per-program dispatch lands, separate programs per material will work with no API change.

## Current limits

- One shading program per frame on both quality paths (above).
- At most 64 mesh-material groups per frame (`TRACED_DRAWS`), on both paths. Objects sharing a mesh and material count once.
- Materials must be double-sided while the sun casts shadows.
- Local lights are unshadowed, and a soft sun (`angular_radius > 0`) must be the only light: the renderer accepts soft sun visibility only for a light list of exactly one directional light. `set_lights` with a soft sun returns `InvalidInput`. Use `angular_radius: 0.0` (hard sun) to combine the sun with local lights. Many-light shading (LC1) is expected to lift this.
- On `NonRt`, removing a mesh does not yet release its GI ray geometry, which the renderer keeps until it drops. Avoid heavy mesh churn on that path.
- Virtual geometry (streamed cluster meshes) and the diagnostic view modes stay on the low-level `Renderer`.

## Module layout and planned extension points

```
renderlab::engine            Engine, EngineDescriptor, Capabilities, RenderPath, ExperimentalRayQuery,
                             MaterialDesc, TextureRef, AssetHandles, Image, headless_adapter
renderlab::engine (settings) Settings, Quality, Exposure, Sky, Sun
renderlab::engine (ids)      Id<T>, MeshId, TextureId, ProgramId, MaterialId, EnvironmentId, ObjectId
renderlab::engine re-exports Camera, Light, Decal, DecalBlend, DecalTexture, DecalTextureKind,
                             DecalTextures, MeshData, Vertex, Value, Limits, Mat4, Vec3, Error,
                             ReloadError, ReloadReport, FrameReport, Events, FrameId, Sampler,
                             TextureData, TangentStream, PreparedEnvironment, Renderer
renderlab::*                 the low-level renderer (expert API, not frozen)
```

Each overnight unit lands in a named slot. The shapes below are the intended API; phase 2 confirms or adjusts them against what each unit merges.

| Unit | Feature | Planned engine API |
|---|---|---|
| TR1b | TR1 default, upscaling presets | `Settings::temporal_reconstruction` default; a `Settings::upscaling` render-scale preset (native, quality, balanced, performance); visible sky on misses |
| G2b | GI default | `Settings::global_illumination` default; no new calls |
| R1 | reflections | a `Settings` field if reflections are switchable; programs read them through `indirect_radiance`/`reflection_lobe()` |
| TP1 | transparency, particles | `MaterialDesc` blend/transmission fields for glass; `create_particles(ParticleDesc) -> ParticlesId`, `set_particles`, `remove_particles` |
| AT1 | atmosphere, fog | new `Sky::Atmosphere { .. }` variant (`Sky` is `#[non_exhaustive]`); `Settings::fog` |
| SK1 | skinned meshes | `create_skinned_mesh(..) -> MeshId` with joints and weights; `set_pose(object, &[Mat4])` |
| LC1 | many lights | `set_lights` keeps its shape; per-light shadows and the light budget arrive as `Light` fields or a `LocalLight` wrapper |
| DC1 | decals | **Wired** (phase 2): `set_decals(&[Decal])`, with `Decal`, `DecalBlend`, `DecalTextures`, `DecalTexture` and `DecalTextureKind` re-exported. See [Decals](#decals). |
| PP1 | post-processing | `Settings::post` (bloom, tone mapper, colour grading); `Exposure::Auto { .. }` variant |

New handles follow the `Id<T>` pattern, new settings are new fields with defaults, and new sky and exposure kinds are new enum variants. None of these change an existing call.

## Stability and freeze notes

Draft, to be finalised in phase 2 once the feature units have merged.

**Stable** (changes only with a recorded migration):

- `Engine` method names and signatures above; `EngineDescriptor`, `Capabilities`, `RenderPath`, `ExperimentalRayQuery`.
- Handle semantics: `Copy` typed `Id<T>`, stale handles rejected, the batching of objects by mesh and material.
- `MaterialDesc`, `TextureRef`, `Sun`, `Sky::Uniform`/`Sky::Map`, `Exposure::Manual`, `Quality::Realtime`/`Reference`, and the meaning of each `Settings` field.
- The program contract: `surface`, `shade`, optional `Parameters`, `DEFAULT_PARAMETERS`, `scatter`, `emission`, and the documented engine inputs.

**May grow** (additive only): new `Settings` fields with defaults, new variants of the `#[non_exhaustive]` enums (`Sky`, `Exposure`, `Quality`), new engine calls and handle kinds, new engine inputs for programs.

**May change**: `FrameReport`, `Events` and the other diagnostic counters; defaults that a unit's ruling flips; every low-level crate-root type and `renderer_mut()` tuning; the current limits listed above, which should only loosen.

## Example

```text
cargo run --example game_scene -- --path nonrt --out D:/Coding/renderlab-work/api1/nonrt
RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
    cargo run --example game_scene -- --path rt --out D:/Coding/renderlab-work/api1/rt
```

Options: `--path rt|nonrt|auto`, `--frames N` (48), `--every K` (8; PNG every K-th frame and the last), `--size WxH` (1280x720), `--exposure STOPS`, `--no-gi`, `--no-temporal`, and `--quarry MANIFEST` to load the registered quarry scans and RLE2 sky in place of the procedural ground. `run.json` beside the frames records the path, settings, per-frame CPU time and GPU render spans. These are development-build timings on a shared GPU, not performance claims.
