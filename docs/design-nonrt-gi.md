# Design: non-RT GI lookup, probe and fill cost (P-DESIGN)

Unit P-DESIGN, 2026-11-09, branch `t3/p-design` from main `9fd0a9e`. Milestone 8: beat Ara on
performance with zero quality loss. This is a study and design note only; no build, lease or
test ran for it.

Labels: **M** measured (cited), **A** reported by another unit, **I** inference made here.

Scope: the world radiance cache (WRC) on the non-RT path. That is the shading lookup
(`gi_prepare`, `shaders/gi_api.wgsl`), the probe (`gi_probe`, `shaders/gi_cache.wgsl`), and the
setup, ray, gather, light and filter passes that fill cells after cuts (`src/gi_cache.rs`,
`shaders/gi_rays_software.wgsl`). The patent constraints are P0's W1-W3 and C1-C5
([m7-p0-claims.md](m7-p0-claims.md)).

## 1. Measured cost

- **Shading lookup** (P-GI §2, ablation on the shading pass, camera moving, M): **about
  0.80 ms**.
  - Contrast refine's finer-level reads take about 0.25 ms, and the bucket walk about 0.18 ms.
  - The rest is about 0.37 ms: hashes, the four start records, accumulation and ES1
    visibility.
  - Reading one corner instead of four saves nothing, because the four start records already
    load in parallel. The cost is the **dependent chains** (refine, walk) at low occupancy.
  - GC1 measured non-RT's lookup increment at 0.78-0.88 ms.
- **Probe:** about 0.45 ms on both paths (P-GI, GC1). With TR1 on, it runs every frame
  (`gi_probe` lists, dedups and finds each 8x8 tile's cells). The jitter changes the idle key
  every frame, so a still view never idles. An exact idle would need a 256-frame zero-work
  confirmation (p-gi.md §7).
- **Other moving-frame passes, non-RT** (P-GI §1, M):
  - setup 317 µs, gather 206, software rays 133, light 62;
  - cache passes p50 1,377 µs;
  - the frame increment over cache-off is +2.26 ms (RT +1.65).
  - G3's filter and refined reads add +0.25 ms non-RT (m7-g3.md §5).
- **Fill after a cut** (P-GI §5, M):
  - the worst non-RT cut frame is 18.2 ms;
  - 40-frame cut sum 383.7 ms against 142-148 cache-off;
  - gather kernels 85.4 ms over 40 frames;
  - the capped gather drain (8,192 cells a frame non-RT, 32,768 RT, rotated by slot under W2)
    runs about 100 frames.
  - RT frames 1-3 are dominated by new cells' setup (query points), visibility rays and finish
    (frame 1: setup 1.26, rays 1.00, finish 1.26 ms). Later frames are the gather, at
    2.8-4.0 ms.
- **ARA1's scoreboard** (P-NRT §1, P-NRT2, M): ARA1 times frames 1-87 while the cache fills.
  The span sits 1.7 ms (game-base) to about 7 ms (quarry) above the settled frame. P-NRT2:
  "the drain's per-frame cost is not the record loads".
- **Memory** (P-GI §6, M): table 224 MiB (2^20 slots), work 56, bounce 28, filter sources 88,
  and 38.7 MiB of software-ray geometry on non-RT.
- **Comparators.**
  - Lumen Epic's GI pass on the same quarry: 1.24 ms HW, 1.21 ms SW (GC1, A).
  - Ara raster's whole frame: 1.86 ms on game-base.

Quality to keep:
- G3's quarry populations (all better than G2b-GC1 on both paths).
- TR1x64 pinholes at cache-off level.
- SV settle.
- The ARA1 GI crop, non-RT .01999 against Ara's .03669.

Each candidate below is either byte-identical or changes only *when* a value becomes
available. The latter needs a settle-quality proof.

## 2. What the best engines do

- **Unreal Lumen** (Wright, Narkowicz and Kelly, SIGGRAPH 2022 Advances;
  [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine)).
  - Screen probes, adaptively placed about every 16 pixels, trace each frame against the
    surface cache: mesh cards captured and relit with a budget, and cached while static.
  - A world-space radiance cache (clipmap probes, placed where screen probes need them, with
    a fixed update budget) serves distant rays.
  - Pixels integrate the screen probes with plane-distance-weighted upsampling.
  - Lumen is cheap per pixel because the per-pixel work is an interpolation of a few probes
    in textures, not a hashed world lookup.
  - Its fill latency is hidden because screen probes trace on the first frame, against a
    surface cache that is mostly precomputed geometry.
  - It approximates (upsampling, temporal filtering) and leans on screen traces first. This
    is the shape to learn from, not the details.
- **DDGI** (Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance
  Fields", JCGT 2019; CC BY-ND, own code only).
  - A dense probe grid, relit with a fixed per-frame budget.
  - Each pixel samples 8 probes through hardware-filtered octahedral textures, with Chebyshev
    visibility. That is a direct index, not a hash.
- **EA SEED GIBS** (Halén et al., "Global Illumination Based on Surfels", SIGGRAPH 2021
  Advances).
  - Surfels spawned from the screen, kept in a spatial hash grid, updated with a ray budget.
  - Lookups are per pixel against the grid's cell lists.
- **Frostbite** shipped precomputed GI (lightmaps and probes, O'Donnell, GDC 2018). The
  geometry visibility is baked and the runtime relights.
- **Ara** (read-only, `ara-main` `1d2d647`, `crates/ara-render/src/look/gi.rs`).
  - "Probe-volume global illumination ... without runtime rays." A one-time acquisition (an
    RT bake, or a raster cube capture) records what every probe sees.
  - Each frame, `GI.Relight` relights a fixed budget of probes from those cached surfaces:
    1,024 on Medium, 2,048 on High. `GI.Gather` reads them at half resolution.
  - Ara is fast because the visibility is baked, the per-frame work is bounded, and the pixel
    lookup is a half-resolution texture read.
  - It is lower in quality because of probe-grid leaks (despite Chebyshev moments) and
    half-resolution detail. Its GI error is 45.5% higher than non-RT Renderlab's (ARA1).

**Conclusion.** The fast engines differ from the WRC in two exact-compatible ways:
1. **Static visibility is precomputed**: Ara's acquisition, Frostbite's bake, Lumen's cached
   cards. So the work after a cut is relighting, not tracing.
2. **Pixel reads are shared**: probes are read by many pixels from cache-friendly textures.

Half-resolution reads, screen traces and temporal filters are the inexact parts. They are
excluded here.

## 3. Candidates, ranked by expected saving

### G-A (largest, for cuts and fill): cooked static cell records

**Idea.**
- A WRC cell's sky visibility and its canonical query point depend only on geometry: fixed
  directions, the key's canonical point, and software rays against the scene's static meshes.
  Lighting does not enter them.
- For static content, cook them offline **with the production kernels** (`gi_setup`,
  `gi_rays`, `gi_finish`) over every cell a level's walkable space can reach, at each level.
  Stream them in on claim instead of tracing.
- Moving casters already invalidate the direction slots their bounds reach (`gi_scan`), and
  those keep runtime rays.
- Optional tier A2: also cook each blocked direction's **gather hit key**. A drain gather then
  reads the hit cell's radiance without a ray, which is Ara's relight with WRC values.

**Expected saving (I).**
- Cut frames 1-3 lose new cells' setup, rays and finish: roughly 3-4 ms per frame on RT's
  attribution, and more non-RT, where the rays are software.
- With A2, the drain's 2.1 ms a frame (85.4 ms over 40 frames of gather kernels) becomes
  record reads. That is most of ARA1's 1.7-7 ms fill excess.
- Steady moving frames save setup and rays for cells entering view (about 0.4 ms non-RT).

**Quality argument.**
- Records cooked by the same kernels from the same inputs are the values runtime tracing
  writes. P-GI showed cache images are cross-process identical for the same plan.
- So the image changes only in *when* a cell has its value: sooner. That is an improvement in
  settle (TR2: non-RT bounce took about 93 frames).
- Byte identity of the settled image is checkable by hash: the cooked-then-streamed table
  against a fully traced table at the same pose.
- Risk: the memory note's "same program in two modules can differ in bits". The cook must run
  the production pipelines, not a CPU port.

**Patent.**
- W2 is held: every reachable cell is cooked, so no ranking decides which cells get rays (US
  11,790,594's (a)).
- W1 is held: values replace, never average.
- G1c's cooked near-field masks set the precedent ("every vertex is traced at cook", P0
  §4.1).

**Programmable shading.**
- Sky visibility and hit keys are geometry, so they are program-independent.
- Bounce radiance stays runtime (`scatter` and `emission` per program). A2's cooked hit keys
  do not change what a program computes at the hit.

**Memory.**
- The GPU table is unchanged: cooked records replace traced ones in the same slots.
- Disk and staging: about 96 bytes of hot record per cell, times the reachable cells.
- A2 adds up to 64 hit keys per cell. That is too large uncompressed, about 256 MiB per 2^20
  cells, so it needs delta-encoded keys and must never be GPU-resident beyond the table.
- **Must not raise process memory above today's.**

**Cheap rejection.**
- Two processes at the quarry pose. In process 1, trace a cut and save the table after
  setup, rays and finish. In process 2, load those records at claim time.
- Hash the frames. Reject if the records are not bit-equal across processes.
- Measure the reachable-cell count per level, which gives the cook size, before any build.

### G-E (recommended first build): cooperative tile lookup in shading

**Idea.**
- Pixels of an 8x4 shading tile read heavily overlapping cells: cells are 4 px, or 2 px where
  refined. Split `gi_prepare`'s record reads at the engine-generated entry into four steps:
  1. Each lane computes its corner keys, as now.
  2. A workgroup dedupe (a small atomic table) keeps one lane per unique key, and that lane
     runs `gi_hot_find`.
  3. After a barrier, the found records (96 bytes each) sit in workgroup memory.
  4. Each pixel accumulates its corners from there **in its own corner order**.
- Refine and coarser fallbacks take a second round the same way. Only lanes that need them
  insert keys; every lane passes the barrier.

**Expected saving (I).**
- Global record loads per tile fall from 32 x 4 (x 2 when refined) to the tile's unique
  cells, likely 4-8x fewer.
- Records sit in workgroup memory instead of registers during the chain, which raises the
  occupancy that P-GI found the lookup bound by.
- Estimate: **lookup 0.80 to about 0.4-0.5 ms, saving 0.3-0.4 ms on every frame**: still,
  moving and recovered.

**Quality argument.**
- Byte-identical by construction:
  - the same keys, the same `gi_hot_find`, so the same slot and the same record words;
  - each pixel adds the same records with the same weights in the same order.
- Records are read-only during shading.
- Cross-site FMA contraction is the residual risk, so hash.

**Patent.**
- Deterministic reads with fixed weights (W3).
- The source never switches by sample count (C5).
- Sharing loads changes no value, so it is not a filter (US 12,423,782's thin point is
  unchanged).

**Programmable shading.** None. The prepare runs in the engine entry, and programs keep
calling `indirect_radiance` and the bounce functions.

**Memory.**
- Workgroup only: about 32 unique x 96 B = 3 KiB, plus a 2 KiB key table, per 8x4 group.
  This competes with the reflections note's R2 queue and R3 stack, so size them together.
- No buffers.

**Cheap rejection.**
- The probe already counts each tile's unique cells (`gi_probe_stats`, `GI_F_STATS`). Read
  one profiled frame's stats on game-base and the quarry.
- If unique cells per 32 pixels exceed 40 (fewer than 3x shared reads), drop G-E.
- After the build: P-GI's identity set (quarry poses 0 and 1, 1x and TR1x64, U0b's pan, game
  fly path; both paths), then a moving shading pair.

### G-B: faster software gather and visibility rays

**Idea.**
- Drain frames are ray-bound: about 8,192 cells x up to 64 blocked directions of closest-hit
  software rays per frame, plus hit-cell finds.
- Bring the reflections note's exact traversal levers to `gi_rays_software.wgsl`'s BVH4:
  - a workgroup-memory stack, keeping the visit order;
  - compressed wide nodes behind the step-1 tie rule;
  - a lane-compacted gather in which all 64 lanes of a gather workgroup trace live directions
    only.
- **Caution:** P-GI's register-kept nearest child made the any-hit kernels 2-3x slower.
  Change one kernel at a time.

**Expected saving (I).** 20-40% of the gather and rays kernels: **about 0.4-0.8 ms per drain
frame**, plus 0.05-0.1 ms on moving frames.

**Quality.**
- The stack move is byte-identical.
- The tree change needs the tie rule. Gathers take closest hits, and the hit cell is read at
  its key, so tie flips change which cell a shared-edge ray reads. That is a one-time
  equal-validity change, verified like the reflections note's R1 step 1.

**Patent.** As now (C1-C4).

**Programmable shading.** None.

**Memory.** Wide compressed nodes are equal or smaller.

**Cheap rejection.** A CPU replay of one drain frame's gather rays (dump the queue's cells and
directions) on both trees.

### G-D: probe skip with exact per-tile invalidation

**Idea.**
- Keep each 8x8 tile's last unique key list and slot generations.
- When this frame's keys equal last frame's list, and every slot's generation word is
  unchanged (one load per unique cell, not a walk), skip the finds. Still mark the cells seen.
- With TR1 jitter, keys rarely change on a still or slowly moving view, because cells are 4 px
  wide.

**Expected saving (I).** Part of the probe's 0.45 ms: **about 0.1-0.25 ms on still and slow
frames**, 0 on cuts.

**Quality.** Byte-identical if the skip is taken only when the result provably equals the find:
same keys, slots still holding those keys.

**Patent.** W2 is unaffected, because the skip changes no ray decision.

**Memory.** Up to 16 MiB for tile lists at 1080p (I). That must be offset, or held as a compact
hash per tile (8 bytes).

**Cheap rejection.** Dump two consecutive still frames' per-tile key lists, using ARA1's
game-base still pose with TR1 on. Count the fraction of tiles with equal lists. Below 70%,
drop G-D.

### Already built: P-NRT3's batched drain screening

Branch `t3/p-nrt3` (`9e4d474`) screens many deferred gather-queue entries in parallel. It is
byte-identical by reading, but unverified: it fails to compile on a WGSL redefinition of
`gi_wg_base` (m7-gaps.md). P-NRT3 itself notes it saves little if the rays dominate the
drain, which P-NRT2's drain observation suggests.

Rename the scalar and run its l2 identity and q1 timing. That is a housekeeping verification,
not a design.

## 4. Recommendation

**First build: G-E, the cooperative tile lookup, gated by the probe's unique-cell statistics.**

| | |
|---|---|
| Expected saving | About 0.3-0.4 ms on every still, moving and recovered frame on both paths (lookup 0.80 to about 0.4-0.5 ms) (I) |
| Quality | Byte-identical: same keys, slots, records, weights and per-pixel order (hash-checked) |
| Patent | W3 deterministic reads; C5 unchanged; no filter |
| Shading | Engine entry only; program API unchanged |
| Memory | About 5 KiB workgroup memory per group; no buffers |

**Next steps.**
- G-A is the structural answer to ARA1's 1.7-7 ms fill and the 18 ms cut. Start its
  cross-process record-identity check and cell-count survey now. They need no renderer change.
- G-B shares its traversal work with the reflections note's R1 and R3. Schedule them with one
  owner for the software-traversal files.
