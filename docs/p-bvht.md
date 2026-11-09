# P-BVHT: packed software-GI BVH timing

2026-11-10 Qatar / 2026-11-09 UTC. Measurement only, branch `t3/p-bvht`,
starting at main `76f07a1`. **KEEP the packed layout. The reported 5–6% quarry
slowdown does not reproduce after separating the tails.** This is bounded
regression clearance for the frozen pair, not proof of zero cost on all workloads.
No renderer change or variant build was made.

## Quiet quarry result

Release/Vulkan, Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14.
ARA1 quarry-daylight, 13,034,903 triangles, 1920×1080, authored settings unchanged,
non-RT only. Synchronous drained service includes CPU submission and completion;
GPU span is a separate timestamp measurement. No clock locking. This is not an
Ara/Unreal comparison or a current-main benchmark.

Two consecutive ABBA blocks, eight processes total. Each loads the asset once
and renders four identical 88-pose tracks: **16 tracks per binary**. Each track's
moving/recovered observation averages eight frames (64–71 / 80–87); cut is frame
72. Statistics below operate on those track observations. P90 is nearest rank;
all samples, including tails, remain in these results. Tracks within a process
are correlated; no independent-sample confidence interval is claimed.

Service milliseconds, unpacked → packed:

| Phase | Median | Change | Mean | P90 |
|---|---:|---:|---:|---:|
| Moving | 8.7766 → 8.8154 | +0.44% | 10.2969 → 10.0534 | 14.8184 → 14.2213 |
| Cut | 38.8012 → 38.4471 | −0.91% | 39.7178 → 39.4493 | 47.7381 → 47.9573 |
| Recovered | 9.8808 → 9.7777 | −1.04% | 11.7094 → 11.5785 | 15.6932 → 15.5712 |

GPU-span milliseconds:

| Phase | Median | Mean | P90 |
|---|---:|---:|---:|
| Moving | 7.2321 → 7.2014 | 8.7814 → 8.4957 | 13.0477 → 12.6319 |
| Cut | 36.8082 → 36.3028 | 37.7672 → 37.4826 | 45.8394 → 46.2203 |
| Recovered | 8.2597 → 8.2284 | 10.1928 → 10.0246 | 14.1316 → 14.0092 |

The moving service median rises only 0.0388 ms while its GPU median falls
0.0307 ms. Means improve in every requested phase. Cut P90 rises 0.2192 ms
(0.46%), with lower median and mean. There is no coherent 5–6% penalty.

A symmetric diagnostic excludes each process's first track, without any
value-based trimming (12 tracks per side). Moving service median/mean becomes
8.7005/8.6663 → 8.7686/8.6761 ms; cut 37.1080/37.1097 →
36.3916/36.7148 ms; recovered 9.6334/10.1302 → 9.7060/10.2805 ms.
The small warm recovered increase remains disclosed; it does not reproduce the
original magnitude. Full distributions, the warm split and each process's
medians are in [the results](p-bvht-results.json).

Reanalysis of P-VERIFY's existing 16 tracks per binary explains why its means
were misleading: moving median was 8.4598 → 8.5658 ms (+1.25%), cut median
37.5748 → 37.4504 ms (−0.33%), recovered 9.6277 → 9.8203 ms (+2.00%).
Its +5.5%/+6.1% moving/cut means were sensitive to broad tails in both binaries.
We do not establish what caused those tails, or certify that their frequency
is identical. The new run supports keeping the memory saving rather than
reverting on that mean-only signal.

## Identity, admission and evidence

The prebuilt pair is `dc0d4ea` plus only `src/gi_bvh.rs` and
`shaders/gi_rays_software.wgsl` from `241b87b` on the packed side:

- Unpacked SHA256: `6766d9e58aea4d30a63558ceca7beda6f231f01a475f81a7bbdc13166d3663de`.
- Packed SHA256: `fc8e651ea9993cf26657d26e9ac8dd90e0db6e07b98f4b752ac7fa551a723b05`.

All **76 float-HDR and 76 PNG witnesses per process** match the first process
at every recorded index: 532/532 cross-process comparisons of each kind, 608
observations of each kind. Scene and asset hashes also match. This covers the
registered sampled poses, not every uncaptured frame. No image-quality setting
changed. The merged-memory executable was not used.

Read the schedule as a stream before admission. Latest root order was
23:38:38 UTC, p-nrt3b then p-bvht in request order; p-nrt3b posted unit complete
at 23:40:17 with no further GPU/build requests. N8 lease ran
23:40:55.330–23:45:07.624 UTC, **252.29 seconds**, exit 0, no guard failures.
Stillkit mutex held 23:40:55.025–23:45:07.649 UTC (252.63 seconds).
Atomic build-slot directory with owner.json was held for this lease and removed
on release. Admission had 13,961 MiB available and >5 GB free on D:.
The 2 GiB runtime guard remained active. All 2,123 lease resource samples
(plus 2,061 per-process samples) report zero compiler overlap; minimum RAM
10,850 MiB. Foreground checks found no game; no window or desktop capture ran.

Local evidence: `D:/Coding/renderlab-work/p-bvht/summary.json`, `identities.json`,
`mutex.jsonl`, `lease/runs/quarry-abba.json`, and `runs/quarry/nonrt/*/engine.json`.
Reproduce using `scripts/p-bvht.py prepare`, then `scripts/p-bvht-queue.ps1`
only after reviewing the live root order. The queue joins the normal stillkit
mutex; `scripts/n8-run.py` supplies the GPU lease/guard through the adapter.
`scripts/p-bvht-summary.py` recomputes the statistics. Python syntax checked;
no Rust build/tests needed for this measurement-only unit. No target directory
was created. No raw dump >5 MB remains; JSON/logs and PNG witnesses are retained.

## Layout audit and action

The node shrinks 80→64 bytes, leaf 32→8, position 16→12. Both shaders load
XYZ as three scalar words and use the same bounds byte shifts/masks; packing
adds no new unpacking algorithm. Node stride is already 16-byte aligned, but
mesh-array bases are concatenated without per-mesh alignment padding. The
12-byte position stride can straddle cache sectors, unlike a 16-byte stride.
Those are possible performance mechanisms, not measured causes here.
P-NRT2's vec4 loads on the unpacked layout gave no established gain; do not
repeat that experiment as an assumed fix.

P-MEM's allocation evidence remains **407.074→311.933 MiB**, about **95.140 MiB
saved (23.4%)**, in the software-GI geometry buffer. This run reuses that memory
measurement; it does not remeasure DXGI reservation. Keep that saving: the
requested phase medians are within +0.44%/−0.91%/−1.04%, with lower GPU medians
and lower service means. There is no retained slowdown requiring a variant or
GI per-pass attribution. The frozen capture binaries expose primary, visibility,
shading, reconstruction and post timestamps, but not individual GI pass times;
no invented GI attribution is inferred from their residual span.

Renderer implementation/build time: zero. Verification preparation, execution,
analysis and documentation: about 12 minutes; measured GPU lease 4.20 minutes.
