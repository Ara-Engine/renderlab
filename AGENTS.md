# Renderlab

Read `docs/brief.md` and `docs/project-handoff.md` before implementation. Read `docs/research/industry-rendering-costs.md` before selecting rendering algorithms or making comparative performance claims.

- This is an independent public-facing Rust renderer. Its own users and development experience drive the design; Ara integration is deferred.
- Keep dependencies and compilation boundaries deliberate. Measure shader edit-to-image, incremental Rust edit-to-running-viewer, and startup-to-image separately from frame time.
- Treat verification latency as development latency. Reject experiments with a short targeted reproducer and paired sample first. Reuse valid baseline evidence; reserve broader checks for retained changes and specific unresolved risks. Scale benchmark repetitions to observed variance and the claim, rather than running a fixed large campaign for every edit. Report implementation and verification time separately during optimization work.
- Establish a small working viewer and shader reload loop before expanding rendering features. Keep the last working pipeline on shader errors.
- Verify the complete image, including silhouettes, shadows, transparency and motion. Region-average lighting scores alone do not certify visual quality.
- Keep fast development builds distinct from optimized performance measurements. Record hardware, scene, resolution, quality, build identity and contention with timings.
- Prefer simple typed APIs and owned resources. Add focused tests at meaningful boundaries; run checks on affected crates.
- Questions are read-only. When asked for advice or feasibility, answer before editing.
- Use the configured pstack workflow with GPT-6.1 Sol and Opus 5.5 only, at large (xhigh) reasoning. Sol handles routine implementation, fixes and verification; Opus handles selected hard implementations, hillclimbs and consolidated reviews. Give parallel workers separate file ownership. This 2026-11-04 choice supersedes older Astra assignments.
- Historical Ara plans are evidence, not inherited architectural decisions. Record proposals and measured reasons before choosing an approach.
- Keep captures and generated measurements in ignored directories. Do not import private assets or third-party source without checking redistribution rights and preserving attribution.
- Public repository: https://github.com/Ara-Engine/renderlab. The project license remains undecided; Ara integration is still deferred.
- Commit and push validated checkpoints to the configured remote routinely; the user has authorized this. Report the pushed SHA and any failed push. Never force-push or include another agent's uncommitted work.
- M0-M4 are complete, with independent Opus acceptance and verified corrections for M2-M4. Evidence and limits are in docs/milestone-2.md, docs/milestone-3.md and docs/m4-final-verification.md. Islam now authorizes the real-scan follow-through in docs/scan-validation-plan.md, including implementation, conversion and unattended GPU/window verification. Use the configured pstack workflow. Performance or quality superiority still requires measured comparative evidence.
- Current authorization extends through M5-M6 in this checkout, alongside isolated M4 streaming-performance and resize-stall hillclimbs. Read docs/milestones-5-6.md before this work. The acceptance floor is Unreal-equivalent visual fidelity with at least 2x performance together on matched workloads; the ambition is higher quality. Record misses explicitly rather than reducing quality to pass the speed gate.
- Protect Islam's foreground game. While he is playing, keep work noninteractive: no viewer/window launches, focus changes, input automation, or desktop/game captures. Defer GPU tests/stress/benchmarks and heavy measurement builds until he says he is done playing. Continue coding and light CPU checks with two Cargo jobs.
