# P-NRT3 (NOT VERIFIED): overlapped frame submission and a cheaper GI drain

Unit P-NRT3, 2026-11-09, branch `t3/p-nrt3` from main `0979ffe` (after P-NRT2).
Islam's bar: "without compromising even a bit in quality". Both changes here are pure
optimisations, so the bar is byte identity.

Scope (root's assignment): frame scheduling and submission (the renderer frame loop, encoders,
uploads, submits) and the GI cache (`src/gi_cache*`, GI shaders). Others own sun shadows
(P-NSQ2), local lights (P-LTS3), TR1 history (P-FLAT) and timing (P-RETIME).

Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## 1. Status: NOT VERIFIED

**Both changes are built but NOT VERIFIED.** No image, timing or test of this branch has run on
the GPU. Every argument below for byte identity is from reading the code (**I**), not
measurement. Root will not merge until leases q1 and l2 (section 4) have both passed.

- **Why.** Eight GPU leases between 15:37 and 16:50 UTC died before their first capture
  finished (section 4).
  - The host was saturated by about 32-45 cargo/rustc processes, most from other projects
    (glaze, stillkit and others), with 2.7-7 GiB of 32 GiB available.
  - Each binary's first ARA1 capture compiles its pipelines. That compile did not finish in
    40-170 s before the lease guard stopped the run.
  - The CPU gap, the first target, cannot be timed on a saturated CPU in any case.
- **Measured here:** nothing.
  - Before figures are P-NRT2's (**A**): 1.8-2.1 ms synchronous CPU gap, `render()` about
    0.85 ms, and GI fill 1.7-7 ms of span over ARA1's first 87 frames.
  - After figures, the GPU idle gap and the cut-fill cost are all unmeasured.
- **Quality settings:** none changed: the gather budget, the number of fill frames, the
  16-ray glossy budget and the 128-light shadow budget are all unchanged.

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
- Input-to-photon latency should be unchanged or lower (**I**, not measured).

### 2.2 GI gather: one parallel screening step for many queue entries (`shaders/gi_cache.wgsl`, `gi_gather`)

Drain frames after a cut run the capped gather: at most `GATHER_CELLS_SOFTWARE` (8,192)
cells on non-RT and 32,768 on RT. The fixed rotation by slot (W2) defers every other queued
cell to the next frame's queue.

Before this change, each of the 4,096 workgroups walked its share of the queue one entry per
step. For every deferred entry, all 64 lanes read the same slot, broadcast whether it was live
(a workgroup barrier), and lane 0 rewrote its state. Lane 0 then appended it to the next
queue (three same-address atomics) and the workgroup synchronised again. With 100-200k
queued cells, that is about 25-50 serial steps per workgroup before and between the few
live cells (**I**). The planned drain trace (q1) would show how much of a drain frame's
gather this overhead is, against the live cells' rays. If the rays dominate, the change
saves little.

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

## 3. Expected quality

Both changes are meant as pure optimisations, so the expected result is byte identity on
every frame, both paths (**I**). Section 2 gives the reasons per change:
- **The split:** the queue writes between the two submits touch only buffers that no
  first-half pass reads, and both halves keep the same queue order.
- **The gather:** each cell's work is unchanged; only the order of queue entries moves,
  and nothing reads that order.

Section 4's l2 lease is the check.

## 4. Verification pending (staged for a quiet host)

### Staged binaries

All in `D:/Coding/renderlab-work/p-nrt3/bin/`, release builds; sha256 prefixes:

| Binary | Source | sha256 |
|---|---|---|
| `base.exe` | `ara1_capture`, main `0979ffe` (`D:/Coding/pnrt3-base-tree`) | `12f16b41e5cbb0b7` |
| `cand1.exe` | `ara1_capture`, the screening build | `0a282532a67ab154` |
| `final.exe` | `ara1_capture`, this branch (`0149582`'s code) | `3f48e24f303bef51` |
| `base-lib.exe` | lib test binary, main `0979ffe` | `29633c0343d825da` |
| `final-lib.exe` | lib test binary, this branch | `6601f9a49e9931ec` |

`cand1.exe` takes two screening switches:
- `RENDERLAB_PNRT3_SPLIT`: 0 is one submit, 1 splits the encode only, and 2 also moves GI and
  post preparation after the first submit, which is the shipped form;
- `RENDERLAB_PNRT3_CPU=1` prints per-stage CPU timestamps from `submit_frame`.

Its GI gather is the shipped one. The switches are not in the branch.

### Plans

In `D:/Coding/renderlab-work/p-nrt3/plans/`, run through `tools/p-nrt/lease.py` with
`PNRT_OWNER=p-nrt3`, `PNRT_ROOT=D:/Coding/renderlab-work/p-nrt3`.
`tools/lease-retry.sh PREFIX PLAN` retries and resumes: `tools/todo.py` skips finished captures
and tests.

- **q1** (`l1.json`, about 12 min, needs COMPILER-QUIET):
  - ARA1 non-RT quality witnesses: `base` against `cand1` split 2, on game-base, ara-pbr,
    game-atmosphere and quarry;
  - 1080p timing, two interleaved repetitions, on game-base and quarry for `base` and
    `cand1` splits 0, 1 and 2, plus split 2 with the CPU probe;
  - Nsight traces (P-NRT's recipe):
    - base quarry still at submits 110 and 150, the drain frames;
    - base and split 2 game-base still at 1200, giving the GPU idle gap before and after.
  - Read with `tools/p-nrt/summarize.py runs/l1` and `tools/p-nrt/attribute.py`.
- **l2** (`l2.json`, about 14 min; identity only, a quiet host is not required):
  - ARA1 quality and timing frames on RT and non-RT, `base` against `final`;
  - the Q-walk (`gpu_tr1_sequence`) and RF1-RF10 (`gpu_r0a_current`), both paths;
  - MP1 mixed-program frames (`gpu_mp1_mixed_programs`), both paths;
  - the lib suite of `final-lib.exe`.
- **Afterwards:** quiet-certified final service timings on both paths need one more
  compiler-quiet lease from root.

### Failed attempts, 2026-11-09

All admitted with at least 8 GiB available, except where noted. No capture finished.

| Label | Ran | Cause |
|---|---|---|
| l1 | 42 s | runtime RAM floor (2 GiB) |
| l1b | 39 s | the guard's process query (`Get-CimInstance`) timed out at 25 s |
| l1c | 0 s | deferred at preflight: 7.2 GiB available, about 45 compiler processes |
| l1r-1 | 112 s | runtime RAM floor |
| l1r-2 | 167 s | the guard's `nvidia-smi` query timed out at 10 s |
| l2-1 | 0 s | deferred: GPU lock held by another unit for the whole 600 s wait |
| l2-2 | 74 s | runtime RAM floor; process query and `taskkill` timed out |
| l2-3 | 70 s | runtime RAM floor ("contention or available memory below 2048 MiB") |

Root could not open a quiet window: 32 of 36 compiler processes belonged to other projects.

## 5. Housekeeping

- **Evidence and binaries:** `D:/Coding/renderlab-work/p-nrt3/` (`bin/`, `plans/`, `tools/`,
  `lease/`), kept for the verification run.
- **Cargo targets:** `D:/Coding/pnrt3-target` and `D:/Coding/pnrt3-target-base` are deleted.
  The base worktree `D:/Coding/pnrt3-base-tree` (main `0979ffe`) is kept for rebuilding base.
- **Incident.** At 15:09 I ran `taskkill /IM cargo.exe` to stop my own build for a compiler
  hold. It killed every cargo.exe on the machine, including p-flat2's cand-lib build, which
  then logged exit 1. I reported it on the schedule, and p-flat2 rebuilt at 15:43.
