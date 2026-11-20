# The test bench (`lightbench`)

One headless process that stays open. It creates a device for each path, with and without ray hardware, once, and builds every fixture on its own renderer on each path. Then it watches the shader sources. When a file changes, it rebuilds only the pass that composes that file, reruns every fixture, and prints one line per fixture and path:

```
bench grey nort energy=0.9981 err=0.0123 off5=0.0012 | ms gi_gather=0.08 ... span=1.23 | 0.41 s
{"bench":"lightbench","round":3,"fixture":"grey","path":"nort","metrics":{...},"pass_ms":{...},"run_s":0.412}
lightbench: round 3: save to numbers 1.84 s (reload gi 0.91 s, fixtures 0.88 s)
```

The second line is the machine-readable copy for the scoreboard and agents (`grep '^{"bench"'`). The last line times the whole loop, from the file's save time to the last printed line.

When a new text fails to compile, the pass keeps its last working pipelines. The bench prints the error and goes back to the last good text for that file.

## Running it

```sh
# build (dev profile, own target on D:)
CARGO_TARGET_DIR=D:/Coding/renderlab-work/targets/<you>-bench cargo build --features bench --bin lightbench -j 4
# copy it out of the target, then run it headless; the RT path needs the acknowledgment every RT test needs
RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK lightbench.exe
```

- The name does not start with `renderlab`, so clean-ups that stop `renderlab*` test executables leave it alone.
- It watches the `shaders/` directory of the checkout it was built from: `CARGO_MANIFEST_DIR`, or `--shaders <dir>`.
- At start-up it reads the bench passes' files from disk, so edits made before it started count.

| option | |
|---|---|
| `--once` | run every fixture once and exit (scripts, CI) |
| `--paths rt,nort` | which paths (default both) |
| `--fixtures grey,slab` | which fixtures (default all) |
| `--shaders <dir>` | the shader directory to watch |

Environment:
- `LIGHTBENCH_HOLD`: the timing-hold directory (default `D:/Coding/renderlab-work/timing-hold`).
- `LIGHTBENCH_REFS`: the reference cache (default `D:/Coding/renderlab-work/bench-refs`).
- `LIGHTBENCH_RUNS`: where the pass log goes (default `D:/Coding/renderlab-work/bench-runs`).
- `LIGHTBENCH_FURNACE_FRAMES`: frames per furnace view (default 64).

**Others' timing rows.** While a timing-hold exists, the bench does no GPU work. It prints `lightbench: idle, a timing-hold is held: <owner>` and resumes when the hold is gone; edits saved meanwhile are picked up then. The bench takes no GPU lease of its own: its work is a few milliseconds of tiny scenes at a time.

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
| `grey`, `orange` | gicut (src/gi_furnace_tests.rs) | closed emitting box, exact L = Le / (1 - rho) | `energy`, `err`, `off5` |
| `slab` | gicut | thin slab over an emitter under a sky; the top reads 0.5 | `energy`, `err`, `off5` |
| `slab-roof` | gicut | the slab under a roof and a black sky; top and roof read 0 | `top.leak_mean`, `top.leak_max`, `roof.*` |
| shadow edges (coming) | clipmap | rail edge, contact edge, thin caster | `lit_in_shadow`, `rail_wobble_px`, `contact_gap_px` |

The furnace fixtures start each run from an empty lighting cache (`reset_indirect`) and render `LIGHTBENCH_FURNACE_FRAMES` frames per view. The default is 64 against the GPU test's 300, so their numbers are the bench's, not the test's.
