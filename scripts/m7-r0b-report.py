"""Write R0b's small durable report from retained local measurement summaries."""
import importlib.util
import json
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
OUT = Path("E:/renderlab-work/m7-r0b")
spec = importlib.util.spec_from_file_location("r0b", REPO / "scripts/m7-r0b.py")
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)


def main():
    b.wait_cpu()
    b.note("write evidence report from retained JSON")
    run = OUT / "runs/final"
    scores = json.loads((OUT / "final-scores.json").read_text())
    assert scores["complete"] and len(scores["rows"]) == 26
    floor = json.loads((b.r.OUT / "scores/c-static.json").read_text())["rows"]
    identity = [json.loads((OUT / "runs/identity-final" / f"identity-{p}.json").read_text()) for p in ("standard","toon")]
    programs = [json.loads((run / f"program-{p}.json").read_text()) for p in ("standard","toon")]
    assert all(x["failed_channels"] == 0 for x in identity + programs)
    evidence = dict(base="bce9924", scope="R0b test-only RT/static, no production default changes",
        checks=json.loads((OUT / "verified-source-identity.json").read_text()), identity=identity, programmable_primary=programs,
        reference_manifest_sha256=scores["reference_manifest_sha256"],
        floors_sha256=b.r.sha(b.r.OUT/"scores/c-static.json"), rows=[], quarry=[],
        leases=json.loads((OUT/"lease-history.json").read_text()),
        pilot_replay=json.loads((OUT/"pilot-replay-and-prune.json").read_text()))
    text = ["""# M7-R0b: test-only RT reflection probe

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
|---|---:|---:|---:|"""]
    for x in identity:
        text.append(f"| {x['program']} | {x['pixels']} | {x['maximum_absolute_radiance_error']:.8f} | {x['failed_channels']} |")
    text.append("""
## Independent programmable-hit control

The second control runs the unchanged production `Renderer` with traced primaries, the same target/material/program, constant sky and no sun. Its mirrored camera image is horizontally flipped to account for look-at handedness. It compares the probe's reflected hit against native primary shading, including source-back normals, vertex colour, GGX/DFG or toon rim behaviour. All channels/pixels must differ by no more than `0.002 cd/m² + 0.2% |native|`, including the native binary16 HDR store. It uses a separate production composition, hit reconstruction and shading dispatch, rather than comparing the probe to itself.

| Program | Maximum absolute error, cd/m² | Maximum relative error | Failed channels |
|---|---:|---:|---:|""")
    for x in programs:
        text.append(f"| {x['program']} | {x['maximum_absolute_radiance_error']:.6f} | {100*x['maximum_relative_error']:.4f}% | {x['failed_channels']} |")
    text.append("""
## RF1–RF10 static scores

References and floors are read-only R0a `reference/final/manifest.json` and `scores/c-static.json`, using the same archive hashes for every comparison. Actual split halves are preserved. There is no fitted gain, noisy-pixel rejection, temporal settling or per-image tuning. The scorer evaluates every registered population and R0a's feature energy, centroid and eligible edge-spread measures. The local score JSON retains all populations, both display exposures (−14/−10), p99 errors, feature comparisons and HW-SC/SW informative comparisons. The table uses the complete reflector population and HW-Hit Epic floor; values are not noise-corrected.

R/L are RGB relative RMS and luminance relative L1 (%); ΔE is signed energy bias (%); D is display RMS in 8-bit F0 codes at −14. N is reference split-half relative RMS (%). The last column counts passed numerical population checks at K8; feature checks are additional. A population count alone never establishes acceptance.

| Fixture / pose | K8 R / L / ΔE % | UE-Hit E R / L / ΔE % | N % | D K4 / K8 / UE-Hit E | K8 population checks |
|---|---:|---:|---:|---:|---:|""")
    for row in scores["rows"]:
        if row["rays"] != 8:
            continue
        other = next(x for x in scores["rows"] if x["fixture"]==row["fixture"] and x["pose"]==row["pose"] and x["rays"]==4)
        f = next(x for x in floor if x["fixture"]==row["fixture"] and x["pose"]==row["pose"] and x["config"]=="hw-hit-epic")
        m=row["measures"]["reflector"]; c=row["comparisons"]["hw-hit-epic"]
        fm=f["measures"]["reflector"]
        text.append(f"| {row['fixture']} / {row['pose']} | {100*m['rgb_relative_rms']:.3f} / {100*m['luminance_relative_l1']:.3f} / {100*m['signed_energy_bias']:+.3f} | {100*fm['rgb_relative_rms']:.3f} / {100*fm['luminance_relative_l1']:.3f} / {100*fm['signed_energy_bias']:+.3f} | {100*m['split_noise_relative_rms']:.3f} | {other['measures']['reflector']['display_rms_-14']:.3f} / {m['display_rms_-14']:.3f} / {fm['display_rms_-14']:.3f} | {c['passed']}/{c['tested']} |")
    text.append("""
**The all-measures floor remains a miss.** RF10's HDR reference remains provisional, as R0a records; no close HDR ranking is certified. High reference noise in RF1 pitch20 and RF6 also remains visible in the numbers. RF7 specifically requires recursive transport, which this one-bounce probe omits. Roughness populations on RF2 and crevices on RF8 expose the sample/transport limits. Native pixel-centre output also lacks the reference's pixel integration and Unreal TSR. The complete image/population scores retain those errors. Static measurements do not certify motion, thin-feature stability, lighting-change recovery or the non-RT path.

## Functional cost, not a performance claim

RTX 4070 SUPER 12 GB, NVIDIA 617.14, Vulkan, Ryzen 7 7800X3D, 32 GB RAM. The test executable is a development build, with renderer code opt-level 1 and debug assertions. One warm-up plus eight measured repeats, same loaded scene/AS, and no frame history. Timings are GPU pass timestamps; readback, CPU preparation, AS construction and shader compilation are outside them. Hit shade includes the eight visibility rays. Each mirror launches only its first sample even in a glossy K run; later passes still have dispatch overhead. Foreign CPU work and desktop GPU activity may contend. These numbers cannot establish half-Unreal cost or a 2× renderer win.

Reflection trace includes ray generation, intersection, hit reconstruction/normal sampling and the record write, rather than traversal alone. Both program-shading stages include their visibility work. This deliberately large diagnostic uses 128-byte primary and secondary records plus 16-byte incident and output values: 288 bytes/pixel, or 569.53 MiB at 1080p, before geometry, textures, AS and driver allocations. It is not a production memory proposal, and total VRAM was not measured.

Each cell below is the median of that stage's eight samples, in ms; medians are descriptive and are not summed into an end-to-end claim.

| Case | K | Primary trace | Primary shade | Reflection trace | Hit shade + visibility | Resolve |
|---|---:|---:|---:|---:|---:|---:|""")
    for row in scores["rows"]:
        med=np.median(row["gpu_stage_ms"],axis=0).tolist()
        text.append(f"| {row['fixture']}/{row['pose']} | {row['rays']} | "+" | ".join(f"{x:.4f}" for x in med)+" |")
        comparisons={name:{key:value[key] for key in ("passes","passed","tested","features_passed","features_tested")} for name,value in row["comparisons"].items()}
        evidence["rows"].append({k:row[k] for k in ("fixture","pose","rays","scene_sha256","reference_archive_sha256","candidate_archive_sha256","same_resource_replay_bit_identical")} | dict(comparisons=comparisons,reflector=row["measures"]["reflector"],median_stage_ms=med))
    for name in ("quarry-Primary-k1","quarry-Reflections-k4","quarry-Reflections-k8"):
        x=json.loads((run/(name+".json")).read_text());med=np.median(x["gpu_stage_ms"],axis=0).tolist()
        text.append(f"| {name} | — | "+" | ".join(f"{v:.4f}" for v in med)+" |")
        evidence["quarry"].append(dict(name=name,extra=x["extra"],median_stage_ms=med,gpu_stage_ms=x["gpu_stage_ms"]))
    text.append("""
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
""")
    evidence["visual_audit"]=json.loads((OUT/"visual-audit.json").read_text())
    evidence["checks"]["summary"]="138 passed, 138 ignored; fmt exit 0"
    (REPO/"docs/m7-r0b.md").write_text("\n".join(text).rstrip()+"\n",encoding="utf-8")
    (REPO/"docs/m7-r0b-evidence.json").write_text(json.dumps(evidence,indent=2)+"\n",encoding="utf-8")


if __name__ == "__main__":
    main()
