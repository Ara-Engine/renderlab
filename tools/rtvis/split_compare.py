"""U-RTVIS: the reflection-hit stratum split (RENDERLAB_RL_SPLIT) against main and the path
tracer, from tools/rtvis/timing.py `split_matched` (CPU only).

    py -3 -B tools/rtvis/split_compare.py RUNS_DIR CROPS_DIR

Per path and frame: the split on against off (mean and largest per-channel difference, pixels more
than 2 and 8 codes apart), and each against the path tracer at the same pose: mean absolute error
over the image and over its 64x64 tile that changed most. Crops (on | off | path tracer, native
resolution) of the three most-changed tiles, and the whole frame with its difference x16.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def load(p):
    return np.asarray(Image.open(p).convert("RGB"), dtype=np.int16)


def tiles(mask, size=64, n=3):
    h, w = mask.shape
    t = mask[: h // size * size, : w // size * size].reshape(h // size, size, w // size, size).sum((1, 3))
    order = np.argsort(t, axis=None)[::-1][:n]
    return [(int(i // t.shape[1]) * size, int(i % t.shape[1]) * size, int(t.flat[i])) for i in order]


def main(runs, crops):
    runs, crops = Path(runs), Path(crops)
    crops.mkdir(parents=True, exist_ok=True)
    for path in ("rt", "nonrt"):
        for kind, frames in (("moving", (16, 32, 47)), ("still", (47,))):
            for f in frames:
                on = load(runs / f"m-{path}-{kind}-split1/frame-{f:03d}.png")
                off = load(runs / f"m-{path}-{kind}-split0/frame-{f:03d}.png")
                ref = load(runs / f"m-reference/frame-{(32 if kind == 'still' else f):03d}.png")
                d = np.abs(on - off).max(axis=2)
                top = tiles(d > 2)
                y, x, _ = top[0]
                e = lambda a, ys=slice(None), xs=slice(None): float(np.abs(a - ref).max(axis=2)[ys, xs].mean())
                ts = (slice(y, y + 64), slice(x, x + 64))
                print(f"{path:5} {kind:6} {f:3}: on-off mean {np.abs(on - off).mean():.4f} max {d.max():3d} >2 {(d > 2).sum():6d} >8 {(d > 8).sum():5d}"
                      f" | vs path tracer: on {e(on):.3f} off {e(off):.3f}; top tile ({y},{x}) on {e(on, *ts):.2f} off {e(off, *ts):.2f}")
                for k, (ty, tx, n) in enumerate(top):
                    if n == 0:
                        continue
                    cy = max(0, min(ty - 96, on.shape[0] - 256))
                    cx = max(0, min(tx - 96, on.shape[1] - 256))
                    strip = np.concatenate([a[cy:cy + 256, cx:cx + 256] for a in (on, off, ref)], axis=1)
                    Image.fromarray(strip.clip(0, 255).astype(np.uint8)).save(crops / f"{path}-{kind}-{f:03d}-tile{k}.png")
                heat = np.repeat((d[..., None] * 16).clip(0, 255), 3, axis=2)
                Image.fromarray(np.concatenate([on, heat], axis=1).astype(np.uint8)).save(crops / f"{path}-{kind}-{f:03d}-diff16.png")


if __name__ == "__main__":
    main(*sys.argv[1:3])
