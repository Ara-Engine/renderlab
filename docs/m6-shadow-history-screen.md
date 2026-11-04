# M6 shadow history: contact screen

The contact screen passed, but the subsequent [quarry work forecast](m6-history-quarry-screen.md) failed its predeclared feasibility screen. The experimental source is preserved privately and the accepted renderer is restored. M6 remains open. This screen extends [the history plan](m6-softness-history-plan.md); it does not establish quarry quality, moving-scene performance or Unreal parity.

## Result

The frozen 257x193 contact sequence passes all 78 captured frames at 1x and 4x, including sixteen consecutive camera steps, a blocker reveal, a camera reset and a light change. Every frame passes the original image thresholds, including frames where the runner only required motion checks. Depth/sample contracts and hidden-query counters also pass. Both Vulkan validation summaries have active Khronos validation and zero errors.

Exact-static frames reuse the previous finite estimate without fresh queries. This does not improve that estimate's accuracy. Cold starts, light changes and explicit resets still cost 256 rays per covered sample. Moving frame 20 averages 6.942 queries/sample at 1x and 7.197 at 4x. The largest moving-frame averages are 7.922 and 7.607 respectively, below the unchanged eight-query limit.

The candidate uses a separate current-frame visibility depth pass and preserves the original strict Greater main depth pass. History contains visibility, count, closest observed blocker distance and geometric guides per sample. Its four packed history textures plus visibility depth cost 36 bytes/sample, about 284.77 MiB at 1920x1080 and 4x, excluding other renderer resources. That full-size allocation was not exercised by this contact screen.

## Corrections and rejected states

The preceding candidate eliminated hidden queries and passed static caching, but failed moving frame 20 at 29.54 queries/sample. Two reconstruction faults inflated resets. Directly summing rounded cubic weights turned an exactly fully-lit field slightly fractional. Rejecting the entire filter whenever one outer tap crossed an image or surface boundary also discarded usable history.

The correction accumulates differences from a compatible nearest sample, preserving constant fields exactly. It excludes incompatible outer taps and normalizes the remaining weights. A valid nearest sample remains mandatory; insufficient signed weight rejects reuse. Surface tolerances and image thresholds are unchanged. This is a tested reconstruction rule, not proof against every possible disocclusion.

The first corrected run passed quality and work through 4x frame 20, then failed a sample-position contract at frame 21. Reprojecting interpolated world position introduced one stored unit of error. The shader now records fragment-position coordinates directly. The failed run is preserved.

Even with zero new queries, the general tracing shader missed the cached-frame cost screen. A cost-only shader with traversal removed was much cheaper. That control fabricates cold visibility and is not an image candidate. The retained experiment compiles a separate cached shader and chooses it only after the existing exact-state comparison succeeds. The cold/moving implementation remains separate. Full contact images still pass after this specialization.

## Timing method and limits

Timing uses the optimized test executable, ordinary lit shading, no diagnostic atomics/readbacks and validation disabled. Each hard/soft/soft/hard block warms the same submitted workload for 250 ms, then records twelve GPU intervals. The measured interval includes the extra visibility depth pass and mesh shading. Cached soft medians are 0.00973-0.01024 ms at 1x and 0.02150 ms at 4x, below the predeclared 0.145 ms steady-state screen.

These are tiny-fixture cached timings, not moving-shadow or engine throughput. The RTX 4070 SUPER/NVIDIA 617.14 began the sampled sessions at 9% utilization, 3302 MiB VRAM and a reported 210 MHz graphics clock. Background contention was not controlled. Early hard blocks and cold-state runs vary substantially as clocks change; all remain in the evidence. No hard-versus-soft speedup claim is made. Cold preparation, light resets and large moving blockers remain expensive and unpriced on the quarry.

Root's initial WGSL-only correction needed no Rust build. Recorded native sessions took 24.06 and 31.31 seconds. The specialization build took 25.99 seconds; its complete image run took 37.46 seconds. A final timing-helper build took 22.87 seconds, and the warmed cost process took 8.99 seconds including setup. These are separate observations, not an isolated coding-time measurement. The final CPU binary passes 61 CPU tests; affected Clippy and formatting pass after removing one redundant integer conversion.

## Evidence and next gate

Root evidence is in `.refs/m6-history-reconstruction/`. `root-summary.json`, per-frame JSON/raw images, `run-warm-cost/cost.json`, command manifests, source snapshots and preserved executables identify the results. The final Clippy-only conversion removal is behaviorally equivalent to the executed source; the source snapshots explicitly retain that difference. The last executed shader is SHA-256 `26e11d8026d22008ce6b0975718dcfaef7c28fe2f90e908f9952e7d366b82ad3`.

Root inspected complete contact, reveal and changed-light sheets. The subsequent sparse quarry screen observed 31 narrow fractional receivers among 1,298 selected receivers and stopped. Its forecast is not a measured moving-frame cost. Broader surface disocclusions and reload/resize lifetime behavior with history enabled remain unverified. An omitted new field in the shared lighting parser also prevents integration; restoring the accepted source removes that incompatibility. The next decision must distinguish the work proxy from actual cost without erasing the failed screen. Do not launch an unconditional 256-ray 1080p 4x quarry cold start.
