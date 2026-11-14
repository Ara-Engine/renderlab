# Sparse sun clipmap (non-RT): design note, DRAFT

U-FIXED, 2026-11-14. Paper only. Root approved it to be written once the Outpost memory lease gives real sizes.
Sizes marked **[lease]** wait for that lease (bin-memrep-761c934b, `RENDERLAB_MEMORY_REPORT`). Source read:
Renderlab 761c934b, `src/virtual_shadow_gpu.rs` and `shaders/virtual_shadow_disk.wgsl`.

## Today

- Each level of the non-RT sun clipmap is a dense window of 16x16 pages of 128 texels (2048x2048 texels),
  centred on the eye with page hysteresis. There are up to `MAX_LEVELS` = 8 levels.
- Per level the storage is:
  - two layers (`N8c clipmap top layer` / `peel layer`) at 8 B per texel: 32 MiB each;
  - max-mips at about a third of a layer's texel count, 4 B each: about 5.3 MiB;
  - plus the per-texel 2-byte map (8 MiB, shared) and per-page slots (12 B per page).
- At 7 levels this is about 490 MiB. The earlier inventory measured ~485 MiB, all non-RT.
- Receivers already request pages (`vd_mark`, `vsm_requests`), and pages are cached across frames (`slots`). The
  storage, however, is the whole window whether requested or not.
- **Coverage defect:** half of every window lies behind the camera. A receiver whose lookup window leaves its
  wanted level's window falls back to a coarser level.
  - Glint's goal rim and Outpost's tarp edge both fall back 1.5-2.5 octaves.
  - The result is a 4-6x too-wide texel, which shows as a staircase.
  - Forward windows (#137, default off) recover about one octave.
  - Filtering the fallback (jit1, the pcf1 variant) softens the edge but can't restore resolution.

## Proposal

- Storage becomes a pool of physical pages: 128x128 texels, both layers, 256 KiB per page. Each level gets a page
  table that maps virtual pages to pool slots (or none).
- Levels become virtual windows several times wider than today's 2048 texels: for example 64x64 pages (8192 texels).
  Only the pages receivers request are resident.
- A receiver then reads its wanted level unless that page is missing this frame. Window edges no longer cause
  fallback; only pool exhaustion and first-frame misses do.
- This fixes coverage and memory together. Memory scales with what receivers see, not with window area.

## The five questions

### 1. Coverage near the camera
- The wanted level is resident wherever the requested pages fit in the pool. That holds near the camera in
  particular, since that is where requests concentrate.
- With a virtual extent of 8192 texels per level, a level reaches 4x further from the eye before its window
  edge forces a coarser level.
- **To measure:** fallback share (debug view 4: chosen minus wanted level) on the Outpost tarp pose, Glint's goal and
  Showcase. Today against sparse; target: no fallback in held views.

### 2. Memory, against today's ~485 MiB and Unreal's non-RT total
- Pool bytes = resident pages x 256 KiB, plus page tables (virtual pages x 4 B: 4096 per level at 64x64, 16 KiB
  per level) and max-mips.
- Today's dense window equals 16x16x7 = 1,792 pages = 448 MiB of layers.
- The receiver page requests per frame on Outpost, Glint and Showcase set the pool size. **[lease]: page-request
  counts and the memory report's by_label.**
- **Outpost totals today:** non-RT 3,161 MiB, Unreal 2,932 MiB, RT 2,338 MiB.
  - Non-RT is 229 MiB over Unreal.
  - The clipmap's ~485 MiB is the largest non-RT-only item.
  - A pool of about 200 MiB (800 pages) would bring non-RT to about Unreal's total by itself. **[lease]** confirms
    whether requests fit in 800 pages.
- **Max-mips:** the mips below page size live in the page (about a third more per page). Above page size, a small
  dense mip per level over its page table (64x64 entries).

### 3. Lookup cost against today's (latency-bound)
- The walk reads a window of about (2 x VD_R + 1)^2 = 49 texels per level it visits, and at most 2x2 pages per level.
- Resolve those page-table entries once per level visited, not per texel. That is one extra dependent load per
  level, then the same texel reads as today.
- Receivers visit about 1-3 levels, so this adds 1-3 dependent round trips to a chain that today has about 3-6
  per level (mip probe, then texel rows).
- **Estimate:** +10-25% on the lookup, Outpost 1.77 ms -> about +0.2-0.4 ms.
  - Partly paid back: receivers that today fall back to a coarser level walk a band of larger texels with more
    max-mip work. Read on their wanted level, the walk is the same length but tighter.
  - **To measure in step 1:** an indirection identical to today's layout.

### 4. Behaviour on a cut
- **Today:** a cut invalidates every level, and every window re-rasterizes in full (the cut frame's page raster was
  1.37 ms on game_scene).
- **Sparse:** the cut frame has no requests yet for the new view.
  - Frame 0 reads what's resident.
  - The coarsest level is kept fully resident, small at 16x16 pages, so missing pages fall back to it for one
    frame (the same fallback as today, one frame instead of always).
  - Frame 1 rasterizes only the requested pages, so cut cost scales with visible pages, not window area.
- Requests come from the receivers of the frame being shaded. Using last frame's requests needs no readback stall;
  the same-frame variant (mark, then raster, then look up) is today's order and keeps working.
- **Risk:** one frame of coarse shadows after a cut, under TR1's own cut handling. Check it on the +1/+5/+20 strip.

### 5. Default-off landing (multi-day)
1. **Indirection alone:** `RENDERLAB_VSM_SPARSE=1` adds the page table and pool at today's window size and residency,
   with dense storage kept. Gate: pictures byte-identical or within noise; lookup and raster ms on Outpost and
   Showcase non-RT.
2. **Requested pages only:** residency from requests, pool capped at today's 1,792 pages. Gate: identical pictures
   in held views; cut strip; memory.
3. **Wider virtual windows:** 32x32, then 64x64 pages per level. Gate: fallback share; tarp and Glint crops; cost.
4. **Pool sizing and eviction:** LRU by last-requested frame, sized from the request counts. Gate: memory against
   Unreal's total; cut strip; three-scene non-RT gate.
5. **Flip:** default on after the three-scene gate and Islam's crop pair (staircase against the sparse edge, with
   the moving strip). Dense storage removed after.

## Open

- Unreal's virtual shadow map pool default (`r.Shadow.Virtual.MaxPhysicalPages`): to check from public docs, not code.
- The peel layer doubles every page. Whether peel can be allocated only for pages that hold a second surface is a
  later saving.
