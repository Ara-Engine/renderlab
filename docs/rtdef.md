# RTDEF: the RT device default with TR1

2026-11-08, branch `t3/rtdef`, base main `976a952`, merging TR1b `fa6f8c1`.
This unit applies Islam's overnight promotion rule: visibly improve the quarry
and S2 without a visible regression in the inspected images. The retained HRT
and film comparisons are recorded; they are not tonight's promotion gate.

## Comparison

- Legacy RT default: raster primaries, 256-direction Exact, no reconstruction.
- Control: traced primaries, eight-direction Interactive, reconstruction off.
- Candidate: traced primaries, Interactive, native-resolution TR1b defaults.

The headless 1920×1080 tests use the RTX 4070 SUPER, Vulkan, the textured
13,034,903-triangle quarry at both registered poses, and S2's four camera/shadow
tracks at point and disk light radii. No new reference or Unreal campaign runs.
The existing `bench-contact-fold-score.py::radiance` supplies linear HDR and
Reinhard/sRGB display RMS at +0 and +4 stops. Quarry motion is inspected; only
its registered still poses have film scores in this unit. S2 uses every retained
per-frame film and its recorded HRT1 radiance scores.

Evidence is external/ignored at `D:/Coding/renderlab-work/rtdef/`; original
references on E: and D: remain read-only. The lease wrapper is
`scripts/rtdef.py`; `scripts/rtdef-score.py` performs CPU scoring only.
