"""Export TR0's retained scores as explicit, per-measure TR1 comparison rows.

This is CPU-only. Historical frozen-target results, new physical-target scores
and quiet cost ceilings remain separate. A missing measurement never passes.
"""
import csv
import hashlib
import json
import math
from pathlib import Path

ROOT = Path("E:/renderlab-work/m7-tr0")
OUT = ROOT / "bar"
OUT.mkdir(exist_ok=True)
SOURCES = {}
ROWS = []
D8 = "d8bd9a8e1e1c7b5d39ee04d70c01ed4ee87bb654463db2b40daec1a96f35f139"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    SOURCES[str(path)] = sha(path)
    return json.loads(path.read_text())


def metric(case, frame, channel, population, measure, value, n=None, method="tsr", phase=None):
    if value is None:
        return
    if not math.isfinite(value):
        raise ValueError("non-finite comparison row")
    ROWS.append(dict(label=case["label"], track=case["track"], path=case["path"], quality=case["quality"],
        method=method, frame=frame, channel=channel, population=population, measure=measure,
        value=value, pixels=n, phase=phase, better="lower"))


def colour_case(label):
    folder = ROOT / "runs" / label
    path = folder / "physical-scores.json"
    if not path.exists():
        return dict(label=label, available=False)
    score = read(path)
    recipe = read(folder / "recipe.json")
    case = dict(label=label, available=True, track=recipe["track"], path=recipe["path"], quality=recipe["quality"],
        frames=len(score["rows"]), total_frames=len(recipe["frames"]), missing_frames=score["missing_frames"],
        age_protocol=score["age_protocol"], original_checkpoints=score["original_checkpoints"],
        score=str(path), score_sha256=SOURCES[str(path)],
        unavailable=score["unavailable"])
    case["full_registered_track_scored"] = (set(r["frame"] for r in score["rows"]) == set(range(case["total_frames"]))
                                             and not case["missing_frames"])
    case["age_measures_available"] = score["age_protocol"] != "unavailable"
    if recipe.get("scene") == "S2G":
        for reference in {r["reference"] for r in score["rows"]}:
            meta = read(Path(reference).with_suffix(".json"))
            binary = meta.get("binary_sha256", meta.get("reference_binary_sha256"))
            if not meta.get("accepted_for_scoring") or binary != D8:
                raise ValueError("new S2G bar requires qualified D8 references")
            if any(r["reference_sha256"] != meta["archive_sha256"] for r in score["rows"] if r["reference"] == reference):
                raise ValueError("score and retained reference provenance differ")
    for row in score["rows"]:
        frame = row["frame"]
        phase = recipe["frames"][frame].get("phase")
        for channel, populations in row["measures"].items():
            for population, values in populations.items():
                # Noise and SE remain in the source JSON as uncertainty, not
                # quality measures that a candidate should try to minimize.
                for method, source in (("tsr", values), ("t1b-black", values.get("t1b_replay_same_physical_reference", {}))):
                    for name in ("rms", "rms_noise_subtracted", "maximum", "signed_mean"):
                        if name in source:
                            value = abs(source[name]) if name == "signed_mean" else source[name]
                            metric(case, frame, channel, population,
                                   "absolute_signed_mean" if name == "signed_mean" else name,
                                   value, values["n"], method, phase)
                shared = values.get("shared_age32", {})
                if shared:
                    metric(case, frame, channel, population + "/age32", "rms_noise_subtracted",
                           shared["rms_noise_subtracted"], shared["n"], phase=phase)
                    metric(case, frame, channel, population + "/age32", "absolute_signed_mean",
                           abs(shared["signed_mean"]), shared["n"], phase=phase)
                old_shared = values.get("same_t1b_age32_pixels", {})
                if old_shared:
                    metric(case, frame, channel, population + "/age32", "rms_noise_subtracted",
                           old_shared["t1b"], old_shared["n"], "t1b-black", phase)
                    metric(case, frame, channel, population + "/age32", "absolute_signed_mean",
                           abs(old_shared["t1b_signed_mean"]), old_shared["n"], "t1b-black", phase)
        for population, value in row.get("highlights", {}).items():
            for name in ("centroid_drift_px", "peak_drift_px"):
                metric(case, frame, "linear", population, name, value[name], value["pixels"], phase=phase)
            metric(case, frame, "linear", population, "absolute_highlight_energy_ratio_error",
                   abs(value["energy_ratio"] - 1), value["pixels"], phase=phase)
        for channel, bands in row["background_distance_bands"].items():
            for band, value in bands.items():
                for method, prefix in (("tsr", "tsr_"), ("t1b-black", "t1b_")):
                    if prefix + "rms" in value:
                        metric(case, frame, channel, "background-distance/" + band, "rms",
                               value[prefix + "rms"], value["n"], method, phase)
        for channel, extent in row.get("background_error_extent", {}).items():
            for method, thresholds in extent["values"].items():
                for threshold, values in thresholds.items():
                    for name, value in values.items():
                        metric(case, frame, channel, "background-error-over-" + threshold,
                               name, value, method="t1b-black" if method == "t1b" else method, phase=phase)
        for channel, values in row.get("changed", {}).get("measures", {}).items():
            for method, prefix in (("tsr", "tsr_"), ("t1b-black", "t1b_")):
                for name in ("rms", "rms_noise_subtracted", "signed_mean"):
                    if prefix + name in values:
                        value = abs(values[prefix + name]) if name == "signed_mean" else values[prefix + name]
                        metric(case, frame, channel, "changed/8-frame-input",
                               "absolute_signed_mean" if name == "signed_mean" else name,
                               value, row["changed"]["pixels"], method, phase)
    for row in score["motion_residuals"]:
        for method, prefix in (("tsr", "tsr_"), ("t1b-black", "t1b_")):
            for name in ("rms", "step_rms"):
                if prefix + name in row:
                    metric(case, row["frames"][1], row["channel"], "covered/same-instance/age8",
                           "reference_subtracted_flicker_rms" if name == "rms" else "frame_step_rms",
                           row[prefix + name], row["pixels"], method, row["phase"])
        for population, values in row.get("material_populations", {}).items():
            for name in ("residual_rms", "step_rms"):
                metric(case, row["frames"][1], row["channel"], population + "/same-instance/age8",
                       "reference_subtracted_flicker_rms" if name == "residual_rms" else "frame_step_rms",
                       values[name], values["n"], phase=row["phase"])
    for name, values in score["transient_pools"].items():
        event, age, population, channel = name.split("/")
        for method, prefix in (("tsr", "tsr_"), ("t1b-black", "t1b_")):
            for measure in ("rms", "rms_noise_subtracted"):
                if prefix + measure in values:
                    metric(case, "pooled-age-" + age, channel, population + "/" + event,
                           measure, values[prefix + measure], values["n"], method)
    return case


cases = [colour_case(label) for label in ("w-e-n-c2", "w-e-r-c1", "w-h-v-c1")]
for entry in read(ROOT / "s2g/synchronized-candidates.json"):
    cases.append(colour_case(entry["label"]))

quiet = read(ROOT / "quiet/tq1/certificate.json")
quiet_checks = read(ROOT / "quiet/tq1/readback-validation.json")
if not (quiet["complete"] and quiet["eligible_for_speed_claim"] and quiet_checks["passed"]):
    raise ValueError("quiet cost certificate and runtime readback must pass")
cost = read(ROOT / "cost-summary.json")
ceilings = []
for case in cost["rows"]:
    if case["scope"] == "quiet" and case["quality"] == 3:
        assert case["pairs"] == 8
        ceilings.append(dict(track=case["track"], path=case["path"], pairs=case["pairs"],
            increment_ceiling_ms={p: case["stats"]["frame_increment_ms"][p] * .5 for p in ("p50", "p95", "p99")},
            whole_frame_ceiling_ms={p: case["stats"]["tsr_gpu_ms"][p] * .5 for p in ("p50", "p95", "p99")}))

destination = OUT / "physical-measures.csv"
with destination.open("w", newline="", encoding="utf-8") as stream:
    writer = csv.DictWriter(stream, fieldnames=list(ROWS[0]))
    writer.writeheader()
    writer.writerows(ROWS)
historical = ROOT / "t1b-recorded.json"
SOURCES[str(historical)] = sha(historical)
result = dict(
    contract="TR1 default only after beating TSR Epic on every measured final-colour measure on both paths, at <=half its quiet cost; no tradeoff or pooled-average override",
    comparison="Candidate must use the same scene, effective inputs, exposure, black-primary protocol, reference identity and fixed masks. Each CSV row is a separate comparison; lower is better. Energy-ratio error is abs(ratio-1), never a fitted gain.",
    units="F0 display codes for colour errors; pixels for spatial drift/extent; ratio for highlight energy error; ms for quiet ceilings",
    physical_target="U0b disk-integrated multi-bounce, zero shutter. Combined final-colour differences include lighting/material/shadow differences; not solely temporal reconstruction.",
    cases=cases, cost_ceilings=ceilings, csv=dict(path=str(destination), sha256=sha(destination), rows=len(ROWS)),
    historical_frozen_target=dict(path=str(historical), sha256=SOURCES[str(historical)], comparable_to_physical_target=False),
    unknowns_block_pass=True, tr1_default_accepted=False,
    limits=["Q-walk: all 60 historical checkpoints, 93/161 outputs; 68 intervening references absent.",
            "Native post-TSR colour does not expose original R_F visibility; full-colour does not expose original R_D.",
            "Original Exact/frozen-reference raw data is deleted. New physical combined errors do not replace its frozen ratios or film-settle gate.",
            "Start geometry ages are not equal empty-history sample counts; native first output has one preparation sample.",
            "Black-primary protocol leaves visible-sky silhouettes untested.",
            "Per-pose GPUTime reporting lag is uncalibrated; qualified costs pool the registered full track."],
    sources=SOURCES, script_sha256=sha(Path(__file__)))
(OUT / "tr1-bar.json").write_text(json.dumps(result, indent=2, allow_nan=False) + "\n", encoding="utf-8")
print("TR1 comparison rows", len(ROWS), "S2G scored tracks", sum(c.get("full_registered_track_scored", False) for c in cases[3:]), "/8")
