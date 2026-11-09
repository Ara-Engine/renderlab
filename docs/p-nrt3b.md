# P-NRT3b: two submits and batched GI drain

Status: **NOT VERIFIED; do not merge yet**. Follow-up to [P-NRT3](p-nrt3.md), M8's byte-identity contract. Implementation started 2026-11-09 20:44 UTC, three-hour time box.

## Source and fix

Created `t3/p-nrt3b` from `t3/p-nrt3` (`9e4d474`), merged current `origin/main` `9fd0a9efe5229b393d92bef2a578fc65d38b4e85` without conflicts (merge `9a4c3e2`). Both scheduling and gather changes remain pending measurements. Renamed the gather scalar `gi_wg_base` to `gi_drain_base`, preserving the setup array's name and every expression. No quality, ray, light or queue budget changed.

CPU Naga validation: 19 validation tests and all 7 GI tests pass. The composed GI kernel modules validate for hardware/software ray sources at both 64 and 128 directions. The GI shading/program composition tests also pass. No additional duplicate declarations were reported.

## Reproduction and resource discipline

Evidence, plans, tools and newly built binaries: `D:/Coding/renderlab-work/p-nrt3b/`. Base is freshly built current main, not P-NRT3's old base. One Cargo target, `D:/Coding/pnrt3b-target`, reused serially, `--release -j 2`. Every build holds atomic `build-slot.lock` with `owner.json` and `Global\\stillkit-cargo-one-at-a-time`. The mutex is acquired in the normal wait by a process whose exit releases it. Locks are released after each build. Every lease holds the same named mutex, checks no cargo/rustc, >=8 GiB available and >=5 GB free on D:. GPU execution uses `scripts/n8-run.py` through the existing P-NRT lease wrapper, <=14 minutes, 2 GiB runtime floor. Root orders are re-read before each lease; alternating owner gates match anchored owner lines.

The local trace runner replaces the inherited image-name termination with PID-tree termination of newly spawned matching executable paths. Test executables run only within our leases. No dev server or viewer is launched.

| Binary | SHA256 prefix |
|---|---|
| base.exe | cdbc726546b7b43a |
| base-lib.exe | a62dd4edf0629d0e |
| final.exe | 3d5136609bbffcfe |
| final-lib.exe | a386a0a6ca017908 |

Initial candidate capture build: 153 s; candidate lib build: 101 s. Base capture including mutex queue: 196 s; base lib: 90 s. These are implementation/verification latency, not frame performance.

## Identity evidence so far

- ARA1 RT: 82/82 captured PNGs byte-identical.
- ARA1 non-RT: 82/82 captured PNGs byte-identical.
- Q-walk RT: 93/93 recorded frame hashes identical.
- Q-walk non-RT: 93/93 recorded frame hashes identical.
- RF1-RF10: both baseline/candidate tests pass on both paths; all 13 saved float buffers per path are byte-identical (26/26 total).
- MP1 non-RT: both tests pass; 7/7 saved PNGs byte-identical.
- MP1 RT and full lib suite pending.

Leases `l2-1` and `l2-2b` each reached their bounded deadlines at 750 s. The second stopped in baseline MP1 RT. Its completed captures/tests remain valid; interrupted steps must resume. Root orders alternated P-NRT3b with P-GIE, then P-R2. The unadmitted second waiter was withdrawn, and follow-up waits for P-GIE's anchored release.

## Timing and latency limits

First ARA1 timing observations are screening evidence only. The combined candidate improves game service time on both paths and non-RT quarry, but RT quarry's early still median regresses (7.21 to 8.93 ms). Separate-change quiet measurements and GPU idle-gap/drain traces are pending. Neither change is promoted on this evidence.

Two submits preserve queue and frame/presentation order. ARA1 is offscreen: its service time is not input-to-photon latency or measured scanout. Additional CPU submit overhead, internal GPU gaps and any end-to-end regression must be reported alongside an earlier GPU start. No latency win is established yet.
