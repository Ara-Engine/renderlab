"""P-STILL count summary (CPU): per run and state, median GPU passes and cache work.

    py -3 -B tools/p-still/counts.py RUN...    RUN: a runs/ directory name (NAME-PATH)
States: still frames split at 'still<272' (inside GI's drain bound) and 'still>=272'.
"""
import json
from pathlib import Path
import statistics as st
import sys

WORK = Path("D:/Coding/renderlab-work/p-still")
KEYS = ("span_ms", "shading_ms", "primary_ms", "visibility_ms", "reconstruction_ms", "post_ms")
GI = ("created", "misses", "traces", "rays", "gathers", "gather_rays", "shades", "light_rays", "bounce_queued")


def med(xs):
    xs = [x for x in xs if x is not None]
    return st.median(xs) if xs else None


def summary(run):
    e = json.loads((WORK / "runs" / run / "engine.json").read_text())
    frames = e["frames"]
    gpu = {g["frame_id"]: g for g in e["gpu"]}
    counts = (e.get("counts") or {}).get("frames", [])
    out = {}
    for i, f in enumerate(frames):
        state = f.get("state") or f.get("phase")
        if f["cut"]:
            state = "cut"
        if state == "still":
            state = "still<272" if i < 272 else "still>=272"
        g = gpu.get(f["frame_id"])
        if not g:
            continue
        row = {k: g.get(k) or 0.0 for k in KEYS}
        row["other_ms"] = row["span_ms"] - sum(row[k] for k in KEYS[1:])
        c = counts[i] if i < len(counts) else {}
        row["gi_ran"] = float(c.get("gi_ran", 0))
        row["gi_ms"] = (c.get("gi_ms") or {}).get("total", 0.0) if c.get("gi_ran") else 0.0
        for k in GI:
            row["gi_" + k] = (c.get("gi") or {}).get(k, 0) if c.get("gi_ran") else 0
        row["page_work"] = float(bool(c.get("vsm_page_work")))
        row["walks"] = c.get("surface_walks")
        row["shadow_rays"] = c.get("shadow_rays")
        out.setdefault(state, []).append(row)
    mean = ("gi_ran", "page_work")  # fractions of frames
    return {s: {k: (st.mean(r[k] for r in rows) if k in mean else med(r[k] for r in rows)) for k in rows[0]}
            | {"frames": len(rows)} for s, rows in out.items()}


if __name__ == "__main__":
    result = {}
    for run in sys.argv[1:]:
        result[run] = summary(run)
        print("==", run)
        for state, s in result[run].items():
            print(f"  {state:11s} n={s['frames']:3d} " + " ".join(
                f"{k.replace('_ms','')}={v:.2f}" if isinstance(v, float) else f"{k}={v}" for k, v in s.items() if k != "frames"))
    (WORK / "counts-summary.json").write_text(json.dumps(result, indent=1))
