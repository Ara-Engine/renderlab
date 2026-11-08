# M7-MQ2: reflections that keep up with the camera, and skinned history with GI on

2026-11-08, branch `t3/m7-mq2`: MQ1 (`f753cfa`) and MP1 (`b276863`) merged onto main `ddd823c`, then main `4c8ef7d` (R1n, GC1, PC1, LC2, AT2, TP2) merged. Rules: AGENTS.md's overnight push (ship when clearly better and not visibly worse in inspected images; gaps go to [m7-gaps.md](m7-gaps.md)), [P0](m7-p0-claims.md). Labels: **M** measured here, **I** inference.

**Status: shipped as the default on both paths.**

**Problem (MQ1's gap 1).** TR1 keeps a pixel's history by surface identity. A mirror or glossy reflection moves across its surface when the view direction turns, so the history trails it: in MQ1's measurements the chrome sphere's error against a 2×2 supersampled control doubled (3.07 → 6.44 codes) and its flicker went 4.05 → 9.05. The reflected arch and gold sphere smeared by about a pixel. The same lag softens the gold sphere's reflections: this is part of the "mushy" glossy look Islam reported (RQ1 owns the rest: ray count, the fade to the cache, demodulation).

**Fix.**
- A program may declare how much of its output moves with the view: `fn view_dependence(s, input) -> ViewDependence { alpha, share }`.
- The shading entry turns that into a per-pixel sensitivity, stored in the HDR sample's alpha: share ÷ the reflected direction's turn across one pixel.
- TR1 caps that pixel's history while its view direction turns, so the reflection's share-weighted lag stays under a quarter pixel.
- Still views keep full history. Programs that declare nothing are unchanged.
- The standard program and the example's photoreal program declare their specular share.

**Skinned characters (MQ1's gap 3).** SK1 rejected every pixel's history scene-wide on each animated frame while GI was on. MQ2 drops that widening; skinned frames instead clamp history to MQ1's 3×3 box, as rigid motion does. The swept volume for direct shadows is kept.

RESULTS_PLACEHOLDER
