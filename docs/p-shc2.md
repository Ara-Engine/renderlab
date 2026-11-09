# P-SHC2: structural compiler screen and background publication

2026-11-09, branch `t3/p-shc2`, base `3910082da2133d30e2e6e0286b82e8378f12ed66`.

This unit screens a deferred engine-input boundary and measures the existing
background reload fallback. **A production structural split is not implemented.**
The 1–2 second first-new-shader target is not established. No production WGSL,
renderer scheduling, quality setting, dispatch, or compiler policy changes.

The retained changes are measurement code: CC1's fresh live tint now wraps the
current game return (including its programmable glow), asserts a unique seam,
and fails instead of silently compiling unchanged source. The mixed reload gate
now follows an unseen heavy shader through file detection, background compilation,
publication and the first updated HDR readback, rather than discarding the result
after eight old frames. It checks exact old HDR while compiling, updated output
against the synchronous reload, exact restoration, and no first-frame pipeline
creation after publication.

## Structural boundary

`buffered-gi` replaces only `gi_prepare` with full-precision loads of its nine sky
and four bounce coefficient vectors and valid flag. `buffered-engine` also models
precomputed reflection incident radiance and light visibility. Both are
**compile-only consumers**, use the existing table binding as an artificial input,
and never render. They do not implement their producer passes or a usable ABI;
the reflection and light placeholders deliberately do not allocate separate records.
Their timings measure the residual driver compilation graph, not edit-to-image,
quality, memory or frame performance of a split renderer.

GI preparation is independent of user `Surface` and is a plausible first seam.
A straightforward padded coefficient record is 224 bytes per pixel, or 464.5 MB
at 1080p, before reflection and visibility records. Packing and sharing queries
are possibilities, not measured optimizations. This screen neither quantizes
coefficients nor constrains user shading models.

The complete split needs more than an engine prepass:

- Reflection directions depend on `reflection_lobe(surface(input), input)`;
  `Surface` is an arbitrary user struct. Reflection hits call user surface and
  shade with hit material inputs, decals and the receiver program's parameters.
- The local light sum and BRDF are user code. `light_sample` and `light_visibility`
  accept caller-supplied inputs. Precomputing only the entry's receiver must not
  silently change helpers called with another input or eliminate custom light loops.
- A general staged interface must therefore carry user-produced ray requests,
  engine hit records, per-program hit shading and returned incident radiance,
  preserving ray order and accumulation order. Opaque user state needs a generated
  per-program storage layout or recomputation; it cannot become a fixed BRDF record.
- MP1's independent dispatches stay required at each user shading stage. Shared
  engine passes must not recompile on a user edit. Added buffers and dispatches
  need final-image identity and matched ARA1 frame-cost evidence before retention.

These are implementation constraints, not a proof that a complete split is
impossible or that its frame overhead would fail. No such overhead was measured
because no complete split renders here. CC1's rejected shared-call runtime result
is not reused as evidence about separate dispatches.

## Measurements and identity

Windows 11, Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Vulkan/wgpu 30.0.1,
release optimization. Native diagnostic device for the compiler screen. These
are single contended samples: up to twelve foreign rustc processes, minimum
sampled available RAM 6,012 MiB after 7 GiB admission. No runtime guard failed.
Do not compare these absolute times directly with P-SHC's quiet edit samples.

| Compile-only consumer | RT native seconds | Non-RT native seconds |
|---|---:|---:|
| Full current kernel, fresh tint | 30.729 | 41.209 |
| Buffered GI coefficients | 25.431 | 31.362 |
| Buffered GI + reflection + light visibility | 11.042 | 16.929 |

All six modules passed naga validation and Vulkan pipeline creation; native
compilation records exist for each. WGSL module calls took 11–17 ms. The last row
still contains `light_sample`, environment/filtering helpers, decals and user
shading. It is a partial boundary, so these numbers do not disprove a broader
split. Even its optimistic residual is far above the target in this screen.
No producer/runtime implementation was promoted and no frame-cost regression
was accepted. The structural goal remains open.

Historical P-SHC release game evidence, **not a P-SHC2 before/after result**:
fresh edit to readback 20.818 s RT / 21.143 s non-RT; empty-application-cache startup
with an unseen initial program 27.224 / 24.183 s; populated replay 0.813 / 0.811 s.
See [P-SHC](p-shc.md) for host, image identity, noise and private-cache limits.

Current main's corrected 1280×720 game probe, ordinary release device, application
cache disabled, fresh visible tint, GI/TR1 disabled at runtime (full shader feature
graph retained):

| Game endpoint | RT seconds | Non-RT seconds |
|---|---:|---:|
| Startup to initial image | 30.171 | 27.255 |
| Fresh API compile transaction | 26.085 | 23.926 |
| Fresh edit to updated readback | 26.180 | 24.081 |
| Cached repeat to updated readback | 0.0441 | 0.0466 |

Both probes require a changed source and reject invalid WGSL. These new samples
measure the unchanged main runtime under contention, not an optimized after build.
There is no demonstrated improvement over the historical 20.818/21.143 s samples.
The first-new-shader target is missed on both paths.

The fallback keeps rendering the previous pipeline until the complete active
pipeline transaction is ready. It does **not** reduce new-shader driver codegen.
The next watcher poll publishes a finished, current transaction; cadence and the
next rendered readback add to its latency. Physical scanout is not measured.

The 1080p mixed-program floor/eight-box fixture has GI and TR1 disabled at runtime,
but compiles an unseen GI-capable standard program. It polls between old-image
HDR readbacks, with a 50 ms sleep between polls. Ordinary release device:

| Background endpoint | RT | Non-RT |
|---|---:|---:|
| File write to publication, seconds | 40.672 | 32.256 |
| File write to updated readback, seconds | 40.701 | 32.306 |
| Publication to updated readback, ms | 28.8 | 50.2 |
| Exact old HDR frames during compile | 286 | 313 |
| Largest old frame including readback, ms | 432.4 | 97.4 |

Both tests passed, including rejection/recovery, active-list recipe invalidation,
lazy inactive variants, exact background-versus-synchronous updated HDR, exact
restored-versus-original HDR, and zero pipeline creations in the first updated
frame. This is a responsiveness fallback; the large maximum RT readback is not
a 60 Hz presentation certificate. Contention and a different program/fixture
prevent interpreting these times as a regression against P-SHC's game screen.
The watcher publishes on its next eligible poll; worker-ready-to-poll delay is
not separately instrumented here.

Release lib suite: **199 passed, 0 failed, 164 ignored**, 3.21 s, inside the owned
verification lease. Production sources and all default shaders are identical to
base `3910082`; only the `#[cfg(test)]` program-list test module, measurement
runner and documentation change. Thus no split frame-cost A/B exists, and no
rendering speedup or engine-wide frame-time claim is made.

Fresh initial game tint, ordinary defaults (GI/TR1 on), two captured frames, empty
application cache followed by its populated replay; NVIDIA private cache remains
uncontrolled:

| Startup to readback | RT seconds | Non-RT seconds |
|---|---:|---:|
| Empty application cache, unseen initial program | 30.120 | 30.309 |
| Populated replay, same initial program | 1.162 | 0.892 |

Complete RGBA8 **and HDR** hashes match at frames 0 and 1 in both empty/populated
pairs. Inspected both frame-1 PNGs: silhouettes, sun shadows, glass, metallic
reflections, foliage, toon character and atmosphere remain present. These are
early frames, not a settled/moving quality campaign. No cold-startup improvement
is demonstrated. Sequential game-probe `edited`/`rejected-retained` and
`initial`/`restored` PNGs do **not** match: they have different frame indices and
are excluded as identity oracles. The difference counts are preserved in
`sequential-probe-differences.json`; this unit does not claim full-game
post-reload identity from those images. The fixed-state mixed fixture and
matched-frame startup pairs are the measured identity evidence.

### ARA1 frame baseline

The unchanged production runtime completed the registered 1080p game, PBR and
quarry base timing specifications, with their registered settings/exclusions.
GPU render-span medians in milliseconds, eight samples per segment (still frames
32–39; moving 64–71), single frame in flight, contended:

| Scene | RT still / moving | Non-RT still / moving |
|---|---:|---:|
| Game | 2.566 / 4.518 | 7.178 / 11.819 |
| PBR | 1.718 / 2.785 | 3.126 / 4.638 |
| Quarry | 2.412 / 3.206 | 10.174 / 13.663 |

CPU service medians, first-cut and settled samples accompany these values in
`summary-ara.json`; raw per-frame records remain under `ara/`. There is **no
frame-time A/B**: no structural rendering variant was retained or rendered.
Consequently no within-noise split-overhead claim is made. This unit ends with
the structural optimization incomplete and the existing background fallback
verified, rather than accepting unmeasured frame cost.

## Reproduction and coordination

Evidence: `D:/Coding/renderlab-work/p-shc2`. Two-job target:
`D:/Coding/pshc2-target`, outside `renderlab-work`. Only test code and the runner
are changed; P-NRT2 GI/fog/reflection/scheduling, P-NSQ2 shadows and P-FLAT history
are untouched. Root received the seam/lease note.

`scripts/p-shc2-run.py build` compiles release lib tests and the game/ARA1 examples,
without running them. `order` stream-reads the latest anchored root directive.
`gpu KIND --label LABEL --expected HASH --after OWNER:LABEL` checks owner-matched
releases and calls `scripts/n8-run.py`, with at most 12 minutes, 7 GiB admission
(8 GiB for quarry/ARA1), and 2 GiB runtime guard. No test executable runs outside
an owned lease. `KIND` is `screen`, `background`, `lib`, `edit`, `startup` or `ara`;
`verify` batches background/lib, the game edit, startup and ARA1. Use a fresh
`P_SHC2_OUT` evidence directory for another startup campaign and stage the
executables there; an existing cache is rejected rather than mislabeled empty.
Compiler-quiet admissions stop builds and prevent leases. A changed root order
rejects the claim; there is no autonomous lease waiter.

The clean release lib-test build took 287.9 s; the example build 41.0 s.
The native screen's lease took 163.6 s. Implementation and verification latency
are recorded separately; GPU queue time is not compiler or rendering time.
The complete verification lease took 426.8 s (7.11 min), exit 0, zero guard
failures, minimum available RAM 6,500 MiB after 8 GiB admission, up to six
foreign rustc processes. Total owned GPU windows: 590.5 s (9.84 min), each
below 15 minutes. Implementation/diagnosis/report work was approximately 25
active minutes (manual estimate), interleaved with the shared queue. No raw GPU
buffer dumps were written.

Automatic approval review rejected the resolved-path, containment-checked
PowerShell deletion of `D:/Coding/pshc2-target`, stating only **"blocked by
policy"**. No alternate deletion method was attempted. The target remains for
coordinator cleanup: **718,225,698 bytes (0.718 GB)**. All executables and
summaries are staged outside it.
No owned build, GPU process, lease waiter or polling job remains.
