# P-GATHER: non-RT batched GI queue screening

Release-freeze work from main `69c667b`, 2026-11-10. **MERGE.** Non-RT moving service improves on game and quarry, all compared images match, and the inspected cut sequence has no visible regression. RT source/module and gather entry remain unchanged; the RT early-still timing is variable and is not certified numerically unchanged.

The candidate recovers P-NRT3's gather from `origin/t3/p-nrt3` (`9e4d474`), retaining today's two-submit scheduling. Consecutive queued cells are screened by separate lanes; deferred appends reserve one queue range per workgroup. Live cells use the original rays, lookup, fixed reduction, schedule and writes. No quality setting, GI cap or fill budget changes. Queue order has no semantic meaning: the rotation is by cell slot and outgoing radiance is read from the previous frame.

`gi_gather_batched` is selected only for software rays. RT keeps `gi_gather` unchanged. The recovered scalar is named `gi_drain_base` to avoid the existing setup array. No shadow-owner files are edited. Shading stays programmable; no new patent fallback or sampling method is introduced.

Evidence, plans, binary digests and build admission records: `D:/Coding/renderlab-work/p-gather`. Builds use the atomic build slot and normal stillkit mutex, one command per hold, two Cargo jobs. Admission requires no compilers, 8 GiB available RAM and 5 GB free on D:. GPU execution uses `scripts/n8-run.py`, current streamed root order, owner-prefixed request and a fresh <=15-minute lease. A copied warm P-S4B target is private to this unit; its source is untouched.

## Verification

- Lib: **200 passed, 0 failed, 164 ignored**, including GI kernel validation for hardware/software at 64/128 directions.
- A/B identity: **51/51 HDR hashes and 43/43 PNGs identical**: non-RT 35 HDR/31 PNG, RT 16 HDR/12 PNG. Extra baseline-repeat comparisons also match (16 HDR/12 PNG). These cover game/quarry timing witnesses on both paths and the 19-frame non-RT quarry quality sequence. HDR hashes are computed before readback disposal; no bulk HDR dumps were produced.
- Verified by looking at `quarry-cut-to-settle.png`: baseline/candidate frames 72, 73, 74, 75, 76, 80 and 87, plus native candidate frames 72 and 87. No missing or newly delayed bounce, rock silhouette change, shadow change or disocclusion defect is visible. Every captured quality frame, including the cut and its recovery, is byte-identical. This preserves existing baseline quality; it is not a fresh Unreal quality claim.
- Final lease `p-gather-abba-3`: **245.112 s, exit 0, zero failures, 58 runtime state samples, zero compiler samples**. All 19 plan steps succeeded. Two-minute compiler-free admission, 8 GiB admission floor, 2 GiB runtime floor. No foreground window or desktop capture was used.

## Quiet ABBA service

Synchronous CPU render-plus-GPU-drain service, excluding image encoding/readback. Native **1920x1080** timing; registered **960x540** quarry quality sequence. RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan, wgpu 30, release builds. ARA1 game-base/quarry-daylight, Realtime defaults, GI/reflections/TR1/post enabled, original rays, 16-ray glossy and 128-light shadow budgets preserved. Each scene/path runs baseline, candidate, candidate, baseline; phase medians pool two runs per binary. Still is frames 1-39, moving 40-71, first cut 72, recovered 73-87.

| Scene / phase | Non-RT baseline ms | Non-RT batched ms | RT baseline ms | RT candidate ms |
|---|---:|---:|---:|---:|
| Game still | 8.1026 | 7.9144 | 3.2611 | 3.0148 |
| Game moving | 11.2180 | 11.0572 | 4.9031 | 4.8922 |
| Game cut | 33.0412 | 33.6534 | 20.1580 | 20.1432 |
| Game recovered | 9.8563 | 9.9210 | 4.6146 | 4.6369 |
| Quarry still | 11.2422 | 10.9070 | 7.4641 | 8.4665 |
| Quarry moving | 11.5544 | 11.2573 | 3.6650 | 3.6969 |
| Quarry cut | 24.8089 | 24.1004 | 14.9006 | 14.9079 |
| Quarry recovered | 13.1278 | 12.5194 | 4.2107 | 4.2362 |

The moving median saves **0.1609 ms (1.43%) game** and **0.2971 ms (2.57%) quarry**. Both candidate moving run-medians beat both baseline moving run-medians on each scene. Whole-track non-RT service means (frames 1-87) are game 10.4594 -> 10.2414 ms and quarry 12.3563 -> 12.3333 ms. The quarry mean gain is small: its moving-phase mean rises 11.6093 -> 12.0392 ms despite the lower median, due to tails. No uniform frame-time improvement is claimed.

**Misses and limits:** game cut is +0.6122 ms in this two-cut cohort; recovered median is +0.0647 ms while its mean decreases. RT quarry early-still pooled median rises 1.0024 ms; per-run medians cross (baseline 9.0113/7.3665, candidate 7.7368/8.5488). This repeats the known variable early RT phase from P-NRT3b. All other RT medians differ by at most 0.032 ms except game still, which improves. The RT module's generated text and selected entry are unchanged by construction, and RT image hashes match, but **this small timing cohort does not establish numerically unchanged RT early-still performance or explain its variation**. No extra campaign was started during the release freeze. Keep this limitation visible; do not interpret the non-RT win as an RT acceleration or a broad Unreal/Ara victory.

The merge recommendation follows M8's clarified visible-image gate and incremental speed rule, rather than requiring every phase median to improve. Root can cherry-pick source `048d9f3`; no main merge was performed by this implementation sub-agent.

## Build identity, reproduction and time

Baseline main `69c667b952c068661b9a8f38e8a359ca4e386fd0`; candidate source `048d9f3f8fdd141bdca1ba73d016882750e2fbcc`. Documentation after that source commit does not affect binaries.

| Binary | SHA256 | Compiler wall seconds |
|---|---|---:|
| base.exe | `116e10e8e4b3171bae10560169300d90ee3bc029183a4daefcca2a8fbc8f91d6` | 43.16 |
| cand.exe | `363357c0704596447ec1e6c5a1a0fbcdc4ae627b1542306274fc8fdcc0be90eb` | 30.51 |
| tests.exe | `1553bcc4279715a51245eba058f66e137dc1328430e1775da3f03d6f5bae17af` | 91.95 |

Local `build.ps1/build.py` hold the stillkit mutex and atomic build slot for one command; `lease.py` delegates execution to `scripts/n8-run.py`; `plan.json` runs lib plus all captures using `tools/p-nrt/plan.py`; `summary.py` checks plan completion and final clean lease before computing `summary.json`. `partial-summary.json` is the interrupted first screen and is **not** the final result. `summary.json`, `plan.results.json`, binary identity JSONs, lease JSONs/logs and inspected PNGs are retained.

Implementation edit time was under five minutes (estimate); compiler time was 165.62 s, separately from approximately 359 s of admitted verification (114 s interrupted + 245 s final), admission waits and host coordination. Initial lease `p-gather-abba-1` stopped when foreign cargo PID 17024 appeared, with 11898 MiB RAM; it was not a renderer failure. Retry 2 never admitted while another project's Cargo/rustc ran (Glaze observed). Once compilers stopped, retry 3 admitted at 01:43:18 UTC and completed around 01:47:21, within the freeze. No foreign process, lock or mutex was removed.

## Cleanup / next step

All requested implementation and verification completed before the release cutoff. No owned build/GPU process or waiter remains. No raw HDR or GPU dumps over 5 MB were produced. **Automatic approval review rejected the native deletion of the owned Cargo target as "blocked by policy"**; `D:/Coding/renderlab-work/p-gather/target` therefore remains for root/user cleanup. Its source warm target was never modified. This is the only cleanup handoff.

Next performance work should explain RT early-fill variability and the quarry moving tails/game-cut cost before claiming uniform improvement. This does not lower quality or introduce a new merge gate.
