# M7-MP1: per-object shading programs

Overnight unit (Islam's complete-base push, 2026-11-08; rules in AGENTS.md). Branch `t3/m7-mp1`, from main `f159fcd`. Inputs: [programmable shading](programmable-shading.md) (the design's per-program tile lists, section 2.5 and phase 3), [R1](m7-r1.md) (reflection hits shade with the hit's program), [G2](m7-g2.md) (cell shading), API1's facade (`docs/api.md` on `t3/m7-api1`). Labels: **M** measured here, **I** inference.

## Status

| Item | Result |
|---|---|
| Mixed-program frames | **Work on both paths.** Traced (RT) and visibility-buffer (non-RT) frames whose draws use up to 32 programs shade each program with its own pipeline over its own tile list. Before MP1 both paths rejected them. |
| One program | The single full-region dispatch, its modules and its layouts are unchanged: MP1 composes no new text into them. |
| Raster | Already switched pipelines per draw in submission order; unchanged. |
| Reflection hits (R1) | A hit on a draw of the receiver's own program is shaded exactly as before. A hit on another program's draw uses the documented fallback (section 1.4). |
| GI cell shading (G2) | Per program: each reflecting program's cell pipeline shades its own draws' cells; cells of programs without `scatter`/`emission` reflect nothing, as a single-program frame of that program would. |
| Temporal film frames | Still one program per frame (`SoftShadows::Temporal` only; not a default). |
| RF10 | RESULT |
| API1 example | RESULT |
| Quarry cost | RESULT |

## 1. Design

### 1.1 Dispatch

A frame's programs are its draws' distinct programs in first-draw order (`program_lists::frame_slots`); slot 0 is the first draw's program. With one slot the frame keeps `primary_shade_tiles` over the region, the zero-overhead case of the design. With K > 1 slots, one compute pass runs:

1. **Binning** (`shaders/program_bins.wgsl`, an engine kernel with no program code). Its grid is the single-program shading grid: the region's 4x4 shading tiles, then rows of 16x16 clear blocks. Each tile ORs `1 << slot` of its pixels into a workgroup mask (a pixel's slot is `slots[batch(placement)]` from its hit record; misses and uncovered trace tiles count as slot 0) and appends the tile to every slot's list in the mask, with an atomic counter per slot that is also that slot's indirect dispatch argument. Clear blocks go to slot 0.
2. **Per slot**, that program's list pipeline (`primary_shade_list`, shaders/program_traced_list.wgsl) in one `dispatch_workgroups_indirect`. A workgroup reads its tile, and each pixel whose hit record belongs to another slot returns. Slot 0's entries carry a flag: it also writes misses and the clear blocks. Every pixel is written by exactly one dispatch.

The list entry is appended to the program's ordinary traced composition (`program::compose_list`); it calls the same `primary_shade_pixel`. The slot and the batch-to-slot table are a 272-byte uniform at a dynamic offset per slot (binding 38), and the tile lists one storage buffer (binding 39), in a separate group-0 layout. No bind group, layout, hit record or dispatch of single-program frames changed.

**Why tile lists, not an uber-shader** (the design's section 3): programs stay separate modules, so an edit compiles one program; the worst program does not set registers for every pixel; and single-program frames pay nothing. RESULT-COST.

### 1.2 Pipelines and compile

List pipelines are a program's `ProgramPipelines::list` pair (Hard, Soft; plain and streamed), built by the first mixed frame that draws with the program (`Renderer::ensure_list`), concurrently, through the program pipeline cache. Reloads, prelude reloads and visibility changes drop them like `streamed`. The binning kernel compiles once per renderer.

### 1.3 Memory

Per allocated slot (a power of two at or above K): 4 bytes per shading tile and clear block of the image (555 KB at 1920x1080), 16 bytes of arguments and 512 bytes of slot table. Two slots: 1.1 MB; eight: 4.4 MB at 1080p. Allocated by the first mixed frame and kept.

### 1.4 Reflection hits (R1) in mixed frames

R1 shades each hit inline inside the receiver's module, with the receiver program's `surface` and `shade`. Another program's code is not in that module. In list modules a hit on another program's draw is therefore shaded by **the receiver's program, with the hit's own material inputs (factors, textures, decals) and the receiver's parameter block** (the hit's block has the other program's layout). The toon hero seen in a photoreal mirror is shaded photoreal with its own albedo; it is present, lit and shadowed, but not in its own style. Hits on the receiver's own program are exactly R1's. This is the recorded fallback (m7-gaps.md, Programs); a per-program hit pass is the fix (section 5).

### 1.5 GI cell shading (G2) in mixed frames

The cell draw table's spare fourth word selects each draw's cell program (`id + 1`, or 0 for a program without `scatter`/`emission`). Each reflecting program's mixed cell pipeline (`mixed_cell_entry`: the G2 entry plus that check) runs over the queue in turn; it shades its own draws' cells, writes "reflects nothing" for selector 0 (every pipeline writes the same value), and leaves other cells alone. The lighting key hashes every program of the frame. Single-program frames keep G2's pipeline and table.

## 2. Verification

RESULTS

## 3. Cost

RESULTS

## 4. Process and evidence

RESULTS

## 5. Gaps

Recorded in [m7-gaps.md](m7-gaps.md) under "Programs".
