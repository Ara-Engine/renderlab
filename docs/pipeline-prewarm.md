# Pipeline preparation and first-view cuts

## Inventory

The forest repeated-cut fixture on release `6a31304b` holds A for frames 0–399,
turns to B at 400, back to A at 460 and to B again at 520. At 1920×1080, both
paths, the resource ledger creates pipelines only on the first arrival at B.
An opt-in compilation-label replay identified these render keys:

| Path | Key suffix | Vertex entry | Fragment entry | Samples | Why absent before B |
| --- | --- | --- | --- | --- | --- |
| RT and non-RT | glass, tangent=false | `vs` | `fs_transparent` | 4 | Transparent standard-program draw was culled |
| RT and non-RT | glass, tangent=true | `vs_tangent` | `fs_transparent` | 4 | Same lazy program set, including its tangent variant |
| RT and non-RT | lobe, tangent=false | `vs` | `fs_tp_lobe` | 1 | Same lazy set |
| RT and non-RT | lobe, tangent=true | `vs_tangent` | `fs_tp_lobe` | 1 | Same lazy set |
| RT and non-RT | film, tangent=false | `vs` | `fs_transparent` | 4 | Standard program declares `FILMS`; same lazy set |
| RT and non-RT | film, tangent=true | `vs_tangent` | `fs_transparent` | 4 | Same lazy set |
| RT and non-RT | pane ID, tangent=false | `vs` | `tp_pane_id_fs` | 4 | First visible program with per-pixel panes |
| RT and non-RT | pane ID, tangent=true | `vs_tangent` | `tp_pane_id_fs` | 4 | Same lazy pane set |
| non-RT | local atlas clear | `vertex` | `fragment` | 1 | A new local-shadow atlas recompiles its clear pipeline |

The first eight rows each name two distinct pipelines, one per sun path: 16
render pipelines, plus the one local clear = 17. The full glass key hashes the
renderer prelude digest, material-program digest, hardware-ray path and parameter
layout presence. The suffix adds tangent/lobe/film choice. Pane keys use that
same source identity plus the pane ID or shading stage. The local-clear key is
the bundled `local_light_clear.wgsl`, no vertex buffers, no bind groups,
Depth32Float, always-write depth, one sample; it does not depend on atlas size.

Additional compute creation at B: pane shading on both paths, and TP2 glass
reflection on RT. The reflection key hashes the opaque shading prelude, first
opaque program and parameter-layout presence. No acceleration structure is built
at B. Returning to A/B creates no pipeline objects.

These creations establish lazy CPU preparation. They do **not** establish that
the observed GPU atmosphere/visibility spikes are compilation of these objects;
the repeated-cut prewarm experiment must separate creation from driver first use.

## Logging through a host engine

Set `RENDERLAB_PIPELINE_LOG=<fresh file.jsonl>` before constructing the wgpu
engine. It appends CPU compilation events (kind, label, milliseconds) and one
frame-end marker for each successful `Renderer::try_render_view`. Ara's Windows
bridge calls that path through `Engine::render_view`. No adapter-specific seam
is required. The option is off by default and issues no GPU readbacks or waits.

Frame records carry the submitted renderer frame ID, cumulative render/compute
and shader resource counts, BLAS/TLAS creation and build counts, owned target and
shadow bytes, and visible instance count. Assign preceding compile events since
the previous marker to the following frame. The first marker also includes
engine/scene loading. Join its `frame` with `RENDERLAB_PASS_LOG`'s renderer frame
ID. Device creation through raw externally owned wgpu handles is not intercepted;
resource ledger counts also count cloned handles, so use compile events to
distinguish actual native creation from ownership changes. Concurrent shader
reload events can interleave: this diagnostic assumes no reload during a cut test.

`RENDERLAB_STARTUP_PROFILE=1` remains independent: it retains its bounded public
timing queue and stderr output; pipeline logging does not drain that queue.
Logging I/O failures do not reject rendering. Compilation milliseconds are CPU
creation time, not GPU execution time or a measurement of driver residency.
