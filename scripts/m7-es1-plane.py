"""M7-ES1 sky-plane summary (docs/m7-es1.md): each case's centre-pixel luminance against the
16,384-spp reference, per program, cache state and path. CPU only.

  py -3 -B scripts/m7-es1-plane.py RUN_DIR   reads RUN_DIR/plane-{rt,nonrt}.json
"""
import json
import sys
from pathlib import Path

import numpy as np

LUM = np.array([0.2126, 0.7152, 0.0722])


def main():
    run = Path(sys.argv[1])
    summary = {}
    for path in ("rt", "nonrt"):
        f = run / f"plane-{path}.json"
        if not f.exists():
            continue
        rows = json.loads(f.read_text())["rows"]
        invalid = sorted({(r["metal"], r["roughness"], r["nv"], r["view"]) for r in rows if r.get("reference_invalid_pixels")})
        if invalid:
            print(f"{path}: {len(invalid)} cases with non-finite reference samples, not scored: {invalid}")
        rows = [r for r in rows if not r.get("reference_invalid_pixels")]
        for key in sorted({(r["program"], r["cache"]) for r in rows}):
            sel = [r for r in rows if (r["program"], r["cache"]) == key]
            ratios = np.array([np.dot(r["radiance"], LUM) / np.dot(r["reference"], LUM) for r in sel])
            within = np.array([abs(np.dot(r["radiance"], LUM) - np.dot(r["reference"], LUM))
                               <= 0.01 * np.dot(r["reference"], LUM) + 4 * np.dot(r["reference_se"], LUM) for r in sel])
            name = f"{path} {key[0]} cache-{'on' if key[1] else 'off'}"
            summary[name] = dict(cases=len(sel), median=float(np.median(ratios)), min=float(ratios.min()),
                                 max=float(ratios.max()), mean_abs_error=float(np.mean(np.abs(ratios - 1))),
                                 within_1pct_4se=int(within.sum()))
            print(f"{name:32s} n={len(sel):3d} ratio median {np.median(ratios):.3f} range "
                  f"{ratios.min():.3f}-{ratios.max():.3f} mean|err| {np.mean(np.abs(ratios - 1)):.3f} "
                  f"within 1%+4SE {within.sum()}/{len(sel)}")
        # Per-case table for the ES1 program with the cache on.
        print(f"\n{path}: roughness, N.V, view, metal: baseline-off / es1-on ratio")
        index = {(r["program"], r["cache"], r["metal"], r["roughness"], r["nv"], r["view"]): r for r in rows}
        for r in rows:
            if r["program"] != "es1" or not r["cache"]:
                continue
            b = index.get(("baseline", False, r["metal"], r["roughness"], r["nv"], r["view"]))
            ref = np.dot(r["reference"], LUM)
            print(f"  r={r['roughness']:.2f} nv={r['nv']:.2f} {r['view']:5s} {'metal' if r['metal'] else 'diel '}"
                  f"  {np.dot(b['radiance'], LUM) / ref if b else float('nan'):.3f} / {np.dot(r['radiance'], LUM) / ref:.3f}")
    (run / "plane-summary.json").write_text(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
