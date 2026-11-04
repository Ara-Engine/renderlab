# Exact-input quarry shadow compute probe

The corrected private probe passes numerical validity and supports a compute-based follow-up. It does not establish a production frame-time win or accept the proposed history reuse rule. Main's accepted renderer is unchanged.

## Validation and scope

The first worker run stopped at invalid SPIR-V. A loop-local `ray_query` caused Naga 30.0.1 to emit a null initializer for an opaque type. Root moved the declaration outside the loop while retaining explicit initialization for every ray. That removed the shader error. The next observation stopped because the reference probe sampled main depth without `TEXTURE_BINDING` usage. Root enabled that usage only for the private diagnostic feature. Both failures remain preserved. No bias, geometry, sampling sequence or threshold changed.

The final validation observation has an active Khronos layer and zero reported errors. The separate timing process repeats the same numerical checks with validation disabled. Both cover 346,520 depth-winning receivers and 88,709,120 executed queries. Every visibility count matches the same-run fragment result exactly. Closest-hit distances are also bit-identical. Terminate-on-first-hit matches every visibility count but does not supply nearest distance. There are no missing or extra receivers, query-count differences, or depth mismatches.

The capture uses float32 attachments to retain the actual winning fragment's ray inputs. It does not reconstruct them from quantized history normals. Extra equal-depth fragment invocations are absent from the depth-selected reference; their cost, along with any unmeasured helper work, can remain in the fragment control. No claim isolates a particular hardware cause of the timing gap.

## Timings

Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, optimized build, twelve opaque double-sided quarry placements, 13,034,903 triangles, 1920x1080, 1x, 0.04-radian light disk, unchanged exp -14 lighting and RLE2 environment. Each block interleaves six samples of each variant. The second block reverses order. Workload warmup precedes measurement.

| GPU timestamp interval | Block 1 median, range (ms) | Block 2 median, range (ms) |
|---|---:|---:|
| Unchanged cold fragment mesh pass, including shading | 139.055, 138.286-140.417 | 130.834, 124.432-139.327 |
| Compute closest-hit dispatch only | 15.649, 15.251-18.420 | 14.116, 13.071-15.219 |
| Compute first-hit dispatch only | 14.620, 14.130-15.812 | 13.705, 12.786-15.098 |

Closest-hit compute measures 0.1764 and 0.1591 ns per executed ray. Both block medians fall below the provisional 0.20 ns forecast line in the [plan](m6-shadow-compute-probe-plan.md). First-hit measures 0.1648 and 0.1545 ns. Preserve closest-hit semantics for the next unit because history still needs nearest distance; the modest first-hit difference does not justify dropping that information.

These intervals contain different surrounding work. Compute excludes rasterization, receiver preparation, material/environment shading, history maintenance and presentation. The fragment control includes its original mesh shading. The result demonstrates cheaper exact visibility evaluation in this tested compute arrangement, not a whole-frame speedup or a measured isolated ray-instruction improvement.

The timing process includes startup and finishes in 19.729 seconds. Its load samples span 0-100% GPU utilization and 210-2835 MHz graphics clocks, including startup and exit. User applications remained running. Both blocks drift downward, and the 18.420 ms compute sample is retained. There is no idle-machine certificate. The gap is sufficient for this bounded mechanism decision; no extra timing campaign ran.

## Costs still outside the useful path

Five float32 capture attachments consume 165,888,000 logical bytes. Compute output and result staging each consume 66,355,200 bytes, with a temporary 33,177,600-byte input-copy buffer. These are instrument payloads, not a proposed production layout or physical residency measurement.

The instrument's one capture frame still runs the full 256-ray fragment work. It took 186.640 ms in validation and 453.163 ms in the timing session. Those single observations are not a reusable receiver-pass estimate and were excluded from dispatch timing. Pipeline/binding setup was 103.602 ms in the timing process. A production follow-up must replace this instrument with economical receiver storage and account for every required pass.

Root made two focused repair builds taking 20.234 and 14.385 seconds. The failed resource-binding observation took 7.112 seconds, successful validation 6.281 seconds, and timing 19.729 seconds. These exclude the original worker's 17.945-second build and 15.289-second failed session. Root's isolated editing time was not measured. Analysis and packaging were CPU-only. The accepted checkout was restored and the slot released.

## Next bounded unit

First test the proposed solid-support fallback against the frozen contact/motion cases using the preserved shader-override binary. It remains a heuristic and must not hide disocclusion or thin shadows. If that passes, implement one private 1x compute path with the same closest-hit semantics, bounded receiver/history storage, and grouped fresh-ray work. Reuse the existing depth pass where possible. Measure the complete moving quarry path against a same-session hard-4x anchor, including receiver preparation, classification, history updates and final shading. No production promotion follows from this probe alone.

The complete instrument, final inputs/binary, raw validation and timing output are in `D:/Coding/renderlab-work/m6-compute-ray-probe-repair2/`. `seal.json` verifies 114 compiler inputs and clean source restoration. Patch SHA-256 is `a100d2378e3465027f328ec18fb9c00f76f4befa618b24ed2d1e558e28774c77`; binary SHA-256 is `b786907bd18c69ca94a6e7255e6f65fe49dec2fa22c4bff74b3902203cfb8fa9`. Earlier failures are in the adjacent original probe and `m6-compute-ray-probe-repair/` directories. The shared target contains the private build and must be rebuilt from the next owner's source.
