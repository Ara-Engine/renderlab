# Receiver adoption follow-up

2026-11-05. Follow-up to [the adoption measurement](m6-receiver-adoption.md),
starting at `22681051979857063499c133a597201dd756b871`, on the same branch.
GPU and bulk evidence: `E:/renderlab-work/receiver-adoption/followup/`.
The Level A bounds, including G2's 0.002, are unchanged.

## G2: a finite reference missed a real partial receiver

Pixel **(1295, 513)** in pose 0 and the return pose has original point-reference
visibility 1.0 and sigma256 0.0. Exact seed zero has visibility 255/256. Its
256-bit blocked mask, in little-endian u32 words, is
`[0, 0, 0, 0, 0x04000000, 0, 0, 0]`: only direction 154 blocks.

The production query hits placement 11, primitive 43498, which is also the
camera-hit triangle. This is a real crossing of that triangle's plane, not a
numerical hit at the offset:

| Quantity | Value |
|---|---|
| Origin | (2.373624802, 1.193302989, -0.495867640) m |
| Direction | (0.436967522, 0.854534745, 0.280766517) |
| Epsilon / t_min | 0.000009999999747 m |
| N dot direction | -0.00834767695 |
| GPU hit distance | 0.001224230626 m |
| GPU barycentrics | (0.642874956, 0.000112571, 0.357012451) |
| Independent f64 hit distance | 0.001204150279 m |
| Independent f64 barycentrics | (0.641599249, 0.002234443, 0.356166308) |

The f64 check uses the exact BLAS f32 vertices and object-to-world matrix
returned by a second diagnostic query, promotes them to f64, and independently
intersects the actual f32 origin and direction. Both intersections lie inside
the triangle, roughly 120 offsets away. Small world/object arithmetic differences
explain the distance difference; this is not a hit within the origin uncertainty.
The viewed-side policy intentionally keeps an opaque receiver on that side
when a disk ray travels back through its plane.

The original 64 independent 256-ray replicas find no blocked rays. With 4096
replicas (1,048,576 rays), 32 rays block in 32 different replicas:
**V = 0.999969482421875, sigma256 = 0.0003439576289**. Seed zero is excluded
from this reference. The instrumented estimator first asserts that its seed-zero
mask equals the original production mask. The second vertex-return query lives
at function scope to avoid Naga 30's invalid loop-local opaque null initializer;
the corrected probe runs with Core + Synchronization validation and zero errors.

`ray-evidence.json`, `probe/rays.f32`, `probe/masks.u32` and
`images/g2-crops.png` contain the witness, exact bit masks and nearest-neighbour
crops. `scripts/m6-receiver-ray-evidence.py` performs the independent intersection
and makes the crops.

## Verified solid classifications

Islam approved high-sample verification of solid reference receivers near
terminators. The selection includes every covered point-reference receiver with
V exactly zero or one and **abs(N dot L) <= 0.041**. This conservatively contains
the disk-plane crossing band (tan(0.04) = 0.04002135), with room for f32 basis
rounding. Both lit and occluded receivers are checked. Each selected receiver
gets all 4096 salted 256-ray replicas, seeds 1 through 4096; the shader records
integer sum(blocked), sum(blocked squared), partial replicas and replica count.
All sums fit u32 and every receiver's completed-replica count is asserted.

| Pose | Verified | Lit to partial | Occluded to partial | Total reclassified |
|---|---:|---:|---:|---:|
| 0 | 3108 | 2 | 2 | 4 |
| 1 | 3145 | 2 | 1 | 3 |
| 2 (return) | 3108 | 2 | 2 | 4 |

Only receivers with verified V strictly between zero and one get their mean and
sigma replaced. The original 16,384-ray files remain immutable. The gate output
uses hard links to the unchanged candidate/reference replicas, with separately
written verified reference images. It checks the original solid labels before
applying the verification. Selection, per-pixel results and their hashes are
recorded in `solid-verification/` and `level-a/gate.json`.

| Level A | Before verification | After verification |
|---|---:|---:|
| G2 maximum solid error, pose 0 seed 0 | 0.00390625 | 0 |
| G2 maximum solid error, pose 2 seed 0 | 0.00390625 | 0 |
| G2 bound | 0.002 | 0.002 |
| G1, G3, G4, G5, G6 | Pass | Pass |
| Complete numeric result | Fail | **Pass** |

Reproduce with `scripts/m6-receiver-solid-verification.py stage`, the ignored
`gpu_receiver_solid_verification` test under the guarded diagnostic runner, and
`scripts/m6-receiver-solid-verification.py score`. The selection is based only
on the old reference and geometry, never on the candidate's error.

## Cost diagnosis

The dominant additional Exact/Interactive work is **surface-proximate ray
traversal**. Restoring only the 1 mm origin distance (with epsilon and t_min still
live) recovers most of the added cost: Exact visibility 8.4825 to 7.7239 ms and
Interactive's combined pass 0.4523 to 0.4112 ms in the paired diagnostic screen.
Restoring only the old t_min changes Exact visibility by about 0.047 ms and does
not improve Interactive. Per-vertex shader declarations and device features are
unchanged in the origin control, so they do not explain that recovery. Removing
traversal leaves about 1.01 ms of Exact visibility work, versus 8.99 ms with it.
These are diagnostic, contended screens, not production alternatives.

A final post-rebase control keeps epsilon dynamic and multiplies only its origin
distance by 100, leaving t_min unchanged. Its paired ratios are hard 0.95156,
Interactive 0.95229 and Exact 0.90147. This also recovers the traversal cost while
retaining the precision dataflow and adding a multiply; the conclusion does not
depend on replacing that dataflow with an immediate constant.

**Retained change:** hard camera hits publish their exact triangle epsilon in an
R32Float texture; programmable hard shading reads that scalar. The hit record
remains 16 bytes. Epsilon uses the same transformed vertices and expression,
without a floor approximation. This removes the three precision transforms from
hard shading's larger dataflow. The observed gain is consistent with reduced
register/lifetime pressure; no hardware register or occupancy counters were
captured. The additional target costs 8,294,400 bytes at 1920x1080 and is retired
with the hit targets. It contains geometric precision only and imposes no BRDF
or material-program restriction.

Quiet confirmation: Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, release
binary, the registered 12-draw/13,034,903-triangle quarry, 1920x1080 1x. Each
process alternates baseline and candidate, rotates modes/order, and uses 24
warmup pairs plus 48 measured pairs per mode. There were no compilers, linkers,
games, Unreal or other Renderlab jobs. The two preflights had 9280/9324 MiB
available; owned allocations reached minima of 5752/5517 MiB. Both used the
original 6 GiB preflight and a 3 GiB runtime abort floor. The 10 Hz resource
sampler records memory/rustc context and sample age with every timing sample.

| Against the unoptimized policy (`2268105`) | Baseline median ms | Retained median ms | Paired ratio | 95% interval |
|---|---:|---:|---:|---:|
| Hard | 0.878944 | 0.825440 | 0.93815 | 0.93286–0.94387 |
| Interactive | 0.864448 | 0.863472 | 0.99801 | 0.99564–1.00421 |
| Exact | 9.765840 | 9.694784 | 1.00055 | 0.96254–1.05601 |

| Against original main (`767c28c`) | Baseline median ms | Retained median ms | Paired median delta ms | Paired ratio |
|---|---:|---:|---:|---:|
| Hard | 0.844384 | 0.823104 | -0.022400 | 0.97355 |
| Interactive | 0.817792 | 0.865600 | +0.047552 | 1.05837 |
| Exact | 8.344512 | 9.811504 | +0.854352 | 1.10264 |

The driver has previously documented fast/slow timing states; raw medians are
not paired deltas. Ratios and deltas above use each adjacent pair. Both remaining
regressions are beyond noise: Interactive's ratio interval is 1.05227–1.06116;
Exact's is 1.10109–1.15378. **Hard's regression is recovered. Interactive and
Exact are not recovered, so the complete no-regression cost requirement remains
unmet.** No quality reduction or origin/t_min change is retained to hide it.

Rejected output-preserving screens include 1/2/4/8/16/32 ray lanes, horizontal
and vertical footprints, interleaving lanes, splitting the thin loop, stride and
query helpers, two/four-way unrolling, 32-thread hard shading, eight parallel
Interactive rays, separate camera/eight-ray passes, and closest-hit visibility
queries. Parallel Interactive is 35% slower, separate passes 29% slower,
16-ray lanes about 20% slower, and closest-hit Exact about 4% slower. Small
sub-percent tile/unroll changes did not consistently recover material cost.
The first-triangle-candidate query screen was aborted twice by the 3 GiB guard
before producing a timing result; it is not an established rejection.

Bulk timings, integer samples, bootstrap intervals, resource contexts and
manifests are in `followup/runs/`, `timing-summary.json`, and each screen's
`paired-timing.json`/`fresh-screen.json`. `m6-receiver-diagnostic-stage.py`
reconstructs diagnostic shader variants from the immutable composed policy
modules. Some early screens preceded per-sample telemetry; their complete
runner resource logs identify contention. Only the two `quiet-*` runs establish
the retained performance claim.

## Rebase and retained-source verification

The branch was rebased onto main `d4d77d8` (including fast reload `cc8ae53`)
without conflicts. The rebased implementation checkpoint is `311fdf9`.

- Release Clippy for the library and tests passes with warnings denied; all
  84 CPU tests pass.
- The complete rebased acceptance run passes with Core + Synchronization
  validation active and zero errors. There are 32 repeats for each of four
  Interactive seeds at poses 0, 1, 0: **384 byte-identical HDR/count repeats**.
  Return, rejected reload preservation, tint/revert and raster fallback pass.
- **36/36 complete captured buffers** (HDR, visibility, world and normals) match
  the inspected `2268105` production captures byte for byte. File sizes and SHA256
  hashes are retained in `byte-identity.json`.
- All 15 R1 fixtures pass on the rebased production renderer. Their film/direct
  buffers are byte-identical to R1; acne has zero false-shadow pixels. The
  sawtooth, thin and stress analytic checks pass. Stress retains the previously
  documented 16 one-sample accepted-count mismatches, within sigma_F.
- The three quarry R1 replays also match byte for byte: **36/36 film/direct
  buffers** across the 15 fixtures plus three poses. The explicit material-thin
  suite passes all raster/traced hard, Exact and Interactive cases. The rebased
  blocking-ray witness and all 4096 replica masks also match the earlier probe
  byte for byte.
- The complete native hard images at both camera poses and the G2 crops were
  inspected. Exact/Interactive images and the remaining inspected adoption images
  are unchanged by the 36-buffer identity check. Level B and film results remain
  the recorded [adoption measurements](m6-receiver-adoption.md); neither image
  output nor those frozen references changed in this optimization.

The repeat harness compares saved return captures one pair at a time rather than
retaining all three poses in memory. Its byte assertions and repetition count are
unchanged; this saves roughly 300 MiB during the guard. An earlier attempt stopped
below 3 GiB amid foreign builds before completing the return pose; it reported no
image mismatch. The reduced-memory rebased run completes in 89.63 seconds.

The unchanged old/new Level B maxima are partial RMS 0.081751 → 0.080903,
faint RMS 0.055783 → 0.054770, contact RMS 0.043088 → 0.042651,
motion RMS 0.099844 → 0.099625 and contour rho 1.266903 → 1.267512;
solid flips and lost contacts remain 0/0. The composite gate passes for all
three poses and four seeds.

| Pose | Mode | R_F partial RMS, old policy → adopted policy |
|---|---|---:|
| 0 | Hard | 0.266478 → 0.253246 |
| 0 | Exact | 0.231586 → 0.203344 |
| 0 | Interactive | 0.234188 → 0.207204 |
| 1 | Hard | 0.267065 → 0.254196 |
| 1 | Exact | 0.232030 → 0.204690 |
| 1 | Interactive | 0.234588 → 0.208537 |

These use each policy's own film reference/populations, as in the adoption
record; the different populations do not establish a policy-independent
perceptual improvement. Exact's whole-image R_L RMS codes remain
2.581593 → 2.581746 and 2.625275 → 2.625478; its sun-only R_D RMS codes at
+4 stops remain 1.418587 → 1.415564 and 1.408939 → 1.415159.

Disk hygiene: Cargo's native cleanup removed this worker's target (1618 files,
667.5 MiB). Automatic approval review rejected both the guarded bulk raw-dump
cleanup and a narrower file-only cleanup as **"blocked by policy"**. Those raw
dumps remain; `cleanup-candidates.csv` lists their exact paths and sizes, and
`cleanup.json` records the block. JSON/CSV/log summaries, compact receiver
evidence, source modules and inspected PNGs remain on E:.
The fixture collector accepts `--output-root`
and `--runner` so a retained source can be checked without overwriting earlier
evidence.

The follow-up ledger records 70 short leases from 10:38:43 to 12:35:27 UTC:
403.18 seconds of build/check holds and 769.37 seconds of GPU-run holds
(startup, uploads and readback included). The longest hold is 89.65 seconds;
all leases were released. These verification times exclude editing, shared-slot
waits and CPU-only scoring; they are separate from the frame timings above.
