"""Reproduce the R1 Markdown tables from retained measurements; no GPU work."""
import argparse
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
DEFAULT=Path("D:/Coding/renderlab-work/m6-exact-quality/r1")

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,default=DEFAULT);parser.add_argument("--out",type=Path,default=ROOT/"docs/m6-receiver-policy.md");a=parser.parse_args()
    data=json.loads((a.root/"scores/summary.json").read_text())
    windows=json.loads((a.root/"gpu-windows.json").read_text())
    lines=[]
    def say(text=""):lines.append(text)
    def table(headers,rows):
        say("| "+" | ".join(headers)+" |")
        say("|"+"|".join("---" for _ in headers)+"|")
        for row in rows:say("| "+" | ".join(map(str,row))+" |")
        say()
    fmt=lambda n:f"{n:.6f}"
    magnitude_failures=[(pose,ch,pop) for pose in ("0","1") for ch,t in data[pose]["policies"]["magnitude"]["populations"].items() for pop,row in t.items() if row["pixels"] and row["exceeds_exact"]]
    recompute=any(c["recompute_predictions"] for pose in ("0","1") for p in data[pose]["policies"].values() for c in p["convergence"].values())
    say("# M6 R1: receiver policy and radiance references\n")
    say("2026-11-05. Test-only, base `eff3553`, branch `t3/m6-receiver-policy-tests`. No production files/defaults changed. Committed locally; no push. Binding scope and stop rules: [film gap §§6–7](m6-film-gap.md). [F0](m6-film-reference.md) and [the exact-quality design](m6-exact-quality-fast-shadows.md) remain frozen evidence.\n")
    say(f"**Magnitude verdict: {'UNSETTLED' if magnitude_failures else 'passes the frozen per-population difference gate'}.** {'See the explicit failures below; F1-AA must not start.' if magnitude_failures else 'This authorizes no production policy change.'} The viewed-side variant is an estimand change presented for Islam’s decision and is **not adopted**. The production default remains Exact with its existing receiver policy.\n")
    say(f"**Convergence: {'predictions require revision' if recompute else 'the replay does not trigger the frozen revision rule'}.** The tables below give the measured N=1–16 fits and recalculated ideal accumulation predictions. No AA implementation or temporal/motion acceptance is claimed.\n")
    say("## What was measured\n")
    say("R1a (`magnitude`) reconstructs the shadow receiver from hit-triangle barycentrics, retains the light-side geometric normal, and uses ε = max(10 µm, 16 ulp(maximum absolute world-space hit-triangle vertex coordinate)) for both normal offset and ray-distance t_min. R1b (`viewed`) changes only the normal’s sign to the viewed side. Every tested material is treated as opaque. A production adoption would require an explicit material thin/opaque contract; no lighting-model data or sidedness contract was added to production.\n")
    say("Each film reference uses F0’s 256 randomized 16×16 footprint strata, independently salted 256-ray Exact visibility per sample. R_F accepts the centre placement and depth within 2%; R_L and R_D include all primary samples and silhouettes. R_D uses the identical material/BRDF, visibility and exposure, with the environment contribution and background set to black. This is Renderlab direct radiance, not a path-traced GI reference.\n")
    say("The first 16 saved samples form a Latin permutation across the 16×16 footprint, rather than F0’s first-row prefix. Every original stratum and its seed are retained in the complete 256-sample reference. Visibility is saved as u16 count/256, with 65535 indicating a rejected surface sample; full and sun-only HDR are RGBA16F. Saved samples total about 597 MB per 1080p pose/policy (including sun-only HDR). R_F uses F0’s weighted interleaved split-half σ_F. Radiance sums change accumulation order; their floating-point roundoff is reported by the minimal replay.\n")
    say("A fresh fixed-10 µm replay supplies R_D, which F0 did not retain. Its visibility, accepted counts and R_L are checked against the immutable F0 before using it. Centre Exact captures stay under the production receiver policy, so the comparison uses the frozen original Exact rather than quietly re-baselining it.\n")
    say("Hardware/workload: RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan with active Khronos validation. Registered quarry: twelve draws, 13,034,903 triangles, 1920×1080, 1x, 16x anisotropy, RLE2 environment, −14-stop exposure, 0.04-rad sun disk. Pose 1 is +1 cm X; pose 2 is an independent pose-0 process. These are reference-generation runs with readbacks and validation, not performance measurements.\n")
    say("## Analytic fixtures\n")
    say("All 24 acne cases pass: flat and convex faceted roofs, translated along all axes by 1 km / 10 km, at 80°, 85° and 89° light incidence, for both variants. Every case has 35,328 covered pixels, zero false shadows and zero visibility error. A 0.001-rad disk keeps every direction on the illuminated side at 89°; convex roof facets slope ±0.002. The complete images were inspected.\n")
    rows=[]
    for fixture in ("sawtooth","thin","stress"):
        for policy in ("magnitude","viewed"):
            d=json.loads((a.root/f"{fixture}-{policy}/validation.json").read_text());r=d["partial"]
            rows.append((fixture,policy,r["n"],fmt(r["rms"]),fmt(r["noise_rms"]),fmt(r["max"]),d["accepted_count_mismatches"],"PASS" if d["passed"] else "FAIL"))
    table(("Fixture","Policy","Partial n","RMS","σ_F RMS","Max error","Count mismatches","Verdict"),rows)
    say("The concave fixture has eight teeth at each of three sizes: approximately 2, 4 and 8 mm facet lengths, 85° incidence and a 0.04-rad disk. Its independent analytic oracle intersects the extruded piecewise-planar profile, unions projected slope intervals and integrates their continuous disk CDF. The profile oracle also passes 262,144 independent f64 rays at five probes, including three partials (maximum difference 1.775e-5). The continuous oracle starts at the offset origin with positive t; its comparison includes the effect of GPU ε t_min rather than adding that numerical guard to the analytic geometry. The thin/stress oracle reuses F0’s polygon/disk quadrature and independent primary intersections, with the actual vertex-scaled ground ε.\n")
    say("The 1 mm **magnitude alone**, even before adding production’s projected-depth t_min, differs from the small-offset continuous sawtooth reference by RMS 0.354465 / 0.354491 (magnitude/viewed), signed mean +0.174451 / +0.170805. Its image is retained in the sawtooth oracle sheet. This is an analytic magnitude control, not a GPU film capture of the full production 1 mm policy.\n")
    say("Thin accepted counts match. Stress retains F0’s 16 boundary pixels differing by one accepted primary sample; none is discarded. All-covered and partial RMS both pass the unchanged σ_F gate. No fixture tolerance was changed. The initial GPU attempt exposed F0’s stale test entry name (`primary_shade` versus `primary_shade_tiles`); the alias was fixed in composed test source. A Numba scalar-conversion error was fixed before oracle results were used.\n")
    say("## Frozen per-population comparison\n")
    say("RMS differences below use populations from the original F0 10 µm R_F, including point-aliased and micro. E is original centre Exact’s error against that same reference. R_D uses the verified fixed-10 µm replay. `occluded` is umbra; `solid` combines lit and umbra. Complete linear HDR, display at registered/+4 stops, mean, p99, maximum, counts and verdicts are in `scores/population-scores.csv`. Image/background/multi-surface populations are included there; visibility excludes background because R_F has no background receiver.\n")
    for pose in ("0","1"):
        say(f"### Pose {pose}\n")
        replay=data[pose]["minimal_replay"]
        say(f"Fixed-10 µm replay: maximum R_F difference {replay['visibility_max']:.9g}; accepted-count mismatches {replay['count_mismatches']}; maximum R_L linear-channel difference {replay['hdr_max']:.9g}.\n")
        policies=data[pose]["policies"]
        rows=[]
        for pop,row in policies["magnitude"]["populations"]["R_F"].items():
            if pop in ("image","background-centre"):continue
            v=policies["viewed"]["populations"]["R_F"][pop]
            rows.append((pop,row["pixels"],fmt(row.get("rms",0)),fmt(v.get("rms",0)),fmt(row["exact_rms"])))
        table(("Population","Pixels","R1a R_F","R1b R_F","E R_F"),rows)
        rows=[]
        for pop in ("partial","faint","narrow","wide","isolated","silhouette","contact","lit","occluded","point-aliased","micro","backlit","multi-surface","background-centre","image"):
            l=policies["magnitude"]["populations"]["R_L_HDR_codes"][pop];lv=policies["viewed"]["populations"]["R_L_HDR_codes"][pop]
            d=policies["magnitude"]["populations"]["R_D_HDR_codes_plus4"][pop];dv=policies["viewed"]["populations"]["R_D_HDR_codes_plus4"][pop]
            rows.append((pop,fmt(l.get("rms",0)),fmt(lv.get("rms",0)),fmt(l["exact_rms"]),fmt(d.get("rms",0)),fmt(dv.get("rms",0)),fmt(d["exact_rms"])))
        table(("Population","R1a R_L codes","R1b R_L codes","E R_L","R1a R_D +4 codes","R1b R_D +4 codes","E R_D +4"),rows)
    if magnitude_failures:
        say("Magnitude failures (pose/channel/population): "+", ".join("/".join(row) for row in magnitude_failures)+". The policy is unsettled; no F1-AA.\n")
    say("## Viewed-side decision\n")
    rows=[]
    for pose in ("0","1"):
        for channel,row in data[pose]["side_rule_backlit"].items():
            rows.append((pose,channel,row["pixels"],fmt(row["rms"]),fmt(row["mean"]),fmt(row["exact_rms"]),row["exceeds_exact"]))
    table(("Pose","Backlit channel","Pixels","R1b−R1a RMS","Signed mean","Frozen E RMS","Above E"),rows)
    say("The side rule changes the opaque surface’s estimand even if a particular aggregate stays below E. It is not adopted. Full R_F/R_L/R_D images at both exposures and five separated maximum-sun-change backlit crops per pose are retained, alongside all seven frozen contact probes. Crop coordinates are recorded in the summary JSON.\n")
    say("The observed side change is not uniformly darker: centre-backlit R_F increases by mean +0.014423 / +0.014231 at poses 0/1. R_F RMS changes are 0.040256 / 0.039487, below original Exact’s 0.271631 / 0.274656 error on that population. R_L/R_D side differences also remain below E. Film footprints can include multiple facet orientations; the centre-backlit population is a diagnostic, not a per-sample sidedness partition. These results do not establish a visual improvement or settle the material policy.\n")
    for pose in (0,1):say(f"- Pose {pose}: [decision crops]({a.root.as_posix()}/images/pose-{pose}-viewed-side-decision-crops.png), [frozen contacts]({a.root.as_posix()}/images/pose-{pose}-frozen-contact-crops.png).")
    say()
    say("## Saved-sample convergence and stop rules\n")
    rows=[]
    for pose in ("0","1"):
        for policy,p in data[pose]["policies"].items():
            for pop,c in p["convergence"].items():
                fit=c["fits"]["visibility_rms"]
                rows.append((pose,policy,pop,fmt(c["beta"]["visibility_rms"]),fmt(c["beta"]["radiance_rms"]),fmt(c["beta"]["sun_rms"]),fmt(fit["predicted_frame8"]),fmt(fit["predicted_frame32"]),c["fits"]["radiance_rms"]["observed_first_below_exact"],c["recompute_predictions"]))
    table(("Pose","Policy","Population","β visibility","β radiance","β sun","V model N=8","V model N=32","R_L first ≤ E","Revise"),rows)
    say("β is an unweighted log-RMS fit at all N=1–16, not the F0 two-point extrapolation. Curves, missing counts, fitted amplitudes and modelled frames-to-Exact are in `scores/summary.json`. These fits describe one deterministic, well-spread sequence of Exact-shaded samples, with the finite 256-spp reference’s own noise and sample inclusion. They do not include 8-ray AA shadow noise, reprojection, motion or history validation. N=32 is an extrapolation. The recalculated values replace the prior ideal static prediction where the revision rule triggers; they do not select Islam’s perceptual T_q or certify F1-AA.\n")
    say("**Earlier prediction miss:** ideal R_L replay on partials first reaches point Exact at N=4 at both poses, for both policies; all-covered and whole-image radiance first reach it at N=5. N=2 is above Exact. Thus the earlier two-frame radiance crossing is unsupported by this saved sequence even though β_L > β_V. The literal R1 β/radiance-rate predicate does not fire; before AA, use the measured 4–5-sample crossing as the new ideal static baseline and measure the actual 8-ray/history cost. The F1-AA two-frame acceptance floor remains frozen: this observation does not relax it. T_q remains Islam’s decision.\n")
    say("Stop-rule verdicts: all fixtures pass unchanged σ_F and zero-acne requirements; magnitude verdict is stated above; viewed-side remains a separate decision; the replay revision predicate is reported without relaxing β=0.45 or the radiance-rate test. No production policy or default changes, and no F1-AA work.\n")
    say("CPU scoring correction: the first replay incorrectly conditioned radiance error on visibility-sample acceptance. The retained replay scores R_L/R_D over the complete requested population, including background silhouettes for the image row; R_F alone uses accepted covered samples. Run 01 is retained as superseded. A final reduction corrected the image row’s visibility baseline and missing count to the identical all-covered visibility domain; radiance curves were unchanged. No GPU capture or tolerance changed.\n")
    say("Independent pose-2 replay identity (byte equality against pose 0):\n")
    table(("Policy","R_F/R_L raw film","R_D raw film","Exact counts","Exact HDR","Exact sun HDR"),[(p,*[d[s] for s in ("film.bin","direct.bin","exact.u32","exact.rgba16f","exact.sun.rgba16f")]) for p,d in data["identity"].items()])
    say("## Inspection, evidence and reproduction\n")
    say("Verified locally: all full acne fixture panels; all six sawtooth/thin/stress oracle sheets; both quarry poses under both policies, complete R_F/R_L/R_D at registered and +4 stops, signed visibility differences, viewed-side decision crops and frozen contact crops. Acne panels have no dark defects. Analytic/GPU fixture boundaries agree; the 1 mm sawtooth control loses grooves. Quarry magnitude differences are sparse and small; viewed-side differences add fine visibility structure around rough faces, ridges and rubble. Full R_L looks substantially alike, while sun-only +4 crops expose small local changes. No blanket darkening or quality improvement is claimed. The complete silhouettes and cast-shadow regions remain present. Static image inspection does not establish motion quality.\n")
    say(f"Evidence: `{a.root.as_posix()}/`, physically on E: through a directory junction after D: filled during build-02. Only this task’s own target/evidence was relocated; old evidence and other workers’ files were not moved. `provenance.json` and `identities.sha256` bind source/scripts, binary, raw buffers, scores and images. `quarry-{{minimal,magnitude,viewed}}-p{{0,1,2}}/` contains references and the 16 saved frames. Fixture subdirectories contain geometry, raw buffers, oracles, validation JSON and images.\n")
    say("The shared GPU lock was acquired by atomic mkdir plus token-bearing owner.json before every build/GPU run; all runs were hidden/offscreen. Preflight checked games, Unreal, other Renderlab jobs and ≥6144 MiB available physical memory; the monitor also aborted on contention or insufficient available memory. The first quarry attempt was stopped when another compilation plus asset upload lowered availability to 4739 MiB, then retried. No foreign application was closed.\n")
    say(f"All {len(windows)} owned windows were released: total {sum(w['seconds'] for w in windows):.3f} s, longest {max(w['seconds'] for w in windows):.3f} s, below the 45-minute limit. Complete UTC acquire/release times:\n")
    table(("Label","Acquired UTC","Released UTC","Seconds"),[(w["label"],w["acquired"],w["released"],f"{w['seconds']:.3f}") for w in windows])
    say(f"Implementation/analysis/reporting overlapped verification and were not separately stopwatched. Verification: builds and GPU windows are timed separately in `runs/*.json`; fixture CPU elapsed times are in `oracle-run-*.log`; final population scoring/replay took {data['seconds']:.3f} s. All 77 nonignored library CPU tests pass (60 GPU tests remain explicitly ignored), plus composed WGSL validation, independent oracle checks, formatting and Python parsing. Accepted GPU runs report active Vulkan validation with zero errors. No frame-time or engine-quality superiority claim is made.\n")
    say("```powershell\n$env:OPENBLAS_NUM_THREADS='2'; $env:OMP_NUM_THREADS='2'; $env:NUMBA_NUM_THREADS='2'\npy -3 -B scripts/m6-receiver-run.py --label fresh-build --minutes 15 -- cargo test -p renderlab --lib --release --no-run -j 2 --target-dir D:/Coding/renderlab-work/m6-exact-quality/r1/target\npy -3 -B scripts/m6-receiver-fixtures.py --check-oracle\npy -3 -B scripts/m6-receiver-collect.py --phase fixtures --attempt fresh\npy -3 -B scripts/m6-receiver-collect.py --phase quarry --attempt fresh\npy -3 -B scripts/m6-receiver-score.py\npy -3 -B scripts/m6-receiver-report.py\n```\n")
    say("Collection reuses complete existing captures and passed validations. To regenerate retained raw captures, choose a fresh evidence location in the guarded runner and collector constants; logs refuse overwrites. Python requires NumPy, SciPy, Numba and Pillow. Source: `src/{film_reference_tests,receiver_policy_tests}.rs`, `support/receiver_policy.wgsl`, F0’s `support/film_accumulate.wgsl`, and `scripts/m6-receiver-*.py`. The focused `m6-receiver-score-test.py` checks that rejected foreground and background samples remain in radiance while visibility baseline/missing counts use covered receivers. Affected release Clippy passes with warnings denied. All edits are test/support/tooling/documentation only.\n")
    a.out.write_text("\n".join(lines),encoding="utf-8")
    print(a.out)

if __name__=="__main__":main()
