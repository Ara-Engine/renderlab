# Design: non-RT reflection traversal cost (P-DESIGN)

Unit P-DESIGN, 2026-11-09, branch `t3/p-design` from main `9fd0a9e`. Milestone 8: beat Ara on
performance with zero quality loss. This is a study and design note only; no build, lease or
test ran for it.

Labels: **M** measured (cited), **A** reported by another unit, **I** inference made here.

Scope: R1n's software reflection rays (`shaders/reflections_software.wgsl`,
`src/reflection_software.rs`), driven by R1's reflection block (`shaders/reflections.wgsl`), with
RQ1's glossy budget of 4 to 16 strata.

## 1. Measured cost

- **Game-base still, frame ~1200 (P-NRT2, M).**
  - R1n traversal is about 31% of the 2.03 ms shading pass, about 0.63 ms. The hot lines are
    box slabs, the triangle test, stack push and pop, `rs_q`, and the leaf and instance loops.
  - In game-atmosphere it is about 16% of 3.95 ms.
- **Game-base, v0.1.0 (P-NRT, M).** The traversal was about 54% of a 3.0 ms shading pass
  before the vec4 record loads. It was latency-bound: long-scoreboard stalls on geometry loads,
  plus a local-memory stack.
- **Reflective fixtures, 1080p (R1n §3 and RQ1 §4.3, M, functional).**
  - R1n added 1.6-3.5 ms of shading on the mirror and puddle fixtures, and 6.7 ms on RF8.
  - RQ1's 16 strata then took **RF8 non-RT shading from 5.57 to 19.19 ms (+13.6)**. On RT the
    same change was 1.83 to 3.97 ms (+2.1).
  - So the software ray costs about 6x the hardware ray on the same rays.
  - RF2 rose 3.51 to 8.27 ms non-RT.
- **Quarry (R1n §4, M).** Nothing is traced: every scan is over 2^19 triangles, so the
  coverage rule zeroes the bands. Even so, with the GI cache on, **shading costs +0.30 to
  +0.52 ms**, because the composed hit-shading code changes the kernel's code generation.
- **Comparator.** Lumen's reflection pass on the matched fixtures is 0.07/0.22 ms (U0, A).
  Ara raster runs no ray: half-resolution SSR, captures and probes.

Structure, from reading the code (I):
- Every ray loops over **all instance records** (a flat list) and tests each world box.
- Each mesh's tree is a **BVH2**: median split, up to 4 triangles a leaf.
- A **32-entry private stack** lives in the shading kernel.
- Each hit is shaded **inline**: `surface()` and `shade()` run once per ray. Each hit's shade
  traces its own sun ray (`rs_blocked`, any-hit), so a 16-stratum pixel runs up to 32
  traversals.
- **Closest hit keeps the first triangle visited at equal t** (`t < best.t`). P-NRT2 noted the
  image therefore depends on traversal order. Any change to the tree or visit order needs a
  traversal-independent tie rule first.

Quality to keep: R1n's RF1-RF9 scores (equal to RT within 0.04 codes), RQ1's gold and chrome
sphere scores (still and flight, both paths), and ARA1's mirror and glossy crops (.04290 and
.03719 non-RT). The 16-stratum budget and one-bounce contract stay (M8).

## 2. What the best engines do

- **Unreal Lumen reflections** (Wright, Narkowicz and Kelly, "Lumen: Real-time Global
  Illumination in Unreal Engine 5", SIGGRAPH 2022 Advances;
  [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine)).
  - One ray per pixel, or per half-resolution pixel, below a maximum roughness. Rough
    surfaces reuse the screen probes or radiance cache.
  - Software tracing marches mesh SDFs and the global SDF. The surface cache lights the hit,
    so no material is evaluated there.
  - A spatial reconstruction reuses neighbours' rays, weighted by the BRDF. A temporal filter
    reprojects by hit distance.
  - The speed comes from fewer rays, cheaper hits (cached lighting) and SDF marching, at the
    cost of approximation. Renderlab's references already beat Lumen SW on RF1-RF7 and RF9
    (R1n).
- **Frostbite and SEED.**
  - Stochastic SSR (Stachowiak, SIGGRAPH 2015) reuses rays across neighbours with ratio
    weighting. It is screen-space, so it is excluded for core lighting.
  - PICA PICA (GDC 2018) shot one RT ray per half-resolution pixel and reconstructed with
    spatio-temporal filtering.
- **Unity HDRP.** SSR, plus RT reflections with a denoiser; the RT tier uses one ray per pixel.
- **id Tech 7 and 8.** Doom Eternal: SSR plus probes. The later id Tech titles use hardware RT
  reflections. None of these is a software tracer.
- **Traversal research** (exact and order-preserving where noted):
  - Aila and Laine, "Understanding the Efficiency of Ray Traversal on GPUs" (HPG 2009):
    persistent threads, while-while loops and stack layout.
  - Ylitie, Karras and Laine, "Efficient Incoherent Ray Traversal on GPUs Through Compressed
    Wide BVHs" (HPG 2017): 8-wide quantized nodes of 80 bytes, which cut memory traffic and
    traversal steps substantially on incoherent rays.
  - Laine, Karras and Aila, "Megakernels Considered Harmful: Wavefront Path Tracing on GPUs"
    (HPG 2013): split ray generation, traversal and shading into kernels with queues, and
    regain occupancy.
  - Woop, Benthin and Wald, "Watertight Ray/Triangle Intersection" (JCGT 2013): already used
    by the sun lookup.

**Ara** (read-only, `ara-main` `1d2d647`, `crates/ara-render/src/look/mod.rs`):
- Its raster `Reflections` struct runs one deterministic **screen-space** mirror ray per
  half-resolution pixel. Roughness widens it through a colour pyramid, up to roughness 0.5.
- Below that it uses local reflection captures, probe lobes and the sky.
- The RT "fidelity" profile traces one ray per pixel below `reflection_roughness`.
- Ara is fast because it traces nothing off screen. Its mirror and glossy errors are 70.0%
  and 68.1% worse than Renderlab non-RT on the ARA1 crops.
- Nothing from it is borrowable without a quality trade.

**Conclusion.** Every fast engine spends fewer rays. M8 forbids that unless the image is
matched, and history-based ray reduction waits on reflection-aware reprojection (RQ1 gap 14).
The exact levers are:
- per-ray cost: the tree, the stack and the instance loop;
- scheduling: lane occupancy across pixels with very different ray counts;
- code generation: the cost paid with nothing traced.

## 3. Candidates, ranked by expected saving

### R2 (recommended first build): workgroup ray compaction

**Idea.**
- A shading warp's pixels need between 0 rays (matte, rough, sky) and 16 (roughness of 0.33
  or more), plus as many hit shadow rays. Today each lane runs its own pixel's loop, so a warp
  waits for its heaviest pixel.
- Restructure `rl_reflection_prepare`'s loop at the engine-generated entry (not the user
  program):
  1. Each lane computes its pixel's lobe, count and ray parameters, as now, and appends
     `(pixel lane, stratum k)` items to an 8x4 workgroup queue. That is at most 32 x 16 = 512
     items of 2 bytes.
  2. After a barrier, lane i traces items i, i + 32, and so on. The queue keeps one pixel's
     strata adjacent, so a warp's rays share an origin and lobe: coherent.
  3. The lane shades its hit with the same `rl_hit_input`, `surface` and `shade` in the same
     module, then writes that stratum's radiance to a workgroup slot (vec3, 6 KiB).
  4. After a barrier, each pixel sums its own slots in k order, as the current loop does.
- This is P-NSQ3's lesson, which took the quarry sun cut from 30.9 to 9.3 ms by load balance.

**Expected saving (I).**
- Mixed tiles: up to `max/mean` of the per-warp ray counts.
- Game-base (chrome and gold spheres among matte surfaces): **about 0.2-0.4 ms of the
  0.63 ms traversal**, and more on RF2-like mixed sweeps.
- RF8 (glossy rocks filling the screen) gains little: its warps are uniform.

**Quality argument.**
- The same rays, the same `rs_trace` and hit functions in the same module, and the same
  per-pixel summation order give byte-identical results by construction.
- Residual risk: the driver may contract FMAs differently once the functions are inlined at a
  new call site (memory: same program in two modules can differ in bits). Only a hash compare
  rules that out.
- **Patent:** rays still start at their own pixel's hit and write only their pixel's value
  (C7). It is per-ray work from the top of the tree, with no shared frustum or projection, and
  no grouping by cell (W8). The queue is scheduling.
- **Programmable shading:** none. `reflection_lobe`, `surface` and `shade` are called as
  before. The barrier sits in the engine entry, so every lane must reach it, including sky and
  untraced pixels, which call it with 0 items.
- **Memory:** workgroup memory only, about 7 KiB, or less with f16 radiance. That costs
  shading occupancy, which is the risk. No buffers.

**Cheap rejection.**
1. **Before building.** In P-NRT2's game-base trace
   (`D:/Coding/renderlab-work/p-nrt2/traces/base-*`), read the shading pass's
   active-threads-per-warp in the traversal lines. Or, on the CPU, compute
   `sum(max)/sum(mean)` of per-warp ray counts from a dumped roughness buffer.
   - Under 1.4: drop R2.
2. **After the build.** RF1-RF10 non-RT and ARA1 game-base and ara-pbr quality witnesses, by
   hash. Then the game-base shading pair.

### R1: traversal-independent ties, then a better tree (wide nodes, instance tree)

**Step 1: the tie rule (one-time image change).**
- Replace `t < best.t` with a lexicographic key `(t, slot, primitive)`: at equal t, the lower
  key wins.
- Inflate every node box at build time by a relative margin (for example 2^-20 of its extent,
  plus the leaf's `RS_EDGE` slack), so that pruning (`near > best.t`) can never drop a triangle
  whose computed t is at or below `best.t`.
- Per-triangle t, u and v do not depend on the tree. They come from the triangle's own record
  and the instance's ray. So after step 1 the closest hit is a function of the geometry alone,
  and any-hit sun rays are order-independent as booleans.
- **Quality:**
  - Pixels change only where two triangles tie at exactly equal t, which means shared edges
    under the 4e-6 slack. There both answers are equally valid hits within a few ulps.
  - Box inflation changes nothing else.
  - Verify: RF1-RF10 and the ARA1 crops must be equal or better. Count the differing pixels;
    expect very few.
  - After this step, every tree change below is byte-identical against it.

**Step 2: the tree.**
- An SAH build (GC1 already uses SAH for the GI's runtime software trees, §3.3).
- 4- or 8-wide nodes, compressed as in Ylitie et al.
- A small instance tree replacing the flat loop. Today every ray visits every traced
  placement's world box.

**Expected saving (I).**
- Wide and compressed nodes cut steps and bytes per ray. Ylitie et al. report about 1.5-2x on
  incoherent rays over binary trees; assume 1.3-2x here.
- Game-base: **0.15-0.3 ms**.
- RF8: **4-7 ms of the 13.6 ms ray cost**.
- The instance tree matters in proportion to the instance count. Read `rl_geometry[0].x` on
  game-base to size it.

**Patent.** P0 §4.11: a rebuilt top-level tree is in no claim read. Keep global-first
traversal and no per-cell launches (W8, C7).

**Programmable shading.** None; the hit record format is unchanged.

**Memory.** Compressed 8-wide nodes are 80 bytes for 8 children, against BVH2's 64 bytes for
2 children, so the tree shrinks. Triangle records stay at 10 words. Net equal or lower than
R1n's 61.5 bytes a triangle (I). Count with P-MEM's method.

**Cheap rejection (CPU only).**
- Dump one game-base frame's reflection rays (origin, direction, tmin) and the packed
  `rl_geometry`.
- Replay on the CPU: the current BVH2 against the candidate tree, under the step-1 tie rule.
  Compare node visits, triangle tests and fetched bytes per ray, and check the hit records are
  identical.
- Reject if visited bytes fall less than 25%.

### R0: no hit-shading code on frames that trace nothing

**Idea.**
- When the frame's bands are zero (the coverage rule, `ReflectionSettings::off`, or no
  reflecting program), bind a second shading pipeline composed without the reflection block,
  with `rl_specular` fixed at zero.
- This removes the measured +0.30 to +0.52 ms code-generation cost on the quarry with the
  cache on (R1n §4).

**Expected saving (M-anchored).** **0.3-0.5 ms** on the quarry and any other untraced frame.
0 on game-base.

**Quality.** Same values: the block returns before writing when `traced <= 0`. Byte identity is
not guaranteed across modules, so hash the quarry's 1x and TR1x64 captures.

**Patent.** None.

**Programmable shading.** Programs compose twice. The P-SHC hitch work must budget the extra
pipeline, compiling it at startup and not at first use.

**Memory.** One pipeline.

**Cheap rejection.** One quarry pair, cache on: shading pass p50.

### R3: stack in workgroup memory, same order

**Idea.**
- Move `stack: array<u32,32>` from private (local) memory into a per-lane workgroup slice, as
  N8l did for the sun lookup's tables.
- The visit order is unchanged, so this is byte-identical even before R1's tie rule.

**Expected saving (I).** 0.05-0.15 ms on game-base, if the stack is the local-memory stall
P-NRT saw.
- **Caution:** P-GI's register-kept nearest child made any-hit kernels 2-3x slower. Measure
  closest-hit and any-hit traversal separately.

**Memory.** 32 x 32 x 4 = 4 KiB of workgroup memory, which competes with R2's queue. If both
land, size them together.

**Cheap rejection.** Nsight's local-memory traffic on the shading pass. If it is not in the top
stall reasons, drop R3.

## 4. Recommendation

**First build: R2, workgroup ray compaction, gated by the warp ray-count ratio.**

| | |
|---|---|
| Expected saving | Game-base about 0.2-0.4 ms of the 0.63 ms traversal; larger on mixed glossy content; little on RF8 (I) |
| Quality | Byte-identical: same rays, functions and per-pixel summation order (hash-checked for the cross-site bit risk) |
| Patent | Each ray still from its own pixel's hit, writing its own pixel; no shared projection (C7, W8) |
| Shading | Unchanged program interface; the engine entry owns the barrier |
| Memory | About 7 KiB workgroup memory; no buffers |

**Next steps.**
- R1, with its CPU replay first. It is the lever for full-screen glossy (RF8's +13.6 ms).
- R0 as a small side item for the quarry.
- Ray reduction (Lumen-style) stays out until reflection-aware reprojection (RQ1 gap 14) can
  prove the moving image matches with fewer rays.
