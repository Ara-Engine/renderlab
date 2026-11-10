"""P-FLICKER clips and crop sheets (CPU): the walk frames 160-223 of several captures side by
side, through the frozen F0 display transform (-14 EV), as an animated GIF at playback speed
(30 fps) and a sheet of frame steps (|frame t - frame t-1| x 4) next to the reference's step.

    py -3 -B tools/p-flicker/clips.py NAME X Y W H SCALE RUN...
      RUN: runs/ directory (clip captures), or ue:CONFIG (p-flicker/ue/quarry/capture/CONFIG)
    Output: D:/Coding/renderlab-work/p-flicker/sheets/NAME.gif and NAME-steps.png
"""
import importlib.util
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-flicker")
spec = importlib.util.spec_from_file_location("pflicker_split", HERE / "split.py")
split = importlib.util.module_from_spec(spec)
spec.loader.exec_module(split)
FRAMES = range(160, 224)
STEP_FRAMES = (161, 162, 163, 164)


def tile(rgb, box, scale, label):
    x, y, w, h = box
    img = Image.fromarray(np.clip(rgb[y:y + h, x:x + w], 0, 255).astype(np.uint8)).resize((w * scale, h * scale), Image.NEAREST)
    ImageDraw.Draw(img).text((4, 4), label, fill=(255, 255, 0))
    return img


def row(tiles):
    out = Image.new("RGB", (sum(t.width for t in tiles) + 4 * (len(tiles) - 1), tiles[0].height))
    x = 0
    for t in tiles:
        out.paste(t, (x, 0))
        x += t.width + 4
    return out


if __name__ == "__main__":
    name, *box, scale = sys.argv[1:7]
    box, scale, runs = [int(v) for v in box], int(scale), sys.argv[7:]
    loaders = {r: split.loaders(r) for r in runs}
    shown = {r: {} for r in runs}
    frames = []
    for i in FRAMES:
        if not all(i in loaders[r] for r in runs):
            continue
        tiles = []
        for r in runs:
            d = split.score.codes(loaders[r][i](), split.STOPS)
            if i in STEP_FRAMES or i + 1 in STEP_FRAMES:
                shown[r][i] = d
            tiles.append(tile(d, box, scale, f"{r} {i}"))
        frames.append(row(tiles))
    out = WORK / "sheets"
    out.mkdir(exist_ok=True)
    frames[0].save(out / f"{name}.gif", save_all=True, append_images=frames[1:], duration=33, loop=0)
    # Steps: each engine's frame-to-frame change, and the reference's where both frames have one.
    rows = []
    for i in STEP_FRAMES:
        tiles = []
        for r in runs:
            if i in shown[r] and i - 1 in shown[r]:
                tiles.append(tile(np.abs(shown[r][i] - shown[r][i - 1]) * 4, box, scale, f"{r} step {i}"))
        f, p = split.FRAMES[i], split.FRAMES[i - 1]
        if f["reference"] and p["reference"]:
            a, b = split.score.reference(f["reference"]), split.score.reference(p["reference"])
            tiles.append(tile(np.abs(a["display"][0][0] - b["display"][0][0]) * 4, box, scale, f"reference step {i}"))
        if tiles:
            rows.append(row(tiles))
    if rows:
        sheet = Image.new("RGB", (max(r.width for r in rows), sum(r.height + 4 for r in rows)))
        y = 0
        for r in rows:
            sheet.paste(r, (0, y))
            y += r.height + 4
        sheet.save(out / f"{name}-steps.png")
    print(out / f"{name}.gif", len(frames), "frames")
