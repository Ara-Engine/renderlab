# N-VSM1: non-RT sun shadows answered from the cached virtual shadow map

Milestone 8 ([milestone-8.md](milestone-8.md), "Non-RT goal"), 2026-11-11, branch `t3/n-vsm1` from main `9771303` (main `6bbdfe3` merged in). Target: [unreal-pass-budget.md](unreal-pass-budget.md), sun shadows at or under 0.6 ms in every state, the cut included. Labels: **M** measured here, **C** read from code, **I** inference.

## Result

**A new lookup, `VirtualLookup::Disk`, answers sun visibility from the cached clipmap alone. It is opt-in; `Surface` stays the default.** The quarry yardstick has not run: its lease needs 14 GiB of free RAM, the machine idles at 12.6 to 13.8 GiB, and root has put the quarry on hold until a window is arranged ([Not done](#not-done)). Everything below is the game scene (courtyard) and the S4 and S5 benches.

| Scene, non-RT, GPU p50 (M) | Sun pass, Surface | Sun pass, Disk | Frame, Surface | Frame, Disk |
|---|---:|---:|---:|---:|
| Courtyard with the fox, 1080p, camera moving, the 20 frames before a cut | 42.4 ms (p95 45.1) | **1.84 ms** (p95 1.96) | 68.0 ms | 28.3 ms |
| Same, the frame of the cut | **159.4 ms** | **1.61 ms** | 188.3 ms | 30.5 ms |
| Same, 121 frames after the cut | 23.6 ms (p95 25.0) | 1.67 ms (p95 1.77) | 56.4 ms | 33.7 ms |
| Courtyard, two-sphere hero, 720p, moving | 16.9 ms | 1.14 ms (p95 1.19) | 32.3 ms | 16.3 ms |
| S4 door, 1080p | 3.63 ms (p95 5.40) | 1.39 ms (p95 1.68) | 11.2 ms | 9.0 ms |
| S5 cube, 1080p | 4.29 ms (p95 5.30) | 0.48 ms (p95 0.83) | 14.2 ms | 10.4 ms |

- **The cut costs what a moving frame costs.** The lookup has no direction budget, so TR1's cold, restart and moving counts change nothing. On the fox scene the cut frame's sun pass falls from 159 ms to 1.6 ms and the frame from 188 ms to 30 ms.
- **The 0.6 ms target is met on S5 only.** The courtyard at dusk (0.04 rad sun, shadows many metres long) costs 1.1 ms at 720p and 1.6 to 1.8 ms at 1080p. Of that the lookup is 0.67 to 1.35 ms, page raster 0.33 to 0.38 ms and page requests 0.08 to 0.17 ms.
- **Picture: level with Surface against the RT path on the courtyard, behind it on S4 and S5.** Whole-frame RMS against the RT path's picture, display codes (M):

| Frames | Surface | Disk |
|---|---|---|
| Courtyard flight, 7 frames | 1.19 to 2.51 | 1.28 to 2.33 (better on 4, worse on 3) |
| Fox scene, 7 frames round the cut | 1.54 to 1.75 | 1.47 to 1.73 (better on 2, worse on 5) |
| S4 door, 3 frames | 1.25 to 1.79 | 1.78 to 2.30 |
| S5 cube, 3 frames | 0.27 to 0.31 | 0.39 to 0.43 |

- **No Unreal ratio and no Unreal picture exists for this unit.** Both come from the quarry lease. The courtyard and the benches have no Unreal build.
- **Verdict: MERGE as an opt-in lookup, not as the default.** The default needs the quarry picture against Unreal and the reference, and the open picture items below.

## Method

Per pixel, the lookup projects the map's occluders back onto the sun disk (`shaders/virtual_shadow_disk.wgsl`). No ray is marched and no triangle is read.

1. **The disk is 64 fixed directions, one bit each.** Each map texel near the receiver is a column of solid. A column part at height `t` above the receiver and light-space offset `q` hides the directions `q / t`: a rectangle for a square texel. A part spanning heights `t0..t1` hides the bounding rectangle of those. All columns' rectangles are OR-ed into the mask, so overlapping occluders never count twice. Visibility is the unset bits.
2. **The samples are a rook pattern.** Sample `s` has the s-th smallest x in the disk and a permuted y rank, both at the quantiles of the disk's marginal distribution (`tools/n-vsm1/pattern.py`). The samples inside a rectangle are then two rank ranges: shifts for x, a 65-entry table for y. An axis-aligned occluder edge crossing the disk passes one sample at a time, 64 steps. The first version used an 8x8 grid, where the same edge passes 8 samples at once: it banded visibly.
3. **Levels follow the occluder's height.** A column at height `t` reaches the disk only from within `slope * t`. Level L serves the heights whose reach is 1.5 to 3 of its texels, over at most a 7x7 window; the next level serves the band above. A texel is therefore a third to a sixth of the penumbra it resolves. Above the coarsest level's band, five max-mip bands continue the ladder.
4. **Column undersides.** The peel layer is the underside when it belongs to the top surface's placement (a closed mesh). Otherwise the underside is unknown and is taken as half the top's height above the receiver. Taking it as solid without end turned every far occluder into a wall beside the receiver and hardened long shadows (sheet below).
5. **Near field from the visible surface.** A map surface closer than one receiver-level texel to the receiver's tangent plane is ignored: LOD receivers lie off the source surface by about that much. Inside that distance the visible surface decides: up to six pixels along the sun's screen direction are occluder samples at their own world points, tested like columns. A sample counts only when its point lies in the receiver's cone toward the disk, so a foreground object that merely overlaps on screen is not an occluder.
6. **Receiver jitter.** On TR1 frames each receiver reads the map from a point of its tangent plane up to half a texel away, a new point each frame, so TR1 integrates a texel-wide box where a texel spans several pixels.
7. **Skips.** One max-mip read gives the highest caster the cone can meet; lit open ground reads nothing else. Each level's window is skipped when its max-mip is below the band. A full mask ends the pixel.

Output is the same 0 to 256 count at the existing `light_visibility` seam. Shading programs see no change.

## Choices: Unreal's structure or ours

| Choice | Taken | Why |
|---|---|---|
| Cached, page-driven clipmap, invalidated by moved casters | Ours as it stands, which is also Unreal's structure | It already exists and is what makes a map lookup cheap. Unchanged here. |
| Per-pixel answer | **Ours: back-projection into a bit mask.** Unreal marches 8 rays of 8 samples and lets TSR remove the noise | The mask has no sampling noise and no budget, so the cut frame needs no extra samples and nothing has to converge after it. Unreal's known failure here is noise and ghosting in penumbrae; this removes the cause. |
| Near field | Unreal's idea, our test: a bias plus a screen-space contribution | The cheap fix N8k was not allowed. Our samples are world points tested against the cone, where a depth-buffer march has to guess thickness. |
| Level per occluder height | Ours | Unreal samples the receiver's level along the ray. Ours keeps the texel count per band fixed, which bounds the cost for wide suns. |
| Dense levels of 2048 texels | Ours, and the limit | Unreal's 16k virtual levels keep about a texel per pixel everywhere. Ours give 1.2 to 2.3 pixels per texel for receivers far to the side of the eye. Under the walks this did not matter (the map was an index); under Disk it sets hard-edge sharpness. A sparse page pool is the fix (Next). |
| Exact geometry rescue | Not built | Nothing here needed it to reach Surface's level on the courtyard. It stays the option for contact edges if the quarry picture asks for it. |

Root's point against the options study (PR #31) holds in measurement: the walk's cost follows its direction count, and no count reaches the target. Disk's cost does not depend on it.

## Picture, inspected

RT path, Surface and Disk, same frames.

- `docs/n-vsm1/courtyard.jpg`: courtyard flight frame 24. Penumbra widths of the spheres, post and crates agree across the three.
- `docs/n-vsm1/crates.jpg`: the crates, with |RT - Disk| x 8. The shadow one crate casts on its neighbour's grazing face matches. **Open:** the lit part of that face is a little brighter than RT and Surface. Surface and Disk both band faintly on the left crate's front face where RT is smooth.
- `docs/n-vsm1/fox-cut-frames.jpg`: frames 24 to 26 and 40 round the cut, the fox walking. No pop at the cut: the frame-to-frame step after it is 4.94 to 5.32 codes for Disk, 4.91 to 5.29 for RT, 4.99 to 5.38 for Surface (M). The fox's shadow follows it.
- `docs/n-vsm1/fox-cut-far.jpg`: the long shadows 8 to 10 m from their casters after the cut. Soft, as RT and Surface. **Open:** less streak detail than RT.
- `docs/n-vsm1/s4-contact.jpg`: the sun patch at the base of S4's panel. The patch edge is straight and sharp. Disk shows the same thin leak line along the panel's base that Surface shows; RT has none. It is a gap both map lookups see, not a new one.

Review sheets, the same three columns plus |RT - Disk| x 8 (courtyard frame 24 unless said):

- `docs/n-vsm1/review-contact.jpg`: the bases of the gold sphere, the post and the crates.
- `docs/n-vsm1/review-leaves.jpg`: the bushes (thin cards) and their shadows.
- `docs/n-vsm1/review-penumbra.jpg`: the post's, arch's and crates' shadows from contact to several metres out.
- `docs/n-vsm1/review-cut.jpg`: the fox scene at frames 24, 25 (the cut), 26 and 28.

On these, Disk and Surface agree with each other more closely than either agrees with RT. Two differences from RT are shared by both non-RT lookups: the sunlit strip on the ground between the post and the crates is brighter, and the bushes' sunlit leaves are darker.

Looked for and not found on Disk: stray bright or dark single pixels, a seam where receivers change level, trails behind the fox.

**Not inspected:** a moving clip at playback speed, S1 to S3, the quarry contact crop, thin geometry apart from the courtyard's lamp post and bushes, and any comparison with Unreal. Flicker was not scored (it needs the quarry capture).

## What failed on the way (M)

| Attempt | Outcome |
|---|---|
| 8x8 sample grid with row and column masks | Banding: axis-aligned texel edges cross 8 samples at once. Replaced by the rook pattern. |
| Bias of one texel of each level | A grazing receiver's whole visible sliver of the disk lost its occluders at coarse levels. The bias is the receiver level's at every level. |
| Raising the finest level so eight levels reach the highest caster | The courtyard's ground plane made the height range huge and the finest texel 3 cm: leaks and soft contacts. Replaced by the max-mip bands. |
| Columns solid without end where the underside is unknown | Far occluders became walls beside the receiver: hard-edged long shadows (fox scene after the cut 2.0 against RT, now 1.65 to 1.73). Replaced by the half-height rule. |
| Bands sized by the widened cone (disk plus sun hysteresis) | A small sun's penumbra was resolved by one to two texels: staircase edges on S4. Bands use the frame's own disk. The courtyard's grazing crate face got slightly brighter with this change (whole frame +0.05 to +0.2 codes against RT); kept for the hard edges, recorded as open. |

Root's step 1, timing Map and Hybrid at head on the quarry, did not run: it is in the queued quarry batch.

## Cuts, moving casters, and the RT path

- **Cut.** Measured on the fox scene's `cut=1` event: Disk's cut frame is 1.61 ms, of which page requests and raster are 0.49 ms. This cut stays inside the level windows. **Not measured:** the first frame of a view never seen (process start, or a cut far enough to move every window), on either engine. The page raster on that frame is the cost to expect, since a moved window re-rasterizes its level whole (C: `prepare_virtual` re-centres a window when the eye leaves its middle two pages).
- **Skinned caster.** With the fox walking, page raster is 0.33 to 0.34 ms a frame against Surface's 0.24 to 0.25 ms. N8k's 0.53 ms plane resolve per caster frame does not exist here: Disk reads the raster's own height words.
- **RT cut (not this unit).** RT's cut frame spends 11.4 ms on 256 cold sun rays. The Disk lookup could answer that one frame on the RT path: it needs the clipmap kept warm on RT (page requests and raster each frame, about 0.5 ms here) or rendered on the cut frame itself, plus a check that the first traced frame after it does not pop against the map's answer.

## Not done

- **Quarry yardstick, every number.** Timing of Surface, Map, Hybrid and Disk with the stage split, the Epic ratios, flicker, the picture against the path-traced reference, the first frame of a brand-new view, and the Unreal capture and side-by-side. The quarry lease's 14 GiB bar was not met from 11:07 UTC on (free RAM 12.5 to 13.8 GiB), and root then held every quarry lease until a window is arranged. Nothing is queued. The whole of it is one prepared batch for one 15-minute lease, with its binaries built (`bin/m0`, `bin/d8`): `py -3 -B tools/n-vsm1/quarry.py q-y4 m0 d8`, then `py -3 -B tools/n-vsm1/quarry.py after q-y4` on the CPU. Estimated at 7 to 8 minutes from P-RELIT's quarry timings; the batch itself has not run.
- **Showcase through Ara.** Not attempted.
- **S1, S2, S3.** Not run. S2's wide-sun penumbra is the check this method most needs.
- **The default.** Unchanged. `Surface` and S4 are untouched and selectable.
- `gpu_non_rt_requested_limit_pipelines` (an ignored GPU test) fails at its view-projection reuse check with `Surface` as the last lookup, as on this branch before Disk joined its list. The Disk pipelines it creates run clean under the validation layer. Not run on main.

## Next

1. The quarry batch, then the default decision on its picture.
2. Lookup cost at 1080p (1.1 to 1.35 ms on the courtyard): the four `asin` per rectangle, and windows read for nothing on sloped receivers, are the first suspects (I). Profile before changing.
3. A sparse page pool so levels can be wider than 2048 texels, and window moves that keep the pages still inside (today a re-centred level is rasterized whole).
4. The grazing-face brightening and the far-shadow streak detail.

## For the ara-gpu backend

- **New uniform buffer:** `VdFrame`, 1,120 bytes, at the disk lookup's group 0 binding 50 (world-to-clip matrix, image size and jitter index, 65 two-word masks, 64 sample positions). No new buffer or texture kind, sampler or device feature.
- **New module** `shaders/virtual_shadow_disk.wgsl` (prefix `const VD_R:u32=3u;`), entries `vd_mark` (the `mark` layout) and `vd_lookup` (group 0: bindings 0, 7, 8, 9, 10, 15, 16, 18, 19, 35, 50; group 3: lighting).
- `VsmFrame`, `shadow_gpu::plan_frame`, `temporal_reconstruction::Planner` and the transparency `frame_block` are unchanged.
- `Renderer::encode_virtual` takes the pass log's mark callback. Under `RENDERLAB_PASS_LOG_DETAIL` the sun pass is logged as `sun_shadow.requests`, `.raster`, `.planes`, `.records`, `.lookup`, `.queued`; without it the log is as before.

## Evidence

- Hardware and build: RTX 4070 SUPER, Ryzen 7 7800X3D, Windows 11 Pro 26300, Vulkan. Release examples, `-j 2`, no validation layer for timings and images. `m0` is main `9771303` plus the pass-log split (Surface rows); Disk rows are `8c606a7`'s shader. RT pictures are the RT path of the same binaries. Machine shared with the Ara integration thread: timings are one run each, p50 over frames 30 on (benches: 8 on), not quiet-window claims.
- Tests: lib suite 219 passed, 0 failed, 170 ignored at `8c606a7` with main `6bbdfe3` merged (lease `t8`). The disk module passes naga validation in `virtual_shadow_module_validates_without_ray_queries`; `disk_pattern_is_a_rook_pattern_in_the_disk` checks the table.
- `D:/Coding/renderlab-work/n-vsm1/`: `runs/` (pass logs, `engine.json`, frames), `sheets/`, `bin/*/build.json`, `lease/`.

```powershell
py -3 -B tools/n-vsm1/host.py build NAME            # release examples into bin/NAME; lib-NAME: the lib test binary
py -3 -B tools/n-vsm1/host.py lease LABEL 14 -- py -3 -B tools/n-vsm1/steps.py OUT NAME=COMMAND ... ";;" ...
py -3 -B tools/n-vsm1/yard.py run NAME:timing:nonrt:passes+RENDERLAB_PASS_LOG_DETAIL=1+bin=NAME/p_yard_capture+RENDERLAB_DIAG=vsm-lookup=disk
py -3 -B tools/n-vsm1/yard.py timing OUT.json RUN...
game_scene --path nonrt --diag vsm-lookup=disk      # the lookup on the courtyard
```
