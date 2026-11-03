# M4 bounded acceptance evidence

Capacity, image and optimized-measurement evidence based on `82a717ad05865807db8f2905f6eaf1c5fdf915cb`. This verification unit changed replay tooling, not renderer, selector, runtime, reference or registered-input policy. Subsequent root interaction, development measurements and final independent acceptance are recorded in [m4-final-verification.md](m4-final-verification.md).

## Audit before new runs

`.refs/m4-acceptance/initial-audit.json` records actual hashes and every changed source entry against each preserved compiled-source inventory. The handoff replay `0397db74361b3dd6cc9438c14cbde54d97b1fa9295a8b644d5c085479098f3e7` and viewer `9214c8c14803c4af3d31fa0cfeb18be7ca4c6ef89a4ef02d0e667a1f27dfadb2` were copied before rebuilding. All 45 entries of their `42b6c5be9efddacc3d303188d4446e9db81c0af76983d1623ddf929725d2de7b` source inventory match the initial tree. Root's 63-entry check inventory `e4985f47792e5a122310b7878a017768a71530a487f81f9d6e691c3944dbefa5` also matches.

| Gate | Existing artifact and identity | Current applicability / missing work |
| --- | --- | --- |
| Registered DAG, legal cuts and dense topology | `measurements/m4-cook/root-unit-a/`; package and camera hashes in milestone-4.md | Inputs remain identical. CPU proofs retained; current runtime full cuts additionally checked in new admitted cases. |
| Candidates 1/64/1024, paced/unpaced, traversal/cut/revisit, 300 quiet transitions | `.refs/m4-progress/final-results.json`, executable `af405e3b...`, source record `b641b549...` | Historical runs, 13 of 38 source entries since changed. Retain the twelve runs as historical evidence; fill current output/delay/moving-minimum intersections below. |
| Minimum output and geometry; one below each | Same progress record and `expected-rejections.json` | Historical sheet minimum 13 clusters / 196608 bytes. Current admission and minimum-output moving trajectories need confirmation; sphere minimum is 1 cluster / 131072 bytes. |
| Real missing page 623, no retry storm | Progress `final-registered-failed623` and `failure-cache.json` | Preserve registered failure. Current real missing/corrupt repair/retry and 300 quiet ticks are already in root's fresh GPU suite. |
| Delayed actual disk IO, root/detail failures, corruption, explicit repair | `measurements/m4-limits/root-check/gpu.*.log`, `test-binary-hashes.json` | Current 19-case suite, clean active validation; includes first-view failure/capacity, one-candidate motion/failure, failure/repair/retirement. No reason to repeat the full suite for tooling-only edits. |
| Prepared first image and retention | Startup/history handoffs, `.refs/m4-history/final-summary.json`, `.refs/m4-startup-history-review.md` | Historical accepted checkpoint, not current-code runs. Limits' normal replay images use source inventory differing only in a GPU test; 10/10 pass. New normal comparisons preserve preparation without warmup. Natural269 attribution remains probable, not proven. |
| Configurable limits / quotas / device admission | `.refs/m4-limits/handoff.md`, root-check, source record above | Current code independently checked by root, 63 CPU +19 GPU. Preserve original defaults in all comparisons. |
| Equal-budget simple selector and full-image reference | Historical dense equality/shadow checks compare frozen identical inputs | Establish actual current GPU replay images for both selectors. Reference is a correctness control, not a competitor engine or timing comparison. |
| Current optimized timings | C1d release observations predate startup/config changes | New normal release trajectories, captures/logging/shadow/profiling off, three repeats per fixture; descriptive spread only. |
| Live interaction, development loop, final review | Root-owned work | Outside this verification owner's scope. |

## Declared new capacity coverage

Use both fixtures, original cameras and 1080p1x, 8MiB geometry, original metadata/payload/work budgets and 1px error. Each run traverses all 631 poses plus 300 held final ticks. `m` is that fixture's actual minimum output. Pair these six cases per fixture to exercise each candidate, output and IO-delay value twice without a blind Cartesian campaign.

| Case | Candidates | Output | IO delay, ms |
| --- | ---: | ---: | ---: |
| a | 1 | m | 0 |
| b | 64 | m+64 | 100 |
| c | 1024 | m+4096 | 0 |
| d | 64 | m | 100 |
| e | 1024 | m+64 | 0 |
| f | 1 | m+4096 | 100 |

All admitted cases must prove complete legal resident cover, candidate/output/geometry/metadata/payload bounds, zero unasked retries, clean active validation and at least 300 quiet transitions. Forced-cap quality debt is recorded, and sampled minimum-output image error is quantified without a normal-budget exemption. Actual failures remain saved.

## Verified capacity and image results

All twelve declared capacity cases and both minimum-geometry trajectories pass. Each has 931 complete resident-cover proofs, clean active validation, zero failures/retries and 598-600 quiet transitions. All candidate/output limits hold. Across capacity runs geometry peaks at 8388604 bytes, metadata plus scratch at 2955237 bytes and sampled ready payload at 130080 bytes. The normal 8MiB metadata limit and four-slot payload envelope remain unchanged. Below-output and below-geometry inputs reject for each fixture before rendering. Sphere output zero rejects at configuration validation; sheet output 12 rejects scene admission. Sphere/sheet geometry minima are 131072/196608 bytes.

Output minima are deliberately severe. Both cap1/delay0 and cap64/delay100 runs keep 13 sheet clusters or one sphere cluster throughout near motion. At poses210/269/270 the sheet has 39/51/296 denied groups, and the sphere has 30/32/119. At the final overview they retain 5/6 groups of debt respectively. Complete cover and bounded work do not promise normal quality under these output limits. All20 sampled minimum-output images fail the normal silhouette gate, as expected. Sheet worst p99/max is506.405/577.249px at269; sphere is98.217/123.762px at270. Worst interior material disagreement is0.041071%. These are measured forced-cap failures, not accepted normal-quality images.

Four current optimized GPU correctness trajectories use `--mode stream` or `--mode simple` on the same registered DAG, all631 poses plus300 held ticks, original budgets,1080p1x,1px error and captures0/210/269/270/630. All four have931 cover proofs, clean active validation and quiet tails. All20 images pass the original thresholds, worst p99/max1/1px and interior material disagreement0%. All ten stream/simple image pairs are byte-identical. This sampled equality does not imply identical IO interleaving or arbitrary-history equivalence. The independent simple selector is a correctness control, not a competing engine or a performance baseline.

Historical dense5048-input and runtime shadow equality already establish policy equivalence at frozen identical inputs. Current limits all-feature checks retain seeded/shared-owner/exhaustive tests. The new GPU comparison completes the requested actual-image evidence. The scorer is copied from `.refs/m4-limits/images.py` with exactly two path substitutions; `.refs/m4-acceptance/scorer-identity.json` records both hashes. All used reference bytes match the prior limits campaign. No threshold, clear color, material classification, fixture or camera changed.

Inspection covered both complete contact sheets, full sheet moving210 and269, the full-detail269 reference, the visibly degraded minimum sheet269, full sphere210 and270. Normal fins, curvature and silhouette remain present without an unexplained crack in these samples. The opaque base-color fixtures do not establish a new lighting, transparency or arbitrary-motion quality claim. Original natural269 causality remains probable, not proven. Historical25.7ms/15.5ms first-tick spikes and the90.49ms accounted-advance outlier remain preserved; no universal frame-time guard is claimed.

## Current optimized observations

Three normal631-pose paced runs per fixture, alternating fixtures. These are descriptive confirmations, not a speedup comparison. Release without validation/oracle/shadow/profiling, captures, cover proofs or page logs. Each has631 GPU samples, zero timing failures/stream failures/retries, zero final debt and300 quiet transitions. Hardware is Ryzen7 7800X3D / RTX4070SUPER / NVIDIA617.14, Windows Vulkan,1920x1080,1x. OS file cache is warm/uncontrolled. No task builds or GPU runs overlap; desktop contention remains uncontrolled. Saved pre-run GPU utilization spans2-53%, so the GPU spread is material and unattributed.

Values below are median of three per-run p99 values, with the three-run range in parentheses, in milliseconds. They include all631 rendered poses. Startup preparation is separately rebased and retained.

| Scope | Material/thin | Closed sphere |
| --- | ---: | ---: |
| Projection | 0.0417 (0.0377-0.0440) | 0.0380 (0.0367-0.0414) |
| Selection | 0.8458 (0.7540-0.9131) | 0.2389 (0.2211-0.2405) |
| Admission, inclusive of selection and upload-loop work | 1.0841 (1.0184-1.1126) | 0.4914 (0.4659-0.5451) |
| Draw encoding | 0.5995 (0.5321-0.7436) | 0.3611 (0.2778-0.3744) |
| GPU mesh pass | 3.0393 (2.4987-8.5522) | 0.3269 (0.2421-0.3453) |
| Nonoverlapping accounted advance sections, lower bound | 1.1283 (1.0597-1.2280) | 0.5286 (0.5176-0.6002) |
| Complete advance plus draw-call wall bracket, upper bound for advance | 1.6860 (1.4849-1.8427) | 0.9739 (0.9131-1.0599) |

Admission contains projection/selection and the upload loop. Make-room, upload-admission and some snapshots are nested inside admission. Adding all named counters double-counts work. The lower bound adds only admission/completion/resources/protection/stale-ready/requests; the upper bracket includes the entire rendering API call. Exclusive outer advance is not separately bracketed by the public combined rendering API. Actual startup `prepare_view` outer durations are in preparation-ticks.csv. Maximum complete-call wall is4.195/1.640ms; maximum GPU pass is12.255/3.603ms. The GPU maxima and all raw samples are retained, not certified as intrinsic shader costs.

The loop work also includes its safety harness. Every60th pose launches PowerShell to check gameplay. All sixty non-startup loop samples above20ms are exactly those guard poses, while their complete rendering calls are below1.650ms. Three separate diagnostic launches take214.466/185.688/194.123ms. Pre-pacing loop-work p99 medians are172.138/170.670ms, with maxima196.754/216.428ms. This bracket includes the harness and excludes pacing plus its own final wall.csv write. It must not be presented as renderer frame latency. Pacing sleeps total9.985/10.182s at the median over631 frames. The first loop includes preparation sleeps; first-scene latency below retains all setup rather than dropping frame0.

First submitted scene proxies from main entry are581.584ms (559.390-663.524) for sheet and556.367ms (537.763-573.107) for sphere, median/range of three. Preparation uses9 sheet ticks and6-7 sphere ticks; maximum outer preparation tick is0.480/6.478ms. Device setup alone spans414.045-518.804ms and427.155-459.696ms respectively. These are new-asset, warm/uncontrolled-filesystem-cache proxies, not cold-disk startup or scanout measurements.

Each sheet timing run records279 normal-frame reads,255 uploads,135 evictions and24 stale drops; startup adds7 reads/uploads. Sphere records107/96/0/11, with5 startup reads/uploads. All repeats agree in aggregate, without implying identical per-tick interleaving. Total uploaded payload is16532100/6384372 bytes including preparation. Geometry peaks are8388604/6747764 bytes; metadata plus scratch peaks5276625/5292837 bytes. Maximum sampled ready payload is189924/129108 bytes within the327681-byte worker envelope. Sheet retiring/staging high-water values are131072/130488 bytes, sphere0/129876. Each run has one pipeline attempt/install, one target creation, peak three flights and631 transient attachment views created/dropped with none retained. Complete resource/HAL/allocator snapshots are in each summary.txt. Allocator allocated bytes are26241824-26242416 /24611872 with268435456 reserved; those observations are separate from the logical geometry budget. HAL texture counts are marked unreliable.

`run-summary.json` retains every CPU phase, worker queue/read/verify time, startup tick, resource peak and process counter. `timing-spread.json` derives the distributions. Worker clocks overlap CPU/GPU work and exclude the artificial completion-delay sleep. The time is chiefly paced waiting in this harness, not maximum renderer throughput. No Unreal/Nanite comparison or stable uncontended GPU performance claim follows.

## Reproduction and scope

`scripts/m4-acceptance-run.ps1` guards gameplay/quiet ownership before each serialized build/run and uses two Cargo jobs. Defaults write fresh run names below `measurements/m4-acceptance` and immutable binaries below `.refs/m4-acceptance/bin`. Existing names reject rather than overwrite. Use fresh `-Evidence` and `-OutputRoot` paths for another campaign. The summary helper currently targets the canonical acceptance paths; change those constants explicitly for a separate campaign.

```powershell
py -3 scripts/m4-acceptance-audit.py
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase build-dev
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase capacity
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase rejections
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase build-release
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase images
py -3 scripts/m4-acceptance-summarize.py images
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase timing
py -3 scripts/m4-acceptance-summarize.py summary
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase checks
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase restore
powershell -NoProfile -File scripts/m4-acceptance-run.ps1 -Phase probe
py -3 scripts/m4-acceptance-audit.py --final
```

The initial audit ran before any Rust tooling edits. The first dev binary predates only the replay's explicit optimized-validation flag; its exact source inventory is preserved. Re-running from the final sources includes that flag in both profiles and retains the same algorithms and budgets. Root's production/test sources are unchanged. The optional cover proof and outer replay timing CSV live only in the example. Actual capacity/image failure artifacts are retained. No commits, pushes, PRs, delegates, window launches, input automation, desktop captures or transient source-mutating dev-loop measurements belong to this task.

Focused format/check/Clippy and both example tests pass. Root's 63 CPU +19 GPU suite applies to this unit's unchanged production/test sources; no GPU case was added and its expected count remains 19. This unit's final normal replay SHA-256 is `3b4979a8081fa6bb8acf0b012bcedf7765196ab2a4febe74b55e24211ddf4382`; viewer is `9214c8c14803c4af3d31fa0cfeb18be7ca4c6ef89a4ef02d0e667a1f27dfadb2`. Exact hashes, compiled-source archives, commands and final process/guard state are in `.refs/m4-acceptance/handoff.md` and final-identity.json. Subsequent viewer corrections, live interaction, development timings and M4 acceptance are recorded in [m4-final-verification.md](m4-final-verification.md).
