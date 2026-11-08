# RT1d: current-frame shadow visibility filter

Branch `t3/rt1d-shadow-filter`, based on main `29d2391`.
**Do not switch the RT default:** the quarry loses to HRT on false shadows and
lit-darkened pixels at both poses. The RT default remains raster plus Exact.

## Implementation and shader contract

`Renderer::set_interactive_shadow_filter(true)` selects one fixed, non-separable
5x5 Gaussian cross-bilateral gather, with offsets -2 through +2 on both axes.
The normal factor is `clamp(dot(n0,n1),0,1)^32`; the plane factor is
`exp(-abs(dot(p1-p0,n0))/(0.002*max(view_depth,0.1)+0.0005))`.
Weights read only current geometry and instance/material identity. A placement
has one material/program; taps must match that placement. Misses, stale uncovered
tiles, pixels outside the scene rectangle, and out-of-image taps are excluded.
Weights are normalized, including the centre tap. No output colour is read.

The disk-source path gathers the existing eight-ray visibility values. The
point-source path evaluates the existing hard visibility at the same current
receiver taps, because its production path does not write a soft count image.
This point-source cost must be measured separately from the disk-source quarry.
The raw visibility texture, hit records, receiver representation and user program
are unchanged. Programs receive a filtered scalar from `hard_shadow`, so custom
materials and arbitrary shading remain programmable. Multiple user visibility
queries repeat the gather; no shading-model assumption is cached.

The implementation is in `shaders/shadow_visibility_filter.wgsl` and
`src/shadow_visibility_filter.rs`. The single marked composition call and spare
frame-uniform word are the only production `primary_ray*` wiring changes. The
setting defaults off. Raster, Exact and the non-RT path bypass it.

## Claim note

This follows the engineering design-around constraints in
[P0 sections 3.D and 6/W6](m7-p0-claims.md):

| Claim reviewed by P0 | Element absent here |
|---|---|
| US 12,423,782 claim 1 | Filter tap locations never jitter or rotate. |
| US 11,113,792 claims 1/11/15 | Fixed size and complete fixed tap set; no size from ray results or historical sample counts, and no temporal filter. |
| US 11,688,042 claims 1/9/15 | One non-separable 2D gather, one radius, one iteration; no directional sub-kernels or separable passes. |
| US 10,600,167 claim 1 | No back-projection, prior-frame surface check, or auxiliary data from a temporal stage. |
| US 12,437,371 claim 1; US 2026/0162231 | No lighting gradients, history comparison or confidence derived from either. |

Current geometric depth sets an edge weight, never the tap radius or number.
Ray-traced visibility is the filtered signal, never a guide. There is no
luminance, variance, hit-distance, reprojection, history-age or temporal input.
This records the claim elements avoided under P0's reading, not a legal opinion
or a broader patent-clearance certificate. No patented fallback was selected.

## Evidence and default decision

The GPU disk-source witness reduced +4-stop
display RMS from 14.083180 to 12.959767, below retained HRT1's 13.665473.
This is one native 512x288 crop of a 1920x1080 frame, not motion acceptance.

Current-source quarry, pose 0 / pose 1; lower is better:

| Measure | Previous traced + Interactive | Filtered | Recorded HRT1 |
|---|---:|---:|---:|
| Partial RMS | .207157 / .208537 | .175532 / .176570 | .370873 / .371257 |
| False shadows | 27 / 26 | **1,594 / 1,589** | 743 / 758 |
| Lit darkened | 0 / 2 | **912 / 906** | 724 / 736 |
| Missed shadows | 9,245 / 9,166 | 4,348 / 4,276 | 23,931 / 23,789 |
| Umbra leaks | 0 / 0 | 605 / 589 | 2,718 / 2,616 |
| Solid flips | 0 / 1 | 0 / 1 | 3,306 / 3,215 |
| Lost umbra edges | 0 / 0 | 0 / 0 | 2,565 / 2,461 |
| Lost contacts | 0 / 0 | 0 / 0 | 2,565 / 2,461 |
| Missing receivers | 0 / 0 | 0 / 0 | 20 / 26 |

Eight complete-HDR repeats per configuration/pose are bit-identical. Hit hashes
are identical with the filter off/on. No quality threshold or film population
was changed. This miss is against HRT, not the non-blocking VSM comparison.

Full S2/S3, S1 and cost tables follow after the ongoing verification batch.
All retained HRT captures and film references are read-only. Regenerated S2/S3
references and new captures go to `D:/Coding/renderlab-work/rt1d` and are compacted
into retained reference archives after scoring.

`docs/milestones-5-6-decisions.tsv` is intentionally untouched. No GI-cache,
temporal or standard-program source is changed.
