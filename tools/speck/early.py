"""U-SPECK: how soon a held camera's picture gets its bounce light, per run.

    py -3 -B tools/speck/early.py RUNS_DIR NAME [NAME ...]     # e.g. first1 first4

For RUNS_DIR/{early,fieldearly}-{rt,nonrt}-NAME (tools/speck/ablate.py `--early`,
`--field-early`: a held camera's first 33 frames, every fourth written): the distance of
frames 4, 8 and 16 from frame 32 (mean, and the largest of the 120 px regions, in 8-bit codes),
and the gathers and cells still waiting per frame; and for RUNS_DIR/{rt,nonrt}-NAME (the
flight) the gathers on the frames after its cut at frame 24.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

LUM = np.array([0.2126, 0.7152, 0.0722])


def regions(path, tile=120):
    a = np.asarray(Image.open(path).convert("RGB"), np.float64) @ LUM
    rows, cols = a.shape[0] // tile, a.shape[1] // tile
    return a[:rows * tile, :cols * tile].reshape(rows, tile, cols, tile).mean(axis=(1, 3))


def log(folder):
    return [json.loads(line) for line in (folder / "gi.jsonl").read_text().splitlines() if line.strip()]


def main():
    root = Path(sys.argv[1])
    for name in sys.argv[2:]:
        for scene in ("early", "fieldearly"):
            for path in ("rt", "nonrt"):
                folder = root / f"{scene}-{path}-{name}"
                if not folder.exists():
                    continue
                settled = regions(folder / "frame-032.png")
                parts = []
                for frame in (4, 8, 16):
                    d = regions(folder / f"frame-{frame:03d}.png") - settled
                    parts.append(f"f{frame} mean {d.mean():+.2f} worst region {np.abs(d).max():.2f}")
                rows = log(folder)
                gathers = " ".join(f"{i}:{rows[i]['gathers']}/{rows[i]['bounce_queued']}" for i in (1, 2, 4, 6, 8, 12, 16) if i < len(rows))
                print(f"{scene} {path} {name}: " + "; ".join(parts) + f" | gathers/waiting at frame {gathers}")
        for path in ("rt", "nonrt"):
            folder = root / f"{path}-{name}"
            if folder.exists():
                rows = log(folder)
                print(f"flight {path} {name}: gathers at frames 24-31 {[r['gathers'] for r in rows[24:32]]}, largest in the flight {max(r['gathers'] for r in rows)}, "
                      f"total {sum(r['gathers'] for r in rows)}, gather rays total {sum(r['gather_rays'] for r in rows)}")


if __name__ == "__main__":
    main()
