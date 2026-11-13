"""U-RTVIS: reflection_matched variants against the path tracer, from tools/rtvis/timing.py
`multi_matched` (CPU only).

    py -3 -B tools/rtvis/multi_compare.py RUNS_DIR CROPS_DIR VALUE...   # the first VALUE is the base

Per path and frame: each variant's mean absolute error against the path tracer at the same pose,
over the image and over the three 64x64 tiles where the variants differ from the base most (the
glossy objects whose band changes). Crops (base | variants... | path tracer, 256x256, native) of
those tiles.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def load(p):
    return np.asarray(Image.open(p).convert("RGB"), dtype=np.int16)


def tiles(score, size=64, n=3):
    h, w = score.shape
    t = score[: h // size * size, : w // size * size].reshape(h // size, size, w // size, size).sum((1, 3))
    order = np.argsort(t, axis=None)[::-1][:n]
    return [(int(i // t.shape[1]) * size, int(i % t.shape[1]) * size) for i in order]


def main(runs, crops, *values):
    runs, crops = Path(runs), Path(crops)
    crops.mkdir(parents=True, exist_ok=True)
    print("error vs path tracer: image | top tiles (y,x) per variant", values)
    for path in ("rt", "nonrt"):
        for kind, frames in (("moving", (16, 32, 47)), ("still", (47,))):
            for f in frames:
                images = [load(runs / f"m-{path}-{kind}-v{v}/frame-{f:03d}.png") for v in values]
                ref = load(runs / f"m-reference/frame-{(32 if kind == 'still' else f):03d}.png")
                change = sum(np.abs(a - images[0]).max(axis=2) for a in images[1:])
                top = tiles(change)
                err = [np.abs(a - ref).max(axis=2) for a in images]
                line = f"{path:5} {kind:6} {f:3}: " + " ".join(f"{e.mean():.3f}" for e in err)
                for k, (y, x) in enumerate(top):
                    line += f" | ({y},{x}) " + " ".join(f"{e[y:y + 64, x:x + 64].mean():5.2f}" for e in err)
                    cy = max(0, min(y - 96, ref.shape[0] - 256))
                    cx = max(0, min(x - 96, ref.shape[1] - 256))
                    strip = np.concatenate([a[cy:cy + 256, cx:cx + 256] for a in (*images, ref)], axis=1)
                    Image.fromarray(strip.clip(0, 255).astype(np.uint8)).save(crops / f"{path}-{kind}-{f:03d}-tile{k}.png")
                print(line)


if __name__ == "__main__":
    main(*sys.argv[1:])
