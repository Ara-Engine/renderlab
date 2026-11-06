# N8c: the non-RT shadow pass's cost, against Unreal's

Follow-up unit to [N8b](non-rt-n8b.md) and [N1c](non-rt-n1c.md), 2026-11-06 UTC, base `f3c6025`, branch `t3/n8c-shadow-cost`. Bar (Islam, 2026-11-06): Unreal's cost with quality at or above Unreal's on every measure. Labels: **M** measured here, **I** inference.

## Result

**Quality: above Unreal VSM on every measured quantity of S2, S3 and the quarry (traced and no-RT at 0.35 px), at the default and at a faster setting. S1 keeps N8b's single miss: 2 umbra leaks on pose 1 (fast setting: 1) against VSM's 0. Cost: every target is missed. The quarry shadow pass falls from 5.7-6.0 ms to 3.1-3.4 ms static and from 24 ms to 4.6-4.8 ms with the sun turning. The no-RT frame falls from 6.44 ms to 3.86-4.15 ms, against VSM's 1.61 ms. No setting beats Unreal on both cost and quality.**

| Quarry, shadow pass (M) | Static ms | Sun turning ms | No-RT frame at 0.35 px, ms |
|---|---:|---:|---:|
| N8b (recorded) | 5.70-5.95 | 24.0 (traced) | 6.44-6.50 |
| N8 (recorded) | 1.51 | 16-17 (traced) | 2.24-2.27 |
| **N8c default** | **3.12-3.38** | **4.56-4.83** | **3.86-4.15** (turning 5.29-5.60) |
| N8c fast (`first_block: 0`, `march_samples: 6`) | 2.35-2.57 | 3.78-4.05 | 3.07-3.35 (turning 4.50-4.83) |
| Unreal VSM (recorded) | 0.50 | about 0.95 | 1.61 |
| Target | ≤ about 0.6 | ≤ about 1.2 | ≤ 1.61 |

S2: 0.54-0.70 ms (fast 0.51-0.66) against VSM's 0.35 ms. S3: 1.18-1.20 ms static and 1.74-1.75 ms rotating (fast 0.98-0.99 and 1.53-1.54), against 0.37 and 0.59 ms. Both miss.

What changed the cost (M, traced quarry screen, 346,517 receivers, static / turning):

1. **Lever 1: pages are rasterized from cluster lists** in software (small clusters) and hardware (large ones), as 64-bit atomics, culled per level to the stale pages. Turning: 23.4 → 7.6 ms at full detail, with N8b's lookup and its counts unchanged (one receiver differs).
2. **Lever 2: the near steps are shared across the eight directions.** Every texel any direction can sample there is checked once, and each candidate's walk is tested exactly against all open directions. A max-mip cone test skips later steps; it is exactly conservative, with identical counts on and off. Static: 5.99 → 3.66 ms.
3. **The first step's 2x2 block now covers other placements only**, beyond the texel under the ray. The block existed for S1's other-mesh contacts. On the quarry, the receiver's own relief in the block cost 0.55 ms for 384 flips and 403 leaks against Interactive. Static: 3.61 → 3.06 ms.

Caster LOD, the rest of lever 1, was built and measured, but fails Unreal's lit-receiver floor (§Caster LOD). It is opt-in, off by default.

## Quality (M)

Same scorers, references and recorded Unreal rows as N8b. S2/S3 score against N7a's regenerated film references (hardlinked). S1 builds its reference in-process. The quarry uses N1c's harness: a 64-seed traced Exact reference rebuilt in-process on an RT device, the traced control and the no-RT 0.35 px visibility buffer with the 1x receiver lift, scored by `n1c-score.py` on pixels both cover. Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, validation on for quality and off for timing.

**Quarry, pose 0** (pose 1 within 2% on every column):

| Mode | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges | Film |
|---|---:|---:|---:|---:|---:|---:|
| N8b traced (N1c's control, recorded) | .1985 | 504 | 2,462 | 0 | 491 | 2.5712 |
| **N8c traced** | **.2237** | **797** | **6,002** | **0** | **779** | 2.5712 |
| N8c fast, traced | .2376 | 1,168 | 7,274 | 0 | 1,105 | 2.5710 |
| N1c + N8b, no-RT 0.35 px (recorded) | .3125 | 3,196 | 5,959 | 66 | 3,104 | 2.6001 |
| **N8c, no-RT 0.35 px** | **.3262** | **3,680** | **9,910** | **60** | **3,582** | 2.6000 |
| N8c fast, no-RT 0.35 px | .3362 | 4,359 | 11,216 | 55 | 4,189 | 2.5999 |
| Unreal VSM native | .483 | 12,390 | 14,935 | 87 | 11,884 | — |

Pose 1, no-RT: .3223 / 3,684 / 10,022 / **55** / 3,595 (default) and .3328 / 4,401 / 11,303 / **43** / 4,237 (fast), against VSM's 88 lit darkened. The lit-darkened margin stays thin (55-60 against 87-88), as in N1c. Leaks rise against N8b: the shared walks aim at the central ray, and the restricted block skips own-surface relief. All of these errors fall on the lit side: blocks remain exact hits, or N8b's continuity rule.

**S1** (static; identical across environments):

| Pose | Mode | Contact loss | Leaks | Partial RMS | Solid flips |
|---|---|---:|---:|---:|---:|
| 0 | N8b | 0/419 | 0 | .0816 | 0 |
| 0 | **N8c** | **0/419** | **0** | **.0822** | **0** |
| 0 | N8c fast | 0/419 | 0 | .0803 | 0 |
| 0 | UE VSM (black / env) | 5 / 6 | 0 / 0 | .1642 / .1644 | 0 / 1 |
| 1 | N8b | 0/335 | 2 | .0802 | 0 |
| 1 | **N8c** | **0/335** | 2 | **.0809** | **0** |
| 1 | N8c fast | 0/335 | 1 | .0784 | 0 |
| 1 | UE VSM (black / env) | 2 / 3 | **0** / 0 | .1628 / .1620 | 0 / 0 |

S1 timings stay unusable, as in N8 (2.9-9.5 ms whole-frame medians from a few frames).

**S2, disk 0.04 rad** (N8b → N8c default → fast; UE VSM recorded):

| Case / phase | Partial RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / lost / leaks / flips / shadow ms) |
|---|---|---|---|---|---|---|
| camera, out | .0746 → **.0746** → .0753 | 0 → 0 → 0 | 0 → 0 → 1 | 0 → 0 → 0 | 0.86 → 0.70 → 0.66 | .1752 / 2,101 / 62 / 167 / **0.35** |
| camera, hold | .0750 → **.0750** → .0755 | 0 → 0 → 0 | 0 → 0 → 0 | 0 → 0 → 0 | 0.79 → 0.56 → 0.52 | .1758 / 1,039 / 57 / 152 / **0.35** |
| camera, return | .0748 → **.0748** → .0754 | 0 → 0 → 0 | 0 → 0 → 1 | 0 → 0 → 0 | 0.85 → 0.69 → 0.66 | .1739 / 1,972 / 62 / 155 / **0.35** |
| shadow step, out | .0735 → **.0735** → .0739 | 0 → 0 → 0 | 0 → 0 → 0 | 0 → 0 → 0 | 0.66 → 0.69 → 0.66 | .1697 / 2,070 / 65 / 120 / **0.36** |
| shadow step, hold | .0727 → **.0727** → .0732 | 0 → 0 → 0 | 0 → 0 → 0 | 0 → 0 → 0 | 0.59 → 0.54 → 0.51 | .1675 / 1,059 / 66 / 119 / **0.35** |

HDR RMS equals N8b's in every phase (.0105, .0112, .0101, .0099, .0099).

**S3, 525 k triangles, 15° sun:**

| Phase | Partial RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / lost / leaks / flips / shadow ms) |
|---|---|---|---|---|---|---|
| static | .0778 → **.0778** → .0781 | 0 → 0 → 0 | 0 → 2 → 4 | 0 → 0 → 0 | 1.85 → 1.20 → 0.98 | .1878 / 2,929 / 111 / 187 / **0.37** |
| rotate 0.25°/s | .0783 → **.0783** → .0782 | 0 → 0 → 0 | 0 → 9 → 7 | 0 → 0 → 0 | 1.95 → 1.75 → 1.54 | .1875 / 11,630 / 111 / 190 / **0.59** |
| rotate 1°/s | .0782 → **.0782** → .0782 | 0 → 0 → 13 | 0 → 9 → 6 | 0 → 0 → 0 | 1.92 → 1.74 → 1.53 | .1863 / 12,289 / 106 / 192 / **0.59** |
| jump recovery | .0762 → **.0762** → .0767 | 0 → 0 → 0 | 0 → 5 → 1 | 0 → 0 → 0 | 1.88 → 1.18 → 0.99 | .1811 / 3,153 / 114 / 226 / **0.37** |

HDR RMS equals N8b's (.0142/.0141/.0141/.0138).

## Caster LOD: built, measured, rejected as a default (M)

`VirtualShadowSettings::caster_error_texels` rasterizes each level from the clusterLOD cut whose simplification error is within that many of the level's texels. The cut depends on the clipmap alone, so camera motion keeps pages valid; N1c's cut instead depends on the camera. No-RT quarry at 0.35 px, lift 1 unless noted, pose 0:

| Casters | Partial RMS | Flips | Leaks | Lit darkened | Lost edges | Shadow static / turning ms |
|---|---:|---:|---:|---:|---:|---|
| Full detail (default) | .3262 | 3,680 | 9,910 | **60** | 3,582 | 3.12-3.41 / 4.54-4.89 |
| 0.25 texel | .3314 | 4,121 | 11,446 | **152** | 3,727 | 2.96-3.25 / 3.82-4.18 |
| 0.5 texel | .3231 | 5,815 | 10,631 | **2,437** | 3,251 | 3.03-3.06 / 3.44-3.76 |
| 0.25 texel, no lift | .2671 | 4,336 | 7,968 | **3,105** | 1,137 | — |
| 0.25 texel, origins moved out by the cut error | .4836 | 9,095 | 17,559 | 66 | 8,783 | — |

Coarse casters do not remove the need for N1c's receiver lift; they add false shadows to it. A coarse caster surface lies up to its error above source receivers, which the lift (1x the receiver's own error, zero on source triangles) does not cover. Moving every origin out by the cut error, a normal offset, removes the false shadows but loses umbra receivers beyond VSM's level. The saving is at most 1.1 ms of the turning frame. Casters therefore default to original geometry (`caster_error_texels: -1`), and the setting documents the miss. A cut made consistent with the camera's (same clusters at the receiver) would need cut-change page invalidation on camera motion (I, not built).

## What was built (M)

`shaders/virtual_shadow.wgsl`, `src/virtual_shadow_gpu.rs`, plus small hooks in `src/visibility_buffer.rs` (shared `ClusterInstances`, `cluster_data_group`), `src/primary_ray.rs` (limits) and test harnesses.

- **Page raster from clusters.** The draw list's visibility-buffer clusters (one record per cluster per placement, N1c's layout) are culled per level: by the cut, then by the light-space box against the window and the pages flagged stale this frame. Clusters of at most 32 texels go to a software raster: one workgroup per cluster, vertices projected once, 8-bit subpixel snapping, exact integer edge functions, inclusive edges. Larger clusters go to a hardware draw whose fragments write the same atomics. Both write `max(normalized height << 32 | id)` per texel centre into a top layer, then the peel layer: the highest surface whose id differs from the texel's top id. Stale pages are zeroed by a compute clear, and writes outside flagged pages are dropped. This replaces the stencil-masked whole-mesh draws, the Depth24/R32 map textures and the indirect templates.
- **Max-mips** of the top layer per level (mips 1..log2 side, 5.6 MB per level at 16 pages), rebuilt on page-work frames.
- **Lookup.** Steps run outer and directions inner, on N8b's geometric schedule in heights. While the cone's footprint stays within half a texel (`VSM_SHARE`), a step is shared. Every texel any direction's sample (or the first step's block) can touch is checked once. A candidate's walk tests every visited triangle against all open directions. A walk ending with the central ray exactly below an interior triangle hands each open direction its own refinement from the located triangle, with N8b's continuity rule. Later steps test the max-mip over the footprint first, then sample per direction as N8b did. Every block is still an exact hit or N8b's continuity rule. `vsm_top_hits` and `vsm_texel_hits` each have one call site.
- **`first_block`** (setting, default 2): the first step's 2x2 block for other placements only, beyond the texel under the ray. 1 is N8b's; 0 is off.
- **Ring horizons** now cover appended LOD triangles. N8b's buffer held source triangles only, so a LOD receiver read another triangle's horizon (a lit-ward error).
- **Requirements.** `VIRTUAL_FEATURES` is now `SHADER_INT64 | SHADER_INT64_ATOMIC_MIN_MAX`; primitive index and indirect first instance are no longer needed. Virtual casts from cluster data, so meshes must be uploaded on a device `visibility_buffer::supported` accepts. `visibility_limits` raises storage buffers per stage to 10, for the lookup's two map layers and max-mips. Memory: 2 x 8 B per texel per level (N8b used 16 B too), max-mips, 64 B cluster records plus raster lists (14 MB + 14 MB on the quarry), 4 B horizons per addressed triangle (quarry with LOD: 104 MB).
- **Programmability.** No lighting-model data entered hit records, clusters or the maps: they hold geometry heights and triangle ids, and the pass still outputs visibility counts that the user's shading programs consume.

## Where the cost goes (M), and what was tried

Decomposition on the traced quarry, before the block change: dispatch alone 0.03 ms; receiver setup (record, eight directions, own triangle) 0.20 ms; plus own-triangle test, horizon and ring walk 0.82 ms; the far march about 2.7 ms, of which the first block was 0.85 ms. A `VSM_DEBUG` trace (pose 0, same build: N8b's full first block) classified the receivers: 89% lit, 4% partial, 6.5% umbra. Per receiver there were on average 4.9 shared walk visits, 1.7 per-direction visits and 3.5 ring triangles. Lit receivers walked the ring 16% of the time; the horizon skipped the rest. Partial receivers cost about 55 visits each and umbra about 24. The pass is bound by dependent loads per visit and per march step, not by the count of exact tests (I).

Tried and rejected (M):

- **Compacted passes**: a classification pass queuing only receivers that need the ring or the march, each then run densely. Counts identical, 4.10-4.17 ms against 3.61-3.73 ms: divergence was not the limiter. Removed.
- **Testing every open direction in per-direction walks** (not only the sample's own): +0.10 ms for 3% fewer leaks against Interactive (6,058 against 6,270). Kept for shared walks only; a candidate for the default if leaks matter more than 0.1 ms.
- **Constant-index unrolling** of the direction tests and select-based component access: 18.8 ms. Code size and register blow-up. Reverted.
- **Skipping own-surface near candidates** for receivers above their ring horizon: −0.12 ms for 1,239 more leaks against Interactive. Removed.
- Cheaper walks (`VSM_WALK_STEPS` 1-4, `VSM_REFINE` 0-1) and fewer samples reach 2.0-2.2 ms at about twice the leaks, near N8's. Not scored as settings.

Not done: lever 3. Clipmap panning by page offset and a static/dynamic split change nothing on these benches: the quarry has no moving casters, and camera-alternating poses rarely move a window (N8b). A wave-level early-out needs subgroup operations the device does not request. Its all-blocked form would trade penumbra accuracy for skipped umbra walks (I).

Next levers (I): per-azimuth ring horizons (most of the 0.6 ms ring cost is receivers whose steepest neighbour faces away from the sun); a camera-consistent caster cut with cut-change invalidation; a subgroup all-blocked early-out after two directions.

## Misses, honestly

- **Every cost target.** Quarry static 3.1-3.4 ms (target ≤ 0.6, VSM 0.50), turning 4.6-4.8 ms (≤ 1.2, VSM about 0.95), no-RT frame 3.9-4.2 ms (≤ 1.61). S2 1.5-2x and S3 about 3x VSM. The fast setting is about 0.7 ms cheaper on the quarry and still misses.
- **S1 pose 1: 2 umbra leaks** (fast: 1) against VSM's 0, N8b's unchanged miss.
- **Quarry quality moved away from N8b**: leaks 2.4x (traced) and 1.7x (no-RT) N8b's, though still below VSM's. The lit-darkened margin over VSM is thin (55-60 against 87-88).
- **Caster LOD** fails the lit-receiver floor (above).
- Not film quality; never presented as meeting the film gate.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8c/`, 128 MB): `final/` and `final-fast/` (S1 `comparison.csv`, `scores.json`; S2/S3 `comparison.json/csv`, `n8-comparison.json`, `n8-score.log`; fixture), `final-q*/quarry/summary.json` and PNGs (quarry quality), `final-t*/quarry-no-rt/timing.json` (no-RT timing), `q1`-`q3` (caster LOD), `r1`-`r14/screen` (variant screens), `r8` (trace classification by `scripts/n8c-classify.py`), `runs/` (every lease's log, preflight and memory samples). All raw arrays (4,596 files, about 11 GB) were deleted after scoring and are listed in `deleted-raw.txt`; N7a's hardlinked references are untouched.
- **Build:** the scored binary (`final/binary.sha256`) also contained the compacted-pass and own-skip code, both disabled. Removing them changed no counts: the cleaned build reproduced the fixture and the quarry screen exactly (r14).
- **Gate:** `cargo test -p renderlab --lib -j 2` passed by exit status 0: 105 passed, 0 failed, 107 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`; the last for `visibility_buffer.rs`'s layout, unchanged from main) and fmt pass. The fixture's cache asserts pass. Agreement with RT is 99.93% at the 40° sun (0 flips) and 99.07% at 15° (82 flips; N8b 99.3% and 10). The rise is the restricted first block on the fixture's grazing own-surface bumps.
- **Contention:** `mypaper-wallpaper` ran throughout. Three static no-RT visits read high and are excluded from the clean ranges above: 4.97 ms (default, final run) and 4.43 and 5.44 ms (caster-LOD run). Each was one of four visits; turning visits were clean. No game was running (Riot launcher only), and no window, focus or capture was used.
- **Leases:** 20 under the shared lock with the 4/2 GiB guard, 1,027 s in total; the three that failed (101) did so at pipeline creation and were rerun.
- **Time (UTC):** 12:06-12:28 reading and lever 1 implementation; 12:28-12:35 its first screens; 12:35-13:05 lever 2, the trace and the rejected variants; 13:05-13:16 first block and caster LOD; 13:16-13:23 final batch; 13:23-13:45 scoring, cleanup, gate and this record. GPU verification took 17 minutes and CPU scoring about 8; implementation took the rest, about 75 minutes, interleaved.
