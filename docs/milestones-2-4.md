# M2-M4 execution contract

Islam explicitly requested implementation of M2-M4 after accepting M1. For M4 he wants improvements over Nanite where possible, addressing shortcomings or extending functionality. Root owns execution, verification, conversation and pushes. M1's stopping instruction is superseded only through M4. The independent renderer, fast development loop, personal Snowy7 identity and deliberate public-source/asset rights remain requirements.

This is an execution contract, not a settled architecture or a promise of superiority. Base is accepted M1 commit `39ada14adcc9a11cc1d651894d41ca8cda8fedd8`. Apply pstack figure-it-out, how, architect/arena and the decision trail. Use Sol/Astra for grounding, candidates and implementation; reserve Opus for consolidated consequential judgment, acceptance review and any hillclimb. Do not copy Unreal source or import Ara's dependency tree.

## Deliverables and falsifiable gates

**M2: scene scale and visibility.** Reusable geometry, many instances, conservative camera visibility and efficient batched submission. A deterministic scene generator and camera paths compare the optimized path against an unculled reference. Fully visible, mostly invisible, intersecting-frustum, nonuniform/mirrored transforms, moving objects and camera cuts must preserve correct full images. Sweep approximately 1k, 10k and 100k instances where host limits permit. Report actual visible/submitted counts, CPU preparation, GPU pass time, uploads, pipeline/resource churn, memory and development latency. Static geometry remains resident. A lack of measured improvement or a correctness failure is reported and repaired rather than hidden by changing the scene. The pre-optimization fixture and evidence are in [m2-baseline.md](m2-baseline.md).

**M3: assets and materials.** Useful static model/texture ingestion and reusable prepared assets, with small typed library interfaces independent of the viewer. Choose a documented supported format subset, validate input, preserve transforms/material assignments, handle normals/UVs and texture color spaces correctly, and report unsupported features clearly. Exercise at least two redistributable or self-authored assets plus malformed/missing-data cases. Verify textured full frames, material boundaries, mip/normal/color-space behavior as supported, cache reuse/invalidation, preprocessing/upload bytes and times. Import/cooking dependencies must not burden ordinary runtime edits unnecessarily. Physically based material representation does not imply M5 lighting or M7 GI is implemented.

**M4: virtualized geometry.** A working clustered hierarchy and screen-error-driven detail selection with bounded residency, real page loading/eviction and fallback coverage. Evidence must include detailed assets exceeding the configured residency budget, camera traversal/cuts, repeated eviction/reload, bounded queues/resources and comparison to full-detail images. Merely keeping every LOD resident is not streaming acceptance. Check cracks/holes, silhouettes and motion during LOD/residency transitions. Record preprocessing/cache cost as well as steady and burst CPU/GPU/upload/memory costs. Select hardware/software raster and optional occlusion machinery from measured needs, not a mandate to replicate Nanite internals.

M4 additionally investigates specific current Nanite limitations from primary sources. Select at least one tractable improvement or extension and make its acceptance predicate explicit before implementation. Candidate dimensions include editable/procedural geometry, preparation latency, feature preservation and inspectable quality/residency control. They are hypotheses until current capabilities and costs are researched. Preserve failed experiments. A feature or benchmark success only supports the demonstrated scope; an overall Nanite/Unreal superiority claim requires a versioned equivalent-content comparison and will remain unproven without it.

## Execution and ownership

1. Ground current interfaces and evidence, research current Nanite capabilities and relevant asset/tooling options. Read-only workers own isolated findings under `.refs/`; root owns project docs.
2. Compare two structurally distinct small designs using the confirmed Sol/Astra roles. Synthesize with limited Opus judgment. Record the chosen API, data shape, risks and measurement predicates before code. Revisit only if implementation disproves it.
3. Establish reproducible scene/image/measurement fixtures and implement M2 in independently verifiable units. Inspect actual viewer behavior, pass GPU validation, measure and push its checkpoint.
4. Implement and validate M3 against those interfaces and fixtures. Record rights/attribution before committing any external asset or code. Push accepted M3.
5. Build and test M4's hierarchy, selection, bounded streaming and chosen improvement. Establish a full-detail baseline before judging reduced-detail quality. Inspect complete frames and motion, including failure/recovery cases. Push validated units and accepted M4.
6. Obtain independent acceptance review, repair findings, publish results and limitations, and stop. Discuss later layers before implementing them.

Only one worker may own renderer integration at a time. Partition file ownership before concurrent edits. GPU jobs and source-mutating dev-loop scripts are serial. Use two Cargo jobs. Check live GPU ownership/load before runs. Preserve M1's positive Khronos activation and zero-error validation gate; the recorded host needs a process-local empty implicit-layer path because of stale overlay registrations. No global system settings may be changed to hide diagnostic failures.

Each milestone keeps last-valid shader recovery, responsive resizing/minimization, bounded in-flight lifetimes and the library/window boundary. Establish useful basic antialiasing during the geometry work if its measured quality/cost warrants it; advanced reconstruction remains later work. Document any necessary scope clarification without silently declaring a missing acceptance condition complete.

## Initial uncertainty and audit

Largest risks are feature-preserving hierarchy construction, avoiding cracks across selected detail levels, reliable streaming fallback without hidden waits, and retaining fast builds while adding import/cooking support. No particular new dependency or algorithm is selected yet. Research and design should resolve those risks rather than produce a general engine architecture.

The decision trail is `milestones-2-4-decisions.tsv`. Raw measurements/captures stay ignored. Each checkpoint records exact source/binary identity, build profile, hardware, content, resolution, quality, validation/layer settings and contention. Development latency and optimized rendering throughput remain separate measurements.

The selected implementation contract and reconciled pstack review are recorded in [m2-m4-design.md](m2-m4-design.md). M2 starts with independently tested bounds/clipping, then persistent instance storage and image-equivalent batching before enabling culling.
