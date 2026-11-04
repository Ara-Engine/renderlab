# Reduced-ray soft-shadow gate metrics (docs/m6-reduced-ray-gate.md). Inputs are per-pixel
# visibility images in [0,1] for one pose; populations come from the reference only.
#
#   metrics(cand, ref, sigma, depth, cover) -> dict of per-image statistics
#   motion(e_a, e_b, part_a, part_b)        -> residual-change statistics between poses
#
# ref:   reference visibility (analytic oracle, or the mean of independent 256-ray replicas)
# sigma: per-pixel standard deviation of the production 256-ray estimator
import numpy as np
from scipy import ndimage

Z_ENVELOPE = 5.0
QUANTUM = 1.0 / 256.0


def populations(ref, depth, cover, eps=1e-9):
    part = cover & (ref > eps) & (ref < 1 - eps)
    lit = cover & (ref >= 1 - eps)
    occ = cover & (ref <= eps)
    # Narrow penumbra: a lit/occluded receiver lies within 1.5 px (width <= 3 px).
    dl = ndimage.distance_transform_edt(~lit)
    do = ndimage.distance_transform_edt(~occ)
    narrow = part & (dl + do <= 3.0)
    faint = part & (ref >= 0.9)
    nb = ndimage.convolve(part.astype(np.int32), np.ones((3, 3), np.int32), mode="constant") - part
    isolated = part & (nb == 0)
    # Silhouette-adjacent: relative depth jump above 5% inside the 3x3 neighbourhood.
    dmax = ndimage.maximum_filter(np.where(cover, depth, 0), 3)
    dmin = ndimage.minimum_filter(np.where(cover, depth, np.inf), 3)
    sil = part & ((dmax - dmin) > 0.05 * np.maximum(depth, 1e-12))
    return {"partial": part, "lit": lit, "occluded": occ, "narrow": narrow, "faint": faint,
            "isolated": isolated, "silhouette": sil}


def along_contour(err, ref, part, depth, half=4):
    """Mean signed error over a (2*half+1)-pixel segment along the local iso-visibility
    contour of the reference, on the same surface. White noise shrinks by ~sqrt(9);
    bands and bias that follow the contour do not."""
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
    return acc[valid] / cnt[valid]


def metrics(cand, ref, sigma, depth, cover, q_prod=None):
    pops = populations(ref, depth, cover)
    err = cand - ref
    out = {}
    for name, m in pops.items():
        e = err[m]
        if e.size == 0:
            out[name] = {"n": 0}
            continue
        out[name] = {"n": int(e.size), "rms": float(np.sqrt((e ** 2).mean())), "mean": float(e.mean()),
                     "p99": float(np.quantile(np.abs(e), 0.99)), "max": float(np.abs(e).max())}
    part = pops["partial"]
    tau = Z_ENVELOPE * sigma
    if q_prod is not None:
        tau = np.maximum(tau, q_prod)
    tau = np.maximum(tau, QUANTUM)
    over = part & (np.abs(err) > tau)
    out["envelope_violations"] = int(over.sum())
    out["envelope_worst_excess"] = float((np.abs(err) - tau)[part].max())
    solid = pops["lit"] | pops["occluded"]
    out["solid_max"] = float(np.abs(err[solid]).max()) if solid.any() else 0.0
    out["missed_partial"] = int((part & (cand >= 1.0) & (ref <= 1 - QUANTUM)).sum())
    out["false_umbra"] = int((part & (cand <= 0.0) & (ref >= QUANTUM)).sum())
    ea = along_contour(err, ref, part, depth)
    out["along_contour"] = {"n": int(ea.size), "rms": float(np.sqrt((ea ** 2).mean())) if ea.size else 0.0,
                            "max": float(np.abs(ea).max()) if ea.size else 0.0}
    return out, err, pops


def motion(err_a, err_b, part_a, part_b):
    m = part_a & part_b
    d = (err_b - err_a)[m]
    return {"n": int(m.sum()), "rms": float(np.sqrt((d ** 2).mean())), "p99": float(np.quantile(np.abs(d), 0.99)),
            "max": float(np.abs(d).max())}
