"""Far-map memory for N5: the visible pose-0 surface's area in the sun's light plane, and
the 128x128-texel virtual pages it touches at each texel size; no renderer.

Reads the saved R1 world positions and coverage. Light-plane area sums the two triangles
of every fully covered 2x2 pixel quad whose 3D edges are all under 5 cm (silhouette quads
are dropped). Pages are counted from every covered pixel and quad-edge midpoint, then
dilated by one page ring to cover far paths that cross a page border.

Run: py -3 -B scripts/non-rt-n5-footprint.py --out E:/renderlab-work/non-rt-n5/footprint.json
"""

import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("nrt", REPO / "scripts/non-rt-shadow-probe.py")
nrt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nrt)

TEXEL_MM = (0.5, 1.0, 2.0, 4.0, 8.0)
PAGE = 128


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--r1", type=Path, default=Path("E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p0"))
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    H, W = nrt.H, nrt.W
    world = np.fromfile(args.r1 / "pose-0.world.f32", "<f4").reshape(H, W, 4)[..., :3].astype(float)
    cover = np.fromfile(args.r1 / "pose-0.hit.u32", "<u4").reshape(H, W, 4)[..., 0] > 0
    u, v, w = nrt.basis()
    light = world @ np.stack([u, v], 1)  # (H, W, 2) light-plane coordinates
    p00, p10, p01, p11 = world[:-1, :-1], world[:-1, 1:], world[1:, :-1], world[1:, 1:]
    ok = cover[:-1, :-1] & cover[:-1, 1:] & cover[1:, :-1] & cover[1:, 1:]
    for a, b in ((p00, p10), (p00, p01), (p11, p10), (p11, p01), (p10, p01)):
        ok &= np.linalg.norm(a - b, axis=-1) < 0.05
    l00, l10, l01, l11 = light[:-1, :-1], light[:-1, 1:], light[1:, :-1], light[1:, 1:]
    cross = lambda a, b: np.abs(a[..., 0] * b[..., 1] - a[..., 1] * b[..., 0]) / 2
    area = (cross(l10 - l00, l01 - l00) + cross(l10 - l11, l01 - l11))[ok].sum()
    world_area = (np.linalg.norm(np.cross(p10 - p00, p01 - p00), axis=-1) / 2
                  + np.linalg.norm(np.cross(p10 - p11, p01 - p11), axis=-1) / 2)[ok].sum()
    pts = np.concatenate([light[cover], ((l00 + l11) / 2)[ok]])
    report = dict(method=__doc__, covered_pixels=int(cover.sum()), quads=int(ok.sum()),
                  light_plane_area_m2=float(area), world_area_m2=float(world_area), maps={})
    for tau_mm in TEXEL_MM:
        side = PAGE * tau_mm / 1000
        pages = np.unique(np.floor(pts / side).astype(np.int64), axis=0)
        ring = np.unique((pages[:, None, :] + np.array([[i, j] for i in (-1, 0, 1) for j in (-1, 0, 1)])[None])
                         .reshape(-1, 2), axis=0)
        texels = len(ring) * PAGE * PAGE
        report["maps"][str(tau_mm)] = dict(
            area_texels=float(area / (tau_mm / 1000) ** 2), pages=int(len(pages)), dilated_pages=int(len(ring)),
            mib_top_f32=texels * 4 / 2 ** 20, mib_top_and_cover_f32=texels * 12 / 2 ** 20)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=1))
    print(json.dumps({k: v for k, v in report.items() if k != "method"}, indent=1))


if __name__ == "__main__":
    main()
