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

## Part 2 (2026-11-11, after the opt-in merge at main `eeb34ae`)

Brief: make `Disk` fit to be the non-RT default. **It is not yet. Recommendation: keep `Surface` the default.** Three fixes landed (`2035f19`); the S1 and S2 penumbra error, the cost and three checks remain open. The quarry stayed closed throughout.

### Which side is right where both non-RT lookups differ from RT (M)

**RT is right; the map lets light through.** The RT path with reference-quality shadows (`--diag quality=reference`) agrees with RT Interactive to 0.09 codes, and the difference survives `--no-glass`, so it is neither sampling nor the glass pane.

- **The diagonal band on the ground** is a thin lit line inside the crates' shadow: +4 to +5 codes on Disk, +8 on Surface, 0 on RT.
- **Cause: the map has two layers.** Where one closed crate stands in front of another along the light, a texel holds the front crate's entry and exit and nothing of the one behind. A ray passing under the front crate's exit and through the hidden crate meets no column. Evidence: with every underside treated as unknown (`VD_PEEL=false`) the line disappears (107 codes, RT 107) and the crate-face error falls from 6.63 to 2.67, but the whole frame gets worse (1.59 against 1.36), because every column is then too thick for its penumbra. Solid-without-end columns (`VD_UNDERSIDE=0`, since removed), a smaller bias and no screen samples change nothing.
- **Not fixed.** A least thickness of 16 texels closes a little of it (face 6.19). The real fix is a third layer or an underside widened at page-raster time from neighbouring texels. `Surface` has the same leak, larger on the ground line.

### Three fixes (M)

| Fix | What was wrong | Effect |
|---|---|---|
| Thin receivers use their light-facing tangent plane | A tilted thin sheet's own texels counted as occluders: whole tiles rendered black on S2 | S1 solid flips 1,318 / 976 to 0; S2 about 1,150 to 0-2 |
| Sheet undersides from same-placement neighbours | A sheet over other geometry was taken down to half its height: wires and bars cast shadows as if twice as close | S1 partial RMS .172 to .142; S2 .17-.29 to .13-.23 |
| A least column thickness of 4 texels | Columns of a thin or oblique closed surface left slits between them | Courtyard without glass 1.36 to 1.34 (measured alone); at 1 texel S2's camera-track leaks rise from 87-92 to 116-117 |

All three together: S2 partial RMS .093-.098, and the fox scene after the cut 1.65-1.73 to 1.38-1.51 against RT.

### S1 to S3 against the film references (M)

`scripts/n8-batch.py` score steps, the unchanged scorers, no TR1 (so no receiver jitter), one repeat. Unreal VSM rows are the recorded ones ([non-rt-n8k.md](non-rt-n8k.md)).

| Bench | Measure | Unreal VSM | Surface | **Disk** |
|---|---|---|---|---|
| S1 contact fold, poses 0 / 1 | Partial RMS | .164 / .163 | .082 / .080 | **.137 / .136** |
| | Solid flips, leaks, contacts lost | 0-1, 0, 5-6 / 2-3 | 0, 0, 0 | 0, 0, 0 |
| S2 wide sun, six phases | Partial RMS | .168-.176 | .073-.075 | **.093-.098** |
| | Flips | 119-167 | 0 | 0-2 |
| | Leaks | 57-67 | 0 | 0 on the shadow track, 8 to 92 on the camera track |
| | Contacts lost | 1,039-2,101 | 0 | 0-14 |
| S3 relief and pillars, six phases | Partial RMS | .181-.188 | .079-.083 | **.078-.087** |
| | Flips | 187-226 | 0 | 190-370 |
| | Leaks | 103-114 | 2-9 | 0 |
| | Contacts lost | 2,929-12,289 | 0 | 0-60 |

- **Disk is above Unreal VSM on every measure except S3's flips, where it is level or worse** (370 against 226 on the jump recovery).
- **Disk is behind Surface on S1 and S2 partial RMS.** `docs/n-vsm1/review-s2-penumbra.jpg` shows why: Surface's error there is unbiased sampling noise (which TR1 integrates in the engine); Disk has no noise but its penumbrae of raised thin casters are darker on the outer side. That is a bias and TR1 will not remove it. Cause not found.
- **S3's flips were not located.**

### Motion (M, with what was and was not watched)

Courtyard flight, 720p, 40 consecutive frames, each path's frame-to-frame step minus the RT path's step (RMS, display codes; lower is steadier):

| | Whole frame, mean (max) | Leaves crop, mean (max) |
|---|---|---|
| Surface | 0.780 (0.916) | 0.880 (1.110) |
| Disk | 0.719 (0.952) | 0.788 (1.021) |

Flipbooks are built: `D:/Coding/renderlab-work/n-vsm1/sheets/flight-surface-disk.gif` and `leaves-rt-surface-disk.gif`. **I scored them and looked at single frames (`docs/n-vsm1/review-leaves-motion.jpg`); I did not watch them play.** The numbers say Disk is a little steadier than Surface on average; they cannot show a crawl or a one-frame pop. Root should play both before any default change.

### Cost (M)

Fox scene after the cut, 1080p, `RENDERLAB_VD` stage splits on one binary:

| | Lookup | Raster | Requests |
|---|---:|---:|---:|
| Disk, part 1 (`8c606a7`) | 1.17 ms | 0.33 | 0.17 |
| Disk now (`2035f19`) | 1.19 ms | 0.33 | 0.17 |
| No map windows (facing and screen samples only) | 0.17 ms | | |
| No screen samples (part 1's shader, the flight without the cut) | 1.51 of 1.54 ms | | |
| Dispatch alone | 0.04 ms | | |

- **The map windows are about 1.0 ms of the 1.2 ms lookup.** Screen samples are about 0.03 ms and setup with the facing test under 0.15 ms.
- **Tried, not kept:** a whole-disk shortcut before the four `asin` (1.19 to 1.25 ms: no gain). A window radius of 2 texels instead of 3 (1.25 to 0.82 ms, minus 34 percent) makes the courtyard worse against RT on 5 of 7 frames (2.61 against 2.32 at worst), so it is not taken.
- **The 0.6 ms target is not met** (the pass is 1.7 ms on this scene; S5 is 0.47 ms, S4 1.46 ms). The texel reads per window are the cost; the next lever is fewer of them (a min-max test per window row, or a coarser first pass), not the mask arithmetic.

### Not done in part 2

- **Showcase through Ara:** not attempted.
- **S4 and S5 against RT with the new shader:** not rescored (their RT and Surface captures were deleted with the raw dumps). Part 1's gap stands unexplained.
- **Hard-edge sharpness far to the side of the eye** (a sparse page pool), **the lost streak detail in long shadows**, **the S4 base line:** untouched.
- Lib suite at `2035f19`: 219 passed, 0 failed, 170 ignored (lease `t13`).
- **Quarry:** closed by root. `bin/m0` and `bin/d13` (the code at `2035f19`) are built: `py -3 -B tools/n-vsm1/quarry.py q-y4 m0 d13`.

### Would I make Disk the default now? No.

What is still worse than `Surface`:

1. Penumbra error on S1 and S2 (partial RMS .137 against .082, .093-.098 against .073-.075), a bias, not noise.
2. S3 solid flips (190-370 against 0) and S2 camera-track leaks (up to 92 against 0).
3. Hard edges where a texel spans several pixels (dense 2048-texel levels).
4. Nothing is known on the quarry, against Unreal, or on Showcase.

What is better: the cut frame (159 ms to 1.6 ms on the fox scene), every frame's cost (23.6 to 1.7 ms after the cut), no sampling noise, a slightly steadier flight, S3 leaks, and the fox scene's picture against RT after the cut (1.38-1.51 against 1.54-1.75).

## Part 3 (2026-11-11, after the part 2 merge at main `934d8e9`): one screen, nothing landed

Brief: the two-layer leak at page raster, the penumbra bias and S3 flips, cost by skipping, S4/S5 rescored, Showcase. **None of the five is done.** This section records the one experiment that ran and what the next session should start from. The code is `2035f19`'s; the default recommendation is unchanged (keep `Surface`).

### The rectangle on S2 is not the least column thickness (M)

Root saw a clean one-signed rectangle in Disk's S2 error (`review-s2-penumbra.jpg`) and asked for a page, level or window boundary to be ruled out first. My first suspect was different: a sheet hovering 2 to 5 texels over the floor (S2's tiles) with a 4-texel least thickness becomes a block standing on the floor. Screened on one test binary through `RENDERLAB_VD`, S1 and S2, unchanged scorers:

| Variant | S1 partial RMS | S2 partial RMS | S2 camera-track leaks |
|---|---|---|---|
| `2035f19` (4 texels) | .137 / .136 | .093-.098 | 87, 8, 92 |
| Thickness at most a quarter of the height above the receiver | .134 / .133 | .093-.098 | 88, 8, 92 |
| The same, with the neighbour rule for closed columns too | .134 / .133 | .093-.098 | 88, 8, 92 |
| 0.25 texel, neighbour rule for all | .133 / .132 | .092-.097 | 88, 8, 93 |
| Surface | .082 / .080 | .073-.075 | 0 |

**Rejected: thickness moves the error by 2 to 3 percent.** The variants were not kept. Root's own suspects (a page, level or window boundary under that rectangle) are **not yet tested**.

### Where to start next

1. **The rectangle.** Dump Disk's count beside the film's for S2 `shadow-step` frame 16 and overlay the receiver level (`vsm_level_in`) and the page grid. If the rectangle follows a page or level edge, the request radius in `vd_mark` or the level hand-over in `vd_map` is the place; if it follows a tile, it is the near field (the one-texel bias against sheets hovering 0.2 to 5 mm, and the six-pixel screen samples).
2. **S1 and S2 may be the map's resolution, not the integration (I).** Their partial pixels are the one-to-two-pixel transitions of contact shadows, where a map-only answer is a texel off and an exact walk is not. If so, the cheap answer is an exact test only for receivers whose mask is neither empty nor full and whose lowest occluder is within a few texels (Hybrid's queue, sized by this test), and its cost is what has to be measured.
3. **Two-layer leak.** Widening undersides cannot serve both lookups: `Surface` needs the hidden caster's triangle as a candidate, so it needs a real third layer (8 bytes per texel per level: 268 MiB more at eight levels of 2048 texels, on top of 536 MiB for two; one more raster pass over stale pages). A widened underside (4 bytes per texel, 134 MiB) would fix Disk alone. Neither is built or measured.
4. **Cost.** Untouched since part 2: the map windows are 1.0 ms of the 1.19 ms lookup.

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

## Quarry window (root, 2026-11-11 13:56-14:00 UTC)

The held quarry batch ran once in an owner-granted RAM window (`tools/n-vsm1/quarry.py q-y5 m0 d13`: main's default against part 2's `Disk`, RTX 4070 SUPER, 1080p, non-RT, one lease of 206 s, no watchdog trip). Unreal is software Lumen at Epic from the same yardstick.

**Frame and sun pass, two visits each (GPU p50):**

| State | Default (`Surface`, S4 on cuts) | `Disk` | Unreal |
|---|---|---|---|
| Still: frame | 8.61-8.68 ms (2.15x slower) | 4.30-4.33 ms (1.07-1.08x slower) | 4.02 ms |
| Moving: frame | 10.81-10.84 ms (2.6x slower) | 4.50-4.53 ms (1.09-1.10x slower) | 4.12 ms |
| Cut: frame | 45.5-46.8 ms (11x slower) | 4.76-4.93 ms (1.16-1.20x slower) | 4.11 ms |
| Still: sun pass | 4.51-4.52 ms | 0.236 ms (lookup 0.142, raster 0.033, requests 0.055) | about 0.6 ms |
| Moving: sun pass | 6.55-6.56 ms | 0.232-0.233 ms | about 0.6 ms |
| Cut: sun pass | 41.1-42.0 ms | 0.378-0.381 ms (raster 0.16) | about 0.6 ms |

- The older lookups, one visit: `Map` 0.79 / 0.77 / 1.18 ms (frame 4.89 / 5.02 / 5.60); `Hybrid` 3.52 / 3.37 / 4.08 ms (its queued walk is 2.6-2.9 ms).
- **The 0.6 ms target is met on the quarry in every state, the cut included.** On the game scene with the fox the pass is still 1.7 ms.

**Picture.** On the quarry `Disk` and `Surface` are nearly the same image: of covered pixels, 0.10-0.39% differ by more than half a code and none by more than 3.3 codes, on the still, walking, cut and cut+1 frames. Scores against the path-traced reference: error 7.289 / 7.641 / 8.263 (still / moving / cut) against the default's 7.278 / 7.633 / 8.257; contact crop 10.95 against 10.93; flicker 4.375 on both. The quarry therefore does not exercise the cases where `Disk` is behind (S1-S3 above). Sheet: [quarry-window-rock.jpg](n-vsm1/quarry-window-rock.jpg), Unreal, `Surface`, `Disk` and their difference x16. Inspected: no stray bright pixels in either Renderlab column; the cut frame is sharp with shadows in place; Unreal's cut and cut+1 frames are visibly softer.

**First frame of a brand-new view (no page exists yet).** Renderlab's first frame rasterises the whole visible clipmap: page raster 56-57 ms with `Disk` (43 ms plus 275 ms of walk records with `Surface`). Unreal's first frame spends 1.9 ms in shadow depths and 0.14 ms marking pages (its whole first frame is 18 ms). A cut inside an area already seen costs Renderlab 0.16 ms of raster. So the remaining cut hitch is a cut to somewhere never seen: OPEN, for the page-structure unit.

## N-VSM2 (2026-11-11, branch `t3/n-vsm2` from main `c341861`)

Brief: find and fix the bias and the flips, then cut the map windows by skipping. Written as the work goes; the result table is at the end of this section once the fixes are scored.

### What the errors were (M)

Baseline at `c341861`, the unchanged scorers, one repeat (lease `n1`): S1 partial RMS .137 / .136; S2 .093-.098 with 87, 8 and 92 camera-track leaks; S3 .078-.087 with 190-370 flips. `Surface` in the same lease: .082 / .081, .073-.075 and .079-.083, no flips, S3 leaks 2-9.

A new CPU check, `tools/n-vsm1/exact.py`, traces S2's 32 triangles from every crop pixel's receiver toward Disk's own 64 sun samples (`exact64`) and toward 1,024 stratified ones. S2 `shadow-step` frame 16, partial pixels (8,338, of which 8,300 are penumbrae wider than 3 pixels):

| Against the film | Partial RMS |
|---|---|
| Exact, 1,024 samples at the pixel centre | .018 |
| Exact, Disk's 64 samples | .020 |
| `Surface` | .074 |
| `Disk` | .089 |

- **The 64-sample pattern is not the problem.** It is within .010 of the 1,024-sample answer with no mean error, and a noise-free centre-point lookup could score .020 where `Surface`'s noise scores .074.
- **Disk's error is the map: 5.2 percent too dark on average** (Disk minus exact64: mean -.052, RMS .091). The previous session's guess that S2's partial pixels are one-to-two-pixel contact transitions is wrong: 99.5 percent of them are wide penumbrae.
- **The darkening is not at a page or level edge.** Every partial receiver is on level 1 and the error follows the shadows, not the page grid (level and page overlay dumped with `tools/n-vsm1/variant.py`).
- **It is the max-mip bands.** S2 has three levels (texels 0.98, 1.95 and 3.9 mm), so real texels serve occluders only up to 0.29 m above a receiver along the light. The blocker (0.38-0.5 m) and the plate's top edge (0.94 m) are served by the bands above, where a cell is its highest texel: any occupied texel fills the cell, so every occluder grows by up to a cell, and a cell is a third to two thirds of the disk's radius. By the mean height of the occluders a receiver sees:

| Occluder height above the receiver | Served by | Partial pixels | Disk minus exact64, mean | RMS |
|---|---|---:|---:|---:|
| 0 - 0.07 m | level 0 | 253 | -.008 | .121 |
| 0.07 - 0.15 m | level 1 | 232 | -.006 | .094 |
| 0.15 - 0.29 m | level 2 | 377 | -.029 | .089 |
| 0.29 - 0.59 m | max-mip band 1 | 2,507 | -.039 | .080 |
| 0.59 - 1.17 m | max-mip band 2 | 3,658 | -.081 | .102 |

  The rectangle root saw is the blocker's shadow drawn from whole cells.
- **A second error has no sign: about .09 RMS in the real levels too.** A band's texel is a third to two thirds of the disk's radius and the raster samples texel centres, so an occluder edge sits up to half a texel from the truth. That is a blotchy, still error of the same size as `Surface`'s noise. Removing the max-mip bias alone therefore cannot bring S2 below `Surface` without TR1.
- **S3's flips are the near-field screen samples.** All 242 on frame 0 are lit relief pixels with count 0. The map alone leaves 240 of them lit; the screen samples alone turn all 242 black. A sample was a column from its own point down to the receiver, so a neighbouring pixel within half a footprint of the light's axis hid the whole disk: any neighbour within about 30 degrees of the sun, where the truth needs 2.3.
- **S2's camera-track leaks are a second occluder the lookup ignored.** They are umbra pixels returning 7 to 14 of 64 samples lit (frame 27: 87 pixels on the tiles; frames 31 on: 8-10 floor pixels), where the plate's and the blocker's shadows overlap. The top layer holds the plate; the blocker under it is in the peel layer, which the lookup read only as the top surface's underside. This is not the two-layer leak of a closed caster behind another (that one is absent from both layers); here the data is in the map.
||||||| c341861

### Step 1: the fixes (M)

Two changes to the lookup and one removal, all in `shaders/virtual_shadow_disk.wgsl` (`89c58f1`, `e88433cc`). `Disk` stays opt-in in the code; the recommendation is at the end.

1. **The bands above the coarsest level read one texel per max-mip cell** (`VD_MIP_POINT`). The cell is the texel at its middle, with that texel's own underside, as a coarser level rasterized at its centres would hold. The max-mip is still read first, as the test that the cell holds anything in the band. This removes the widening: S1 partial RMS .137 to .080, S2 .093-.098 to .062-.067.
2. **The peel layer is a second occluder where it is another placement's surface** (`VD_PEEL2`): a second sheet under the first, or the solid a sheet lies over. S2 camera-track leaks 87 / 8 / 92 to 1 / 1 / 1.
3. **The visible-surface samples are removed.** See "The near field" below: every rule tried cost more than it gave.

| Bench | Measure | Unreal VSM | `Surface` | Disk before | **Disk after** |
|---|---|---|---|---|---|
| S1 contact fold, poses 0 / 1 | Partial RMS | .164 / .163 | .082 / .081 | .137 / .136 | **.080 / .080** |
| | Flips, leaks, contacts lost | 0-1, 0, 5-6 / 2-3 | 0, 0, 0 | 0, 0, 0 | 0, 0, 1 |
| S2 wide sun, six phases | Partial RMS | .168-.176 | .073-.075 | .093-.098 | **.062-.067** |
| | Flips | 119-167 | 0 | 0-2 | 0-1 |
| | Leaks, camera track | 57-67 | 0 | 87, 8, 92 | **1, 1, 1** |
| | Leaks, shadow track | 57-67 | 0 | 0 | 0 |
| | Contacts lost | 1,039-2,101 | 0 | 0-14 | 32-92 |
| S3 relief and pillars, six phases | Partial RMS | .181-.188 | .079-.083 | .078-.087 | **.063-.074** |
| | Flips | 187-226 | 0 | 190-370 | **2-7** |
| | Leaks | 103-114 | 2-9 | 0 | 0-1 |
| | Contacts lost | 2,929-12,289 | 0 | 0-60 | 32-158 |

Same scorers and references as part 2, one repeat, no TR1 (leases `n1`, `n11`). Unreal rows are the recorded ones ([non-rt-n8k.md](non-rt-n8k.md)).

- **Against the coordinator's bar** (S2 camera-track leaks and S3 flips at Unreal's level or better, the S1/S2 bias explained): **met.** Leaks 1 against 57-67, flips 2-7 against 187-226, and the bias is explained and gone.
- **Against `Surface`:** partial RMS is now lower on all three benches. Flips are 0-7 against 0, S2 leaks 1 against 0, S3 leaks 0-1 against 2-9. **Contacts lost is the one measure that got worse**: 32-158 per phase, which is 1 to 3 pixels a frame of about 300 (S2) and 1,500 (S3) contact pixels, against 0 for `Surface` and thousands for Unreal. It is the price of removing the screen samples.
- **What is left of the penumbra error is a still pattern of both signs** (`docs/n-vsm1/n-vsm2-s2-error.jpg`): a texel is a third to two thirds of the disk's radius, so an edge sits up to half a texel off. TR1 cannot average it, because it does not change from frame to frame. Reading a finer level with a stride and a per-frame offset would turn it into noise TR1 integrates; that is not built.
- **Thin casters high above a receiver are now hit or miss** at the size of a cell (S1's strips, S2's wires): before they were always hit and a cell wide. `Surface` at eight directions dots them too (`review-s1-contact.jpg`).

Sheets: `docs/n-vsm1/n-vsm2-s2-error.jpg` (S2 `shadow-step` frame 16, visibility and signed error against the film), `n-vsm2-s3-flips.jpg` (S3 frame 0, the flipped pixels marked), `n-vsm2-courtyard.jpg` (flight frame 24 against the RT path). `tools/n-vsm1/review2.py` builds them.

### The near field (M)

Within one receiver-level texel of the tangent plane the map is not used, and N-VSM1 read up to six pixels of the visible surface there. Four rules were scored with the two map fixes in place:

| Rule for a visible-surface sample | S3 flips | S2 contacts lost | Courtyard against RT, frame 24 | Fox scene against RT, 9 frames |
|---|---|---|---|---|
| A column from the sample down to the receiver (N-VSM1) | 190-370 | 3-14 | 1.07 | not run |
| Range along the march axis between joined samples | 2-7 | 0-10 | **11.3** (ground far to the side goes dark) | 18.0 |
| Horizon over the tangent plane from the receiver up | 198-226 | 32-80 | not run | not run |
| Range of elevations over the tangent plane | 73-92 | 32-80 | 1.07 | 1.38 |
| **No samples** | **2-7** | 32-92 | 1.07 | 1.43 |

- The cause of the first row's flips: a pixel beside the receiver is far wider than the sun's disk (half a footprint at one footprint's distance is half a radian; the disk is 0.04), so any model that gives the sample its pixel's width hides the whole disk or none of it.
- The second row is right on relief and wrong on flat ground seen obliquely: pixel snapping moves a ground sample along the ground's horizon, which is not along the march axis there. It darkened a third of the courtyard and must not come back in that form.
- The last two rows differ by 0.05 codes on the fox scene (the bushes' leaves) and by 70 to 90 flips on S3. With the coordinator's direction (keep the map fixes, drop the screen rule if it cannot hold the courtyard), **the samples are removed**. `VdFrame::clip` is no longer read; the uniform's layout is unchanged.
- **Open:** contact within one texel of height. If the quarry's contact crop asks for it, the place to start is the fourth row's rule (block the directions between the least and the greatest elevation over the tangent plane among the joined samples after the first; it is not kept in the tree), restricted to smooth receivers.

### Courtyard and fox scene against the RT path (M)

Whole-frame RMS in display codes, 720p, the same frames as part 1 (leases `n5`, `n11`):

| Frames | `Surface` | Disk before | **Disk after** |
|---|---|---|---|
| Courtyard flight, 7 frames | 1.19-2.51, mean 1.57 | 1.25-2.32, mean 1.49 | **1.07-2.30, mean 1.35** (lower than before on all 7) |
| Courtyard, contact crop (sphere, post, crates), mean | 3.25 | 3.99 | **3.09** |
| Fox scene round the cut, 9 frames | 1.41-1.86, mean 1.65 | 1.23-1.51, mean 1.43 | 1.22-1.55, mean 1.43 |

The lit line on the ground between the post and the crates and the bright crate face are still there (`n-vsm2-courtyard.jpg`): that is the two-layer leak, which is the page-pool unit's.
