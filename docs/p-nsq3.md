# P-NSQ3: splitting a cut's direction batches over lanes

Unit P-NSQ3, 2026-11-09, branch `t3/p-nsq3` on main `ee361f5`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It follows P-NSQ2 ([p-nsq2.md](p-nsq2.md)), which found that on a 1080p cut the non-RT sun-visibility
pass spends about 32 ms (game 31.7-32.0, quarry 32.8-34.2), almost all of it in the shadow-map march repeated
for each of the cut's 32 batches of eight directions. Labels: **M** measured here, **I** inference.

## Result

**NOT VERIFIED: the candidate is built and validated by naga, but no GPU screen ran.** Every lease
this unit took (l1, l3, l6) was stopped by the 2 GiB runtime RAM guard within 17-45 s, before its first
capture, while other units' and other projects' builds (20-74 cargo/rustc processes, up to 10 GiB)
compiled. After root's 17:00 build rule (no GPU lease while any other cargo/rustc runs), the q1 lease
waited from 17:03 to the end of the time box and never started: a non-Renderlab workspace build
compiled throughout. So there are no timings, and no identity verdict. The branch carries the
candidate so that one short quiet lease can decide it (see [Next](#next)). **Do not merge before
that lease passes.**

- **RT** is untouched: no RT file changed.
- **Still frames** (one batch) run the old code path: with one batch each pixel's split is one lane.

## The candidate (I, by construction)

The default non-RT lookup is `VirtualLookup::Surface`. Its queued walk, `vsc_walk` in
`shaders/virtual_shadow_cache.wgsl`, gave each queued pixel one lane, which walked all of the frame's
direction batches in turn (`vsm_walk_batches`). On a cut that is 32 batches. A warp then waits for the
heaviest of its 32 pixels. P-NSQ2's split says lit pixels and pixels whose marches meet no candidate
are cheap, while penumbra pixels pay for every batch. The queue already gave a pixel several lanes
(`spread`, up to 8) when it was small, but only the first lane worked.

The change:

1. `vsm_walk_batches(px, first, stride)` walks batches `first`, `first + stride`, ... The Exact
   lookup passes 0 and 1 (all batches, as before).
2. `vsc_spread()`: on a frame of several batches (with persistent workgroups, the default), each
   queued pixel takes at least `VSC_BATCH_LANES` lanes (default 8; at most the batch count). The
   count is rounded down to a power of two so that a pixel's lanes share one 32-lane chunk.
   `vsc_missed` covers its own lane count, so `vsc_args`' dispatch is unchanged.
3. The pixel's first `min(spread, batches)` lanes each walk every `split`-th batch. Each leaves its
   visible count and work in its own slot of the existing `vsm_wg_dirs` table (its directions are no
   longer needed), so the change adds no workgroup memory. After a barrier, `vsc_finish` on the
   pixel's first lane sums them, writes the count and the queue word for `vsc_claim` as before.

Why it is exact. Each batch is N8e's eight-direction lookup: `vsm_visits`, the march's
`vsm_undecided` and `vsm_last` reset per batch, and its directions come only from the pixel's
sequence and the batch index. No batch reads another's results, so a batch gives the same verdicts
whichever lane runs it. Batch 0 still takes its directions from `vsm_prepare`'s call, and every
other batch from the loop's call, as before. The pixel's count is an integer sum. The work estimate
is summed from per-lane values clamped at 2^20. The reader takes only its bit length capped at 15,
so the sum gives the same 4 bits as the unclamped total (any clamped lane already puts the total
above 2^14). The partial values pass through the f32 table as exact small integers. The risk that
remains is the driver compiling the same WGSL to different floating-point code in the new kernel
(the same program in two modules has differed in bits before). Only a capture can rule that out.

Cost expectations (I): helper lanes repeat `vsm_prepare` (the same addresses in one warp, so mostly
issue cost) and one wasted direction batch each. P-NSQ2's directions-only screen puts a cut's 32
direction batches at about 1.5 ms. Moving frames (4 batches) also split, over 4 lanes. Whether the
balance gain beats that overhead, and which `VSC_BATCH_LANES` (2-32) is best, is what the screen
measures.

## Measurements (M)

None on the GPU. CPU checks only:

- naga parses and validates both modules (Exact, and Surface with the cache appended) for main and
  for the candidate at 2, 4, 8, 16 and 32 lanes (`tools/p_nsq3_naga.rs`).
- The lib suite was not run: test binaries run only inside a lease, and no lease ran.

Leases, all under the shared lock (owner p-nsq3, scripts/n8-run.py):

| Lease | Taken (UTC) | Outcome |
|---|---|---|
| l1 | 15:17:26 | runtime RAM guard after 88 s, no step finished (one second after root cleared a stale lock) |
| l3 | 15:40 | runtime RAM guard after 17 s, no step |
| l6 | 16:43:47 | admitted at 9.7 GiB; 4.9 GiB by 45 s with 23 rustc running; guard, no step |
| q1 | not taken | waited 17:03-17:49 for no cargo/rustc on the machine (root's 17:00 rule); a stillkit workspace build compiled throughout |

## Next

One quiet lease of about 3 minutes runs `plans/q1.json` (14 prebuilt steps, binary in
`D:/Coding/pnsq3-target`, deleted at the end of this unit, so rebuild it first; see Evidence):

- 1080p game timing scene, main against 4, 8 and 32 lanes, interleaved (frames 41, 72, 73 and 87
  captured): sun-visibility pass still/moving/cut/recovered;
- quarry timing, main against 8 lanes;
- identity of every captured frame against main: wide-sun and original-sun game quality scenes
  (12 frames each, still to recovered) and quarry quality (10 frames).

Keep a lane count only if every frame is byte-identical and the cut falls. If the split helps but
helper `vsm_prepare` dominates at high counts, the next step is a prepared-pixel broadcast through
`vsm_wg_shear` slots, at the same zero added shared memory.

P-NSQ2's own lead (sharing the shared steps' candidate walks across batches through workgroup
memory) stays open. The lookup is at 12 warps per SM with its shared memory full (N8m: 8,448 B a warp),
so any added table costs a warp of occupancy, which the record would have to beat.

## Evidence

`D:/Coding/renderlab-work/p-nsq3/` (external): `variants/` (main's two shader files as `base*.wgsl`,
the candidate at each lane count as `f<N>.wgsl` / `f<N>.vsc.wgsl`), `plans/` (`q1.json`, the
discarded l1-l6 plans), `runs/` (lease logs and preflight JSON with the RAM samples), `tools/`
(waiter `lease.py` with request-order and no-compiler gates, `holds.py`, `build.py` with the build
slot, `plan.py`, `mkplan.py`, `timing.py`, `identity.py`, `block.py`, and `p_nsq3_naga.rs`).
`tools/screen-override.patch` is the screen-only change that lets release examples load
`RENDERLAB_VSM_SHADER` and `RENDERLAB_VSC_SHADER`. It is not on the branch. Rebuild the screen
binary with that patch applied:
`cargo build --release -j 2 --example p_nsq_screen` with `CARGO_TARGET_DIR=D:/Coding/pnsq3-target`.
