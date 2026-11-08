# M7-R0b: test-only RT reflection probe

Base `bce9924`, branch `t3/m7-r0b`. This is a diagnostic for R1, **not a shipping reflection implementation**. The mirror identity and production-program controls pass. RF1–RF10 are measured statically at native 1920×1080; the complete Unreal floor is not met. No production setting or default changes.

## Implementation and contract

`src/reference.rs` registers `reflection_probe.rs` only under `cfg(test)`. Its typed `Mode` and per-dispatch `ProbeSettings` are the test-only setting; no environment switch can enable it in a library/viewer build. The harness owns its geometry, AS, records and pipelines. Each fixture is uploaded once for all configurations. Mirror materials use one reflected ray from that pixel's traced primary hit. Glossy materials use K=4/8 fixed equal-probability radial strata of the GGX visible-normal lobe, with a fixed per-pixel azimuth rotation. This is deterministic quadrature, without history or a denoiser.

The secondary material ID selects its own unchanged standard or toon source. Both `surface()` and `shade()` execute at the secondary hit with its local view vector, material, UV, vertex colour, source-front flag and normal. Program entry points/types are namespaced when composing the diagnostic shader. There is no stored lit-surface output. Visibility is eight fixed world-space sun-disk rays per shaded hit; it is included in hit-shading cost.

The probe is deliberately bounded: one reflection bounce; open constant-sky diffuse and DFG specular at secondary hits; no local diffuse GI, recursive reflections, texture footprints, normal derivatives or arbitrary program registry/parameter overrides. Samples use texture level zero, and input differentials are zero. The tested toon uses its source defaults. This validates the two tested programs, not the entire programmable ABI. R1 needs full hit frames/differentials, per-program dispatch and GI before this adapter could become production code.

## Mirror-camera identity

For the plane n·x=h with unit normal n, reflection is M(x)=x−2n(n·x−h), and directions transform by its linear part H=I−2nnᵀ. At the primary plane hit P, H(d)=reflect(d,n). Because P lies on the plane, P−M(C)=H(P−C): the reflected primary segment and mirrored camera are the same forward line. Shade the next hit with view vector −H(d), and its own program produces the same outgoing radiance. A unit-white metal mirror has unit Fresnel, so the incident reflection is the final reflected term in this identity fixture.

The GPU oracle independently reflects the complete camera basis and eye across y=0 and clips against that analytic plane before tracing. It does not look up candidate hits or project/reassign secondary results. Reflecting only eye/target with an ordinary look-at camera reverses the horizontal pixel order; the complete-basis oracle preserves the mirrored pixel lattice. The candidate traces its primary first and launches the independent reflection from that hit.

The fixture is 256×144, with a white planar mirror and a spatially varying vertex-colour target, tested with standard and toon programs. All RGB channels of every pixel remain. Precision was registered before GPU testing as `|candidate−camera| ≤ 0.002 cd/m² + 2e−5 |camera|`, covering f32 interpolation and the separately offset origins. Repeated fixed-resource images are bit-identical.

| Program | Pixels | Maximum absolute error, cd/m² | Failed channels |
|---|---:|---:|---:|
| standard | 36864 | 0.00009155 | 0 |
| toon | 36864 | 0.00013733 | 0 |

## Independent programmable-hit control

The second control runs the unchanged production `Renderer` with traced primaries, the same target/material/program, constant sky and no sun. Its mirrored camera image is horizontally flipped to account for look-at handedness. It compares the probe's reflected hit against native primary shading, including source-back normals, vertex colour, GGX/DFG or toon rim behaviour. All channels/pixels must differ by no more than `0.002 cd/m² + 0.2% |native|`, including the native binary16 HDR store. It uses a separate production composition, hit reconstruction and shading dispatch, rather than comparing the probe to itself.

| Program | Maximum absolute error, cd/m² | Maximum relative error | Failed channels |
|---|---:|---:|---:|
| standard | 0.008256 | 0.1015% | 0 |
| toon | 0.007784 | 0.0974% | 0 |

## RF1–RF10 static scores

References and floors are read-only R0a `reference/final/manifest.json` and `scores/c-static.json`, using the same archive hashes for every comparison. Actual split halves are preserved. There is no fitted gain, noisy-pixel rejection, temporal settling or per-image tuning. The scorer evaluates every registered population and R0a's feature energy, centroid and eligible edge-spread measures. The local score JSON retains all populations, both display exposures (−14/−10), p99 errors, feature comparisons and HW-SC/SW informative comparisons. The table uses the complete reflector population and HW-Hit Epic floor; values are not noise-corrected.

R/L are RGB relative RMS and luminance relative L1 (%); ΔE is signed energy bias (%); D is display RMS in 8-bit F0 codes at −14. N is reference split-half relative RMS (%). The last column counts passed numerical population checks at K8; feature checks are additional. A population count alone never establishes acceptance.

| Fixture / pose | K8 R / L / ΔE % | UE-Hit E R / L / ΔE % | N % | D K4 / K8 / UE-Hit E | K8 population checks |
|---|---:|---:|---:|---:|---:|
| RF1 / pitch30 | 6.245 / 1.371 / -0.660 | 29.239 / 28.267 / -27.287 | 6.159 | 4.114 / 3.786 / 9.331 | 66/84 |
| RF1 / pitch20 | 66.600 / 5.734 / +0.785 | 68.343 / 19.396 / -15.501 | 66.198 | 3.639 / 3.521 / 8.184 | 67/84 |
| RF1 / pitch40 | 4.159 / 2.542 / +0.006 | 19.466 / 17.282 / -8.459 | 0.352 | 3.869 / 3.542 / 7.325 | 67/84 |
| RF2 / sweep | 87.375 / 29.429 / -17.502 | 91.335 / 35.813 / -35.206 | 6.123 | 13.175 / 12.220 / 15.195 | 160/231 |
| RF3 / pitch30 | 2.410 / 0.862 / -0.048 | 28.763 / 29.601 / -29.115 | 2.299 | 3.402 / 3.246 / 10.140 | 73/91 |
| RF4 / pitch30 | 6.473 / 1.538 / -0.547 | 29.316 / 28.780 / -26.826 | 6.378 | 4.822 / 4.476 / 10.469 | 204/266 |
| RF5 / pitch30 | 6.245 / 1.371 / -0.660 | 29.239 / 28.267 / -27.287 | 6.159 | 4.114 / 3.786 / 9.331 | 66/84 |
| RF6 / grazing5 | 72.161 / 9.337 / +9.320 | 73.954 / 19.083 / -5.444 | 71.541 | 6.749 / 6.711 / 9.529 | 51/84 |
| RF6 / grazing10 | 71.940 / 8.518 / +5.485 | 72.268 / 14.641 / -10.484 | 71.225 | 6.352 / 6.295 / 6.750 | 44/84 |
| RF7 / between-panels | 90.352 / 34.924 / -4.184 | 90.735 / 33.460 / -30.802 | 82.322 | 23.245 / 23.194 / 20.203 | 27/63 |
| RF8 / pitch30 | 12.311 / 8.461 / -1.407 | 13.524 / 8.923 / -9.536 | 0.577 | 6.753 / 6.634 / 3.913 | 29/84 |
| RF9 / pitch30 | 6.485 / 2.395 / -2.115 | 29.250 / 28.558 / -27.758 | 6.145 | 7.526 / 7.420 / 9.606 | 42/98 |
| RF10 / pitch30 | 57.900 / 19.339 / -21.544 | 58.565 / 23.968 / -25.378 | 48.427 | 10.185 / 9.941 / 15.611 | 76/98 |

**The all-measures floor remains a miss.** RF10's HDR reference remains provisional, as R0a records; no close HDR ranking is certified. High reference noise in RF1 pitch20 and RF6 also remains visible in the numbers. RF7 specifically requires recursive transport, which this one-bounce probe omits. Roughness populations on RF2 and crevices on RF8 expose the sample/transport limits. Native pixel-centre output also lacks the reference's pixel integration and Unreal TSR. The complete image/population scores retain those errors. Static measurements do not certify motion, thin-feature stability, lighting-change recovery or the non-RT path.

## Functional cost, not a performance claim

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Vulkan, Ryzen 7 7800X3D, 32 GB RAM. The test executable is a development build, with renderer code opt-level 1 and debug assertions. One warm-up plus eight measured repeats, same loaded scene/AS, and no frame history. Timings are GPU pass timestamps; readback, CPU preparation, AS construction and shader compilation are outside them. Hit shade includes the eight visibility rays. Each mirror launches only its first sample even in a glossy K run; later passes still have dispatch overhead. Foreign CPU work and desktop GPU activity may contend. These numbers cannot establish half-Unreal cost or a 2× renderer win.

Reflection trace includes ray generation, intersection, hit reconstruction/normal sampling and the record write, rather than traversal alone. Both program-shading stages include their visibility work. This deliberately large diagnostic uses 128-byte primary and secondary records plus 16-byte incident and output values: 288 bytes/pixel, or 569.53 MiB at 1080p, before geometry, textures, AS and driver allocations. It is not a production memory proposal, and total VRAM was not measured.

Each cell below is the median of that stage's eight samples, in ms; medians are descriptive and are not summed into an end-to-end claim.

| Case | K | Primary trace | Primary shade | Reflection trace | Hit shade + visibility | Resolve |
|---|---:|---:|---:|---:|---:|---:|
| RF1/pitch30 | 4 | 0.7368 | 1.3332 | 4.7828 | 3.0820 | 1.4814 |
| RF1/pitch30 | 8 | 0.7079 | 1.2778 | 9.6760 | 6.0954 | 3.1969 |
| RF1/pitch20 | 4 | 0.6686 | 1.3177 | 4.8116 | 2.7734 | 1.4657 |
| RF1/pitch20 | 8 | 0.6722 | 1.1815 | 9.7997 | 5.6091 | 3.0371 |
| RF1/pitch40 | 4 | 0.6721 | 1.5256 | 4.7886 | 2.7468 | 1.4765 |
| RF1/pitch40 | 8 | 0.6715 | 1.2470 | 9.8894 | 5.4089 | 2.9893 |
| RF2/sweep | 4 | 0.6728 | 1.3176 | 4.9763 | 2.8152 | 1.4878 |
| RF2/sweep | 8 | 0.6738 | 1.3123 | 9.9387 | 5.7555 | 2.9661 |
| RF3/pitch30 | 4 | 0.6721 | 1.3936 | 4.7674 | 2.8437 | 1.4920 |
| RF3/pitch30 | 8 | 0.6696 | 1.2748 | 9.7100 | 5.8582 | 3.0341 |
| RF4/pitch30 | 4 | 0.7076 | 1.2199 | 5.0831 | 2.8999 | 1.4774 |
| RF4/pitch30 | 8 | 0.6712 | 1.2199 | 9.7670 | 5.6736 | 3.0134 |
| RF5/pitch30 | 4 | 0.6706 | 1.2121 | 4.7574 | 2.8793 | 1.4954 |
| RF5/pitch30 | 8 | 0.6720 | 1.2117 | 9.6124 | 5.7731 | 3.0001 |
| RF6/grazing5 | 4 | 0.6717 | 1.1432 | 4.7956 | 2.5786 | 1.5404 |
| RF6/grazing5 | 8 | 0.6711 | 1.1435 | 9.6057 | 4.8934 | 2.9584 |
| RF6/grazing10 | 4 | 0.6712 | 1.1574 | 5.0361 | 2.5455 | 1.4544 |
| RF6/grazing10 | 8 | 0.6716 | 1.1547 | 9.8018 | 5.1781 | 3.1192 |
| RF7/between-panels | 4 | 0.6955 | 0.9590 | 4.7702 | 3.4477 | 1.5027 |
| RF7/between-panels | 8 | 0.6707 | 0.9478 | 9.5938 | 7.1529 | 3.1933 |
| RF8/pitch30 | 4 | 0.6872 | 1.5335 | 4.8657 | 3.0356 | 1.4733 |
| RF8/pitch30 | 8 | 0.6733 | 1.5278 | 10.0250 | 6.3924 | 3.0102 |
| RF9/pitch30 | 4 | 0.6726 | 1.2648 | 4.8913 | 2.9472 | 1.4909 |
| RF9/pitch30 | 8 | 0.6822 | 1.2610 | 9.7941 | 6.0800 | 3.0432 |
| RF10/pitch30 | 4 | 0.6836 | 1.0042 | 4.7581 | 2.7463 | 1.4724 |
| RF10/pitch30 | 8 | 0.6706 | 1.0028 | 9.8384 | 5.5341 | 3.0157 |
| quarry-Primary-k1 | — | 1.2762 | 1.2507 | 0.0000 | 0.0000 | 0.0000 |
| quarry-Reflections-k4 | — | 1.2829 | 1.2548 | 7.2208 | 2.9716 | 1.4764 |
| quarry-Reflections-k8 | — | 1.2512 | 1.2293 | 14.1786 | 5.8422 | 3.0183 |

The actual quarry loads its existing prepared full-detail asset, 13,034,903 placed triangles. R0a found all twelve materials at roughness .85. Therefore K4/K8 here are explicitly **forced all-roughness stress rows**, with level-zero textures and a constant sky, not the proposed R1 B0/B1 workload or a matched Unreal quarry quality comparison. Its AS/resource/pipeline initialization wall is recorded separately in the quarry JSON. The nominal <.5 B0/B1 population on this asset is empty; an R1 rough-band/visibility experiment is still needed.

## Claim avoidance and source provenance

This follows [P0 §4.8–4.10](m7-p0-claims.md) and constraints C1/C7, an engineering mapping rather than legal clearance. Candidate rays are coupled to their own traced primary hit and write only their own pixel. There is no common projection, grouping into a reflector projection, matching or reassignment, keeping US 12,051,145 claim 1's decoupling/common-projection/matching elements absent. Traced primaries alone are not the avoidance.

Glossy sampling is fixed stratified VNDF, without reservoir/RIS, prior light selections, spatial reuse or history. No WRC is read or written, so the US 11,315,310 reservoir-plus-GI-data-structure combination is absent. There are no repeated deviated carrier projections (US 10,565,776 / 10,789,759), triangle shading grids, per-cell launches or cache averages. The mirrored-camera render exists solely as an independent analytic-plane-clipped test oracle. No variance clipping, SVGF or patent fallback is used. No Epic, NVIDIA SDK or third-party shader source was copied; production Renderlab helpers and programs are composed from this repository.

## Recommendations for R1

1. Keep the per-pixel traced-hit contract and shade the hit's actual program. Preserve the mirror-camera and native-primary controls as hard contract gates. Add textured, parameterized and non-flat-normal controls when full hit frames/differentials exist.
2. Use the measured mirror result as the undenoised B0 starting point. R1 still needs pixel integration/temporal validation, off-screen and thin-feature motion checks, and equivalent non-RT quality.
3. Do not promote fixed K4/K8 glossy sampling as meeting the Unreal floor. Integrate secondary-hit incident GI, recursive specular policy, finite-sun/specular AA and correct texture footprints, then measure the residual sampling error by roughness. More samples remain a permitted first response; these static misses do not trigger a reservoir or patented fallback.
4. Benchmark the retained production design in an optimized build and final quiet window. This deliberately separate trace/shade/resolve harness prices the work but is not a production performance result. The quarry's .85 materials belong to the rough-band experiment, not a misleading mirror/glossy speed comparison.

## Verification, identity and retention

`cargo test -p renderlab --lib -j 2 -- --test-threads=2`: 138 passed, 138 ignored, exit 0. Fmt is clean. `verified-source-identity.json` ties the tested sources to the pinned executable; the committed evidence summary retains its binary/source hashes, the reference manifest/floor hashes, identity errors, per-case cost/score summaries and lease history. GPU runs are headless, under `D:/Coding/renderlab-work/gpu-slot.lock`, with game/process/memory guards and no work inside foreign QUIET windows. CPU-only tools announce in `E:/renderlab-work/gpu-schedule.md`.

Implementation wall was not instrumented separately; coding/preparation overlapped verification. Cargo/fmt duration and guarded GPU-process/lease durations are separate in the evidence. They are not GPU busy time or renderer startup latency. A CPU image-byte hash check also found all eight pilot/final RF1/RF2 states bit-identical across independently built scenes and different binaries/device feature sets; this is limited evidence for those states, not general independent-AS determinism. There is no shipping quality, motion, non-RT, total-VRAM or comparative speed acceptance in R0b.

Canonical reference archives and Unreal packages remain untouched until integrated M7 acceptance. Compact candidate archives support CPU rescoring; JSON and inspected comparison PNGs remain under `E:/renderlab-work/m7-r0b/`. Disposable f32 staging images and superseded pilot intermediates are pruned. The owned Cargo target is removed at handoff. `docs/milestones-5-6-decisions.tsv`, standard/GI/environment/temporal/shadow files and production defaults are unchanged.

Reproduce with `py -3 scripts/m7-r0b.py checks --full`; guarded `run --label identity-final --test gpu_r0b_identity`; `run --label final --test gpu_r0b_program_primary,gpu_r0b_fixtures,gpu_r0b_quarry`; `score --label final`; and `py -3 scripts/m7-r0b-report.py`. All run commands block on their own leases and finish or fail within the turn. See the retained recipes for exact binary and scene identities.

AA state (IQ1, 2026-11-08; [iq1-quarry-image.md](iq1-quarry-image.md)): this unit's quarry and fixture captures rendered 1x with TR1 off and anisotropy 1, the shipping default (un-anti-aliased).
