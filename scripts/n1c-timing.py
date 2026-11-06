"""Summarize an N1c quarry timing run: per cut configuration and pose parity, the median
over repeated pose visits of each visit's median primary/shadow/frame GPU time, the
visit range, and median stage times. Writes timing-summary.json beside the inputs.

    py -3 -B scripts/n1c-timing.py <run>/quarry-no-rt
"""
import json
import statistics as s
import sys
from pathlib import Path

STAGES = ["cull 1", "software 1", "hardware 1", "HZB 1", "cull 2", "software 2", "hardware 2", "HZB 2", "resolve"]


def main():
    run = Path(sys.argv[1])
    timing = json.loads((run / "timing.json").read_text())
    stages = {x["label"]: x["stage_ms"] for x in json.loads((run / "stages.json").read_text())}
    configs = {}
    for label in dict.fromkeys(t["label"] for t in timing if t["label"] != "warm"):
        pose, config = label[1:].split("-", 1)
        rows = [t for t in timing if t["label"] == label]
        visit = {k: s.median(t[k] for t in rows) for k in ("primary_ms", "shadow_ms", "span_ms")}
        visit["stages"] = stages.get(label)
        configs.setdefault(config, {}).setdefault(int(pose) % 2, []).append(visit)
    out = {}
    for config, poses in configs.items():
        for pose, visits in poses.items():
            row = {}
            for k in ("primary_ms", "shadow_ms", "span_ms"):
                v = [x[k] for x in visits]
                row[k] = dict(median=s.median(v), min=min(v), max=max(v), visits=len(v))
            if all(x["stages"] for x in visits):
                row["stage_ms"] = {name: s.median(x["stages"][i] for x in visits) for i, name in enumerate(STAGES)}
                row["stage_sum_ms"] = sum(row["stage_ms"].values())
            out[f"{config} pose{pose}"] = row
            st = row.get("stage_ms", {})
            print(f"{config:20s} pose{pose} primary {row['primary_ms']['median']:.3f} [{row['primary_ms']['min']:.3f}-{row['primary_ms']['max']:.3f}]"
                  f" shadow {row['shadow_ms']['median']:.3f} frame {row['span_ms']['median']:.3f} [{row['span_ms']['min']:.3f}-{row['span_ms']['max']:.3f}] |"
                  + " ".join(f"{v:.3f}" for v in st.values()) + (f" sum {row['stage_sum_ms']:.3f}" if st else ""))
    (run / "timing-summary.json").write_text(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
