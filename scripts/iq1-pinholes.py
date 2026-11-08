"""IQ1 (docs/iq1-quarry-image.md): classify every quarry pinhole from `gpu_iq1_quarry`'s
diagnostic dumps, per path and pose, and write 6x crops around a sample of them.

  py -3 -B scripts/iq1-pinholes.py <dump dir> <out json> [path ...]

Per pinhole (scripts/iq1-analyse.py `pinholes`, against U0b's 4,096-spp archive): the
engine's visibility and shading/geometric N.L (`shadow`), N.V and the vertex normal's
N.L and the frame flags (`frame`), the hit record's smallest barycentric (`hits`) and
the unlit albedo (`albedo`), each against the 8-neighbour median. CPU only.
"""
import importlib.util
import json
import sys
from pathlib import Path

import numpy as np

spec = importlib.util.spec_from_file_location("iq1", Path(__file__).with_name("iq1-analyse.py"))
iq1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(iq1)


def classify(dump, path, pose, ref, covered_hits):
    std = iq1.radiance(dump / f"{path}-standard-pose{pose}.f32")
    covered = covered_hits[..., 0] != 0 if covered_hits is not None else std.sum(-1) > 0
    mask = iq1.pinholes(std, ref, covered)
    ys, xs = np.nonzero(mask)
    shadow = iq1.load(dump / f"{path}-shadow-pose{pose}.f32")
    frame = iq1.load(dump / f"{path}-frame-pose{pose}.f32")
    albedo = iq1.load(dump / f"{path}-albedo-pose{pose}.f32").mean(-1)
    vis, nsl, ngl = shadow[..., 0], shadow[..., 1] * 2 - 1, shadow[..., 2] * 2 - 1
    nsv, nvl, flags = frame[..., 0] * 2 - 1, frame[..., 1] * 2 - 1, np.rint(frame[..., 2]).astype(int)
    med = lambda x: np.median(iq1.neighbours(x), 0)
    albedo_ratio = albedo / np.maximum(med(albedo), 1e-6)
    vis_med = med(vis)
    if covered_hits is not None:
        b = covered_hits[..., 2:4].copy().view("<f4")
        bary = np.minimum(np.minimum(b[..., 0], b[..., 1]), 1 - b[..., 0] - b[..., 1])
    else:
        bary = np.full(vis.shape, np.nan)
    classes = {}
    rows = []
    for y, x in zip(ys, xs):
        f = flags[y, x]
        if not f & 1:
            c = "source-back hit"
        elif ngl[y, x] <= 0:
            c = "facet faces away from the sun"
        elif vis[y, x] < 0.5 and vis_med[y, x] >= 0.5:
            c = "shadowed, neighbours lit"
        elif vis[y, x] < 0.5:
            c = "shadowed, neighbours shadowed"
        elif nsl[y, x] <= 0:
            c = "shading normal faces away from the sun"
        elif nsv[y, x] <= 0:
            c = "shading normal faces away from the view"
        elif albedo_ratio[y, x] < 0.4:
            c = "dark albedo texel"
        else:
            c = "other"
        classes[c] = classes.get(c, 0) + 1
        rows.append(dict(x=int(x), y=int(y), cls=c, vis=float(vis[y, x]), vis_neighbours=float(vis_med[y, x]),
                         nsl=float(nsl[y, x]), ngl=float(ngl[y, x]), nsv=float(nsv[y, x]), nvl=float(nvl[y, x]),
                         flags=int(f), albedo_ratio=float(albedo_ratio[y, x]), min_bary=float(bary[y, x])))
    return dict(count=int(mask.sum()), classes=classes,
                min_bary_quantiles=[float(q) for q in np.nanquantile(bary[mask], [.1, .5, .9])] if mask.any() and covered_hits is not None else None,
                all_min_bary_median=float(np.nanmedian(bary[covered])) if covered_hits is not None else None,
                pixels=rows), mask


def main():
    dump, out = Path(sys.argv[1]), Path(sys.argv[2])
    paths = sys.argv[3:] or sorted({p.name.split("-")[0] for p in dump.glob("*-shadow-pose*.f32")})
    result = {}
    for pose in (0, 1):
        with np.load(iq1.U0B / f"quarry-p{pose}.npz") as r:
            ref = (r["L0"] + r["L1"]) / 2
        for path in paths:
            if not (dump / f"{path}-shadow-pose{pose}.f32").exists():
                continue
            hit_file = dump / f"{path}-pose{pose}-hits.u32x4"
            h = iq1.hits(hit_file) if hit_file.exists() else None
            row, _ = classify(dump, path, pose, ref, h)
            result[f"{path}-p{pose}"] = row
            print(path, pose, row["count"], row["classes"], row["min_bary_quantiles"], row["all_min_bary_median"])
    out.write_text(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
