"""M7-MQ1: score and sheet game_scene flights (docs/m7-mq1.md). CPU only.

The reference is the same flight rendered with TR1 off at FACTOR x the size (`--size`,
`--no-temporal`), box-filtered in linear light to the frame size: a lighting-matched
supersampled control (FACTOR^2 pixel samples, each with its own reflection rays), not a
path-traced reference. Regions follow the example's camera path and object placements
(examples/game_scene.rs), projected per frame.

    py -3 -B scripts/m7-mq1-score.py reference RAW_DIR OUT_DIR --factor 4   # downsample, then delete RAW pngs
    py -3 -B scripts/m7-mq1-score.py score RUN_ROOT --reference REF_DIR --variants a,b --path rt --out score.json
    py -3 -B scripts/m7-mq1-score.py sheet RUN_ROOT --reference REF_DIR --variants a,b --path rt --out DIR
"""
import argparse
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

W, H, FPS, FOV = 1280, 720, 30.0, math.radians(55.0)
GOLD = ((1.8, 1.0, 3.2), 1.0)
CHROME = ((-1.2, 0.8, 5.6), 0.8)
CRATES = [(4.0, 0.5, 1.0), (5.1, 0.5, 1.3), (4.5, 1.5, 1.1), (4.2, 0.5, 2.3)]
ARCH = [(-2.6, 0.0, -3.5), (2.6, 4.8, -2.5)]
# The example's anchor: zero on its own ground, (0, 0.6, -0.15) over the quarry (--anchor quarry).
ANCHOR = np.zeros(3)


def camera(frame):
    """Eye and view-projection rows of the example's fly path at `frame` (glam conventions)."""
    t = frame / FPS
    walk = 0.35 * t
    hero = ANCHOR + np.array([3 * math.sin(walk), 0.55 + 0.12 * abs(math.sin(6 * t)), -1 + 3 * math.cos(walk)])
    orbit = 0.6 + 0.25 * t
    radius = 14.0 - 4.0 * min(t / 2.0, 1.0)
    eye = ANCHOR + np.array([radius * math.sin(orbit), 5.5 - 2.0 * min(t / 2.0, 1.0), radius * math.cos(orbit)])
    target = hero + 0.5 * (ANCHOR + np.array([0.0, 1.0, 0.0]) - hero)
    f = target - eye
    f /= np.linalg.norm(f)
    s = np.cross(f, [0.0, 1.0, 0.0])
    s /= np.linalg.norm(s)
    u = np.cross(s, f)
    return eye, (s, u, f)


def project(frame, p, w=W, h=H):
    eye, (s, u, f) = camera(frame)
    d = ANCHOR + np.asarray(p, float) - eye
    z = d @ f
    k = 1.0 / math.tan(FOV / 2)
    x = (d @ s) * k / (z * w / h)
    y = (d @ u) * k / z
    return np.array([(x + 1) * 0.5 * w, (1 - y) * 0.5 * h]), z


def sphere_mask(frame, sphere, shrink=1.0):
    (c, r) = sphere
    centre, z = project(frame, c)
    eye, _ = camera(frame)
    dist = np.linalg.norm(ANCHOR + np.asarray(c) - eye)
    pr = math.tan(math.asin(min(r / dist, 0.999))) * (H / 2) / math.tan(FOV / 2) * shrink
    yy, xx = np.mgrid[0:H, 0:W]
    return (xx + 0.5 - centre[0]) ** 2 + (yy + 0.5 - centre[1]) ** 2 <= pr * pr, centre, pr


def bbox(frame, corners, pad=6):
    pts = np.array([project(frame, c)[0] for c in corners])
    lo = np.floor(pts.min(0)).astype(int) - pad
    hi = np.ceil(pts.max(0)).astype(int) + pad
    return max(lo[0], 0), max(lo[1], 0), min(hi[0], W), min(hi[1], H)


def crate_corners():
    out = []
    for (x, y, z) in CRATES:
        for dx in (-0.75, 0.75):
            for dy in (-0.5, 0.5):
                for dz in (-0.75, 0.75):
                    out.append((x + dx, y + dy, z + dz))
    return out


def arch_corners():
    (a, b) = ARCH
    return [(x, y, z) for x in (a[0], b[0]) for y in (a[1], b[1]) for z in (a[2], b[2])]


def regions(frame):
    """Name -> boolean mask at frame size."""
    out = {}
    out["gold"], _, _ = sphere_mask(frame, GOLD, 0.9)
    out["chrome"], _, _ = sphere_mask(frame, CHROME, 0.9)
    for name, corners in (("crates", crate_corners()), ("arch", arch_corners())):
        m = np.zeros((H, W), bool)
        x0, y0, x1, y1 = bbox(frame, corners)
        m[y0:y1, x0:x1] = True
        out[name] = m
    return out


def srgb_to_linear(c):
    c = c / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(c, 0, 1)
    return 255.0 * np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float64)


def frames_of(directory):
    return sorted(int(p.stem.split("-")[1]) for p in Path(directory).glob("frame-*.png"))


def reference(args):
    raw, out = Path(args.raw), Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    k = args.factor
    for f in frames_of(raw):
        lin = srgb_to_linear(load(raw / f"frame-{f:03d}.png"))
        h, w, _ = lin.shape
        small = lin.reshape(h // k, k, w // k, k, 3).mean((1, 3))
        Image.fromarray(np.round(linear_to_srgb(small)).astype(np.uint8)).save(out / f"frame-{f:03d}.png")
        if args.delete_raw:
            (raw / f"frame-{f:03d}.png").unlink()
    print("reference frames", len(frames_of(out)))


def edges(ref):
    """Pixels whose 3x3 display range exceeds 24 codes: silhouettes and hard edges."""
    lum = ref.mean(2)
    pad = np.pad(lum, 1, mode="edge")
    stack = np.stack([pad[1 + dy:1 + dy + H, 1 + dx:1 + dx + W] for dy in (-1, 0, 1) for dx in (-1, 0, 1)])
    return stack.max(0) - stack.min(0) > 24


def rms(a, b, m):
    return float(np.sqrt(((a - b)[m] ** 2).mean())) if m.any() else None


def score(args):
    root, ref = Path(args.root), Path(args.reference)
    variants = args.variants.split(",")
    frames = [f for f in frames_of(ref) if all((root / f"{v}-{args.path}" / f"frame-{f:03d}.png").exists() for v in variants)]
    rows = {v: [] for v in variants}
    for f in frames:
        r = load(ref / f"frame-{f:03d}.png")
        masks = regions(f)
        masks["edges"] = edges(r)
        masks["full"] = np.ones((H, W), bool)
        prev_ref = ref / f"frame-{f - 1:03d}.png"
        for v in variants:
            img = load(root / f"{v}-{args.path}" / f"frame-{f:03d}.png")
            row = dict(frame=f, **{k: rms(img, r, m) for k, m in masks.items()})
            prev = root / f"{v}-{args.path}" / f"frame-{f - 1:03d}.png"
            if prev_ref.exists() and prev.exists():
                # Motion-compensated flicker: the frame-to-frame change against the reference's.
                dv = img - load(prev)
                dr = r - load(prev_ref)
                for k in ("full", "gold", "chrome", "crates", "arch", "edges"):
                    row["flicker_" + k] = rms(dv, dr, masks[k])
            rows[v].append(row)
    summary = {}
    for v in variants:
        keys = [k for k in rows[v][0] if k != "frame"] if rows[v] else []
        summary[v] = {k: float(np.mean([r[k] for r in rows[v] if r.get(k) is not None]))
                      for k in keys if any(r.get(k) is not None for r in rows[v])}
    result = dict(path=args.path, reference=str(ref), frames=frames, summary=summary, rows=rows)
    Path(args.out).write_text(json.dumps(result, indent=1))
    keys = ["full", "edges", "gold", "chrome", "crates", "arch", "flicker_full", "flicker_gold", "flicker_chrome", "flicker_crates", "flicker_edges"]
    print(f"{args.path} frames={len(frames)} (display RMS codes vs {ref.name}; lower is better)")
    print("variant".ljust(14) + "".join(k.replace("flicker_", "fl_").rjust(11) for k in keys))
    for v in variants:
        print(v.ljust(14) + "".join((f"{summary[v][k]:.2f}" if k in summary[v] else "-").rjust(11) for k in keys))


def crop_box(frame, name):
    if name in ("gold", "chrome"):
        _, c, pr = sphere_mask(frame, GOLD if name == "gold" else CHROME, 1.0)
        r = int(pr * 1.15) + 4
        return int(c[0]) - r, int(c[1]) - r, int(c[0]) + r, int(c[1]) + r
    return bbox(frame, crate_corners() if name == "crates" else arch_corners(), 10)


def tile(img, box, size):
    x0, y0, x1, y1 = box
    canvas = Image.new("RGB", (x1 - x0, y1 - y0), (255, 0, 255))
    canvas.paste(img.crop((max(x0, 0), max(y0, 0), min(x1, W), min(y1, H))), (max(-x0, 0), max(-y0, 0)))
    scale = min(size / canvas.width, size / canvas.height)
    return canvas.resize((max(1, int(canvas.width * scale)), max(1, int(canvas.height * scale))), Image.NEAREST)


def sheet(args):
    """Per inspected frame: rows = reference and variants, columns = gold, chrome, crates, arch."""
    root, out = Path(args.root), Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    variants = args.variants.split(",")
    sources = ([("reference", Path(args.reference))] if args.reference else []) + [(v, root / f"{v}-{args.path}") for v in variants]
    frames = [int(f) for f in args.frames.split(",")]
    size = args.tile
    for f in frames:
        rows = []
        for name, d in sources:
            p = d / f"frame-{f:03d}.png"
            if not p.exists():
                continue
            img = Image.open(p).convert("RGB")
            tiles = [tile(img, crop_box(f, n), size) for n in ("gold", "chrome", "crates", "arch")]
            rows.append((name, tiles))
        sheet_img = Image.new("RGB", (4 * (size + 4) + 4, len(rows) * (size + 20) + 4), (24, 24, 24))
        from PIL import ImageDraw
        draw = ImageDraw.Draw(sheet_img)
        for i, (name, tiles) in enumerate(rows):
            y = 4 + i * (size + 20)
            draw.text((6, y), f"{name}  frame {f}", fill=(240, 240, 240))
            for j, t in enumerate(tiles):
                sheet_img.paste(t, (4 + j * (size + 4), y + 14))
        sheet_img.save(out / f"sheet-{args.path}-{f:03d}.png")
    # A strip of consecutive frames of one region (motion inspection): rows = sources.
    if args.strip:
        name, first, count = args.strip.split(":")
        first, count = int(first), int(count)
        small = args.tile // 2
        img = Image.new("RGB", (count * (small + 2) + 2, len(sources) * (small + 14) + 2), (24, 24, 24))
        from PIL import ImageDraw
        draw = ImageDraw.Draw(img)
        for i, (label, d) in enumerate(sources):
            draw.text((4, 2 + i * (small + 14)), label, fill=(240, 240, 240))
            for j in range(count):
                p = d / f"frame-{first + j:03d}.png"
                if p.exists():
                    img.paste(tile(Image.open(p).convert("RGB"), crop_box(first + j, name), small), (2 + j * (small + 2), 14 + i * (small + 14)))
        img.save(out / f"strip-{args.path}-{name}-{first:03d}.png")
    if args.gif:
        # Side-by-side flight (variants left to right) at half size, for viewing motion.
        frames_all = frames_of(sources[-1][1])
        seq = []
        for f in frames_all:
            parts = [Image.open(d / f"frame-{f:03d}.png").convert("RGB").resize((W // 2, H // 2), Image.BILINEAR)
                     for _, d in sources if (d / f"frame-{f:03d}.png").exists()]
            canvas = Image.new("RGB", (len(parts) * W // 2, H // 2))
            for j, p in enumerate(parts):
                canvas.paste(p, (j * W // 2, 0))
            seq.append(canvas)
        seq[0].save(out / f"flight-{args.path}.gif", save_all=True, append_images=seq[1:], duration=33, loop=0)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("reference")
    r.add_argument("raw")
    r.add_argument("out")
    r.add_argument("--factor", type=int, default=4)
    r.add_argument("--delete-raw", action="store_true")
    r.set_defaults(run=reference)
    for name, fn in (("score", score), ("sheet", sheet)):
        s = sub.add_parser(name)
        s.add_argument("root")
        s.add_argument("--reference")
        s.add_argument("--variants", required=True)
        s.add_argument("--path", required=True)
        s.add_argument("--out", required=True)
        s.add_argument("--frames", default="8,16,24,32,40,47")
        s.add_argument("--tile", type=int, default=260)
        s.add_argument("--strip", help="REGION:FIRST:COUNT")
        s.add_argument("--gif", action="store_true")
        s.add_argument("--anchor", choices=("scene", "quarry"), default="scene")
        s.set_defaults(run=fn)
    args = p.parse_args()
    if getattr(args, "anchor", "scene") == "quarry":
        ANCHOR[:] = (0.0, 0.6, -0.15)
    args.run(args)


if __name__ == "__main__":
    main()
