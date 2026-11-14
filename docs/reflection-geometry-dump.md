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
The Windows path refuses a dump unless D: has the payload estimate plus a
10 GiB floor free. Other hosts check the destination filesystem. The successful
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
recorded. Singular placements remain in the provenance list but are marked
excluded, matching the native packing.

The renderer does not retain bridge asset/batch/terrain/foliage names in these
objects. Those provenance fields are explicitly null, rather than guessed from
triangle counts. A bridge-side mapping can add that attribution later. The raw
packing still supplies exact submitted software instance and BLAS counts.

This diagnostic does not contain the GPU glossy-pixel list, actual reflection
directions, or rays-per-slot counters. A separate frame-matched pass-list probe
is required for replaying real rays. Keep diagnostic runs out of timing claims;
raw geometry and manifests belong in ignored local output directories.
