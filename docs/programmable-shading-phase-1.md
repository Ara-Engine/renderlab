# Programmable shading, phase 1: the standard program through the API

2026-11-05 (00:43–02:10 UTC), based on `af6578c`, implementing phase 1 of [programmable shading](programmable-shading.md). RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, release test binaries, registered full-resident quarry (12 draws, 13,034,903 triangles) at 1920×1080 1x unless noted. No window, viewer, game capture or push.

**Gate result: not passed outright.** Every output set is byte-identical, and the shading-pass and frame-time gates pass. One condition is unmet: the static-repeat check failed in 2 of 13 candidate runs. The baseline failed 8 of 12 in the same alternating sessions (details below), so this is the climb-2 driver hazard resurfacing on main, not a change introduced here. The gate says a byte difference stops the unit, so this is reported for a decision, not argued away. Edit-to-image also regressed on traced-selected renderers (+40%), and the viewer does not yet watch program files (C17).

| Gate | Result |
|---|---|
| Traced primary acceptance (285 files) | 285/285 identical to `primary-accept-c9` |
| Interactive acceptance (36 files) | 36/36 identical in every passing run, and in every file a failing run wrote |
| Quarry / thin / stress replays | 66/66, 575/575, 575/575 identical |
| Single-sided/mirrored fixture | Passes |
| 18 raster lit suites | 18/18 pass; 385/385 test readbacks byte-identical to the baseline build |
| Validation | Active, 0 errors on every acceptance and suite run |
| Static repeat (Interactive acceptance, alternating processes) | **Candidate 11/13, baseline 4/12** (see the hazard section) |
| Shading pass, frozen against program, in-process paired | Soft 0.9955–0.9976 and hard 0.9916–0.9947 per process (gate ≤1.01); outputs byte-identical |
| Interactive frame | 0.8198 / 0.8224 / 0.8155 ms against baseline 0.8162 / 0.8159 / 0.8160 (gate: within 1% of 0.816, ≤0.8242) |
| Raster lit passes | Fresh lighting 1.003× baseline (fast-state processes); hardware lit 0.989× |
| Reload | Invalid and stage-invalid programs rejected with program lines, image kept; edits apply; restores byte-identical (traced, raster, prelude and program) |
| Edit-to-image | Measured; see the regression below |

## What changed

- **Shading programs.** A program is WGSL defining `surface(input: SurfaceInput) -> Surface` and `shade(s, input) -> vec3<f32>`, optionally with `struct Parameters`, read as `params`. The engine composes it as prelude + `shaders/program_api.wgsl` + stage API + parameter binding + program + generated entry (`src/program.rs`).
  - The raster entry is `shaders/program_raster_entry.wgsl`.
  - The traced entry, `shaders/program_traced_entry.wgsl`, holds the hit reconstruction moved from `primary_rays.wgsl`.
  - naga checks the raster composition before any device compile. Rejections name program lines: syntax, types, redefinitions, program-declared bindings and missing `surface`/`shade`. Implicit sampling and `dpdx` in traced shading are located by an IR scan.
- **Standard program.** `shaders/programs/standard.wgsl` is the former `fs_lit` body. Sampling, normal mapping, base, metal and rough are in `surface`; view vector, DFG, environment and light loop are in `shade`. Every expression keeps its original order. `scene.wgsl` is now the engine prelude, without `fs_lit`.
- **API.**
  - `ProgramDescriptor`, `Program`, `Value`, `STANDARD_PROGRAM`.
  - `Renderer::{standard_program, try_create_program, reload_program, try_update_parameters}`.
  - `MaterialDescriptor::{program, parameters}` (None is the standard program).
  - `ReloadReport::zeroed_parameters` lists members that are new or retyped.
  - `PipelineStats::created` counts compiled pipelines.
  - `reload(wgsl)` stays one transaction over the prelude, the engine pipelines and every program.
- **Parameters.**
  - Reflected from naga IR: f32, vec2/3/4<f32>, u32 and i32 members, at most 256 B.
  - Values are checked by name and type; omitted members are zero.
  - Every material owns a 256 B block. Raster reads it at group 2 binding 7; traced shading reads it through a per-draw binding array at binding 17. A reload repacks live materials by name.
  - `try_update_parameters` is a buffer write: the API test asserts zero new pipelines.
- **Seams removed.** `compute_lit` (exact-text rewriting of `fs_lit` into `primary_lit`), the fresh `PIXEL_SEAM`/`GEOMETRIC_SEAM` rewriting and the `textureSample` spelling checks are gone.
  - Every visibility variant now takes an engine-private `Receiver { world, geometric, pixel, slot }` through `hard_shadow(light, receiver, l)`, which `light_visibility` calls. `light_visibility` stays engine-owned and world-space.
  - Two engine-internal seams remain, both in prelude or kernel text rather than any program: the pinned `vs_tangent` line (traced shading reuses it) and the fresh kernel's coverage line.
- **Module split (C7).**
  - Trace and visibility kernels (`primary_trace*`, traced `fresh_compute`, `primary_trace_interactive`) compile from program-free modules (`shader_sources`).
  - Each program owns its pipelines: the raster lit pair, the hardware and fresh pairs, and the traced Hard and Soft shading (`shading_heads`).
  - The hit record is unchanged (placement, primitive, barycentrics).
  - One visible program keeps the single shading dispatch. A traced frame whose draws use more than one program is rejected before any write (mixed dispatch is phase 3). Raster frames switch pipelines per draw in submission order.
- **Device ABI.** `traced_limits` rises from 10 to 11 binding-array elements per draw (the parameter blocks). naga is a dependency (already in `Cargo.lock` through wgpu).

## Determinism hazard (the unmet condition)

The Interactive acceptance was run in alternating candidate and baseline processes. The baseline is `b0t`: `af6578c` plus test-only commits.

| Build | Runs | Pass | Failures |
|---|---:|---:|---|
| Candidate (c1, c4: same production code) | 13 | 11 | Return frame differs (first run); one post-reload frame differs |
| Baseline b0t | 12 | 4 | Static-repeat `static HDR seed=1`, 8 times |

- In the candidate's first failure, poses 0 and 1 were byte-identical in all 24 files. On the return pose, the Exact and every Interactive HDR image differed at the same 13 pixels by 1–5 f16 ulps, while counts, receivers and the hard image were identical. The long-lived soft shading pipeline therefore changed its arithmetic mid-run, the signature recorded in [climb 2](m6-interactive-climb-2.md).
- The baseline now fails more often than it did at climb 2, which passed 16 of 16. The trigger has not been found.
- Two further runs were lost when the shared D: drive filled. They are excluded from the counts; the drive was freed by trimming my own failure logs.

## Timing (validation off)

**Shading pass, in-process paired** (`gpu_primary_kernel_screen`). The frozen baseline module (`af6578c` composition) and the standard-program module run on the production ABI: 96 warmup and 48 timed rotations per process, 3 processes. Medians in ms; the ratio is program/frozen per rotation. In every process the frozen and program shading wrote byte-identical HDR.

| Shading | Program | Frozen | Program/frozen |
|---|---|---|---|
| Soft (Interactive/Exact) | 0.3861, 0.3855, 0.3860 | 0.3930, 0.3900, 0.3914 | 0.9976, 0.9955, 0.9957 |
| Hard (inline rays) | 0.5336, 0.5352, 0.5361 | 0.5503, 0.5535, 0.5452 | 0.9947, 0.9916, 0.9940 |

**Complete frames** (`gpu_soft_shadow_complete_paired_timing`). Triplets with 24 warmup and 24 pairs; processes alternate b0t, c1. Median ms.

| Pass | b0t 1 / 2 / 3 | c1 1 / 2 / 3 |
|---|---|---|
| Interactive span | 0.8162 / 0.8159 / 0.8160 | 0.8198 / 0.8224 / 0.8155 |
| Interactive shading | 0.3665 / 0.3661 / 0.3650 | 0.3678 / 0.3700 / 0.3659 |
| Hard span | 0.8311 / 0.8472 / 0.8491 | 0.8385 / 0.8359 / 0.8441 |
| Hard shading | 0.5261 / 0.5335 / 0.5326 | 0.5308 / 0.5306 / 0.5323 |
| Paired Interactive/hard span | 0.9812 / 0.9639 / 0.9701 | 0.9753 / 0.9950 / 0.9672 |

- The candidate's mean Interactive span is 0.4% above the baseline. Every process is within 1% of 0.816 ms.
- The complete-frame shading median is +0.25% to +1.07% across processes. The paired in-process screen above is the gate measurement.

**Raster lit passes** (`gpu_primary_paired_timing`, 24 warmup and 24 pairs; raster frames on the hardware renderer).
- Fresh soft lighting, fast-state processes: c1 3.2886 / 3.2911 / 3.2983 / 3.2931 ms against b0t 3.2778 / 3.2896 / 3.2804 / 3.2829 ms (1.003×).
- One process per build was in the known slow GPU state, with every pass slower: c1 3.6053 and b0t 3.6214 ms.
- Hardware hard lighting: c1 4.8282 / 4.7288 / 4.7386 ms against b0t 4.7933 / 4.8036 / 4.8507 ms.

## Edit-to-image and compile cost (regression)

**Edit-to-image** (`gpu_program_edit_to_image`): a tint edit and its revert, 5 each, reloaded from memory. Each sample runs from the reload call to the GPU completion of one frame. File polling (the viewer's 50 ms) and presentation are excluded. Every revert was byte-identical. Medians [min, max] in ms.

| Renderer state | Edit | b0t (whole reload) | Candidate | Pipelines compiled |
|---|---|---|---|---:|
| Raster selected | program tint | — | 4213 [4177, 4863] | 6 |
| Raster selected | prelude tint | 4513 [4055, 7443] | 4434 [4276, 5351] | 12 |
| Traced selected | program tint | — | 9922 [7235, 10602] | 8 |
| Traced selected | prelude tint | 7111 [7078, 7152] | 10379 [10146, 10897] | 18 |

- This is the first traced reload baseline: about 7.0 s compile on this quarry renderer. Even raster reloads take about 4.4 s on a hardware-shadow renderer, so the design's 100 ms target is out of reach without a pipeline cache or asynchronous compilation (R7).
- **Regression:** on traced-selected renderers, a candidate program edit takes 9.9 s and a prelude edit 10.4 s, against the baseline's 7.1 s whole reload.
- Raster-selected edits improve slightly: 4.2 s against 4.5 s.

**Per-variant compile cost** (`gpu_program_compile_cost`): each compile has its own TINT value, so no driver cache can serve it, and the frozen and program sources get the identical edit. Medians of 4, in ms.

| Variant | Frozen | Program |
|---|---:|---:|
| Raster lit pair | 1796 | 2062 |
| Hardware lit pair | 1996 | 2206 |
| Fresh lit pair | 1742 | 1961 |
| Traced shading pair | 4281 | 4537 |

- The program compositions cost 6–15% more driver compile time, a measured fact whose cause is undiagnosed.
- A program edit also compiles every variant the renderer can reach: 8 pipelines with traced selected.
- The design's target of compiling only the active variants needs on-demand compilation at first use. That is not built: a frame could then hitch, and rejections could surface late.
- Unexplained: the baseline whole reload is faster (7.1 s) than its frozen variants sum to here (about 9.8 s); some cache or sharing effect inside a whole reload is not modelled.
- With cached sources, compiles take 4–10 ms, as the first `gpu_program_compile_cost` run (c2) showed.

## Constraints and differences from the design

- **C17 open.** The viewer still polls only `scene.wgsl` (viewer code was outside this unit's ownership). Live editing of lit shading in the viewer now needs it to watch `shaders/programs/standard.wgsl` and call `reload_program`.
- **R2.** Raster mesh-uv helpers keep implicit derivatives, which keeps the standard program byte-identical. Programs must call them in uniform control flow or use the `_grad` forms. naga does not reject implicit sampling in divergent raster code.
- **Grad helpers take the receiver:** `sample_*_grad(input, uv, dx, dy)`, because traced sampling needs the draw's material slot.
- **Visibility.** Programs cannot move the shadow-ray origin or choose traced lights. Soft visibility remains `lights[0]` only (C15). Mixed-program traced frames are rejected until phase 3.
- **Store contract.** `rl_finite` clamps every program's output at store. It is the identity on the standard program's values (verified by byte identity).
- **Out of ownership.** One-line `..Default::default()` additions in `src/virtual_gpu.rs` and `viewer/src/prepared.rs` keep exhaustive `MaterialDescriptor` literals compiling. `src/observe.rs` gained the two report fields. `support/fresh_fragment_reference.wgsl` moved to the receiver signature with the same arithmetic. The README and the design doc's status line point here.
- **Tests that reached into `fs_lit`** now edit the standard program (`program::standard_returning`, `Renderer::test_reload_standard`) or swap the standard program's pipelines.
  - Seam-drift rejection tests became program-interface tests: line-mapped syntax errors, program bindings, missing functions and traced implicit sampling.
  - The 4x sample-index oracle edits the generated entry in a test-composed module.
  - `gpu_film_reference` still names the `primary_shade` entry that `af6578c` removed; this is pre-existing, so it is stale and was not run.

## Checks and time

- **CPU.** 79 library tests pass: composition, Parameters reflection and packing, program-line diagnostics, kernel/shading module SPIR-V and grids, and traced-stage rejection. Workspace Clippy `--all-targets -D warnings`, `cargo fmt --check` and `git diff --check` pass.
- **Further GPU tests pass with validation:**
  - the new `gpu_program_api_raster_and_traced`;
  - the five `hardware_shadow_tests`, including 1x/4x per-sample receivers;
  - `gpu_main_depth_order_and_reload`, `gpu_images_uploads_reload_and_timing` and `gpu_batched_culling_oracle`;
  - three `shadow_tests`.
- **Not run:** traced-mode suites (`RENDERLAB_TEST_PRIMARY=traced`), the fresh fragment-reference quarry tests and the quality reference images.

**Implementation:** about 00:43–01:40 UTC of reading, design and editing, interleaved with verification. Editing time was not isolated.

**Verification:**
- 6 release test builds took 293.5 s (b0 cold 115.1 s, others 33–38 s).
- 113 recorded offscreen GPU child processes took 1,825.8 s; 12 exited nonzero: the ten hazard failures above and the two disk-full runs. One earlier baseline suite run failed on a missing evidence directory; its record was replaced by the passing rerun.
- The GPU slot was held in 121 windows, 2,400.8 s in total and at most 301.2 s each. All were released, the last at 02:04:54 UTC. Two runs deferred while another worker held the slot.
- Every accepted preflight found no game, Unreal Editor or foreign renderlab GPU job, and at least 6 GiB available.

Evidence is in `D:/Coding/renderlab-work/prog-shading-p1/`:
- `run.py`, `build.py`, `retry.py`, `scripts/` (verify, suite, alternate, summarize, pixeldiff) and `tools/`;
- `builds/` (b0, b0t, c1–c4, each with SHA-256, HEAD and source diff);
- `frozen/` (the baseline compositions);
- `runs/` (logs, compare results, timing JSON, raster suite captures);
- `gpu-windows.json`.
- The baseline worktree is `prog-shading-p1/base`, at `2ce4509`: `1ce772c` plus a test-only edit-to-image commit, not on any branch.
