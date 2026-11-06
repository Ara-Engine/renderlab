"""N8e: score every label of an N1c quarry run (one per configuration and N8e variant,
`p{pose}-...-v{name}`) against its pose's 64-seed Exact reference, with n1c-score.py's
shadow metrics on the pixels both the label and the pose's traced control cover.
References may be hardlinked from another run of the same binary and poses (--reference).

    py -3 -B scripts/n8e-score.py <run>/quarry [--reference <run with p*.reference>]
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path

for variable in ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS"):
    os.environ[variable] = "2"
import numpy as np

spec = importlib.util.spec_from_file_location("n1c_score", Path(__file__).with_name("n1c-score.py"))
n1c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(n1c)

W, H = 1920, 1080


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("--reference", type=Path, help="directory holding p{pose}.reference")
    a = p.parse_args()
    references = a.reference or a.run
    rows = {}
    timings = json.loads((a.run / "timing.json").read_text()) if (a.run / "timing.json").exists() else []
    for path in sorted(a.run.glob("p*.counts")):
        label = path.stem
        pose = int(label[1:].split("-", 1)[0])
        ref_path = references / f"p{pose if pose < 2 else 0}.reference"
        controls = sorted(a.run.glob(f"p{pose}-Traced*.hit"))
        if not ref_path.exists() or not controls:
            continue
        ref = np.fromfile(ref_path, "<f4").reshape(-1, 2)[:, 0].reshape(H, W)
        covered = np.fromfile(path.with_suffix(".hit"), "<u4").reshape(H, W, 4)[..., 0] != 0
        control = np.fromfile(controls[0], "<u4").reshape(H, W, 4)[..., 0] != 0
        values = np.fromfile(path, "<u4").reshape(H, W) / 256.
        row = n1c.shadow(values, ref, covered & control)
        times = [t for t in timings if t["label"] == label and t.get("shadow_ms") is not None]
        if times:
            row["shadow_ms_p50"] = float(np.median([t["shadow_ms"] for t in times]))
        rows[label] = row
    (a.run / "n8e-scores.json").write_text(json.dumps(rows, indent=2))
    for label, s in rows.items():
        print(f"{label:48s} pRMS={s['partial'].get('rms', float('nan')):.4f} flips={s['solid_flips']} leaks={s['umbra_leaks']} "
              f"litdark={s['lit_darkened']} lost_edges={s['lost_umbra_edges']}")


if __name__ == "__main__":
    main()
