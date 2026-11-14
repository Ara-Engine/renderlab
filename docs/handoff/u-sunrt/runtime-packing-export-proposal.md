# Missing runtime reflection inventory: smallest CPU export seam

Status: proposal only. Existing artifacts cannot supply exact final packing.
Root explicitly requested CPU/paper only and no renderer edits; no hook has
been implemented, no build queued, and authored counts remain labelled.

Candidate switch RENDERLAB_REFLECTION_GEOMETRY_DUMP=<owned directory>, default
unset. Scope: primary_ray_gpu.rs packing function where `meshes`, `traced`,
`bases`, packed `words` and final `records` already exist. U-RTVIS owns the seam.
Dump an opt-in CPU snapshot at a requested settled frame (e.g.30), not only the
initial upload: foliage/residency/batch construction can change after load.

Metadata: frame/generation; exact written header instance count and reserved
count; per mesh pointer-stable generation id, node/triangle base, original
primitive count, bounds, node count/depth and original mesh/batch label; every
instance placement slot, transform bits and casts flag; TLAS base/root/refit
count; actual batch and terrain/foliage origin if bridge provenance is present.
Print only provenance actually carried by the runtime, not inferred names.

Payload: packed CPU geometry and instance words (or unchanged mesh generation
plus a frame's records). These are CPU vectors before queue upload, so no GPU
readback or traversal instrumentation is needed. If upload words are no longer
retained at frame30, repack ONLY while the diagnostic is enabled from the same
stored mesh objects and instance records. Do not keep a permanent duplicate.
Optional bridge metadata maps generated batch/foliage/terrain origins to those
mesh/placement ids; without it raw AS counts are still exact but origin labels
remain unknown.

Logging off performs no packing/serialization/allocation or shader changes;
source modules and pixels unchanged. Logging on is an ordinary correctness
capture, never a timing row. Raw geometry remains ignored under the unit's D:
folder, with scene/player hashes; after CPU summary, remove only own summarized
non-image dumps above5MB if permitted. Keep every image.

CPU replay can then compute exact runtime BLAS/TLAS counts/depths, box overlap,
SAH proxy and original primitive/instance hit ties; build SAH4 over the same final
triangle sets and simulate sampled world rays through both full structures.
This is the missing input, not a need for another shader step counter yet.


## Approved implementation checkpoint (2026-11-14)

Root approved the CPU export first from main0ebcc116. The implementation is
on t3/reflection-ray-cost; no renderer build or GPU gate has run yet. The dump
names nonrt-software-reflections explicitly and excludes driver-built hardware
AS. A source/shading-head/bindings-key invariant is asserted around serialization.
It attempts once per renderer at the selected zero-based frame, requires payload
plus10GiB free on D: (destination filesystem on other hosts), and prints bytes.
U-RTVIS owns the matching GPU lobe-list dump and frame-selectable existing probe;
that separate measurement may read back GPU data, but this CPU geometry export
never does. Preserve L, primary_frame.sided.y and receiver epsilon for exact-ray
replay. No traversal-step counter is added yet.
