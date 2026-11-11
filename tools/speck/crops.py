"""U-SPECK crop sheets: the same native-resolution crop from several runs side by side.

    py -3 -B tools/speck/crops.py OUT.png FRAME X,Y,W,H[,ZOOM] DIR[=LABEL] [DIR[=LABEL] ...]

Each DIR holds frame-NNN.png; the crop is cut at native resolution and enlarged ZOOM times
(nearest neighbour, default 1), one panel per DIR with its label.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw


def main():
    out, frame, box = Path(sys.argv[1]), int(sys.argv[2]), [int(v) for v in sys.argv[3].split(",")]
    x, y, w, h = box[:4]
    zoom = box[4] if len(box) > 4 else 1
    panels = []
    for spec in sys.argv[4:]:
        folder, _, label = spec.partition("=")
        image = Image.open(Path(folder) / f"frame-{frame:03d}.png").convert("RGB").crop((x, y, x + w, y + h))
        image = image.resize((w * zoom, h * zoom), Image.NEAREST)
        ImageDraw.Draw(image).text((4, 2), label or Path(folder).name, fill=(255, 255, 0))
        panels.append(image)
    sheet = Image.new("RGB", (sum(p.width for p in panels) + 4 * (len(panels) - 1), panels[0].height), (0, 0, 0))
    at = 0
    for p in panels:
        sheet.paste(p, (at, 0))
        at += p.width + 4
    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(out, sheet.size)


if __name__ == "__main__":
    main()
