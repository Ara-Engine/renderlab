"""T1 follow-up: within-prefix order at N=2-3, per-population second-sample ratios, and the
linear-HDR crossing of centre-first sequences.

Reads R1 viewed pose 0 in place (Exact-ray samples); writes prefix-check.json under the T1 evidence root.
py -3 -B scripts/m6-temporal-prefix.py
"""
import json
from pathlib import Path

import numpy as np

R = Path("D:/Coding/renderlab-work/m6-exact-quality/r1/quarry-viewed-p0")
H, W = 1080, 1920
DT = np.dtype([("counts", "<u4", (4,)), ("moments", "<f4", (4,)), ("radiance", "<f4", (4,))])


def codes(x, s):
    x = np.maximum(0, x * 2 ** s); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - .055)


film = np.fromfile(R / "pose-0.film.bin", dtype=DT).reshape(H, W)["radiance"][..., :3].astype(np.float64)
direct = np.fromfile(R / "pose-0.direct.bin", dtype=DT).reshape(H, W)["radiance"][..., :3].astype(np.float64)
cover = np.fromfile(R / "pose-0.hit.u32", dtype="<u4").reshape(H, W, 4)[..., 0] != 0
ys, xs = np.nonzero(cover); crop = (slice(ys.min() - 8, ys.max() + 9), slice(xs.min() - 8, xs.max() + 9))
L = np.stack([np.fromfile(R / f"samples/{i:02}.rgba16f", dtype="<f2").reshape(H, W, 4)[crop][..., :3].astype(np.float64) for i in range(16)])
D = np.stack([np.fromfile(R / f"samples/{i:02}.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[crop][..., :3].astype(np.float64) for i in range(16)])
compL = (film[crop] - L.sum(0)) / 240; compD = (direct[crop] - D.sum(0)) / 240
cov = cover[crop]; n_img = H * W
exL = np.fromfile(R / "pose-0.exact.rgba16f", dtype="<f2").reshape(H, W, 4)[crop][..., :3].astype(np.float64)
exD = np.fromfile(R / "pose-0.exact.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[crop][..., :3].astype(np.float64)


def score(l, d):
    return dict(L0_image=float(np.sqrt(np.mean((codes(l, 0) - codes(compL, 0)) ** 2, -1).sum() / n_img)),
                L0_covered=float(np.sqrt(np.mean((codes(l, 0) - codes(compL, 0)) ** 2, -1)[cov].mean())),
                D4_covered=float(np.sqrt(np.mean((codes(d, 4) - codes(compD, 4)) ** 2, -1)[cov].mean())),
                lin_image=float(np.sqrt(np.sum((l - compL) ** 2, -1).sum() / (3 * n_img))),
                lin_covered=float(np.sqrt(np.sum((l - compL) ** 2, -1)[cov].sum() / (3 * cov.sum()))))


out = dict(exact=score(exL, exD), centre=score(L[8], D[8]), orders={})
prog = [int(f"{k:04b}"[::-1], 2) for k in range(16)]
orders = {"centre-progressive (8,0,12,4,..)": [(p + 8) % 16 for p in prog],
          "centre then quarter pair (8,12,4,0,..)": [8, 12, 4, 0] + [(p + 8) % 16 for p in prog][4:],
          "centre then quarter pair (8,4,12,0,..)": [8, 4, 12, 0] + [(p + 8) % 16 for p in prog][4:]}
for name, order in orders.items():
    acc_l = np.zeros_like(compL); acc_d = np.zeros_like(compD); curve = []
    for k, i in enumerate(order):
        acc_l += L[i]; acc_d += D[i]
        curve.append(score(acc_l / (k + 1), acc_d / (k + 1)))
    out["orders"][name] = dict(curve=curve[:8] + [curve[15]], first_linear_image_below_exact=next((k + 1 for k, c in enumerate(curve) if c["lin_image"] <= out["exact"]["lin_image"]), None),
                               first_linear_covered_below_exact=next((k + 1 for k, c in enumerate(curve) if c["lin_covered"] <= out["exact"]["lin_covered"]), None))


def second_sample_ratios():
    """Per population, both poses: RMS error of the centre plus one more sample over the centre alone."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("film_score", Path(__file__).with_name("m6-film-score.py"))
    f0 = importlib.util.module_from_spec(spec); spec.loader.exec_module(f0)
    ratios = {}
    for pose in (0, 1):
        d = Path(f"D:/Coding/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{pose}")
        ref = f0.decode(d, pose); point = np.load(f0.FROZEN / f"gate-final/reference/pose-{pose}-reference.npz")["ref"]
        pops = f0.populations(ref, point)
        pops.update({"backlit": ref["cover"] & ~ref["front"], "all-covered": ref["cover"], "multi-surface": ref["cover"] & (ref["n"] < 256), "background-centre": ~ref["cover"]})
        full = {"L": ref["hdr"][..., :3] * 256, "D": np.fromfile(d / f"pose-{pose}.direct.bin", dtype=DT).reshape(H, W)["radiance"][..., :3].astype(np.float64)}
        smp = {c: {i: np.fromfile(d / f"samples/{i:02}.{'' if c == 'L' else 'sun.'}rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float64) for i in range(16)} for c in ("L", "D")}
        comp = {c: (full[c] - sum(smp[c].values())) / 240 for c in ("L", "D")}
        for second in (0, 12, 4):
            for ch in ("L0", "L4", "D0", "D4"):
                c, s = ch[0], int(ch[1])
                e1 = np.mean((codes(smp[c][8], s) - codes(comp[c], s)) ** 2, -1)
                e2 = np.mean((codes((smp[c][8] + smp[c][second]) / 2, s) - codes(comp[c], s)) ** 2, -1)
                worst = max((float(np.sqrt(e2[m].sum() / e1[m].sum())), k) for k, m in pops.items() if m.any() and e1[m].sum() > 0)
                ratios.setdefault(f"8,{second}/{ch}", []).append(worst)
    return ratios


out["second_sample_worst_ratio"] = second_sample_ratios()
for key, value in out["second_sample_worst_ratio"].items():
    print("N2/N1 worst", key, value)
Path("D:/Coding/renderlab-work/m6-exact-quality/t1/prefix-check.json").write_text(json.dumps(out, indent=1))
for name, o in out["orders"].items():
    print(name, "linear first image/covered:", o["first_linear_image_below_exact"], o["first_linear_covered_below_exact"])
    for k, c in enumerate(o["curve"][:4]):
        print("  N", k + 1, {a: round(b, 4) for a, b in c.items()})
print("exact", out["exact"], "\ncentre", out["centre"])
