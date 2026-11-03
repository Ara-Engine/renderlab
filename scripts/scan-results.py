"""Collect real scan run evidence and turn renderer readbacks into inspectable PNGs."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import re
import statistics

from PIL import Image, ImageDraw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("captures", type=Path)
    parser.add_argument("--runs", nargs="*", default=["full-1-grounded", "full-4-grounded", "stream-1", "stream-4"])
    args = parser.parse_args()
    args.captures.mkdir(parents=True, exist_ok=True)
    composition = json.loads((args.root / "quarry.json").read_text())
    manifest = json.loads((args.root / "scan-manifest.json").read_text())
    runs = []
    previews = []
    for label in args.runs:
        path = args.root / label
        if not path.exists():
            continue
        stderr = args.evidence / (label + ".stderr.log")
        process = args.evidence / (label + ".process.json")
        report = {"label": label}
        if process.exists():
            report["process"] = json.loads(process.read_text())
            report["process"].pop("sources", None)
        if stderr.exists():
            text = stderr.read_text()
            report["validation"] = re.findall(r"gpu_validation_summary[^\n]+", text)
            report["asset_upload"] = re.findall(r"asset_uploaded[^\n]+", text)
        for name in ("startup", "frames", "gpu", "wall"):
            file = path / (name + ".csv")
            if file.exists():
                with file.open(newline="") as source:
                    rows = list(csv.DictReader(source))
                report[name] = rows
                if name == "gpu" and rows:
                    values = [float(r["mesh_pass_us"]) for r in rows]
                    report["gpu_observation_us"] = {"n": len(values), "median": statistics.median(values),
                                                    "minimum": min(values), "maximum": max(values),
                                                    "label": "single dev validation run, uncontrolled, not final performance"}
        for ppm in sorted(path.glob("frame-*.ppm")):
            destination = args.captures / f"{label}-{ppm.stem}.png"
            with Image.open(ppm) as source:
                assert source.size == (1920, 1080)
                source.save(destination)
                thumb = source.copy()
                thumb.thumbnail((640, 360))
                previews.append((destination.name, thumb))
            report.setdefault("images", []).append({"path": str(destination),
                "sha256": hashlib.sha256(destination.read_bytes()).hexdigest()})
        runs.append(report)
    summary = {"scene_sha256": composition["glb_sha256"], "triangles": composition["triangles"],
               "scans": len(manifest["scans"]), "bounds": composition["bounds_gltf"],
               "source_vertices": sum(o["vertices"] for s in manifest["scans"] for o in s["inspection"]["objects"]),
               "prepared_vertices": sum(e["vertices"] for e in composition["entries"]),
               "geometry_verification": {s["id"]: s["inspection"]["geometry_verification"] for s in manifest["scans"]},
               "runs": runs}
    (args.root / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
    if previews:
        sheet = Image.new("RGB", (2 * 640, ((len(previews) + 1) // 2) * 388), (28, 31, 36))
        draw = ImageDraw.Draw(sheet)
        for i, (label, preview) in enumerate(previews):
            x, y = i % 2 * 640, i // 2 * 388
            sheet.paste(preview, (x, y))
            draw.text((x + 8, y + 365), label, fill="white")
        sheet.save(args.captures / "contact-sheet.png")
    print(json.dumps({"scans": summary["scans"], "triangles": summary["triangles"],
                      "runs": [{"label": r["label"], "validation": r.get("validation"),
                                "exit": r.get("process", {}).get("exit")} for r in runs]}, indent=2))


if __name__ == "__main__":
    main()
