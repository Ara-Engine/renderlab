# Material pass and the material record

`RENDERLAB_MATERIAL_PASS=1` (off by default) splits primary shading in two:
- **The material pass** decodes each pixel's hit, runs the program's `surface` and writes the material record.
- **Shading** rebuilds `Surface` from the record instead of running `surface` again.

A program opts in by declaring `material_out` / `material_in` (`MaterialRecord`, shaders/program_api.wgsl); the standard program does. A program without them still gets a receiver-only record: model 255, the interpolated normal as its shading normal, material fields zero, and its `surface` does not run. Its shading keeps the single kernel. The port logs how many program slots shade from the record and how many keep the single kernel.

The code is in `src/program.rs`, section "Material pass" (`material_variant`, `MATERIAL_PACKING`). The port's dispatch is in `src/backend_ara/forward.rs` (`material_records`); the wgpu wiring is Renderlab #98.

## Record (48 bytes a pixel, three rgba32uint images, group 0 bindings 75, 78 and 74)

| word | x | y | z | w |
|---|---|---|---|---|
| 0 (75) | shading normal, octahedral 2 x unorm16 | base red, green (unorm16 each) | base blue (unorm16), coverage and metallic (unorm8 each) | emission x exposure, rgb9e5 |
| 1 (78) | the receiver's geometric normal (`Receiver::geometric`), octahedral 2 x unorm16 | model (8 bits), MQ2 lobe alpha (unorm8), roughness (unorm16, rounded up) | MQ2 view share (unorm16), TR1 view hint (f16, `tr1_view_hint`) | slot (bits 0-27), near a triangle edge (28), `thin` (29), source-front (30), lit-front (31) |
| 2 (74) | trace point x (f32) | trace point y | trace point z | epsilon as an f32 exponent (bits 0-7; 0 = the 1e-5 floor), interpolated vertex normal (octahedral 2 x 12, bits 8-31) |

- **Memory:** 100 MB at 1080p, on top of the 16-byte hit image, which visibility, the material pass, the standard program's shading and TR1/ids still read. The GI pass's 48-byte target can keep only its lookup result once it reads the receiver from words 1 and 2.
- **Model 255:** a pixel whose program has no mapping (it keeps its own `shade`).
- **Roughness is rounded up,** so the decoded value is never below the evaluated one. A value authored at a threshold, such as `LocalShadows::unshadowed_hits_from`, stays on its side of it.
- **The first record (24 bytes)** had a 10-bit base colour and nearest-rounded roughness. It failed three port cases with the pass on (Linux Radeon; two also on the M5):
  - the 10-bit base moved every pixel of a flat floor by one f16 step;
  - it turned a texture's sRGB decode difference of a hair into a whole step;
  - roughness at the threshold decoded just below it.
- **No compact record is bit-exact.** Even an f32 octahedral normal does not round-trip bit for bit, so some pixels still move by one f16 step with the pass on. `traced_frame_agrees_with_the_mesh_pass_over_misses_sky_and_post` counts only differences past one step while the pass is on.

### The receiver (words 1 and 2)

The receiver is what the sun, GI and light-visibility passes and shading read instead of decoding the hit: `rl_mr_receiver` gives `Receiver`, `rl_mr_vertex_normal` the interpolated normal, `rl_mr_near` the edge mark. `RENDERLAB_MATERIAL_DEBUG=receiver` (the port case `the_record_rebuilds_the_receiver`) holds it against the sun path's receiver for a fresh camera ray, edge repair included, on the M5:
- **Exact:** on all 16,137 hits, toon and standard, the trace point, slot, `thin`, both sides and the epsilon are bit for bit wherever the repair leaves the hit.
- **Geometric normal:** within 3.2e-5 rad.
- **Interpolated normal (12 bits):** within 8.1e-4 rad (0.047 degrees). Today shading's GI and ES lookups still bin by the decoded normal; the GI pass will read this one, so a cell boundary within 0.047 degrees of a normal can flip which cell it reads.
- **Repaired edges:** 2 pixels in the case are moved by the repair. The record carries shading's barycentric point there and the near-edge mark, so a reader that needs the sun path's point repairs those hits itself (`MATERIAL_TRACE_RECEIVER` is the sun path's `primary_penumbra_receiver`, body for body). The writer does not run the repair: its ray-query code made the pass 0.56 ms (Showcase) and 0.74 ms (Glint) slower on the M5 for a handful of pixels. The sun pass already carries that code.

Order in an interactive port frame: trace (raw hit; near-edge pixels queued) → `primary_repair_interactive` (repaired hit record) → material pass → shading. **The port keeps the camera ray and the sun's rays together in the trace kernel,** which has the hit in hand, so a separate sun kernel would buy nothing there by itself. The record feeds GI, the lights and shading. The desktop runs visibility → material → sun → GI → light visibility → shading (#98).

**Misses:** with the pass on, the camera-hit trace kernels write a miss sentinel on every pixel without a hit: word 1.w all ones, so the slot reads `0x0fffffff` (`rl_mr_miss`, from word 1 alone). This covers the region's exterior too: the trace dispatches over the whole image, so the fast-clear blocks are written in the same kernel. The stores cost nothing measurable in the trace kernel.

The port case `misses_carry_the_sentinel_after_the_camera_moves` (a wall, then the camera turns past its top) asserts it, under both producers:
- the 110 misses shading visits, the pixels the wall covered a frame earlier among them;
- the 11,328 pixels shading never visits (fast-cleared tiles of sky), read back from word 1 (`ForwardImages::material_receiver`, filled only under the receiver check);
- no hit carries it.

**What word 2's point means depends on the producer.** Readers follow one rule:

| producer | word 2's point | near-edge mark (word 1, bit 28) |
|---|---|---|
| the material pass's writer (#107) | shading's barycentric point | set where the hit is near an edge; there the sun path's repair may move the point |
| the trace tail (`tracetail`, the port's plan) | the sun path's point: the repair kernel rewrites the hits it moves | never set |

The rule: **a marked hit's point is shading's.** A reader that needs the sun path's point (the sun pass) repairs a marked hit from the hit image, as `primary_penumbra_receiver` does. Every other reader uses word 2 as it is. Unmarked points are never repaired, so a repaired point is never repaired again; the trace tail marks nothing. GI, the lights and shading may read word 2 as is under either producer; the sun pass must honour the mark.

### What shading still reads after `surface` (the audit)

- **The specular filter** (`specular_brdf`, U1b method 7, per light): the interpolated normal and its derivatives, the tangent frame, `source_front`, the footprint and, for normal-mapped materials, uv and its gradients with up to 256 normal-map samples. It is light-dependent, so the standard program's shading keeps its vertex attribute fetch. That is what the standard path keeps.
- **`material_normal` per light inside the filter:** taken from the record's shading normal (`rl_material_use`, set after the reflection hits are shaded, which use their own surfaces).
- **GI and ES lookups (`gi_prepare` / `es_prepare`):** the interpolated normal. They ride on shading's attribute fetch today; the GI pass reads word 2.
- **`rl_reflection_prepare`:** the interpolated normal and its derivatives (lobe curvature). They ride on the filter's fetch.
- **MQ2's `tr1_view_hint`:** computed in the material pass and stored in word 1.
- **Decals:** only through `material_normal`, so covered by the record.

### What the receiver-only record closes off

A program with its own `shade` cannot change the receiver the sun, GI and light passes see. Point and normals come from geometry and the interpolated normal, not from the program. A program that displaces or bends its shading normal gets that only in its own `shade`. If a program later needs a custom receiver normal, that is a material output, not shader code. Custom `shade` stays supported.

## Cost (M5, Ara main, Showcase at EV 10 / Glint goal at EV 13.5, medians of 3 interleaved rounds)

`RENDERLAB_MATERIAL_PROBE` (measurement only) splits the cost. All figures are ms.

| | Showcase, 32 B | Glint, 32 B | Showcase, 48 B | Glint, 48 B |
|---|---|---|---|---|
| shading, pass off | 7.37 | 6.71 | 7.37 | 6.68 |
| shading, pass on | 7.07 | 6.19 | 6.89 | 5.99 |
| material pass | 1.00 | 0.91 | 1.20 | 1.11 |
| └ launch and list walk (`empty`) | < 0.10 | 0.11 | < 0.10 | 0.11 |
| └ hit decode (`decode` - `empty`) | 0.72-0.75 | 0.69 | 0.72-0.75 | 0.69 |
| └ `surface`, packing, near-edge test (`nostore` - `decode`) | 0.16 | 0.10 | 0.23 | 0.16 |
| └ store (full - `nostore`) | 0.01 | 0.00 | 0.15 | 0.15 |
| record load and unpack in shading (`loadplus` - off) | 0.49 | 0.13 | 0.24 | 0.03 |
| vertex attributes still read by shading (on - `readlean`) | 0.25 | 0.28 | | |
| position side of the decode, alone (`readpos`, includes the list walk) | 0.63 | 0.60 | | |

What these show:
- **The writer is decode-bound on the M5:** about 0.7 ms of 1.1-1.2, against 0.1 for launch and list walk. On the RTX (#98), 0.20 of its 0.25 ms is launch and list.
- **The third word's store is 0.15 ms;** the first two words stored for about nothing.
- **Shading saved 0.19 / 0.21 ms** from taking the per-light `material_normal` and MQ2's hint from the record.
- **Net per frame:** Showcase +0.72 ms (7.37 against 6.89 + 1.20), Glint +0.42 (6.68 against 5.99 + 1.11). The 32-byte record was +0.70 / +0.39, so the extra 16 bytes cost the frame about nothing.
- **The pass pays once the passes after it** (sun, GI, light visibility) **stop decoding hits:** each of them saves its own decode.

## Decode-free shading does not pay on the M5 (not built)

What standard shading would need beyond words 0-2 to shade with no hit decode:
- the vertex normal's two derivatives (6 x f16), for the specular filter's quadrature and reflection curvature;
- for normal-mapped materials, the tangent (octahedral + handedness), uv (2 x f32) and its gradients (4 x f16), for the quadrature's normal-map samples.

That is 28-32 more bytes; words 0-2 have no spare bits. The upper bound, `RENDERLAB_MATERIAL_PROBE=nodecode` (`SurfaceInput` from words 1 and 2 alone; derivatives, tangent and uv zero):

| | Showcase | Glint |
|---|---|---|
| shading, full reader | 6.92 | 5.98 |
| shading, `nodecode` (best case) | 6.37 (-0.55) | 5.46 (-0.52) |
| net with the writer | +0.18 | -0.12 |
| picture against the full reader | max 50 codes, 7,340 px over 1 | max 20, 161 px over 1 |

The writer's own decode (about 0.7 ms) is a floor that shading can give back at most about 0.5 of, and the exact reader needs the extra words (0.15 ms per stored word, plus its read). So the fourth word is not built.

## The port writes the receiver from the trace

The trace kernels already have the hit in hand, with the receiver (point, geometric normal, epsilon, slot, `thin`) and the near-edge test. `RENDERLAB_MATERIAL_PROBE=tracetail` has them write words 1 and 2 as their tail:
- **the interpolated normal:** the only addition, one more vertex load per vertex plus the normal transform;
- **both sides:** the shading entry's lit front reduces to its source front;
- **edge repair:** the repair kernel rewrites the words for the hits it moves, so the record holds the sun path's repaired point;
- **material:** no word 0 and no separate writer kernel. Shading keeps its single kernel, decoding once as before.

| | Showcase | Glint |
|---|---|---|
| trace kernel, pass off | 2.74 | 3.03 |
| trace kernel, `tracetail` | 2.99 (+0.25) | 3.23 (+0.20) |
| shading | 7.37, as off | 6.68, as off |
| net | +0.25 | +0.20 |
| picture against the pass off | byte-identical | byte-identical |

The separate writer path is +0.72 / +0.42. On the port the receiver for the later passes costs a quarter to a half of that from the trace. Custom-program pixels get it the same way.

## Decision: the material pass is the decode-once pass

A separate decode pass writing a G-buffer before the material pass would cost more. The material pass needs the whole decode anyway, for `surface`. A decode pass would have to write uv, gradients, tangent frame, colour and normal (60 bytes or more) for one reader, and reads, not stores, are what cost.

Words 1 and 2 carry the receiver for every hit, whatever its program. A pass reading them runs one path for all pixels and never decodes a hit; only shading branches (model 255: the program's own shading). The rejected alternative, a hit-decode fallback inside each reader, would put the decode back into every pass.
