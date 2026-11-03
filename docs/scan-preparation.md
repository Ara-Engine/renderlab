# Quarry preparation performance unit

This unit addresses one measured selector cost and the replay's unconditional final group dump. It does not finish real-scan acceptance. Materials, lighting, texture quality, AA acceptance, resize, presentation latency and broader comparisons remain separate work.

## Replay diagnostics

`virtual-stream` writes `groups.csv` only with `--dump-groups`. Its final per-group closure diagnostics can cost more than the rendered replay on the quarry package. The flag changes post-run diagnostic output. It does not change preparation, rendering or budgets. Output directories must remain fresh.

```powershell
target/debug/examples/virtual-stream.exe --manifest measurements/quarry-scan/quarry-virtual.manifest --cache measurements/quarry-scan/cache --cameras measurements/quarry-scan/cameras.json --frames 16 --samples 4 --validate --limits measurements/quarry-scan/limits.json --dump-groups --output measurements/quarry-preparation/FRESH-DIAGNOSTIC
```

The existing replay wrappers forward remaining arguments, including this flag. A tracked-source search found no script that reads `groups.csv`. Historical commands without the flag now omit it. The older diagnostic description in `docs/virtual-runtime.md` predates this opt-in behavior.

The replay checks the quiet-window lock and protected game processes before startup and every sixty poses. Windows process enumeration uses a native snapshot instead of launching PowerShell inside timed work. The external scan observer continues checking games throughout preparation and replay, and terminates only its owned process if a protected game starts. Enumeration failure rejects the replay.

## Measured root cause and bounded change

The baseline's prepared package already validates and indexes metadata once. Repeated selection still performs per-view domain checks, initial-cut checks, sorting, planning and publication. The separate dev diagnostic run recorded 1,250,034 publication transactions rejected only for unavailable pages. Those transactions repeatedly walked, changed and rolled back owner closures. Publication accounted for 27.61 seconds in that instrumented run. Structural domain checks accounted for 0.75 seconds. Make-room accounted for about 0.29 seconds in preparation. These are diagnostic observations, not the paired benchmark.

The fast path proves that neither capacity can deny any publication for this call. The mesh's entire cluster count must fit the output capacity. The union of every mesh page with pinned and protected pages must fit geometry capacity. It then checks the candidate's own outgoing shards. An unavailable shard still on the admitted plan's boundary cannot be replaced by a publication contained within that plan. That candidate's publication transaction can be skipped. Planning, candidate charging, demand and final debt reporting continue normally. When either capacity proof fails, the original transaction path runs.

One temporary page bitmap computes that conservative page union while constructing selector state. It fits within the existing sixteen-bytes-per-page scratch bound and changes no public API. No selection threshold, camera, candidate budget, resource budget, upload rate or worker scheduling policy changes.

The patched dev diagnostic skipped 1,148,528 transactions. Publication work fell to 2.66 seconds. Other work remains, including repeated full-cut setup/emission, page-array scans and global protection unions as small upload batches become resident. The diagnosis predicted an improvement rather than practical dev startup in a few seconds. That target remains open.

## Measurement boundaries

The canonical content is the existing twelve-scan quarry composition with 13,034,903 source triangles, 13,126 virtual groups, 214,062 clusters and 16,339 package pages. The package, RLA, camera file, placements and limits are unchanged. The scene still uses opaque, double-sided, unlit 2K base color. This does not establish source material fidelity or lighting quality.

The paired campaign uses five alternating baseline/fixed runs in each build configuration. Dev uses active Khronos validation. Optimized uses no validation. Every run uses the original sixteen 1920x1080, 4x MSAA camera poses and effective budgets. Cache files are explicitly read before each process starts. Timed runs omit captures, cover proofs, group/page dumps and selector profiling. Both sides include the same external tasklist/nvidia-smi observer. Its overhead and system load samples are retained. These are offscreen timings; no acquisition, presentation or scanout latency is measured.

On the Ryzen 7 7800X3D / RTX 4070 SUPER, NVIDIA 617.14, Rust 1.96.0 host, the completed campaign measured the following first settled submission times. Each cell gives the median and minimum-to-maximum range of five runs.

| Configuration | Baseline seconds | Fixed seconds | Median reduction |
| --- | --- | --- | --- |
| Dev, active validation | 74.931 (74.760–75.506) | 53.650 (53.271–54.115) | 28.4% |
| Optimized, no validation | 7.203 (7.152–7.299) | 3.823 (3.734–3.885) | 46.9% |

All twenty timed processes passed the audit and retained the same initial settled counts and effective budgets. The GPU baseline samples were 0%, 0% and 2% utilization. This remained a shared PC. Host CPU samples included brief competing-load spikes up to 66.6%; the sides were alternated and their ranges remained disjoint. GPU pass timestamps varied substantially, so this unit makes no GPU throughput claim. First submission is a CPU submission event, not an image-completion or presentation measurement.

First submission is the replay's process-start-to-first-settled-scene-submit event. It includes device initialization, bootstrap, preparation and CPU submission. First legal cover is recorded separately. A successful initial settled run must still read and upload 1,776 pages, publish 19,214 clusters, use 116,391,936 geometry-budget bytes, and report zero unresolved groups. Moving frames can retain residency debt after abrupt camera changes.

Timers are not additive. `admission` includes scene selection/projection and a later upload interval. `selection` is nested inside it. Make-room and upload admission also sit inside admission. Snapshot time includes snapshots nested in those intervals and snapshots outside them. Worker IO queue/read/verify counters overlap main-thread work and can accumulate across pages. Polling is measured separately. The optional `selection-profile` build adds bootstrap read, digest, decode and prepared-validation/index spans, plus selector phase/transaction attribution. Profiling runs are excluded from the paired results.

The selector profiler's `admission_ns` is its initial-cut validation/setup phase, distinct from runtime `admission_total_us`. On a prepared package, its `index_ns` measures per-call state/reference initialization; it does not mean the retained incoming CSR is rebuilt. The temporary page-union bitmap increases this setup cost. In the separate dev profiles it rose from 3.03 to 4.71 seconds over the complete replay, while the larger publication cost fell.

## Reproduction and evidence

`scripts/scan-perf-run.py` preserves executable hashes and compiled source hashes, warms caches, derives replay arguments from the original process record, and launches the existing observer. Fresh labels preserve previous evidence.

```powershell
py scripts/scan-perf-run.py build FRESH-DEV
py scripts/scan-perf-run.py build FRESH-RELEASE --release
py scripts/scan-perf-run.py run FRESH-LABEL FRESH-DEV --validate
```

The local campaign's immutable executables and compiled identities are under `.refs/quarry-preparation/bin/`. With those preserved baseline/fixed names present, rerun all five pairs using `py scripts/scan-perf-run.py campaign FRESH-TAG`, then audit using `py scripts/scan-perf-report.py --tag FRESH-TAG`. `py scripts/scan-perf-run.py quality` uses fixed held views and moving captures with active validation and cover proofs. Its labels are fixed and must not be reused without selecting new labels in a local copy.

The complete local handoff is `.refs/quarry-preparation/handoff.md`. Exact runs, distributions, effective limits, errors, source/binary identities, captures, remaining debt and reproduction commands are retained there and in `results.json`. The hypothesis and predicted benefit were recorded before the selector edit in `decision.tsv`. Source and binary baselines, raw measurements, assets and captures remain ignored. No private scan assets are added to the repository.

Verification includes the independent two-track selector policy/oracle suite, runtime preparation/admission/IO tests, the registered moving-fin GPU publication test, held full-frame comparisons and clean active validation. The opt-in tool change reproduced all four original quarry readbacks byte-for-byte with the dump absent. Parent review owns final acceptance and publication. This unit makes no renderer, Nanite or Unreal superiority claim.

All four 4x held views settled with zero unresolved groups and matched baseline pixels exactly. All sixteen matched-cadence 4x motion captures also matched exactly. The dev 1x replay completed all sixteen poses and cover proofs with clean active validation and no group dump. These motion paths still end with residency debt around 1,699 groups and maximum denied error about 3.02 pixels after abrupt cuts. This unit preserves that limitation; it does not certify settled motion quality or AA acceptance.
