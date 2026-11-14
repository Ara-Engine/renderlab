# Transparency stage measurements

Default off. Set `RENDERLAB_TRANSPARENCY_LOG=<jsonl path>` to append CPU-side
work counts for each nonempty transparency encode. The destination's parent must
exist. Unset (or empty), the hook writes nothing and inserts no GPU commands.
No shader text, key, binding, render-pass attachment or dispatch changes.

For timestamps also set `RENDERLAB_PASS_LOG=<passes.jsonl>` and
`RENDERLAB_PASS_LOG_DETAIL=1`. The existing pass timeline receives these parts:

- `transparency.depth`: opaque depth preparation.
- `transparency.sun_map`: a sun-map refresh, when invalidated.
- `transparency.particles`: particle prewarm/update, lighting and GPU sort.
- `transparency.setup`: interval after particles and before the first draw operation.
- `transparency.copy`: HDR copies, including pane/span updates.
- `transparency.lobe`: full-target glass lobe clear/rasterization.
- `transparency.reflection`: the TP2 glass reflection dispatch.
- `transparency.pane_ids` and `.pane_shade`: pane coverage and compute shading.
- `transparency.raster_seed_glass_resolve`: one unchanged raster pass, including
  its seed and 4x resolve where applicable. It also covers particle raster passes.

Repeated names add up. Sum `transparency` and all `transparency.*` values for
the group total. Names describe GPU command intervals, not isolated CPU costs.
The existing 64-stamp frame budget is preserved: the diagnostic leaves space for
ids, post and the final stamp. At exhaustion it records `transparency.remainder`
and sets `detail_complete=false`; do not attribute that remainder to the previous
part. Missing DETAIL/timeline also sets `detail_complete=false`. Other stages may
already have exhausted the global pool, so the complete flag must be checked.

The CPU JSONL includes the renderer's zero-based frame and view, path and target
size; whether the sun map refreshed; draw/emitter/capacity counts; particle update
and sort dispatch counts; copy calls/pixels; lobe runs and runs whose programs
cannot declare reflection; lobe target pixels (one view area per run, not three
attachments' bytes); reflection dispatches/pixels; pane ID/shade dispatches and
rectangle pixels; raster/MSAA passes and full-view seed/resolve pixels. Areas count
repeated work, not unique coverage. They do not count executed rays, actual shaded
fragments, bandwidth or GPU traversal. Non-reflective runs are a diagnostic only:
they are still rendered exactly as before.

Join counters and pass logs with their documented frame conventions. The counter
uses `Renderer.frames` before submission (first frame 0); the timeline has its own
per-renderer frame counter (first frame 1). Keep view and source identities in the
capture manifest. The file appends across runs: use a fresh path per visit.

No new GPU resource is allocated by this diagnostic. Logging and additional
timestamp passes perturb measurement; compare paired rows under the same setup.
A GPU trace is needed to split seed, shader work and resolve inside a raster pass
without changing its boundaries. No such split is claimed by this instrumentation.
