# S5: lighting reflection hits from a cache (design note, 2026-11-13)

Status: draft for the root and U-RTVIS. Author: the Mac GPU thread (hit shading). No code.

## Why
- **Where the cost is:** about 2.1 of Showcase's 2.49 ms of reflections is fixed per glossy pixel,
  and the hit's shading is about 62% of it.
- **What the other steps leave:** S2 still shades one hit a pixel. S4a (`metal=0`) reuses about 20%
  of hits from the screen, and the screen trace replaces traversal only for on-screen hits.
- **S5:** replace the remaining hits' `surface` + `shade` with a cached answer where that answer is
  within half a display code after the receiver's lobe.

## 1. Where a cached answer is good enough
**What a hit sends toward the receiver:** its diffuse (Lambertian albedo x irradiance) plus its
specular toward the receiver's direction.
- **The cache today** (`gi_cache` cells, read by `RL_CACHE_HITS`): emission plus Lambertian
  reflection only. A metal hit has no diffuse, so it comes back nearly black. That is the flattened
  copper ball.

**What the receiver's lobe does to the hit's specular:** at 0.35-0.5 (GGX alpha 0.12-0.25) the lobe
averages the hit over a cone a few degrees to about 15 degrees wide. That moves where a highlight
lands and how sharp it looks. It does not change how much energy the highlight carries.
- **Dropping a dielectric hit's specular** is therefore an amplitude error, the same kind as the
  footprint stills (a missing highlight darkens the reflection; the lobe does not average it back).
- **Its size:** F x E_lamp at the hit against albedo x E for the diffuse. That is about 0.04 / 0.5
  (8%) at normal incidence, and up to tens of percent at grazing hits, which is what floor rays
  hitting the sides of props are.
- **Replacing it with a prefiltered environment term** is right for the sky and the bounce
  (smooth, low-frequency), and wrong for lamp highlights (concentrated energy).

**Safe region**, for a cache that holds diffuse plus a low-order directional specular:

| Receiver | Hit | Cached? |
|---|---|---|
| 0.35-0.5 (the band: 59% of Showcase's reflected light) | dielectric, any roughness | yes, if the cache holds the hit's lamp specular to low order (below); diffuse-only fails at grazing hits under lamps |
| 0.35-0.5 | metal, rough >= 0.5 | yes, with a low-order directional specular; never diffuse-only |
| 0.35-0.5 | metal, rough < 0.5 (copper 0.2, chrome) | no: a near-mirror hit's reflection is high-frequency, so shade it (or S4a reuse) |
| < 0.35, and mirrors | any | no: the lobe shows the cell's spatial blur and the hit's own highlight; shade it (or reuse) |

## 2. What the cache must hold to go further
| Option | Holds | Memory | Cost | Covers |
|---|---|---|---|---|
| a. L1 SH of outgoing radiance per cell (4 coefficients x RGB, f16) | diffuse + low-order directional | +24 B a cell over today's 16 (about 6 MB at 256k cells) | the cell update projects each light's specular onto 4 terms (about 2x today's update); lookup is one dot product | rough dielectrics under lamps, rough metals |
| b. L2 SH per cell (9 terms) | sharper directional | +54 B a cell (about 14 MB) | about 4x the update | metals to about 0.35; still blocky in space |
| c. Hit-space specular history (hash of placement, primitive cell and direction bin) | exact-surface, directional | about 8 B x direction bins x entries (tens of MB for 8 bins) | a hash insert per shaded hit; no per-cell lighting | anything shaded once and seen again from a similar direction: an S4a history in hit space |
| d. Shade only where the table above says no | nothing new | 0 | a class test per hit (receiver roughness, hit metalness and roughness) | the rest |

**What I would build:** d with a, a classifier plus an L1 cell. d is required anyway. a closes the
dielectric-lamp-specular amplitude gap that diffuse-only leaves, at a 2x cell-update cost the cache
already budgets. b and c can come later if the band's metals need them. Spatial blockiness is
bounded by the existing rule (the cone must be at least the cell's edge), which the band's wide
cones meet at prop distances (footprint about 2 alpha t, 0.25-1.5 m at 1-3 m).

## 3. Measurement on main with one measurement-only switch
- **Switch:** `RENDERLAB_RL_CACHE_HITS_TUNE=metal=M,hit_rough=R,mask=K`, applied on top of
  `RL_CACHE_HITS=0.35`.
  - Hits whose material metallic factor is >= M, or whose roughness factor is < R, shade as today.
  - `mask=1/2/3` are the share forms used for S4a (cached hits 1 / every hit 1 / every hit 0).
- **Rows:** Showcase and Glint's goal, at `metal=0.1,hit_rough=0` (diffuse-only, dielectric hits)
  and at `metal=1.1` (today's RL_CACHE_HITS), exposure locked.
- **Report:** the share; the picture against inline (still crops of the stage floor's reflections
  of props, the copper ball and the pedestals); and against the root's path-traced reference.
- **What it tells:** the cached-share ceiling for d, and how large the missing lamp specular is,
  which sizes a. About a day with the M5 rows, the RTX rows following.

## 4. Order and expected worth (M5 Showcase: glossy reflections 2.30 ms at cap 4, inline)
1. **S2 + the screen trace with S4a's verifier** (`rl_reuse_at`): on-screen hits skip traversal
   and shading together. This is the cheapest large step, because it reuses what exists.
2. **S5 (d + a) for the rest:** off-screen hits and those S4a rejects, in the safe region.
   - Expected about 0.3-0.5 ms on the M5 once compacted: about 40% of hits cached, times about
     1.4 ms of hit shading, less the lookup.
   - Inline, divergence keeps it near zero, as S4a showed. S5 needs the compacted pass on the port.
3. **c only if the band's metals still dominate** after 1 and 2.

Unreal's 0.248 ms gets its speed from a surface cache that is diffuse and blocky. Its dark,
grainy copper is the result of that, and it is the picture we will not ship. The classifier in d
is what keeps our copper shaded.
