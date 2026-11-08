# M7-R1n: software reflection rays on the non-RT path

Overnight unit (Islam's complete-base push, 2026-11-08; rules in AGENTS.md). Branch `t3/m7-r1n`, from main `ffdaee9` (R1 merged). Inputs: [R1](m7-r1.md) (the RT design and its hooks), [R0c](m7-r0c.md) (cook-time cluster-LOD BVH), [P0](m7-p0-claims.md) §4.11 (R1n's claim notes), [G1d](m7-g1d.md) (the GI cache's software traversal and merged world BVH), [R0a](m7-r0a.md) and its floors. Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## Status

| Item | Result |
|---|---|
| Non-RT path | **On by default** on visibility-buffer frames whose drawn meshes all carry software geometry. R1's reflection block runs unchanged except for its two ray queries, which become a software traversal of per-mesh BVHs over the drawn triangles. Each hit is shaded by the frame's program with R1's contract, including `decal_prepare`. |
| Quality, RF1-RF9 static (M) | Equal to R1's RT scores within 0.04 codes on every fixture. Every registered population better than environment-only, none worse. Beats UE SW Epic on RF1-RF7 and RF9; misses it on RF8 (7.96 vs 5.24). Beats UE HW-Hit Epic on RF1-RF6 and RF9; misses RF7 and RF8, as R1 does. |
| Mirror-camera identity, non-RT (M) | **Pass**: 0 of 110,592 channels over 0.002 cd/m² + 0.5%; max error 0.0078; means 9.04852 / 9.04828 (R1's RT run read 9.04852 / 9.04827). |
| TR1 (default on) | Its frame rotation turns the glossy strata as on RT. RF2's 0.3/0.4 sparkle is integrated (section 2.2). |
| Quarry | QUARRY_STATUS |
| Cost on reflective content | TIMING_STATUS |
| Default ruling (overnight rule 1) | Clearly better than environment-only on every RF fixture, not visibly worse in any inspected image, and the quarry is untouched (coverage rule). **On by default.** |
| `ReflectionSettings` (root's request) | `Renderer::set_reflections(ReflectionSettings { enabled, max_roughness })`, default on at 0.5 (R1's bands bit for bit). Honoured by R1's traced reflections and by R1n, through one word group in the frame uniform (section 1.5). |
| Patents | Design-around tier (P0 §4.11, C1/C7, W8); no fallback used (section 5). |

## 1. Design

### 1.1 What is reused from R1

The non-RT path runs R1's reflection block (`shaders/reflections.wgsl`) unchanged except for its two ray queries:

- the same program ABI: `reflection_lobe` declares the GGX lobe, `specular_incident` returns `vec4` (rgb the mean exposed radiance over the lobe, w the traced fraction);
- the same bands: one mirror ray below roughness 0.05, four fixed VNDF strata to 0.35, the traced fraction fading to 0 at 0.5;
- the same per-pixel azimuth hash, turned by TR1's frame rotation word, so TR1 (on by default) integrates the glossy strata;
- the same ray-cone footprint, the same exact-epsilon origin on the viewed side of the receiver's plane;
- the same hit reconstruction (`rl_hit_input`): the hit's `surface()` and `shade()` run with its own material parameters, view vector −d, ray-cone uv and normal differentials, and DC1's decals through `decal_prepare`;
- the same constraints: one bounce (`specular_incident` is 0 at hits), hits read the open environment, hit light visibility is one hard ray.

`src/program.rs` composes this software variant when the head declares `rl_geometry` (only non-RT shading heads do) and the program declares both halves of the pair. It replaces the two ray-query blocks by text seams (asserted to match exactly once):

| R1 (RT) | R1n (non-RT) |
|---|---|
| `rayQuery` closest hit, opaque, both faces | `rs_closest(origin, d, epsilon, RL_FAR)` → the same `(placement + 1, primitive, barycentrics)` record |
| `rayQuery` first hit toward the light | `rs_blocked(origin, l, epsilon, tmax)` |

RT modules are byte-for-byte what R1 composed: the software text is added only on heads that declare `rl_geometry` (CPU test `software_reflection_composition`). Programs that name neither hook still get no reflection text on any head.

### 1.2 Geometry: why not R0c's LOD BVH or G1d's merged world, yet

The brief named R0c's cooked cluster-LOD BVH, chosen by ray-cone footprint, or G1d's merged near-field world BVH. Neither can feed R1's hit contract today:

- **Hit shading needs the drawn triangle.** R1 reconstructs a hit from `(placement, primitive, barycentrics)` in the draw's own index and vertex buffers, then runs the program with that triangle's normals, uvs, tangents and colours. G1d's world BVH is built from transformed far-field cuts and keeps only positions; its hits map to a placement's material evaluated over its whole texture (`gi_hit_primitive: none`). R0c's full-detail build (`near_field::build_bvh`) reorders triangles inside leaves and keeps only the first triangle's source offset. R0c's cooked cut keeps source cluster and page identity, but only the quarry has a cooked cut, and its LOD triangles are not the drawn triangles of a create-mesh draw.
- **RF content is small.** Every RF fixture mesh is a few hundred to a few hundred thousand triangles, where a full-detail tree is cheap and exact.

So R1n builds its own per-mesh BVH (`src/reflection_software.rs`), with R0c's and G1d's properties where they apply (object-space per-mesh trees, never refit, a flat instance list rebuilt on the CPU, global-first traversal per ray) and the one property they lack (drawn-triangle identity):

- **BVH2 over the drawn triangles**, median split on the longest centroid axis (deterministic; ties break by triangle index), at most four triangles per leaf.
- **Nodes** are 16 words: the two children's float boxes and their tagged IDs (`LEAF | first << 3 | count`).
- **Triangles** are 10 words: corner a, edges b−a and c−a in index-buffer corner order, and the primitive index. Möller–Trumbore's (u, v) are then the weights of corners b and c, exactly the hardware query's barycentrics, so `rl_hit_input` is unchanged.
- **Built once at upload** on non-RT visibility-buffer renderers, for meshes up to `MAX_TRIANGLES` (2^19 triangles). Measured 61.5 bytes a triangle (1,520 triangles: 93,504 bytes; build 0.6 ms in the test build). Larger meshes and virtual (streamed clusterLOD) meshes get none.
- **Coverage rule.** A non-RT frame traces reflections only when every drawn mesh has software geometry. Otherwise the frame keeps every program's untraced estimate: a ray through an uncovered mesh would show the sky behind it, a light leak worse than the environment estimate. The quarry's twelve scans (0.99-2.0 M triangles each) are all over the cap, so the quarry traces nothing (gap 2).
- **Per frame** (`prepare_software_reflections`, in the traced/visibility-buffer frame preparation): every distinct drawn mesh's BVH is packed once into one storage buffer behind a 24-word instance record per traced placement (world-to-object rows, a world box, the mesh's bases, its root and placement slot). The buffer is repacked only when the drawn meshes or the instance count change; transform edits rewrite only the records. Group 0 binding 36 of the non-RT shading layouts (no new bindings on RT layouts).

LOD selection from the ray cone (R0c's `select_cone_level`) is not used: full detail is the only level for create-mesh geometry. It becomes useful once large meshes carry a cooked cut with drawn-triangle identity (gap 2).

### 1.3 Traversal (`shaders/reflections_software.wgsl`)

Each ray is its own work item. It loops over the instance records, rejects an instance by its world box, transforms the ray into object space (unnormalised, so t is shared), and walks the BVH2 with a 32-entry stack, nearer child first. Closest hit for reflection rays; first hit for light visibility. Both faces block, as R1's opaque queries.

### 1.4 Ownership and shared edits

New files: `src/reflection_software.rs`, `src/reflection_software_tests.rs`, `shaders/reflections_software.wgsl`, `scripts/m7-r1n-run.py`. Marked shared edits (`M7-R1n`):

- `src/program.rs`: the software branch of `reflection_api` and its seams;
- `src/primary_ray_gpu.rs`: the head declaration, binding 36 in the non-RT shade layouts and bind groups, the bindings key's generation word, `SoftwareReflections` and its preparation;
- `src/mesh.rs`, `src/renderer.rs`, `src/virtual_gpu.rs`: the mesh's `reflection` field and its upload-time build;
- `src/reference_r0a_tests.rs`: opt-in `RENDERLAB_R1N_PROGRAM` (environment-only baseline) and `RENDERLAB_R1N_TIMES` (timed frames) for the RF A/B.

The GI cache (G2b's) is not touched.

### 1.5 Global settings (`ReflectionSettings`)

Root asked for a global switch for API1 (until now reflections could be disabled only per program). `src/reflections.rs`:

```rust
pub struct ReflectionSettings { pub enabled: bool, pub max_roughness: f32 }  // default: true, 0.5
Renderer::set_reflections(settings) -> Result<(), Error>; Renderer::reflections()
ReflectionSettings::off()
```

- `max_roughness` (in [0, 1]) is the perceptual roughness where the traced fraction reaches 0; it starts to fall at 0.7 of it. The default reproduces R1's 0.35/0.5 bands bit for bit (CPU test `reflection_settings_bands`).
- Disabled, the bands are zero: nothing is traced on either path, and every program keeps its untraced estimate (`specular_incident` w = 0).
- Both paths read the bands from one new `vec4` at the end of `PrimaryFrame` (`primary_frame.reflection`; `FRAME_BYTES` 112 → 128). The non-RT coverage rule writes zero bands for frames with uncovered meshes.
- Programs still opt in per program by declaring `reflection_lobe` and reading `specular_incident`; the setting bounds what any program gets.
- **Hit shading still goes through the program interface R1 uses** (`rl_load_parameters`, then the composed program's `surface` and `shade` at the hit). When MP1's per-program dispatch arrives, R1n's hits follow it with R1's: the software path changes only how the hit record is found.

## 2. RF1-RF10 static (M)

Run `d1` (binary `d`: main `f159fcd` merged, the shipped code), cross-checked against `b1` (binary `b`, before the merge): all scores identical. Non-RT: visibility-buffer primaries, Virtual disk shadows, on a device without ray queries; 1920x1080, 1x, TR1 off, eight warm frames, GI cache off (R0a's protocol). Scored by R1's `scripts/m7-r1-score.py` against `reference/final`. "Environment-only" is the same build with main's pre-R1 standard program (`ae95d3a`) on every non-toon material (`RENDERLAB_R1N_PROGRAM`); it reproduces R0a's current-renderer row (to within 0.001 codes in a few RF1-pitch20 populations, the non-RT run-order state R1 recorded).

Whole-reflector display RMS at −14 stops, 8-bit F0 codes (lower is better):

| Fixture / pose | Environment-only | **R1n** | R1 (RT, A) | UE SW Epic | UE HW-Hit Epic | Populations better / worse |
|---|---:|---:|---:|---:|---:|---|
| RF1 pitch30 | 46.100 | **4.154** | 4.154 | 37.165 | 9.331 | 20 / 0 |
| RF1 pitch20 | 50.191 | **3.634** | 3.634 | 29.193 | 8.184 | 20 / 0 |
| RF1 pitch40 | 38.857 | **4.002** | 4.001 | 34.707 | 7.325 | 20 / 0 |
| RF2 sweep | 20.956 | **13.791** | 13.789 | 15.920 | 15.195 | 48 / 0 |
| RF3 pitch30 | 45.757 | **3.457** | 3.457 | 32.202 | 10.140 | 20 / 0 |
| RF4 pitch30 | 45.697 | **4.870** | 4.870 | 40.611 | 10.469 | 72 / 0 |
| RF5 pitch30 | 46.100 | **4.154** | 4.154 | 37.165 | 9.331 | 20 / 0 |
| RF6 grazing5 | 52.315 | **6.713** | 6.715 | 12.273 | 9.529 | 20 / 0 |
| RF6 grazing10 | 51.754 | **6.369** | 6.369 | 12.457 | 6.750 | 20 / 0 |
| RF7 between panels | 48.035 | 23.284 | 23.284 | 35.645 | 20.203 | 14 / 0 |
| RF8 pitch30 | 9.901 | 7.964 | 7.930 | 5.241 | 3.913 | 20 / 0 |
| RF9 pitch30 | 46.163 | **7.556** | 7.555 | 37.020 | 9.606 | 22 / 0 |
| RF10 pitch30 | unsupported | unsupported | unsupported | 53.124 | 15.611 | — |

Bold: below both Unreal Epic floors. RF8's small RT/non-RT difference is the rocks' own shadowing (Virtual against Interactive), not the reflection. RF10 (a toon sphere in RF1) still produces no frame: mixed-program frames are rejected on both paths (MP1's work).

### 2.1 Inspection

Sheets: `D:/Coding/renderlab-work/m7-r1n/runs/b1/sheets/` (reference | image | 4x |difference| at −14 stops, per case for environment-only, R1n and R1n + TR1) and the stacked `inspect-*.png`.

- **RF1/RF3/RF5:** the puddles reflect the red and blue targets and their off-screen parts; the environment-only puddles are flat grey (the whole puddle area is the brightest difference). R1n's difference panels are dark except the primary-miss rectangle (R0a's registered background item).
- **RF4:** the thin rails and poles appear in the puddles.
- **RF2:** the 0-0.2 metal spheres reflect the floor, wall and neighbours. Without TR1, the 0.3/0.4 spheres sparkle (four static rays), as R1 recorded on RT; with TR1 the sparkle is integrated.
- **RF6:** grazing puddles show the stretched target reflections.
- **RF7/RF8/RF9:** as R1 on RT: one bounce between the panels; the rough rocks change little; the shell's back faces reflect.
- Nothing is visibly worse than environment-only in any inspected image.

### 2.2 With TR1 (the default)

Run `b1`, 32 still TR1 frames from a cut (`RENDERLAB_R1_TR1=1`):

| Fixture / pose | R1n, no TR1 | R1n + TR1 | R1 + TR1 (RT, A) |
|---|---:|---:|---:|
| RF1 pitch30 / 20 / 40 | 4.154 / 3.634 / 4.002 | 2.826 / 2.986 / 2.744 | 2.889 / 2.988 / 2.381 |
| RF2 sweep | 13.791 | 12.884 | 12.881 |
| RF3 pitch30 | 3.457 | 2.605 | 2.682 |
| RF4 pitch30 | 4.870 | 3.595 | 3.547 |
| RF5 pitch30 | 4.154 | 2.915 | 2.844 |
| RF6 grazing5 / 10 | 6.713 / 6.369 | 6.106 / 4.332 | 6.177 / 4.318 |
| RF7 between panels | 23.284 | 23.161 | 23.161 |
| RF8 pitch30 | 7.964 | 7.528 | 7.501 |
| RF9 pitch30 | 7.556 | 7.071 | 7.053 |

Against environment-only at −14 and −10 stops, the TR1 rows are better in 17-76 populations per case and worse in at most three: directly viewed objects at −10 (+0.05 to +0.12 codes, TR1's own resolve, not reflections) and RF9's shell (6.57 → 6.70), which R1 recorded on RT too.

## 3. Cost (M)

TIMING_SECTION

## 4. Quarry (M)

QUARRY_SECTION

## 5. Claim notes (engineering reading, not clearance)

P0 §4.11 reads the R1n tier as clear of US 12,051,145 and US 10,380,785 on its operative constraints. As built:

- **US 12,051,145 (Adshir/Snap).** Each pixel's rays start at its own visibility-buffer hit and write only its own value. There is no grouping, packet, beam, shared frustum, common projection or matching (C7).
- **US 10,380,785 / 10,614,614 / 10,818,072.** No spatial cells with objects mapped onto them. Per-mesh BVHs are per object; primary hits come from the visibility buffer and are never registered to a cell; rays are per-pixel work items traversing a flat instance list from the top, never launched per cell or node (W8).
- **US 11,315,310 and the ReSTIR family.** Fixed stratified VNDF quadrature, no reservoir or resampling (C1). Hits read the open environment; nothing reads or writes the world radiance cache.
- **Denoiser and history families.** None; TR1 integrates R1n's output as an ordinary shaded sample, as on RT.
- Möller–Trumbore (1997) and the slab test are textbook methods; no third-party or Epic code was copied.

## 6. Verification and evidence

| Check | Result |
|---|---|
| `cargo test -p renderlab --lib` inside R1n leases | `a1` (binary `a`): 155 passed, 0 failed, 147 ignored. `d1` (binary `d`, the shipped code): **161 passed, 0 failed, 148 ignored**. Includes `reflection_bvh_keeps_primitive_identity` (a CPU replay of the traversal: every triangle of a 23x23 grid found by its centroid ray with its own primitive and barycentrics; rebuilds word-identical), `reflection_instance_records`, `software_reflection_composition` (both non-RT heads: hook once, no ray query, naga-valid, SPIR-V; toon composes nothing; RT heads keep R1's ray queries; both read the bands) and `reflection_settings_bands` |
| Non-RT mirror identity | `a1`: failed (90 channels; a shared-edge seam, fixed by the edge slack). `b1`: **pass**. RT_CHECKS |
| RF1-RF10 non-RT | `b1` and `d1` scores identical |
| Quarry | QUARRY_VERIFY |
| `cargo fmt`, `clippy` | FMT_CLIPPY |

**Process.** Every GPU process ran inside an R1n lease holding `D:/Coding/renderlab-work/gpu-slot.lock` (`scripts/m7-r1n-run.py`: R1's runner with its own owner, output and Cargo target on D:). The guard stopped lease `d1` when another unit's renderlab build script started; its finished steps are kept and the rest reran as `d2`. Builds ran outside QUIET windows and never during my own lease. Implementation took about 2 h and verification about 1 h, overlapping (a 15-minute cold build, then about 2 minutes per incremental build, plus lock waits).

**Evidence** in `D:/Coding/renderlab-work/m7-r1n/runs/`: `a1` (lib, the failed identity), `b1` (RF A/B, TR1, the first quarry run, identity; sheets), `d1` (lib, RF on the shipped code), `d2` (timed environment-only RF, TR1, quarry A/B with TR1 off, RT checks, identity with the off switch). Raw `.f32` captures are deleted once scored; scores, sheets, JSON and logs stay.

```powershell
py -3 -B scripts/m7-r1n-run.py build LABEL
py -3 -B scripts/m7-r1n-run.py gpu RUN nonrt:lib nonrt:rf:16 nonrt:rfbase:16 nonrt:rftr1 nonrt:quarry:static,abcost rt:rf rt:identity nonrt:nrtidentity --binary LABEL
py -3 -B scripts/m7-r1-score.py D:/Coding/renderlab-work/m7-r1n/runs/RUN
py -3 -B scripts/m7-r1-quarry.py D:/Coding/renderlab-work/m7-r1n/runs/RUN
```

## 7. Gaps

Recorded in [m7-gaps.md](m7-gaps.md) under "Reflections", item 1.
