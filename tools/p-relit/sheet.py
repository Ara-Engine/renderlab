"""P-RELIT crop sheets (CPU; docs/p-relit.md): labelled grids of courtyard frames or of crops
centred on a world point, projected with each frame's recorded camera.

    py -3 -B tools/p-relit/sheet.py OUT.png SPEC...
      SPEC: LABEL=RUN_DIR@FRAME            one cell (cells fill a row; `--row` starts a new row)
      options before the cells: --at X,Y,Z  centre crops on this world point (default: full frame)
                                --size W,H  crop size in pixels (default 240,160)
                                --zoom N    nearest-neighbour upscale (default 1)
                                --diff A:B  append |A - B| x 8 cells for the cells labelled A and B in each row
"""
import json
import math
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

FOV_Y = math.radians(55.0)


def project(run, frame, point):
    entry = next(f for f in run["frames"] if f["frame"] == frame)
    eye, target = np.array(entry["eye"]), np.array(entry["target"])
    w, h = run["size"]
    forward = (target - eye) / np.linalg.norm(target - eye)
    right = np.cross(forward, [0.0, 1.0, 0.0])
    right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    d = np.array(point) - eye
    z = d @ forward
    f = 0.5 * h / math.tan(0.5 * FOV_Y)
    return int(round(w / 2 + f * (d @ right) / z)), int(round(h / 2 - f * (d @ up) / z))


def cell(run_dir, frame, at, size):
    run_dir = Path(run_dir)
    image = Image.open(run_dir / f"frame-{frame:03d}.png").convert("RGB")
    if at is None:
        return image
    run = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
    x, y = project(run, frame, at)
    w, h = size
    x = min(max(x - w // 2, 0), image.width - w)
    y = min(max(y - h // 2, 0), image.height - h)
    return image.crop((x, y, x + w, y + h))


def main():
    args = sys.argv[1:]
    out = Path(args.pop(0))
    at, size, zoom, diff = None, (240, 160), 1, None
    rows = [[]]
    while args:
        a = args.pop(0)
        if a == "--at":
            at = tuple(float(v) for v in args.pop(0).split(","))
        elif a == "--size":
            size = tuple(int(v) for v in args.pop(0).split(","))
        elif a == "--zoom":
            zoom = int(args.pop(0))
        elif a == "--diff":
            diff = args.pop(0).split(":")
        elif a == "--row":
            rows.append([])
        else:
            label, spec = a.split("=", 1)
            run, frame = spec.rsplit("@", 1)
            rows[-1].append((label, cell(run, int(frame), at, size)))
    if diff:
        for row in rows:
            named = {label.split(" ")[0]: image for label, image in row}
            if all(d in named for d in diff):
                a, b = (np.asarray(named[d], np.int16) for d in diff)
                row.append((f"|{diff[0]} - {diff[1]}| x 8", Image.fromarray(np.clip(np.abs(a - b) * 8, 0, 255).astype(np.uint8))))
    cw, ch = rows[0][0][1].width * zoom, rows[0][0][1].height * zoom
    pad = 16
    sheet = Image.new("RGB", (max(len(r) for r in rows) * cw, len(rows) * (ch + pad)), (24, 24, 24))
    draw = ImageDraw.Draw(sheet)
    for j, row in enumerate(rows):
        for i, (label, image) in enumerate(row):
            if zoom != 1:
                image = image.resize((cw, ch), Image.NEAREST)
            sheet.paste(image, (i * cw, j * (ch + pad) + pad))
            draw.text((i * cw + 4, j * (ch + pad) + 2), label, fill=(230, 230, 230))
    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(out, sheet.size)


if __name__ == "__main__":
    main()
