# Native visibility-tail receiver

`RENDERLAB_MATERIAL_PASS` selects one producer: unset, `off` or `0` keeps today's
visibility and shading; `tail` publishes receiver words at visibility's tail;
`full` or legacy `1` keeps the separate material writer and reader. Default is off.
The tail leaves programmable shading intact and does not run the material writer.

The tail uses `program::material_trace_writes`, not a second packing implementation.
Binding 75 is one frame-owned `rgba32uint` 2D array. Tail allocates two layers:
logical word 1 at layer 0, logical word 2 at layer 1. Full allocates three layers,
adding material word 0 at layer 2. Allocation accounting includes the layers.

RT hard/exact, Interactive, deferred repair, reconstructed held-shadow and film
camera kernels publish the receiver. Deferred camera hits are provisional until
the existing repair dispatch finishes; repaired hits overwrite their record.
Non-RT resolve publishes after triangle repair and before the sun's LOD lift.
Word 2 is therefore the repaired, unlifted point. Word 1's material fields y/z
are zero; its near-edge bit is clear. Every in-frame miss writes the shared
sentinel and zero word 2, including outside the scene rectangle and fast-clear
tiles. Out-of-frame lanes still participate in existing workgroup barriers.

Native modules are edited at checked engine seams after the shared composition;
the port's raw modules and the shared WGSL remain unchanged. With off/full these
producer edits return the original source verbatim. Only tail producer layouts
gain the array storage entry. Full shading alone binds the sampled record.
Enabled modes require six storage textures per shader stage; initialization
refuses a smaller adapter or caller-supplied device before pipeline creation.

No later pass consumes the tail yet. Each pass owner must measure its own decode
cost and state whether it needs the repaired receiver or shading's original point.
Readers retain hit-image checks; this change does not retire that image.

`RENDERLAB_MATERIAL_TAIL_CHECK=1` scans hits, misses and fast-clear misses after
visibility, reporting sentinel, slot, material-field, near-bit and finite-point
failures asynchronously. It is diagnostic and excluded from timing.
`RENDERLAB_MATERIAL_TAIL_PROBE=geometric` substitutes the already-live geometric
normal for the interpolated vertex normal, keeping dynamic packing and stores.
Tail minus this probe measures the RT normal attribute gather/transform/interpolation
as a separate line. This probe's metadata is unsuitable for receiver consumers.
