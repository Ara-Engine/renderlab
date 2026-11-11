"""N-VSM1 crop sheets (CPU): labelled rows of the same crop from several images.

    py -3 -B tools/n-vsm1/sheet.py OUT.png [--crop X,Y,W,H]... [--zoom N] [--diff A:B] [--stops S] LABEL=IMAGE...

IMAGE is a PNG (game_scene frames), a yardstick `hdr-NNNN.bin` (Renderlab's HDR target; its
engine.json beside it gives the size and pre-exposure) or a yardstick Unreal `.npz` capture.
HDR images show through the yardstick's display transform (x/(1+x), sRGB) at --stops (default
-14, the quarry's exposure). Each --crop adds a row; without one the row is the whole frame.
--diff A:B appends |A - B| x 8 for the cells labelled A and B.
"""
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw


def display(x, stops):
    x = np.maximum(x * np.float32(2.0 ** stops), 0)
    x = x / (1 + x)
    return np.clip(255 * np.where(x <= .0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - .055), 0, 255)


def load(path, stops):
    path = Path(path)
    if path.suffix == ".png":
        return np.asarray(Image.open(path).convert("RGB"), np.float32)
    if path.suffix == ".npz":
        with np.load(path) as c:
            return display(c["hdr"][..., :3].astype(np.float32) / np.float32(c["pre_exposure"]), stops)
    info = json.loads((path.parent / "engine.json").read_text())["hdr"]
    w, h = info["width"], info["height"]
    raw = np.fromfile(path, "<f2")
    row = raw.size // h  # rows are padded to 256 bytes
    hdr = raw.reshape(h, row)[:, :w * 4].reshape(h, w, 4)[..., :3].astype(np.float32) / np.float32(info["pre_exposure"])
    return display(hdr, stops)


def main(args):
    out, crops, zoom, diff, stops, cells = Path(args[0]), [], 1, None, -14.0, []
    it = iter(args[1:])
    for a in it:
        if a == "--crop":
            crops.append([int(v) for v in next(it).split(",")])
        elif a == "--zoom":
            zoom = int(next(it))
        elif a == "--diff":
            diff = next(it).split(":")
        elif a == "--stops":
            stops = float(next(it))
        else:
            label, path = a.split("=", 1)
            cells.append((label, load(path, stops)))
    rows = []
    for crop in crops or [None]:
        row = []
        for label, image in cells:
            if crop:
                x, y, w, h = crop
                image = image[y:y + h, x:x + w]
            row.append((label, image))
        if diff:
            a, b = (dict(row)[k] for k in diff)
            row.append((f"|{diff[0]} - {diff[1]}| x 8", np.clip(np.abs(a - b) * 8, 0, 255)))
        rows.append(row)
    tiles = [[(label, Image.fromarray(image.astype(np.uint8)).resize((image.shape[1] * zoom, image.shape[0] * zoom), Image.NEAREST))
              for label, image in row] for row in rows]
    width = max(sum(t.width + 4 for _, t in row) for row in tiles)
    height = sum(max(t.height for _, t in row) + 18 for row in tiles)
    sheet = Image.new("RGB", (width, height), (24, 24, 24))
    draw = ImageDraw.Draw(sheet)
    y = 0
    for row in tiles:
        x = 0
        for label, tile in row:
            draw.text((x + 2, y + 2), label, fill=(230, 230, 230))
            sheet.paste(tile, (x, y + 16))
            x += tile.width + 4
        y += max(t.height for _, t in row) + 18
    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(out, sheet.size)


if __name__ == "__main__":
    main(sys.argv[1:])
