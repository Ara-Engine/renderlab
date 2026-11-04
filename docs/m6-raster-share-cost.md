# M6 fresh receiver-depth sharing: rejected cost screen

No production optimization is retained. Both implemented designs preserved the
tested output, but neither demonstrated a reliable frame-cost reduction. The
requested next cost reduction remains unmet. Production renderer and shaders are
identical to base `7ec2fbd`; only a focused first-winner tie fixture is retained.
Streaming and viewer/surface code are unchanged. No quality gate was relaxed.

## Designs and rejection

Plain `Equal` depth testing without a tie mechanism is rejected by inspection:
the receiver's strict `Greater` keeps the first primitive at the winning depth,
whereas ordinary overwrite blending under `Equal` would retain the last. Draw
order alone cannot fix ties between primitives inside one indexed draw.

1. **Equal depth plus first-winner blending.** Shading loads receiver depth, uses
   `Equal` without depth writes, and uses destination alpha as a filled bit to
   preserve the first finite opaque result. One alpha-only fullscreen triangle in
   the same shading pass restores alpha on uncovered pixels. The five-pose fixture
   and two opposite-order red/blue ties pass exactly with active validation. Both
   quarry timing poses match baseline HDR/depth exactly. A six-pair probe is
   inconclusive; the subsequent 24-pair process gives shading ratio 0.961 and span
   ratio 0.993, with receiver control 0.940 and compute 0.998. No reliable gain.
   This design is not certified against the full three-pose quarry count reference.
2. **Depth seeding.** One depth-only fullscreen triangle initializes shading depth
   from `fresh_world.w` to its preceding positive float, keeping zero at zero.
   Shading keeps its unchanged strict `Greater` test and writes original depth.
   No representable float lies between the seed and the original winner, so only
   the first winning primitive passes and restores the exact original depth.
   No blending, stencil, new texture, or extra device feature is needed. All
   fixture and quarry gates below pass. Three 24-pair processes give span ratios
   1.006 / 0.970 / 1.004; shading ratios are 1.016 / 0.975 / 0.982, alongside control
   variation. This does not establish an attributable cost reduction.

Both candidates still submit all 13,034,903 triangles in each geometry pass.
Depth sharing can reject hidden fragments; it does not remove the second vertex
and primitive submission. These results do not establish that a structural
single-geometry-pass design cannot work. Such a design was not implemented.

Fresh compute is currently admitted only at 1x. Both experimental fullscreen
pipelines were 1x and confined to that path. The existing per-sample 4x path was
not changed. No depth format, camera matrix, ray, count, texture chain or quality
setting changed. Production 1x/4x behavior is restored exactly at this checkpoint.

## Exact gates and inspected images

Depth-seed binary `candidate-2b` passes with Khronos validation positively active:
`errors=0 missing_layer=0 active_layer=1` in both fixture and quarry processes.

| Gate | Covered receivers | Partial shadows | HDR / count / depth differences |
|---|---:|---:|---|
| Thin/contact poses 0-4 | 49,152 each | 897 / 752 / 897 / 897 / 752 | 0 / 0 / 0 each |
| Quarry original | 346,520 | 8,547 | 0 / 0 / 0 |
| Quarry forward 1 cm | 345,883 | 8,603 | 0 / 0 / 0 |
| Quarry return | 346,520 | 8,547 | 0 / 0 / 0 |

Return images match originals. All 48 comparable pose files (30 fixture and 18
quarry, including the equivalence records and reference outputs) are byte-identical
to `m6-compute-cost/runs/fixture-c3` and `quarry-c3r`. Two extra red/blue primitive
orders also match fragment HDR/count/depth exactly and produce different images,
making their first-winner check nonvacuous. Removed-blocker ownership, disabled
target release and rejected shader reload checks pass.

Opened every complete PNG at original resolution using the renderer's
Reinhard-plus-sRGB mapping:
`images/quarry-{0,1,2}.png`, `images/fixture-sheet.png`, `images/tie-sheet.png` under
the evidence directory. Inspected outer silhouettes, slab and wedge contacts,
rubble, wall crevices, the thin-strip partial shadow, the sharp contact blocker,
and the opposite red/blue tie winners. No candidate-specific change was observed;
the raw output equality provides the exact comparison. These opaque fixtures do
not establish transparency or Unreal acceptance.

## Paired per-pass GPU times

Same method as [the compute cost screen](m6-fresh-compute-cost.md): each native
process renders 12 warmup frames, then 48 pipelined frames alternating baseline
and candidate, with the pose alternating after each pair and no per-frame drain.
Each side has 24 samples. Baseline uses the current production lane-split compute
and independent shading depth; candidate differs only by the depth seed. Both are
compiled in the same release test executable. Full HDR/depth equality at both
timing poses is checked before timing. Cells are median [min, max] milliseconds;
the last column is median per-pair candidate/baseline. Lower ratios are faster.

| Process | Pass | Baseline ms [range] | Seed ms [range] | Paired ratio |
|---|---|---|---|---|
| 1 | Receiver raster | 3.929 [3.158, 8.179] | 4.126 [3.102, 7.453] | 0.995 |
| 1 | Visibility compute | 10.652 [9.317, 20.676] | 11.206 [9.084, 20.366] | 1.000 |
| 1 | Shading, including seed | 4.624 [4.063, 10.501] | 4.728 [4.229, 8.070] | 1.016 |
| 1 | Tone mapping | 0.059 [0.053, 0.340] | 0.056 [0.052, 0.502] | 0.905 |
| 1 | Render span | 19.029 [17.417, 38.758] | 19.718 [17.618, 35.606] | 1.005 |
| 2 | Receiver raster | 3.892 [2.844, 4.456] | 3.766 [2.840, 4.533] | 0.951 |
| 2 | Visibility compute | 10.509 [9.509, 14.839] | 10.221 [8.977, 12.108] | 0.972 |
| 2 | Shading, including seed | 4.432 [3.960, 6.272] | 4.554 [3.906, 5.334] | 0.975 |
| 2 | Tone mapping | 0.056 [0.052, 0.069] | 0.055 [0.052, 0.074] | 0.990 |
| 2 | Render span | 18.735 [17.290, 23.811] | 17.944 [16.708, 21.701] | 0.969 |
| 3 | Receiver raster | 3.812 [2.847, 5.390] | 3.632 [2.863, 5.195] | 0.999 |
| 3 | Visibility compute | 10.073 [8.697, 12.884] | 10.646 [8.759, 12.738] | 1.019 |
| 3 | Shading, including seed | 4.666 [3.312, 5.794] | 4.361 [3.896, 6.011] | 0.982 |
| 3 | Tone mapping | 0.056 [0.052, 1.051] | 0.056 [0.052, 0.267] | 1.001 |
| 3 | Render span | 18.986 [15.945, 23.410] | 18.892 [15.860, 22.908] | 1.004 |

Hardware: Windows/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14.
Headless 1920x1080, 1x, 12 quarry draws, 13,034,903 triangles, 256 unchanged rays,
0.04 rad disk, exposure -14, original 4K texture chains with 16x filtering, RLE2
environment. Validation is off for timing, active for exact gates. Render span
includes receiver, visibility, shading/seed, tone and intervening barriers; it
excludes loading, AS admission/build, readbacks and presentation.

Every native GPU launch first checked processes and foreground identity: no game,
Renderlab test, renderer or viewer was running. All runs were background/offscreen;
no focus changes, input automation or desktop capture occurred. Foreign Renderlab
and game samples remain empty throughout recorded runs. Other applications stayed
open; preflight utilization was 17-30%, with animated-wallpaper work and occasional
foreign CPU builds. One attempt deferred at 32% before launching. This is a
contended shared-desktop result, not an idle-machine certificate. Untouched pass
variation is why small span differences are not retained as speedups.

## Checks, failures and time

The restored production code plus retained tie fixture passes 30 CPU shadow
tests and release changed-package Clippy (`--lib --tests -D warnings`). The final
GPU fixture, including both tie orders, passes with active validation and zero
errors. All 42 fixture/tie output files match the inspected depth-seed outputs.
Formatting and diff whitespace checks pass. The prior 4x/other shadow GPU evidence
is reused: production code, shaders and their compiler inputs are unchanged, so
the 12 unrelated GPU suites were not repeated.

One depth-seed attempt failed command validation before rendering: its shading
pipeline layout required unused groups 1 and 2 before the fullscreen draw.
Giving the seed a group-0-only layout fixes it; this corrected binary passes all
gates. No image gate failed. The cold build's process monitor hit a `tasklist`
timeout; GPU runs use the corrected timeout, and all their monitors complete.

Verification totals from native manifests: successful GPU processes **64.008 s**,
one failed GPU process **5.736 s**, four builds **212.203 s** (including the
133.122 s cold target), Clippy **26.374 s**. The 30-test CPU suite itself takes
0.03 s; its monitored wrapper takes 0.687 s. Preflights, image conversion,
inspection, formatting and packaging are additional. Implementation/design and
editing ran across 13:00-13:25 UTC on 2026-11-04, interleaved with verification;
active editing was not isolated, so no net implementation duration is inferred
by subtracting checks from wall time. There was no Rust/shader iteration-latency
or idle-GPU performance claim.

Evidence: `D:/Coding/renderlab-work/m6-raster-share/`. All builds use its own
`target` with `-j 2`. `run.py` retains commands/preflights/process samples;
`builds/` retains source manifests, diffs and copied executables; `runs/` retains
raw outputs and samples. `paired-summary.json`, `time-accounting.json`,
`prior-output-equality.json` and `final-fixture-equality.json` capture analysis.
`builds/candidate-1/diff-vs-head.patch` preserves the blending attempt;
`rejected-depth-seed.patch` preserves the complete corrected seed experiment and
paired harness. `handoff.json` identifies the local commit and checkpoint patch.
No push was made.
