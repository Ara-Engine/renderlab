# P-SHOW: Ara's Showcase scene through the bridge

Unit P-SHOW, 2026-11-10, branch `t3/p-show` from main `46caa60` (v0.1.2 plus `Engine::queue()`).
Milestone 8 ([milestone-8.md](milestone-8.md)). Labels: **M** measured here, **A** reported by
another unit, **I** inference.

## Result

**MERGE** the skinned-pose fix. On Showcase, RT frames get about **1.9x faster**: still 23.6-27.8
to 13.2-14.0 ms; moving 26.1 to 14.0-15.8; cut 27.9 to 14.2-15.9. Non-RT frames get 1.1-1.2 ms
faster. Images are unchanged within a few codes, inspected still, moving and on cuts.

- **Cause (M).** The GI cache and the non-RT virtual shadow cache treated every skinned placement as
  a moved caster on every frame, whether or not its pose had changed. Showcase holds two skinned
  mannequins, so their bounds were re-invalidated every frame:
  - GI re-traced their swept boxes and relit one eighth of every cell (`RELIGHT_PERIOD`) per frame,
    forever. That is 13-17 ms of GI passes, outside shading, on both paths.
  - The shadow cache re-rendered their caster pages: about 1.2 ms of non-RT shadow compute.
  - Our own scenes (game, PBR, quarry) settle their caches when nothing moves. Showcase never did.
- **Fix.** A placement deforms only when its skinned mesh or its pose revision changed since the
  previous frame (`skinning::deformed`). `set_joint_palette` bumps the revision only on a new
  palette, so an unchanged pose leaves the mesh's positions, and every cache built from them, valid.
  - The shadow cache uses it on non-RT. GI uses it on hardware-ray frames.
  - **GI on software-ray (non-RT) frames keeps the old behaviour, as scaffolding.** With the cache
    settled, non-RT flat white faces show their GI cells as blotches (below). The constant relight
    had been averaging them away through TR1. That is the known non-RT grain defect (P-DEFECT's
    unit). The one-line switch in `gi_cache.rs` goes when that is fixed, which then saves another
    ~16 ms of non-RT GI on this scene (M: 43 to 31 ms).
- **Not caused by the bridge.** Anisotropy 16, the constant sky, RGBA8 textures, TR1, local shadows
  and the media/post stack each measure within noise. Showcase stays under 64 mesh-material groups
  (no `ARA_RENDERLAB_LIMIT` in any log). The bridge's per-frame `set_joint_palette` is harmless,
  since Renderlab returns early on an equal palette. No bridge change is needed for speed.
- **Rejected: glossy ray scaling** (below). It was the second-largest non-RT lever, -12 ms of
  shading, but it brightened the white glossy specks, so it doesn't ship.

## Reproduction

Showcase at 1920x1080 on an RTX 4070 SUPER (12,282 MiB), NVIDIA 617.14, Ryzen 7 7800X3D, Windows 11.
`ara capture --camera Camera --ev100 10.17 --stability 60`, the mode the integration thread used
(A): a still camera, 16 settle frames, then live frames; the frame number is the median
`GpuSample::render_span`. The mannequins hold their pose in this mode (the capture's flicker map
shows no motion on them). The moving and cut runs are `--video 60 --video-move 2,0,0.5 --video-yaw 25`
and `--video 8 --video-yaw 720`. `WGPU_VALIDATION=0 WGPU_DEBUG=0`.

| Build | RT ms | non-RT ms | Source |
|---|---:|---:|---|
| Ara dev build, Renderlab v0.1.1 | 29.8 | 47.3 | A, PR #4 |
| Ara dev build, Renderlab v0.1.2 | 22.4 | 43.6 | A, integration `pc-v012` |
| Ara `perf` profile (release opt-level, no debug assertions), Renderlab 46caa60 | 22.2-27.8 | 42.6-43.4 | M |

Ara Epic is 3.0 ms and Ara Auto 2.1 ms on the same view (A). Renderlab's GPU time does not depend on
the Rust profile (the dev numbers already had Renderlab at opt-level 3 as a dependency), so dev and
release agree within run-to-run spread. RT baselines vary from 22 to 28 ms across leases, a spread
the tables below show inside one lease.

Build identity: Renderlab worktree at `46caa60` plus this branch; Ara `f3248b339` (PR #4's head) in
a detached worktree with `renderlab` patched to the local path and two local, unpushed ablation
hooks in the bridge (`PSHOW_OFF`, `PSHOW_ANISO`, `PSHOW_FREEZE`, `PSHOW_NOSKIN`, a per-sample pass log).

## Per-pass breakdown (M)

Medians of the last 60 GPU samples. "GI etc." is the span minus the timed passes; with GI off it
falls to 0.5-1.2 ms, so it is GI's own cache passes. One lease per block.

| Run | Path | Span | Shading | Shadow compute | GI etc. |
|---|---|---:|---:|---:|---:|
| baseline | RT | 25.96 | 7.76 | - | 16.68 |
| GI off | RT | 7.31 | 4.72 | - | 1.17 |
| reflections off | RT | 18.25 | 2.24 | - | 14.57 |
| GI, reflections, local shadows, fog, post off | RT | 3.19 | 1.06 | - | 0.80 |
| poses frozen (old code) | RT | 26.44 | 7.77 | - | 17.05 |
| no skinned meshes | RT | 15.00 | 8.41 | - | 4.85 |
| skinned-pose fix | RT | 13.23 | 7.79 | - | 3.60 |
| baseline | non-RT | 43.25 | 20.78 | 3.55 | 17.82 |
| GI off | non-RT | 24.91 | 19.87 | 3.55 | 0.48 |
| reflections off | non-RT | 24.84 | 2.18 | 3.56 | 18.09 |
| GI, reflections, local shadows, fog, post off | non-RT | 6.04 | 1.11 | 3.57 | 0.48 |
| poses frozen (old code) | non-RT | 47.06 | 21.07 | 3.60 | 19.03 |
| no skinned meshes | non-RT | 30.86 | 24.96 | 2.22 | 3.24 |
| skinned-pose fix, GI included | non-RT | 31.74 | 24.81 | 2.32 | 3.53 |

TR1 off, anisotropy 1, local shadows off and fog/post off each move the span by less than run noise.
Receiver (0.47 / 0.87 ms), reconstruction (0.44) and present (0.13) are unchanged throughout.

Freezing the poses changes nothing on the old code, and the fix matches a scene without skinned
meshes, so the cost was the skinned meshes' presence, not their animation.

Where the rest goes:
- **Non-RT shading, 20.8 ms, is mostly glossy reflections** (2.2 ms with them off). Software rays
  (R1n) trace a flat instance list, and hits shade inline; Showcase has many glossy props, a glossy
  turntable and a concrete floor whose roughness map puts 23 % of its texels in the 0.35-0.5 fade
  band, where each traces the full 16 strata.
- **RT shading, 7.8 ms**: 2.4 ms glossy reflections, 3 ms GI lookups.
- **Non-RT shadow compute, 2.3-3.6 ms**: the virtual sun lookup.

Nsight GPU Trace was not needed: the pass timings and ablations attributed the large costs. A trace
script for the bridge path is in the evidence folder (`trace.sh`), unused.

## After (M)

One lease, shipping-equivalent binary (the cache fix; see the confirmation run below), against the
baseline binary in the same lease.

| Phase | Path | Baseline ms | P-SHOW ms |
|---|---|---:|---:|
| still | RT | 23.62 / 27.81 | 13.19 / 14.05 / 13.68 |
| moving | RT | 26.11 | 14.04 / 15.75 |
| cut | RT | 27.87 | 14.23 / 15.89 |
| still | non-RT | 43.43 | 42.0-44.7 (shadow compute 3.55 → 2.3-2.4) |

The second non-RT lease ran with heavy contention (its baseline repeat read 58 ms), so non-RT is
compared by pass: shadow compute -1.2 ms, everything else unchanged by design.

### Images (M, inspected)

- **Baseline determinism:** two baseline runs in different leases are byte-identical (RT and non-RT).
- **RT still:** mean 0.29 codes, max 20, 0.003 % of pixels over 8 codes. Only a faint cell
  texture on the white plinths, invisible at 1x (`diff-final-rt.png`, x8).
- **Moving, frame 59:** RT 0.004 % of pixels over 8 codes (max 25); non-RT 0.002 % (max 26).
- **Cut, frames 1 and 7:** at most 0.010 % of pixels over 8 codes (max 124 on one non-RT pixel).
- No new stray bright pixels, grain or edge crawl in the crops of the stage, turntable, plinths
  and props.

## Rejected and blocked (M)

- **Glossy strata scaled by the pixel's traced share** (reflectance x traced fraction, squared).
  Non-RT shading 25 → 11.5 ms, RT 7.8 → 4.7 ms. Rejected: the dark dielectric turntable (roughness
  0.35, reflectance about 0.05) dropped from 16 strata to 4, and the white specks on it got brighter
  and denser on both paths (`crop-turn-*.png`). Ordinary grain stays bounded, but rare bright hits
  are 4x brighter per ray. It can return once the white-speck source is fixed.
- **Fade-band-only scaling** (strata x traced fraction, linear, so no ray carries more than a fully
  traced pixel's share): within noise on both paths (RT shading 7.5 vs 7.7, non-RT 20.9 vs 20.8).
  The floor's fade-band texels are not where the shading time goes. Dropped.
- **Settled non-RT GI** (the fix on software-ray frames too): non-RT 43 → 31 ms, but the plinths'
  flat white faces show blocky GI cells that the baseline's churn hid (`crop-plinth-skin-nonrt.png`:
  baseline, fix, no skinned meshes). Same image as any Renderlab scene without skinned meshes; it
  is the non-RT grain defect. Blocked on it, scaffolding in place.

## Visible defects seen in Showcase (for P-DEFECT)

- **White specks in glossy reflections**, both paths, densest on the black turntable toward
  grazing angles, also on glossy props. Present in the baseline.
- **Non-RT flat-face blotches**: GI cells on the white plinth sides, 8-16 px blocks, when the cache
  settles; in the baseline, the constant relight shows as frame-to-frame flicker of the same cells
  (`b0-nonrt.live.flicker.png`).
- The stepped glass rim was not inspected closely here.

## Standing harness

`tools/showcase/showcase.py` drives Ara's player against this checkout:
`prepare` (a detached Ara worktree with `renderlab` patched to this repo), `build` (Ara's `perf`
profile, two jobs; wrap it in the build slot), `fetch` (Ara's `ara-samples fetch showcase` and
`kits showcase`), `capture rt|nonrt|ara NAME [-- capture args]` (inside a GPU lease) and `summary`.

- **Content stays out of this repository.** Showcase's third-party assets are Poly Haven (CC0) and
  Khronos glTF sample models (licences per model), listed in Ara's `samples/THIRD_PARTY_ASSETS.md`.
  Ara does not commit them either: `ara-samples fetch` downloads them from pinned URLs with BLAKE3
  checks.
- The capture number is the bridge's `render_span` median. Per-pass splits need the bridge to log
  `GpuSample`'s pass fields (recommended to the integration thread).

## Gaps

- **Moving characters still cost GI.** A skinned pose that really changes every frame (gameplay,
  not this capture) still relights one eighth of all cells per frame, about 13 ms on RT here. The
  relight is global for any moved caster. A relight bounded to cells whose gathers can see the swept
  box is the next lever; unmeasured.
- **Non-RT glossy reflections** (20 ms of shading here) and **non-RT GI** (16 ms) wait on the two
  defects above.
- Mannequins use the bridge's four-influence skinning; Ara's animation was not exercised (the
  capture holds the pose).

## Evidence

`D:/Coding/renderlab-work/p-show` (deleted at the end except summaries): `captures/*.json`,
`*.samples.csv` (per-sample pass log), `lease/runs/*.log`, crop sheets and diffs, `passes.py`,
`plan.sh`, `plans/`.
