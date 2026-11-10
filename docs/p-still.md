# P-STILL: cost that follows change

Unit P-STILL, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10, branch `t3/p-still` from main `9e343a0`. This is a design unit with a counted-work gate: count what TR1's per-frame jitter makes the world-space caches redo, and build only if the count shows a clear win. Labels: **M** measured here, **C** read from code, **A** another unit's measurement, **I** inference.

## Result

**NO-GO for now. Nothing to merge, and the count gate was not run.** No GPU lease ran in the time box. The build slot and GPU were held by Ara work and p-show2 from 12:31 to 15:14 UTC. My first admitted lease (`pstill-c1`) was stopped by this unit's own build waiter, which was admitted at the same instant (no data). The requeued lease (`pstill-c2`) died inside its admission check with `WinError 1455` (paging file too small) during the C: emergency. Per the root steer it was not retried.

What exists:
- the diagnosis from code (section 1);
- the design (section 3);
- the counting harness and candidate knobs, built and ready (section 4). The count needs one 15-minute lease with six runs.
- prior units' measurements that bound the win (section 2).

| Quarry, 1080p (P-YARD2, A) | Renderlab ms | vs Unreal Epic | After this design (I, unmeasured) |
|---|---:|---|---|
| RT still | 3.82 | 1.18x faster (4.50) | about 3.0-3.4 ms if the GI passes idle: 1.3-1.5x faster |
| RT moving | 4.26 | 1.07x faster (4.57) | unchanged by design |
| non-RT still | 8.65 | 2.15x slower (4.02) | about 7.6-8.2 ms if GI idles; page-work share unknown |
| non-RT moving | 10.57 | 2.57x slower (4.12) | unchanged by design |
| Flicker (moving frames) | 4.39 RT / 4.44 non-RT | Unreal 3.66-3.79 | **expected unchanged** (section 5) |

The flicker before/after was not measured. No change was built to measure.

## 1. Where "still" is decided today (C)

TR1 jitters every reconstructed frame by one sub-pixel offset of a 256-point lattice (`temporal_reconstruction::sample_offset`), carried by the camera projection. P-NSQ rotates the non-RT sun directions with the same frame index: `rotation(k) = 8 (k mod 32)`.

| Decision | Keyed on | On a still camera under TR1 |
|---|---|---|
| TR1's still test and shadow budget (`prepare_reconstruction`: `sampling_still`, `held_rays`) | the **unjittered** matrix, compared as bits on the CPU | Correct already. RT sun rays settle to 8 a pixel after 16 frames. P-YARD2's RT receiver pass is 0.50 ms still and 1.42 moving (M, re-read from its `engine.json`). |
| GI cache idle (`gi_cache.rs` `idle_key`, `view_key`) | the **jittered** matrix | Never idle. The key changes every frame. |
| GI idle bound with the default gather cap | `IDLE_AFTER_BOUNCE + 64 (gathers + 1)` = 272 identical frames | Even an unjittered still view stays busy 272 frames, longer than P-YARD's 160-frame still segment and ARA1's 39. |
| Virtual-shadow page work (`virtual_shadow_gpu.rs` key) | the **jittered** matrix plus P-NSQ's rotation | Request, compaction and raster run every frame. |
| Surface-cache "still" (`vsc` flag 2: walking pixels keep last frame's count) | page work off | Never still. **This is correct and should stay that way.** A walking pixel that kept its count would freeze its 8-direction sample instead of rotating it. TR1 would then converge to that one sample: frozen noise. |

P-YARD2's per-pass split (M, re-read from `D:/Coding/renderlab-work/p-yard2/rl/quarry/timing-*/*/engine.json`, medians):

| Quarry | span | shading | receiver | visibility compute | reconstruction | present | other (GI cache, local shadows, ...) |
|---|---:|---:|---:|---:|---:|---:|---:|
| RT still | 3.33-4.30 | 0.93 | 0.50 | - | 0.47 | 0.63 | 0.81-0.91 |
| RT walk | 4.26-4.50 | 0.93 | 1.42 | - | 0.47 | 0.62 | 0.82 |
| non-RT still | 8.61-8.69 | 0.94 | 0.63 | 4.17-4.27 | 0.47 | 0.63 | 1.69 |
| non-RT walk | 10.69-10.70 | 0.94 | 0.62 | 6.11-6.14 | 0.47 | 0.62 | 1.88 |

On RT, a still frame already costs 0.9 ms less than a moving one, all in sun rays. On non-RT the still-to-moving difference is the sun-visibility pass (4.2 against 6.1 ms) at its held budget. The "other" bucket is the same still and moving, which is what a never-idle GI cache looks like.

## 2. What the count would have to show, and what earlier units measured

- **GI (A, [p-gi.md](p-gi.md) and [m7-gaps.md](m7-gaps.md)).** On a moving quarry frame the cache passes take 793 µs RT and 1,500 µs non-RT, of which the probe is about 445-459 µs and the filter about 103 µs. P-GI recorded that "still frames under TR1 never idle" and that the probe runs on every still frame. On a drained still view the gathers, setup and rays should be near zero, which leaves about **0.5-0.8 ms a frame of jitter-only GI work on both paths** (I). This is the most likely win.
- **Showcase's 3.4 ms of still-frame GI (A, [p-show.md](p-show.md))** is not explained by jitter alone. Showcase RT "GI etc." is 4.4-5.1 ms still against 1.8-2.4 ms moving. A probe-sized jitter cost (about 0.5 ms) cannot make still frames dearer than moving ones. The likelier cause (I) is that Showcase's still capture starts 16 frames after a cut, so its still frames carry the capped gather drain. The `gi_ran`, `gathers` and `bounce_queued` per-frame counts would settle this. Showcase is P-SHOW2's harness and needs an Ara build, so it was not run here.
- **Non-RT page work.** The share of the 4.2 ms still sun-visibility pass taken by the request, compaction and raster passes is unmeasured. `vsm-stable` isolates it.

## 3. Design

Rule: **"still" is a CPU decision on the unjittered camera (bits), and a cache may idle only once its own lagged counters prove it has nothing left to do.** Jitter and direction rotation stay exactly as they are, because they are TR1's sampling, not change.

1. **GI keys on the unjittered view** (`gi-stable`). `idle_key` and `view_key` hash `camera.unjittered()`. This is safe because the GI cache's values are deterministic per key: W1 replaces values, and a fixed direction set re-traces to the same value. Re-running the passes on an unchanged view rewrites identical cells. Idling cannot freeze noise.
2. **A drained signal instead of the 272-frame worst case** (`gi-drain`, `src/gi_drain.rs`):
   - After each cache frame's passes, the 176-byte counter block goes into a 4-slot mappable ring.
   - The next frame maps it, without stalling.
   - Once 8 read-back cache frames in a row show zero misses, traces, integrates, created, invalidated, lost, shades, gathers, queued gathers and deferrals, the gather-capped idle bound drops to `IDLE_AFTER_BOUNCE` (16 identical frames).
   - A table clear resets it.
   - Relights, moved casters and environment changes still set flags, which never idle.
   - Risk to check by count and by eye: a jitter offset not seen during the quiet frames could land a pixel in a cell that does not exist yet. The count's `created` on late still frames measures that directly. P-GI's "exact idle needs 256 zero-work frames" is the conservative alternative: correct, but it never triggers in practice.
3. **Virtual-shadow page work keyed on the unjittered view without the rotation** (`vsm-stable`). The surface cache's still flag additionally requires unchanged jitter bits and rotation, so walking pixels keep resampling every frame as today (no frozen noise). Risk: pages a new jitter offset needs at a page or LOD boundary are not requested while page work is skipped. Check this by image diff on still and cut frames, both paths.
4. **Not adopted: accumulating jittered samples into converged cache entries.** That is a per-cell running average. It is exactly US 10,614,613's (e)-(g), plus dependent 2 (jitter before quantizing), which W1 and C8 design around (section 6). The GI cache needs no such average anyway, since its values are deterministic. For sun visibility, accumulation belongs in TR1's per-pixel history, which already does it.

## 4. Harness, ready to run (C; built, not run)

- `RENDERLAB_P_STILL` knobs (`temporal_reconstruction::p_still`): `nojitter` (offset 0, the attribution baseline), `norotate`, `gi-stable`, `gi-drain` and `vsm-stable`. All are off unless set; with the variable unset the code paths match main.
- `tools/p-still/count.rs`, included by `examples/p_yard_capture.rs` and `examples/ara1_capture.rs` under `RENDERLAB_P_STILL_COUNT`. Per frame it records:
  - whether the GI cache ran, its counters and per-pass profile;
  - virtual-shadow page work, moved rectangles and S4;
  - the surface cache's queued walks (`Renderer::p_still_surface_walks`, doc-hidden);
  - TR1's shadow budget.
- `tools/p-still/run.py`, `counts.py` (per-state medians, still split at frame 272), `flicker.py` (P-YARD2's scorer over this unit's captures, same manifest and metric) and `host.py` (build slot plus stillkit mutex, admission before locks, 6 GiB builds and 7 GiB quarry leases, TMP/TEMP on D:).
- Manifests in `D:/Coding/renderlab-work/p-still/`:
  - `manifest-count.json`: the P-YARD quarry with 448 still frames, then its walk, hold, return, cut and recover;
  - `game-count.json`: ARA1 game-base with 400 still frames, then move and cut.
- **Resume:** one build (`host.py build`, about 3 min warm, 1 min incremental). Then **run the build and the lease one after the other, never both waiters at once** (see section 7). Then one lease:
  `run.py q-base:count:quarry:count:rt q-cand:count:quarry:count:rt:gi-stable+gi-drain+vsm-stable q-base:count:quarry:count:nonrt q-cand:count:quarry:count:nonrt:gi-stable+gi-drain+vsm-stable q-nojit:count:quarry:count:rt:nojitter q-nojit:count:quarry:count:nonrt:nojitter+norotate`
  Then `counts.py` over the six runs.
  - **The gate:** the jitter-only share is (base - nojitter) / base, per counter and per pass, on still frames; the candidate's saving is base - candidate.
  - **If the gate passes:** capture `yard` runs (base and candidate, both paths), run `flicker.py`, and inspect still, cut and recover frames by eye.

## 5. Flicker

Flicker is scored on the walk and return frames: consecutive moving frames, 1 cm and 0.05° a frame. Every part of this design acts only on frames whose unjittered camera, scene and lighting are unchanged, so **it cannot change the moving-frame flicker number** (I, by construction; to be confirmed by `flicker.py` before any merge). The 4.39 / 4.44 against 3.66-3.79 gap comes from noise present while moving. P-YARD2 saw it as heavy grain on flat walls in S4, worse on non-RT. That needs its own unit on moving frames: GI cell level changes and new-cell starts under motion, the 32-direction moving sun budget, and TR1's history weights.

A converged GI cache that stops re-running does not freeze noise, because its values are deterministic per key. A sun-visibility cache that stopped resampling would freeze noise, and this design keeps it resampling.

## 6. Patent notes ([m7-p0-claims.md](m7-p0-claims.md), policy in AGENTS.md)

- **Unjittered idle keys, and the CPU deciding stillness:** these are engine bookkeeping, not an element of any flagged claim.
- **Lagged counter readback (`gi-drain`):** it decides only *whether* the cache runs at all on an unchanged view. It does not rank cells or choose which rays are cast, so W2 holds and US 11,790,594's ranking is avoided. It keeps no per-cell sample count, so W1 holds and US 10,614,613 (f)-(g) is avoided.
- **World-space keys:** a world-space cache alone is allowed under the policy. No resampling or reservoirs are involved (US 11,315,310 needs them together with the cache). Lookups stay deterministic (W3, US 12,423,782).
- **Rejected for patent reasons and not needed:** per-cell accumulation of jittered samples (US 10,614,613 (e)-(g) and dependent 2; W1, C8).

## 7. Process (M)

- **Build 1** (`a38dd78`, both adapters, release): 183 s, admitted 13:03 UTC after Ara and p-show2.
- **Lease `pstill-c1`** (13:03, 72 s): this unit's own second build waiter was admitted in the same second. Its cargo tripped the lease's guard ("contention or available memory below 2048 MiB"), so there is no data. The binary copy failed with WinError 32 because the lease was running that exe. That is a new memory: `own-build-and-lease-waiters-race`.
- **Build 2** (`ed8ed95`, adds `gi-drain`): 53 s incremental. Binaries were copied by hand afterwards (`build.json` records the hashes).
- **Lease `pstill-c2`**: queued from 14:05. It was restarted twice for root steers (TMP/TEMP on D:, then the 15:02 admission bars). At 15:13 it died in its admission check (`resource_sample`) with WinError 1455. No lock was held. It was not retried, and its release line was posted.
- **Time:** about 1 h 10 min of reading, code and harness; about 2 h 50 min queued for the build slot and GPU.
- **Cleanup:** the Cargo target, binaries and TMP folder on D: are deleted. `D:/Coding/renderlab-work/p-still` keeps 1.8 MB of logs, the build identities and the two count manifests for the resume. Nothing was written to C: or E: except the schedule lines.
