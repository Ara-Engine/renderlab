# Combined quarry ordinary-frame cost

The short integrated screen identifies GPU mesh work as the limiting stage. No rendering optimization is retained. The only source changes add `--ordinary-cost` to the existing headless `m6-shadows` example and its tool module. Renderer, hardware receiver, environment reconstruction, geometry, materials and quality settings remain unchanged.

This is a contended attribution result, not a quality-preserving speedup or an Unreal comparison. Two short interleaved blocks were sufficient to distinguish the large costs. No third block or long campaign ran.

## Workload and method

The source base is `d4792dac0daca2d7cfdc2d5b012f796b21cec9a0`. The host is Windows/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14. Compilation uses rustc 1.96.0, `ac68faa20`, the ordinary optimized release profile, two jobs, offline and locked dependencies, and no compiler wrapper. There are no extra optimizer flags.

Every measured frame submits all 13,034,903 quarry triangles in twelve draws at 1920x1080, with the original 4K texture chains, 16x anisotropy, 4x MSAA, the actual RLE2 field and optional hardware directional shadows. Lighting is the accepted combined exposure-only preset at -14 stops. The two registered cameras differ by one centimetre in X. Static uses the first pose. Motion alternates those two nearby poses. There is no asset recook, resolution change, virtual simplification or timing exemption.

The tool models a control, a block, accepted frame reports, asynchronous GPU samples and observed completions. It uses the renderer's existing three-flight admission mechanism. It retries only when admission reports busy and drains events after each attempt. It never waits for each accepted frame to complete. Each 24-frame block drains once at its end. Six untimed warm frames precede each control. Shader reloads and settings changes occur outside the blocks. Captures happen only after every measured block, after restoring the original candidate shader and shadow settings.

There are two repetitions, four controls and static plus motion observations. Repetition zero visits candidate, sample-rate no-query, sample-rate no-query/no-map, then pixel-rate no-query. Repetition one reverses that order. The screen retains 384 measured frames, 48 warm frames, one startup frame and one final candidate capture frame. The sixteen measured block intervals total 3.066 seconds. No frame or block was discarded.

Controls deliberately change the image. The no-query control removes only the `hard_shadow` call from `fs_lit`, while hardware composition keeps the sample-qualified world input used by the lighting calculation. The no-map control additionally disables the map branch, retaining constant ambient and direct lighting. The pixel control disables shadow requests and uses the ordinary pixel-frequency pipeline with the no-query shader. Neither sample-rate control removes per-sample receiver interpolation. These controls are attribution tools, not quality-equivalent implementations.

## Measured GPU and throughput proxy

Each cell gives block zero and block one. GPU cells are per-block medians of 24 timestamp samples. The final two columns divide each finite block's start-through-final-drain wall time by 24. They include pipeline fill/drain, timestamp handling and tool overhead, so they are a headless throughput proxy rather than native presentation throughput.

| Control | Static GPU mesh, ms | Motion GPU mesh, ms | Static drained ms/frame | Motion drained ms/frame |
|---|---:|---:|---:|---:|
| Unchanged candidate | 10.820 / 11.477 | 9.877 / 11.217 | 12.170 / 11.536 | 10.147 / 11.437 |
| Sample rate, no visibility query | 7.542 / 6.439 | 7.289 / 6.431 | 9.433 / 6.981 | 7.304 / 6.901 |
| Sample rate, no query or map | 5.703 / 5.659 | 5.398 / 4.983 | 5.827 / 5.848 | 5.811 / 5.438 |
| Pixel rate, no visibility query | 7.326 / 7.063 | 6.867 / 6.904 | 7.318 / 7.208 | 7.195 / 7.197 |

The candidate's mesh samples range from 9.155 to 21.004 ms. The first static no-query block retains a 24.775 ms outlier. All sample ranges and individual rows remain in `summary.json`, `summary.md` and the raw block JSON. `profile.svg` plots the two block medians separately.

The separate HDR tone-mapping pass has candidate block medians of 0.0189 to 0.0205 ms. Directly measured render span is retained separately from the individual passes. There is no presentation, surface acquisition or scanout measurement here.

Removing the visibility-query path at the same sample frequency changes the whole mesh median by 2.588 to 5.038 ms in corresponding blocks. Removing RLE2 reconstruction from that same-frequency no-query control changes it by another 0.780 to 1.891 ms. These are marginal whole-shader observations. They include compiler scheduling and register-pressure changes, not isolated traversal or texture-load cycle counts. They cannot be added into an exact independent stage budget.

The residual 4.983 to 5.703 ms includes unchanged full geometry, rasterization, material textures, direct and constant-environment shading, and attachment resolve. It is not a geometry-only measurement. The sample-rate versus pixel-rate no-query gap changes sign across blocks. This screen therefore does not quantify a reliable standalone shading-frequency penalty. The naive enabled/disabled hardware delta must still not be called pure ray-query cost.

## CPU, waits, uploads and churn

Candidate accepted render-call medians are 0.157 to 0.215 ms. Caller camera and draw-list preparation medians are 0.0006 to 0.0007 ms. The renderer's disjoint recorded CPU fields give these ranges of candidate block medians.

| CPU field | Median range, ms |
|---|---:|
| Input preparation | 0.00010 to 0.00020 |
| Visibility | 0.00060 to 0.00200 |
| Packing | 0 |
| Shadow preparation, including AS reuse checks | 0.01030 to 0.01105 |
| Shadow upload | 0.00255 to 0.01340 |
| Other upload | 0.00030 to 0.01785 |
| Encode | 0.07875 to 0.11190 |
| Queue submission | 0.04385 to 0.05090 |
| Poll inside accepted call | 0.01580 to 0.01995 |

These fields do not include every caller bookkeeping instruction. The accepted-call wall timer covers that unitemized remainder. Independent medians are not an additive CPU frame budget.

Admission medians are 9.655 to 11.274 ms for the candidate, including its accepted call and the preceding busy retries. Each candidate block reaches three flights and records 194,093 to 240,543 rejected attempts. Rejected-call timers account for 3.297 to 4.193 ms per frame. Explicit polling accounts for 2.464 to 3.051 ms per frame, and yields account for 2.874 to 3.202 ms per frame, including the final drain. Remaining loop and JSON overhead is also in wall time. The retry loop consumes host CPU while the GPU is busy. Those retry costs must not be presented as scene preparation cost or as the production viewer's sleep policy.

Observed submission-to-completion medians are 30.193 to 35.235 ms because three submissions remain in flight. These are host collection observations with queueing, not GPU execution durations or input-to-display latency. Similarity between drained time and mesh time, the small accepted CPU service cost, and the diagnostic shader changes support the GPU-mesh limiter. They do not establish isolated device throughput under contention.

All steady blocks have zero new geometry, texture, environment, material, normal, transform, instance or visible-ID uploads. The candidate's static block writes only 8,064 shadow bytes, 336 bytes per frame. Motion additionally writes 1,472 camera bytes and 276 lighting-uniform bytes over the 24-frame block. Light records do not change. Static records 24 writes and motion 70. There are no steady BLAS or TLAS builds, target creations, shadow-resource creations, pipeline installations or persistent-object creations. There is one transient caller attachment view per frame. Owned counters are not driver allocation or residency claims.

## Startup and AS accounting

The first screen reaches prepared-ready in 8.442 seconds. Its log separately records 932.051 ms of asset I/O, 659.079 ms of decode and 691.463 ms of resource-upload wall time. Startup includes hashing the 1.55 GB input, device/pipeline creation, field loading and other preparation. The separate validation process reaches ready in 4.077 seconds. This variation is retained, not interpreted as a startup optimization.

Twelve BLAS builds are already complete before the first rendered frame. Their GPU duration is not separately timestamped and cannot be extracted from the upload wall interval. The first frame builds one TLAS. It records 0.0609 ms CPU shadow preparation, 0.0045 ms AS command encoding, 6.5441 ms accepted CPU render time, 18.378 ms render span and 21.150 ms submit-to-observed completion. The startup frame is excluded from ordinary blocks. GPU render timestamps begin after AS construction, so neither its render span nor the steady mesh number prices GPU AS building.

The first completed scene has 911,788,608 bytes of named AS allocator attribution and 102,260,784 owned field bytes. These are approximately 912 MB and 102 MB in decimal units. AS admission estimates remain separate from those allocator observations and from physical residency.

## Contention and limits

Before compilation and measurement, GPU observations included 50% utilization with 3,269 MiB used, then 41% with 3,273 MiB used. The foreign wallpaper application and unrelated Cargo checks were observed and left running. During the complete screen process, sampled GPU utilization spans 4% to 100%, memory spans 3,273 to 6,678 MiB, graphics clocks span 465 to 2,820 MHz and temperature spans 36 to 50 C. These samples include startup and transitions and are not isolated per-block counters. Raw timestamped samples are in `screen.json`.

The two interleaved blocks establish coarse attribution. Their variation does not support a precise query-cost percentage, a shading-frequency speed ratio, an idle-machine result, temporal-quality acceptance or the required Unreal-equivalent-quality and 2x-performance gate. The tool's busy retry loop and timestamp/JSON instrumentation remain additional limits. No rerun waited for the machine to become idle or selected a faster-looking result.

## Retained decision and verification

No production fix was attempted. CPU preparation and uploads are too small to justify an optimization in this screen. Query work and RLE2 shading both have material marginal costs, and a substantial residual remains. Removing queries, dropping sample-qualified receivers, changing reconstruction or reducing geometry would change the required image.

The smallest next behavior-preserving hypothesis is to count samples whose direct-light BRDF is exactly zero and only then test skipping their unused visibility queries. This screen does not count that population, so no gain is predicted and no such change is implemented. Moving the environment body to another shading frequency would require a separate design and correctness decision.

The original candidate is restored before capture. Its complete 1920x1080 image was inspected, including silhouettes, material detail and shadow contacts. Screen and validation RGBA16F captures are byte-identical to the accepted combined exposure-adjusted capture. Their SHA-256 is `4bff86db54061289c2efd329cca02c662caa499aa5c2d50db6632f126b14b276`. All values are finite, nonnegative and below the saturation bound. The validation capture positively activates Khronos and reports zero errors and missing layers. Timing explicitly disables validation and records zero active validation layers. Existing unused shadow vertex-attribute warnings remain in the validation log.

Release compilation, example execution, source/content audits, `cargo fmt --all -- --check` and `git diff --check` pass. No global test campaign or extra Clippy build ran. The audit verifies all 108 recorded current source inputs. It also verifies the 106 unchanged integration inputs after allowing LF/CRLF encoding differences. The first raw source comparison rejected seven newline-only differences. That rejected analysis assertion is retained in `rejected-audit.json`; no timing sample was rejected.

The necessary release build took 21.695 seconds. The screen process took 17.507 seconds and the targeted validation capture took 4.607 seconds. Those verification processes total 22.114 seconds, or 43.809 seconds including compilation. Editing, source inspection, image inspection, analysis and documentation wall time were not separately instrumented. These command totals are not total task duration. There was no rendering-fix implementation time because no fix was retained.

## Reproduction and identities

Local evidence is `.refs/combined-cost/` in worktree `C:/Users/snowy/.t3/worktrees/renderlab/t3-54e3982c`. It includes raw blocks, all warm/startup samples, controls, logs, command/environment records, binary/source/content hashes, the original preserved executable, the analysis script and the chart. Generated measurements and private images remain ignored.

Root copied and hash-verified the complete evidence to `D:/Coding/renderlab-work/combined-cost/`. Root independently checked all 108 source identities, all 384 measured frames, the candidate medians, workload counts and both exact restored HDR images. The imported example also passes focused Clippy with warnings denied. No production renderer or shader changed, so no broader GPU suite was repeated for this tooling checkpoint.

Before target reuse, the accepted executable and the existing shared-target executable were copied to `preserved/`. Both have SHA-256 `5285afc79239d803ac2a5bfdca3b3beb4e12f2fbe8557cac89428a37dba0a1fa`. The new tool executable has SHA-256 `69bf088f49cd8c4890ea7caa677fdff7373a5358a0e72981f952ac8c51204438`.

The changed example hashes are `6ed5392e9a667e0ee8c138dd171091eca5189e3162c5523dbd67a0f141b107d7` for `examples/m6-shadows.rs` and `e4e7e0b0fb325c935041e90d8e656f65e2e6958c8e33ea3fa1996307e4869186` for `examples/m6/ordinary_cost.rs`. The immutable content hashes are recorded in `content-identities.json`.

| Input | SHA-256 |
|---|---|
| Full quarry RLA | `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3` |
| RLE2 field | `3ccd61b142d26b7c04571242eb620729366847674522ef990b06661bd54c1764` |
| Limits | `6e417dc134313669a653383ed2b38a605119c9c0349ecf82999e1610ff4dd121` |
| Registered cameras | `8a9c055339a625121360dbe93b512fcbb75afd583594a403ffe2d7d2c3606dd1` |
| Exposure-only lighting | `064d75ab6d0aa626aa498de33867a01ecefa37b36efe1383d23a3d5d2cf2ef25` |

The recorded build command is below. All local Rust, WGSL and manifest input mtimes were advanced before building, forcing this checkout's asset and renderer crates to rebuild without cleaning the shared target.

```powershell
$env:CARGO_TARGET_DIR='D:/Coding/renderlab-work/m4-stream-followup/target'
$env:RUSTC_WRAPPER=''
git ls-files '*.rs' '*.wgsl' 'Cargo.toml' '*/Cargo.toml' 'Cargo.lock' | ForEach-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc = [DateTime]::UtcNow }
cargo build --release --example m6-shadows --offline --locked -j2
```

The exact process commands and environment are in `screen.json` and `validation.json`. The screen command is below, using the preserved newly built executable.

```powershell
.refs/combined-cost/m6-shadows.exe --hardware-shadows --ack-experimental-wgpu-risk --hardware-max-estimated-bytes 8589934592 --samples 4 --motion static --asset D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla --limits D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/limits.json --cameras D:/Coding/renderlab-work/m6-hardware-msaa/quarry-cameras.json --lighting D:/Coding/projects/desktop/renderlab/.refs/m5-m6-combined/quarry-hdr-lighting.json --environment D:/Coding/renderlab-work/m5-independent-production/sun128.rle2 --ordinary-cost --frames 24 --output .refs/combined-cost/screen
```

`py -3 .refs/combined-cost/analyze.py` reruns the CPU-only evidence audit and summary generation. `run.py` records the fixed screen followed by one validation capture, samples contention and refuses to overwrite retained evidence. A future authorized GPU owner can run `py -3 .refs/combined-cost/run.py .refs/combined-cost-repeat` with a new destination. The example is a narrow registered-quarry tool, not a general benchmarking framework.

GPU and shared-target ownership were released at `2026-11-04T02:19:27.757430+00:00`, immediately after the final native verification process. No owned process remained. Analysis and packaging used CPU-only operations. No main-checkout source edit, child agent, PR or push occurred.
