"""U-RTVIS cut-refill pictures (tools/rtvis/timing.py `refill4still`, `refill4move`), CPU only.

    py -3 -B tools/rtvis/refill_compare.py PICS_DIR CROPS_DIR

Still clips (camera frozen, the cut turns or jumps the view at frame 24, then holds): each of
frames 23-32 with the refill on and off against frame 59 of the off run, the settled image of
that pose: mean absolute error over the image and over edge pixels (luminance off its 5x5 box
mean by more than 4 codes). Moving clips: on against off per frame. Crops (native, 256x256:
on | off | settled for still clips) of the cut frame and +1..+8 at the 64x64 tile where on and
off differ most on the cut frame, and for still clips at the image centre too.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def load(d, f):
    return np.asarray(Image.open(d / f"frame-{f:03d}.png").convert("RGB"), dtype=np.int16)


def edges(a):
    l = a @ np.array([0.2126, 0.7152, 0.0722])
    p = np.pad(l, 2, mode="edge")
    box = sum(p[i:i + l.shape[0], j:j + l.shape[1]] for i in range(5) for j in range(5)) / 25
    return np.abs(l - box) > 4


def top_tile(mask, size=64):
    h, w = mask.shape
    t = mask[: h // size * size, : w // size * size].reshape(h // size, size, w // size, size).sum((1, 3))
    y, x = np.unravel_index(np.argmax(t), t.shape)
    return int(y * size), int(x * size)


def crop(images, y, x):
    y = max(0, min(y - 96, images[0].shape[0] - 256))
    x = max(0, min(x - 96, images[0].shape[1] - 256))
    return np.concatenate([i[y:y + 256, x:x + 256] for i in images], axis=1)


def main(pics, crops):
    pics, crops = Path(pics), Path(crops)
    crops.mkdir(parents=True, exist_ok=True)
    for on in sorted(pics.glob("p-*-refill1")):
        off = on.with_name(on.name[:-1] + "0")
        clip = on.name[2:-8]
        still = "-still" in clip
        settled = load(off, 59) if still else None
        m = edges(settled) if still else None
        a24, b24 = load(on, 24), load(off, 24)
        ty, tx = top_tile(np.abs(a24 - b24).max(axis=2) > 2)
        print(f"== {clip}: frame on-off(mean max >2 >8)" + (" | vs settled on/off (all; edges)" if still else ""))
        rows, centre = [], []
        for f in range(23, 33):
            a, b = load(on, f), load(off, f)
            d = np.abs(a - b).max(axis=2)
            line = f"{f:3d} {np.abs(a - b).mean():.3f} {d.max():3d} {(d > 2).sum():7d} {(d > 8).sum():6d}"
            if still:
                ea, eb = np.abs(a - settled).max(axis=2), np.abs(b - settled).max(axis=2)
                line += f" | {ea.mean():.2f}/{eb.mean():.2f}; {ea[m].mean():.2f}/{eb[m].mean():.2f}"
            print(line)
            if f >= 24:
                extra = [settled] if still else []
                rows.append(crop([a, b] + extra, ty, tx))
                if still:
                    h, w = a.shape[:2]
                    centre.append(crop([a, b, settled], h // 2, w // 2))
        Image.fromarray(np.concatenate(rows, axis=0).clip(0, 255).astype(np.uint8)).save(crops / f"{clip}-tile.png")
        if centre:
            Image.fromarray(np.concatenate(centre, axis=0).clip(0, 255).astype(np.uint8)).save(crops / f"{clip}-centre.png")


if __name__ == "__main__":
    main(*sys.argv[1:3])
