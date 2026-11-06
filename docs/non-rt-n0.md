# N0: software BVH traversal probe

2026-11-06 local (2026-11-05/06 UTC), base `64dbfcf`. Unit N0 of [non-rt-path.md](non-rt-path.md). Test-only; no production file changed. Labels: **M** measured here, **I** inference.

## Result

- **Parity: pass (M).** At poses 0, 1 and return, every software/hardware difference is explained at its cause. That covers hit records, 8-ray counts from the hardware receivers, Exact 256-ray masks and the fused trace. There are 0 "other" mismatches. Validation was active with 0 errors.
- **Cost: stop rule triggered (M).** Software 8-ray shadows take **9.43 ms**, 21.9x hardware, after the hillclimb. The doc's stop line is 1.0 ms. Per the frozen rule, N1 is not started. The traversal limiter is reported below.
- **Projected non-RT Interactive frame: 11.7 ms** (fused software trace 11.24 + 0.45 + 0.03). That is above 1.61 ms, so the result is "quality-first non-RT mode only; record the cost miss". It is 7.3x slower than Unreal VSM's 1.61 ms static frame.
- **Software primaries: 2.84 ms** (M), above 0.40 ms, so N1b (culled raster visibility buffer) would be needed for primaries.
- **BVH memory: 412.5 MB (M)**, under the 0.5 GB rule but above the 0.15–0.35 GB estimate. **Build: 13.4–18.7 s** single-threaded for 13,034,903 triangles.

The doc's 2–5x per-ray ratio (I) does not hold for this implementation on this content: 11.9x for camera rays, 21.9x for shadow rays.

## What was built

- `src/software_bvh.rs`: CPU builder, GPU layout, CPU reference traversal and f64 replay helpers.
  - **Builder:** one BVH per mesh (12). Binned SAH with 32 bins builds a binary tree, collapsed to 8-wide by repeatedly opening the largest-area child. Any binary subtree of at most 4 triangles becomes one leaf.
  - **Nodes:** uncompressed f32 child boxes, 32 B per child and 256 B per node. Boxes are padded by 16 ulps of the mesh's largest coordinate.
  - **Leaves:** they index a reordered triangle list into the meshes' own vertex and index buffers. No triangle is copied.
  - Node and leaf buffers are per mesh, bound as binding arrays like the vertex and index buffers. This keeps every binding under the 128 MiB default storage-binding limit; one concatenated node buffer did not fit.
- `shaders/software_bvh.wgsl`: watertight triangle test (Woop, Benthin and Wald 2013) in f32 with the hardware barycentric convention. Edge functions are sign exact (Kahan difference of products with explicit `fma`), so neighbours sharing an edge never both miss.
  - **Box test:** slab test widened by 1e-6 of t (Ize 2013).
  - **Closest hit:** a private stack with entry distances, nearest child on top, and ties going to the lower placement, then primitive.
  - **First hit:** a separate lean loop with no distances.
  - **Top level:** a brute-force loop over the 12 placements' world boxes, transforming rays to object space as hardware does.
- **Composition:** `software_bvh::traced_source` takes the production traced soft module and removes both ray-query enables, both acceleration structures and the three ray-query uses. Each edit must match exactly once. It then appends the WGSL. The probe therefore traces the production rays, with the same receivers, ε, R2 directions, light-box tmax and output contract. Any surviving hardware ray use would fail to compile.
- **Kernels:** the production entry points on that module:
  1. `primary_trace`: camera rays writing the 16 B hit record and ε.
  2. `fresh_compute`: 8 or 256 rays from the hardware path's own receivers and coverage.
  3. `primary_trace_interactive`: the fused non-RT Interactive trace.
- `src/software_bvh_tests.rs`: two CPU tests and the batched GPU test `gpu_software_bvh_probe`, nested under `primary_ray_gpu_tests`.

## CPU checks (M)

- **Fixtures:** a 3,000-triangle soup, a bumpy 40x40 shared-vertex grid, and stacked, colinear and point triangles. Leaf sizes 1 and 4, padding 4 and 16 ulps. For every fixture:
  - every triangle is in exactly one leaf, and every box contains its subtree;
  - the CPU traversal equals brute force (distance, primitive, barycentrics) on 3,000 random rays, closest and any-hit;
  - every ray through an interior grid vertex or edge midpoint hits (watertight).
- **Quarry:** all 12 meshes pass the invariants. 10,000 random rays per mesh give 0 mismatches against brute force, closest and any-hit, at both 16 and 4 ulps of padding. This took 234 s on 4 threads.
- **naga:** the composed module and its diagnostic variant validate. Every entry point binds only the probe layout's bindings.

## GPU arithmetic (M, RTX 4070 SUPER, NVIDIA 617.14)

On 4,096 near-cancelling inputs:

- WGSL `fma` is fused: 4096/4096 equal the CPU's fused result.
- The Kahan difference is bit-identical to the CPU's.
- Plain `a*b-c*d` is contracted: 0/4096 equal the unfused CPU result.
- f32 division is not correctly rounded, so the shear and t differ from the CPU by an ulp.

Two consequences:

- Software results are not bit-reproducible between CPU and GPU. non-rt-path.md's "bit-reproducible intersection across vendors" advantage needs correctly rounded or division-free ray setup (I).
- GPU/CPU kernel comparisons are classified by f64 replay, not by bits.

The GPU traversal against the CPU reference shows 0 "other" on both sets:

| Queries | Exact | Rounded | Grazing | Tie | Edge | Any-hit equal | Any-hit edge |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fixtures, 16,000 | 4,107 | 3,758 | 8 | 88 | 39 | 7,974 | 26 |
| Quarry, 240,000 (10k closest + 10k any per mesh) | 54,935 | 63,343 | 39 | 1,595 | 88 | 119,994 | 6 |

## Parity against hardware (M)

The workload is the registered quarry: 12 draws, 13,034,903 triangles, 1920x1080, 1x, 0.04 rad disk, Interactive soft shadows. Build b12 ran with validation active (errors=0, active layer). The hardware side checks itself:

- the split receiver trace plus 8-ray pass equals the fused counts;
- the diagnostic hit masks reproduce the counts;
- every diagnostic pass's counts equal production's.

The software diagnostic masks reproduce the software counts.

**Classifier.** Each difference is replayed in f64: the pixel-centre ray for hit records, and the software kernel's own dumped ray for shadow rays. Candidates come from the BVH, with boxes widened by the rounding scale.

- The rounding scale is 4 f32 ulps of the largest coordinate in world **or object space**, plus 1e-6 of the distance along the ray.
- Object space is included because both backends intersect there. Placement 11's object coordinates lie 256–512 m from its origin, so its f32 spacing is about 30 µm, three times ε.
- With world-space magnitudes only, one Exact ray at pose 1 was "other" at 4.7 world ulps (0.28 µm from an edge). It is "edge" under the object-space scale. No other difference exceeds the world-space scale.

| | Pose 0 | Pose 1 | Return |
|---|---:|---:|---:|
| Covered pixels, hardware = software | 346,517 | 345,882 | 346,517 |
| Same triangle, world distance ≤ 0.1 mm (barycentric rounding; max 78 / 88 µm) | 342,052 | 341,278 | 342,052 |
| Different triangle: edge tie / silhouette / grazing (same triangle, >0.1 mm, ray at a grazing angle) | 101 / 3 / 2 | 111 / 4 / 0 | 101 / 3 / 2 |
| Largest edge-tie and silhouette margin | 1.7 µm | 1.7 µm | 1.7 µm |
| 8-ray, hardware receivers: receivers / rays differing, all "edge" | 24 / 24 | 29 / 29 | 24 / 24 |
| Exact 256: receivers / rays differing, all "edge" | 534 / 796 | 542 / 815 | 534 / 796 |
| Largest shadow-ray margin to a triangle edge | 0.33 µm | 0.38 µm | 0.33 µm |
| Fused, on the hardware triangle (346,413 / 345,767 receivers): receivers / rays differing, all "edge" | 49 / 52 | 41 / 43 | 49 / 52 |
| Largest fused margin (its receivers carry the software barycentrics) | 0.69 µm | 3.5 µm | 0.69 µm |
| Other | **0** | **0** | **0** |

Further checks:

- ε at bit-identical records equals the hardware's.
- Eight repeats of every software kernel are byte-identical.
- The return pose reproduces pose 0's records, ε at hits, counts and fused outputs. ε is unspecified at misses on both backends.
- Mismatch maps were inspected (`pose-*-mismatch.png`): differences are isolated pixels on silhouettes and triangle edges, with no clusters or structure.

## Cost (M)

Build b12 ran with validation off. Each of 24 measured pairs (8 warmup) is its own submission, alternating which side runs first. Medians [min, max] in ms:

| Kernel | Hardware | Software | Paired ratio |
|---|---:|---:|---:|
| Camera rays + hit record + ε | 0.241 [0.218, 0.271] | 2.838 [2.827, 5.217] | 11.9 |
| 8 rays from hardware receivers | 0.425 [0.417, 0.926] | 9.432 [9.013, 14.846] | 21.9 |
| Fused primary + 8 rays | 0.444 [0.441, 0.631] | 11.238 [10.946, 17.206] | 25.2 |
| Exact 256 rays | 8.487 [8.466, 9.687] | 140.1 [114.0, 196.9] | 16.4 |

**Conditions:**

- Windows/Vulkan, RTX 4070 SUPER 12 GB with RT units idle on the software side, Ryzen 7 7800X3D.
- Other workers compiled during the timing (up to 8 rustc). Free memory never fell below 3.9 GB.
- `mypaper-wallpaper.exe`, a live wallpaper renderer from another project, has run since 23:28:47 UTC. It is not on the runner's game list.
- The first timing process put a 160–1000 ms software Exact dispatch in the same submission as the other kernels; its later samples drifted 2–4x upward. Timing now uses one submission per pair, and those runs show no drift.

This is a test-binary measurement, not a final-image frame span.

## Hillclimb and limiter

| Step | Shadows8 software ms | Outputs | Verdict |
|---|---:|---|---|
| Baseline (shared closest/first-hit loop) | 10.9–13.0 | — | — |
| Box padding 16 → 4 ulps (CPU brute force still exact) | 11.6 (no gain) | identical | Rejected |
| **Lean first-hit loop**, no entry distances | **9.1–9.4** | identical | **Retained** |
| Same loop returning the hit primitive (u32) instead of a bool | 40–44 | identical | Rejected: an NVIDIA 617.14 code-generation cliff of 4.4x, recorded in the WGSL |

Diagnostic screens on the retained source (results change, so they only locate the cost):

- **No triangle tests:** shadows8 4.48 ms, camera 1.88 ms. Triangle tests and their index→vertex fetches are about 52% of the shadow cost, and 34% of the camera cost. Node traversal alone is still 10.5x the whole hardware 8-ray pass.
- **Shadow rays capped at tmin + 25 cm:** no gain (10.9 against 10.1 ms). Long rays are not the cost. The cost is surface-proximate traversal near the 10 µm-offset origins, as on the hardware path ([receiver follow-up](m6-receiver-followup.md#cost-diagnosis)). This keeps option (c), the hybrid, rejected.

**Named limiter (M plus I):** surface-proximate traversal of a dense mm-scale scan. Each shadow ray starts inside overlapping leaf boxes, so traversal visits many 256 B nodes, about 4 of 8 slots used, through a private-memory stack. It then runs about 3-triangle leaves with indirect index→vertex fetches. The private stack and memory-locality split is an inference: no hardware counters were captured.

**Not attempted, and why.** The stop line is 9.4x away, and traversal-only is already 4.5x over it.

- Quantized CWBVH nodes (Ylitie 2017; obvhs, tray_racing, tinybvh per [reuse-non-rt.md](research/reuse-non-rt.md)) would shrink nodes about 3x and speed only the traversal half: an estimated 1.2–1.5x overall (I).
- A split register/LDS short stack, octant child ordering, a real top-level BVH and quantized leaf vertices are further levers. An optimistic combination is about 3–4x (I), which leaves shadows at 2.5–3 ms, still above 1.0 ms.
- tray_racing's 5.4x (primary + AO, RTX 4070 Ti) suggests a CWBVH port could bring primaries to about 1.3 ms (I). That is still above 0.40 ms.
- Triangle data in leaves (a Woop transform per triangle) would cost about 625 MB, breaking both the no-copy rule and the 0.5 GB cap.

## Survey items considered

- **obvhs** (MIT/Apache) was not adopted for step 1. The in-house builder was already written and verified, keeps the shared watertight test and hardware barycentric convention, and needs no dependency or notice.
- obvhs, tray_racing and tinybvh remain the references for the not-attempted CWBVH step. No third-party code was copied.

## Stop-rule verdicts

| Rule | Value | Verdict |
|---|---|---|
| Unexplained "other" mismatch | 0 | Pass |
| Software 8-ray shadows > 1.0 ms after the hillclimb | 9.43 ms | **Stop: limiter reported; no production work (N1 not started)** |
| Projected non-RT Interactive frame | 11.7 ms | Above 1.61: quality-first non-RT mode only, cost miss recorded. Price the far-field map lever (c) only under unchanged gates; the 25 cm screen gives it no cost lever |
| Software primaries ≤ 0.40 ms | 2.84 ms | Miss: N1b would be needed |
| BVH memory > 0.5 GB | 412.5 MB | Pass |

## Evidence and time

Evidence is in `E:/renderlab-work/n0/`:

- `builds/b1`–`b12` with source snapshots (b12 sha256 in `builds/b12/sha256.txt`);
- `runs/` with summaries, logs, preflights, mismatch PNGs and `*-others.json`;
- `run.py`, a copy of `scripts/m6-film-run.py` with a 4 GiB preflight and 2 GiB runtime floor, plus a timing switch that skips the validation layer;
- `manifest-*.json` and `gpu-windows.json`.

The final runs are `final-parity-4` (validated parity) and `final-timing-2` (timing).

**Verification time:**

- GPU: 23 leases took 609.7 s in total; the longest was 72.6 s. That includes deferrals, three failing fixture runs, two runs aborted by the memory guard, and builds b1–b12. Every lease was released.
- CPU: release test builds took 37–60 s each (the first was 137 s cold); quarry CPU checks 234 s and 243 s; CPU f64 replays 19 s and 18 s; Clippy about 40 s.

**Implementation:** about 22:45 UTC to 00:15 UTC, interleaved with the checks above. Editing time was not isolated.

There was no window, viewer, game capture or Unreal launch.
