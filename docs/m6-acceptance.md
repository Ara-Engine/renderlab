# M6 acceptance record

2026-11-07 UTC, branch `t3/m6-close`, merged main `24980a2` (N8p's Surface default and [M5-DL](m5-dev-loop.md)). Contract: [milestones-5-6.md](milestones-5-6.md). Labels: **M** measured here, **R** recorded in the cited unit, **I** inference.

## Verdict

**RT measurement scope:** every Renderlab RT timing, speed ratio and RT memory
comparison below is **measured on the non-default fast setup (traced + Interactive)**,
unless a row explicitly names Traced + Virtual. The RT-device default remains raster
primaries with 256-ray Exact shadows. [RT1](rt1-interactive.md) diagnoses the false-shadow
miss; it does not promote the fast setup or certify the default's speed.

**M6 is not accepted.** Shadow quality beats Unreal 5.7 on almost every recorded measure, on both paths. Static shadows are free on the non-RT path, and memory is below Unreal's on every path. Speed fails where the work changes every frame on the non-RT path:

| Shadow pass per frame, quarry 1080p | Non-RT: Surface (default) | Non-RT: Exact (option) | RT: Traced + Interactive | Unreal VSM no-RT | Unreal VSM, RT on | Unreal HRT |
|---|---:|---:|---:|---:|---:|---:|
| Static | **0.000** pass | 0.577-0.598 miss | ≤ 0.276 pass vs HRT; parity vs VSM-RT | 0.497-0.498 | 0.497-0.505 | 0.626-0.655 |
| Sun turning | 0.574-0.598 parity, 2x miss | 0.585-0.620 parity, 2x miss | ≤ 0.276 pass | 0.881-0.906 | 0.928-0.935 | 0.637-0.643 |
| **Camera moving** | **1.049-1.065 miss** | 1.497-1.544 miss | **≤ 0.276 pass vs HRT; parity vs VSM-RT, 2x miss by 0.02-0.03** | **0.495-0.500** | **0.501-0.505** | **0.627-0.635** |
| Caster moving | 1.591-1.604 miss | 1.723-1.746 miss | ≤ 0.276 pass | 0.670-0.686 | 0.683-0.685 | 0.631-0.634 |

- "Pass" means at least 2x faster than the matched Unreal mode. "Parity" means faster, but not 2x. Unreal's figures are measured directly here (below).
- Non-RT rows (M) come from this unit's final quiet lease (dev/test profile, GPU timestamps). M5-DL's release run agrees: Surface's shadow pass costs 0.000 / 0.595 / 1.049 / 1.591 ms.
- The RT row is the eight hardware rays' cost: the fused trace's increment over the hard trace, paired per frame (M, below). It is the same in every phase because Interactive keeps no history. M5-DL's release spans confirm it (0.865-0.868 ms in all four phases).
- The non-RT path compares with Unreal VSM no-RT; the RT path with HRT and VSM RT-on.

## 1. Unreal 5.7's shadow cost, measured directly (M)

N8f's ~0.5 ms camera-moving figure was an estimate. It now holds as a measurement.

**Method** (`scripts/ue-shadow-motion.py`):
- The matched e2e launches: packaged Development `UnrealGame.exe` 5.7.4 CL51494982, D3D12 SM6, `-RenderOffscreen`, hidden process, no window.
- Same quarry, camera, sun, materials and settings as [the matched comparison](quarry-end-to-end-comparison.md).
- VSM no-RT changes only `r.RayTracing=False` (N8h's method).
- 160 frames per process: frames 0-63 still at pose 0, then the motion from frame 64. Statistics cover frames 96-159.
- **The motion is our harness's own**, not the e2e's alternating 1 cm poses. That removes M5-DL's mismatch caveat.
  - Camera: eye and target 1 mm a frame along source X (UE +0.1 cm Y).
  - Sun: 0.5 mrad a frame about source Y.
  - Caster: placement 0 (`Quarry_00`) 1 mm a frame.
- Shadow cost per frame: the CSV's `GPU/ShadowDepths` + `GPU/ShadowProjection` medians. ProfileGPU trees at frames 100, 116, 132 and 148 price the stages; those frames and the next are excluded from the CSV statistics.
- Two processes per cell, in reversed block order. The GPU read 1-6% busy before every s1 launch.

**Shadow pass and stages, ms per frame** (range over two processes):

| Mode / workload | Shadow pass | Invalidation | Page marking | Page allocation | Raster | Merge | Projection (SMRT) | Whole GPU frame |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| VSM no-RT / static | 0.497-0.498 | 0.001 | 0.083-0.092 | 0.040-0.046 | 0.208-0.211 | 0.005 | 0.156-0.157 | 1.555-1.559 |
| VSM no-RT / **camera** | **0.495-0.500** | 0.001-0.002 | 0.082 | 0.043-0.047 | 0.207-0.209 | 0.004-0.005 | 0.149-0.156 | 1.553-1.574 |
| VSM no-RT / sun | 0.881-0.906 | 0.001 | 0.079-0.080 | 0.059-0.066 | 0.593-0.606 | 0.004-0.006 | 0.142-0.144 | 1.921-2.059 |
| VSM no-RT / caster | 0.670-0.686 | 0.001 | 0.080-0.086 | 0.058-0.060 | 0.370-0.374 | 0.004-0.005 | 0.159 | 1.736-1.790 |
| VSM RT on / static | 0.497-0.505 | 0.001 | 0.093-0.096 | 0.041-0.044 | 0.202-0.220 | 0.004-0.005 | 0.152-0.156 | 1.554-1.583 |
| VSM RT on / **camera** | **0.501-0.505** | 0.001 | 0.083-0.091 | 0.041-0.043 | 0.206-0.216 | 0.004-0.005 | 0.157 | 1.572-1.587 |
| VSM RT on / sun | 0.928-0.935 | 0.001 | 0.087 | 0.065-0.082 | 0.604-0.641 | 0.004 | 0.141-0.159 | 1.990-2.039 |
| VSM RT on / caster | 0.683-0.685 | 0.001 | 0.084-0.090 | 0.055-0.059 | 0.383-0.386 | 0.004 | 0.150-0.155 | 1.783-1.805 |

| HRT (1 spp + denoiser) | Shadow batch (rays + denoiser) | of which denoiser | RT scene | Nanite RT streaming | Total | Whole GPU frame |
|---|---:|---:|---:|---:|---:|---:|
| Static | 0.536-0.563 | 0.309-0.316 | 0.048-0.050 | 0.042 | 0.626-0.655 | 1.743-1.753 |
| **Camera** | 0.535-0.536 | 0.309-0.311 | 0.050 | 0.042-0.049 | **0.627-0.635** | 1.764-1.766 |
| Sun | 0.542-0.547 | 0.316-0.319 | 0.050-0.051 | 0.045-0.047 | 0.637-0.643 | 1.738-1.751 |
| Caster | 0.538-0.541 | 0.311-0.315 | 0.051 | 0.042 | 0.631-0.634 | 1.776-1.778 |

- **Camera motion costs VSM nothing extra.** Its pages stay cached; the 0.5 ms is the per-frame marking, the cull-heavy Nanite raster pass over cached pages, and SMRT projection on every pixel.
- **The caster workload made all twelve meshes movable.** Console `ke` can set mobility only on every `StaticMeshComponent` at once. A control run (`s2`) made them movable without moving anything: 0.502-0.516 ms (VSM no-RT and RT on), within 0.01 ms of static. So the caster cost is the motion itself: about +0.16 ms of raster.
- `GPUMem/LocalUsedMB` reads 3,913-3,915 (VSM no-RT), 3,945-3,948 (VSM RT on) and 5,407-5,410 MiB (HRT): N8h's totals.
- Evidence: `E:/renderlab-work/m6-close/ue/s1/` (24 runs plus the probe) and `s2/` (the 6 control runs). Each has a `summary.json`, the CSVs, `ue.log` with the ProfileGPU trees, and `process.json` with the command line.

## 2. Camera-moving shadow speed (M)

**Non-RT, Surface (the default): miss.** 1.049-1.065 ms against VSM no-RT's 0.495-0.500: 2.1x slower. Exact, the option, costs 1.497-1.544 (3x slower).

**RT, measured on the non-default fast setup (traced + Interactive): pass against HRT; parity against VSM RT on.**
- The eight rays cost **0.276 ms** a frame, in the paired median of 32 frames (range 0.237-0.327).
- That is 2.3x below HRT's 0.627-0.635: a 2x pass.
- It is 1.8x below VSM RT on's 0.501-0.505: faster, but 0.02-0.03 ms over that 2x floor (0.250-0.253).
- The whole fused trace, primary visibility included, costs 0.484 ms in M5-DL's release run, below both Unreal shadow passes alone. This unit's paired figure on the dev binary, with the bench's own lighting, is 0.526 ms: just above VSM RT on's 0.50.

**RT device with non-RT shadows** (traced primaries + Virtual Surface): 1.023-1.203 ms (R, N8o). That is a miss against VSM RT on's 0.50.

**Frame totals on matched motion.** RT totals are **measured on the non-default fast setup (traced + Interactive)**. Our GPU spans are M5-DL's release runs, against Unreal's whole-frame `GPUTime` from §1. The boundaries differ: ours runs from the first rendered pass through tone mapping, D3D12 against Vulkan.

| Phase | No-RT span | VSM no-RT | Ratio | RT span | HRT | Ratio | VSM RT on | Ratio |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Static | 0.734 | 1.555-1.559 | 2.12x | 0.867 | 1.743-1.753 | 2.01-2.02x | 1.554-1.583 | 1.79-1.83x |
| Sun | 1.327 | 1.921-2.059 | 1.45-1.55x | 0.867 | 1.738-1.751 | 2.00-2.02x | 1.990-2.039 | 2.30-2.35x |
| **Camera** | **1.792** | **1.553-1.574** | **0.87-0.88x** | 0.868 | 1.764-1.766 | 2.03x | 1.572-1.587 | 1.81-1.83x |
| Caster | 2.368 | 1.736-1.790 | 0.73-0.76x | 0.865 | 1.776-1.778 | 2.05-2.06x | 1.783-1.805 | 2.06-2.09x |

M5-DL's RT spans are its clean visits (12 of 16, or 11 for caster). Unfiltered RT visits reached 1.4-2.8 ms (R). The RT 2x margins are thin.

## 3. The hillclimb: what each step did (M)

The no-RT quarry, both poses, interleaved A/B in one process, 48 camera frames of 1 mm. Hit rates come from `VSC_STATS`, as the median of the phase's last 16 frames.
- **The baseline:** camera frames show 85.2% of receivers from cells. 12.0% are found but not shown, and 14.8% (50,800) walk.
- **Where the walks come from:** only about 5,000 are partial (penumbra) pixels. The rest are solid pixels whose 3x3 consensus a differing neighbour spoils.

| Step (lease) | Change | Walks while moving | Camera ms | Verdict |
|---|---|---:|---:|---|
| L1-L3 | Show penumbra cells whose computing pixels' 3x3 was all partial (`surface_partial`, margins 2-4) | 3-9 cells certify a frame | unchanged | removed: the quarry's 8-direction penumbrae are thin and noisy |
| L4 | Consensus tolerance: a neighbour agrees within 32 or 64 / 256 | 14.3% / 13.9% | within noise | removed |
| L5 | Lit cells shown unless a neighbour is fully blocked (224) / always (256). Showing lit never darkens a receiver | 12.9% / **7.6%** | 0.995 / 1.002 | walks halved, cost −4%: **the queued walk is floor-bound** |
| L6 | + N8e's cap rule 2 (no proven block after 16 visits: undecided directions lit) | — | 0.961-0.984 (rule 2 alone); **0.762-0.785 (both)** | −27%, still 1.5x VSM. N8e/N8f measured rule 2 at +3,400-3,700 lit umbra receivers on the quarry, and S1 has no leak margin over VSM. Not kept |
| L7 | L5 at 256 with the walk sorted heaviest first below 32k (16k, 8k) | 7.6% | 0.968 | no gain; turning slower |

- **Why it misses.** The camera walk takes 0.68-0.74 ms (N8o trace) whatever its size. Halving the queue saved 0.04 ms. The time sits in the slowest 32-lane chunks: lanes in a chunk wait for its longest walk, and each walk is a chain of dependent geometry fetches (N8l, N8m). Only bounding the longest walks moved it (L6), and that bound costs leaks.
- **What remains untried.** Shortening the dependent chain itself, with walks that change no output, is the open lever. N8l-N8o exhausted the ordering, splitting, state-machine and occupancy variants.
- **Not shipped.** No renderer change from this hillclimb is kept. The branch keeps one test-only aid: `RENDERLAB_VSC_SHADER` loads a variant of the cache shader without a rebuild, as `RENDERLAB_VSM_SHADER` does for the exact lookup.
- **Identity.** The final binary reproduces N8o's Surface and Exact counts in every phase and pose: 12 hashes, including camera-moving `3e0786a5…` / `03fe73d4…` and caster-moving `95d7350c…` / `4c9b1ac3…`.

## 4. RT path: the surface cache stays non-RT (decision (b), M)

All RT speed claims in this section are **measured on the non-default fast setup (traced + Interactive)**.

Porting the surface cache to hardware-ray visibility cannot buy what it costs.

- **What the rays cost.** The RT path's eight shadow rays cost 0.276 ms a frame, the same static or moving. They are fused with the camera trace, so their setup reuses the primary hit while it is in cache.
- **What the cache costs.** Its fixed passes on a camera frame alone cost 127 µs (N8o trace): probe 69, sort 3.5, claim and resolve 54. Under motion 15% of pixels would still trace. Even if those rays cost only their share (0.04 ms), the best case is about 0.17 ms: 0.11 ms saved on camera frames, and 0.28 ms on static ones.
- **Why that best case is out of reach.** It needs the rays moved out of the fused primary trace into a queued pass over scattered pixels, which loses what makes them cheap. That kernel is primary-visibility code, outside this unit's files.
- **What it would cost in quality.** Cell reuse moves RT quality from Interactive's own walk-free output toward Surface's, below its uncached result. The traced Surface rows in N8o show that cost: partial RMS .308 against .286, solid flips 714 against 340.
- **What is already true.** The RT path's shadows are 2.3x faster than HRT's in every phase, and at least 1.8x faster than VSM RT on (static and camera; 2.5x caster, 3.4x sun). The only RT shadow-speed miss is the 2x floor against VSM RT on's static and camera frames, by 0.02-0.03 ms.

**Recorded under "win with and without RT":** RT's camera motion beats Unreal on both comparators without the cache. The miss is on the non-RT side (§2), where the cache lives.

## 5. Quality against Exact and Unreal (R)

Unchanged by this unit. Every row comes from [N8o](non-rt-n8o.md) / [N8p](non-rt-n8p.md): the same scorers and film references, with the quarry approached by 16 camera frames of 2 mm. Pairs are pose 0 / pose 1. Lower is better.

**Quarry, non-RT:**

| Measure | Surface (default) | Exact | Unreal VSM | Surface − Exact | Surface − VSM |
|---|---:|---:|---:|---:|---:|
| Partial RMS | .365 / .358 | .353 / .348 | .483 / .483 | +.012 / +.010 | −.118 / −.125 |
| Solid flips | 2,685 / 2,724 | 2,434 / 2,474 | 12,390 | +251 / +250 | −9,705 / −9,666 |
| Umbra leaks | 3,452 / 3,525 | 3,289 / 3,366 | 14,935 | +163 / +159 | −11,483 / −11,410 |
| Lit darkened | 37 / 30 | 60 / 55 | 87 / 88 | −23 / −25 | −50 / −58 |
| Lost umbra edges | 2,651 / 2,699 | 2,376 / 2,424 | 11,884 | +275 / +275 | −9,233 / −9,185 |

**Quarry, traced receivers** (the RT device's Virtual shadows), Surface / Exact: partial RMS .308/.309 against .286/.290; flips 714/756 against 340/356; leaks 1,447/1,478 against 1,202/1,213; lit darkened 1/1 against 0/0; lost edges 713/755 against 339/356. All are far below VSM's.

**S1 contact fold:**
- Surface: partial RMS .1085 / .1063; flips, leaks and lost contacts 0.
- Exact: .1085 / .1065, with **2 leaks at pose 1**. VSM has 0, so this is Exact's miss; Surface has none.
- VSM: .1642 / .1628, flips 0 (env 1: 1), 5-6 / 2-3 contacts lost.

**S2 and S3, every motion phase:**
- **Against VSM.** Surface is better on partial RMS (.0759-.1266 against .1675-.1878), HDR RMS (.0099-.0152 against .0145-.0219), lost contacts (0 against 1,039-12,289), leaks (0-2 against 57-114) and flips (0 against 119-226).
- **Against Exact.** Partial RMS is equal or better in every phase. S3 rotate 1 shows 2 leaks against Exact's 1.
- **Flicker** (frame-to-frame residual change against the film reference): 4-350x below VSM in every moving phase. It matches or beats Exact in 9 of 11 phase means, and is 0.3% / 1.0% above Exact in S3 rotate .25 / rotate 1.

**RT path, Traced + Interactive:**
- **S1** ([bench-contact-fold.md](bench-contact-fold.md)): 0 lost contacts, 0 leaks and 0 flips, partial RMS .0868-.0878. VSM: 2-6 contacts lost, .162-.164. HRT1: 26-34 lost, 348-448 leaks.
- **S2 / S3** ([bench-motion.md](bench-motion.md)): partial RMS .0742-.0779 against VSM's .170-.186 and HRT1's .137-.254. Lost contacts 0 against VSM's 2,101-12,289; leaks 0 against Unreal's 62-126; flips 0 against 167-200.
- **Flicker,** computed here from the recorded per-frame rows (`motion-benches/S*/per-frame.json`): below VSM and HRT1 in every phase. S2 disk camera .0132-.0191 against VSM's .0605-.0615; S3 rotate .0025-.0050 against VSM's .080-.082 and HRT1's .123-.124.
- **Quarry film reference** ([m6-film-reference.md](m6-film-reference.md), F0): partial RMS .234 / .235 against VSM's .371 / .372 and HRT's .346 / .345. Missed film shadows 9,768 / 9,682 against 24,049 / 24,059.
- **One miss:** false shadows (film-lit receivers shown ≤ 0.9).
  - Policy reference: Interactive 9 / 11 against VSM's 9 / 10.
  - Minimal reference: 21 / 17 against 11 / 9.
  - Exact has the same miss (9 / 11, 20 / 17). HRT has 743-770.
  - These are F0-era scores, not re-measured here.
  - [RT1](rt1-interactive.md) reproduces those witnesses and distinguishes the
    later receiver policy. Its current-source capture measures **27 / 26** false
    shadows against that policy's own film reference, versus rescored VSM's
    **11 / 9**. A targeted 1,048,576-direction point replay leaves 15 / 16 of the
    selected failures. The miss persists; most F0-era coordinates have cleared.

Neither path meets the film-quality target. Neither is presented as doing so.

## 6. Memory against Unreal (M, R)

Allocator reserved / WDDM, MiB, quarry static, with `renderlab::MEMORY_HINTS` blocks. Unreal's figure is DXGI `LocalUsedMB` (N8h, and §1 again).

| Path | Renderlab | Source | Unreal | Below by |
|---|---:|---|---:|---:|
| No-RT, Surface (default) | 3,503.7 / 3,583.9 | M5-DL | VSM no-RT 3,912-3,915 | 330-410 |
| No-RT, Exact | 3,439.7 / 3,512.3 | M3 | VSM no-RT 3,912-3,915 | 400-470 |
| RT, Traced + Interactive | 2,641.2 / 2,716.3 | M5-DL | HRT 5,407-5,410 (VSM RT on 3,946) | 2,690-2,770 (1,230) |
| RT device, Traced + Virtual Surface | **3,386.0 / 3,461** | this unit, M1 harness | VSM RT on 3,945-3,948 | 485-560 |

N8o and N8p cite 4,138-4,202 MiB for the RT device. That figure came from the quarry screening harness, which keeps other resources resident; it is not the M1 inventory harness that the rows above use. The difference is not itemized here (I).

## 7. Defaults and opt-ins, and why

| Setting | Default | Opt-ins | Why the opt-ins are not defaults |
|---|---|---|---|
| `VirtualLookup` | **Surface** (N8p) | Exact (the reference) | Exact costs 0.58 ms on static frames against 0; Surface beats Unreal on every recorded measure and fixes Exact's S1 leaks |
| | | **Map** (N8k) | Lit darkened 263 / 242 against VSM's 87 / 88: below Unreal, so never a default |
| | | **Hybrid** (N8k) | Above Unreal, but camera frames cost 1.51-1.53 ms, slower than Surface's 1.05, and it keeps both lookups' resources |
| `TextureResidency` | **Full** | Exact, Feedback (M3) | Exact costs +45-240% on motion frames to save 192-256 MiB in static wide views. Feedback shows rare missed samples, a quality reduction |
| Surface screen constants | main's | `VSC_*` (screens only) | Measured in N8n, N8o and §3; none beat the defaults within the quality rule |

**Defaults, docs and the decisions trail agree, with two points for Islam:**
- **The compared RT path is not the RT-device default.**
  - On an RT device the library defaults to raster primaries with the `SoftShadows::Exact` preference (256 rays).
  - Every RT comparison (e2e, M5-DL, this record) sets `PrimaryVisibility::Traced` and `SoftShadows::Interactive` explicitly.
  - The viewer inherits the library defaults.
  - [RT1](rt1-interactive.md) owns the conditional default decision. Promotion requires every static and motion measure to pass; the film false-shadow miss still prevents it.
- The trail's N8p row says camera motion is "about 0.5, estimate". The measured 0.495-0.500 replaces it (rows below).

## 8. M5 status: the matched Unreal comparison (measurement only)

**Measured on matched motion:**
The RT speed claims are **measured on the non-default fast setup (traced + Interactive)**.

- **Speed:** §2's frame table. No-RT static passes 2x (2.12x). The no-RT sun misses 2x (1.45-1.55x). No-RT camera and caster are slower than Unreal (0.87x and 0.73-0.76x). RT passes 2x against HRT in all four phases, thinly (2.00-2.06x), and reaches 1.79-2.09x against VSM RT on.
- **Memory:** §6, below Unreal on every path.
- **Development loop** (M5-DL):
  - A fresh shader edit takes 1.08-1.09 s to a completed image, missing the historical 100 ms target. A revert takes 0.10-0.12 s.
  - A small library Rust edit takes 7.2 s to a changed viewer; a viewer-only edit takes 2.7 s.
  - Quarry startup takes 8.4 s, and the small fixture's 0.72 s.
  - Invalid shaders keep the last working image, byte-identical.

**Not measured; still open:**
- **Image fidelity of the direct-light/material subset.** [The full-detail comparison](unreal-quarry-full-detail-comparison.md) left radiance parity unproved: the main view's exposure and display mismatch, the sharp-highlight disagreement, and mapped-light radiance. Closing them means diagnosing, not re-measuring.
- **Glossy motion and environment visibility** folded into [M7](milestone-7.md): [m5-glossy-motion.md](m5-glossy-motion.md), [m5-environment-visibility.md](m5-environment-visibility.md). M7 owns them.

**M5 stays open** on matched Unreal acceptance and development-loop latency.

## 9. Every remaining miss

1. **Non-RT camera-moving shadows: 1.049-1.065 ms against VSM's 0.495-0.500 (2.1x slower).** The frame is slower than Unreal's (0.87x). §3 measured why.
2. **Non-RT caster-moving shadows: 1.591-1.604 ms against 0.670-0.686 (2.4x slower).** The frame is 0.73-0.76x Unreal's.
3. **Non-RT sun turning beats VSM, but not by 2x:** 0.574-0.598 against 0.881-0.906 ms, and 1.45-1.55x for the frame.
4. **RT, measured on the non-default fast setup (traced + Interactive), against VSM RT on:** 0.276 ms against 0.497-0.505 on static and camera frames, 0.02-0.03 ms over the 2x floor. Its frame is 1.79-1.83x faster on static and camera frames. Against HRT every phase passes, thinly (2.00-2.06x), and unfiltered RT visits reached 1.4-2.8 ms.
5. **The RT path's film-reference false shadows:** **27 / 26 against VSM's 11 / 9** on the adopted viewed-side reference (M, [RT1](rt1-interactive.md)); the older 9-21 against 9-11 remains F0-era evidence, not a current score.
6. **Surface is below Exact** on four quarry measures (+3-12%), S3 rotate 1's leaks (2 against 1) and two S3 flicker phases (+0.3%, +1.0%). It is above Unreal on all of them.
7. **The compared RT configuration is not the library's RT-device default** (§7).
8. **Not film quality** on either path, in any mode.
9. **M5's image fidelity against Unreal is unproved,** and M5's development loop misses its shader target (§8).

## Rows proposed for `docs/milestones-5-6-decisions.tsv` (root owns the file)

```
2026-11-07	M6-close	Measure Unreal 5.7.4 shadow cost per frame directly, with our quarry harness's motions (camera 1 mm/frame, sun 0.5 mrad/frame, caster 1 mm/frame), stages priced from ProfileGPU	N8f's camera-moving ~0.5 ms was an estimate; M5-DL's frame comparison had an alternating-pose caveat	VSM no-RT static 0.497-0.498, camera 0.495-0.500, sun 0.881-0.906, caster 0.670-0.686 ms; VSM RT on camera 0.501-0.505; HRT 0.627-0.635 in every phase; scripts/ue-shadow-motion.py; docs/m6-acceptance.md	recorded
2026-11-07	M6-close	Non-RT camera-moving shadow cost: a miss; no hillclimb step retained	The queued walk is floor-bound: halving it saved 4%; bounding long lit walks (N8e cap rule 2) gave -27% but costs about 3,500 umbra leaks and stays 1.5x VSM	Surface 1.049-1.065 ms vs VSM 0.495-0.500; steps L1-L7 in docs/m6-acceptance.md §3; final binary reproduces N8o hashes	recorded miss
2026-11-07	M6-close	Do not port the surface cache to the RT shadow path (option b)	Measured on the non-default fast setup (traced + Interactive): eight fused rays cost 0.276 ms in every phase; the cache's fixed passes cost 0.127 ms and would need the rays moved out of the fused trace, with Surface's reuse quality cost	RT camera 0.276 ms vs HRT 0.627-0.635 (2.3x, pass) and VSM RT on 0.501-0.505 (1.8x, 2x miss by 0.02-0.03); docs/m6-acceptance.md §4	recorded
2026-11-07	M6	M6 not accepted: quality and memory beat Unreal on both paths; non-RT camera, caster and sun motion and the RT 2x floor vs VSM RT on miss; the compared RT configuration is not the RT-device default	Acceptance needs speed and quality together on both paths	docs/m6-acceptance.md	open
```

## Evidence, checks and time

- **Evidence:** `E:/renderlab-work/m6-close/`.
  - `ue/s1`, `ue/s2`: the Unreal runs.
  - `l1-*` … `l7-*`: the screens. `final-t`: no-RT timing and hashes. `rt-paired`: the RT paired frames. `final-mem`: traced Virtual memory.
  - `runs/`: every lease's log and preflight, and the gate logs.
  - `l*.sh`, `final.sh`, `variants/debug-partial.wgsl`.
- **Builds:** the final binary is `bin/final.exe`, sha256 `24d6c407…`, built from main `24980a2` plus this branch's script and test-only override. Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, validation off for timing.
- **Gate** (CPU, after the last lease):
  - `cargo test -p renderlab --lib -j 2`: exit 0, 114 passed, 0 failed, 112 ignored.
  - Clippy `--all-targets -D warnings` (with N8o's two allowances) and `cargo fmt --check`: exit 0.
  - Again after merging main `ab167bc` (M7's later commits), on the pushed head: 116 passed, 0 failed, 113 ignored; Clippy and fmt exit 0. The GPU measurements above ran on `24980a2`'s binary, without M7's later commits.
- **Leases:** ten under the shared lock with the 4/2 GiB guard, at least 12.1 GiB available at every start, about 15 minutes in all.
  - ue-motion 154 s; l1 102; l2 51 (rejected at start: a screen constant missing from the whitelist); l2b 113; l3 28; l4 105; l5 101; l6 51; l7 67; final 124.
  - l4 started after m7-u0's request for the slot. That was an error, recorded in the schedule; the unit then waited for m7-u0's release.
  - No game ran. No window, focus change or capture was used.
  - During l7 and the final Unreal control, the GPU read 10-21% busy before some launches, and several visits read contended. Ranges use clean visits only (primaries ≤ 0.45 ms).
- **Time (UTC):**
  - 11:00-11:20: reading.
  - 11:20-11:25: the Unreal script and its lease.
  - 11:25-11:55: hillclimb implementation and screens.
  - 11:55-12:00: the final lease and gate.
  - Then this record.

  GPU leases took about 15 minutes; reading, implementation and analysis took the rest.
