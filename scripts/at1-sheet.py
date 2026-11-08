"""AT1: a labelled half-resolution contact sheet of one run's PNGs per path, for inspection.

  py -3 -I scripts/at1-sheet.py <run dir with raster/ nonrt/ traced/> <out.png>
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

CONFIGS = ["pre-at1", "default", "noon", "low", "low-volumetric"]


def main():
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    paths = [p for p in ["raster", "nonrt", "traced"] if (root / p).exists()]
    w, h = 960, 540
    sheet = Image.new("RGB", (w * len(CONFIGS), h * len(paths)), "black")
    draw = ImageDraw.Draw(sheet)
    for row, path in enumerate(paths):
        for col, config in enumerate(CONFIGS):
            f = root / path / f"{path}-{config}.png"
            if f.exists():
                sheet.paste(Image.open(f).resize((w, h), Image.LANCZOS), (col * w, row * h))
            draw.text((col * w + 8, row * h + 8), f"{path} {config}", fill="yellow")
    sheet.save(out)
    print(out)


if __name__ == "__main__":
    main()
