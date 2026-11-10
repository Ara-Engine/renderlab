# Renderlab 0.1.1

A stop point after the first round of milestone 8 ([milestone-8.md](milestone-8.md)), tagged on 2026-11-10 so Ara can depend on Renderlab as an external package. It is a drop-in update from [0.1.0](release-0.1.0.md): the frozen `renderlab::engine` API is unchanged except for one behaviour fix (below). As before, it ships source only. The private quarry scan isn't needed, and the license is still undecided.

Every number below is a 1080p service median on an RTX 4070 SUPER under Vulkan (driver 617.14), measured in the ARA1 harness ([m7-ara1-plan.md](m7-ara1-plan.md)). Each row compares the build before and after that change, in one paired run, so rows taken at different times don't chain exactly. Evidence for each row is in [milestones-5-6-decisions.tsv](milestones-5-6-decisions.tsv) (rows `M8-*`) and the linked unit notes.

## Behaviour change

- `Engine::cut()` now also restarts volumetric fog history, so fog no longer carries the old view across a camera cut. The cut's fog frames move toward a fresh render (about 42 percent less error there). `Renderer::reset_atmosphere_history()` is new and public ([p-nrt2.md](p-nrt2.md), [p-post.md](p-post.md)).

## Faster, same image

These changes are byte-identical to 0.1.0's output on both paths, checked by HDR/PNG hashes.

| Change | Measure | Before → after (ms) |
|---|---|---|
| 32-lane shading tiles ([p-lts2.md](p-lts2.md)) | 64 shadowed lights, RT / non-RT | 10.10 → 6.63 / 15.86 → 13.69 |
| same | 64 unshadowed lights, still, RT / non-RT | 8.72 → 4.76 / 9.99 → 9.04 |
| Local-light exact-zero skips and 4x4 PCF ([p-lts.md](p-lts.md)) | 64 shadowed lights, RT / non-RT | 12.13 → 11.08 / 25.52 → 18.00 |
| Light-loop rework ([p-lts4.md](p-lts4.md)) | 64 shadowed lights, non-RT | 13.26 → 12.13 |
| Five non-RT cuts ([p-nrt.md](p-nrt.md)) | game, still / moving / cut, non-RT | 10.41 / 10.78 / 12.38 → 8.99 / 9.39 / 10.59 |
| Sun-shadow batches spread over lanes ([p-nsq3.md](p-nsq3.md)) | game cut sun-visibility pass, non-RT | 31.41 → 26.72 |
| Two submits per frame ([p-nrt3b.md](p-nrt3b.md)) | GPU idle between frames, non-RT / RT | 1.85 → 1.25 / 0.87 → 0.45 |
| same | quarry cut, non-RT / RT | 25.34 → 24.45 / 15.06 → 14.77 |
| Six GI cost cuts ([p-gi.md](p-gi.md)) | quarry moving GI cost, non-RT; cut-window GI sum | +2.45 → +2.26; non-RT −12 %, RT −3 % |
| Shader reload bookkeeping ([p-shc.md](p-shc.md)) | first updated frame after a reload, RT | 1635 → 147 |
| Non-RT batched GI gather ([p-gather.md](p-gather.md)) | moving, game / quarry, non-RT | 11.22 → 11.06 / 11.55 → 11.26 |

The batched gather's game cut is +0.6 ms in its small sample. The RT kernel is unchanged, and RT's early-still timing varies from run to run, as it already did before this change.

GPU memory ([p-mem.md](p-mem.md), [p-bvht.md](p-bvht.md)):
- Quarry daylight, total: non-RT 4177 → 3944 MiB, RT 3376 → 3344 MiB.
- 64 lights, non-RT warm: 1827 → 1411 MiB.
- The packed GI BVH saves 95 MiB, and its suspected quarry slowdown didn't reproduce in a quiet rerun.

## Better image

- **Non-RT sun shadows** use the same rotated directions and budget as RT under TR1: 256 on a cut, 32 in motion, 8 when held ([p-nsq.md](p-nsq.md)). This was approved by Islam.
  - Held penumbra grain on the S2 fixture: 13.4–14.5 → 1.4–2.3 (RT is 1.3–1.7).
  - Courtyard error against RT: in flight 0.0174 → 0.0114, still 0.0206 → 0.0108.
  - The cost is +2.6–3.3 ms on moving frames and +28–30 ms on cuts; the speed-ups above have since won some of it back.
- **Courtyard crate noise** in the example scene ([p-flat.md](p-flat.md)): the spot lamp sat inside its own post. On RT the crate-top speckle went 0.759 → 0.079, within the path-traced reference's own grain (0.080–0.096).
- **TR1** keeps history for one shadowed sun where no moving caster can change it ([p-flat.md](p-flat.md)).

## Parked, not in this release

These branches are pushed to origin with a stop record and a next step. Each is unmerged and excluded from 0.1.1:

| Branch | What it is | Where it stopped | Next step |
|---|---|---|---|
| `t3/p-s4b` (`2fe2d1b`) | S4: a non-RT sun-visibility gather with weight-free half-space tests ([p-s4b.md on its branch](https://github.com/Ara-Engine/renderlab/blob/t3/p-s4b/docs/p-s4b.md)) | The 1080p game cut's sun-visibility pass drops from 26.7–27.3 to 8.3–8.8 ms. The image looks the same, and it matches RT on 99.994 % of pixels. But cuts in the 960×540 quality scenes get slower (3.40 → 5.93 ms with the original sun, 5.31 → 16.69 ms with the wide sun). It refuses the quarry. | Choose S4 or the march per frame from the queued pixel count and the sun size. After that, replace the march on every frame, which retires the VIA exposure below. |
| `t3/p-rts2` (`293ff7a`) | Recheck of the earlier P-RTS listing variants for RT sun shadows on cuts ([p-rts2.md on its branch](https://github.com/Ara-Engine/renderlab/blob/t3/p-rts2/docs/p-rts2.md)) | Not run: the release freeze came first. | Inspect the S2 large-light moving fringe for each variant, then port and time any variant that passes. |

Patches kept outside the repo: `s3q-replay.patch` (rejected, slower) and the P-LTS3 C froxel culling patch (unfinished), both under `D:/Coding/renderlab-work`.

## Known gaps

The full per-feature list is in [m7-gaps.md](m7-gaps.md). The largest:

- **Patent exposure: the shipping non-RT sun-shadow method.** The non-RT Surface sun-shadow march (`shaders/virtual_shadow.wgsl`) is **at risk** under VIA Technologies' US 8,471,853 (active to 2032-04-25) and its divisional US 8,605,088 (active to about 2028-10-10). Both are fee-paid. In the shipped lookup, the steps the claims recite are all present: it reads a triangle's three light-space vertices, computes coordinate weights, tests consistency from them, and reconstructs and compares the occluder's height. The unresolved question is whether the march's displaced sample counts as the claims' "testing pixel".
  - This is an engineering claim chart, not legal clearance ([patent-via-8471853.md](patent-via-8471853.md)).
  - The RT path does not use this method.
  - The planned replacement, S4 ([design-nonrt-sunvis-s4.md](design-nonrt-sunvis-s4.md)), is a BVH gather with weight-free half-space tests. It charts clear of both patents, but only once it replaces the march on every frame, not just cuts.
  - Anyone shipping the non-RT path in a product should get a legal review first.
- **Speed against Ara:** non-RT is still several times slower than Ara's raster path; game cut frames take 20–33 ms; 64 lights take about 4.8–6.6 ms on RT and 9–12 ms on non-RT, against a 3.5 ms target; GPU memory is above Ara's.
- **Against Unreal:** about 3x Unreal's speed at better visual quality is the direction, and it isn't demonstrated yet.
- **Development loop:** the first edit to a new shader still takes about 21–26 s to reach the image. Background compiles keep old frames exact but can hitch up to 432 ms ([p-shc2.md](p-shc2.md)).
- **Non-RT sun shadows:** the quarry contact crop has slightly more near-field speckle (0.01496 → 0.01505, accepted).
- **Non-RT timing:** under post workloads, non-RT in-flight replay can change scene HDR between runs. The responsible resource is unisolated, so a cross-frame race isn't ruled out ([p-post.md](p-post.md)).
- **Unchanged from 0.1.0:** meshes over 2^19 triangles, streamed meshes and skinned meshes are not traced in non-RT reflections. Vulkan validation still reports inherited SPIR-V layout diagnostics (VUID-StandaloneSpirv-None-10684).

## Clean-clone check

A fresh `git clone` of `27ab341` from GitHub (the release commit minus this paragraph) was built in its own debug target with no private assets and no Fox, on 2026-11-10 at about 02:00 UTC.

| Check | Result |
|---|---|
| `cargo test -p renderlab --lib` | 200 passed, 0 failed, 164 ignored (GPU-only) |
| `game_scene --path rt --frames 48 --every 8` | exit 0, 72 s debug; frames inspected |
| `game_scene --path nonrt --frames 48 --every 8` | exit 0, 68 s debug; frames inspected |

All three ran in one GPU lease (`root-v011-checks-1`, 148 s) on the RTX 4070 SUPER.
