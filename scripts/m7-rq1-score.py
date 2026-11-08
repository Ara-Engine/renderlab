"""M7-RQ1: score matched reflection_matched renders against its path-traced reference (docs/m7-rq1.md).
CPU only. References come from `reflection_matched --reference SPP` (frame-NNN.png, with the
independent half-sample images frame-NNN-a.png / -b.png); renders from `reflection_matched --matched`.
Both use the legacy display transform (exposure, x/(1+x), sRGB), so codes compare directly.

Per region (sphere interiors at 0.9 radius from MQ1's per-frame projection; the full frame):
- `rms`: display RMS error, 8-bit codes.
- `detail` (fine): RMS of the high-pass band (image minus a Gaussian blur, sigma 1.5 px) in
  the render over the same in the reference; `detail_rms` the RMS of its difference. Above 1
  the render carries extra grain or blocky edges.
- `mid`: the same ratio for the band that carries reflected shapes 2-8 px across
  (blur sigma 1 minus blur sigma 4); `mid_rms` the RMS of its difference. Below 1 the
  render has lost reflected detail ("mushy"); `mid_rms` also counts misplaced detail.
- `bias_rms`: RMS of the blurred (sigma 4) difference: lighting and colour error.
- `ref_noise`: the reference's own split-half standard error over the region, codes RMS.

    py -3 -B scripts/m7-rq1-score.py score ROOT --ref REF --runs a,b --frames 16,32,47 --out score.json
    py -3 -B scripts/m7-rq1-score.py crops ROOT --ref REF --runs a,b --frame 47 --out DIR
    py -3 -B scripts/m7-rq1-score.py rf BEFORE_RUN AFTER_RUN --out rf-bands.json
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SIGMA = 1.5


def load_module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


mq1 = load_module("m7-mq1-score")


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float64)


def blur(image, sigma=SIGMA):
    r = int(math.ceil(3 * sigma))
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / sigma) ** 2)
    k /= k.sum()
    out = np.pad(image, ((r, r), (r, r), (0, 0)), mode="edge")
    out = sum(k[i] * out[i:i + image.shape[0], :, :] for i in range(2 * r + 1))
    out = sum(k[i] * out[:, i:i + image.shape[1], :] for i in range(2 * r + 1))
    return out


def regions(frame, shape):
    h, w = shape
    if (w, h) != (mq1.W, mq1.H):
        mq1.W, mq1.H = w, h
    gold, _, _ = mq1.sphere_mask(frame, mq1.GOLD, 0.9)
    chrome, _, _ = mq1.sphere_mask(frame, mq1.CHROME, 0.9)
    return {"gold": gold, "chrome": chrome, "frame": np.ones((h, w), bool)}


def band(image):
    """The mid band that carries reflected shapes 2-8 px across: blur(1) - blur(4)."""
    return blur(image, 1.0) - blur(image, 4.0)


def measure(render, ref, half_a, half_b, mask):
    hr, hf = render - blur(render), ref - blur(ref)
    mr, mf = band(render), band(ref)
    m = mask[..., None].repeat(3, 2)
    rms = lambda x: float(np.sqrt(np.mean(x[m] ** 2)))
    return dict(
        rms=rms(render - ref),
        detail=rms(hr) / max(rms(hf), 1e-9),
        detail_rms=rms(hr - hf),
        mid=rms(mr) / max(rms(mf), 1e-9),
        mid_rms=rms(mr - mf),
        bias_rms=rms(blur(render, 4.0) - blur(ref, 4.0)),
        ref_noise=rms((half_a - half_b) / 2),
    )


def score(args):
    root, ref = Path(args.root), Path(args.ref)
    rows = []
    for run in args.runs.split(","):
        for frame in map(int, args.frames.split(",")):
            path = root / run / f"frame-{frame:03d}.png"
            if not path.exists():
                continue
            image = load(path)
            r = load(ref / f"frame-{frame:03d}.png")
            a, b = load(ref / f"frame-{frame:03d}-a.png"), load(ref / f"frame-{frame:03d}-b.png")
            for name, mask in regions(frame, image.shape[:2]).items():
                rows.append(dict(run=run, frame=frame, region=name, **measure(image, r, a, b, mask)))
    Path(args.out).write_text(json.dumps(rows, indent=1))
    print(f"{'run':28} {'frame':>5} {'region':7} {'rms':>6} {'fine':>6} {'fRMS':>6} {'mid':>6} {'mRMS':>6} {'bias':>6} {'refN':>5}")
    for row in rows:
        print(f"{row['run']:28} {row['frame']:5d} {row['region']:7} {row['rms']:6.2f} {row['detail']:6.3f} "
              f"{row['detail_rms']:6.2f} {row['mid']:6.3f} {row['mid_rms']:6.2f} {row['bias_rms']:6.2f} {row['ref_noise']:5.2f}")


def crops(args):
    """Gold and chrome crops (3x, nearest): reference, then each run, labelled by order."""
    root, ref, out = Path(args.root), Path(args.ref), Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    frame = args.frame
    images = [load(ref / f"frame-{frame:03d}.png")] + [load(root / run / f"frame-{frame:03d}.png")
                                                       for run in args.runs.split(",")]
    h, w = images[0].shape[:2]
    mq1.W, mq1.H = w, h
    for name, sphere in (("gold", mq1.GOLD), ("chrome", mq1.CHROME)):
        _, centre, radius = mq1.sphere_mask(frame, sphere, 1.0)
        pad = int(radius * 1.25)
        x0, y0 = max(int(centre[0]) - pad, 0), max(int(centre[1]) - pad, 0)
        x1, y1 = min(int(centre[0]) + pad, w), min(int(centre[1]) + pad, h)
        tiles = [np.kron(i[y0:y1, x0:x1], np.ones((3, 3, 1))) for i in images]
        gap = np.full((tiles[0].shape[0], 6, 3), 24.0)
        row = np.concatenate([t for pair in zip(tiles, [gap] * len(tiles)) for t in pair][:-1], axis=1)
        Image.fromarray(np.clip(row, 0, 255).astype(np.uint8)).save(out / f"{name}-{frame:03d}.png")
    print("crops", out, "order: reference," + args.runs)


def rf(args):
    """Per-population (roughness band) display RMS at -14 stops, before vs after, from two
    `scripts/m7-r1-score.py` runs, plus each pose's shading-pass and frame-span medians (frames
    5-16 of R1n's timed frames)."""
    load_rows = lambda d: {(r["fixture"], r["pose"], r["config"]): r for r in
                           json.loads((Path(d) / "rf-scores.json").read_text())["rows"]}
    before, after = load_rows(args.before), load_rows(args.after)
    out, lines = [], []
    lines.append("| Fixture / pose | Path | Population | Before | After | Change |")
    lines.append("|---|---|---|---:|---:|---:|")
    for key, row in before.items():
        if "measures" not in row or key not in after:
            continue
        for name, value in row["measures"].items():
            if not (name == "reflector" or name.startswith("reflector-")) or "display_rms_-14" not in value:
                continue
            a, b = value["display_rms_-14"], after[key]["measures"][name]["display_rms_-14"]
            out.append(dict(fixture=key[0], pose=key[1], path=key[2][10:], population=name, before=a, after=b))
            lines.append(f"| {key[0]} {key[1]} | {key[2][10:]} | {name} | {a:.3f} | {b:.3f} | {b - a:+.3f} |")
    timing = []
    for label, root in (("before", args.before), ("after", args.after)):
        for times in sorted(Path(root).glob("rf-*/*.times.json")):
            t = json.loads(times.read_text())
            med = lambda v: float(np.median(v[4:]))
            timing.append(dict(run=label, path=times.parent.name, case=times.stem.split(".")[0],
                               shading_ms=med(t["shading_ms"]), span_ms=med(t["span_ms"])))
    Path(args.out).write_text(json.dumps(dict(bands=out, timing=timing), indent=1))
    print("\n".join(lines))
    print("\n| Case | Path | Shading before | after | Span before | after |\n|---|---|---:|---:|---:|---:|")
    for t in timing:
        if t["run"] != "before":
            continue
        a = next(x for x in timing if x["run"] == "after" and x["path"] == t["path"] and x["case"] == t["case"])
        print(f"| {t['case']} | {t['path']} | {t['shading_ms']:.3f} | {a['shading_ms']:.3f} | {t['span_ms']:.3f} | {a['span_ms']:.3f} |")


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("score")
    s.add_argument("root")
    s.add_argument("--ref", required=True)
    s.add_argument("--runs", required=True)
    s.add_argument("--frames", default="47")
    s.add_argument("--out", required=True)
    c = sub.add_parser("crops")
    c.add_argument("root")
    c.add_argument("--ref", required=True)
    c.add_argument("--runs", required=True)
    c.add_argument("--frame", type=int, default=47)
    c.add_argument("--out", required=True)
    r = sub.add_parser("rf")
    r.add_argument("before")
    r.add_argument("after")
    r.add_argument("--out", required=True)
    args = p.parse_args()
    {"score": score, "crops": crops, "rf": rf}[args.cmd](args)


if __name__ == "__main__":
    main()
