# Parallel runtime receiver adjacency

The runtime-only follow-up to the rejected `Cooked-adjacency` decision. Source
asset encoding, shader code and the packed GPU suffix remain unchanged. The
quarry remains 12 meshes / 13,034,903 triangles; no cooked adjacency is required.

## Implementation

`receiver_adjacency::build` sorts exact position-bit records in parallel, with
signed zero canonicalized as before. Each welded group uses its lowest original
vertex index. It reuses the same allocation for undirected edge records and
sorts those in parallel too. Two-entry edge groups retain the previous opposite
vertex/triangle checks; boundary, ambiguous and degenerate edges retain the
missing-neighbour fallback. Equal-key sort order cannot affect any table entry.

`Renderer::prepare_mesh_uploads` validates and moves owned asset primitives into
immutable `MeshUpload` resources, building eligible tables across meshes in the
same Rayon pool. Indexed collection preserves the original primitive order.
The regular single-mesh upload API uses the same parallel builder. Upload still
checks renderer/device limits and falls back when the suffix does not fit.
Geometry and table cannot be mismatched through the public preparation API.

The prepared viewer moves vectors instead of cloning geometry. It uploads in
the original order and drops each CPU resource after acceptance; Busy retains
the pending resource. Suffix bytes go straight into wgpu's write-only staging
view, eliminating the temporary byte Vec and its padding reallocation.

Rayon was absent from the workspace. The dependency supplies nested work
stealing and in-place parallel unstable sorting without per-mesh thread pools,
unsafe scatter or merge scratch. Cargo.lock adds Rayon, rayon-core, either,
crossbeam-deque and crossbeam-epoch; crossbeam-utils was already present.
Scratch is one 16-byte record per max(vertex count, index count), one u32 per
vertex and the unchanged output table. Parallel mesh preparation raises peak
simultaneous scratch relative to serial upload; completed scratch is released
immediately, and the 4/2 GiB guard covers the registered run.

## Verification and measurements

Evidence: `E:/renderlab-work/parallel-adjacency/`. The guarded batch uses
`scripts/parallel-adjacency-run.py`, its retained `check.py` and `batch.json`,
the shared `D:/Coding/renderlab-work/gpu-slot.lock`, and reservations/releases
in `E:/renderlab-work/gpu-schedule.md`. All Cargo commands use the worker's
`E:/renderlab-work/parallel-adjacency/target` and `-j 2`.

The CPU oracle retains the previous HashMap/sort builder under `cfg(test)`.
It loads the original quarry once, compares all neighbours and all three edge
counts, takes eight alternating serial/parallel wall samples, and checks
identical tables at 1, 2, 4, 8 and 16 threads. Timing includes output allocation
and scratch destruction; comparisons are outside timed regions.

The offscreen GPU test takes one legacy/parallel startup pair. The test-only
legacy control builds each table at its upload as before; the candidate uses
batched preparation. Both use the same current buffer-packing/upload code.
Startup starts before renderer creation and ends after the first completed
1920x1080 Interactive image, including environment/file decode/upload/BLAS
work. It excludes test-process launch and is not a native-window measurement.
Both loads use the original asset; OS file caches are uncontrolled.

The gate compares full HDR, hit and soft visibility hashes at three poses in
Hard, Exact and Interactive modes. Each configuration runs eight frames, with
subsequent repeats compared on GPU using a 12-byte mismatch readback. It then
checks all nine registered 1024-square receiver micro-grids on the same loaded
candidate. Shader output or quality reductions are not part of this change.

Hardware: Ryzen 7 7800X3D (8 cores / 16 logical processors), RTX 4070 SUPER,
NVIDIA 617.14, Windows/Vulkan, rustc 1.96.0, release build, validation enabled.
Base `5d339d9`; source and executable hashes are retained with the evidence.

| Measurement | Legacy runtime | Parallel runtime |
|---|---:|---:|
| CPU build median, eight alternating pairs | 2173.837 ms | 353.186 ms |
| CPU build observed range | 2057.294–2600.773 ms | 335.581–422.335 ms |
| In-upload adjacency build / batch preparation | 2250.440 ms | 557.988 ms |
| Prepared upload wall, including the above | 3097.571 ms | 1719.948 ms |
| Environment + asset loading/upload | 5446.283 ms | 3733.905 ms |
| Startup to first completed offscreen image | 9943.990 ms | 7395.235 ms |

The CPU median improves **6.15x**, saving **1820.651 ms** on this host. The
startup pair improves by **2548.755 ms**, but is only one pair with uncontrolled
OS/driver caches and compiler contention: do not attribute all of that change
to adjacency or treat it as a stable native-viewer startup speedup. The legacy
control uses the frozen builder and current staging code, not a pristine old
executable. No quiet-window performance certification is claimed.

All tables and edge counts match at 1/2/4/8/16 threads. Single samples for those
pool sizes are respectively 2566.034 / 1303.414 / 891.254 / 586.716 / 409.464 ms.
The one-thread sort implementation is slower than the legacy median; the
demonstrated gain depends on parallel execution. The GPU gate passes all
18 legacy/candidate configurations, eight repeats each, and all nine micro-grids
with zero opposite/corner samples. Validation: zero errors, active layers in
both renderer instances. Inspected all three saved Interactive PNGs; the full
quarry, silhouettes and contacts are intact, with exact image identity providing
the regression evidence. This is no broader film-quality or motion acceptance.

The original asset remains **1,548,588,300 bytes**; no asset is written. Geometry
upload accounting remains **631,261,656 bytes** in both runs, including the same
**156,418,836-byte** adjacency suffix/padding. No large raw GPU dumps were saved.

Final lease: **2026-11-06 00:36:21–00:39:00 UTC**, **158.522 s**, released.
Preflight: **6239 MiB** available; 10 Hz recorded minimum: **2260.527 MiB**,
above the 2048 MiB abort floor. Unrelated nightly compilation remained active:
CPU-phase rustc count 0–5; GPU phase 1–5. No game/other renderer was detected,
and no memory abort occurred. Phase ranges use the check log creation/write
times to select resource samples; all raw samples remain in the run JSON.

Development time is distinct from runtime: implementation/fixes took roughly
5.5 minutes excluding checks after initial inspection. The first cold compilation
attempt took 126.398 s and failed on a write-only wgpu API mismatch and missing
test-helper visibility. After correction, retained-source library compilation
and tests took 54.807 s (test execution **0.89 s**, **99 passed / 95 ignored**,
exit **0**); Clippy took **38.021 s**, the all-quarry CPU test **34.291 s**, and
the single GPU gate **26.793 s**. Fmt and affected renderlab/viewer all-target
release Clippy with `-D warnings` pass. No GPU gate was run before the retained
variant. After recording identities, `cargo clean` removed the worker's entire
Cargo target (**695.6 MiB**); the target directory no longer exists.
