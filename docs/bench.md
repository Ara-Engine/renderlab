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
| `--fixtures walk,grey` | which fixtures (default all but the named-only slab cases) |
| `--shaders <dir>` | the shader directory to watch |

Environment:
- `LIGHTBENCH_HOLD`: the timing-hold directory (default `D:/Coding/renderlab-work/timing-hold`).
- `LIGHTBENCH_REFS`: the reference cache (default `D:/Coding/renderlab-work/bench-refs`).
- `LIGHTBENCH_RUNS`: where the pass log goes (default `D:/Coding/renderlab-work/bench-runs`).
- `LIGHTBENCH_FURNACE_FRAMES`: frames per furnace view (default 64).
- `LIGHTBENCH_WALK_STEP` / `LIGHTBENCH_WALK_FRAMES`: the walking camera's step in metres per frame (default 0.02) and how many frames it walks (default 48).
- `LIGHTBENCH_GPU_LOCK`: the GPU lock whose owner marks a round contended (default `D:/Coding/renderlab-work/gpu-slot.lock`).

**Others' timing rows** (main thread's rule, 20 Nov). The bench takes no GPU lease of its own: a round is about 3 s of tiny frames (320x180, about 2.4 ms each).
- **Idle.** It does no GPU work while the timing-hold belongs to a timing lease or is exclusive, that is, while its `owner.json` lacks `"exclusive":false`. `n8-run.py --timing` and similar leases write no `exclusive` field; an `--exclusive` smoke writes `true`. Meanwhile it prints `lightbench: idle, a timing lease holds the machine: <owner>`. Edits saved during the idle are picked up when it resumes.
- **Running beside a gate.** During ordinary smokes and play gates (`"exclusive":false`) it runs normally. Any fixture line whose run overlapped someone's lease (a timing-hold or `gpu-slot.lock`) ends with `| contended: <owner> <label>`, and its JSON carries `"contended"`. Nobody quotes those milliseconds as clean.
- **What its ms are for.** The bench's ms per pass compare an edit against the round before it on the same process. They are never scoreboard numbers.

## Passes it reloads

| pass | files | rebuild |
|---|---|---|
| `gi` | gi_cache, gi_cells, gi_near, gi_emitters, gi_gather_software, gi_rays_software, gi_rays_hardware, gi_shade_entry (`.wgsl`) | the lighting cache's kernels and every program's cell shading (`Renderer::bench_reload_gi`) |

**Not yet reloadable.** An edit to any other file prints `<file> changed; no bench pass reloads it yet`.

These files reach their kernels through compile-time constants (`include_str!`):
- the program shading sets;
- the reflection passes;
- TR1;
- the shadows;
- `gi_cells.wgsl` and `gi_api.wgsl` as composed into program shading. Note that `gi_cells.wgsl` does reload, but only into the cache's own kernels.

**Adding a pass** takes two pieces:
1. **Override.** Route the pass's sources through `program::gi_shader` (or a function like it), which composes the bench's text when the feature is on.
2. **Rebuild.** Write a `bench_reload_*` that rebuilds the pass's pipelines and keeps the old ones on an error, then add a row to `PASSES` in `src/bench/mod.rs`.

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

**`reference`**, when a fixture has one, renders it once (the live path tracer). The bench caches it at `<refs>/<name>-<hash of scene_key>.lbref`, so every later start loads it. A fixture with an exact answer needs none.

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
| `grey`, `orange` | gicut (src/gi_furnace_tests.rs) | closed emitting box, exact L = Le / (1 - rho) | `energy`, `err`, `off5` |
| `slab` (named only) | gicut | thin slab over an emitter under a sky; the top reads 0.5 | `energy`, `err`, `off5` |
| `slab-roof` (named only) | gicut | the slab under a roof and a black sky; top and roof read 0 | `top.mean`, `top.leak_max`, `roof.*` |
| shadow edges (coming) | clipmap | rail edge, contact edge, thin caster | `lit_in_shadow`, `rail_wobble_px`, `contact_gap_px` |

**Known bug.** The slab cases read black on the bench, where the furnace test reads 0.5 on the top face. So they run only when `--fixtures` names them.

The furnace fixtures start each run from an empty lighting cache (`reset_indirect`) and render `LIGHTBENCH_FURNACE_FRAMES` frames per view. The default is 64 against the GPU test's 300, so their numbers are the bench's, not the test's.
