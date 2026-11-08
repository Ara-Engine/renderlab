"""M7-R1: compare main's standard program (baseline) with R1's on the quarry, from one
ES1-protocol process per path (scripts/m7-r1-run.py `quarry` steps; docs/m7-r1.md).

    py -3 -B scripts/m7-r1-quarry.py D:/Coding/renderlab-work/m7-r1/runs/LABEL

Images: per path, cache state and pose, the pixels and maximum display-code difference
between the two programs (exposure -14). Cost: the paired visits' shading pass and frame
span medians per program, visits 2-4 (visit 1 warms the driver's shader cache), as
ES1 section 6. Functional timing only, not a quiet-window claim. Writes quarry.json.
"""
import importlib.util
import json
import statistics
import sys
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main(run):
    codes = load("m7-u0b-score").codes
    report = {}
    for path in ("rt", "nonrt"):
        root = run / path
        if not root.exists():
            continue
        entry = report.setdefault(path, {"images": {}, "cost": {}})
        for cache in ("off", "on"):
            for pose in (0, 1):
                a, b = (root / f"{p}-{cache}-p{pose}.f32" for p in ("baseline", "es1"))
                if not (a.exists() and b.exists()):
                    continue
                x, y = (np.fromfile(f, "<f4").reshape(1080, 1920, 3) for f in (a, b))
                d = np.abs(codes(y, -14) - codes(x, -14)).max(axis=-1)
                entry["images"][f"{cache}-p{pose}"] = dict(
                    bit_identical=bool(np.array_equal(x, y)),
                    pixels_differing=int((np.abs(y - x).max(axis=-1) > 0).sum()),
                    pixels_over_one_code=int((d > 1).sum()),
                    max_display_code_difference=float(d.max()))
        data = json.loads((root / "quarry.json").read_text())
        for program, caches in data.get("paired", {}).items():
            for cache, rows in caches.items():
                for phase in ("motion", "cuts"):
                    # Each visit: per-frame shading pass and frame span (ms); visit 1 warms.
                    visits = rows[phase][1:]
                    entry["cost"].setdefault(f"{cache}-{phase}", {})[program] = {
                        key: [round(min(statistics.median(v[key]) for v in visits), 4),
                              round(max(statistics.median(v[key]) for v in visits), 4)]
                        for key in ("shading_ms", "span_ms")}
    (run / "quarry.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
