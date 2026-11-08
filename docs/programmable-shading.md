# Programmable shading

**Status:** Phase 1 is implemented and recorded in [programmable-shading-phase-1.md](programmable-shading-phase-1.md), with its gate result and open constraints. Reload latency and C17 are followed up in [fast-shader-reload.md](fast-shader-reload.md). Sections 1 and 4 describe the code before it.

The [API follow-through](programmable-shading-api-gaps.md) adds caller-owned program
file watches, WGSL parameter defaults, fixed arrays and shared normal mapping.

The subsequent [receiver-policy adoption](m6-receiver-adoption.md) adds
`MaterialDescriptor::thin` (false by default), exposed as `input.material.thin`,
and requires callers to enable `renderlab::RECEIVER_FEATURES` for raster triangle
vertices. Visibility remains independent of the program's lighting model.

Design decision, 2026-11-05, based on `48eabf5`. This is CPU-only design: no renderer change, no GPU or window run and no repository build. The one piece of new evidence is a naga 30.0.1 composition probe outside the repository ([evidence](#evidence-and-time)). It uses the same naga that wgpu 30 validates with. Labels: **M** is measured, either in the cited records or by the probe; **I** is an inference.

Islam's requirement: users write their own shaders to give games completely different looks, as in other engines. Examples are photoreal PBR, toon or cel shading, painterly rendering, stylised outlines, flat-shaded low-poly and custom lighting models. Programmability must not reduce quality, and the realistic path must stay very fast. The realistic-path quality bar (AGENTS.md at `48eabf5`) is the final image, static and in motion, judged against a high-sample, film-quality reference such as a path tracer, Cycles or Unreal's path tracer. Unreal's high-fidelity game rendering is the minimum. Temporal accumulation, caching and denoising are allowed. Core lighting, shadows and GI must not be screen-space.

## Decision

1. **Authoring model: shading programs.** A program is a WGSL fragment that defines `fn surface(input: SurfaceInput) -> Surface` and `fn shade(s: Surface, input: SurfaceInput) -> vec3<f32>`, written against an engine API. `Surface` is the program's own struct; the engine never stores it. A program may also declare `struct Parameters`, a typed per-material block. Each material names its program. Today's photoreal lit shader becomes the built-in **standard** program: the same body, split at the surface/lighting boundary.
2. **Frame-level post programs** run between shading and presentation. Only they may read screen buffers, and only those they declare in `ScreenInputs`. Outlines live here. Node graphs come later, and must compile to this same WGSL interface.
3. **Core inputs are world-space or ray-based.** The engine offers programs lights, per-light visibility at the program's own receiver, prepared environment fields and, later, world-space indirect light. Surface and shade code cannot read neighbouring pixels, screen buffers or derivatives. Every program receives the same visibility and lighting inputs, so a toon frame has exactly the photoreal frame's ray-traced shadows.
4. **Performance: per-program pipelines with tile worklists, never an uber-shader.** The traced hit record stays a visibility buffer (placement, primitive, barycentrics) with no surface or lighting fields.
   - When one program is visible, the frame keeps today's single full-image shading dispatch with that program's pipeline. That is the zero-overhead case.
   - When several programs are visible, the trace pass ORs a per-tile program mask, a small pass compacts per-program tile lists, and each visible program gets one indirect dispatch.
   - Raster draws switch pipeline per program, in submission order.
5. **Edit latency.** A program edit recompiles only that program's active shading pipelines. Trace and visibility kernels and the other programs stay as they are. Parameter edits are buffer writes.
6. **First unit:** the standard program through the API, byte-identical and with no frame-time regression; then a toon program, mixed-program frames and an outline post program, with reload preserved ([section 5](#5-first-unit)).

## 1. What exists today

- One scene source (`shaders/scene.wgsl`) shades every draw. `fs_lit` fuses texture sampling, normal mapping, the GGX/Lambert BRDF, the environment, the light loop and exposure in one entry point (`scene.wgsl:301-377`). The only way to change shading is `Renderer::reload(wgsl)` with a whole new source (`renderer.rs:1028-1113`).
- Hardware shadows, fresh shadows and the traced path build their modules by exact-string surgery on that source. `compute_lit` requires each of three `textureSample(...)` spellings to occur exactly once, rejects any other `textureSample` or `dpdx`, and rewrites them into descriptor-indexed `textureSampleGrad` (`primary_ray_gpu.rs:27-94`). An ordinary user edit to sampling breaks traced mode.
- **Measured costs that bound the design (M)**, from [primary-ray production](m6-primary-ray-production.md#hillclimb-one-mechanism-at-a-time) and the [Interactive climb](m6-interactive-climb.md):
  - Traced shading is gather-bound. Replacing the lit body changes nothing (ratio 1.04). Removing textures gives 0.898 and removing the environment 0.861.
  - Per-draw compaction with serialized indirect dispatches cost about 10 µs fixed per draw in shading: 0.539-0.541 ms, against 0.448 ms for one descriptor-indexed dispatch.
  - The tile-flag workgroup atomic and barrier cost about 0.006 ms.
  - The Interactive frame spans 0.8606-0.8615 ms, of which shading is 0.395-0.396 ms. 11,065 of 64,800 trace tiles are covered at 1080p.
  - Disabling naga runtime checks was slower (0.449 against 0.430 ms) and unsafe for user-edited WGSL.
- Raster reload compiles in a 4.093 ms median, and shader write to first present is 47.108 ms (M, [M4 verification](m4-final-verification.md), five reloads with 50 ms polling). Traced-mode reload time has never been measured.

## 2. Authoring model

### 2.1 Shading programs

A program is ordinary WGSL with no bindings. The engine composes it as engine prelude + stage API + `params` binding (if declared) + program + generated entry points. The probe validates exactly this composition in raster-fragment and traced-compute form, using the real `scene.wgsl` prelude (M). The toon proof program:

```wgsl
struct Parameters { shadow_tint: vec3<f32>, bands: f32, rim_color: vec3<f32>, rim_power: f32, fill: f32, flat_shaded: u32 };
struct Surface { albedo: vec3<f32>, n: vec3<f32> };

fn surface(input: SurfaceInput) -> Surface {
    let albedo=clamp(input.material.base_color.rgb*input.color.rgb*sample_base_color(input).rgb,vec3<f32>(0.0),vec3<f32>(1.0));
    var n=input.geometric;
    if params.flat_shaded==0u && dot(input.normal,input.normal)>1e-12 {
        n=normalize(input.normal);
        if !input.source_front { n=-n; }
    }
    return Surface(albedo,n);
}

fn shade(s: Surface, input: SurfaceInput) -> vec3<f32> {
    let v=normalize(input.eye-input.world);
    var lux=ambient(vec3<f32>(0.0,1.0,0.0))*params.fill;
    var lit=vec3<f32>(0.0);
    var tinted=vec3<f32>(0.0);
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        // The same ray-traced visibility as the standard program, then banded.
        let incident=max(dot(s.n,light.direction),0.0)*light_visibility(input,light);
        let band=floor(incident*params.bands)/params.bands;
        lit+=light.intensity*light.attenuation*band;
        tinted+=light.intensity*light.attenuation*(1.0-band)*params.shadow_tint;
    }
    let rim=pow(1.0-clamp(dot(s.n,v),0.0,1.0),params.rim_power)*params.rim_color;
    return expose(s.albedo*(lux+lit+tinted)/PI)+rim;
}
```

The standard program is the `fs_lit` body split into `surface` (samples, normal mapping, base, metal, rough) and `shade` (view vector, DFG, environment, light loop), with every expression in its original order. It is 53 lines in the probe (`wgsl/standard.wgsl`).

**Engine API available to programs** (the probe's `api_*.wgsl` files):

| Group | Functions and types |
|---|---|
| Receiver | `SurfaceInput`: `world`, `geometric` (unit source-front plane normal), `normal`, `tangent`, `handedness` (interpolated frame), `uv`, `uv_dx`, `uv_dy` (one pixel's uv footprint), `color`, `source_front`, `eye`, `pixel`, `material` (imported glTF factors), plus engine-private `receiver` |
| Lights | `light_count()`, `light_sample(i, input) -> LightSample{index, direction, attenuation, intensity}` (the fixed loop's exact arithmetic), `light_visibility(input, sample) -> f32` |
| Environment | `environment_enabled()`, `environment_constant()`, `environment_irradiance(n)`, `environment_reflection(direction, roughness)`, `environment_dfg(n·v, roughness)`; `ambient(n)`, unexposed lux, for simple programs |
| Exposure | `expose(radiance)`, `exposed_product(a, b)`, `environment_product(energy, response)` (the overflow-safe mantissa forms) |
| Library | `brdf(n, v, l, base, metal, rough)`, `unit_normal`, `PI`, `TINT` |
| Textures | `sample_base_color(input)`, `sample_metallic_roughness(input)`, `sample_normal_map(input)` at the mesh uv; `sample_*_grad(uv, dx, dy)` for custom mappings such as triplanar or scrolling uv |
| Parameters | `params`, when the program declares `struct Parameters` |
| Planned, not first unit | `scene_visible(origin, direction, t_max)` (an inline ray for custom ray effects), `indirect_diffuse` and `indirect_specular` (world-space GI, [section 2.4](#24-realistic-path-quality-bar-and-world-space-inputs)), `time`, generic texture slots |

**Rust API sketch:**

```rust
pub struct ProgramDescriptor<'a> { pub label: &'a str, pub source: &'a str }
/// Owned handle. Materials retain their program; the renderer owns its pipelines.
pub struct Program { /* identity, reflected Parameters layout */ }
pub enum Value { F32(f32), Vec2([f32; 2]), Vec3([f32; 3]), Vec4([f32; 4]), U32(u32), I32(i32) }
pub struct ScreenInputs { pub depth: bool, pub normal: bool, pub ids: bool }
pub struct PostDescriptor<'a> { pub label: &'a str, pub source: &'a str, pub inputs: ScreenInputs, pub parameters: &'a [(&'a str, Value)] }

pub struct MaterialDescriptor<'a> {
    // existing fields: imported factors, three texture bindings, double_sided
    pub program: Option<&'a Program>,          // None: the standard program
    pub parameters: &'a [(&'a str, Value)],    // checked by name and type against Parameters
}

impl Renderer {
    pub fn standard_program(&self) -> &Program;
    /// Composes and compiles the active variants now; Error::Shader names program lines.
    pub fn try_create_program(&mut self, d: &ProgramDescriptor<'_>) -> Result<Program, Error>;
    /// One transaction over that program's variants. On rejection its last working
    /// pipelines keep rendering; other programs and their materials are untouched.
    pub fn reload_program(&mut self, p: &Program, source: &str) -> Result<ReloadReport, ReloadError>;
    /// Buffer writes only, no compile.
    pub fn try_update_parameters(&mut self, m: &Material, values: &[(&str, Value)]) -> Result<(), Error>;
    pub fn try_set_post(&mut self, post: Option<&PostDescriptor<'_>>) -> Result<(), Error>;
}
```

`Renderer::reload(wgsl)` keeps its meaning for the engine prelude, `scene.wgsl` without `fs_lit`, and stays one transaction over every program. The standard program becomes an engine-owned, hot-reloadable file, `shaders/programs/standard.wgsl`.

The engine finds `Parameters` by reflection: the probe reads member names and offsets from naga IR. Toon's block is 48 B: `shadow_tint@0`, `bands@12`, `rim_color@16`, `rim_power@28`, `fill@32`, `flat_shaded@36` (M). Materials pass named values. Unknown names or wrong types are rejected; omitted fields are zero. On reload, existing materials are repacked by field name. New or retyped fields become zero, and the report lists them, so a live edit never fails on its parameter layout. The first unit caps the block at 256 B. Raster reads it as a uniform, and traced shading reads it through the per-draw storage array.

### 2.2 Post programs and screen inputs (opt-in)

```wgsl
struct Parameters { ink: vec3<f32>, depth_ratio: f32, crease_cos: f32, program: u32, width: i32, pad: u32 };
fn post(pixel: vec2<u32>, hdr: vec3<f32>) -> vec3<f32> {
    let here=screen_sample(vec2<i32>(pixel));   // {covered, depth, normal, instance, program}
    // ... compare four neighbours; ink silhouettes, instance edges, depth steps, creases
}
```

- `screen_sample` and `screen_size` exist only in modules for posts that declare `ScreenInputs`. Without them, the probe's outline is rejected at its own line 14 with "no definition in scope for identifier: `screen_sample`" (M). The opt-in is structural, not a convention.
- Screen inputs hold engine data only: view distance, the geometric source-front normal, instance+1 and program id. The shading stage's engine wrapper writes them, never program output, and only while a post program needs them. That is 20 B per pixel, about 41 MB at 1080p, and they are released when the post is cleared.
- On traced frames the shading dispatch writes them. On raster frames the generated fragment entry writes them as extra colour targets in a separate pipeline variant. The first unit supports 1x on both paths; 4x raster rejects screen-input posts.
- Tone mapping (`fs_present`, currently `x/(1+x)`) stays in the engine prelude and runs after the post.

### 2.3 What the engine guarantees to user shaders

| Topic | Guarantee |
|---|---|
| Colour and units | Linear Rec.709 RGB, metres, and lux / candela / cd·m⁻² as in `lighting.rs:3-4`. The base-colour texture is decoded from sRGB by its texture format when sampled; metallic-roughness, normal and vertex colour are linear. |
| Output | `shade` returns exposed linear HDR. The engine stores it as finite, nonnegative and at most 65504 in the Rgba16Float target (`rl_finite`: NaN and negatives become +0; it is the identity on valid values). Programs that add exposure-independent terms, like toon's rim, are explicitly exposure-dependent styles. |
| Receiver | Raster frames give the interpolated world position and quad-derivative footprints. Traced frames give the exact pixel-centre hit and ray-differential footprints. The same program source runs on both. Their differences are the already-measured derivative and receiver class: at most 1 code on the base-colour route ([primary-ray production](m6-primary-ray-production.md)). |
| Lights | Every `Lighting` light, with the fixed loop's direction and attenuation arithmetic. |
| Visibility | `light_visibility` is world-space and ray-traced (hardware hard rays, or Exact and Interactive disk visibility) or light-space shadow maps on raster renderers, never screen-space. It is computed at the engine's receiver, so the result is identical for every program at a pixel. Its quality is set by the renderer's shadow mode, not by the program. Soft visibility currently exists for one directional light (C15). |
| Environment | Prepared world-space diffuse and specular fields, or the constant environment. |
| Indirect light | Future world-space or ray-based services. Until GI exists, the environment terms are the only indirect light. |
| Reflections (M7-R1, [m7-r1.md](m7-r1.md)) | A program that declares `fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe` (`{ normal, alpha, weight }`, GGX) and reads `specular_incident(input) -> vec4<f32>` gets traced reflections on traced RT frames: rgb is the mean exposed radiance over the lobe, each hit shaded by its own program with view vector -d; w is the traced fraction (1 below roughness 0.35, 0 from 0.5, and 0 on raster, visibility-buffer and cell stages and at the hits themselves), where the program keeps its untraced `indirect_radiance` estimate. Hits read the open environment from the indirect inputs, and so does `surface()` in reflecting modules; `shade()` reads the receiver's prepared values. Programs naming neither get no reflection code. Non-GGX styles declare their nearest GGX lobe or none. |
| TR1 view dependence (MQ2, [m7-mq2.md](m7-mq2.md)) | A program may declare `fn view_dependence(s: Surface, input: SurfaceInput) -> ViewDependence` (`{ alpha, share }`): the share of its output that moves with the view like a GGX reflection of `alpha`. The traced shading entry (RT and visibility-buffer frames) stores `1 + share / width` in the HDR sample's alpha, `width` being the reflected direction's turn across one pixel (pixel angle, twice the normal's curvature, twice `alpha`); TR1 caps the history of pixels whose view direction turns and overwrites alpha with 1. Programs declaring nothing store 1 (view-independent). |
| Isolation | `surface` and `shade` see only their own receiver: no derivatives, neighbours or screen buffers. Their output depends only on the pixel's inputs, so dispatch grouping cannot change it. |
| Screen buffers | Post programs only, by declared `ScreenInputs`. |
| Determinism | Static repeat and return frames are bit-identical. There is no time input yet. |
| Reload | Each program reloads in its own transaction and keeps its last working pipelines. Diagnostics name program lines. Parameter edits need no compile. Engine-prelude reload stays one transaction over everything. |
| Safety | Programs cannot declare bindings, and naga runtime bounds checks stay on. |

Not guaranteed yet: generic texture slots (there are three fixed slots with fixed colour roles), alpha-tested or transparent materials (C14), vertex deformation (C13), custom vertex attributes (C12), programmable background or sky (C9), time, and evaluation of program materials at secondary ray hits.

### 2.4 Realistic-path quality bar and world-space inputs

The standard program is the realistic path, and the film-quality bar applies to its final image. Programmability must not cap that bar, which has three consequences:

1. **`surface` must be callable at any ray hit, not just at camera pixels.** Programs are built from hit-record reconstruction, get their footprints from ray differentials (and later ray cones), and cannot use derivatives or screen access. That makes ray-traced reflections and GI that see program materials possible later. Compiling programs into compute modules enforces this: naga rejects `textureSample` and `dpdx` in program code (M, below).
2. **The engine accumulates and denoises incident quantities, never program output.** Visibility, irradiance and radiance caches can use temporal accumulation, caching and denoising, keyed in world space or per receiver. Programs receive the filtered inputs, so every style benefits without depending on a specific technique. Screen-reprojected history of core lighting is avoided because of its off-screen and disocclusion failures. A final engine-owned image reconstruction (AA) remains style-independent.
3. **Programs need a secondary-hit response contract before GI lands.** One candidate is an optional `fn scatter(s, input, wi, wo) -> vec3<f32>` plus emission. The standard program would provide it through `brdf`. A program without one cannot be read by the engine, because `Surface` is user-defined. This is the main open interface decision ([R4](#7-open-risks)).

No screen-space technique enters the core: no SSAO, SSR, SSGI or screen-space contact shadows. The outline post reads screen buffers only because its program opts in.

### 2.5 Raster and traced execution

**Raster.**

- Each program gets its own render pipelines: lit with and without tangents, plus the hardware or fresh shading pairs on those renderers. They come from one generated entry point that does three things:
  - computes `source_front`, the derivative plane normal and the uv footprints in uniform control flow;
  - fills `SurfaceInput` and calls `surface`;
  - applies the single-sided discard, then calls `shade` and stores the result through `rl_finite`.
- The draw loop switches pipeline when the program changes. It cannot sort draws by program, because draw order defines equal-depth winners (`lib.rs:2`).
- The engine's single-sided discard moves after `surface`. Discarded fragments output nothing, so survivors are numerically unchanged.

**Traced.**

- The hit record is unchanged (`primary_trace.wgsl:34`). A per-draw program table sits next to the placement table, and each pixel's program is `table[draw(placement)]`.
- Trace and visibility kernels (`primary_trace*`, `fresh_compute`, the fused Interactive pass) contain no program code. They compile once per engine-prelude version, in their own modules.
- Per-program shading modules hold the prelude, the API, the program and the existing reconstruction (`primary_shade_pixel` builds `SurfaceInput` instead of `VertexOutput`).
  - **One visible program:** today's dispatch, `primary_shade` or `primary_shade_tiles`, over the whole image.
  - **K > 1 programs:** an override constant switches the trace kernels to OR `1 << program` into a workgroup atomic, producing one u32 mask per 8×4 tile (nonzero still means covered). Interactive frames already have that atomic; the Hard and Exact traces gain it. A compaction dispatch over the tiles (64,800 at 1080p) appends each tile to every program list whose bit it holds and writes the indirect arguments. Each program then shades its tiles in one indirect dispatch, and pixels of other programs exit after reading their record. At most 32 programs are visible per frame.
- **Visibility stays program-independent.** The engine computes it from the hit triangle's exact point and plane before shading: inline hard rays inside the shade pass, Exact receivers, or the fused Interactive 8-ray pass. This does not bind a lighting model, because visibility is a geometric property of receiver and light. The recorded constraint is that programs cannot move the shadow-ray origin (no parallax or pixel-depth offset) or choose which lights are traced.
- Misses keep the engine's clear colour. Uncovered Interactive tiles never run program code.

### 2.6 Reload and diagnostics (probe, M)

| Case | Result |
|---|---|
| Standard and toon programs, raster fragment and traced compute | Valid. naga parse and validate take 1.84-2.98 ms per composed module of 469-487 lines (release naga, CPU only, no driver compile). |
| `textureSample` or `dpdx` in program code, traced | Rejected: "Entry point rl_shade at Compute is invalid <- Uses operations forbidden at this stage". The error has no span. An IR scan locates the cause at program line 14:68 and line 20:26 in `fn surface`. The engine reports that location. |
| Same source, raster | Valid. Implicit sampling, even inside a per-pixel branch, passes naga 30's validation (see R2). |
| Program-declared `@group(2) @binding(9)` | Parses. The engine detects it by span (`["mine"]`) and rejects it. |
| Program redefines `light_count` | Rejected at program line 40:4: "redefinition of `light_count`". |
| Syntax error | Rejected at program line 32:60: "expected `)`, found `;`". |
| Missing `shade` | Rejected at an engine line. The engine checks for `surface` and `shade` first and names the missing function. |

The viewer polls each program file as it now polls `scene.wgsl` (`viewer/src/main.rs:653-706`). A rejection keeps the program's last working pipelines and sets the title.

### 2.7 How the requested styles map

| Style | Program |
|---|---|
| Photoreal PBR | Standard |
| Toon or cel | Banded `shade` over the shared lights and visibility, plus the outline post |
| Flat-shaded low-poly | `n = input.geometric` |
| Painterly | Triplanar or world-noise brush textures via `sample_*_grad` and world footprints; an optional screen-input post for stroke filtering |
| Stylised outline | Post with `ScreenInputs` |
| Custom lighting model | Any `shade` |

## 3. Performance architecture

| Frame | Cost against the fixed lit shader | Basis |
|---|---|---|
| One program, standard | Zero by construction: the same dispatch and the same per-pixel code. Gate: paired ratio ≤ 1.01 | Structure; Phase 1 measures it |
| One program, other | Only the program body. The pass is gather-bound, and a body the size of the lit body was free | M: ratio 1.04 |
| K visible programs | About K × 10 µs of dispatch overhead, 10-20 µs of compaction and about 0.006 ms of mask atomics, plus early-outs in mixed tiles. Two programs on the quarry: about 0.03-0.05 ms, 3.5-6% of the Interactive span | M per-dispatch fixed cost; I for the sum |
| Post with screen inputs | About 41 MB of writes (about 0.08 ms at 504 GB/s) plus a pass over the HDR image and the inputs (about 0.1-0.15 ms) | I; paid only when opted in |
| Raster | One pipeline switch per program change in draw order | I |

**Why not an uber-shader?**

- An edit to any program would recompile all of them.
- The worst program would set register pressure and occupancy for every pixel.
- Divergence in mixed tiles happens per pixel inside a warp.

Per-program pipelines keep the standard program's code and occupancy independent of whatever else is in the scene.

**Compile and edit-latency targets.**

- A program edit compiles one module per active path and only that program's shading pipelines. On a hardware renderer that is the raster lit pair; with traced selected, also the active `Shading` variant (one or two pipelines). Today's traced reload rebuilds four trace and visibility pipelines (two traces, fresh and Interactive) plus three shading pipelines inside two monolithic modules (`primary_ray_gpu.rs:98-130`, `598-644`), and it rebuilds the raster set too.
- **Targets:**
  - The pipeline count per program edit is verified through `PipelineStats`.
  - Compile time does not grow with the number of other programs or draws.
  - Program edit to first present stays at most 100 ms median with the existing 50 ms poll.
  - Engine-prelude edits do not regress against today's whole reload, measured on the same build.
- The traced reload baseline is measured first, because none exists.
- Startup is unchanged with only the standard program, which replaces the `fs_lit` pipelines one for one. Many programs make startup scale with their pipelines; a wgpu pipeline cache is the later lever.

## 4. Constraints in the current code

| ID | Location | Constraint | Resolution |
|---|---|---|---|
| C1 | `shaders/scene.wgsl:301-377` | `fs_lit` fuses material sampling, normal mapping, BRDF, environment, light loop and exposure, with no surface/lighting boundary | Split into the standard program plus engine entry points |
| C2 | `src/renderer.rs:280-291`, `909-942` | `Pipelines` holds one lit pair and one hardware pair, all from one module with entry `fs_lit` | A pipeline set per program |
| C3 | `src/renderer.rs:2373-2386`, `2394-2400`; `src/lib.rs:2` | Per-draw pipeline choice is keyed by tangent presence only; draws cannot be reordered | Key on program and tangent; switch in submission order |
| C4 | `src/renderer.rs:1028-1113` | `reload(wgsl)` is the only shader entry, and one transaction recompiles everything | Per-program transactions; prelude reload keeps the whole transaction |
| C5 | `src/primary_ray_gpu.rs:9`, `20-22`, `27-94` | Traced shading is built by exact-string rewriting of `fs_lit` and rejects other sampling | Program composition plus naga stage validation and an IR scan for locations |
| C6 | `src/fresh_shadow_gpu.rs:3-4`, `16-36`; `src/hardware_shadows.rs:98-99`, `106-151` | Fresh and hardware composition need exact pixel and geometric seam lines in `fs_lit` and the visibility markers | Generated entry wrappers own those lines; markers stay in the prelude |
| C7 | `src/primary_ray_gpu.rs:98-130` | The traced modules are two monoliths of trace kernels plus shading, so a material edit recompiles the trace kernels | Separate engine-kernel modules from program shading modules |
| C8 | `src/primary_ray_gpu.rs:407-418`, `598-644`, `1212-1253`; `shaders/primary_rays.wgsl:91-122` | One shading pipeline per `Shading` mode, one full-image dispatch, and `primary_shade_pixel` calls the single `primary_lit` | Per-program pipelines; tile-list dispatch when K > 1 |
| C9 | `shaders/primary_interactive.wgsl:10-35`, `38-46` | Tile coverage is a boolean, and uncovered tiles store the clear colour without running shading | Program mask under an override constant; background stays engine-owned |
| C10 | `shaders/scene.wgsl:6-13`; `src/material.rs:58-67`, `127-140`; `src/renderer.rs:567-605`, `1647-1700`; `shaders/primary_rays.wgsl:14-26`; `src/primary_ray_gpu.rs:533-544`, `862-888` | The material ABI is a fixed 48 B glTF factor block plus exactly three texture/sampler pairs with fixed roles, and there are no per-program parameters | Add a Parameters block (≤256 B) at new bindings; keep the factor block engine-owned and readable; generic slots later |
| C11 | `src/primary_ray.rs:20`, `32-45`, `47-66` | Binding arrays hold 10 per draw × 64 draws; the placement table holds only draw, tangent and sidedness | Add a per-draw program table; at most 32 programs per frame; report limits |
| C12 | `shaders/scene.wgsl:17-33`; `src/renderer.rs:42-45`; `shaders/primary_rays.wgsl:59-67`, `97-113` | The 48 B vertex format and `VertexOutput` are fixed: no uv1, vertex AO or custom attributes | Recorded; `SurfaceInput` is that fixed set |
| C13 | `src/hardware_shadows.rs:107-119` | The affine-vertex equality check rejects every vertex edit | Vertex deformation is unsupported; the BLAS must match the drawn geometry |
| C14 | `shaders/primary_trace.wgsl:21`; `shaders/fresh_shadows.wgsl:72`; `shaders/hardware_shadows.wgsl:30`; `src/hardware_shadow_gpu.rs:232`; `src/primary_ray_gpu.rs:191-211` | Camera and fresh shadow rays force opacity, BLAS geometry is built opaque, and admission is opaque only | No alpha-tested program materials yet; a future `opacity` stage with per-instance non-opaque flags |
| C15 | `shaders/fresh_shadows.wgsl:8-11`, `35-36`; `src/fresh_shadow_gpu.rs:48` | Soft visibility is a per-pixel count for `lights[0]` and one directional request | `light_visibility` quality is per mode; recorded |
| C16 | `shaders/scene.wgsl:378-385`; `src/renderer.rs:2419-2443` | Presentation reads only exposed HDR; there is no post stage or screen input, and traced frames allocate no depth target | Add a post stage with opt-in inputs written by shading |
| C17 | `viewer/src/main.rs:653-706` | The viewer polls a single shader file | Watch program files too |

These are already generic and stay unchanged:

- the 16 B hit record;
- the exact traced receivers;
- the fused Interactive visibility, which is geometric;
- explicit ray-differential gradients (`primary_rays.wgsl:70-85`);
- the descriptor-indexed geometry gather.

## 5. First unit

Four phases. Each is retained only if its gate passes, and later phases do not start on a failed gate. GPU, timing and window work waits until Islam is not playing (AGENTS.md); coding and CPU checks can run meanwhile with two jobs. Implementation goes to Sol. Opus judges acceptance.

**Phase 1: standard program through the API.**

- Build: the engine prelude and API files, `shaders/programs/standard.wgsl`, and generated raster and traced entry points. Raster gets lit, lit-tangent, hardware and fresh pairs; traced gets Hard, Soft and Interactive. Also `Program`, `try_create_program`, `reload_program`, `standard_program`, separate trace-kernel modules, and naga promoted from a dev-dependency to a dependency. naga 30.0.1 is already in `Cargo.lock` through `wgpu-core`, so this adds no crate.
- **Byte identity:** these must be byte-identical to the pre-change build:
  - traced primary acceptance `accept-b12` (285 files);
  - Interactive acceptance `accept-b7` (36 files);
  - thin and stress fixture replays (575 + 575 files);
  - all 18 raster lit suites;
  - hardware 4x, mirrored/nonuniform normal-map, environment and single-sided fixtures.

  Validation must be active with zero errors.
- **Time:**
  - Shading pass: paired in-process screen of the frozen `primary_lit` module against the standard program module on one ABI, 3 processes × 48 pairs, median ratio ≤ 1.01 in every process.
  - Complete Interactive frames with the triplet method: span within 1% of 0.8606-0.8615 ms, and a paired Interactive/hard ratio of 0.900-0.902 or lower.
  - Raster lit pass: paired ratio ≤ 1.01.
- **Reload:**
  - Invalid standard WGSL is rejected with its program line, and the frame is retained.
  - The tint edit applies, and its revert is byte-identical.
  - Prelude reload stays transactional.
  - Report the traced reload compile baseline and the per-program compile time.
- **Stop rule:** a byte difference stops the unit.
  - Diagnose it first: diff naga's SPIR-V for the old and new code, then classify the first differing pixels.
  - Then try keeping the expression structure across the function boundary.
  - On raster, use an engine-only implicit-derivative helper for the standard program's mesh-uv samples (R2).
  - If it still differs, report to Islam. Do not substitute a reference-equivalence argument.
  - A time regression above 1% is diagnosed (registers, call boundaries) before Phase 2.

**Phase 2: toon program.**

- Build: Parameters reflection and packing, `MaterialDescriptor::{program, parameters}`, `try_update_parameters`, and the toon program in `shaders/programs/toon.wgsl`.
- **Acceptance:**
  - All-toon quarry and fixture frames on traced (Exact and Interactive) and raster are inspected, full image and crops: bands, shadow tint, rim, silhouettes, contacts, motion 0→1→return.
  - Visibility counts are byte-identical to the standard frame at each pose and seed. That proves shading programmability does not touch shadows.
  - Static repeat and return frames are bit-identical.
  - Raster/traced toon differences are reported in the same class as standard's.
- **Reload:**
  - A toon edit recompiles only toon pipelines (`PipelineStats`).
  - Standard pipelines and images are untouched.
  - An invalid toon edit leaves both programs rendering.
  - A parameter edit applies with zero compiles.

**Phase 3: mixed-program frames.**

- Build: the per-draw program table, mask override constants on the trace kernels, compaction and indirect dispatch.
- **Acceptance:**
  - With some quarry draws toon and the rest standard, standard pixels equal the all-standard frame and toon pixels equal the all-toon frame, byte for byte.
  - Hard, Exact and Interactive modes are all covered.
  - A CPU test checks that tile lists cover every covered tile once per program bit.
  - Single-program frames still meet the Phase 1 time gate.
- **Cost:** report the two-program span and shading against single-program.
- **Stop rule:** above 0.06 ms of overhead, screen finer tiles and an uber-switch for mixed tiles only, before retaining.

**Phase 4: outline post.**

- Build: `try_set_post`, `ScreenInputs` written by the shading wrapper (traced) and an MRT variant (raster) at 1x, and the outline in `shaders/post/outline.wgsl`.
- **Acceptance:**
  - Ink lands on toon silhouettes, instance edges and creases on both paths.
  - Standard pixels in a mixed frame stay byte-identical with the post enabled.
  - Screen inputs appear in the ledger only while the post is set.
  - The post reload is transactional.
  - The motion flip-book is inspected.
  - Cost is reported.
- 4x raster rejects screen-input posts.

**Out of scope:**

- generic texture slots and bindless textures;
- alpha test and transparency;
- vertex deformation;
- background programs;
- time input;
- secondary-hit evaluation and GI;
- node graphs;
- a programmable present stage, which stays editable through the prelude.

**CPU checks:**

- composition and seam-free validation;
- the binding-span rejection and the line-mapping tests;
- Parameters reflection and packing;
- tile-list coverage;
- Clippy with `-D warnings`, `cargo fmt --check` and `git diff --check` on the renderer and viewer.

## 6. Rejected options

| Option | Reason |
|---|---|
| Whole-file scene WGSL replacement as the user model (status quo) | One shader for all materials; exact-string seams (C5, C6) mean traced mode rejects ordinary sampling edits; users must know engine internals |
| Fixed shading models with parameters only | Cannot express toon, painterly or custom lighting without engine changes; the standard library functions cover the realistic case instead |
| Fixed G-buffer plus a user lighting pass | Stores a surface model (base, metal, rough, normal), which AGENTS.md forbids baking in, and adds bandwidth to a gather-bound pass. The hit record already is a model-free visibility buffer |
| Uber-shader switching on program id | Any edit recompiles every program; worst-case registers for all pixels; per-pixel divergence; kept only as a measured fallback for mixed tiles |
| Per-draw or per-material-instance dispatch | About 10 µs fixed per dispatch (M); programs are far fewer than draws |
| Per-pixel compaction into program lists | Loses 2-D locality for hit records and textures; the measured per-draw compaction variant was slower |
| Engine-owned per-light callback only (`light()`) | Fixes the accumulation order, and styles that shape the total (banding the sum) cannot be written. It may return as a library helper |
| BRDF-only hook | Too narrow: no rim, banding or custom ambient |
| Node graphs now | YAGNI. A graph must later compile to the same WGSL interface, with no separate runtime |
| Screen-space core inputs (SSAO, SSR, SSGI, screen contact shadows) | Off-screen and disocclusion failures conflict with the bar (AGENTS.md `48eabf5`) |
| User-declared bindings or arbitrary user passes | The engine must own layouts for transactional reload, descriptor arrays and per-path ABIs. Post with declared inputs covers outlines |
| An import system (naga_oil or WESL) | A new dependency; prefixing and composition order suffice |
| Disabling naga runtime checks | Measured slower (0.449 against 0.430 ms) and unsafe for user code |
| Engine denoising of program output | Output is style-specific. The engine filters incident quantities instead |

## 7. Open risks

- **R1 Byte identity.** Splitting the standard body across functions and modules can change driver code generation, for example FMA contraction, since naga emits no `NoContraction`. Phase 1 decides; the stop rule above applies.
- **R2 Raster derivatives.** naga 30 accepts implicit sampling inside a per-pixel branch (probe, M). In divergent quads that gives undefined derivatives, which a traced compute module cannot have. Public raster helpers should therefore use the engine's uniform-flow footprints with `textureSampleGrad`. Whether that matches raster `textureSample` bit for bit on this GPU is unmeasured. If it does not, the standard program alone keeps an engine-internal implicit helper.
- **R3 Multi-program cost** is estimated, not measured. Mixed-tile overhead grows with program boundaries on screen.
- **R4 Secondary-hit contract.** Film-quality GI and reflections need each program's response at off-screen hits, and the contract is undecided (section 2.4). It must be settled before GI, or user materials will either be invisible to GI or forced into a fixed model.
- **R5 Visibility coverage.** Soft visibility covers one directional light (C15). Point and spot lights need ray-traced visibility for the film bar. Every program inherits what the engine offers.
- **R6 Limits.** 64 draws, 32 programs per frame, three texture slots and 256 B of parameters. Generic slots grow descriptor counts, and bindless needs partially bound arrays or padding.
- **R7 Compile latency and startup** grow with programs. There is no pipeline cache yet, and the traced reload baseline is unmeasured.
- **R8 Runtime faults.** A looping or very slow user shader can cause device loss, which is an `Error::Fault`. The last-working fallback covers only compile-time rejection.
- **R9 Screen-space outlines at 1x** alias and can shimmer under motion. They are stylistic and opt-in. Final-image AA comes later, and 4x raster screen inputs are undefined.
- **R10 Background.** Misses run no program code. A programmable sky would run over about 83% of tiles on the quarry view.
- **R11 Mixed exposure.** Programs that add exposure-independent terms, like toon's rim, change appearance with exposure. This is documented, not prevented.

## Evidence and time

Specular-filter API update (M7 U1): `SurfaceInput.normal_dx/normal_dy` are
one-pixel differentials of the interpolated world-space vertex shading normal.
Raster supplies quad derivatives and compute supplies hit-plane ray
differentials. They are geometric inputs, with no neighbouring lighting.
`geometric_normal_variance(input)` normalizes/differentiates that normal;
`material_normal_variance(input)` reads the variance-bearing normal map and
normal scale. Custom GGX programs may opt in with
`filtered_roughness(rough, material_normal_variance(input) + geometric_normal_variance(input))`
and use the result for both direct and environment lobes. Programs with their
own perturbed normals must supply matching variance themselves. Non-callers
adopt no filtering. The material `normal_mapped` flag remains 0/1; the new
`normal_variance` flag identifies the versioned texture encoding. Both flags
are supplied by the engine. The 48-byte material block and 16-byte hit record
are unchanged. Limits and measured acceptance are in [the U1 record](m7-u1.md).

M7 U1b adds `SurfaceInput.world_dx/world_dy`, one-pixel world-position
differentials on both paths, and the optional `specular_brdf(input, light_direction,
base, metallic, roughness)` helper. The standard program leaves
`SPECULAR_AA=false` (opt-in because cost misses the floor). Enabling it calls
this helper for direct lights, retaining the original
roughness in each GGX summand. Environment sampling stays unchanged. The helper
integrates 256 deterministic spatial samples in the current footprint; it reads
no previous frame or neighbouring lighting. It filters the BRDF response, while
light direction, attenuation and visibility remain evaluated at the centre.

The fixed spatial helper also works on existing RGBA8 normal maps, without
recooking or a companion texture. Missing variance metadata conservatively
selects the full 256-sample rule. Covariance/mixture experiments need the cooker's
`--normal-maps --normal-moments` encoding (RLA v5, two RGBA16Float layers,
four times RGBA8 storage). Adaptive-count experiments can use the v3
`--normal-variance` encoding at ordinary RGBA8 storage cost.
`normal_statistics(input)` exposes mean slope and full covariance after
texture filtering, independent of any BRDF. `normal_variance=2` identifies this
encoding; 0 and 1 keep their U1 meanings. `sample_normal_layer_grad` supplies
explicit-footprint layer access; `sample_normal_map` keeps its decoded-normal
contract. Legacy mapped materials retain their fine normal field even though
their mips discarded variance; fixed quadrature integrates that field directly.
Unmapped curved normals use geometric differentials.
Known zero-variance distributions bypass integration; legacy maps without an
estimate use the conservative full rule. Non-calling custom programs
keep their own shading. The material block, hit records and dispatch remain
unchanged. Custom perturbed normals, tangent discontinuities, environment AA,
coverage AA and spatially varying visibility are outside this helper's measured
contract. Covariance-based `filtered_brdf` selectors are experiments, not the
accepted standard method. See [U1b](m7-u1b.md) for cost, gate results and limits.

The probe is at `D:/Coding/renderlab-work/programmable-shading-probe/`:

- `src/main.rs` (sha256 prefix `f3371700d76f2e19`) and `wgsl/` with `api_common`, `api_raster`, `api_traced`, `api_screen`, `standard`, `toon` and `outline`;
- outputs `run-1.txt` and `run-2.txt`;
- the release binary (`9869e70462c9b428…`).

It reads this checkout's `shaders/scene.wgsl`, removes `fs_lit`, composes each program as section 2.1 describes, and validates with naga 30.0.1 using all validation flags and capabilities. It is built offline with `cargo build --release -j 2` in its own target directory. Reproduce with `target/release/program-probe.exe <checkout> <probe dir>`.

The probe checks only that the WGSL validates. It does not check numeric equality with `fs_lit`, which needs the GPU (Phase 1), and the traced API uses one bound material where production uses per-draw binding arrays.

Time:

- Grounding and design were CPU reading, not stopwatched.
- The probe's first dependency build was not timed. Each rebuild took about 0.9 s, each run under 1 s, and naga parse and validate took 1.84-2.98 ms per module.
- No repository build, test, GPU or window run happened, and there is no push.
