"""P-FLAT crop sheet (docs/p-flat.md): the same crop of several frames, enlarged with nearest
sampling (no smoothing hides grain) and labelled, in a grid of COLUMNS per row.

    py -3 -B scripts/p-flat-sheet.py OUT.png X0,Y0,X1,Y1 SCALE COLUMNS LABEL=PNG [LABEL=PNG ...]
"""
import sys

from PIL import Image, ImageDraw


def main():
    out, box, scale, columns = sys.argv[1], [int(v) for v in sys.argv[2].split(",")], int(sys.argv[3]), int(sys.argv[4])
    panels = [a.split("=", 1) for a in sys.argv[5:]]
    w, h = (box[2] - box[0]) * scale, (box[3] - box[1]) * scale
    label_h = 22
    rows = (len(panels) + columns - 1) // columns
    sheet = Image.new("RGB", (columns * w, rows * (h + label_h)), (24, 24, 24))
    draw = ImageDraw.Draw(sheet)
    for k, (label, path) in enumerate(panels):
        x, y = (k % columns) * w, (k // columns) * (h + label_h)
        crop = Image.open(path).convert("RGB").crop(box).resize((w, h), Image.NEAREST)
        sheet.paste(crop, (x, y + label_h))
        draw.text((x + 6, y + 4), label, fill=(235, 235, 235))
    sheet.save(out)


if __name__ == "__main__":
    main()
