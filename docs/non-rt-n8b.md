# N8b: N8's quality misses root-caused and fixed; cost levers (non-RT Interactive)

Follow-up unit to [N8](non-rt-n8.md), 2026-11-06 UTC, base `aee5007`, branch `t3/n8b-virtual-shadow-quality`. Bar (Islam, 2026-11-06): Unreal's quality on every measure at about Unreal's cost. Labels: **M** measured here, **I** inference.

## Result

**Quality: N8b now beats Unreal VSM on every measured quantity of S2, S3 and the quarry, and beats N8 on every measure of all four benches. One miss remains: S1 pose 1 has 2 umbra leaks against VSM's 0. Cost: worse than N8 and well above Unreal's. The lookup now does exact work where N8 guessed, and the cost levers delivered little (horizon) or were not reached (caster LOD, panning, static/dynamic split).**

- **Lit receivers wrongly darkened on the quarry: 8,477 → 0** (VSM 87). Umbra leaks 2,528 → 2,438 (VSM 14,935), solid flips 6,470 → 497, lost umbra edges 901 → 484, partial RMS .298 → .198 (VSM .483).
- **S2 moving-plate leaks: 150 → 0 per frame** (VSM 62); flips 4-6 → 0; lost contacts 64-106 → 0; partial RMS .074-.080 → .073-.075.
- **S1: leaks 3/3 → 0/2** (VSM 0/0); partial RMS .0828/.0809 → .0816/.0802 (RT Interactive .0878).
- **S3** improves too: partial RMS .106-.113 → .076-.078, the level of RT Interactive (.0775) on the same eight directions; flips 46-69 → 0; lost contacts 0-56 → 0.
- **Cost (M):** quarry shadow 5.95 ms static (N8 1.8-4.0, VSM 0.50) and 24.0 ms with the sun turning (N8 16-17, VSM about 0.95); S2 0.59-0.86 ms (VSM 0.35); S3 1.68-1.95 ms (VSM 0.36-0.59). **All cost targets are missed** (quarry ≤ 1 ms static, ≤ 2 ms turning; S2/S3 ≤ VSM).
- A higher-quality setting, `peel_own`, takes the quarry further (leaks 987, flips 280, partial RMS .168) for 8.2 ms static.

## Root causes (M)

Every decision of N8's lookup was traced per pixel and direction (`VSM_DEBUG`, `scripts/n8-diagnose.py`) against hardware Interactive on the same eight directions. Where Interactive lights a receiver fully, every direction N8 blocks is wrong; where it shadows fully, every direction N8 lights is wrong. Each wrong direction was attributed to the decision that made it.

| Miss | First wrong decision (trace evidence) | Fix |
|---|---|---|
| Quarry lit darkened (8.5k) | **The texel-column thickness test.** All 9,257 of 9,258 falsely darkened pixels (vs Interactive), 50,363 directions. 50,170 fired with the ray's xy outside the texel-centre top triangle, 40,218 at the first sample. On scans, triangles are smaller than texels: texel-centre heights of the receiver's own relief, and of overhang silhouettes, put the ray "inside a solid" whose underside lies below the receiver. | Removed. Every map candidate is decided exactly; the second layer is a peel (below). |
| S1 false shadows (716/500 px vs Interactive) | The same thickness test, in its range-overlap form: it fired with the ray **above** the texel's top surface (897/897 directions), between two separate fold quads. | Same. |
| Quarry leaks once exact-only | **The march's reach cap.** A level's march stops at 124 texels / disk slope (about 1.5 m of light-space height on the finest level). A distant occluder is then tested only at samples far from where the ray crosses it, so its exact test misses. The thickness test had been hiding this, catching distant solids and making the false shadows at the same time. Neither 3x the samples nor a 2x finer map changed the leaks (11.4k both). | The march continues through every coarser level up to the highest caster, with those levels' pages requested. A walk to the crossing then confirms exactly (below). |
| Sub-texel candidates (quarry) | The texel-centre triangle and its 3 edge neighbours are often not the triangle above the ray (13.1k leaking pixels exact-only). | Candidates are walked across the exact-position adjacency to the triangle under the ray's light-space xy. If the ray is below that plane, the crossing is predicted along its drift and walked to again (3 refinements), testing every visited triangle exactly. A continuity rule covers crossings beyond the walk (below). |
| S2 moving-plate leaks (150) | **Hidden occluder.** The reveal plate is above the shadow blocker in light space, so the single top layer hides the blocker exactly where the plate's footprint sweeps over the blocker's shadow. The leaking rays pass under the plate's edge (212 of 227 pixels: "under the plate, exact miss") and are really blocked by the blocker, which is never a candidate. Not page staleness: invalidation was correct, and the leaks follow the plate's position, including at rest. | The back-face layer becomes a **peel layer**: the highest surface at each texel centre other than the top triangle, with its triangle id, decided like the top. |
| S1 leaks (3) | (a) **Other-mesh contacts inside `near`:** floor receivers under strips 0.2 mm above them. The ray crosses the strip at about 0.6 mm, but the far march started at `near` = 2 mm and its exact tests used tmin = `near`; only the receiver's own 1-ring was tested closer. (b) Stacked/fold occluders hidden under another surface (same as S2). | The march's first step starts at the ray origin, exact tests use the receiver's epsilon, and the first step tests the 2x2 texel block around the sample. The peel layer covers (b). |
| S3 (regression found on the way) | An unconditional continuity rule blocked rays grazing smooth crests at the 15° sun (936 flips). | Continuity only when the predicted crossing lies more than 1.5 spans of the located triangle beyond the sample and the walk ran out of reach there; never at open edges, folds or the receiver's own triangle/1-ring. |

**S1 pose 1, 2 leaks (not fixed, M):** two floor pixels at the umbra edge of a strip 0.2 mm above the floor, each with one of eight directions lit. Their rays cross the strip at about 0.6 mm, within a texel of its edge, and no texel centre of the 2x2 first-step block lies on the strip there. Centring the block on the ray origin instead did not remove them. A sub-texel near-contact candidate test (e.g. conservative raster of the id layers) is the next lever (I).

## What changed (M)

`shaders/virtual_shadow.wgsl`, `src/virtual_shadow_gpu.rs` (and tests/scripts):

- **Exact confirmation only.** The map gives candidate triangles; no texel-height decision blocks a ray. The only non-exact decision is the continuity rule above: a ray still exactly below an interior surface triangle after the refinements, with the predicted crossing beyond what was tested, is taken to cross that continuous surface.
- **Two layers, top and peel**, each with heights and triangle ids (the peel pass discards the texel's top triangle and samples the top ids). Memory: 16 B per texel per level (was 12).
- **Peel scope** (`VirtualShadowSettings::peel_own`, default false): by default, a peel surface of the same placement as the top is treated as a fold of it and not walked. Folds are common on scans and at grazing suns, and the top walk decides their crossings. `peel_own` also walks them (scan overhangs).
- **The march** covers [0, highest caster] through all coarser levels (2 samples per extra level), and the mark pass requests those levels' pages.
- **Ring horizons** (lever 1): `vsm_horizon` computes once per draw list, per triangle and side, the exact steepest elevation at which a ray from the triangle can reach any 1-ring triangle (the projection of the normal onto the cone of ring-minus-triangle vectors; 90° when they overlap with the ring above). The lookup skips the fan walk when all eight directions rise more steeply. It is conservative (counts identical with it on and off) and applies only to similarity-transformed placements. 4 B per triangle.
- Diagnostics: `VSM_DEBUG` trace, `gpu_*_virtual_diagnose`, `gpu_*_virtual_screen` (variant screens with lookup override constants), `scripts/n8-diagnose.py`.
- Removed: the back-face layer and the thickness test, and with them `SHADER_PER_VERTEX` from `VIRTUAL_FEATURES`.

## Benchmarks (M)

Same scorers, references and recorded Unreal rows as N8. S2/S3 score against N7a's regenerated film references (hardlinked; the harness skips regeneration). The quarry scores against N8's method of a 64-seed traced reference, rebuilt once on today's receivers. Build: development/test profile (renderlab opt-level 1), Vulkan validation layer on, RTX 4070 SUPER, NVIDIA 617.14. Final binary SHA-256 in `E:/renderlab-work/n8b/final-binary.sha256`. Its embedded shader was checked byte-identical to the working tree's, and only the shader's header comment was edited afterwards.

**S1** (static; N8b is identical across environments):

| Pose | Mode | Contact loss | Leaks | Partial RMS | Solid flips |
|---|---|---:|---:|---:|---:|
| 0 | N8 | 0/419 | 3 | .0828 | 0 |
| 0 | **N8b** | **0/419** | **0** | **.0816** | **0** |
| 0 | UE VSM (black / env) | 5 / 6 | 0 / 0 | .1642 / .1644 | 0 / 1 |
| 1 | N8 | 0/335 | 3 | .0809 | 0 |
| 1 | **N8b** | **0/335** | 2 | **.0802** | **0** |
| 1 | UE VSM (black / env) | 2 / 3 | **0** / 0 | .1628 / .1620 | 0 / 0 |

**S2, disk 0.04 rad** (N8 → N8b; UE VSM recorded):

| Case / phase | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / lost / leaks / flips / shadow ms) |
|---|---|---|---|---|---|---|---|
| camera, out | .0790 → **.0746** | .0106 → .0105 | 106 → **0** | 150 → **0** | 4 → **0** | 0.45 → 0.86 | .1752 / 2,101 / 62 / 167 / **0.35** |
| camera, hold | .0799 → **.0750** | .0113 → .0112 | 64 → **0** | 13 → **0** | 2 → **0** | 0.37 → 0.79 | .1758 / 1,039 / 57 / 152 / **0.35** |
| camera, return | .0778 → **.0748** | .0102 → .0101 | 87 → **0** | 151 → **0** | 6 → **0** | 0.44 → 0.85 | .1739 / 1,972 / 62 / 155 / **0.35** |
| shadow step, out | .0745 → **.0735** | .0099 → .0099 | 64 → **0** | 2 → **0** | 0 → 0 | 0.42 → 0.66 | .1697 / 2,070 / 65 / 120 / **0.36** |
| shadow step, hold | .0739 → **.0727** | .0099 → .0099 | 64 → **0** | 0 → 0 | 0 → 0 | 0.33 → 0.59 | .1675 / 1,059 / 66 / 119 / **0.35** |

Whole-frame p50 is 0.77-1.04 ms against Unreal's 2.12-2.18 ms, mostly because the primaries are still traced.

**S3, 525 k triangles, 15° sun:**

| Phase | Partial RMS | HDR RMS | Lost contacts | Max leaks | Max flips | Shadow p50 ms | UE VSM (pRMS / lost / flips / shadow ms) |
|---|---|---|---|---|---|---|---|
| static | .1060 → **.0778** | .0154 → **.0142** | 0 → **0** | 0 → 0 | 48 → **0** | 0.66 → 1.85 | .1878 / 2,929 / 187 / **0.37** |
| rotate 0.25°/s | .1072 → **.0783** | .0156 → **.0141** | 14 → **0** | 0 → 0 | 62 → **0** | 0.84 → 1.95 | .1875 / 11,630 / 190 / **0.59** |
| rotate 1°/s | .1073 → **.0782** | .0156 → **.0141** | 56 → **0** | 0 → 0 | 69 → **0** | 0.85 → 1.92 | .1863 / 12,289 / 192 / **0.59** |
| jump recovery | .1129 → **.0762** | .0156 → **.0138** | 0 → **0** | 0 → 0 | 46 → **0** | 0.67 → 1.88 | .1811 / 3,153 / 226 / **0.37** |

RT Interactive on the same directions scores .0775 (static).

**Quarry, 13 M triangles** (pose 0; pose 1 within 1% on every column):

| Mode | Partial RMS | Partial mean | G1 violations | G4 RMS | Solid flips | Umbra leaks | Lit darkened | Lost umbra edges |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| N8 | .298 | -.037 | 9,888 | .151 | 6,470 | 2,528 | 8,477 | 901 |
| **N8b (default)** | **.198** | +.076 | **9,141** | **.149** | **497** | **2,438** | **0** | **484** |
| N8b, `peel_own` | .168 | +.045 | 8,858 | .111 | 280 | 987 | 0 | 280 |
| RT Interactive | .080 | .000 | 8,139 | .033 | 0 | 0 | 0 | 0 |
| UE VSM native | .483 | +.350 | 11,019 | .351 | 12,390 | 14,935 | 87 | 11,884 |

The signed partial mean moves from -.037 to +.076, a light bias left by the remaining leaks, against VSM's +.350.

**Quarry cost (M; shadow p50 / p95 ms over 32 frames):**

| Mode | Static | Camera alternating 0/1 | Sun turning |
|---|---|---|---|
| N8 | 1.8-4.0 | 1.85-2.59 | 16.9-17.4 |
| **N8b (default)** | 5.95 / 9.17 | 6.00 / 9.59 | 24.0 / 26.1 |
| N8b, `peel_own` | 8.18 / 13.7 | 8.14 / 14.2 | 26.8 / 32.5 |
| UE VSM (recorded) | 0.50 | - | about 0.95 |

## Cost: where it goes, and the levers (M)

Variant screens on one process per bench (`gpu_*_virtual_screen`; counts against Interactive plus shadow p50; S2/S3 timings there vary ±0.3 ms between identical variants, the quarry's by ±0.05 ms). Quarry static decomposition, before the first-step block: about 0.5 ms base, 1.25 ms ring walk, 1.9 ms same-placement peel walks, 2.3 ms of walk refinements, about 1.5 ms of samples beyond 5 per direction. The first-step 2x2 block adds about 3 ms on the quarry and 0.8 ms on S3. In exchange it halves quarry leaks (6.0k → 2.9k vs Interactive) and makes S3 exact against Interactive (flips 16 → 0). The turning sun is page raster: 17-18 ms of its 24 ms.

1. **Ring horizon skip: done, small.** Exact and conservative. It saves 0.3 ms of the quarry's 1.25 ms ring walk (5.24 → 4.92 ms in the screen), and S2/S3 within noise. At millimetre scale one steep neighbour in a scan triangle's 1-ring forces the whole walk.
2. **Caster LOD for page rasterization: not done.** The quarry is an M3 prepared asset with no cooked cluster LOD (no RVM1 package exists for it), so there was nothing to rasterize coarse levels from. N1c (gpu-schedule.md, 10:50Z) is appending cooked cluster-LOD triangles after each mesh's source triangles, in the same index buffer. N8b's walk and peel ids address triangles by primitive index and read the adjacency at `(words-1)/2`, so they work unchanged on that layout. Rasterizing pages from the same cut as the primaries is the step that can bring a turning sun near Unreal's cost (I).
3. **Clipmap panning by page offset and a static/dynamic page split: not done.** On these benches they would change little: the quarry has no moving casters and its camera poses rarely move a window (camera-alternating costs the same as static). S2's movers are 32 triangles.
- **Tried and rejected:** marching all eight directions together, grouped by texel with one walk per group, was slower everywhere (S3 2.4 ms vs 1.2; quarry 6.3 vs 5.0).

## Misses, honestly

- **Cost is not at Unreal's on any bench.** The shadow pass is 1.7-2.5x VSM on S2, 3-5x on S3 and about 12x static (25x turning) on the quarry. It is also above N8 everywhere: exact decisions cost walks where N8 used texel heights.
- **S1 pose 1: 2 umbra leaks against VSM's 0** (root cause above).
- **Not film quality.** Partial error is still 1.0x (S3), 1.0x (S2) and 2.5x (quarry) RT Interactive's, which itself is below the film bar. N8b is never a default and is not presented as meeting the film gate.
- **The continuity rule is a judgement**, though a narrow one: it fires only for a ray exactly below an interior triangle whose crossing lies beyond the tested neighbourhood. It produced no false shadow on any lit receiver here (quarry lit darkened 0). Penumbra receivers can be over-darkened by it (I).
- **Still not a non-RT frame:** receivers and records come from traced primaries until N1b/N1c land.

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8b/`): `final/` (S1 `comparison.csv`, `scores.json`; S2/S3 `comparison.json/csv`, `per-frame.json`; `n8-comparison.json`), `final-q/` and `final-peel-own/` (quarry `scores-traced.json`, `quarry.json`), `score-k2/` (the first full scoring, including the rebuilt quarry reference), `batch*/diag/*/classes.json` (every trace classification), `score-*/screen/` (variant screens), and `runs/` (every lease's log, preflight and memory samples). All raw arrays (counts, traces, crops, references and staged test binaries; 5,060 files, about 16 GB) were deleted after summarizing; they are listed in `deleted-raw.txt`, and 86 MB of summaries and PNGs remain. N7a's hardlinked S2/S3 references are untouched.
- **Gate:** `cargo test -p renderlab --lib -j 2` passed by exit status 0: 101 passed, 0 failed, 104 ignored. `gpu_virtual_fixture` passed its cache asserts (static repeat identical and skipping page work; a moved box re-rasterizes only its pages). Against RT on the fixture's 15° sun, N8b agrees on 99.3% of receivers (N8 94.1%) with 10 flips (N8 3,098).
- **Leases:** 27 leases of 7-239 s (1,256 s in total) under the shared lock with the 4/2 GiB guard, between N1b's. Two quarry traces tripped the 2 GiB floor at asset upload while another worker's build ran, and were rerun once memory recovered. One lease ran `bash` from the wrong shell (exit 127), one hit a bind-group usage conflict in the first peel build, and one ran a sibling test through cargo's substring filter (now skipped). All were rerun.
- **Time (UTC):** 09:12-09:20 reading; 09:20-09:28 trace harness and first build; 09:28-10:07 root-cause traces and the fixes they drove (13 leases); 10:11-10:16 first full scoring; 10:16-10:36 cost decomposition, the rejected shared march and the continuity criterion (6 screens); 10:36-10:43 ring horizons; 10:43-10:47 peel scope and first-step block; 10:48-10:57 final scoring, an S1 retry and the gate; 10:57-11:05 record, cleanup and commit. Verification was 21 minutes of GPU leases plus about 8 minutes of CPU scoring; implementation took the rest, about 80 minutes.
