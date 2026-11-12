"""U-RTVIS: the same picture clips from two binaries (CPU only).

    py -3 -B tools/rtvis/bin_compare.py RUNS_A RUNS_B          # game clips: per-frame rgba8 hashes
    py -3 -B tools/rtvis/bin_compare.py --png DIR_A DIR_B       # two PNG folders, pixel by pixel

Game clips (run.json from tools/rtvis/timing.py): for each clip present in both, the frames whose
hashes differ. PNG folders: per file, the pixels that differ and the largest difference.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def clips(a, b):
    for d in sorted(Path(a).iterdir()):
        other = Path(b) / d.name
        if not (d / "run.json").exists() or not (other / "run.json").exists() or d.name == "reference":
            continue
        x, y = (json.loads((p / "run.json").read_text())["frames"] for p in (d, other))
        differ = [f["frame"] for f, g in zip(x, y) if f["rgba8_sha256"] != g["rgba8_sha256"]]
        print(f"{d.name:22} frames {len(x)}  differing {len(differ)}: {differ[:12]}")


def pngs(a, b):
    for p in sorted(Path(a).glob("*.png")):
        q = Path(b) / p.name
        if not q.exists():
            continue
        x = np.asarray(Image.open(p).convert("RGB"), dtype=np.int16)
        y = np.asarray(Image.open(q).convert("RGB"), dtype=np.int16)
        d = np.abs(x - y).max(axis=2)
        print(f"{p.name:20} differing px {(d > 0).sum():7d}  max {d.max():3d}")


if __name__ == "__main__":
    if sys.argv[1] == "--png":
        pngs(*sys.argv[2:4])
    else:
        clips(*sys.argv[1:3])
