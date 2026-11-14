# U-FIXED close-out notes (2026-11-14)

- `sparse-clipmap-note.md`: design for the sparse non-RT sun clipmap (page pool + page tables, wider virtual
  windows). This branch has no code yet; next step: step 1 of the note's landing (page table + pool at today's
  window size, residency from receiver requests), default on behind an off-switch, gated by `smoke`.
- `pcf1-report.md`, `jit1-report.md`: the two fallback-filter attempts (neither shipped).
- `staircase.md`: the closed vd_lookup staircase investigation.
- `shaders/`: P203's Disk shader and the measurement variants (nz: slide gate override; pcf: walk-blend filter;
  dbg: level views 3/4/6/7).
Edge1 (tarp edge, window fallback 1.5-2.5 octaves cold and settled): evidence in D:/Coding/renderlab-work/unit-fixed/runs/edge1.
