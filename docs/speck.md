# U-SPECK: blobs, grain and specks on moving frames

Unit U-SPECK, branch `t3/speck` from main `30e7e26c`, 2026-11-11. Islam's report on the game scene's flight (1080p, 180 frames, camera cut at frame 24; RT, non-RT `Surface` and non-RT `Disk` captures in `D:/Coding/renderlab-work/n-vsm1/flipbooks/raw`): "almost as you go all frames has noise on ground alot of points of white... also on all of the objects", then "not just on the ground... I think this happens in shadowed areas only maybe". Labels: **M** measured here, **C** read from code, **I** inference.

**Status: diagnosis only. No fix is in this branch yet.**

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
- **Mechanism (C, I; not yet tested):** cell shading adds each local light at the cell's query point with inverse-square falloff (`shaders/gi_shade_entry.wgsl`, the light loop in `gs_shade`). The lamp is 0.25 m under its bulb, a white sphere of radius 0.12 m, so the bulb's underside is 0.13 m from a 30,000 cd light: about 1.8 million lux, a reflected radiance above the bulb's own 60,000 cd/m² glow. A ground cell gathers along 64 fixed directions, each standing for 2π/64 sr. The bulb subtends far less, so the few cells whose direction lands on it take its radiance at many times its true solid angle, and the 3×3 cell filter spreads each into a soft blob. Frame 0 is clean only because no cell has gathered yet. The lintel's lamp-lit underside may contribute too. `tools/speck/replay.py` replays one ground cell's 64 directions against the bulb and the lintel on the CPU; `--diag bulb=off` and `bulb=dark` in the example tell the two apart on the GPU.

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

## 2. Split

(c), a1 and a2 are the GI cache (`shaders/gi_*.wgsl`, `src/gi_cache.rs`) and stay with this unit. (b), a3 and a4 are reflections, TR1 and the fire's restart sphere; they share no file with the cache. They stay here after (c) until a second unit takes them.

## 3. Reference

`renderlab::reference` path-traces a sun, a sky or map and emissive materials: no point or spot lights, atmosphere, fog, particles, glass or the Fox. Every switch above is a local light or the dusk sky, so the flight's frames have no path-traced reference yet. Approved routes (root): `reflection_matched`'s matched scene for a1, and the lamps as small emissive spheres of the same power for (c). In that reference a sphere stands in for each point light.

## 4. Crops

Native pixels, enlarged 2-3× nearest-neighbour, under `D:/Coding/renderlab-work/unit-speck/sheets/`:

- The captured flight, RT | Surface | Disk: `before-f014-cube.png`, `before-f097-pillar.png`, `before-f097-wall.png`.
- Switches: `c-ground-f098-nonrt.png`, `a-wall-f098-rt-2x.png`, `a-pillar-f098-2x.png`, `a-cube-f021-2x.png`, `b-cube-f014-rt.png`, `b-cube-f014-nonrt.png`, `b-cube-f014-nonrt2.png`, `a-wall-f098-rt.png`, `a-wall-f098-nonrt.png`.

## 5. Not done yet

- The fix for (c), its detector numbers on the three sequences (this flight, the quarry flight, Showcase with a moving camera), the settled-frame check by region and the cost.
- a1 and a2.
- The pillar's stair-stepped edge at frame 100.
- The gap entries in [m7-gaps.md](m7-gaps.md).
