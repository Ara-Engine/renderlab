# M7-G2c: GI speckles and edge streaks in API1's game scene

Unit G2c, branch `t3/m7-g2c` (main `f159fcd` with G2b's GI default, plus `t3/m7-api1` for its example). API1's game scene ([examples/game_scene.rs](../examples/game_scene.rs)) showed two GI-only defects on both paths, with and without TR1: bright gold speckles over the near ground, and faceted dark streaks on the arch's lintel end and the top of its right pillar. Both are fixed at the cause. Labels: **M** measured here, **I** inference. Overnight-push rules (AGENTS.md, 2026-11-08): lib tests in a lease, inspected images on both paths, one timing run. Gaps are listed under "GI" in [m7-gaps.md](m7-gaps.md).

## 1. Status

| Item | Cause | Fix | Result (M) |
|---|---|---|---|
| Ground speckles | Cell shading evaluated each light as `scatter(l, n)`, the program's radiance toward the cell's normal alone, and stored it as the cell's view-independent outgoing radiance. A glossy or mirror cell facing the sun stored its specular peak and sent it in every direction. The few gather rays that met such cells on the gold and chrome spheres lit single ground cells several times over | A cell reflects as a Lambertian surface with the program's own directional albedo toward its normal, capped at 1 per channel. Its value is the cosine-weighted mean of what the program sends, so the reflected power is kept but no highlight is amplified (section 2) | Ground hot pixels per frame (RT / non-RT, mean over 48 frames): 106 / 72 → **2.9 / 0**. The RT residue is shadow-penumbra pixels, not speckles. No-GI image: 0 |
| Arch streaks | As the camera closes in, the nearest surfaces switch first to a finer cache level. The new cells have sky in their first frame but no bounce until their first gather, which the per-frame gather cap can defer by several frames. The lookup read "not gathered yet" as "no bounce" | Gathers now mark "gathered, no light" (`GI_BOUNCE_NONE`) apart from "not gathered yet" (0). The lookup treats a corner without a gather like an absent corner for bounce only, and fills its share from the next coarser level, as it already did for absent corners (section 3) | Gone on both paths: the edge's 6% dip (139-143 against 148-151 codes) is flat at 148-151 in frame 47 (`analysis/c1-arch-047.png`) |
| Quarry, SV | — | — | Quarry unchanged: every population within 0.01 codes of G2b's final run on RT, TR1x64 pinholes 142 → 141. RESULT-QUARRY |
| Lib suite | — | — | `cargo test -p renderlab --lib` in lease `g2c-c1`: 159 passed, exit 0 |
| Cost | — | — | Equal within noise on the game scene: RT p50 6.17 / 6.08 ms (main) against 6.02 / 5.93 (G2c), non-RT 9.49 / 9.28 against 9.58 / 9.36 (section 4) |

## 2. Ground speckles

**Diagnosis (M, run `a1`, every frame of API1's 48-frame fly path on both paths).** Pixels 8+ codes above their 9×9 median, counted over the ground in front of the camera (x < 620, y > 520, a box the spheres never enter):

| | RT mean / max | Non-RT mean / max |
|---|---:|---:|
| Main (G2b) | 106 / 158 | 72 / 103 |
| G2c cell shading | **2.9 / 13** | **0 / 0** |
| API1's GI-off capture (7 frames) | — | 0 / 0 |

`analysis/a1-ground-047.png` (main | G2c, RT frame 47) and `analysis/a1-hot-008.png` (marked pixels, frame 8) show the speckles gone. The 13 remaining RT pixels lie on the gold sphere's shadow penumbra.

**Why they were there (I, from the BRDF and the scene).** The gold sphere has roughness 0.25 (α = 0.0625), so its GGX peak D = 1/(πα²) ≈ 81. A sphere cell facing the sun evaluated `brdf(n, v = n, l)` near that peak: about 20 times a white Lambertian surface's radiance, gold-tinted. The chrome sphere (roughness 0.02) reaches 10⁴ times in the cells within a fraction of a degree of the sun direction. Physically the highlight leaves those cells back toward the sun. The cache stored it as their radiance in every direction instead. A ground cell's gather traces 64 fixed directions; when one of them met such a cell it added that radiance times 2π/64, about twice the ground's own sun irradiance, so isolated cells lit up. Per-cell rotation of the direction set scattered them into speckles.

**Fix** ([gi_shade_entry.wgsl](../shaders/gi_shade_entry.wgsl)). The cell's value is now

  emission + a/π · (Σ lights reached: intensity · attenuation · max(n·l, 0) + incident sky and bounce irradiance),

with a = min(Σ_k scatter(s, input, d_k, n) · 2π/16, 1) over G2's sixteen fixed directions: the program's directional albedo toward n. G2 already reflected the cache's incident light through this albedo; the lights now use it too.
- **Exact for Lambert**, as before. The G2 gate program and the quarry's standard program change only in their specular share.
- **Energy.** The cosine-weighted mean of the outgoing radiance is the cell's radiosity over π, so the reflected power is unchanged for diffuse surfaces and bounded for every surface.
- **Programmability.** Only the program's own `scatter` and `emission` decide the reflected light, as in G2. What no longer reaches bounce is a program's dependence on the light direction beyond the cosine (wrap lighting, transmission, retro-reflection): a cell reflects lights as a Lambertian surface would. Recorded in the gaps.

**The cap at 1 per channel** is a passive surface's energy bound (directional albedo ≤ 1). It is deterministic and per cell. Bias: it changes only programs whose `scatter` integrates above 1 (not energy-conserving), or a sharp lobe that one of the sixteen fixed directions meets head-on (a normal-mapped mirror whose reflected normal lands on a quadrature direction). For energy-conserving programs it never binds.

**Recorded bias: shiny metals bounce too little.** Sixteen fixed directions cannot integrate a lobe much narrower than their spacing (about 36°). Gold at roughness 0.25 gets a ≈ 0.4-0.5 against a true directional albedo near 0.8; chrome at 0.02 gets about 0. Physically, a mirror's bounce is a caustic, which no view-independent cache represents. The cache now under-lights near shiny metals (energy lost) instead of speckling.

## 3. Arch streaks

**Diagnosis (M, runs `a1`, `b1`).**
- The streaks are a narrow band (about two cells, 6-8 px) on both faces beside the vertical edge where the right pillar and the lintel's end face the camera. They are about 6% darker than the face. They appear at frame 46 and are strong at frame 47; frames 0-45 are clean (`analysis/a1-arch-seq.png`, frames 41-47).
- They are unchanged by the speckle fix (`analysis/a1-arch-047.png`).
- **They are in the bounce term.** Two diagnostic builds of the example: with the pixel's bounce read as zero, the edge is clean in every frame. With bounce alone shown (×5), the edge dips from 169 to 129-134 codes at frame 47 and not at frame 45 (`analysis/b1-arch.png`).
- **Why there and then (I).** At frames 45-47 the descending fly-path camera is closest to that edge, at the edge's height. The cache level comes from the footprint at the camera distance, so the nearest surfaces switch to a finer level first. New cells trace their sky at once (RT) or over four frames (non-RT), and are queued for three gathers. The per-frame gather cap (G2b: 32,768 cells hardware, 8,192 software) defers the rest of a busy queue by a fixed slot rotation. Until a cell's first gather, its bounce word was 0, which the lookup could not tell from a cell that has no bounce. The lookup weighed it fully, so the new level's corners pulled the band's bounce toward zero.

**Fix** ([gi_api.wgsl](../shaders/gi_api.wgsl), [gi_cells.wgsl](../shaders/gi_cells.wgsl), [gi_cache.wgsl](../shaders/gi_cache.wgsl)):
- A gather that finds no light writes `GI_BOUNCE_NONE` (-0.0, which every reader already takes as no bounce). 0 now means only "not gathered yet". New and reclaimed cells start at 0, as before.
- The lookup sums a second weight per level, `gi_lv_bw`: the weight of corners that have been gathered.
- Bounce is filled from the next coarser level over its own weight, exactly as the sky's missing weight already was: corners with sky but no gather count as absent for bounce only. Where the coarser level has no gathered value either, the level's own gathered corners take the rest.
- With bounce off, or once every corner has been gathered, every factor equals G2's, so settled images are unchanged.
- The cost is one extra coarser lookup, only for pixels that read a not-yet-gathered corner.

**Result (M, run `c1`, final code).** Row profiles across the edge in frame 47 (8-bit codes, lintel row y = 100 and pillar row y = 200):
- Before (speckle fix only), RT and non-RT alike: 151 → 143 → 148 → 139 → 148 (lintel), 151 → 140-142 → 148 (pillar).
- After: 151 → 150 → 148 on both rows and both paths, the plain face-to-face step.

`analysis/c1-arch-047.png` (RT before | RT after | non-RT before | non-RT after) shows the band and the mottle beside the edge gone.

The full frames are otherwise alike (`analysis/c1-full-rt-047.png`, `analysis/c1-full-nonrt-024.png`). One visible difference beyond the two defects: on non-RT, main's chrome sphere and blue crate showed a blotchy reflection pattern, which G2c no longer has. Their untraced specular reads the cache's bounce lobe, which (I) held the amplified light of the highlight cells nearby.

## 4. Verification

Final code: example build `fix2`, test binary `c`, leases `g2c-c1` to `g2c-c8`. These are development builds on an RTX 4070 SUPER: functional runs, with contention not classified.

**API1's game scene** (1280×720, 48-frame fly path, TR1 on, both paths, every frame written in `c1`): covered in sections 2 and 3. No new defect was found in the inspected frames (0, 8, 24, 28, 41-47). Both paths look alike.

**Quarry** (`gpu_g2_quarry`, static poses 0 and 1, 1x, TR1x8 and TR1x64; ES1's metric, F0 codes at −14 stops; G2b's final run `k1` as the base, whose cache-off captures this run reproduces exactly):

| RT, TR1x64 | all-covered | lit | occluded | narrow | faint | isolated | multi-surface | hollow | crop | pinholes |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| G2b (`k1`), pose 0 | 7.39 | 6.35 | 12.71 | 10.27 | 10.13 | 9.45 | 13.51 | 13.53 | 6.79 | 142 |
| G2c, pose 0 | 7.39 | 6.34 | 12.71 | 10.27 | 10.13 | 9.45 | 13.51 | 13.53 | 6.79 | 141 |
| G2b (`k1`), pose 1 | 7.39 | 6.35 | 12.62 | 10.40 | 10.02 | 9.07 | 13.63 | 13.40 | 6.77 | 126 |
| G2c, pose 1 | 7.39 | 6.35 | 12.62 | 10.39 | 10.02 | 9.07 | 13.63 | 13.39 | 6.77 | 126 |

The 1x captures are equal to G2b's too (pose 0 all-covered 9.53, pinholes 1,459 in both). This is as expected: the quarry's standard program on rough rock is close to Lambert, and its static captures are settled. RESULT-QUARRY-NONRT

RESULT-SV

**Timing** (`c2`, the game scene, last frame only written, main and G2c interleaved twice per path in one lease; p50 of the per-frame GPU span over frames 8-47):

| | Main, visit 1 / 2 | G2c, visit 1 / 2 |
|---|---:|---:|
| RT | 6.17 / 6.08 ms | 6.02 / 5.93 ms |
| Non-RT | 9.49 / 9.28 ms | 9.58 / 9.36 ms |

The differences (−0.15 ms RT, +0.09 ms non-RT) are within the visit-to-visit spread. The coarser fill costs a lookup only for pixels reading a not-yet-gathered corner. Cell shading now calls `scatter` 16 times a cell instead of 16 plus once per light, which is not measurable here. One non-RT process's frame 0 took 130 ms: the first process after the shader change compiled its pipelines.

## 5. Claim notes (m7-p0-claims.md)

- **W1** (no counter, no running mean): cell values are still replaced on each gather and each shading. The albedo is a fixed 16-direction quadrature per shading, not an accumulation. `GI_BOUNCE_NONE` is a state, not a sample count.
- **W2** (no ranking): nothing here decides which rays are cast or which cells are gathered first.
- **W3** (fixed weights, no jitter): the bounce fill uses the lookup's fixed geometric weights and the fixed coarser level. The query point is the cell's fixed query point.

## 6. Changes to other units' assumptions

- **Cell shading contract.** `scatter` is still evaluated toward the cell's normal, but lights now reach bounce through the program's directional albedo times the cosine. docs/m7-g2.md section 2.4 is updated. Programs that relied on `scatter(l, n)` for lights in bounce see Lambertian-equivalent bounce instead.
- **Hot record.** Bounce scale word 16 has a third state, `GI_BOUNCE_NONE`. Table readers that treat any word ≤ 0 as no bounce are unaffected.
- **Test builds.** `RENDERLAB_GI_SHADERS` now also overrides `gi_shade_entry.wgsl` (src/program.rs), so cell-shading variants can be screened on one binary.
- **Runner.** `scripts/m7-g2-run.py` gains `example LABEL` (builds API1's `game_scene`) and a `PATH:game:BUILD[:ARGS]` lease step; `scripts/m7-g2c-run.py` runs it under G2c's name.

## Evidence

Under `D:/Coding/renderlab-work/m7-g2c`: `runs/a1` (main and G2c cell shading, game scene, every frame, both paths), `runs/b1` (bounce-diagnostic builds), `runs/c1` (final code: game scene, lib suite, quarry, SV), `runs/c2` (timing), `analysis/` (inspected crops and hot-pixel maps), `tools/` (crop, row, speckle and hot-map scripts), `variants/base` (main's GI shaders for same-binary comparisons). Per-frame PNG sets are deleted once summarized; inspected frames are kept.
