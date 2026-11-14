# Non-RT reflections, round 2

No runtime default from this round is retained. The compacted reflection pass remains on by default on both paths from the first round; outgoing-hit caching remains opt-in with `RENDERLAB_SW_REFLECTION_CACHE=0.35` (`0` disables it).

The mirror-list and split-mirror-kernel experiments were reverted. Moving mirrors out of primary shading saved about 0.55 ms in combined shading/reflection GPU time before the separate GI receiver pass became the non-RT default. Against that newer main, the combined mirror/cache variant saved only 0.07 ms in Glint course frame time and was flat in Showcase and Outpost. No shared shading-kernel changes remain.

## Cache-only result

RTX 4070 SUPER, 1920x1080, default scene quality, game closed, exclusive smoke lease (Cargo builds blocked). Test source: `c0a1cafbbdf8644027e13a9e74ab25f865b37c4b`; yardstick: `96925a7879c2db19a1670a3d90e2ef157f738b9b` (main's fog/GI defaults). Run: `20261115-022348-bin-smoke-c0a1cafb`. All three scenes on both paths completed with zero errors.

GPU milliseconds below are main -> cache at roughness 0.35. Total is the median of the summed reflection stages, not the sum of stage medians.

| Scene, non-RT | Trace | Hit shading | Reflection total |
| --- | --- | --- | --- |
| Showcase | 0.4925 -> 0.6405 | 0.6185 -> 0.4966 | 1.1402 -> 1.1576 |
| Outpost | 0.2739 -> 0.3594 | 0.5771 -> 0.4321 | 0.8622 -> 0.8059 |
| Glint | 0.4055 -> 0.4367 | 0.3123 -> 0.2985 | 0.7286 -> 0.7823 |

The hit-shading saving is offset by the cache lookup's cost in the trace kernel. Register pressure versus lookup traffic was not isolated. No ray count, history, roughness cutoff or ray length was reduced.

Frame milliseconds (main -> experiment): Showcase non-RT 7.21 -> 6.88; Outpost 12.78 -> 12.91; Glint course 9.89 -> 10.00. The Showcase wall-time saving was not supported by a reflection GPU saving. Glint goal was flagged slower (11.79 -> 13.14 ms), but the reflection-stage change was only 0.054 ms, so that whole-frame outlier is not attributed to this cache. The experiment was rejected independently because its own GPU cost did not improve materially.

Hardware frame times were within smoke's tolerance: Showcase 5.91 -> 5.96, Outpost 10.47 -> 10.54, Glint course 7.64 -> 7.66 ms. The cache returns early without changing shaders or bindings on ray-query devices.

## Follow-up

The round's frame-time targets were not met. Test hit-light reuse after traversal, where it cannot increase BVH-kernel cost; then evaluate screen-space hit reuse with software fallback or reflection-aware history. Preserve the complete off-screen ray fallback and inspect emissive reflections and motion before changing defaults.
