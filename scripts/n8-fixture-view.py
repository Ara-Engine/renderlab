"""N8 fixture screen: count images and N8-minus-Interactive difference maps, plus timing.

    py -3 -B scripts/n8-fixture-view.py E:/renderlab-work/n8/screen-1/fixture
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def main(root):
    root = Path(root)
    info = json.loads((root / "fixture.json").read_text())
    for case in ("sun-40deg", "sun-15deg"):
        shape = (540, 960)
        n8 = np.fromfile(root / f"{case}-n8.u32", dtype="<u4").reshape(shape) / 256
        rt = np.fromfile(root / f"{case}-interactive.u32", dtype="<u4").reshape(shape) / 256
        d = n8 - rt
        rgb = np.zeros((*shape, 3), "u1")
        rgb[..., 0] = np.clip(-d * 255, 0, 255)  # red: N8 darker (false shadow)
        rgb[..., 2] = np.clip(d * 255, 0, 255)  # blue: N8 lighter (lost shadow)
        rgb[..., 1] = (rt * 60).astype("u1")
        grey = lambda v: np.repeat((v * 255).astype("u1")[..., None], 3, -1)
        Image.fromarray(np.concatenate([grey(rt), grey(n8), rgb], 1)).save(root / f"{case}-view.png")
    rows = {}
    for t in info["timing"]:
        rows.setdefault(t["label"], []).append(t)
    for label, values in rows.items():
        def med(key):
            v = [x[key] for x in values if x[key] is not None]
            return float(np.median(v)) if v else None
        print(f"{label:28s} n={len(values):3d} span={med('span_ms')} receiver={med('receiver_ms')} shadow={med('shadow_ms')}")


if __name__ == "__main__":
    main(sys.argv[1])
