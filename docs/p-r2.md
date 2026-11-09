# P-R2: workgroup reflection ray compaction — rejected at the gate

Unit P-R2, 2026-11-09 UTC (2026-11-10 Qatar), branch `t3/p-r2`, based on
`origin/main` `9fbc022`. **DON'T MERGE a renderer implementation.** The requested
pre-build rejection gate fails on both target scenes, on both paths. No renderer
implementation was made, built, or screened.

## Gate method

The existing P-NRT2 Nsight traces report `sass_counters.available=false`; their
`active_threads_per_warp=0` is unavailable data, not a measurement of occupancy.
Instead, the prebuilt P-NRT2 `cand3.exe` and `cand3-lib.exe` captured a diagnostic
program, followed by a CPU replay using [gate.py](../tools/p-r2/gate.py).
The production standard program is unchanged between that binary's source
(`a5ae72a` plus P-NRT2) and this branch's base.

The diagnostic retains production `surface` and `reflection_lobe`, computes
`sqrt(clamp(alpha,0,1))`, applies the default 0.5 roughness cutoff, lobe weight
and normal/view rejection, then computes RQ1's exact loop bound:
one below 0.05, otherwise `clamp(ceil(48*rough),4,16)`. It emits exposed HDR
`(count/16,1,0)`. The last two channels distinguish diagnostic geometry from
background and nonparticipating programs. This is a measurement program, not a
production reflection variant or a quality witness. No reflection hits are shaded.

The replay uses x-fastest 8x4 image tiles, including zero-count pixels, and reports
`sum(tile maximum) / sum(tile mean)`. Empty tiles contribute zero to both sums.
These are the loop bounds proposed for the queue, before per-stratum below-plane
rejection. It does not measure variation in BVH traversal length or hit-shading cost.
RF10's mixed-program dispatch currently uses 4x4 groups; its reported 8x4 replay
describes the proposed layout, not current MP1 warp occupancy.

ARA1 uses the first still camera of the frozen 1080p timing scenes, first-frame
TR1 with no history. GI, lights, atmosphere, fog, reflection evaluation and post
effects are disabled for the diagnostic; surface/lobe inputs are retained. PNG
count decoding checks all 17 Filmic/sRGB codes, accepting at most one display
code of rounding. RF uses the registered fixture poses at 1080p with TR1 off,
reading integer count codes from raw HDR; fractional codes cause rejection.


## RF count gate (M)

TR1 off, 1080p; every raw HDR diagnostic count decoded as an integer.

| Fixture / pose | Non-RT ratio | RT ratio |
|---|---:|---:|
| RF1-pitch20 | 1.018365 | 1.018358 |
| RF1-pitch30 | 1.024252 | 1.024252 |
| RF1-pitch40 | 1.017562 | 1.017561 |
| RF10-pitch30 | 1.024662 | 1.024662 |
| RF2-sweep | 1.083637 | 1.083645 |
| RF3-pitch30 | 1.027639 | 1.027639 |
| RF4-pitch30 | 1.070075 | 1.070073 |
| RF5-pitch30 | 1.024252 | 1.024252 |
| RF6-grazing10 | 1.041681 | 1.041681 |
| RF6-grazing5 | 1.051556 | 1.051532 |
| RF7-between-panels | 1.010626 | 1.010626 |
| RF8-pitch30 | 1.026047 | 1.026045 |
| RF9-pitch30 | 1.024252 | 1.024252 |

All 26 fixture/path count captures are below 1.4.

## Decision

| Target scene | Non-RT ratio | RT ratio | Required |
|---|---:|---:|---:|
| game-base | 1.079703 | 1.079703 | >=1.4 |
| ara-pbr-oriented | 1.143350 | 1.143330 | >=1.4 |

The design's expected 0.2–0.4 ms saving from about 0.63 ms of traversal is not
supported by this ray-count imbalance. Even an ideal elimination of loop-bound
imbalance would remove only about 7.4% (game-base) or 12.5% (ara-pbr) of equal-cost
ray work, before queue, barrier and occupancy costs. Those percentages are an
arithmetic interpretation of the gate, not measured traversal-time savings.
Per-ray BVH and hit-shading variability could motivate a different experiment,
but does not override this unit's explicit pre-build rejection rule.

## Memory and quality

Added production workgroup memory: **0 bytes**. No queue was implemented.
The design's 6 KiB radiance estimate needs correction before any future attempt:
WGSL arrays of `vec3<f32>` have 16-byte stride, so 512 results alone occupy 8 KiB.
A packed 16-bit queue adds 1 KiB; an unpacked u32 queue adds 2 KiB. Shared ray
parameters would be additional. f16 radiance is excluded by the byte-identity bar.
No occupancy measurement was needed after rejection.

Renderer source and shaders are unchanged. **Candidate image identity is not
tested:** there is no candidate. ARA1 A/B (`abplan.py` / `summarize.py`), RF quality
identity, Q-walk, MP1 hit-shading identity and the full lib suite were not run, because the
specified rejection gate terminates the experiment before implementation.
Diagnostic RF captures are count measurements, not those quality tests.

| Timing evidence | Non-RT | RT |
|---|---|---|
| Quiet reflection pass before / after | not measured / no candidate | not measured / no candidate |
| Quiet service before / after | not measured / no candidate | not measured / no candidate |

No performance improvement or cross-site arithmetic identity is claimed.
Programmable hit shading and all quality settings in the renderer remain unchanged.

## Operations and reproduction

Evidence root: `D:/Coding/renderlab-work/p-r2`. No Cargo build or target was
created; consequently `D:/Coding/pr2-target` needed no deletion. No worker or
foreign process was stopped. No viewer, window or input automation was used.

The runner stream-reads the schedule, waits for the anchored P-NRT3b owner
release, normally acquires `Global\stillkit-cargo-one-at-a-time`, checks no
foreign cargo/rustc, >=8 GiB available and >=5 GB free on D:, then wraps the
command in `scripts/n8-run.py` through the P-NRT lease adapter. Every lease is
at most 14 minutes, with the 2 GiB runtime guard. The build-slot lock was not
needed because no build was requested; the mutex covers one lease at a time.

The first attempt failed program validation because removing the original shade
body removed the token that composes `EnvironmentLobe`. The diagnostic generator
now retains that unused API dependency. The first RF batch inherited the renderer's
default temporal reconstruction despite its metadata's `tr1:false` (that field
reflects only the explicit `=1` mode). Its fractional count output was rejected;
the retained RF measurement explicitly sets `RENDERLAB_R1_TR1=0`.

```powershell
py -3 -B tools/p-r2/gate.py shaders/programs/standard.wgsl --program --out D:/Coding/renderlab-work/p-r2/counts.wgsl
# Inside a guarded lease: ARA1_STANDARD_PROGRAM=counts.wgsl for ARA1;
# RENDERLAB_R1N_PROGRAM=counts.wgsl and RENDERLAB_R1_TR1=0 for gpu_r0a_current.
py -3 -B tools/p-r2/gate.py D:/Coding/renderlab-work/p-r2 --out D:/Coding/renderlab-work/p-r2/gate.json
```

CPU checks: synthetic uniform tile -> 1.0, one active lane -> 32.0, empty tile ->
no ratio; diagnostic generation reproduced the capture shader; `git diff --check`.

Retained count evidence: `gate.json` (four ARA1 frames, 26 RF pose/path captures),
count PNGs, RF metadata and logs; invalid temporal captures are summarised in
`invalid-rf-summary.json`. Raw RF dumps over 5 MB were deleted after the replay.
Lease counts-1: 3.1 s (validation rejection); counts-2: 254.6 s (ARA1 retained,
RF temporal captures rejected); counts-3: 218.5 s (26 clean RF captures).
The final lease released before P-NRT3b's queued quiet timing work.

Time: approximately 7 minutes of reading, tooling and reporting; about 8 minutes
of active GPU verification and about 15 minutes of queue/mutex waiting. No
renderer implementation or compilation time was spent. Total about 30 minutes,
inside the three-hour time box.
