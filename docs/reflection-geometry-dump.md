# Software reflection packing snapshot

`RENDERLAB_REFLECTION_GEOMETRY_DUMP=<directory>` enables a CPU-only diagnostic
on the **non-RT software reflection path**. Hardware RT BLAS/TLAS are built by
the driver and are not represented. The default is off. No shader module,
shading-head key or binding key reads this switch. The hook checks that their
identity stays unchanged across serialization; frame image identity is checked
separately in the capture gate.

`RENDERLAB_REFLECTION_GEOMETRY_DUMP_FRAME` selects the renderer's zero-based
pre-submission frame counter (default 30). Use a settled frame after loading.
The dump is attempted once per renderer, including on I/O or space failure.
The dump is refused unless its destination filesystem has the payload estimate
plus a 10 GiB floor free, checked at the nearest existing ancestor. The successful
log prints the actual metadata-plus-payload byte count. I/O failure does not
reject rendering.

The snapshot has a JSON header and a `.words.bin` payload of little-endian u32s.
It repacks existing CPU mesh objects only on the selected diagnostic frame and
overlays the exact live instance/TLAS records. The temporary copy is released
after writing. There is no GPU readback, wait, resource allocation, or persistent
duplicate of geometry.

The header names the path and format, frame and geometry generation, written
and reserved instance counts, TLAS base/root/refit count, and each mesh's bases,
triangle/node count, maximum depth, root and local bounds. Floating-point bounds
and original placement matrices are stored as u32 bits. Mesh generation IDs are
stable within a geometry generation; they are not cross-process asset IDs.
Source draw index ranges, slot order, skinned status and casts flags are also
recorded. Singular placements and placements whose inverse is non-finite remain
in the provenance list but are marked excluded, using the native packing's check.

The renderer does not retain bridge asset/batch/terrain/foliage names in these
objects. Those provenance fields are explicitly null, rather than guessed from
triangle counts. A bridge-side mapping can add that attribution later. The raw
packing still supplies exact submitted software instance and BLAS counts.

The geometry snapshot itself does not contain the GPU glossy-pixel list,
actual reflection directions, or rays-per-slot counters; the glossy-list
snapshot below supplies the list. Keep diagnostic runs out of timing claims;
raw geometry and manifests belong in ignored local output directories.

## Glossy-pixel list snapshot

Where the reflection pass runs (with ray hardware by default once its default
is on, otherwise `RENDERLAB_RL_PASS=1 RENDERLAB_RL_DEFERRED=1`), the same switch
and frame also copy that frame's glossy-pixel list L after the trace
(src/reflection_list_dump.rs). It uses the same directory and the same stem, so
a frame's two snapshots pair up:

- `<stem>.glossy.bin`: L's 16-byte header (word 0 is the appended length) and
  its listed entries, 48 bytes each, in the layout `rl_trace_pixels` reads
  (shaders/reflections_trace.wgsl; the JSON spells it out).
- `<stem>.glossy-epsilon.bin`: each listed entry's receiver epsilon (the
  trace's tmin, the entry's word 2 z) as f32 bits.
- `<stem>.glossy.json`, written last: path (`rt` or `nonrt`), frame, view,
  generation (the software geometry's, 0 with ray hardware), the trace's
  direction seed `primary_frame.sided.y` as written that frame, the room, the
  view size, the appended length, the capacity and the listed count. Appended
  entries beyond the capacity stayed inline and are not listed.

The directions follow from these exactly: `rl_trace_pixels` derives each
stratum from the entry, its pixel and the seed. It is one buffer copy, mapped
on submission, with no wait. While the switch is set, L gains `COPY_SRC`, a
usage flag only. Without the switch,
nothing is copied and no usage changes. With ray hardware there is no geometry
snapshot: the driver builds that path's acceleration structures.
