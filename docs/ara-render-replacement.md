# Replacing ara-render with Renderlab

Root, 2026-11-11. Islam (about 17:10 UTC): "I honestly do not care about ara's renderer... everything here is not being used by anyone yet all of it can be thrown away with no issues including ara's rendering". Nothing in Ara or Renderlab has users: no compatibility, migration or deprecation work in either repository.

Approved plan: ara-render is frozen; Renderlab is the default in Ara's player and the editor's Game view now (the Linux bridge thread flips it); ara-render is deleted once the editor's Scene view, picking and the Mac work without it. This is a second line of work beside the non-RT goal ([milestone-8.md](milestone-8.md#non-rt-goal-2026-11-11)), not a replacement for it.

**Decision, 2026-11-11 evening (Islam, relayed by the coordinator; Ara records it as D40 in its docs/decisions.md):** "I want ara's old renderer toally removed icl like totally and replaced with ours then the scene view is figured out, I do not think it is that hard to do". This replaces the last step above. ara-render is deleted now, everywhere, in two Ara stages, with no fallback in the preview; anything only it drew becomes a gap row. The editor's Scene view runs on Renderlab only, and the Mac runs on the ara-gpu port (`src/backend_ara`) as its only renderer, so the port follows each Renderlab output the editor reads (the id image first). For Renderlab this makes the packed id image, the one-pixel id request and views per `Engine` (RL#39) in main tonight the floor for the next day's preview; the editor's next asks are ranked in [m7-gaps.md](m7-gaps.md#editor-after-ara-render-2026-11-11).

The list is the "Blocks deleting `ara-render`?" column of `crates/ara-renderlab/AGENTS.md` in Ara (read at Ara main `aab165a87`). Below are the rows that are Renderlab's, in the order root would take them. Order and method are our engineering call.

## Order

| # | Row | Kind | What the Ara side needs to do | Judged on |
|---|---|---|---|---|
| 1 | **Entity-ID output and view modes** (picking, capture legends, debug views, the rendering debug switches the row names) | Small unit; an API addition | Map Ara entities to Renderlab objects when it spawns them; read the ID target for picking and legends; route its view-mode enum to Renderlab's | Every pixel's ID matches a CPU ray cast on the game scene and Showcase; transparent and particle pixels follow a stated rule; zero cost when off; under 0.3 ms at 1080p when on; each view mode matches a reference image of what it claims to show |
| 2 | **More than one view per `Engine`** (the editor's Scene view beside the Game view) | Large unit; reaches through per-view state (below) | Create one view per editor viewport; render each with its own camera and size | Each view's frames identical to the same camera rendered alone; scene memory shared (second view adds only its per-view targets and histories, stated in MiB); cost of the second view at the Scene view's size |
| 3 | **Editor-acceptable frame cost** (Showcase 17-34 ms through Ara on the RTX against about 3 ms for ara-render) | Units, mostly ones already planned | Pick the tier per viewport (Game view: full; Scene view: the cheap tier) | Showcase through Ara at 1080p under a stated budget per tier (proposed: editor tier at or under 8 ms on the RTX on both paths) with no visible artefact against the full tier, still and while the editor camera moves |
| 4 | **Material graphs to Renderlab WGSL programs** (vertex paint, second UV set, AO maps) | Ara-side compiler; small Renderlab API additions | Write the graph-to-WGSL compiler against Renderlab's program API | Renderlab: vertex colour, UV1 and an occlusion input reach programs on both paths (a test per input). Ara: its material library renders through Renderlab within the bridge's tolerance of ara-render's frames |
| 5 | **BC7/BC5 texture upload** | Small API addition | Pass the compressed blocks instead of decoding to RGBA8 | Texture memory on Showcase down about 5x; frames within the bridge's tolerance of the RGBA8 path (BC7 is lossy at the source, not at upload) |
| 6 | **Large levels on the non-RT path** (shadow cost with caster triangles; Outpost's foliage) | The page-pool unit, already briefed (`D:/Coding/renderlab-work/n-pool/brief.md`) | Outpost as a test level | Outpost renders on both paths on the Radeon within a stated frame budget; shadow cost no longer scales with caster triangles |
| 7 | **Rect, disc and tube lights; IES profiles** | Unit | Map its light components | Against path-traced references of each shape and a measured profile, on both paths, still and moving |
| 8 | **Atmosphere: luminance factors, clouds, local fog volumes, fog emission** | Unit | Map its sky and fog components | Against Ara's authored scenes and a reference; cost on the quarry and Showcase |
| 9 | **Decal emission, occlusion, a per-object receive mask** | Small API additions | Pass the extra maps and the mask | Per-feature fixtures on both paths |

Metal is not on this list: it is the ara-gpu port's (draft PR #3), which makes that port part of this plan.

## Why this order
- **ID output first.** It is small, it unblocks picking and agents' captures at once, and its design fixes how per-object data leaves the renderer, which multi-view then has to carry per view.
- **Multi-view second.** It decides when the Scene view moves, and it is the largest structural change; doing it after the ID output means each view carries its own ID target from the start.
- **Frame cost third** because it is mostly in flight: the `Disk` sun lookup (quarry non-RT frame 8.6 to 4.3 ms), P-GLOSSY (Showcase glossy), then GI. The editor tier is a settings choice on top, not a separate renderer.
- The rest are independent and can run when a slot is free.

## Row 2 in detail: what is per view today
`Renderer` holds one of each of these, and several are keyed to the camera:
- temporal reconstruction's histories, restart zones and upscale state;
- the post chain's histories (exposure, motion);
- the atmosphere's temporal fog and aerial volumes;
- the primary-visibility state (hit records, the visibility buffer, depth, MSAA targets) and transparency's targets;
- the sun shadow map's page requests and clipmap placement (levels follow the camera; the pages themselves are world data);
- the per-pixel GI probe and its idle key (the GI world radiance cache itself is world-space and shareable);
- reflection film accumulation; the frame-in-flight slots and timing.

So multi-view is not contained: it means splitting `Renderer` into shared scene and cache state and a `View` that owns everything above. World caches (GI cells, shadow pages, the BVHs, uploaded meshes and textures) stay shared; a second view's page requests add to the same page pool, which is one more reason the page pool should land first.
