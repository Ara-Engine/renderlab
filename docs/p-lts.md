# P-LTS: exact local-light work reduction

2026-11-09, branch `t3/p-lts`, from `dc93b7e`. The renderer baseline matches
ARA1's `ae857501` / v0.1.0. **The 3.5 ms target is unmet.** Candidate E saves
about 9% on RT and 29.5% on shadowed non-RT in the matched steady replay, with
the original quality settings and shadow coverage.

## Implementation

- The standard material rejects exact-zero attenuation and its unfiltered BRDF's
  zero receiving hemisphere before requesting visibility. Positive contributions
  retain their evaluation and accumulation order. The optional filtered specular
  BRDF bypasses the hemisphere rejection. This decision stays in the standard
  material: custom programs retain their own light loop and visibility decisions.
- The map path fetches an interior hard-PCF 4x4 block with four depth gathers.
  It still compares all sixteen texels against their own receiver-plane depth
  and accumulates in the original row-major order. Tile/cube-face edges use the
  original reprojecting path. The existing DFG sampler supplies level-zero,
  unfiltered gathers; no binding, layout or device feature is added.
- LC2's soft blocker search/filtering and the complete local ray routine are
  unchanged. An attempted radius-zero disk-setup guard changed six pixels in
  the RT soft-spot fixture (maximum 3/255); it was rejected and fully reverted.
- ARA1's adapter optionally hashes tightly packed resolved linear RGBA16F bytes
  (`ARA1_HDR_HASH=1`) at its capture witnesses. Readback/hashing stays outside
  frame service timing. The frozen baseline has the same instrumentation.
- TP1's historical standard-program reconstruction removes the later local-light
  optimization before checking its original source digest. Its expected digest
  is unchanged.

The 128-map default, atlas allocation/resolution, caster set, held-map cache,
shadow distance, every contributing RT light, soft-source samples, GI,
reflections and reconstruction settings are preserved. Clustered addressing
remains available to user programs. Receiver/hit data and dispatch do not assume
a particular material model. No reservoir, reuse cache or denoiser is added.

## Profiling and source study

Read-only Ara study: `shaders/baseline.slang::tileMain` and
`shaders/look-shading.slangh` in
`C:/Users/snowy/.t3/worktrees/snengine/t3-9c4a1f92`. Its shipping tiled culler
preserves authored ranges and ordered indices, and its material evaluates the
local contribution before querying visibility. The separate V2 experiments were
studied but not substituted for shipping Ara.

Read-only Unreal 5.7 study: installed `LightGridCommon.ush` and
`LightGridInjection.usf` (grid addressing, logarithmic depth slices and
conservative shape tests). No Epic code was copied. See Epic's public
[grid description](https://dev.epicgames.com/documentation/en-us/unreal-engine/forward-shading-renderer-in-unreal-engine)
and [clustered-deferred atlas context](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-light-functions-in-unreal-engine).
The gather implementation follows the [WGSL depth-gather semantics](https://www.w3.org/TR/WGSL/#texturegather);
it does not replace receiver-plane comparisons with a constant-depth hardware
comparison filter.

Own baseline RT GPU Trace, `profile-steady-c-rt`: 1080p, fixed ARA1 camera, four
complete frames after a 45 s delay. `primary_shade_tiles` averages **10.65 ms**;
`lc_build` averages **0.0098 ms**. Pipeline identities are joined by Vulkan
handle; timeline and catalog pipeline indices are different namespaces. This
trace has timing but no sampled shader/source data.

P-NRT supplied the baseline non-RT light traces at the same camera/resolution,
around frame 1100. Four-frame median: **34.98 ms frame / 30.24 ms deferred
shading** with local shadows, versus **11.23 / 6.44 ms** unshadowed. Clustering
costs about **0.010 ms**. The unshadowed shading range is 6.44–10.84 ms, so
subtracting medians is attribution evidence, not an isolated-shadow benchmark.

In the longest shadowed shading range (33.14 ms), shader 40
`primary_shade_tiles` attributes **58.0%** of sampled states to `rl_local_tap`,
**12.5%** to `rl_local_shadow_map`, **10.0%** to `exposed_product`, and **1.8%**
to cluster-word reads. These are sample shares, not additive function times.
They select map sampling over rewriting the culling kernel. Original traces
remain owned by P-NRT; own extracted catalogs/source/hotspot summaries are under
`D:/Coding/renderlab-work/p-lts/analysis/`.

## ARA1 paired measurements

RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan, optimized release,
native 1920x1080. Synchronous CPU submission-to-GPU-idle service, excluding
readback/PNG/hashing. GPU clocks were not changed; ordinary OS activity remains.

The original 88-frame ARA1 replay is preserved. Because early-still timings
varied across initial screens, timing additionally returns to the original
still camera, cuts history, warms 128 frames, and measures eight still frames.
The final tail image is also hashed. All quality/settings/geometry are unchanged.

| Steady still service, ms | Frozen baseline | E | Reduction |
|---|---:|---:|---:|
| Shadowed RT | 12.128 | 11.083 | 8.6% |
| Shadowed non-RT | 25.515 | 18.000 | 29.5% |
| Unshadowed RT | 10.161 | 9.245 | 9.0% |
| Unshadowed non-RT | 13.140 | 11.985 | 8.8% |

The original ARA1 still segment (frames 32–39) also improves: RT shadowed
12.069→11.035, non-RT shadowed 28.215→20.710, RT unshadowed 10.104→9.187,
non-RT unshadowed 15.661→14.363 ms. Neither view reaches the target. These are
the actual paired baselines; the earlier ARA1 report's numbers are not reused
as measurements of this lease. Ara's smaller shadow budget remains a comparison
limit, not a reason to reduce Renderlab's coverage.

Evidence: `runs/e-timing/summary.json`, lease `e-screen-1`.
All timing pairs completed by 09:23:43 UTC, before the first foreign
Cargo/rustc observation at 09:23:47.6. That later event stopped the lease during
quality-only work. The completed timing prefix has no compiler contention;
interrupted data is not used. Remaining image-only checks ran separately.

The night-courtyard scoreboard also ran both paths with the same eight-frame
throughput probe. Still RT is 13.211 to 13.163 ms, and non-RT 36.792 to 36.709 ms.
This probe includes final-color copy/readback, unlike the ARA1 service metric;
the small difference is diagnostic, not evidence of a courtyard speedup.

## Quality verification

- **197 lib tests passed**, zero failed, including all shading heads and the
  unchanged historical TP1 digest.
- LC2 point/spot hard/soft, LC3's 64-light leakage control and forced map-slot
  fade: **24 candidate PNGs byte-identical** to the baseline on both paths.
- ARA1 shadowed/unshadowed on RT/non-RT: all original still/motion/cut witnesses
  at **960x540** and **1920x1080**, plus the timing tail, are **HDR-byte and
  PNG-byte identical**. Missing/empty hash sets fail the harness.
- Night courtyard with the Fox, 27 local lights, 128-map setting, default
  GI/TR1/reflections/post/glass/particles/fog: 16 still and 32 moving frames on
  each path. **All 16 candidate captured PNGs match** the corresponding baseline.
- Full registered quarry, 13,034,903 triangles, 261 lights, original 1080p
  cameras/settings and the registered software near field on non-RT: both path
  pairs pass every fixture assertion and all **14 float-HDR / 28 PNG witnesses
  per path are byte-identical**. This includes moving lights, the 16-frame camera walk,
  hard/soft and legacy-budget controls, GI on/off, day and dusk. Its completed
  traced prefix is `runs/e-quarry-byte-final/summary.json`; the non-RT pair is
  `runs/e-quarry-nonrt-final/summary.json`. RT retains 260 shadowed locals; maps
  retain 128, redraw zero cached faces in the static check, and report zero
  cluster/CPU mismatches, zero overflow and zero unclustered differing channels.
  The non-RT lease completed in 90.73 s, with zero compiler observations and
  minimum available RAM 5725 MiB. The traced prefix also has zero compiler
  observations (minimum 6071 MiB). The standard 2 GiB runtime guard never tripped.

Inspected: all four ARA1 witness contact sheets, the LC fixture sheet and all
night courtyard captures and both quarry sheets in `inspection/`, plus full
final night frame 031 on both paths, both quarry night presentations and the
non-RT final walk presentation.
Silhouettes, contact shadows, soft penumbrae, glass, local pools and
the moving Fox remain coherent. Existing grain on flat surfaces, the visible
glass rectangle and the late-motion mirror-sun glint remain baseline defects;
this optimization does not certify the Unreal floor or fix those separate gaps.
Byte identity preserves the registered crops/populations' reference error;
no new reference-convergence or general scene-quality claim is made.

## Reproduction and resource policy

All own output/targets are under `D:/Coding/renderlab-work/p-lts/`. Staged baseline
and E binaries remain separate; JSON records binary, scene and asset identities.
No Ara checkout was modified; private scan assets remain outside the repository.

- `scripts/p-lts-run.py`: latest anchored root order checked during handoff and
  admission, optional owner-only release gate, <=15 min, at least 6 GiB admission.
  Quarry uses >=8 GiB admission and the authorized standard **2 GiB runtime
  guard**; it cannot be lowered. Initial runs used an unnecessarily strict 6 GiB
  runtime guard until Islam clarified that the 6 GiB rule means admission.
- The final `quarry-verify-final-2` attempt admitted with 9270 MiB available,
  then stopped on foreign compiler contention during baseline upload: Cargo PID
  23276 and rustc PID 33528 appeared immediately after admission. The minimum
  sampled RAM was 6162 MiB, so the 2 GiB runtime floor did **not** trip. This
  attempt produced no scored pair and is excluded. P-LTS released immediately
  for P-SHC's final turn and held renewals until its owner-matched release.
- `quarry-verify-final-3`, after P-SHC, completed the traced baseline (37.98 s,
  14 float-HDR and 28 PNG hashes), then stopped during non-RT baseline setup
  when foreign Cargo PID 12076 / rustc PID 42020 appeared. Minimum sampled
  available RAM was 5413 MiB. This is also incomplete, excluded pair evidence;
  the standard 2 GiB runtime guard remains unchanged.
- `quarry-final-quiet` stopped at 10:23:40 UTC when a chained foreign baseline
  build began just after admission (minimum 6298 MiB available). The final
  `quarry-byte-final` turn uses an explicit hold through that exact owner release
  line, preventing an earlier aborted release from ending the compiler hold.
- `scripts/p-lts-profile.py LABEL`: headless baseline GPU Trace; no screenshot,
  UI or clock change, 50 ms / 200 MB trace buffer, 45 s delay and 10,000 still
  frames. The filtered Nsight-only Vulkan layer directory follows memory notes.
- `scripts/p-lts-batch.py LABEL --binary E --baseline-binary BASE`: both paths
  and game-light cases. `--resolution quality` selects original ARA1 quality
  witnesses; timing defaults to 1080p. `--steady-warmup 128` appends the explicitly
  labelled tail without modifying the original replay.
- `scripts/p-lts-fixtures.py LABEL --candidate TESTS --game GAME`: lib, LC
  fixtures, quarry and night courtyard. Any image mismatch fails immediately.
  Quarry HDR dumps are hashed before removing disposable buffers over 5 MB.
  `--paths nonrt` limits a follow-up to its unresolved pair. The inherited n8
  runner reserves 90 s for cleanup: a four-minute lease permits about 150 s of
  command work. The first compiler-quiet full batch reached that bound after its
  completed traced pair, during the final non-RT upload; that partial non-RT
  result is excluded. Each retained baseline/candidate path pair shares a lease.

Rejected setup evidence is retained separately: shader-sampled/debug traces
crossed the initial strict RAM guard with concurrent rustc; an early no-debug
trace captured startup uploads. The delayed trace produced complete frame
events. The source-radius optimization failed the hard image gate and is absent
from the final patch. No light range, sample count, filter, LOD or quality setting
was reduced to obtain the measurements.

Cleanup: automatic command approval rejected the requested removal of the own
Cargo target and summarized raw trace with the reason **"blocked by policy"**.
No deletion executed. `target-cleanup.json` records the retained target size;
the target remains `D:/Coding/renderlab-work/p-lts/target`. Raw trace summaries
and image hashes are retained independently of the disposable binaries/buffers.
The retained target is 716,154,316 bytes; the summarized raw trace is 9,762,692
bytes (SHA-256 `148fe052c2a83168c942e6aa38a0b8379404ff3515b3c5f789b4b22f1412c075`).

Implementation/exploration ran from about 07:43 to the retained E build at 09:21
UTC. Its optimized example build took 36.66 s and its lib-test build 103 s.
Final retained verification began at 09:21 UTC; its elapsed wall time includes
GPU handoffs and excluded interruptions and ended at 10:36 UTC (75 min elapsed).
All recorded GPU lease occupancy totals 17.78 min across separate bounded turns;
no individual turn exceeds 15 min. These development/build/verification times
are separate from the frame-service measurements above. The combined retained
check ledger is `D:/Coding/renderlab-work/p-lts/evidence-summary.json`.
