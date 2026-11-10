# P-S3Q: replaying the quarry cut's shared walks, rejected at its first screen

Unit P-S3Q, 2026-11-10, branch `t3/p-s3q` from main `a862d9b`. Milestone 8 ([milestone-8.md](milestone-8.md)).
It builds the quarry lead in [p-s1.md](p-s1.md#next): design S3 of
[design-nonrt-sunvis.md](design-nonrt-sunvis.md) for the quarry cut, where P-S1 counted 34% of the
walk work (9.99 M of 29.2 M visits at cut frame 72) as replayable. Labels: **M** measured here,
**I** inference.

## Result

**DON'T MERGE. The replay is exact but slower, and the shader is not changed on the branch.** The
branch carries this record and the gaps entry. The candidate is kept as a patch in the evidence.

- **Identity (M).** All 40 captured frames are byte-identical to main (decoded PNG pixels): quarry and
  game timing frames 41 (moving), 72 (cut), 73 and 87 (recovered), across 5 runs per scene and every
  variant. Main's shaders are unchanged since P-S1's base, so P-S1's numbers apply.
- **Occupancy (M), Nsight multi-pass trace of the `vsc_missed` walk.** Unchanged: 168 registers per
  thread, 8,448 B of shared memory per warp, a 12-warp plateau. The candidate holds no workgroup
  memory, so occupancy does not explain the result.
- **First paired sample (M), the sun-visibility pass, medians in ms:**

  | Scene | Variant | still | moving | cut | recovered |
  |---|---|---:|---:|---:|---:|
  | quarry | main | 1.273 | 1.600 | 9.222 | 1.951 |
  | quarry | S3Q | 1.347 | 1.646 | **11.004** | 2.065 |
  | game | main | 2.864 | 5.596 | 26.734 | 5.566 |
  | game | S3Q | 3.032 | 5.927 | **28.658** | 5.883 |

  The quarry cut is 1.8 ms slower, the target was faster, and every other phase is slower too. Per
  the brief, the unit stops at this screen. No ABBA, lib or ARA1 run was spent on a candidate that
  is slower.
- **Diagnosis (M), one more lease with variant files and no rebuild:**

  | Variant | quarry cut | game cut | game moving |
  |---|---:|---:|---:|
  | main | 9.494 | 26.723 | 5.596 |
  | replay compiled out (`VSM_REPLAY` false) | 9.203 | 26.741 | 5.598 |
  | records, never replays | 9.180 | 27.765 | 6.484 |
  | full S3Q (l1) | 11.004 | 28.658 | 5.927 |

  Recording costs nothing on the quarry cut. The replayed tests themselves are what is slower than
  the walk they replace: about +1.8 ms on the quarry cut. On the game, recording alone costs about
  1 ms of cut and also slows moving frames, which never record. That is codegen.
- **Why (I).** The walk's chained loads are cache-hot: L1TEX hit rate is 59-64% in the trace, and the
  walk only goes one triangle at a time. The record sits in per-lane local memory: 64 tests and 16
  keys, about 0.5 KB a lane. That is 190 KB per SM at 384 resident lanes, against the 28 KB of L1
  that the 100 KB shared carve-out leaves, so the record is served from L2. Each replayed test then
  waits on an L2 record read where the walk waited on an L1 vertex and adjacency read, and the
  8-direction hit test runs either way. Prefetching the next entry and triangle did not recover
  it. This is the same failure as P-NSQ3's register ring replay (40% slower), from the other side:
  neither registers nor local memory hold a record cheaply at 12 warps. Workgroup memory is full
  (N8m).
- **RT** runs no file the candidate touched. The non-RT default is main's.

## The candidate (kept as `s3q-replay.patch`)

All of it is in `shaders/virtual_shadow.wgsl`, with nothing in the atmosphere module's prefix:

- **Why it is exact.** A shared step's candidate walk, `vsm_texel_hits`' attempt 0 with `j == 8`,
  aims at the shared origin's xy along the central ray. With `walk_cap: 0` and `VSM_LEAN` 0, its path
  depends only on its start triangle, its height `q.z` and its step bound. So do the triangles it
  tests and its end flags (bits 8-10 and y). The batch's directions decide only each test's bits,
  and the early stop when the whole mask is blocked.
- **Recording.** In a lane's first batch, when that lane has later batches (cuts only), each shared
  walk that ends naturally is recorded: its tests in order, with bit 31 marking a walk step counted
  in `vsm_visits`. The key is (start id, `q.z` bits, steps), with the end flags and last triangle.
- **Replay.** A later batch that finds the key (a cursor plus a 4-record window) tests its own mask
  against the recorded triangles in order. It stops where the walk would, takes the recorded flags
  at the end, and loads the next entry and triangle ahead.
- **Fallback.** Anything else walks as before: a missing key, a full record, or a walk that stopped
  early.

The byte identity confirms the exactness argument. The cost rejects the mechanism.

## Next

- Shared-walk replay across batches is closed for this kernel: registers (P-NSQ3), local memory
  (here) and workgroup memory (full, N8m) all fail to hold a record more cheaply than re-walking a
  cache-hot path.
- The quarry cut's 34% stays a lead only with a different shape, one that removes the repeated
  8-direction tests rather than their location chain. For example, test all of a lane's batches'
  directions against each shared-walk triangle in one pass. That means restructuring the march to
  run batches together at shared steps, and it changes the order and early-stop structure that
  byte identity needs, so it needs its own exactness argument first.
- The game cut's leads are unchanged (P-S1): a cheaper exact per-direction walk, or S4.

## Evidence

`D:/Coding/renderlab-work/p-s3q/` (external):
- `s3q-replay.patch`: the candidate, against main `a862d9b`.
- `variants/`: `base.wgsl` (main), `s3q.wgsl`, `s3q-off.wgsl`, `s3q-rec.wgsl` and `base.vsc.wgsl`.
- `plans/`: `l1`, `l1b` and their `results.jsonl` (per-step exit status and PNG SHA-256), plus the
  unrun `l2` (ABBA) and `l3` (ARA1 identity on both paths, with HDR hashes, and the lib suite).
- `runs/`: `l1/` and `l1b/` (`run.json` timing and frames per step, and `trace-*/occ.json`
  occupancy), with the lease preflight and sample records in `runs/lease/runs/`.
- `tools/`: `guard.py` (stillkit mutex, build slot, admission, stop orders), `build.py`, `plan.py`
  (screen, ARA1 capture, multi-pass trace steps), `mkplan.py` and `identity.py`.
- `bin-sha256.txt`.

Build:
- **One build** (build-1, 5.1 min at -j 2). It ran with the build slot and the stillkit mutex held, no
  other compiler running and 14.3 GiB available.
- It made the screen-patched release `p_nsq_screen` and `ara1_capture` (P-S1's `screen_patch.py`,
  reverted after the build), the naga checker and the release lib test binary.
- SHA-256 prefixes: `p_nsq_screen` d7418d7c, `ara1_capture` 8c90ea44, lib 69c2eb2c.
- The target (650 MB), the binaries and the GPU traces were deleted at the end.

Leases, under `scripts/n8-run.py` as owner p-s3q, root order 23:48 (p-s3q alone):
- `p-s3q-l1`: 8.2 min. naga ok for both modules of each variant; two traces; the two pairs.
- `p-s3q-l1b`: 7.2 min. Admitted at 14.1 GiB; at least 5.5 GiB available throughout.

Hardware: RTX 4070 SUPER, driver 617.14, 1080p ARA1 timing scenes (`game-base-timing`,
`quarry-daylight-timing`), Surface lookup, release build.

Not run:
- The ABBA blocks, the quality scenes, ARA1 identity on both paths and the lib suite. The candidate
  failed its first paired sample, and the branch changes no code.

Time:
- implementation (design, shader, harness): about 1.2 h;
- verification (build, two leases, analysis): about 0.8 h.
