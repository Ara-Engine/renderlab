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

## First GPU result and bounded correction

The first candidate passes the frozen contact image gates at cold start and static frame eight, but is rejected by the work gate. The diagnostic observes 6,004,024 queries for 49,601 final samples at frame eight. Receiver-first draw order shades hidden receivers whose history belongs to the foreground blocker; they reset and trace before being overwritten. Storage atomics can affect early depth behavior, so these are diagnostic invocation counts, not production throughput. No 4x, motion, reveal, ordinary cost or quarry gate ran.

Root preserved and checked 227 evidence hashes at `D:/Coding/renderlab-work/m6-shadow-history/`, inspected the complete static sheet, and recomputed the query total. All 193 single-invocation surface resets lie in the leftmost pixel column. This points to history reprojection at the image boundary, not an actual changed surface. The candidate remains rejected and the accepted source is unchanged.

The next bounded correction must prevent hidden receivers from executing expensive visibility work. A current-frame depth-only pass may supply a visibility depth texture sampled before tracing. Keep a separate ordinary main depth attachment with strict Greater so exact equal-depth draw order is preserved; changing shading to Equal alone would allow later tied fragments to overwrite the first winner. Validate identical depth/sample coverage between the two passes rather than adding an arbitrary epsilon. Count and price the additional geometry pass and memory.

For an exactly unchanged raster/shadow state, fetch the completed previous visibility at the same pixel and sample instead of reprojecting it or tracing fresh rays. This is deterministic cache reuse, not a claim that a few equal ray results prove visibility. Camera, geometry, transform/order, light, extent, bias, shader, mode and target-size changes must disable that shortcut. It preserves the already-tested finite estimate; it does not improve its error. Static cost must be labeled cached, with first-frame and moving costs reported separately. This also avoids the logical conflict between always tracing four fresh rays, allowing contradiction resets, and requiring a strict average of at most four.

This correction is not permission to loosen the image or moving-ray gates. It must first clear the same static fixture, then the originally planned actual-GPU motion/reveal sequence. Stop at the next decisive failure; no quarry expansion follows a static-only pass.

## Direct quarry cost decision after the failed work forecast

The [contact sequence](m6-shadow-history-screen.md) now passes, while the [sparse quarry screen](m6-history-quarry-screen.md) fails its predeclared eight-query forecast. That failure remains recorded. A subsequent focused Opus judgment traced eight to an engineering proxy, not the user's frame-time contract. Its CPU check also rejected replacing narrow resets with a pixel-density keyframe: even an exact visibility keyframe resampled once fails the original contact image gates at most tested subpixel offsets.

Root therefore authorizes one bounded direct cost experiment of the unchanged history mechanism on the actual quarry. Use the existing pipelined ordinary-cost tool, 1920x1080, the registered static and 1 cm motion poses, radius 0.04 rad, and matching content/lighting/environment. Compare hard 1x, soft 1x, and a diagnostic soft control with narrow resets disabled, then hard 4x in the same session. The diagnostic control deliberately changes quality and must never be presented as a candidate image. Keep cold preparation separate and inspect complete real soft images. Two reversed-order repetitions suffice; at most one repeat may resolve contradictory branches. Stop on validation/device failure or a soft cold frame exceeding two seconds. No 4x soft cold start is authorized here.

If moving soft 1x costs no more than the hard 4x anchor and shows no gross image defect, proceed to focused lifecycle checks; this does not establish 4x quality or Unreal parity. If only the narrow-reset control removes the excess, investigate the reset execution path. If the control is also slower than the anchor, reject this implementation's current floor before tuning it. Cost differences include changed history and control flow; they do not alone prove warp divergence or rule out every ray-based design. The hard 4x anchor is a practical screen, not a quality-equivalent comparison or the missing Unreal denominator.

This is a new measured-cost decision, not a retroactive pass of the failed work forecast. Preserve all old thresholds, failures and sources. Root's corrections to the judgment prevent treating a quality-changing control as proof of a single hardware cause. The judgment and its 5.08-second CPU keyframe check are preserved at `D:/Coding/renderlab-work/m6-history-next-decision/`.
