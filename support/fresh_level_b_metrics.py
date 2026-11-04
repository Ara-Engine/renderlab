"""Shared Level-B loading, populations and statistics (docs/m6-level-b.md).

CPU only. Inputs are the immutable quarry reference R (seeds 1-64, 16,384 rays), the saved
production counts and hit masks, and the decoded Unreal native masks. Nothing is written
outside the m6-level-b evidence directory.

  load_pose(p)              -> dict(ref, sigma, depth, cover, world, ngeo, front)
  populations(d)            -> dict of boolean masks, all defined from the reference only
  stats(err, pops, cand, d) -> dict of per-population statistics and defect counts
  contour_bias(err, d, pop) -> along-contour 9-px segment means (G4 machinery)
"""
import sys
from pathlib import Path
import numpy as np
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fresh_gate_metrics as gate  # sha256 0ffacfc6...7322, unchanged

R = Path()  # Configured by fresh_level_b.py; immutable traced production counts.
NATIVE = Path()
REFERENCE = Path()  # Cached mean/spread of the immutable traced counts.
SHAPE = (1080, 1920)
EYE = {0: (5.2, 3.4, 6.2), 1: (5.21, 3.4, 6.2), 2: (5.2, 3.4, 6.2)}
TO_LIGHT = np.array([0.4, 0.8, 0.3]) / np.linalg.norm([0.4, 0.8, 0.3])

# Native configurations: production-native (vsm, hrt1) and diagnostics.
CASES = {
    "vsm": "native-vsm-baseline", "hrt1": "native-hrt-denoised", "hrt256": "native-hrt-256",
    "vsm-screen0": "native-vsm-screen-zero", "vsm-normal0": "native-vsm-normal-zero",
}


def native_dir(case, pose):
    stem = CASES[case]
    suffix = "02" if pose == 0 and case in ("vsm", "hrt1", "hrt256") else "01"
    return NATIVE / f"{stem}-p{pose}-{suffix}" / "decoded-v2"


def counts(pose, seed, kind="production"):
    return np.fromfile(R / f"pose-{pose}-seed-{seed}-{kind}.u32", dtype="<u4").reshape(SHAPE)


def hits(pose, seed):
    """Per-receiver 256-bit production hit masks; bit i (little-endian) is direction i, 1 = blocked."""
    h = np.fromfile(R / f"pose-{pose}-seed-{seed}-hits.u32", dtype="<u4").reshape(*SHAPE, 8)
    return np.unpackbits(h.view("u1").reshape(*SHAPE, 32), axis=-1, bitorder="little")


def load_pose(pose):
    ref = np.load(REFERENCE / f"pose-{pose}-reference.npz")
    world = np.fromfile(R / f"pose-{pose}-world.f32", dtype="<f4").reshape(*SHAPE, 4)
    normal = np.fromfile(R / f"pose-{pose}-normal.f32", dtype="<f4").reshape(*SHAPE, 4)
    n = normal[..., :3].astype(float)
    view = np.asarray(EYE[pose]) - world[..., :3]
    # Saved derivative normals have arbitrary sign; orient them toward the camera.
    n = np.where((np.einsum("ijk,ijk->ij", n, view) < 0)[..., None], -n, n)
    n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-12)
    ndl = n @ TO_LIGHT
    cover = ref["cover"]
    return dict(ref=ref["ref"], sigma=ref["sigma"], depth=ref["depth"], cover=cover,
                world=world[..., :3], ngeo=n, ndl=ndl, front=cover & (ndl > 0))


def populations(d):
    ref, depth, cover = d["ref"], d["depth"], d["cover"]
    p = gate.populations(ref, depth, cover)
    lit, occ, part = p["lit"], p["occluded"], p["partial"]
    dl = ndimage.distance_transform_edt(~lit)
    do = ndimage.distance_transform_edt(~occ)
    p["wide"] = part & (dl + do > 3.0)
    p["wide6"] = part & (dl + do >= 6.0)
    # Contact band: dark receivers (V <= 0.5) within 3 px of a fully lit receiver, away from
    # depth silhouettes. A dark receiver next to a lit one has a <= 3 px transition, i.e. a
    # blocker within roughly 0.3 m at this view: the receivers VSM's offsets erase.
    dmax = ndimage.maximum_filter(np.where(cover, depth, 0), 5)
    dmin = ndimage.minimum_filter(np.where(cover, depth, np.inf), 5)
    smooth = cover & ((dmax - dmin) <= 0.05 * np.maximum(depth, 1e-12))
    p["contact"] = cover & (ref <= 0.5) & (dl <= 3.0) & smooth
    p["solid"] = lit | occ
    return p


def contour_segments(err, d, part, half=4):
    """G4 machinery: mean signed error along 9-px iso-contour segments on the same surface.
    Returns (segment means, centre index mask)."""
    ref, depth = d["ref"], d["depth"]
    gy = ndimage.sobel(ref, 0); gx = ndimage.sobel(ref, 1)
    g = np.hypot(gx, gy)
    tx = np.where(g > 0, -gy / np.maximum(g, 1e-12), 1.0)
    ty = np.where(g > 0, gx / np.maximum(g, 1e-12), 0.0)
    ys, xs = np.nonzero(part)
    acc = np.zeros(ys.size); cnt = np.zeros(ys.size)
    h, w = ref.shape
    for s in range(-half, half + 1):
        yy = np.rint(ys + s * ty[ys, xs]).astype(int); xx = np.rint(xs + s * tx[ys, xs]).astype(int)
        ok = (yy >= 0) & (yy < h) & (xx >= 0) & (xx < w)
        yy = np.clip(yy, 0, h - 1); xx = np.clip(xx, 0, w - 1)
        ok &= part[yy, xx] & (np.abs(depth[yy, xx] - depth[ys, xs]) <= 0.01 * depth[ys, xs])
        acc += np.where(ok, err[yy, xx], 0.0); cnt += ok
    valid = cnt >= 5
    centre = np.zeros(part.shape, bool); centre[ys[valid], xs[valid]] = True
    return acc[valid] / cnt[valid], centre


def summary(e):
    if e.size == 0:
        return {"n": 0}
    a = np.abs(e)
    return {"n": int(e.size), "rms": float(np.sqrt((e ** 2).mean())), "mean": float(e.mean()),
            "p99": float(np.quantile(a, 0.99)), "max": float(a.max())}


def stats(cand, d, pops, valid, domains=("all", "front")):
    """Per-population statistics of cand - ref on receivers valid for cand.
    Domain 'front' keeps receivers whose camera-facing geometric normal faces the light."""
    err = cand - d["ref"]
    out = {}
    for dom in domains:
        keep = valid & (d["front"] if dom == "front" else d["cover"])
        o = {}
        for name in ("partial", "narrow", "wide", "wide6", "faint", "isolated", "silhouette",
                     "lit", "occluded", "contact"):
            o[name] = summary(err[pops[name] & keep])
        lit, occ, part = pops["lit"] & keep, pops["occluded"] & keep, pops["partial"] & keep
        o["flips"] = {
            "lit_dark_gt_0.5": int((lit & (err < -0.5)).sum()),
            "lit_dark_gt_0.1": int((lit & (err < -0.1)).sum()),
            "lit_dark_gt_0.02": int((lit & (err < -0.02)).sum()),
            "umbra_lit_gt_0.5": int((occ & (err > 0.5)).sum()),
            "umbra_lit_gt_0.1": int((occ & (err > 0.1)).sum()),
            "contact_lost_gt_0.5": int((pops["contact"] & keep & (err > 0.5)).sum()),
        }
        faint = pops["faint"] & keep
        o["faint_shown_lit"] = int((faint & (cand >= 1 - 1 / 256)).sum())
        o["partial_shown_uniform"] = int((part & ((cand <= 1 / 256) | (cand >= 1 - 1 / 256))).sum())
        seg, _ = contour_segments(np.where(keep, err, 0.0), d, part)
        o["contour"] = {"n": int(seg.size), "rms": float(np.sqrt((seg ** 2).mean())) if seg.size else 0.0,
                        "p99": float(np.quantile(np.abs(seg), 0.99)) if seg.size else 0.0,
                        "max": float(np.abs(seg).max()) if seg.size else 0.0}
        out[dom] = o
    return out, err
