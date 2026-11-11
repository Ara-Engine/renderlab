# Renderlab: direct ownership and agreed plan

Recorded 2026-11-02 from Islam's rendering discussion. This records the latest user direction; older milestone numbering in brief.md, AGENTS.md and milestone-1.md predates it. New instructions from Islam take precedence.

## Ownership and workflow

Islam will communicate directly with the existing Renderlab implementation thread. That agent owns the project conversation, planning, implementation, validation, documentation and pushes. The original discussion agent has stepped out; do not require its approval, supervision or relay to proceed.

Current authorization: **finish the revised Milestone 1 using pstack**. Locate and follow the installed pstack workflow. If unavailable, explain that directly to Islam rather than silently substituting a workflow. His updated model selection is GPT-6.1 Sol, Astra, and Opus 5.5, with the large (xhigh) pstack budget. Resolve any unavailable requested model directly with him. Any parallel work must have explicit file ownership.

Complete the authorized milestone autonomously, including focused builds/runs, tests, visual inspection, measurements and fixes. Keep concise progress updates and durable checkpoints. Discuss the next layer with Islam before starting the next milestone. Questions are requests for answers, not implicit permission to edit.

## What this project is

- An independent, public Rust rendering library/engine with its own users and use cases. It is not merely an Ara benchmark harness or temporary extraction.
- Extremely fast development and compilation are central product requirements: persistent viewer, immediate parameter updates, shader reload without recompiling Rust, short incremental Rust rebuilds, reusable prepared assets when those arrive.
- Ara integration is deferred. Do not grow engine-specific adapters or import its architecture to satisfy this milestone.
- Keep APIs typed, ownership clear, dependencies deliberate, and implementation simple. Avoid speculative abstractions and oversized crates.
- Public repository: https://github.com/Ara-Engine/renderlab. Local root: D:/Coding/projects/desktop/renderlab. Renderlab is a provisional name; project license is still undecided.

## Milestone numbering and existing evidence

**Milestone 0: development environment, completed.** The initial implementation/report called this milestone 1; preserve its measurements as historical evidence when renaming or clarifying the docs.

It established wgpu 30 with Vulkan/WGSL, a separate winit viewer, editable parameters, transactional shader reload, GPU image tests and development-loop measurements. The library has no windowing dependency. The fixture is analytic spheres and a floor drawn through a fullscreen shader, not a mesh renderer. No importer, ECS or Ara source was introduced.

The old fixture has fixed shadow sample bands, distant checkerboard aliasing, constant ambient fill, approximate transparency and no temporal reconstruction. It proves a development loop, not production quality or performance. Its existing last-valid-pipeline behavior must survive future changes.

See milestone-0.md for the original measured methods/results and scripts/measure-dev-loop.ps1 for reproduction. That script temporarily edits/restores source files: do not run it concurrently with edits to those files. Historical measurements were development builds on this PC with uncontrolled contention; do not turn them into production frame-time promises.

## Milestone 1: reliable, measurable GPU foundation

This is the currently authorized implementation, not the completed analytic fixture.

Build a small real mesh scene with colored triangles/cubes, a camera and correct depth testing. Establish the resource and execution foundation needed for later geometry and lighting work:

1. Owned buffers, textures and pipelines with explicit safe creation, updates, replacement and release.
2. Persistent geometry uploaded once; moving objects changes transforms without rebuilding mesh buffers or pipelines.
3. Correct command sequencing, synchronization and in-flight lifetimes. Avoid unnecessary CPU/GPU serialization in ordinary rendering.
4. A responsive viewer through movement, resize/minimize, shader edits, rejected reloads and recovery. Define honest recovery behavior for device/surface errors.
5. Separate observable costs for CPU preparation, GPU execution, waits/presentation, uploaded bytes, resource allocation/churn and pipeline creation. Timing collection should not force the frame loop to wait for every result.
6. Preserve and remeasure development latency: shader edit-to-image proxy, representative Rust edit-to-running-viewer, startup, no-op build and clean-target build where relevant. Label proxies instead of claiming measured scanout.

Acceptance evidence must establish all of the following:

- Correct full images and depth/occlusion while camera/objects move and targets resize; actually inspect rendered results and exercise the viewer.
- Static frames do not repeatedly upload geometry or recreate persistent resources/pipelines. Transform changes have bounded, identified uploads.
- Repeated mesh, target and pipeline replacements do not produce unbounded retained resource growth; distinguish owned allocation accounting from actual driver memory observations.
- GPU validation stays clean, including replacement with work in flight and repeated resize/minimize/recovery paths.
- Bad shader syntax or incompatible bindings preserve the last valid rendering pipeline; correcting the shader restores live updates.
- Focused checks/tests pass, and development latency remains practical with actual measurements and any regressions explained.
- Report what was tested, the exact build identity and host/scene/resolution/quality/contention, and any unverified behavior. If an acceptance condition fails, fix it or explicitly report the milestone incomplete.

Implementation details are the owning agent's responsibility. No new mesh architecture, render graph, MSAA method or timing implementation was locked in the discussion. Choose the smallest design that satisfies the acceptance conditions and document consequential choices.

## Later layers: direction, not authorization to implement now

Work from foundations upward, one layer at a time, discussing the next step with Islam. Geometry/visibility comes after the GPU foundation: mesh storage, instancing, visibility/culling and scalable rendering. A Nanite-like system would then require its own substantial work on hierarchical triangle clusters, detail selection, residency/streaming and specialized rasterization/visibility handling. It is not part of M1 and can be developed before advanced lighting.

Materials, direct illumination, shadows, indirect light, reflections, transparency and image reconstruction remain future topics. Their exact milestone boundaries and algorithms have not been agreed. Do not present a fabricated fixed roadmap or automatically advance into them. Deferral sets build order, not permanent feature cuts.

Islam subsequently set the long-term ambition at 2-5 times Unreal's image quality and 10 times its rendering performance together. See brief.md for the comparison requirements and unresolved definition of the quality multiplier. No claim that this design beats Lumen or any production renderer has been demonstrated. Islam specifically noticed rough/jittery cube/model silhouettes, overly dark/hard shadows and pixelated shadow edges. Judge complete images and motion, not only region-average lighting scores. When lighting arrives, evaluate shadow softness, darkness, edges and stability explicitly alongside cost.

## Evidence before optimization

Read research/industry-rendering-costs.md before selecting major rendering algorithms or making industry comparisons. It contains primary-source production breakdowns and their limitations. Published pass times depend on hardware, scene, quality, resolution and overlapping execution; generic internet millisecond tables are not universal budgets.

The central lesson is to measure where time actually goes and understand work that production renderers avoid, share, cache or amortize. Geometry, visibility, materials, shadows and presentation matter alongside GI. Do not optimize a fashionable subsystem while ignoring a larger measured cost.

Ara's render-v3 campaign produced experiments but no accepted promoted replacement. Preserve the numerical distinction: B1's roughly 0.83 ms was the unshadowed pipeline, while roughly 0.49 ms was lean shading alone. Dense-light cost was dominated by visibility. Other campaign acceptance failures and source locations are in brief.md; old proposals are research evidence, not inherited requirements for this independent library.

## Publishing and checkpoints

Islam explicitly requires active pushes of validated checkpoints, not local commits only. Publish to Ara-Engine/renderlab and report the SHA. Use personal **Snowy7**, not the work identity IslamSpoilz. Repository-local Git identity was set to Snowy <97126807+Snowy7@users.noreply.github.com>; verify it before committing.

Published history was corrected with explicit user authorization immediately before this handoff. All four existing commits were verified through GitHub as authored and committed by Snowy7. Main at handoff: 2b7b9be26f4d04986470c79d887b179efb60c826. A local backup branch preserves the old attribution. Do not force-push again or restore the old history. Old measurement records may contain pre-rewrite SHAs; code trees were preserved by that identity-only correction.

The discussion agent made no implementation changes for M1. Its only authored file in this handoff is this document. Islam authorizes the owning agent to include it in a normal checkpoint and link it from the active brief/instructions while reconciling the milestone numbering.

Use the existing two-job Cargo setting. Windows/RTX 4070 SUPER is the current tested GPU host; check current load and any active GPU quiet-window ownership before measurements. Keep captures/raw measurements ignored. Do not introduce persistent services or close T3. Other platforms and licensing decisions remain to be discussed directly with Islam.

## 2026-11-10: optional ara-gpu backend authorized

Islam authorized the pass-by-pass WGSL/naga migration in `src/backend_ara` behind
`backend-ara`, starting with post and easy leaves. The default wgpu renderer and
PC-owned pass files stay unchanged. Draft PR #3 and `docs/ara-gpu-backend.md` track
Metal replay proof, native adaptations and pending Vulkan verification. Earlier
integration-deferred directions are superseded for this isolated candidate backend.
