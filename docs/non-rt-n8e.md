# N8e: spending the non-RT shadow pass's quality surplus on cost

Follow-up unit to [N8d](non-rt-n8d.md) and [N8c](non-rt-n8c.md), 2026-11-06 UTC, base `bb1e8df`, branch `t3/n8e-shadow-surplus`. Brief (Islam, 2026-11-06): bring the non-RT shadow pass to Unreal's cost by spending some of N8c's quality surplus over Unreal, staying at or above Unreal on every measure of S1, S2, S3 and the quarry, traced and no-RT. Targets: quarry static ≤ about 0.6 ms, sun turning ≤ about 1.2 ms, no-RT frame ≤ 1.61 ms. Labels: **M** measured here, **I** inference.

## Result

**New non-RT default: `walk_cap: 16, refresh: 16, sun_hysteresis: 0.01` (directions 8). It stays above Unreal VSM on every measured quantity of S2, S3 and the quarry, traced and no-RT. S1 keeps N8c's single miss: 2 umbra leaks on pose 1 against VSM's 0. Cost falls about 4x on frames whose camera and casters hold still, and the sun-turning target is met. The static target is missed by 0.1-0.35 ms, and the no-RT frame lands at the 1.61 ms target within noise. Frames whose camera or casters move gain only the walk cap's sixth: they cost 2.9 ms on the quarry against VSM's 0.5 and miss.**

| Quarry, shadow pass (M, same binary, clean visits) | Static ms | Sun turning ms | Camera moving ms | No-RT frame at 0.35 px, ms |
|---|---:|---:|---:|---:|
| N8c / N8d output (old settings) | 3.01-3.05 | 4.97-5.05 | 3.43-3.47 | 3.86-4.20 (turning 5.32-5.71) |
| **N8e default (refresh 16)** | **0.72-0.95** | **0.70-0.96** (max 0.95-1.45) | 2.92-3.16 | **1.55-1.71** (turning 1.61-1.71) |
| N8e, refresh 8 | 0.84-1.13 | 0.87-1.16 (max 1.34-2.75) | 2.86-3.12 | 1.78-1.95 (turning 1.81-1.93) |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded | 1.61 |
| Target | ≤ about 0.6 | ≤ about 1.2 | — | ≤ 1.61 |

Traced quarry screen columns are the shadow pass on traced receivers (pose 0); the no-RT column is the whole frame (primaries 0.39-0.41 ms, shadow pass 0.81-0.91 ms clean). The camera-moving row is the camera translating 1 mm a frame, which resets the rolling refresh every frame (below).

S2 and S3 (M): S3's shadow pass falls from 1.09-1.67 ms to **0.24-0.25 ms** (VSM 0.37 static, 0.59 rotating). S2 falls from 0.53-0.66 ms to **0.15-0.20 ms** in its hold phases (VSM 0.35), and stays at 0.65-0.69 ms in its camera-motion and caster-motion phases (VSM 0.35-0.36), where every frame is a full lookup.

## What spends the surplus (M)

Three settings, each measured on its own first (traced quarry screens, `scripts/n8-batch.py screen-quarry`, against Interactive on the same eight directions and N8c's shader on the same binary):

1. **Walk cap (lever 1).** Once a receiver's exact walks have visited 16 triangles and one of its directions is proven blocked, every undecided direction is taken as blocked. Only proven blocks trigger it, so a fully lit receiver is never darkened: lit-darkened counts are unchanged on every bench. It spends partial-shadow accuracy, biased dark, and buys fewer leaks and a sixth of the cost: quarry umbra receivers lit against Interactive 6,673 → 1,436, static 3.00 → 2.48 ms.
2. **Map hysteresis (the turning half of lever 4).** The clipmap keeps its light basis and cached pages while the sun stays within 0.01 rad of the direction it was rasterized for. The map only finds candidates; every walk tests the current sun's directions exactly. Its march and page requests widen their cone by the angle. Turning frames then cost what static ones do (traced turning 4.97 → 3.26 ms on its own), and the output differs from N8c's by a few pixels (+14 leaks of 9,910).
3. **Rolling refresh (lever 2, in screen space).** On a frame that keeps the last frame's camera, casters, receivers and settings, one 8x4 trace tile in every 4x4 block is looked up again, in ordered-dither order. The others keep their counts in the visibility target. A static frame's output is byte-identical to the dense lookup's (hashes equal on every screen). Under a sun turning inside the hysteresis, a kept count lags the sun by at most 15 frames. Any other writer of the visibility target, moved casters, a new map basis, or a receiver change resets to a full frame.

The rolling refresh does not scale as 1/N (M, screen 2): static cost fits about 0.95 ms + 2.05 ms/N at N = 4, 8, 16. At refresh 16 the setup-only lookup costs 0.02 ms and the lookup without the far march 0.14 ms (screen 3). The floor is the latency of the slowest tiles' marches and walks, which the dense dispatch hides under its throughput. The walk cap lowers it (refresh 16: 1.12 → about 0.72 ms clean).

## Quality (M)

Same scorers, references and recorded Unreal rows as N8c/N8d. The quarry is scored by `scripts/n8e-score.py` (n1c-score's metrics on pixels both the label and the traced control cover, against the 64-seed traced Exact reference rebuilt in-process). It reproduces N8c's table to every digit for the old settings. Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan; validation on for quality and screens, off for no-RT timing.

**Quarry** (static output; identical at refresh 8 and 16, and with or without the refresh, by hash):

| Pose / mode | Partial RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|
| 0, no-RT 0.35 px, N8c | .3262 | 3,680 | 9,910 | 60 | 3,582 |
| **0, no-RT 0.35 px, N8e** | **.3529** | **2,434** | **3,288** | **60** | **2,376** |
| 1, no-RT 0.35 px, N8c | .3223 | 3,685 | 10,023 | 55 | 3,596 |
| **1, no-RT 0.35 px, N8e** | **.3476** | **2,474** | **3,366** | **55** | **2,424** |
| 0, traced, N8c | .2237 | 797 | 6,002 | 0 | 779 |
| **0, traced, N8e** | **.2863** | **340** | **1,201** | **0** | **339** |
| 1, traced, N8c / **N8e** | .2268 / **.2896** | 805 / **356** | 5,985 / **1,213** | 0 / **0** | 795 / **356** |
| Unreal VSM native (pose 0 / 1 lit darkened) | .483 | 12,390 | 14,935 | 87 / 88 | 11,884 |

Partial RMS moves toward VSM's but keeps a wide margin; every solid measure improves on N8c's. The lit-darkened margin is unchanged and still thin (55-60 against 87-88). With the sun turned 16 frames (0.008 rad), the refresh-16 output has 12 lit receivers shadowed against Interactive at the turned sun (refresh 8: 5; dense: 3), from the lag.

**S1** (static): contact loss 0/419 and 0/335, leaks 0 and **2** (VSM 0 and 0; N8c's unchanged miss), solid flips 0, partial RMS .1085 and .1065 (N8c .0822 and .0809; VSM .1642 and .1628).

**S2, disk 0.04 rad** (N8c → N8e; UE VSM recorded):

| Case / phase | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / HDR / lost / leaks / flips / ms) |
|---|---|---|---|---|---|---|---|
| camera, out | .0746 → .0833 | .0105 → .0107 | 0 → 0 | 0 → 0 | 0 → 0 | 0.66 → 0.66 | .1752 / .0151 / 2,101 / 62 / 167 / 0.35 |
| camera, hold | .0750 → .0865 | .0112 → .0115 | 0 → 0 | 0 → 0 | 0 → 0 | 0.52 → **0.20** | .1758 / .0154 / 1,039 / 57 / 152 / 0.35 |
| camera, return | .0748 → .0809 | .0101 → .0103 | 0 → 0 | 0 → 0 | 0 → 0 | 0.65 → 0.65 | .1739 / .0145 / 1,972 / 62 / 155 / 0.35 |
| shadow step, out | .0735 → .0759 | .0099 → .0099 | 0 → 0 | 0 → 0 | 0 → 0 | 0.66 → 0.65 | .1697 / .0152 / 2,070 / 65 / 120 / 0.36 |
| shadow step, hold | .0727 → .0766 | .0099 → .0100 | 0 → 0 | 0 → 0 | 0 → 0 | 0.53 → **0.15** | .1675 / .0150 / 1,059 / 66 / 119 / 0.35 |
| shadow step, return | .0735 → .0759 | .0099 → .0099 | 0 → 0 | 0 → 0 | 0 → 0 | 0.66 → 0.66 | .1692 / .0152 / 2,057 / 67 / 120 / 0.35 |

**S3, 525 k triangles, 15° sun:**

| Phase | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / HDR / lost / leaks / flips / ms) |
|---|---|---|---|---|---|---|---|
| static | .0778 → .1186 | .0142 → .0150 | 0 → 0 | 2 → 1 | 0 → 0 | 1.10 → **0.24** | .1878 / .0194 / 2,929 / 111 / 187 / 0.37 |
| rotate 0.25°/s | .0783 → .1217 | .0141 → .0151 | 0 → 0 | 9 → 1 | 0 → 0 | 1.67 → **0.25** | .1875 / .0194 / 11,630 / 111 / 190 / 0.59 |
| hold slow | .0781 → .1266 | .0141 → .0152 | 0 → 0 | 4 → 1 | 0 → 0 | 1.09 → **0.25** | .1873 / .0194 / 3,006 / 109 / 191 / 0.36 |
| rotate 1°/s | .0782 → .1218 | .0141 → .0150 | 0 → 0 | 9 → 1 | 0 → 0 | 1.66 → **0.24** | .1863 / .0200 / 12,289 / 106 / 192 / 0.59 |
| hold fast | .0779 → .1210 | .0141 → .0150 | 0 → 0 | 2 → 0 | 0 → 0 | 1.10 → **0.25** | .1875 / .0205 / 3,378 / 103 / 188 / 0.36 |
| jump recovery | .0762 → .1159 | .0138 → .0146 | 0 → 0 | 5 → 0 | 0 → 0 | 1.09 → **0.24** | .1811 / .0219 / 3,153 / 114 / 226 / 0.37 |

S2 and S3 quality is identical at refresh 8 and 16 to the printed digit, rotation phases included: the lag is not measurable at S3's rates. S3's partial RMS rise is the walk cap's dark bias on grazing relief. Not film quality, and never presented as meeting the film gate.

## The frontier (M)

Traced quarry pose 0, static / turning shadow ms (clean visits), and no-RT pose 0 quality (partial RMS / leaks / lit darkened):

| Setting | Static | Turning | No-RT quality | Verdict |
|---|---:|---:|---|---|
| N8c (old default) | 3.00 | 4.97 | .326 / 9,910 / 60 | above Unreal, about 6x its cost |
| directions 4 / 2 (no accumulation) | 2.36 / 2.12 | 4.38 / 4.10 | not scored | per-receiver fixed costs dominate; small saving |
| walk cap 16 / 8 | 2.48 / 2.43 | 4.40 / 4.35 | .353 / 3,279 / 60; .385 / 2,715 / 60 | above Unreal |
| hysteresis 0.01 | 3.03 | 3.26 | .327 / 9,924 / 60 | above Unreal |
| cap 16 + hysteresis + refresh 4 | 1.06 | 1.09 | .353 / 3,288 / 60 | above Unreal |
| cap 16 + hysteresis + refresh 8 | 0.84-0.91 | 0.87-0.93 | same | above Unreal; frame 1.78-1.95 ms |
| **cap 16 + hysteresis + refresh 16 (default)** | **0.71-0.73** | **0.70-0.75** | same | **above Unreal; frame 1.55-1.71 ms** |
| cap 8 + hysteresis + refresh 16 | 1.19-1.23 | 1.22 | .385 / 2,715 / 60 (static, as cap 8) | no faster than cap 16 under contention |

The cheapest setting at or above Unreal everywhere is the default above. The cap and hysteresis are needed for its quality and turning cost; refresh 16 over 8 buys 0.1-0.2 ms for a doubled lag bound.

## Tried and rejected (M)

- **Sparse fill (lever 3's spatial form).** Anchors every 2-4 pixels, then each other pixel copies its anchors' count where all are fully lit (or umbra) on its placement, and looks up otherwise. Static 3.47-3.68 ms against 3.00 (one 8.13 ms read), and 80-163 lit receivers shadowed against Interactive (default: 1). The quarry's shadow work is too fragmented: most waves still hold a lane that needs its own lookup, and umbra fill darkens lit pixels between shadowed anchors. Removed.
- **Hard caps** (a receiver with no proven block stops at the cap, lit where undecided): umbra receivers lit against Interactive 4,830-5,169 at caps 16-64 (cap 16 with proof: 1,438). Many umbra receivers need over 64 visits before their first proof. Not kept (`VSM_CAP_RULE` 2 remains a screen constant).
- **Lean walks** past the cap, shorter coarse-level walks (`VSM_WALK_SHIFT`), fewer march samples, fewer coarse samples: no gain resolvable above the contention (0.67-1.33 ms visits of the 0.71-2.40 ms control), with up to 420 more umbra receivers lit against Interactive. Kept as screen constants only.
- **Reprojected rolling refresh** (built, screened, reverted; `E:/renderlab-work/n8e/variants/s9-reprojection*`). Camera motion would keep counts reprojected from the previous frame into ping-pong history words (count, age, drift, f16 eye distance). Plain nearest-pixel reuse displaced shadow edges: at about 0.14 px of motion a frame, each reuse rounds back to the same pixel, so a count stays put while its surface slides. After a 2 px camera step, 6,159 static pixels were wrong (1,792 lit receivers shadowed against Interactive), all at shadow edges and shifted copies of the correct counts. Bounding the accumulated drift at 6/16 pixel cut that to 508 pixels (117 lit shadowed), but forced scattered lookups every 2-3 frames: moving frames then cost 1.8-4.1 ms, no better than dense, and static frames read 0.99-1.00 ms in their clean visits against 0.84-0.91 without it. A world-anchored history (counts kept per surface point, not per pixel) is the remaining way to reuse under camera motion (I).
- **Direction accumulation** (k directions a frame continuing R2's sequence, T1a's static rule) was built first and dropped with the refresh: per-frame savings at k = 2-4 are 20-30%, and the refresh amortizes everything a receiver does.

Not built: the camera-consistent caster cut (N8c's caster LOD). The hysteresis removes the turning frames' page work without changing casters, so the cut's lit-receiver risk was not needed for the turning target.

## What was built (M)

`shaders/virtual_shadow.wgsl`, `src/virtual_shadow_gpu.rs`, a renderer hook in `src/renderer.rs`, test harnesses and `scripts/n8e-score.py`:

- **Settings** (`VirtualShadowSettings`): `directions` (1-8, default 8), `walk_cap` (16), `refresh` (1/2/4/8/16; 16), `sun_hysteresis` (radians, at most 0.05; 0.01). N8c's output is `walk_cap: 0, refresh: 1, sun_hysteresis: 0.0`; on the final binary its hashes equal N8c's and N8d's on every screen.
- **Frame uniform** gains a uvec4 (directions, refresh schedule, cap); `FRAME_BYTES` 896 → 912.
- **Page key** now also holds the disk slope (the march radius depends on it) and the receivers' producer and cut (primary mode, LOD pixels, software extent, lift). Before this, switching primaries at a fixed camera skipped the page requests and reused the previous receivers' per-pixel levels (no measured effect: old-settings outputs are unchanged). The refresh needs the receiver key to be correct.
- **Renderer hook:** a soft-shadow frame that is not a traced Virtual frame clears the rolling flag, so a frame after Interactive, Exact, Temporal or raster fresh shadows looks up every tile.
- **Tests:** `rolling_refresh_visits_every_tile_once_per_cycle` (each tile once per `refresh` frames, so no kept count is older than `refresh - 1` frames) and validation of the new settings. The quarry screen also records turning p50 and max, agreement against Interactive at the turned sun, and a moving-camera timing, and can dump counts (`N8E_DUMP`). `gpu_visibility_quarry` takes every `VirtualShadowSettings` field and `virtual_variants` (one process, every variant). New screen constants: `VSM_CAP_RULE`, `VSM_WALK_SHIFT`, `VSM_LEAN`, `VSM_LEAN_STEPS`.
- **Programmability:** no lighting-model data entered hit records, clusters, the maps or the refresh; the pass still outputs visibility counts the user's shading programs consume. Kept counts are visibility only.
- **Memory:** unchanged (no new buffers; the refresh keeps counts in the existing visibility target).

## Misses, honestly

- **Static cost:** 0.72-0.95 ms against the ≤ 0.6 target and VSM's 0.50, from the slowest tiles' latency (above). About 0.1-0.35 ms off.
- **Camera or caster motion:** every such frame is a full lookup. The quarry costs 2.9-3.2 ms with the camera moving (VSM about 0.5, I), and S2's motion phases 0.65-0.69 ms against VSM's 0.35-0.36. The walk cap is the only saving there. This is the largest remaining gap; the reprojected form was measured and does not close it.
- **No-RT frame:** 1.55-1.71 ms in clean visits against 1.61; at the target within noise, not reliably under it. Refresh 8 misses it (1.78-1.95 ms).
- **Sun turning:** met (0.70-0.96 ms against ≤ 1.2 and VSM's about 0.95). When the sun leaves the 0.01 rad window, one frame rasterizes every page again (turning max 0.95-1.45 ms over 16 frames, which stayed inside the window; the rebase frame looks up every tile after rasterizing every page, about N8c's turning frame of 5 ms traced, I). Kept counts lag a turning sun by up to 15 frames.
- **S1 pose 1: 2 umbra leaks** against VSM's 0 (N8c's miss, unchanged).
- **Partial RMS regressed against N8c** on every bench (quarry .326 → .353, S1 .082 → .108, S2 .073-.075 → .076-.087, S3 .078 → .116-.127), still well inside Unreal's. The lit-darkened margin on the quarry is unchanged and thin (55-60 against 87-88).
- Not film quality; never presented as meeting the film gate.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8e/`): `s1`-`s9/screen/screen-quarry.json` (variant screens with hashes; `screen.py` summarizes), `s3-q`, `final-q` (quarry quality, `n8e-scores.json`, PNGs), `s4-t`, `final-t`, `final2-t` (no-RT timing, `timing-summary.json`), `s6-d/-a/-b`, `final`, `final2` (S1 `scores.json`/`comparison.csv`, S2/S3 `comparison.json`, `n8-comparison.json`, `n8-score.log`), `variants/` (every shader built, the reverted reprojection and its Rust diff), `edits/` (scripted edits, `diff.py`), `runs/` (every lease's log, preflight and memory samples). Raw arrays are deleted after scoring and listed in `deleted-raw.txt`.
- **Build:** final binary (`final2-binary.sha256`) is the committed source; `final-binary.sha256` is the same source with `refresh: 8` as the default.
- **Gate:** `cargo test -p renderlab --lib -j 2` exit status 0 on both final binaries: 106 passed, 0 failed, 107 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) and fmt pass.
- **Contention:** `mypaper-wallpaper` (29% SM in `nvidia-smi pmon`) and `GrowthOS.exe` ran throughout; contended visits read up to 3x and are excluded by the clean-range rule (primaries at 0.39-0.46 ms mark clean no-RT visits). Every screen interleaved N8c's settings on the same binary. No game ran (Riot client only), and no window, focus or capture was used.
- **Leases:** 12 under the shared lock with the 4/2 GiB guard, 1,431 s in total. The minimum available memory was 2.9 GiB, during the second final lease while CPU scoring of the first ran beside it. One lease (s7) failed at pipeline creation on a duplicated shader function and was rerun after the fix.
- **Time (UTC):** 15:10-15:28 reading and the first implementation (directions, fill, cap, accumulation); 15:28-15:57 screens 1-6 (GPU about 12 minutes) with the refresh and hysteresis built between them; 15:57-16:22 S1/S2/S3 scoring and the reprojection, built, screened and reverted (GPU about 3 minutes); 16:23-16:34 two final batches and gates (GPU about 8 minutes); 16:34-17:00 scoring and this record. GPU verification took 24 minutes and CPU scoring about 20; implementation took the rest, about 65 minutes, interleaved.
