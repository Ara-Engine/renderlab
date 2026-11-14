# pcf1: deterministic filter for fallback receivers (U-FIXED, 2026-11-14, not shipped)

Leases fixed-pcf1a-1 (14:29-14:36 UTC) and fixed-pcf1b-1 (14:40-14:44 UTC). bin-P203 copy (Renderlab 0ebcc116),
shader-only through `RENDERLAB_VD_SHADER=shaders/disk-p203-pcf.wgsl`, `RENDERLAB_VD=VD_PCF=Nu`.
`VD_PCF=2`: the receiver's whole Disk walk at the 2x2 texel centres of the level read, blended bilinearly;
`3`: 3x3 centres, quadratic B-spline weights. Only opaque receivers reading a coarser level than wanted.

Lookup ms (sun_shadow.lookup, median frames 100-299, held, rounds A / B):
| | Outpost | Showcase |
|---|---|---|
| base | 1.496 / 1.979 | 0.148 / 0.179 |
| pcf0 (filter off) | 1.710 / 1.975 | - |
| pcf2 | 5.406 / 5.931 | 0.208 / 0.210 |
| pcf3 | 12.826 / 12.926 | 0.357 / 0.356 |
Outpost frame span 33.7 -> 37.8 ms with pcf2. Cost scales with the walk count: most of Outpost's view falls back.

Picture: pcf0 byte-identical to base at clip frames 40 and 59 (max 1 code at the cut). Moving tarp crop, isolated
pixels (|px - 3x3 median| > 8): base 104, pcf2 44, pcf3 34.5, RT 34.5 (jit1's slide at gate 0.05: 474). The steps
are softened, not removed; RT's edge is straight. Glint goal replays ran on all three (ARA_BIN must name ara.exe).

Verdict: not shipped (+3.9 ms, limit 0.3 ms). Fix coverage instead: sparse-clipmap-note.md.
Evidence: D:/Coding/renderlab-work/unit-fixed/runs/pcf1 (sheets/moving-40-base-pcf2-pcf3-rt.png).
