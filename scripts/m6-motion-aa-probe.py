"""Decodes gpu_film_t2_probe output (support/film_t2_probe.wgsl) into a per-record report.

py -3 -B scripts/m6-motion-aa-probe.py <probe dir> [--out report.json]
For each probe pixel and frame: the 3x3 fresh planes, every landed record in gather order (source
pixel and slot, age, placement, value, landing position, world point, the five plane tests with
both sides, the exact-ray result, and the closest hits of camera rays through the landing point
from the near plane and around the record's own point), the kept records, and for each kept record
its birth: the moving frame it was traced in (age), the pixel its world point projects to then,
that frame's fresh record there (to confirm the match) and its hit record (placement, triangle,
barycentrics, distance to the nearest edge) and visible rays. Window arrays come from the
`f<frame>-w<i>.bin` dumps (layout in probe.json).
"""
import argparse
import json
from pathlib import Path
import struct

import numpy as np

W, H = 1920, 1080


def f16(bits):
    return float(np.array([bits & 0xffff], dtype="<u2").view("<f2")[0])


def f32(u):
    return struct.unpack("<f", struct.pack("<I", u & 0xffffffff))[0]


def record(r):
    x, y, z, w = (int(v) for v in r)
    return dict(value=[f16(x), f16(x >> 16), f16(y)], placement=y >> 16, depth=f32(z),
                position=[(w & 0xfff) / 4096, ((w >> 12) & 0xfff) / 4096], age=(w >> 24) & 0xf,
                flag28=bool(w >> 28 & 1), tile=bool(w >> 29 & 1), side=bool(w >> 30 & 1), valid=bool(w >> 31 & 1))


def hit(u, k):
    return dict(kind=int(u[k]), t=f32(u[k + 1]), placement=int(u[k + 2]), primitive=int(u[k + 3]),
                bary=[f32(u[k + 4]), f32(u[k + 5])], front=int(u[k + 6]))


class Windows:
    def __init__(self, directory, info):
        self.dir, self.size, self.k = directory, info["size"], info["records"]
        self.windows = info["windows"]
        self.layout = info["window_arrays"]

    def origin(self, i):
        x, y = self.windows[i][:2]
        s = self.size
        return min(max(x - s // 2, 0), W - s), min(max(y - s // 2, 0), H - s)

    def load(self, frame, i):
        p = self.dir / f"f{frame:03}-w{i}.bin"
        if not p.exists():
            return None
        data, out, at, n = p.read_bytes(), {}, 0, self.size * self.size
        for name, _, bytes_ in self.layout:
            out[name] = np.frombuffer(data[at:at + n * bytes_], dtype="<u4" if name != "hdr" else "<u2").copy()
            at += n * bytes_
        s = self.size
        out["hdr"] = out["hdr"].view("<f2").reshape(s, s, 4).astype(np.float64)
        for name, c in (("hit", 4), ("planes", 4), ("fresh", 4), ("colour", 4)):
            out[name] = out[name].reshape(s, s, c)
        out["colour"] = out["colour"].view("<f4").reshape(s, s, 4)
        out["anchor"] = out["anchor"].reshape(s, s, 2)
        out["diagnostics"] = out["diagnostics"].reshape(s, s, 2)
        out["counts"] = out["counts"].reshape(s, s)
        out["covered"] = out["covered"].reshape(s, s)
        out["moved"] = out["moved"].reshape(self.k - 1, s, s, 4)
        return out

    def find(self, frame, pixel):
        """A window dump of `frame` containing `pixel`, as (arrays, local x, local y)."""
        for i, w in enumerate(self.windows):
            if not w[2] <= frame <= w[3]:
                continue
            x0, y0 = self.origin(i)
            lx, ly = pixel[0] - x0, pixel[1] - y0
            if 0 <= lx < self.size and 0 <= ly < self.size:
                d = self.load(frame, i)
                if d is not None:
                    return d, lx, ly
        return None


def project(matrix, x):
    m = np.array(matrix, dtype=np.float64).reshape(4, 4).T
    c = m @ np.array([*x, 1.0])
    n = c[:3] / c[3]
    return np.array([(n[0] + 1) / 2 * W, (1 - n[1]) / 2 * H])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("probe", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    info = json.loads((a.probe / "probe.json").read_text())
    k = info["records"]
    frames = info["frames"]
    moving = [f["frame"] for f in frames if f["moving"]]
    windows = Windows(a.probe, info)
    report = []
    for frame, pixels in sorted(info["probes"].items(), key=lambda kv: int(kv[0])):
        frame = int(frame)
        u = np.fromfile(a.probe / f"probe-f{frame:03}.bin", dtype="<u4").reshape(3, 8192)
        for slot, pixel in enumerate(pixels):
            h = u[slot]
            assert h[0] == 0x50524f42, ("no probe header", frame, pixel)
            planes = []
            for j in range(9):
                b = 16 + 12 * j
                planes.append(dict(placement=int(h[b]), world=[f32(h[b + i]) for i in (1, 2, 3)],
                                   normal=[f32(h[b + i]) for i in (4, 5, 6)], side=int(h[b + 7]), distance=f32(h[b + 8]),
                                   index=int(h[b + 10]) >> 24, band=int(h[b + 11]) & 1))
            entries = []
            for n in range(int(h[13])):
                e = h[256 + 64 * n:256 + 64 * (n + 1)]
                r = record(e[4:8])
                tests = [(f32(e[15 + 2 * i]), f32(e[16 + 2 * i])) for i in range(5)]
                entries.append(dict(source=[int(e[0]), int(e[1])], slot=int(e[2] & 0xff), listed=bool(e[2] >> 8 & 1),
                                    valid=bool(e[2] >> 9 & 1), count_before=int(e[3]), record=r,
                                    moved=[f32(e[8]), f32(e[9]), f32(e[10])], world=[f32(e[11]), f32(e[12]), f32(e[13])],
                                    plane_mask=int(e[14]), plane_tests=tests, occluded=int(e[25]),
                                    first_hit=hit(e, 26), ray_length=f32(e[33]), own_hit=hit(e, 34) if r["placement"] else None))
            kept = [record(h[128 + 4 * i:132 + 4 * i]) for i in range(int(h[14]))]
            row = dict(frame=frame, pixel=pixel, geometry=int(h[3]), background=int(h[4]), discontinuity=bool(h[5]),
                       near=f32(h[6]), far=f32(h[7]), window=[int(np.int32(h[i])) for i in (8, 9, 10, 11)],
                       radians_per_pixel=f32(h[12]), landed=int(h[13]), planes=planes, entries=entries, kept=kept)
            here = windows.find(frame, pixel)
            if here is not None:
                d, lx, ly = here
                row["image"] = d["hdr"][ly, lx, :3].tolist()
                row["fresh"] = record(d["fresh"][ly, lx])
                row["fresh_hit"] = d["hit"][ly, lx].tolist()
                row["fresh_visible_rays"] = int(d["counts"][ly, lx]) // 32
            # Births: kept records' world points at the moving frame they were traced in.
            births = []
            for r in kept:
                match = [e for e in entries if e["valid"] and e["record"]["value"] == r["value"]
                         and e["record"]["placement"] == r["placement"] and e["record"]["age"] + 1 == r["age"]]
                e = match[0] if match else None
                b = dict(kept=r, entry=None if e is None else entries.index(e))
                if e is not None and r["placement"] and r["age"] < 15:
                    before = [f for f in moving if f <= frame]
                    born = before[-1 - r["age"]] if len(before) > r["age"] else None
                    b["born"] = born
                    if born is not None:
                        q = project(frames[born]["matrix"], e["world"])
                        b["born_at"] = q.tolist()
                        cell = [int(np.floor(q[0])), int(np.floor(q[1]))]
                        b["born_pixel"] = cell
                        there = windows.find(born, cell)
                        if there is not None:
                            d, lx, ly = there
                            fresh = record(d["fresh"][ly, lx])
                            hr = d["hit"][ly, lx]
                            bary = [f32(int(hr[2])), f32(int(hr[3]))]
                            b.update(born_fresh=fresh, born_fresh_matches=fresh["value"] == r["value"],
                                     born_hit=dict(placement=int(hr[0]), primitive=int(hr[1]), bary=bary,
                                                   edge=min(bary[0], bary[1], 1 - bary[0] - bary[1])),
                                     born_visible_rays=int(d["counts"][ly, lx]) // 32,
                                     born_plane_band=int(d["planes"][ly, lx][3]) & 1,
                                     born_plane_index=int(d["planes"][ly, lx][2]) >> 24)
                births.append(b)
            row["births"] = births
            report.append(row)
            print(f"frame {frame} pixel {pixel}: geometry {row['geometry']} background {row['background']} "
                  f"discontinuity {row['discontinuity']} landed {row['landed']} kept {len(kept)} image {row.get('image')}")
            if "fresh" in row:
                print(f"  fresh value {row['fresh']['value'][0]:.4f} placement {row['fresh']['placement']} "
                      f"hit {row['fresh_hit']} visible rays {row['fresh_visible_rays']}")
            for i, e in enumerate(entries):
                r = e["record"]
                own = e["own_hit"]
                print(f"  [{i}] src {e['source']} slot {e['slot']} age {r['age']} pl {r['placement']} V {r['value'][0]:.4f} "
                      f"valid {e['valid']} mask {e['plane_mask']:05b} occl {e['occluded']} moved ({e['moved'][0]:.4f},{e['moved'][1]:.4f}) "
                      f"first hit pl {e['first_hit']['placement']+1 if e['first_hit']['kind'] else 0} prim {e['first_hit']['primitive']} "
                      f"t/L {e['first_hit']['t'] / e['ray_length'] if e['ray_length'] else 0:.6f}"
                      + (f" own pl {own['placement']+1 if own['kind'] else 0} prim {own['primitive']} t/L {own['t']/e['ray_length']:.7f}" if own else ""))
            for b in births:
                print("  kept", {kk: v for kk, v in b.items() if kk not in ("kept", "born_fresh")}, "V", b["kept"]["value"][0])
    # Static frames (the hold): each window centre's T1a mean and weight; a frame's own sample is
    # n mean_n - (n - 1) mean_(n-1), the seed frame's the records' mean. Samples off the solid
    # value are listed with their hit (triangle, barycentric distance to an edge) and visible rays.
    holds = []
    for i, w in enumerate(windows.windows):
        x, y, first, last = w
        x0, y0 = windows.origin(i)
        previous = None
        for f in range(first, last + 1):
            if frames[f]["moving"] or frames[f]["reset"]:
                previous = None
                continue
            d = windows.load(f, i)
            if d is None:
                continue
            mean, n = float(d["colour"][y - y0, x - x0, 0]), float(d["colour"][y - y0, x - x0, 3])
            if previous is not None and n == previous[1] + 1:
                sample = n * mean - previous[1] * previous[0]
                hr = d["hit"][y - y0, x - x0]
                bary = [f32(int(hr[2])), f32(int(hr[3]))]
                row = dict(window=i, pixel=[x, y], frame=f, weight=n, sample=round(sample, 4), placement=int(hr[0]),
                           primitive=int(hr[1]), bary=bary, edge=min(bary[0], bary[1], 1 - bary[0] - bary[1]),
                           visible_rays=int(d["counts"][y - y0, x - x0]) // 32)
                if abs(sample - round(sample)) > 1e-3 or round(sample) != round(mean):
                    holds.append(row)
                    print("  hold sample off the mean", row)
            elif previous is None:
                print(f"hold window {i} pixel {[x, y]} frame {f}: seeded weight {n} mean {mean:.5f}")
            previous = (mean, n)
    if a.out:
        a.out.write_text(json.dumps(dict(probes=report, hold_samples=holds), indent=1))


if __name__ == "__main__":
    main()
