"""P-RELIT on the quarry yardstick: tools/p-flicker's run.py and timing.py over this unit's work
folder and binaries (bin/NAME/p_yard_capture.exe), and the game scene's per-pass medians.

    py -3 -B tools/p-relit/yard.py run STEP...          # inside a lease; STEP as tools/p-flicker/run.py,
                                                        # with bin=NAME/p_yard_capture
    py -3 -B tools/p-relit/yard.py timing OUT.json RUN...   # quarry span p50 per state, with Epic ratios
    py -3 -B tools/p-relit/yard.py hashes RUN_A RUN_B   # quarry captures: frames with identical files
    py -3 -B tools/p-relit/yard.py passes OUT.json RUN_DIR...   # game_scene passes.jsonl: per-pass p50
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-relit")


def flicker_tool(name):
    sys.path.insert(0, str(HERE.parent / "p-flicker"))
    spec = importlib.util.spec_from_file_location("pflicker_" + name, HERE.parent / "p-flicker" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.WORK = WORK
    return module


def timing(out, runs):
    t = flicker_tool("timing")
    result = {run: t.summary(run) for run in runs}
    Path(out).write_text(json.dumps(dict(runs=result, epic={k: {s: v["p50"] for s, v in e.items()} for k, e in t.EPIC.items()}), indent=1))
    for run, states in result.items():
        epic = t.EPIC["ue-hw-e" if run.endswith("-rt") else "ue-sw-e"]
        print(run, " | ".join(f"{s} {states[s]['gpu']:.2f} ms ({epic[s]['p50'] / states[s]['gpu']:.2f}x Epic)"
                              for s in ("still", "moving", "cut", "recover") if s in states and "gpu" in states[s]))


def hashes(a, b):
    """Frames of two quarry captures whose stored images are the same bytes."""
    def digests(run):
        return {p.name: hashlib.file_digest(p.open("rb"), "sha256").hexdigest()
                for p in sorted((WORK / "runs" / run).iterdir()) if p.suffix in (".bin", ".hdr", ".raw", ".npy", ".png", ".exr")}
    da, db = digests(a), digests(b)
    equal = sum(da[k] == db.get(k) for k in da)
    print(f"{a} vs {b}: {equal}/{len(da)} image files identical")
    return equal == len(da) and len(da) > 0


def passes(out, runs):
    """Per-pass GPU p50 over frames 30 on (the flight has closed in on the hero by then)."""
    result = {}
    for run in runs:
        lines = [json.loads(l) for l in (Path(run) / "passes.jsonl").read_text().splitlines() if l.strip()]
        lines = [l for l in lines if l["frame"] >= 30]
        names = sorted({n for l in lines for n in l["passes"]})
        row = dict(frames=len(lines), span=float(np.median([l["span"] for l in lines])),
                   passes={n: float(np.median([l["passes"].get(n, 0.0) for l in lines])) for n in names})
        result[Path(run).name] = row
        top = sorted(row["passes"].items(), key=lambda kv: -kv[1])[:8]
        print(f"{Path(run).name}: span {row['span']:.2f} ms ({row['frames']} frames) | " + " ".join(f"{n} {v:.2f}" for n, v in top), flush=True)
    Path(out).write_text(json.dumps(result, indent=1))


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "run":
        run = flicker_tool("run")
        codes = [run.run(s) for s in sys.argv[2:]]
        raise SystemExit(max(codes) if codes else 0)
    if mode == "timing":
        timing(sys.argv[2], sys.argv[3:])
    elif mode == "hashes":
        raise SystemExit(0 if hashes(*sys.argv[2:4]) else 1)
    elif mode == "passes":
        passes(sys.argv[2], sys.argv[3:])
