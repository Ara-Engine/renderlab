# RT sun pass measurement

`RENDERLAB_SUN_PASS=1` splits reconstructed Interactive frames on traced RT
primaries into camera visibility and sun visibility. It is off by default.
Exact, unreconstructed Interactive, film accumulation and non-RT keep their
existing dispatches.

The camera dispatch is the existing `primary_trace_receivers`: it runs
`primary_closest`, including edge correction, and publishes the corrected
receiver's world position, epsilon and geometric normal as float32 values.
The sun dispatch is the existing `primary_vb_sun`, in adjacent 8x4 pixel groups.
It reads those receiver values and the corrected hit record, then calls the
same `primary_sun` as the fused reconstruction kernels. Direction rotation,
per-receiver restart budgets, cold and held probe stops, ray flags and ray
bounds stay in that shared estimator. History is read from the same parity
and written after the sun estimate.

This experiment adds no textures, buffers or bindings. It reuses the existing
float32 receivers, R32Uint visibility count and held-history targets; it does
not introduce a second visibility allocation. The visibility count's existing
0..256 integer contract and programmable shading ABI are unchanged.

`RENDERLAB_PASS_LOG` reports camera work as `visibility` and the following sun
dispatch as `sun_shadow`. The ordinary frame queries also timestamp each pass
separately. Without the switch, `visibility` contains the original fused work.
The split changes camera scheduling from deferred edge repair to the existing
full correction kernel; its cost must be reported with the sun cost. An
8x4 coherent launch is already used by the fused kernel, so no gain from the
tile shape alone is assumed.

The first measurement decides whether sun rays or camera traversal account
for the outstanding visibility cost. This is not yet a demonstrated performance
improvement or an accepted default.

## Verification

2026-11-13, main `c35a19ee` plus this change, RTX 4070 SUPER, Vulkan,
`game_scene` at 1280x720 with Fox, default flight and `--night-lanterns`:

- Switch off against the coordinator's main picture reference: 30/30 PNG hashes
  equal in each of the four scene/path cases (RT and non-RT).
- Switch on against off: 34/34 moving frames byte-identical in every case.
  Zero differing pixels, zero pixels over eight codes and zero frame-to-frame
  change residual. All twelve launches exited successfully without validation
  errors or panics. Frame 33 of both clips on both paths was inspected.
- Linux lib suite on this source: 271 passed, zero failed, 180 adapter-dependent
  tests ignored. Includes the focused composed sun-module Naga validation.
- Windows release build: 26.14 seconds after merging the caster masks.
- Warm-cache CPU pipeline creation in the picture run: camera receiver entry
  0.000867 seconds; reconstructed sun entry 0.066103 seconds. These are pipeline
  creation measurements with existing caches, not cold shader compile times.

Evidence is under `D:/Coding/renderlab-work/u-sunrt`: `pictures.json`, per-frame
`hashes.json` files, `linux-lib-casts.log`, `build-casts.log`, and
`on-rt-flight.log`. The ordinary picture lease was
`u-sunrt-casts-pictures-17` (123.48 seconds).

## Paired timing

`u-sunrt-timing-03`, an isolated `--timing` lease, finished in 50.67 seconds
without runner failures. Same 1280x720 RT clips with Fox, 160 frames per visit,
121 samples at frame 40 or later. Round one visited B1/off/on, round two
on/off/B1. Each row reports the median of each pass, in milliseconds:

| Clip / round | Off: fused | On: camera | On: sun | On: sum | Sum minus off | Whole frame off / on |
|---|---:|---:|---:|---:|---:|---:|
| Flight 1 | 1.2339 | 0.1260 | 1.1213 | 1.2473 | +0.0134 | 11.2497 / 11.2527 |
| Flight 2 | 1.2319 | 0.1260 | 1.1192 | 1.2452 | +0.0133 | 11.2271 / 11.2538 |
| Lanterns 1 | 1.2339 | 0.1260 | 1.1244 | 1.2504 | +0.0165 | 16.8438 / 16.8458 |
| Lanterns 2 | 1.2370 | 0.1260 | 1.1213 | 1.2473 | +0.0103 | 16.8796 / 16.8888 |

The B1 reference's fused visibility was 1.2349 ms on flight and 1.2360 ms on
lanterns in both rounds. GPU clocks were 225/810 MHz (P5) at admission and
2820/10501 MHz (P0) at release; measurements discard the first 40 frames.
The split costs about 0.01–0.02 ms and stays off. Sun visibility is about 90%
of the isolated pair on these clips, so reducing the sun rays is the next
target. This split changes kernel scheduling; its separate pass times are
not an instruction-level decomposition of the fused kernel. The coordinator's
Outpost reference (3.01 ms RT visibility in a 9.71 ms held frame) was not
measured by this fixture. `timing-summary.json` and the per-visit JSONL files
retain the measurements.
