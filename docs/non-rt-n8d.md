# N8d: conservative skips for the non-RT shadow pass, measured

Follow-up unit to [N8c](non-rt-n8c.md), 2026-11-06 UTC, base `8af0ddf`, branch `t3/n8d-shadow-cost`. Brief (Islam, 2026-11-06): cut N8c's cost with levers that are conservative skips, so outputs stay byte-identical to N8c's default; targets quarry static ≤ about 0.6 ms, turning ≤ about 1.2 ms, no-RT frame ≤ 1.61 ms. Labels: **M** measured here, **I** inference.

**Follow-up: [N8e](non-rt-n8e.md)** spends the quality surplus instead: a walk cap, a map that keeps its sun basis through small turns, and a rolling refresh of one trace tile in sixteen on frames that hold still. Quarry static 3.0 → 0.72-0.95 ms, turning 5.0 → 0.70-0.96 ms, no-RT frame 1.55-1.71 ms, all above Unreal's quality except N8c's S1 pose 1 leaks. Frames with a moving camera or caster still cost a full lookup.

## Result

**No lever cut the cost. Every target is missed, by the same margins as N8c. Every variant built was byte-identical to N8c on every bench where it was checked, and the retained code is byte-identical on all of them.** The retained change is a correctness fix for mirrored placements (below), which no bench exercises. Quality equals N8c's on every measure: Unreal VSM is beaten everywhere except S1 pose 1 (2 umbra leaks against 0), N8c's unchanged miss.

The trace explains why conservative skips cannot reach the targets. The pass is not spent on lit receivers: 90% of quarry receivers are lit, but they are nearly free. The cost is the exact walks of the 10% in partial shadow or umbra, multiplied about 4x by wave divergence. Conservative skips cannot remove those walks without changing the output.

| Quarry, traced screen (M, same binary and lease) | Static ms | Sun turning ms |
|---|---:|---:|
| N8c shader | 3.09-3.12 | 5.07-5.08 |
| **N8d (retained)** | **3.07-3.12** | **5.01-5.11** |
| Unreal VSM (recorded) / target | 0.50 / ≤ 0.6 | about 0.95 / ≤ 1.2 |

No-RT quarry at 0.35 px (M; final lease, clean visits, N8c's shader on the same binary in brackets): shadow pass 3.10-3.34 ms (3.12-3.32), frame 3.82-4.12 ms (3.86-4.06); sun turning 4.55-4.80 ms (4.52-4.79), frame 5.27-5.57 ms (5.25-5.53). Two of four static visits of the final shader read high under contention (3.51 and 4.49 ms) and are excluded. Target ≤ 1.61 ms (VSM 1.61). S3 1.17 ms static, 1.72 ms rotating (N8c 1.18-1.20, 1.74-1.75; VSM 0.37, 0.59). S2 0.67 ms, unchanged (VSM 0.35).

## Where the cost is (M)

**Decomposition** (traced quarry, screen 1, on an instrumented build 0.37 ms above N8c's; differences between variants): far march off 0.82 ms; candidate walks off 1.30 ms; ring off (receiver triangle only) 2.85 ms; default 3.46 ms. So the walks are about 2.2 ms, the march loop without them 0.5 ms, the ring 0.6 ms, and setup with the receiver triangle 0.2 ms.

**Per receiver** (`VSM_DEBUG` trace with extra counters, pose 0, 346,517 receivers):

| Class | Receivers | Walk visits | Steps | Far footprint tests / passed by N8c's max-mip | Texels read | Ring walked |
|---|---:|---:|---:|---:|---:|---:|
| Lit | 310,387 | 0.46 | 10.9 | 5.90 / 5.80 | 12.2 | 16% |
| Partial | 14,097 | 45.2 | 10.7 | 5.73 / 3.87 | 19.3 | 64% |
| Umbra | 22,033 | 21.5 | 1.9 | 0.41 / 0 | 5.0 | 42% |

93% of lit receivers walk no candidate at all, and 39% have no texel above its lower bound at any step. Partial and umbra receivers make 88% of all walk visits.

**Per wave** (8x4 lookup workgroups, one 32-lane wave each): summed over waves, the slowest lane's visits are 4.0x the lane total. 66% of waves contain a walking lane and 33% a shadowed lane. Ring walks: 20% of lanes walk, but 83% of waves contain one. If no lit receiver walked its ring, 28% of waves would, and the ring's per-wave cost would fall to 32% (137k → 44k triangle steps summed over waves).

## The levers (M)

All variants ran in one lease each, with N8c's shader on the same binary before and after (`RENDERLAB_VSM_SHADER`, below). Output identity was checked by SHA-256 of the static and turned outputs; timings are clean-range medians.

| Lever | What was built | Identity vs N8c | Quarry static ms (N8c same lease) | Verdict |
|---|---|---|---|---|
| **1. Whole-cone conservative lit test** | Where N8c's coarse 2x2 max-mip test fails on a far step: the max-mip one level finer (≤ 3x3 cells), then footprints within 4x4 texels tested texel by texel against the receiver plane, both conservative bounds on every sample's lower bound | identical (quarry) | 3.39 vs 3.46 (instrumented build); finer-mip-only variant 5.25 | No gain. The march already is a conservative whole-cone test: lit receivers pass 98% of far steps by N8c's max-mip, and the finer tests clear 0.07 more steps per lit receiver. Lit receivers' remaining cost is their own-surface candidates, which a texel bound cannot certify. Removed. |
| **2. Wave early-out** | Not built as specified: a wave already leaves a loop once every lane has, so SMRT's early-out has nothing byte-identical to skip. Built instead: the ring's vertex fans dealt out across all 32 lanes through workgroup memory, owner state included, results merged atomically (`vsm_lookup_coop`) | identical (quarry, S2, S3) | 3.32-3.44 vs 3.08-3.11 | Slower. The state save, two barriers and 6 KB of workgroup memory per wave cost more than the fan balance saved. Removed. |
| **3. Per-azimuth ring horizons** | Eight 45° sectors per triangle side, 2-bit codes relative to N8b's bound, in a second buffer. First the cone's azimuth interval (no effect: an edge-adjacent neighbour's generators include both directions of the shared edge, so it covers every sector), then the exact maximum over cone ∩ wedge (generators, pair spans' boundary crossings, pair spans' closest approach). A walk only the sectors skip uses exact ring membership for the march's ring filter, so the march matches a walked receiver. | identical (fixture, quarry, S2, S3) | 3.07-3.08 vs 3.04-3.11 | Lit ring walks −20% (49.9k → 39.9k), all ring walks −16%, but the per-wave ring cost only −5% (137k → 130k), about 0.03 ms. Not kept: +4 B per triangle (+104 MB on the LOD quarry) and a heavier first-frame horizon pass. |
| **4. Camera-consistent caster LOD** | Not built | — | — | Changes outputs and needs its own quality gates (N8c's caster LOD failed Unreal's lit-receiver floor); not reached in this unit. |

Other byte-identical restructures, from the cost data (all identical, none kept):

- **Walk adjacency prefetch** (a triangle's three neighbours read with its indices; neighbours' loads together): 3.11 vs 3.10 ms. Neutral, so dependent-load latency on that chain is not the limiter.
- **Interleaved ring fans** (three cursors per round): slower (turning 5.55-5.80 against 5.05 ms).
- **Cone cull before the walks' eight ray tests** (the ring's cull): 3.07-3.08 vs 3.10-3.11 ms, within the noise.
- **Set-bit direction loops**: 4.45 vs 3.11 ms. The fixed eight-direction loop compiles to constant-indexed registers; a dynamic index breaks that.
- **Lookup workgroups** 8x8, 16x4, 16x8: 3.08-3.26 vs 3.10-3.12 ms. Occupancy is not limited by blocks per SM.

**A codegen hazard (M).** Adding `VSM_DEBUG`-gated counters and an equivalent restructuring of the ring-membership condition made the default lookup 0.37 ms (12%) slower, with identical output. It was bisected with the hunk-subset variants (E:/renderlab-work/n8d/bisect.py) on one binary. Perf screens of this shader must compare against N8c's shader on the same binary, never against a recorded number.

Unexplained (M): with some variants (interleaved fans, full prefetch, cooperative ring, finer mip only), the 24 static frames' median read 7-8 ms while the same variant's turning frames were normal. Default and N8c visits never did this (≥ 30 visits). It is recorded, not explained; none of those variants was kept.

## What changed (M)

- **Mirrored placements** (`vsm_beyond_horizon`): N8b's ring horizon chose the side's bound by the world winding. Under a negative-determinant similarity, the world winding normal is the transform of the object's −n, so a receiver on the winding side read the object's up-side bound for its down side. That skip was not conservative for mirrored meshes. The side now swaps when the placement mirrors. No bench has a mirrored placement, and all outputs are byte-identical.
- **Tests:** quarry and motion variant screens record SHA-256 of the static and last turned outputs (`hash`, `turned_hash`). `RENDERLAB_VSM_SHADER` (tests only) substitutes a shader variant file, so one binary screens shader variants without a rebuild.
- **Programmability:** no lighting-model data entered hit records, clusters or maps.

## Quality (M)

Byte-identical outputs: SHA-256 equal between the final shader and N8c's, run on the same binary in the final lease, for 2,224 arrays. These cover S1 (48), S2 (988 frames' arrays), S3 (1,178), the fixture (6), and the quarry traced and no-RT 0.35 px counts at poses 0 and 1. The scorers reproduce N8c's tables to every digit: quarry pose 0 no-RT .3262 / 3,680 / 9,910 / 60 / 3,582 and traced .2237 / 797 / 6,002 / 0 / 779 (partial RMS / solid flips / umbra leaks / lit darkened / lost umbra edges; film 2.6000 / 2.5712), against VSM's .483 / 12,390 / 14,935 / 87 / 11,884. S1 is 0/419 and 0/335 contacts lost, with 0 and 2 leaks against VSM's 0 and 0. S2 and S3 quality fields are equal to N8c's `comparison.json`. Not film quality, and never presented as meeting the film gate.

## Misses, honestly

- **Every cost target**, unchanged from N8c: quarry static 3.1 ms (≤ 0.6), traced turning 5.0-5.1 ms and no-RT turning 4.6-4.8 ms (≤ 1.2), no-RT frame 3.8-4.1 ms (≤ 1.61).
- **Levers 1-3 deliver nothing measurable**, and lever 4 was not built.
- **S1 pose 1: 2 umbra leaks** against VSM's 0 (N8c's miss).
- Next (I): conservative skips are exhausted at this level. The cost is exact walks in shadowed waves. The candidates left are a profiler (no Nsight on this machine; every restructure here was a blind experiment) and per-direction redistribution of the far march: its directions are independent after the shared steps, and the trace estimates 54k → 20k wave-visits, about 0.4 ms, though the cooperative ring's overhead counsels caution. Beyond that, levers that change output, under the quality gates: fewer exact walks (N8c measured 2.0-2.2 ms at about twice the leaks), SMRT-style sampled decisions away from the penumbra, and the camera-consistent caster cut for the turning sun.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8d/`, 76 MB): `s1`-`s10/screen` (variant screens with hashes), `s1/classes-pose0.json` (trace classification by `n8d-classify.py` on the instrumented shader `variants/v3-cone.wgsl`), `variants/` (every shader variant screened, `bisect.py`), `final/` (S1 `score.log`, S2/S3 `comparison.json/csv`, `n8-score.log`), `final-q/quarry/summary.json` and PNGs, `final-t*/quarry-no-rt/timing.json`, `final-screen*`, `runs/` (every lease's log, preflight and memory samples). All raw arrays (4,527 files, about 11 GB) were deleted after scoring and are listed in `deleted-raw.txt`; N7a's hardlinked references are untouched.
- **Build:** development/test profile (renderlab opt-level 1), RTX 4070 SUPER, NVIDIA 617.14, Vulkan; validation on for quality and screens, off for no-RT timing. Final binary SHA-256 `80095bd2…5efa6d`; it and the Cargo target were removed after the gate. The SPIR-V validation messages (VUID-StandaloneSpirv-None-10684, an explicit layout on a Function variable) appear with N8c's shader too and come from elsewhere.
- **Gate:** `cargo test -p renderlab --lib -j 2` exit status 0: 105 passed, 0 failed, 107 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) and fmt pass.
- **Contention:** `mypaper-wallpaper` and `GrowthOS.exe` (28% SM, 42% memory bandwidth in `nvidia-smi pmon` at 14:12) ran intermittently. Every screen bracketed its variants with N8c's shader, and outliers are excluded by the clean-range rule. No game ran (Riot client only), and no window, focus or capture was used.
- **Leases:** 12 under the shared lock with the 4/2 GiB guard (minimum available 6.1 GiB), 1,353 s in total. Two failed and were fixed: a 12-byte horizon buffer exceeded the 128 MiB binding limit (156 MB on the quarry), and the sector binding exceeded 10 storage buffers per stage.
- **Time (UTC):** 13:40-13:55 reading and the first instrumented build; 13:55-14:10 screen 1, the trace analysis, the latency variants and the bisection; 14:10-14:30 cone cull, set-bit loops, cooperative ring and workgroup shapes; 14:30-14:48 lever 3 in two forms and its trace; 14:51-14:58 final lease with gate; 14:58-15:10 scoring and this record. GPU verification took 23 minutes and CPU scoring about 6; implementation took the rest, about 60 minutes, interleaved.
