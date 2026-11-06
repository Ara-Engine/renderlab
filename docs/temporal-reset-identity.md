# S3 reset identity: replay light state

Investigation based on `a011218`, Windows/Vulkan, RTX 4070 SUPER, NVIDIA
617.14, wgpu/naga 30. Development/test profile, two Cargo jobs. This is a
correctness investigation; no performance or quality superiority is claimed.

## First wrong decision

S3 began independent mode replays with the previous replay's transported light
bases. Matching the camera, sun and object transforms did not match all inputs.
`ShadowState::plan` transports the last admitted light orientation;
`fresh_estimate` takes its disk axes from the resulting shadow-view matrix.
Running the sun trajectory repeatedly changes that orientation. Consequently,
Interactive and Temporal were sampling slightly different disk directions.

The reset branch itself already runs the same production Interactive and Soft
pipeline objects. History blending, coverage and module identity are not the
cause of this reproduced mismatch. The earlier static T1a test did not move the
light and therefore did not expose the replay setup error.

## Reproduction and witness

The focused test uploads S3 once and runs eight warm trajectories per mode,
then compares contiguous native-resolution replays on the registered crop.
An unwarmed 34-frame screen passed; the warmed screen reproduced differences
on frames 0, 32 and 33. Hit records and tile flags matched; counts and HDR did
not. Host camera/light/lighting bytes were equal.

A temporary GPU probe captured pixel **(805,513)** in both failing frames.
The receiver, geometric normal, offset origin and sun were bit-identical.
The first difference was the supplied light-view matrix, before disk sampling:

```text
                    Interactive             Temporal replay
view column 0 x     0.07616967707872391       0.07616581022739410
view column 0 y    -0.01382597628980875      -0.01384724676609039
ray 2 direction x   0.5423972010612488        0.5423997640609741
ray 2 direction y   0.2506746053695679        0.2506703734397888
ray 2 direction z   0.8018524646759033        0.8018520474433899
ray 2 blocked       false                    true
visible count      192                      160
```

The instrumented frame has six differing crop pixels, maximum RGB difference
0.0227661 and crop RGB RMS difference 0.00017744. This is a bounded reproduction,
not a replacement for the original 257/258 failure record. The probe still
reproduced the error, and its entire source delta is saved outside Git. All
temporary shader instrumentation was removed.

Local evidence is at `E:/renderlab-work/temporal-reset/`:

- `witness/pixel-proof.json`: both input sets, raw GPU f32 words, all eight
  directions and blocked decisions, and the differing pixel bytes.
- `witness/*-interactive.rgba16f`, `*-temporal.rgba16f`: the failing crop pairs.
- `witness/reproduced-difference.png`: inspected pair and amplified difference.
- `probe.patch`, `runs/repro-warm.log`, `runs/probe-lf.log`: instrumentation and
  failing test exits. The uninstrumented warm replay fails too.

## Fix and regression boundary

Every independent motion benchmark replay now starts with empty transported
light bases: timing repeats, captured mode tracks and the reference track.
Transport continues normally within a track, including through Temporal resets.
The helper is test-only; production light transport, shaders, pipeline objects,
history and programmable shading contracts are unchanged.

Captured frames record camera, light, lighting and shadow-view identity.
Before comparing a Temporal reset with Interactive, the benchmark asserts that
these inputs match. The focused `gpu_motion_reset_identity` regression tests
that same boundary after warm moving-light trajectories; it checks the reset
count, crop HDR/hits/counts/tiles and optional complete-image HDR SHA-256.
`seed_replays: false` deliberately reproduces the old setup for diagnosis.

The historical S2/S3 quality/timing tables are not rescored by this fix. Their
original captures retain the earlier replay initialization; in particular, the
reported S3 RGB differences were not evidence of a broken reset dispatch.

## Verification

Run the single final batch under the shared lease and schedule:

```powershell
py -3 -B scripts/temporal-reset-run.py --label final-batch --minutes 15 --fast-check -- py -3 -B scripts/temporal-reset-check.py
```

The runner requires 4 GiB available memory to start, aborts below 2 GiB, rejects
games/foreign GPU jobs and launches hidden offscreen children. It records
preflight, resource samples, checked exits, source/binary identities and lease
release on E:. The batch uses its own `E:/renderlab-work/temporal-reset/target`
and `-j 2`. It expects two local manifests:

- `final-s3.json`: the existing S3 scene manifest, a fresh `output`,
  `full_hdr: true`, `expected_resets: 258`, no truncated `frames` field.
- `static-t1a.json`: the original T1a quarry manifest, three poses, 128 frames,
  eight repeats and the original checkpoints/cut.

`checks.json`, `final-s3/reset-identity.json`, `static-t1a/temporal.json` and
`runs/final-batch.json` are the canonical check records. Results are recorded
in `verification-summary.json`:

- S3: **258/258** reset frames have identical inputs, complete 1920x1080 HDR
  hashes and crop HDR/hit/count/tile bytes. Zero differing crop pixels.
- Original static T1a: **9/9** HDR and hit/count identities across three poses
  and full/sun/visibility programs. All eight 128-frame repeats match, including
  history. Tile-limited and fully materialized histories produce **128/128**
  identical images, with zero differing retained-history pixels.
- `cargo test -p renderlab --lib -j 2`: checked exit **0**, **99 passed,
  97 ignored**. Affected library/test Clippy with warnings denied, formatting,
  diff whitespace and Python syntax checks pass.
- Static T1a validation: active layer, **zero errors**. S3's validation caveat
  below remains explicit.
- Guarded batch: **537.10 s**. S3 test **230.38 s**, original static T1a
  **227.93 s**; these include verification readbacks, not renderer frame times.
  The checked library subprocess, including its incremental compile, takes
  36.74 s; Clippy takes 35.30 s. Implementation/diagnosis was interleaved with
  reproduction and was not independently stopwatch-instrumented.
- No game or foreign GPU process at launch. Available memory starts at
  **6,861 MiB**, sampled minimum **3,633.64 MiB**; no guard failures.
  The final reservation is released at **06:23:06.794315 UTC**.

`cleanup.json` records deletion of **2,251,175,680** raw bytes and the entire
owned Cargo target (**1,271,251,339** bytes). Raw failing pixel witnesses remain;
passing raw buffers are compacted to hashes. Inspected PNGs retain the corrected
S3 crops and the three original quarry poses at T1a frames 1 and 128.

The S3 harness retains its existing validation-off renderer setup and the
previously documented `VUID-StandaloneSpirv-None-10684` startup warnings;
they are not claimed as a clean validation run. The original static T1a
harness uses its validation-enabled manifest.
