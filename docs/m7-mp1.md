# M7-MP1: per-object shading programs

Overnight unit (Islam's complete-base push, 2026-11-08; rules in AGENTS.md). Branch `t3/m7-mp1`, from main `f159fcd`, main merged at `f870cb4` (with API1). Inputs: [programmable shading](programmable-shading.md) (the design's per-program tile lists, section 2.5 and phase 3), [R1](m7-r1.md) (reflection hits shade with the hit's program), [G2](m7-g2.md) (cell shading) and API1's facade ([api.md](api.md)). Labels: **M** measured here, **I** inference.

## Status

| Item | Result |
|---|---|
| Mixed-program frames | **Work on both paths.** Traced (RT) and visibility-buffer (non-RT) frames whose draws use up to 32 programs shade each program with its own pipeline over its own tile list. Before MP1 both paths rejected them. |
| Correctness (M) | At 1920x1080, with TR1 and GI off and no traced reflections, every pixel of a frame mixing standard, toon and a custom program is bit-identical to the same pixel of the all-standard, all-toon or all-custom frame: 2,073,600 / 2,073,600 on RT and on non-RT. |
| One program (M) | Unchanged: the single full-region dispatch, its modules and its layouts get no new text. RF1-RF9 score exactly R1's values on both paths; the quarry's shading pass equals main's within noise (section 3). |
| Raster | Already switched pipelines per draw in submission order; unchanged. |
| RF10 (M) | Renders on both paths (before: no frame). Whole-reflector RMS at -14 stops: **RT 12.34**, under UE HW-Hit Epic 15.61 and SW Epic 53.12; **non-RT 62.27**, over SW Epic 53.12, because non-RT reflects only the environment (R1's gap 1, R1n). |
| Reflection hits (R1) | A hit on the receiver's own program is shaded exactly as before. A hit on another program's object uses the documented fallback (section 1.4): RF10's reflected toon sphere scores 22.92 against UE HW-Hit Epic's 10.42. |
| GI cell shading (G2) | Per program: each reflecting program's cell pipeline shades its own objects' cells; cells of programs without `scatter`/`emission` reflect nothing, as in a single-program frame of that program. |
| API1 example (M) | The toon hero is now its own program, `examples/game_toon.wgsl`, beside the photoreal props' `examples/game_scene.wgsl`; the `style` workaround is gone. Renders on both paths (section 2.3). |
| Cost (M) | One program: the shading pass equals main's within noise. Quarry with 2 and 8 programs (standard plus toon variants), cache off: shading +0.06-0.11 ms (2) and +0.05-0.07 ms (8) over one program; with the cache on (the default) mixed frames are cheaper, because toon pixels make no GI lookups. Pure dispatch overhead (identical-cost programs) was not isolated (section 3). |
| Temporal film frames | Still one program per frame (`SoftShadows::Temporal`, not a default). |

## 1. Design

### 1.1 Dispatch

A frame's programs are its draws' distinct programs in first-draw order (`program_lists::frame_slots`); slot 0 is the first draw's program. With one slot the frame keeps `primary_shade_tiles` over the region, the design's zero-overhead case. With K > 1 slots, one compute pass runs:

1. **Binning** (`shaders/program_bins.wgsl`, an engine kernel with no program code). Its grid is the single-program shading grid: the region's 4x4 shading tiles, then rows of 16x16 clear blocks. Each tile ORs `1 << slot` of its pixels into a workgroup mask (a pixel's slot is `slots[batch(placement)]` from its hit record; misses and uncovered trace tiles count as slot 0) and appends the tile to the list of every slot in the mask. Clear blocks go to slot 0. Each list stores its length in a header word and raises its own indirect arguments to `min(n, 65535) x ceil(n / 65535)`.
2. **Per slot**, that program's list pipeline (`primary_shade_list`, `shaders/program_traced_list.wgsl`) in one `dispatch_workgroups_indirect`. A workgroup reads its tile, and each pixel whose hit record belongs to another slot returns. Slot 0's entries carry a flag: it also writes misses and the clear blocks. Every pixel is written by exactly one dispatch.

**Why rows.** wgpu's indirect validation skips any indirect dispatch whose arguments exceed 65,535 workgroups in a dimension, silently. A 1080p list holds up to about 139,000 tiles. The first version dispatched `n x 1`: at 1080p slot 0's dispatch vanished, so RF10 showed the previous fixture's pixels and the 2- and 8-program quarry rows timed skipped work. The 384x216 gate had passed. The gate now runs at 1080p, where slot 0 holds about 77,000 tiles.

The list entry is appended to the program's ordinary traced composition (`program::compose_list`) and calls the same `primary_shade_pixel`. The slot and the batch-to-slot table are a 272-byte uniform at a dynamic offset per slot (binding 38), and the lists one storage buffer (binding 39), in a separate group-0 layout. No bind group, layout, hit record or dispatch of single-program frames changed.

**Why tile lists, not an uber-shader** (the design's section 3): programs stay separate modules, so an edit compiles one program; the worst program does not set registers for every pixel; single-program frames pay nothing. The design's stop rule (above 0.06 ms of overhead, screen finer tiles and an uber-switch for mixed tiles) is addressed in section 3.

### 1.2 Pipelines and compile

A list pipeline is built per program, per `Shading` (Hard, Soft) and per streaming state, by the first mixed frame that needs it, all of a frame's missing ones concurrently, through the program pipeline cache (`Renderer::ensure_lists`). Mixed frames build no single-program traced set. Reloads, prelude reloads and visibility changes drop list pipelines like `streamed`; the next mixed frame rebuilds them. The binning kernel compiles once per renderer.

### 1.3 Memory

Per allocated slot (a power of two at or above K): one 4-byte word per shading tile and clear block of the image plus the header (555 KB at 1920x1080), 16 bytes of arguments and 512 bytes of slot table. Two slots take 1.1 MB at 1080p, eight 4.4 MB. Allocated by the first mixed frame and kept.

### 1.4 Reflection hits (R1) in mixed frames

R1 shades each hit inline, inside the receiver's module, with the receiver program's `surface` and `shade`. Another program's code is not in that module. In list modules a hit on another program's object is therefore shaded by **the receiver's program, with the hit's own material inputs (factors, textures, decals) and the receiver's parameter block** (the hit's block has the other program's layout). RF10's toon sphere seen in the photoreal puddles is present, lit and shadowed, with its own albedo, but smooth rather than banded. Hits on the receiver's own program are exactly R1's. A per-program hit pass is the fix (Gaps).

### 1.5 GI cell shading (G2) in mixed frames

The cell draw table's spare fourth word selects each draw's cell program (`id + 1`, or 0 for a program without `scatter`/`emission`). Each reflecting program's mixed cell pipeline (`mixed_cell_entry`: the G2 entry plus that check) runs over the queue in turn. It shades its own draws' cells, writes "reflects nothing" for selector 0 (every pipeline writes the same value), and leaves other cells alone. The lighting key hashes every program of the frame. Single-program frames keep G2's pipeline and table.

### 1.6 Shared edits (marked MP1)

`primary_ray_gpu.rs` (admission, slots, list bind groups, the shading branch), `program.rs` (`compose_list`), `program_gpu.rs` (list pipelines, `frame_sets`), `gi_cache.rs` (cell selection), `renderer.rs` (two call sites), `temporal_reconstruction.rs` (the history key adds a mixed frame's other programs; unchanged bytes for one program), `viewer/src/prepared.rs` (`set_programs`, for measurements), `environment_specular_tests.rs` (the `mp1` and `mp1std` quarry sections) and `reference_r0a_tests.rs` (an RF10 diagnostic switch). New: `program_lists.rs`, `program_lists_tests.rs`, `shaders/program_bins.wgsl`, `shaders/program_traced_list.wgsl`. API1's example and `docs/api.md` changes are in section 2.3.

## 2. Verification (M)

Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, dev-profile test binaries (`opt-level 1`), two Cargo jobs. Not QUIET windows: other units' leases and builds alternated with mine.

### 2.1 Gates

| Check | Result |
|---|---|
| `cargo test -p renderlab --lib` (binary `d`, final code) | 163 passed, 0 failed, 148 ignored, exit 0. Includes `list_compositions_validate` (naga on the binning kernel and every program's list module, RT and non-RT, plain and streamed), `frame_slots_follow_first_draw_order`, `list_capacity_holds_every_region_tile_and_clear_block` and the mixed cell entry's naga check |
| `gpu_mp1_mixed_programs`, RT (Interactive) and non-RT (Virtual) | Identity: 0 unmatched of 2,073,600 pixels on each path. Pixels owned by one program only: standard 1,236,429, toon 116,363, custom 73,358 (RT). 2-program, 8-program and mirror frames with the defaults (TR1 and GI on) finite. Pipelines created in the whole test: 31 RT, 34 non-RT (39 and 42 before section 1.2's trims) |
| RF1-RF10 (`gpu_r0a_current`, R0a protocol: 1x, TR1 off, GI off) | RF1-RF9 scores identical to R1's on both paths; RF10 renders (Status) |
| RF10 with its toon material on the standard program (diagnostic) | Single-program frame correct: separated the stale-pixel bug (section 1.1) from the reflection fallback |
| `cargo fmt`, `cargo clippy -p renderlab --lib --tests` | Clean in MP1's files |

### 2.2 Inspected images

`D:/Coding/renderlab-work/m7-mp1/runs/g5/`:
- `mp1-{rt,nonrt}/identity-{standard,toon,custom,mixed}.png`: eight boxes and a floor. No missing tiles, holes or seams at program boundaries; shadows cross program boundaries unchanged.
- `mp1-{rt,nonrt}/{two-programs,two-programs-mirror,eight-programs-mirror}.png`: the RT standard mirror reflects all eight boxes, other programs' boxes through the fallback. Non-RT mirrors show the environment only (R1n is not on main). The mottle on the standard box with GI on is the cache's known speckle (G2c), not MP1.
- `sheets/RF10-pitch30-{rt,nonrt}.png` (reference | MP1 | 4x difference): the toon sphere is banded in the primary view and smooth in the puddle reflection.

### 2.3 API1 example

`examples/game_scene.rs` now creates two programs: `game_scene.wgsl` (photoreal, with R1's `reflect` parameter) and `game_toon.wgsl` (the hero: banded light, tinted shadow side, rim, GI fill, flat-Lambert `scatter`). The `style` member and branch are gone, and `docs/api.md` lists the new limits (32 programs per frame; Temporal frames one; the reflection fallback).

Run `g4` (code before the row fix, which only affects lists over 65,535 tiles; the 1280x720 example's lists stay below that): 48 frames per path, exit 0. `example-{rt,nonrt}/frame-*.png` show the toon hero (bands, rim) as its own program beside the photoreal props on both paths, and the RT chrome sphere reflects the hero through the fallback. Debug example build, first-frame CPU time (all pipeline compiles): RT 49.0 s and non-RT 24.8 s, against 88.2 s and 37.4 s before section 1.2's trims. API1's one-program runs took 11.6-43.8 s (RT) and 13.3-23.7 s (non-RT). Later frames: 1.6-6.2 ms CPU. The final code (`d`, run `g10`) renders the same frames on both paths; its 4.0 s RT first frame is not comparable, because a stopped attempt seconds earlier had warmed the driver's cache.

## 3. Cost (M)

ES1's quarry protocol (`gpu_es1_quarry`): U0b's full-radiance quarry, 13,034,903 triangles, 1920x1080, `sun128.rle2`, RT traced primaries with Interactive shadows, non-RT visibility buffer with Virtual shadows. Per cache state, visits of a ten-frame cut then 24 frames moving 1 cm a frame; medians per visit, first visit dropped. "Shading" is the traced deferred shading pass, including binning in mixed frames. Functional timing, not a QUIET-window claim.

**One program, MP1 against main** (run `g3`: main's binary `main` and MP1's binary `a`, ES1's `abcost` rows, the same program; range of visit medians, ms):

| Path | Cache | Shading, main | Shading, MP1 | Frame span, main | Frame span, MP1 |
|---|---|---:|---:|---:|---:|
| RT | off | 0.518-0.522 | 0.518-0.520 | 1.471-1.490 | 1.459-1.469 |
| RT | on (default) | 1.747-1.758 | 1.749-1.756 | 6.40-7.32 | 6.47-7.74 |
| non-RT | off | 0.518-0.519 | 0.518-0.523 | 2.78-2.82 | 2.78-3.02 |
| non-RT | on (default) | 1.408-1.415 | 1.399-1.406 | 8.82-9.44 | 9.11-9.76 |

The single-program shading pass is unchanged within noise on both paths; the frame span varies with the cache's run-to-run cut work, in both directions.

**2 and 8 programs** (run `g10`, binary `d`: the quarry's twelve materials on program i % K, standard then toon and distinct toon variants; ES1's `mp1` section, the K = 1, 2, 8 rows interleaved in three visits; median and range of visit medians after the first, ms):

| Path | Cache | Shading, K=1 | K=2 | K=8 | Frame span, K=1 | K=2 | K=8 |
|---|---|---:|---:|---:|---:|---:|---:|
| RT | off | 0.521-1.379 (one contended visit) | 0.613-0.654 | 0.575-0.609 | 1.47-3.48 | 1.56-1.66 | 1.52-1.61 |
| RT | on (default) | 1.749-1.766 | 1.376-1.387 | 0.781-0.793 | 7.21-7.34 | 6.57-6.97 | 5.90-6.58 |
| non-RT | off | 0.521-0.559 | 0.619-0.624 | 0.612-0.617 | 2.75-2.88 | 2.85-2.86 | 2.82-3.12 |
| non-RT | on (default) | 1.408-1.419 | 1.221-1.245 | 0.764-0.770 | 8.87-9.36 | 9.50-10.57 | 9.30-9.80 |

- **Cache off:** two programs add about 0.06-0.11 ms of shading over one (RT against the clean K=1 visits and the `g3` one-program rows, 0.518-0.521), and eight about 0.05-0.07 ms. Eight is no dearer than two: the per-slot dispatch is cheap, and toon pixels cost slightly less than standard ones, offsetting it. The design estimated 0.03-0.05 ms for two programs; its stop rule (screen finer tiles and an uber-switch for mixed tiles above 0.06 ms) is not met cleanly, because these rows mix the program's own cost with the overhead.
- **Cache on (the default):** mixed frames are cheaper (1.38 and 0.79 ms against 1.76 on RT), because toon materials make no GI lookups. That is the programs' cost, not MP1's.
- **Not isolated:** the `mp1std` section (eight copies of the standard program, so only the dispatch differs) was stopped in every attempt: other units' build scripts tripped the lease guard (runs `g8`-`g10`, six attempts), and in `g5` and `g9` available memory fell below the guard's 2 GiB during the eight-photoreal-program run (`g3`'s toon rows bottomed at 5.1 GiB). Recorded as a gap.
- **The first `g3` and `g4` multi-program rows are void**: they timed skipped work (section 1.1).

## 4. Process and evidence

Every GPU process ran inside an m7-mp1 lease holding `D:/Coding/renderlab-work/gpu-slot.lock` (`scripts/m7-mp1-run.py`, R1's runner with per-step binaries, an `example` step and retries of steps stopped by other units' builds). Leases: `g3` (18:07-18:19 UTC, all steps), `g4` (18:33-18:39, stopped by a foreign build in the quarry step), `g5` (18:51-18:55, stopped by the memory guard in the quarry step), `g6`-`g9` (quarry steps stopped by foreign builds or memory; `g7` and `g8` before any test ran), `g10` (19:31-19:39). Two requests (`g1`, `g2`) were withdrawn before taking the lock. Builds ran outside my leases, with one lapse: a 40 s example build ran inside rtdef2's QUIET window after my test build had correctly stopped for it, recorded in the schedule.

Binaries (`D:/Coding/renderlab-work/m7-mp1/bin/`): `main` (main `f159fcd`, the one-program baseline), `a` (first MP1), `c` (compile trims), `d` (row fix, final code; `dstd` and `dtoon` are copies so each quarry section writes its own directory), and `game_scene_*` (the example).

Evidence under `D:/Coding/renderlab-work/m7-mp1/runs/`: `g3` (one-program quarry A/B, RF1-RF9 baseline scores, first example run), `g4` (example after the trims, the RF10 toon-as-standard diagnostic), `g5` (final gates at 1080p, RF scores), `g10` (2- and 8-program quarry rows, final example), and `inspect/` (RF10 crops). Raw `.f32` captures are deleted once scored; scores, summaries, sheets and inspected PNGs remain.

```text
py -3 -B scripts/m7-mp1-run.py build LABEL
py -3 -B scripts/m7-mp1-run.py gpu RUN rt:lib rt:mp1 nonrt:mp1 rt:rf nonrt:rf rt:quarry:mp1:BIN rt:example:-:EXAMPLE --binary LABEL
py -3 -B scripts/m7-r1-score.py D:/Coding/renderlab-work/m7-mp1/runs/RUN
```

The unit took about 2.5 hours, inside the 2-3 hour box: about 1.5 hours of reading, design and code (including the row fix the RF10 check found), and verification interleaved with it, mostly waiting for the shared lease and retrying stopped steps (about 25 minutes of GPU held in all).

## 5. Gaps

Recorded in [m7-gaps.md](m7-gaps.md) under "Programs".
