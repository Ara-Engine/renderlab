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
- The capacity change that fits both paths is to give gather hits coarser cells (one sized to the ray's cone at the hit, not to 4 px at the hit's distance from the camera): the table's need falls toward the pixels' 100,000 cells. It changes bounce values and needs the reference check. Not done yet.

### Measurement switches (both through any embedder)

- **`RENDERLAB_GI`**: overrides the cache's tuning over the engine's own choice, `key=value` pairs separated by `;`. Keys: `lookup=nearest|trilinear`, `filter`, `filter_edge`, `rotate`, `bounce`, `directions`, `cell_pixels`, `level_blend`, `gathers`, `gather_cells`, `claim_cells`, `near_light_rays`, `slots_log2`, `evict_frames`, `pressure_eviction`. Unset, it changes nothing. An unknown key, a pair without `=` or a value that does not parse is an error (named on stderr), so a mistyped switch cannot pass for the default. Example: `RENDERLAB_GI="slots_log2=17;pressure_eviction=0"`.
- **`RENDERLAB_GI_LOG=<file>`**: appends one JSON line per rendered frame with the cache's counters: `misses`, `miss_overflow`, `created`, `full` (failed inserts, every band, once per claim round), `full_pixel` (the pixels' share), `lost`, `orphans`, `traces`, `invalidated`, `splits`, `local_overflow`, `shades`, `gathers`, `gather_rays`, `gather_misses`, `gather_requests`, `gather_deferred`, `claim_deferred`, `bounce_queued`, `evicted`, `evicted_recent`, `table_bytes`. It is a blocking readback each frame: measurement only, never with a timing.

### For the ara-gpu port

The work buffer's counters grew from 44 to 48 words (`COUNTER_WORDS`; `GI_W_CLAIMS` is 48), `IndirectCounters` has `evicted`, `evicted_recent` and `full_pixel`, `Planner::begin` writes header word 26 (`pressure_eviction`), and the near-light helpers are in `shaders/gi_near.wgsl`, which `kernel_source` appends. No new buffer, texture or binding.

### Open

- The Outpost rerun with the log (integration thread).
- The same-binary before/after on the quarry.
- The capacity change above, and the starved gather queue ([m7-gaps.md](m7-gaps.md)).
