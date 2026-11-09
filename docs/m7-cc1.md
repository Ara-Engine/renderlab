# M7-CC1: cold shader compilation

Overnight unit, based on main `4c8ef7d`, branch `t3/m7-cc1`. Scope is shader
structure and compilation only: preserve programmable programs, MP1's independent
per-program pipelines, rendering algorithms and complete-image output. PC1's cache
remains enabled by applications that request it.

**Outcome: retain the small single-call-site GI refinement loop.** Completed game
and quarry comparisons are bit-identical in final RGBA8 output, with no measured
frame-time regression. This is a partial cold-compile improvement, not a solution
to first-install/edit hitches. The full requested matrix is incomplete: the unit
yielded to ARA1's held window after its current render, as detailed below.

## Native stage screen

Windows 11 Pro 10.0.26300, Ryzen 7 7800X3D (8 cores/16 threads), 32 GB RAM,
RTX 4070 SUPER (12 GB), NVIDIA 617.14, Vulkan/wgpu 30.0.1.
Development-profile test executable, renderer opt-level 1, naga/wgpu opt-level 2.
These are single samples on the shared machine with other workers and CPU builds;
they are diagnostic observations, not a quiet-window performance certificate.
The initial screen overlapped Cargo/rustc/link processes, including CC1's first
release build. The shared-call and GI-only game gates also sampled other workers'
Cargo/rustc processes. The GI-only gate's minimum sampled free RAM was 9,201 MiB.

`gpu_cc1_codegen` composes the game photoreal program's mixed-program Soft list
pipeline on each path. Each variant changes one stage, never renders an ablated
pipeline, and includes a fresh live tint to avoid reusing an earlier shader. Native
timing surrounds `vkCreateComputePipelines`; bytes are the complete SPIR-V module
given to Vulkan. The driver then removes unreachable functions and specializes its
call graph, so module size alone does not describe the size of the optimized kernel.
Module creation was 12.8-20.5 ms in this screen.

| Removed stage | RT native seconds | RT SPIR-V bytes | Non-RT native seconds | Non-RT SPIR-V bytes |
|---|---:|---:|---:|---:|
| None | 38.612 | 397,800 | 28.473 | 409,408 |
| Reflection hit shading | 21.368 | 397,472 | 19.831 | 409,096 |
| GI lookup | 17.160 | 354,184 | 18.850 | 365,852 |
| Program light loop | 23.653 | 397,400 | 28.385 | 409,008 |
| Decals | 27.374 | 380,660 | 33.768 | 392,268 |
| Software traversal | n/a | n/a | 18.307 | 395,268 |

GI lookup and hit shading/traversal both contribute substantial compile work. These
ablations are not additive: removing decals is slower in the non-RT sample. The
retained optimization must be measured as a complete shader. The software-trace
probe returns a miss, so downstream hit shading can also fold away; its delta is
the compiler's response to that removal, not an isolated traversal microbenchmark.

The first refinement-loop diagnostic used WGSL's reserved `target` identifier.
Its module was rejected and no native compile ran; that row is excluded. The
diagnostic now validates its WGSL/IR explicitly, checks wgpu error scopes, and
requires a native timing record, so an invalid pipeline cannot count as a fast one.

## Structure candidates

The small refinement loop keeps the original base-level / finer-level / restore
sequence with one `gi_sample` call site. A second prototype routed all receiver
lookup phases through one sampler. In the next native screen, small loop / full
routing were **23.404 / 21.908 s RT**, and **25.726 / 24.916 s non-RT**. Those modest
extra savings do not justify the routing prototype's added control flow; it is not
retained. Both complete SPIR-V modules actually grew (399,160 / 405,088 bytes RT,
410,792 / 416,584 bytes non-RT), illustrating why raw module bytes alone are a poor
proxy for driver expansion.

[DL4](dl4-env-helpers.md) already rejected looping the four ordinary environment
taps: its non-RT runtime was 4-8% slower. CC1 reuses that evidence and preserves
the four expressions and their texture-load parallelism.

The larger prototype shared the engine's calls into opaque user `Surface` and
`shade` code. Its per-pixel sequence was:

1. Build and retain the receiver's surface with open indirect inputs.
2. Process the same one or four reflection rays, in the same order. For each hit,
   reconstruct the same inputs, load the same MP1 parameter block, call surface
   and shade, then add its radiance in the original order.
3. Restore receiver parameters, prepare its indirect and clustered-light inputs,
   and shade the retained receiver surface.

These phases used one surface call site and one shade call site. Primary shading
occupied the final phase for every lane, even when their ray counts differed. The
engine stored opaque program types without inspecting material-model fields or
combining programs. Its paired native screen was:

| Shader structure | RT native seconds | RT bytes | Non-RT native seconds | Non-RT bytes |
|---|---:|---:|---:|---:|
| Main | 36.854 | 400,832 | 34.182 | 412,344 |
| Shared surface/shade, runtime phase bound | 24.500 | 408,772 | 17.699 | 420,288 |
| Shared surface/shade, constant phase bound | 23.582 | 405,524 | 15.883 | 417,132 |

The runtime-bound variant was compile-only; no host frame field was changed. The
constant-bound prototype rendered all three application-cache modes on both paths
in both builds. Every captured game pixel matched main, but release drained frame
time rose **5.832 to 10.709 ms RT (+84%)** and **9.192 to 10.705 ms non-RT (+16%)**.
Development frame time rose 5.882 to 10.676 ms RT and 9.180 to 10.720 ms non-RT.
This prototype is **rejected and fully reverted**, including its film/transparency
adapters and changed reflection contract tests. A lower compile time cannot trade
away the required steady-state performance.

No lighting-model shortcut, GI/shadow/reflection/light algorithm change, feature
reduction, program merging, new receiver schema or background fallback is shipped.
A separately compiled trace/hit-shade architecture remains a follow-up; it was not
implemented or measured in this time-box.

## Startup and image baseline

The example uses the PC1 harness with its ordinary quality defaults: GI, TR1,
reflections and the game props. Game resolution is 1280x720. RT uses traced
primaries/Interactive shadows; non-RT uses the visibility buffer/Virtual shadows.
Each process renders 48 frames, hashes complete RGBA8 frames 0 and 47, then measures
a drained 32-frame segment including final-colour copy/readback, without PNG encoding.
The opt-level-1 development build and optimized release build are reported separately.

Main `4c8ef7d` startup-to-first-image, seconds:

| Build/path | Cache disabled | Empty application cache | Populated application cache | Drained ms/frame |
|---|---:|---:|---:|---:|
| Dev RT | 1.135 | 1.206 | 1.065 | 5.882 |
| Dev non-RT | 1.187 | 1.286 | 1.142 | 9.180 |
| Release RT | 30.760 | 0.723 | 0.833 | 5.832 |
| Release non-RT | 36.540 | 0.708 | 0.603 | 9.192 |

`--no-pipeline-cache` controls the application cache; NVIDIA's private cache remains
uncontrolled. A later empty-application-cache process can hit that private cache,
and the dev baseline was privately warm. These labels are not universal cold/warm
speedup claims. Fresh live shader constants in the stage screen and edit probe
exercise previously unseen variants separately from this ordinary startup matrix.

GI-only candidate, same game matrix (seconds except the final column):

| Build/path | Cache disabled | Empty application cache | Populated application cache | Drained ms/frame |
|---|---:|---:|---:|---:|
| Dev RT | 25.914 | 1.162 | 1.154 | 5.758 |
| Dev non-RT | 25.194 | 1.203 | 1.151 | 8.803 |
| Release RT | 21.356 | 0.791 | 0.630 | 5.747 |
| Release non-RT | 22.370 | 0.639 | 0.571 | 8.791 |

All game hashes equal main, across both builds, both paths and all cache modes.
Release drained frame time shows no regression in this screen (-1.4% RT, -4.4%
non-RT); it is one shared-machine timing run per configuration, not a quiet-window
certificate. The dev disabled-cache comparison begins with a privately warm main
and an unseen candidate, so 1.135 to 25.914 s is not an optimization regression.

The release disabled-cache startup samples fall by 31%/39% (RT/non-RT), but that
entire difference cannot be attributed to CC1. The two per-program list API calls
were **3.662/25.678 s before versus 0.014/20.494 s after on RT**, and
**5.315/33.258 s before versus 0.015/21.472 s after on non-RT**, sorted by duration.
The smaller pipeline is privately warm in the after run. The fresh-constant native
stage screen above is the separate evidence for the GI-heavy program. No large
reduction in the smaller program's unseen compile has been demonstrated. Driver
compilation of the candidate still takes tens of seconds.

The main and rejected shared-call candidate game matrices have identical complete
frame hashes for every cache mode in dev and release:

| Path/frame | RGBA8 SHA-256 |
|---|---|
| RT 0 | `4cf21a5e44625f8a0501d334d4420b98bd9d96a3ec1712f226ad4219786ae23a` |
| RT 47 | `e21e87450a5ec715211735cabf3a5932405ab11190b2cba7bccfa69ed5f39a07` |
| Non-RT 0 | `08738b14fb5615dcbbdc33b45705d2df2e46b8ff9b42b4708c198078516e58e4` |
| Non-RT 47 | `47f5c40cec9e8a164b11e9c86b32a89f6e768226229ad58f4bc8fcf568902326` |

The inspected main contact sheet and GI-only release frames on both paths contain
the wall, arch, crates, red/blue/metallic
objects, toon hero and decal, including their reflections and shadows. Existing
non-RT edge/reset artifacts are not a CC1 quality claim or a cross-path identity
claim. Quarry uses the full registered 13,034,903-triangle asset at 1920x1080,
replacing the ground with the same game props and camera. It is not the low-level
viewer's registered quarry walk. Its inspected dev frames on both paths preserve
the rocks, silhouettes, props, decals and each path's existing reflection behavior.

Quarry with application persistence disabled:

| Build/path | Main startup s | CC1 startup s | Main ms/frame | CC1 ms/frame |
|---|---:|---:|---:|---:|
| Dev RT | 34.083 | 31.069 | 8.146 | 7.932 |
| Dev non-RT | 50.355 | 44.368 | 11.805 | 11.739 |
| Release RT | 28.954 | 28.202 | 8.128 | 7.890 |
| Release non-RT | 44.392 | not run: held | 11.914 | not run |

Frames 0 and 47 match main exactly in all three completed quarry pairs. RT hashes
are `644aa9e027c057a19b62505718a282ff4c6777485522ba664f4507266f28e17b` and
`dcb34ac332b4e258e28594c7d4f5b4bcd63a7cb3b9dac71fa6a2001b9b5bf586`;
non-RT hashes are `f288e855ab911c68f5c539cdf352757f2cae83ea3ed728dc6c9b21f09716eade`
and `0b52e2674d91e0213479dc5f4aa6643fdd99c745134aaf8a48179c17ede2353e`.
The release main hashes also equal its dev hashes. The missing CC1 release non-RT
quarry run is not certified by that observation.

The first startup batch completed the main dev game's six cache/path configurations
and its lib suite (**191 passed, 0 failed, 159 ignored**). Frame 0 and frame 47 are
identical across the three cache modes on both paths. The first quarry preparation
then crossed the live memory guard (minimum 1,726 MiB available), and n8 stopped
the process. That incomplete quarry run supplies no startup/image result. Further
quarry launches require at least 7,168 MiB before loading the asset; the 2 GiB live
abort floor remains in place.

Coordinator holds for MV1's final release and root gates b10/b12 were honored,
including the distinction between attempt/deferral and final release lines.
Heavy Cargo work paused until MV1's tests started. The later RD1, VF2 and LC3
priority order was honored, including RD1's subsequent steps. At 00:37:22 UTC a
new anchored root order reserved ARA1's held startup window. CC1 finished only its
active release RT quarry process and released at 00:37:34, without starting the
release non-RT process. No more GPU leases, Cargo builds or test renders are needed
for this unit. The held window is not bypassed for missing verification.

## Remaining gaps

- The large-factor goal for both heavy programs is not met. The smaller program's
  unseen compile is not improved by evidence here; the GI-heavy program still
  takes about 20-26 seconds. A complete split into independently compiled trace
  and programmable hit-shading passes, or a correct asynchronous fallback, remains
  unimplemented. The faster shared-call prototype failed the frame-time constraint.
- Release non-RT quarry identity/timing, empty/populated application-cache quarry
  startup on both builds/paths, and CC1's fresh API edit-to-image matrix on both
  scenes/builds/paths were not completed before the time-box/ARA1 hold. PC1's
  historical 31.335-second RT release edit remains a before observation, not a new
  CC1 after measurement. Native screens use fresh shader constants but do not
  replace an API edit/readback measurement.
- The game and quarry startup samples have an uncontrolled driver-private cache
  and CPU-build contention. No quiet-window large-factor claim, physical scanout,
  file-watch debounce, new Unreal comparison or exact-repeat certification is made.

## Reproduction and evidence

`scripts/m7-cc1-run.py` reuses PC1's startup matrix and edit/readback endpoint. New
bulk files and the two-job Cargo target live under `D:/Coding/renderlab-work/cc1`.
Every lib/GPU batch uses `scripts/n8-run.py`, a lease of at most 15 minutes, the
shared lock, game/RAM guards and the latest root order. Queue waits stay in-process.
`reviewed-order.txt` must match the exact directive the agent has read; a changed
directive holds the next acquisition for review instead of guessing its priority.
An optional `unit-deadline.json` with an ISO `latest_process_start` stops later
processes; reset it and review the latest root order before a new unit's replay.
All rendering is headless; no desktop or foreground-game capture is used.

```text
py -3 -B scripts/m7-cc1-run.py build dev
py -3 -B scripts/m7-cc1-run.py build release
py -3 -B scripts/m7-cc1-run.py gpu ablation-a screen
py -3 -B scripts/m7-cc1-run.py gpu gi game-gate
py -3 -B scripts/m7-cc1-run.py gpu gi quarry-check
py -3 -B scripts/m7-cc1-run.py gpu gi all
```

Stage records: `runs/ablation-a/{rt,nonrt}/codegen.json`, `runs/screen-b`,
`runs/gate-c`, the composed WGSL files and native-call logs. Main game matrix:
`runs/base-final-{dev,release}`. Rejected shared-call matrix: `runs/final-{dev,release}`,
summarized in `final-summary.json`. Retained game matrix: `runs/gi-{dev,release}`
and `gi-summary.json`; paired quarry results: `quarry-summary.json`. Before binaries were staged under `baseline/`;
their source changes were diagnostic-only and did not change normal frames. Each
matrix has an executable SHA-256 manifest; it refuses to mix executables on resume.

Lib suites inside the owned leases: main, shared-call candidate and GI-only
candidate each **191 passed, 0 failed, 159 ignored** (4.20/3.62/3.64 s). Native stage
screens completed successfully on both paths. Clippy on the GI-only source
completed successfully (16 lib and
27 test warnings already present elsewhere); formatting, diff whitespace and Python
syntax checks pass. Completed game/quarry identity and timing checks pass. The
final matrix process returned 75 to yield to the new root order; its lease record
has no failures. This is an intentional incomplete matrix, not a failed lib test.

Implementation, tooling and report editing took about 80 active minutes (manual
estimate), interleaved with compulsory queue holds. Wall time was about three
hours. Verification used five owned leases totaling **19.62 minutes**, including
guard/asset-preparation time: ablation-a (283.29 s), screen-b (133.34 s, memory
abort after its valid compile records), gate-c (345.11 s), GI game (127.84 s), and
GI quarry (287.57 s, root-order yield). All individual leases were below 15 minutes.
The final GI-only Cargo builds reported 81 s for the dev example, 65 s for the dev
lib executable and 31.20 s for the release example, always with two jobs. Earlier
builds overlapped queue/implementation time; their total was not separately timed.

All owned jobs ended before cleanup. No raw GPU buffer dumps were produced.
Automatic approval review rejected the exact-path PowerShell deletion of
`D:/Coding/renderlab-work/cc1/target`, stating only **"blocked by policy"**.
The target remains **3,240,485,241 bytes (3.24 GB / 3.02 GiB)** for coordinator
cleanup; no alternative deletion method was attempted. SHA manifests, JSON/log
summaries, inspected PNGs and small source dumps remain under the evidence root,
along with staged before/prototype executables outside the target.
