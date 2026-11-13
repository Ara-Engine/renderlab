# Light bound: skipping shadow rays that cannot change a display code

Status: a measurement switch, off by default (`RENDERLAB_LIGHT_BOUND`, program.rs `light_bound`). It is a bounded picture change, not exact, so it goes to Islam as a crop pair before it can become a default.

## Why

On the M5 port at 32 lanes (Ara b50edafbd, reflections ablated, frame 48), most local shadowed lights tested from a camera receiver contribute less than half a display code before their shadow ray even runs:

| scene | local lights tested | under ½ code at black | under ½ code at the running sum |
|---|---|---|---|
| Showcase | 461k | 72.6% | 98.0% |
| Glint goal | 96k | 17.0% | 97.8% |

Those counts used sRGB alone (no tone curve). Shadows cost 0.95 ms (Showcase) and 1.27 ms (Glint) of the shading pass there.

## The rule

In the standard program's light loop, the bound runs per receiver (`shade` call), per channel, and only for local shadowed lights (`kind.w != 0`):

1. Let c = the light's unshadowed exposed contribution: intensity × transmission × BRDF × attenuation × exposure.
2. Let `total` = c plus the contributions of lights already skipped at this receiver.
3. If `total × slope < codes / 255` in every channel, the light's shadow ray is skipped and `total` is kept.
4. A skipped light is either added unshadowed (`RENDERLAB_LIGHT_BOUND_SKIP=unshadowed`, the default) or dropped (`drop`). Either way, its error is at most its c, so a receiver's error from all its skipped lights is under `codes` display codes.

`slope` is the derivative of Ara's display transform: the Filmic curve (Narkowicz's ACES fit, `Tonemapper::Filmic`), then sRGB, in display units per exposed unit.

- `black` (order-free): the transform's steepest slope, 6.38 at x ≈ 0.0087. The slope at zero (2.77) would be 2.3× too loose on dark-mid pixels, because a skipped light adds to whatever value the receiver ends at.
- `sum` (order-dependent): the steepest slope at or above the loop's running sum. The receiver's final value is never lower than the running sum. Below the peak the slope still rises, so the peak is used there; above it the slope only falls (checked numerically), so it is the slope at the running sum.

`RENDERLAB_LIGHT_BOUND_CODES` sets the size (default 0.5; 0.25 for the tighter row).

## What the bound leaves out

The bound covers the tone curve and sRGB only. White balance (`white.rgb`, which scales before the curve), the contrast power, saturation, a colour LUT, bloom and vignette are not in it. **A grade applied after the tone curve can steepen the slope.** A scene with a strong contrast grade or LUT needs the bound re-derived through that grade, or a smaller size. TR1 can also accumulate a sub-code bias coherently over frames, and bloom spreads it. That is why every row reports its largest per-pixel difference against the full picture, and where any pixel exceeds one code.

## Failure to look for

In `sum` mode, a light whose c sits near the bound can be skipped on one frame and traced on the next, as the running sum moves. A skipped shadow would then flicker in and out. The 30-frame moving strip compares frame-to-frame change against the full picture's for exactly this. `black` mode does not depend on order.
