# Receiver shared-edge repair

Validated receiver repair, 2026-11-06. The selected two-pass adjacency variant
passes the focused quality gates. Its remaining frame-time cost is an accepted
miss; there is no claim of a performance win or complete film-quality rendering.
Base: `539c233`; diagnostic checkpoint: `b813457`. All runs are offscreen on an
RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1. Evidence lives
under `E:/renderlab-work/receiver-origin/`, outside the public repository.

## Two defects, distinguished by the same-ray proof

The three quarry witnesses are pose 0 (1347,380), pose 1 (1119,713), and pose 2
(1325,276). The hardware returns the farther of two edge neighbours for the
GPU's **own actual f32 ray**. The nearer facet faces the sun; the returned facet
faces away. This is not evidence of a physically visible backlit sliver caused
by constructing a slightly different f64 camera ray. Exact rational intersections,
the bit patterns, t gaps and limitations are recorded in
[determinism.md](determinism.md#receiver-investigation-same-ray-traversal-precision-and-residual-arithmetic).
Pose 2's gap is 0.501345 float spacings, so comparing two ordinary f32 long-distance
t values is insufficient even after an apparently successful re-test.

There is also a real receiver-origin defect, independently of wrong primary
identity. On an analytic concave fold with the correct initial facet, the normal
clearance can cross the neighbour's plane. Neighbour ordering alone removes all
nine quarry micro-grid bands but leaves **5,099 / 8,192** false-shadow samples in
the simplest 2 mm, slope-2, normal-light analytic row. This is the evidence for
retaining a restricted inset. A blanket inset was rejected because it creates
false shadows on convex roofs.

## Geometry-only correction and bounded receiver motion

The retained quality algorithm in `primary_trace.wgsl` works only in an
error-scaled strip at a triangle edge. The strip is derived from four times the
existing normal-clearance epsilon and the triangle's altitude; barycentric
margins are capped at 0.25. It uses returned object-space vertices and the
instance's linear transform, avoiding subtraction of already translated vertices
when constructing local edges.

Canonical lexicographic endpoint order identifies the same shared edge from
either initial triangle. The primary ray is rebased beside that edge with explicit
FMA, and short, laterally separated camera-direction probes discover only
triangles in the same instance with exactly the same two edge endpoints. The
probes preserve source single-sided visibility. They are temporary search rays;
their grazing-dependent extent is never a shadow-origin displacement.

Where two facets project onto the same side of the edge, depth is ordered in
the plane perpendicular to that edge. Cross-multiplied local slopes avoid
subtracting two long f32 t values. A scaled floating-point uncertainty test and
primitive-ID tie rule are deterministic and independent of illumination. A
winning replacement snaps to the canonical edge only within two epsilons of
the original point. If the original facet wins, its exact original point is kept.

For an actual concave fold, a second probe can discover the opposite projected
side. An inset is allowed only when that neighbour lies above the viewed-side
plane **and** the proposed geometric-normal clearance crosses its plane beyond
the local arithmetic guard. The inset projects onto the inset edge segment and
caps tangential displacement at four epsilons. Flat, convex and already-clear
cases retain their barycentrics. This is not mesh smoothing or a shadow-terminator
adjustment.

The normal displacement remains **exactly epsilon**. Including the inset, the
constructed origin moves at most five epsilons before final floating-point
rounding; the primary correction branch is bounded by three. There is no
viewer-directed shadow offset or `1 / abs(NÃ‚Â·V)` growth in origin displacement.
Opaque materials retain viewed-side normals; the material's explicit thin flag
retains the existing light-side rule. Hit identity and receiver data contain no
Standard-BRDF or sun-specific classification, preserving programmable shading.

### Relation to production practice

[Current Cycles `integrate_surface_ray_offset`](https://raw.githubusercontent.com/blender/blender/main/intern/cycles/kernel/integrator/shade_surface.h)
normally starts at the surface and skips the originating primitive; it offsets
only when its self-triangle re-test misses, and explicitly handles neighbours. Its fallback tests depend on a watertight triangle routine and a
matching inverse instance transform, and its comments identify small
barycentrics/grazing rays and moving the point inside the triangle as relevant
cases. Renderlab cannot assume its f32 software test exactly matches opaque
hardware traversal. The same-ray witnesses therefore motivate explicit local
neighbour ordering instead of relying on a self-primitive exclusion alone.

UE 5.7's path tracer uses a geometric-normal offset scaled by coordinate/hit
precision, preserves the viewed side for opaque materials, and flips for
transmission (`RayTracingCommon.ush:424`, `PathTracingCore.ush:2017Ã¢â‚¬â€œ2023`).
Renderlab keeps that side distinction and its existing smaller error floor.
It does not adopt the separate real-time HRT shading-normal/camera-related
bias which erased measured contacts in
[the Unreal mismatch analysis](unreal-shadow-mismatch.md).

[WÃƒÂ¤chter and Binder, Ray Tracing Gems chapter 6](https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_6.pdf)
recommend barycentric reconstruction over reconstructing a point from long ray
t, and explain why primitive exclusion fails at shared edges and grazing angles.
The repair retains barycentric reconstruction and minimal normal clearance,
then handles the diagnosed concave shared-edge geometry explicitly. The fixture
and displacement bound are essential: increasing a global bias could hide the
band while deleting real contacts.

No third-party source was copied into this implementation.

## Focused quality evidence

`gpu_receiver_fold` samples 8,192 logarithmically spaced hits per row on 2, 4
and 8 mm facets, slopes 2, 4 and 8, normal/grazing light and opaque/thin receivers:
36 cases. Earlier runs used 32 repeats; the final batch used eight, following
the later check-rarely instruction. Its independent segment/plane oracle also
checks real occlusion, a five-epsilon displacement limit, and unchanged
flat/convex controls. All corrected false-shadow counts are zero; old bands
remain in the control. `fold/fold.json` records the band's barycentric intervals.

`gpu_receiver_fold_traced` adds real hardware primary traversal and neighbour
acquisition on the same 18 geometric/light combinations, using separate
translated instances and the production repair. All interior false-shadow
counts are zero; actual neighbour occlusion remains. Earlier outputs were byte-identical over 32 repeats; the retained adjacency
variant passes eight final repeats. At `u=1`, the far outer vertex, a rounded translated ray can be
a primary miss. The fixture records those separately and asserts that misses
occur only at that outer endpoint; it does not classify a miss as a shadow.
The guarded `pass-fold` run passes with active Core + Synchronization validation.

The current quality algorithm clears **all nine** quarry micro-grids: each
witness at spacings 0.001, 0.00001 and 0.0000001 pixels, with a 1024Ã‚Â² grid and
32 salted shadow rays per accepted sample. `final-batch/micro/footprint.json` reports
zero opposite samples and zero fully blocked samples in every grid; visibility
is exactly 1.0. The finest two grids accept all 1,048,576 samples per witness.
The widest grid can cross neighbouring centre surfaces and records rejections
under the pre-existing 2% centre-depth conditioning rule.

Front-side grazing views hold the eye above the groove peaks and preserve the
world-space pixel footprint by adjusting FOV. The continuous f64 profile oracle
uses the actual camera inverse and independently integrated disk directions.

| View from normal | Contact RMS | Sawtooth RMS | False / lost / missing solid samples |
|---|---:|---:|---:|
| 80Ã‚Â° | 0.000043389 | 0.000151237 | 0 / 0 / 0 |
| 85Ã‚Â° | 0.000032200 | 0.000152941 | 0 / 0 / 0 |
| 89Ã‚Â° | 0.000021791 | 0.000147878 | 0 / 0 / 0 |

All six rows are below their split-half noise estimates. The oracle uses the
existing finite R1 normal clearance; it is not a claim of exact agreement with
an infinitesimal zero-bias physical surface at every boundary. Boundary count
differences and their complete-image RMS are reported separately rather than
silently excluded. Evidence: `final-batch/fixtures/*/grazing-oracle.json`.

### The retained 89Ã‚Â° underside control

The original fixed-distance 89Ã‚Â° groove camera is below the peaks and sees sheet
backs. Its approximately 0.00663 residual was **introduced by the candidate**,
not pre-existing main. Removing the inset or retaining the original point did
not resolve it. The cause was comparing facet depths without projecting the
depth axis perpendicular to the shared edge. At grazing views, along-edge depth
contaminated the ordering and changed the selected facet.

Using `unit_normal(cross(edge_axis, projected_side))` fixes the comparison.
Complete-boundary RMS becomes **0.00029990**, versus **0.00033245** on original
main; partial RMS becomes **0.00039648**, versus **0.00049997**. False, lost and
missing solid shadows are all zero. This item is resolved in the candidate,
and must not be filed as a pre-existing main defect. Evidence:
`underside-depth-fixed/` and its original-main control.

## Selected dispatch and adjacency

The retained implementation is `adjacency-32` (labelled `legacy` in the final
comparison). The hot fused Interactive pass traces all eight shadow rays from
the original hit, even at near edges. One append per 32-lane workgroup queues
only near-edge raw hits. A 32-thread cold pass orders the neighbouring facets
and checks concave clearance. It traces eight new shadow rays only if the facet
or point changes. Each pixel owns its outputs; queue order cannot affect results.

At 1920x1080 the two registered timing poses queue **20,612 / 20,380** pixels
(**0.9940% / 0.9828%**). Only **2 / 0** hit records change. The hot result is kept
for an unchanged receiver. Oversize queues fall back to the full quality kernel.

Originally, triangle adjacency was built on CPU upload from exact source positions, including
UV seams and signed zero. Open, degenerate, duplicate and non-manifold edges
retain the camera-relative probe fallback. The three original indices precede
three adjacency words per triangle in the same storage buffer, with one padding
word. BLAS and raster draws use the original index count. Limits that cannot
admit this derived suffix retain the original buffer and probe fallback.

The [prepared-adjacency follow-through](m6-prepared-adjacency.md) moves that exact
build into the cooker. Schema-3 assets upload the stored words directly;
legacy/procedural meshes retain the same builder as a compatibility fallback.

The full asset has 13,034,903 triangles in 12 meshes. Adjacency adds
**156,418,884 bytes (149.17 MiB)**, with 19,481 boundary half-edges and no ambiguous
or degenerate half-edges. Its summed CPU build cost is **1669.04 ms**, and the
summed CPU `queue.write_buffer` call time is **9.795 ms**, not isolated GPU-copy
time. Actual near-edge fallback is one pixel / one query per measured pose.
The ABI stores geometry and raw hit records, not a lighting-model decision.

The last requested screen split the cold work into a traversal-free ALU/memory
module and a tiny ray-query module. It was byte-identical but slower. At pose 0,
medians were hot 0.441616 ms, check 0.043120 ms, retry 0.021296 ms, with gaps
0.004768 / 0.004608 ms. The check reads one adjacency word and three source
vertices (three index reads plus three position reads); a candidate neighbour
adds another three indices and three positions. Further branches can visit the
other near edges. Only three pixels reached the retry list (two changed plus one
fallback). Packing adjacency beside indices had already been screened. No more
structure or compaction screens were run after the final time box.

Historical contended screens explain the selection: cold eight rays for every
near-edge pixel cost about +14.6%; repair-disabled cold eight rays still +13%;
only changed receivers reduced that to about +6.2%; adjacency reduced it again.
The private split-pass composer remains available for reproducing the rejected
screen; the shipped shader selects the combined cold pass.

## Final batch and old/new results

`runs/final-batch-r2.json` records the single-process final GPU check, **115.70 s**
inside the test / 118.27 s child wall time, with **one original full quarry load**.
Fixtures ran first. Both dispatch candidates match the frozen full-quality
kernel at all three T1 poses, hard/Exact/Interactive, across **eight byte-identical
repeats** of HDR, hits and visibility counts. All nine 1024-square micro-grids
have zero opposite and zero fully blocked samples. The actual deferred fold
control changes 99,203 falsely blocked samples to zero.

CPU replay caught an error in the batch harness: its thin/stress labels initially
called the profile fixture and created flat controls. The harness is corrected.
`runs/final-rectangles.json` is the single focused 13.13 s rerun of both proper
rectangle controls, sharing a renderer and loading no quarry. All **21** controls
pass: twelve flat/convex 1/10 km acne cases, sawtooth, thin, stress, and six
front-side grazing contact/sawtooth cases. Thin/stress partial RMS are
0.00040789 / 0.00031038, below split-half noise 0.00471420 / 0.00139434, with no
missing oracle samples. Stress records 16 boundary acceptance-count differences.

The retained variant regenerates T1 film references at **1024 primary samples x
32 shadow rays**, with independent halves, full radiance and sun-only radiance,
for all three poses. Old and new centre images are scored against the **same new
reference and populations**, avoiding a population-change advantage.
`final-batch/final-score.json` contains every population on L0/L4/D0/D4/F;
`scripts/m6-receiver-final-score.py` replays saved buffers with two CPU threads.
The final CPU replay took **61.00 s**. Full images and fixture oracle PNGs were
inspected offscreen, including silhouettes, shadows and the 89-degree grooves.

| Pose / mode | Partial visibility RMS old -> new | Contact visibility RMS (both) | S2 lost contacts old -> new |
|---|---:|---:|---:|
| 0 / hard | 0.249524438 -> 0.249486368 | 0.240365758 | 537 -> 537 |
| 0 / exact | 0.200206383 -> 0.200158932 | 0.191798605 | 190 -> 190 |
| 0 / interactive | 0.203808545 -> 0.203761933 | 0.194810495 | 202 -> 202 |
| 1 / hard | 0.250599031 -> 0.250599031 | 0.238168271 | 546 -> 546 |
| 1 / exact | 0.201654632 -> 0.201654632 | 0.188827010 | 201 -> 201 |
| 1 / interactive | 0.205251947 -> 0.205251947 | 0.191845517 | 203 -> 203 |
| 2 / hard | 0.248245549 -> 0.248245549 | 0.224939107 | 1145 -> 1145 |
| 2 / exact | 0.169634665 -> 0.169634665 | 0.168385040 | 340 -> 340 |
| 2 / interactive | 0.176940829 -> 0.176940829 | 0.172602241 | 372 -> 372 |

Contact radiance and visibility are exactly unchanged in all nine comparisons;
no S2 lost-contact count increases. Pose 0 sun-only partial D4 RMS improves:
hard 8.258009 -> 8.254590, Exact 7.177180 -> 7.173246, Interactive 7.250038 ->
7.246144 display codes. Full-radiance partial L0 changes by about +0.00001457
codes at pose 0 and is identical at poses 1/2. These tiny changes do not repair
the pre-existing centre-sample film aliasing or add GI/environment occlusion.

The final time-boxed gates did **not** regenerate the 64-replica Level A point
reference or re-run its G1-G6 campaign. Earlier point-reference outputs under
this evidence root belong to rejected origin candidates and are not acceptance
evidence for this commit. No new Level A pass is claimed.

### Final paired timing: remaining cost miss

The same loaded original asset, optimized release, validation on, 1920x1080,
eight paired frames after two warmup pairs; the order alternates within each
pair. This is the final requested batched, potentially contended measurement,
not a quiet-window or below-noise certification. Full span covers primary,
shadow evaluation, shading and tone timestamps, and excludes target clears.

| Mode | Full span old / new (ms) | Median paired change | Receiver old / new (ms) |
|---|---:|---:|---:|
| hard | 0.817040 / 0.845136 | +3.265% | 0.260992 / 0.286768 |
| exact | 9.177664 / 9.204928 | +0.233% | 0.280384 / 0.301792 |
| interactive | 0.867776 / 0.911872 | +4.671% | 0.455520 / 0.508928 |

The split candidate's Interactive full-span paired change was +7.551%, versus
**+4.671%** retained. Its median receiver time was 0.563200 ms versus retained
0.508928 ms. The original no-regression speed gate is a **miss**, accepted by
Islam/root under the finalization instruction; quality was not reduced to pass.
Hard's +3.265% is also reported rather than hidden by the long Exact pass.

### Conditions, verification latency and ownership

All runs were background/offscreen, with no game, Unreal Editor or other owned
renderer job at preflight. The final rule was **4 GiB available-memory preflight /
2 GiB in-run abort**, with no waiting for quiet builds. Final-batch preflight was
6415 MiB; 10 Hz minimum was **2529.18 MiB**, rustc count **0-15**. Rectangle
preflight was 8426 MiB; minimum **7529 MiB**, rustc count **1-4**. Resource samples
and executable/source identities are in the run JSONs. No memory abort occurred.

Final GPU lease: **2026-11-05 22:48:47 UTC to 22:50:43 UTC**; focused corrected
controls: **22:58:47 to 22:58:58 UTC**. Reservations and `receiver window released`
lines are in `E:/renderlab-work/gpu-schedule.md`; neither lease approaches 45 min.
Earlier diagnostic leases and failed attempts remain in the same run directory.

Implementation and finalization are separate from verification: the final
selected-source release build took **62 s**; the final CPU test execution took
0.52 s (**92 passed**, 86 GPU tests ignored). After rebasing onto `bc126f7`,
**97 tests passed** in 0.47 s (92 GPU tests ignored), including T1 composition;
Clippy with warnings denied passed in 5.51 s (38 s before the rebase), and final GPU/CPU verification took the times above. Historical
optimization attempts are not presented as fast edit-to-image latency.

Shared changes are limited to mesh upload/BLAS original-count accounting,
primary dispatch plumbing and six guard-override lines in the common runner.
No temporal module, reprojection shader or history code was edited. T1 evidence
uses our owned archive of `ad80825` with receiver-only integration and test
helpers; it does not modify T1's checkout. Source snapshots are hashed before
large raw buffers are removed. The worker's own Cargo target is deleted at handoff.

`VUID-10684` startup warnings also occur on original main with validation off;
they remain a separate pre-existing item in [determinism.md](determinism.md#separate-pre-existing-vulkan-layout-warnings).
Active Core + Synchronization quality runs recorded no validation errors.

## Research alternatives, not adopted

[The reuse survey, items 5-6](research/open-source-reuse.md#ranked-takes-against-open-problems)
suggests (5) one bounded back-face-culled primary re-trace for near-edge committed
back faces, and (6) a fold-safe offset direction at unchanged 10 micrometre
magnitude. Both remain unmeasured alternatives for this commit. The first could
remove adjacency and handle unwelded neighbours, but must preserve inconsistent
scan winding and source-sidedness, clear the three witnesses and folds, match
a counted quarry capture byte-for-byte, and beat adjacency in a paired sample.
The second could simplify the concave inset, but must preserve contacts and the
viewed/thin rules. Changing the adopted magnitude requires Islam. No optional
screen was run before pushing the selected repair.
