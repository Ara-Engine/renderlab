# P-RETIME: frozen-binary quiet timing

2026-11-09. Measurement only; no renderer changes. **Partial: the original
three-hour campaign ended at 16:58:43 UTC.** The exact soft quarry default is
cleared on both paths. Quiet unshadowed-light and packed-BVH verdicts remain
unresolved; no timing values are invented for them.

| Requested check | Verdict |
|---|---|
| A, 64 unshadowed lights, RT/non-RT | Unresolved: no qualifying quiet pair |
| A, quarry soft default, RT/non-RT | **No regression observed**, captures byte-identical |
| Packed runtime software-GI BVH, non-RT game/quarry | Unresolved: no qualifying quiet pair |

This closes the soft-default quarry portion of `M8-P-LTS2-final`. The unshadowed
checks in `M8-P-LTS2-A` / `M8-P-LTS2-final` and the `M8-P-MEM` timing check stay open.

## Frozen pairs

- A: `71f72283aee494d263c0f297603bb72d81ebf832` ->
  `6112205e3ee77ddd60fae7db24a4d01061038111`. `a5ae72a` is a documentation
  checkpoint whose parent already contains A. The actual merge's first parent is
  the clean baseline. Only three production dispatch files differ.
- Packed runtime software-GI BVH: baseline `dc0d4ea3fd990f8d89c5d5f9ff9d48f0d8aa3d77`,
  candidate the same source with only `src/gi_bvh.rs` and
  `shaders/gi_rays_software.wgsl` from `241b87bdd99f0bf980cd323a6329cce82f61f9a0`.
  This isolates BVH packing from the merge's unrelated memory changes. A full
  merged executable was also built but is not the isolated candidate.

All executables use release builds, `-j 2`, with the shared warm target at
`D:/Coding/pretime-target`. Source snapshots and frozen binaries live under
`D:/Coding/renderlab-work/p-retime`; no production file in this branch changes.

## Exact soft quarry default

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, Ryzen 7 7800X3D. These are the
existing P-LTS2 LC1/LC2/LC3 full-quarry fixture's drained frame-service timings,
not the different ARA1 daylight workload. Every light, soft local shadow, GI
geometry and the default 128-map budget is retained. The fixture has eight timed
frames per configuration, after its existing warmup. Two independent process
blocks per binary run in ABBA order; cells average the two block medians.

| Path | Still before -> A, ms | Camera moving before -> A, ms | Moving lights before -> A, ms | Regression |
|---|---:|---:|---:|---|
| RT | 13.59 -> 13.03 | 10.40 -> 9.60 | 19.23 -> 17.65 | No observed regression |
| Non-RT | 20.22 -> 19.25 | 22.95 -> 20.95 | 45.13 -> 43.64 | No observed regression |

All captured PNG and float-HDR witnesses match across the four blocks on each
path. The earlier contended 13.86 -> 14.27 / 19.87 -> 20.94 ms slowdown does not
reproduce in these quiet pairs. This is a bounded workload result, not an
engine-wide performance claim.

## Admission and audit

The waiter streams the schedule line by line, reads the latest anchored root
order before every lease, and checks the atomic GPU lock at 50 ms intervals.
RAM/compiler/schedule checks run once a second while queued, with an immediate
recheck before claiming. Admission requires >=8192 MiB available and no Cargo,
rustc or linker from any project. Every actual admission posts the bare
`p-retime retime admitted` line, followed by an explicit `QUIET active` marker
for compiler watchers. Each lease is <=15 minutes, with a 2048 MiB runtime guard.

The existing N8 guarded launcher supervises the process tree. A 10 Hz Toolhelp
audit additionally records RAM and Cargo/rustc/link/lld-link/cl/clang-cl/clippy
process names and PIDs for each binary process. A process with any observed
compiler overlap is rejected. An interrupted ABBA pair restarts in full.

The accepted RT fixture lease lasted 46.73 s; the non-RT fixture lease 223.86 s.
Neither has observed compiler overlap. Runtime RAM minima were 7957.52 MiB RT
and 6683.31 MiB non-RT; the 8 GiB threshold is an admission threshold.
Audits and rejected attempts remain under the local evidence directory's
`lease/runs` and `rejected` directories. Several attempts were correctly aborted
when Renderlab or other-project builds started after admission. Initial collector
attempts also used an obsolete `lc1-*.json` filename; LC2 writes `lc2-*.json`.
Those incomplete sequences contribute no retained result.

## Full-track RT quarry daylight

The independent ARA1 daylight workload also passes byte identity across all 76
HDR and 76 PNG witnesses per block. Eight track repeats per binary, ABBA order,
zero observed compilers. Cells are mean GPU span / drained service milliseconds.

| Phase | Before A | A |
|---|---:|---:|
| Still | 3.30 / 4.32 | 2.93 / 3.88 |
| Moving | 4.21 / 5.17 | 3.63 / 4.53 |
| First cut | 20.67 / 21.96 | 17.66 / 18.95 |
| Recovered | 6.22 / 7.14 | 5.80 / 6.70 |

The process audits contain 1086 resource samples; minimum available RAM was
6600.55 MiB. [Machine-readable evidence](p-retime-results.json) records the frozen
binary identities, timing samples, hashes and per-process audit summaries.

The CPU-only `scripts/p-retime-pnrt.py` also reuses the existing
`tools/p-nrt/abplan.py` and `summarize.py`: each saved four-track report is split
into four 88-pose reports with unchanged frame IDs and hard-linked PNGs. The
original summarizer confirms 152/152 PNG witnesses identical, eight complete
tracks per binary. Its canonical whole-phase medians are retained separately
from the settled-window means above. Matched capture/hash/PNG work occurs outside
the measured frame-service interval.

The original admission loop read but failed to enforce an order putting another
unit NEXT. Its prematurely started non-RT lease was stopped and is unscored.
The corrected parser gates on the NEXT owner's actual release after the latest
root order, with a second check at lock claim. Focused checks cover both NEXT
spellings and reject release phrases quoted by other owners. The coordinator's
explicit p-lts3 handoff was awaited in a CPU-only schedule reader before restart.
Automatic approval review blocked cleanup of the stopped token-owned lock;
subsequent coordinated leases establish that the stale lock was cleared.

## Unresolved checks after the wait limit

The ARA1 adapter runs eight complete 88-pose tracks per binary at 1920x1080,
in ABBA process order (four tracks per process). Timing uses still poses 32-39,
moving 64-71, first cut 72 and recovered 80-87. The quality spec's capture flags
are carried onto all tracks; every expected PNG and resolved-HDR hash is required.
The original scene settings and poses are unchanged.

The unshadowed 64-light A pairs, ARA1 daylight quarry non-RT A pair, and isolated
packed BVH game/quarry non-RT pairs remain open. Three of eight pairs completed.
The waiter exhausted the original 13:58:43-16:58:43 UTC allowance without another
qualifying admission. The closing queue audit recorded **5536.27 MiB available,
17 Cargo and 22 rustc processes**, with no GPU lease owner. Both admission gates
failed. The earlier 16:50 audit had 4747.82 MiB, 14 Cargo, 21 rustc and 10 clippy
drivers. The final queue audit is retained in the machine-readable evidence.

The wait ended with no owned GPU, compiler or waiter process. No frame-time
regression clearance for packed BVH, and no quiet unshadowed speed claim, follows
from the completed quarry checks. Frozen binaries remain ready for a later lease.

Renderer implementation time is zero: this unit only added measurement helpers.
Frozen build preparation took 668.51 s in total, including both A fixture binaries
and the isolated BVH executable; these shared-host build times are not a new
development-loop benchmark. The three-hour campaign includes queue waits,
rejected attempts, coordinator handoff and accepted GPU work. Exact accepted
process intervals and per-process audit counts are in the JSON evidence.

## Cleanup and the subsequent build rule

Automatic approval review rejected native deletion of the summarized large F32
dumps with **"blocked by policy"**. The local `raw-cleanup.json` lists 197 files,
4,901,990,400 bytes. It also rejected removal of the verified, explicitly named
`D:/Coding/pretime-target`. Those files and the target remain; no alternative
deletion method was attempted. JSON/PNG witnesses, logs and frozen executables
remain under `D:/Coding/renderlab-work/p-retime`.

After the campaign, Islam required one Renderlab build at a time and foreign
Cargo/rustc processes to finish before a build or GPU lease. The build helper now
takes `D:/Coding/renderlab-work/build-slot.lock` with atomic mkdir, checks the
compiler/GPU gates again after claiming it, and releases its token-owned lock
immediately after each Cargo command. GPU admission also checks the build slot.
The new guard was syntax-checked; no further build or GPU run was launched.

## Reproduction

```powershell
py -3 -B scripts/p-retime.py build
py -3 -B scripts/p-retime.py build-bvh
py -3 -B scripts/p-retime.py measure
py -3 -B scripts/p-retime-pnrt.py
py -3 -B scripts/p-retime-summary.py --export
```

The P-LTS2 fixture commands retain their existing LC1/LC2/LC3 workload; the new
adapter changes ownership, ABBA ordering and auditing. When an explicit owner
handoff is required, use `wait-owner OWNER ROOT_ORDER_UTC` instead of `measure`.
`check-gate` runs the focused scheduling reproducer without any GPU work.
For an incomplete campaign, add `--partial` to the two CPU summary commands.
