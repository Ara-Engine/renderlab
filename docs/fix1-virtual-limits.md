# Fix1: Virtual device limits and still-frame reuse

2026-11-08, branch `t3/fix1-virtual-limits`, based on `bcc6eb5`.
Addresses [TR1's two owner defects](m7-tr1.md#7-defects-found-outside-tr1).

`visibility_limits` now requests 11 storage buffers per shader stage, including G1's
radiance-cache binding in Virtual's lighting group. `traced_limits` inherits the same
minimum, fixing Virtual on RT devices too. Larger caller limits are preserved. The
existing capability check keeps devices below this optional minimum on raster primaries;
callers must check requested limits against their adapter before requesting the device.

Virtual's reuse key now compares the full uploaded view-projection's float bits on the
CPU, including jitter and depth planes. Identical matrices keep reuse; changed projection
samples refresh their receivers. The renderer no longer clears rolling state merely
because reconstruction is enabled. No shader, shadow algorithm or quality setting changes.

The focused ignored GPU test `gpu_non_rt_requested_limit_pipelines` requests the production
limits, creates the non-RT engine pipelines at 1x/4x and every Virtual lookup (Exact, Map,
Hybrid, Surface), and runs reconstruction. It also renders raster fallback on default and
10-buffer devices, checks explicit rejection of visibility primaries below the minimum,
and checks reuse across identical, jittered and depth-plane-changed cameras. The normal
library suite checks the public limit helpers and preservation of higher caller limits.

Verification evidence is recorded under `E:/renderlab-work/fix1-virtual-limits/`.

- RTX 4070 SUPER, NVIDIA 617.14, Vulkan: the adapter reports
  `max_storage_buffers_per_shader_stage = 524288`; the test device requests exactly 11.
  Raster fallback was verified with devices capped at the default 8 and at 10.
- Focused GPU pipeline/reuse test: pass, including all four Virtual lookups and non-RT
  reconstruction, under wgpu's requested-limit validation.
- `cargo test -p renderlab --lib --release -j2 --target-dir
  E:/renderlab-work/fix1-virtual-limits/target -- --test-threads=2`: exit 0;
  **139 passed, 0 failed, 130 ignored**. `cargo fmt --all --check` and `git diff --check` pass.
- `scripts/n8p-default-check.py`: **all 24 recorded hashes match**; Surface/default,
  explicit Exact and every caster/determinism gate pass on unjittered views.
- TR1: all 93 captured Q-walk frames replay identically on RT and non-RT, both within
  and across processes. Non-RT on an RT device equals the no-RT device on every frame,
  for reconstructed and unreconstructed output. RT output and unreconstructed non-RT
  output also match the recorded TR1 baseline.

Reconstructed non-RT output changes on 57 of the 93 captured frames, starting at frame
65 (the hold). The full projection key now also reruns receiver clipmap level/page
requests on jittered holds; TR1's old rolling-only workaround skipped those requests.
This baseline difference is recorded separately from the passing replay gates.
Verified: the new frame 128 capture matches its deterministic-run HDR hash, and its
inspected L0/L4 display images are pixel-identical to TR1's retained 960x540 PNGs.
The sheet is `E:/renderlab-work/fix1-virtual-limits/inspect/f128-inspection.png`;
this bounded inspection does not establish a new reconstruction quality gate.

**Still cost:** exclusive announced `cost-quiet` lease, 06:48:17-06:50:50Z, including
121.17 compiler-free idle seconds before the runs, wallpaper paused/restored, no game,
foreign renderer or compiler. Two quarry poses, 1920x1080, 1x, N1c LOD 0.35 px,
software extent 32, receiver lift 1, default Virtual Surface, 32 warm frames and eight
measured frames per pose: **median 0.000064 ms**, range **0.000032-0.000096 ms**
(16 samples). These are effectively timestamp-only shadow spans. Residual desktop
activity remained: the timing-process preflight reported 37% GPU utilization at an
idle 315 MHz / 16.84 W. This confirms the still fast path, not a comparative frame-time
claim. Evidence: `still-cost-summary.json`, `runs/cost-quiet/`, `still-cost/timing.json`.

An additional run of the older `gpu_visibility_fixture` hits its independent clipmap
buffer-size guard with the default 16-page map and 128 MiB storage binding limit.
The new focused fixture uses four pages and passes; no map settings or buffer-size policy
were changed to conceal that older fixture's failure.

Implementation edits took about four minutes, including the test-fixture correction.
Verification compilation took 5m15s clean plus 1m35s incremental, two Cargo jobs;
five owned leases totalled 361.59 s, including the 121 s quiet idle and an earlier
42 s quiet-start rejection before any test. Scheduling waits and launcher adjustments
are separate from those timings. Release test binary SHA-256:
`db5a94af9dac5cd220a79b3d9073d1e75337fe4d0c62640cbc1fecd5042f1584`.
The worker target and the inspected raw HDR were deleted; JSON/logs and PNGs remain.
