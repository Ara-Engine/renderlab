# RT1: Interactive false shadows and the RT default

2026-11-07 UTC. Base `bce1e39`, branch `t3/rt1-interactive-default`.

**RT1 is incomplete. No production rendering change or default promotion is
retained.** F0's recorded miss mixes an obsolete receiver origin with
pixel-footprint integration error. Two of the frozen Interactive witnesses
additionally cross the false-shadow threshold because of the eight-direction
estimate. Changing that prefix alone cannot remove the shared old-origin miss;
agreement with Exact or the point reference does not by itself identify its cause.
A correction satisfying every static/motion measure at the recorded cost has not
been established. Current-source measurements still fail **27/26 versus VSM's
11/9**; the targeted point replay leaves 15/16, with 13/14 outside the repair strip.

The library/viewer RT-device default remains raster primaries with
`SoftShadows::Exact` (256 directions). Traced primaries and Interactive remain
explicit settings. Every RT speed claim below is **measured on the non-default
fast setup (traced + Interactive)**. No speed for the default is inferred from it.

## Reproducer and cause

`scripts/rt1-film-replay.py` reads the frozen F0 defect enumerations, film
references, original centre Exact counts, independent 16,384-direction point
reference and decoded Unreal masks. It reproduces the original Exact and native
partial RMS/false counts, without launching a renderer or changing a reference.
Every original Interactive false pixel is retained in the report and cropped.

| Pose / F0 reference | Interactive false | Exact also false at these witnesses | 16,384-ray point also false | Unreal VSM false, whole domain |
|---|---:|---:|---:|---:|
| 0 / policy | 9 | 9 | 9 | 9 |
| 0 / minimal | 21 | 20 | 20 | 11 |
| 1 / policy | 11 | 11 | 10 | 10 |
| 1 / minimal | 17 | 17 | 17 | 9 |

These are four overlapping populations, not 58 independent pixels. The
16,384-ray point reference itself has **9/10** false shadows on the policy
reference and **20/17** on the minimal reference. Even eliminating all prefix
noise leaves the minimal-reference miss above VSM's **11/9**.

The two extra estimator witnesses are:

- Pose 0, **(974,640)**, minimal: film 0.994665, point 0.965515,
  Exact 0.953125, Interactive 0.875.
- Pose 1, **(1169,384)**, policy: film 0.996094, point 0.903320,
  Exact 0.898438, Interactive 0.875. Exact also crosses 0.9 here.

Most other witnesses are completely shadowed at the **old displaced centre** and
almost completely lit in the footprint average. Inspected 17x17 crops show
isolated centre shadows and narrow chains. This is consistent with the earlier
[film-gap diagnosis](m6-film-gap.md), but does not distinguish a displaced-origin
defect from physical footprint aliasing on those particular receivers.

Three independent CPU source-ray probes additionally find occlusion from a
different scan placement, and select the recorded receiver triangle for camera
rays through its saved world point:

| Pose-0 pixel | Receiver mesh / primitive | Light-axis blocker mesh / primitive | Distance from the 1 mm displaced origin |
|---|---|---|---:|
| (1002,491) | 9 / 361644 | 8 / 473501 | 0.4065 mm |
| (910,635) | 4 / 362512 | 2 / 625053 | 0.3504 mm |
| (1212,653) | 7 / 275323 | 11 / 298585 | 2.1003 mm |

Mesh indices here are zero-based; captured placement words are one-based. These
are geometric corroboration using the saved world points and the source loader's
floating transforms, **not exact replays of unavailable F0 hardware ray bits**.
The disk-axis ray is not one of the sampled disk directions. Positive blocker
hits establish small intersections from the displaced ray. At **10 µm**, all three
disk-axis probes find no blocker. A missing ray does not prove the whole disk is
clear, and this is not an exact hardware-bit replay, but it specifically implicates
the obsolete 1 mm origin at these witnesses. The current policy must be checked
before treating them as surviving physical contacts. Same-instance shared-edge
repair does not change these other-instance intersections.

### The later receiver policy must not be confused with F0

F0 uses the old 1 mm light-side policy and a separate fixed-10-um film diagnostic.
Its saved centre captures are unchanged between those references. In particular,
R1's `quarry-viewed-p*/pose-*.exact.u32` exports the **frozen old centre**, not the
adopted production centre. It cannot be used as proof that current output is
unchanged. [R1](m6-receiver-policy.md) documents this explicitly.

[Receiver adoption](m6-receiver-adoption.md#film-results) already records the
new-policy Interactive result against its own reference: **27/26 false shadows**
(Exact **15/18**), versus F0's **9/11**. Populations and the estimand changed, so
these are not an apples-to-apples regression delta. They also mean the old 9–21
range in the acceptance record is historical evidence, not a current pass.

The viewed-side rule, explicit material thin flag, bounded origin displacement,
adjacency repair and explicit primary-ray FMA are retained. No light-facing
primary substitution, global bias increase, discarded receiver, widened threshold,
or Standard-material-only visibility decision is introduced.

### Current-source result (M)

The 39.35 s correctness lease loaded the original quarry once and ran the
production shaders from this branch. Complete hit/count/HDR hashes match across
eight repeats per pose. **Zero** of 2,073,600 count pixels differ between fused
Interactive and the separate eight-direction control at either pose; **zero**
primary hit records differ between Interactive and Exact. Thus there is no
reproduced fused handoff/FMA discrepancy at these two poses. Both paths retain
the same geometry-only repair; this does not independently prove every shared
primary decision.

Only **4/9, 2/21, 5/11 and 1/17** of the original four F0 witness lists still
cross the threshold in the current output. The three CPU origin witnesses above
are all cleared. Current failures must be enumerated afresh, rather than treating
all 9–21 old coordinates as surviving defects.

Against the adopted viewed-side film reference, the current fast setup still
has **27/26** false shadows versus VSM's **11/9**. Exact has **15/18**. These
static results prevent promotion even before any new motion campaign. At pose 0,
partial RMS is .207157 rather than the receiver-adoption checkpoint's .207204;
pose 1 matches its .208537. The current edge repair does not remove this count miss.

| Quarry, viewed-side film reference, poses 0/1 | Previous fast setup (receiver adoption, R) | Current fast setup (M) | Current Exact (M) | Unreal VSM (M replay) | Unreal HRT1 (M replay) |
|---|---:|---:|---:|---:|---:|
| Partial RMS | .207204 / .208537 | .207157 / .208537 | .203296 / .204690 | .398791 / .399889 | .370873 / .371257 |
| False shadows (film >= .99, output <= .9) | 27 / 26 | **27 / 26** | 15 / 18 | **11 / 9** | 743 / 758 |
| Missed film shadows | 9,245 / 9,166 | 9,245 / 9,166 | 8,570 / 8,484 | 27,071 / 27,070 | 23,931 / 23,789 |
| Solid flips | Not tabulated there | 0 / 1 | 0 / 1 | 4,129 / 4,118 | 3,306 / 3,215 |
| Umbra leaks | Not tabulated there | 0 / 0 | 0 / 0 | 5,537 / 5,524 | 2,718 / 2,616 |
| Lit darkened | Not tabulated there | 0 / 2 | 0 / 2 | 5 / 4 | 724 / 736 |
| Lost umbra edges | Not tabulated there | 0 / 0 | 0 / 0 | 3,825 / 3,810 | 2,565 / 2,461 |
| Missing covered receivers | 0 | 0 / 0 | 0 / 0 | 20 / 26 | 20 / 26 |

These are film-reference populations from the unchanged gate code; the lit/umbra
definitions differ from the relaxed false/missed-shadow thresholds. Native missing
receivers remain missing, rather than being filled with zero error. Native scores
use their valid domain; `current-replay/replay.json` preserves availability counts.
The native reference has 49,922 / 49,881 available partials against Renderlab's
49,928 / 49,889. These rows do not certify the unavailable native pixels.

Against F0's **obsolete 1 mm** policy reference, current Interactive false counts
are 394/387; against its fixed-10-µm light-side reference, 28/30. Those mixed-policy
scores are diagnostics, not valid current-policy quality acceptance. Both native
comparators are rescored against the adopted reference in the table above.

### Current failures: prefix noise versus the centre estimand (M)

One additional 24.00 s targeted lease evaluates **only these 53 current failing
pixels**, with the existing `gpu_receiver_solid_verification` test: 4,096
independent salts, 256 directions each, or **1,048,576 directions per receiver**.
This reuses production receiver reconstruction and has no film-primary jitter.
The pose-0 return statistics are byte-identical. It does not generate a new full
point-reference image or rerun the fixture acceptance campaign.

| Current false-shadow witnesses | Pose 0 | Pose 1 |
|---|---:|---:|
| Interactive failures selected | 27 | 26 |
| Still <= .9 in independent point estimate | **15** | **16** |
| Point estimate > .9: eight-ray threshold errors | 12 | 10 |
| Persistent failures outside the current four-epsilon edge strip | **13** | **14** |
| VSM whole-domain false count on this film reference | **11** | **9** |

All point estimates lie on the same side of .9 even after adding/subtracting
1.96 replica standard errors. These are estimated sampling errors, not a formal
geometric bound. The CPU edge-strip replay uses the saved current barycentrics,
source vertices/transforms and production's epsilon/altitude rule. It does not
assert exact reconstruction of hardware ray bits.
All 53 selected failures have available native VSM outputs, so this residual
comparison is not caused by native missing receivers.

Even perfect removal of **both** the eight-ray threshold errors and all persistent
near-edge failures leaves at least **13/14** of the selected false shadows, already
above VSM's **11/9**. Some persistent receivers have viewed-side N·L inside the
0.04-radian disk, including negative values: their point estimate samples grazing
microfacets while the film average mostly samples different facets. Other points
remain shadowed by nearby geometry. Primary footprint integration is the remaining
fix class supported by this evidence; fusion/stratification/edge repair alone
cannot remove this residual while retaining the current receiver estimand.

## Fix decision

**No new shadow-only correction is accepted.** More centre rays with the frozen
old origin cannot resolve its minimal-reference failure even at 16,384 directions.
The 10-µm production origin is already implemented; RT1 must not claim it as a new
fix or assume its output from F0's frozen counts. Replacing a nearer back face
with a farther lit/front face was already disproved by
[the back-face retry screen](m6-backface-retry.md). Removing neighbouring scan
geometry to clear a displaced-origin ray would change authored scene visibility.

The established fix class is full-image footprint integration, including each
primary sample's programmable shading, as described in [the film-gap design](m6-film-gap.md).
That is a direction for the M7/temporal owner, not an implemented or accepted RT1
fix. A visibility-only footprint average attached to a different centre surface
does not integrate the product of material, geometry and visibility and would
need its own final-radiance/contact/motion proof. RT1 does not edit the M7 files.

The retained implementation is diagnostic only: `gpu_rt1_film_defects` loads the
quarry once, explicitly pins Exact for the reference export, evaluates the same
eight directions in a separate pass, and captures production Interactive with
eight repeats at both poses. It compares complete hit/count/HDR hashes and
records every fused/separate count or primary identity difference. It is ignored
by ordinary CPU tests and needs the GPU lease/experimental acknowledgment.

## Per-measure ledger

**R** means reused recorded evidence, **M** means replayed here. No production
algorithm changed, so the “retained” column is the previous fast setup; it is not
a new all-measure acceptance run. Static F0 witnesses are M; all other rows cite
their original workload and limits. Smaller is better except speed ratios.

| Measure / workload | Previous fast setup | RT1 retained setup | Unreal comparator | Status / evidence |
|---|---:|---:|---:|---|
| Quarry film partial RMS, policy, poses 0/1 | .234188 / .234588 | Same algorithm; no corrected candidate | VSM .370607 / .371869; HRT .345569 / .345214 | R, F0 |
| Quarry film partial RMS, minimal | .252656 / .252437 | Same algorithm; no corrected candidate | VSM .402986 / .404545; HRT .375384 / .376230 | R, F0 |
| Quarry missed film shadows, policy | 9,768 / 9,682 | Same algorithm | VSM 24,049 / 24,059; HRT 20,940 / 20,817 | R, F0 |
| Quarry false shadows, policy | 9 / 11 | No fix | VSM 9 / 10; HRT 757 / 770 | M witness replay; miss at pose 1 |
| Quarry false shadows, minimal | 21 / 17 | No fix | VSM 11 / 9; HRT 743 / 757 | M witness replay; miss at both poses |
| S1 partial RMS, near/far | .087840 / .086842 | Same algorithm | VSM .161977–.164364; HRT1 .161410–.177903 | R, S1, both environments |
| S1 lost contacts | 0 / 0 | Same algorithm | VSM 2–6; HRT1 26–34 | R, S1 static distances |
| S1 umbra leaks | 0 | Same algorithm | VSM 0; HRT1 348–448 | R, S1 |
| S1 solid flips | 0 | Same algorithm | VSM 0–1; HRT1 6–16 | R, S1 |
| S1 HDR RMS, black world, near/far | .003513 / .003176 | Same algorithm | VSM .004533 / .003998; HRT1 .004606 / .004280 | R; constant-environment final-image background mismatch remains |
| S2 disk, all moving tracks, partial RMS | .07424–.07517 | Same algorithm | VSM .16969–.17555; HRT1 .13731–.14447 | R, S2 crop |
| S2 disk, camera .25 px/frame, HDR RMS | .01045 | Same algorithm | VSM .01508; HRT1 .01485 | R, S2 |
| S2 disk, .25 px/frame, lost contacts | 0 | Same algorithm | VSM 2,101; HRT1 2,498 | R, pixel-frame counts |
| S2 disk, .25 px/frame, max leaks / flips | 0 / 0 | Same algorithm | VSM 62 / 167; HRT1 126 / 195 | R, per-frame maxima |
| S2 hard-source motion, partial / HDR RMS | .27330 / .01141 | Same hard-source algorithm | VSM .31736 / .01455; HRT1 .37890 / .01701 | R; Interactive does not run an eight-ray disk here |
| S3 sun rotation 1 degree/s, partial / HDR RMS | .07790 / .01414 | Same algorithm | VSM .18628 / .02001; HRT1 .25387 / .02712 | R, S3 crop |
| S3 rotation, lost contacts | 0 | Same algorithm | VSM 12,289; HRT1 23,005 | R, pixel-frame counts |
| S3 rotation, max leaks / flips | 0 / 0 | Same algorithm | VSM 106 / 192; HRT1 126 / 200 | R |
| S3 jump, mask / HDR lag90 | 0 / 0 frames | Same algorithm | VSM 0 / 1 | R; diagnostic, not a film-quality pass |
| S2 moving flicker, camera tracks | .0132–.0191 | Same algorithm | VSM .0605–.0615 | R, M6 acceptance replay |
| S3 moving flicker, rotations | .0025–.0050 | Same algorithm | VSM .080–.082; HRT1 .123–.124 | R, M6 acceptance replay |
| Quarry shadow increment, static/sun/camera/caster | .276 ms | No new timing claim | HRT .626–.655; VSM RT-on .497–.935 by phase | R; measured on the non-default fast setup (traced + Interactive) |
| Quarry complete GPU span, static/sun/camera/caster | .867 / .867 / .868 / .865 ms | No new timing claim | HRT 1.738–1.778; VSM RT-on 1.554–2.039 by phase | R; measured on the non-default fast setup (traced + Interactive) |
| Quarry allocator reserved / WDDM | 2,641.2 / 2,716.3 MiB | No resource change | HRT DXGI 5,407–5,410; VSM RT-on 3,946 | R; allocator and driver counters differ |

F0's full numerical rows are in [m6-film-reference.md](m6-film-reference.md).
S1's static radiance/background and coverage limits are in
[bench-contact-fold.md](bench-contact-fold.md); its contiguous motion track was
not certified. [S2/S3](bench-motion.md) supply moving crop evidence, not an
unrestricted full-frame film gate. Quarry point-reference solid flips, lit-darkened,
umbra leaks and lost-edge diagnostics from the current-source capture are reported
separately from film-defined metrics so their populations are not conflated.
No claim of passing every static and motion measure follows from the reused rows.

## Checks, evidence and reproduction

Evidence root: `E:/renderlab-work/rt1/`. Bulk data never enters Git.

```powershell
py -3 -B scripts/rt1-film-replay.py --output E:/renderlab-work/rt1/frozen-replay
# Optional geometric corroboration (CPU only, read-only source asset):
py -3 -B scripts/rt1-film-replay.py --output E:/renderlab-work/rt1/source-replay --source-check <full.rla>
# Compile/run the ordinary CPU checks outside any owned GPU lease, two Cargo jobs.
cargo test -p renderlab --lib -j 2 --target-dir E:/renderlab-work/rt1/target
# Use the resulting test executable directly, never Cargo, under the shared runner:
$env:N8_OWNER="rt1"
$env:N8_OUT="E:/renderlab-work/rt1"
py -3 -B scripts/n8-run.py --label current --minutes 4 --fast-check --manifest <manifest.json> -- <test.exe> gpu_rt1_film_defects --ignored --nocapture --test-threads=1
# Then replay saved buffers, without a GPU lease:
py -3 -B scripts/rt1-film-replay.py --current E:/renderlab-work/rt1/current --output E:/renderlab-work/rt1/current-replay
# The point lease uses gpu_receiver_solid_verification with a manifest pointing
# output at point-reference/, containing pose-{0,1,2}-pixels.u32 (flat u32 IDs).
py -3 -B scripts/rt1-film-replay.py --current E:/renderlab-work/rt1/current --point-capture E:/renderlab-work/rt1/point-reference --geometry-asset <full.rla> --output E:/renderlab-work/rt1/final-replay
```

The manifest uses the existing private `asset`, `limits`, `cameras`, `lighting`,
`environment` and `output` fields. Experimental hardware-ray acknowledgment is
provided by the guarded runner. The runner owns the token-checked atomic
`D:/Coding/renderlab-work/gpu-slot.lock`, the shared reservation/release lines,
no-game checks and the 4 GiB start / 2 GiB in-run memory guard. No windows are used.

CPU batch: **116 passed, 0 failed, 114 ignored**, exit 0 (139.04 s including the
cold two-job compilation; test execution 0.61 s). `cargo fmt --all --check` passes
(2.21 s). Strict Clippy fails on the existing `nonminimal_bool` diagnostics at
`src/virtual_shadow_gpu.rs:203` and `items_after_test_module` at
`src/visibility_buffer.rs:1076`. Those files are unchanged. Clippy with the same
two explicit baseline allowances used by M1/N8o passes; this is **not a clean
strict Clippy result**. Logs retain both invocations.

Verification conditions and latency are separate from implementation:

- **Hardware/build:** RTX 4070 SUPER, NVIDIA 617.14, Windows/Vulkan, wgpu/naga
  30.0.1, 1920x1080, 1x, original 13,034,903-triangle quarry, 16x anisotropy,
  0.04-radian disk. Development/test profile (library opt-1), validation active;
  these are correctness captures, not optimized performance measurements.
  Executable SHA-256: `3a965e447d7f8838b3c84ae0996f708a4170d5c51f903dfe8287244969ba99a3`.
- **Current capture:** 12:33:23–12:34:02 UTC, 39.35 s guarded / 34.21 s test.
  Start RAM 10,377 MiB; sampled minimum 6,620 MiB. Foreign Cargo processes and
  up to one rustc were observed. RT1 ran no Cargo in its lease.
- **Selected point reference:** 12:35:47–12:36:12 UTC, 24.00 s guarded / 20.58 s
  test. Start RAM 8,844 MiB; sampled minimum 5,573 MiB. Foreign compilation reached
  19 rustc processes. No new timing result is inferred from either contended lease.
- Both tests exit 0 with `errors=0`, `missing_layer=0`, `active_layer=1`.
  The two owned leases total **63.35 s** and are released; one prior attempt
  deferred on the foreign lock without running. No game, window, input automation,
  desktop capture, memory abort or owned compiler occurred during either lease.
- **CPU replay:** frozen replay 4.81 s; source-ray corroboration plus replay
  46.70 s; current full score replay 11.42 s; final replay including the selected
  point and geometry classification 12.47 s. Later compact-metadata extraction
  took 6.46 s. These are analysis/verification latency, not frame times.
- **Implementation/analysis:** reading, the diagnostic test/script and docs were
  interleaved with queue waits and the checks above; editing time was not separately
  stopwatched. There is no retained optimization whose implementation time is
  represented by its verification time.

Inspected offscreen: `frozen-replay/pose-0-minimal-false.png`,
`final-replay/pose-0-current-hdr.png` and
`final-replay/pose-1-viewed-false.png`. The full frame includes the outer silhouette,
rubble, contact folds and wall shadows. This is static diagnostic inspection, not
new transparency or motion acceptance. Final summaries preserve per-pixel raw hit
words, estimates, errors, input/output hashes and the selected point statistics.
The owned Cargo target was removed with `cargo clean --target-dir
E:/renderlab-work/rt1/target` (**2,294 files, 1.1 GiB**, target absent afterward).
Automatic approval review rejected cleanup of the twelve raw capture files,
including an explicit non-recursive `Remove-Item -LiteralPath` command, with
**"blocked by policy"**. Those files remain on E:; `deleted-raw.json` is their
planned-deletion inventory with hashes and sizes, not a successful deletion log.
No rejection was bypassed. Summaries, inspected PNGs and selected point records
are retained independently of the raw files.

## Proposed decisions rows (root owns the TSV)

`docs/milestones-5-6-decisions.tsv` is unchanged.

```tsv
2026-11-07	RT1	Keep raster primaries and 256-ray Exact as the RT-device default; Traced plus Interactive remains opt-in	Current own-reference false shadows 27/26 vs VSM 11/9; independent 1048576-direction point replay leaves 15/16, including 13/14 outside the repair strip; zero fused/separate differences; the F0 origin-era witnesses are mostly obsolete	Single-upload current capture and targeted point lease, CPU film/edge replay; docs/rt1-interactive.md	open; no default promotion
2026-11-07	RT1	Label RT speed comparisons as measured on the non-default fast setup (traced + Interactive)	The measured 0.276 ms shadow increment and roughly 0.867 ms frame span do not describe the raster plus Exact RT-device default	Recorded M6-close/M5-DL figures; no new performance claim; docs/m6-acceptance.md, docs/m5-dev-loop.md, docs/memory-m1.md, docs/memory-m2.md, docs/memory-m3.md	corrected scope
```
