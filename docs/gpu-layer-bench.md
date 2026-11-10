# GPU layer benchmark: wgpu 30 (Renderlab v0.1.1) vs ara-gpu (Ara 1e04fe5dd)

2026-11-10, research unit `gpu-layer-bench` (not M8). Raw numbers: `D:/Coding/renderlab-work/gpu-layer-bench/results.json`.
Probe source: `D:/Coding/renderlab-work/gpu-layer-bench/renderlab-probe.patch` (branch `glb-probe` in `renderlab/`, on tag v0.1.1 = 87ba60a).
Replay/naga/memory bench: `glb/`. Scripts: `tools/`. Plans and per-step logs: `plans/`. Lease records: `lease/runs/`.

Labels: **M** = measured here; **I** = inferred (reasoning or estimate, not measured); **R** = reused evidence from Renderlab docs.

## Answer in one paragraph

On measured performance, the GPU layer itself is not the big lever:
- **GPU time is layer-neutral.** It comes from whoever produces the SPIR-V.
  - The largest GPU win found here is available inside wgpu today. Dropping wgpu's per-module shader runtime checks (`create_shader_module_trusted(.., ShaderRuntimeChecks::unchecked())`) cut the non-RT game frame by **11.8%** (9.82 → 8.66 ms) and the RT frame by **4.9%** (4.38 → 4.17 ms). The cost is about one fp16 ULP on ~0.003% of pixels (**M**). ara-gpu never adds those checks.
  - Running `spirv-opt -O` over naga's output makes the GPU **11-14% slower** (**M**), so the cheap optimiser pass does not recover the codegen swings.
- **CPU cost is where ara-gpu measurably wins.** Replaying Renderlab's exact per-frame command shape, ara-gpu records and submits **2.1x** cheaper than wgpu on the non-RT frame (0.19 vs 0.40 ms) and **1.7x** on RT (0.10 vs 0.17 ms) (**M**).
  - In real frames that is roughly 0.3-0.4 ms (non-RT) and 0.1-0.15 ms (RT) of a 0.5-0.9 ms `render()` (**I**).
  - It disappears when frames are pipelined. Renderlab already keeps two frames in flight, and pipelined frames run at exactly the GPU span (**M**).
- **The 21-26 s first-edit stall is the NVIDIA driver, not naga.** naga's front end takes under 1 ms per kernel. The driver's compile time depends heavily on SPIR-V shape: the same deferred-shading kernel compiles in 43-50 s from naga's SPIR-V and in 7-8 s after `spirv-opt` (**M**).
- **Memory:** wgpu used less device memory than ara-gpu for the same resources: +1.4% versus +16.7% over requested (**M**).
- **What decides the foundation is architecture, not speed:**
  - ara-gpu has a working Metal 4 backend (tests pass on an M5 Pro, **M**), runs on Ara's single device, and uses one shader language.
  - Renderlab needs many features ara-gpu lacks: 3D and array textures, MSAA, comparison samplers, raster state, dynamic and refit acceleration structures, ray position fetch, per-pass bindings, and more (M1).
- **My verdict:**
  - ara-gpu is the right long-term base *for Ara's renderer*, for those architectural reasons, with a phased port that closes the gaps first.
  - The measured speed advantage of switching layers is small (CPU only).
  - The GPU win and the indirect-dispatch fix should be taken in wgpu now. They carry over unchanged to a port.

## M1. Capability and port inventory (read-only)

Sources: Renderlab `69c667b`, counted before the tag existed. v0.1.1 (`87ba60a`) differs only by the
P-GATHER merge: `shaders/gi_gather_software.wgsl` (+175 lines), 19 lines of `src/gi_cache.rs`, and docs and version
files, so the counts below are within a few lines of v0.1.1. Ara `1e04fe5dd`
(`crates/ara-gpu`, 19.9k lines src incl. tests, 17.4k non-test; `crates/ara-shader-cook`, 972 lines).
Counts are from the source tree, not runtime, unless marked.

### What Renderlab uses from wgpu, against ara-gpu

| Renderlab dependency (evidence) | ara-gpu at 1e04fe5dd | Gap / effort to close |
|---|---|---|
| Vulkan 1.3 device, one graphics queue | Vulkan 1.3; graphics + **required** dedicated compute-only family; optional 2nd graphics queue | Equal on NVIDIA/AMD. Ara refuses devices with no compute-only family (`create.rs:117-124`). Small: make it optional. |
| Binding model: 107 `BindGroupLayout`, 44 `create_bind_group` sites, 167 `set_bind_group` calls, up to 4 groups, 13 storage buffers + 24 sampled textures + 5 storage textures per stage, 3 dynamic-offset bindings, 27 uniform buffers | One global bindless set: runtime arrays of sampled images (binding 5, ≤65,536), storage buffers (binding 1, **cap 128**, baked into the ABI hash `storage128`), 64 storage images, 3 fixed samplers, 16 AS; **32-byte root constants**; no uniform buffers, no per-pass descriptor sets | **Redesign.** Every pass and every shader changes its resource access to heap indices (8 root words, so most passes need an index table). The 128-buffer cap needs an ABI bump (all Ara pipelines recook). Medium in ara-gpu, large in Renderlab. |
| Samplers: linear/nearest, Repeat/MirrorRepeat/ClampToEdge, **comparison samplers** (2 sites), 16x aniso materials | 3 fixed samplers (aniso repeat, env U-repeat/V-clamp, trilinear clamp); no comparison, no nearest | Missing. Small–medium (sampler table + ABI). |
| Formats: Rgba16Float, Rgba8Unorm(+Srgb), Rgba32Float, Depth32Float, **Rgba32Uint, R32Uint, R32Float, R16Float, Rg32Uint, Rg16Float, Depth16Unorm** | Portable enum: 7 formats. `VulkanDevice::create_image` takes any `vk::Format`, but heap/storage views and Metal mapping assume the portable set | Partly present (Vulkan-native path). Small for Vulkan, medium for Metal parity. |
| **3D textures** (4 sites), **2D-array views** (13), mip chains (5 allocations), `write_texture` (9), `copy_texture_to_texture` regions (3) | 2D only, `layer_count(1)`; mip chains for sampled uploads; `CopyImage` is mip-0, equal size; `UploadImage` whole image | Missing. Medium (image kinds, views, subresource copies, barriers). |
| Hardware MSAA x4 + resolve (`m6-hardware-msaa`) | 1 sample only (`pipeline.rs:209`) | Missing. Medium. |
| Raster state: depth compare Greater/Less/LessEqual/GreaterEqual/Always, **depth bias** (constant 2, slope 2), cull modes, multiple vertex buffers incl. **instance step**, 15 `set_vertex_buffer` sites | Fixed: `GREATER_OR_EQUAL`, no cull, triangle list, one vertex buffer with a fixed 48-byte (pos, normal, uv, color) layout, premultiplied blend only, no depth bias, no stencil | Missing. Medium (extend `RenderState` + cook schema). |
| `clear_buffer` (26 sites), `copy_buffer_to_buffer` (19), `write_buffer` (84) | `CopyBuffer`/`CopyBufferRange`; `UpdateBuffer` ≤ 64 KiB; host `upload` only after the buffer's GPU use completes; **no fill/clear** | Small (`vkCmdFillBuffer`), plus a staging ring for large writes. |
| 64-bit storage atomics (min/max) for the visibility buffer | `shaderBufferInt64Atomics` enabled when present | **Present.** |
| Ray queries + **hit-vertex return** (`EXPERIMENTAL_RAY_HIT_VERTEX_RETURN`) | Ray query yes; `VK_KHR_ray_tracing_position_fetch` **not enabled**. Slang emits `RayQueryPositionFetchKHR` correctly (checked with slangc 2026.17) | Small (enable extension + AS flag). |
| BLAS/TLAS: **per-frame TLAS rebuild, skinned BLAS update (`ALLOW_UPDATE`, `PreferUpdate`), compaction**, 4,096 instances | **Static only**: "build once; replacement uses a fresh identity" | Missing. Medium (update/refit modes, scratch per build, per-frame instance upload). |
| Timestamp queries (40 compute-pass + 7 render-pass timestamp writes, `resolve_query_set`) | Automatic per-pass timestamps via `BeginPass`/`EndPass`, calibrated clocks, pooled | **Equal or better** (calibrated cross-queue). Different shape. |
| Pipeline cache (`PIPELINE_CACHE`) | Own persisted `VkPipelineCache`, vendor/driver/UUID-checked | **Equal.** |
| Readback: `map_async` (7), `map_buffer_on_submit` (8), `on_submitted_work_done` (8) | Coherent readback buffers (`create_readback_buffer`), `read`/`read_completed`, timeline points | Present, different shape (timeline polling instead of callbacks). Small. |
| Automatic resource-state tracking and barriers (wgpu-core) | **None**: explicit whole-resource `Barrier { before, after }`, one `vkCmdPipelineBarrier2` per barrier, no batching, no subresource ranges | **Redesign in Renderlab**: a frame graph or hand-written barriers for ~60 passes. Biggest correctness risk of a port; sync validation exists to catch it. |
| Validation: wgpu-core validates every call, runtime shader checks (bounds `Restrict`, int-div checks, workgroup zero-init) | Lightweight contract validation of handles, scopes, formats, copy ranges; Khronos + sync validation in debug builds | Different; ara-gpu leaves shader safety to robustness features and validation layers. |
| Indirect dispatch: wgpu validates each indirect dispatch with an extra compute pass (`InstanceFlags::VALIDATION_INDIRECT_CALL`, **on by default in release**), zeroing any dispatch above `min(maxComputeWorkGroupCount[x,y,z])` = 65,535 on NVIDIA | `vkCmdDispatchIndirect` directly; no validation pass, no clamp | ara-gpu has no silent skip; the x limit is the device's own (2^31-1 on NVIDIA). In wgpu this is fixable by clearing the flag (`WGPU_VALIDATION_INDIRECT_CALL=0`), which also removes the extra pass; Renderlab's GI cache already uses fixed 4,096-group direct grids to avoid it (`gi_cache.rs:89-97`). |
| `PASSTHROUGH_SHADERS` (tests only) and direct Vulkan via `wgpu::hal` (`program_vulkan*.rs`, 1.3k lines: device-creation callback, pipeline-library compilation, dispatch-table hooks) | Native: everything is SPIR-V | Renderlab's raw-Vulkan code would mostly disappear on ara-gpu. |
| Error scopes, `on_uncaptured_error`, device-lost faults | `GpuError` results; failed submits poison the device | Different shape. Small. |
| Backends compiled: Renderlab enables **only** wgpu's `vulkan` feature | Vulkan + Metal 4 (Apple9+ RT tier, IOSurface sharing, offline metallib via Metal Shader Converter) | Renderlab has **no Mac path today**. wgpu's Metal backend exists, but Renderlab's experimental ray queries, 64-bit atomics and binding arrays on Metal were not tested here (unverified). |

### What ara-gpu offers that Renderlab's wgpu path lacks

| ara-gpu capability | Renderlab today |
|---|---|
| Metal 4 backend (D12/D15), offline metallib, RT tier on Apple9+ | Vulkan only (`wgpu` built with `vulkan` alone) |
| Mesh/task shaders (`DrawMeshTasksIndirect`) | Not used; wgpu 30 has experimental mesh shaders, not enabled |
| Dedicated async-compute queue with explicit `Handoff` ownership transfers and two queue timelines | One queue |
| Two frames in flight via `begin/end_deferred_completion` | **Already built**: `FLIGHTS = 3` slots, admission at `occupied() >= FLIGHTS - 1` (`renderer.rs:85, 1583`), so up to two frames in flight. ARA1 measures synchronously by choice (`synchronous: true`). |
| Calibrated timestamps and cross-queue timing windows | Per-pass timestamps, one queue |
| Background PSO preparation workers, shader-cohort hot reload with last-good fallback | Own reload transaction (P-SHC) on wgpu |
| External memory (Win32 NT handles) and Metal IOSurface shared surfaces | None |
| Bindless heap (no per-frame bind-group creation) | Bind groups, some created per frame (measured in M3) |
| No silent indirect clamp; no indirect-validation pass | Both by default |
| Global queue priority (HIGH) for windowed devices | wgpu default |
| One device and one shader language with the rest of Ara | Second device inside Ara; WGSL beside Slang |

### Port size by Renderlab module (non-test, v0.1.1 candidate)

wgpu exposure per file = `wgpu::` references / command-level calls / lines.

| Module group | Files (refs/calls/lines) | Mechanical | Needs redesign |
|---|---|---|---|
| Frame orchestration | renderer.rs (264/66/3,894), engine/* (49/3/1,483) | encoder/submit/poll plumbing | Frame graph + barriers; flight/readback model onto timelines |
| Visibility buffer, culling, HZB, SW/HW raster | visibility_buffer.rs (119/44/1,493), virtual_gpu.rs (30/10/733) | buffer creation, dispatches | Bind groups to heap indices; 64-bit atomics map 1:1 |
| Shadows (VSM clipmap, surface cache, fresh, hardware) | virtual_shadow_gpu.rs (244/100/2,890), virtual_shadow_cache.rs (62/33/599), fresh_shadow_gpu.rs (115/20/616), shadow_gpu.rs (53/12/885), hardware_shadow_gpu.rs (71/7/679) | dispatch sequences | Depth-bias raster state, 2D-array atlases, comparison samplers, dynamic TLAS |
| Primary rays / traced visibility | primary_ray_gpu.rs (267/74/2,837) | | Position fetch, per-frame TLAS, binding arrays of 64 draws × 12 slots |
| GI cache | gi_cache.rs (163/54/3,179) | most kernels | Fixed grids could return to indirect dispatch |
| Lighting, clusters, atmosphere, environment, reflections | lighting_gpu.rs (167/14/814), light_cluster_gpu.rs (85/23/753), atmosphere_gpu.rs (91/33/1,106), environment_gpu.rs (32/5/426), reflection_probe.rs (89/24/1,102) | | 3D textures (atmosphere LUTs, froxels), cube/array views |
| Transparency, post, TR1, film | transparency_gpu.rs (288/71/2,167), post_gpu.rs (103/21/897), temporal_reconstruction.rs (58/15/1,673), film_accumulation.rs (74/34/908) | | MSAA, blend states, render passes |
| Programs (user shading) | program*.rs (≈8.6k lines incl. tests), program_vulkan*.rs (1.3k) | | Composition + reflection: naga to Slang, or naga emitting Ara's ABI (see below) |
| Skinning, textures, mesh LOD, decals | skinning.rs (37/25/524), texture_residency.rs (50/10/881), mesh_lod.rs (7/6/582), decal_gpu.rs (13/1/124) | mostly mechanical | BLAS refit |

Totals: 71 non-test files, 48.9k lines, about 2,900 `wgpu::` references; 61 test files, 44.5k lines,
1,188 references (93 of 132 `src/` files mention wgpu; 111 of 231 `.rs` files repo-wide). Shaders:
88 WGSL files, 14.5k lines (85 under `shaders/`, `support/`; 14.3k).

**The 14.5k lines of WGSL.** Two independent decisions are bundled in "port to ara-gpu":
1. *GPU layer.* ara-gpu consumes SPIR-V, so it does not require Slang. naga can keep compiling
   WGSL if the shaders adopt Ara's bindless ABI (runtime arrays at set 0, 32-byte push constants).
   Every WGSL binding declaration changes either way.
2. *Language.* A Slang rewrite is largely syntactic for kernels (the M2(c) port of the 100-line
   software raster took ~30 minutes and needed three semantic notes: `round` is `rint`, WGSL's
   eager `select`, and matrix layout). The program system is the hard part: Renderlab composes user
   `surface`/`shade` WGSL at runtime and reflects `struct Parameters` from naga IR. Ara never
   links a compiler at runtime (offline cook, `ara-shader-cook` runs slangc out of process, about
   165 ms process floor each, plus a DXIL compile for Metal). User-written Renderlab programs are WGSL
   today; moving them to Slang is a user-visible language change. Slang modules/interfaces could
   express `surface`/`shade` more cleanly than text concatenation; that is a design proposal, not
   measured.

## Run conditions (all GPU measurements)

| | |
|---|---|
| Hardware | RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, 32 GB, Windows 11 |
| Builds | release. Renderlab harness `glb_frames` (examples/, probe branch) on rustc 1.96 stable (as production). Replay/memory/naga bench `glb` on rustc 1.99.1 (ara-gpu needs it); wgpu 30.0.1 in both. `CARGO_BUILD_JOBS=2`, build-slot protocol (`tools/build.py`). |
| Scene | ARA1 `game-base-timing` (game-opaque-frozen-base), 1920×1080, GI + reflections + 128-light shadow budget (no lights in scene), uniform sky; 88 camera frames (40 still, 32 moving, 16 cut), run twice per process (176 frames) unless noted. Quarry not measured. |
| Isolation | Every GPU process ran inside a ≤14 min lease (`tools/p-nrt/lease.py`, owner `gpu-layer-bench`; leases glb-p1b 158 s, glb-p2 150 s, glb-p3 235 s, glb-p4 12 s). The lease runner's 10 Hz samples show **0 rustc** processes and ≥ 11 GB free throughout. The validation layer the runner exports was removed for every process. No window, focus change or capture. Wallpaper/background GPU load was not controlled. |
| Root order | All runs under the 01:58:55 HOLD order ("no unit may build or lease for Renderlab M8"). This unit is not M8, and the brief authorised post-tag runs. Requests were posted as `gpu-layer-bench`, and the order hash was checked before each lease. |
| Probe | A patched Vulkan dispatch table on Renderlab's existing development device (`RENDERLAB_STARTUP_PROFILE_DRIVER`). It counts every `vkCmd*`/submit and, optionally, times them, brackets each compute dispatch with timestamps, and swaps production pipelines for variant pipelines built from other SPIR-V with the same layout, bind groups and dispatches. Counting adds +0.05 ms to `render()`; counting plus driver timing adds +0.09 ms (non-RT). |

## M2. Shader code generation (GPU)

Method (**M**): one process per comparison. Frames alternate production pipeline (A) and variant (B) as A B B A, with 86 frames per variant after warm-up. Per-kernel GPU time is the sum of that kernel's dispatches per frame (bottom-of-pipe timestamps around each dispatch), median over frames. The frame column is Renderlab's own GPU span. Identity compares per-frame HDR (RGBA16F) SHA-256 between separate processes. Production repeats exactly run to run: **88/88** identical frames on both paths.

Variants:
- **(a)** naga SPIR-V as shipped.
- **(b)** (a) after `spirv-opt -O` (SPIRV-Tools 2026.4).
- **(c)** the software ID raster (`visibility_buffer.wgsl` + `visibility_buffer_sw.wgsl`, entry `raster`) hand-ported to Slang (`slang/vb_raster.slang`) and cooked with Ara's settings (`-O2 -profile sm_6_6+spirv_1_6 -matrix-layout-column-major -D VULKAN=1`, plus `-fvk-use-entrypoint-name` to keep the entry name).
- **(d)** naga SPIR-V with wgpu's shader runtime checks off: `ShaderRuntimeChecks::unchecked()`, which removes naga's `index: Restrict` clamps, integer-division checks, loop bounding and ray-query initialisation tracking. Workgroup zero-initialisation stays on.

Non-RT (frame = Renderlab GPU span, ms; kernels in µs):

| | A frame | B frame | Δ frame | Deferred shading `primary_shade_tiles` | Sun miss walk `vsc_missed` | `vsc_claim` | Sum of kernels | Identity (HDR) |
|---|---:|---:|---:|---|---|---|---:|---|
| (b) spirv-opt -O | 10.08 | 11.19 | **+11.0%** | 2,076 → 2,817 (+35.7%) | 3,705 → 3,916 (+5.7%) | +4.2% | +12.0% | 0/88 identical; ≤ 0.0008% px, max abs 4.9e-4, ≤ 1 fp16 ULP |
| (d) unchecked, all 64 mapped modules | 9.82 | 8.66 | **−11.8%** | 2,026 → 1,966 (−3.0%) | 3,682 → 2,773 (**−24.7%**) | −15.6% | −13.7% | 0/88 identical; ~0.003% px (≈60 px), max abs 4.9e-4 (≈1 fp16 ULP), max rel luma 6.6e-4, no NaN/Inf |
| (d) unchecked, visibility-buffer modules only | — | — | ~0 | — | — | — | +0.8% (noise) | **88/88 identical**; raster −4.8%, cull −5.4%, finish −22.8% |
| (c) Slang raster | — | — | ~0 (kernel is 5 µs) | — | — | — | −0.3% | 0/88 identical; 0.13-0.44% px changed, 423-447 px > 1% rel luma, PSNR 106-111 dB (luma) |

RT:

| | A frame | B frame | Δ frame | Deferred shading | Primary trace | Sum | Identity |
|---|---:|---:|---:|---|---|---:|---|
| (b) spirv-opt -O | 4.56 | 5.21 | **+14.3%** | 1,012 → 1,667 µs (+64.7%) | +0.1% | +15.0% | 2/88; ≤ 1 fp16 ULP-scale |
| (d) unchecked, 43 modules | 4.38 | 4.17 | **−4.9%** | 1,006 → 869 µs (−13.6%) | −0.5% | −5.5% | 0/88; same ≈1 ULP scale, ~0.003% px |

Notes:
- **The baseline differs between processes** (9.82 vs 10.08 ms non-RT). Process-to-process drift is why each variant is compared within one process.
- **(c) Slang raster: 5.3 → 2.0 µs (−63%), but this scene barely uses it.** Most clusters raster in hardware, so it is 0.05% of the frame.
  - Its differences are localised (`runs/m2c-slang-diff-0040.png`, inspected): one cluster above 1% at the small figure behind the gold sphere, where clusters are software-rastered, and scattered 0.1%-level patches on flat surfaces. That pattern is consistent with depth-bit differences, from different float association or contraction, propagating through the GI cache and TR1 history. No structural or geometry error is visible.
  - **I:** part of Slang's win is that wgpu also zero-initialises the kernel's 1.5 KB of workgroup memory on every workgroup, and most workgroups exit early. Unchecked naga still pays that (5.1 µs) and Slang does not (2.0 µs). Not separately measured.
- **(b) helps the driver, not the GPU** (M4: shading compile 43-50 s → 7-8 s). It is not a runtime win.
- **naga 30 output fails current `spirv-val`** (SPIRV-Tools 2026.4) on **39 of 89** non-RT and **33 of 59** RT modules, rule `VUID-StandaloneSpirv-None-10684`: explicit layout decorations on Function/Workgroup/Private types. That includes the sun lookup and deferred shading. The NVIDIA driver accepts them. `spirv-opt` had to skip input validation to run. A future driver or validation-layer tightening could reject these modules.
- **The (d) image differences are rounding-level on this scene, not semantic.** No out-of-range index changed visible output here. Other scenes could index out of range where the clamps were silently repairing it. That risk is **I**, and it was not tested beyond game-base.
- **Not done in the time-box:** a Slang port of a heavy kernel (deferred shading or the sun walk); this M2(c) covers one small kernel only. Also not done: GI lookup separately (the GI kernels appear in the tables above as `gi_*`).

## M3. CPU cost per frame

### Attribution of Renderlab's `render()` (**M**, synchronous ARA1 protocol, 174 frames per run, medians, ms)

| Part | Non-RT | RT | How measured |
|---|---:|---:|---|
| `render()` total (two runs) | **0.920** (0.929 / 0.912; p10 0.82, p90 1.12) | **0.498** (0.464 / 0.533) | harness timer |
| wgpu `CommandEncoder::finish` replay (validation, state tracking, barrier generation, hal encode), excluding driver | **0.320** | **0.143** | timer around `finish()` minus probe driver time |
| Driver `vkCmd*` (inside `finish`) | **≤ 0.263** | **≤ 0.077** | probe timers (include ≈ 25 ns/call timer overhead × ~1,300 calls) |
| Encode excluding `finish` (Renderlab's command building plus wgpu's recording API) | 0.167 | 0.122 | `CpuFrame.encode` − finish |
| `queue.submit` | 0.104 = wgpu 0.032 + `vkQueueSubmit` 0.072 | 0.094 = 0.035 + 0.059 | CpuFrame + probe |
| `write_buffer` uploads | 0.041 | 0.039 | CpuFrame.upload |
| Other Renderlab CPU (preparation, visibility, shadows) | 0.026 | 0.023 | CpuFrame |
| **Split** | wgpu ≈ 0.35-0.56, driver ≈ 0.27-0.33, Renderlab ≈ 0.03-0.23 | wgpu ≈ 0.18-0.34, driver ≈ 0.10-0.14, Renderlab ≈ 0.02-0.16 | ranges: the 0.167 / 0.122 and upload rows mix wgpu's recording API with Renderlab's own code |

**Synchronous frame cost (M):**

| Path | Interval, synchronous | Interval, pipelined | Cost of synchronous |
|---|---:|---:|---:|
| Non-RT | 10.97 ms (service 10.44, GPU span 9.60) | 9.49 ms (= GPU span) | **1.47 ms/frame** |
| RT | 5.65 ms (GPU span 4.40) | 4.85 ms | **0.80 ms/frame** |

Pipelined means `GLB_PIPELINED`: no drain, using Renderlab's existing 3 flight slots, so up to 2 frames in flight. The m7-gaps figure of 1.8-2.1 ms came from older builds and other scenes; today's game-base figures are as above.

**Frames in flight: wgpu does not prevent two frames in flight; Renderlab already has them.** `FLIGHTS = 3` with admission at `occupied >= FLIGHTS-1` (`renderer.rs:85,1583`). The synchronous gap is ARA1's measurement choice (`synchronous: true`), not a wgpu or Renderlab limitation (**M**).

**wgpu's indirect validation (M):** off via `WGPU_VALIDATION_INDIRECT_CALL=0`.
- Non-RT: `render()` 0.92 → 0.83 ms, `finish` 0.58 → 0.49 ms, GPU span 9.51-9.68 → 9.34 ms.
- RT: no change (one indirect dispatch).
- Output identity was not checked (**not verified**).

**Per-frame command shape (M):**

| Level | Non-RT | RT |
|---|---|---|
| wgpu (API trace) | 32 compute + 5 render passes (8 timed); 56 `set_pipeline` (47 distinct); 161 `set_bind_group`; 37 direct + 51 indirect dispatches; 12 indirect + 1 direct draws; 11 `clear_buffer`; 19 `write_buffer` (9.4 KB); **0 bind groups created per steady frame** | 21 compute + 1 render pass (6 timed); 26 `set_pipeline`; 63 `set_bind_group`; 24 + 1 indirect dispatches; 1 draw; ~13 writes (2.6 KB) |
| Vulkan (probe) | 142 dispatches (51 indirect), 156 compute pipeline binds, 381 descriptor-set binds, 257 `vkCmdPipelineBarrier` (398 buffer + 89 image barriers), 55 push constants, 2 `vkQueueSubmit` carrying **83 command buffers** | 26 dispatches, 27 binds, 68 set binds, 50 barrier calls (109 + 35), 2 submits / 54 command buffers |

The non-RT Vulkan level shows about 54 more dispatches, about 100 more pipeline binds and 55 push constants than Renderlab issues. That is wgpu's own indirect-validation work: Renderlab declares no push constants.

### Paired replay: same command shape through wgpu and through ara-gpu (**M**)

One process (`glb replay`), both devices, A B B A frames, trivial shaders. Shapes are taken from the table above:
- Barriers are explicit for ara-gpu: 385 per non-RT frame, which is wgpu's 487 resource barriers minus those its indirect validation adds; 144 per RT frame.
- Bind-group sets become root-constant updates (bindless).
- Writes become `UpdateBuffer`.
- Live resources: 180 buffers + 40 textures (non-RT), 60 + 20 (RT).

Timed: CPU record + submit, plus post-completion maintenance (wgpu `poll`; ara-gpu `resolve` + `collect`). Two independent samples per shape (300 and 400 frames per layer).

| Shape | wgpu record+submit, ms (p10-p90) | ara-gpu record+submit, ms (p10-p90) | Maintenance wgpu / ara | wgpu ÷ ara (incl. maintenance) |
|---|---:|---:|---:|---:|
| Non-RT | 0.400 / 0.399 (0.37-0.49) | 0.175 / 0.176 (0.155-0.215) | 0.0003 / 0.012-0.014 | **2.1x** |
| RT | 0.165 / 0.166 (0.147-0.203) | 0.084 / 0.088 (0.070-0.110) | 0.0004 / 0.011-0.012 | **1.7x** |
| Non-RT ×4 (dispatch scaling) | 1.443 (1.30-2.36) | 0.594 (0.55-1.00) | 0.0006 / 0.017 | **2.4x** |
| Non-RT, wgpu indirect validation off | 0.321 (0.31-0.39) | 0.181 (0.17-0.22) | 0.0004 / 0.015 | 1.6x |

Limits of the replay:
- Trivial shaders and synthetic resources.
- ara-gpu's barrier count is an assumption, not a frame graph's output.
- The replay wgpu frame (0.40 ms) is lighter than Renderlab's real `finish()` + submit (0.69 ms), because real bind groups hold textures, samplers and arrays.
- **I:** applying the ratio to the real frame saves ≈ 0.3-0.4 ms per non-RT frame and ≈ 0.1-0.15 ms per RT frame in synchronous frames, and nothing visible in pipelined GPU-bound frames.
- ara-gpu's own costs that scale badly: a fresh `vkAllocateCommandBuffers` per submit, freed at resolve; every live resource stamped on every submit (O(resources)); one `vkCmdPipelineBarrier2` per barrier (no batching).

## M4. Shader iteration

**Front end (M):** the software raster kernel; composed WGSL 121 lines, SPIR-V 14 KB.

| Stack | Step | Time |
|---|---|---:|
| naga 30, in process (50 reps, median) | parse | 0.49 ms |
| | validate | 0.05 ms |
| | SPIR-V write | 0.12 ms |
| | **total** | **0.67 ms** |
| slangc 2026.17 (5 runs per step) | `-target spirv` | 188-214 ms |
| | `-target dxil` (Ara cooks it too, for Metal) | 204-252 ms |
| | `spirv-val` | 12-25 ms |
| | **Ara cook per kernel per edit (serial)** | **≈ 0.42 s** |
| | process floor (trivial kernel) | 161-195 ms |

**Driver compile, fresh** (**M**): each module is uniquified with an `OpSourceExtension` nonce so neither the NVIDIA disk cache nor the application cache can serve it. Times are `vkCreateComputePipelines`, with the production layout and pipeline cache.

| Kernel (naga SPIR-V size) | naga as shipped | after spirv-opt -O | Slang |
|---|---:|---:|---:|
| `primary_shade_tiles`, deferred shading (227 KB) | **49.5, 43.5, 42.9 s** | **7.8, 7.6, 6.9 s** | not ported |
| `vsm_lookup` (99 KB) | 575 ms | 448 ms | — |
| `vsc_missed` (94 KB) | 563 ms | 446 ms | — |
| `vmap_lookup` (85 KB) | 305 ms | 192 ms | — |
| `gi_gather_batched` (59 KB) | 221 ms | 208 ms | — |
| `raster` (14 KB) | 18.9 ms | 16.8 ms | 14.3 ms |

The repeat order was opt, naga, opt, naga; each sample is one process.

**Where the 21-26 s stall belongs: the driver** (**M** here, and **R** from P-SHC: 25.69 s of a 25.79 s pipeline call). naga plus wgpu is under 0.1 s of it.
- The driver's time depends strongly on the SPIR-V it receives: the same shading kernel compiles **6x faster** after `spirv-opt` (whose output then runs 36% slower, M2).
- **I:** Slang at `-O2` runs its own SPIR-V optimisation, so a Slang-built shading kernel would likely compile closer to the `spirv-opt` time than to naga's. Not measured: no heavy kernel was ported.
- Switching the GPU layer does not change this; the SPIR-V producer does.
- ara-gpu's reload path cooks out of process (slangc ×2 + spirv-val ≈ 0.4 s per kernel) before the driver compile, and Metal additionally needs the Metal Shader Converter.

## M5. Memory overhead (**M**, synthetic set)

The set is 1,000 storage buffers (256 B to 1 MiB, log-spaced), 24 1080p RGBA16F storage+sampled images and 300 64×64 RGBA8 textures: **529.5 MB requested**. Measured as `VK_EXT_memory_budget` device-local heap usage of this process, before and after each layer allocates, one layer at a time.

| | Heap usage delta | Over requested | Notes |
|---|---:|---:|---|
| wgpu (gpu-allocator) | **536.9 MB** | **+1.4%** | Allocator report: 556.6 MB allocated, **738.2 MB reserved** in 4 blocks, 1,327 sub-allocations. Creating the wgpu device itself moved heap usage by +207.8 MB (**unexplained**; excluded from the comparison). |
| ara-gpu | **618.1 MB** | **+16.7%** | One `vkAllocateMemory` per buffer and per image (1,324 native allocations), exact-size reuse pool only |

This driver reports `maxMemoryAllocationCount` = 2^32−1, so ara-gpu's one-allocation-per-resource design is not capped here. It would be capped on drivers reporting the Vulkan minimum of 4,096 (**I**, portability risk).

## M6. Mac (**M**, report only)

MacBook Pro Mac17,9, **Apple M5 Pro**, macOS 27.0, Xcode 26.6, rustc 1.99.1, fresh clone at 1e04fe5dd:

| Command | Passed | Failed | Ignored |
|---|---:|---:|---:|
| `MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 cargo test -p ara-gpu --lib metal` | 29 | 0 | 2 |
| `cargo test -p ara-gpu --lib metal` | 29 | 0 | 2 |
| `cargo test -p ara-gpu` | 60 | 0 | 22 |

- The two `harness=false` tests passed: SDL3 Metal presentation (10 frames) and cross-process IOSurface import.
- The Vulkan GPU tests are ignored on macOS by design.
- The RT tier was not reported by any test that ran; the RT timing benchmark is opt-in and was ignored.
- **Metal Shader Converter is not installed on that Mac**, so it cannot cook new metallibs; the tests use committed artifacts.
- Renderlab today compiles wgpu with the `vulkan` backend only, so it has no Mac path. wgpu's Metal backend with Renderlab's experimental ray queries, 64-bit atomics and binding arrays was **not tested**.

## Verdict per axis

| Axis | Verdict | By how much | Confidence |
|---|---|---|---|
| GPU time from shader codegen | **Equal as a layer.** GPU time is decided by the SPIR-V. wgpu adds runtime checks by default, worth 11.8% (non-RT) and 4.9% (RT) of game-base GPU time, but they can be turned off per module in wgpu today. ara-gpu adds none. | wgpu default is 4.9-11.8% slower; equal after `create_shader_module_trusted` | High for game-base (M); other scenes not measured |
| CPU encode + submit | **ara-gpu better** | 1.7-2.4x cheaper in replay (M). Real frame ≈ −0.3-0.4 ms non-RT, −0.1-0.15 ms RT (I) | Replay high; real-frame projection medium |
| Frames in flight / synchronous gap | **Equal.** Renderlab already pipelines two frames on wgpu. | Pipelined interval = GPU span (M) | High |
| Shader edit to pipeline | **Equal as a layer.** The stall is the driver. naga's in-process front end is ~300x faster than slangc's process; Ara's cook adds ≈ 0.4 s per kernel. SPIR-V shape changes driver time 6x. | — | High (M); Slang's effect on big-kernel driver time not measured |
| Indirect dispatch | **ara-gpu better by default**: no 65,535-group silent skip, no validation pass. wgpu fixes it with one instance flag. | −0.09 ms CPU, −0.17-0.34 ms GPU non-RT (M) | Medium (identity not checked) |
| Memory | **wgpu better** in this test | +1.4% vs +16.7% heap over requested (M) | Medium (synthetic set; wgpu device baseline unexplained) |
| Feature coverage for Renderlab | **wgpu far better today**: 3D/array textures, MSAA, comparison samplers, raster state, uniform buffers, dynamic and refit acceleration structures, position fetch, partial copies, buffer clears, formats | ~15 gaps (M1) | High (source read) |
| Mac / Metal | **ara-gpu better**: tested Metal 4 backend vs none | (M) | High for tests passing; no Metal performance measured |
| One device, one shader language inside Ara | **ara-gpu better** (architectural) | not measurable as a number | — |

## If ara-gpu is chosen: phased port plan (**I** throughout)

1. **Phase 0, now, in wgpu (days). Wins that carry over unchanged:**
   - Build modules with `create_shader_module_trusted(.., unchecked())`. This is −11.8% GPU non-RT and −4.9% RT on game-base. Accept a one-time re-baseline: about 1 fp16 ULP on ~0.003% of pixels.
   - Turn off `VALIDATION_INDIRECT_CALL`, with Renderlab-side group-count guards replacing wgpu's.
   - Measure `zero_initialize_workgroup_memory: false` for kernels that initialise their own workgroup memory.
   - Then the GI cache's fixed 4,096-group grids can become indirect dispatches again.
2. **Phase 1: close ara-gpu's gaps.** Each item needs Metal parity, which roughly doubles some of the work:
   - any `vk::Format`, 3D and array images and views, mip and region copies, buffer fill;
   - a sampler table with comparison and nearest samplers;
   - `RenderState` with depth compare, depth bias, cull, topology, vertex layouts and instance rate, MSAA and resolve;
   - per-frame TLAS rebuild, BLAS update/refit and compaction, `VK_KHR_ray_tracing_position_fetch`;
   - a larger storage-buffer heap (an ABI bump: all Ara pipelines recook);
   - optional compute-only family;
   - batched barriers and subresource ranges;
   - command-buffer recycling (instead of allocate/free per submit).

   Rough size: 4-6k lines in ara-gpu.
3. **Phase 2: move Renderlab to the bindless ABI while still on wgpu.** wgpu 30 has binding arrays and immediates. This separates the shader rewrite from the layer swap and keeps an A/B path the whole way.
4. **Phase 3: swap the layer pass by pass** behind a frame graph that emits explicit barriers. Renderlab's ~60 passes, 30k lines of GPU-module Rust, and `program_vulkan*.rs` (1.3k lines) mostly disappear. Sync validation is mandatory in this phase.
5. **Phase 4, a separate decision: shader language.**
   - ara-gpu accepts SPIR-V, so naga-compiled WGSL can stay.
   - Moving the 88 WGSL files (14.5k lines) to Slang is mostly syntax: the 100-line raster port took about 30 minutes. User programs are a user-visible language change, and runtime composition needs either slangc at runtime or the out-of-process cook.

**Main risks:**
- Explicit synchronisation bugs.
- Loss of byte identity: every image baseline re-certifies after any SPIR-V change.
- ara-gpu's per-resource allocations (+17% memory here; allocation-count caps on some drivers) and its O(resources) per submit.
- Metal parity for every new feature.
- `spirv-val` rule 10684 failures in naga output if naga is kept.

**Effort:**
- Phase 0: about 1-2 days including re-baselining.
- Phases 1-3: on the order of 4-8 weeks of focused agent work plus verification.
- Phase 4: 1-2 weeks for the engine shaders, more for user-program tooling.

These are estimates from code size and one small port, not measured.

## The middle path: stay on wgpu, feed it better SPIR-V (all **M**, game-base 1080p)

| Option | GPU frame non-RT | GPU frame RT | Driver compile, deferred shading | Identity |
|---|---:|---:|---:|---|
| Production (naga, checked) | 9.82-10.08 ms | 4.38-4.56 ms | 43-50 s | — |
| **naga, runtime checks off** (`create_shader_module_trusted`) | **−11.8%** | **−4.9%** | not measured | ≈1 fp16 ULP on ~0.003% px |
| naga → `spirv-opt -O` (passthrough-equivalent swap) | **+11.0% (worse)** | **+14.3% (worse)** | **7-8 s (6x faster)** | ≤1 fp16 ULP on ≤ 0.0008% px |
| Slang-built SPIR-V, one kernel (raster) | kernel −63% (5.3 → 2.0 µs); frame unchanged | — | 14.3 vs 18.9 ms (raster) | 0.13-0.44% px, PSNR ≥ 106 dB |
| Indirect validation off (instance flag) | −0.17-0.34 ms GPU, −0.09 ms CPU | ~0 | — | not checked |

`spirv-opt` is a compile-time tool only: it would suit a development "preview" policy, never shipped pipelines.

## What could not be measured, and why

- **A Slang port of a heavy kernel** (deferred shading, sun walk, GI lookup). Time-box: the shading kernel is the composed standard program (~227 KB of SPIR-V), and a faithful port is days, not hours. So "does Slang's codegen beat checks-off naga on big kernels" and "does Slang SPIR-V compile faster in the driver" are open. The raster result hints yes for small kernels, partly because of workgroup zero-init (**I**).
- **Renderlab actually running on ara-gpu.** That needs the port; M3's replay stands in for it.
- **Metal performance**, wgpu's Metal backend for Renderlab, and Metal Shader Converter cooks (not installed on the Mac).
- **The quarry and other ARA1 scenes.** Only game-base was run.
- **Identity of the indirect-validation-off run**, and `zero_initialize_workgroup_memory: false` on its own.
- **A sampling profile of wgpu internals.** No profiler is installed. The wgpu/driver split comes from timers around `finish`/`submit` plus probe-timed driver calls, which bound but do not itemise wgpu-core's validation versus tracking.
- **The +207.8 MB heap jump on wgpu device creation** (M5) is unexplained.

## Housekeeping and time

- Cargo targets (`target-rl` 643 MB, `target-glb` 613 MB) are deleted.
- Deleted after summarising: 22 HDR dumps (16.6 MB each), both wgpu API traces (58 MB and 48 MB; their counts are in `results.json` → `m3.replay_shapes` and the table above), pipeline caches and staged binaries.
- My 6.7 GB scratch Ara clone is deleted. Recreate it with `git clone --no-checkout D:/Coding/projects/desktop/snengine ara && git -C ara checkout --detach 1e04fe5dd`.
- Kept: `dump/` (9 MB of production SPIR-V by hash), `variants/` (18 MB), per-run `frames.jsonl`, the diff PNG, `renderlab/` (73 MB, branch `glb-probe`).
- No edits were made to D:/Coding/projects/desktop/renderlab or snengine.
- Time:
  - Implementation (probe, harness, replay bench, Slang port, scripts) ≈ 70 min.
  - Builds: 6 Cargo commands (2 failed on my compile errors), about 5.3 min of compiler time; about 12 min queued behind ara-integration.
  - GPU: 4 leases, 9.3 min.
  - Total wall ≈ 2 h 15 min.
