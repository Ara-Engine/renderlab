# P-GIE: cooperative tile lookup rejection gate

Unit P-GIE, Milestone 8. Baseline `21088f37f5159385e69d3b7f5691bf185cb33359`
(origin/main at unit start), branch `t3/p-gie`. Design: G-E in
[design-nonrt-gi.md](design-nonrt-gi.md). The unit first applies the requested
40-unique-cell limit per 32-pixel tile; an average cannot establish this maximum.

## Decision

**DON'T MERGE G-E: the requested rejection gate fails on both scenes and paths.**
No cooperative shading lookup was implemented. This branch contains the gate report,
measurement source-preparation/summarization tools and a gaps update only.

## Measured rejection gate

Lease `p-gie gate-1`, 2026-11-09 21:19:13-21:21:40 UTC, RTX 4070 SUPER,
NVIDIA 617.14, release measurement build. Each configuration ran all 88 registered
1080p frames (40 still, 32 moving, 16 cut). No local-table overflow occurred.
Numbers below are maxima of unique probe keys in **one 8x4 tile**, not averages.

| Scene / path | Still maximum | Moving maximum | Cut maximum | Gate (<=40) |
|---|---:|---:|---:|---|
| Game / RT | 58 | 56 | 52 | FAIL |
| Game / non-RT | 83 | 56 | 52 | FAIL |
| Quarry / RT | 123 | 126 | 143 | FAIL |
| Quarry / non-RT | 128 | 125 | 143 | FAIL |

Across phases, unique keys per 32 covered pixels average about 11.30-11.35 on game,
and 24.56-25.54 on quarry. Thus the *average* would pass; the explicit rule
"if a tile reads more than 40 unique cells per 32 pixels" does not. Rejecting this
bounded design does not prove that a future cooperative lookup with an exact
spill/fallback cannot pay. That would require a new design and gate.

## Method

`tools/p-gie/probe.py` freezes the baseline into an external measurement source tree.
Only that tree changes: the existing probe is dispatched at 8x4 rather than 8x8,
its lane stride is 32, and the ARA1 adapter reads the existing `GI_F_STATS`
`ProbeCounters` after every profiled frame. Neither the production shading lookup
nor its arithmetic changes. Both registered ARA1 1080p timing scene plans are kept,
including still, move and recovered/cut frames, on RT and non-RT.

These are **probe request unions**, including conservative bin/level ties, parents
and refinement rounds. They are the design's specified cheap rejection statistic,
not exact counts of shading-only live records. Changing probe tile grouping can
change insertion order; this measurement binary is not an image-identity candidate.
Its serialized counter readbacks make its service times unsuitable for a speed claim.

Raw counters, device limits, binary SHA256 and summaries live under
`D:/Coding/renderlab-work/p-gie`. The source-preparation script checks every edit
seam before replacement. `tools/p-gie/summarize.py` reports maxima separately from
means and records counter-file hashes.

## Workgroup memory

A `GiHot` record is 96 bytes. A possible bounded G-E design would use 32 records
(3,072 bytes), 64 key/index entries (512 bytes) and 16 bytes of control state:
**3,600 bytes**, with an exact direct-lookup overflow path. This is a sizing proposal,
not an implemented or occupancy-certified kernel. Forty records alone use 3,840
bytes; the original roughly 5 KiB proposal cannot assume unchanged occupancy.

R2's proposed queue/results need about 7 KiB, and R3's stack 4 KiB. With a 3,600-byte
G-E allocation they total about 14.52 KiB. At a 16 KiB device limit that leaves
1,520 bytes, before other live shared variables and alignment. Fitting the API limit
is not proof of unchanged occupancy. NVIDIA's [Ada tuning guide](https://docs.nvidia.com/cuda/ada-tuning-guide/index.html)
specifies 24 resident blocks and 100 KiB shared storage per SM; a 5 KiB block already
limits the shared-storage block count below 24, even before other allocations.
Actual Vulkan occupancy also depends on driver allocation and register usage, so
an accepted implementation would need a compiled-kernel measurement.


The measured `max_compute_workgroup_storage_size` is **16,384 bytes on both paths**.
Because G-E is dropped, it consumes **zero bytes** and leaves that entire device
budget to existing code and future R2/R3. Their proposed 11 KiB leaves about 5 KiB
before alignment and any other live allocations. No occupancy claim is made.

## Identity and timing verdict

- **Identity: not run / not applicable to a G-E candidate.** There is no production
  shader or Rust change. Production source files are unchanged against the baseline.
  The measurement-only probe binary is not promoted and has no byte-identity claim.
- Per the order "drop G-E" at step 1, ARA1 identity (`abplan.py` / `summarize.py`),
  RF1-RF10, Q-walk, MP1, G3 population checks and lib tests were **not run**.
  They remain mandatory if a revised implementation is proposed.
- **Quiet GI lookup and service ms before/after, RT and non-RT: unavailable.**
  No before/after candidate exists. Counter readbacks serialize these probe runs;
  their service times must not substitute for the requested quiet pair. The historical
  approximately 0.8 ms lookup is attribution from P-GI, not a new quiet baseline.
- P0 W1-W3 and C1-C5 are unaffected; there is no shading or transport change.

## Execution and evidence

The measurement source derives from main `21088f3`. Its binary SHA256 is
`40060ae502353601b86585ecf6b7f6227c5641d1534af4f9171ba2d1141913ac`.
`D:/Coding/renderlab-work/p-gie/gate-summary.json` records hashes of the four
`runs/*/probe.json` counter files. Device-limit files and `lease/runs/gate-1.json`
record admission and runtime samples. All four adapter processes exited 0; the
lease exited 0 without guard failures. It held stillkit's named mutex for that
single lease, with the 8 GiB admission and 2 GiB runtime floors. Admission had no
compiler processes. No desktop/window capture or test binary was launched.

The one build used `D:/Coding/pgie-target`, `-j 2`, the atomic build slot with
`owner.json`, and the normal stillkit mutex acquire. Both locks were released as
soon as that build ended. It waited behind P-NRT3b's anchored `l2-1` release;
the lease read and retained the latest root order. Stillkit compiled between the
build and lease, while this unit waited through the mutex. The lease was released
at 21:21:40 UTC so P-NRT3b could resume.

Implementation: no G-E implementation; about 10 minutes for the gate tooling and
report. Verification: 2m32s for the fresh measurement build and 2m27s for the GPU
lease, plus about 15 minutes in queue/mutex waits. No raw GPU dump was generated.
Cleanup of the unit's Cargo target was attempted after summarization. Automatic
approval review rejected both the verified-path and literal-path PowerShell
recursive deletions with "blocked by policy". Therefore `D:/Coding/pgie-target`
and `D:/Coding/renderlab-work/p-gie/probe-source` remain; no deletion workaround
was attempted. No raw dumps larger than 5 MB were generated. Counter JSON, logs
and source/binary identities remain on D:.
