# RT1d: current-frame shadow visibility filter

Branch `t3/rt1d-shadow-filter`, based on main `29d2391`.
**Do not switch the RT default:** the quarry loses to HRT on false shadows and
lit-darkened pixels at both poses. The RT default remains raster plus Exact.
The filter also retains the +4-stop miss in all 12 S2 point-source phases,
introduces point-source leaks/false shadows, and loses the strict-solid measure
in all six S3 phases and both S1 pose-1 configurations. All 12 S2 disk-source
phases pass their recorded measures. Visibility flicker passes every S2/S3 phase.
The single crop's improvement does not override these losses.

All speed values below describe **non-default experimental settings**. Raster
and 256-ray Exact remain selectable and remain the RT-device default. Since
there is no default change, no historical gate's implicit selection changes;
the new quality/cost gates explicitly select Traced/Interactive, and reference
generation explicitly selects Exact. No promotion or 2x claim is made.

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

## Every retained measure against HRT

Each cell counts passing configurations/phases: **previous fast setup → filtered**. Ties pass the unchanged no-worse floor. The CSV/JSON artifacts retain every paired value, population count, sum and maximum. No missing measure is counted as a pass.

| Measure | S2 (24 phases) | S3 (6 phases) | S1 (4 configurations) |
|---|---:|---:|---:|
| partial_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| lit_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| occluded_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| narrow_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| faint_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| silhouette_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| wide_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| wide6_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| contact_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| solid_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| contour_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| contact_lost_sum | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| contact_lost_max | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| leaks_sum | 24/24 → 12/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| leaks_max | 24/24 → 12/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| solid_flips_sum | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| solid_flips_max | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| solid_error_gt_002_sum | 24/24 → 12/24 | 6/6 → 0/6 | 4/4 → 2/4 |
| solid_error_gt_002_max | 24/24 → 12/24 | 6/6 → 0/6 | 4/4 → 2/4 |
| missing_covered_sum | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| missing_covered_max | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| false_shadows_sum | 24/24 → 12/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| false_shadows_max | 24/24 → 12/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| missed_shadows_sum | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| missed_shadows_max | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| linear_hdr_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| display_+0_codes_rms | 24/24 → 24/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| display_+4_codes_rms | 0/24 → 12/24 | 6/6 → 6/6 | 4/4 → 4/4 |
| visibility_flicker_mean | 24/24 → 24/24 | 6/6 → 6/6 | — |
| visibility_flicker_max | 24/24 → 24/24 | 6/6 → 6/6 | — |

## All S2/S3 phases

Each value is **previous / filtered / HRT1**. Display RMS is in 8-bit codes. Flicker is mean visibility residual change, using the original screen-coordinate, film-relative metric; it is not a final-HDR flicker certificate.

| Case / phase | Partial RMS | +0 display RMS | +4 display RMS | Visibility flicker |
|---|---:|---:|---:|---:|
| S2 camera-0.25px-r0 / hold | 0.271445 / 0.189391 / 0.377261 | 6.799355 / 6.628993 / 8.792108 | 17.067548 / 17.059946 / 15.284476 | 0.000970 / 0.000376 / 0.076278 |
| S2 camera-0.25px-r0 / out | 0.273299 / 0.192284 / 0.378899 | 6.573729 / 6.473611 / 8.170027 | 16.573076 / 16.774754 / 14.281831 | 0.029989 / 0.015347 / 0.073703 |
| S2 camera-0.25px-r0 / return | 0.272201 / 0.194538 / 0.373656 | 6.372727 / 6.356426 / 7.994609 | 16.071155 / 16.474726 / 13.965439 | 0.028957 / 0.014913 / 0.067321 |
| S2 camera-0.25px-r0.04 / hold | 0.074709 / 0.048753 / 0.149742 | 5.729304 / 5.296984 / 7.435625 | 14.721532 / 13.153794 / 13.665040 | 0.000362 / 0.000148 / 0.059657 |
| S2 camera-0.25px-r0.04 / out | 0.074881 / 0.047578 / 0.144318 | 5.585226 / 5.195568 / 7.110558 | 14.377052 / 12.998069 / 13.522232 | 0.013213 / 0.005889 / 0.059435 |
| S2 camera-0.25px-r0.04 / return | 0.075124 / 0.047614 / 0.143782 | 5.504547 / 5.141447 / 7.147869 | 14.156851 / 12.879858 / 13.742022 | 0.012741 / 0.005704 / 0.057379 |
| S2 camera-0.5px-r0 / hold | 0.274478 / 0.191861 / 0.370748 | 6.630220 / 6.439658 / 8.796565 | 16.760670 / 16.724706 / 15.242260 | 0.001200 / 0.000515 / 0.074714 |
| S2 camera-0.5px-r0 / out | 0.273213 / 0.193333 / 0.377448 | 6.555042 / 6.456161 / 8.546645 | 16.517855 / 16.722473 / 14.910526 | 0.037068 / 0.019263 / 0.072033 |
| S2 camera-0.5px-r0 / return | 0.272063 / 0.195328 / 0.373011 | 6.351263 / 6.344948 / 8.377456 | 16.022568 / 16.447297 / 14.474786 | 0.035788 / 0.018660 / 0.068194 |
| S2 camera-0.5px-r0.04 / hold | 0.073333 / 0.047492 / 0.147767 | 5.555384 / 5.150832 / 7.475393 | 14.378476 / 12.900411 / 13.648584 | 0.000474 / 0.000225 / 0.059243 |
| S2 camera-0.5px-r0.04 / out | 0.075079 / 0.047650 / 0.143633 | 5.587109 / 5.189878 / 7.304705 | 14.372408 / 12.967822 / 13.745360 | 0.016309 / 0.007793 / 0.058982 |
| S2 camera-0.5px-r0.04 / return | 0.075016 / 0.047798 / 0.143625 | 5.493108 / 5.131150 / 7.326859 | 14.130478 / 12.848958 / 13.830126 | 0.015707 / 0.007569 / 0.057769 |
| S2 camera-1px-r0 / hold | 0.271570 / 0.196024 / 0.373153 | 6.691448 / 6.526632 / 8.701707 | 16.937956 / 16.932275 / 15.182396 | 0.001312 / 0.000683 / 0.071253 |
| S2 camera-1px-r0 / out | 0.273026 / 0.195485 / 0.376604 | 6.460665 / 6.382721 / 8.434913 | 16.344098 / 16.574581 / 14.631687 | 0.040354 / 0.023472 / 0.071049 |
| S2 camera-1px-r0 / return | 0.271435 / 0.197755 / 0.372779 | 6.292605 / 6.292661 / 8.173696 | 15.917939 / 16.324714 / 14.066482 | 0.038611 / 0.022972 / 0.065930 |
| S2 camera-1px-r0.04 / hold | 0.075561 / 0.049315 / 0.149912 | 5.775493 / 5.355072 / 7.541764 | 14.848481 / 13.315675 / 13.811416 | 0.000588 / 0.000326 / 0.058410 |
| S2 camera-1px-r0.04 / out | 0.075172 / 0.048052 / 0.144475 | 5.569899 / 5.184836 / 7.296716 | 14.361932 / 12.982731 / 13.630753 | 0.019083 / 0.010582 / 0.060335 |
| S2 camera-1px-r0.04 / return | 0.075391 / 0.048282 / 0.144569 | 5.499867 / 5.139559 / 7.312996 | 14.152182 / 12.870543 / 13.803187 | 0.018253 / 0.010237 / 0.058253 |
| S2 shadow-step-r0 / hold | 0.271821 / 0.194255 / 0.367331 | 6.235123 / 6.242387 / 8.488701 | 15.579837 / 16.025511 / 14.451436 | 0.000297 / 0.000145 / 0.062029 |
| S2 shadow-step-r0 / shadow-out | 0.273075 / 0.194150 / 0.372720 | 6.232880 / 6.292368 / 8.613875 | 15.600543 / 16.211850 / 14.649103 | 0.009761 / 0.004308 / 0.063717 |
| S2 shadow-step-r0 / shadow-return | 0.273075 / 0.194150 / 0.369469 | 6.232880 / 6.292368 / 8.553248 | 15.600543 / 16.211850 / 14.587549 | 0.009757 / 0.004313 / 0.063069 |
| S2 shadow-step-r0.04 / hold | 0.073883 / 0.049332 / 0.137602 | 5.518826 / 5.230846 / 7.435571 | 14.018696 / 12.937005 / 13.769081 | 0.000125 / 0.000049 / 0.053559 |
| S2 shadow-step-r0.04 / shadow-out | 0.074235 / 0.049255 / 0.137306 | 5.533840 / 5.229553 / 7.420175 | 14.077559 / 12.953186 / 13.717891 | 0.003976 / 0.001499 / 0.054174 |
| S2 shadow-step-r0.04 / shadow-return | 0.074235 / 0.049255 / 0.136968 | 5.533840 / 5.229553 / 7.466535 | 14.077559 / 12.953186 / 13.863624 | 0.003976 / 0.001499 / 0.054018 |
| S3 sun-r0.04 / hold-fast | 0.077448 / 0.059616 / 0.254630 | 5.762235 / 4.812970 / 13.640169 | 13.989372 / 12.013059 / 24.288857 | 0.000000 / 0.000000 / 0.124875 |
| S3 sun-r0.04 / hold-slow | 0.078006 / 0.059900 / 0.253876 | 5.587990 / 4.772083 / 13.448271 | 13.641695 / 11.979130 / 24.118333 | 0.000000 / 0.000000 / 0.123311 |
| S3 sun-r0.04 / jump-recovery | 0.076084 / 0.058727 / 0.250671 | 5.808255 / 5.166918 / 13.501805 | 14.204706 / 12.648942 / 23.935954 | 0.001058 / 0.000906 / 0.127352 |
| S3 sun-r0.04 / rotate-0.25 | 0.078030 / 0.059883 / 0.254967 | 5.567585 / 4.747527 / 13.455071 | 13.671750 / 11.917225 / 24.124745 | 0.002525 / 0.000751 / 0.123382 |
| S3 sun-r0.04 / rotate-1 | 0.077901 / 0.059709 / 0.253865 | 5.705409 / 4.813783 / 13.521944 | 13.851984 / 12.047803 / 24.123926 | 0.005041 / 0.001501 / 0.123724 |
| S3 sun-r0.04 / static | 0.077495 / 0.059755 / 0.255519 | 5.546477 / 4.734819 / 13.426890 | 13.676308 / 11.883338 / 24.103285 | 0.000000 / 0.000000 / 0.123648 |

## S1 static values

| Configuration | Partial RMS (previous / filtered / HRT) | +0 display RMS | +4 display RMS | Strict-solid defects |
|---|---:|---:|---:|---:|
| p0-e0 | 0.087840 / 0.037650 / 0.165304 | 2.084255 / 1.862747 / 2.735305 | 5.471364 / 4.883024 / 6.741099 | 0 / 3514 / 4174 |
| p0-e1 | 0.087840 / 0.037650 / 0.161410 | 1.836986 / 1.618055 / 12.336992 | 3.599625 / 3.179781 / 64.231604 | 0 / 3514 / 4293 |
| p1-e0 | 0.086842 / 0.039707 / 0.177903 | 1.855495 / 1.690298 / 2.474870 | 4.854999 / 4.438876 / 6.010041 | 0 / 2701 / 2610 |
| p1-e1 | 0.086842 / 0.039707 / 0.175632 | 1.631093 / 1.465653 / 12.566350 | 3.180769 / 2.881995 / 65.672037 | 0 / 2701 / 2617 |

S1's environment rows retain the previously documented Unreal black-background
versus Renderlab visible-environment mismatch; their radiance comparison is not
an isolated BRDF gate. Strict-solid defects are `abs(visibility-film)>0.002` on
the registered lit/umbra population; the threshold is unchanged. For example,
S3 static has 2,800 filtered defects versus HRT's 1,157 (previous fast: zero).

These are the registered visibility-flicker comparisons. Retained HRT per-frame
scores do not provide a final-HDR flicker series, and many native frame buffers
were deleted before the retention correction. No missing final-HDR flicker,
S1 motion, quarry motion, transparency or full-frame film gate is claimed to
pass. New Unreal runs were unnecessary to reject this candidate.

## Quiet cost and memory

Optimized release binary on Ryzen 7 7800X3D / RTX 4070 SUPER 12 GB, NVIDIA
617.14, Vulkan, 1920x1080 full-detail textured quarry. Direct lighting and the
prepared environment match the M1/RT1c harness; GI and temporal reconstruction
are off. The disk source uses Interactive's eight rays. Each fresh process
uploads once, warms 32 frames per visit, then runs eight visits at both poses.
Fresh processes keep the two memory inventories independent. No image readback
or diagnostic shading runs inside the timed blocks.

Times are **p50 / p95 milliseconds of individual rendered frames**, retaining
all 16 visits and tails, with no clean-visit selection. There are 128 static,
768 camera, 256 sun and 768 caster samples per configuration. HRT has 112 frames
per phase from the retained M6-close CSVs: the original warm block and exclusion
of ProfileGPU-trigger frames and their successors are unchanged.

| Phase | Current filter off | Filter on | Recorded HRT1 |
|---|---:|---:|---:|
| Static | .906 / 2.230 | 1.438 / 3.255 | 1.748 / 1.999 |
| Camera | .902 / 1.473 | 1.425 / 2.238 | 1.764 / 1.920 |
| Sun | .907 / 1.559 | 1.446 / 3.256 | 1.742 / 1.906 |
| Caster | .906 / 1.481 | 1.425 / 2.139 | 1.778 / 2.059 |

Shading p50 rises from .379-.380 to .895-.900 ms. The observed whole-span median
increment is .523-.539 ms; it is a separate-process comparison, not an isolated
GPU filter-pass timestamp. The gather is inside the shading pass. Traced primary
p50 remains .499-.502 off / .503-.510 on. The HRT/filtered p50 ratio is
1.20-1.25, short of 2x, and filtered p95 loses in every phase.
The Renderlab boundary is rendered span through tone mapping; Unreal's is D3D12
whole-frame GPUTime. This is a bounded direct-light subset, not a whole-engine
speed comparison. Point-source fixture timings remain diagnostic, not a new
quiet quarry point-source certificate.

| Configuration | Allocated MiB | Reserved MiB | WDDM warm/process peak MiB | Warm WDDM samples |
|---|---:|---:|---:|---:|
| Filter off | 2,509.807 | 2,641.247 | 2,716.289 | 3 |
| Filter on | 2,509.807 | 2,641.247 | 2,716.289 | 4 |
| Recorded HRT1 | not supplied | not supplied | 5,407-5,410 local usage | retained DXGI/CSV |

The filter adds no texture/buffer allocations. WDDM is per-process Dedicated
Usage, not total desktop VRAM; three/four warm samples are observations, not
instantaneous peak bounds. The updated inventory names the existing disabled
indirect-cache header and environment-lobe table from their allocation sites;
it does not alter either subsystem.

The successful shared-lock/quiet window was **13:15:51.590-13:16:18.395 UTC on
2026-11-08** (26.797 s, including compiler-free settling). The two GPU child
processes and inventory work took 17.978 s. The first two attempts declined
preflight because compilers were active; a third stopped when new Cargo
processes appeared. Their timings are discarded. `quiet-cost-4.json` records
the successful guard samples; the quiet interval was released immediately and
TR1 acquired the next lease. The runtime compiler guard remained enabled.
All four guard samples have zero foreign GPU/game/build processes; the 10 Hz
resource samples have zero rustc and minimum available host memory 8,059.5 MiB.

## Verification, evidence and retention

The release library test binary ran **inside the RT1d n8 lease**: **143 passed,
0 failed, 142 ignored**, 4.12 s. Formatting is clean; Python syntax and the final
diff whitespace check pass. The test validates both composed shading sources;
quarry outputs repeat bit-for-bit eight times per pose/configuration and retain
identical primary hit hashes. S2/S3 capture every frame without readback-dependent
production timing, then capture their diagnostic visibility separately.

The Vulkan layer emits the pre-existing `VUID-StandaloneSpirv-None-10684`
function-layout diagnostic, also present in RT1c's retained witness log. This
unit does not claim an allowance-free Vulkan validation run. All GPU tests exit
0 and HDR readback finiteness checks pass. No viewer, window, focus change,
desktop capture or game-time GPU work was used.

Implementation/compilation interval: first filter source at 12:34 through the
final test executable at 12:46 UTC, about **12 minutes**, a filesystem-time
proxy, not active editing time. Three no-run release compilations took
**175 / 98 / 102 s** (375 s total). Verification is separate: S2 / S3 / S1
successful leases took **375.015 / 184.589 / 58.023 s**; quarry **18.658 s**;
final quiet **26.797 s**. CPU S2 scoring took **450.279 s** (summed per-case
scorer durations), S3 **83.955 s**, S1 **13.263 s**, quarry
**19.082 s**. These durations include harness/scoring overhead and overlap;
they are not frame times. Resource/lease queue waits are not implementation time.

Binary SHA-256: `59e25022ae9b2c8b72c392124c3edca424d0b17aea4c7829dce902a309fc5c40`.
Production Rust/WGSL is checkpoint `e639fc8`; later edits are measurement Python
and this report. No production source changed after the measured executable.

Evidence root: `D:/Coding/renderlab-work/rt1d/` (ignored/external, not committed):

- `witness-score.json`, `witness-comparison.csv`, `witness-plus4.png`;
- `quarry/gate.json`, `determinism.json`, `new-false-witnesses.json`, inspected
  `p0-filtered.png` and `p0-new-false.png`, and both poses' PNGs;
- `motion/S{2,3}/gate.{json,csv}`, complete `per-frame.json`, per-case scores,
  reference NPZ archives and inspected crop PNGs;
- `S1/gate.json`, four reference archives and display PNGs;
- `cost/summary.json`, `cost/{previous,filtered}/rt/` raw timestamp rows,
  allocator reports/CSVs, WDDM samples, manifests and stdout;
- `runs/`, `gpu-windows.json` and the input SHA-256 identities.

Final artifact verification pins **1,669 reference archives** and twelve
canonical quarry/HRT inputs. Archive structure and representative endpoint
arrays pass; the pilot's filtered HDR crop is byte-identical to S2's frame-0
batch capture. `final-verification.json` records those hashes and source identity.

Cleanup removed **62 intermediate dumps / 1,592,524,800 bytes**, after scoring
and retaining the reference NPZs, summaries and PNGs. `cleanup.json` records
each removed path/size/hash. Automatic review rejected a combined computed-path
cleanup command; Cargo's scoped clean and explicit literal file paths succeeded.
`cargo clean --target-dir D:/Coding/renderlab-work/rt1d/target` removed **1,066
files / 535.1 MiB**; the owned target is absent. No owned process or lease remains.

`scripts/rt1d-shadow-filter.py` provides CPU preparation/scoring and the GPU
batch drivers. Run quality/test binaries only under `scripts/n8-run.py` with
`N8_OWNER=rt1d-shadow-filter`, `N8_OUT=D:/Coding/renderlab-work/rt1d` and the
experimental-ray acknowledgement supplied by that guard. The final quiet
wrapper `scripts/rt1d-quiet.py` imports the same n8 guard, holds its lock while
existing compilers drain, and retains `--fast-check --no-build-contention`.
It neither starts Cargo nor bypasses the game/memory/foreign-renderer checks.

All retained HRT captures and film references are read-only. Regenerated S2/S3
references and new captures go to `D:/Coding/renderlab-work/rt1d` and are compacted
into retained reference archives after scoring.

`docs/milestones-5-6-decisions.tsv` is intentionally untouched. No GI-cache,
temporal or standard-program source is changed.
