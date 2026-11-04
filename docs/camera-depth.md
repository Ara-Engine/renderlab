# Main-camera depth

`Camera::matrix(width, height)` returns the actual main raster matrix with finite reversed depth. Near maps to 1 and far maps to 0 after division by W. This intentionally changes the pre-release depth convention. Its signature, right-handed view, forward -Z direction, authored near/far planes, FOV, and X/Y/W calculation are preserved.

For positive near `n`, far `f`, and view-space Z, the projection uses:

```text
C = n / (f - n)
D = n * f / (f - n)
clip_z = C * view_z + D
clip_w = -view_z
```

The constructor computes C and D in f64, then stores their f32 values. It rejects nonfinite coefficients and coefficients that round to zero with `Error::InvalidInput`. The f64 product avoids premature f32 overflow. There is no new minimum near plane or arbitrary far cap. The existing finite-matrix validation remains, and render submission checks the matrix at the actual target aspect.

The main diagnostic, normal, lit, tangent, and hardware-shadowed pipelines share strict `Greater` and clear depth 0. Exact equal-depth fragments still keep the first draw and ascending instance-slot winner. Geometry exactly at the cleared far depth still fails the strict comparison. No production primitive-ID feature is required.

The separate light-space shadow pipeline retains `Less`, clear 1, forward matrices, comparison sampling, biases, and finite hardware-ray algebra. Camera-view distances for cascade selection and virtual projected error remain positive distances along -Z. No shader, geometry, material, transform, culling face, sampling setting, or quality threshold changed.

## Caller migration

Regular rendering and virtual rendering both upload `Camera::matrix` and construct their conservative visibility volume from that matrix. The homogeneous inequalities remain `-W <= X,Y <= W` and `0 <= Z <= W`. Z=0 bounds the far side and Z=W bounds the near side. The culler needed no production change.

Unprojection must use Z=1 for the near point and Z=0 for the far point. The photometry and shadow-slice reference tests now do this. Independent tolerances are unchanged. The existing culling fixture's depth-coverage check now compares against clear 0. `examples/m6-shadows.rs` continues to export the actual matrix and additionally labels it `finite_reversed_z_near_1_far_0`.

`scripts/quarry-shadow-probe.py` also consumes captured matrices. It orders unprojected endpoints by camera-view distance so that archived forward-depth captures still work. Eight CPU checks cover its two conventions at the four frozen pixels. No forward projection compatibility API or runtime knob was added. Internal depth readback is test-only; the public HDR capture API contains color rather than camera depth.

## Frozen quarry result

Verified on 2026-11-04 from base `8e5c0a398f11b9c8f991c391d5c26c4b86cb3dcc`. Evidence originated in `.refs/camera-depth/` in worktree `C:/Users/snowy/.t3/worktrees/renderlab/t3-760f1878` and is preserved at `D:/Coding/renderlab-work/camera-depth/`. `identities.json` and `manifest.json` identify source, binaries, inputs, and outputs. Root integrated the patch, inspected full original/moved/4x images and pixel crops, and checked all 107 tested-source identities. Independent Opus review accepts this bounded checkpoint with no code blocker. Post-review source changes are comment-only corrections; executable behavior is unchanged.

The original private asset was read in place. Its SHA-256 is `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`. The workload retains 12 placements, 13,034,903 triangles, 6,633,832 vertices, original 4K mip chains, 1920x1080 output, and 16x anisotropy. The original camera retains near 0.005 and far 100. Lighting is the unchanged quarry preset with constant environment. Hardware shadows retain the original finite cascades and biases.

The private depth-accepted receiver probe observes these actual GPU results at 1x. All four isolated faces still cover their respective selected pixel.

| Pixel | Earlier back triangle | Closer front triangle | Back depth bits | Front depth bits | Actual winner |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1140,726 | 17881 | 540043 | 975797414 | 975797516 | 540043 |
| 1222,752 | 568793 | 728455 | 976325962 | 976326474 | 728455 |

The original forward-depth pairs tied at bits 1065342370 and 1065341853. Both closer front faces now win. Their shading-normal dot light values are 0.902427532 and 0.821762944, agreeing with the earlier isolated front-face observations. The immediate right-hand neighbors retain their original winners and exact HDR. Contact at 1167,695 remains exact and still darkens when shadows are enabled.

The complete probe HDR and depth buffers equal the same-run ordinary candidate buffers byte for byte. The camera preflight reproduces the archived forward matrix, verifies unchanged view and X/Y/W bits, and independently checks the new depth coefficients and row bits. The final production executable, with the hook removed, reproduces the candidate's complete HDR exactly. Its preserved forward executable also reproduces the retained original image exactly.

Only 341 of 2,073,600 HDR pixels change at the original 1x pose. The coverage mask is identical. The maximum changed HDR channel difference is 0.487121582. A registered one-centimetre camera translation changes 376 pixels versus its matched forward-depth capture. The original 4x pose changes 1,452 pixels versus its retained 4x capture. Both retain the image footprint. All captures are finite and their resolved HDR values are below f16 saturation. This does not establish unsaturated individual MSAA samples.

Full before/after images, frozen-pixel crops, the change mask, and crops of the largest original-pose differences were inspected. Rock silhouettes, cavities, rubble, sand, and contact shadows remain intact. The two selected 1x dots are corrected. Other changed pixels are quantified and visually inspected, without claiming an independent source-triangle oracle for every changed pixel. Remaining dark texture or geometric detail is not automatically a defect. The selected four 4x pixels were already clear controls and remain byte-identical.

Review these local artifacts:

- `frozen-01/before.png`, `after.png`, `crops.png`, `changed-pixels.png`, and `analysis.json`.
- `frozen-01/receivers.json`, `isolated-faces.json`, `probe-equivalence.json`, and `camera-contract.json`.
- `followup-01/before1x/pose-001.png` and `after1x/pose-001.png` for the registered translation.
- `followup-01/after4x/pose-000.png` and `4x-original-crops.png` for 4x. Its before image is `D:/Coding/renderlab-work/m6-hardware-msaa/quarry4x/pose-000.png`.
- `followup-01/analysis.json` for all matched HDR and shadow-disabled control values.

## Verification and reproduction

All 61 ordinary library CPU tests pass. These include endpoint mapping, monotonic depth, close-surface separation, X/Y/W bit equality, large and tiny representable ranges, explicit coefficient failures, conservative general-finite and mirrored/nonuniform culling, selection projection, and forward light-volume algebra. Review found that the initial CPU log lacked a source-identity record. Root closed that gap by rerunning the preserved final-source `gates-02/test.exe` (SHA-256 `6c7de9fb1087ed09a91f670ca4fde317cc423de34f97b7c5f80c99982f1c7c63`) without ignored tests: 61 passed, 43 ignored, 0 failed in 0.61 seconds, with no compilation or GPU run. The log is preserved as `root-final-cpu.log` alongside the evidence.

Nine focused GPU gates pass:

1. `tests::gpu_main_depth_order_and_reload` verifies two surfaces 0.1 mm apart at 20 m, independent expected depth, draw-order independence, equal-depth first-wins, clear 0, and reload recovery.
2. `tests::gpu_batched_culling_oracle` compares complete culled, unculled, batched, and reference images/depth, including mirrored and general finite transforms.
3. `tests::virtual_pages::gpu_virtual_ranges_reference_and_settled` verifies regular/virtual equality, normal and lit output, 1x/4x, and reload recovery. Its existing 300 tiny settled frames remain part of that test.
4. `tests::lighting_tests::gpu_lit_photometry_1x_4x` preserves independent photometry tolerances and tests the migrated near-point oracle.
5. `tests::hardware_shadow_tests::gpu_hardware_shadow_orientation_blocker_motion_reload_and_release` preserves finite coverage, positive occlusion, and reload behavior.
6. `tests::hardware_shadow_tests::gpu_hardware_shadow_normal_map_uses_geometric_origin` passes at 1x/4x.
7. `tests::hardware_shadow_tests::gpu_hardware_shadow_msaa_sample_receivers` passes its independent analytical footprints and rejects centre-query substitution.
8. `tests::shadow_tests::gpu_shadow_offscreen_local_and_reload` preserves raster directional/local shadows and reload.
9. `tests::normal_maps::gpu_normal_maps_integration_shadow_plane_is_geometric` passes the raster normal-map gate at 1x/4x.

All 14 completed GPU validation summaries report zero errors, no missing layer, and a positive active Khronos layer count. Existing unused shadow vertex-attribute performance warnings remain. The first gate-run command used the wrong `virtual_gpu` module name. It ran zero tests, failed the runner's validation-summary assertion, and was not counted as a pass. The corrected `virtual_pages` invocation passed. There were no renderer-test failures or GPU validation errors. Read-only command discovery also encountered unsupported PowerShell wildcard and date syntax, corrected without modifying content.

The first culling run still used the old `< 1.0` depth-coverage assertion, which is vacuous when depth clears to zero. The final `gates-02` run uses `> 0.0` and is the counted culling pass. The earlier result is preserved, not relied on for coverage.

Use the granted target only while owning it. These are the ordinary checks:

```powershell
$env:CARGO_TARGET_DIR = 'D:/Coding/renderlab-work/m4-stream-followup/target'
$env:RUSTC_WRAPPER = ''
cargo test --offline --locked -j 2 -p renderlab --lib -- --test-threads=2
cargo clippy --offline --locked -j 2 -p renderlab --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

`frozen-01/commands.json`, `gates-01/commands.json`, `gates-02/commands.json`, and `followup-01/commands.json` preserve exact executable invocations, environments are in the runner sources, and binaries are copied beside the logs. GPU commands use active Khronos validation, an empty implicit-layer directory, and explicit experimental ray-query acknowledgment. All execution was headless. No window, focus, input, desktop capture, foreign-app change, private-asset copy, or recook occurred.

For a new receiver reproduction, apply the private `receiver-hook.patch` in an owned checkout, place the preserved `probe.rs` at `.refs/quarry-receiver/probe.rs`, then run `py -3 -B .refs/camera-depth/run.py <fresh-output-name>`. The source hook and primitive-ID instrumentation are absent from the shipping patch. `analyze.py` and `analyze_followup.py` recheck existing files without GPU work. Preserve overwritten executables and touch local Rust, WGSL, and Cargo input mtimes after switching the shared target between checkouts, as the runner does.

The host was RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Observed GPU utilization before the frozen probe was 22 percent with 3308 MiB in use. Contention was uncontrolled. This is correctness verification, not a performance comparison. The initial CPU command reports 32.17 seconds compilation and 0.53 seconds test execution. Later timed compiles total 88.39 seconds; Clippy takes 7.34 seconds. Completed GPU processes total 75.97 seconds, including the repeat culling test. The empty-filter mistake adds 0.07 seconds. `timing.json` retains each command. Implementation was not independently stopwatched; reading, editing, and verification overlapped. No exclusive coding duration is inferred from these timings.

GPU and warm-target ownership were released at `2026-11-04T03:10:00.289635Z`, before packaging, with no owned process remaining. No commit, PR, push, or child agent was created.

This closes the two frozen main-depth defects and the bounded migration checks after root integration and independent review. It does not close M5, M6, broad motion stability, other quarry speckles, or matched Unreal fidelity and 2x performance acceptance.
