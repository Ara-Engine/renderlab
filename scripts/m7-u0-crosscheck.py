"""M7-U0b: independent check of the reference path tracer against Mitsuba 3 (BSD-3, external tool).

  mitsuba  run inside the isolated Mitsuba venv (python -I): render a generated mitsuba/scene.xml
           (scripts/m7-u0-scenes.py) with the scalar_rgb CPU variant, save float32 RGB .npy.
  compare  system Python: our split-half reference against two independent Mitsuba seeds.

No Mitsuba code is copied; only its binaries render. The scenes, cameras and BSDFs are written
from the same scene.json the tracer reads. Registered tolerance (fixed before comparing):
  - global bias |sum(ours) - sum(mitsuba)| / sum(mitsuba) < 0.5 %, and within 4 combined sigma;
  - over 8x8-pixel blocks with signal, the normalised difference (ours - mitsuba) / sigma has
    RMS < 1.5 and fewer than 0.5 % of blocks beyond 4 sigma.
Pixels within 1 px of a geometric edge are kept: both renderers box-filter the same footprint.
Block sigma has a floor of 1e-5 x the block mean: noise-free blocks differ only by f32 rounding.
"""
import argparse
import json
from pathlib import Path
import time


def mitsuba(args):
    import mitsuba as mi
    import numpy as np
    mi.set_variant("scalar_rgb")
    import drjit as dr
    dr.set_thread_count(2)
    start = time.perf_counter()
    scene = mi.load_file(str(args.scene / "mitsuba" / "scene.xml"))
    image = np.array(mi.render(scene, spp=args.spp, seed=args.seed), dtype=np.float32)[..., :3]
    np.save(args.out, image)
    meta = dict(spp=args.spp, seed=args.seed, seconds=time.perf_counter() - start, mitsuba=mi.__version__,
                variant="scalar_rgb", shape=list(image.shape))
    args.out.with_suffix(".json").write_text(json.dumps(meta, indent=1))
    print(json.dumps(meta))


def blocks(a, size=8):
    h, w = a.shape[0] // size * size, a.shape[1] // size * size
    return a[:h, :w].reshape(h // size, size, w // size, size, -1).mean(axis=(1, 3))


def compare(args):
    import numpy as np
    meta = json.loads((args.ours / f"{args.name}.json").read_text())
    w, h = meta["width"], meta["height"]
    ours = np.fromfile(args.ours / f"{args.name}-mean.f32", "<f4").reshape(h, w, 3).astype(np.float64)
    ours_se = np.fromfile(args.ours / f"{args.name}-se.f32", "<f4").reshape(h, w, 3).astype(np.float64)
    a, b = (np.load(p).astype(np.float64) for p in args.mitsuba)
    mits = (a + b) / 2
    mits_se_pixel = np.abs(a - b) / 2  # split-half standard error of the two-seed mean
    # Block statistics: means of 64 pixels; their errors from the per-pixel split halves.
    ob, mb = blocks(ours), blocks(mits)
    sigma = np.sqrt(blocks(ours_se ** 2) / 64 + blocks(mits_se_pixel ** 2) / 64)
    # Noise-free blocks (constant sky seen directly) still differ by f32 accumulation rounding.
    sigma = np.maximum(sigma, 1e-5 * np.abs(mb))
    signal = mb > 1e-3 * mb.max()
    z = ((ob - mb) / np.maximum(sigma, 1e-30))[signal]
    total_o, total_m = ours.sum(), mits.sum()
    # RGB shares a path. Sum channel error bounds within each pixel before pooling
    # independent pixels; the old formula understated grayscale uncertainty by sqrt(3).
    total_sigma = np.sqrt((ours_se.sum(-1) ** 2).sum() + (mits_se_pixel.sum(-1) ** 2).sum())
    report = dict(
        case=args.name, ours_samples=meta["samples"], mitsuba=[json.loads(Path(p).with_suffix(".json").read_text()) for p in args.mitsuba],
        global_relative_bias=float((total_o - total_m) / total_m), global_bias_sigma=float((total_o - total_m) / total_sigma),
        relative_l1=float(np.abs(ours - mits).sum() / np.abs(mits).sum()),
        mitsuba_seed_relative_l1=float(np.abs(a - b).sum() / np.abs(mits).sum()),
        block_z_rms=float(np.sqrt((z ** 2).mean())), block_beyond_4_sigma=float((np.abs(z) > 4).mean()), blocks=int(z.size),
        per_channel_relative_bias=((ours.sum(axis=(0, 1)) - mits.sum(axis=(0, 1))) / mits.sum(axis=(0, 1))).tolist())
    report["pass"] = bool(abs(report["global_relative_bias"]) < .005 and abs(report["global_bias_sigma"]) < 4
                          and report["block_z_rms"] < 1.5 and report["block_beyond_4_sigma"] < .005)
    out = args.ours / f"{args.name}-crosscheck.json"
    out.write_text(json.dumps(report, indent=1))
    # Side-by-side display image for inspection: ours | mitsuba | 8x |difference|.
    from PIL import Image
    exposure = 2.0 ** args.stops
    tone = lambda x: np.clip(((x * exposure) / (1 + x * exposure)) ** (1 / 2.2) * 255, 0, 255).astype(np.uint8)
    sheet = np.concatenate([tone(ours), tone(mits), tone(8 * np.abs(ours - mits))], axis=1)
    Image.fromarray(sheet).save(args.ours / f"{args.name}-crosscheck.png")
    print(json.dumps(report, indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("mitsuba")
    p.add_argument("scene", type=Path)
    p.add_argument("out", type=Path)
    p.add_argument("--spp", type=int, default=2048)
    p.add_argument("--seed", type=int, default=0)
    p = sub.add_parser("compare")
    p.add_argument("ours", type=Path, help="directory with <name>-mean.f32, -se.f32 and .json")
    p.add_argument("name")
    p.add_argument("mitsuba", type=Path, nargs=2)
    p.add_argument("--stops", type=float, default=-9)
    args = parser.parse_args()
    dict(mitsuba=mitsuba, compare=compare)[args.command](args)


if __name__ == "__main__":
    main()
