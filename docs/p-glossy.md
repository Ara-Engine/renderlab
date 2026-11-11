# P-GLOSSY: cheaper glossy reflections (parked)

Unit P-GLOSSY, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-11, branch `t3/p-glossy`. Briefs: `D:/Coding/renderlab-work/p-glossy/brief.md`, restated by `brief-2.md`. **Parked at the coordinator's stop (17:05 UTC) before its last lease ran. Verdict: DON'T MERGE yet** (section 5). Labels: **M** measured here, **C** read from code, **I** inference.

Hardware and builds: RTX 4070 SUPER, NVIDIA 617.14, Windows 11, shared PC (Ara integration and N-VSM units leasing the same GPU one at a time). Showcase runs through Ara's player and Ara's bridge (`tools/showcase/showcase.py`), with Ara worktree `ede758f7a` (pre-rewrite id), perf profile, 1920x1080 and the map sky. Times are per-pass medians of the last 60 frames from `RENDERLAB_PASS_LOG`. The matched scene is `examples/reflection_matched.rs --matched --stage --metal-balls`, release build at 1280x720, against its 16,384-spp path-traced reference. Pure main means `68e02ddc` built into the same Ara player. Lease D's GPU state: start P5, 480 MHz, 36 °C; end P3, 900 MHz, 44 °C.

## Result

| Step | Showcase shading | Picture | Verdict |
|---|---|---|---|
| Shared glossy strata: 4 traced strata a pixel, shared inside the shading group (`RENDERLAB_GLOSSY_SCREEN=traced=4`) | non-RT **11.4 ms** against pure main's **23.5-24.1** (lease D); RT **5.77** against **8.69** (lease B2) | Held and moving frames match the 16-strata base on every crop, both paths. On cuts, a faint soft mottle replaces base's sparse bright specks (section 3) | **DON'T MERGE yet.** It is not the default in the source, and the entry restructure it needs costs about 8-12 ms on the 16-strata path (section 4). |
| Rough band at one ray (the options study's bound) | non-RT 20.9 ms against 31.9-36.5 on the same branch; RT 6.70 | Visibly worse on the 0.42 aluminium ball: speckle on cuts, held flicker 2.5x | Bound only; not a candidate at one ray |
| Binned-SAH reflection BVH2 (`a05260e8`/`2e244f86`) | not measured | Hits are the same by construction; ties may differ | Built and unit-tested on the CPU. Not timed: lease F was withdrawn |

## 1. Where non-RT glossy time goes (M)

Showcase non-RT, still, branch `60ada122` at 16 strata (this branch's base carries the restructure cost of section 4, so read differences, not absolutes):

| Screen (`RENDERLAB_GLOSSY_SCREEN`) | Shading, ms | Difference |
|---|---:|---:|
| `traced=16` (base) | 31.35 | |
| `noshadow`: hit sun walks off (cost only) | 23.11 | −8.2: any-hit sun walks from hits |
| `noshade`: sky at every hit (cost only) | 19.60 | −11.8: hit shading including those walks |
| `mark4`: no glossy tracing at all (lease D) | 2.59 | the non-glossy remainder |
| `traced=4,noshare` | 10.90 | |

So with 16 strata, closest-hit walks plus reconstruction take about 17 ms, sun walks from hits about 8 ms and hit shading about 3.5 ms (I: differences, not additive). The matched scene has 17 traced instances over 3 meshes. The instance list is short; the walks are the cost.

**The rough band is most of Showcase's glossy pixels (M, `mark4` capture):** 65,985 traced glossy pixels at roughness 0.4 or above against 27,361 below, i.e. 71%. They sit in the fade band (0.35-0.5), where the traced weight is under 0.67, yet they pay the full count.

## 2. Shared glossy strata (method)

Each glossy receiver traces 4 of its footprint's strata. The cone (texture footprint and hit regularization) stays the 16-stratum one, so nothing passes by blurring (`footprint` and `count` are split in shaders/reflections.wgsl). Every sample goes to workgroup memory: the hit point (or a miss's direction), the radiance its own hit program shaded, and the density its lobe drew it with. After one barrier, each receiver adds the samples of 6 neighbours inside its 8x4 shading group (4x4 on mixed-program lists). The neighbours are picked per pixel and frame from the 5x5 window: a hashed start, stride 7 over its 24 cells, mirrored at the group edge. For each neighbour sample the receiver:

- reconnects to the neighbour's hit point from its own origin;
- weights the sample by its own lobe density over the sample's (pairwise, at most 2, with the squared-distance Jacobian);
- normalizes over all the samples it took.

Rules:
- The reflected image stays the receiver's own, since each sample is placed by its hit point.
- A rare bright hit spreads over neighbours with its energy kept, not clamped.
- Not reused: neighbours across a normal or depth edge, mirrors, below-plane directions, and reconnections with a distance ratio outside [0.5, 2].
- Hits are shaded only by their own program, so M8-HIT-CONTRACT is unchanged. Sharing changes which receiver averages a shaded value, not how a hit is shaded.
- Glass reflections (`tp_reflect`) keep every stratum and share nothing.

**Entries (C):** naga rejects a barrier after a non-uniform return, so every lane of a shading group now reaches the exchange in uniform control flow. `primary_shade_pixel(id, size, live, lane, width)` works on a `live` flag; `primary_shade_tiles` and `primary_shade_list` turn their per-pixel returns into `live=false`. src/film_accumulation.rs's seams follow: `film_distance` is taken where the hit's corners are in scope. The CPU compose and naga tests pass (`lib-compose-d.log`: 17 passed).

**For the ara-gpu backend:** this adds a resource kind the shading kernel has not used before: **workgroup memory** (`rl_share_point`, `rl_share_radiance`: 128 vec4 each; `rl_share_origin`, `rl_share_side`: 32 vec4 each; about 5 KB) and a `workgroupBarrier` in the traced shading entries. No new buffers, textures, samplers or device features.

Patent note (habit only, policy suspended): resampled reuse across pixels is the family of US 11,315,310; this has no reservoirs.

## 3. Pictures (M)

Matched scene, columns per crop: RMS against the reference at the cut (flight 0), moving (flight 47) and held (frozen pose, frame 47 against reference frame 16); white specks held (P-DEFECT's count); held flicker (mean frame step, frames 41-47); moving flicker (high-passed frame step, flight frames 25-32). Lease D (`runs/e/score-nonrt.json`, `score-rt.json`):

| Non-RT | turntable | aluminium (0.42) | copper | gold |
|---|---|---|---|---|
| base 16 | 6.87/3.73/4.44, 2, 0.024, 1.167 | 11.42/5.78/7.09, 0, 0.039, 1.195 | 1.64/11.03/7.25, 1, 0.057, 2.132 | 11.06/18.49/6.68, 0, 0.060, 1.830 |
| shared 4, 5x5 picks | 6.90/3.80/4.45, 2, 0.028, 1.198 | 11.47/5.81/7.16, 0, 0.040, 1.204 | 1.68/10.92/7.35, 1, 0.059, 2.102 | 11.14/18.56/6.81, 0, 0.061, 1.779 |
| shared 2, 5x5 picks | 7.09/3.82/4.51, 2, 0.037, 1.296 | 11.71/5.88/7.20, 0, 0.051, 1.289 | 1.99/10.80/7.46, 1, 0.072, 2.209 | 11.34/18.70/6.86, 0, 0.073, 1.910 |
| one ray from 0.4 | 7.63/3.68/4.55, 2, 0.035, 1.340 | 12.41/5.73/7.54, 0, **0.099, 1.852** | 1.64/11.41/7.27, 1, 0.060, 2.145 | unchanged |
| unshared 4 (lease C1) | 7.52/3.82/4.53, 2, 0.043, 1.440 | 12.41/5.95/7.17, 0, 0.059, 1.485 | 2.24/11.14/7.49, 1, 0.086, 2.600 | 11.72/18.65/6.80, 0, 0.083, 2.117 |

RT gives the same picture within 0.1 code.

**Cut frames, inspected at 3x** (`D:/Coding/renderlab-work/p-glossy/sheets/cut-alu.png`, `cut-alu-2.png`):
- The first version shared a fixed 3x3 window. It stamped a rare bright hit (on the copper ball, and the turntable beside it) as a visible 3x3 square where base shows single bright specks. The crop scores did not catch this.
- The 5x5 picks remove the squares. What remains on the cut is a faint, soft, low-frequency mottle on the turntable and copper ball where base has sparse bright specks; the aluminium ball is smoother than base. From frame 1 on, and on held and moving frames, it matches base.
- My reading: not worse than base's specks. It has been seen only on this one scene at 3x and only at the cut, so it needs an owner's look in motion before merging.
- Two shared strata leave blockier mottle; one ray from 0.4 speckles the aluminium ball.

## 4. The ~10 ms question: it is this branch, not main (M)

Lease D interleaved pure main and the branch, both at 16 strata on Showcase non-RT:

| Capture | Shading median (p10-p90) |
|---|---|
| main #1 | 24.07 (20.78-34.55) |
| branch #1 | 36.53 (31.80-43.06) |
| main #2 | 23.46 (20.97-38.24) |
| branch #2 | 31.92 (31.06-37.63) |

So the branch costs about 8-12 ms more at 16 strata. Nothing is shared at 16 strata, yet the exchange and the restructured entry are still compiled in. RT is unaffected: branch 8.69 against P-SKY's 8.27. Main has also moved from P-SKY's 21.07 to about 23.5. The lease was noisier than lease B (wide p90). Whether 11.4 ms at 4 strata also carries this penalty is unknown.

**Leading hypothesis (I, untested):** the shading kernel used no shared memory before. About 5 KB of workgroup memory makes the driver carve L1 into shared memory, and the software walk is L1-bound. That would explain why non-RT pays and RT does not. Other candidates are the entry restructure itself (program `Surface`/`SurfaceInput` held as function `var`s across the barrier) and naga's workgroup zero-initialization.

**Prepared, not run (lease F, withdrawn):** `plan-f.txt` bisects on one binary:
- `traced=16` alone, then with `noexchange` (no exchange at all, so no shared memory), `nobarrier`, and `noshmem` (sample arrays of 1);
- the SAH tree against `median`;
- `rough4=2` (2 strata from roughness 0.4);
- pure-main references at the start and the end.

## 5. Verdict and what is left

**DON'T MERGE.** As committed, the source default is still 16 strata (`RL_GLOSSY_TRACED=16`), which only adds the restructure's cost. Turning `traced=4` on is a 2x non-RT shading win on Showcase (11.4 against 23.5-24.1 ms on pure main) and 2.9 ms on RT. Its pictures hold, but its penalty is unexplained.

**Exact next step for whoever resumes:**
1. Rebuild (`D:/Coding/renderlab-work/p-glossy/build-branch.sh` inside `guard.py build`, at -j 2). Targets were deleted at parking. Re-copy the Ara target from `D:/Coding/renderlab-work/p-sky-check/sc-cand/target` to `p-glossy/sc/target` first, and rebuild `sc-main` with `build-pair.sh` (pure main `68e02ddc`, or merge-base main).
2. Run lease F (`chain-f.sh`; `f1.py` runs the compose tests, then `plan-f.txt`, then `plan-g.json`), opening on the pure-main reference.
3. If `noexchange` or `noshmem` recovers the 8-12 ms, shrink the exchange:
   - pack the samples (point as 3 floats, radiance as `pack2x16float`, pdf in the spare word: about 3 KB);
   - or exchange through subgroup shuffles. That needs a device feature, so name it for ara-gpu.

   If neither recovers it, the restructure is the cost: try keeping the program values as `let`s inside a helper that returns before the barrier.
4. Then flip the default to `RL_GLOSSY_TRACED=4` and measure Showcase still, moving and cut, plus the game scene. Have the owner look at the matched-scene cut sheets in motion, and merge if the penalty is gone.
5. Then the larger change brief-2 asks for. The rough band holds 71% of Showcase's glossy pixels, and one ray there is visibly worse. Try `rough4=2` with sharing first. After that comes a store that can carry a lobe for roughness 0.4-0.5 (the cells' SH cannot). The software walks remain the largest cost (about 17 ms at 16 strata), so time the SAH tree in the same lease.

**Quarry and Epic ratio:** the quarry traces no reflection rays (every scan is over 2^19 triangles), so nothing here moves the quarry ratios (non-RT 2.1x slower than Epic still, RT 1.34x faster; rank-by-epic-ratio memory). Glossy is Showcase's cost only. **Flicker** on the matched scene is in the table above; Showcase flicker was not scored.

## 6. Process

- **Leases:**
  - B1/B1r (non-RT split; compose tests caught a reserved word, `active`, and a moved software seam before any capture);
  - C1, C2 (matched scene, 3x3 sharing);
  - B2r (RT and moving);
  - D (pure main against branch, bound, marker, 5x5 picks, 2 strata, matched scene).
  - A and F never ran: A's guard died on a PowerShell start failure, and F was withdrawn at the stop.
- **Builds:** Ara player 4 times (19-56 s incremental) and Renderlab release 3 times (about 1.5 min each); -j 2 after root's order.
- **Evidence** in `D:/Coding/renderlab-work/p-glossy/`:
  - `sc/captures`, `sc-main/captures`: Showcase JSON, pass logs and PNGs;
  - `runs/{c1,c2,e}`: matched-scene frames, `score-*.json` and `sheets/`;
  - `sheets/`, plans, `tools/score.py` and `tools/batch.py`;
  - `guard.py`, `lease.sh`, `show.py` and the chain scripts.
- **Scaffolding to delete before merging:** `RENDERLAB_GLOSSY_SCREEN` (src/program.rs `glossy_screen`, its use in src/reflection_software.rs, and the instance-count log in src/primary_ray_gpu.rs).
