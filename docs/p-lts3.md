# P-LTS3: exact local-light structural screens

2026-11-09, `t3/p-lts3`, baseline `17b4134c31e361442bf5441059823d071c047964`.
**No renderer optimization is promoted. The 3.5 ms target remains unmet.**
This is an evidence/handoff checkpoint. Production sources are unchanged from
main. No quiet before/after pair was obtained and the complete acceptance matrix
could not finish under host contention; do not present these experiments as ready.

## Current-main profile

Warm non-RT unshadowed ARA1 game-lights at 1920x1080, RTX 4070 SUPER,
NVIDIA 617.14, Windows/Vulkan, optimized release. Nsight GPU Trace starts after
1200 submits, collects four submits (50 ms maximum), with explicit WGPU_DEBUG=1
and experimental ray-query acknowledgment. No screenshot, UI or clock change.
This attribution run overlaps the owned two-job test build; times are diagnostic.

Primary shading averages 3.544 ms; vsc_claim 1.155 ms; vsc_missed 1.023 ms.
Sample shares: light_sample 22.92%, rs_trace 13.02%, unit_normal 10.47%,
shade 10.45%, rl_light_index 7.54%, rs_box 6.82%, exposed_product 6.27%,
transparent_sun_transmission 3.50%, brdf 2.80%. Sample shares are not additive
function times. No usable achieved-occupancy counter was recovered.

Evidence root: `D:/Coding/renderlab-work/p-lts3`. `profile-main-nonrt` retains
full composed source, catalog, per-line hotspots, function shares and pipeline
summaries. The summarized 52,451,645-byte raw trace was removed; its digest is in
`summarized-raw-traces.json`.

## Experiments (all unretained)

- Archived P-LTS2 BRDF preparation hoist: all four 1080p ARA1 HDR/PNG pairs match.
  The contended screen establishes no consistent gain. `runs/brdf-screen`.
- A shares light_sample's identical scaled vector between normalization and
  distance and branches around discarded transparent-sun work for other lights.
  All four 1080p HDR/PNG pairs match. No consistent speedup was established.
  `runs/a-screen2`; source `candidate-api.wgsl`. An earlier prototype introduced
  CRLF into an existing LF-sensitive reflection composition seam and panicked
  before a candidate image. LF was restored before the completed screen.
- B memoizes the receiver-plane tangent only for bit-identical geometric normals,
  handling changes of receiver within an invocation. The non-RT shadowed 1080p
  HDR/PNG pair matches. The unstable 15.81/30.79 ms baseline/candidate tail
  demonstrates no gain and does not establish a regression. `runs/b-screen`;
  source `b-cluster-api.wgsl`.
- C adds conservative padded-sphere rejection by the froxel's four side planes
  after the existing AABB test; its CPU cluster oracle matches the new test.
  Retained light order, the 128-light map budget, every contributing RT shadow,
  LC2 softness and source samples remain unchanged. `c-ready.patch` archives the
  two-file experiment; `bin/c-*` and `bin/base-*` preserve independent binaries.
  Its contended times are excluded; there is no speedup or regression clearance.

No public helper signature/meaning, material model, receiver/hit record or shading
dispatch is changed in the final branch. No patented technique was introduced.
D's uniform light copy/index cache was not repeated; P-LTS2 already screened it.

## Completed C quality prefix

- **199 lib tests passed, zero failed, 164 ignored**, including shader composition
  and the unchanged historical TP1 digest (`runs/c-small/lib-None-candidate`).
- Original 960x540 shadowed/unshadowed, RT/non-RT: all four HDR and PNG pairs match
  (`runs/c-quality-quality`).
- 961x541 odd-edge/clear-boundary, both paths: HDR/PNG pairs match
  (`runs/c-quality-edge`).
- 1080p: shadowed on both paths and unshadowed RT match HDR/PNG witnesses, including
  the appended tail. The final unshadowed non-RT pair was interrupted
  (`runs/c-screen`). A and the BRDF hoist independently completed all four pairs.
- LC1-3 point/spot hard/soft, leakage and slot-fade controls: all **24 candidate
  PNGs** match on both paths (`runs/c-small`).

Inspected `inspection/c-completed.png`: eight point/spot hard/soft presentation
views and the four 960p game-light stills. Their silhouettes, shadow edges and
soft penumbrae preserve the baseline; hashes establish the completed witnesses'
identity. This is not complete quarry, transparency, motion or reference acceptance.

**Unresolved:** MP1, courtyard, full-quarry pair, the 128/129 boundary, the fourth
C 1080p pair and all quiet before/after timings. No final quality certification is
claimed for C. Production quality is preserved by leaving every experiment out.

## Host limits and handoff

`scripts/p-lts3-run.py` streams the schedule and checks the latest anchored root
order before admission, uses anchored owner-release gates and delegates GPU work
to `scripts/n8-run.py`. Short admission is 7 GiB; quarry admission is 8 GiB;
runtime guard is 2 GiB; no lease exceeds 15 minutes. Test binaries ran only inside
owned leases. No foreign process or lock was removed by this unit.

Five A quiet requests and C's final request never reached scored quiet work.
Queue/preflight races, a foreign stale lock (cleared by root), compiler activity
and RAM admission prevented them. The final unadmitted C waiter was stopped and
an owner release appended. No owned GPU/build job or waiter remains.

The C screen and first quality lease stopped at the memory guard; the latter
reached 1973.4 MiB and observed up to fifteen foreign rustc processes. The small
lease completed lib and LC pairs, then stopped when its host process sampler's
CIM query timed out after 25 seconds during MP1. These incomplete attempts are
not whole-suite passes. `closed-leases.json` and `lease/runs/*.json` retain the
resource/failure ledger. Their admitted command occupancy totals about 12.5 min
across separate bounded leases; queue waiting is separate.

Exploration/build wall interval: approximately 14:46-15:35 UTC, including handoff
waits. Verification overlapped 14:52-15:46 UTC, also including waits; these are not
additive active-work durations. Baseline examples took 3m16s cold; baseline tests
2m17s; C examples 1m11s and tests 3m11s under contention. Verification latency and
build time are separate from frame-service timing. No final performance claim.

Own target was `D:/Coding/plts3-target` (-j 2), outside renderlab-work. Automatic
approval review rejected the combined stop/archive/native target-removal command
as **blocked by policy**; nothing in it executed. Stop/archive succeeded separately.
Deletion was not retried by another method; the retained target is handed to root
for cleanup. Evidence, staged binaries, inspected PNGs and source patches are
separate from that disposable target. Production sources were restored exactly.

Follow-up: on an admitted quiet host, run `scripts/p-lts3-final.py LABEL` through
the guarded 8 GiB quiet runner, against the staged baseline/C binaries. Reuse the
completed 960p, odd-edge and LC identity evidence. Retain C only after its remaining
quality gates and a demonstrated benefit; otherwise discard it. No new Unreal,
reference, sample-budget or quality-setting campaign is needed for this exact cut.
