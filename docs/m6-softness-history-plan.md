# Bounded shadow-history experiment

The two rejected softness screens leave the accepted hard-shadow renderer unchanged. A focused Opus judgment recommends per-sample visibility history with fresh sampling where camera motion cannot preserve a narrow penumbra. This is a proposed M6 shadow mechanism, not general temporal antialiasing or a completed quality result.

The CPU toy passes its contact sequence only after cubic history reconstruction, fresh estimates for narrow penumbrae during camera movement, and explicit invalidation of the shadow footprint of moved blockers. Plain history fails continuous camera and light motion. The toy also uses authored blocker distance, so its result does not establish that real nearest-hit observations provide sufficient information. Evidence and exact source are in `.refs/m6-softness-next/` and preserved at `D:/Coding/renderlab-work/m6-softness-next/`.

## Selected first unit

Implement the small contact fixture at 1x and 4x using actual hardware queries and per-sample history. Keep the original continuous-disk reference and quality thresholds. Start with four fresh directions per sample on reusable history and 256 on reset, retaining the tested sequence and narrow-penumbra threshold. These are experiment settings, not a general quality guarantee. The shader must obtain blocker distance from actual closest-hit queries; reference geometry or analytical visibility cannot guide it.

Reproject using actual previous sample locations, main-camera reversed depth and geometric surface checks. Reject invalid history rather than treating uncovered or mismatched surfaces as valid. Record sample counts, reset causes, history memory and GPU work separately from the lit shader's other costs. Current-frame output remains governed by ordinary depth testing. Busy admission and failed reload must not advance history state.

Root adds these conservative corrections to the proposed toy protocol:

- Every light-direction or extent change invalidates history. A single-step success does not justify retaining history under the continuous rotation that failed the toy. Price the reset separately; no efficient moving-light claim follows.
- If moved-instance invalidation cannot represent all changes, invalidate the whole history. A shorter history is not a correctness fallback for a missed blocker reveal.
- Cold start uses the fresh estimate. Do not classify a sample as certainly lit merely because a small pilot sample found no blocker.
- Shader installation, resize, camera cuts and history incompatibility reset safely. Failed shader reload keeps the installed renderer; no speculative candidate state is published.

## Stop rules

The first test covers cold start, eight static frames, a one-millimetre camera step followed by fifteen continuous steps, a blocker reveal, and a light change. Each checked frame must pass the original static gates against its own reference, and consecutive motion must pass the original motion thresholds. No single-step result substitutes for continuous motion. Preserve all failures.

Stop before a quarry campaign if the fixture fails image quality, exceeds the proposed average four-ray steady or eight-ray moving-camera budget, or misses the short steady-state cost screen. Count actual queries, including resets and diagnostic overhead where present. Do not infer runtime cost solely from average rays: divergent work and closest-hit traversal must be measured.

Only if the small fixture passes, a bounded quarry diagnostic may estimate the narrow-penumbra/reset fraction. Do not launch an unconditional 256-ray-per-sample 1080p 4x quarry cold start before pricing that work. A high reset fraction can reject the mechanism without a long render.

This experiment does not establish efficient moving lights or large moving blockers, arbitrary virtual geometry, shadowed environment lighting, general scene motion, native interaction, or Unreal fidelity/performance parity. Its purpose is to decide whether the actual GPU information and history ownership can support the first useful softness unit without the earlier banding and contact blur.
