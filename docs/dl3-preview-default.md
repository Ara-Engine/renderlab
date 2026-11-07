# DL3: development preview default

Base **156405d**, branch **t3/dl3-preview-default**, 2026-11-07.

The development viewer now selects helper-boundary preview compilation automatically
when the Vulkan interop device is available. `--shader-preview optimized` selects the
ordinary device/reload path. Unsupported interop returns to ordinary compilation;
device initialization errors are logged before attempting that fallback. Library
`ShaderWatch` defaults and release viewer defaults remain ordinary optimized
compilation. Preview remains transient, visibly marked, and replaced in the background.

## Call-boundary screen

The ignored `gpu_vulkan_helper_boundaries` test compiles fresh, unique standard
program sources on raster and traced-compute pipelines. It changes only SPIR-V
`DontInline` hints through the program compiler. No WGSL helper/shading body is edited.
Each candidate has two repeats; pipeline caches are disabled for this diagnostic.
These native-call screens isolate compiler work and are **not edit-to-image timings**.
Only a retained candidate proceeds to the full five-repeat M5-DL method.

Initial driver medians in ms (two repeats):

| Boundary choice | Graphics | Traced compute |
|---|---:|---:|
| DL2 environment set | 259.653 | 351.197 |
| + `shade` | 256.819 | 348.543 |
| + `surface` | 258.855 | 367.515 |
| + `brdf` | 249.689 | 326.946 |
| + `material_normal` | 248.466 | 369.063 |
| + `light_sample` | 270.025 | 385.865 |
| + `exposed_product` | 226.803 | 327.634 |
| + `hard_shadow` | 245.974 | 365.441 |
| + `unit_normal` | 232.545 | 335.978 |
| + `surface`, `shade` | 282.212 | 349.050 |
| + `surface`, `shade`, `material_normal`, `brdf` | 271.962 | 345.880 |
| Only `environment_bilinear` | 903.241 | 1,144.090 |
| Only `environment_radiance` | 8,389.583 | 8,523.373 |
| Only bilinear + radiance | 927.818 | 1,124.816 |
| Only `environment_tap` | 504.576 | 605.971 |
| All non-entry functions | 312.710 | 449.623 |

The reduced-entry idea is tested by retaining `surface` and `shade` call boundaries:
their existing generated entry calls remain small instead of inlining those bodies.
It supplies no material win. Retaining only a top environment function makes its
remaining expanded body dramatically more expensive. The promising individual
additions are small relative to variance, so their combinations require another screen
before adoption. No speed claim is made from these two-repeat observations.

The screen completed before the game started. Its resource samples recorded foreign
compiler activity in 8/15 samples. Evidence is **E:/renderlab-work/dl3/boundary-a/**;
the source hashes, native/API/module durations and shader sizes are recorded per call.

A CPU-only call-site replay explains why naming a small outer function is insufficient:
`environment_radiance` has two `environment_level` call sites; each level has three
conditional bilinear sites, each bilinear has four taps, and each tap has four
conditional remap sites. Fully expanding every syntactic branch therefore yields
**2 × 3 × 4 × 4 = 96 remap code sites**. This is not 96 runtime samples or an ISA
measurement; branches and driver optimization determine actual execution. The saved
call graph and source hash are in `environment-callgraph.json`. M7 owns any change
to that graph. This unit adjusts compiler boundaries only.

The three-repeat combination screen during the relaxed hold is **contended diagnostic
data, invalid for speed claims**. Graphics ranges reach 268–1,059 ms depending on
candidate/repeat; even the control varies from 315 to 963 ms. No production hint is
changed on that evidence. The post-game five-repeat comparison below determines the
retained set. The test accepts
`RENDERLAB_BOUNDARY_COMBINATIONS` or `RENDERLAB_BOUNDARY_FINALISTS` to select those
smaller screens and records per-call source identities and timestamps.

## Retained compiler policy

After the hold was released, the quiet paired screen (`paired-final-b`,
16:13:13–16:13:26 UTC) compiled both hot graphics pipelines concurrently for each
fresh source. Each row has five unique edits; no compiler or foreign GPU process was
observed during the lease. API wall time covers both pipeline creations, not an image.

| Helper boundaries / linking | Paired API median (range), ms |
|---|---:|
| DL2 environment set | 279.600 (237.728–296.225) |
| Environment + GPL | 270.945 (230.425–305.247) |
| Environment + GPL without retained fragment LTO IR | 252.717 (226.271–292.556) |
| Environment + `exposed_product`, `unit_normal` | 238.718 (204.705–255.422) |
| Same + GPL | 225.323 (197.217–259.063) |
| Same + GPL without retained fragment LTO IR | 230.770 (193.872–243.073) |
| Environment + `exposed_product`, `brdf`, `unit_normal` | **231.004 (208.959–263.132)** |

The last row is retained: a 17.4% lower paired median than the DL2 hint set on this
screen, while preserving the existing monolithic pipeline path. GPL builds only one
fresh fragment library and links it twice, but its additional gain is small and the
ranges overlap. That extra production cache/retention policy is not adopted. The
private test scopes retain these experiments for reproduction; public GPL modes keep
their existing retained-IR behavior. All optimized replacements use original SPIR-V
and ordinary optimization, without the preview's `DontInline` changes.

Overlapping module creation is not retained: the M5 viewer needs one raster module
before either hot graphics pipeline can start, and has no additional cold module
creation to overlap with those calls. DL1/DL2's unchanged frontend evidence places
that whole module/reflection portion at roughly 8–9 ms. Even perfect removal of it
cannot close the remaining >100-ms driver gap. More complex multi-module scheduling
would add verification and failure-path cost without solving this measured workload.

## Edit-to-image result

The final quiet window is **17:11:35.171–17:13:40.010 UTC** (`window-retry-0`).
All **five standard and five user repeats are clean**, with forty successful actions.
The 100-ms process observer records **0/1,007 build samples**; the independent lease
guard records **0/29 foreign-build samples**, and its 1,064 fast resource samples
record zero rustc processes. Minimum available RAM is **10,108.2 MiB**. No game or
foreign renderer is detected. Normal OS/desktop activity remains; this is not a
fixed-clock laboratory claim. The foreground window handle/PID is unchanged at the
lease endpoints. The viewer closes and the lock is released before the quiet handoff.

Milliseconds, median (min–max), five repeats per row. Completion means the first
image's GPU completion observed by the viewer; present means its present-call proxy,
not scanout. The optimized endpoint includes background replacement work after the
preview. Image readback/PNG encoding is outside these endpoints.

| Program / action | Save to preview completion | Save to preview present call | Save to optimized completion |
|---|---:|---:|---:|
| Standard / fresh edit | **293.791 (289.977–358.675)** | 277.153 (275.479–341.161) | 1,379.408 (1,346.169–1,462.057) |
| Standard / restore | 93.414 (84.253–104.142) | 44.445 (34.883–47.781) | 135.113 (118.058–139.383) |
| Standard / recovery | 100.764 (69.364–117.708) | 45.247 (34.054–61.924) | 152.011 (106.528–161.828) |
| User / fresh edit | **319.932 (300.011–350.928)** | 289.712 (287.452–334.533) | 1,426.534 (1,388.601–1,543.836) |
| User / restore | 110.543 (93.414–126.117) | 52.696 (32.524–65.756) | 139.423 (128.557–143.639) |
| User / recovery | 100.498 (87.786–111.837) | 40.245 (34.556–48.155) | 143.119 (126.242–162.461) |

Every fresh edit changes the full HDR hash. All ten invalid saves retain the last
working image, and every restore/recovery returns exactly to its baseline hash.
Restores/recoveries reuse previously compiled sources; their timings do not establish
fresh-edit latency. Rejection worker time is recorded separately from save-to-image.

Before/after fresh-edit medians, standard / user, in ms:

| Component | M5-DL | DL2 helpers | DL3 default helpers |
|---|---:|---:|---:|
| Save to detection | 34.305 / 37.120 | 11.710 / 12.851 | 11.989 / 12.325 |
| Module + hot pipelines | 1,038.546 / 1,051.737 | 317.119 / 321.652 | 262.192 / 269.959 |
| Module/reflection portion | Not isolated | 7.614 / 8.791 | 7.262 / 7.686 |
| Two hot pipelines, concurrent wall | Not isolated | 310.559 / 314.550 | 254.930 / 263.351 |
| Save to completed first image | ~1,083 / ~1,095 | 356.959 / 364.356 | **293.791 / 319.932** |

Component medians are not additive. M5/DL2 are reused historical evidence, with their
documented contention/endpoint limits; they are not paired causal speedup samples.
The same-session quiet paired screen above establishes the compiler-policy effect.
No timing taken during the game hold is used in these result tables.

The **100-ms target remains missed by 193.791 ms standard and 219.932 ms user** at
the median completed-image endpoint. In the isolated quiet Lit-pair screen, the
retained policy's slowest native call per pair has median **219.194 ms** and range
**198.611–251.265 ms**; the shortest individual fresh `vkCreateGraphicsPipelines`
call is **192.166 ms**. These are observed compiler floors for this screen, not a
universal lower bound. In the actual viewer, hot-pair creation still dominates at
255–263 ms. Optimized replacement pipeline creation remains **1,037.096 / 1,080.706
ms** in the background, with modules **3.779 / 4.394 ms**.

The next material option is M7's smaller equivalent environment helper graph, tested
against the same full-image and motion gates. More extensive `DontInline` and GPL
subsets did not remove this floor. Parameter updates and source caches can avoid
compiling repeated values/sources; they do not solve unseen shader bodies. Separate
backend mechanisms require a measured fresh compiler result before adoption. No
helper body or rendered quality is reduced to claim 100 ms.

## Verification protocol

The final window run uses [M5-DL](m5-dev-loop.md): full-resident quarry, static camera
0, 1920x1080, 1x, 16x anisotropy, original lighting/environment, development profile,
active Vulkan validation, five unique visible edits per standard/user file, each
followed by restore, invalid save and recovery. Save-to-first-preview GPU completion,
present call and optimized completion are separate endpoints. Readback/PNG encoding
happens after those endpoints. No explicit preview flag or control-file command is
used in the default timing run.

An untimed quality run queues an HDR copy of the actual first-preview revision and
compares it to the optimized replacement and exact-source ordinary reference. The
ordinary CLI option gets a separate inactive fixture check without the interop
measurement environment. Library default behavior remains covered by existing tests.

The host is the DL2 Windows/Vulkan configuration: RTX 4070 SUPER, NVIDIA 617.14,
Ryzen 7 7800X3D, Rust 1.96, wgpu 30.0.1, development profile and Vulkan validation.
The full-resident quarry has 13,034,903 triangles. This measures the existing raster
development viewer; traced compilation and API correctness have separate fixtures.
No startup, Rust-edit latency or frame-time speedup is inferred from shader timings.

The retained implementation checkpoint is **eb12ed6**. The library suite reports
**123 passed, 121 ignored, exit 0**. Final formatting and library/viewer all-target
Clippy pass with the same two pre-existing, out-of-ownership exceptions documented
by DL1/DL2 (`nonminimal_bool` in `virtual_shadow_gpu.rs` and
`items_after_test_module` in `visibility_buffer.rs`). Strict repository-wide Clippy
therefore remains a baseline miss. Those files are unchanged.

The actual first-preview quality run (`quality-b`, 16:42:08–16:42:45 UTC) asserts the
captured revision is the preview revision before allowing optimized promotion. Both
standard and custom user previews, and both optimized replacements, differ from the
exact-source ordinary monolithic reference by **0/2,073,600 HDR pixels**, maximum
and mean absolute error **0**. Complete quarry PNGs, including silhouettes, were
inspected. An earlier quality attempt captured the optimized revision instead; its
assertion failed and it is excluded. The opt-in untimed probe now waits for a flight
slot and queues the exact preview copy before further rendering/promotion. That pause
is absent from normal development and all timed runs.

The registered-program GPU fixture passes raster and traced compute, all four preview
policies, optimized promotion, invalid/stale saves, recovery, parameters/defaults and
in-flight old-frame use. All eight preview/replacement cases differ from the ordinary
reference by **0/4,096 HDR pixels**, with no validation errors. A separate fixture
launch verifies `--shader-preview optimized` uses no interop device, emits no preview
stage, and still reloads a visible edit. These functional checks are contended and
provide no speed claim.

Zero difference on these sources is output preservation, not a guarantee for arbitrary
user programs or other drivers. Helper-boundary preview remains a transient dev-only
state and is replaced with the original optimized pipeline in the background. Release
callers and library defaults do not opt into it.

## Game hold and queued work

No DL3 GPU lease or viewer was active when the game hold arrived. The CPU guard
stopped the in-progress preparation build when CS2 appeared. Its interrupted Rust
incremental cache caused a subsequent link failure; the worker's own incremental
directory was removed before rebuilding, without deleting shared dependency artifacts.
The later preparation of the final comparison was also stopped when the configured
four-GiB relaxed-hold RAM floor was crossed (minimum 4,089 MiB). Its legacy failure
string says two GiB; the evidence record explicitly corrects the threshold to four.
The later post-game preparation and final checks complete successfully.

Under the explicitly relaxed hold, only short, headless compilation screens run at
below-normal CPU priority. They use the shared atomic GPU lease, the reservation and
release log, six-GiB launch/four-GiB runtime memory floors, and no Cargo inside a lease.
All timings during play are labelled contended and excluded from speed claims. After
the explicit release at 15:36Z, viewer runs use inactive windows, renderer-owned image
copies and WM_CLOSE for the owned viewer. No focus/input automation or desktop/game
captures are used. Every GPU run uses the shared atomic lock and reservation/release
lines, at least 4 GiB available RAM at launch and a 2 GiB runtime abort floor, with no
Cargo inside an owned GPU lease.

Foreign `mypaper_wallpaper` compilation repeatedly interrupted the final quiet window.
Only owned jobs were stopped. The foreign processes were never paused or terminated.
The requested protocol waits for 60 seconds without Cargo/rustc/link activity before
reserving another quiet window; after roughly 30 minutes of restarts it permits clean
gaps, with each repeat's contention recorded and only clean repeats retained. Rejected
attempts and their partial results remain in the evidence root.

The final uninterrupted quiet run succeeds after the 60-second wait, so clean-gap
fallback is **not needed**. All ten retained repeats are clean; no contended repeat
is used in the edit-to-image table. The earlier aborted standard-only prefix is also
excluded. U0b receives the explicit quiet completion/handoff after the final viewer
closes. DL3 has no further GPU job queued.

## Evidence and development cost

Evidence is outside Git at **E:/renderlab-work/dl3/**: `final-summary.json`,
`window-retry-0/results.json`, its `runs/` lease/resource records, `paired-final-b/`,
`quality-b/quality.json`, `api-final/preview-images.json`, `ordinary-option/`, the
measurement scripts, source identities and build/check logs. These preserve every
sample, action/source hash, compiler breakdown and contention label. Failed attempts
remain labelled and are excluded from claims.

The measured viewer SHA-256 is
`03ac2fa11e21a29a168959dae38cc8f7a0ed7a26a139f20c93b940ff5e045075`.
Its unchanged standard baseline HDR hash is
`1c4462b43466188d58f0e6e47ef03060e8adb64a1c0e8f3aff88e470172c12f1`;
the custom baseline remains
`10ee30f19e0f50ecde34c3cca32c3bc806e539aa9eba73f2d0cd1c259ae33690`.

Implementation/analysis time was not independently stopwatched and interleaves with
game, queue and foreign-build waits. Recorded CPU preparation/checks total
**718.014 s**, including first dependency compilation and aborted preparations; the
final fmt/Clippy/viewer-build batch is **8.776 s**. Owned GPU lease holds total
**401.308 s**, including CPU driver compilation, loading, readback, failed attempts
and the contended diagnostic screens. These are verification wall intervals, not GPU
execution time or shader edit-to-image time. The retained final window alone takes
**124.838 s**; the quiet paired compiler screen takes **12.711 s**. Foreign-build
waiting is separate from those holds. Large raw dumps, uninspected PNGs and this
worker's Cargo target are removed after summarization; compact evidence and inspected
images remain on E:.

The M7 helper-graph proposal remains with M7. This unit does not edit environment,
lighting, shading, GI/temporal or shadow bodies, or the M5/M6 decision TSV.
