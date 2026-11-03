# M4 streaming: reject unavailable publication owner closures

2026-11-03. One selector optimization is retained after frozen real-input equivalence,
a short live screen and inspected quarry captures. The optimized moving-stream target
of at most 8 ms for advance plus draw remains unmet. This does not establish Unreal parity.

## Change and proof boundary

The selector already rejects a publication when its seed needs a missing page in the
final admitted plan. The check now visits inactive owners required by that publication,
stopping when any of their plan-uncovered boundary pages is unavailable. This avoids
building, applying and rolling back a transaction whose publication cannot succeed.

The check runs only when the entire mesh's cluster count and the complete page universe,
including external protection, fit the output and geometry capacities. Otherwise the
ordinary transaction runs and reports capacity debt as before. The traversal reuses
the existing call-owned journal and proposal bitmap, clears its marks on both outcomes,
and changes no active cover, page references or retained resources. Candidate order,
demand, debt, scratch admission and in-flight ownership are unchanged.

## Evidence and limits

Frozen inputs came from actual stream calls at canonical path frames 250, 253, 261 and
277, using all twelve scans, 4K textures, 4x MSAA, 16x filtering, 512 MiB geometry and
read128/upload64 admission. All 59 captured calls match the unchanged independent
policy reference and checked/prepared selectors for cut, demand, debt and candidate
count. Both historical references remain unchanged.

Profiling attributed 39.272 of 52.956 ms across those calls to publication transactions;
7,079 of 7,095 publication attempts failed only for unavailable pages. The new check
reduced publication transaction touches from 8,448,926 to 1,398,993. Total counted
cluster touches fell from 12,877,326 to 6,963,452, including the new proof traversal.

Without profiling, five batches of four corpus passes (236 calls per batch) measured
median 215.199 ms before and 84.869 ms after, ranges 210.343–217.895 and
83.780–89.517 ms. Thread-cycle medians fell from 865.6 million to 345.4 million.
A later control bracket remained 217.359 ms median. These are repeated frozen selector
calls, excluding projection, runtime bookkeeping, I/O, GPU work and presentation.

One short live pair used canonical configuration and path frames 229–337. Motion
statistics cover frames 230–277: CPU advance-plus-draw p50 was 26.278 / 14.014 ms and
p95 35.856 / 23.540 ms before/after. Host CPU peaks were 69.0% / 80.4%; the control's
startup was much colder. Both finished without debt, I/O failures or retries and stayed
within geometry budget. Asynchronous publication differed slightly. This supports the
frozen mechanism result, but is neither quiet latency nor a replacement measurement
of the full 338-pose path or its historical 37.620 ms p95 baseline.

A separate quality pair used frames 249–277 and 24 final settled frames. Both had active
Khronos validation, zero errors, 53 legal-cover proofs and zero final debt. All four raw
captures (249, 253, 261, 277) are byte-identical. The before/after contact sheet and full
1920×1080 cut frame were inspected: the complete visible silhouettes, quarry patches
and texture detail are retained. Existing unlit appearance and authored patch boundaries
remain; this does not certify lighting, shadows, transparency or arbitrary motion.

Focused checks passed: 14 selector tests, including 512 new owner-witness/capacity/
refinement cases and the existing exhaustive/seeded policy comparisons; 18 runtime CPU
tests covering retained-cut liveness, quiet tails and shared admission; asset Clippy with
both selector features and all targets; formatting. No broad GPU campaign was repeated.

The complete local handoff, source archives, input/output hashes, commands and timings
are in `.refs/m4-stream-next/handoff.md` in the isolated selector worktree, backed by
`D:/Coding/renderlab-work/m4-stream-next`. The earlier page-range candidate is not
included. Root owns integration and any full-path acceptance measurement.
