# jit1 (lease fixed-jit1b-1, 2026-11-14 11:55:52-12:01:01 UTC, 304 s)
bin-P203 copy (ara.exe 409464b9, ara-player.exe 9e89b992), shader edge1/disk-p203-nz.wgsl (be45434c),
outpost.scene f3583f2f, showcase.scene bc174591. Non-RT 1920x1080.
- Held Outpost frame 299: base vs 0.05 >2 6,680 px, >8 2,889 px, all on shadow edges (tarp, robot feet on the
  tarp, one far edge); none on open ground; isolated-pixel count whole frame 10,107 -> 9,989. 0.05 vs 0.1: max 6, 16 px >2.
- Moving rows 20-59, tarp crop (544,864 +-160): isolated pixels (|px - 3x3 median| > 8) median 104 -> 474, max 127 -> 673
  (0.05 and 0.1 identical). Visible stipple on every moving frame (sheets/moving-21.png, moving-40.png).
- Cut +1/+5/+20: no visible change (soft near-level shadows); crop counts equal.
- Lookup ms (median frames 100-299): Outpost 1.772 / 1.789 / 1.787 (base/0.05/0.1); Showcase 0.148 / 0.146.
- Identity row o-def unusable: it kept only frames 40,59 (others kept 43 frames), so readback timing differs.
- Glint rows failed at start: "starting ara mcp: Access is denied (os error 5)"; nothing rendered.
