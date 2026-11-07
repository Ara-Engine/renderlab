# M7-G1b: world radiance cache cost, cut refill and rotating environment

Unit G1b continues [M7-G1](m7-g1.md) on branch `t3/m7-g1b` from main `1660d4c`. Contract: [milestone-7.md](milestone-7.md). Claim constraints: [m7-p0-claims.md](m7-p0-claims.md) W1-W3, W8, C1-C5 and C8. Unreal comparator: M7-U0b checkpoint `35a2759` (`t3/m7-u0b`, not yet on main). Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## Status

The cache stays **off by default** (`IndirectSettings::enabled`), as at G1.

**Cost (item 1).** These are camera-moving increments over the cache-off frame. The setup is SV at 1080p with the Lambert gate program, no per-pass timestamps, and quiet window `q2` (section 1).

| Path | G1 (base, same window) | G1b final | Stricter budget (root) |
|---|---:|---:|---:|
| RT | ⟨Q2 base RT⟩ | **⟨Q2 final RT⟩** | 0.45 ms |
| Non-RT | ⟨Q2 base NR⟩ | **⟨Q2 final NR⟩** | 0.46 ms |

**Measurement correction.** G1's quoted costs (+0.684 / +0.809 ms) included its own per-pass timestamps, about 0.05 ms a frame. Without them, G1's code measures about +0.60 / +0.74 ms.

**What the cost cut is made of:**
- whole-record lookups;
- 16x16 probe tiles;
- direct fixed-grid passes (no indirect dispatches, so no wgpu indirect-validation passes);
- multi-lane setup and finish;
- single-hash lookups;
- a constant sky applied at lookup;
- nearest-first software traversal.

Every claim constraint holds, and every gate passes (section 5).

**Cut spike (item 2).**
- **Non-RT:** a reset now spreads new cells' rays over four frames (W1 form 2 direction groups, with untraced directions filled from the nearest traced direction). The non-RT cut drops from 5.7-5.8 ms to ⟨Q2 cut NR⟩. It settles exactly by frame 4.
- **First frame after a cut:** 6.82% / 9.98% (overview / gaps), against Lumen SW Epic's 49.6% / 101.0%.
- **RT:** the refill stays off. The RT cut is ⟨Q2 cut RT⟩, a recorded spike.

**Sky steps.**
- A constant-sky change now costs nothing: the lookup scales the cached unit-radiance values.
- A prepared map's yaw re-integrates every cell and equals a fresh cache on the next frame: 0 pixels differ over six 15° steps, on both paths.
- **Miss:** a continuously rotating map costs ⟨Q2 env⟩ a frame (section 3).

**SV gate against Unreal (Lumen Epic, U0b `35a2759`).**
- G1b beats Lumen HW Epic (the RT comparator) and Lumen SW Epic with mesh SDFs (the non-RT comparator) on every lighting population, static and in motion, on both paths. The table is in section 4.
- **The one loss is the edge population:** 45.8% / 34.4% against Lumen HW's 26.9% / 18.4%. Edges measure single-sample anti-aliasing against a pixel-integrated reference, and Lumen's image passes through TSR Epic, so this is TR1's measure.

**Correction to G1's record.** G1's overview-pose scores counted about 323,000 stale sky records as surface pixels, which lowered its overview RMS. The default reads **1.95%**, not 1.64%; gaps-pose numbers stand (section 4.3).

**Quarry (items 3 and 4): not done.**
- The cook-time near field is not built.
- There is no quarry gate. U0b's quarry comparator is still to come, and its retained quarry references include bounce, not G1's depth-2 sky term.

Section 6 has the plan.

**Memory:** 114.4 MiB (G1 110.1), far under both Lumen totals.

## 1. Cost

### 1.1 Method

**Bench and harness.**
- SV overview pose, 1920x1080, the G1 harness (`gpu_gi_sky_visibility_sv`, `src/gi_cache_tests.rs`), RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development test profile.
- The default timing program is the G1 Lambert gate, the same as G1's numbers. One row uses the standard program.

**Sequences.** Each timing visit resets the cache, then runs:
- 24 still frames;
- 48 camera frames at 1 cm a frame;
- 48 caster frames at 2 mm a frame;
- 16 cut frames, a reset every two;
- 16 sky-step frames.

The renderer's per-frame GPU spans are read once per sequence, and p50s are reported.

**Variant screening.**
- Test builds read shader variants from `RENDERLAB_GI_SHADERS` (`program::gi_shader`), so one binary screens many WGSL variants, one process each.
- Naga checks of every variant ran on a renamed copy of the test binary outside `renderlab-work`, so no lease guard saw it.

**Profiling.**
- `RENDERLAB_M7_G1_PROFILING=0` drops the per-pass timestamp ring.
- With the ring on, frames are about 0.05 ms slower (M: B1 `a-base` 1.045 ms against `k-base-noprof` 0.996 ms, RT camera). Its pass times are for attribution only.

**Nsight.**
- GPU Trace with the real-time shader profiler: 200 ms windows inside camera motion (`RENDERLAB_M7_G1_PROFILE_MODE=camera`), and inside 1/16-rotation frames for tracing (`rotation`).
- Analysis used ngfx-mcp with the N8 scripts, copied to `E:/renderlab-work/m7-g1b/scripts`.

**Conditions.**
- Screens A1, B1, C2 and C3 are functional leases, not quiet windows. Visit-to-visit noise is about ±0.05 ms, and RT cache-off visits are bimodal (0.31-0.32 or 0.38-0.43 ms).
- A foreign CPU build (`stillkit`, about 30 rustc processes) ran from 20:28:11Z. Screen C3's timed steps ended at 20:27:44Z. Lease F1 ran entirely beside it, so its timings are **contended** and none is used.
- The cost claim is quiet window `q2`, started after stillkit's compilers had been gone for 60 s.

### 1.2 What the profiles showed (M, Nsight, G1 code, RT camera motion)

**Probe, 176 µs.**
- 29% of its samples were two dependent global loads per unique cell, run serially in a lane loop over a 256-entry table:
  - the find (14.3%);
  - a read-before-write of the seen stamp (14.7%).
- Hashing was 7.5%, the tile and hit-record loads 7.2%, and the shared clear 3.7%.

**Shading, Lambert gate, 431 µs.**
- About half of the samples are the lookup, at 0.28 ms.
- Each bilinear corner paid two dependent round trips: the key word, then the second sector of the 64-byte record (10.3% + 9.2%, long scoreboard).
- The 9-coefficient loop was not unrolled (8.4%), and hashing took 5%.
- Shading runs 4x4 workgroups (primary-ray files), so latency hides poorly.

**Frame structure.**
- Each of 10 indirect dispatches per frame was preceded by wgpu's indirect-dispatch validation pass: 2.4 µs each, 24 µs a frame.
- The viewer's default `InstanceFlags` keeps that validation on, so it is a product cost.

**Non-RT setup, rotation frames.**
- 85% of its samples are software BVH traversal for the canonical probe.
- Up to five candidates were traced serially per cell. About 5% of cells are orphans that try all five, so most warps waited on their slowest cell.
- Writing 64 ray items per cell took 9%.

**After v1:** the probe (122 µs) is ALU- and barrier-bound. The stalls are Wait 21%, Barrier 15%, Short scoreboard 10% and Long scoreboard 10%.

### 1.3 Per step (M)

**Columns.**
- "Lookup" is shading with the cache minus shading without it.
- "Frame" is the camera-moving increment over the cache-off frame, as a range over visits, without timestamps.
- Pass times are profiled p50s from the first visit.

| Step | Change | Probe µs | Lookup ms | RT frame | Non-RT frame | Verdict |
|---|---|---:|---:|---:|---:|---|
| G1 base | — | 184-186 | 0.27-0.28 | +0.58-0.68 | +0.72-0.80 | — |
| L1a | Fetch all four corners' records, then compare | — | 0.17-0.19, but cache-off shading +0.05 ms | — | — | No: register pressure slows the whole program |
| **L1b** | Each corner's 64-byte record loads whole before its key compare, in four vec4 loads; the same arithmetic | — | **0.18-0.20** | — | — | **Yes** |
| L1c, L2 | Corners in pairs; basis in a register struct | — | +0.022 / +0.009 over L1b | — | — | No |
| **v1** | L1b, plus 16x16 probe tiles with a compact cell list, an unconditional seen store, request-table clears by list (no 8 MiB clear a frame) and unencoded empty scan/integrate passes | **126** | 0.18-0.20 | +0.43-0.48 | +0.56-0.61 | Yes, except v1's 64-lane finish: caster finish 382 → 922 µs, cut 648 → 1,653, sky-step integrate 685 → 1,633 |
| **v2** | Direct fixed-grid passes that read their counts in the kernel: no indirect dispatch, so no validation passes, and no argument passes | 125 | 0.18-0.20 | **+0.41-0.43** | +0.55-0.60 | Yes. Claims and resolves 42 → 19 µs. Its 16-lane finish is still slower than G1's at large counts (caster 496 µs) |
| **F4** | Four lanes per cell, 16 cells per workgroup, fixed tree sum | 125 | — | — | — | **Yes.** The best finish at every count: camera 26 µs, caster 444, cut 742. RT caster frame 1.51 ms (G1 1.64), sky step 1.56 (1.73). F1 (one lane per cell) reads 63-71 µs on camera frames |
| **L4** | Bucket start offset from key a's top bits, so a lookup hashes key a alone | — | **−0.021** | — | — | **Yes**, on both paths |
| **S1** | Software rays visit BVH children nearest first | — | — | — | −0.010 (setup 134 → 128, rays 53 → 49 µs) | **Yes.** Identical image hashes |
| **v3** | Setup with eight lanes per cell: candidates in parallel, items written by lanes. Plus the constant sky at lookup, the probe reusing its tile key with a 256-entry tile table, and the cut refill with a neighbour fill | 125-140 | 0.13-0.18 | +0.37-0.41 | +0.39-0.65 (noisy) | Setup 24 → 14 (RT) and 133 → 55 µs (non-RT). **Regression:** cuts without the refill, setup 453 → 1,044 (RT) and 2,983 → 7,299 µs (non-RT) |
| **v4 (final)** | Lane 0 tries candidate 0 first; lanes 1-4 trace the other candidates only when it fails. Rays past the item capacity are spread over the cell's eight lanes. Setup grid 4,096 | ⟨Q2⟩ | ⟨Q2⟩ | **⟨Q2⟩** | **⟨Q2⟩** | Table hashes equal v3's (`3000682f6ad7f495` RT, `ad7d818201c12a39` non-RT) |

### 1.4 Final cost, quiet window `q2` (M)

⟨Q2 table: per phase (static, camera, caster, cut, sky step) p50 and p95 for off, G1 base and final, both paths; the final's pass breakdown; the standard program's row; the rotating map row⟩

**Against the budget.**
- Root's stricter reading is about 0.45 ms (0.3 × Lumen over TSR, RT) and 0.46 ms (L_sw/2, non-RT).
- ⟨Q2 verdict⟩
- These are SV costs. Lumen's L is measured on the quarry, and a same-bench cost waits for the quarry harness (section 6).

## 2. Cut spike (item 2)

**Cause (M, G1 code).**
- A cut makes every visible cell new: 129,000 cells × 64 directions = 8.3 M rays, against 2^20 parallel ray items.
- On non-RT, setup traced the rest serially per cell, 3.0-3.3 ms of the 5.7-5.8 ms cut frame.

### 2.1 Cut refill (`IndirectSettings::cut_phases`)

**What it does.**
- For the S frames from a reset, each queued cell traces one fixed direction group, k mod S == g: the first group with untraced directions.
- Partially traced cells re-queue through the scan for those frames and one frame more. A cell made late in the window then traces everything left.

**Untraced directions.**
- An untraced direction takes the visibility bit of its nearest traced direction (`gi_open`).
- A generated table holds each direction's eight nearest directions, with a full search as the fallback (`neighbour_table`), so the fill costs one to three checks.
- Weights stay fixed at 2π/K. **No count of known directions normalizes anything** (W1, C8).

**Defaults and capacity.**
- `cut_phases` 0, the default, picks 4 on software rays and 1 (off) on hardware rays.
- Ray items grow to 2^21 (+4.1 MiB), so a cut's first group fits: 129,000 × 16 = 2.06 M.

**Why the RT path keeps it off (M, screens C2 and C3).**
- On RT the refill saves only about 0.2 ms at a cut: 2.59 → 2.39 ms with F4.
- Its first frame at the gaps pose, 9.98%, is above Lumen HW's first cut into gaps, 9.51%.

**Quality after a reset (M, F1, the default rotated set; overview / gaps).**

| Frame after the reset | No refill (RT default) | Refill, S = 4 (non-RT default) | Lumen HW Epic, first cut | Lumen SW Epic mesh SDF, first cut |
|---|---:|---:|---:|---:|
| 1 | 1.95 / 2.75 | 6.82 / 9.98 | 10.59 / 9.51 | 49.56 / 101.05 |
| 2 | 1.95 / 2.75 | 4.30 / 6.48 | — | — |
| 5 (settled) | 1.95 / 2.75 | 1.95 / 2.75 (the same images) | — | — |

**How the cuts differ.**
- Lumen's cut is a camera cut that clears TSR history with its world state warm.
- G1b's empties the whole cache in place, a cold start, which is the harsher test.

### 2.2 Cut cost (M)

⟨Q2 table: cut frame p50/p95 per path for G1 base, final without refill and final default; the steady camera frame for the 1.5x rule⟩

**Profiled breakdown (screens).**
- **Non-RT with the refill:** setup 342-353 µs, rays 500-547, finish 775, probe 260-280.
- **RT without the refill:** probe 260, claims and resolves 110, setup ⟨Q2⟩, rays 177, finish 738.

**The finish dominates.** A cut needs every new cell's 64-direction SH projection in its first frame. The refill spreads rays, not projections.

**Remaining spike (miss).** No cut frame is within 1.5x of the steady frame.
- The cut is recorded as a registered cold start, as G1 recorded it.
- Next levers, not tried:
  - a precomputed local-frame SH table per direction for constant skies, a sum of constant vectors plus one L2 rotation;
  - parent-first creation (it needs a P0 reading, because it schedules by level).

## 3. Rotating environment (item 5)

**Test.** `section("environment")` uploads a directional prepared map:
- SV's sky on average;
- a lobe toward one azimuth, dim below the horizon;
- a brute-force cosine-convolved irradiance cube.

It then yaws the map 15° a frame, six steps, comparing each stepped frame with a fresh cache at the same yaw. A continuous 1°-a-frame rotation is then timed.

**Results (M, F1 equality; `q2` cost).**

| Measure | RT | Non-RT |
|---|---:|---:|
| Stepped frame equals fresh, six steps | **yes, 0 pixels** each step | **yes, 0 pixels** each step |
| Pixels the yaw changes against open sky | 1,323,093-1,323,125 | same |
| Cells re-integrated per step | 87,490 | 87,483 |
| Rotating frame p50 / p95, ms | ⟨Q2⟩ | ⟨Q2⟩ |
| Integrate pass p50, µs | ⟨Q2⟩ | ⟨Q2⟩ |

**Constant skies.** A constant sky's change now costs nothing.
- Cells cache unit radiance, and the lookup multiplies by the current constant (`gi_sky_scale`).
- So a constant-sky step re-integrates nothing. The sky-step frame equals a static frame (M, C3: RT 0.45-0.57 ms, non-RT 0.68 ms, against 1.56-1.73 and 1.76-1.79).
- A map's intensity was already applied at lookup.

**Rotating maps (miss).** A yaw re-integrates every cell with 64 prefiltered-cube fetches per cell, 5.6 M fetches on SV.
- The cost is exact and has 0 frames of lag, but it is far over the budget.
- Mitigations, not tried:
  - re-integrate only cells seen this frame, and the rest when next seen;
  - spread the re-integration under the 8-frame settle rule. Lumen itself lags seconds, but each frame then mixes yaws.

**Not tested.** The reference tracer on main has only a constant sky (`reference.rs`, outside G1b's files), so rotating-map error against a path-traced map is not measured.

## 4. SV gate against Unreal (both paths)

**Comparator.**
- M7-U0b `35a2759`: `docs/m7-u0b.md` and `E:/renderlab-work/m7-u0b/sv-sky/{g1-comparator.json,b/g1-comparator.json}`, extracted by `E:/renderlab-work/m7-g1b/scripts/ue_rows.py`.
- U0b uses G1's estimator: max depth 2, specular 0, 2,048 spp, the floored metric, the edge mask, and the same cameras and cuts.
- Unreal settings: radiosity 0, reflections 0, D3D12 SM6, PreExposureOverride 0, TSR Epic, screen traces at Epic defaults.

**Comparators per path.**
- RT: Lumen HW Epic.
- Non-RT: Lumen SW Epic with mesh SDFs and runtime RT off. The global-DF row is supplementary.

**G1b rows.** From F1 (final code, deterministic): the default rotated set, 64 directions, 4 px cells. The non-RT default's settled image is the same image, with identical hashes.

### 4.1 Static, per population (relative RMS %, overview / gaps)

| Population | G1b RT | G1b non-RT | Lumen HW Epic | Lumen SW Epic, mesh SDF | Lumen SW Epic, global DF |
|---|---:|---:|---:|---:|---:|
| frame | **1.95 / 2.75** | **1.95 / 2.75** | 10.71 / 11.16 | 38.91 / 87.18 | 35.92 / 68.31 |
| open | 1.77 / 2.02 | 1.77 / 2.02 | 7.95 / 6.27 | 34.14 / 30.42 | 33.64 / 27.34 |
| crease-90 | 2.71 / 2.74 | 2.71 / 2.74 | 18.51 / 5.40 | 33.89 / 61.07 | 27.36 / 41.49 |
| wedge-30 | 3.19 / — | 3.19 / — | 24.83 / — | 45.77 / — | 33.57 / — |
| underside | 2.62 / 3.43 | 2.62 / 3.43 | 15.71 / 11.07 | 34.38 / 88.98 | 31.93 / 77.60 |
| gaps | 3.73 / 4.37 | 3.73 / 4.37 | 30.52 / 20.81 | 111.14 / 175.94 | 84.66 / 135.65 |
| contact | 1.87 / — | 1.87 / — | 14.40 / — | 32.93 / — | 22.85 / — |
| ground near crease-90 | 1.87 / 2.17 | 1.87 / 2.17 | 6.08 / 7.43 | 10.00 / 29.63 | 8.43 / 22.46 |
| ground near wedge-30 | 3.62 / 1.25 | 3.62 / 1.25 | 17.14 / 6.24 | 93.41 / 6.93 | 89.85 / 4.22 |
| ground near underside | 2.22 / 2.24 | 2.22 / 2.24 | 10.78 / 9.80 | 50.48 / 69.62 | 63.01 / 78.02 |
| ground near gaps | 2.03 / 2.22 | 2.03 / 2.22 | 6.19 / 5.67 | 25.13 / 29.74 | 22.18 / 23.55 |
| ground near contact | 1.29 / 1.54 | 1.29 / 1.54 | 4.66 / 4.84 | 5.08 / 6.02 | 4.85 / 3.13 |
| crevice pixels (frame) | 4.21 / 3.93 | 4.21 / 3.93 | 23.78 / 16.10 | 96.93 / 137.23 | 89.44 / 107.57 |
| p99 (frame) | 6.38 / 8.99 | 6.37 / 8.99 | 31.51 / 41.44 | 123.43 / 172.99 | 105.65 / 136.04 |
| over-occluded open pixels | **0 / 0** of 588,963 / 233,038 | **0 / 0** | 407,581 / 226,107 | 112,027 / 139,633 | 176,102 / 155,105 |
| **edges (anti-aliasing)** | **45.83 / 34.42** | **45.83 / 34.43** | 26.85 / 18.39 | 61.13 / 91.94 | 52.96 / 73.65 |

A dash means no valid visible samples at that pose.

- **The populations match U0b's.** The frame has 1,230,320 / 1,433,922 pixels, U0b 1,230,314 / 1,433,922. Open has 588,963 / 233,038 on both sides. Edges have 32,057 / 30,650, U0b 32,057 / 30,653.
- **The controls agree.** G1b's cache-off rows read 67.65 / 128.79%; U0b's Lumen-off control reads 68.20 / 129.72%.

**Verdict (M).** G1b beats both comparators on every lighting population on both paths. The smallest margin is 2.0x (gaps-pose crease-90: 2.74% against Lumen HW's 5.40%); the largest is 40x (the gaps population against Lumen SW with mesh SDFs).

**The edge population is a miss.**
- Edge pixels are scored with the single camera sample at the pixel centre, against a pixel-integrated reference.
- Lumen's frame is TSR Epic's output, and G1b's has no temporal reconstruction yet.
- That is TR1's measure, so G1b does not claim it. It stays a recorded loss until TR1, and **the Unreal floor is met on every measure except edges.**

### 4.2 First frame and motion (overview / gaps)

| Measure | G1b RT | G1b non-RT | Lumen HW Epic | Lumen SW Epic, mesh SDF |
|---|---:|---:|---:|---:|
| First frame after a cut | 1.95 / 2.75 (cold, no refill) | 6.82 / 9.98 (cold, refill S = 4) | 10.59 / 9.51 (incoming cut, warm world) | 49.56 / 101.05 |
| Pan residual change, mean / max, cold start | **8.55 / 10.43** | **8.55 / 10.44** | — | — |
| Pan residual change, mean / max, non-RT default refill, cold start | — | ⟨Q2 motion2⟩ | — | — |
| Pan residual change, mean / max, warm (U0b's protocol) | ⟨Q2⟩ | ⟨Q2⟩ | 20.18 / 45.84 | 166.17 / 191.52 |

**Pan definition.** The 9-frame pan at the gaps pose moves 2 cm a frame. The value is the frame-to-frame change of the floored relative residual over covered non-edge pixels, as in bench-motion.
- **Cache-off control:** 244.26 / 285.91 here, 245.58 / 287.43 in U0b's control, so the two metrics agree.
- **Warm against cold:** U0b's pan starts from warm Lumen and TSR histories. G1b's original pan empties the cache first, a harder start.

### 4.3 Correction to G1's scores (M)

**Cause.**
- The trace leaves the hit records of uncovered trace tiles stale.
- G1's harness read the raw hit texture, so a pose scored after another pose counted that pose's stale records in sky tiles as surface pixels.
- The scene's sky shows there (clear colour against visible sky), so those pixels scored about zero error.

**Effect.**
- At the overview pose about 323,000 extra pixels were counted (frame 1,553,565, edges 46,849).
- Only the first configuration of each path was scored correctly, because no other pose had run yet.
- Gaps-pose counts were unaffected.

**Fix.** `Rig::placements` now counts a pixel only inside the traced region, in a tile whose coverage flag is set.

**Corrected overview numbers.**

| G1 configuration | G1 reported | Corrected |
|---|---:|---:|
| Default, rotated | 1.64% | **1.95%** |
| Unrotated | 1.93% | 2.25% |
| Level blend, unrotated | 1.91% | 2.23% |
| 128 directions, rotated, blend | 1.23% | 1.51% |
| Nearest | 2.24% | 2.53% |
| 2 px | 1.91% | 2.71% RT, 2.29% non-RT; it overflows the item capacity and buckets, so it differs between paths and versions (gaps 4.78% / 4.75%, G1 3.90%) |
| 8 px | 9.76% | 18.27% |
| Cache off | 67.65% | 67.65% (it was scored first) |

The gaps-pose column of G1's table stands.

## 5. Gates (M, F1 final code, both paths)

**Determinism.**
- Repeating a reset sequence gives equal tables and images in process.
- Two processes (F1 `gates-a`, `gates-b`) give the same table and image hashes:
  - RT `3000682f6ad7f495` / `9478be32f6dcb6c0`;
  - non-RT `ad7d818201c12a39` / `bb2abe5f18d6da75`.
- Serial tracing gives the identical table.

**Caster and staleness.**
- Ten frames of sphere motion end byte-identical to a fresh cache: 0 pixels differ.
- Fixed-rotation staleness reaches 0 differing pixels at age 7.

**Sky steps.** Constant and map steps equal a fresh cache on the next frame (sections 2-3).

**Non-callers.** Toon with the cache enabled runs no cache pass and draws byte-identical images.

**Programmability.** The hit record is unchanged (16 B). The lookup ABI is unchanged: `indirect_irradiance(input, n)`.

**Suites.**
- `cargo test -p renderlab --lib` exits 0, run inside lease F1: 129 passed, 123 ignored.
- `cargo fmt --check` is clean.
- Clippy reports nothing in G1b's files. It reports five pre-existing warnings in `virtual_shadow_gpu.rs` and `visibility_buffer.rs`, which this branch does not change.

## 6. Quarry: near field and gate (items 3 and 4, not done)

**Not built:**
- the cook-time short-range occlusion;
- a quarry G1 harness;
- the quarry gate.

**Why.**
- **The quarry comparator is still to come from U0b** (root, 2026-11-07).
- U0b's retained quarry references are full-depth: sky plus multi-bounce, with a prepared RLE2 sky (`docs/m7-u0b.md`, "Retained quarry references"). They do not isolate G1's no-bounce sky term.
- The reference tracer on main renders only a constant sky.
- A depth-2, sky-only quarry reference with the prepared sky is the input the gate needs.

**Plan, in order.**
1. A quarry G1 harness reusing the full-resident prepared-asset loader (the pattern of `fresh_shadow_tests::load_quarry`), timing both paths at quarry poses 0/1.
2. **Non-RT far field:** R0c's cooked cluster BVHs through `set_indirect_bvh`, starting rays beyond the near radius (R0c's `select_cone_level`).
3. **Near field, G-N1.** A cook step writes each source vertex's K fixed-direction visibility within R_near against static geometry (8-16 B a vertex: 50-110 MB for the quarry's source vertices, inside the approved 50-150 MB). The cell's direction k reads it as the AND of its triangle's three vertex masks for the nearest cook direction. Movers keep their per-instance software BLASes.
4. Score both paths against U0b's depth-2 sky reference and Lumen Epic sky-only quarry rows once on main, then run the cost gate on the quarry, where Lumen's L was measured.

## 7. Memory (M)

| Item | Bytes |
|---|---:|
| Table: 2^19 slots × 160 B, plus header | 83,886,336 (80.0 MiB) |
| Work: claims, misses, queues, 2^21 ray items and staged bits, request table | 35,913,792 (34.25 MiB) |
| Non-RT SV geometry | 137,328 |
| **WRC total** | **114.4 MiB** (G1 110.1; under the 160 MiB stop) |

Against Unreal (M7-U0, LocalUsedMB):
- RT: 2,641 + 114 = 2,755 MiB, against 6,444 MiB for Lumen HW Epic.
- Non-RT: 3,503.7 + 114.4 + 29.6 (R0c's quarry BVH) ≈ 3,648 MiB, against 5,142 MiB.

These are buffer sums, not allocator residency (I).

## 8. Claim-avoidance notes (engineering reading, not clearance)

| Change | Constraint | As built |
|---|---|---|
| Cut refill | W1, C8 | Per-direction slots with fixed weights 2π/K. Untraced directions take the bit of a fixed nearest traced direction; no count of known directions scales or averages anything. Values are replaced, never blended |
| Cut refill schedule | W2 | Driven by frames since a reset and fixed direction groups (k mod S). Every new cell gets its query point and first group in the frame it appears. Nothing ranks cells |
| Eight-lane setup | W2 | The same five canonical candidates in the same fixed order, so the key alone still decides the point (table hashes equal across v3/v4 and the serial form) |
| Nearest-first software traversal | W8 | Per-ray, from the top; child order by entry distance within one ray's traversal. No per-node hit lists or launches |
| Direct fixed grids, multi-lane finish | C4, C1 | Claims stay atomicMax-and-winner-writes. The tree sums are fixed-order. No resampling |
| Single-hash start, constant sky at lookup | W3 | Lookups stay fixed-weight bilinear with no jitter |

No patented fallback is enabled.

## 9. Misses and open items

1. **Edge population against Lumen HW Epic** (anti-aliasing, TR1's). Every other SV measure passes on both paths.
2. ⟨Q2 cost verdict against 0.45 / 0.46 ms⟩. These are SV costs; the quarry cost is open.
3. **Cut frames above 1.5x steady** on both paths. They are recorded as a cold start (section 2.2).
4. **A rotating prepared map costs ⟨Q2⟩ a frame** (section 3). Rotating-map error against a path-traced map is untested.
5. **The quarry near field and gate are not built** (section 6).
6. **2 px cells:** first frame at the gaps pose is 21-23%, against 4.8% settled. A 2 px cut overflows the item capacity and buckets. It is not a default.
7. **Bucket-full inserts grow as the table fills** (M, G1's motion run: 61 → 294 a frame). A larger table or wider buckets would remove them; the quarry will show the size needed.
8. **Not measured in a quiet window:** F1's environment and motion step costs ran beside stillkit, and Q2 re-measures the rotation.

## 10. Evidence and process

**Location.** Evidence is in `E:/renderlab-work/m7-g1b/runs/<label>/`, with `g1-sv.json` per step, `plan.json`, process records and contention notes.

**Runs.**
- **A1:** base against L1a/L1b, and Nsight traces of G1 code.
- **B1:** v1, L1c/L2/L3, gates, no-profiling timing, traces.
- **C2:** v2, F1/F4, L4/L4F4, S1, the cut refill, the rotating environment.
- **C3:** v3.
- **F1:** final functional, contended.
- **Q2:** the final quiet window.
- **C1:** stopped at its first step on a WGSL reserved word (`active`). Naga checks then moved outside leases.

**Other files.**
- Analysis: `analysis/` (hotspots, stall legends). Unreal rows: `ue-rows.json`.
- Variant shader sets: `variants/`. Build and lease scripts: `scripts/` (`build.sh`, `lease-force.sh`, `quiet-after-idle.sh`, `naga.sh`, `summ2.py`, `ue_rows.py`).
- Raw traces and staged binaries are deleted at close.

**Process.**
- Two Cargo jobs beside other owners' functional leases, never inside a quiet window (`build --beside-leases`).
- Lease C2 waited 14 minutes while two owners reacquired back to back. It then took the lock at its first free moment (`--force-schedule`), as recorded in `gpu-schedule.md`.

**Reproduce.**

```powershell
py -3 -B scripts/m7-g1b-run.py build --as NAME [--beside-leases]
py -3 -B scripts/m7-g1b-run.py plan <label> plan.json --minutes 10 [--quiet] [--force-schedule]
```

Plan steps take `sections`, `configs`, `visits`, `gate`, `shaders` (a variant directory), `env`, `lib`, `test` and `command`. The harness reads `RENDERLAB_M7_G1_{TIMING,TIMING_PROGRAM,PROFILING,PROFILE_FRAMES,PROFILE_MODE}`.
