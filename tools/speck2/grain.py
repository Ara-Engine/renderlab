"""U-SPECK2 grain and speck scores for named regions of the game-scene flight (U-SPECK's
tools/speck/grain.py measures, with a box on the cube's right face, where symptom (b) sits).

    py -3 -B tools/speck2/grain.py RUN_DIR [RUN_DIR ...]

Per run and region (a box at one flight frame, `--every 7` frames 14, 21 and 98):
- grain: RMS of luminance minus its 5x5 mean, in 8-bit codes;
- specks: pixels more than 6 codes above the median of their 7x7 neighbourhood;
- mean: the box's mean luminance in codes (the energy check between two runs).
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage as ndi

# name: (frame, x, y, w, h)
REGIONS = {
    "cube-right f14": (14, 723, 520, 13, 50),
    "cube-left f14": (14, 640, 515, 52, 40),
    "cube f21": (21, 648, 525, 68, 50),
    "wall f21": (21, 600, 345, 270, 80),
    "bush-shadow f98": (98, 1652, 700, 56, 140),
    "brazier-ground f98": (98, 1330, 622, 110, 30),
    "cube-shade f98": (98, 1345, 545, 70, 50),
    "open-ground f98": (98, 60, 760, 560, 280),
}


def scores(path, box):
    x, y, w, h = box
    lum = np.asarray(Image.open(path).convert("RGB"), np.float32) @ np.array([0.2126, 0.7152, 0.0722], np.float32)
    pad = 4
    region = lum[y - pad:y + h + pad, x - pad:x + w + pad]
    grain = (region - ndi.uniform_filter(region, 5))[pad:-pad, pad:-pad]
    specks = (region - ndi.median_filter(region, 7))[pad:-pad, pad:-pad] > 6.0
    return float(np.sqrt((grain ** 2).mean())), int(specks.sum()), float(region[pad:-pad, pad:-pad].mean())


def main():
    names = list(REGIONS)
    print("run".ljust(30) + "".join(n.rjust(22) for n in names) + "   (grain codes / specks / mean)")
    for run in map(Path, sys.argv[1:]):
        cells = []
        for n in names:
            frame, *box = REGIONS[n]
            path = run / f"frame-{frame:03d}.png"
            if not path.exists():
                cells.append("-".rjust(22))
                continue
            g, s, m = scores(path, box)
            cells.append(f"{g:5.2f}/{s:4d}/{m:5.1f}".rjust(22))
        print(run.name.ljust(30) + "".join(cells))


if __name__ == "__main__":
    main()
