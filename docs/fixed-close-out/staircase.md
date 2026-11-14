# vd_lookup staircase (U-FIXED, 2026-11-14, closed)

The 2026-11-14 investigation (U-FIXED, leases fixed-gip5 to gip8) found a repeatable staircase in the non-RT `vd_lookup` dispatch on the game_scene forest fixture. It steps 14 -> 28 -> 55 ms over about 12 frames, once per run, usually around frame 78. The dispatch input is identical and frames are byte-equal.

Ruled out: clocks, TR1 frame index, `k & 0xff`, jitter, prewarm, GI cache, page work, VRAM paging (memory.used and PCIe flat), wall-clock timers, fixed GPU-work totals, heat, power and bandwidth (memory utilisation 6-13%), and long-dispatch scheduling (splitting into 4 dispatches keeps it). The start frame moves unpredictably (78, 253, 327, 62 across variants). Afterwards the lookup settles FASTER (about 6.9 ms against 7.35). Best unconfirmed hypothesis: the driver remaps the about 800 MB of shadow allocations in the background. Staircase leases were stopped there. The cheap test left: allocate the big shadow buffers as dedicated allocations.

Full staircase seen only on the fixture. Ara Outpost non-RT shows a single 2x `sun_shadow` step (6 to 12 ms, about 15 frames) in the held settle around clip frames 508-527, the same family. It's absent with ray hardware and on Showcase.

**How to apply:** every non-RT worst-frame figure on the fixture says whether the staircase is inside its window (the forest "first 40 frames" figures end before it). Don't reopen the investigation without driver-level tools or the dedicated-allocation test. Related: [[sun-shadow-stage-includes-gi]], [[gpu-slow-state-reference-run]].
