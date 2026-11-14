# GI receiver pass defaults

`RENDERLAB_GI_PASS` follows the reflection pass's per-renderer default pattern:

| Value | Decision |
|---|---|
| Absent or empty | This renderer's path default |
| `0` | Inline receiver lookup |
| `1` | Separate receiver pass |

Other values also follow the path default. The environment is read once per process.
`RT_DEFAULT` and `NON_RT_DEFAULT` in `src/gi_receiver_pass.rs` are both false.
They can be changed independently after the game-scene picture and memory checks.
The path uses the device's ray-query feature, as the reflection pass does.

Only enabled renderers allocate the prepared-irradiance buffer and dispatch arguments.
The same resolved decision gates the shading binding, pipeline preparation and dispatch.
A renderer whose default is on marks its own shading heads; composed programs read that
mark unless explicitly overridden. Unmarked heads retain inline lookup by default.
This keeps defaults local to a renderer, including when multiple devices coexist.

The record remains 48 bytes per pixel at binding 68: separate f32 sky/bounce scales,
27 sky L2 and 12 bounce L1 snorm8 coefficients, and one padding byte. A negative sky
scale is invalid. Neither the format nor quantization changes here. The normal and
position remain reconstructed by the existing receiver helpers. At 1920x1080 the
prepared buffer is 99,532,800 bytes, allocated only when the decision is enabled.

The port keeps its existing explicit-switch refusal and default off. It does not
inherit the wgpu path constants. Programmable shading and secondary-hit inline lookup
retain their existing eligibility rules.
