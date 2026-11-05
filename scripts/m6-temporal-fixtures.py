"""T1 fixtures against the R1 analytic oracles, at T1's own sample positions (CPU, two threads).

Reads gpu_film_t1_fixtures output folders. Rectangle fixtures (thin, stress) compare T1's
centre-surface visibility with the continuous disk oracle averaged over exactly the sample
positions T1 traced: the lattice index of each frame, with each pixel's symmetry
(shaders/film_accumulation.wgsl film_offset). Acne fixtures (flat, convex) require every
covered pixel to stay exactly lit.

py -3 -B scripts/m6-temporal-fixtures.py E:/renderlab-work/t1-gpu/fixtures/*
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time

import numpy as np
from numba import njit, prange, set_num_threads

spec = importlib.util.spec_from_file_location("receiver_fixtures", Path(__file__).with_name("m6-receiver-fixtures.py"))
rf = importlib.util.module_from_spec(spec); spec.loader.exec_module(rf)
score, f0 = rf.score, rf.f0
GENERATOR = 71
MASK = 0xffffffff


def fresh_hash(x):
    x = np.asarray(x, dtype=np.uint64) & MASK
    x = ((x ^ (x >> 16)) * 0x7feb352d) & MASK
    x = ((x ^ (x >> 15)) * 0x846ca68b) & MASK
    return x ^ (x >> 16)


def positions(count):
    """In-pixel position of each frame's sample, [frame, symmetry, xy] (film_offset)."""
    out = np.zeros((count, 4, 2))
    for f in range(count):
        k = f % 256
        for s in range(4):
            if k == 0:
                out[f, s] = .5
                continue
            order = ((0x0231 >> (4 * k)) & 0xf) if k < 4 else k
            j = int(f"{order:08b}"[::-1], 2)
            p = [j, (GENERATOR * j) & 0xff]
            if s & 1:
                p = [(256 - v) & 0xff for v in p]
            if s & 2:
                p = p[::-1]
            out[f, s] = [(((v + 8) & 0xff) + .5) / 256 for v in p]
    return out


@njit(parallel=True)
def rect_film(rects, hit, depth, table, symmetry):
    """rect_film of m6-receiver-fixtures.py with per-pixel positions."""
    h, w = hit.shape; ref = np.ones((h, w)); accepted = np.zeros((h, w), np.int32)
    scale = math.tan(math.radians(55) / 2) * 2 / h
    for y in prange(h):
        for x in range(w):
            if hit[y, x] == 0:
                continue
            total = 0.; n = 0
            for f in range(table.shape[0]):
                jx, jy = table[f, symmetry[y, x]]
                ox = (x + jx - w / 2) * scale; oy = (h / 2 - y - jy) * scale
                z = 0.; instance = 1
                for ri in range(len(rects)):
                    cx, cy, rz, hx, hy, angle = rects[ri]
                    if rz <= z:
                        continue
                    px = ox * (4 - rz) - cx; py = oy * (4 - rz) - cy; c = math.cos(angle); s = math.sin(angle)
                    if abs(c * px + s * py) <= hx and abs(-s * px + c * py) <= hy:
                        z = rz; instance = ri + 2
                ds = 4 - z
                if instance != hit[y, x] or abs(ds - depth[y, x]) > .02 * depth[y, x]:
                    continue
                bias = 16 * float(np.spacing(np.float32(8))) if instance == 1 else 1e-5
                total += f0.disk_visibility(ox * ds, oy * ds, z + bias, rects); n += 1
            if n:
                ref[y, x] = total / n
            accepted[y, x] = n
    return ref, accepted


def check(directory):
    d = score.decode(directory, 0)
    info = json.loads((directory / "pose-0.json").read_text())
    work = json.loads((directory / "geometry.json").read_text())["workload"] if (directory / "geometry.json").exists() else None
    if work in ("flat", "convex"):
        valid = d["cover"] & (d["n"] > 0)
        false = int((valid & (d["ref"] != 1)).sum())
        return dict(kind="acne", passed=false == 0, false_shadow_pixels=false, covered=int(valid.sum()),
                    max_error=float(np.max(np.abs(d["ref"][valid] - 1))), frames=info["frames"])
    h, w = d["hit"].shape[:2]
    ys, xs = np.mgrid[0:h, 0:w]
    symmetry = (fresh_hash((xs + ys * w) ^ 0x2f6b1e5d) & 3).astype(np.int64)
    rectangles = json.loads((directory / "rectangles.json").read_text())
    rects = np.array([[*r["center"], *r["half"], r["angle"]] for r in rectangles])
    reference, n = rect_film(rects, d["hit"][..., 0], d["surface_depth"], positions(info["frames"]), symmetry)
    valid = d["cover"] & (n > 0) & (d["n"] > 0)
    partial = valid & (reference > 1e-9) & (reference < 1 - 1e-9)
    rows = dict(kind="rectangles", frames=info["frames"])
    for name, mask in (("all", valid), ("partial", partial)):
        rows[name] = score.summary((d["ref"] - reference)[mask], d["sigma"][mask])
        rows[name]["within_sigma_F"] = rows[name].get("rms", 0) <= rows[name].get("noise_rms", 0)
    solid = valid & ((reference <= 1e-9) | (reference >= 1 - 1e-9))
    rows["solid"] = dict(n=int(solid.sum()), max_error=float(np.abs(d["ref"] - reference)[solid].max()) if solid.any() else 0.)
    rows["accepted_count_mismatches"] = int((d["cover"] & (d["n"] != n)).sum())
    rows["missing_oracle_samples"] = int((d["cover"] & ((n == 0) | (d["n"] == 0))).sum())
    rows["passed"] = all(rows[k]["within_sigma_F"] for k in ("all", "partial")) and rows["missing_oracle_samples"] == 0
    np.savez_compressed(directory / "analytic.npz", ref=reference, n=n)
    score.sheet([("T1 positions, analytic film", score.vis(reference, d["cover"])), ("T1 film visibility", score.vis(d["ref"], d["cover"])),
                 ("T1 minus analytic x8", score.error(d["ref"] - reference, d["cover"], valid))], width=512).save(directory / "oracle.png")
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__); p.add_argument("directories", type=Path, nargs="+"); a = p.parse_args()
    set_num_threads(2); start = time.perf_counter(); passed = True
    for directory in a.directories:
        result = check(directory)
        (directory / "validation.json").write_text(json.dumps(result, indent=2))
        print(directory.name, json.dumps(result), "elapsed", round(time.perf_counter() - start, 1), flush=True)
        passed = passed and result["passed"]
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
