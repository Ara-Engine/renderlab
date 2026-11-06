# Non-RT path: the same rays, traced in software

Design decision, 2026-11-06, base `3bf0558`. It answers the standing rule in AGENTS.md: win with and without hardware ray tracing, with the same film-quality gates on both paths. CPU reading and arithmetic only. There was no prototype, GPU run, build, window or Unreal launch. Labels: **M** is measured in the cited records; **I** is an inference or estimate.

## Decisions

1. **One contract, two ray backends.** The non-RT path traces exactly the rays the RT path traces: the same primaries, receivers, 10 µm view-side ε policy, direction sequences and estimators (Hard, Interactive, Exact, T1a). It traces them in WGSL compute against a cooked 8-wide BVH over the same full-detail geometry. The hit record, programs and shading dispatch are unchanged.
   - Quality cannot diverge by construction. Every gate runs on both backends against the same R_L, R_D and R_F references, which need no regeneration, because geometry and estimand are identical.
   - The only expected differences are intersection rounding at triangle edges. They are enumerated and classified, as in the [primary-ray probe](m6-primary-ray-probe.md).
2. **Primary visibility.** The 16 B hit record (placement, primitive, barycentrics) is the fixed contract. The first unit's measurement decides which pass produces it:
   - **Software primary rays** if they cost ≤ 0.40 ms. This is the simplest option: one geometry representation and identical receivers.
   - Otherwise, a **full-detail, cluster-culled raster visibility buffer.** It draws the finest virtual clusters (the original triangles) after frustum and two-phase HZB occlusion culling, into an id + depth target. The shading pass then recomputes exact pixel-centre barycentrics with the same intersection function the traced path uses, so the receiver is the exact surface point on both paths.
   - **Cluster LOD is rejected for content that fits in memory.** A 1 px cut moves receivers by up to about 7.3 mm at pose 0, against the 10 µm ε. Shadow rays from LOD receivers against a full-detail BVH self-shadow or leak, and a LOD BVH would change the casters. Simplification also deletes the sub-footprint relief that carries most of the film error: 74% of covered pixels have ≥ 20° normal spread ([film gap §2](m6-film-gap.md#2-radiance-against-the-film-image)). LOD returns only for larger-than-memory scenes, with LOD-consistent cluster BVHs on both backends.
3. **Shadows: option (a), a software BVH running the same estimators.** Virtual shadow maps with SMRT, hybrids and distance fields are rejected as core shadows ([table below](#shadow-options)). On non-RT, a single software hard ray also replaces the stippling shadow-map adapter as the hard mode.
4. **Motion AA.** T1a runs unchanged on non-RT primaries. T2 stays stopped on both backends. Motion is labelled "Interactive 1x in motion, below the film bar" on both ([§ Motion](#motion-aa-and-film-convergence)).
5. **Measurement.** A typed backend switch, with paired in-process timing. Proof that no hardware RT is used comes from runs on a device created without ray-query features ([§ Measuring](#measuring-both-paths-on-the-rtx-4070-super)).
6. **First unit: N0, a software BVH traversal probe on the quarry** ([§ Units](#units-and-stop-rules)). Throughput of software rays is the single unknown that decides cost for primaries, shadows and every ray-based mode. It has no fixed-function fallback for shadows. Its CPU half can start now, while Islam plays.
7. **Expected result (I).** Non-RT should match RT on image quality and cost 1.0–2.3 ms per frame, against matched Unreal VSM's 1.61 ms static. That is between 1.6x faster and 1.4x slower than VSM, at measurably higher quality. **The 2x VSM floor (0.80 ms) is not reachable on this content on either backend**: the RT Interactive frame is already 0.82–0.87 ms (M).

## Per-pass budgets

Quarry: 12 draws, 13,034,903 triangles, 1920×1080, 1x, directional light with a 0.04 rad disk. RT values are measured ([primary-ray production](m6-primary-ray-production.md), [Interactive climbs](m6-interactive-climb.md), [receiver follow-up](m6-receiver-followup.md)). Non-RT estimates assume software rays cost 2–5x hardware rays per ray (I). That ratio has no Renderlab measurement; N0 measures it.

| Pass | RT (M) | Non-RT budget | Non-RT estimate (I) |
|---|---:|---:|---:|
| Primary visibility | 0.23 traversal + 0.07 fixed (fused pass) | ≤ 0.40 | Software rays 0.46–1.15 + 0.07; culled raster 0.30–0.50 |
| Shadow visibility, Interactive (8 rays) | 0.12 (fused) | ≤ 0.35 | 0.24–0.60 |
| Shading (same program and dispatch) | 0.396 | 0.42 | 0.40–0.43 (the raster producer adds barycentric recompute) |
| Tone | 0.049 | 0.05 | 0.05 |
| T1a static accumulation | +0.025 | 0.03 | 0.03 |
| Acceleration maintenance | AS build 192 ms once | 0 when static | Cooked BLAS. Top level over 12 instances per mover frame: < 0.01 |
| **Interactive frame** | **0.82–0.87** | **≤ 1.20** | **1.0–2.3** |
| Hard frame (1 ray) | 0.82–0.96 (the inline ray is about 0.19 of it) | ≤ 1.20 | 1.0–2.3. The cold first ray dominates, so Hard costs about what Interactive does |
| Exact visibility (reference mode only) | 7.6–10.0 | none | 15–50 |
| Memory | AS 912 MB + 276 MB scratch | ≤ 0.5 GB | 0.15–0.35 GB: nodes plus primitive references into the existing vertex and index buffers |

Against Unreal on this GPU (M, [end-to-end comparison](quarry-end-to-end-comparison.md)): VSM static 1.61 ms, HRT 1.75 ms. The 2x floors are 0.80 ms (VSM) and 0.87 ms (HRT). Lumen software is not compared, because neither side has GI in the matched workload. It becomes the non-RT GI denominator when Renderlab's GI lands.

**Mid-range non-RT GPU class (I, wide).** Take a GTX 1660 Super / GTX 1070 / RX 5700 class card: about 4.5–7x less FP32, 1.5–4 MB of L2 against 48 MB, and 256–448 GB/s. Software traversal was locality-limited on the hardware path ([fresh compute cost](m6-fresh-compute-cost.md#limiter)), so it should lose more than the FLOP ratio: primaries plus 8 rays at about 3.5–17 ms, shading 1.6–2.8 ms, and a frame of about 5–20 ms at 1080p. On that class the culled raster visibility buffer is likely needed, because fixed-function raster scales better than software traversal. Unreal VSM on that class is unmeasured. No mid-range claim is made until N3.

## Shadow options

| Option | Quarry cost (I) | Mid-range (I) | Quality against the film reference | Beats Unreal VSM? | Verdict |
|---|---|---|---|---|---|
| **(a) Software BVH, same estimators, receiver policy and edge handling** | Interactive 0.24–0.60 ms; Hard 0.1–0.5; Exact 15–50 | 1.5–5 ms (Interactive) | Same as the RT path: Level B U- and S-checks pass with 0 solid flips and 0 lost contacts (M on RT). Film parity with RT, including T1a static convergence | Quality: yes. VSM loses 31% of contacts, makes 2,191 solid flips and has faint RMS worse than omission (M, [Level B](m6-level-b.md#what-unreals-error-against-the-reference-contains)). Cost: 0.4–1.0 ms of rays plus primaries, against VSM's whole 1.61 ms frame | **Chosen** |
| (b) Virtual shadow maps + shadow-map ray marching, held to the film gate | Raster of all 13 M casters: about 3 ms per map (M: three 4096 maps took 6.2–10.1 ms). With cluster culling and page caching, cheap while static; a moving sun invalidates every page | Raster-bound; likely cheaper than (a) when static | Fails structurally. Light-space texels are about 23 mm against mm relief; the 1024 maps stipple and 4096 still speckles (M, [m6-shadows](m6-shadows.md)). A depth map needs receiver bias, which erases contacts (UE VSM, M). SMRT cannot represent sub-texel micro-occlusion, which carries the film partials | Cost possibly; quality no | **Rejected as core** |
| (c) Hybrid: maps for umbra and bulk, software rays near edges | Saves ≤ 8%: rays capped at 25 cm keep 0.917 of the cost (M, hardware). For 8 rays, rendering the map costs more than the rays it saves | Same argument | Equal to (a) only where the rays decide | No gain | **Rejected.** Reopen only if N0 shows long rays dominating software cost, the opposite of the hardware measurement, as a cost lever under unchanged gates |
| (d) Distance-field cone tracing, Lumen-SW style | Low | Low | mm contacts need mm voxels, far beyond memory for 13 M triangles of scan. A global field loses contacts | No | Rejected for core shadows; possible later for GI far field |

## Motion AA and film convergence

- **T1a** needs exact surface identity and one jittered primary per frame. Software rays jitter natively. The raster producer jitters the projection by the sample's sub-pixel offset, and shading intersects the jittered ray with the identified triangle. T1a's code is unchanged; its static gates rerun on the software backend against the existing references.
- **T2** stopped on cost on the RT path: 3.55 ms (K = 4) and 5.76 ms (K = 8) moving, with exact discontinuity rays taking 0.6–1.15 ms of that (M, [T2a](m6-motion-aa-gpu.md#6-cost)). On software those rays cost 2–5x more. Its gathers are not rays and cost the same on both backends. Any T2 successor, such as the scatter lever, is priced and gated on both backends.
- **Labelling.** Until a motion method passes Level F, motion is "Interactive 1x in motion, below the film bar" on both backends. It is never a default and never presented as meeting the bar.

## Measuring both paths on the RTX 4070 SUPER

- **Switch.** `Renderer::set_ray_backend(RayBackend::Hardware | RayBackend::Software)`, with the backend each frame used reported in `FrameReport`. Software works on a renderer whose device has no ray-query features.
- **Proof of non-RT.** Each gate also runs once in a process whose device is created without `EXPERIMENTAL_RAY_QUERY` and acceleration-structure features, so wgpu rejects any hardware use. Its outputs must be byte-identical to the paired run's software outputs.
- **Timing.** Paired in-process on one RT-capable device, alternating backends, 24 pairs, the existing fresh-screen method, with fast and slow GPU state reported. On the software backend the RT cores sit idle; the SMs, L2 and memory system are the same.
- **Unreal.** The matched VSM static, camera and sun rows already exist. A moving-sun comparison is the case most likely to favour Renderlab, because ray cost does not depend on light motion while VSM pages invalidate (I).
- **What this GPU cannot tell us:**
  - software traversal on small-L2, low-bandwidth GPUs, since 48 MB of L2 likely flatters a locality-limited traversal;
  - AMD wave64 and RDNA1, Intel, and Apple compiler behaviour on long WGSL traversal loops. wgpu exposes no ray queries on Metal, so Apple M1/M2 is a real non-RT target;
  - 8 GB VRAM capacity with 0.47 GB of geometry, 1.07 GB of 4K textures and the BVH;
  - Unreal's VSM cost on those devices.

  These need a real device. **Decision for Islam:** which non-RT target class to buy or borrow. Proposed: one of GTX 1660 Super / GTX 1070, one RX 5700-class card, and Apple M1/M2. Until then, every non-RT result is labelled "RTX 4070 SUPER, RT units idle".

## Units and stop rules

**N0: software traversal probe (first unit).** Test-only, no production change. Sol implements; Opus accepts. **Result ([non-rt-n0.md](non-rt-n0.md)):** parity passes. Software 8-ray shadows cost 9.43 ms, 21.9x hardware, so the shadow stop rule triggered and N1 is not started.

- **Files owned:**
  - `src/software_bvh.rs`: CPU builder, GPU layout and a CPU reference traversal;
  - `shaders/software_bvh.wgsl`: closest-hit and any-hit traversal and the watertight triangle test;
  - `src/software_bvh_tests.rs`: CPU tests, plus GPU tests in the style of `primary_ray_gpu_tests.rs`.

  Read, without editing: `shaders/primary_trace.wgsl` and the Interactive kernels, for ray setup, the direction sequence and the adopted ε. The existing hardware path is the comparison.
- **Builder.** One BLAS per mesh: binned SAH (32 bins), collapsed to 8-wide, leaves of at most 4 triangles. Leaves reference triangles through a reordered primitive-index list into the existing vertex and index buffers, with no triangle copy.
  - Step 1 uses uncompressed f32 child boxes for a simple, exact start. Quantized compressed-wide nodes (Ylitie et al. 2017) are the first hillclimb step.
  - The top level is a brute-force loop over the 12 instance bounds, with rays transformed to object space as the hardware does.
  - Building per mesh keeps the CPU peak near 100–200 MB. A third-party builder crate needs a license and notice check first.
- **Intersection.** The watertight test (Woop, Benthin and Wald 2013) in f32, with the hardware path's barycentric convention. Shadow rays are opaque any-hit with first-hit termination, t_min = ε and t_max = ∞.
- **Kernels:**
  1. A pixel-centre closest-hit primary that writes the 16 B hit record.
  2. The 8 Interactive directions from the hardware path's own exact receivers. This isolates shadow traversal from primary differences. It also runs the full 256 directions on a 64×64 crop for Exact parity.
  3. A fused primary plus 8 rays: the non-RT Interactive trace.
- **CPU checks, which can run now:**
  - every triangle lies in exactly one leaf, and every box contains its children;
  - the CPU traversal equals brute force on the small fixtures, and on 10,000 random quarry rays per mesh (hit distance and primitive);
  - naga validation of the WGSL;
  - build time and bytes reported per mesh.
- **GPU checks, only when Islam is not playing:**
  - hit-record differences against hardware at poses 0, 1 and 2: coverage, placement, primitive, and world position beyond 0.1 mm, classified as silhouette, edge tie or other;
  - per-receiver 8-ray counts against hardware, with mismatches classified by f64 replay as in the [receiver follow-up](m6-receiver-followup.md);
  - paired timing of the three kernels against hardware: one process × 24 pairs first, more only near a threshold;
  - validation active on the exactness runs.
- **Hillclimb.** At most three retained mechanisms, one at a time, each exact against the CPU reference. Candidates: quantized nodes, leaf size, and stack placement (private array against workgroup memory).
- **Stop rules, frozen now:**
  - An unexplained "other" mismatch in hit records or counts: fix the implementation, never the comparison.
  - Software 8-ray shadows above 1.0 ms (more than 8x hardware) after the hillclimb: stop and report the traversal limiter before any production work.
  - The projected non-RT Interactive frame is the fused software trace + 0.45 ms + 0.03 ms:
    - ≤ 1.20 ms: proceed to N1. Use software primaries if they are ≤ 0.40 ms, otherwise also run N1b.
    - 1.20–1.61 ms: proceed. Record that the frame beats VSM by less than 1.33x and misses 2x.
    - Above 1.61 ms: proceed only as a quality-first non-RT mode and record the cost miss. Price the far-field map lever of option (c) under unchanged gates. No quality reduction.
  - BVH memory above 0.5 GB: compress before N1.
- **Expected verification:** CPU build and tests under a minute for the quarry, 2–4 release test builds and about 10 GPU windows of 90 s or less.

**N1: production software backend.** Opt-in `RayBackend::Software`, defaults unchanged. Scope: regular full-resident geometry, 1x, a directional light, and the Hard, Interactive, Exact and T1a static modes.

- **Gates:**
  - Level A parity with the RT counts, with classified rounding mismatches only;
  - Level B U- and S-checks;
  - Level F static (T1a) against the existing references;
  - the standard and toon program suites, mixed-program frames and reload transactions;
  - the no-ray-query device run;
  - paired timing against RT and against VSM's 1.61 ms.
- **Stop:** any F4 excess over the RT path, any gate the RT path passes and software fails, or the N0 cost rules recomputed on production code.

**N1b: culled raster visibility buffer.** Runs only if N0's software primaries exceed 0.40 ms. Full-detail finest clusters, GPU frustum and two-phase HZB culling, one indirect draw, an id + depth target and exact barycentrics in shading.

- **Gate:** hit records equal the traced ones except classified silhouette and tie pixels; Level F static unchanged; primary producer ≤ 0.40 ms.
- A raster win also transfers to the RT path, whose primaries cost 0.30 ms.

**N2: motion and movers on both backends.** A per-frame top level for rigid movers. Q-walk, Q-blocker and Q-sun under Level F, with the labels above. Deforming geometry needs BLAS refit on both backends; it is already unsupported (C13).

**N3: a real non-RT GPU,** after Islam's device decision. Run N1's gates and timing, plus matched Unreal VSM on the same device.

**Later: larger-than-memory scenes.** Cook a small BVH per virtual cluster, and build a per-frame top level over the selected cut. Shadows then trace the same LOD as primaries. wgpu exposes no cluster acceleration structures, so the software backend may become the LOD backend for RT GPUs too.

## Where non-RT matches or beats RT, and where it cannot

- **Matches:** the image on every current gate, because the rays and estimand are the same and the references are shared.
- **Can beat (I, until N0):**
  - memory: 0.15–0.35 GB against the 0.91 GB AS;
  - bit-reproducible intersection across vendors, since the watertight test is Renderlab's own and hardware intersection numerics are vendor-defined;
  - cluster-LOD BVHs, which wgpu does not expose for hardware;
  - no dependence on experimental wgpu ray-query features.
- **Cannot:**
  - per-ray cost, estimated at 2–5x hardware;
  - every ray-hungry mode scales with that ratio: Exact (15–50 ms), T2's validation rays and future GI;
  - the 2x VSM floor, which the RT path already misses;
  - on small-L2 GPUs the gap likely widens (N3).

The industry-cost audit had already recommended a software wide BVH as the gate backend, with hardware as a labelled diagnostic ([research §5](research/industry-rendering-costs.md#5-prioritized-experiments--proposed-not-run)). This design makes both backends first-class instead.

## Rejected options

| Option | Reason |
|---|---|
| Shadow maps or VSM + SMRT as core non-RT shadows | Light-space texel aliasing on mm relief; bias erases contacts; fails Level B composite and the film gate (M on Renderlab maps and UE VSM) |
| Hybrid maps plus rays | Saves ≤ 8% (M, hardware near-field share) |
| Distance-field tracing for core shadows | Cannot hold mm contacts |
| Cluster-LOD visibility buffer on content that fits | Receiver/BVH mismatch against the 10 µm ε; deletes the relief that carries the film error |
| Raster visibility buffer of all 13 M triangles, unculled | About a 3 ms floor (M) |
| Screen-space contact shadows to repair map contacts | AGENTS.md policy: no screen-space core shadows |
| A separate non-RT estimator or separate references | Lets quality diverge; doubles gate work |
| Waiting for a wgpu software ray-query fallback | None exists; Metal exposes no ray queries |

## Risks

- **Software throughput is unmeasured.** The 2–5x ratio is an inference; N0 decides, and every non-RT figure above moves with it.
- **WGSL limits:** no traversal intrinsics, private-array stacks spill to local memory, no 8-bit loads, and long loops compile differently per driver.
- **Rounding at triangle edges** differs from hardware. Parity holds statistically and by classification, not bitwise.
- **This GPU flatters software traversal** through its 48 MB L2. Mid-range figures are wide guesses until N3.
- **8 GB cards:** geometry, 4K textures and the BVH together approach their capacity.
- **Two primary producers** could fork the receiver contract. Using one shared intersection function for barycentrics mitigates it.

## Evidence and time

Inputs are the cited records only. No CPU prototype was run: a BVH built over 13 M triangles in a CPU script would exceed the 1.5 GiB limit for prototypes and would not answer the GPU throughput question, which N0's builder and kernels do directly. Reading and writing took about an hour and were not stopwatched. There was no GPU, window, build or Unreal launch.
