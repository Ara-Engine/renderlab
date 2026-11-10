# P-RTS2: earlier RT penumbra-listing variants

2026-11-10. Branch `t3/p-rts2`, starting main `69c667b`.

**DON'T MERGE (parked for the v0.1.1 release freeze).** No renderer default is
changed. The final wide-probe design remains rejected for visible S2 fringe loss.
Earlier variants require their own visual decision; their old numeric misses
alone do not decide that question under Islam's clarified M8 bar.

## Scope and preparation

Read AGENTS.md, milestone-8.md, m8-retriage.md rank 1 and its ledger, p-rts.md,
and the `penumbra-listing-limits` memory. Historical implementation ownership was
traced through `0d27dc4`, `892e8e2`, `35fc132`, and `db326de`.

The prepared screen materializes `db326de` outside the checkout, under
`D:/Coding/renderlab-work/p-rts2/source`. It changes only that snapshot's
`src/temporal_reconstruction.rs` and `src/primary_ray_gpu.rs` to choose the earlier
probe count, disk/cone split and list radius per process. It keeps the full-budget
estimator. No P-S4B or P-GATHER owned file is edited. This is historical screening,
not a port to main or proof of equivalence to current main.

The six candidates are 8 in-disk probes with 5x5, 7x7 and 9x9 windows; 16 in-disk
probes with 5x5 and 7x7 windows; and 8 probes on a 1.5x cone with a 5x5 window.
The first rejection screen is the complete 160-frame S2 `camera-1px-r0.04` track
at native 1920x1080, with its registered 512x288 crop, `TR1-Auto32`, including
frame zero. It uses registered direct lighting, no GI or atmosphere, and no timing
repetitions. The baseline generates the registered 256-primary x 256-sun-ray film
reference. Other S2 tracks and current-main verification remain required for a
candidate that survives this first track.

Host inventory: Windows, Ryzen 7 7800X3D, RTX 4070 SUPER (12282 MiB), NVIDIA
617.14. Inventory was read without GPU rendering. New bulk data and targets are
confined to `D:/Coding/renderlab-work/p-rts2`.

## Results

**S2 visual screen: not run.** No crop sheets, frame-by-frame inspection, numeric
scores, current-main comparison, or reference comparison were produced. None of
the six earlier variants has a new acceptance or rejection on visual grounds.
No performance measurement ran.

The first admission waiter was stopped before it built anything to strengthen
the queue gate: a P-GATHER build release must not be mistaken for its GPU release.
The second waiter admitted one build at **01:32:24 UTC**, after P-GATHER's first
GPU lease released at 01:32:18. Admission recorded 12801 MiB available, no
compilers, over 5 GB free on D:, and the streamed 01:17:21 root order. The atomic
build slot owner was `p-rts2` (PID 53500), held together with the normal
`Global\stillkit-cargo-one-at-a-time` mutex for this one command:

```text
cargo test --release --locked -j 2 --lib --no-run --message-format=json
```

P-GATHER then requested a quiet retry. To yield to the preceding unit and keep
the release window clear, P-RTS2 stopped its own verified rustup/Cargo process tree
at **01:34:26 UTC**, after **122.18 seconds** of compilation. The wrapper released
its own slot and mutex. Exit 1 records this intentional stop, not a diagnosed
compiler failure. No screen executable was completed, no tests were run, and
P-RTS2 took no GPU lease. The foreign Cargo PID 17024 reported as interrupting
P-GATHER was not this build: our Cargo PID was 60356, started after that release.

Local build/admission evidence: `D:/Coding/renderlab-work/p-rts2/build-identity.json`
and `screen-build.log`. Build input is historical `db326de`, with the selector
patch reproduced by `tools/p-rts2/prepare.py`; checkout main is
`69c667b952c068661b9a8f38e8a359ca4e386fd0`. There is no completed binary hash.
Preparation utilities pass Python syntax compilation only; their Rust changes,
GPU replay and scorer are unvalidated. Renderer implementation time: none;
compilation: 122.18 seconds; image verification: not run.

**Cleanup handoff:** automatic approval review rejected the native PowerShell
removal of the verified owned target `D:/Coding/renderlab-work/p-rts2/target`,
stating "blocked by policy". The target remains (567225388 bytes, 1229 files);
no alternate deletion method was
attempted. There are no GPU raw dumps because no rendering ran. No owned build,
GPU job, waiter, slot or mutex remains active.

## Stopped at / Next step

Root's release steer supersedes the original port/timing plan: finish or park by
02:00 UTC, with no build or lease that cannot finish by 01:55. P-S4B and P-GATHER
precede this unit. The build admission cutoff is 01:35 to reserve screen and
cleanup time. No port, lib acceptance run, ARA1 ABBA, or game/quarry cut inspection
will be attempted in this release window.

Stopped before completing the historical screen binary. No port was attempted.
Current main's modern RT retry pipeline disables the historical penumbra pass,
so merely enabling its old flag would not establish a working port.

Resume with the S2 visual screen, updating the utilities' release-window deadlines
and waiting for the preceding unit's complete handoff, including retries. The
prepared admission tool now requires an explicit unit-complete/handoff line,
rather than an individual GPU lease release. If a historical candidate survives, compare it
against current main and the film reference on all three registered S2 tracks,
still, moving and cold/cut. Inspect unamplified final-image crops frame by frame;
visibility deltas and RMS are supporting evidence. Only then port, run lib tests,
take one quiet game/quarry ABBA across still/moving/cut/recovered, and inspect the
game/quarry cut frames. Keep a faster variant only if it is not visibly worse.

The tools in `tools/p-rts2/` are screen preparation, admission, lease, replay and
CPU scoring utilities, not a production feature or acceptance result.
