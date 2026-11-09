# P-POST: post/HDR replay invariance

Milestone 8, 2026-11-09. Branch `t3/p-post`, based on the then-current origin/main `c83dffb0971d66f264303531e2d3ad67b0bb163d`.

**Verdict for the reported neutral assertion: stale test setup, not a demonstrated post write into HDR/TR1.** VF1 enabled jittered, temporally reused volumetric fog, but PP1's replay never restarted that independent history or sample sequence. The invariant stays exact. A separate non-RT in-flight repeatability problem remains open below.

## First bad merge

The first-parent bisect identifies **VF1, `b388368fb669ea9e76637cf6a3473d7619699234`**, immediately after TP1 `34f7dc9`. This is the merge introducing the HDR comparison mismatch, with two already-landed compatibility repairs held constant in older snapshots.

| Snapshot | Test result |
|---|---|
| Current main `c83dffb`, untouched | Exact original assertion fails, 39.17 s |
| PP1 `9f98869`, untouched | Full fixture sweep passes |
| RQ1 `6608310`, untouched | Original HDR assertion fails |
| LC2/AT2/TP2 checkpoint `622db4d`, untouched | Original HDR assertion fails |
| Batch checkpoint `e5c4623`, normalized | Original HDR assertion fails |
| TP1 `34f7dc9`, normalized | Full fixture sweep passes |
| Following formatting commit `bccf730`, normalized | Original HDR assertion fails |
| VF1 `b388368`, normalized | Original HDR assertion fails; first bad merge |

The unmodified `e5c4623` and MP1 `04cc1ac` snapshots cannot reach this assertion: post declares a 112-byte minimum for the 128-byte `PrimaryFrame` introduced by RTDEF2. Those failures were recorded as **skips**, not HDR failures. The normalized builds apply only the existing TP2 `b3060c2` binding correction (112 to 128) and, where present, `ddd823c`'s removal of a duplicate ray-query declaration. Neither changes shader math. Both edited files were restored after each diagnostic build. No historical compatibility patch is part of this branch.

Evidence: `D:/Coding/renderlab-work/p-post/bisect-untouched.txt`, `bisect-normalized.txt`, `bisect-results.jsonl`, `first-bad.txt`, and each snapshot's build JSON/log and stdout log. Binary SHA-256 values are recorded with every result.

## Cause and fix

Each variant renders eight static frames, followed by eight moving frames. PP1 restarted GI, virtual shadows, TR1 and post motion between variants. Atmosphere kept `previous` and `frame`: the next variant therefore used a later R2 fog sequence and inherited visibility history from the preceding camera flight. Matching TR1 indices did not make the scene HDR inputs equivalent. VF1 changed the default from no volumetric fog to enabled fog, exposing that missing reset.

The counterexample is decisive for the reported comparison: on current main, changing the **neutral variant to disabled post as well** still fails the original assertion. No post-setting difference is required. See `control-off-off/stdout.log` (43.74 s).

`Renderer::reset_atmosphere_history()` now clears the fog reprojection source and restarts its sample index. It is a public, explicit reset, alongside `reset_indirect()` and `cut_reconstruction()`. The sweep calls it without disabling fog or changing any quality setting. It also completes each static refill frame before admitting the next, matching the moving sequence's existing completion discipline.

The gate now compares **all 13 post variants**, still and at the end of motion. It compares scene HDR bytes directly, plus SHA-256 of the reconstructed HDR output and TR1 colour/weight history. Upscaled output is checked separately because `try_capture_hdr()` returns the internal image at reduced render scales. Failure messages name the differing stage. Readbacks stay outside the eight moving-frame timing samples.

## Proof and limits

Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14; test profile, 1280x720 procedural bright-sky fixture, default fog/GI/TR1, 1x samples. The non-RT device explicitly omits ray-query features. No viewer/window or desktop capture was used.

| Retained case | Result |
|---|---|
| Native RT / non-RT | Both pass, all 13 variants |
| Quality RT / non-RT | Both pass, all 13 variants |
| Performance RT / non-RT | Both pass, all 13 variants |
| Lib gate | 198 passed, 0 failed, 164 ignored |
| Public reset API: `cargo check -p renderlab --lib -j 2` | Pass, 32.34 s |

Every retained case has one hash per stage across all variants, separately for still and moving frames, and non-null TR1 history hashes. Vulkan error lines: zero. `final-summary.json` contains the stage counts and resource guards; `drained-*/bright-sky/timing.json` contains the individual hashes. Retained binary SHA-256: `32bb6def0ac49c926f5b913eaa0c124f0a62585ad3d4b4f32f3ea98a386e85ad`.

The GPU run used the identical reset body under its original test-helper name. The root coordinator then requested promotion to the public method; only its name, visibility and cfg changed. A normal-library CPU build check validates that promotion; no additional GPU lease was requested.

All six contact sheets in `D:/Coding/renderlab-work/p-post/inspect/` were inspected: silhouettes, the small bright emitter, bloom, motion, near/far DOF and display grading remain visible as expected. No shader, post allocation, TR1 implementation, ARA1 harness or RF fixture source changed. No new ARA1/RF image campaign or full-quarry run was performed; this is an invariant/test correction and an opt-in API addition, not a rendering-quality or performance claim.

### Open: non-RT in-flight repeatability

After the fog reset alone, native RT passes all variants, but non-RT with eight unfenced static refill frames can fail at motion or DOF. The failure moved between those variants in two runs; scene HDR, reconstructed HDR and TR1 history all differed. Keeping every variant's post disabled passes all 13. Adding a drain after each static refill draw passes the full six-case gate above.

This establishes sensitivity to in-flight workload/completion. It does **not** identify the responsible resource or establish that the renderer is race-free. The comparison now uses matched completed refill steps; a possible production cross-frame bug is explicitly retained in `m7-gaps.md`. No renderer synchronization change or production race fix is claimed.

### Camera cuts: code audit

**A normal cut does not explicitly reset fog history today.** `cut_reconstruction()` only sets TR1's cut flag (`src/primary_ray_gpu.rs:404`). `Engine::cut()` calls it and `mark_camera_cut()` (`src/engine/mod.rs:1015`); the latter only marks texture residency for verification (`src/texture_residency.rs:326`). Neither calls the new atmosphere reset.

`AtmosphereGpu::HistoryKey` (`src/atmosphere_gpu.rs:55`) has no camera/cut key. `prepare_atmosphere()` (`:568`) invalidates for volume/settings, geometry uploads and lighting/environment/shadow changes, but not camera displacement. `atmosphere_submitted()` (`:1098`) retains the previous camera, and fog reprojects its history. A teleport can reject out-of-volume samples, or trigger another independent invalidator, but there is no explicit teleport/cut reset. **Fog history survives camera cuts; possible ghosting after cuts.** Ghosting was not visually measured here. Call `reset_atmosphere_history()` explicitly until the engine cut path is integrated.

## Time and resource accounting

Implementation is limited to the reset API and GPU-test setup/assertions; no rendering algorithm was changed. Active source/document edits took roughly 15 minutes (estimate); most remaining elapsed unit time was compilation, queueing and diagnosis. Warm snapshot builds took approximately 60-127 s each with two Cargo jobs. Owned verification leases total **1,086.05 s (18.10 min)** across four separate leases; the longest was 387.52 s, all below 15 minutes. These totals include startup, readbacks, encoding, historical builds in the first lease and expected failing probes, not frame-time measurements. The retained six-case plus lib run took 253.44 s.

Every lease used `scripts/n8-run.py`, 7 GiB admission and the unchanged 2 GiB runtime guard. Lowest observed available RAM was 5,334.85 MiB. Compiler-quiet holds were honored; the guarded rebuild stopped on P-LTS2 admission and resumed after release. The owned target is `D:/Coding/ppost-target`, outside renderlab-work paths. Evidence is ignored on D:, with no raw HDR dumps.

Cleanup blocker: automatic approval review rejected both the verified cleanup batch and a standalone `Remove-Item -LiteralPath 'D:/Coding/ppost-target' -Recurse -Force`, stating only **"blocked by policy"**. No alternate deletion mechanism was attempted. The target and temporary executables in `D:/Coding/renderlab-work/p-post/bin` therefore remain for root/user cleanup; summaries, logs and inspected PNGs are retained intentionally.
