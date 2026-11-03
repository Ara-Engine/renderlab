# Quarry responsiveness unit

This unit reviewed the uncommitted selector patch from the preparation unit, then made one measured attempt at moving-camera CPU cost on the real quarry scan. It does not finish real-scan acceptance. Materials, lighting, AA acceptance, resize, presentation latency and residency recovery after cuts remain open.

## Review of the preparation patch

The publication early skip, its capacity proof, profile-only bootstrap spans, `--dump-groups` and the native game guard are accepted for checkpoint. The proof holds because a publication stays inside the closed admitted plan. A seed shard that is unavailable and unreplaced by the plan therefore always enters the published cut. The skip applies only when neither output nor geometry capacity can deny any transaction for that mesh, so no debt reason is lost.

Mutation checks found one coverage gap. Removing only the output-cluster half of the capacity guard failed no test. A doomed publication can need more than its accepted plan charged: an unpublished ancestor's clusters, replaced in the plan by a planned sibling, reenter the publication cut. `doomed_publication_still_reports_capacity_beyond_the_plan` now pins both guard halves against the independent policy reference. Held and moving image equality, binary identity and the measurement attribution were rechecked. Details are in the local `.refs/quarry-responsiveness/review.md`.

## Paced continuous path

`scripts/scan-responsive-run.py path` derives `measurements/quarry-responsiveness/paced-cameras.json` from the unchanged canonical camera file (SHA-256 `ab342818…`). It has 338 poses. Every canonical pose appears exactly at a segment end. Intermediate poses interpolate eye and target only.

| Segment | Canonical poses | Frames | Motion |
| --- | --- | --- | --- |
| settle_wide | 0 | 1 | first settled view |
| dolly_wide | 0 to 3 | 24 | linear, 4.4 mm per frame |
| fly_close | 3 to 4 | 90 | smoothstep, peak about 9 cm per frame |
| dolly_close | 4 to 7 | 24 | linear |
| fly_grazing | 7 to 8 | 90 | smoothstep, about 2 cm per frame |
| dolly_grazing | 8 to 11 | 24 | linear |
| cut_reverse | 11 to 12 | 1 | the canonical abrupt cut |
| dolly_reverse | 12 to 15 | 24 | linear |
| hold_reverse | 15 | 60 | camera stops |

Motion statistics use the 277 frames from `dolly_wide` through `dolly_reverse`. The hold is reported separately. The replay paces at 60 Hz with `--frame-period-ms 16.666667`. Pacing sleep is recorded separately and never counted as frame cost. The metric is `advance_draw_wall_us`: one streaming tick, selection, draw encoding and queue submission on the CPU. It is offscreen. It is not acquisition, presentation or input-to-photon latency. The original sixteen-pose path stays unchanged and is still used for image identity.

Preparation is paced too, so first settlement in these runs includes pacing sleep. About 896 ticks at two uploads per tick take at least 15 seconds at 60 Hz. That startup figure is a property of the unchanged upload policy, not of this change.

## Measured limiter before the attempt

Each of about 21,400 cut clusters was its own draw. Before each draw the pass rebound two bind groups, two vertex buffers and the index buffer, about 150,000 pass commands per frame. On the paced path, encoding plus submission was 54% of the dev motion frame and 35% of the optimized one. The selector was the other large share.

## Mechanism

`render_virtual_cover` now extends the previous draw when the next cut cluster uses the same resident page, material and placement slot, and its index span starts where the previous span ended. The cooker packs each group's clusters contiguously in cluster order inside a page, so such runs are common. `submit_prepared` sets bind groups, vertex buffers and the index buffer only when they change, comparing object identity. Draw order and the primitive sequence are unchanged, so rasterization is the same. No budget, camera, threshold, sample count, upload rate or quality setting changed.

## Results

Hardware: Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Windows, Rust 1.96.0, two Cargo jobs. Content: the twelve-scan quarry package `3deb5931…`, 1920x1080, 4x MSAA, 512 MiB geometry, metadata and texture budgets. Runs alternate before and after. Each run follows an explicit warm read of 1,349,789,324 package and page bytes. Timed runs have no capture, cover proof, dump or profiling. Every run passed its audit with zero errors and settled its first view.

| Configuration | Motion p95, before | Motion p95, after | Motion p50, before | Motion p50, after |
| --- | --- | --- | --- | --- |
| Dev, active Khronos validation, 3 pairs | 444.2 ms (418.1–484.6) | 259.4 ms (251.8–264.8) | 373.6 ms | 200.7 ms (197.8–207.3) |
| Optimized, no validation, 5 pairs | 21.66 ms (20.84–31.04) | 15.22 ms (14.71–26.61) | 19.16 ms (18.87–22.55) | 13.55 ms (13.47–18.51) |

Values are the median of per-run statistics, with the run range. Dev p95 fell 41.6% with disjoint ranges. Optimized p50 fell 29.3% with disjoint ranges. Optimized p95 fell 29.7% at the median. One optimized pair reversed while host CPU load from other processes reached 43–72%. No user application was closed.

The work counts move as predicted before the edit. Draws per frame fell from 21,410 to 3,056.5 at the median while cut clusters stayed at 21,410. Dev encode fell from 183.5 to 27.3 ms and dev submit from 21.6 to 3.8 ms. Optimized encode fell from 6.77 to 0.95 ms. Selection, projection and upload admission did not change. On the deterministic part of the path, cuts, triangles and uploads match frame for frame across sides. The held 60-frame stop fell from 290.8 to 120.9 ms in dev and from 11.27 to 5.34 ms optimized. Peak working set was unchanged at about 735 MB dev and 713 MB optimized.

The dev campaign was shortened at Islam's request after three separating pairs. Dev pairs four and five and the planned dev no-validation control were not run. The validation layer's share of the remaining dev encode is therefore unmeasured.

## Remaining limiter

After the change, a dev motion frame is about 198 ms. The selector takes about 110 ms of it, over 20 calls per frame. About 47 ms more falls inside admission but outside the selector, projection and upload timers. Encode takes 27 ms, submit 4 ms and projection 3.5 ms. An optimized frame is about 13.6 ms, of which the selector takes 10.4 ms (76%) and encode 0.9 ms.

The 16 ms dev and 8 ms optimized targets are not met. The selector alone exceeds the optimized target. The preparation unit's profile located most selector time in per-call setup, emission and sorting rather than proposals. Those passes run over every cluster of each mesh and over package-wide page arrays sized 16,339 for each of the twelve meshes. Candidate next mechanisms, none attempted here:

- Restrict the selector's page arrays to each mesh's own page range, charging other meshes' protected pages as a count. The cooker assigns page IDs per mesh, so this removes most page-array work. Inference from code; not measured.
- Compile the stable asset selector and wgpu with optimization in the dev profile, keeping the main crate unoptimized for fast edits. The dev-to-optimized selector ratio is about 10x. Rust edit latency and clean-build cost would need measuring.
- GPU-driven cluster submission (one geometry pool, indirect draws, GPU visibility). This is the architectural route to sub-millisecond submission for much larger scenes. It is a design proposal, not a measured result.

## Image and temporal verification

Separate optimized runs with active validation and cover proofs captured held poses 0, 4, 8, 12 and 15 after settling with zero unresolved groups. They also captured all sixteen canonical moving poses. All 21 images are byte-identical before and after, and identical to the preparation unit's fixed captures. Every one of these runs reported zero validation errors.

Paced capture runs also passed all 338 cover proofs per run with clean validation. Their later images differ before and after because the cuts differ. Explicit readbacks hold flight slots and reject uploads (743 busy uploads before, 215 after), so captured paced runs do not reproduce timed residency. They are temporal evidence, not identity evidence.

Inspected at full resolution: the abrupt cut frame, the end of the hold, settled pose 15, and a contact sheet across the path. The cut shows a complete cover with intact silhouettes and softer detail. Sixty frames after the cut, about 1,250 groups remain unresolved at about 3.0 px maximum denied error in timed runs. The hold-end capture differs from settled pose 15 by RGB MAE 0.33 of 255, on 25% of pixels. Recovery after cuts is bounded by the unchanged two uploads per tick, about 120 pages per second at 60 Hz. It remains open.

## Reproduction

From the repository root, with fresh labels:

```powershell
py scripts/scan-responsive-run.py path
py scripts/scan-responsive-run.py build FRESH-DEV
py scripts/scan-responsive-run.py build FRESH-RELEASE --release
py scripts/scan-responsive-run.py campaign FRESH-TAG BEFORE-BINARY AFTER-BINARY --validate --pairs 3
py scripts/scan-responsive-run.py summary FRESH-TAG
py scripts/scan-responsive-run.py run FRESH-HELD AFTER-BINARY --validate --canonical --start 15 --frames 1 --hold-pose --capture 15 --verify-cover
```

`path` refuses a changed canonical camera file. `build` preserves the executable, its compiled source hashes and an exact source archive. `run` refuses existing labels and records content hashes, including the paced file. The local handoff `.refs/quarry-responsiveness/handoff.md` lists every run, binary and source identity.
