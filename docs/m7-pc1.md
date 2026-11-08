# PC1: startup pipeline compilation

PC1 makes the engine's optional persistent Vulkan pipeline cache cover engine kernels
as well as user programs, and loads it before renderer initialization. `game_scene`
enables it by default in the ignored `measurements/pipeline-cache` directory;
`--pipeline-cache DIR` overrides it and `--no-pipeline-cache` disables persistence.
The engine descriptor remains opt-in, so applications choose their writable directory.

## Implementation

- `pipeline_compilation.rs` owns a device facade, shared cache and opt-in CPU timings.
  Each module still goes through wgpu's ordinary WGSL parser and validator. Each
  pipeline uses its explicit cache, or the shared cache when no explicit one exists.
  First-use kernels, mixed-program shading and background reload workers use the same
  facade. No shader source, rendering algorithm, quality setting or material ABI changes.
- `Engine::new` opens the cache before creating renderer pipelines. Program pipelines
  initially share it too; the existing bounded module cache and transactional reload
  behavior remain. The low-level renderer's legacy program-cache setter remains usable.
- Cache filenames include wgpu's Vulkan adapter key, an SHA-256 of the adapter/driver
  information and a format namespace. The payload is bounded to 64 MiB and checksummed;
  reads are bounded even if a file grows concurrently. wgpu additionally validates its
  header and driver identity. Missing, stale, damaged and unreadable data fall back to
  compilation. Source/pipeline-state identity belongs to wgpu and the Vulkan driver;
  editing one source does not invalidate unrelated cached pipelines.
- Normal teardown saves to a unique temporary file and atomically replaces the cache.
  Concurrent engines/processes use different temporary files; the last complete writer
  wins. Cache I/O errors cannot reject a frame. Crashes do not guarantee cache saving.
- `RENDERLAB_STARTUP_PROFILE=1` records module and pipeline wall times. The optional
  `RENDERLAB_STARTUP_PROFILE_DRIVER=1` uses the existing instrumented Vulkan device for
  native driver timings; that device is diagnostic and is not used for ordinary
  startup comparisons.

Small shared-file edits are marked `PC1`. They pass the facade through existing
pipeline-creation helpers in atmosphere, light-cluster setup, primary visibility,
temporal reconstruction, virtual shadows and fresh shadows. Lighting, shadow, GI,
reflection and transparency algorithms are unchanged.

The first cache-disabled RT run found an inherited pipeline-layout mismatch after
MP1: `PrimaryFrame` was 128 bytes but PP1's `post primary surface` layout still declared
112. PC1 exposes the existing frame-size constant to sibling modules and uses it in
that layout. This is a pipeline-creation prerequisite, not a post-processing change.
The failed run produced no first image and is excluded from startup results.

## Measurement contract

`scripts/m7-pc1-run.py` builds with two Cargo jobs into
`D:/Coding/renderlab-work/pc1/target`, and runs headlessly through `scripts/n8-run.py`.
Every GPU/lib-test batch owns the shared atomic lock, uses the foreground-game and
memory guards, and is bounded to 15 minutes. Requests and releases are recorded in
`E:/renderlab-work/gpu-schedule.md`.

The matrix uses `game_scene` at 1280x720 and the same engine example with the registered
13,034,903-triangle quarry at 1920x1080 (the quarry replaces the ground; the example's
props remain). Both paths use their default GI, reconstruction, atmosphere/fog and
post-processing. Two successive animated frames are captured per configuration.

For every scene, path and build, three fresh processes use: persistence disabled,
an empty application cache, then the populated application cache. These are application
cache cold/warm measurements; NVIDIA's private cache and the OS file cache are not
deleted. First-image time runs from entry to `main` through the first completed RGBA8
readback, before PNG encoding. It is an offscreen proxy, not physical scanout latency.
Scene-ready and per-frame CPU render times are recorded separately.

Each captured frame has a SHA-256 of its complete uncompressed RGBA8 output. PNGs are
retained for inspection. The reload probe changes the photoreal program's red response,
renders the edit, rejects invalid WGSL and restores the original program; GI and TR1
are disabled for that isolated reload check.

Commands:

```text
py -3 -B scripts/m7-pc1-run.py build dev
py -3 -B scripts/m7-pc1-run.py build release
py -3 -B scripts/m7-pc1-run.py gpu RUN dev
py -3 -B scripts/m7-pc1-run.py gpu RUN release
py -3 -B scripts/m7-pc1-run.py gpu RUN probe
py -3 -B scripts/m7-pc1-run.py gpu RUN driver
```

## Results

Windows 11 Pro 10.0.26300, AMD Ryzen 7 7800X3D (8 cores / 16 threads), 32 GB RAM,
RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Dev uses the repository's opt-level-1 renderer
and opt-level-2 naga/wgpu dependencies; release uses the optimized release profile.
Measured Rust source is checkpoint `df09611` (the later report commit changes docs only).
Executable SHA-256:

- Dev: `450d5d908efb2ff231e57a482075d252c75c4d584451960d986013fcbbaf4b42`
- Release: `6e7c989b545621d7a57b6193f94b40564db1b70119ebe3b16fb52f2b2b58ec31`

These are **single-process samples per cell in a shared-machine diagnostic run**, not
QUIET-window performance claims. Other units compiled concurrently. In the main
303.5-second lease, 49 of 63 resource snapshots contained compiler processes; minimum
available RAM was 4,652 MiB. No game/window/input/capture automation was used. Vulkan
validation layers were disabled for these timings; wgpu validation remained enabled.

### Startup to first completed image

Seconds, with the same complete output in all three columns:

| Build | Scene | Path | Persistence off | Empty application cache | Populated application cache |
|---|---|---|---:|---:|---:|
| Dev | game | RT | 1.171 | 1.210 | 1.153 |
| Dev | game | non-RT | 1.259 | 1.251 | 1.443 |
| Dev | quarry | RT | 9.627 | 9.498 | 9.479 |
| Dev | quarry | non-RT | 27.273 | 27.281 | 27.306 |
| Release | game | RT | 37.497 | 48.771 | 0.745 |
| Release | game | non-RT | 22.333 | 0.807 | 0.769 |
| Release | quarry | RT | 42.601 | 5.030 | 4.415 |
| Release | quarry | non-RT | 29.199 | 16.571 | 15.236 |

The populated cache avoids the long release pipeline compiles. The RT game pair
reproduces the reported tens-of-seconds startup and brings its next launch below a
second. The empty-cache launch still takes 48.771 s, versus 37.497 s with persistence
off: **cold startup is not solved**. Once NVIDIA's private cache was already warm, the
dev samples show no consistent benefit, including a slower non-RT game sample. Do not
attribute the entire off-to-warm difference to PC1, or infer a stable multiplier from
these samples. The first earlier dev screen also encountered 45.305 s on RT quarry
and 53.406 s on non-RT quarry before subsequent launches warmed the private cache.

First render-call CPU time, empty / populated application cache, ms:

| Build | Scene | RT | non-RT |
|---|---|---:|---:|
| Dev | game | 497.5 / 417.5 | 651.8 / 711.1 |
| Dev | quarry | 538.4 / 511.3 | 1,318.5 / 1,045.9 |
| Release | game | 47,112.4 / 126.4 | 268.4 / 239.4 |
| Release | quarry | 267.6 / 175.9 | 655.9 / 594.5 |

These are first-use costs, not steady frame times. Shader output, render settings and
pipeline counts are unchanged: 51/80 pipelines for game RT/non-RT, 53/82 for quarry.

### Where startup time goes

Release RT game, seconds:

| Boundary | Persistence off | Empty cache | Populated cache |
|---|---:|---:|---:|
| Adapter discovery | 0.221 | 0.232 | 0.237 |
| Device + renderer initialization | 1.692 | 1.076 | 0.179 |
| User program creation / reflection / module validation | 0.139 | 0.165 | 0.172 |
| Remaining scene preparation | 0.016 | 0.017 | 0.015 |
| First render call | 35.221 | 47.112 | 0.126 |
| First completed readback, from main entry | 37.497 | 48.771 | 0.745 |

The two mixed-program list pipelines dominate the uncached first frame: 31.167 and
4.257 s with persistence off; 42.774 and 5.290 s with an empty cache. Complete module
creation/validation is only 0.235 / 0.278 / 0.267 s summed across startup. Compute
pipeline API calls sum to 40.803 / 53.095 / 0.048 s; render pipeline calls sum to
0.086 / 0.047 / 0.003 s. These are aggregate thread durations and can overlap.
Pipeline-call timings include wgpu validation, SPIR-V emission and native driver
compilation; the native-only diagnostic was not run. Cache loading/creation adds
3.3-5.1 ms in the final warm samples. Composed-source construction is included in the
program-creation boundary, rather than measured as its own pass.

Warm quarry preparation, seconds (same full asset on both paths):

| Build / path | Asset read + decode | Asset preparation + upload | Environment read + upload | Total first image |
|---|---:|---:|---:|---:|
| Dev RT | 0.810 | 6.296 | 1.038 | 9.479 |
| Dev non-RT | 0.778 | 23.495 | 1.032 | 27.306 |
| Release RT | 0.801 | 2.273 | 0.573 | 4.415 |
| Release non-RT | 0.773 | 12.525 | 0.563 | 15.236 |

The asset-preparation boundary includes resource creation, CPU geometry/ray preparation,
uploads and acceleration-structure scheduling; it does not separate individual BLAS
builds from other work. GI/environment algorithms were not changed. The remaining
quarry startup is largely in this boundary and is outside this pipeline-cache unit.

### Program edit

Dev RT, two programs, GI and TR1 disabled to isolate reload, same 1280x720 scene:
changing `return exposed` to `return exposed * vec3(0.83, 1, 1)` took **31.335 s from
API edit submission to updated readback**. `reload_program` itself took 95.7 ms; the
new mixed-program list pipeline compiled on the following render call (31.081 s).
Invalid WGSL was rejected; the edited tint kept rendering, and restoring the original
source restored its appearance. This sequence is inspected in `inspect/reload-rt.png`.
It is a program-API proxy and excludes file-watch debounce and physical scanout.
A never-before-compiled edit still has a large hitch; persistence accelerates reuse,
not a genuinely new shader. No claim is made that PC1 fixes that cold-edit cost.

## Verification and evidence

- Final-source lib suite, inside PC1's lease: **186 passed, 0 failed, 154 ignored**,
  3.57 s. Includes damaged/truncated/oversize cache rejection and adapter/driver key
  separation. The earlier retained cache build also passed 186/0.
- Dev and release examples compile; Clippy on library/tests/example passes with 26
  existing test warnings (15 shared with the library), none introduced by PC1.
- All **8 scene/path/build groups**, two matched frame indices each: full RGBA8 hashes
  identical with persistence off, application-cache empty and application-cache warm.
  The earlier four-group dev screen also passes. Final dev and release images also
  match each other at the same frame indices.
- Inspected: `inspect/dev.png`, `inspect/release.png` and `inspect/reload-rt.png`.
  The arch, spheres, crates, hero, decals and scan geometry render on both paths;
  RT reflections and non-RT's different reflection output remain as before. The quarry
  example has a black sky and covers a small quarry area from its game camera; it is
  not the viewer's registered quarry walk. Its full asset preparation is measured.
  The cache comparison establishes unchanged output, not an Unreal quality pass.
- Application cache files were about 2.3-2.8 MB in the first screen. The source-change
  probe revalidates a real changed program and rejects invalid syntax. Runtime driver
  replacement and a GPU corrupt-cache replay were not exercised; corruption handling
  is covered at the CPU cache boundary and uses wgpu's own fallback for stale headers.

Evidence root: `D:/Coding/renderlab-work/pc1/` (ignored/external, not committed):
`runs/dev-b` (initial screen), `runs/final-a-dev`, `runs/final-a-release`,
`runs/final-a-probe/game-rt-probe`, the lease JSON/logs, build/Clippy logs, and inspected
PNGs. Every run directory keeps its executable hash and settings in JSON. Matched
image hashes are in `summary.json` and each example's `run.json`.

Leases: `dev-a-2` (20:31:15-20:32:13 UTC, inherited PP1 layout failure), `dev-b-1`
(20:41:18-20:45:23), `final-a-22` (20:53:32-20:58:35, yielded after a scene process),
and `final-a-210204-16` (21:04:02-21:05:53). All are under 15 minutes. The last lease
was acquired before root-gate-b6: the scheduler incorrectly matched a quoted
`[root order]` mention in another worker's acknowledgment. It was released after the
active process finished. The runner now matches anchored directive/release lines,
checks the exact directive before each subprocess, yields on changes, and resumes
completed configurations only when the executable hash matches. Interrupted next
configurations are excluded. No further GPU leases are needed for PC1.

Implementation took approximately 35 minutes; builds/checks/GPU verification were
interleaved with it and with roughly half an hour of queue waits. The unit occupied
about 75 minutes of wall time, including reporting. GPU leases totaled about 12 minutes;
those lease totals include guard/CPU preparation time. Implementation and verification
are separate costs; these are not steady rendering benchmarks.

No raw GPU buffers were produced. Requested cleanup could not finish: automatic
approval review rejected both the exact-path Cargo-target deletion and bounded
per-file duplicate/cache cleanup, reporting only "blocked by policy". The Cargo target,
measurement cache files and duplicate PNGs remain under `D:/Coding/renderlab-work/pc1/`.
Required summaries and inspected PNGs are present. The retained Cargo target is 3.34 GB. Rendering/startup misses are under **Startup** in
[m7-gaps.md](m7-gaps.md).
