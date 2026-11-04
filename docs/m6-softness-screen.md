# Directional visibility-disk screen

The first bounded softness attempt is rejected. It implemented a typed optional angular extent and 32 directional visibility queries per receiver sample, then failed the predeclared small-fixture quality and cost gates. No renderer change is retained. All source and tests are restored to base `35e23df96f1b70be40d906e2fc8129fe6257b657`. This document records the result for root's review. No quarry campaign, child, commit, push or PR followed the failure.

## Implemented model and boundary

The rejected patch adds `DirectionalVisibilityDisk::new(half_extent_radians)` and an optional field on `Shadows`. The finite allowed half-extent is 0 through 0.1 radians. Positive values require hardware directional shadows. Raster and local requests reject before settings mutation. None and zero use the unchanged hard-shadow query. The fixture confirms identical HDR output for None and zero at both 1x and 4x. Unsupported-input transaction behavior was inspected in source, not separately device-tested after the rejection.

The model averages binary visibility uniformly over a slope disk. For unit central direction L and transported light-view axes U and V, each direction is `normalize(L + tan(alpha) * (u*U + v*V))`. Thirty-two equal-weight Vogel points use radius `sqrt((i+0.5)/32)` and angle `i*2.39996323`. The pattern remains fixed across space and time. Every actual MSAA receiver uses all 32 directions, up to 128 queries per fully covered 4x pixel before overdraw.

This is a visibility-only approximation, not physical directional-disk lighting. Existing central-direction BRDF evaluation and light lux remain unchanged. The measure is uniform slope area, not solid angle. Glossy highlights do not broaden. Physical disk integration would require per-direction BRDF/cosine evaluation and a defined irradiance normalization.

The geometric-normal origin remains oriented toward central L. Each perturbed ray computes its own normalized-depth rate, `-delta.z`, depth-bias interval start and first exit from the original finite light-space box. The central packed rate remains confined to the hard branch. Cascade selection, normal bias, main-camera reversed depth and forward light-space depth remain unchanged. The patch does not expand caster coverage or claim finite-volume boundary parity beyond the tested fixture.

## Exact-reference fixture and result

The authored fixture is 257 by 193 pixels, at both 1x and 4x, with alpha 0.04 radians and the existing depth/normal biases. The receiver is a two-triangle plane. A two-triangle horizontal blocker supplies five cases: contact at 0.06 m, distances 0.8 m and 1.6 m, a 0.025 m strip at 0.8 m, and a receiver with slope 0.35. Each case has a static image, a 0.001 m camera translation, and a light-direction X change of 0.001 before normalization. There are 30 full-frame HDR captures. Blockers behind the camera remain in the TLAS.

The CPU oracle unprojects finite reversed-Z camera samples in f64 and intersects the authored planes. A blocker boundary becomes a linear inequality in disk coordinates. Its exact area uses `F(x)=0.5+(asin(x)+x*sqrt(1-x*x))/pi`, clamped outside the unit disk. Thin strips subtract two segment areas. Foreground blocker visibility is included. Four samples use the [Vulkan standard sample locations](https://docs.vulkan.org/spec/latest/chapters/primsrast.html#primsrast-standard-sample-locations) and resolve visibility before comparison. Only pixels adjoining a raster silhouette are excluded from numerical gates. Full images include them.

The gates were recorded before implementation in `.refs/m6-softness/decision.md`. Penumbra RMSE must be at most 0.02, p95 error 0.04 and maximum error 0.075. Solid visibility error must be at most 0.002. Thin-strip blocked energy must be within 5%. Transition width must agree within the larger of one pixel or 10%. Excess adjacent-pixel error steps must be at most 0.04. Motion residual RMSE and p95 must be at most 0.02 and 0.04. A failed gate stops the attempt.

Twenty-three of the 30 images fail at least one quality gate. The static thin-strip RMSE is 0.0291 at 1x and 0.0211 at 4x. Its maximum excess band step is 0.0994 and 0.0759 respectively. Average blocked energy nevertheless agrees within 2.23% and 0.54%. That average hides the visible bands. Two 1x light-motion cases also fail. The thin-strip motion residual has RMSE 0.0240 and p95 0.0602. All ten 4x motion residual cases pass these bounded motion thresholds, but their static image quality does not pass.

Contact-hardening responds to distance. At 1x, measured 10%-90% widths for contact, near and far are 1.93, 22.20 and 44.60 pixels. Exact widths are 1.92, 22.67 and 45.35. Solid lit and umbra values match exactly. There is no spatial random noise in these row-invariant images. The failure is coherent banding and its motion, not stochastic speckle. A separate CPU evaluation of the same finite pattern matches every observed candidate pixel exactly. That is an implementation cross-check; only the continuous disk reference supplies the quality verdict.

All six full-image sheets were inspected, covering all 30 candidates, references and amplified errors. Distant penumbrae show visible steps. Thin shadows show multiple uneven bands where the reference has a smooth shallow shadow. Contact and foreground blocker silhouettes agree with the reference. 4x softens some steps but retains the defect. These are linear visibility images, not acceptance of final lit materials, glossy appearance or arbitrary moving geometry.

## Short ordinary-frame cost

The release test uses the original lit shader for timing. At each sample count it executes hard, soft, soft, hard blocks, with four warm frames and twelve measured ordinary frames per block. Captures stay outside timing. The existing asynchronous renderer timestamps measure the mesh pass and exclude preceding AS builds. The submitted scene contains both objects; camera culling leaves one receiver draw with two triangles in the timed raster pass. The blocker still participates in hardware shadows.

| Samples | Hard mesh medians, ms | Soft mesh medians, ms | Soft accepted CPU medians, ms | Soft drained ms/frame |
|---|---|---|---|---|
| 1x | 0.0369 / 0.0369 | 0.5832 / 0.5791 | 0.1085 / 0.1084 | 0.8477 / 0.6930 |
| 4x | 0.0326 / 0.0322 | 0.5637 / 0.7501 | 0.0881 / 0.1126 | 0.6366 / 0.7807 |

Both soft medians exceed both hard medians by more than 2x and 0.10 ms, the permissive first-screen rejection threshold. Individual 4x soft mesh samples span 0.5458 to 1.1530 ms. CPU accepted-call time excludes admission retries. The drained proxy includes retries, fill/drain and collection, so it is not presentation throughput. The small geometry workload makes repeated visibility/shader work the evident limiter, but this is not an isolated traversal measurement. Do not extrapolate these ratios to the quarry or infer a sample-frequency speed ratio.

The host is Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan. Preflight CPU load was 15% over 16 logical processors. Free space was approximately 32 GB on C and 179 GB on D. GPU observations before image and timing processes were 37% and 23%, with approximately 3290 MiB occupied. Foreign applications were left running. Contention is uncontrolled and no in-block GPU-utilization trace was collected. The large cost failure suffices to reject this attempt; it does not establish an isolated performance ratio or Unreal comparison.

The only release build took 30.813 s. The validation fixture process took 3.975 s and the unvalidated timing process 5.267 s. These sum to 9.241 s of native verification and 40.055 s including compilation. CPU analysis and implementation were not independently stopwatched. Their durations must not be inferred from these command totals. Active Khronos validation reports zero errors, zero missing layers and two active instances. Timing reports zero active validation layers. Existing unused shadow-attribute warnings remain. No affected CPU/Clippy campaign followed because no code was retained and the first gate had failed.

## Evidence, reproduction and next boundary

Evidence is ignored under `D:\Coding\renderlab-work\m6-softness-screen/`. It contains the pre-edit decision, rejected candidate patch and full source copies, preserved prior release test binary, candidate test executable, per-input SHA-256 identities, exact commands/environment, all raw HDR captures and fixture records, CPU oracle, full-image sheets, quality and motion metrics, and raw cost samples. The accepted prior camera-depth executable at `D:/Coding/renderlab-work/camera-depth/gates-02/test.exe` was untouched. No quarry/RLE2 file was opened, copied or recooked because the first gate failed.

| Artifact | SHA-256 |
|---|---|
| Rejected candidate patch | `5bc430e4a8ea38ce4fc159852f098be8497fd78dd414687566b1254b209b736c` |
| Candidate test executable | `3a87faa577aaa094e98b7abf586286642967df2a0313a63e61a2277f6dccf737` |
| Fixture inputs and cameras | `23f42d165b400f63b75b6b2b17272bf2394906e0ae0aba3438721f9bc595750a` |
| Raw ordinary-frame records | `6075ddec9ed780bce6db1ef98f0972eda60f3871d07f4d632a11d085b1c51ae2` |
| Compiled source identity manifest | `07198071c31fd636e8ef05de272d66923573ffd7e6cc82d0bc5880c8e7a5fe35` |

The exact build was `cargo test -p renderlab --lib --release --offline --locked -j2 --no-run --message-format=json`, with `CARGO_TARGET_DIR=D:/Coding/renderlab-work/m4-stream-followup/target` and an empty `RUSTC_WRAPPER`. Local tracked Rust, WGSL and Cargo input mtimes were advanced before building. The candidate binary was copied out before execution. The exact test filter was `tests::shadow_softness_tests::gpu_directional_visibility_disk_screen --exact --ignored --test-threads=1 --nocapture`. The second invocation adds `RENDERLAB_SOFTNESS_COST=1`. Both require `RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK`. `commands.json` records output paths and the empty implicit-layer directory.

`py -3 .refs/m6-softness/analyze.py` rechecks existing evidence using CPU only. The rejected patch passes `git apply --check` against the restored base. Rebuilding it requires explicitly applying that patch in an authorized checkout and acquiring the GPU/target slot. Preserve this run's evidence before using any original one-shot runner. Two CPU analysis attempts failed while serializing NumPy booleans; converting the result to a Python boolean fixed the analyzer without changing GPU inputs or gates. A PowerShell UTC-format inspection also failed because this host lacks `Get-Date -AsUTC`. There were no compile or native-run failures.

The last owned heavy process exited at `2026-11-04T03:40:53.616765+00:00`. GPU and warm-target ownership were released after CPU-only gate analysis, before packaging, with no owned process remaining. No heavy command ran after that process.

The smallest next hypothesis is analytic disk-area integration of a locally identified straight blocker edge, tested on this same fixture with an explicit unsupported case for multiple/curved boundaries. It could avoid quadrature bands without increasing every receiver's ray count. Conservative discovery of all relevant blockers, especially thin or overlapping blockers, remains unresolved; a few agreeing rays cannot certify that absence. This is a proposal for a bounded experiment, not an implemented replacement, general solution or permission to hide difficult cases. Raising sample counts or adding an unmeasured temporal system was not attempted. M6 and the matched Unreal fidelity/performance gate remain open.

Root verified all recorded artifact hashes, preserved the complete evidence outside the worktree, and inspected the full 4x static sheet. The candidate remains rejected; no production source is imported.

## Root follow-up decision

The straight-edge analytic proposal above is not selected for production: general blocker discovery is unresolved, and solving only the authored half-plane would not address arbitrary quarry geometry. The next bounded preflight tests spatially distributed low-ray-count visibility with edge-aware reconstruction. It must use only information available to the renderer, preserve the original gates, and test thin contacts and receiver discontinuities rather than smoothing away the failures. CPU numerical rejection comes before another Rust build or GPU run.

[AMD's shadow-denoiser documentation](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/denoiser/) describes jittered low-ray-count visibility, depth/normal guidance, temporal reprojection and edge-avoiding spatial filters. This motivates the experiment; it is not evidence that a spatial-only filter can meet Renderlab's first-frame and motion requirements. No external implementation or new dependency is imported, and no temporal system is authorized by this preflight.
