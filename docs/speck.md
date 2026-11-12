# U-SPECK: blobs, grain and specks on moving frames

Unit U-SPECK, branch `t3/speck` from main `30e7e26c`, 2026-11-11. Islam's report on the game scene's flight (1080p, 180 frames, camera cut at frame 24; RT, non-RT `Surface` and non-RT `Disk` captures in `D:/Coding/renderlab-work/n-vsm1/flipbooks/raw`): "almost as you go all frames has noise on ground alot of points of white... also on all of the objects", then "not just on the ground... I think this happens in shadowed areas only maybe". Labels: **M** measured here, **C** read from code, **I** inference.

**Status: (c) is fixed in the engine (section 2) and merged (main `338144be`). The cell mosaic Ara's Outpost showed is the cache's table filling up (section 7); its fix is merged (main `6f47207f`) and awaits the Outpost rerun. Open: the cost of (c), the capacity change for gather hit cells, the starved gather queue, a1's mottle, a2, the quarry and Showcase sequences, and the path-traced reference. (b), a3 and a4 moved to U-SPECK2 (`t3/speck2`).**

## 1. Three symptoms, six mechanisms (M)

One lease (`speck-ablaterr`, 390 s, RTX 4070 SUPER): the flight on both paths with one switch off at a time, 26 runs, 27 frames each (every seventh, which includes frames 14, 21 and 98). Binary: `D:/Coding/renderlab-work/n-vsm1/bin/n3/game_scene.exe` (base `d5391d8`), the one that made the captured flight. Runner: `tools/speck/ablate.py`. Scores: `tools/blobs.py` (soft blobs) and `tools/speck/grain.py` (pixel grain and isolated specks in named boxes). Raw numbers: `D:/Coding/renderlab-work/unit-speck/ablate/blobs.json` and `grain.txt`.

### (c) Soft blobs on the ground, both paths: the GI cache's bounce of the arch lamp

`tools/blobs.py`, median blob pixels over the flight. Frame 0 reads 5.6-5.8k. The flight with GI off reads 7.5k / 8.0k, which is the clean level for these camera poses.

| Run | RT | Non-RT | Blobs |
|---|---:|---:|---|
| Default | 20,380 | 20,677 | yes |
| GI off | 7,487 | 8,020 | gone |
| Bounce off (`gi-bounce=0`) | 9,898 | 9,223 | gone |
| All local lights off (`lights=0`) | 9,008 | 9,001 | gone |
| Fire light only (`lights=4`) | 8,422 | 8,517 | gone |
| Crate spot only (`lights=2`) | 8,856 | 9,368 | gone |
| Arch lamp only (`lights=1`) | 18,898 | 19,366 | yes |
| Fire flicker off | 19,977 | 20,600 | yes |
| Particles off | 20,036 | 20,029 | yes |
| Reflections off | 19,757 | 20,495 | yes |
| TR1 off | 21,141 | 21,524 | yes |
| Sun off | 40,962 | 40,284 | yes, on darker ground |
| GI direction rotation off (`gi-rotate=0`) | 15,082 | 15,536 | fewer, larger |

- **Cause (M):** the blobs need GI, its bounce term and the arch's hanging point lamp. Nothing else in the table matters.
- **Mechanism (C, then M in section 2):** cell shading adds each local light at the cell's query point with inverse-square falloff (`shaders/gi_shade_entry.wgsl`, the light loop in `gs_shade`). The lamp is 0.25 m under its bulb, a white sphere of radius 0.12 m, so the bulb's underside is 0.13 m from a 30,000 cd light: about 1.8 million lux, a reflected radiance above the bulb's own 60,000 cd/m² glow. A ground cell gathers along 64 fixed directions, each standing for 2π/64 sr. The bulb subtends far less, so the few cells whose direction lands on it take its radiance at many times its true solid angle, and the 3×3 cell filter spreads each into a soft blob. Frame 0 is clean only because no cell has gathered yet. The lintel's lamp-lit underside may contribute too. `tools/speck/replay.py` replays one ground cell's 64 directions against the bulb and the lintel on the CPU; `--diag bulb=off` and `bulb=dark` in the example tell the two apart on the GPU.

### (a) "Grain on surfaces lit only by indirect light" is four things

| | Where, frame | Paths | Removed by | Not removed by | Mechanism |
|---|---|---|---|---|---|
| a1 | Red wall and arch pillar faces, 98 | both | GI off | bounce off, lights off, flicker off, reflections off | The cache's sky term: streaky cell-sized mottle (I: per-cell 64-direction sums against the dusk sky). By eye on 2× crops; the pixel-scale grain score does not see cell-sized mottle |
| a2 | Blue cube's shaded faces, 21 | both | GI off (grain 0.79 / 0.90 → 0.16 / 0.15), bounce off (0.39 / 0.39) | lights off (0.63 / 0.73), flicker off (0.66 / 0.77), reflections off (0.98 / 1.12) | GI bounce on the spinning cube: a moving object's cells |
| a3 | Ground by the brazier and in the bush's shadow, 98 | RT only (grain 2.63 codes, 73 specks; non-RT 0.63) | flicker off (1.02, 8), lights off (0.30, 0) | GI off (2.41), TR1 off (3.61), fire only (2.81) | The fire light's RT soft shadow restarts every frame while the fire flickers (P-RELIT's restart sphere) |
| a4 | Cube's left face, 14: a row of 2-3 px light dots, 35 specks | both | reflections off (3 / 2), flicker off (1 / 6), lights off (2 / 3) | GI off (35 / 34), fire only (34 / 36), particles off (35), TR1 off (32 / 22) | Traced glossy reflections of fire-lit things, not accumulating inside the fire's restart sphere |

Grain is the RMS of luminance minus its 5×5 mean in 8-bit codes; specks are pixels more than 6 codes above their 7×7 median; values are RT / non-RT.

### (b) White specks on the non-RT paths only

Cube's right face at frame 14, near pixel (732, 553). By eye on 3× crops, not scored yet (the score box missed that face).

- Present: default, GI off, bounce off, particles off, fire only, flicker off, sun off.
- Gone: reflections off, all lights off, crate spot only. Faint with the arch lamp only. RT has none.
- So: non-RT traced reflections of something a local light makes very bright. A steady fire keeps them, so it is not a history effect.

## 2. (c) fixed: rays aimed around local lights

### Which surface (M)

- **CPU replay** (`tools/speck/replay.py`, the arch's bulb, lamp and lintel, ground cells of 6 cm): with 64 fixed directions 1.5 % of ground cells hit the bulb and take up to 26,431 lux, the rest none; after the 3×3 cell filter the worst cell is 6,543 lux above a 4,096-direction answer, where the lamp's own light under the arch is 2,037 lux. The lintel's lit underside adds a mottle of 45 lux RMS. Map: `D:/Coding/renderlab-work/unit-speck/sheets/replay-near16.png` (64 directions | the fix | 4,096 directions).
- **GPU** (lease `speck-bulb`, main plus `--diag bulb=off|dark`): without the bulb's sphere the flight's median blob pixels are RT 9,580 / non-RT 10,144; with a bulb that glows but reflects nothing, 9,583 / 10,365; default 20,380 / 20,677.

So the bulb is the source: a white sphere 0.13 m from its own 30,000 cd light, reflecting more than it glows. The scene stays as it is (root, 2026-11-11): Ara's scenes will have bulbs beside their lights, and the cache has to handle a small bright surface whatever the scene does.

### The fix (C)

A surface a local light reaches from a distance r reflects a radiance that grows as 1/r², while what it sends a cell falls with its solid angle. A fixed direction stands for 2π/64 sr whatever it hits, so it cannot resolve a lit surface much nearer the light than the cell is. The surfaces near a light are therefore gathered by rays aimed at them (`shaders/gi_cache.wgsl`, `gi_near_owner` and `gi_near_gather`; both gather kernels):

- For a cell at distance d from a local light in its upper hemisphere and within the light's range, the light's near region is the ball of radius d/3 about it. The cell's fixed directions add nothing for hits inside that ball.
- 16 rays leave the cell around the light's direction, their angle from it log-uniform from a quarter of the light's source radius (at least 1 cm) to the ball's edge: a density of 1/angle² per solid angle, which cancels the lit surfaces' 1/r². Azimuths are a golden-angle sequence turned by the cell's own angle. One more ray goes at the light itself, for the cap inside the smallest angle. Each ray adds its hit's radiance over (rays × density), only for hits inside the ball.
- The two sets of rays split the hemisphere's hits by where the hit lies, so their sum estimates the same integral: nothing is clamped and no light is added. A hit inside two lights' balls belongs to the lower-indexed light.
- Beyond a light's range no ray is aimed, and the fixed directions leave out a ball of at most a sixth of the range: the light those surfaces send that far is negligible, and without this a far cell could still meet the bulb.
- Setting: `IndirectSettings::near_light_rays` (16; 0 is the old behaviour), header word 15, `--diag gi-light-rays=N` in the examples. The rays are fixed per cell, as the directions are: no sample counts and no ranking.

**Why not the two simpler changes (the coordinator's suggestion):** weighting a hit by min(2π/64, the hit cell's own solid angle) would remove nearly all bounce light, since cells are about 4 px wide and nearly every hit cell subtends far less than a direction's cone; a fixed direction is right to stand for its cone when the neighbourhood of the hit looks like the hit. Flooring a point light's distance at its source radius changes nothing here: the bulb's underside is 0.13 m from a light of radius 0.12 m.

### Result (M, lease `speck-s1`, one binary with the rays off and on, white bulb, scene unchanged, 1080p)

`tools/blobs.py` blob pixels on the flight (27 frames, every seventh):

| | RT rays off | RT rays 16 | Non-RT rays off | Non-RT rays 16 | GI off (RT / non-RT) |
|---|---:|---:|---:|---:|---:|
| Median | 20,380 | **9,710** | 20,677 | **10,189** | 7,487 / 8,020 |
| Frame 0 | 5,784 | 5,784 | 5,633 | 5,633 | 6,691 / 5,760 |
| Frame 14 | 5,940 | 4,504 | 6,711 | 4,912 | 5,358 / 5,499 |
| Frame 21 | 5,818 | 3,522 | 6,531 | 4,052 | 4,532 / 5,187 |
| Frame 28 (4 after the cut) | 17,431 | 8,788 | 17,858 | 8,399 | 8,913 / 9,100 |
| Frame 35 | 22,518 | 10,063 | 22,496 | 9,803 | 9,499 / 10,151 |
| Frame 56 | 21,752 | 9,262 | 23,796 | 10,436 | 10,605 / 10,388 |
| Frame 98 | 24,146 | 12,649 | 26,004 | 13,902 | 10,899 / 12,384 |
| Frame 175 | 13,528 | 5,040 | 14,100 | 5,711 | 3,281 / 3,418 |

- With the rays the flight reads the GI-off level through the cut and to frame 56; later frames keep 1-2k above it, which is the cache's mottle (a1, a2), not blobs. Inspected at native resolution on both paths: no blob on the ground, the pillars or under the arch at frames 14, 21 and 98.
- With 8 rays: 9,861 / 10,588. 16 is kept: in the replay 8 rays read the lintel's bounce 8 % high.
- **No light lost.** Settled still frame (frozen camera, steady fire, frame 95), means over 120 px regions, rays off against on: largest change 0.14 codes RT, 0.13 non-RT; no region over 1 code. Flight frames 14, 21, 98 and 175: largest change 0.38-0.77 codes, frame mean +0.01 to +0.02; no region over 1 code (`tools/speck/energy.py`).
- Replay: the bulb's error after the cell filter falls from 365 to 32 lux RMS, the worst cell from 6,543 to 120 lux, with the mean at 62.6 lux against the 4,096-direction 62.1.
- Lib test `gi_cache::tests::kernels_validate_on_both_ray_sources` passes (both ray sources, 64 and 128 directions).

Crops, before | after on RT and non-RT, under `D:/Coding/renderlab-work/unit-speck/sheets/`: `c-fix-f014.png`, `c-fix-f021.png`, `c-fix-f098.png`, `c-fix-f098-pillar.png`, `c-fix-f098-ground.png`, `c-fix-f098-arch.png`.

### Not measured yet

- **Cost.** A gathered cell traces 17 more rays per local light in range. The timing lease (reference run first, game scene and quarry, both paths) was queued and never admitted. On the quarry, which has no local light, the change is two loops over a light list that holds only the sun.
- The quarry flight and Showcase with a moving camera (`tools/blobs.py`), and the path-traced reference with the lamp as an emissive sphere (`examples/reflection_matched.rs --lamp`, written, not yet built).
- A GPU test that runs the aimed rays against a stub scene (written, not yet built).

### For the ara-gpu port

`gi_cache::Planner::begin` writes header word 15 (`near_light_rays`), and `IndirectSettings` has the new field. No new buffer, texture or binding.

## 3. Split

(c), a1 and a2 are the GI cache (`shaders/gi_*.wgsl`, `src/gi_cache.rs`) and stay with this unit. (b), a3 and a4 are reflections, TR1 and the fire's restart sphere; they share no file with the cache and belong to U-SPECK2 (`t3/speck2`).

## 4. Reference

`renderlab::reference` path-traces a sun, a sky or map and emissive materials: no point or spot lights, atmosphere, fog, particles, glass or the Fox. Every switch above is a local light or the dusk sky, so the flight's frames have no path-traced reference yet. Approved routes (root): `reflection_matched`'s matched scene for a1, and the lamps as small emissive spheres of the same power for (c). In that reference a sphere stands in for each point light.

## 5. Crops

Native pixels, enlarged 2-3× nearest-neighbour, under `D:/Coding/renderlab-work/unit-speck/sheets/`:

- The captured flight, RT | Surface | Disk: `before-f014-cube.png`, `before-f097-pillar.png`, `before-f097-wall.png`.
- Switches: `c-ground-f098-nonrt.png`, `a-wall-f098-rt-2x.png`, `a-pillar-f098-2x.png`, `a-cube-f021-2x.png`, `b-cube-f014-rt.png`, `b-cube-f014-nonrt.png`, `b-cube-f014-nonrt2.png`, `a-wall-f098-rt.png`, `a-wall-f098-nonrt.png`.

## 6. Not done yet

- (c): the items under "Not measured yet" in section 2. Its lib tests did run later (section 7).
- The mosaic: the Outpost rerun, and the items under "Open" in section 7.
- a1's remaining mottle (about a code on flat surfaces) and a2.
- The pillar's stair-stepped edge at frame 100.
- The gap entries in [m7-gaps.md](m7-gaps.md).

## 7. The cell mosaic: the cache's table fills up

Ara's Outpost (integration thread, RTX 4070 SUPER, 1080p, both paths): a camera that walks 4 m and turns 12 degrees over 120 frames leaves the ground and a slab covered in flat, sharp-edged, cell-sized blocks, the near field first at frame 30 and everything by frame 60; frames 0 and 10 are smooth. Pixel rows across it read plateaus up to 13 codes apart (82, 82, 82, ..., 95, 95, 95, ...). Ara's old renderer is clean.

### Cause (M in the fixture; an inference for Outpost until its counters come back)

The table holds 2^20 cells in buckets of 8, and a cell is kept until it has gone unseen for 120 frames, whatever the pressure. A moving camera and the cells gathers request for their hits fill it well inside that time. A cell that cannot be created leaves its pixels on a lone neighbouring cell, a coarser one or the open sky: each reads as one flat value over the cell's footprint.

It is not the cells' sky sums differing from their neighbours' (the first reading): a flat plane under the dusk sky stays smooth in the fixture, and a lone cell read at full weight is what makes a plateau.

The fixture is `game_scene --diag field=1`: 300 boxes and a pale slab on an empty quarter of the ground, the camera walking 4 m while turning 12 degrees over the run's frames (120 here), 1080p. Counters per frame come from `RENDERLAB_GI_LOG` (below); `tools/speck/gilog.py` summarizes them.

| Before the fix, default table (RT) | Frame 0 | 1 | 10 | 30 | 60 | 119 |
|---|---:|---:|---:|---:|---:|---:|
| Cells created | 103,431 | 86,657 | 5,402 | 6,925 | 2,619 | 1,671 |
| Failed inserts (every band, once per claim round) | 0 | 0 | 4,171 | 27,692 | 73,036 | 216,759 |
| Cells waiting for a gather | 97,367 | 181,294 | 560,324 | 686,371 | 769,826 | 848,478 |

- 962,111 cells were created in 120 frames against 1,048,576 slots. Non-RT reads the same (967,597).
- **What fills it is gather hit cells, not pixels' cells.** Frame 0 creates the pixels' 103,431; frames 1-9 create 486,089 more, nearly all requested by gathers for their hits (`gather_requests` 82,049 in frame 1, 154,208 in frame 2). A hit is given a cell at the 4 px level of its distance from the camera, though a gather ray stands for a wide cone.
- **Forced:** with a 2^17-slot table (`RENDERLAB_GI=slots_log2=17`) the far boxes, the far ground and the slab's edge turn into the same flat blocks on both paths (`D:/Coding/renderlab-work/unit-speck/sheets/evict-f119-rt-far.png`, left panel). At the default size the fixture ends 92 % full and shows a 1-code mottle; Outpost is a larger scene.

### Fix (C): a full bucket gives up its least recently seen cell

`shaders/gi_cache.wgsl`, `gi_claim_miss`: when no slot of the bucket is free or past the eviction age, the miss takes the bucket's least recently seen cell, once that cell has gone unseen for the miss's own band's age (`gi_pressure_age`):

- a pixel's cell takes one no pixel or gather has read for 2 frames;
- a cell a gather requests for a hit takes one unseen for 16 frames (32 for the second generation), so gathers cannot push out cells the view reads.

It is the existing eviction with an age that yields under pressure. The seen stamps are complete before the claim rounds (the probe and the gathers write them), so the choice does not depend on thread order. A visible cell (seen this frame or the last) is never taken. Setting: `IndirectSettings::pressure_eviction` (on; off only to measure), header word 26.

### Result (M, lease `speck-evict5r`, the merged commit `b24f2f0c`, 1080p, both paths)

- **Picture, frame 119:** with the forced 2^17 table the flat blocks are gone on the far boxes, the far ground and the slab on both paths (`evict-f119-rt-far.png`, `evict-f119-nonrt-far.png`: before | after). The default table is clean before and after.
- **Counters at frame 119, RT / non-RT:**

| | Default 2^20, before | after | Forced 2^17, before | after |
|---|---:|---:|---:|---:|
| Failed inserts (every band, once per round) | 216,759 / 221,670 | 27,471 / 31,542 | 393,204 / 393,207 | 386,982 / 386,798 |
| Cells created | 1,671 / 1,609 | 5,776 / 5,869 | 0 / 0 | 1,583 / 1,614 |
| Evicted under pressure | - | 4,083 / 4,157 | - | 1,583 / 1,614 |
| ... of them seen within 8 frames | - | 107 / 118 | - | 1,348 / 1,377 |

- Failed inserts stay high because that counter mixes the pixels' misses with the gathers' requests, which a full table refuses by design. The pixels' share is counted separately since `57ecce8c` (`full_pixel`): see the next table.
- **No thrash of visible cells:** by construction (a victim must have gone unseen for 2 frames). With a held camera and the default table, creations and evictions both fall to 0 by frame 90. With a held camera at 2^17, 120-330 cells a frame are still created as hit cells age out.
- **When the table is still too small** (2^17 against the same view): no mosaic, but less bounce light: frame 119 is 2.4 codes darker on average than with the default table, up to 9.7 codes in a 120 px region, on both paths. The hit cells are what is refused.
- **The game flight does not fit either:** 213,687 evictions under pressure over its 180 frames (786 of cells seen within 8 frames).
- Lib tests in that lease: `gpu_near_light_rays_keep_the_gathered_light` and `kernels_validate_on_both_ray_sources`, "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 396 filtered out". (The lease that first claimed the second one for (c) had run 0 tests: its runner named the wrong module path with `--exact`. The runner now fails unless every named test reports ok.)

### Same binary, eviction off against on (M, leases `speck-ab`, `speck-ab2rr`, commit `c8b6812c`, 1080p)

`RENDERLAB_GI=pressure_eviction=0` against the default, one binary. Pixels' failed inserts are counted once per claim round (`full_pixel`).

| | RT off | RT on | Non-RT off | Non-RT on |
|---|---:|---:|---:|---:|
| Game flight, 180 frames: pixels' failed inserts, total | 2,998,909 | 362 | 3,032,915 | 373 |
| ... at frame 90 | 24,716 | 0 | 24,916 | 0 |
| ... at frame 179 | 14,276 | 6 | 14,338 | 3 |
| Game flight: median blob pixels (`tools/blobs.py`) | 9,600 | 9,198 | 10,215 | 10,059 |
| Field walk, default table: pixels' failed inserts at frame 119 | 111,249 | 81 | not run | 116 |
| ... total over 120 frames | 5,349,917 | 4,378 | not run | 4,920 |
| Field walk, 2^17 table: pixels' failed inserts at frame 119 | 254,598 | 60,602 | 248,553 | 57,898 |

- **Unchanged where the table has room.** The settled still (frozen camera, steady fire, frame 95) is byte-identical with the eviction off and on, on both paths: no insert fails and nothing is evicted there. In the flight, frames 14 and 21 are identical by 120 px region (0.00 codes), before the table fills.
- **Where it does not fit, the picture moves little and the cells exist.** Flight frames 56, 98 and 175 differ by at most 0.22 codes (RT) and 0.33 (non-RT) by region, with no region over 1 code, while the pixels' failed inserts fall from about 3.0 million to under 400 over the flight.
- **At 2^17 the pixels' inserts still fail** (about 60,000 a frame, counted once per round): 103,000 pixel cells do not fit 131,072 slots in buckets of 8. The picture has no flat blocks there, because most of each pixel's neighbouring cells now exist; it has less bounce light (above).
- Lib tests in `speck-ab`: "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 396 filtered out".
- **Not run: the quarry.** Its lease needs 14 GiB free and the machine held 9-11 GiB all evening.

### Table size and what else it costs

- The table is 192 MiB at 2^20 (192 bytes a slot), plus 16 MiB of outgoing radiance, 88 MiB of filter sources and 4 MiB of claim words. 2^21 adds about 300 MiB and does not fit the non-RT device as one buffer (384 MiB against its 256 MiB limit).
- The capacity change that fits both paths is to give gather hits coarser cells, sized to the ray's cone at the hit: section 8.

### Measurement switches (both through any embedder)

- **`RENDERLAB_GI`**: overrides the cache's tuning over the engine's own choice, `key=value` pairs separated by `;`. Keys: `lookup=nearest|trilinear`, `filter`, `filter_edge`, `rotate`, `bounce`, `directions`, `cell_pixels`, `level_blend`, `gathers`, `gather_cells`, `claim_cells`, `near_light_rays`, `slots_log2`, `evict_frames`, `pressure_eviction`. Unset, it changes nothing. An unknown key, a pair without `=` or a value that does not parse is an error (named on stderr), so a mistyped switch cannot pass for the default. Example: `RENDERLAB_GI="slots_log2=17;pressure_eviction=0"`.
- **`RENDERLAB_GI_LOG=<file>`**: appends one JSON line per rendered frame with the cache's counters: `misses`, `miss_overflow`, `created`, `full` (failed inserts, every band, once per claim round), `full_pixel` (the pixels' share), `lost`, `orphans`, `traces`, `invalidated`, `splits`, `local_overflow`, `shades`, `gathers`, `gather_rays`, `gather_misses`, `gather_requests`, `gather_deferred`, `claim_deferred`, `bounce_queued`, `evicted`, `evicted_recent`, `table_bytes`. It is a blocking readback each frame: measurement only, never with a timing.

### For the ara-gpu port

The work buffer's counters grew from 44 to 48 words (`COUNTER_WORDS`; `GI_W_CLAIMS` is 48), `IndirectCounters` has `evicted`, `evicted_recent` and `full_pixel`, `Planner::begin` writes header word 26 (`pressure_eviction`), and the near-light helpers are in `shaders/gi_near.wgsl`, which `kernel_source` appends. No new buffer, texture or binding.

### Open

- The Outpost rerun with the log (integration thread).
- The same-binary before/after on the quarry.
- The starved gather queue ([m7-gaps.md](m7-gaps.md)).

## 8. Gather hit cells sized to the ray's cone

A fixed gather direction stands for a cone about 0.36 of the hit's distance wide (64 directions), yet a hit read, and requested, the cell a pixel there would read: 4 px at the hit's distance from the camera. Those cells were most of the table (section 7). A hit now reads and requests a cell sized to the cone: `IndirectSettings::hit_cone_cells` cells across the cone's width, never finer than the pixels' level (`shaders/gi_cache.wgsl`, `gi_hit_level`; header word 27 holds the cell edge over the hit's distance). Near a contact the cone is narrow and the pixels' level stands. The rays aimed around a light keep the pixels' level: they resolve small things. Default 4; 0 is the old behaviour (`RENDERLAB_GI=hit_cone_cells=0`).

### Against the path tracer (M, leases `speck-cone`, `speck-cone-flight`, 1280x720, both paths)

`examples/reflection_matched.rs --field` is the field walk in the matched scene (uniform sky, sun, no fog), which `renderlab::reference` path-traces (16,384 samples at frames 60 and 119). `tools/speck/cone.py` runs and scores it: 80 px region means, 8-bit display codes.

| Field walk | Cells 0 | 16 | 8 | 4 |
|---|---:|---:|---:|---:|
| RT, frame 60: RMS against the reference | 0.43 | 0.42 | 0.42 | 0.42 |
| RT, frame 119 | 0.42 | 0.41 | 0.40 | 0.40 |
| Non-RT, frame 60 | 0.44 | 0.43 | 0.43 | 0.42 |
| Non-RT, frame 119 | 0.44 | 0.43 | 0.42 | 0.42 |
| Largest change of a region against cells 0 (either path, either frame) | - | 0.21 | 0.22 | 0.28 |
| RT: cells created over the walk | 620,054 | 432,919 | 390,129 | 346,278 |
| RT: evicted under pressure | 10,692 | 1,174 | 614 | 267 |
| RT: cells waiting for a gather at the end | 492,559 | 326,302 | 243,863 | 214,918 |

- The walk's error against the reference does not depend on the setting; the cells created fall by 44 % at 4.
- **The one place the picture moves is the walking hero, and it moves toward the reference.** On the example's own 48-frame flight (8,192-sample references at frames 16, 32 and 47), the only regions a setting changes by more than 0.75 codes are one or two a frame, on the hero (the two spheres walking beside the gold sphere; `D:/Coding/renderlab-work/unit-speck/sheets/cone-flight-moved-f047.png`). Their distance from the reference, cells 0 against 4: frame 16, 4.95 to 4.07 codes (RT) and 4.98 to 3.71 (non-RT); frame 32, 8.16 to 6.58 and 8.07 to 6.50; frame 47, 11.35 to 8.44 and 11.42 to 8.37. 16 and 8 lie between. The hero's underside is still 4-8 codes too dark: that is a2, a moving object's cells. (This entry first named the spinning crate; the regions were not inspected then. They are the hero.)

### On the game scene (M, lease `speck-cone-game`, 1080p, one binary, cells 0 against 4)

| | RT 0 | RT 4 | Non-RT 0 | Non-RT 4 |
|---|---:|---:|---:|---:|
| Field walk: cells created in 120 frames | 1,168,406 | 623,733 | 1,176,983 | 652,289 |
| ... evicted under pressure | 199,062 | 11,109 | 202,753 | 14,117 |
| ... pixels' failed inserts (once per claim round) | 4,378 | 8,298 | 4,920 | 11,534 |
| Flight: cells created in 180 frames | 1,189,721 | 735,240 | 1,195,485 | 740,530 |
| ... evicted under pressure | 213,687 | 22,362 | 217,008 | 23,193 |
| ... pixels' failed inserts | 362 | 1,226 | 373 | 1,236 |
| Flight: median blob pixels | 9,198 | 9,787 | 10,059 | 9,428 |

- **Settled still** (frozen camera, steady fire, frame 95), 120 px regions, cells 0 against 4: largest change 0.42 codes RT, 0.44 non-RT (worst channel 0.89 and 0.97); no region over 1 code.
- **Flight frames 56, 98 and 175:** at most 0.71 codes RT, 0.90 non-RT. Frame 21 has one region over 1 code, +4.9 codes: the spinning cube's shaded face (`D:/Coding/renderlab-work/unit-speck/sheets/cone-cube-f021.png`), a moving object as the hero in the reference check above is; the cube itself has no reference yet.
- The pixels' failed inserts rise a little while everything else falls (about 70 a frame on the field walk, of some 100,000 pixel cells). Not explained yet; a coarse hit cell is read by many gathers, so it is seldom old enough for a pixel's miss to take.
- Lib tests: "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 396 filtered out".

### Not measured

- Cost: fewer cells are created, traced and shaded, so the cache's own work should fall; no timing lease has run.
- The quarry, and the full lib suite (the bounce gates there were tuned at the pixels' level).

### For the ara-gpu port

`Planner::begin` writes header word 27 (an f32: the hit cell's edge over the hit's distance); `IndirectSettings` has `hit_cone_cells`. No new buffer, texture or binding.

## 9. The gather queue (open)

A cell gets its sky term the frame it is made and its bounce light only after gathers: one gather after its hit cells have values, then up to two more as theirs settle. A frame gathers at most 32,768 cells (`gather_cells`); with more queued, the rotation serves slot % period == frame % period, the period being the next power of two that fits.

### What a held camera shows (M, leases `speck-early`, `speck-first`, `speck-uncapped`, 1080p, the game scene with a steady fire, binary `dfc6fab3`)

Distance of frame N from frame 32 of the same held camera, 120 px regions, 8-bit codes (mean, and the worst region):

| | Frame 4 | Frame 8 | Frame 16 |
|---|---|---|---|
| RT | -0.91, 7.07 | -0.51, 4.03 | -0.18, 1.52 |
| Non-RT (its sky term takes four frames first) | -1.14, 9.66 | -0.74, 6.25 | -0.28, 2.40 |
| RT, no gather budget (`RENDERLAB_GI=gather_cells=4294967295`) | -0.55, 4.19 | -0.26, 2.08 | -0.09, 0.86 |
| Non-RT, no gather budget | -1.18, 9.95 | -0.54, 4.64 | -0.18, 1.69 |

- With the budget about 120,000 cells wait from frame 1 and about 30,000 are gathered a frame until frame 12; without it the queue is empty by frame 6 (RT), at up to 111,566 gathers in one frame.
- **A bound for any schedule:** with no budget at all the held camera is still 2.1 codes from its settled frame in the worst region at frame 8, and 0.9 at frame 16. About half of the early darkness is therefore not the queue. What the other half is has not been found (TR1's history is a guess).
- On the field walk's first pose 315,000 cells still wait at frame 16 and a frame gathers only about 19,700: the power-of-two period leaves up to half the budget unused.

### Tried and dropped: a new pixel cell's first gather sooner (M)

Gathering a pixel's cell that has no bounce value yet at 4, 16 or 64 times the rotation's rate changed nothing for the better: frame 8's worst region went from 4.03 to 4.72 codes on RT and from 6.25 to 7.18 on non-RT, and on the field pose frame 16 went from 2.10 to 3.82. Frame 1 then gathers 92,162 cells whose hit cells have no values yet, and the gathers that matter come no sooner. Not kept.

### In progress, not merged (`230f157b`)

The cells pixels read this frame rotate over the budget alone and the rest share a quarter of it more; the period is the smallest that fits the budget. Its early-frame and cut timings are not in yet.

### A camera cut every frame: what the eviction costs (M, leases `speck-cuts-time3`, `speck-cuts-time17b`)

`game_scene --diag field=1 --diag cut=every`: the field walk with every frame a cut to another point of the walk, 120 frames, 1080p, the eviction off against on (`RENDERLAB_GI=pressure_eviction=0|1`), interleaved twice; GPU span p50 from `RENDERLAB_PASS_LOG` (`tools/speck/timing.py cuts`). Both leases opened with the reference run (this unit's s8 binary on the non-RT field walk, 28.8 ms by root's timing) and read 28.37 and 28.71 ms. **Caveat:** the binary is this branch at `11fe4fc5` plus the cut fix, with the then-unmerged `gather_pixels_first` on; the off/on comparison is same-binary, the absolute times are not main's.

| | Non-RT off | Non-RT on | RT off | RT on |
|---|---:|---:|---:|---:|
| Default table: frame, ms (two visits) | 28.30 / 28.38 | 28.46 / 28.26 | 42.56 / 42.68 | 42.59 / 42.68 |
| ... GI stages, ms | 20.0 | 20.1 | 10.2 | 10.2 |
| ... evicted under pressure in 120 frames | 0 | 8,593 | 0 | 7,963 |
| ... pixels' failed inserts | 29,044 | 693 | 44,819 | 1,106 |
| Forced 2^17 table: frame, ms | 20.49 / 20.52 | 24.84 / 25.00 | 39.25 / 39.31 | 39.48 / 39.57 |
| ... GI stages, ms | 10.45 | 14.9 | 4.70 | 4.98 |
| ... evicted under pressure | 0 | 1,976,229 | 0 | 1,963,342 |
| ... cells created | 131,072 | 2,107,301 | 131,072 | 2,094,414 |

- At the default size the eviction costs nothing here, even with a cut every frame: cone-sized hit cells keep the table at about 640,000 cells, and the cuts stay within one field, so the views share cells. This fixture does not reproduce Outpost's churn.
- With the table forced to 2^17 it does: 99.8 % of the evicted cells had been seen within 8 frames (Outpost's cut clip before the cone cells: 94 %). That churn costs 4.4 ms a frame on non-RT (`gi_update` 1.32 to 3.77 ms, `gi_gather` 6.57 to 8.77) and 0.2 ms on RT. It is real work, about 17,000 cells a frame made, traced and gathered, on cells another cut needs back within 8 frames. With the eviction off that table stops creating cells and shows the mosaic.
- An age floor would cut the work and bring the mosaic back in those frames. Whether Outpost still needs one after the cone cells is open: its cut clip is being rerun on main with the log.

### Pixels' cells first in the gather budget (M, lease `speck-pixfirst3`; `IndirectSettings::gather_pixels_first`, off by default)

The cells pixels read this frame (the probe's stamp) rotate over the budget alone, at most the view's pixels over a cell's, and the rest share a quarter of it more; the rotation's period is the smallest that fits the budget, not the next power of two. Held camera, distance of frame N from frame 32 (mean, worst 120 px region):

| | Frame 4 | Frame 8 | Frame 16 |
|---|---|---|---|
| Game scene, RT: rotation | -0.91, 7.07 | -0.51, 4.03 | -0.18, 1.52 |
| ... pixels first | -0.85, 6.38 | -0.47, 3.56 | -0.16, 1.36 |
| Game scene, non-RT: rotation | -1.14, 9.66 | -0.74, 6.25 | -0.28, 2.40 |
| ... pixels first | -1.14, 9.70 | -0.71, 6.03 | -0.26, 2.29 |
| Field pose, RT: rotation | -1.98, 5.90 | -1.38, 3.94 | -0.76, 2.10 |
| ... pixels first | -2.03, 6.27 | -1.06, 3.26 | -0.37, 1.12 |
| ... no budget (the bound) | -1.32, 4.05 | -0.62, 1.83 | -0.21, 0.63 |
| Field pose, non-RT: rotation | -2.65, 9.24 | -1.76, 5.59 | -0.89, 2.83 |
| ... pixels first | -3.03, 10.41 | -1.87, 6.10 | -0.66, 2.08 |
| ... no budget | -3.39, 11.61 | -1.55, 4.84 | -0.53, 1.58 |

- Where most queued cells are gather hit cells (the field pose) frame 16 is twice as close to the settled frame on RT and a quarter closer on non-RT. On the game scene, whose queue is mostly pixels' cells already, it changes little. Non-RT's frames 4 and 8 on the field pose are slightly worse; not explained.
- It uses the budget in full: the field pose gathers about 29,500 cells a frame against 19,700, and the flight 5.28 million gathers against 4.27 million (+23 %). Its cost has not been timed.
- Off by default until it is judged on a large scene (Outpost, the Radeon still): `RENDERLAB_GI=gather_pixels_first=1`. Off, nothing changes: the gathers' stamps and the rotation are as before.
