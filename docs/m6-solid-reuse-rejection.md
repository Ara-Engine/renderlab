# Solid-support history reuse rejected

The real-quarry quality screen rejects the proposed unanimous solid support (USS) fallback. The accepted renderer and shared build target are unchanged. No compute implementation or new build occurred in this unit.

All 78 frozen contact and motion gates passed, but the fallback fired zero times in those planar cases. Those passes do not establish its quality. In the actual quarry forward step, the fallback accepted 189,758 receivers. Eight became fully lit where the original same-frame 256-ray estimate was partially shadowed. At pixel (1274,458), the visibility changed from 0.671870 to 1.0. Four deterministic clear rays did not detect the missing occlusion.

On the reverse step, pixel (1196,329) became fully lit instead of the original strict result 0.734371. The finite cold reference at this exact camera is 0.714839. The exact-static cache then retained the wrong value. Root inspected the causal crops and verified the source, result and release hashes. No particular underlying blocker type has been established.

The same-population reverse comparison uses all 346,520 common receivers. Mean absolute error against the finite cold reference rises from 0.0002209424 to 0.0002220697, and errors above 0.1 rise from 34 to 35. The increase is small globally but includes visible missing partial shadows. A low mean does not excuse them. The finite reference remains an estimate, not continuous ground truth.

Cold values, guides and depths match the retained baseline exactly. Main/prepass depth equality, zero hidden queries, and reset reasons remain intact. Both native runs report active Khronos validation with zero errors. This is a reuse-policy failure rather than a depth or capture mismatch.

Native verification took 33.673 seconds for contact and 11.590 seconds for quarry, with no build. CPU analysis took 2.265 seconds. The recorded 193.968-second preparation span overlaps reads and verification, so isolated implementation time is unavailable. Evidence is preserved in `D:/Coding/renderlab-work/m6-compute-history-t3-40219f60/`. The rejected complete patch has SHA-256 `0f8f2d90d8bd1e79005eecf4bfef7a4a06107d6e9f846cc0904a31f49591b7f7` and was not promoted.

## Next unit separates compute from reuse

The [exact-input compute result](m6-shadow-compute-probe.md) remains valid. The next private implementation uses fresh closest-hit visibility in compute, without temporal reuse. Each covered receiver gets the unchanged 256-direction estimate. A minimal receiver pass supplies exact world position and geometric normal, compute writes visibility, and ordinary shading consumes it. Reuse existing raster/depth infrastructure and keep ownership small; the five-attachment measurement instrument is not a production layout.

This intentionally spends fresh work every frame. The rejected history design's four/eight-ray work forecasts do not apply to it, and it cannot claim to pass that history screen. Preserve its unchanged image-quality thresholds. First prove equivalence to fresh fragment reference images, including thin/contact and moving-view cases, then measure the complete actual quarry frame. Include receiver rasterization, compute, shading and required resource costs together.

The unit's purpose is a usable, quality-preserving compute implementation and an honest complete-frame baseline. The hard-4x internal comparison and the matched Unreal fidelity/2x performance contract remain separate, unmet targets. A useful intermediate speedup is not milestone completion. No new reuse heuristic, soft-4x expansion, or benchmark campaign is authorized by this unit. Stop if the focused quality check fails; retain a correct improvement for root review even if it still misses the final performance target.
