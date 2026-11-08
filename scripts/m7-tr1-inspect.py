"""M7-TR1: inspection sheets for reconstruction outputs on the physical Q-walk target.

  py -3 -B scripts/m7-tr1-inspect.py <out-dir> --frame 64 --candidate name=<dir> [...]
         [--crop x0,y0,x1,y1] [--stops 0 4]

Writes, per frame and exposure, the reference and each candidate as display images (the
frozen F0 transform), and each candidate's absolute error on one fixed 0-64-code scale
(saturated above). Full frames are downscaled 2x; `--crop` adds native-resolution crops.
"""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image

REFERENCE = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
RECIPE = Path("E:/renderlab-work/m7-tr0/runs/w-e-r-c1/recipe.json")
W, H = 1920, 1080


def codes(hdr, stops):
    x = np.maximum(0, hdr * (2 ** stops)); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - .055)


def reference(frame):
    recipe = json.loads(RECIPE.read_text())
    key = np.asarray(recipe["frames"][frame]["eye"] + recipe["frames"][frame]["target"], dtype="<f4").tobytes()
    for path in list(REFERENCE.glob("q-walk-*.json")) + list(REFERENCE.glob("quarry-p*.json")):
        meta = json.loads(path.read_text())
        f = meta.get("frame", {})
        if f and f.get("open") == f.get("close") and not f.get("sky_only") and path.with_suffix(".npz").exists():
            if np.asarray(f["open"]["eye"] + f["open"]["target"], dtype="<f4").tobytes() == key:
                with np.load(path.with_suffix(".npz")) as z:
                    return (z["L0"] + z["L1"]) * .5 * 2 ** -14
    raise SystemExit(f"no reference for frame {frame}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out")
    parser.add_argument("--frame", type=int, action="append", required=True)
    parser.add_argument("--candidate", action="append", required=True, help="name=directory of fNNN.rgba16f")
    parser.add_argument("--crop", action="append", default=[])
    parser.add_argument("--stops", type=int, nargs="+", default=[0, 4])
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    candidates = [c.split("=", 1) for c in args.candidate]
    for frame in args.frame:
        ref = reference(frame)
        images = {}
        for name, folder in candidates:
            raw = np.fromfile(Path(folder) / f"f{frame:03}.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3]
            images[name] = raw.astype("f4")
        for stops in args.stops:
            r = codes(ref, stops)
            tiles = [("reference", r)]
            for name, image in images.items():
                c = codes(image, stops)
                tiles.append((name, c))
                tiles.append((name + "-error", np.clip(np.abs(c - r).mean(-1, keepdims=True) * 4, 0, 255).repeat(3, -1)))
            for label, tile in tiles:
                Image.fromarray(tile[::2, ::2].round().astype("u1")).save(out / f"f{frame:03}-L{stops}-{label}.png")
            for crop in args.crop:
                x0, y0, x1, y1 = map(int, crop.split(","))
                strip = np.concatenate([t[y0:y1, x0:x1] for _, t in tiles], axis=1)
                Image.fromarray(strip.round().astype("u1")).save(out / f"f{frame:03}-L{stops}-crop-{x0}-{y0}.png")
        print("frame", frame, "written; tiles:", ", ".join(t for t, _ in tiles))


if __name__ == "__main__":
    main()
