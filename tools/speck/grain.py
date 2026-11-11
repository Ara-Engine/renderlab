"""U-SPECK grain and speck scores for named flat regions of the game-scene flight.

    py -3 -B tools/speck/grain.py RUNS_DIR [RUN ...]        # every run folder, or the named ones

Per run and region (a box on a flat surface at one flight frame):
- grain: RMS of luminance minus its 5x5 mean, in 8-bit codes (pixel-scale noise; a flat,
  noise-free surface reads about 0.1-0.3, a textured one its texture);
- specks: pixels more than 6 codes above the median of their 7x7 neighbourhood (isolated
  bright points; soft blobs are tools/blobs.py's).
Regions are the ones Islam circled (root, 2026-11-11), at the frames `ablate.py --every 7`
writes: 14, 21 and 98 (his frame 97).
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage as ndi

# name: (frame, x, y, w, h)
REGIONS = {
    "cube-left f14": (14, 640, 515, 52, 40),
    "cube-right f14": (14, 699, 505, 12, 40),
    "wall f21": (21, 600, 345, 270, 80),
    "cube f21": (21, 648, 525, 68, 50),
    "pillar-front f98": (98, 612, 300, 56, 380),
    "pillar-inner f98": (98, 702, 330, 36, 290),
    "wall f98": (98, 1750, 430, 150, 430),
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
    return float(np.sqrt((grain ** 2).mean())), int(specks.sum()), float(region.mean())


def main():
    root = Path(sys.argv[1])
    runs = [root / r for r in sys.argv[2:]] or sorted(p for p in root.iterdir() if (p / "frame-000.png").exists())
    names = list(REGIONS)
    print("run".ljust(24) + "".join(n.rjust(22) for n in names) + "   (grain codes / specks / mean)")
    for run in runs:
        cells = []
        for n in names:
            frame, *box = REGIONS[n]
            g, s, m = scores(run / f"frame-{frame:03d}.png", box)
            cells.append(f"{g:5.2f}/{s:4d}/{m:5.1f}".rjust(22))
        print(run.name.ljust(24) + "".join(cells))


if __name__ == "__main__":
    main()
