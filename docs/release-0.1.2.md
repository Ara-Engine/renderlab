# Renderlab 0.1.2

Released 2026-11-10. This release takes the GPU-layer speed measured in [gpu-layer-bench.md](gpu-layer-bench.md). Output is byte-identical to [0.1.1](release-0.1.1.md), and everything from 0.1.1's notes still applies, including the known gaps. As before, it ships source only.

## Changes

- **Engine shaders are created without wgpu's runtime checks.** All engine shader modules go through `create_shader_module_trusted` with `ShaderRuntimeChecks::unchecked()`. Arbitrary user program sources stay checked, as do transparent program compositions and background glass compilation. A custom low-level prelude switches all later compilation to checked mode.
- **Indirect-dispatch validation is off.** `engine::instance_flags()` turns it off for the engine's bounded indirect arguments, and both the headless adapter and the viewer use it. Callers that supply their own adapter must create their instance with these flags themselves.
- **New setting:** `EngineDescriptor::checked_engine_shaders` (default `false`) brings back the checks for development; an environment override exists too ([api.md](api.md#shader-runtime-checks-p-free)). It is the only API addition, and code using `..Default::default()` needs no change.
- **Workgroup zero-initialisation stays on.** Skipping it gave no gain on RT, and the non-RT attempts timed out.

## Measured

Taken from [p-free.md](p-free.md): RTX 4070 SUPER, NVIDIA 617.14, 1080p, ARA1 timeline. The build was a development build (renderlab at opt-level 1), so these are matched GPU spans, not release CPU throughput. All 96 paired HDR and PNG frames are byte-identical, with no NaN or Inf. The frames were also inspected still, moving and on cuts.

| Scene | Path | Still ms | Moving ms |
|---|---|---|---|
| game | non-RT | 6.96 → 6.21 (−10.7 %) | 10.18 → 8.67 (−14.9 %) |
| game | RT | 3.02 → 2.74 (−9.2 %) | 4.52 → 4.30 (−4.9 %) |
| PBR | non-RT | 4.75 → 4.22 (−11.2 %) | 4.49 → 3.80 (−15.4 %) |
| PBR | RT | 2.30 → 2.16 (−6.3 %) | 3.06 → 2.81 (−8.0 %) |
| quarry daylight | non-RT | 9.65 → 9.29 (−3.7 %) | 10.04 → 9.85 (−1.8 %) |
| quarry daylight | RT | 3.40 → 3.26 (−4.0 %) | 3.33 → 3.25 (−2.6 %) |

First-cut frames vary widely from run to run. In one pass, RT first cuts were slower on two scenes. A targeted repeat on game RT reversed that, with the median going from 80.7 to 68.0 ms. No uniform first-cut gain is claimed.

## Clean-clone check

[CLEAN-CLONE]
