# P-GLOSSY: cheaper glossy reflections without a visible loss

Unit P-GLOSSY, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-11, branch `t3/p-glossy`. Briefs: `D:/Coding/renderlab-work/p-glossy/brief.md`, restated by `brief-2.md`. Labels: **M** measured here, **C** read from code, **I** inference.

Hardware: RTX 4070 SUPER (NVIDIA 617.14), Windows 11, shared PC (Ara integration and N-VSM units leasing the same GPU one at a time). Showcase: Ara's player through Ara's bridge (`tools/showcase/showcase.py`, Ara worktree `ede758f7a`, perf profile), 1920x1080, map sky, the last 60 frames' per-pass medians from `RENDERLAB_PASS_LOG`. Matched scene: `examples/reflection_matched.rs --matched --stage --metal-balls`, 1280x720, release build, against its 16,384-spp path-traced reference.

## Result so far

| Step | Verdict |
|---|---|
| Shared glossy strata: 4 traced strata a pixel, shared inside the shading group | in progress (below) |

## 1. Where the non-RT glossy time goes (M)

Showcase non-RT, still, branch at `60ada122` (main `68e02ddc` merged). `RENDERLAB_GLOSSY_SCREEN` (src/program.rs, measurement scaffolding) edits the composed reflection block per process.

| Screen | Shading, ms | Difference |
|---|---:|---:|
| base: 16 strata at roughness 0.33 and above | 31.35 | |
| hit shadow rays off (`noshadow`, cost only) | 23.11 | −8.2: the any-hit sun walks |
| hit shading off, sky at every hit (`noshade`, cost only) | 19.60 | −11.8: hit shading, including its shadow walks |
| 4 traced strata, footprint of 16, not shared (`traced=4,noshare`) | 10.90 | −20.5 |
| 4 traced strata, shared (3x3 version) | 11.23 | −20.1 |

So roughly 17 of the glossy milliseconds are closest-hit walks and reconstruction, 8 are the hits' sun walks and 3.5 the hits' own shading (I, from the differences; not additive by construction). The matched scene has 17 traced instances over 3 meshes.

**Open:** this base is about 10 ms above P-SKY's 21.07 ms on the same scene; RT is within 0.4 ms of P-SKY. A pure-main binary is timed against the branch in the next lease.

## 2. Shared glossy strata (method)

Each glossy receiver traces 4 of its footprint's strata. The cone (texture footprint and hit regularization) stays the 16-stratum one, so nothing passes by blurring. Every sample goes to workgroup memory: the hit point (or a miss's direction), the radiance the hit's own program shaded, and the density its lobe drew it with. After one barrier, each receiver adds the samples of neighbours on the same surface inside its 8x4 shading group (4x4 on mixed-program lists). It reconnects to each neighbour's hit point from its own origin, weights the sample by its own lobe density over the sample's (pairwise, at most 2, with the squared-distance Jacobian), and normalizes. The rules:

- The reflected image stays the receiver's own, since each sample is placed by its hit point, not by the neighbour's ray.
- A rare bright hit spreads over neighbours with its energy kept, rather than being clamped.
- Neighbours across a normal or depth edge, mirrors, below-plane directions and reconnections with a distance ratio outside [0.5, 2] are not reused.

Hits are shaded only by their own program; sharing changes which receiver averages a shaded value, never how a hit is shaded (M8-HIT-CONTRACT unchanged).

Entries: every lane of a shading group now reaches the exchange in uniform control flow (`primary_shade_pixel(id, size, live, lane, width)`), since naga rejects a barrier after a non-uniform return. Glass reflections (`tp_reflect`) keep every stratum and share nothing.

Patent note (habit only, policy suspended): reservoir-free; resampled reuse across pixels is the family of US 11,315,310.

### 2.1 Matched scene, both paths (M)

Columns per crop: RMS against the reference at the cut (flight 0), moving (flight 47) and held (frozen pose, frame 47 against reference frame 16); white specks held; held flicker (mean frame step over frames 41-47); moving flicker (high-passed frame step over flight frames 25-32).

Non-RT:

| Crop | base 16 | shared 4 (3x3) | unshared 4 |
|---|---|---|---|
| turntable | 6.87 / 3.73 / 4.44, 2, 0.024, 1.167 | 6.89 / 3.78 / 4.45, 2, 0.027, 1.239 | 7.52 / 3.82 / 4.53, 2, 0.043, 1.440 |
| aluminium | 11.42 / 5.78 / 7.09, 0, 0.039, 1.195 | 11.41 / 5.81 / 7.14, 0, 0.040, 1.238 | 12.41 / 5.95 / 7.17, 0, 0.059, 1.485 |
| copper | 1.64 / 11.03 / 7.25, 1, 0.057, 2.132 | 1.60 / 10.90 / 7.24, 1, 0.058, 2.179 | 2.24 / 11.14 / 7.49, 1, 0.086, 2.600 |
| gold | 11.06 / 18.49 / 6.68, 0, 0.060, 1.830 | 11.07 / 18.57 / 6.74, 0, 0.059, 1.795 | 11.72 / 18.65 / 6.80, 0, 0.083, 2.117 |

RT gives the same picture (`runs/c2/score-rt.json`).

**Seen in the frames:** held and moving, shared 4 strata is indistinguishable from base. On the cut frame the 3x3 version stamps a rare bright hit (the copper ball, the turntable beside it) as a visible 3x3 square where base shows single-pixel specks (`sheets/cut-alu.png`). The crop scores did not catch this. The current version picks 6 neighbours a pixel per frame from a 5x5 window instead, so a bright hit scatters like noise; screened next.
