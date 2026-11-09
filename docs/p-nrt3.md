# P-NRT3: overlapped frame submission and a cheaper GI drain (zero quality loss)

Unit P-NRT3, 2026-11-09, branch `t3/p-nrt3` from main `0979ffe` (after P-NRT2).
Islam's bar: "without compromising even a bit in quality". Both changes here are pure
optimisations, so the bar is byte identity.

Scope (root's assignment): frame scheduling and submission (the renderer frame loop, encoders,
uploads, submits) and the GI cache (`src/gi_cache*`, GI shaders). Others own sun shadows
(P-NSQ2), local lights (P-LTS3), TR1 history (P-FLAT) and timing (P-RETIME).

Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## 1. Status

RESULTS-PLACEHOLDER

## 2. Changes

### 2.1 Two submits per lit traced frame (`src/renderer.rs`, `submit_frame`)

P-NRT2 measured a synchronous CPU gap of 1.8-2.1 ms per frame. `render()` itself took about
0.85 ms, of which "encode" was 0.67-0.89 ms. That figure covers the GPU-side preparation
(`prepare_primary`, the visibility buffer, Virtual shadows, TR1, the GI cache and post) as
well as command recording. The GPU stays idle until the single submit at the end.

Lit traced frames (both paths' default: visibility buffer or traced primaries, any soft-shadow
mode except Temporal film) now submit twice:

1. **First half.** Skinning, light clusters, the TLAS build (RT), shadow maps, transmission,
   atmosphere, fresh shadows, the visibility or primary pass, and sun visibility (Virtual's
   pages and N8n, or the fresh/interactive passes, with VF1's fog injection). These are
   submitted as soon as they are encoded.
2. **Second half.** The CPU then prepares the GI cache (`prepare_gi`) and post
   (`prepare_post`). It encodes the GI passes, deferred shading, TR1, the residency readback,
   transparency, HDR present and the timing resolves, and submits them.

Why the images cannot change:

- **Same order.** Both halves run on one queue in the same order as before. wgpu inserts the
  transitions between submissions, as it did between passes.
- **Write audit.** `queue.write_buffer` data reaches the GPU before the next submission. The
  writes that now land between the two submits are:
  - `prepare_gi`: the cache header, frame block, draw table, arrays and software geometry;
  - `prepare_post`: post's frame and settings uniforms;
  - `encode_transparency`: its frame, draw-slot, emitter and reflection-slot buffers.

  None of these is read by any first-half pass. The only first-half shaders that compose the
  GI API are program-composed ones; none of them runs before shading, and the atmosphere
  kernels do not include it. Every write made before the first submit lands before it, as
  before.
- **CPU state.** No first-half encoder reads state that `prepare_gi` or `prepare_post` sets.
  The GI plan, post and transparency state are read only by second-half encoders, and the
  lighting bind group is rebuilt in `upload_lighting`, which still runs first.
- **Frame bookkeeping.** `self.submissions` (resource retirement) and the frame's flight are
  recorded at the second submit. Completion of the second half implies completion of the
  first.

Other frames (raster, diagnostic, Temporal film) keep one submit and their preparation order.

`CpuFrame` keeps its meaning: `submit` is now the sum of both submit calls, and `encode`
excludes them.

**Failure mode.** If GI or post preparation fails after the first half is submitted, the
renderer faults (`fail`), as any preparation error does. The first half then completes on
the GPU without a flight record. A faulted renderer accepts no further frames, so nothing
reads its results.

**Latency and present order.** `render()` returns after the second submit, as before, and
presents the same frame in the same order. Its CPU time grows by one `queue.submit` call
(about 0.08 ms per frame, P-NRT2's submit timing). The GPU starts that much earlier:
everything after the first submit overlaps GPU work.
- In a synchronous loop (ARA1's service), the frame completes earlier by the overlapped CPU
  time.
- In a pipelined game loop (frames in flight), a GPU-bound game sees its frame finish earlier
  after its CPU submit. A CPU-bound game pays the extra submit call.
- Input-to-photon latency is unchanged or lower (section 4 measures it).

### 2.2 GI gather: one parallel screening step for many queue entries (`shaders/gi_cache.wgsl`, `gi_gather`)

Drain frames after a cut run the capped gather: at most `GATHER_CELLS_SOFTWARE` (8,192)
cells on non-RT and 32,768 on RT. The fixed rotation by slot (W2) defers every other queued
cell to the next frame's queue.

Before this change, each of the 4,096 workgroups walked its share of the queue one entry per
step. For every deferred entry, all 64 lanes read the same slot, broadcast whether it was live
(a workgroup barrier), and lane 0 rewrote its state. Lane 0 then appended it to the next
queue (three same-address atomics) and the workgroup synchronised again. With 100-200k
queued cells, that is about 25-50 serial steps per workgroup before and between the few
live cells.

Now `width = clamp(count / groups, 1, 64)` lanes each screen one consecutive entry at once:

- **Deferred entries** rewrite their own state with the same formula as clearing
  `GI_BQUEUED` and then calling `gi_schedule(slot, max(left, 1))`. The workgroup adds its
  deferrals to the next queue's counter, `GI_W_BQUEUE` and `GI_W_GATHER_DEFERRED` once, and
  each lane stores its slot at its offset.
- **Live entries** go into a workgroup list. All 64 lanes then gather them one after another
  with the original code: the same directions, rays, hit lookups, fixed-tree sums, writes and
  schedule.
- When the queue is shorter than the grid (`count <= groups`), `width` is 1 and the kernel
  walks one entry per step as before.

**Why the images cannot change.**
- Each cell's computation and its writes are the same. A cell is in the queue once
  (`GI_BQUEUED`), and its state is written only by its own processing.
- Gathers read other cells' last-frame outgoing radiance and orphan bits, which this pass
  does not write.
- What changes is the order of entries in next frame's gather queue and in the shading
  queue. That order was already set by atomics across workgroups, and nothing reads it as
  meaning: the gather's slot group is chosen by slot, not position, and the filter's
  first-marker rule writes the same words whichever item wins.
- Counter totals are equal.

P0 constraints hold: no counter averaging, no ranking, no jitter, a fixed query point, and the
same gather budget and number of fill frames.

### 2.3 Tools

- `tools/p-nrt/plan.py`: capture and trace steps take an optional per-step `env`. A trace
  passes it through ngfx's `--env`.

## Seams touched (for root)

- `src/renderer.rs` `submit_frame`: the frame's GI and post preparation moved after the
  first submit on lit traced frames (new helper `prepare_cache_and_post`). Units that add a
  pass must place it in the half that matches its inputs. A first-half pass must not read a
  buffer written by `prepare_gi`, `prepare_post` or `encode_transparency`. A pass that does
  belongs after the split.
- `shaders/gi_cache.wgsl` `gi_gather` (mine).
