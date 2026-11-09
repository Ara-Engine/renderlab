# P-VERIFY: Milestone 8 verification

2026-11-09. Verification-only branch `t3/p-verify`, based on origin/main
`a41d165e44c0bc8aefb8e4be565eb99d315eb177`. No renderer code changes.

## Admission and evidence

Islam's rule: "one build at a time please do not hog ram, let them finish then u go".
Every build and GPU lease requires no foreign Cargo/rustc and at least 8192 MiB
available. Builds use an atomic `D:/Coding/renderlab-work/build-slot.lock`,
`owner.json`, two jobs and `D:/Coding/pverify-target`. GPU leases are bounded
to 15 minutes, read the shared schedule line by line and log request/admission/
release. Test binaries run only within an owned GPU lease. Foreign processes
are never stopped. Owned process trees are stopped by PID.

Host: Windows/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER (12282 MiB), NVIDIA
617.14. Frozen binary SHA-256 audit (29 executables) is stored locally in
`D:/Coding/renderlab-work/p-verify/binary-identities.json`.
All new bulk evidence stays under that root; source snapshots are archives,
not edits to renderer code in this branch.

Initial requests at 17:53:16 and 17:55:10 UTC were deferred before a GPU lock
or test process was started. RAM was 13223/13287 MiB; foreign Cargo PIDs 57888
(`build --release --workspace`, started 17:20 UTC) and 58904
(`build --release --example stillkit-bar-spike`, started 17:48 UTC) remained
present, with zero rustc. `queue.jsonl` records subsequent one-second samples.
No frame-time claim can be made from these admission failures.

First admission was at 18:22:05 UTC with 13946 MiB available, no observed
compilers and no foreign renderer. The 28.62-second attempt was stopped when
foreign Cargo PID 28932 appeared. Last sampled RAM was 10625 MiB, so this was
compiler contention rather than a memory-floor failure. No capture completed;
the attempt is unscored. The PID-owned process tree was stopped and the lease
released at 18:22:31 UTC. Audit: `lease/runs/nrt3-q1-ready2.json`.

Two further attempts at 18:23:45 and 18:24:28 UTC stopped after 20.15 and
11.06 seconds on foreign compiler starts, also before a capture completed.
The runtime compiler-abort was subsequently made optional: compiler-free
admission remains mandatory, but correctness captures may finish if another
project starts afterward. Such timing/trace intervals are diagnostic only.
The 2 GiB runtime memory guard remains mandatory.

The coordinator approved joining stillkit's existing named mutex queue,
`Global\stillkit-cargo-one-at-a-time`, for one build or GPU lease at a time.
`scripts/p-verify-queue.ps1` waits for its current owner normally; it never
stops that owner or changes stillkit's scripts. It releases immediately after
the command. Admission waits at most 30 seconds once acquired, and our GPU
lease is shortened to 14 minutes (build timeout 810 seconds), keeping the
whole mutex hold below 15 minutes. The process owns the native mutex handle,
so process exit cannot leave a persistent lock file. Every acquire/release
is recorded in local `mutex.jsonl`; exact completed intervals will be listed
here. Builds also retain Renderlab's atomic build-slot lock and two-job limit.

## P-NRT3 — DON'T MERGE

| Gate | Result | Numbers |
|---|---|---|
| l1 quiet timings and drain/idle traces | Not scored; superseded by a correctness failure | Three short interrupted starts |
| l2 ARA1 | **FAIL: final binary cannot create its GI shader** | Baseline RT game-base: 88 frames, 19 HDR witnesses; candidate exit 1, zero images |
| Q-walk, RF1–RF10, MP1, lib | Remaining plan stopped after decisive shader failure | Not certified |
| Merge recommendation | **DON'T MERGE** | WGSL duplicate `gi_wg_base` declaration |

Frozen branch `9e4d474`; baseline `0979ffe`. Plan copies preserve the staged
binary paths and redirect output into this unit's evidence root. The wrapper
preserves per-step environment switches and replaces trace image-name killing
with exact executable-path/PID tree termination.

Final binary SHA-256
`3f48e24f303bef51fc8b32ed02985e7b55e5a69fc6b079347ffbb961d011c98c`
fails `Device::create_shader_module`, label `indirect cache kernels`:
`gi_wg_base` is first declared as `array<u32,GI_SETUP_CELLS>` at composed WGSL
line 1992 and redeclared as scalar `u32` at line 2684. Both game-base and
ara-pbr candidate captures exited 1. This is a renderer shader failure, not a
timing or memory failure. The remaining owned plan process tree was stopped;
the lease released at 18:53:32 UTC. No renderer fix was made by this unit.

Shared mutex acquired 18:50:55.159 UTC; the exact release is in `mutex.jsonl`.
Lease elapsed 156.75 seconds; resource audit is
`lease/runs/nrt3-l2-mutex1.json`. No speed comparison is certified.

## P-RETIME open checks — pending

| Gate | Regression verdict | Numbers |
|---|---|---|
| A, 64 unshadowed lights, RT | Unresolved | No new quiet pair |
| A, 64 unshadowed lights, non-RT | Unresolved | No new quiet pair |
| Packed BVH only, non-RT game | Unresolved | No new quiet pair |
| Packed BVH only, non-RT quarry | Unresolved | No new quiet pair |

Reuse the accepted soft-quarry evidence in `p-retime.md`; it does not close
these open checks. No regression yes/no is inferred from missing measurements.

## P-NSQ3 — pending

| Gate | Result | Numbers |
|---|---|---|
| Build screen override on `c981356` | Awaiting compiler gate | Source archive and patch prepared |
| q1 byte identity and lower cut time | Not run | No lane count retained |
| Merge recommendation | DON'T MERGE pending evidence | No measured cut benefit |

## P-FLAT2 — pending

| Gate | Result | Numbers |
|---|---|---|
| Build `68a902b` silhouette correction | Awaiting compiler gate | Source archive prepared |
| w2 flicker/crop and complete acceptance matrix | Not run | Earlier pre-correction evidence is not this build |
| Merge recommendation | DON'T MERGE pending evidence | Corrected build unverified |

## P-LTS3 C — pending

| Gate | Result | Numbers |
|---|---|---|
| Remaining 1080p, 128/129, MP1, courtyard, full quarry | Not run | Prior completed prefix remains valid |
| Quiet four-case C/main timing | Not run | No speedup or regression clearance |
| Merge recommendation | DON'T MERGE pending evidence | C patch remains experimental |

Implementation time: zero renderer implementation. Verification preparation,
compiler waiting and actual GPU/build durations are recorded separately; queue
waiting is not rendering cost.
