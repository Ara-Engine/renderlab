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
| Cost (M) | Q-STATUS |
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

Run `g4` (code before the row fix, which only affects lists over 65,535 tiles; the 1280x720 example's lists stay below that): 48 frames per path, exit 0. `example-{rt,nonrt}/frame-*.png` show the toon hero (bands, rim) as its own program beside the photoreal props on both paths, and the RT chrome sphere reflects the hero through the fallback. Debug example build, first-frame CPU time (all pipeline compiles): RT 49.0 s and non-RT 24.8 s, against 88.2 s and 37.4 s before section 1.2's trims. API1's one-program runs took 11.6-43.8 s (RT) and 13.3-23.7 s (non-RT). Later frames: 1.6-6.2 ms CPU. EXAMPLE-G6

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

Q-TABLE

## 4. Process and evidence

PROCESS

## 5. Gaps

Recorded in [m7-gaps.md](m7-gaps.md) under "Programs".
