# MQ2's view hint: what it repeats, and how to stop paying twice

Status: proposal (2026-11-13). Cost measured on the M5 port, Ara b50edafbd, 1280x720, `RENDERLAB_LIST_LANES=32`, reflections ablated. `RENDERLAB_SHADE_CUT=mq2` saves 0.33 ms (Showcase) and 0.50 ms (Glint goal) of the list shading pass's 5.84 / 6.49. Those bins predate the camera-ray hoist (`t3/view-ray-hoist`).

## 1. What the hint repeats

`tr1_view_hint` (shaders/temporal_view.wgsl) calls the program's `view_dependence(s, input)`, then adds the engine's own terms. In the standard program, `view_dependence` recomputes values that `shade` (and `reflection_lobe`, when reflections run) compute with the same arguments:

| value | `view_dependence` | `shade` | `reflection_lobe` | shareable |
|---|---|---|---|---|
| `v = unit_normal(eye - world)`, the `s.n` fallback | yes | yes | yes | yes (ALU) |
| `environment_dfg(dot(s.n, v), s.rough)`: a texture sample | yes | yes | yes | **yes, same coordinates** |
| `f0` from `s.base` | from `s.base` | from `base = select(s.base*s.alpha, s.base, film)` | from the alpha base | no (they differ on transparent, non-film surfaces) |
| specular and diffuse albedo dots | yes | no | no | hint only |

On the engine's side, the hint's camera ray is what the hoist removed: the entry now computes it once, for both footprints and the hint. The curvature term (`input.normal_dx/dy`) is the hint's alone. The toon program declares no `view_dependence`, so it gets the stub (hint 1) and pays nothing today.

## 2. The proposal, smallest first

**(a) No interface change: the traced API's `environment_dfg` memoizes its last call.** It keeps the last `(n_dot_v, roughness)` and result in private variables. A call with bit-identical arguments returns the kept value without sampling. Every program gets this without doing anything. The standard program's up to three samples a pixel become one. A user program writes exactly what it writes today. The cost is two compares, a branch and four registers.

The view vector is plain ALU, which GPU compilers already share after inlining. It is not worth an interface.

**(b) Only if (a) leaves most of the 0.33 / 0.50:** an optional report, in the pattern `reflection_report` already sets. The engine provides `view_report(d: ViewDependence)`. A program may call it from `shade` with values computed there. When it is called, the hint uses the reported value and does not call `view_dependence`. A program that never calls it keeps today's path and today's cost, so nothing becomes harder to write. For the standard program to stay exact, `shade` computes the report from `s.base` (not its alpha base) with its own `v` and `ab`, which is the same arithmetic as `view_dependence`.

**(c) Noted, not proposed for timing:** TR1 reads the hint in exactly one place, `drift = s.view * tr1_view_turn(s)` (temporal_resolve.wgsl). `tr1_view_turn` is zero for a still camera and still surfaces, so on such frames the hint changes nothing, and a frame flag could store 1 instead. This helps only frames nothing moves in. Games rarely have those, but our still-frame captures do, so (c) would flatter every still measurement. It should be judged only on moving clips.

## 3. Exactness and the expected saving

- **(a):** the memo returns the result of a sample at bit-identical coordinates, which `textureSampleLevel` returns deterministically. The picture is byte-identical. It will be checked like the hoist: the Showcase and Glint stills and yaw clips against main's own output set, where Showcase's one-pixel flip at (55,400) is main's.
- **(b):** the same expressions on the same values, so byte-identical by the same argument.
- **Expected saving:** the hint's DFG sample and its duplicated ALU, a share of the 0.33 / 0.50 that only a row can give. The plan for the next build window, at 32 lanes, interleaved: full, the hoist, hoist plus memo, and the `mq2` cut as the floor of what any change can save. Then (b) only if the gap between memo and `mq2` stays large.
