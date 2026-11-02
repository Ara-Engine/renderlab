# Renderlab

Read `docs/brief.md` before implementation. Read `docs/research/industry-rendering-costs.md` before selecting rendering algorithms or making comparative performance claims.

- This is an independent public-facing Rust renderer. Its own users and development experience drive the design; Ara integration is deferred.
- Keep dependencies and compilation boundaries deliberate. Measure shader edit-to-image, incremental Rust edit-to-running-viewer, and startup-to-image separately from frame time.
- Establish a small working viewer and shader reload loop before expanding rendering features. Keep the last working pipeline on shader errors.
- Verify the complete image, including silhouettes, shadows, transparency and motion. Region-average lighting scores alone do not certify visual quality.
- Keep fast development builds distinct from optimized performance measurements. Record hardware, scene, resolution, quality, build identity and contention with timings.
- Prefer simple typed APIs and owned resources. Add focused tests at meaningful boundaries; run checks on affected crates.
- Questions are read-only. When asked for advice or feasibility, answer before editing.
- Use Astra or Opus for implementation if delegation is explicitly authorized. Ordinary work should stay with one agent.
- Historical Ara plans are evidence, not inherited architectural decisions. Record proposals and measured reasons before choosing an approach.
- Keep captures and generated measurements in ignored directories. Do not import private assets or third-party source without checking redistribution rights and preserving attribution.
- Public repository: https://github.com/Ara-Engine/renderlab. The project license remains undecided; Ara integration is still deferred.
- Commit and push validated checkpoints to the configured remote routinely; the user has authorized this. Report the pushed SHA and any failed push. Never force-push or include another agent's uncommitted work.
- Current sequencing: milestone 1 is complete. Discuss rendering foundations with Islam one layer at a time before choosing the next implementation milestone; do not automatically advance into mesh loading or advanced lighting.
