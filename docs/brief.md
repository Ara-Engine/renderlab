# Project brief

## User intent

Islam wants a separate, public Rust rendering library/engine on this PC, useful independently of Ara. It may eventually be reused in Ara, but integration is explicitly not a current priority.

The immediate motivation is unacceptable iteration speed in the large engine. Treat development latency as a product feature: immediate parameter changes, shader edits without Rust rebuilds, short incremental Rust rebuilds, and prepared scene assets reused across launches.

The public name is undecided; Renderlab is a reversible working name. The repository is local, with no remote or selected public license. This bootstrap makes no rendering or performance claims.

## First working milestone

Start from this small repository. Select and explain a minimal backend/tooling approach based on dependency weight, shader iteration, validation and intended renderer capabilities. Reuse selected proven infrastructure only where it actually improves this project's goals; do not copy the engine wholesale.

Implement a small persistent viewer and a visible scene. Establish editable camera/material/light parameters and a shader reload path that retains the last valid pipeline on compile failure. Exercise the same library code from the viewer. Avoid speculative APIs for engine integrations.

Measure cold compile, no-op compile, a representative incremental Rust edit, shader edit-to-visible-image and startup-to-image. Record actual timings and circumstances; establish targets from evidence rather than promising instantaneous compilation. Separate optimized GPU timing from the quick development loop. Build/run of this first milestone is part of the authorized continuation.

Before expanding to GI/reflections, use the industry research to explain what work existing renderers avoid, share, cache or amortize. Assess geometry/material costs as well as lighting. Preserve the user's image-quality concerns: clean model silhouettes, correct shadow softness and darkness, smooth shadow edges and stability in motion.

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
