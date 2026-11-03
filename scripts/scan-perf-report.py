"""Audit and summarize the fixed five-pair quarry preparation campaign."""
import argparse
import collections
import csv
import json
from pathlib import Path
import re
import statistics

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".refs/quarry-preparation"
OUTPUT = ROOT / "measurements/quarry-preparation"


def rows(path):
    with path.open(newline="") as file:
        return list(csv.DictReader(file))


def spread(values):
    return {"n": len(values), "median": statistics.median(values), "min": min(values), "max": max(values)}


def profile(label):
    counts = collections.Counter()
    stages = {"preparation": collections.Counter(), "moving": collections.Counter()}
    stage_calls = collections.Counter()
    stage = "preparation"
    calls = 0
    lines = (EVIDENCE / f"{label}.stderr.log").read_text().splitlines()
    for line in lines:
        if line.startswith("first_view_preparation="):
            stage = "moving"
        if line.startswith("selection_profile "):
            calls += 1
            values = {k: int(v) for k, v in re.findall(r"(\w+)=([0-9]+)", line)}
            counts.update(values)
            stages[stage].update(values)
            stage_calls[stage] += 1
    report = {"calls": calls, "seconds": {k: v / 1e9 for k, v in counts.items() if k.endswith("_ns")},
              "counts": {k: v for k, v in counts.items() if not k.endswith("_ns")},
              "bootstrap": [line for line in lines if line.startswith("bootstrap")],
              "stages": {name: {"calls": stage_calls[name],
                  "seconds": {k: v / 1e9 for k, v in values.items() if k.endswith("_ns")},
                  "counts": {k: v for k, v in values.items() if not k.endswith("_ns")}}
                  for name, values in stages.items()}}
    (EVIDENCE / f"{label}.totals.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def run(label):
    output = OUTPUT / label
    process = json.loads((EVIDENCE / f"{label}.process.json").read_text())
    compiled = json.loads((EVIDENCE / f"{label}.compiled.json").read_text())
    startup = {row["event"]: row for row in rows(output / "startup.csv")}
    frames = rows(output / "frames.csv")
    preparation = rows(output / "preparation.csv")
    ticks = rows(output / "preparation-ticks.csv")
    wall = rows(output / "wall.csv")
    gpu = rows(output / "gpu.csv")
    errors = []
    if process["exit"] or process["stop_reason"] or process["changed_sources"]:
        errors.append("process failure or mixed sources")
    if process["binary_sha256"] != compiled["identity"]["binary_sha256"]:
        errors.append("compiled binary mismatch")
    for forbidden in ["--capture", "--dump-groups", "--page-log", "--verify-cover"]:
        if forbidden in process["command"]:
            errors.append(f"timed diagnostic {forbidden}")
    if compiled["identity"]["profile"]:
        errors.append("profiled binary in timed campaign")
    if len(frames) != 16 or len(wall) != 16 or not gpu:
        errors.append("missing rendered work or GPU timestamps")
    if (output / "groups.csv").exists() or list(output.glob("*.rgba8")):
        errors.append("readback or dump in timed campaign")
    if int(preparation[-1]["unresolved"]) != 0 or preparation[-1]["phase"] != "Settled":
        errors.append("initial pose did not settle")
    limits = json.loads((output / "limits.json").read_text())["runtime"]
    for row in preparation + frames:
        if (int(row["failures"]) or int(row["peak"]) > limits["geometry_bytes"]
                or int(row["metadata"]) + int(row["scratch"]) > limits["metadata_bytes"]
                or int(row["candidates"]) > limits["max_candidates"]
                or int(row["cut_clusters"]) > limits["max_clusters"]):
            errors.append("IO failure or effective resource/work budget exceeded")
            break
    stderr = (EVIDENCE / f"{label}.stderr.log").read_text()
    run_text = (output / "run.txt").read_text()
    if "oracle=false shadow=false profiling=false" not in run_text:
        errors.append("unexpected oracle/shadow/profile build")
    if "gpu_validation_summary errors=0 missing_layer=0" not in stderr:
        errors.append("renderer diagnostics reported an error")
    if "-dev-" in label and "gpu_validation_summary errors=0 missing_layer=0 active_layer=1" not in stderr:
        errors.append("missing clean active validation")
    if "-release-" in label and "--validate" in process["command"]:
        errors.append("validation enabled in optimized timing")
    observations = process["observations"]
    host_cpu = [o["host_cpu_busy_percent"] for o in observations if o.get("host_cpu_busy_percent") is not None]
    idle_gpu = [int(o["gpu"].split(",")[0]) for o in observations]
    before = preparation[-1]
    if any(int(before[k]) != v for k, v in {"reads": 1776, "uploads": 1776,
            "cut_clusters": 19214, "geometry": 116391936}.items()):
        errors.append("initial settled quarry work changed")
    result = {
        "label": label, "errors": errors,
        "first_submit_s": int(startup["first_scene_submitted"]["process_us"]) / 1e6,
        "first_cover_s": int(startup["first_cover"]["process_us"]) / 1e6,
        "bootstrap_s": (int(startup["asset_ready"]["process_us"]) - int(startup["device_ready"]["process_us"])) / 1e6,
        "preparation_ticks": len(ticks), "poll_s": sum(int(r["poll_us"]) for r in ticks) / 1e6,
        "advance_s": sum(int(r["advance_us"]) for r in ticks) / 1e6,
        "moving_median_ms": statistics.median(int(r["advance_draw_wall_us"]) / 1000 for r in wall[1:]),
        "moving_max_ms": max(int(r["advance_draw_wall_us"]) / 1000 for r in wall[1:]),
        "gpu_median_ms": statistics.median(float(r["mesh_pass_us"]) / 1000 for r in gpu),
        "process_wall_s": process["wall_seconds"], "process_cpu_s": process["process_cpu_seconds"],
        "peak_working_set_bytes": process["peak_working_set_bytes"],
        "host_cpu_busy_percent": spread(host_cpu) if host_cpu else None,
        "gpu_utilization_percent": spread(idle_gpu),
        "observer_seconds": sum(o.get("observer_seconds", 0) for o in observations),
        "startup_counts": {k: before[k] for k in ["reads", "uploads", "cut_clusters", "geometry", "peak", "unresolved", "failures"]},
        "startup_timers_s": {k: int(v) / 1e6 for k, v in before.items() if k.endswith("_total_us")},
        "moving_final": {k: frames[-1][k] for k in ["reads", "uploads", "unresolved", "max_error", "candidate_debt", "output_debt", "residency_debt", "failures"]},
        "limits": limits, "content": compiled["content"], "warm_read_bytes": compiled["warm_read_bytes"]}
    return result


def quality():
    from PIL import Image, ImageChops, ImageDraw
    captures = ROOT / "captures/quarry-preparation"
    captures.mkdir(exist_ok=True)
    records = []
    for label in [f"held-{pose}-{side}" for pose in (0, 4, 8, 12) for side in ("baseline", "fixed")] + [
            "moving-4-baseline", "moving-4-fixed", "moving-1-fixed"]:
        path = OUTPUT / label
        process = json.loads((EVIDENCE / f"{label}.process.json").read_text())
        stderr = (EVIDENCE / f"{label}.stderr.log").read_text()
        assert process["exit"] == 0 and not process["stop_reason"] and not process["changed_sources"], label
        assert "gpu_validation_summary errors=0 missing_layer=0 active_layer=1" in stderr, label
        assert not (path / "groups.csv").exists(), label
        images = []
        for ppm in sorted(path.glob("*.ppm")):
            destination = captures / f"{label}-{ppm.stem}.png"
            with Image.open(ppm) as source:
                assert source.size == (1920, 1080)
                source.save(destination)
            images.append(str(destination.relative_to(ROOT)))
        preparation = rows(path / "preparation.csv")[-1]
        records.append({"label": label, "images": images, "initial_phase": preparation["phase"],
                        "initial_unresolved": preparation["unresolved"], "initial_max_error": preparation["max_error"],
                        "final": rows(path / "frames.csv")[-1],
                        "summary": (path / "summary.txt").read_text().splitlines()[0]})

    def difference(a, b):
        with Image.open(a) as left, Image.open(b) as right:
            channels = ImageChops.difference(left.convert("RGB"), right.convert("RGB")).split()
            maximum = ImageChops.lighter(ImageChops.lighter(channels[0], channels[1]), channels[2])
            histogram = maximum.histogram()
            changed = sum(histogram[1:])
            error = sum(sum(i * n for i, n in enumerate(channel.histogram())) for channel in channels)
            pixels = left.width * left.height
            return {"exact": changed == 0, "changed_pixels": changed, "changed_percent": 100 * changed / pixels,
                    "mean_absolute_rgb_8bit": error / (pixels * 3)}

    comparisons = {}
    for pose in (0, 4, 8, 12):
        a = captures / f"held-{pose}-baseline-frame-{pose:04}.png"
        b = captures / f"held-{pose}-fixed-frame-{pose:04}.png"
        full = ROOT / f"measurements/quarry-scan/full-4-grounded/frame-{pose:04}.ppm"
        comparisons[f"held-{pose}"] = {"baseline_vs_fixed": difference(a, b),
            "full_vs_baseline": difference(full, a), "full_vs_fixed": difference(full, b)}
    for pose in range(16):
        comparisons[f"moving-{pose}"] = difference(
            captures / f"moving-4-baseline-frame-{pose:04}.png",
            captures / f"moving-4-fixed-frame-{pose:04}.png")
    for side in ("baseline", "fixed"):
        sheet = Image.new("RGB", (1920, 4 * 294), (28, 31, 36))
        draw = ImageDraw.Draw(sheet)
        for pose in range(16):
            with Image.open(captures / f"moving-4-{side}-frame-{pose:04}.png") as source:
                source.thumbnail((480, 270))
                x, y = pose % 4 * 480, pose // 4 * 294
                sheet.paste(source, (x, y))
                draw.text((x + 8, y + 273), f"{side} pose {pose}", fill="white")
        sheet.save(captures / f"moving-4-{side}-sheet.png")
    report = {"runs": records, "comparisons": comparisons,
              "note": "Held before/after and original full-detail references; moving readback cadence matched. Numerical differences are not a visual acceptance threshold."}
    (EVIDENCE / "quality.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"held": {k: v for k, v in comparisons.items() if k.startswith("held")},
                      "moving_changed": {k: v for k, v in comparisons.items() if k.startswith("moving") and not v["exact"]}}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", default="paired")
    parser.add_argument("--partial", action="store_true")
    parser.add_argument("--quality", action="store_true")
    args = parser.parse_args()
    if args.quality:
        quality()
        return
    runs = [run(p.name.removesuffix(".process.json")) for p in sorted(EVIDENCE.glob(f"{args.tag}-*.process.json"))]
    groups = {}
    for profile_name in ["dev", "release"]:
        groups[profile_name] = {}
        for side in ["baseline", "fixed"]:
            selected = [r for r in runs if f"-{profile_name}-" in r["label"] and r["label"].endswith(side)]
            if not args.partial:
                assert len(selected) == 5, (profile_name, side, len(selected))
            if selected:
                groups[profile_name][side] = {k: spread([r[k] for r in selected]) for k in [
                    "first_submit_s", "first_cover_s", "bootstrap_s", "moving_median_ms", "moving_max_ms",
                    "gpu_median_ms", "process_wall_s", "process_cpu_s", "peak_working_set_bytes"]}
        if all(side in groups[profile_name] for side in ["baseline", "fixed"]):
            a, b = [groups[profile_name][side]["first_submit_s"]["median"] for side in ["baseline", "fixed"]]
            groups[profile_name]["first_submit_reduction_percent"] = 100 * (a - b) / a
    if runs:
        assert all(r["limits"] == runs[0]["limits"] and r["content"] == runs[0]["content"] for r in runs)
    errors = [{"label": r["label"], "errors": r["errors"]} for r in runs if r["errors"]]
    profiles = {label: profile(label) for label in ["profile-baseline-dev", "profile-baseline-release",
        "profile-fixed-dev", "profile-fixed-release"]}
    report = {"label": "Offscreen, warm filesystem cache. Dev active validation; optimized no validation. Equal external tasklist/nvidia-smi observer.",
              "complete": not args.partial, "groups": groups, "errors": errors, "runs": runs, "profiles": profiles}
    destination = EVIDENCE / ("results-partial.json" if args.partial else "results.json")
    destination.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"groups": groups, "errors": errors}, indent=2))
    assert not errors, errors


if __name__ == "__main__":
    main()
