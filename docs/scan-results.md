# Real quarry: delivered demo and measured results

2026-11-03. The local demo renders twelve distinct scans from Islam's South African slate quarry pack: **13,034,903 source triangles**, twelve 4K base-color textures, 4x MSAA and 16x anisotropic filtering at 1920x1080. It has an interactive camera, shader reload, and verified resize/minimize recovery. The source pack contains separate assets; the quarry arrangement is authored for this test, not an imported assembled level.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/quarry-demo.ps1
```

Arrows orbit, WASD moves, R resets, Escape exits. The command reuses the existing optimized binary and private cooked content; it does not rebuild or import. Add `-Profile dev -Build` for development, `-QualityMode streamed` for bounded streaming, or `-Pose 4` for the close view. Paths and budgets remain configurable. The private detail folder is `measurements/quarry-quality/detail`, a junction to `D:/Coding/renderlab-quality-all4k-20261103`. No scan assets or captures are published.

## Decision: use the full geometry when it fits

This quarry fits in the RTX 4070 SUPER's memory. Its full path submits twelve draws; the adaptive path submits thousands of cluster ranges and spends much more CPU time selecting and managing them. The default therefore keeps the full source geometry resident. This is a content-dependent renderer choice, not evidence that virtual geometry is unnecessary for larger worlds.

Same content, 1080p, 4x MSAA, 16x filtering, optimized build, validation off, same 338-pose camera path paced at 60 Hz. Motion statistics cover frames 1–277. CPU is streaming advance plus draw preparation/submission, excluding pacing and presentation. GPU is the timestamped mesh pass. These overlap and must not be added or converted into measured viewer FPS.

| Quiet-start observation | Full source geometry | Adaptive streaming |
| --- | ---: | ---: |
| CPU p50 / p95 | 0.141 / **0.197 ms** | 19.969 / **37.620 ms** |
| GPU p50 / p95 | 5.214 / **5.876 ms** | 3.574 / **7.642 ms** |
| Worst CPU / GPU sample | 0.820 / 6.203 ms | 47.117 / 11.449 ms |
| Final triangles | 13,034,903 | 3,981,802 |
| Final unresolved detail groups | 0 | 0 |
| Startup-to-prepared-frame proxy | 6.219 s | 5.145 s |
| Peak process working set | 3.586 GiB | 2.164 GiB |

Host: Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14, about 31 GiB RAM, Windows/Vulkan. Each final run started after two GPU-utilization samples at or below 3%; continuing CPU/GPU observations were retained. Browser windows were temporarily minimized with their placement restored afterward. No applications or services were killed. This establishes a quiet starting condition, not a frequency-locked or contention-free machine throughout. Observed host CPU peaks were 24.4% for full and 38.1% for streaming.

Earlier full runs had CPU p95 0.232–0.583 ms and GPU p95 7.538–10.165 ms; streaming had CPU p95 35.941–37.179 ms and GPU p95 6.187–11.348 ms. One contended full run reached 24.449 ms GPU and 96.6% host CPU utilization. Those runs are preserved rather than removed. Warm full startup was 2.752 s in the first comparison; that number does not describe the final quiet-start run, which was not explicitly rewarmed. All six final comparison runs completed with zero final detail debt.

Full geometry uploads 474,842,772 bytes once. Twelve complete 4K texture chains use 1,073,741,808 bytes, versus 268,435,440 at 2K. A native full-scene observation reported 475,371,104 buffer bytes, 1,140,610,048 texture bytes and 1,615,984,512 allocator-allocated bytes (1,946,157,056 reserved). These are renderer/backend observations, not physical driver-memory certification or process working set. The streamed geometry charge peaked around 492 MiB under its 512 MiB budget; it does not save geometry memory on this particular full-resident comparison.

## Quality: measured fixes, not a quality multiplier

The reference uses full source geometry at 3840x2160 with 4x MSAA, reduced in linear RGB to 1920x1080. It is a finite supersampled reference for geometry edges and base-color detail, not a physical-lighting reference. The gates were retained across failed and successful candidates.

| Isolated change | Measured error reduction | Scope |
| --- | ---: | --- |
| 1x to 4x MSAA, same 4K textures | **48.30%** silhouette-band error | Twelve spatial frames and eight adjacent grazing-motion pairs pass |
| Ordinary to 16x anisotropic filtering, same 4K/4x | **43.92%** close/grazing texture error | Twelve spatial frames and eight adjacent motion pairs pass |
| Regenerated 2K to 4K, both with 16x filtering | **17.37%** close/grazing texture error | Two matched close/grazing frames; exceeds the 10% gate |

The initial ground-only and all-texture resolution changes without anisotropic filtering improved texture error by only 8.30% and 8.46%; both failed the 10% gate and remain recorded. Resolution alone did not solve grazing blur. The final filtering comparison's maximum silhouette-band mean absolute error was 0.00377, with maximum temporal edge error 0.00544 and texture error 0.00990, under the registered metric. These reductions are separate experiments and must not be multiplied into an engine-quality claim.

Root inspected the full grazing image, native-resolution before/after/reference crops, the twelve-frame contact sheet, and restored/portrait native-window captures. Ground detail is visibly clearer; complete silhouettes and aspect are retained. The scorer's raw `visual_status: UNRUN` is deliberately untouched: manual inspection is recorded here separately from automated scoring. The short motion sequence does not certify every possible movement. Scan patch perimeters appear in the full source geometry too; they are content/layout boundaries.

The optional renderer anisotropy setting preserves explicit nearest filtering and disabled mipmaps. The library default remains 1; the quarry launcher chooses 16. Shader reload preserves the chosen sample/filter configuration. Full source vertex/index data and material assignments were retained across texture variants; regenerated 2K/4K controls share the original 8K source decode and linear-light mip policy.

## Interaction and development latency

The final native full-detail session presented 2,205 frames with active Khronos validation and zero errors. It exercised portrait/landscape resizing, minimize/restore, camera input, malformed shader rejection and successful correction. Five actual in-flight resize deferrals occurred; every configuration began after draining held frames and renderer submissions. Initial configure took 225.502 ms; six subsequent configures took 11.908–20.521 ms. Restored and portrait images were inspected and complete. The original 4.93 s driver configure outlier was not reproduced; its exact historical cause remains unknown. The verified change removes avoidable reconfiguration while work is in flight, rather than claiming a universal driver latency bound.

The final development profile keeps viewer code at opt-0, the renderer at opt-1 and selected stable dependencies/asset selection at opt-2, preserving debug assertions and validation. The root opt-1 trial reduced full-scene upload/setup wall time from 8.584 to 3.590 s and startup-to-present proxy from 10.913 to 5.693 s in single observations. The first profile rebuild took 10.18 s Cargo time. This is a compilation tradeoff, not an algorithmic improvement.

One final edit-loop run on the same full 4K/4x/16x quarry with active validation measured:

| Operation | Observation |
| --- | ---: |
| No-op build, process wall | 0.375 s |
| Real renderer body edit build / restore build | 3.597 / 3.558 s |
| Rust edit to externally observed completed/presented frame log | 10.898 s |
| Startup-to-present / completion observed, shader-run launch | 7.136 / 7.156 s |
| Shader pipeline compilation | 8.062 ms |
| Shader write to present-call / completion-observed proxy | 32.121 / 89.486 ms |

These are single samples with ordinary desktop contention, not percentiles or scanout measurements. The viewer checks shaders every 50 ms. Process launch/log polling and loading roughly 1.5 GB of prepared content contribute to Rust edit-to-image; shader edits keep that content resident. The helper restored source bytes exactly and rebuilt the normal binary. No clean-target rebuild or repeated timing campaign was added. Earlier viewer-only body-edit timing was 2.73 s; it is a different edited package, not a direct before/after renderer comparison.

## Verification and identities

The consequential selector/coalescing and bounded streaming changes were independently reviewed by Opus before their checkpoint. Final filtering/preset integration received focused root checks: sampler-compatibility CPU test, the existing material/sampler/normal GPU test with active validation, four quality-tool CPU tests, workspace/all-target Clippy and formatting, the actual image comparisons, and native interaction above. The broad historical GPU stress suite was not repeated for this final sampler option/profile change. All final quality renders and the native session had active validation and zero errors.

Raw evidence is local and ignored:

- Timings and identities: `.refs/quarry-tuning/final-measurements.json`, corresponding `*.process.json`, `measurements/quarry-tuning/detail-*`, and `.refs/quarry-tuning/window-isolation*.json`.
- Quality: `D:/Coding/renderlab-quality-{aa,filter,resolution-filter}-report-20261103/results.json`, comparison images/contact sheets, and the retained failed resolution reports.
- Window: `D:/RenderlabEvidence/quarry-detail-window/result.json`, `restored.png`, `resize-620x900.png` and its source record. Earlier camera/reset verification: `quarry-interactive-3`.
- Development loop: `measurements/dev-loop/20261103-204558/results.json`; `.refs/quarry-tuning/detail-dev-{startup,opt1-startup}.process.json` and logs.
- Focused material GPU check: `.refs/quarry-tuning/sampler-check.log`, `measurements/quarry-tuning/sampler-check`.

| Artifact | SHA-256 |
| --- | --- |
| Full 4K prepared content | `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3` |
| 4K virtual package | `f33547c57dd1f0a2efb93f352aa4df9c4d4cb3d2c9bdabeeddf7ce21c077d5ac` |
| 338-pose camera path | `e1f79a07f58901d0669e5825b9dce5a8ab3647d6f860a6f84f0e1a3269c70823` |
| Optimized measurement replay | `9c0ce4d26520d745bc8c7c3f1b57facf28ac837aaf797dbdbd7f1f0fb70298be` |
| Final native optimized viewer | `fba2eea1c2f67bea45285c1fe0b7d948b81aa5690b4893e61cec658c3cd7cbc0` |

The replay/native source records precede only test/helper/documentation edits and the dev-only opt-1 override; runtime rendering sources match. Final dev-loop identities are recorded separately. The checkpoint containing this document is the source closure, rather than a claim that every measured binary was built from an already committed tree.

## What this closes and what it does not

There is now a usable, inspected real-scan geometry demo with sharper edges/textures, measured full-resident performance, working resize/reload, and a practical shader loop. The adaptive path recovers detail and respects its budgets, but its moving CPU p95 remains about 38 ms on this scene; the 8 ms optimized streaming target is **not met**. The full-resident default is a justified fast path, not a fix for that remaining scaling cost. The request to eliminate every caveat is therefore not claimed complete.

Materials currently display opaque unlit base color. Normal-map response, physical lighting, shadows, transparency and temporal reconstruction remain future layers. This scene proves substantial scan ingestion and rendering, not arbitrary production-world scalability or superiority to Unreal/Nanite. Further geometry optimization should have a specific larger-than-memory workload or measured streaming requirement; it should not delay using this demo to assess the next visual layer.
