# Joint reflection replay checkpoint

Renderlab main d7f57fa9 merged (fast-forward, no rebase) into t3/reflection-ray-cost.
Export #131 is merged. No renderer edit, build or GPU lease is needed for this preparation.

Owned CPU tool: `replay_geometry.py`, beside this note. Geometry-only invocation:

```
py -3 -B replay_geometry.py <snapshot.json> --out <owned-output-directory>
```

The reader verifies the payload size, header/layout, live count, included provenance,
mesh bases and tree counts/depths. `inventory.json` reports actual written/reserved
instances, distinct meshes and per-mesh multiplicity, plus TLAS topology. These
runtime counts will be compared with the authored 629/74 placements, with provenance
labels kept unknown where the renderer does not retain them.

The tool also has a CPU float32 model of today's `rs_trace`: TLAS ordering, instance
box gate, transformed unnormalised direction, BLAS near-first ordering, barycentric
slack, any-hit casts flag and lower-instance equal-distance retention. Per-ray CSV
counts TLAS internal nodes, TLAS leaf-instance entries (calls before instance gate),
instance box tests/entries, BLAS visits, BLAS internal nodes/box tests and triangle
tests. Both stacks reject overflow instead of dropping nodes. Flat-list mode counts
its instance calls in the same instance-entry column.

`glossy_to_rays.py` implements U-RTVIS's bfeb48ef list layout and the current
shader ray generator using L records, seed frame and receiver epsilon. It keeps
slot IDs, records horizon rejection separately, and refuses mismatched stems,
frame/view/generation or incomplete payloads. CPU float32 trig/FMA can differ from
the GPU; direction bit identity is not yet certified. Match the shader revision
at capture before using real rows. No synthetic distribution substitutes for L.

Adapter output is JSONL: `frame`, `view`, `origin[3]`, `direction[3]`, `tmin`, `tmax`,
and optional `id`, `slot`, `any_hit`, `gpu_record[4]`. Supply with `--rays <file>`;
frame/view mismatch is rejected. Saved f32 values must round-trip exactly. The
summary checks primitive identity against GPU records when available; CPU float32
dot/cross operations can round differently from shader FMA at edges/ties. Replay
counts are CPU estimates until that validation, not GPU timestamps or an exact-hit
certification. The list adapter reports requested slots and horizon rejections
before emitting accepted rays. Compare its counts with the reflection probe.
Important correction: #133 selects the SUN census frame; the reflection probe
still uses PROBE_FRAME=30 at d7f57fa9. Select renderer frame 30 for both dumps
unless U-RTVIS adds a reflection-probe selector with the list work.

The two CPU checks (`test_replay_geometry.py`) pass flat/TLAS hit/miss and reversed
equal-t ties, plus glossy layout/epsilon, mirror horizon rejection, overflow
exclusion and frame refusal. These are fixture checks, not real-scene evidence.
No Outpost or Showcase snapshot has been captured yet.

After the list-dump PR lands, request one ordinary GPU turn from root for
owned Outpost and Showcase non-RT scenes, matched renderer frame/view for both
dumps and the selected probe. Record binary, scene, shader and dump hashes. Keep
the existing scene hashes visible and validate against main before taking rows.
Use no timing claim from that diagnostic lease. Release immediately after two
visits; replay and aggregation are CPU work afterwards. All images remain.
