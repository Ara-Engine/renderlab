"""Resampling blur at solid pixels next to a hard edge (docs/m6-motion-aa.md section 1). CPU, analytic.

1D, noise-free expectation: a step edge fixed on the surface, a pixel grid moving by f px per frame,
and history h_t = (1 - a) * fetch(h_{t-1}) + a * film_t, where film_t is the pixel's box integral
(the expected value of any unbiased in-pixel sample). fetch is bilinear or Catmull-Rom (clamped
at zero) at the reprojected pixel centre. 'history2x' keeps the history at half-pixel cells, as
Unreal TSR does at r.TSR.History.ScreenPercentage = 200, and shows the mean of a pixel's two cells.
Reports the steady-state maximum |h - film| on solid pixels (film exactly 0 or 1) by the gap
between the footprint and the edge. Noise only adds to these values.

py -3 -B scripts/m6-motion-aa-blur.py --out E:/renderlab-work/motion-aa/blur.json
"""
import argparse
import json
from pathlib import Path

import numpy as np


def cr(t):
    t2 = t * t
    return 0.5 * np.array([t * (t * (2 - t) - 1), t2 * (3 * t - 5) + 2, t * (t * (-3 * t + 4) + 1), t2 * (t - 1)])


def film(n, phase, cell):
    """Box integrals of the step (1 left of x = 0) over cells [x0, x0 + cell), x0 = (k - n/2) cell + phase."""
    x0 = (np.arange(n) - n // 2) * cell + phase
    return np.clip(-x0 / cell, 0, 1), x0


def run(f, a, kernel, cell=1.0, frames=400, n=64):
    hist, _ = film(n, 0.0, cell)
    worst = {}
    for t in range(1, frames + 1):
        phi_prev = ((t - 1) * f) % 1.0; phi = (t * f) % 1.0
        target, x0 = film(n, phi, cell)
        # Current cell centre in the previous cell grid (centre convention).
        pos = (x0 + cell / 2 - phi_prev) / cell + n // 2 - 0.5
        base = np.floor(pos).astype(int); fr = pos - base
        def tap(k):
            return hist[np.clip(base + k, 0, n - 1)]
        if kernel == "bilinear":
            fetched = (1 - fr) * tap(0) + fr * tap(1)
        else:
            w = cr(fr)
            fetched = np.maximum(w[0] * tap(-1) + w[1] * tap(0) + w[2] * tap(1) + w[3] * tap(2), 0)
        hist = (1 - a) * fetched + a * target
        if t > frames - 64:
            if cell == 1.0:
                est, ref, xs = hist, target, x0
            else:
                # Output pixels are pairs of cells aligned to the pixel grid (phase in cell units).
                est = 0.5 * (hist[0::2] + hist[1::2]); ref = 0.5 * (target[0::2] + target[1::2]); xs = x0[0::2]
            solid = (ref == 0) | (ref == 1)
            right = xs >= 0; gap = np.where(right, xs, -(xs + 1.0))
            err = np.abs(est - ref)
            for lo, hi in ((0, .05), (.05, .1), (.1, .25), (.25, .5), (.5, 1), (1, 2), (2, 4)):
                m = solid & (gap >= lo) & (gap < hi) & (np.abs(xs) < 8)
                if m.any():
                    key = f"{lo}-{hi}"; worst[key] = max(worst.get(key, 0.0), float(err[m].max()))
    return worst


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, default=Path("E:/renderlab-work/motion-aa/blur.json"))
    args = p.parse_args()
    out = {}
    for f in (0.05, 0.1, 0.25, 0.5):
        s = f * (1 - f)
        for W in (1 + 1 / s, 4.6, 2.0, 1.1):
            for kernel in ("bilinear", "catmull-rom"):
                for cell, name in ((1.0, "history1x"), (0.5, "history2x")):
                    if cell == 0.5 and W < 2:
                        continue
                    key = f"f={f}/W={W:.2f}/{kernel}/{name}"
                    out[key] = run(f, 1 / W, kernel, cell)
                    print(key, {k: round(v, 4) for k, v in out[key].items()}, flush=True)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
