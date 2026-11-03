# Quarry image quality checks

The tools in this unit prepare a bounded comparison on the actual twelve-scan quarry. Root acceptance results are in [scan-results.md](scan-results.md). The full 4K/4x/16x preset passed the registered spatial and short-motion gates. Existing images show soft close textures, aliased single-sample silhouettes, and authored scan patch boundaries. The initial CPU tooling left rendering unchanged; root subsequently added optional anisotropic filtering and measured it without changing the gates.

## Evidence and scope

The inspected images include the full-detail close 1x view, the full-detail grazing 4x view, the sixteen streamed 4x poses, and native grazing crops from all four full/stream 1x/4x combinations. Their paths are under `captures/quarry-scan/` and `captures/quarry-preparation/`. The source inspection compared corresponding texel rectangles in `xbnhajp_8K_BaseColor.jpg` and its derived `basecolor.jpg`. The 8K crop retains small aggregate detail that the 2K crop loses. This establishes a texture-resolution limitation. It does not measure a new rendered candidate.

The grazing crops show smoother geometry boundaries with 4x MSAA. Soft interior textures persist. A contact sheet cannot establish temporal stability, and the old sixteen-pose replay contains cuts. Streamed moving images also carry residency debt. Compare settled full-detail output first, then compare streamed output at the same texture content, camera, and sample count. A visible authored patch perimeter also appears in full geometry. Treating that perimeter as a streaming crack would misdiagnose the content.

The material remains opaque, double-sided, unlit base color. The final local demo uses all twelve 4K chains; the original 2K cache remains available. Source normal, roughness, AO, and other maps remain unused. No lighting bake or new illumination is introduced. Full material lighting belongs to M5.

## Texture memory and format admission

For one square RGBA8 texture of side `s`, a complete mip chain uses `4 * sum((s >> level)^2)` bytes. The twelve-texture totals below exclude geometry, encoded metadata, staging, targets, and driver overhead.

| Texture selection | Complete mip bytes | Approximate resident texels |
| --- | ---: | ---: |
| Twelve 2K textures | 268,435,440 | 256 MiB |
| Twelve 4K textures | 1,073,741,808 | 1 GiB |
| Twelve 8K textures | 4,294,967,280 | 4 GiB |
| One 8K texture and eleven canonical 2K textures | 603,979,760 | 576 MiB |

The full 4K RLA is planned at 1,548,588,300 bytes. Its virtual package is 1,084,303,132 bytes. The RLV resource section contains an RLA header, materials, mip headers, and texels. Twelve 8K textures would require a 4,294,970,136-byte resource section. That exceeds its structural u32 length field. The tool rejects this combination before writing. An unlimited policy cannot remove that format constraint.

The 4K candidate raises only finite per-run texture, bootstrap, prepared, and renderer admission where required. Geometry, selection, and read/upload budgets retain the canonical settings. No default policy changes. One-at-a-time JPEG decode and strip-based linear filtering avoid retaining twelve decoded sources. Actual process RAM and driver memory still require observation. The recorded 13 GiB available RAM, 45 GB disk space, and 12 GB GPU are scheduling context, not capacity certification.

## Preserve content while changing resolution

`scripts/scan-quality-variants.py` reads the canonical manifests and schema-1 boundaries without loading vertices. Its default invocation prints a plan and writes nothing. `--create` requires a fresh D-drive directory outside the canonical scan directory.

With no `--size`, creation writes only camera, policy, and command records. With `--size`, the tool copies geometry, placements, and material bindings byte-for-byte. It verifies intake source texture hashes, writes lossless RGBA8 mip chains, and copies unchanged virtual geometry metadata. Virtual pages become hard links in the new cache. They are never recooked or rewritten. Source pages must remain immutable. Each variant records whole-file hashes and hashes of the unchanged RLA prefix and RLV geometry tail. The virtual manifest has a private variant key. It is not published as a cooker cache lookup key.

The 2K and 4K controls use the same original 8K JPEG decode and opaque linear-light 2x2 area reduction. Every level rounds back to RGBA8. The 2K chain is the shared lower portion of the 4K chain. This separates resolution from Blender's original scaling and JPEG re-encoding. Comparing canonical 2K directly with regenerated 4K changes both preparation and resolution. Label that comparison accordingly.

Run from the integrated repository after the current measurement window releases. Use new output names for every attempt.

```powershell
$scan = 'D:/Coding/projects/desktop/renderlab/measurements/quarry-scan'
$quality = 'D:/Coding/renderlab-quality-aa-20261103'
py -3 -B scripts/scan-quality-variants.py $scan $quality --create
```

Prepare texture variants only when needed. The selective 8K command promotes one foreground ground scan and retains the other eleven canonical textures.

```powershell
py -3 -B scripts/scan-quality-variants.py $scan D:/Coding/renderlab-quality-2k-20261103 --size 2048 --create
py -3 -B scripts/scan-quality-variants.py $scan D:/Coding/renderlab-quality-4k-20261103 --size 4096 --create
py -3 -B scripts/scan-quality-variants.py $scan D:/Coding/renderlab-quality-8k-ground-20261103 --size 8192 --ids xbnhajp --create
```

The materialized variants still need the existing Rust decoders to validate their complete semantics. A focused CPU command for a chosen variant is:

```powershell
target/release/renderlab-cook.exe m4-inspect D:/Coding/renderlab-quality-4k-20261103/cache D:/Coding/renderlab-quality-4k-20261103/virtual.manifest --limits D:/Coding/renderlab-quality-4k-20261103/limits.json
```

## Run the smallest spatial pilot first

For an initial edge and material result, prepare only the foreground ground scan at regenerated 2K and 4K. Keep the other eleven textures canonical. The 4K candidate uses 335,544,304 texel bytes, about 320 MiB. Run this only after root releases the build/GPU window.

```powershell
$two = 'D:/Coding/renderlab-quality-ground2k-20261103'
$four = 'D:/Coding/renderlab-quality-ground4k-20261103'
py -3 -B scripts/scan-quality-variants.py $scan $two --size 2048 --ids xbnhajp --create
py -3 -B scripts/scan-quality-variants.py $scan $four --size 4096 --ids xbnhajp --create
function Invoke-QuarryPilot([string]$folder, [string]$kind) {
    $pilotPlan = Get-Content "$folder/plan.json" -Raw | ConvertFrom-Json
    $pilotCommand = @($pilotPlan.commands.$kind)
    $pilotCommand[[array]::IndexOf($pilotCommand, '--frames') + 1] = '2'
    $pilotCommand[[array]::IndexOf($pilotCommand, '--capture') + 1] = '1,2'
    $pilotCommand += @('--start', '1')
    py -3 -B scripts/scan-execute.py --gpu --timeout 180 "$folder/evidence/$kind" -- @pilotCommand
    if ($LASTEXITCODE -ne 0) { throw "Quality pilot failed: $kind" }
}
Invoke-QuarryPilot $two 'full-4'
Invoke-QuarryPilot $four 'full-4'
Invoke-QuarryPilot $four 'reference-4'
py -3 -B scripts/scan-quality-compare.py "$four/plan.json" "$two/full-4" "$four/full-4" "$four/reference-4" D:/Coding/renderlab-quality-ground-report-20261103 --goal texture --baseline-plan "$two/plan.json" --frames 1,2
```

These three GPU runs capture six images total. They check the close and grazing views against a full-detail, doubled-dimension 4x reference. The same comparison reports absolute silhouette errors and the texture-resolution gate. Inspect native candidate images, reference reductions, and silhouette masks. This pilot leaves temporal acceptance UNRUN and does not establish a 1x-to-4x AA improvement. Stop after this result unless its evidence justifies another check.

## Expand to the registered motion check when needed

The camera file has twelve frames. Frame 0 is canonical wide pose 0. Frame 1 is canonical close pose 4. Frames 2 through 10 interpolate canonical grazing poses 8 through 11 over eight small movements. Frame 11 is canonical reverse pose 12. Only adjacent grazing frames enter temporal calculations. Cuts never do.

Native images are 1920x1080. The reference preserves projection and full geometry while doubling both dimensions to 3840x2160 and using 4x MSAA. Reference reduction averages linear RGB. This is a finite supersampled reference for this feature set.

Read the generated exact commands, then run each once. `scan-execute.py` supplies existing process identity, validation logs, memory observations, game guard, and deadline behavior. No server, build, window, or benchmark campaign is needed.

```powershell
$qualityPlan = Get-Content "$quality/plan.json" -Raw | ConvertFrom-Json
function Invoke-QuarryQuality([string]$label) {
    $qualityCommand = $qualityPlan.commands.$label
    py -3 -B scripts/scan-execute.py --gpu --timeout 180 "$quality/evidence/$label" -- @qualityCommand
    if ($LASTEXITCODE -ne 0) { throw "Quality run failed: $label" }
}
Invoke-QuarryQuality 'full-1'
Invoke-QuarryQuality 'full-4'
Invoke-QuarryQuality 'reference-4'
```

Compare the 1x baseline and 4x candidate against their identical-content reference:

```powershell
py -3 -B scripts/scan-quality-compare.py "$quality/plan.json" "$quality/full-1" "$quality/full-4" "$quality/reference-4" D:/Coding/renderlab-quality-aa-report-20261103
```

The comparer verifies registered cameras, exact full-asset digests or admitted virtual-package identities, and image extents. For AA acceptance, inputs must have identical texture content. It hashes input images and records the contract before scoring pixels. It saves reference reductions, silhouette masks, and 4x-amplified error maps for the four named static views. `candidate-motion.webp` preserves the nine native grazing images losslessly. Its 100 ms playback interval is illustrative and makes no frame-rate claim.

For texture screening, run only `full-4` on the regenerated 2K control and the 4K candidate, plus `reference-4` on the 4K candidate. Point `$quality` at each directory and reload its plan before invoking the function. Use `--goal texture --baseline-plan PATH/plan.json` with the 4K plan. A selective 8K reference can be supplied with `--reference-plan PATH/plan.json`. Compare it only with controls that differ in that same selected scan. This avoids confounding an 8K foreground change with eleven unrelated 4K changes. Four-kilopixel reference screening does not certify complete 8K source fidelity.

## Declare acceptance before candidate rendering

These initial gates are stored in `plan.json`. They are not results or a perceptual-quality multiplier. A changed contract needs a new plan and an explicit reason before the next comparison.

| Measurement | Initial gate |
| --- | --- |
| Linear RGB MAE in a two-pixel reference silhouette band | At most 0.04 per frame |
| Per-pixel channel-mean absolute error p95 in that band | At most 0.15 per frame |
| Smooth foreground interior linear RGB MAE | At most 0.02 per frame |
| Background/foreground disagreement outside the reference edge band | At most 0.0002 of all pixels per frame |
| Temporal change in reference-relative edge residual | MAE at most 0.04 per adjacent grazing pair |
| Temporal change in reference-relative texture residual | MAE at most 0.025 per adjacent grazing pair |
| Regression against baseline, each spatial region and temporal region | At most baseline MAE * 1.05 + 0.001 |
| AA candidate mean silhouette error | At least 20% reduction, or error at most 0.001 |
| Resolution candidate mean texture error in close and grazing views | At least 10% reduction, or error at most 0.001 |

Texture regions use reference luminance gradients of at least 0.04 outside the silhouette band. Smooth interior errors are reported separately. Reference foreground classification uses the known background RGB8 value `[36,44,56]` with one code-value tolerance. Similar-colored material can confuse this classification. The saved masks require inspection. Internal occlusion edges are not silhouettes against the background and need visual inspection too.

The temporal metric subtracts each same-pose reference before taking adjacent-frame residual differences. It measures reference-relative error variation at fixed screen pixels. It is not motion-compensated surface shimmer or a complete temporal reconstruction metric. The short grazing sequence and full-frame inspection supplement it.

A numeric PASS remains separate from visual acceptance and active clean validation. Inspect native close and grazing images, full silhouettes, patch overlaps, and the short motion sequence. Reject new holes, material assignment changes, new cracks, disruptive texture crawl, or blur that hides the defect. A stable authored perimeter is a content limitation. Residency debt is a streaming limitation. Neither is a new lighting requirement.

## Sampler decision and verification state

The quarry's stored samplers use repeat U/V, linear mag, linear min, and linear mip filtering. `src/material.rs` preserves these choices. Its current default anisotropy is 1. Installed wgpu 30.0.1 requires linear min, mag, and mip filters whenever anisotropy differs from 1. It clamps the requested anisotropy to 16 on supported backends and otherwise uses 1. Blindly enabling 16 would change material sampling policy without comparative evidence.

Disabled mips still use the M3-corrected positive LOD ceiling of 0.25 and nearest mip choice. This preserves independent min/mag behavior on Vulkan. The later matched captures showed that resolution alone missed the improvement gate. Root added `RendererOptions::anisotropy_clamp` (1..=16, default 1) and viewer/replay `--anisotropy`. Only all-linear mipmapped bindings use the requested value; explicit nearest or disabled-mip samplers retain their existing semantics. At fixed 4K/4x, anisotropy 16 reduced the close/grazing texture-reference error by 43.92%, while all twelve spatial and eight temporal comparisons passed. This is a quality option, not a claim that the authored trilinear sampler was invalid. Reproduce the candidate by appending `--anisotropy 16` to the candidate and supersampled-reference commands, using fresh output directories.

Four small CPU tests cover linear-light mip behavior, nested resolution controls, byte-preserving resource/page reuse, truncated input rejection, and end-to-end scorer rejection of a missing region. Run `py -3 -B scripts/scan-quality-tests.py`. Root materialized matched 2K/4K variants, admitted the full and virtual packages through Rust, inspected the supersampled comparisons and twelve-view motion sheet, and exercised the final native viewer. The sampler compatibility CPU test, focused material GPU test, and active-validation captures pass. The selective 8K experiment was not needed or run. The raw results and remaining bounds are recorded in scan-results.md.
