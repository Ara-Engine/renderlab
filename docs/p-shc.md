# P-SHC: shader compilation and startup

2026-11-09, branch `t3/p-shc`, baseline `dc93b7e12b48b1334acd774bcf498dca854c56e4`.
Scope: pipeline creation, program composition diagnostics and reload transactions.

**Partial result: the 1–2 s first-new-shader target is not met.** Native driver
compilation remains the dominant cost. No compiler experiment is promoted. The
retained implementation completes the mixed-program reload transaction before
publication, including its dependent glass reflection shader. Shader source,
lighting algorithms, material interfaces and MP1's per-program dispatch stay intact.

## Retained changes

- MP1 list pipelines now join ordinary pipelines in a program's reload worker.
  The previous program, parameter layout and pipelines remain active until every
  requested pipeline succeeds. Syntax and pipeline errors reject the transaction.
- The last frame's active list variant is tracked separately from ordinary hot
  sets. Previously used, inactive variants stay lazy. Switching to another variant
  while a worker runs invalidates its recipe instead of publishing incomplete work.
- Glass reflection hit shading depends on the first opaque program. Its current
  pipeline is compiled alongside that program's list, using its existing layouts;
  publication replaces the pipeline and source key while retaining its targets.
  Replaced reflection resources invalidate the snapshot by pipeline identity.
- Synchronous `reload_program` uses the same transaction. This moves compilation
  into the API call; the watcher uses the worker and keeps rendering the old revision.
- `RENDERLAB_STARTUP_PROFILE` adds composition, program analysis, module/cache and
  source-size records. Existing pipeline and native-driver timings remain available.

## Measurement correction

The current `game_scene --reload-probe` replaced `return exposed;`, but the game
program's final return had gained an emission term in `47c9fda2`. The replacement
no longer matched. P-SHC's first 0.213 s "edit" sample was a no-op and is excluded
from new-edit evidence. Its initial pipeline compilation records remain valid.

The probe now wraps the actual emission-inclusive return, checks that its seam
matches exactly once and requires changed source. It records the tint, an edited
image hash, rejection/recovery and a cached repeat. An optional fresh initial tint
exercises startup with an unseen program. Optional complete HDR hashes supplement
the normal RGBA8 hashes without writing raw buffers.

This correction does not invalidate historical measurements on earlier source
versions. ARA1's separate standard-program edit is also a different endpoint.

## Host and protocol

Windows 11, Ryzen 7 7800X3D (8 cores/16 threads), 32 GB RAM, RTX 4070 SUPER 12 GB,
NVIDIA 617.14, Vulkan/wgpu 30.0.1, Rust 1.96. Two Cargo jobs. Bulk output and both
owned targets are under `D:/Coding/renderlab-work/p-shc/`.

PC1's `game_scene` harness: 1280×720 game fixture; RT traced primaries/Interactive
shadows and non-RT visibility-buffer/Virtual shadows. Fresh-edit screens disable
GI and TR1 at runtime, preserving the shader's complete compiled feature graph.
Quality captures use the ordinary defaults, frames 0 and 47, complete RGBA8 and
HDR hashes, and a drained 32-frame timing segment. No shader feature was removed
to obtain a compile result. PNG encoding is outside edit/readback timing.

Every GPU process uses `scripts/n8-run.py`, a lease at most 15 minutes and a strict
6 GiB admission/runtime floor. Root orders and owner-matched releases are checked.
GPU work is headless. Native diagnostic screens additionally enable
`RENDERLAB_STARTUP_PROFILE_DRIVER=1` and `WGPU_DEBUG=1`; final measurements use the
ordinary device with `WGPU_DEBUG=0`. Application caching and the uncontrolled
NVIDIA private cache are distinguished. Contended diagnostics are not quiet-window
performance certificates.

## Where the time goes

Corrected fresh RT edit, release diagnostic binary C, `hints-c`, seconds:

| Boundary | Seconds |
|---|---:|
| Composition | 0.00083 |
| Program analysis | 0.02317 |
| Ordinary module creation/cache work | 0.05869 |
| List pipeline API | 25.79277 |
| Native driver inside that API | 25.69461 |
| Complete reload | 25.94621 |
| Edit to updated readback | 27.58105 |

The pipeline API's residual ~98 ms includes SPIR-V generation and wgpu work.
Native compilation is over 99% of that pipeline call. The traced composition is
164,317 bytes of WGSL; the edited native SPIR-V module is 409,144 bytes. These
sizes do not measure the driver's optimized/inlined instruction graph.

The extra ~1.635 s after reload included dependent glass compilation. With the
retained glass transaction in binary D, the sampled RT gap was 0.147 s
(31.310 s API / 31.457 s readback). Native compilation varied substantially with
contention; these two samples establish the moved boundary, not an end-to-end
speedup. D's non-RT sample was 24.757 s API / 25.000 s readback.

## Rejected compiler screens

Release/native diagnostic device, application persistence off, fresh visible
tints. Single contended samples; ordinary frames of completed loop-hint screens
match their same-path baseline. GI-off screens alone do not certify GI-on quality.

| Experiment | RT edit/readback s | Non-RT s | Decision |
|---|---:|---:|---|
| Corrected C baseline | 27.581 | — | Profile only |
| SPIR-V DontUnroll, refinement loop | 30.255 | — | No improvement |
| SPIR-V DontUnroll, GI functions | 32.137 | — | No improvement |
| `min` runtime bounds, three GI loops | 25.482 | 35.416 | No consistent gain |
| Opaque fixed counts in reserved header word | 28.976 | 30.228 | No improvement |
| Vulkan DISABLE_OPTIMIZATION | 31.757 | — | No improvement |
| DontInline, GI / environment helpers | incomplete | — | 6 GiB guard stopped each |

The interrupted NoInline cases ran approximately 101.4 s (GI) / 32.1 s
(environment) before the guard ended their processes; neither supplies a complete
edit timing. Their minimum sampled available RAM was 5,989 / 6,035 MiB, and both
batches overlapped foreign builds. The resource limit is unchanged.

The opaque-count experiment's cached repeats were 235 ms RT / 141 ms non-RT;
DISABLE_OPTIMIZATION's repeat was 237 ms. Cached reuse does not solve unseen edits.
The uniform header write, all composition transforms and native hint changes are
reverted. No new quality setting, helper-call policy, binding or default is shipped.
Experiments are preserved in the external `source-e.patch` and staged binaries.

## Final verification

Final retained-code executable SHA-256:
`10e163fbd5844a7069c55796a00960dabd683d3633482057d72f0ad894ccebd9`.
Original-runtime baseline with the corrected harness:
`5cce471fba974b2a0a70f90e0d17ab647d15f1e98cce21beaf10145cf57e618b`.
Both use release optimization and the ordinary device, validation/debug off.

| Game / path | Main before | P-SHC after |
|---|---:|---:|
| RT, fresh edit → readback, s | 21.612 | 20.818 |
| Non-RT, fresh edit → readback, s | 23.770 | 21.143 |
| RT, cached repeat → readback, ms | 1,189.3 | 93.8 |
| Non-RT, cached repeat → readback, ms | 103.0 | 42.5 |
| RT, first cache-off image, s | 28.595 | 28.761 |
| Non-RT, first cache-off image, s | 27.131 | 27.633 |
| RT, drained ms/frame | 14.859 | 14.840 |
| Non-RT, drained ms/frame | 24.026 | 24.090 |

The fresh tints differ between latency samples to avoid previously compiled edits;
their shader structure and workload match. These are single paired screens, not a
large-factor performance certificate. Frame cost differs by at most 0.3% in this
screen. The cached RT improvement includes routing the glass shader through the
program cache. The first-new-shader target is missed; no cold-startup improvement
is demonstrated. First-image samples disable application persistence and leave the
driver-private cache uncontrolled. The registered cold-startup/cache matrix is a
separate endpoint, not inferred from these samples.

- **Library:** 197 passed, 0 failed, 164 ignored, final retained-source test binary.
- **Mixed reload, both paths:** new list installed before the first edited frame;
  invalid source rejected with previous HDR intact; restore HDR exact; a newly used
  list invalidates an older recipe; inactive warmed variants remain lazy.
- **Heavy background responsiveness:** eight complete old-image HDR/readback frames
  during a genuinely new GI-capable program compile, on each path. Maximum frame
  completion was 83.541 ms RT / 75.755 ms non-RT. This includes capture and readback;
  it is not a presentation or scanout measurement.
- **Main identity:** all eight selected captures (main/final × RT/non-RT × frames
  0/47) match completely in both RGBA8 and HDR, with the ordinary GI/TR1, reflection,
  transparency, atmosphere, particle and post settings. This covers the game fixture,
  not every scene or arbitrary custom shader.
- **Post-reload recovery oracle:** the original-runtime and final `restored.png`
  files are also byte-identical on both paths after the edit/rejection/restore
  sequence, covering reuse of the glass targets. Inspected both final default
  frame-47 PNGs and the RT probe baseline: geometry, silhouettes, shadows, glass,
  mirror/gold reflections, foliage, particles and the toon hero remain present.
- **ARA1 devloop:** the existing tool completed startup to a fresh GPU-readback marker
  on the matched game scene: 28,905.7 ms RT / 27,988.9 ms non-RT. It is a two-frame
  startup replay, not a new Rust-edit measurement or a controlled cold-cache claim.
- Formatting, diff whitespace and Python syntax checks pass. No default WGSL file
  or rendering algorithm changed. Full ARA1 scene/frame-cost and quarry identity
  campaigns were not rerun; no engine-wide performance claim is made.

Evidence: `runs/final-gates`, `runs/matched-before`, their lease logs/resource
samples, binary/source manifests and inspected game PNGs. The old baseline artifact
that Cargo reused is excluded. The separate-target rebuild is validated by its
source path in Cargo's log and by the absence of the new list-factory label.

The delegated coordinator granted the final `startup-final-1` quiet window after
P-LTS's owner release. Ordinary device, fresh initial program per path/build,
empty application cache followed by its populated replay; the NVIDIA private cache
remains uncontrolled. These single samples show no consistent startup improvement:

| Startup / path | Main before, s | P-SHC after, s |
|---|---:|---:|
| Empty application cache + unseen initial program, RT | 24.257 | 27.224 |
| Same, non-RT | 30.023 | 24.183 |
| Populated cache, RT | 0.756 | 0.813 |
| Populated cache, non-RT | 0.779 | 0.811 |

All cold/populated pairs have identical complete RGBA8 images on both selected
frames, for both paths and binaries. The quiet lease completed in 118.657 s,
with zero sampled foreign compilers and at least 7,902 MiB available throughout.
The admission floor was 8 GiB; the runtime floor remained 6 GiB. No more GPU leases
or builds are requested for this unit.

Verification used seven GPU windows totaling 23.67 minutes, including guards and
setup; the longest was 6.26 minutes. The final gate and main-baseline windows also
sampled zero foreign compilers, with minima 8,041 / 8,934 MiB. Two diagnostic
windows stopped at the RAM guard; three yielded between processes to a new root
order. Completed results are retained, incomplete cases are excluded.

Implementation, diagnosis and report editing took roughly 75 active minutes
(manual estimate), interleaved with queue/quiet holds. Recorded Cargo work took
about 18 minutes, separately from GPU verification; the before target's clean
release build took 154.99 s, final warm release/test builds 34.19 / 70.37 s. A
second cached build invocation took about one second per final artifact. No new
Unreal or path-traced campaign ran. The three-hour unit ends with the latency and
cold-startup goals explicitly missed.

Automatic approval review rejected the exact-path, containment-checked PowerShell
removal of both owned Cargo targets, stating only **"blocked by policy"**. No
alternative deletion method was attempted. They remain for coordinator cleanup:

- `D:/Coding/renderlab-work/p-shc/target`: 2,346,629,105 bytes.
- `D:/Coding/renderlab-work/p-shc/target-before`: 685,336,369 bytes.
- Total: 3,031,965,474 bytes (3.03 GB).

No owned GPU process, build, lease waiter or poller remains. Retained executables,
JSON/log summaries and inspected PNGs stay outside the targets. No raw GPU dumps
were written. The compiler hold through P-LTS's following quarry retry was
recorded and relayed to the other active workers.

The next structural candidate is separating unchanged engine work from user
shading. It needs a typed, shading-model-independent interface and measured buffer,
dispatch and frame costs; CC1's slower shared-call prototype remains rejected.

## Reproduction

```text
py -3 -B scripts/p-shc-run.py build --baseline-ref dc93b7e
py -3 -B scripts/p-shc-run.py order
py -3 -B scripts/p-shc-run.py gpu LABEL rt:lib rt:reload-gate nonrt:reload-gate rt:quality-baseline nonrt:quality-baseline rt:baseline nonrt:baseline --expected-order SHA256
```

`P_SHC_BINARY` selects a staged executable (default `bin/final.exe`). `cold`/`warm`
cases use a fresh initial program and empty/populated application cache;
`devloop` reuses `tools/ara1/devloop.py` on the matched game scene. The ARA1 helper
accepts an explicit lease owner while keeping `m7-ara1` as its default.

The main snapshot keeps the corrected measurement example but original runtime
sources. It has its own target: sharing a target initially let Cargo reuse a
same-name path-package artifact with older archived mtimes. That artifact was
discarded before any baseline measurement. Binary hashes, source/harness hashes,
commands, cache state, root order and resource samples accompany the runs.
