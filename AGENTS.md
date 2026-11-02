# Renderlab

Read `docs/brief.md` and `docs/project-handoff.md` before implementation. Read `docs/research/industry-rendering-costs.md` before selecting rendering algorithms or making comparative performance claims.

- This is an independent public-facing Rust renderer. Its own users and development experience drive the design; Ara integration is deferred.
- Keep dependencies and compilation boundaries deliberate. Measure shader edit-to-image, incremental Rust edit-to-running-viewer, and startup-to-image separately from frame time.
- Establish a small working viewer and shader reload loop before expanding rendering features. Keep the last working pipeline on shader errors.
- Verify the complete image, including silhouettes, shadows, transparency and motion. Region-average lighting scores alone do not certify visual quality.
- Keep fast development builds distinct from optimized performance measurements. Record hardware, scene, resolution, quality, build identity and contention with timings.
- Prefer simple typed APIs and owned resources. Add focused tests at meaningful boundaries; run checks on affected crates.
- Questions are read-only. When asked for advice or feasibility, answer before editing.
- Use the configured pstack workflow for M2-M4. Islam permits GPT-6.1 Sol, Astra, and Opus 5.5, with a large (xhigh) reasoning budget. Give parallel workers separate file ownership.
- Historical Ara plans are evidence, not inherited architectural decisions. Record proposals and measured reasons before choosing an approach.
- Keep captures and generated measurements in ignored directories. Do not import private assets or third-party source without checking redistribution rights and preserving attribution.
- Public repository: https://github.com/Ara-Engine/renderlab. The project license remains undecided; Ara integration is still deferred.
- Commit and push validated checkpoints to the configured remote routinely; the user has authorized this. Report the pushed SHA and any failed push. Never force-push or include another agent's uncommitted work.
- Current sequencing: M0 and M1 are complete. Islam explicitly authorizes implementation and verification of M2-M4, including investigation of improvements beyond Nanite for M4. Follow docs/milestones-2-4.md; push validated milestones in order and stop after M4. Performance or quality superiority requires measured evidence.
- M2 is complete with independent Opus acceptance; evidence and carried limits are in docs/milestone-2.md. Use docs/m2-m4-design.md for the selected interfaces and M3/M4 proof obligations. Strengthen the near/portrait and within-collection tie fixtures in the first M3 GPU run; measure basic antialiasing during M3/M4.
- Protect Islam's foreground game. While he is playing, keep work noninteractive: no viewer/window launches, focus changes, input automation, or desktop/game captures. Defer GPU tests/stress/benchmarks and heavy measurement builds until he says he is done playing. Continue coding and light CPU checks with two Cargo jobs.
