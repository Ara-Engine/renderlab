# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Skinning

- SK1 uses ALLOW_UPDATE/PreferUpdate for animated BLASes and the frame's TLAS;
  wgpu 30 currently rebuilds both. Actual incremental refit remains open.
- The non-RT path skins full-detail source clusters. Skinned cluster LOD is
  unavailable; bounds are conservative mesh-wide bounds. CPU software-GI BVHs
  rebuild for changed poses, and caster invalidation is conservative.
- RLS v1 supports four influences, TRS nodes and one active sampled clip. Matrix
  nodes, additional influence sets and morph targets are explicitly rejected.
  Clip blending, events, dual-quaternion skinning and automatic root motion remain
  outside the overnight base.
- Existing TR1 model-transform edits still reset history. SK1 supplies previous
  deformed vertex positions; its walk example puts translation in the palette.
- Hardware shadows retain the existing opaque, double-sided caster contract. The
  example enables double-sided materials explicitly.
- No new Unreal or path-traced comparison was run for this overnight unit. Its
  visual and timing evidence, and their limits, are in [m7-sk1.md](m7-sk1.md).
