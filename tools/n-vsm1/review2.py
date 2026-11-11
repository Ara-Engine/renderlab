"""N-VSM2 review sheets (CPU) from bench.py's packed counts and game_scene frames.

    py -3 -B tools/n-vsm1/review2.py BEFORE AFTER SURFACE COURT_RUNS OUT_DIR
        BEFORE, AFTER, SURFACE: bench roots (bench.py score); COURT_RUNS: "rt=DIR,surface=DIR,before=DIR,after=DIR"

Writes n-vsm2-s2-error.jpg (S2 shadow-step frame 16: visibility and signed error against the
film, x4, blue darker), n-vsm2-s3-flips.jpg (S3 frame 0: lit pixels turned dark, marked red on
the film's visibility) and n-vsm2-courtyard.jpg (flight frame 24 against the RT path).
"""
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
from exact import Frame


def counts(root, bench, case, frame):
    with np.load(Path(root) / f"vis-{bench}-{case}.npz") as z:
        return z["counts"][list(z["frames"]).index(frame)] / 256.0


def gray(v):
    return np.repeat((np.clip(v, 0, 1) * 255).astype("u1")[..., None], 3, -1)


def signed(e, cover, gain=4):
    out = np.stack([np.clip(e * gain, 0, 1), np.zeros_like(e), np.clip(-e * gain, 0, 1)], -1)
    out[~cover] = 0.12
    return (out * 255).astype("u1")


def sheet(rows, labels, path, zoom=1):
    h, w = rows[0][0].shape[:2]
    image = Image.new("RGB", (w * zoom * len(rows[0]), (h * zoom + 18) * len(rows)), (20, 24, 32))
    draw = ImageDraw.Draw(image)
    for r, row in enumerate(rows):
        for c, cell in enumerate(row):
            image.paste(Image.fromarray(cell).resize((w * zoom, h * zoom), Image.NEAREST), (c * w * zoom, r * (h * zoom + 18) + 18))
            draw.text((c * w * zoom + 4, r * (h * zoom + 18) + 3), labels[r][c], fill=(235, 235, 235))
    image.save(path, quality=88)


if __name__ == "__main__":
    before, after, surface, court, out = sys.argv[1:6]
    out = Path(out)
    f = Frame("S2", "shadow-step-r0.04", 16)
    ref, cover = f.d["ref"], f.d["cover"]
    crop = (slice(40, 210), slice(0, 360))
    v = {n: counts(r, "S2", "shadow-step-r0.04", 16) for n, r in (("Surface", surface), ("Disk before", before), ("Disk after", after))}
    sheet([[gray(ref)[crop]] + [gray(x)[crop] for x in v.values()],
           [gray(ref * 0)[crop]] + [signed(x - ref, cover)[crop] for x in v.values()]],
          [["Film visibility"] + list(v), [""] + [n + ": error x4 (blue darker)" for n in v]], out / "n-vsm2-s2-error.jpg", 2)
    f = Frame("S3", "sun-r0.04", 0)
    ref, lit = f.d["ref"], f.pops["lit"]
    cells, names = [], []
    for n, r in (("Surface", surface), ("Disk before", before), ("Disk after", after)):
        x = counts(r, "S3", "sun-r0.04", 0)
        flips = lit & (x - ref < -0.5)
        image = gray(ref * 0.6 + 0.2)
        # A 3x3 mark per flipped pixel so single pixels show on the sheet.
        ys, xs = np.nonzero(flips)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                image[np.clip(ys + dy, 0, image.shape[0] - 1), np.clip(xs + dx, 0, image.shape[1] - 1)] = (255, 40, 40)
        cells.append(image)
        names.append(f"{n}: {int(flips.sum())} lit pixels turned dark")
    sheet([cells], [names], out / "n-vsm2-s3-flips.jpg")
    runs = dict(item.split("=", 1) for item in court.split(","))
    frame = {n: np.asarray(Image.open(Path(p) / "frame-024.png").convert("RGB"), np.float32) for n, p in runs.items()}
    part = (slice(330, 620), slice(380, 1280))
    diff = lambda n: np.clip(np.abs(frame[n] - frame["rt"]).max(-1, keepdims=True).repeat(3, -1) * 8, 0, 255).astype("u1")[part]
    rms = lambda n: float(np.sqrt(((frame[n] - frame["rt"]) ** 2).mean()))
    sheet([[frame[n].astype("u1")[part] for n in ("rt", "surface")], [frame[n].astype("u1")[part] for n in ("before", "after")],
           [diff("before"), diff("after")]],
          [["RT path", f"Surface (RMS {rms('surface'):.2f})"], [f"Disk before (RMS {rms('before'):.2f})", f"Disk after (RMS {rms('after'):.2f})"],
           ["|Disk before - RT| x8", "|Disk after - RT| x8"]], out / "n-vsm2-courtyard.jpg")
