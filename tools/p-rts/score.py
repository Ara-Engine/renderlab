"""CPU scoring of tools/p-rts/ab.py captures: byte identity, then ARA1 reference error.

    score.py AB_OUT REPORT.json

Quality scenes: every registered frame of the candidate against the baseline (identical
bytes, differing pixels, largest 8-bit channel change), and both against ARA1's own
qualified path-traced references through tools/ara1/images.py::score on the ARA1 crops
(linear display RGB RMS). Timing scenes: per-process medians of ARA1's segments.
"""
import json
from pathlib import Path
import statistics
import sys

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[2]
ANALYSIS = Path("D:/Coding/renderlab-work/ara1/analysis")
sys.path.insert(0, str(REPO / "tools/ara1"))
from images import score  # noqa: E402

SEGMENTS = {"still": range(32, 40), "move": range(64, 72), "cut_first": [72],
            "cut_recovery": range(73, 80), "cut_settled": range(80, 88),
            # P-RTS stream scenes (tools/p-rts/scenes.py): repeated cuts at frame 72's pose.
            "cut_stream": range(88, 112)}


def pixels(path):
    with Image.open(path) as image:
        return np.asarray(image.convert("RGBA"), dtype=np.int16)


def quality(case, root, out, label="cand"):
    base, cand = root / "base" / f"{case}-0", root / label / f"{case}-0"
    identity = []
    for frame in sorted(base.glob("frame-*.png")):
        a, b = pixels(frame), pixels(cand / frame.name)
        changed = np.any(a != b, axis=2)
        identity.append(dict(frame=frame.name, identical=bool(not changed.any()), pixels=int(changed.sum()),
                             max_code_change=int(np.abs(a - b).max())))
    template = json.loads((ANALYSIS / f"{case}-hybrid.json").read_text())
    rows = []
    for row in template["frames"]:
        name = f"frame-{row['index']:04}.png"
        row = dict(index=row["index"], ara_label="Renderlab RT baseline", ara=str(base / name), rt=str(cand / name),
                   **({"reference": row["reference"]} if row.get("reference") else {}))
        rows.append(row)
    manifest = out / f"{case}-{label}-manifest.json"
    manifest.write_text(json.dumps(template | dict(frames=rows), indent=2) + "\n")
    scores = out / f"{case}-{label}-scores.json"
    score(manifest, scores)
    crops = []
    by = {}
    for r in json.loads(scores.read_text())["rows"]:
        if "engine" in r:
            by.setdefault(r["index"], {})[r["engine"]] = r
    for index, pair in sorted(by.items()):
        for crop, stat in pair["ara"]["linear_display_rgb"].items():
            b, c = stat["rms"], pair["rt"]["linear_display_rgb"][crop]["rms"]
            crops.append(dict(index=index, phase=pair["ara"]["phase"], crop=crop, base_rms=b, cand_rms=c,
                              verdict="equal" if b == c else "better" if c < b else "worse"))
    return dict(identity=identity, crops=crops,
                verdicts={v: sum(c["verdict"] == v for c in crops) for v in ("equal", "better", "worse")})


def timing(case, root):
    result = {}
    for run in sorted(root.glob(f"*/{case}-*/engine.json")):
        label = run.parents[1].name
        e = json.loads(run.read_text())
        gpu = {g["frame_id"]: g for g in e["gpu"]}
        segments = {}
        for name, indices in SEGMENTS.items():
            frames = [f for f in e["frames"] if f["index"] in indices]
            if not frames:
                continue
            segments[name] = dict(
                cpu_service_ms=statistics.median(f["cpu_service_ms"] for f in frames),
                span_ms=statistics.median(gpu[f["frame_id"]]["span_ms"] for f in frames if f["frame_id"] in gpu),
                primary_ms=statistics.median(gpu[f["frame_id"]]["primary_ms"] for f in frames if f["frame_id"] in gpu))
        result.setdefault(label, []).append(dict(run=run.parent.name, segments=segments))
    summary = {}
    for label, runs in result.items():
        summary[label] = {name: {k: statistics.median(r["segments"][name][k] for r in runs) for k in
                                 ("cpu_service_ms", "span_ms", "primary_ms")} for name in SEGMENTS
                          if name in runs[0]["segments"]}
    return dict(runs=result, median_of_process_medians=summary)


def main():
    root, report = Path(sys.argv[1]), Path(sys.argv[2])
    out = report.parent / (report.stem + "-scores")
    out.mkdir(parents=True, exist_ok=True)
    value = dict(quality={}, timing={})
    cases = sorted({p.name.rsplit("-", 1)[0] for p in root.glob("*/*-*") if p.is_dir()})
    for case in cases:
        if case.endswith("-quality"):
            for label in sorted(p.name for p in root.iterdir() if p.is_dir() and p.name != "base"):
                if (root / label / f"{case}-0").exists():
                    value["quality"][f"{case}:{label}"] = quality(case, root, out, label)
        elif case.endswith(("-timing", "-stream")):
            value["timing"][case] = timing(case, root)
    report.write_text(json.dumps(value, indent=2) + "\n")
    for case, q in value["quality"].items():
        same = sum(i["identical"] for i in q["identity"])
        print(case, f"identical {same}/{len(q['identity'])}", "max pixels", max(i["pixels"] for i in q["identity"]), q["verdicts"])
    for case, t in value["timing"].items():
        for label, s in t["median_of_process_medians"].items():
            print(case, label, {k: round(v["cpu_service_ms"], 3) for k, v in s.items()},
                  "primary", {k: round(v["primary_ms"], 3) for k, v in s.items()})


if __name__ == "__main__":
    main()
