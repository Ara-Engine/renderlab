"""Paced continuous quarry motion: derive the path, preserve binaries, run alternating pairs, audit.

The paced path is derived only from the canonical sixteen-pose camera file, which stays unchanged.
Every canonical pose appears exactly at a segment end. Timed runs carry no capture, proof, dump or
profiling; `capture` runs are separate correctness evidence.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / ".refs/quarry-responsiveness"
OUTPUT = ROOT / "measurements/quarry-responsiveness"
CANONICAL = ROOT / "measurements/quarry-scan/cameras.json"
CANONICAL_SHA256 = "ab342818ea91347da81e6cdcdbc05cead2579ca18b929a032df1dc8351e54a85"
PACED = OUTPUT / "paced-cameras.json"
PERIOD_MS = 1000 / 60
# (phase, from canonical pose, to canonical pose, generated frames, interpolation)
SEGMENTS = [
    ("dolly_wide", 0, 3, 24, "linear"),
    ("fly_close", 3, 4, 90, "smooth"),
    ("dolly_close", 4, 7, 24, "linear"),
    ("fly_grazing", 7, 8, 90, "smooth"),
    ("dolly_grazing", 8, 11, 24, "linear"),
    ("cut_reverse", 11, 12, 1, "cut"),
    ("dolly_reverse", 12, 15, 24, "linear"),
    ("hold_reverse", 15, 15, 60, "hold"),
]
MOTION = {"dolly_wide", "fly_close", "dolly_close", "fly_grazing", "dolly_grazing", "cut_reverse", "dolly_reverse"}


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def sources():
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    paths += [p for folder in ("src", "examples", "crates/asset", "support", "shaders")
              for p in (ROOT / folder).rglob("*") if p.is_file()]
    return {str(p.relative_to(ROOT)).replace("\\", "/"): digest(p) for p in sorted(paths)}


def path():
    if digest(CANONICAL) != CANONICAL_SHA256:
        raise RuntimeError("canonical camera file changed")
    canonical = json.loads(CANONICAL.read_text())["frames"]
    frames = [dict(canonical[0], frame=0, phase="settle_wide", canonical=0)]
    for phase, a, b, count, mode in SEGMENTS:
        for k in range(1, count + 1):
            t = 1.0 if mode in ("cut", "hold") else k / count
            s = t * t * (3 - 2 * t) if mode == "smooth" else t
            pose = dict(canonical[b], frame=len(frames), phase=phase)
            if k == count or mode in ("cut", "hold"):
                pose["canonical"] = b
            else:
                for key in ("eye", "target"):
                    pose[key] = [x + s * (y - x) for x, y in zip(canonical[a][key], canonical[b][key])]
            frames.append(pose)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    PACED.write_text(json.dumps({"frames": frames}, indent=1) + "\n")
    record = {"source": str(CANONICAL.relative_to(ROOT)), "source_sha256": CANONICAL_SHA256,
              "segments": SEGMENTS, "period_ms": PERIOD_MS, "poses": len(frames),
              "paced_sha256": digest(PACED)}
    (EVIDENCE / "paced-path.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record))


def build(label, release):
    binary = EVIDENCE / "bin" / f"{label}.exe"
    if binary.exists():
        raise FileExistsError(binary)
    binary.parent.mkdir(parents=True, exist_ok=True)
    before = sources()
    command = [shutil.which("cargo"), "build", "--locked", "-p", "renderlab", "--example",
               "virtual-stream", "--no-default-features", "-j", "2"] + (["--release"] if release else [])
    with (EVIDENCE / f"build-{label}.log").open("wb") as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=log, check=True)
    if sources() != before:
        raise RuntimeError("sources changed during build")
    shutil.copy2(ROOT / f"target/{'release' if release else 'debug'}/examples/virtual-stream.exe", binary)
    with zipfile.ZipFile(binary.with_suffix(".source.zip"), "w", zipfile.ZIP_DEFLATED) as archive:
        for name in before:
            archive.write(ROOT / name, name)
    record = {"command": command, "binary_sha256": digest(binary), "sources": before, "release": release,
              "source_zip_sha256": digest(binary.with_suffix(".source.zip"))}
    binary.with_suffix(".json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"built {label} {record['binary_sha256']}", flush=True)


def warm():
    manifest = ROOT / "measurements/quarry-scan/quarry-virtual.manifest"
    cache = ROOT / "measurements/quarry-scan/cache/virtual-v1"
    count = 0
    for item in [cache / "packages" / manifest.read_bytes()[40:72].hex(), *sorted((cache / "pages").rglob("*"))]:
        if item.is_file():
            with item.open("rb") as file:
                while block := file.read(4 << 20):
                    count += len(block)
    return count


def run(label, binary, validate, extra, paced=True):
    identity = json.loads((EVIDENCE / "bin" / f"{binary}.json").read_text())
    executable = EVIDENCE / "bin" / f"{binary}.exe"
    if digest(executable) != identity["binary_sha256"]:
        raise RuntimeError("preserved binary changed")
    output = OUTPUT / label
    if output.exists():
        raise FileExistsError(output)
    warm_bytes = warm()
    # Content, budgets and quality come from the original 4x stream record; only the path changes.
    original = json.loads((ROOT / ".refs/quarry-scan/stream-4.process.json").read_text())["command"]
    arguments, i = [], 1
    while i < len(original):
        if original[i] in ("--capture", "--output") or (paced and original[i] in ("--cameras", "--frames")):
            i += 2
        elif original[i] in ("--validate", "--verify-cover"):
            i += 1
        else:
            arguments.append(original[i])
            i += 1
    if paced:
        poses = len(json.loads(PACED.read_text())["frames"])
        arguments += ["--cameras", str(PACED), "--frames", str(poses), "--frame-period-ms", f"{PERIOD_MS:.6f}"]
    arguments += (["--validate"] if validate else []) + extra + ["--output", str(output)]
    label_text = (f"Offscreen quarry, {'paced 60 Hz continuous path' if paced else 'canonical 16-pose path'}, "
                  f"warm caches, equal tasklist/nvidia-smi observer; validation={validate}; extra={extra}")
    content = [ROOT / "measurements/quarry-scan/quarry-virtual.manifest", CANONICAL,
               ROOT / "measurements/quarry-scan/limits.json"] + ([PACED] if paced else [])
    (EVIDENCE / f"{label}.compiled.json").write_text(json.dumps({
        "binary": binary, "identity": identity, "warm_read_bytes": warm_bytes,
        "content": {str(p.relative_to(ROOT)): digest(p) for p in content}, "label": label_text}, indent=2) + "\n")
    subprocess.run([sys.executable, str(ROOT / "scripts/scan-execute.py"), "--gpu", "--timeout", "900",
                    "--measurement-label", label_text, str(EVIDENCE / label), "--", str(executable), *arguments],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    if validate and "gpu_validation_summary errors=0 missing_layer=0 active_layer=1" not in (
            EVIDENCE / f"{label}.stderr.log").read_text():
        raise RuntimeError("missing clean active Khronos validation")
    print(f"completed {label}", flush=True)


def rows(file):
    with file.open(newline="") as handle:
        return list(csv.DictReader(handle))


def percentile(values, q):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, max(0, round(q * (len(ordered) - 1))))]


def audit(label):
    output = OUTPUT / label
    process = json.loads((EVIDENCE / f"{label}.process.json").read_text())
    compiled = json.loads((EVIDENCE / f"{label}.compiled.json").read_text())
    paced = json.loads(PACED.read_text())["frames"]
    frames, wall = rows(output / "frames.csv"), rows(output / "wall.csv")
    startup = {row["event"]: int(row["process_us"]) for row in rows(output / "startup.csv")}
    preparation = rows(output / "preparation.csv")
    errors = []
    if process["exit"] or process["stop_reason"] or process["changed_sources"]:
        errors.append("process failure or mixed sources")
    if process["binary_sha256"] != compiled["identity"]["binary_sha256"]:
        errors.append("compiled binary mismatch")
    for forbidden in ["--capture", "--dump-groups", "--page-log", "--verify-cover"]:
        if forbidden in process["command"]:
            errors.append(f"timed diagnostic {forbidden}")
    if len(frames) != len(paced) or len(wall) != len(paced):
        errors.append("missing rendered frames")
    if preparation[-1]["phase"] != "Settled" or int(preparation[-1]["unresolved"]):
        errors.append("initial pose did not settle")
    if (output / "groups.csv").exists() or list(output.glob("*.rgba8")):
        errors.append("readback or dump in timed run")
    if any(int(r["failures"]) for r in frames):
        errors.append("page failures")
    by_phase = {}
    for pose, w, f in zip(paced, wall, frames):
        by_phase.setdefault(pose["phase"], []).append((int(w["advance_draw_wall_us"]) / 1000, f))
    motion = [ms for phase, items in by_phase.items() if phase in MOTION for ms, _ in items]
    hold = [ms for ms, _ in by_phase.get("hold_reverse", [])]
    final = frames[-1]
    pacing = [int(w["pacing_actual_us"]) for w in wall]
    draws = [int(f["draws"]) for f in frames]
    clusters = [int(f["cut_clusters"]) for f in frames]
    summary = {
        "label": label, "binary": compiled["binary"], "errors": errors, "frames": len(frames),
        "motion_frames": len(motion),
        "motion_ms": {"p50": statistics.median(motion), "p95": percentile(motion, 0.95), "max": max(motion)},
        "hold_ms": {"p50": statistics.median(hold), "p95": percentile(hold, 0.95), "max": max(hold)},
        "phase_p50_ms": {p: statistics.median(ms for ms, _ in items) for p, items in by_phase.items()},
        "cut_frame_ms": by_phase["cut_reverse"][0][0],
        "encode_ms_p50": statistics.median(int(f["encode_us"]) for f in frames[1:]) / 1000,
        "submit_ms_p50": statistics.median(int(f["submit_us"]) for f in frames[1:]) / 1000,
        "draws_p50": statistics.median(draws), "cut_clusters_p50": statistics.median(clusters),
        "triangles_total": sum(int(f["triangles"]) for f in frames),
        "pacing_sleep_s": sum(pacing) / 1e6,
        "first_settled_s": startup.get("first_scene_submitted", 0) / 1e6,
        "preparation_ticks": len(preparation),
        "final": {"unresolved": int(final["unresolved"]), "max_error": float(final["max_error"]),
                  "residency_debt": final["residency_debt"]},
        "peak_working_set_bytes": process["peak_working_set_bytes"],
        "observer_seconds": sum(o.get("observer_seconds") or 0 for o in process["observations"]),
        "host_cpu_busy_max": max([o.get("host_cpu_busy_percent") or 0 for o in process["observations"]] or [0]),
    }
    return summary


def summary(tags):
    """Median and range of per-run statistics for each side of every campaign tag."""
    result = {}
    for tag in tags:
        labels = sorted(p.name[:-len(".process.json")] for p in EVIDENCE.glob(f"{tag}-*.process.json"))
        runs = [audit(label) for label in labels]
        sides = {}
        for item in runs:
            sides.setdefault(item["binary"], []).append(item)
        result[tag] = {"runs": runs, "sides": {}}
        for side, items in sides.items():
            def spread(values):
                return {"median": statistics.median(values), "min": min(values), "max": max(values)}
            result[tag]["sides"][side] = {
                "n": len(items), "errors": sum(len(i["errors"]) for i in items),
                "motion_p95_ms": spread([i["motion_ms"]["p95"] for i in items]),
                "motion_p50_ms": spread([i["motion_ms"]["p50"] for i in items]),
                "motion_max_ms": spread([i["motion_ms"]["max"] for i in items]),
                "hold_p50_ms": spread([i["hold_ms"]["p50"] for i in items]),
                "cut_frame_ms": spread([i["cut_frame_ms"] for i in items]),
                "encode_p50_ms": spread([i["encode_ms_p50"] for i in items]),
                "submit_p50_ms": spread([i["submit_ms_p50"] for i in items]),
                "draws_p50": spread([i["draws_p50"] for i in items]),
                "cut_clusters_p50": spread([i["cut_clusters_p50"] for i in items]),
                "first_settled_s": spread([i["first_settled_s"] for i in items]),
                "peak_working_set_bytes": spread([i["peak_working_set_bytes"] for i in items]),
                "host_cpu_busy_max": max(i["host_cpu_busy_max"] for i in items),
                "final_unresolved": spread([i["final"]["unresolved"] for i in items]),
            }
    (EVIDENCE / "results.json").write_text(json.dumps(result, indent=1) + "\n")
    return {tag: value["sides"] for tag, value in result.items()}


def campaign(tag, before, after, validate, pairs):
    for repeat in range(1, pairs + 1):
        for side in (before, after):
            run(f"{tag}-{repeat}-{side}", side, validate, [])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("path")
    p = sub.add_parser("build")
    p.add_argument("label")
    p.add_argument("--release", action="store_true")
    p = sub.add_parser("run")
    p.add_argument("label")
    p.add_argument("binary")
    p.add_argument("--validate", action="store_true")
    p.add_argument("--canonical", action="store_true", help="original sixteen-pose path, unpaced")
    p.add_argument("extra", nargs=argparse.REMAINDER)
    p = sub.add_parser("campaign")
    p.add_argument("tag")
    p.add_argument("before")
    p.add_argument("after")
    p.add_argument("--validate", action="store_true")
    p.add_argument("--pairs", type=int, default=5)
    p = sub.add_parser("report")
    p.add_argument("labels", nargs="+")
    p = sub.add_parser("summary")
    p.add_argument("tags", nargs="+")
    args = parser.parse_args()
    if args.command == "path":
        path()
    elif args.command == "build":
        build(args.label, args.release)
    elif args.command == "run":
        # REMAINDER also captures these flags when they follow the binary name.
        extra = [a for a in args.extra if a != "--"]
        validate = args.validate or "--validate" in extra
        canonical = args.canonical or "--canonical" in extra
        extra = [a for a in extra if a not in ("--validate", "--canonical")]
        run(args.label, args.binary, validate, extra, paced=not canonical)
    elif args.command == "campaign":
        campaign(args.tag, args.before, args.after, args.validate, args.pairs)
    elif args.command == "summary":
        print(json.dumps(summary(args.tags), indent=1))
    else:
        print(json.dumps([audit(label) for label in args.labels], indent=1))


if __name__ == "__main__":
    main()
