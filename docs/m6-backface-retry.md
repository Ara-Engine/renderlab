# Bounded back-face retry: rejected screen

2026-11-06, base `cb94fd0`, branch `t3/back-face-retry-screen`. One algorithm
variant and one final seven-ray diagnostic; no further hillclimb. Production
keeps the adjacency repair. The experiment remains test-only and is not an
alternative public rendering mode.

The retry is cheaper in this short paired screen and fixes the three original
witnesses, but fails the quality condition. Six changed centre pixels select a
strictly farther, valid front-facing triangle over the valid back-facing primary.
Adjacency preserves the nearer primary at all six. These are not ambiguous
adjacency choices. A committed back face near an edge is not sufficient evidence
that the primary is wrong.

## Variant and measured gates

`support/receiver_backface_retry.wgsl` retries only a back-facing committed hit in
the existing four-epsilon barycentric edge strip, capped at 0.25. The distance
bound is `4 * epsilon / max(abs(N dot ray), 0.0001)`, using the committed hardware
`t`. It traces once from the same origin/direction with `CULL_BACK_FACING`, to
`t + bound`, and accepts only a hit with `abs(t' - t) <= bound`. A miss or failed
bound keeps the original hit. Candidate confirmation preserves source-sidedness.
The geometry-only concave inset retains its existing conditions, arithmetic and
displacement limit, acquiring its neighbour with the existing hardware probe
fallback. The candidate reads no adjacency suffix. No third-party code was copied,
and the experiment introduces no material/shading-model restriction.

The complete paired process loaded the quarry once, in **4.206 s**, then used
the same resident asset for the alternatives, counted images and timings.

- The 36 analytic fold rows pass. Both traced/deferred fold variants pass eight
  repeats, and their complete returned buffers are byte-identical. The deferred
  fixture retains zero blocked corrected samples, versus 99,203 in its raw control.
- All nine 1024 x 1024 witness micro-grids (three poses, spacings 0.001, 0.00001,
  0.0000001 pixels) have byte-identical **32-byte summary counters**, zero opposite
  samples and visibility 1.0. Each accepted sample traces 32 salted shadow rays.
  This screen runs each grid once; it does not claim per-hit micro-grid byte
  identity or eight micro-grid repeats.
- Replaying the three preserved actual GPU f32 rays selects the exact nearer
  primitive with either algorithm: 96182, 130985 and 73189. The exact competing
  depth gaps are 3.049770650, 7.325132102 and 0.501344727 float spacings.
- Three quarry poses x hard/Exact/Interactive x eight repeats per variant are
  deterministic within each variant. Pose 0 is byte-identical throughout. Pose 1
  changes one hit/HDR pixel in every mode; pose 2 changes six hit records and five
  HDR pixels in every mode. Soft receiver/count differences are recorded per
  buffer in `diff-*.json`. Comparison includes masked hit records, HDR, soft
  receiver world/normal exports and visibility counts, using SHA-256 plus byte
  comparisons. These repeat checks use full readbacks; they are not GPU hash-only
  verification.

Because the screen is rejected, no broader thin/stress/grazing-fixture campaign,
film-reference regeneration or adoption gate is claimed.

## Exact classification of every changed pixel

A second, **5.04 s** test loads the scene only to read seven actual centre rays,
their original/repaired records and the uploaded model-matrix bits. Its adjacency
records must exactly match the counted first capture before CPU classification.
`scripts/m6-backface-retry-proof.py` uses rational arithmetic for the full affine
transform and triangle intersections. The six distinct facet selections below
have both intersections strictly inside their triangles, with positive `t`.

| Pose / pixel | Adjacency primitive | Retry primitive | Exact retry minus adjacency t | Classification |
|---|---:|---:|---:|---|
| 1 / (1131,588) | 1615059 | 1578889 | +0.00002496108901 | Valid farther front face |
| 2 / (927,554) | 1550523 | 1490431 | +0.00003312635877 | Valid farther front face |
| 2 / (462,560) | 468299 | 468299 | 0 | Same facet, different barycentrics; coverage ambiguity |
| 2 / (563,569) | 985149 | 985106 | +0.00003466183098 | Valid farther front face |
| 2 / (557,750) | 244972 | 782895 | +0.00003401958831 | Valid farther front face |
| 2 / (1472,977) | 323179 | 326016 | +0.00001495201435 | Valid farther front face |
| 2 / (1157,1062) | 995619 | 995618 | +0.0000001385230295 | Valid farther front face |

At (462,560), both repaired records identify the same front facet, but its exact
first barycentric is -0.0002269585: the exact ray misses that triangle. The raw
back facet also has a negative exact barycentric. This one point is a genuine
hardware edge-coverage case; it cannot excuse the other six strictly ordered
interior intersections. All seven original hardware primaries are back-facing.
The six failures preserve exactly the raw primary under adjacency; the retry
instead discards that nearer surface merely because of its winding.

`difference-exact.json` preserves exact rational t values and barycentrics,
`difference-rays.json` preserves the GPU bits, and `difference-crops.png` shows
all seven pixels with adjacency/retry crops and display-difference masks. The
same-facet point has no displayed HDR difference. Full three-pose comparison
PNGs and these crops were inspected offscreen.

## Winding screen

These are source-winding normals against raw centre camera rays, using the same
edge-strip definition as the repair. They demonstrate back-facing visible scan
surfaces away from numerical edge cases; counts alone do not distinguish an
inconsistently wound patch from a legitimately viewed open-sheet back.

| Pose | Primary hits | Back faces | Back faces near edges | Back faces away from edges |
|---|---:|---:|---:|---:|
| 0 | 346,517 | 62 | 5 | 57 |
| 1 | 345,882 | 60 | 3 | 57 |
| 2 | 1,285,147 | 221 | 20 | 201 |

## Paired cost and memory

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga 30.0.1, optimized
release, active Core + Synchronization validation, 1920 x 1080, original
13,034,903-triangle quarry. Eight alternating old/new pairs after two warmup
pairs, using the two registered timing cameras. Full span includes primary,
visibility, shading and tone timestamps, excluding target clears. Exact retains
256 shadow directions; Interactive retains eight. The desktop had 0-2 rustc
processes during the batch. This is a potentially contended screen, not a quiet
performance certificate or an accepted quality/performance win.

| Mode | Full span adjacency / retry (ms) | Median paired change | Receiver adjacency / retry (ms) |
|---|---:|---:|---:|
| hard | 0.840176 / 0.816720 | -2.429% | 0.285760 / 0.270032 |
| Exact | 10.807856 / 10.067440 | -6.165% | 0.304864 / 0.283952 |
| Interactive | 0.949760 / 0.932352 | -1.626% | 0.546816 / 0.527872 |

The shared loaded asset includes the adjacency table for the paired baseline;
the retry does not read it. Its measured suffix is **156,418,884 bytes / 149.17
MiB**, CPU build **1664.87 ms**, and summed CPU upload-call time **7.922 ms**.
The candidate's hypothetical removal would avoid that suffix and construction;
this screen does not measure a table-free startup or physical VRAM reduction.
**Actual retained memory and load-time savings: zero**, because adoption fails.

## Latency, guards and reproduction

Evidence root: `E:/renderlab-work/backface-retry/`; successful outputs are in
`screen-1/`, with lease/resource/build logs in `runs/`. The first completed batch
is `screen-1b`: **50.86 s** inside the test / **53.03 s** guarded wall time.
The seven-ray follow-up is `screen-2`: **5.04 s** / **7.26 s**. CPU tests pass:
**98 passed, 94 GPU tests ignored**, **0.63 s**. Final-source release compilation
is **41.38 s**. First variant/harness implementation spans roughly **13 minutes**
(23:11-23:24 UTC); incremental builds, verification, coordination and reporting
are separate from that figure. Cold dependencies and harness corrections cost
additional build time; the run logs retain those attempts.

Every build/GPU launch uses the shared token-bearing `gpu-slot.lock` lease and
`gpu-schedule.md`, checks for games/other renderer processes, requires **4 GiB**
available RAM to start and aborts below **2 GiB**. No game, window, desktop capture,
input automation or foreign-process shutdown was used. The paired run starts at
6962 MiB and reaches a minimum of **2863.63 MiB**, without a memory abort. The
diagnostic starts at 8220 MiB. A first build was stopped by the generic guard's
Cargo-build-script classification, fixed only in the owned wrapper. A later
rebuild stopped when another worker's renderer appeared. One initial GPU attempt
completed both folds but hit a diagnostic-composition error before quarry
comparisons; the same variant then completed successfully. Deferred attempts
and these failures are retained rather than counted as successful screens.

The ignored entry points are `gpu_backface_retry_screen` and
`gpu_backface_retry_differences`. Use `scripts/m6-backface-retry-run.py` with
`--manifest <manifest> --fast-check -- <test-binary> <entry> --ignored --nocapture
--test-threads=1`. The manifest uses the established quarry fields,
`t1_cameras`, three `footprint` rows per pose, `repeats: 8`, and `witness_rays`
pointing at the preserved original GPU-bit witnesses. Then run the report and
proof scripts with that manifest. Raw dumps over 5 MB were removed after
summaries, hashes and inspected PNGs were preserved. The owned E: Cargo target
remains: automatic approval review rejected its explicit recursive deletion
with "blocked by policy", including a separate literal-path deletion command.

No software-BVH source was edited. No production shader, adjacency construction,
resource layout or public API changes.
