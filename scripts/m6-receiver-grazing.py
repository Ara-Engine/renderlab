"""Independent continuous-profile oracles for grazing-view receiver contacts."""
import argparse
import importlib.util
import json
import math
from pathlib import Path

import numpy as np
from numba import njit, prange, set_num_threads

spec = importlib.util.spec_from_file_location("receiver_oracle", Path(__file__).with_name("m6-receiver-fixtures.py"))
oracle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oracle)


@njit
def visibility(ox, oz, profile, theta, slope):
    intervals = np.empty((len(profile), 2))
    count = 0
    for j in range(len(profile) - 1):
        x0, z0 = profile[j]
        x1, z1 = profile[j + 1]
        if x1 <= ox:
            continue
        if x0 <= ox:
            if x0 == x1:
                continue
            a = (z1 - z0) / (x1 - x0)
            z = z0 + a * (ox - x0)
            k0 = 1e35 if z > oz else -1e35
            if abs(z - oz) < 1e-15:
                k0 = a
        else:
            k0 = (z0 - oz) / (x0 - ox)
        k1 = (z1 - oz) / (x1 - ox)
        lo, hi = min(k0, k1), max(k0, k1)
        i = count
        while i > 0 and intervals[i - 1, 0] > lo:
            intervals[i] = intervals[i - 1]
            i -= 1
        intervals[i, 0] = lo
        intervals[i, 1] = hi
        count += 1
    end = -1e36
    blocked = 0.
    for j in range(count):
        lo, hi = intervals[j]
        if hi > max(lo, end):
            blocked += oracle.slope_cdf(hi, theta, slope) - oracle.slope_cdf(max(lo, end), theta, slope)
        end = max(end, hi)
    return max(0., min(1., 1. - blocked))


@njit(parallel=True)
def reference(profile, inverse, eye, forward, depth, cover, jitters, theta, disk, epsilon):
    height, width = depth.shape
    result = np.ones((height, width))
    counts = np.zeros((height, width), np.int32)
    for y in prange(height):
        for x in range(width):
            if not cover[y, x]:
                continue
            total = 0.
            count = 0
            for j in range(len(jitters)):
                dx = 2 * (x + jitters[j, 0]) / width - 1
                dy = 1 - 2 * (y + jitters[j, 1]) / height
                sx4 = inverse[0, 0] * dx + inverse[0, 1] * dy + inverse[0, 3]
                sy4 = inverse[1, 0] * dx + inverse[1, 1] * dy + inverse[1, 3]
                sz4 = inverse[2, 0] * dx + inverse[2, 1] * dy + inverse[2, 3]
                sw4 = inverse[3, 0] * dx + inverse[3, 1] * dy + inverse[3, 3]
                nw = sw4 + inverse[3, 2]
                nearx = (sx4 + inverse[0, 2]) / nw
                neary = (sy4 + inverse[1, 2]) / nw
                nearz = (sz4 + inverse[2, 2]) / nw
                dirx, diry, dirz = sx4 / sw4 - nearx, sy4 / sw4 - neary, sz4 / sw4 - nearz
                best = 1e30
                px = pz = nx = nz = py = 0.
                for k in range(len(profile) - 1):
                    x0, z0 = profile[k]
                    x1, z1 = profile[k + 1]
                    sx, sz = x1 - x0, z1 - z0
                    det = dirx * sz - dirz * sx
                    if abs(det) < 1e-30:
                        continue
                    ax, az = x0 - nearx, z0 - nearz
                    t = (ax * sz - az * sx) / det
                    u = (ax * dirz - az * dirx) / det
                    yy = neary + t * diry
                    if 0 <= u <= 1 and 0 <= t < best and abs(yy) <= 8:
                        best = t
                        px = x0 + sx * u
                        pz = z0 + sz * u
                        py = yy
                        length = math.sqrt(sx * sx + sz * sz)
                        nx, nz = -sz / length, sx / length
                if best == 1e30:
                    continue
                d = (px - eye[0]) * forward[0] + (py - eye[1]) * forward[1] + (pz - eye[2]) * forward[2]
                if abs(d - depth[y, x]) > .02 * depth[y, x]:
                    continue
                if nx * (eye[0] - px) + nz * (eye[2] - pz) < 0:
                    nx, nz = -nx, -nz
                total += visibility(px + nx * epsilon, pz + nz * epsilon, profile, theta, disk)
                count += 1
            if count:
                result[y, x] = total / count
            counts[y, x] = count
    return result, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", type=Path, nargs="+")
    parser.add_argument("--physical", action="store_true", help="infinitesimal viewed-side clearance instead of the frozen R1 clearance")
    args = parser.parse_args()
    set_num_threads(2)
    for directory in args.directories:
        score(directory,args.physical)


def score(directory,physical=False):
    d = oracle.score.decode(directory, 0)
    geometry = json.loads((directory / "geometry.json").read_text())
    info = json.loads((directory / "pose-0.json").read_text())
    expected, counts = reference(np.array(geometry["profile"]), np.array(info["camera_inverse"]).reshape(4, 4).T,
        np.array(info["eye"][:3]), np.array(info["forward"][:3]), d["surface_depth"], d["cover"], oracle.f0.jitters(),
        math.radians(geometry["incidence"]), math.tan(geometry["disk_angle"]),
        1e-10 if physical else 16 * float(np.spacing(np.float32(8))))
    valid = d["cover"] & (counts > 0) & (d["n"] > 0)
    matched = valid & (counts == d["n"])
    partial = matched & (expected > 1e-9) & (expected < 1 - 1e-9)
    rows = {name: oracle.score.summary((d["ref"] - expected)[mask], d["sigma"][mask])
            for name, mask in (("all", matched), ("partial", partial), ("including_boundary_mismatches", valid))}
    lost = valid & (expected <= .01) & (d["ref"] >= .99)
    false = valid & (expected >= .99) & (d["ref"] <= .01)
    rows.update(view_incidence=geometry["view_incidence"], lost_shadow_pixels=int(lost.sum()),
                false_shadow_pixels=int(false.sum()),
                accepted_count_mismatches=int((d["cover"] & (counts != d["n"])).sum()),
                missing_samples=int((d["cover"] & ((counts == 0) | (d["n"] == 0))).sum()))
    rows["passed"] = (rows["lost_shadow_pixels"] == 0 and rows["false_shadow_pixels"] == 0 and rows["missing_samples"] == 0
        and all(rows[k].get("rms", 0) <= rows[k].get("noise_rms", 0) for k in ("all", "partial")))
    label = "grazing-physical" if physical else "grazing-oracle"
    (directory / f"{label}.json").write_text(json.dumps(rows, indent=2))
    oracle.score.sheet([("Analytic surface oracle", oracle.score.vis(expected, d["cover"])),
        ("Production film", oracle.score.vis(d["ref"], d["cover"])),
        ("Signed error x8", oracle.score.error(d["ref"] - expected, d["cover"], valid))], width=256).save(directory / f"{label}.png")
    print(json.dumps(rows), flush=True)
    if not rows["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
