# Renderlab 0.1.3

Released 2026-11-10. This release brings fixes found on Ara's Showcase scene, Renderlab's first real engine content, and adds a per-pass GPU timeline. The engine API is unchanged from [0.1.2](release-0.1.2.md), and everything in the [0.1.1](release-0.1.1.md) and 0.1.2 notes still applies, including the known gaps and the VIA patent flag on the non-RT sun-shadow march. Source only.

Showcase figures below are median GPU time per frame at 1080p through Ara's bridge, on an RTX 4070 SUPER with NVIDIA 617.14. Quarry ratios compare against Unreal 5.7.4 Epic from [p-yard.md](p-yard.md). Evidence: [p-show.md](p-show.md), [p-defect.md](p-defect.md), [p-show2.md](p-show2.md).

## Faster

| Change | Showcase | Quarry vs Unreal Epic |
|---|---|---|
| Skinned meshes count as moved only when their pose changes. Before, both GI and the non-RT shadow cache relit every frame. | RT still 25.4–28.8 → 13.7–14.8 ms | — |
| The non-RT relight scaffolding is removed, so non-RT GI settles too. | non-RT still 55.2 → 33.1 ms (GI 20.5 → 0.9 ms) | non-RT still 8.66 ms, 2.15x slower than SW Lumen Epic; moving 10.74 ms, 2.61x slower |
| Small faces in mip-chain sky maps are banked at load, so lookups stay in-face. | RT still with the real sky map 52.5 → 33.9 ms | RT still 4.16 ms, 1.08x faster than HW Lumen Epic; moving 4.32 ms, 1.06x faster |

## Better image

- **Stray white pixels in glossy reflections, fixed on both paths.** The cause was glossy rays landing on mirror balls' sun highlights. Reflection hits now widen their lobe by the ray's cone, which keeps the energy but spreads it. On the turntable, stray pixels per frame fell from 12–24 to 2–4, and the glow now matches the path tracer. User programs with their own lobe still shade hits exactly.
- **Non-RT GI blotches after cuts, fixed.** Non-RT gathers 32,768 cells per frame, the same as RT. Flat faces are clean by frame 8 after a cut instead of about frame 100.

## Tools

- **`RENDERLAB_PASS_LOG=<file>`** writes a named per-pass GPU timeline (skinning, TLAS, sun shadow, atmosphere, visibility, the GI stages, shading, TR1, transparency, post) as one JSON line per frame. It costs nothing when unset.
- **`RENDERLAB_ABLATE`** switches features off for measurement.
- **`tools/showcase/showcase.py`** runs Ara's Showcase as a standing test scene, fetching the content with Ara's own tools.
- **`tools/p-yard/`** replays the matched Unreal yardstick.

## Known gaps added

- **Sky-map shading cost.** Under any map sky, RT shading takes about 17 ms more than with a constant sky. That is the cost of the reflection rays' environment lookups themselves; it needs a profiler pass.
- **Stepped glass rim.** Still open on both paths; the next step is a back-face refraction.
- **Brushed aluminium reads matte.** Rough metals use sky visibility around the normal for their unreflected half, so ground-facing reflections read as sky. A fix is built but not yet measured.
- **Speed against Unreal.** Distance to the 3x target: RT needs about 2.5x more, non-RT about 7x. Cut frames remain the worst case (quarry RT 12.6 ms, non-RT 43.1 ms against Unreal's 4.1–4.5 ms).

## Verification

- `cargo test -p renderlab --lib`: 207 passed, 0 failed, 164 ignored (GPU-only). This ran in P-SHOW2's lease on `58ffc20`; the merged tree's code is identical to it.
- Showcase images were inspected still, moving and on cuts, on both paths.
