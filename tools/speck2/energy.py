"""U-SPECK2 settled-frame energy check: region means of two frames of the same view.

    py -3 -B tools/speck2/energy.py BEFORE.png AFTER.png [--tile 60] [--diff OUT.png]

Splits the frame into TILE x TILE boxes and prints the largest change of a box's mean, per
channel and in luminance, in 8-bit codes, with the boxes that moved by more than one code.
`--diff` writes |after - before| x 16 for a look at where the change sits.
"""
import sys

import numpy as np
from PIL import Image


def main():
    before, after = (np.asarray(Image.open(p).convert("RGB"), np.float32) for p in sys.argv[1:3])
    tile = int(sys.argv[sys.argv.index("--tile") + 1]) if "--tile" in sys.argv else 60
    h, w, _ = before.shape
    luma = np.array([0.2126, 0.7152, 0.0722], np.float32)
    rows = []
    for y in range(0, h - tile + 1, tile):
        for x in range(0, w - tile + 1, tile):
            a, b = before[y:y + tile, x:x + tile], after[y:y + tile, x:x + tile]
            d = b.mean((0, 1)) - a.mean((0, 1))
            rows.append((float(abs(d @ luma)), float(np.abs(d).max()), x, y, float(a.mean()), float(d @ luma)))
    rows.sort(reverse=True)
    over = [r for r in rows if r[1] > 1.0]
    print(f"{len(rows)} boxes of {tile} px: largest luminance change {rows[0][0]:.2f} codes, largest channel change "
          f"{max(r[1] for r in rows):.2f}; {len(over)} boxes above 1 code; whole-frame mean change {float((after - before).mean()):+.3f}")
    for lum, channel, x, y, mean, signed in over[:12]:
        print(f"   box ({x},{y}) mean {mean:5.1f}: luminance {signed:+.2f}, worst channel {channel:.2f}")
    print(f"   pixels changed by more than 2 codes: {int((np.abs(after - before).max(2) > 2).sum())}")
    if "--diff" in sys.argv:
        Image.fromarray(np.clip(np.abs(after - before) * 16, 0, 255).astype(np.uint8)).save(sys.argv[sys.argv.index("--diff") + 1])


if __name__ == "__main__":
    main()
