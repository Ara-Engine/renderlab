# M4 follow-through priority

Islam renewed the request on 2026-11-04 after the streaming and resize issues remained open through the M5/M6 shadow work. Root let those workstreams become inactive. Their earlier improvements do not close either issue.

The original M4 milestone scope is accepted. The real-scan follow-through remains unfinished. Islam then asked for M5/M6 to continue at the same time. A third worker (GPT-6.1 Sol) owns the bounded fresh compute shadow unit in its own worktree. It does source and CPU work, and stays out of streaming internals and viewer resize code until root transfers the GPU slot. Reuse heuristics and soft-4x expansion remain paused. Existing shadow candidates and evidence stay preserved.

## Streaming owner: Opus 5.5

Target optimized CPU p95 of at most 8 ms for streaming advance plus drawing on the registered quarry, with unchanged content, quality and budgets. The historical full-path result is 37.620 ms. The retained owner-witness optimization reached 23.540 ms in a shorter live screen; those are different run extents, not a certified full-path improvement.

Start from the retained 59-call frozen corpus and current source. Profile the remaining work, change one measured mechanism at a time, and reject weak ideas using CPU replay first. Preserve exact cuts, demand, debt, legal coverage, resource bounds and timing semantics. Use a short paired live screen for a promising candidate. Run the full path only when needed to establish the target or resolve a specific discrepancy. Full-resident rendering does not count as fixing streaming.

## Resize owner: GPT-6.1 Sol

Own the historical 4.933-second `Surface::configure` stall through a causal conclusion or a useful, verified diagnostic boundary. Prior draining and recovery improvements are retained. Zero renderer flights do not exclude presentation-idle waits inside Vulkan. Current outer probes cannot distinguish those waits from swapchain recreation or scheduling delays.

Prepare one bounded native/API or scheduling observation that separates the possible internal waits, with a concrete slow-call trigger and an external deadline. Reuse the existing logs, probes and native helper. Keep foreground focus unchanged and capture only the owned renderer window if images are needed. No speculative synchronization bypass, present-mode change or driver workaround follows from source inspection alone. A run without recurrence cannot be called a root-cause fix.

## Coordination and closure

Each worker owns a separate tool-created worktree and ignored evidence. Root owns this record, integration, acceptance and pushes. Opus first owns the shared GPU/heavy-build slot; Sol does source and light CPU work until root transfers that slot explicitly. Their files do not overlap. Use GPT-6.1 Sol and Opus 5.5 only, with xhigh reasoning and two Cargo jobs.

Report actual source/binary/content identities, focused image or equivalence evidence, costs and remaining failures. Separate implementation from verification time. Preserve rejected evidence and avoid repeated campaigns after a decisive result. Root must either publish the verified fix or state the concrete remaining blocker; these workstreams must not silently disappear behind another milestone again.

## Streaming result, 2026-11-04

Opus's hillclimb roughly halves full-path p95 but misses the 8 ms target. On the full 338-frame quarry path, interleaved base/final runs at 47-51% host load give p95 34.06 ms -> 17.28 ms and p50 18.25 ms -> 10.09 ms. At 72% load p95 goes 47.96 ms -> 19.29 ms. Final debt, triangles (3,981,802), upload counts and geometry peak are unchanged. Late-window captures are byte-identical to base with zero validation errors.

Retained mechanisms: one upload submission per tick, running page-state totals (checked against full recounts over the whole path), linear merge of sorted page lists, up-front rejection of doomed selector publications, and sorting only refinable groups. Selector outputs match on all 59 frozen calls.

The remaining limiter is upload frames. 72-76 of 277 motion frames upload 48-64 pages; their median is 14.7 ms (selection 4.7, uploads 4.3, draw encoding 4.1). Frames without uploads have a median of 6.26 ms. The next candidate is pooling page buffers, which touches the shared `Mesh` ownership. Host contention from other builds inflates the absolute numbers. Evidence: `D:/Coding/renderlab-work/m4-stream-renewed-t3-0298b78f/`.

## Streaming page pool, 2026-11-04

Opus's second hillclimb pools virtual page geometry into shared 32 MiB buffers with fixed slots, queues one write per page and packs in place. Over 8 interleaved full-path runs against 7f756d6, p95 drops 19.27 -> 9.16 ms at about 56% host load and 21.61 -> 10.55 ms at about 70%. The best final run is 8.46 ms p95 / 7.13 ms p50 at 33% load. The 8 ms target remains narrowly unmet under heavy foreign contention.

Final state matches: debt 0, 3,981,802 triangles, and late/upload-window captures 4/4 byte-identical including mid-stream frames, with zero validation errors. Live buffers drop from 15,755 to 43. Page memory rises from 497.7 to 536.6 MB because each page takes a full slot, still within the 536.9 MB budget. Larger 134 MB chunks were rejected because their dedicated allocations caused blocked uploads.

Selection is now the limiter: 4.6-4.9 ms median per frame and 6-7 ms on the grazing frames that set p95. About 35% of that is the blocked-publication check. A 49-call grazing corpus is captured for the next pass. Separately, the replay harness's 60-frame game-process check (about 24 ms) couples to page reading and can shift the final cut under contention. Evidence: `D:/Coding/renderlab-work/m4-stream-pool-t3-3c27758a/`.

## Resize observation, 2026-11-04

Root ran Sol's bounded `scripts/resize-causal.py` sequence twice on a background viewer, using the real 4K quarry, 4x MSAA, 16x anisotropy, FIFO and active validation, with no game running. Both runs completed all 16 transitions with no slow configure: the initial configure took 304/199 ms and later ones 15-55 ms. They presented 467/323 frames, request-to-configure waits stayed under 1 ms with zero deferrals, and validation reported 0 errors. The owned-window recovery image is complete.

The first run exposed a launcher defect: CDB rejects `-pd` with noninvasive `-pv` ("does not support detach on exit"). Root removed `-pd`. The second run verified PID, window thread and viewer symbols in the stack snapshot. The armed method is now validated, but the historical 4.933 s stall did not recur, so its cause remains unresolved. Any later slow configure will capture the window-thread stack automatically. No renderer change follows. Evidence: `D:/Coding/renderlab-work/m4-resize-renewed-t3-3f89cd2d/native-20261104-114357/`.

## Streaming target met, 2026-11-04

Opus's third hillclimb meets the 8 ms target at the contention levels measured. On the full 338-frame quarry path, interleaved with 4 runs per side at 21-50% host load, the median p95 drops from 8.01 to 6.23 ms and p50 from 6.94 to 5.16 ms. All candidate runs fall between 6.12 and 7.21 ms p95. Heavier contention (56-70%), where earlier runs reached 9-10 ms, was not available today, so the target is not established there.

Kept selector mechanisms, each measured with frozen replay: ascending-order early stop in the blocked-publication check, per-mesh page-window state instead of package-wide state, and whole-group skipping in cluster cuts. Selector cost falls by about 40% on both the 59-call and 49-call grazing corpora. Outputs are exact on both corpora and on all 3,701 live calls in a reference-checked full-path run. All 8 late/upload-window captures are byte-identical to base, with zero validation errors. The replay harness's game check now runs on its own thread, removing the 12-15 ms frame spikes every 60 frames.

Remaining caveat: under an IO stall the final debt-0 cut can settle at 4,293,908 instead of 3,981,802 triangles (1 of 8 runs). That is a retention-policy dependence on IO timing, not a selector or harness defect. Evidence: `D:/Coding/renderlab-work/m4-stream-selector/`.
