# P-RETIME: frozen-binary quiet timing

2026-11-09. Measurement only; no renderer changes. **In progress:** the exact
soft quarry fixture is complete; the full-track light/BVH comparisons are queued.

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

| Path | Still before -> A, ms | Camera moving before -> A, ms | Regression |
|---|---:|---:|---|
| RT | 13.59 -> 13.03 | 10.40 -> 9.60 | No observed regression |
| Non-RT | 20.22 -> 19.25 | 22.95 -> 20.95 | No observed regression |

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

## Remaining measurements

The ARA1 adapter will run eight complete 88-pose tracks per binary at 1920x1080,
in ABBA process order (four tracks per process). Timing uses still poses 32-39,
moving 64-71, first cut 72 and recovered 80-87. The quality spec's capture flags
are carried onto all tracks; every expected PNG and resolved-HDR hash is required.
The original scene settings and poses are unchanged.

The unshadowed 64-light A pairs, ARA1 daylight quarry A pairs, and isolated packed
BVH game/quarry non-RT pairs remain pending a compiler-quiet window. No verdict
is inferred for those pending measurements from the completed fixture pair.
