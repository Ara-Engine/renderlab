"""U-SPECK settled-frame energy check: region means of two renders of the same frame.

    py -3 -B tools/speck/energy.py BEFORE.png AFTER.png [--tile 120] [--sheet OUT.png]

Splits the frame into TILE x TILE pixel regions and prints, in 8-bit codes, the largest and
the RMS change of the regions' mean (per channel and luminance), the regions that moved by
more than 1 code, and the frame mean. A fix that only spreads light keeps every region within
1 code; a clamp shows as regions that lost light. `--sheet` writes the signed per-region
luminance change, mid-grey = 0, 32 grey levels a code.
"""
import argparse

import numpy as np
from PIL import Image


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--tile", type=int, default=120)
    ap.add_argument("--sheet")
    a = ap.parse_args()
    before = np.asarray(Image.open(a.before).convert("RGB"), np.float64)
    after = np.asarray(Image.open(a.after).convert("RGB"), np.float64)
    h, w = before.shape[:2]
    t = a.tile
    rows, cols = h // t, w // t
    tiles = lambda x: x[:rows * t, :cols * t].reshape(rows, t, cols, t, 3).mean(axis=(1, 3))
    change = tiles(after) - tiles(before)
    lum = change @ np.array([0.2126, 0.7152, 0.0722])
    worst = np.abs(change).max()
    moved = np.argwhere(np.abs(lum) > 1.0)
    print(f"{rows}x{cols} regions of {t} px: luminance change max {np.abs(lum).max():.2f} codes, RMS {np.sqrt((lum ** 2).mean()):.2f}, "
          f"mean {lum.mean():+.2f}; worst channel {worst:.2f}; regions over 1 code: {len(moved)}")
    for r, c in moved[:12]:
        print(f"  region x {c * t}-{c * t + t}, y {r * t}-{r * t + t}: {lum[r, c]:+.2f} codes (before {tiles(before)[r, c].mean():.1f})")
    if a.sheet:
        image = np.clip(127.5 + 32.0 * lum, 0, 255).astype(np.uint8)
        Image.fromarray(image).resize((cols * 16, rows * 16), Image.NEAREST).save(a.sheet)


if __name__ == "__main__":
    main()
