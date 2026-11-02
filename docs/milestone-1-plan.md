# Milestone 1 execution plan

Status: complete. Implementation, GPU/window acceptance, development measurements, and Opus review corrections are verified. See [the M1 evidence record](milestone-1.md) for scope and limitations.

Islam explicitly ended the gameplay hold. Final GPU and window runs captured Khronos validation diagnostics with zero errors under a process-local loader configuration excluding broken implicit overlay registrations. No machine settings changed. Stop here until Islam authorizes the next layer.

## Scope and completion

Two existing crates, a mesh shader, focused GPU tests, viewer interaction checks, and measurement scripts. The library remains independent of winit. Completion means every acceptance condition in project-handoff.md has evidence from the actual build. Unverified behavior is not a pass.

## Pstack workflow

The installed poteto-mode routes this cross-cutting task to figure-it-out. Apply how for grounding, architect/arena for alternative resource designs, and show-me-your-work for a committed decision log. Use benchmark-checklist before accepting timings. Apply deslop before each commit and independent review before declaring completion.

Confirmed model roles use GPT-6.1 Sol for routine implementation and exploration, Astra for hardest implementation, and Opus 5.5 for hillclimb and selected judgment/review. Islam trusts Opus more but wants to conserve its usage. Sol and Astra generate design candidates; Opus provides one consolidated design review and a final acceptance/trail review. Avoid routing routine documentation or repeated small fixes to Opus. Budget is large (xhigh). Exact model IDs and provider-specific effort options are in the user's pstack-models.mdc; delegates receive them explicitly because T3 does not automatically apply Cursor rules.

## Units and gates

1. Preserve M0 evidence, correct numbering, ground the existing library/viewer, and compare resource ownership designs. Record a small chosen interface and verification contract before implementation.
2. Implement the GPU foundation with real persistent meshes, transforms, depth, owned targets and transactional pipelines. Prove depth with order-independent overlapping geometry, invalid reload with full-image equality, and safe replacement with GPU work in flight. Check and push the coherent library/viewer checkpoint.
3. Add asynchronous GPU timing and honest CPU/upload/resource counters. Exercise static and moving workloads and bounded replacement loops. Inspect captures and real window resize/minimize/reload behavior. Repair any failed predicate before accepting results.
4. Run final focused checks, repeat development timings without concurrent source edits, inspect evidence, obtain independent review, document limitations, and push the final checkpoint. Stop at M1.

## Throughput and ownership

The root owns repository edits and integration until explicitly assigning paths. Grounding and design agents are read-only or write isolated proposal artifacts. Implementation can split only at agreed module boundaries. No concurrent GPU measurements, source-mutating timing runs, or commits. Cargo stays at two jobs. Baseline artifacts remain ignored under captures/ and measurements/.

The highest-risk unknown is measuring GPU work asynchronously while keeping replacement lifetimes and accounting correct. Resolve it before adding viewer features. Rigor comes from targeted adversarial GPU tests, repeated replacements, real interaction, and independent review rather than broad unrelated test suites.
