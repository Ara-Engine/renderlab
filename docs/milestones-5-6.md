# M5-M6: lighting and shadows

Authorized by Islam on 2026-11-03 after the real-quarry checkpoint `95f9de6a3e0a0cbe4c80289ba228a25f69351e50`. Implement M5 and M6 in the main checkout while isolated pstack hillclimbs address M4's moving-stream CPU cost and historical resize stall. This supersedes earlier stop-after-M4 text. M7 and later remain outside implementation scope. Update 2026-11-07: Islam authorized M7 (see [milestone-7.md](milestone-7.md)). M5's glossy-motion and environment-visibility items fold into M7, since GI and reflections replace those mechanisms. M5's development-loop timings and matched Unreal acceptance, and M6's camera-moving shadow cost, continue in an isolated hillclimb worktree.

## Acceptance target

The quality floor is Unreal-equivalent visual fidelity. The performance floor is at least 2x Unreal at that quality, on the same scene, hardware, camera, light/material settings and output requirements. Better visual quality remains the ambition. A 2x throughput claim needs measured end-to-end evidence, including CPU critical path, GPU work, updates and required preparation, not one selected pass. State sample counts, resolution/reconstruction, validation, build identity and contention. Until those comparisons pass, report progress rather than parity or superiority.

Use installed Unreal 5.7 as the initial versioned comparison target after verifying its exact build. Compare the M5-M6 direct-light/material/shadow subset with equivalent Unreal settings. A comparison that omits GI, reflections, atmosphere or reconstruction on both sides is explicitly a subset comparison, not the final engine-quality gate. Use independent numerical/reference scenes and full-frame inspection to avoid matching a defect. Do not copy Unreal implementation source into this public library.

## Deliverable

M5 introduces physically based opaque surface shading, directional/point/spot lights, normal-map and roughness/metallic response, environment illumination and basic exposure/tone mapping. Preserve correct color spaces, mirrored/nonuniform transforms and material assignment. Import/preparation remains isolated from ordinary renderer/viewer edits. The actual quarry is a required visual artifact; a small material/light fixture supplies controllable correctness cases. Light parameters must be editable without rebuilding pipelines or reuploading meshes.

M6 adds direct shadows with measured softness, contact quality, distance handling and movement stability. Exercise camera motion/cuts, moving lights and blockers, thin features, grazing receivers and high-detail scans. Coverage and bias errors must be visible in full-frame/reference comparisons. Choose the smallest justified implementation after grounding and competing design sketches; neither Virtual Shadow Maps nor ray tracing is mandated. Price shadow generation, filtering, cache maintenance and invalidation separately. A static cache result cannot establish moving-light cost.

Basic exposure/tone mapping belongs in M5 so lighting can be evaluated immediately. Advanced temporal reconstruction stays later. Environment illumination is not general local bounce GI. M5-M6 do not claim Lumen-equivalent indirect transport or full lighting support for transparency/volumes.

## Non-RT shadow default (N8p, 2026-11-07)

Islam authorizes `VirtualLookup::Surface` as the default for `SoftShadows::Virtual`,
including the viewer's inherited settings. `VirtualLookup::Exact` remains selectable as
the reference; historical Exact identity/reference gates select it explicitly. This does
not change the separate `SoftShadows` sampling preference.

[N8p](non-rt-n8p.md) records every N8o quality, flicker, timing and memory comparison.
Surface beats Unreal VSM on every recorded quality measure and fixes S1 pose 1's
2-leak miss, while remaining below Exact on four quarry measures. Static shadow work
is 0 ms; camera motion is 1.043-1.046 ms against Exact's 1.493-1.503 ms, still above
VSM's estimated ~0.5 ms. Non-RT reserved memory is 3,503.7 MiB against Exact's 3,439.7
and VSM's 3,912. This is a default decision within the measured shadow subset, not M6
acceptance or film-quality parity. A planned refinement will reuse a cell only when
it never scores worse than the pixel's own walk, bringing the default toward Exact's quality.

## Execution and verification

1. Ground material/normal/texture, render-pass, virtual-geometry and viewer interfaces. Reuse the M4 evidence and scan artifacts. Read industry-rendering-costs.md; Ara designs are historical evidence, not constraints or inherited implementations.
2. Compare two small structural designs using configured Sol/Astra perspectives. Decide caller usage, typed light/material data, HDR targets, shadow ownership and shader reload behavior before implementation. Keep the library independent of the viewer.
3. Land a lit opaque fixture and the quarry with a small typed lighting API, correct linear/HDR shading and exposure. Verify before expanding light/material features.
4. Add shadows as a separately measurable unit, then material/scene stress relevant to the changed ownership. Keep invalid reload recovery and bounded in-flight lifetimes intact.
5. Prepare the same content in Unreal and run focused paired image/performance comparisons. Fix identified deficiencies. Record rejected attempts and failed gates. Run broader verification once for retained integrated changes, not after every small edit.
6. Review, push validated checkpoints as Snowy7 and report actual results. Do not auto-advance to GI or other later milestones.

Throughput checkpoint:

- Blocking first steps: current API grounding, Unreal build identification and baseline identities; no speculative rendering architecture before those are known.
- Independent workstreams: M4 streaming and resize each own a separate tool-created worktree; M5-M6 owns the main checkout. Root owns integration, acceptance, main documentation and pushes.
- Shared mutable state: every worker uses separate source/evidence/build outputs. Only one owner may run GPU/window measurements or heavy builds at a time. Root allocates these windows explicitly; no foreign application shutdowns. Source-mutating dev-loop helpers run only in their owner's checkout.
- Smallest safe decomposition: lighting/shader/HDR integration has one implementation owner because its layouts, pipelines and callers change together. Shadow integration follows that contract. Independent comparison tooling may be separated once its inputs are fixed.

## Parallel M4 work

Streaming baseline is the final 338-pose quarry path with 4K textures, 4x MSAA, 16x filtering and the responsive admission policy. Existing optimized motion CPU p95 is approximately 38 ms; the target remains at most 8 ms for advance plus drawing, with unchanged quality, residency budgets and recovery. Start with the measured selector/bookkeeping bottleneck. Preserve the existing full-resident path and immutable assets. Reuse baseline evidence, then use a short paired sample to reject weak ideas before broad checks. Do not impose a large minimum iteration count contrary to Islam's short development-cycle requirement.

Resize begins with diagnosis. The historical 4.93 s configure stall is unexplained; recent draining and recovery checks pass. The [verified opt-in probes](resize-followup.md) now separate application work, surface configuration and acquisition/presentation, with a bounded native control/probe pair and inspected recovery images. No recurrence occurred, so internal driver/desktop attribution remains inconclusive. No speculative synchronization fix was made. The first streaming hillclimb likewise retained no change: correctness passed, but gameplay-contended paired timings did not establish a reliable gain. Its candidate and evidence are preserved locally; the streaming target remains unmet.

The next [streaming attempt](streaming-owner-witness.md) is retained: an unavailable-owner proof avoids publication transactions that cannot succeed. Frozen selector calls improve with identical outputs; the short live screen remains above the 8 ms target. It is not a new measurement of the full canonical path. Root integrated it with M5-A and the [CPU shadow planner](m6-shadow-plan.md); 14 selector, 18 runtime and 13 shadow-plan CPU tests, regular/virtual lit image equality with active validation, affected Clippy and formatting pass. Evidence is `measurements/m5-m6/integration-457f291/`. The planner does not yet render shadows.

Next-unit throughput checkpoint:

- Blocking first steps: M5-A is pushed and accepted; the selector/planner integration checks pass. Existing how/design/Opus decisions govern the next units; no new broad design round.
- Independent workstreams: one main-checkout owner integrates GPU shadows; an isolated worker prepares tangent and normal-map cooking; the existing UE worker owns only calibration scripts and its private project.
- Shared mutable state: shadow renderer/WGSL/layout changes have one owner. Normal-map preparation stays in its own checkout and does not migrate the runtime layout yet. Root allocates GPU and heavy builds to one worker at a time: first the bounded UE calibration run while shadow implementation proceeds, then shadow verification. Cooker CPU checks use a separate cached target.
- Smallest safe decomposition: finish one shadowed fixture and quarry with explicit coverage/quality limits, and separately return tested tangent preparation. Root owns integration, comparisons, closing documentation and pushes. Shadow-demand refinement and the normal-map runtime adapter follow these concrete seams.

The decision trail is `milestones-5-6-decisions.tsv`; local per-attempt hillclimb logs stay in each worktree's ignored evidence directory. Hardware baseline remains this Windows PC with RTX 4070 SUPER/NVIDIA 617.14. Recheck foreground-game state and contention before interactive or GPU work.
