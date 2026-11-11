"""U-SPECK mottle view and score: cell-sized unevenness on flat surfaces.

    py -3 -B tools/speck/mottle.py OUT.png FRAME X,Y,W,H GAIN DIR[=LABEL] ...

Per DIR (frame-NNN.png inside), the crop's luminance minus its 31x31 mean, times GAIN, about
mid-grey: a cell mosaic a code or two deep, invisible in a small crop, shows plainly. Prints
per panel the band-pass RMS in 8-bit codes (3x3 mean minus 31x31 mean: structure from a few
pixels to a few cells wide, which pixel grain and smooth gradients both miss).
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage as ndi


def main():
    out, frame, box, gain = Path(sys.argv[1]), int(sys.argv[2]), [int(v) for v in sys.argv[3].split(",")], float(sys.argv[4])
    x, y, w, h = box
    panels = []
    for spec in sys.argv[5:]:
        folder, _, label = spec.partition("=")
        rgb = np.asarray(Image.open(Path(folder) / f"frame-{frame:03d}.png").convert("RGB"), np.float32)
        lum = rgb @ np.array([0.2126, 0.7152, 0.0722], np.float32)
        band = (ndi.uniform_filter(lum, 3) - ndi.uniform_filter(lum, 31))[y:y + h, x:x + w]
        flat = (lum - ndi.uniform_filter(lum, 31))[y:y + h, x:x + w]
        print(f"{label or Path(folder).name}: band-pass RMS {np.sqrt((band ** 2).mean()):.3f} codes, mean {lum[y:y + h, x:x + w].mean():.1f}")
        image = Image.fromarray(np.clip(127.5 + gain * flat, 0, 255).astype(np.uint8)).convert("RGB")
        ImageDraw.Draw(image).text((4, 2), label or Path(folder).name, fill=(255, 255, 0))
        panels.append(image)
    sheet = Image.new("RGB", (sum(p.width for p in panels) + 4 * (len(panels) - 1), panels[0].height))
    at = 0
    for p in panels:
        sheet.paste(p, (at, 0))
        at += p.width + 4
    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(out)


if __name__ == "__main__":
    main()
