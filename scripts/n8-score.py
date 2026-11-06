"""N8 S2/S3 scoring: the unchanged bench-motion scorer on the N8 batch roots, then one table
per case and phase with N8 and RL Interactive (this run) beside the recorded Unreal rows
(E:/renderlab-work/motion-benches/S*/comparison.csv; the Unreal arrays were compacted, so
their recorded summaries are reused, not rescored).

    py -3 -B scripts/n8-score.py --root E:/renderlab-work/n8/batch
"""
import argparse
import csv
import importlib.util
import json
from pathlib import Path

import numpy as np

RECORDED = Path("E:/renderlab-work/motion-benches")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--skip-scoring", action="store_true")
    args = parser.parse_args()
    motion = load("bench-motion-score")
    out = []
    for bench in ("S2", "S3"):
        root = args.root / bench
        if not (root / "renderlab/timings.json").exists():
            continue
        cases = [p.name for p in (root / "renderlab").iterdir() if p.is_dir()]
        if not args.skip_scoring:
            for case in cases:
                motion.score_case(root, case, True)
            motion.comparison(root)
        table = json.loads((root / "comparison.json").read_text())
        timings = json.loads((root / "renderlab/timings.json").read_text())
        recorded = list(csv.DictReader((RECORDED / bench / "comparison.csv").open()))
        scene = json.loads((root / "fixture/scene.json").read_text())
        for case in cases:
            track = next(t for t in scene["tracks"] if case.startswith(t["name"] + "-r"))
            for phase in dict.fromkeys(p["phase"] for p in track["frames"]):
                frames = {p["frame"] for p in track["frames"] if p["phase"] == phase}
                for row in table:
                    if row["case"] == case and row["phase"] == phase:
                        mode = row["mode"].removeprefix("RL ")
                        shadow = [t["visibility_ms"] for t in timings if t["case"] == case and t["mode"] == mode and t["frame"] in frames and t.get("visibility_ms") is not None]
                        out.append(dict(bench=bench, source="this run", case=case, phase=phase, mode=row["mode"].replace("Virtual", "N8"),
                                        partial_rms=row["partial_rms"], hdr_rms=row.get("hdr_rms"), contact_lost=row["contact_lost"], contact_n=row["contact_n"],
                                        leaks_max=row["leaks_max"], flips_max=row["solid_flips_max"], gpu_p50=row.get("gpu_ms_p50"), gpu_p95=row.get("gpu_ms_p95"),
                                        shadow_p50=float(np.median(shadow)) if shadow else None, shadow_p95=float(np.quantile(shadow, .95)) if shadow else None))
                for row in recorded:
                    if row["case"] == case and row["phase"] == phase and row["mode"] in ("UE VSM", "RL Interactive", "UE HRT1"):
                        f = lambda k: float(row[k]) if row.get(k) not in (None, "") else None
                        depth, projection = f("shadow_depths_ms_p50"), f("projection_ms_p50")
                        out.append(dict(bench=bench, source="recorded", case=case, phase=phase, mode=row["mode"], partial_rms=f("partial_rms"), hdr_rms=f("hdr_rms"),
                                        contact_lost=int(float(row["contact_lost"])), contact_n=int(float(row["contact_n"])), leaks_max=int(float(row["leaks_max"])),
                                        flips_max=int(float(row["solid_flips_max"])), gpu_p50=f("gpu_ms_p50"), gpu_p95=f("gpu_ms_p95"),
                                        shadow_p50=(depth + projection) if depth is not None and projection is not None else None, shadow_p95=None))
    (args.root / "n8-comparison.json").write_text(json.dumps(out, indent=2))
    for r in out:
        fmt = lambda v: "-" if v is None else (f"{v:.4f}" if isinstance(v, float) else str(v))
        print(" | ".join(fmt(r[k]) for k in ("bench", "case", "phase", "source", "mode", "partial_rms", "hdr_rms", "contact_lost", "contact_n", "leaks_max", "flips_max", "gpu_p50", "shadow_p50", "shadow_p95")))


if __name__ == "__main__":
    main()
