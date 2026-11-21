# The test bench (`lightbench`)

One headless process that stays open. It creates a device for each path, with and without ray hardware, once, and builds every fixture on its own renderer on each path. Then it watches the shader sources. When a file changes, it rebuilds only the pass that composes that file, reruns every fixture, and prints one line per fixture and path:

```
bench walk nort mean=0.8799 energy=0.8799 err=0.1201 off5=1.0000 | cache ms/frame median 0.21 max 0.28 over 48 frames, span 1.03 | ms gi_cell_shading=0.02 gi_claim=0.04 ... | 0.12 s
{"bench":"lightbench","round":3,"fixture":"walk","path":"nort","metrics":{...},"pass_ms":{...},"cache_ms_frames":[...],"contended":null,"run_s":0.121}
lightbench: reloaded in 0.87 s: 72 pipelines in 0.86 s, longest gi_gather_batched 0.77 s
lightbench: round 3: save to numbers 1.47 s (reload gi 0.87 s, fixtures 0.54 s)
```

The JSON line is the machine-readable copy for agents (`grep '^{"bench"'`).
- **`cache ms/frame`** sums every `gi_*` stage, the lighting cache's whole cost. `cache_ms_frames` lists it frame by frame: `null` where a frame compiled.
- **The window** is the fixture's measured frames: for `walk`, the walking frames only.
- **`save to numbers`** times the whole loop, from the file's save time to the last printed line.

When a new text fails to compile, the pass keeps its last working pipelines. The bench prints the error and goes back to the last good text for that file. The error's line numbers are in the composed module (`wgsl:1332`), not in the file.

## Measured edit to numbers (20 Nov, RTX PC, release bench, three furnace fixtures)

Each edit gives the first float literal on a code line new trailing digits (`0.5` → `0.5000123`), so the driver has never seen the SPIR-V and the compile is cold. A comment edit changes no SPIR-V in a release build, so the driver's cache serves it in about 0.1 s. Times run from the save to the last printed line.

| file edited | nort only | nort and RT | longest kernel recompiled |
|---|---:|---:|---|
| gi_cache.wgsl | 0.72 s | 1.39 s | gi_scan 0.08 s (the edit reached few kernels) |
| gi_cells.wgsl | 0.76 s | 1.44 s | gi_probe 0.28 s |
| gi_near.wgsl | 0.49 s | | (no kernel changed) |
| gi_emitters.wgsl | 0.51 s | | gi_emitters_build 0.02 s |
| gi_gather_software.wgsl | 1.47 s | 1.76 s | gi_gather_batched 0.77 s |
| gi_rays_software.wgsl | 1.33 s | 1.76 s | gi_gather_batched 0.78 s |
| gi_rays_hardware.wgsl | | 1.51 s | |
| gi_shade_entry.wgsl (cell shading) | 0.82 s | 1.50 s | gi_shade_cells 0.28 s |
| a shader error (kept the old pipelines) | 0.54 s | 0.94 s | none |

**Fixture time.** A round of the three fixtures takes 0.3-0.5 s on nort and 0.6-1.0 s on both paths. It is CPU-bound in a dev build (1.1-1.7 s), so run the release build.

**Reload.** It builds the cache's kernels together under four compile slots. Done one at a time, the same reload took 4.1-4.4 s on a dev build.

These numbers were taken with smokes running beside the bench.

## Running it

```sh
# build (release: the fixtures' frames are CPU-bound in a dev build; own target on D:)
CARGO_TARGET_DIR=D:/Coding/renderlab-work/targets/<you>-bench cargo build --release --features bench --bin lightbench -j 4
# copy it out of the target, then run it headless; the RT path needs the acknowledgment every RT test needs
RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK lightbench.exe
```

- The name does not start with `renderlab`, so clean-ups that stop `renderlab*` test executables leave it alone.
- It watches the `shaders/` directory of the checkout it was built from: `CARGO_MANIFEST_DIR`, or `--shaders <dir>`.
- At start-up it reads the bench passes' files from disk, so edits made before it started count.

| option | |
|---|---|
| `--once` | run every fixture once and exit (scripts, CI) |
| `--paths nort,rt` | which paths (default `nort`: the walking-cost goal's path) |
| `--fixtures walk,grey` | which fixtures (default all but the named-only real-size scenes `city` and `terrain`) |
| `--shaders <dir>` | the shader directory to watch |

Environment:
- `LIGHTBENCH_HOLD`: the timing-hold directory (default `D:/Coding/renderlab-work/timing-hold`).
- `LIGHTBENCH_REFS`: the reference cache (default `D:/Coding/renderlab-work/bench-refs`).
- `LIGHTBENCH_RUNS`: where the pass log goes (default `D:/Coding/renderlab-work/bench-runs`).
- `LIGHTBENCH_FURNACE_FRAMES`: frames per furnace view (default 64).
- `LIGHTBENCH_WALK_STEP` / `LIGHTBENCH_WALK_FRAMES`: the walking camera's step in metres per frame (default 0.02) and how many frames it walks (default 48).
- `LIGHTBENCH_SHADOW_FRAMES` / `LIGHTBENCH_SHADOW_DUMP=<dir>`: the shadow fixture's frames per run (default 8), and a directory for its measured, exact and unshadowed images as PGM.
- `LIGHTBENCH_GPU_LOCK`: the GPU lock whose owner marks a round contended (default `D:/Coding/renderlab-work/gpu-slot.lock`).
- `LIGHTBENCH_CACHE`: the pipeline cache directory, one cache per path shared by its renderers (default `D:/Coding/renderlab-work/bench-cache`).
- `LIGHTBENCH_REFLECT_FRAMES`, `LIGHTBENCH_GLASS_FRAMES`: frames per run of `reflect` and `glass` (default 32 each).
- `LIGHTBENCH_TR1_STEP` / `LIGHTBENCH_TR1_FRAMES`: `tr1-edge`'s slide in metres per frame (default 0.03) and its frames (default 40).
- `LIGHTBENCH_FIXTURE_DUMP=<dir>`: `reflect`, `glass` and `tr1-edge` write their frames, references and clean frames there as PFM.

**Others' timing rows** (main thread's rule, 21 Nov; it replaces the 20 Nov rule that let the bench run beside gates). The bench takes no GPU lease of its own. It does no GPU work under a gate.
- **Why.** On 21 Nov, a bench beside a play gate read slowed every pass of the read 2.5-2.6x when the fixture was `glint-course`, and about 10% with the small fixtures. 10% of a 5 ms clip row is 0.5 ms, above the gates' 0.3 ms line.
- **Idle.** Before each round, the bench checks the timing-hold and the GPU lock. It waits while either `owner.json` names a `gate` (the clip gate `smoke`, the play gate `play`, or `scoreboard`, which write it through `smoke.Lease(..., gate=...)` on their verdict and judging leases alike). It also waits while the timing-hold belongs to a timing lease or is exclusive, that is, while its `owner.json` lacks `"exclusive":false`. `n8-run.py --timing` and similar leases write no `exclusive` field; an `--exclusive` smoke writes `true`. Meanwhile it prints `lightbench: idle: <owner>'s gate (<tool>: <label>)` or `lightbench: idle: a timing lease: <owner> <label>`. When the hold clears, it prints `lightbench: resuming after N s idle`. Edits saved during the idle are picked up when it resumes. A round already running when a gate starts finishes first; a round is a second or two.
- **Running beside other leases.** Ordinary leases without a `gate` (one-off scripts) leave it running. Any fixture line whose run overlapped someone's lease (a timing-hold or `gpu-slot.lock`) ends with `| contended: <owner> <label>`, and its JSON carries `"contended"`. Nobody quotes those milliseconds as clean.
- **What its ms are for.** The bench's ms per pass compare an edit against the round before it on the same process. They are never scoreboard numbers.
- **Cause line.** A fixture whose series carry the cause counters (the dump fixtures) prints `bench <fixture> <path> cause | NEW .. | MOVER .. | REFRESH .. | GATHERED .. | screen .. | other ..` (src/bench/cause.rs, agreed with gicut and shade, 21 Nov): per measured frame the cells each cause traced and shaded, and its estimated ms. NEW is cells created and untraced or partial cells traced; MOVER is cells a moved caster invalidated or reshaded; REFRESH is the refresh rotation and relights; GATHERED is cells shaded after their gather. After-trace shades take the trace split. `gi_update` splits by traces, `gi_claim` goes to NEW, `gi_cell_shading` and `gi_integrate` (mostly the cell filter, nine items per shaded cell) split by shades, `gi_gather` goes to GATHERED, and the probe and receivers are the screen's. The counters are same-address atomics on the cache's queue paths, so default frames carry none: src/gi_causes.rs composes them into the kernels' text only with `RENDERLAB_GI_CAUSES=1` (the bench sets it; `LIGHTBENCH_CAUSES=0` runs the default kernels, and the line then says `off`) or `RENDERLAB_GI_LOG`.
- **GPU clock.** Each round ends with `lightbench: round N GPU clock median M MHz, lowest L` (nvidia-smi sampled every 200 ms through the round; nothing on other GPUs). A round whose clock sat low was a slow-state round: rerun it rather than quote it.

## Passes it reloads

| pass | files | rebuild | fixtures run |
|---|---|---|---|
| `gi` | gi_cache, gi_cells, gi_near, gi_emitters, gi_gather_software, gi_rays_software, gi_rays_hardware, gi_shade_entry (`.wgsl`) | the lighting cache's kernels and every program's cell shading (`Renderer::bench_reload_gi`) | all |
| `vsm` | virtual_shadow, virtual_shadow_records, virtual_shadow_hybrid, virtual_shadow_lookup, virtual_shadow_disk (`.wgsl`) | the virtual sun shadows' pipelines (`Renderer::bench_reload_vsm`). The composed modules are checked with naga first, then a new set of pipelines replaces the old one; the view keeps its pages and bind groups. | all |
| `shading` | scene, program_api, program_traced*, program_view, program_bins, programs/standard, programs/toon, light_cluster_api, light_vis_pass, decals, environment_lobes, specular_filter, shadow_visibility_filter, gi_api | the prelude rebuild (below) | `grey`, `orange`, `shadow-edges`, `reflect` |
| `reflections` | reflections*, reflection_cache_hits, reflection_hit_bins, reflection_hit_sun, reflection_hits | the prelude rebuild | `reflect` |
| `transparency` | transparency, transparency_shadow, id_glass, pane_pixels, transmission_reduce | the prelude rebuild | `glass` |
| `gi_receiver` | gi_receiver_pass | the prelude rebuild | `walk`, `grey` |
| `tr1` | temporal_resolve, temporal_zones, temporal_coverage_cycle, temporal_coverage_pack, temporal_view | the prelude rebuild | `tr1-edge` |
| `visibility` | primary_rays, primary_trace, primary_interactive, primary_edge_check, primary_edge_queue, visibility_buffer* | the prelude rebuild | `shadow-edges`, `tr1-edge` |

**Which fixtures run.** After a reload only the pass's fixtures rebuild and run. Every other rig records the pass as stale (`Rig::stale`) and rebuilds it before it next runs. The rigs rebuild at once, one thread per renderer, so both paths compile together.

**The prelude passes' text** (src/bench/edits.rs). A saved file becomes hunks against the text the bench started with: each changed run of lines, widened by whole lines of context until it occurs once in that file. Every shader module the process creates (`pipeline_compilation::shader_module`) has them applied wherever their old text occurs. These passes compose their files through constants, slices and seams; the hunks reach whatever survives of the edited lines, with no per-site plumbing.
- **Checked first.** A module the hunks change is checked with naga before the device sees it. When it fails, the module is created from the last working hunks and the bench prints the error.
- **Reached modules.** After each round the bench prints how many modules each edit reached, counted after the fixtures' frames because some kernels compile on first use. When it is 0, it prints a warning: the numbers are the old kernels'. A line the composition replaces through a seam reaches none.
- **The starting text.** Hunks are taken against the files as the bench starts. They match the binary's built-in text only when the two agree, so start the bench from the checkout it was built from.

**The prelude rebuild** (src/bench/reload.rs). Their kernels all compose the traced heads or the prelude, and a file of one pass can compose into another's kernels (the reflection and TR1 view files go into program shading). So one rebuild serves all six:
1. The engine's own prelude reload (`Renderer::reload` with the installed prelude): the engine pipelines and every program's modules in one transaction.
2. The hashes that key cached pipelines are salted, so the receiver's, transparency's and the visibility buffer's caches miss. The reflection trace and every program's cell shading are dropped.
3. `prepare_view_pipelines` builds what the programs of the fixture's last frame draw with.
4. Their lean shading is compiled and waited for. A lean compile running from before the reload is dropped: its text is the old one, and its digests cannot tell.

Kernels whose SPIR-V did not change are cache hits.

**One pipeline cache per path.** The bench's devices are the engine's own (`engine::device::request_device`: the path's features and limits, so the reflection pass and its composite run as in the game). Each path's facade holds one pipeline cache for all its renderers, saved under `LIGHTBENCH_CACHE` after each round.
- A revert, an A/B toggle or a restart is a cache hit.
- Startup with a warm cache: 10 fixtures on 2 paths in 26 s, against 126 s for three fixtures before. Three fixtures alone take 1.7 s.

**Measured (21 Nov, RTX PC, release bench).**
- **Setup:** one process with `grey,orange,walk,shadow-edges,reflect,glass,tr1-edge`. Each edit gives a float literal on a code line new digits (cold: new SPIR-V, new digits each run), then reverts.
- **Contention:** other agents' non-exclusive smokes were running.
- **Driver script:** `main-thread/clipmap/bench/pass_times.py`.

| pass (file edited) | modules reached | save to numbers, nort | nort and RT | reload | longest kernel |
|---|---:|---:|---:|---:|---|
| `shading` (programs/standard.wgsl) | 28 / 64 | 6.0-7.5 s | 7.2-8.3 s | 5.5-7.5 s | traced program shading 2.6-3.1 s |
| `reflections` (reflections_trace.wgsl) | 2 | | 0.8 s | 0.5 s | reflection trace 0.19 s |
| `reflections` (reflection_hits.wgsl) | 14 | | 8.8 s | 8.5 s | the hit shading composes the program: traced program shading 3.3 s |
| (reflections_resolve.wgsl) | 0 | composed only with `RENDERLAB_RL_RESOLVE_PASS=1` (off): the bench rightly warns | | | |
| `transparency` (transparency.wgsl) | 4 | 2.9 s | 3.9 s | 0.3 s | TP1 pipelines compile in the fixture's first frame |
| `gi_receiver` (gi_receiver_pass.wgsl) | 70 / 140 | 4.2 s | 7.9 s | 1.3-1.4 s | the receiver composes into traced shading; `walk` takes 2.9-6.4 s |
| `tr1` (temporal_resolve.wgsl) | 5 / 10 | 2.1 s | 3.9 s | 1.0-1.2 s | tr1_resolve 0.7-0.9 s |
| `visibility` (visibility_buffer_resolve.wgsl) | 12 | 2.0 s | 3.3 s | 0.7-0.8 s | resolve 0.2 s |
| `visibility` (primary_trace.wgsl) | 4 / 10 | 1.0 s | 3.0 s | 0.7-2.4 s | primary_trace_interactive 0.56 s |
| a revert (cache hit) | | 0.4-0.9 s | 0.6-1.3 s | 0.2-0.4 s | |

**Under 2 s?** Edits that reach only their own pass's kernels are: TR1, visibility and reverts.
- **Program shading:** its floor is one cold compile of the traced program shading kernel, 2.5-3 s on this driver with both paths compiling.
- **The receiver:** it composes into that kernel, so it pays the same compile.
- **Transparency:** its pipelines compile in the fixture's first frame. They are built by frames, not by `prepare_view_pipelines` (`prewarm_transparency` needs the materials, which the renderer does not keep).
- **One outlier:** a program-shading revert once compiled cold (6.3 s) where the others were cache hits (0.9-1.3 s).

**Adding a pass** is a row in `PASSES` (src/bench/mod.rs): its files, its rebuild and the fixtures that read it.
- A pass whose kernels compose the prelude or the traced heads takes `edits: true` and `Renderer::bench_reload_prelude`.
- A pass with its own pipelines takes a `bench_reload_*` that rebuilds them and keeps the old ones on an error, with its sources routed through `program::engine_shader` or `gi_shader` (`edits: false`).

## Adding a fixture

A fixture is one file under `src/bench/` and one line in `src/bench/fixtures.rs`. It implements:

```rust
pub(crate) trait Fixture {
    fn name(&self) -> &str;                       // printed on every line; unique
    fn scene_key(&self) -> String;                // optional: hashed into the reference cache's file name
    fn load(&mut self, r: &mut Renderer, path: BenchPath) -> Result<(), Error>;
    fn run(&mut self, r: &mut Renderer, path: BenchPath, reference: Option<&RefImage>)
        -> Result<Vec<(String, f64)>, Error>;
    fn reference(&mut self, r: &mut Renderer) -> Option<Result<RefImage, Error>>; // optional
}
```

**`load`** builds the scene once on this fixture's renderer on `path`. The bench calls `fixtures()` once per path, so each path's instance keeps its own handles.

**`run`** renders the fixture's frames and returns `name=value` metrics, printed in that order. Two names have fixed meanings:
- `err`: error against the reference.
- `energy`: ratio to the exact answer, where one exists.

The bench adds the per-pass GPU milliseconds itself. They are medians over the run's frames, from the renderer's pass log, leaving out frames that compiled. Keep a run well under a second per path.

**`reference`**, when a fixture has one, renders it once (the live path tracer). `kit::traced_reference` traces it on a renderer of its own with ray hardware, so a fixture on either path gets one. The bench caches it at `<refs>/<name>-<hash of scene_key>.lbref`, so every later start loads it. A fixture with an exact answer needs none.

**Shared helpers** are in `src/bench/kit.rs`:
- `part`: a mesh from quads, with its instance and material;
- `box_faces` and `quad`;
- `frame`: one lit frame, waiting for admission;
- `capture_radiance`: the HDR read back as cd/m²;
- `color_target`;
- `drain`;
- `luminance`.

A fixture is a child of the `renderer` module, so crate-internal renderer state is in reach for scorers that need it.

## Fixtures

| fixture | owner | scene | metrics |
|---|---|---|---|
| `walk` | gicut | the grey box, a camera walking 0.02 m per frame for 48 frames after 64 still | `energy`, `err`, `off5` on the last frame; cache ms per walking frame |
| `walk-wrap` | gicut | `walk` with the cache's frame stamp started so the walk crosses frame 4096 (a 12-bit stamp's wrap) 24 frames in | as `walk` |
| `city` (named only) | gicut, shade (src/bench/city.rs) | a closed furnace room (exact L = 1) holding 3,000 instanced boxes and 150 instanced 320-triangle rocks (85,548 triangles, 3,153 placements), two skinned columns swung each frame, a 1 lux sun the room occludes (`LIGHTBENCH_CITY_SUN_LUX`); camera walking 0.05 m per frame for 64 frames after 32 still | `energy`, `err`, `off5`; `geometry_bytes` (the cache's software geometry); `triangles`, `placements`; per walking frame `gather_rays`, `light_rays`, `aimed_rays` (series); cache ms per walking frame |
| `terrain` (named only) | gicut, shade | the same room over one 320 x 320 heightfield (204,800 triangles in one mesh) | as `city` |
| `glint-course` (named only) | gicut, shade (src/bench/dump.rs) | Glint from a scene dump on disk (`LIGHTBENCH_GLINT`, default D:/Coding/renderlab-work/bench-scenes/glint-goal: `scene.rla`, `scene.json`, the sky map; not committed), 137,084 triangles, its sun, local lights, sky map and emitting materials, at 1920x1080; the ball's course as the bot rolls it (53 m) at 9.5 m/s in 33 Hz frames under Glint's course camera | the cache's ms per walking frame split into trace (update, claim, probe), gather, integrate; rays, gather rays, light rays, cells created and queued, cells shaded, integrated, traced and gathered, invalidated and reshaded per walking frame |
| `glint` (named only) | gicut | the same dump, walking 13.4 m at 4.5 m/s into the dump's own view, then holding 90 frames there | against the dump's 16,384-sample path-traced reference (`reference.pfm`): `arrive_energy`, `arrive_err`, `arrive_region` (the view's central 20%), the same on the last held frame, `converge_frames` (held frames until that region is within 2%, -1 if never); the counters as `glint-course` |
| `outpost`, `glint-play` (named only) | shade, gicut (src/bench/dump.rs, replay.rs) | the play gate's own Outpost and Glint sessions from scene dumps armed with `motion` (`LIGHTBENCH_OUTPOST`, `LIGHTBENCH_GLINT_PLAY`; default D:/Coding/renderlab-work/bench-scenes/outpost-play and glint-play; not committed; made by `u-sunrt/bench/motion_dump.py`: seed 7, the dump at tick 1200, then the gate's 300 still ticks and 360 ticks of Move forward, one play.step per tick), replayed one play step a frame after 60 settle frames: the camera, every placement that moved (Outpost's player, Glint's ball) and every skinned mesh posed as the game posed it (drawn as posable instances, so the cache refits and patches them per pose). TR1 on, as the engine runs it. An atmosphere sky (Outpost's) is drawn by the product atmosphere, the sun at its top-of-atmosphere lux. `LIGHTBENCH_DUMP_WINDOW=still` measures the still steps instead of the walk. 21 Nov, Outpost without ray hardware, `RENDERLAB_GI_FIXED=32768`, against the game's own counters on the same session (RENDERLAB_GI_LOG): standing within about 10% (traces 1,658 against 1,631, mover shades 12,236 against 13,244, 4 moved boxes each); walking short (created 3,856 against 8,084, invalidated 2,782 against 7,398, mover shades 16,714 against 80,892), with 5 moved boxes against the game's 7: two moving things the dump does not carry (it skips see-through instances) | the cache's ms per walking frame split as `glint-course`, the counters as `glint-course` plus `refreshed`, `trace_partial`, the five shade causes and `moved_boxes`, and the cause line |
| `grey`, `orange` | gicut (src/gi_furnace_tests.rs) | closed emitting box, exact L = Le / (1 - rho) | `energy`, `err`, `off5` |
| `slab` | gicut | thin slab over an emitter under a sky; the top reads 0.5 | `energy`, `err`, `off5` |
| `slab-roof` | gicut | the slab under a roof and a black sky; top and roof read 0 | `top.mean`, `top.leak_max`, `roof.*` |
| `shadow-edges` | clipmap (src/bench/shadow_edges.rs) | a grey floor under a soft sun (0.02 rad), the sun only: a 3 m rail 0.9 m up turned against the map's grid, a 0.3 m block resting on the floor, a 5 mm wire; 256x256, `LIGHTBENCH_SHADOW_FRAMES` frames (default 8) with TR1 | against the exact answer (the sun's disk cast on the CPU from each floor pixel, once at load): `err` over the edge band, `lit_in_shadow`, `rail_wobble_px`, `contact_gap_px`, `thin_missing`; `mask_mismatch` checks the camera |
| `reflect` | clipmap (src/bench/reflect.rs) | a mirror (metal, roughness 0.02) and a copper sphere (metal, roughness 0.3) over a grey floor with a red box, sun 1,000 lux and a uniform sky of 20; 192x144, the lighting cache and TR1 on, 32 frames from a cut and an empty cache | against the live path tracer (16,384 paths, cached on disk): `err` and `energy` over both objects, `mirror.*`, `sphere.*`, `floor.err` |
| `glass` | clipmap (src/bench/glass.rs) | a tinted glass pane (index 1.5) and a blue film (index 1, half coverage) over a grey floor before a red and a white wall; 192x144, 32 frames as `reflect` | against the live path tracer: `err`, `energy`, `glass.*`, `film.*`, `shadow.err` (the floor in their sun shadows) |
| `tr1-edge` | clipmap (src/bench/tr1_edge.rs) | a white box sliding 0.03 m per frame (about 2 px) for 40 frames over a dark floor, then a camera jump without `cut_reconstruction`; 256x160, the lighting cache off | against clean frames rendered still at load: `pass_ghost_px` (median, over five lines, of how far behind the box the floor still differs by over 10%), `pass_ghost_max_px`, `cut_ghost_px` (pixels differing on the first frame after the jump), `cut_frames` (frames until under 0.2% differ; 17 = not within 16) |

**Real size, checked against the game (20 Nov).** The play gate's 1080p Glint course reads, without ray hardware:
- gather 11.38 ms;
- update 4.27 ms, plus claim and probe;
- integrate 0.74 ms;
- a 30.7 ms frame.

`glint-course` on the bench, without ray hardware, beside other leases:
- trace 5.1 ms;
- gather 7.3 ms;
- integrate 0.69 ms;
- 14.4 ms of cache in an 18.7 ms span.

So the bench is in the game's range. A round under heavier contention read 31.5 ms of cache. The counts repeat exactly round to round: 12.2k cells created, 573k rays (382k gather) and 14k queued per frame. The milliseconds move with whatever else holds the GPU.

With ray hardware the cache is 3.4 ms (trace 1.0, gather 0.66, integrate 0.69). A round takes 5-11 s without ray hardware and 2.3 s with it. The goal walk (`glint`) takes 7-28 s, because it reads the HDR back every held frame until its region converges.

The box city scenes (`city`, `terrain`) read 2 ms of cache. They are fast checks, not real size.

**Atmosphere.** The furnace and real-size fixtures turn the atmosphere off, as lib tests start (`Atmosphere::off`). The default physical sky, which every non-test build gets, is black without a sun, and it puts height and volumetric fog into the rooms. Under it the slab's top read 0 instead of 0.5, and the boxes read about 0.4% darker.

The furnace fixtures start each run from an empty lighting cache (`reset_indirect`) and render `LIGHTBENCH_FURNACE_FRAMES` frames per view. The default is 64 against the GPU test's 300, so their numbers are the bench's, not the test's.
