# M3: texture mip residency

2026-11-07 UTC. Base `f09d2d0`, merged with main `311075c` (N8l, N8k) before the
final checks. Branch `t3/m3-texture-residency`.

**RT scope:** all Interactive RT timings, speed claims and memory rows here are
**measured on the non-default fast setup (traced + Interactive)**. The RT-device
default remains raster + 256-ray Exact; see [RT1](rt1-interactive.md). Traced + Virtual
rows describe a separate explicit opt-in. In this record, "default" describes full
texture residency within each explicitly selected path. Texture `Exact` is distinct
from `SoftShadows::Exact`.

**Merged opt-in, streaming off by default (Islam, 2026-11-07).** `TextureResidency::Full`
is the default in the renderer, the viewer and every test and bench. The default frame
is main's frame: the same textures, the same shading module source, group-2 layout and
bind group. No residency buffer, probe, readback or wait. Residency code is compiled
only into a separate streamed pipeline pair, built the first time a frame streams
textures. `Feedback` is never a default: its rare blurrier samples in motion are a
quality reduction.

Opt in with `Renderer::set_texture_residency(TextureResidency::Exact)` before uploading
through `try_upload_streamed_texture`. Then streamed textures keep only the source mip
levels their own samples touch. Requests are recorded inside the programmable sampling
helpers, so user programs are covered. Images stay **byte-identical** and **no returned
frame samples a missing level**. Results:

- **Default proof** (one batched lease, same harness, main and candidate interleaved
  A B A B):
  - All 12 quarry SHA-256 fields match main's (hit/count/HDR/world, both poses, three
    paths).
  - Allocated and reserved memory are byte-equal to main's: 3,344.297 / 2,509.556 /
    3,164.990 MiB allocated. The same allocations by name and size; their report order
    varies between runs of main itself.
  - Shading-pass time, frame spans and motion wall-clock are within noise of main's on
    every path and phase ([below](#default-full-against-main)).
- **Opt-in memory.** At the quarry's static wide pose, textures fall from 1,024 to
  816 MiB.
  - Reserved memory falls by **192.0 / 256.0 / 192.0 MiB** (no-RT / RT / traced
    Virtual), to 3,247.7 / 2,385.2 / 3,066.1 MiB, against Unreal's recorded 3,912 /
    5,410 / 3,946.
  - The motion phases request level 0 sporadically. The three reduced scans then grow
    back to their full chains and keep them for the 120-frame hold window, so after the
    motion phases residency equals main's.
  - Peaks are not above main's. Close and reverse quarry views need every level of all
    twelve scans at 16x anisotropy.
- **Anisotropy.** On NVIDIA 617.14, an anisotropically filtered mip tail differs from the
  full chain at its own base level, so anisotropic textures keep one more level than
  they touch. At anisotropy 1, the quarry's close views keep 704 instead of 1,024 MiB,
  also byte-identical.
- **Opt-in startup.** Loads upload only each scan's 64x64-and-smaller tail, so
  post-upload reserved memory falls by 1,024 MiB. The upload phase takes 8.7-9.2 s
  instead of 9.6-9.9 s. The first frame uploads what it samples before it is returned.
- **Opt-in cost (measured, why it is not the default):**
  - The recording helpers cost 1.3-5.7% of the traced shading pass (0.004-0.020 ms).
  - `Exact` waits for each streamed frame before returning it: +45% to +240%
    wall-clock per frame in motion.
  - `Feedback` avoids the wait but shows 15-18 missed samples per 1,125 frames.

The probe measurements behind the anisotropic rule, the first design's misses, and a
rejected follow-up are below.

## Measure first: when is a mip tail exact?

wgpu 30 exposes no sparse or tiled textures, and a texture's allocation covers every
level it declares. So a partially resident texture must be a *different* texture: one
whose level 0 is source level t. Sampling it with the same normalized coordinates,
gradients and sampler selects LOD λ - t, because its level-0 extent is the source's
divided by 2^t. Whether that is *bit-exact* is a hardware question, so it was measured
before any production code.

`gpu_residency_probe` (one process, no window, 5 s) samples random content with
independent random levels, so every filter weight shows in the result. It uses 2^20
footprints per sampler and extent:

- random LOD -4..13 and anisotropy up to 128:1 in level-0 texels, with random
  orientation and skew;
- isotropic and 4:1 sweeps approaching every integer LOD from both sides by
  2^-1..2^-24.

It compares bit patterns of the full chain against tails holding source levels t.. for
t = 1..6, and decodes a 64x64 level probe and a full-extent probe sampled with the same
sampler (below). Per finest touched level L (full-extent probe), here are the samples
whose t-tail differs from the full chain, 4096², of 2^20:

| L | samples (16x run) | trilinear, t ≤ L | trilinear, t = L+1 | aniso 2x/16x, t ≤ L-1 | aniso 16x, t = L | aniso 2x, t = L |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 65,893 | 0 | all | 0 | 37,166 | 20,530 |
| 2 | 66,435 | 0 | all | 0 | 37,524 | 22,793 |
| 3 | 66,299 | 0 | all | 0 | 37,545 | 23,401 |
| 4 | 66,525 | 0 | all | 0 | 37,212 | 23,089 |
| 5 | 66,135 | 0 | all | 0 | 37,251 | 23,002 |
| 6 | 66,423 | 0 | all | 0 | 37,771 | 23,392 |
| 7-12 | 448,883 | 0 | — | 0 | — | — |

1024² and 2048x512 extents give the same pattern. Bilinear/nearest-mip and
nearest/linear-mip samplers behave like trilinear. Findings:

- **The probe is exact and tight.** The 64x64 probe and the full-extent probe decode the
  same level for every sample under every sampler (0 disagreements). No isotropic
  sample over-reports: the next coarser tail always differs. Decode offsets 2^-30 to
  2^-10 and R16F or R32F probes give identical counts.
- **Isotropic:** a tail that keeps the finest touched level is bit-identical.
- **Anisotropic (2x, 16x):** the probe still measures L exactly, but a tail whose base is
  exactly L differs for 56% (16x) or 35% (2x) of samples, at every level and footprint
  ratio. With one more level (base L-1), every sample is bit-identical. NVIDIA 617.14's
  anisotropic filter treats the bound texture's base level specially.

The first screen only tested t ≤ L. It reported 21% violations under anisotropy, which
first suggested anisotropic textures could drop levels only while unsampled. The
per-(L, t) breakdown above found the one-level margin.

## Design

### Residency unit and transitions

`Renderer::try_upload_streamed_texture(Arc<asset::Texture>)` keeps the prepared chain on
the CPU, shared with the caller rather than copied. `Prepared` (viewer and test loader)
now moves its decoded chains into `Arc`s and uses it. Under `Full` (the default) it is
`try_upload_texture`: the same immutable full texture as main, with no CPU copy kept. A
mode is therefore chosen before uploading. Switching a renderer back to `Full` makes
its streamed textures fully resident from the next frame.

A streamed texture is a GPU texture holding source levels `top..`. A residency change
creates the new texture and queues the levels it lacks from the CPU. It also copies the
levels both ranges share, GPU to GPU, all in one immediate submission, so no later pass
can see an incomplete texture. wgpu then retires the old texture after that submission.

Textures upload with only their tail, source levels 6.. (21 KiB of a 4K chain); the
first frame that samples them uploads more. Extents that are not powers of two upload
fully resident, because the probe scale and the tail's level-0 extent must be exact.

### Requests at the programmable sampling boundary

Programs reach textures only through the engine's helpers; they declare no bindings.
Default frames compose the traced API (`shaders/program_traced.wgsl`) exactly as main
does.

A frame that binds streamed textures outside `Full` shades with the program's
*streamed* pipelines instead (`Renderer::ensure_streamed`, built lazily, dropped on
reload). Their composition (`Stage::TracedStreamed`) replaces only the traced
`sample_*_grad` helpers with `shaders/program_traced_residency.wgsl`, on a group-2
layout with two extra bindings. Each of those helpers records a request for its own
sample. The mesh-uv `sample_*` forms call them, so user-chosen coordinates and
footprints are covered:

1. Load the slot's residency word: zero for untextured slots and textures from
   `try_upload_texture`.
2. Sample a shared 64x64 R16F probe, whose level j holds 2^-j. The sample uses the
   material's own sampler and coordinates, with the footprint scaled exactly to the
   probe (a power of two per axis, built from exponent bits). The hardware picks the
   same LOD, trilinear pair and anisotropic footprint as for the material sample. A blend
   of levels j and j+1 lies in (2^-(j+1), 2^-j], so `floor(-log2 v)` is the finest
   touched level.
3. Subtract the slot's margin (1 for anisotropic samplers, else 0). Raise the frame's
   request (atomic max of `16 - level`, guarded by an atomic load). Count a miss when
   the level is below the resident top.

Counters have 32 copies, chosen by 8x8 screen tile and 2,304 bytes apart, because a
single word would put every warp's atomics on one L2 slice. Each frame clears them
before shading. The frame's flight copies them to a mapped readback, and `collect`
takes the max and sum over copies.

Requests come from the shading that produced the image, not from visibility.
Explicit-gradient calls, both trilinear levels and the anisotropic footprint are the
hardware's own selection, not an estimate. A test hook, `survey`, also records the raw
touched level: what an extent-preserving sparse texture would keep.

### Policy

- A request finer than the resident top grows the texture before the next frame.
- A level stays for 120 frames after the last frame whose finest request reached it.
  The texture then shrinks to the finest level still requested, or to its tail when
  nothing sampled it.
- Textures no completed frame has sampled never shrink: residency changes only on
  evidence.
- Each servicing step queues at most 256 MiB of levels; verification waits between
  steps, so staging memory stays bounded.

### Verification: no returned frame samples a missing level

**The first design used feedback and missed.** It verified only a texture's first use,
camera cuts and resizes, and its first quarry run passed every hash. But during the
motion phases, 15 / 6 / 6 samples per path asked for level 0 of a scan resident from
level 1 or 2. Those pixels filtered from the finest resident level: a byte difference
in frames no hash covers. They are isolated pixels whose footprints select
magnification, consistent with near-degenerate ray-differential gradients.

**Default: `TextureResidency::Full`.** It keeps every level, never waits, and runs
no residency code.

**Opt-in `TextureResidency::Exact`**, for applications that want the memory and accept
the throughput cost. Each streamed frame is encoded and submitted unchanged, including
presentation, timestamps and its counter copy. Then:

1. The renderer waits for the frame and reads its counters.
2. If any sample requested a level that was not resident, it uploads the requested
   levels.
3. It shades the HDR target again, presents into the caller's color view again, and
   checks again. Data-dependent sampling can need more rounds; each round grows some
   texture.

Shading reads only hit records, visibility counts and materials, and presentation only
the HDR target, so repeating them is exact. No other pass runs twice, and timestamps
are the frame's own. The cost is that the CPU waits for each streamed frame (below).

**Opt-in `TextureResidency::Feedback`, never a default.** It waits only on frames the
renderer cannot vouch for:

- the first frame binding a never-sampled texture;
- target-size changes and `Renderer::mark_camera_cut`;
- view changes beyond 5 degrees, 0.25 m or a 1% zoom.

Its other misses filter from the finest resident level, a quality reduction, and are
counted in `ResidencyStats`.

### Full-resident fallback

- `try_upload_texture` is unchanged: every level, always.
- Non-power-of-two streamed textures upload fully resident.
- Raster frames (forward lit and diagnostic passes) and temporal film frames sample
  without requests. The streamed textures they bind become fully resident before
  encoding and stay so until traced frames observe them.
- A material's raster group binds the renderer's 1x1 defaults for streamed slots, so it
  never retains their levels. Passes that sample build a group over the current levels.
- The traced material group (group 2) is rebuilt alone when a bound texture's residency
  changes, keyed by texture identity and generation. Geometry tables, which Virtual
  shadows key on, are never replaced for residency.

## Constraints on user shaders (AGENTS.md)

Shading stays programmable. Residency adds no lighting-model data to receivers, hit
records or dispatch, and the program API is unchanged. It does constrain:

1. **Textures are sampled only through the engine helpers.** This was already the
   program contract; residency relies on it.
2. **Streaming frames' helper calls cost extra work; default frames' cost nothing.** In
   streamed pipelines, every call loads its slot word and branches. For streamed
   textures it also takes one probe sample with the material's sampler from a
   cache-resident 64x64 texture, plus an atomic load (and rarely an atomic max). The
   cost is per call, so programs that sample many times pay it per sample.
3. **Helper names `rl_*`, `RL_*`, `primary_residency`, `primary_level_probe` and
   `PrimaryResidency` are reserved.** A program defining one composes in default frames
   but is rejected (`Error::Shader`) by the first streaming frame. That frame builds
   the streamed pipelines lazily, outside the program's reload transaction.
4. **Data-dependent sampling verifies in several rounds.** A program that derives
   coordinates from another sample (parallax, flow maps) can request a different level
   once its first sample is exact. Verification repeats, at most 8 rounds.
5. **Raster and temporal frames are full-resident.** Their helpers record nothing, so
   the textures they sample are made fully resident first. They save no memory there.
6. **Anisotropic samplers keep one extra level.** The margin is measured on NVIDIA
   617.14. On other hardware, rerun `gpu_residency_probe` before relying on byte
   identity; a startup calibration would make this automatic (next steps).
7. **The counter copy is chosen by `input.pixel`.** Programs cannot observe this. A future
   helper evaluated at secondary ray hits (R4 in [programmable-shading.md](programmable-shading.md))
   needs another stable hash there.

## Memory per step against Unreal

The harness and quantities are M1's (`scripts/memory-m1.py`, `memory-wddm.ps1`): quarry,
1920x1080, both poses, MiB = 2^20. **Reserved / WDDM warm max** are measured after
warm-up at the static wide pose p0. The pose-1 report follows the motion phases.
Unreal's recorded DXGI totals are N8h's, not rerun.

| Step | No-RT | RT | Traced Virtual |
|---|---:|---:|---:|
| 0. Main `311075c` (= M2 exactly) | 3,439.7 / 3,512.2 | 2,641.2 / 2,716.4 | 3,258.1 / 3,383.0 |
| 1. Anisotropic textures full while sampled (first policy) | 3,439.7 (derived) | 2,641.2 (derived) | 3,258.1 (derived) |
| 2. One-level margin, Feedback (first quarry run) | 3,247.7, but 15 motion misses | 2,385.2, 6 misses | 3,066.1, 6 misses |
| **3. Margin + Exact (opt-in), p0** | **3,247.7 / 3,484.9** | **2,385.2 / 2,716.3** | **3,066.1 / 3,333.1** |
| 3. Opt-in, pose 1 after motion | 3,439.7 | 2,641.2 | 3,258.1 |
| **4. Default `Full` (merged), both poses** | **3,439.7 / 3,512.3** (= main) | **2,641.2 / 2,716.3** (= main) | **3,258.1 / 3,333.1** (= main) |
| Unreal 5.7.4 recorded DXGI | VSM no-RT: 3,912 | HRT: 5,410 | VSM RT enabled: 3,946 |

- Step 1 is derived, not separately run: the survey shows all twelve scans sampled at
  both poses, so under that policy every texture stays full.
- Live allocations at p0 fall by 207.8 MiB on every path; textures are 816.0 MiB.
- Three scans drop levels at p0: two keep source levels 1.. and one keeps 2... The other
  nine need level 0.
- The traced-Virtual WDDM figure is below main's by about 50 MiB, which is within
  N8h's ±70 MiB process variation. No sampled peak exceeds main's.
- Residency's own buffers total 0.3 MiB: the 13.5 KiB probe, 72.8 KiB of words and
  counters, and up to three 72 KiB readbacks.
- The per-label inventory is [memory-m3-after.csv](memory-m3-after.csv).
- Survey projection at p0: extent-preserving sparse residency, without the margin,
  would keep 716 instead of 816 MiB of textures.
- Anisotropy-1 screen (not matched to Unreal): the quarry's close and reverse poses keep
  704 instead of 1,024 MiB.

## Identity and miss counters

Complete buffers were compared, not region averages:

- **Quarry, default `Full`:** all **12 SHA-256 fields** (hit/count/HDR/world, both
  poses, three paths) match main `311075c`, on the merged binary (`ea2bbeb3cdfd`).
  Main's own match M2's.
- **Quarry, opt-in `Exact`:** all 12 fields also match main on the opt-in binary
  (`8a13bea1d4d1`). The pipeline split that followed changed only which composition
  default frames use. Its opt-in side is rechecked by the camera-cut fixture on the
  merged binary (below).
- **S1-S3:** S1's **904** non-timing fields and S2/S3's **72 each** match M1's retained
  scores. These benches are untextured, so this also shows the helper code changes no
  arithmetic.
- **Camera cut (fixture, `gpu_residency_camera_cut`, all three paths, rerun on the merged
  binary):**
  - The scene is a ground (isotropic 1024²) and a wall (16x anisotropic 512²).
  - Sequence: a mid view (resident from levels 2 and 1), then 128 frames looking away
    (both textures drop to their tails), then a cut to a close view needing ground
    level 0 and finer wall levels, then six frames of motion.
  - Exact and Feedback each run against Full in separate renderers on the same
    sequence. Every captured frame's HDR and presented bytes are identical, including
    the **first frame after the cut**: submitted with both textures at their tails, it
    serviced 283,546 missed samples before it was returned.
  - Shown misses: 0. Full never waited.
- **Camera cut (quarry):** the quality cameras' close pose, the registered `cut_reverse`
  cut, then back, in Feedback against Full. First-frame HDR and hit hashes are identical
  for every configuration, path and anisotropy (16x and 1x). Each cut was verified;
  1.53 M samples were serviced on the first.
- **Miss counters:** the final bench run shows 0 at every report. The timing runs,
  1,125 frames per path, show **0 misses in Exact**, with 4 / 17 / 17 frames
  re-shaded, and 15 / 18 / 18 in Feedback.
- **Probe on the final binary:** 0 margin violations in all six anisotropic and
  isotropic configurations.

## Cost per stage

RT timings and speed comparisons in every subsection below are **measured on the
non-default fast setup (traced + Interactive)**, with the named texture residency
policy applied to that explicitly selected path.

### Default `Full` against main

One batched lease ran main and candidate alternately, A B A B, through the same harness
binary build. That is two processes each, eight visits per path, sixteen visits pooled
per side. Values are medians of clean visit medians [min-max], in ms.

| Path | Phase | Span: main | Span: default | Shading: main | Shading: default |
|---|---|---:|---:|---:|---:|
| no-RT | static | 1.286 [1.270-1.423] | 1.296 [1.277-1.331] | 0.320 [0.318-0.329] | 0.319 [0.318-0.321] |
| no-RT | move | 2.695 [2.655-3.147] | 2.683 [2.652-2.742] | 0.298 [0.295-0.309] | 0.297 [0.297-0.298] |
| no-RT | caster | 2.682 [2.639-2.860] | 2.677 [2.641-2.708] | 0.300 [0.296-0.311] | 0.299 [0.299-0.301] |
| RT | static | 0.878 [0.870-1.078] | 0.874 [0.859-0.929] | 0.357 [0.348-0.422] | 0.349 [0.348-0.355] |
| RT | move | 0.869 [0.852-0.958] | 0.867 [0.852-0.917] | 0.361 [0.348-0.421] | 0.348 [0.347-0.354] |
| traced Virtual | static | 1.199 [1.180-1.523] | 1.196 [1.181-1.255] | 0.360 [0.358-0.683] | 0.360 [0.357-0.368] |
| traced Virtual | move | 2.644 [2.605-3.127] | 2.619 [2.605-3.108] | 0.354 [0.348-0.415] | 0.349 [0.346-0.350] |

Every range overlaps. Frame-span medians shift by under 1%, in both directions. Shading
medians are equal or lower: up to 3.6% lower on RT, where main's pooled visits include
contended outliers up to 0.42 ms. The default composes main's shading source, so these
differences are noise. The turn phase matches static. Motion wall-clock per frame is equal: for example, no-RT camera
2.76 / 2.86 ms for main against 2.74 / 2.74 for the default, RT 0.93 / 0.96 against
0.92 / 1.02. The default costs nothing measurable. All rows are in
`timing-summary-t3.json`.

### Opt-in modes against main

Same harness as M1/M2: RTX 4070 SUPER, NVIDIA 617.14, Vulkan, wgpu 30.0.1, test profile
(not release-performance data), anisotropy 16. Eight visits per process at pose 0, one
asset load per path, baseline then candidate in one lease. Clean visits use M1's primary
thresholds (≤0.45 / 0.65 / 0.30 ms); rejected visits stay in `timing-summary-t1.json`.

The baseline is main `311075c` plus two test-only harness columns:

- the traced shading pass, timed with the existing mesh-pass stamps;
- wall-clock totals of the turn, camera and caster phases, through their tail drains.

Values are medians of clean visit medians [min-max], in ms.

| Path | Phase | Span: main | Span: Exact | Span: Feedback | Shading: main | Shading: Exact |
|---|---|---:|---:|---:|---:|---:|
| no-RT | static | 1.303 [1.275-1.315] | 1.317 [1.299-1.440] | 1.295 [1.286-1.345] | 0.319 [0.318-0.320] | 0.323 [0.322-0.333] |
| no-RT | move | 2.699 [2.654-2.705] | 2.726 [2.669-3.201] | 2.675 [2.667-2.775] | 0.297 | 0.308 |
| no-RT | caster | 2.663 [2.640-2.700] | 2.852 [2.646-3.667] | 2.664 [2.650-2.717] | 0.300 | 0.309 |
| RT | static | 0.873 [0.868-0.927] | 0.951 [0.877-1.132] | 0.914 (1 clean) | 0.350 [0.348-0.353] | 0.370 [0.361-0.430] |
| RT | move | 0.867 [0.863-0.916] | 0.946 [0.870-1.127] | 1.117 [0.905-1.136] | 0.348 | 0.370 |
| traced Virtual | static | 1.196 [1.173-1.264] | 1.223 [1.200-1.279] | 1.249 [1.203-1.290] | 0.358 [0.357-0.366] | 0.373 [0.366-0.376] |
| traced Virtual | move | 2.629 [2.605-2.652] | 2.817 [2.672-3.087] | 2.679 [2.630-3.124] | 0.349 | 0.368 |

Wall-clock per frame of the motion phases (ms; throughput, which GPU spans cannot show):

| Path | Turn: main / Exact / Feedback | Camera: main / Exact / Feedback | Caster: main / Exact / Feedback |
|---|---:|---:|---:|
| no-RT | 1.39 / 2.26 / 1.46 | 2.76 / 4.09 / 2.80 | 2.73 / 3.95 / 2.76 |
| RT | 0.94 / 1.78 / 1.29 | 0.91 / 3.12 / 2.33 | 0.97 / 2.03 / 1.31 |
| traced Virtual | 1.28 / 2.00 / 1.39 | 2.68 / 5.04 / 3.95 | 2.78 / 3.87 / 2.86 |

What these show:

- **The helpers cost a resolved 1.3-5.7% of the shading pass.** The sampling-boundary
  feedback adds 0.004-0.020 ms per frame.
- **Frame spans:** no-RT ranges overlap main's. RT and traced-Virtual medians rose
  2-9%, with overlapping ranges (RT keeps few clean visits under wallpaper
  contention). Exact also lets the GPU idle between frames.
- **Exact roughly doubles motion-phase frame cost on RT.** The CPU waits for each
  frame, so the next frame's CPU work no longer overlaps GPU work.
- **Feedback's motion cost comes from residency churn.** Textures shrink after the hold
  and regrow on sporadic level-0 requests: 15-34 transitions and 1.0-1.5 GiB of
  uploads per run in both modes.

These opt-in costs are why streaming is off by default. `Exact` is byte-exact and
never shows a missing level, but halves motion throughput. `Feedback` is nearly free
in steady state but shows rare blurrier samples, a quality reduction, so it is never a
default. On this scene the memory gain is 192-256 MiB, and only in static wide views.

### Rejected: sentinel-only reporting

To cut the helper cost, fully resident textures reported only one pixel per 4x4 block
(misses still reported everywhere). The shading pass barely moved (no-RT 0.321 against
0.323, RT 0.369 against 0.370 ms). Camera-motion throughput got much worse (Exact no-RT
4.1 → 7.9 ms per frame), because used textures shrank between sentinels and regrew with
64 MiB uploads. It was measured on one timing lease (`timing-t2`) and reverted; the
patch is `rejected-sentinel.diff`.

## Next steps

Lower priority than the moving-camera shadow work; not started.

1. **Extent-preserving sparse residency through Vulkan interop.** A sparse `VkImage` of
   the full extent, wrapped with `create_texture_from_hal` and bound with
   `vkQueueBindSparse`, would sample exactly like the full chain. It would drop the
   anisotropic margin (716 instead of 816 MiB at p0). Unbacked levels could stay unbound
   while the probe still detects requests, but a missing level would still need a wait
   or a fallback. It is unsafe and outside wgpu's exposed API, so it was not done here.
2. **Hold policy against churn.** Keep a level resident after a re-shade until it has been
   unrequested for longer than the motion it served, or grow only on repeated requests
   when the miss is a few isolated pixels. That would cut the 1.0-1.5 GiB of uploads;
   measure it with the same wall-clock columns.
3. **Startup calibration of the margin per device** with a 64K-footprint version of the
   probe, instead of the measured NVIDIA constant.
4. **Requests on the raster lit path** (fragment atomics in `program_raster.wgsl`), so
   raster frames stream instead of forcing full residency.
5. **Skip verification on exact repeat frames.** This only works with a complete
   inventory of program inputs (camera, transforms, lighting, parameters, environment,
   visibility settings); a missed input would show a missing level, so it was not
   attempted.

## Checks

- `cargo test -p renderlab --lib -j 2`: **exit 0, 112 passed, 112 ignored**. This
  includes six new CPU tests: word packing, hold policy, transition levels,
  power-of-two admission, cut thresholds, and that default traced shading composes
  without residency code.
- Fmt passes. All-targets Renderlab Clippy passes with `-D warnings` and M1/M2's
  allowances, on the merged tree.
- `shaders/program_traced.wgsl` has no diff against main.
- `gpu_primary_memory_modes`, `gpu_shared_cluster_tables` and `gpu_virtual_fixture` exit
  0. Validation layers stay on, as in M2.
- No `shaders/virtual_shadow*.wgsl`, `src/virtual_shadow_gpu.rs` or
  `docs/milestones-5-6-decisions.tsv` change; the merge took main's versions.
- No window, input automation or desktop capture. The shared runner owned the GPU lock,
  the reservation and release lines, game detection and the 4 GiB start / 2 GiB abort
  guard.

## Reproduction, evidence and time

Evidence root: `E:/renderlab-work/m3-texture/`.

- `cut/` holds `probe.json` and `camera-cut.json`.
- `after/` and `base-after/` hold the candidate and main quarry runs; `m3-check.json`
  compares them.
- `quarry-cut/` and `quarry-cut-aniso1/` hold `cut-summary.json`; `scores/` holds
  `identity.json`.
- `timing-t1-{before,exact,feedback}/` and `timing-summary-t1.json` hold the timing;
  `timing-t2-*` holds the rejected sentinel screen.
- `build-identities-cand6.json` holds the opt-in binary SHA-256 `8a13bea1d4d1` and the
  baseline `8ffc6922a811`. `build-identities-cand9.json` holds the merged default-`Full`
  binary, `ea2bbeb3cdfd`.
- `after-default/`, `timing-t3-{main,cand}-{a,b}/`, `timing-summary-t3.json` and
  `cut-opt-in/` hold the default proof lease (`proof-step.py`).

```powershell
$env:N8_OWNER='m3-texture'; $env:N8_OUT='E:/renderlab-work/m3-texture'
py -3 -B scripts/n8-run.py --label gpu --minutes 40 --fast-check -- py -3 -B E:/renderlab-work/m3-texture/gpu-step.py <binary> <tag>
py -3 -B scripts/n8-run.py --label timing --minutes 45 --fast-check -- py -3 -B E:/renderlab-work/m3-texture/timing-step.py <tag> <binary>
```

After hashing and scoring, 111 raw files over 5 MB (2.63 GiB, staged executables
included, their SHA-256s in `cleanup.json`) and 4,246 scored raw frames (5.5 GiB logical,
including reference hardlinks) were removed, as were the three Cargo targets (3.4 GiB)
and the baseline and probe worktrees. 112 MB of summaries, logs and patches remain. No
owned GPU/build job or lease remains.

`gpu-step.py` runs, in order: the probe, the cut fixture, M2's GPU tests, the quarry
(`memory-m1.py`, survey on), main's quarry, the quarry cut, S1-S3 and the anisotropy-1
screen. `score.py`, `quarry-check.py` and `analyze.py` summarize them.

The opt-in follow-up (default `Full`, split pipelines, proof) took 4 more leases,
735.5 s, including a 28 s proof attempt that a shared stand-in buffer failed. That
failure is why default frames now keep main's layout.

Time: roughly 2 h 20 min wall-clock from the first probe to the last timing run,
interleaved with shared-slot waits; implementation and analysis is an estimate within
that. Verification is measured separately: 23 guarded GPU/build leases total **2,266 s
(37.8 min)**. That includes three short failed fixture runs (Virtual clipmap limits in
the first camera choices) and two probe screens. Cold test builds took 125-126 s,
incremental 27-85 s, the CPU suite 0.53 s. S1-S3 scoring ran on the CPU outside the
leases, about 2 min per pass.
