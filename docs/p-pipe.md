# P-PIPE: cold start, cache persistence and compile memory

2026-11-10, branch `t3/p-pipe`, main at `f820af9` (v0.1.3 + RL#2, #6, #12, #13, P-STILL) plus
the runner watchdog `91e9bc1`. Windows 11, Ryzen 7 7800X3D, 32 GB, RTX 4070 SUPER, NVIDIA
617.14, Vulkan, wgpu 30.0.1, Rust 1.96.0, release builds, two Cargo jobs.

| Step | Result | Verdict |
|---|---|---|
| 1. Persist both caches after each compile batch | Killed-then-restarted game scene: non-RT 126.1 → 35.6 s, RT 143.8 → 42.6 s | **MERGE** |
| Memory (incident in lease pipe1) | 18 GB was driver compile memory; now one heavy compile at a time, peak 5.1-6.1 GB | **MERGE** (with step 1) |
| 2. Structure: compile only what the frame uses | Unused Hard/Soft shading variant no longer compiles: small-scene RT cold 148.6 → 74.6 s | **MERGE** (with step 1) |
| 2. Engine/program kernel split | Measured where the time is; not implemented | Open |
| 3. Tiered optimiser-first compile | Not started | Open |

## Step 1: caches saved during compilation

Before, `Compilation` (pipeline_compilation.rs) and `ProgramCache` (program_gpu.rs) wrote
only in `Drop`, so a killed process kept nothing.

- `PipelineDevice` adds each pipeline's creation time to an unsaved total. Once the total
  reaches `max(0.5 s, 20 × the last save's duration)`, it saves the cache. A kill therefore
  loses at most that much finished work plus the compile in flight. Saves took 1-5 ms each,
  against 0.2-96 s per compile.
- `ProgramCache` (the viewer's `set_program_cache_directory`) saves after each counted batch.
- Both now use one `write_cache`: each save writes its own same-directory temporary file
  (`<pid>-<sequence>.tmp`) and renames it over the cache, so readers see a whole old or a
  whole new file. The old `ProgramCache` temporary was per process, so two renderers in one
  process could share it. `read_cache` checks size and SHA-256, as before.

Game scene (examples/game_scene.rs, 1280×720, two frames), candidate binary. Each process
had its own NVIDIA disk-cache directory (`__GL_SHADER_DISK_CACHE_PATH`). A check showed the
driver honours it: a fresh application cache on a reused driver directory started in 0.77 s,
and a fresh driver directory took the full 88 s. Only the application cache carried work
between processes. The kill is a TerminateProcess at the stated time.

| Game scene | Cold | Warm | Killed at | Restart after kill | Peak private |
|---|---:|---:|---:|---:|---:|
| Non-RT | 125.5 s | 0.92 s | 100.8 s | 34.8 s | 5.23 GB |
| RT | 143.0 s | 0.94 s | 114.9 s | 42.1 s | 6.08 GB |

(First-image seconds from `run.json`. A kill at 0.6 × cold, 86 s on RT, fell inside the first
95.7 s kernel and saved only the small pipelines: restart 129.6 s. Work in flight cannot be
saved.) Main's binary writes only on drop, so its restart after a kill is a cold start.

RGBA8 and HDR hashes are identical across cold, warm and after-kill runs on each path. Main's
binary, drawing the game scene from copies of the candidate's caches, gives the same RGBA8
and HDR hashes on both paths (frames 0 and 1).

**Two engines, one cache directory** (examples/p_pipe_engines.rs, 640×360 test scene; main
now contains RL#2 and RL#6, so main is the Ara PR #13 tree):

| Case | First images | Peak private | Min free |
|---|---:|---:|---:|
| RT, second engine after the first's image (cold) | 74.5 / 75.0 s | 5.10 GB | 16.4 GB |
| RT, both at once, warm | 0.68 / 0.68 s | 1.26 GB | 20.5 GB |
| Non-RT, both at once, warm | 0.74 / 0.74 s | 1.09 GB | 20.6 GB |
| RT, both at once, cold, one compile in flight | 75.6 / 75.6 s | 5.74 GB | 15.9 GB |
| RT, one engine after the concurrent runs | 0.55 s | 0.76 GB | 20.5 GB |

All runs passed with identical image hashes per path. RADV was not available, so Ara's
SIGSEGV case (three GPU tests in one process on a fresh cache, RADV) is not reproduced here.
The writes it shares with this case are now per-writer temporary files plus rename.

## The 18 GB: driver compile memory

Lease pipe1 ran the cold two-engine RT step with no compile limit. Its working set reached
16.0 GB and private memory 18.3 GB, the PC fell to 3.8 GB free, and the coordinator killed it.
Found while separating it:

- The shared lease runner (scripts/m6-film-run.py) forces
  `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` on every leased child. pipe1 ran under
  the validation layer, and so has every n8-run timing since F0. P-PIPE's plan runner strips it
  for every later run. **The runner should stop forcing it for measurements.**
- One RT engine, validation off, at most 4 compiles in flight: past 8 GB private in 38.7 s.
- One RT engine, one compile at a time: 148.6 s, 5.6 GB. Private memory rises from 0.7 to
  5.5 GB during the first program shading compile and stays there.
- Warm start: 0.65 GB RT, 1.10 GB non-RT. Our buffers and caches are small; the caches
  are 1-3 MB.

So the memory is NVIDIA's compiler working on the program shading kernels: about 4.5-5 GB
per kernel in flight. The engine had no limit: `program_gpu::in_parallel` spawns one thread
per pipeline, and two engines compile independently.

Changes:

- **Process-wide compile limit** (`CompileSlot` in pipeline_compilation.rs): every engine,
  reload worker and parallel set shares it. The default is 1 because two heavy compiles at
  once pass 8 GB. `RENDERLAB_PIPELINE_COMPILES` raises it. The cost is serial compilation:
  a mixed-program scene pays the sum of its kernels, about 143 s on the RT game scene, where
  parallel compiles would have needed about 10 GB.
- **Compile only the traced shading a frame uses** (next section). That halves both memory
  and time on single-program scenes.

| Cold start, one engine, 640×360 | Before | After |
|---|---:|---:|
| RT seconds to first image | 148.6 (one compile at a time) | 74.6 |
| RT peak private | > 8 GB at 4 in flight (killed) | 5.11 GB |
| Non-RT seconds to first image | killed at 66 s (> 8 GB) | 78.7 |
| Non-RT peak private | > 8 GB at 4 in flight (killed) | 5.26 GB |

A second view or engine does not double the peak (5.10-5.74 GB above).

## Step 2: where the cold start goes

Per-pipeline native creation times (`RENDERLAB_STARTUP_PROFILE`, one compile in flight,
validation off, fresh driver cache). Wall time is creation inside the compile slot.

| Scene / path | Pipeline | Seconds | Share of pipeline time |
|---|---|---:|---:|
| Test scene RT, before | traced program shading (Soft) | 74.0 | 50% |
| | traced program shading (Hard, unused) | 68.9 | 46% |
| | 49 others | 5.3 | 4% |
| Test scene RT, after | traced program shading (Soft) | 68.6 | 93% |
| | 49 others | 5.3 | 7% |
| Test scene non-RT, after | traced program shading (Soft) | 71.5 | 92% |
| | 79 others | 6.1 | 8% |
| Game scene RT | MP1 list shading, standard program | 95.7 | 67% |
| | MP1 list shading, toon program | 30.7 | 22% |
| | TP2 glass reflection | 8.1 | 6% |
| | TP1 transparent | 1.1 | 1% |
| | 62 others | 6.3 | 4% |
| Game scene non-RT | MP1 list shading, standard program | 91.0 | 73% |
| | MP1 list shading, toon program | 25.8 | 21% |
| | TP1 transparent | 1.0 | 1% |
| | 93 others | 6.7 | 5% |

Against the census in draft PR #11 (CPU SPIR-V measurements): the standard program's shading
kernels are about 207-233 KB and cost 69-96 s each on NVIDIA. Toon's are about 104-121 KB and
cost 26-31 s. Shared engine kernels (traversal, VSM, GI tracing and gathering, TR1, post), all
together, cost 5-7 s. **Nearly the whole start is the program shading kernel**, whose size
comes from the engine helpers composed into it (local shadows, GI preparation, BRDF/environment
filtering, decals). The census's third and fourth boundaries (receiver-service kernels for GI
lookup and local-shadow visibility, and reconstruction split from the user shader) move that
code out of every program's kernel. That split is the next structural unit. It was not
attempted here. Its frame cost must be measured against these compile numbers.

The coordinator's four questions:

1. **Variants a scene needs at start.** A frame shades with Hard *or* Soft (Soft whenever the
   sun has a disk), but `Variant::Traced` built both, and `ensure_streamed` built both
   streamed kernels on one thread. Now `Variant::Traced(Shading)` is a one-pipeline set and
   `streamed` is per shading index, each compiled on first use. Switching to the other
   shading later compiles it then (once, then cached). MP1 lists were already per shading.
2. **One kernel in seconds:** 69-74 s single-program, 91-96 s as an MP1 list kernel, about
   5 GB of driver memory while compiling (above).
3. **Hard/soft and streamed/resident as run-time branches.** For cold start this no longer
   matters: only the needed variant compiles. A merged binary would cut the census count,
   not start time. Not measured.
4. **Trust-policy fan-out.** program_gpu.rs compiles the exact bundled standard program as
   engine-owned (unchecked). Any edited text compiles checked. So the first edit of the standard
   program changes the check mode of every kernel it has (24 binaries) instead of only the
   kernels the edit touches (2). Pinning programs to checked would keep an edit at 2. It
   changes the standard program's shipped kernels and needs a frame-time A/B, which was not
   run. The census also notes that GI-cell and glass reflected-hit pipelines compose user
   text through the unchecked engine API. That deserves a fix on its own.

The census tool was not rerun: these changes alter which binaries compile at start, not the
set the composers emit.

## Not measured

- **Quarry.** It cannot start inside the 8 GB child watchdog on either binary. Main reached
  8.5 GB private in 16 s (uncapped compiles plus scene preparation). This branch reached
  8.0 GB private (4.7 GB working set) at 48 s even with a warm cache and almost no compiling,
  so scene preparation or upload staging grows private memory by about 8 GB before the
  first image. That is a separate defect. Root's earlier cold baselines (110 s RT,
  124 s non-RT, main) predate the watchdog.
- **Showcase.** Not built here. Ara's integration thread reruns it from main.
- **Step 3.** spirv-opt is an external development tool (P-PREVIEW), not a shippable path,
  and the kernel split ranks first.

## Reproduction

Tools are under `D:/Coding/renderlab-work/p-pipe/tools` (not in the repository):
`build.py` (admission-first build slot), `lease.py` (n8-run lease at ≥ 12 GB),
`plan.py` (steps under a 0.5 s watchdog: kill at < 8 GB free or > 8 GB child working set or
private; per-step minimum free RAM, peaks, memory CSV and timestamped stderr; strips the forced
validation layer), and `tables.py`. Plans: pipe3 (attribution), pipe4 (test scene and
two-engine cases), pipe5/6 (game scene RT/non-RT), pipe7/8 (main's identity, quarry).

Binaries (SHA-256): candidate `game_scene` `cbe341d4…`, `p_pipe_engines` `989af76f…`,
lib tests `fb0f7509…`; main `game_scene` `909f6423…`. Lib tests in the lease: 207 passed,
0 failed, 167 ignored (GPU tests are ignored in that suite).

Implementation and diagnosis took about 1.5 h of active work. Verification: eight leases,
about 33 min of GPU time, and four builds totalling about 13 min. The first 70 minutes of the
unit were lost to a waiter blocked on a dead 15:08 `ara-vk` request (runner now treats
unreleased requests older than 60 min as stale, like P-STILL).
