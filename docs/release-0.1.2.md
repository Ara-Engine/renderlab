# Renderlab 0.1.2

Released 2026-11-10. This release takes the GPU-layer speed measured in [gpu-layer-bench.md](gpu-layer-bench.md), and it also ships S4 sun shadows on large non-RT cut frames (below). Everything from [0.1.1](release-0.1.1.md)'s notes still applies, including the known gaps.

**How the tag came to include S4.** The tag was meant to mark the shader-check change alone. A shell mistake during tagging committed a pending merge of P-S4C, so tag `v0.1.2` (commit `ba4d282`) contains both changes. Its own copy of this file is incomplete: it still holds the clean-clone placeholder and doesn't mention S4. The tagged commit was then checked from a fresh clone (results below). This copy on main is the corrected record. As before, it ships source only.

## Changes

- **Engine shaders are created without wgpu's runtime checks.** All engine shader modules go through `create_shader_module_trusted` with `ShaderRuntimeChecks::unchecked()`. Arbitrary user program sources stay checked, as do transparent program compositions and background glass compilation. A custom low-level prelude switches all later compilation to checked mode.
- **Indirect-dispatch validation is off.** `engine::instance_flags()` turns it off for the engine's bounded indirect arguments, and both the headless adapter and the viewer use it. Callers that supply their own adapter must create their instance with these flags themselves.
- **New setting:** `EngineDescriptor::checked_engine_shaders` (default `false`) brings back the checks for development; an environment override exists too ([api.md](api.md#shader-runtime-checks-p-free)). It is the only API addition, and code using `..Default::default()` needs no change.
- **Workgroup zero-initialisation stays on.** Skipping it gave no gain on RT, and the non-RT attempts timed out.

## S4 sun shadows on non-RT cut frames

Measured in [p-s4c.md](p-s4c.md) and [p-s4b.md](p-s4b.md).

- On camera cuts and history restarts, the non-RT sun-visibility pass can now use S4. S4 gathers occluders through a BVH and tests each sun direction against the triangle's planes with a weight-free half-space test. The renderer chooses between S4 and the old march per frame, from the batch count, the pixel count and the sun disk size. S4 runs only where it was measured to win.
- **1080p game cut:** the sun pass drops from 26.54 to 8.41 ms, and the whole frame from 31.7 to 13.6 ms.
- **Other cases:** 960x540 cuts and the wide-sun 1080p cut keep the march, and their output is byte-identical.
- **Accuracy:** S4 matches RT on 99.994 % of cut pixels; the march matched on 99.988 %. On the 1080p cut, the sun-shadow crop's error against RT drops from 0.00242 to 0.00078. The images were inspected on the cut and the frames after it.
- **Patents:** still, moving and smaller cut frames still use the march, so the VIA patent flag from 0.1.1 stays open. [p-s4c.md](p-s4c.md) lists what S4 needs before the march can be deleted.

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

A fresh `git clone` of tag `v0.1.2` (`ba4d282`) from GitHub was built in its own debug target with no private assets, on 2026-11-10.

| Check | Result |
|---|---|
| `cargo test -p renderlab --lib` | 205 passed, 0 failed, 164 ignored (GPU-only) |
| `game_scene --path rt --frames 48 --every 8` | exit 0; frames inspected |
| `game_scene --path nonrt --frames 48 --every 8` | exit 0; frames inspected, including cold frame 0 |

All three ran in GPU lease `root-v012b-checks-1` on the RTX 4070 SUPER. Before the tag existed, `caff100` (the shader-check change alone) had passed the same check with 200 lib tests in lease `root-v012-checks-2`.
