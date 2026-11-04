# M6 Interactive soft-shadow cost hillclimb

2026-11-05 local (2026-11-04 UTC), based on `3b7e05c`. One measured mechanism at a time on the traced quarry's [Interactive](m6-level-b-implementation.md) frame. Every retained step reproduces the gated Interactive output byte for byte, and Exact and hard output are unchanged, so the Level-B gate results carry over without a rescore. **The 2x HRT goal is met (span 0.861–0.862 ms against ≤1.075). The 2x VSM stretch (≤0.80 ms) misses by 0.062–0.063 ms.** No push.

## Result

Complete frames: RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, release test binary, validation off. The registered quarry is 12 full-resident draws (13,034,903 triangles) at 1920×1080 1x with a 0.04 rad disk. Each process runs 24 warmup triplets, then 24 paired triplets in rotating hard/Exact/Interactive order on alternating registered poses, pipelined with no per-frame drain. Cells are medians [min, max] in ms.

| Pass | Baseline (b5, processes 1 / 2 / 3) | Final c11, process 1 | Process 2 | Process 3 |
|---|---|---:|---:|---:|
| Trace | 0.3232 / 0.3177 / 0.3227 | fused: 0.4007 [0.3962, 0.4056] | 0.4001 [0.3967, 0.4062] | 0.4018 [0.3959, 0.4057] |
| 8-ray visibility | 0.3593 / 0.3573 / 0.3596 | (in the fused pass) | | |
| Shading | 0.3976 / 0.3907 / 0.3964 | 0.3960 [0.3911, 0.4139] | 0.3959 [0.3903, 0.4209] | 0.3950 [0.3916, 0.4088] |
| Tone | 0.0496 / 0.0492 / 0.0491 | 0.0492 [0.0451, 0.0532] | 0.0492 [0.0461, 0.0543] | 0.0492 [0.0471, 0.0543] |
| **Interactive span** | **1.1469 / 1.1308 / 1.1420** | **0.8615 [0.8462, 0.8716]** | **0.8614 [0.8442, 0.8838]** | **0.8606 [0.8497, 0.8696]** |
| Paired Interactive/hard span | 1.1981 / 1.1827 / 1.1934 | 0.9006 | 0.9015 | 0.9002 |
| Hard span (unchanged path) | 0.9605 / 0.9530 / 0.9597 | 0.9560 | 0.9574 | 0.9557 |
| Exact span (unchanged path) | 9.5606 / 10.3047 / 8.8096 | 8.3927 | 8.3961 | 8.3906 |

Trace plus visibility drops from 0.675–0.683 ms to 0.400–0.402 ms. The span drops about 24%, and an Interactive soft frame now costs 0.90 of the hard frame. All three c11 processes were in the fast GPU state (Exact visibility 7.61 ms). The c10 processes, before the final CPU-only gating edit, measured 0.8617–0.8627 ms. The paired hard ratio is the state-independent figure.

Against the stored [native Unreal timings](unreal-native-shadow-mask.md) (UE 5.7.4, D3D12, same quarry subset, 1x/no AA, not relaunched or paired contemporaneously): **2.50x HRT** (2.14975 ms) and **1.85–1.86x VSM** (1.59650 ms). The 2x VSM floor is 0.798 ms, about 7% below the frame. This covers the bounded direct-light/shadow subset, not whole-engine parity.

**Remaining limiter.** The fused pass is mostly closest-hit camera traversal. Diagnostics before steps 2–4 measured 0.43 ms in total: 0.07 ms fixed, about 0.23 ms primary traversal and about 0.12 ms shadow traversal. Shading of covered pixels is bound by the vertex-attribute gather; the [earlier probe](m6-primary-ray-production.md#hillclimb-one-mechanism-at-a-time) found the lit body itself nearly free. Tone mapping and the HDR background clear are full-resolution bandwidth.

## Retained steps

Each step was screened in-process: paired, rotating dispatch sequences over the traced pose-0 frame (`interactive_climb_tests::gpu_interactive_climb_screen`), with 96 warmup and 48 timed repetitions per process. Sequences are timed from first pass begin to last pass end. Values are per-process medians in ms.

| Step | Mechanism | Paired screen (candidate vs predecessor) | Frame span | Limiter named next |
|---|---|---|---:|---|
| 1 (c3) | **Fuse 8-ray visibility into the camera-ray dispatch.** `primary_trace_interactive` traces each pixel's closest hit, then its 8 directions from the receiver in registers. There is no receiver target round trip and no second dispatch. | Trace+visibility 0.429–0.443 vs 0.663–0.678; paired 0.642–0.654 (3 processes) | 0.927–0.930 | Shading after the fused pass is 0.036 ms slower than at submission start. Diagnostics locate this in the miss writes: skipping miss hit records saves 0.046 ms of fused→shade, and miss counts save 0.010 |
| 2 (c7) | **Per-tile coverage flags.** Each 8×4 trace workgroup ORs its hits into a workgroup atomic and writes one flag. Tiles without a hit skip hit-record writes, and Interactive shading (`primary_shade_tiles`) clears them without reading records | fused→shade 0.832–0.834 vs 0.881–0.883 (c5 kernel, 3 processes) | 0.893–0.895 | Camera rays over empty background |
| 3 (c8) | **Scene pixel rectangle.** The CPU projects every placement's conservative world bounds (`shadow_gpu::world_bounds`) and widens the bounding rectangle by 2 px; pixels outside it are misses without a ray. Bounds reaching behind the eye give the whole image | fused 0.396–0.397 vs 0.421; fused→shade 0.810–0.813 vs 0.831 (3 processes) | — | Count writes in uncovered tiles |
| 4 (c9) | **Uncovered tiles publish only their flag**, with no counts | fused→shade 0.796–0.797 vs 0.808–0.810 (3 processes) | 0.860–0.862 | Primary traversal, shading gather, full-resolution clear and tone (above) |
| 5 | **Traced frames allocate no raster depth target** | memory only: −8.29 MB at 1080p; tile flags add 0.26 MB | — | — |

On the quarry, 11,065 of 64,800 tiles are covered; the background pixels lie outside x 581–1611, y 274–802.

## Rejected attempts

| Attempt | Result |
|---|---|
| Fused workgroup shapes 4×8, 8×8, 16×2, 16×4, 4×4 | Exact. 0.458–0.460, 0.583–0.585, 0.450, 0.589–0.590 and 0.433, against 0.430 for 8×4 (2 processes) |
| Disabling naga runtime checks: ray-query tracking only, or all checks (unsafe trusted module) | Exact but slower: 0.449 against 0.430. It would also remove safety checks from user-edited composed WGSL |
| Writing the hit record after the shadow loop, so it is newer in L2 for shading | Exact. Fused 0.447–0.448 vs 0.430–0.431; fused→shade 0.885–0.886 vs 0.868–0.871 |
| Shading workgroups after fusion: 8×4, 4×8, 2×8, 8×2 | Exact. Fused→shade 0.896–0.899, 0.893–0.894, 0.871–0.874 and 0.871, against 0.868–0.871 for 4×4 |
| Skipping HDR clear stores for misses | Diagnostic: −0.028 ms, but the HDR target is public (`try_capture_hdr`); not adopted. Reusing last frame's clear would be history |
| `RAY_FLAG_SKIP_AABBS`; cull flags | Rejected by analysis. naga 30 never declares `RayTraversalPrimitiveCulling` and wgpu-hal does not enable it. Face culling changes double-sided semantics. Opaque-cull flags conflict with `FORCE_OPAQUE`. First-hit termination is already active (`tmin > 0`) |
| Tone mapping fused into shading | Rejected by analysis. Caller targets are `Rgba8UnormSrgb` render attachments without storage usage, and the present transform lives in user-editable scene WGSL |
| Receiver ordering | Covered by the shape screen: one 8×4 tile is one warp, so lane order inside it cannot change SIMT coherence. Dispatch-order swizzles were 0.99–1.01 in the [fresh compute climb](m6-fresh-compute-cost.md) |
| Subgroup `any` instead of the workgroup atomic | Not attempted: it needs the `SUBGROUP` device feature, and the atomic and barrier cost about 0.006 ms (flags with full writes 0.439 vs 0.433) |

## Equivalence and inspection

With active Khronos validation (errors=0, active_layer=1):

- Traced Interactive acceptance, final c11 (and c9, c10): 36 of 36 outputs are byte-identical to the gated checkpoint `accept-b7`. These are Exact and hard HDR, receivers, and Interactive HDR and counts at 3 poses × seeds 0–3. Static repeat, return, reload (invalid WGSL, seam drift, tint and restore) and raster fallback assertions pass.
- Quarry mask replay (c9): 66 of 66 files are identical to `quarry-b6`.
- Fixture replays, final c11 (and c9, c10): thin and stress are each 575 of 575 identical to b8. That includes the public Interactive frames at 5 poses × 16 seeds and the removed-blocker check.
- Primary traced acceptance (c9): 285 of 285 files are identical to `accept-b12`, so hard, Exact and raster are unchanged. c11 only gates the CPU rectangle to Interactive frames.
- Single-sided/mirrored traced fixture passes (c9, c11). The traced lit suites give the same 11 of 18 as before; the same 7 cannot run on a traced renderer, for the reasons recorded in [primary-ray production](m6-primary-ray-production.md).
- Inspected (from c10) `pose-0-interactive-16x.png`, `pose-1-interactive-1x.png` and 2× crops at the seven frozen contact/self-shadow pixels. The maximum HDR difference against the gated frame is 0.

Interactive hit records and counts are now complete only in covered tiles. Tests compare them through `interactive_outputs`, which fills uncovered tiles with their miss values (a zero record and count 256). The HDR image stays complete. Interactive frames no longer publish fresh world/normal receivers; the fixture replay renders an Exact frame of the same pose for its receivers.

## Interface changes

- Only Interactive frames compute the scene rectangle on the CPU: one conservative world bound per placement, so O(placements) per frame.
- `traced_limits` now also raises `max_acceleration_structures_per_shader_stage` to 2: the fused pass binds the TLAS with and without vertex return. Camera rays moved to binding 19.
- In Interactive frames, `GpuSample::receiver_pass` times the fused pass and `visibility_compute` is `None`.
- Hard and Exact keep their passes. `primary_shade`'s body became `primary_shade_pixel`, and their outputs are byte-identical.

## Checks and time

The renderer CPU filters `shadow primary soft` pass (37 passed, 27 GPU tests ignored), including the new `scene_pixels_contain_every_reaching_ray`. Release Clippy `--lib --tests -D warnings`, `cargo fmt --check` and `git diff --check` pass.

**Implementation and analysis:** 21:22:48–22:13:37 UTC, 50.8 min elapsed. This interval was interleaved with verification, and editing time was not isolated. Reading and grounding preceded it; this record followed.

**Verification:**

- Eleven release test builds took 414.2 s (c1 cold 96.3 s; the others 28–40 s).
- 67 offscreen GPU child processes took 454.6 s. Eight exited nonzero: the first screen hit the per-stage acceleration-structure limit, which led to the `traced_limits` change, and the 7 known suite cases.
- The shared GPU slot was held in 87 windows, 967.5 s in total, at most 97.7 s each (the cold build); all were released, the last at 22:13:26.201 UTC.
- Nine runs deferred on free RAM below 8 GiB (minimum 4.99 GiB) while another worker's nightly `rustc` held about 3 GB; every accepted preflight had at least 8.41 GiB.
- No game, Unreal or foreign renderlab GPU job was detected. Foreign CPU builds (0–8 cargo/rustc processes) and desktop applications ran during some timed runs, and free RAM dipped to 6.1 GiB after preflight; all samples are retained.
- CPU tests took 0.36 s, Clippy 5.6 + 3.6 s, and a cargo check about 34 s cold then 2–3 s per check. Image generation was not timed.

Evidence is in `D:/Coding/renderlab-work/m6-interactive-climb/`:

- `run.py` and `build.py`: lock, preflight and contention sampling;
- `builds/c1`–`c11`: binaries, SHA-256, HEAD and source diffs. The final c11 is `96d4dcc5…13363`;
- `runs/`: manifests, logs, `screen.json`, `timing.json` and the acceptance and replay outputs;
- `scripts/`: variant generators, `timing.py`, `suite.py` and `images.py`;
- `images/` and `gpu-windows.json`.

Reproduce a screen with `py -3 -I scripts/variants_d7.py`, then `run.py <label> builds/c11/climb-tests.exe renderer::fresh_shadow_tests::primary_ray_gpu_tests::soft_shadow_probe_tests::interactive_climb_tests::gpu_interactive_climb_screen <manifest>`.

## Next boundary

2x VSM needs 0.062–0.063 ms more. The remaining exact levers are small: the workgroup flag atomic, background tone and clear bandwidth, and inter-pass drains. A larger cut needs cheaper closest-hit camera traversal or a smaller vertex-attribute gather, and an attribute-format change alters precision, so it would re-enter the Level-B gate. Tile flags and the scene rectangle also apply to hard and Exact traced frames, but this unit left those paths unchanged.
