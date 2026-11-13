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
`u-sunrt-casts-pictures-17` (123.48 seconds). Paired timing follows separately.
