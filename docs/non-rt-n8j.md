# N8j: independent virtual-shadow configuration history

2026-11-07 UTC, base `15fd73f`, branch `t3/n8j-order-state`.

## Cause

The N8h 513-pixel difference is carried **rolling refresh history**, not walk-record
contents or a device/pipeline cache. The quarry harness captures its counts after
40 static frames, 16 turning-sun frames and just one sun-restored frame. That last
frame still contains counts from different sun directions. Its image depends on
the starting refresh phase, even though the restored camera and light agree.

Two omissions made independent configurations share that phase:

- The harness assigned settings but never explicitly restarted history. Assigning
  the same settings intentionally does nothing, preserving pages, hysteresis
  anchors and counts. A configuration matching the preceding restored settings
  therefore started with an extra frame of history rather than a fresh frame.
- After initialization, `set_virtual_shadow_settings` updated `VirtualState` but
  left `Renderer::virtual_settings` stale. The caster identity check used that
  stale copy for its dense lookup and restore. In N8h's `final-order`, the initial
  records-off settings were restored even after the all/source configurations;
  `d-off` consequently made no settings transition. The first restored frame had
  history index 56 in fresh configurations and 57 in this reused configuration.

A real settings change already allocated fresh page storage and forced a full
lookup. The purported surviving settings-change reset was actually a same-value
assignment following the stale restore. Continued frames are allowed to retain
the documented temporal lag; an independent replay needs an explicit boundary.

## Fix

- Keep the renderer's saved settings current after every successful setter call.
- Share a reset between settings changes and `reset_virtual_shadow_cache()`.
  It discards the cache (including sun/window/height/model anchors), page resources,
  request key, rolling flag, frame index, schedule and caster-only flag. The next
  frame requests fresh pages and looks up every pixel. In-flight resources retain
  their normal submission-based retirement; geometry records and pipelines stay warm.
- Reset at the start of every quarry bench configuration, including identical ones.
- Add one ignored, leased GPU test, `gpu_virtual_configuration_reset`: a 320x192
  fixture compares complete receiver/count buffers against its initial fresh
  sequence after changed settings and explicit same-settings resets, with caster
  and light changes between configurations. It also checks the saved settings.
- `n1c-batch.py --hash-only` keeps count/hit SHA-256 summaries without HDR/world dumps.

No shader or shading contract changed. Same-value settings calls retain history
in normal rendering. This does not make individual frames of an intentionally
unreset temporal sequence independent of its history.

## Evidence and historical scope

Evidence is under `E:/renderlab-work/n8j/`, with reservation/release lines in
`E:/renderlab-work/gpu-schedule.md`. The original small probe changes 2 of 61,440
pixels on both `15fd73f` and `79e8ba2` (N8f, before N8g/N8h). The latter was built
from a Git archive with only that probe appended. The stale setter is also present
at `bb1e8df` (N8d); rolling refresh itself arrived in N8e. No first-bad-commit claim
is made for the earlier history.

N8h's fresh-process/first-configuration quality scores and S1-S3 evidence remain
applicable. Later configurations' post-turn snapshots can contain a different
temporal phase, so they cannot establish byte identity without reset. The stale
restore could also substitute default settings in dense caster checks of
non-default variants. No new performance superiority or replacement timing claim
is made here.

Final batch (`runs/verify3.log`, `verification-steps.json`, `build-identity.json`):

- Original full-quarry replay: `a-off`, `b-all`, `c-src` hash to `05e50a1d8ad17426…`;
  `d-off` hashes to `db9e1a6068e1721f…`, with exactly **513** differing pixels.
  See `original-quarry-summary.json`.
- Fixed: all seven `final-order` variants at both 1920x1080 poses match fresh
  controls exactly, including receiver hashes. Refresh 16 retains N8h's fresh
  counts hashes `05e50a1d8ad17426…` / `bed48bf7099343bb…`; refresh 8 matches a
  separate fresh process at both poses (`fixed-hashes.json`, `fixed-fresh8/`).
  Every caster rolled-versus-dense check reports zero differing pixels.
- `cargo test -p renderlab --lib -j 2`: exit 0, **106 passed, 108 ignored**.
  The new ignored GPU test was also run explicitly and passed all five sequences.
  Fmt passes. Clippy passes on renderlab's all-targets with `-D warnings` and the
  existing baseline allowances `-A clippy::nonminimal_bool -A clippy::items_after_test_module`.
- Visually inspected `fixture/reset-Off-true.png`; all four fixture PNGs are
  byte-identical (`3f77f0e316283420…`). Exact full count/hit comparisons are the
  acceptance signal; no visual-quality or timing superiority is inferred.

RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, development/test profile. The fixture
had validation enabled; quarry replays used the original timing manifests with
validation off, 0.35 px LOD, lift 1, software extent 32, turn 16 and move 48.
The stock guarded launcher held the shared lock, wrote reservations/releases,
opened no windows and saw no game. Available memory stayed above 13,222 MiB in
the final batch, under the required 4 GiB start / 2 GiB abort guard.

Reading/diagnosis occupied approximately 00:11-00:17 UTC; reset/test/harness edits
00:17-00:22, overlapping the historical replay. Verification was separate:
initial cold-target compile 176 s, historical compile/replay 98 s, and final batch
227 s (retained rebuild 39 s, fmt/clippy/lib 32 s, fixture 2 s, original quarry
48 s, fixed quarry 77 s, fresh refresh-8 control 25 s). All leases, including two
failed helper-compilation attempts, total 531 s. Raw dumps over 5 MB, staged
binaries and this worker's Cargo target were deleted after summarizing.

To repeat the quarry comparison, wrap `scripts/n1c-batch.py <test-binary> <out>
quarry-no-rt --timing --lod 0.35 --lift 1 --poses 0 1 --turn 16 --move-frames 48
--stage-repeats 0 --hash-only` in `scripts/n8-run.py --fast-check` with the original
`final-order` variants in `N8E_VARIANTS`. Exact manifests and the batched driver
are retained in the evidence directory. `docs/milestones-5-6-decisions.tsv` is unchanged.
