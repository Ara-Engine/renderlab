"""P-FLICKER timing summary (CPU): per-state GPU span p50 (tools/p-yard/score.py's method) and
CPU submit/service p50 for runs, with per-pass medians from passes.jsonl when present, against
P-YARD2's Unreal Epic visits (D:/Coding/renderlab-work/p-yard2/timing.json; Unreal runs D3D12,
so the Vulkan layer never touched it).

    py -3 -B tools/p-flicker/timing.py OUT.json RUN...   RUN: runs/ directory (NAME-PATH)
"""
import os
import json
from pathlib import Path
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
UNIT = Path("D:/Coding/renderlab-work/p-flicker")  # Unreal captures and the clip manifest
WORK = Path(os.environ.get("P_FLICKER_WORK", UNIT))  # runs, sheets, builds (another unit sets its own)
FRAMES = json.loads((HERE.parent / "p-yard/manifest.json").read_text())["scenes"]["quarry"]["frames"]
STATE = {f["index"]: ("moving" if f["state"] in ("walk", "return") else f["state"]) for f in FRAMES}
EPIC = {k: v for k, v in json.loads(Path("D:/Coding/renderlab-work/p-yard2/timing.json").read_text())["summary"].items()
        if k in ("ue-hw-e", "ue-sw-e")}


def summary(run):
    e = json.loads((WORK / "runs" / run / "engine.json").read_text())
    span = {g["frame_id"]: g["span_ms"] for g in e["gpu"]}
    by = {}
    for r in e["frames"]:
        s = STATE[r["index"]]
        row = by.setdefault(s, dict(gpu=[], submit=[], service=[]))
        if r["frame_id"] in span:
            row["gpu"].append(span[r["frame_id"]])
        row["submit"].append(r["cpu_submit_ms"])
        row["service"].append(r["cpu_service_ms"])
    out = {s: {k: float(np.median(v)) for k, v in row.items() if v} for s, row in by.items()}
    passes = WORK / "runs" / run / "passes.jsonl"
    if passes.exists():
        index = {r["frame_id"]: r["index"] for r in e["frames"]}
        per = {}
        for line in passes.read_text().splitlines():
            p = json.loads(line)
            s = STATE.get(index.get(p["frame"], -1))
            if s:
                for name, ms in p["passes"].items():
                    per.setdefault(s, {}).setdefault(name, []).append(ms)
        for s, names in per.items():
            out.setdefault(s, {})["passes"] = {n: float(np.median(v)) for n, v in names.items()}
    return out


if __name__ == "__main__":
    result = {run: summary(run) for run in sys.argv[2:]}
    Path(sys.argv[1]).write_text(json.dumps(dict(runs=result, epic={k: {s: v["p50"] for s, v in t.items()} for k, t in EPIC.items()}), indent=1))
    for run, states in result.items():
        epic = EPIC["ue-hw-e" if run.endswith("-rt") else "ue-sw-e"]
        print(run, " | ".join(f"{s} {states[s]['gpu']:.2f} ms ({epic[s]['p50'] / states[s]['gpu']:.2f}x Epic) submit {states[s]['submit']:.2f}"
                              for s in ("still", "moving", "cut", "recover") if s in states and "gpu" in states[s]))
