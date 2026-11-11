# P-FREE: engine shader runtime checks

**MERGE** the retained shader/indirect-validation policy. Still and moving GPU work is faster with unchanged inspected output. Workgroup zero-initialisation stays enabled.

Implementation unit, 2026-11-10. Base: `17bb249` (v0.1.1 plus GPU-layer report); retained implementation
and measurement identity are recorded below. Bulk evidence: `D:/Coding/renderlab-work/p-free`.

## Implementation

One central module-creation helper calls `create_shader_module_trusted` with either
`ShaderRuntimeChecks::unchecked()` or `checked()`. Engine module sites explicitly
use the device facade's engine method. The same descriptors, error scopes, pipeline
cache and reload transaction are used in both modes. No duplicate pipeline path.

`EngineDescriptor::checked_engine_shaders` and the environment override are documented
in [api.md](api.md#shader-runtime-checks-p-free). Arbitrary program sources, transparent
program compositions and background glass compilation stay checked. (The exact bundled
standard program bytes were trusted here; P-SPLIT made every program module checked, so an
edit recompiles only the kernels its text changes.) Custom low-level preludes latch
future compilation into checked mode, including across worker clones and after a rejected
edit. Existing modules still contain their originally compiled source.

`engine::instance_flags()` disables indirect-call validation for engine-generated bounded
arguments, with explicit wgpu environment overrides retained. Both the headless adapter and
viewer use it. Adapters supplied by callers must use this instance policy themselves:
wgpu does not expose a device-level switch after adapter creation.

The probe's unchecked module API choice is reused; its Vulkan interception and duplicate
measurement pipelines are not imported. The P-FREE capture example reuses ARA1's full
scene setup and timeline, adding optional raw HDR dumps for numerical comparisons.

Only four module method names change in `src/virtual_shadow_gpu.rs`. No shadow algorithms
or `sunvis_s4*` files are changed.


## Verification and timings

Windows; Ryzen 7 7800X3D; RTX 4070 SUPER (12,282 MiB); NVIDIA 617.14. Native 1920x1080, ARA1 phase-2 scene settings and 88-frame camera timeline. No quality setting changes. Debug/development Rust build: renderlab opt-level 1, wgpu/naga opt-level 2; these are matched GPU timings, not release CPU-throughput claims.

ARA1 synchronous mode is enabled: one explicit GPU drain per frame. GPU span excludes uploads/readbacks. A = checked engine modules plus indirect validation; B = unchecked engine modules with indirect validation off. One guarded ABBA per scene/path, identical binary, six captures per run (39, 70, 71, 72, 73, 87). Still excludes frames 0-7; moving is 40-71; cut window is 72-87; first cut is frame 72. Table values pool the two runs per mode and take their median GPU span. First-use/cut spans are variable and are reported separately rather than hidden.

| Scene | Path | Phase | A ms | B ms | Time saved |
|---|---|---|---:|---:|---:|
| game-base | nonrt | still | 6.957 | 6.213 | 10.69% |
| game-base | nonrt | move | 10.183 | 8.665 | 14.90% |
| game-base | nonrt | cut | 10.045 | 8.445 | 15.93% |
| game-base | nonrt | first_cut | 119.635 | 90.117 | 24.67% |
| game-base | rt | still | 3.021 | 2.743 | 9.20% |
| game-base | rt | move | 4.523 | 4.300 | 4.92% |
| game-base | rt | cut | 24.720 | 4.478 | 81.88% |
| game-base | rt | first_cut | 66.759 | 89.980 | -34.78% |
| ara-pbr-base | nonrt | still | 4.746 | 4.215 | 11.19% |
| ara-pbr-base | nonrt | move | 4.489 | 3.799 | 15.38% |
| ara-pbr-base | nonrt | cut | 5.251 | 4.383 | 16.52% |
| ara-pbr-base | nonrt | first_cut | 72.389 | 49.904 | 31.06% |
| ara-pbr-base | rt | still | 2.303 | 2.159 | 6.27% |
| ara-pbr-base | rt | move | 3.056 | 2.812 | 7.97% |
| ara-pbr-base | rt | cut | 33.211 | 30.145 | 9.23% |
| ara-pbr-base | rt | first_cut | 72.446 | 45.350 | 37.40% |
| quarry-base | nonrt | still | 9.689 | 9.229 | 4.75% |
| quarry-base | nonrt | move | 10.139 | 9.956 | 1.81% |
| quarry-base | nonrt | cut | 28.872 | 27.461 | 4.89% |
| quarry-base | nonrt | first_cut | 27.250 | 22.888 | 16.01% |
| quarry-base | rt | still | 3.387 | 3.385 | 0.06% |
| quarry-base | rt | move | 3.320 | 3.255 | 1.97% |
| quarry-base | rt | cut | 13.918 | 12.337 | 11.36% |
| quarry-base | rt | first_cut | 14.238 | 14.146 | 0.65% |
| quarry-daylight | nonrt | still | 9.647 | 9.294 | 3.66% |
| quarry-daylight | nonrt | move | 10.037 | 9.853 | 1.83% |
| quarry-daylight | nonrt | cut | 12.380 | 11.641 | 5.97% |
| quarry-daylight | nonrt | first_cut | 73.616 | 43.616 | 40.75% |
| quarry-daylight | rt | still | 3.397 | 3.260 | 4.02% |
| quarry-daylight | rt | move | 3.334 | 3.249 | 2.56% |
| quarry-daylight | rt | cut | 11.996 | 4.084 | 65.96% |
| quarry-daylight | rt | first_cut | 33.327 | 43.975 | -31.95% |

## HDR evidence

Pairs are A0/B1 and A3/B2, six frames each. Max abs covers all finite RGBA16F components; pixel count means any differing component. NaN/Inf counts cover both images. SHA-256 comparisons independently confirm byte identity.

| Scene | Path | Pairs | Max abs | Max differing pixels/frame | NaN/Inf |
|---|---|---:|---:|---:|---:|
| game-base | nonrt | 12 | 0 | 0 | 0 |
| game-base | rt | 12 | 0 | 0 | 0 |
| ara-pbr-base | nonrt | 12 | 0 | 0 | 0 |
| ara-pbr-base | rt | 12 | 0 | 0 | 0 |
| quarry-base | nonrt | 12 | 0 | 0 | 0 |
| quarry-base | rt | 12 | 0 | 0 | 0 |
| quarry-daylight | nonrt | 12 | 0 | 0 | 0 |
| quarry-daylight | rt | 12 | 0 | 0 | 0 |

## Zero-initialisation experiment

Workgroup zero-initialisation is a separate PipelineCompilationOptions flag in wgpu 30; ShaderRuntimeChecks::unchecked() does not disable it. The experiment toggled only that compute-pipeline option, with unchecked engine modules and indirect validation off in both modes.

| Path | Phase | Initialise ms | Skip ms | Time saved |
|---|---|---:|---:|---:|
| rt | still | 2.830 | 2.843 | -0.47% |
| rt | move | 4.311 | 4.336 | -0.57% |
| rt | cut | 4.489 | 4.502 | -0.29% |
| rt | first_cut | 82.326 | 82.460 | -0.16% |

Both non-RT skip-zero attempts timed out after first readback, before frame 39 (120.129 and 120.784 seconds). The initialise controls completed in 8.530 and 8.833 seconds. No complete non-RT HDR comparison exists for skip-zero; differing-pixel and NaN/Inf statistics there are unavailable. RT completed: all 12 HDR pairs have max abs 0, differing pixels 0 and NaN/Inf 0; the inspected frames match, but performance does not improve. **REJECT skip-zero**: its code and environment switch were removed. The rejected experiment is archived in `zero-init-experiment.patch` under the evidence root.


## Targeted RT-cut recheck

First-cut samples (ms): checked [86.275072, 82.004992, 79.405056, 72.261632]; unchecked [51.051520000000004, 85.423104, 84.896768, 50.640896]. Medians 80.705 -> 67.974. The original run remains in the table above.

## Build identity and evidence

Base 17bb249560f18b8f34f3d32ed387bb365ef2b4c6; timing executable SHA-256 `6aeee6ddcdea8b60d36ba266d634ec41fea98c5b7c2031783b050f09015952ec`; source manifest digest `712ca8e45e7eaa93313603a01b9ca6437fc1cd25fb031a11d3855f7e4c58a7c4`. Full identity: `D:/Coding/renderlab-work/p-free/build-identity.json`.

Guard records: `lease/runs/abba-1.json` and `lease/runs/zero-cut-1.json`; raw timing rows: `runs/*/engine.json`; numerical comparisons: `summary.json`, `summary-zero.json`; plans and analysis scripts remain in the same evidence directory. No assets or measurements enter the public repository.

## Visual inspection and acceptance limits

Looked at all six captured states on both paths in game-base, PBR, quarry-base and
quarry-daylight: settled 39, adjacent moving 70/71, first cut 72, next frame 73 and
settled-after-cut 87. The eight `*-sheet.png` contact sheets are under the evidence
root. Also inspected native-resolution details in `game-base-nonrt-crops.png`,
`ara-pbr-base-nonrt-crops.png`, `ara-pbr-base-rt-crops.png` and
`quarry-daylight-crops.png`; the rejected zero-init RT pair is `zero-rt-sheet.png`.
The original full-resolution PNGs remain in `runs/<case>/frame-<index>.png`.

No new stray white/bright pixels, noise, silhouette changes, shadow changes or trails
were seen between the paired modes. This does not call the existing image flawless:
game frame 72 has glossy grain and stepped edges; the mirror has a small bright patch
in both modes (visible in the native crop). PBR has cut-edge jaggies and bright broken
highlight chains, and its RT image is darker than non-RT. These are existing issues,
identical between A/B, rather than fixes made by this unit. Quarry-daylight's cavities,
textures, contacts and silhouettes remain unchanged on movement and cuts. Quarry-base
is nearly black at its inherited -14 exposure; the pre-existing ARA1 daylight preset
(-8.5) was added to make visual inspection meaningful, with the same exposure in A/B.

All **96 paired main-change HDR comparisons** have max abs 0, differing-pixel count 0,
NaN 0 and Inf 0. All 96 also match HDR SHA-256 and PNG SHA-256 exactly; see
`png-identity.json`. These are sampled full-frame comparisons, not a certification of
all renderer features or of Unreal superiority.

The clear steady/moving wins are game and PBR. Quarry improvements are smaller;
quarry-base RT still (0.06%) is neutral within run variance. Cut timing is not uniformly
better: original game RT first cut regressed 34.8%, and daylight quarry RT first cut
regressed 32.0%. The targeted game RT repeats reverse the median direction, with a
wide/bimodal unchecked distribution. No uniform first-cut improvement is claimed.
The recheck uses the same camera timeline and PNG captures, with HDR hashing disabled.
The merge recommendation rests on the steady/moving GPU gains and unchanged visible
output, following the November 10 quality bar; the cut-latency misses remain explicit.

## Checks, execution time and cleanup

Final retained-source build passed (`compile-final`, 65.271 s). Lib tests ran in both
the original and final leases: **200 passed, 0 failed, 164 ignored** each time. The
ignored GPU suites were not run as a separate campaign; the ARA1 captures above are
the device verification for this unit. No viewer/window or desktop capture was used.

Implementation and harness preparation took roughly 25 minutes of elapsed work,
overlapping resource waits. Cargo execution took 289.037 s total (including the first
compile attempt, whose missing internal helper arguments were fixed before testing).
GPU lease execution took 544.909 s for the broad ABBA, 481.564 s for daylight/zero-init
and 71.770 s for final tests/cut repeats: 1,098.243 s total. The middle lease was ended
intentionally after the zero-init rejection, before its planned cut repeats; those
repeats were moved to the final lease. All three guards recorded zero rustc processes
and no resource/contender failures; minimum available memory was respectively
10,996.6, 12,974.4 and 15,123.4 MiB. Each lease was below 15 minutes. Priority queue
waiting is excluded from these execution totals and dominated overall elapsed time.

Builds used the atomic build slot plus the normal stillkit mutex, one Cargo command
per acquisition, two jobs, >=8 GiB available and >=5 GB free on D:. Ara requests were
given priority; no foreign lock was taken or deleted. GPU admission used
`scripts/n8-run.py`; requests and releases are in the shared schedule.

Deleted 228 raw HDR dumps (3,782,246,400 bytes); numerical summaries, hashes, logs,
plans, the rejected experiment patch and inspected PNGs remain. Automatic approval
review rejected recursive deletion of `D:/Coding/renderlab-work/p-free/target` and
`runs/program-cache` as **blocked by policy**. The command was not executed; those
owned directories remain for cleanup. No alternate deletion method was attempted.

Implementation commit: `4129a84193c1f347f2e4eb1b3b8489262f3f7985`. Final retained executable SHA-256: `00fc57d36b0e567d3995783544002cfa893138f1c024ba6d08d6bec4d7f08041`. Final source manifest: `5998fd1c5234497a051317104b1cd2b8af8be3f615a8ed8ed87aaff54b63d64d`. The final build removes only the rejected zero-init switch and its measurement metadata; default pipeline options remain zero-initialised, matching B in the successful image matrix. Final guard: `lease/runs/cut-final-1.json`.
