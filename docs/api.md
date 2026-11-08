# Renderlab engine API

The engine-facing API is `renderlab::engine`. A game owns one `Engine`. It creates resources and objects through typed handles, sets lights, sky and settings, and calls `render` once per frame. Everything else in the crate root is the low-level renderer the engine wraps. That renderer's tuning surface changes as features are measured; the engine module is the part that stays stable (see [Stability](#stability-and-freeze-notes)).

Status: **frozen** (unit API1b, 2026-11-09, main `4c8ef7d`). The engine API below is frozen apart from the additions listed under [Stability and freeze notes](#stability-and-freeze-notes). Every overnight feature unit merged by then is wired: DC1, TR1b, R1, R1n, G2b, G2c, AT1, AT2, VF1, PP1, TP1, TP2, LC1, LC2, SK1, MP1, MQ1, PC1 and GC1.

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
| `pipeline_cache` | `None` | directory for the persistent shared pipeline cache (kernels and programs; loaded before initialization) |

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
| `create_skinned_mesh(&Primitive, &[Influence], joints)` | `MeshId` | one independently posed skinned primitive (see [Skinned characters](#skinned-characters)) |
| `add_skinned_asset(&SkinnedAsset)` | `SkinnedHandles` | a cooked skinned `.rls`: textures, double-sided materials, one posable mesh per skinned primitive of each binding |
| `create_emitter(&EmitterDescriptor, sprite)` | `EmitterId` | a GPU particle emitter (see [Glass, foliage and particles](#glass-foliage-and-particles)) |
| `remove_*(id)` | | meshes and materials only once no object uses them; materials keep their own textures and program alive |

`MaterialDesc::default()` is an untextured, double-sided, white dielectric (metallic 0, roughness 0.5) on the standard program. Keep `double_sided: true` for now: both paths' shadowed primaries currently need double-sided materials. `thin: true` lets shadow rays leave either side, for leaves and cards. `transparency: Some(Transparency { opacity, ior, thickness })` makes the material glass (below).

## Objects

`spawn(mesh, material, transform) -> ObjectId`, `set_transform(object, Mat4)`, `transform(object)`, `despawn(object)`.

Objects that share a mesh and a material are drawn together as one instanced draw. Moving an object is a transform write on the next frame. Spawning or despawning re-uploads that group's transforms on the next frame. One group holds at most `limits.max_instances` objects (raise it in the descriptor for large crowds or forests).

## Skinned characters

SK1's skinned meshes are engine meshes whose pose you set each frame:

```rust
let asset = renderlab::asset::skin::decode(&std::fs::read(rls_path)?, Default::default())?;
let fox = engine.add_skinned_asset(&asset)?;           // textures, materials, posable meshes
let objects: Vec<ObjectId> = fox.bindings[0].iter()
    .map(|&(mesh, material)| engine.spawn(mesh, material, model))
    .collect::<Result<_, _>>()?;
let mut player = ClipPlayer::new(1);                   // the asset's walk clip
// every frame:
player.advance(dt)?;
let pose = player.sample(&asset)?;
for (binding, parts) in fox.bindings.iter().enumerate() {
    let palette = pose.joint_palette(&asset, binding)?;
    for &(mesh, _) in parts { engine.set_joint_palette(mesh, &palette)?; }
}
for &object in &objects { engine.set_transform(object, model)?; } // move the character
```

- Cook the glTF source with the cooker's skin recipe (`renderlab_cook::skin::cook_file`, an RLS file). `create_skinned_mesh(&Primitive, &[Influence], joint_count)` uploads one primitive yourself.
- Objects spawned from one skinned mesh share its pose; create another skinned mesh for another pose. A skinned mesh holds its bind pose until the first `set_joint_palette`.
- Skinning works on both paths. RT refits the mesh's acceleration structure every posed frame (an explicit SK1 cost gap); non-RT's GI and shadows see the deforming caster.
- Materials may use any program: the example draws the Fox with its toon program and the Fox's own texture.

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

## Glass, foliage and particles

**Glass (TP1, TP2).** A material with `transparency: Some(Transparency { opacity, ior, thickness })` leaves the opaque paths and draws in a forward pass after reconstruction, back to front, refracting what is behind it through a thin slab of `thickness` metres. Its program shades it. A program that declares `fn transmittance(s: Surface, input: SurfaceInput) -> vec3<f32>` gets `shade + transmittance x behind`; the standard program does (Fresnel and the base colour as tint), and the example's glass pane uses it. Glass casts coloured sun shadows on both paths, and reflects the scene on RT (TP2). It casts no local-light shadows, and transparent surfaces are absent from GI rays.

**Cutout foliage (TP1).** Alpha-tested cards are cut once, at load, into ordinary opaque geometry, so every path, shadow method and GI ray sees the exact cutout at no per-frame cost:

```rust
let mask = AlphaMask::from_rgba8(width, height, &rgba8);    // alpha of a leaf texture
let card = cutout(&card_mesh_data, &mask, 1.0, 0.5, 64)?;    // opacity, cutoff, cells per uv unit
let bush = engine.create_mesh(&card.data())?;
let leaves = engine.create_material(&MaterialDesc { thin: true, ..leaf_material })?;
```

**Particles (TP1).** `create_emitter(&EmitterDescriptor, sprite: Option<TextureId>) -> EmitterId` starts a GPU emitter that simulates and draws every frame until `remove_emitter`. It is lit by the sun (with its shadows and glass tint) and the environment, sorted on the GPU, faded against opaque depth (soft particles) and fogged. Leave `EmitterDescriptor::texture` None and pass the sprite by id. `update_emitter` changes a live emitter (same capacity and sprite presence). `set_particle_step(seconds)` sets the simulation time per frame (default 1/60); set it to the game's frame time. Particles cast no shadows. Fire and other light sources should pair the emitter with a point light, as the example's brazier does.

## Lighting and exposure

- **Time of day (AT2).** `Sun::at(elevation, azimuth)` places a default sun by angles (radians; azimuth clockwise from north, with +Y up, -Z north and +X east). `Sun::at_hour(hour, latitude) -> Option<Sun>` follows an equinox sun path: it rises in the east at 6, peaks toward the equator at 12, sets in the west at 18, and is None below the horizon. Calling `set_sun` every frame is cheap. AT2 updates the procedural sky in place, and GI relights progressively without a flush, so a day cycle is just `engine.set_sun(Sun::at_hour(hour, latitude))` per frame.
- `set_sun(Option<Sun>)`. The sun is the scene's one shadowed directional light. `Sun::travel` is the direction its light travels. `illuminance` is lux per channel: under `Sky::Atmosphere` it is the sun outside the atmosphere (default 120,000), which the atmosphere dims and warms; under the other skies it arrives as given (clear midday is about 100,000). `angular_radius` (radians, 0-0.1) sets penumbra width. Its default, 0.04, is the measured shipping configuration on both paths; 0 gives hard shadows. The visible sun disk keeps the real sun's size (the medium's `sun_radius`), independent of the penumbra.
- `set_lights(&[Light])`: local point and spot lights, in candela with an optional range. They are culled per camera cluster, and the most important ones are shadowed each frame (LC1, [m7-lc1.md](m7-lc1.md); `Settings::local_shadows`, 4 by default); the rest are unshadowed. Every light also reaches GI (LC2). They combine with a soft sun. `Light::Point` and `Light::Spot` take `source_radius` in metres (LC2, [m7-lc2.md](m7-lc2.md)): zero keeps hard shadows, positive values give soft local shadows on both paths. Existing Rust light literals need `source_radius: 0.0`; JSON lighting files default it to zero. Directional lights go in the sun.
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
    pub local_shadows: LocalShadows,      // shadowed local lights per frame (count, map size, near)
    pub post: PostSettings,               // bloom, motion blur, DOF, tone mapper, grading
    pub reflections: ReflectionSettings,  // traced reflections: enabled, max_roughness
}
```

- `Quality::Realtime` is each path's measured real-time configuration (the table above).
- `Quality::Reference` integrates sun visibility exactly: all 256 disk directions per pixel on RT, and the exact triangle-tested map lookup without RT. It is for stills, captures and A/B checks, not real-time play.
- `render_scale` is TR1b's upscaling preset: with temporal reconstruction on, every pass renders at the scale's internal size and TR1 reconstructs the target. It needs `temporal_reconstruction`; a scale below `Native` without it is rejected.
- `local_shadows` (LC1, LC2): `LocalShadows { count, resolution, near }`, default 4 lights at 1024² map faces. `count: 0` turns local shadows off. Penumbrae come from each light's `source_radius`.
- `post` (PP1): `PostSettings` with `enabled`, `bloom`, `motion_blur`, `shutter_angle_degrees`, `depth_of_field: Option<DepthOfField>`, `tonemapper: Tonemapper`, white balance, `contrast`, `saturation`, `vignette`, `grain`, chromatic aberration and `lut_strength`. It is on by default; `enabled: false` presents the plain tone-mapped image. `Engine::set_colour_lut(Option<ColourLut>)` sets the grading LUT.
- `reflections` (R1n): `ReflectionSettings { enabled, max_roughness }`; `ReflectionSettings::off()` leaves every program its untraced estimate.
- `set_settings` applies a change only when something changed, and validates before touching the renderer. It overwrites any overlapping tuning made through `renderer_mut()`.

Defaults follow each feature unit's default ruling, not the facade's opinion. With G2b and AT1 merged:

- GI is **on** (G2b; `global_illumination: false` maps to `IndirectSettings::off()`).
- Temporal reconstruction is **on** at native scale (TR1b; its remaining losses to no-AA and TSR are in [m7-gaps.md](m7-gaps.md)).
- The physical atmosphere sky, aerial perspective, height fog and a light volumetric haze are **on** (AT1, VF1).
- Traced reflections are on wherever a program asks for them, on both paths (R1, R1n; see [Reflections](#reflections)).
- Post (bloom, motion blur, grading, vignette) is **on**; depth of field is opt-in (PP1). Four local lights are shadowed (LC1).

Gaps against Unreal and the reference for each are in [m7-gaps.md](m7-gaps.md).

Rendering is single-sample on both quality paths. Temporal reconstruction is the antialiasing; MSAA belongs to the low-level raster path only.

## Reflections

Reflections are a property of the shading program, not a setting. A program asks for them with the optional hook

```wgsl
fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe  // SpecularLobe { normal, alpha, weight }
```

and reads the result with the engine input `specular_incident(input) -> vec4<f32>`. Its rgb is the mean exposed radiance over the lobe, and w is the traced fraction, so the program blends it over its own untraced estimate: `mix(untraced, traced.rgb * reflectance, traced.w)`. `alpha` is perceptual roughness squared, and `weight > 0` requests rays.

| Path | What a requesting program gets (M7-R1 [m7-r1.md](m7-r1.md), M7-R1n [m7-r1n.md](m7-r1n.md)) |
|---|---|
| `HardwareRt` | One mirror ray below roughness 0.05, and four fixed GGX strata up to 0.35, fading to the untraced estimate by 0.5 (w falls from 1 to 0 over 0.35-0.5). Hits are shaded by the frame's program, with the hit's own material, decals and a hard shadow ray. TR1 integrates the glossy strata over frames. One bounce. |
| `NonRt` | The same rays, bands, hit shading and TR1 integration, traced in software over per-mesh BVHs of the drawn triangles (R1n). A frame that draws any mesh over 2^19 triangles, or any streamed cluster mesh, traces nothing and keeps the program's untraced estimate (`indirect_radiance`), so no ray shows sky through a mesh without software geometry. |

The standard program requests them for every material. The example's program shows per-material control: its `reflect` parameter is the lobe weight, so a material with `reflect: 0` asks for no rays. `Settings::reflections: ReflectionSettings { enabled, max_roughness }` bounds every program on both paths: on by default up to roughness 0.5 (the bands above), and `ReflectionSettings::off()` traces nothing.

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

**Programs per object.** Each material names its program, and a frame may mix up to 32 programs on both quality paths: traced RT primaries and the non-RT visibility buffer shade each program's pixels with that program's own pipeline (MP1, [m7-mp1.md](m7-mp1.md)); raster frames switch pipelines per draw. The example's props use examples/game_scene.wgsl and its hero examples/game_toon.wgsl. Two limits remain: a reflection hit on another program's object is shaded by the reflecting surface's program with the hit's material inputs, and `SoftShadows::Temporal` frames still use one program.

## Current limits

- At most 32 shading programs per frame; `SoftShadows::Temporal` frames use one (above).
- At most 64 mesh-material groups per frame (`TRACED_DRAWS`), on both paths. Objects sharing a mesh and material count once.
- Materials must be double-sided while the sun casts shadows.
- Only the most important local lights cast shadows (`Settings::local_shadows`, 4 by default, at most 16); the rest are unshadowed. See the Lights section of [m7-gaps.md](m7-gaps.md#lights).
- Glass casts coloured sun shadows only (no local-light shadows) and is invisible to GI rays; particles cast no shadows (TP1, TP2).
- Small, very bright emitters (a lamp bulb's `emission`) turn into GI fireflies, because each cache cell traces a few fixed directions. Light such sources with a point light and keep their glow out of `emission`, as the example's bulbs do.
- On `NonRt`, removing a mesh does not yet release its GI ray geometry, which the renderer keeps until it drops. Avoid heavy mesh churn on that path.
- On `NonRt`, reflections are traced only in frames whose meshes all have software geometry (none over 2^19 triangles, no streamed cluster meshes), and volumetric fog is unshadowed (no sun shafts).
- Virtual geometry (streamed cluster meshes) and the diagnostic view modes stay on the low-level `Renderer`.

## Module layout and planned extension points

```
renderlab::engine            Engine, EngineDescriptor, Capabilities, RenderPath, ExperimentalRayQuery,
                             MaterialDesc, TextureRef, AssetHandles, SkinnedHandles, Image,
                             headless_adapter
renderlab::engine (settings) Settings, Quality, Exposure, Sky, Background, Sun
renderlab::engine (ids)      Id<T>, MeshId, TextureId, ProgramId, MaterialId, EnvironmentId, ObjectId,
                             EmitterId
renderlab::engine re-exports Camera, Light, Mat4, Vec3, MeshData, Vertex, Value, Limits,
                             RenderScale, AtmosphereMedium, HeightFog, VolumetricFog,
                             LocalShadows, PostSettings, Tonemapper, DepthOfField, ColourLut,
                             ReflectionSettings, Transparency, EmitterDescriptor, MAX_PARTICLES,
                             AlphaMask, CutoutMesh, cutout, ClipPlayer, AnimationPose,
                             SkinnedAsset, Influence, Primitive,
                             Decal, DecalBlend, DecalTexture, DecalTextureKind, DecalTextures,
                             Sampler, TextureData, TangentStream, PreparedEnvironment,
                             Error, ReloadError, ReloadReport, FrameReport, Events, FrameId, Renderer
renderlab::*                 the low-level renderer (expert API, not frozen)
```

Each overnight unit landed in a named slot; every row is now wired.

| Unit | Feature | Planned engine API |
|---|---|---|
| TR1b | TR1 default, upscaling presets | **Wired**: `Settings::temporal_reconstruction` on by default; `Settings::render_scale` (`RenderScale::{Native, Quality, Performance}`); `Background::Sky` for the visible sky |
| G2b | GI default | **Wired**: `Settings::global_illumination` on by default, off maps to `IndirectSettings::off()`; no new calls |
| R1 | reflections | **Wired**: the `reflection_lobe` hook and the `specular_incident` input (see [Reflections](#reflections)). |
| TP1, TP2 | glass, foliage, particles | **Wired**: `MaterialDesc::{transparency, thin}`; `cutout`/`AlphaMask` for foliage; `create_emitter`/`update_emitter`/`remove_emitter`/`set_particle_step` with `EmitterId`. TP2's coloured glass shadows, RT glass reflections and coverage AA need no API. |
| AT1 | atmosphere, fog | **Wired**: `Sky::Atmosphere(AtmosphereMedium)` (the default sky); `Settings::{aerial_perspective, height_fog, volumetric_fog}`. See [Sky, atmosphere and fog](#sky-atmosphere-and-fog). |
| SK1 | skinned meshes | **Wired**: `create_skinned_mesh`, `add_skinned_asset -> SkinnedHandles`, `set_joint_palette(MeshId, &[Mat4])`; `ClipPlayer` and `AnimationPose` re-exported. The example's hero is the walking Fox. |
| LC1, LC2 | many lights, soft local shadows | **Wired**: `set_lights` unchanged; `Settings::local_shadows: LocalShadows`; `Light::{Point, Spot}::source_radius` (LC2). The example has three soft-shadowed lanterns. |
| AT2 | time of day | **Wired**: `Sun::at(elevation, azimuth)`, `Sun::at_hour(hour, latitude)`; per-frame `set_sun` is cheap. |
| DC1 | decals | **Wired** (phase 2): `set_decals(&[Decal])`, with `Decal`, `DecalBlend`, `DecalTextures`, `DecalTexture` and `DecalTextureKind` re-exported. See [Decals](#decals). |
| PP1 | post-processing | **Wired**: `Settings::post: PostSettings`, `Engine::set_colour_lut`; `PostSettings`, `Tonemapper`, `DepthOfField` and `ColourLut` re-exported. No auto exposure yet (`Exposure` may grow an `Auto` variant). |
| PC1, GC1, MQ1, VF1 | startup cache, GI cost, moving-object history, volumetric fog default | **Wired**, no API change beyond `EngineDescriptor::pipeline_cache` (PC1) and the `volumetric_fog` default (VF1) |
| R1n | non-RT traced reflections, reflection switch | **Wired**: `Settings::reflections: ReflectionSettings` (`enabled`, `max_roughness`), re-exported, applied with the other settings; the program hooks are unchanged |
| MP1 | per-program dispatch | **Wired**, no API change: materials of different programs share a frame (see [Shading programs](#shading-programs)). The example's hero has its own toon program, examples/game_toon.wgsl. |
| G2c | GI speckles and streaks | **Wired**, no API change |

Units still arriving (MV1 per-object motion vectors, LC3, RQ1, TP3) are internal: their planned changes need no new engine API. Anything they do need is an addition under the rules below.

New handles follow the `Id<T>` pattern, new settings are new fields with defaults, and new sky and exposure kinds are new enum variants. None of these change an existing call.

## Stability and freeze notes

**The engine API is frozen as of this revision.** Changes from here are additions only, as listed under "May grow", or a recorded migration agreed with Islam. The additions made since the phase-1 freeze notes are recorded in the migration table below.

**Stable** (changes only with a recorded migration):

- `Engine` method names and signatures above; `EngineDescriptor`, `Capabilities`, `RenderPath`, `ExperimentalRayQuery`.
- Handle semantics: `Copy` typed `Id<T>`, stale handles rejected, the batching of objects by mesh and material.
- `MaterialDesc`, `TextureRef`, `Sun`, `Sky::Atmosphere`/`Uniform`/`Map`, `Background::Color`/`Sky`, `AtmosphereMedium`, `HeightFog`, `VolumetricFog`, `Exposure::Manual`, `Quality::Realtime`/`Reference`, `RenderScale`, and the meaning of each `Settings` field.
- `set_decals(&[Decal])` and the re-exported decal types.
- Skinned meshes (`create_skinned_mesh`, `add_skinned_asset`, `set_joint_palette`), emitters (`create_emitter`, `update_emitter`, `remove_emitter`, `set_particle_step`), `set_colour_lut`, `cutout`/`AlphaMask`, `Sun::at`/`at_hour`.
- The program contract: `surface`, `shade`, optional `Parameters`, `DEFAULT_PARAMETERS`, `scatter`, `emission`, `reflection_lobe`, and the documented engine inputs.

**May grow** (additive only): new `Settings` fields with defaults, new variants of the `#[non_exhaustive]` enums (`Sky`, `Background`, `Exposure`, `Quality`), new engine calls and handle kinds, new engine inputs for programs.

**May change**: `FrameReport`, `Events` and the other diagnostic counters; defaults that a unit's ruling flips; every low-level crate-root type and `renderer_mut()` tuning; the current limits listed above, which should only loosen.

**Re-exported renderer types** (`AtmosphereMedium`, `HeightFog`, `VolumetricFog`, the decal types, `RenderScale`, `Light`, `LocalShadows`, `PostSettings`, `ReflectionSettings`, `Transparency`, `EmitterDescriptor`) are plain parameter structs and enums owned by their units. They are part of the stable surface. A field added to one of them is a breaking change for struct literals that spell out every field, so construct them with `..Default::default()`.

### Migrations since phase 1

| Change | When | Migration |
|---|---|---|
| `Sky::Map { backdrop }` became `Sky::Map { background: Background }` | TR1b | `backdrop: c` becomes `background: Background::Color(c)`; `Background::Sky` shows the map |
| `Sky::default()` is `Sky::Atmosphere(AtmosphereMedium::default())`, not `Uniform([0; 3])` | AT1 | set `Sky::Uniform(..)` explicitly for the old look |
| `Sun::default().illuminance` is 120,000 lux (outside the atmosphere) | AT1 | under `Uniform`/`Map` skies, set the illuminance that reaches the ground |
| `Settings::default()`: GI and TR1 on, aerial perspective, height fog and volumetric fog on | G2b, TR1b, AT1, VF1 | set the fields off explicitly to reproduce older frames |
| `Settings` gained `render_scale`, `aerial_perspective`, `height_fog`, `volumetric_fog` | TR1b, AT1 | none for `..Default::default()` constructions |
| Soft sun plus local lights renders (was rejected) | LC1 | none |
| `Light::Point`/`Spot` gained `source_radius` | LC2 | add `source_radius: 0.0` to light literals for hard shadows |
| `MaterialDesc` gained `thin` and `transparency` | TP1 | none for `..Default::default()` constructions |
| `Settings` gained `local_shadows`, `post`, `reflections`; post and volumetric fog are on by default | LC1, PP1, R1n, VF1 | `post: PostSettings { enabled: false, .. }` and `volumetric_fog: None` reproduce older frames |
| New calls: skinned meshes, emitters, `set_colour_lut`, `Sun::at`/`at_hour`; new `EmitterId`, `SkinnedHandles` | SK1, TP1, PP1, AT2 | additions only |

## Example

```text
cargo run --example game_scene -- --path nonrt --out D:/Coding/renderlab-work/api1/nonrt
RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK \
    cargo run --example game_scene -- --path rt --out D:/Coding/renderlab-work/api1/rt
```

The scene is a dusk courtyard with every feature on by default:
- props and materials: glossy, mirror and glass objects, cutout bushes, decals;
- light: three soft-shadowed lanterns with glowing bulbs, and a brazier with fire and smoke particles;
- the walking skinned Fox, drawn with its own toon program (`--fox Fox.glb`, from `scripts/m7-sk1-asset.py`, or `RENDERLAB_FOX_GLB`; without it the hero is two toon spheres);
- the atmosphere at sunset, volumetric haze, GI, reflections, TR1, and post with bloom.

Options:
- `--path rt|nonrt|auto`; `--frames N` (48); `--every K` (8: a PNG every K-th frame and the last); `--size WxH` (1280x720).
- `--hour H` (17.5) or `--time dusk|noon`; `--sun-speed DEG_PER_S` with `--sun-after FRAME`; `--still`; `--timing-frames N`.
- `--exposure STOPS`; `--no-gi`, `--no-temporal`, `--no-post`, `--no-glass`, `--no-particles`, `--no-volumetric-fog`; `--render-scale native|quality|performance`; `--sky atmosphere|uniform`.
- `--quarry MANIFEST` loads the registered quarry scans and RLE2 sky in place of the procedural ground, shown as a visible sky. `run.json` beside the frames records the path, settings, per-frame CPU time and GPU render spans. These are development-build timings on a shared GPU, not performance claims.


PC1 startup controls: `game_scene` persists its Vulkan pipeline cache in the ignored
`measurements/pipeline-cache/` directory. Set `--pipeline-cache DIR` (or
`RENDERLAB_PIPELINE_CACHE_DIR`) to choose a directory; `--no-pipeline-cache` disables
persistence. Applications opt in with `EngineDescriptor::pipeline_cache`; its default
remains `None`. Cache I/O failures fall back to compilation. Driver and adapter changes
use a different file, and wgpu keys compiled pipelines by shader and pipeline state.
Shader edits still validate; rejected edits retain the last working pipelines.

`RENDERLAB_STARTUP_PROFILE=1` records CPU module/pipeline compilation in the example's
`run.json`, alongside scene-ready and first-image readback times and complete RGBA8
hashes. `--reload-probe` applies a visible program edit, attempts invalid WGSL, then
restores the source, writing separate diagnostic images. For an isolated pipeline
reload check, use it with `--no-gi --no-temporal`. Optional
`RENDERLAB_STARTUP_PROFILE_DRIVER=1` uses the existing instrumented Vulkan device to
separate native driver compilation from the pipeline API's SPIR-V and validation work;
use those rows as diagnostics, not as timings of the ordinary device.
