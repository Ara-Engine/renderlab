# P-LTS4: fixed-index hard local PCF

2026-11-10, branch `t3/p-lts4`, main baseline
`3c4fcce7012011def99423471bc72479a2c585c4`.
The approximately 3.5 ms frame target remains open on all four configurations.
**Verdict: MERGE the exact hard-PCF optimization.** The completed 1080p matrix
shows an 8.49% shadowed non-RT improvement with byte-identical images. This
verdict does not certify the 3.5 ms target or an unmeasured scene/resolution.

## Change and scope

`shaders/light_cluster_api.wgsl` expands the interior hard-map PCF loop into
sixteen fixed-component comparisons. Four depth gathers, all texel coordinates,
receiver-plane comparisons, weights and row-major floating-point accumulation
remain unchanged. This removes the dynamic vec4 indexing and row selection in
the hottest measured region. The original reprojecting edge path and all soft
blocker-search/filtering code remain intact.

There is no budget change: 128 maps, every contributing light, atlas size and
resolution, LC2 source softness and ray sample counts are preserved. No API,
receiver/hit representation or user shading restriction changes. P-S4B's
`virtual_shadow.wgsl`, `virtual_shadow_gpu.rs` and sunvis files were not edited.
No light-space barycentric point location or height reconstruction was added;
the VIA policy does not select or motivate a new shadow representation here.

## Profile first

Fresh optimized main, RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D,
Windows/Vulkan, 1920x1080 ARA1 shadowed 64-light scene. Nsight Graphics 2026.4.0
GPU Trace uses a 1200-submit warm trigger, 50 ms maximum / 200 MB buffer,
sampled shaders, WGPU_DEBUG=1, the filtered Nsight-only Vulkan layer directory,
unaltered clocks and no screenshot or UI. The 50 ms window contains two shading
ranges, averaging **7.641 ms**. `vsc_claim` / `vsc_missed` average
**0.980 / 0.999 ms**. These are GPU ranges, separate from drained frame service.

Primary shading sampled-state shares: `rl_local_shadow_map` **49.00%**,
`light_sample` 9.86%, `rl_local_tap` 6.82%, `unit_normal` 6.38%,
`rl_pcf_weight` 3.88%, `rl_light_index` 3.80%. The hottest source line is the
dynamic row selection inside interior hard PCF. Shares are attribution, not
additive function times or occupancy measurements. This selects a map-loop
code-generation experiment rather than a clustering rewrite.

Read the `nsight-graphics-install`, `wgsl-array-params-copy` and
`shader-codegen-cost-hazard` memory notes. No array-valued hot helper parameter
was found in the owned light-loop code. The dynamic vector loop was screened
against an independently frozen main binary because equivalent shader source
can change backend code generation. P-LTS2/3's rejected record caches, BRDF
hoist and tangent memoization were not repeated.

Frozen executable SHA-256 identities (`ara1_capture`, Cargo release):

- Main: `23c92d0775978e4f5717482dfd84acdd3c24718d78124e97db4793aac86f0bad`.
- Candidate: `6b9b94d92ff4e593e9df25674deb76769dda4b6fdd47a906d6402681bcf4a785`.

## Paired service measurements

Native 1080p; original ARA1 88-frame still/motion/cut replay, followed by a cut
back to the original still camera, 128 warm frames and eight measured frames.
CPU submission-to-GPU-idle service excludes PNG/HDR capture and hashing.
One compiler-quiet ABBA block per configuration, two runs per side. Table values
are the median of those two eight-frame run medians, not pooled frame medians.
Ordinary OS activity remains; clocks were not locked.

| Steady service, ms | Main | Candidate | Interpretation |
|---|---:|---:|---|
| Shadowed RT | 6.672 | 6.678 | Flat (+0.09%) |
| Shadowed non-RT | 13.256 | 12.131 | **8.49% reduction** |
| Unshadowed RT | 4.335 | 4.353 | Flat within observed spread (+0.42%) |
| Unshadowed non-RT | 8.714 | 8.661 | Flat within observed spread (-0.60%) |

Shadowed non-RT's main run medians are 13.122 / 13.391 ms; candidate
11.890 / 12.372 ms. Both candidate runs beat both main runs. The early rejection
screen independently measured 13.178 to 12.013 ms, with identical images;
its original still/move/cut-first/cut-settled medians improved
14.670/18.184/40.861/16.009 to 13.471/17.141/39.544/14.852 ms.
Only the retained variant received lib tests and ABBA. No other trial was built.

These current-main pairs supersede neither historical P-LTS measurements nor
Ara measurements. The older 11.08/18.00 and 9.25/11.98 ms figures are not this
baseline. There is no claimed RT or unshadowed gain, no Ara win, and no new
Unreal/reference-convergence claim.

## Evidence and verification

Ignored evidence root: `D:/Coding/renderlab-work/p-lts4/`.
`identities.json` records full baseline commit, both shader digests, binary
digests, Cargo commands and build admission. `evidence-summary.json` retains
the ABBA run medians and host audit. `profile-main-nonrt/` retains catalog,
composed source, hotspots, function shares, timing ranges and raw-trace digest.
`runs/a-screen-1` and `runs/a-abba-1` retain scene/asset/binary hashes and all
original still, motion, cut and warmed-tail HDR/PNG witnesses.

- **200 lib tests passed**, zero failed; 164 opt-in GPU tests ignored.
- All four 1080p ABBA configurations match HDR and PNG bytes across all four
  executions, including the warmed tail. Empty/missing witness sets fail.
- Verified by inspection: `inspection/a-screen-1.png`, baseline/candidate
  non-RT shadowed still, motion, cut and warmed-tail images. Silhouettes,
  local shadows and reflective objects retain the baseline image.
- Additional original 960x540 replay: **not completed**. Three attempts stopped
  on foreign Cargo activity before producing a scored pair; a fourth unadmitted
  waiter was canceled after further compiler waiting. These attempts contribute
  no quality or performance evidence. The complete retained matrix above is
  native 1920x1080. No new full-quarry or soft-local fixture campaign was run.
- Also inspected `inspection/a-abba.png`: all four configurations, still, motion,
  cut and warmed-tail candidate witnesses; all corresponding baseline hashes match.

The profile, early screen and ABBA have zero observed rustc processes and zero
sampled foreign builds. All admit above 8 GiB; their runtime minima are
6518 / 6818 / 6805 MiB, above the standard 2 GiB runtime guard. All own builds
take the atomic build-slot directory with owner.json and the normal stillkit
mutex, one Cargo command per acquisition, two Cargo jobs. GPU commands use
`scripts/p-lts4-run.py` through `scripts/n8-run.py`, stream the latest root
order and use fresh labels. No lease exceeds fifteen minutes.

Implementation build time: baseline 159.54 s; retained example **38.36 s**, lib-test
build **111.89 s**. Verification GPU occupancy
is 92.42 s profile, 140.01 s screen and 244.79 s ABBA; queue waits are
separate. Interrupted extra-quality leases occupied 6.60 / 6.49 / 12.63 s
before stopping on foreign Cargo/rustc, with over 8 GiB available; the shared
runner's generic contention-or-memory message was not a memory-floor failure.
The final runner requires a 120-second compiler-free admission interval after
these repeated gaps between foreign commands. Its final unadmitted waiter was
stopped; no owned build, GPU job or waiter remains. Implementation/exploration
ran approximately 00:24-00:32 UTC; retained verification completed at 00:46:37
UTC. Additional quality attempts and handoff waits closed around 01:02 UTC.
Development elapsed time is not frame performance.

Cleanup was attempted using native PowerShell with the resolved own target
checked against its exact parent. Automatic approval review rejected the
target/trace removal command as **"blocked by policy"** before execution.
The own target `D:/Coding/renderlab-work/p-lts4/target` and summarized raw
trace `profile-main-nonrt/base_2026_11_10_03_31_00.ngfx-gputrace`
(43,128,460 bytes) therefore remain for root cleanup. The target occupies
703,409,371 bytes; `target-cleanup.json` records the handoff. No alternate deletion
method was attempted. The raw trace's SHA-256 is recorded separately.
