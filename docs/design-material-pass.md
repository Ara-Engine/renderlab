# Material pass and the material record

`RENDERLAB_MATERIAL_PASS=1` (off by default) splits primary shading in two. A material pass decodes each pixel's hit, runs the program's `surface` and writes a material record. Shading then rebuilds `Surface` from the record instead of running `surface` again. A program opts in by declaring `material_out` / `material_in` (`MaterialRecord`, shaders/program_api.wgsl). The standard program does; any other program keeps its single kernel.

The code is in `src/program.rs`, section "Material pass" (`material_variant`, `MATERIAL_PACKING`); the port dispatch is in `src/backend_ara/forward.rs` (`material_records`). The wgpu wiring is not built yet.

## Record (32 bytes a pixel, group 0 bindings 75 and 78, two rgba32uint images)

| word | x | y | z | w |
|---|---|---|---|---|
| 0 | shading normal, octahedral 2 x unorm16 | base red, green (unorm16 each) | base blue (unorm16), coverage and metallic (unorm8 each) | emission x exposure, rgb9e5 |
| 1 | geometric normal, octahedral | model (8 bits), MQ2 lobe alpha (unorm8), roughness (unorm16, rounded up) | MQ2 view share (unorm16), 16 bits spare | spare |

- Model 255 marks a pixel whose program keeps its own `shade`.
- Roughness is rounded up so that the decoded value is never below the evaluated one. A value authored at a threshold, such as `LocalShadows::unshadowed_hits_from`, stays on its side of it.
- The first record was 24 bytes, with a 10-bit base colour and nearest-rounded roughness. It failed three port cases with the pass on (Linux Radeon; two of them on the M5 as well):
  - the 10-bit base moved every pixel of a flat floor by one f16 step;
  - it also turned a texture's sRGB decode difference of a hair into a whole step;
  - roughness at the threshold decoded just below it.
- Even an f32 octahedral normal does not round-trip bit for bit, so some pixels still move by one f16 step with the pass on. `traced_frame_agrees_with_the_mesh_pass_over_misses_sky_and_post` therefore counts only differences past one step while the pass is on.

## Cost (M5, Ara main, Showcase at EV 10 / Glint goal at EV 13.5, medians of 3 interleaved rounds)

`RENDERLAB_MATERIAL_PROBE` (measurement only) splits the cost. All figures are ms, Showcase / Glint.

| | Showcase | Glint |
|---|---|---|
| shading, pass off | 7.37 | 6.71 |
| shading, pass on (32 B) | 7.07 | 6.19 |
| material pass | 1.00 | 0.91 |
| └ dispatch and list walk (`empty`) | < 0.10 | 0.11 |
| └ hit decode (`decode` - `empty`) | 0.72-0.75 | 0.69 |
| └ `surface` and packing (`nostore` - `decode`) | 0.16 | 0.10 |
| └ store (full - `nostore`) | 0.01 | 0.00 |
| record load and unpack in shading (`loadplus` - off) | 0.49 | 0.13 |
| vertex attributes still read by shading (on - `readlean`) | 0.25 | 0.28 |
| position side of the decode, alone (`readpos`, includes the list walk) | 0.63 | 0.60 |

What these show:
- **The store is free.** Going from 24 to 32 bytes changed neither the pass nor the read measurably (the 24-byte record read 0.47 / 0.15).
- **`surface` costs 0.10-0.16 ms in the lean pass and about 0.6 ms inside the shading kernel.** The split's gain so far is occupancy.
- **What runs twice is the hit decode.** The pass needs all of it, to run `surface`. Shading still needs its position side (receiver, footprint, geometric normal) and some vertex attributes.

## Decision: the material pass becomes the decode-once pass

The record grows by one rgba32uint word: the trace point (xyz, f32) and the receiver's vertex-precision epsilon (f32). Word 1's spare w carries the slot (29 bits), `thin` and the front-face bits. Every reader builds `Receiver` and a thin `SurfaceInput` from words 1 and 2 plus the pixel's camera ray, without touching the hit image or vertex buffers: shading, the GI lookup, local-light visibility and the sun pass. The footprint (`world_dx` / `world_dy`) is the neighbouring pixels' rays against the plane (trace point, geometric normal).

A separate decode pass writing a G-buffer before the material pass would cost more. The material pass needs the whole decode anyway, for `surface`. A decode pass would have to write uv, gradients, tangent frame, colour and normal (60 bytes or more) for one reader, and the reads are what cost here (0.13-0.49 ms per record in shading), not the stores.

Before shading can drop its decode, the vertex attributes it still reads must come from the record or from `surface`'s output. `readlean` (attributes constant) changes Showcase by up to 50 codes, so something after `surface` reads uv, colour, the interpolated normal or decals. That audit comes first.

Expected effect: shading saves its decode, about 0.75 ms (position side plus attributes), minus 16 more bytes read. That would turn Showcase's +0.70 ms net (7.37 against 7.07 + 1.00) to about even, and Glint's +0.39 to about -0.35. Each of the GI, light-visibility and sun passes then saves its own decode.
