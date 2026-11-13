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
4. A skipped light either adds the c computed for the test, which is exactly the value the loop adds at visibility 1 (`RENDERLAB_LIGHT_BOUND_SKIP=unshadowed`, the default), or adds nothing (`drop`). Either way, its error is at most its c, so a receiver's error from all its skipped lights is under `codes` display codes.

`slope` is the derivative of Ara's display transform: the Filmic curve (Narkowicz's ACES fit, `Tonemapper::Filmic`), then sRGB, in display units per exposed unit.

- `black` (order-free): the transform's steepest slope, 6.38 at x ≈ 0.0087. The slope at zero (2.77) would be 2.3× too loose on dark-mid pixels, because a skipped light adds to whatever value the receiver ends at.
- `sum` (order-dependent): the steepest slope at or above the loop's running sum. The receiver's final value is never lower than the running sum. Below the peak the slope still rises, so the peak is used there; above it the slope only falls (checked numerically), so it is the slope at the running sum. The shader uses a bound without `pow`: `min(6.443, 0.2888/x, 0.1082/x², 0.7648/x³)`, fitted with a 1% margin. It never understates the slope and overstates it by at most 1.6× (1.0–1.13× above x = 0.18). Looser only skips less.

`RENDERLAB_LIGHT_BOUND_CODES` sets the size (default 0.5; 0.25 for the tighter row).

## What the bound leaves out

The bound covers the tone curve and sRGB only. White balance (`white.rgb`, which scales before the curve), the contrast power, saturation, a colour LUT, bloom and vignette are not in it. **A grade applied after the tone curve can steepen the slope.** A scene with a strong contrast grade or LUT needs the bound re-derived through that grade, or a smaller size. TR1 can also accumulate a sub-code bias coherently over frames, and bloom spreads it. That is why every row reports its largest per-pixel difference against the full picture, and where any pixel exceeds one code.

## Failure to look for

In `sum` mode, a light whose c sits near the bound can be skipped on one frame and traced on the next, as the running sum moves. A skipped shadow would then flicker in and out. The 30-frame moving strip compares frame-to-frame change against the full picture's for exactly this. `black` mode does not depend on order.

## Measured on the M5 port (2026-11-13)

Ara b50edafbd, 1280x720, 32 lanes, Ara's default frame, `PrimaryShadeList` ms, medians of 3 interleaved rounds. "Skipped" is the share of local shadowed lights tested whose ray was skipped (frame 48). The difference columns compare against the closest of three full captures, in display codes.

| | Showcase ms | skipped | max / mean diff, pixels > 1 code | Glint goal ms | skipped | max / mean diff, pixels > 1 code |
|---|---|---|---|---|---|---|
| full | 7.74 | — | — | 6.72 | — | — |
| black 0.5 | 7.66 | 84% | 1 / 0.004, 0 | 6.78 | 23% | 1 / 0.000, 0 |
| sum 0.5 | 7.82 | 96% | 1 / 0.007, 0 | **5.89** | 99.9% | 1 / 0.001, 0 |
| black 0.25 | 7.78 | 69% | 1 / 0.002, 0 | 6.83 | 16% | 0 |
| black 0.5, drop | 7.51 | 84% | 1 / 0.034, 0 | 6.76 | 23% | 0 |

In 30-frame, 6° yaw strips, every frame of `black` and `sum` stays within 1 code of the full frame, on both scenes. Two full strips agree exactly on Showcase, and within 1 code with no flags on Glint. Some pixels' frame-to-frame change differs from full's by 2 codes: Showcase `black` 694 distinct pixels, `sum` 853; Glint `sum` 73, `black` 0. Each pixel is flagged about once over the clip (one pixel three times), so these are scattered rounding flips of a sub-code bias as the view moves, not a shadow toggling in and out.

Skipping most of Showcase's local rays saves little on the M5. The test computes each light's c (BRDF and transmission) before its ray, and a lane that skips still waits for the lanes in its SIMD group that trace. Glint's `sum` row is where the bound pays.
