"""U-RTVIS step-3 picture comparison: the cold probe stop on against off (tools/rtvis/timing.py
`pictures`), CPU only.

    py -3 -B tools/rtvis/compare.py PICS_DIR [CROPS_DIR]

Per RT clip and frame: the mean and largest per-channel difference on against off (0-255),
the pixels more than 2 and 8 codes apart, and their 64x64 tile with the most changed pixels.
For the still forest clip (camera and objects frozen, cut at 24) each variant's cut frame and
the five after it are also scored against frame 59, the settled image of the same pose: mean
absolute error over the image and over penumbra pixels (where the settled frames 23 and 59
are neither the image's darkest nor brightest tenth of luminance... approximated as pixels
whose settled luminance differs from its 15x15 median by more than 4 codes). CROPS_DIR gets
native-resolution crops (256x256) of the most changed tile, on | off | settled, per frame.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image

CLIPS = ("p-forest-still", "p-forest-walk", "p-game-rt")


def load(d, f):
    return np.asarray(Image.open(d / f"frame-{f:03d}.png").convert("RGB"), dtype=np.int16)


def tile_of(mask, size=64):
    h, w = mask.shape
    t = mask[: h // size * size, : w // size * size].reshape(h // size, size, w // size, size).sum((1, 3))
    y, x = np.unravel_index(np.argmax(t), t.shape)
    return int(y * size), int(x * size), int(t.max())


def luminance(a):
    return a @ np.array([0.2126, 0.7152, 0.0722])


def edges(a):
    # Pixels near a luminance step: |L - 5x5 box mean| > 4 codes, a cheap penumbra/edge mask.
    l = luminance(a)
    k = 5
    p = np.pad(l, k // 2, mode="edge")
    box = sum(p[i:i + l.shape[0], j:j + l.shape[1]] for i in range(k) for j in range(k)) / (k * k)
    return np.abs(l - box) > 4


def crop(images, y, x, out):
    y = max(0, min(y - 96, images[0].shape[0] - 256))
    x = max(0, min(x - 96, images[0].shape[1] - 256))
    strip = np.concatenate([i[y:y + 256, x:x + 256] for i in images], axis=1).clip(0, 255).astype(np.uint8)
    Image.fromarray(strip).save(out)


def main(pics, crops=None):
    pics = Path(pics)
    crops = Path(crops) if crops else None
    if crops:
        crops.mkdir(parents=True, exist_ok=True)
    for clip in CLIPS:
        on, off = pics / f"{clip}-cold1", pics / f"{clip}-cold0"
        frames = sorted(int(p.stem[6:]) for p in on.glob("frame-*.png") if p.stem[6:].isdigit())
        settled = load(off, frames[-1]) if clip == "p-forest-still" else None
        print(f"== {clip}: frame mean max >2 >8 (tile y,x,n)")
        for f in frames:
            a, b = load(on, f), load(off, f)
            d = np.abs(a - b).max(axis=2)
            y, x, n = tile_of(d > 2)
            line = f"{f:3d} {np.abs(a - b).mean():.4f} {d.max():3d} {(d > 2).sum():6d} {(d > 8).sum():5d} ({y},{x},{n})"
            if settled is not None and 22 <= f <= 30:
                m = edges(settled)
                ea, eb = np.abs(a - settled).max(axis=2), np.abs(b - settled).max(axis=2)
                line += f" | vs settled: on {ea.mean():.3f}/{ea[m].mean():.3f}  off {eb.mean():.3f}/{eb[m].mean():.3f} (all/edges)"
            print(line)
            if crops and (f in (0, 23, 24, 25, 26, 27, 28, 29, frames[len(frames) // 2], frames[-1])):
                crop([a, b] + ([settled] if settled is not None else []), y, x, crops / f"{clip}-{f:03d}.png")


if __name__ == "__main__":
    main(*sys.argv[1:3])
