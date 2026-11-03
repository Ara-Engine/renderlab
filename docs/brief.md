# Project brief

## User intent

Islam wants a separate, public Rust rendering library/engine on this PC, useful independently of Ara. It may eventually be reused in Ara, but integration is explicitly not a current priority.

The immediate motivation is unacceptable iteration speed in the large engine. Treat development latency as a product feature: immediate parameter changes, shader edits without Rust rebuilds, short incremental Rust rebuilds, and prepared scene assets reused across launches.

Islam additionally requires development policy limits to be configurable or explicitly disableable where valid. Small source-file, texture, metadata and streaming-work defaults must not become permanent product ceilings. Expose effective limits through the library and relevant tools, while preserving structural validation, checked arithmetic, format constraints and device limits. This is implemented through typed library limits and an optional shared tooling limits file; see [limits.md](limits.md). Keep registered test budgets unchanged so prior evidence remains comparable. Removing a policy cap does not establish large-asset scalability; that still requires measurement.

The public name is undecided; Renderlab is a reversible working name. The public repository is https://github.com/Ara-Engine/renderlab. The license remains undecided. See milestone-0.md for the completed development environment and project-handoff.md for the current scope.

## Long-term quality and performance target

Islam wants the complete renderer to exceed Unreal in image quality and performance, aiming for 2-5 times better quality and 10 times the rendering performance together. This is the project ambition, not a demonstrated result or a feasibility guarantee. Development latency remains a separate product requirement.

The quality multiplier needs an agreed operational definition. One candidate is 2-5 times lower error against high-quality reference images, supplemented by full-frame inspection and explicit silhouette, shadow, transparency, and motion-stability checks. Lower numerical error alone does not establish perceptual quality. This candidate metric has not yet been selected.

Future comparisons must name the Unreal version, content, hardware, internal/output resolution, reconstruction, enabled effects, quality settings, and timing method. Measure the complete rendering workload, including geometry, materials, visibility, lighting, updates, and required preparation. Report CPU and GPU costs and memory separately. The intended success is the higher accepted quality at the performance target in the same configuration. Separate fast and high-quality configurations do not establish that combined result. Representative workloads must be agreed before making an engine-wide claim; a tiny fixture cannot establish it.

M1 established the correctness and cost measurements needed to test future approaches. Islam has now explicitly authorized M2-M4, including investigation and implementation of justified improvements beyond Nanite. The working scope and acceptance gates are in [milestones-2-4.md](milestones-2-4.md). Stop after M4; the remaining proposed roadmap is not implementation authorization.

M2's persistent instancing and conservative visibility are complete, with root verification and independent Opus acceptance. [Milestone 2](milestone-2.md) records full-image/depth, in-flight replacement, actual viewer, paired performance and development-loop evidence, including corrected oracle-coverage limits, single-sample aliasing and expensive sparse updates.

M3's prepared assets, isolated cooker, material rendering and 1x/4x AA are complete after independent Opus review and a verified sampler correction. [Milestone 3](milestone-3.md) records images, resource stress, live interaction, development latency and carried limitations. M4's bounded virtual geometry follows [its execution plan](milestone-4-plan.md).

M4 is complete with independent Opus acceptance and verified corrections. It provides registered dense fixtures, legal-cut proofs, bounded streaming, safe retirement, constrained refinement/coarsening, prepared first views and configurable development policy. Sampled normal images and the development selection target pass. Final capacity/failure coverage, live interaction, development-loop measurements and accepted limits are in [the acceptance matrix](m4-acceptance-matrix.md) and [root verification](m4-final-verification.md). The unexplained resize outlier, contended GPU measurements and limited content coverage remain disclosed.

On 2026-11-03 Islam explicitly authorized using one of his local scan packs, fixing the outstanding caveats and returning with real results, leaving the PC available for unattended work. This supersedes the earlier stop-after-M4 instruction for that scope. Follow [the scan validation plan](scan-validation-plan.md). It requires actual scanned content, inspected images, responsive interaction and controlled measurements; it does not turn the long-term Unreal ambition into a demonstrated result.

## Milestone 0: development environment (complete)

The analytic viewer established shader reload, editable parameters, a persistent window, and measured development loops. Preserve its results in [milestone-0.md](milestone-0.md). It did not establish a mesh renderer or production image-quality claims.

## Milestone 1: reliable, measurable GPU foundation (complete)

Completed and verified on Windows/Vulkan. [The M1 record](milestone-1.md) contains validation-layer evidence, live-window checks, measurements, review corrections and accepted limitations. The scope below records its contract; later layers still require discussion and authorization.

Follow the scope and acceptance in [project-handoff.md](project-handoff.md). Use the installed pstack workflow with the confirmed large reasoning budget and GPT-6.1 Sol, Astra, and Opus 5.5. This thread owns implementation and decisions directly with Islam.

Implement real triangles/cubes, camera and depth, owned persistent GPU resources, transform-only movement, safe in-flight replacement, responsive resize/minimize and shader recovery. Measure CPU preparation, GPU execution, acquisition/presentation and waits separately. Account for uploads, allocations and pipeline creation without forcing per-frame GPU completion waits.

Verify complete images, occlusion, movement, resize, failed reload and correction. Stress resource replacement with work in flight. Distinguish owned resource counters from driver memory. Remeasure practical development latency on the recorded build and host. Push validated checkpoints as Snowy7 without rewriting history.

No importer, Nanite/virtual geometry, GI, advanced lighting or Ara integration belongs in this milestone. Do not auto-advance beyond it. The execution record is [milestone-1-plan.md](milestone-1-plan.md).

## Historical context and lessons

Ara's render-v3 campaign produced substantial code but no promoted replacement. Lights and moving-sun shadows exceeded budgets; moving-body bounce lighting had large errors and repair delays; reflections improved some images but lacked accepted main-candidate timings. Full-frame G5 results were for the old-renderer control, not an integrated new renderer. Some planned features remained unbuilt.

Do not inherit its assumptions as proven. In particular, B1's unshadowed pipeline cost was about 0.83 ms, while its lean shading pass was about 0.49 ms. Earlier conversation incorrectly described the first number as shading alone. Dense-light cost was dominated by visibility.

Development friction included shared mutable shader cooking, competing GPU timing windows, oversized test/build cycles, and long scene construction/replay. A separate repository must solve these causes, not merely relocate files.

Research is copied into `docs/research/industry-rendering-costs.md`. It contains primary-source engine breakdowns, hardware/resolution caveats and proposals. Historical proposals in that note are not instructions to integrate with Ara or preserve render-v3.

Local evidence, consult on demand:

- Ara checkout: `C:/Users/snowy/.t3/worktrees/snengine/t3-9c4a1f92`
- Final saved campaign commit reported by coordinator: `1e714c2b5` on `t3/render-v3`
- Original architecture: `D:/AraBuild/render-research/council-v3/FINAL-architecture.md`
- Results: `D:/AraBuild/render-v3/gates/G1` through `G6`, especially `B*-RESULT.md`
- B3 report: `D:/AraBuild/render-v3/B3-report.md`
- Existing GPU coordination rules: `D:/AraBuild/render-v3/README.md`; check live ownership before sharing this GPU with any continuing Ara jobs.

Windows RTX 4070 SUPER is the first development host. Broader backend/platform commitments should be made deliberately. Keep initial compilation parallelism at two jobs on this shared PC.
