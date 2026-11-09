"""P-FLAT speckle scorer (docs/p-flat.md): per-surface noise on the courtyard's flat materials.

Masks come from the scene itself, not from the images: each pixel's centre ray is cast against
the courtyard's analytic geometry (the boxes, spheres and ground of examples/game_scene.rs at
the frame's time, camera from run.json's eye/target), and labelled crate top, crate side,
ground, arch or wall. Occluders (spheres, the spinning crate, glass, bushes, the post, the
fire and smoke, the hero) and the ground decals are excluded, and each face mask is eroded
by 3 px so silhouettes and TR1's jitter never enter it.

Per class and frame it reports, in display codes (0-255) of Rec.709 luma:
  speckle  mean |Y - median3x3(Y)|: isolated grain; edges and smooth gradients score ~0;
  lap      mean |4-neighbour Laplacian of Y|;
  and, with --reference DIR (same frames, same camera), the RGB RMS against it and the
  reference's own speckle (its floor: penumbrae, gradients and split-half noise).

    py -3 -B scripts/p-flat-score.py OUT.json RUN_DIR [RUN_DIR ...] [--reference REF_DIR] [--frames 8,16]
"""
import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

CLASSES = {1: "ground", 2: "arch", 3: "wall", 4: "crate_top", 5: "crate_side"}
OCCLUDER, DECAL = 9, 8
FOV_Y = math.radians(55.0)
FPS = 30.0
CRATES = [(4.0, 0.5, 1.0, 0.1), (5.1, 0.5, 1.3, -0.25), (4.5, 1.5, 1.1, 0.4), (4.2, 0.5, 2.3, 0.0)]


def rot_y(a):
    c, s = math.cos(a), math.sin(a)
    return np.array([[c, 0, s], [0, 1, 0], [-s, 0, c]])


def rot_x(a):
    c, s = math.cos(a), math.sin(a)
    return np.array([[1, 0, 0], [0, c, -s], [0, s, c]])


def scene(t, fox, head=(3.0, 3.2, 3.0)):
    """(boxes, spheres) at time t: boxes are (centre, 3x3 linear map of the unit cube, label).
    `head` is the post lamp's bulb (run.json `lamp_head`; the post top before P-FLAT)."""
    boxes, spheres = [], []

    def box(c, size, rot=np.eye(3), label=OCCLUDER):
        boxes.append((np.array(c, float), rot @ np.diag(size), label))

    for x in (-2.2, 2.2):
        box((x, 2.0, -3.0), (0.8, 4.0, 0.8), label=2)
    box((0.0, 4.4, -3.0), (5.2, 0.8, 1.0), label=2)
    box((-5.5, 1.5, 0.0), (0.3, 3.0, 6.0), label=3)
    for x, y, z, turn in CRATES:
        box((x, y, z), (1, 1, 1), rot_y(turn), label="crate")
    box((5.4, 0.8, 3.6), (2.2, 1.6, 0.04), rot_y(0.8))  # glass
    for x, z in ((-4.0, -2.2), (3.6, -3.8), (6.2, -0.6)):
        box((x, 0.8, z), (2.0, 1.7, 2.0))  # bush cards' hull
    box((-3.8, 0.3, 2.0), (0.6, 0.6, 0.6))  # brazier
    box((-3.1, 2.3, 2.0), (2.8, 3.6, 1.6))  # fire and smoke
    box((0.0, 3.85, -3.0), (0.06, 0.5, 0.06))
    box((3.0, 1.6, 3.0), (0.16, 3.2, 0.16))
    dx, dz = head[0] - 3.0, head[2] - 3.0
    box((3.0 + dx / 2, 3.32, 3.0 + dz / 2), (0.44 + abs(dx), 0.1, 0.44 + abs(dz)))
    box((-2.5, 0.6 + 0.3 * math.sin(2 * t), 3.0), (1, 1, 1), rot_y(1.5 * t) @ rot_x(0.6 * t))
    spheres += [((1.8, 1.0, 3.2), 1.0), ((-1.2, 0.8, 5.6), 0.8), ((0.0, 3.55, -3.0), 0.14),
                (tuple(head), 0.12)]
    walk = 0.35 * t
    ground = np.array([3 * math.sin(walk), 0.0, -1 + 3 * math.cos(walk)])
    if fox:
        box(ground + (0, 0.55, 0), (1.8, 1.3, 1.8), rot_y(math.atan2(math.cos(walk), -math.sin(walk))))
    else:
        centre = ground + (0, 0.55 + 0.12 * abs(math.sin(6 * t)), 0)
        spheres += [(tuple(centre), 0.57), (tuple(centre + (0, 0.8, 0)), 0.37)]
    return boxes, spheres


DECALS = [((0.0, -0.6), 2.6, 2.6, 0.3), ((1.5, 4.6), 4.0, 2.8, -0.4)]


def labels(eye, target, t, fox, size, head=(3.0, 3.2, 3.0), points=False):
    """Per-pixel face id (object * 8 + face) and class label; with `points`, also each pixel's
    world hit point (infinite where the ray misses)."""
    w, h = size
    eye, target = np.array(eye, float), np.array(target, float)
    f = target - eye
    f /= np.linalg.norm(f)
    r = np.cross(f, (0, 1, 0))
    r /= np.linalg.norm(r)
    u = np.cross(r, f)
    th = math.tan(FOV_Y / 2)
    xs = (2 * (np.arange(w) + 0.5) / w - 1) * th * w / h
    ys = (1 - 2 * (np.arange(h) + 0.5) / h) * th
    d = f + xs[None, :, None] * r + ys[:, None, None] * u
    d = d.reshape(-1, 3)
    best = np.full(len(d), np.inf)
    face = np.zeros(len(d), np.int32)
    label = np.zeros(len(d), np.int32)
    with np.errstate(divide="ignore", invalid="ignore"):
        tg = -eye[1] / d[:, 1]
        hit = (tg > 0) & (d[:, 1] < 0)
        p = eye + tg[:, None] * d
        hit &= (np.abs(p[:, 0]) < 30) & (np.abs(p[:, 2]) < 30)
        best = np.where(hit, tg, best)
        face = np.where(hit, 1, face)
        lab = np.ones(len(d), np.int32)
        for (cx, cz), wd, dp, turn in DECALS:
            q = p[:, [0, 2]] - (cx, cz)
            c, s = math.cos(turn), math.sin(turn)
            a, b = c * q[:, 0] - s * q[:, 1], s * q[:, 0] + c * q[:, 1]
            lab = np.where((np.abs(a) < wd / 2 + 0.25) & (np.abs(b) < dp / 2 + 0.25), DECAL, lab)
        label = np.where(hit, lab, label)
        boxes, spheres = scene(t, fox, head)
        for k, (c, m, lab) in enumerate(boxes):
            inv = np.linalg.inv(m)
            o = inv @ (eye - c)
            dl = d @ inv.T
            t0 = (-0.5 - o) / dl
            t1 = (0.5 - o) / dl
            near = np.minimum(t0, t1)
            far = np.maximum(t0, t1)
            tn = np.nanmax(near, axis=1)
            tf = np.nanmin(far, axis=1)
            hit = (tn <= tf) & (tn > 0) & (tn < best)
            axis = np.nanargmax(near, axis=1)
            sign = np.sign(-dl[np.arange(len(d)), axis])
            normal = (m @ np.eye(3))  # columns: world images of local axes (unnormalised)
            n_world = normal[:, axis].T * sign[:, None]
            up = n_world[:, 1] / np.linalg.norm(n_world, axis=1) > 0.9
            if lab == "crate":
                lab_k = np.where(up, 4, 5)
            else:
                lab_k = np.full(len(d), lab)
            best = np.where(hit, tn, best)
            face = np.where(hit, 16 + k * 8 + axis * 2 + (sign > 0), face)
            label = np.where(hit, lab_k, label)
        for k, (c, rad) in enumerate(spheres):
            oc = eye - np.array(c)
            b = d @ oc
            cc = oc @ oc - rad * rad
            a = np.einsum("ij,ij->i", d, d)
            disc = b * b - a * cc
            ts = (-b - np.sqrt(np.maximum(disc, 0))) / a
            hit = (disc > 0) & (ts > 0) & (ts < best)
            best = np.where(hit, ts, best)
            face = np.where(hit, 1000 + k, face)
            label = np.where(hit, OCCLUDER, label)
    if points:
        return face.reshape(h, w), label.reshape(h, w), (eye + best[:, None] * d).reshape(h, w, 3)
    return face.reshape(h, w), label.reshape(h, w)


def eroded(face, radius=3):
    """True where every pixel within `radius` (square) sees the same face."""
    h, w = face.shape
    keep = np.ones_like(face, bool)
    pad = np.pad(face, radius, mode="edge")
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            keep &= pad[radius + dy:radius + dy + h, radius + dx:radius + dx + w] == face
    keep[:radius] = keep[-radius:] = False
    keep[:, :radius] = keep[:, -radius:] = False
    return keep


def luma(rgb):
    return rgb @ np.array([0.2126, 0.7152, 0.0722])


def median3(y):
    h, w = y.shape
    p = np.pad(y, 1, mode="edge")
    stack = np.stack([p[1 + dy:1 + dy + h, 1 + dx:1 + dx + w] for dy in (-1, 0, 1) for dx in (-1, 0, 1)])
    return np.median(stack, axis=0)


def laplacian(y):
    p = np.pad(y, 1, mode="edge")
    return 4 * y - p[:-2, 1:-1] - p[2:, 1:-1] - p[1:-1, :-2] - p[1:-1, 2:]


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), np.float64)


MASKS = {}
# --frames F,F,...: score only those frames (every-frame runs).
FRAMES = None


def masks(entry, run, size):
    key = (tuple(entry["eye"]), tuple(entry["target"]), entry["t"], run["fox"], run["head"])
    if key not in MASKS:
        face, label = labels(entry["eye"], entry["target"], entry["t"], run["fox"], size, run["head"])
        inside = eroded(face)
        MASKS[key] = {name: inside & (label == k) for k, name in CLASSES.items()}
    return MASKS[key]


def camera(t, fox):
    """The example's fly path (eye, target) at time t, for runs that predate the recorded pose."""
    orbit, ease = 0.6 + 0.25 * t, min(t / 2, 1.0)
    radius = 14.0 - 4.0 * ease
    eye = (radius * math.sin(orbit), 5.5 - 2.0 * ease, radius * math.cos(orbit))
    walk = 0.35 * t
    ground = np.array([3 * math.sin(walk), 0.0, -1 + 3 * math.cos(walk)])
    focus = ground + (0, 0.8 if fox else 0.55 + 0.12 * abs(math.sin(6 * t)), 0)
    return eye, tuple((focus + (0, 1, 0)) / 2)


def frames_of(run_dir, still):
    run = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
    fox = bool(run.get("fox"))
    out = []
    for f in run["frames"]:
        png = run_dir / f"frame-{f['frame']:03d}.png"
        if not png.exists() or (FRAMES is not None and f["frame"] not in FRAMES):
            continue
        t = f.get("t", 0.0 if still else f["frame"] / FPS)
        eye, target = (f["eye"], f["target"]) if "eye" in f else camera(t, fox)
        out.append(dict(frame=f["frame"], png=png, eye=eye, target=target, t=t))
    return dict(fox=fox, size=tuple(run["size"]), frames=out, head=tuple(run.get("lamp_head", (3.0, 3.2, 3.0))))


def score(run_dir, reference=None, still=False):
    run = frames_of(run_dir, still)
    rows = []
    for e in run["frames"]:
        m = masks(e, run, run["size"])
        rgb = load(e["png"])
        y = luma(rgb)
        spk = np.abs(y - median3(y))
        lap = np.abs(laplacian(y))
        ref = None
        if reference is not None:
            path = reference / e["png"].name
            ref = load(path) if path.exists() else None
        if ref is not None:
            yr = luma(ref)
            spk_ref = np.abs(yr - median3(yr))
            sq = ((rgb - ref) ** 2).mean(axis=2)
        for name, mask in m.items():
            n = int(mask.sum())
            if n < 200:
                continue
            row = dict(run=run_dir.name, frame=e["frame"], cls=name, pixels=n,
                       speckle=float(spk[mask].mean()), lap=float(lap[mask].mean()),
                       mean_y=float(y[mask].mean()))
            if ref is not None:
                row.update(ref_speckle=float(spk_ref[mask].mean()), rms=float(math.sqrt(sq[mask].mean())),
                           ref_mean_y=float(yr[mask].mean()))
            rows.append(row)
    return rows


def main():
    args = sys.argv[1:]
    reference = None
    if "--reference" in args:
        i = args.index("--reference")
        reference = Path(args[i + 1])
        del args[i:i + 2]
    if "--frames" in args:
        i = args.index("--frames")
        global FRAMES
        FRAMES = {int(v) for v in args[i + 1].split(",")}
        del args[i:i + 2]
    still = "--still" in args
    args = [a for a in args if a != "--still"]
    out, runs = Path(args[0]), [Path(a) for a in args[1:] if (Path(a) / "run.json").exists()]
    rows = []
    for r in runs:
        rows += score(r, reference, still or "still" in r.name)
    out.write_text(json.dumps(rows, indent=1), encoding="utf-8")
    # Compact table: per run and class, the mean over frames.
    table = {}
    for row in rows:
        table.setdefault((row["run"], row["cls"]), []).append(row)
    for (run, cls), rs in sorted(table.items()):
        line = f"{run:40s} {cls:11s} n={len(rs)} speckle={np.mean([r['speckle'] for r in rs]):6.3f} lap={np.mean([r['lap'] for r in rs]):6.3f}"
        refd = [r for r in rs if "rms" in r]
        if refd:
            line += (f" | {len(refd)} ref frames: speckle={np.mean([r['speckle'] for r in refd]):6.3f}"
                     f" ref_speckle={np.mean([r['ref_speckle'] for r in refd]):6.3f} rms={np.mean([r['rms'] for r in refd]):6.3f}")
        print(line)


if __name__ == "__main__":
    main()
