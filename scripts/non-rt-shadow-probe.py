"""CPU light-space probe for non-RT shadow options on the saved quarry pose; no renderer.

For a random sample of 16x16 screen tiles at pose 0 it uses the saved exact receivers
(R1 viewed policy, eps = 10 um) and the read-only RLA1 source to measure, along the
central sun direction (a directional light is one 2D point query per receiver):

1. exact hard visibility by point-in-triangle tests in light space (a CPU irregular
   z-buffer), checked against the saved 256-ray Exact counts;
2. whether dropping light-facing (or light-backfacing) casters, the closed-mesh cull of
   tiled soft shadow volumes, changes that visibility on the open scans;
3. depth-map aliasing at Unreal's VSM texel ladder: map depth at the texel centre against
   the receiver, plain and with receiver-plane depth, over a bias sweep;
4. caster-candidate counts that price IZB and per-triangle soft shadow volumes: per tile
   (Kalbertodt-style tile lists) and per pixel (cone of the 0.04 rad sun disk).

Run: py -3 -B scripts/non-rt-shadow-probe.py --out E:/renderlab-work/non-rt-2
"""

import argparse
import importlib.util
import json
import time
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("probe", REPO / "scripts/quarry-shadow-probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

H, W, TILE = 1080, 1920, 16
TO_LIGHT = np.array([0.4, 0.8, 0.3]) / np.linalg.norm([0.4, 0.8, 0.3])
EYE = np.array([5.2, 3.4, 6.2])
TAN = np.tan(0.04)
TEXELS_MM = [10.0, 5.0, 2.5, 1.25, 0.625]  # Unreal clipmap texels are 2^(i-13) cm
BIAS_MM = [0.0, 0.1, 0.5, 1.0, 2.0, 5.0]
GRID = 0.064  # coarse light-space cell, metres


def basis():
    w = TO_LIGHT
    u = np.cross(w, [0.0, 0.0, 1.0])
    u /= np.linalg.norm(u)
    return u, np.cross(w, u), w


def load_triangles(source, centre):
    """Light-space vertices (a, b, s) of every triangle as f32 relative to centre, plus
    winding facing sign against the light and the global max s."""
    u, v, w = basis()
    m = np.stack([u, v, w], 1)
    tri = np.empty((source.triangles, 3, 3), np.float32)  # (T, 3 vertices, (a, b, s))
    facing = np.empty(source.triangles, np.int8)
    at = 0
    for mesh, matrix in source.placements:
        (_, vertices, indices), = source.meshes[mesh]
        rot, off = matrix[:3, :3].astype(float), matrix[:3, 3].astype(float)
        for start in range(0, len(indices), 1 << 20):
            p = vertices[indices[start:start + (1 << 20)], :3].astype(float) @ rot.T + off
            n = len(p)
            facing[at:at + n] = np.sign(np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]) @ w)
            tri[at:at + n] = (p - centre) @ m
            at += n
    assert at == len(tri)
    return tri, facing


def receivers(source, hit, cover):
    """Exact on-surface points, viewed-side origins, and own-triangle light-space data."""
    ys, xs = np.nonzero(cover)
    place = hit[ys, xs, 0].astype(np.int64) - 1
    prim = hit[ys, xs, 1].astype(np.int64)
    bary = hit[ys, xs, 2:].view("<f4").astype(float)
    b = np.c_[1 - bary.sum(-1), bary]
    p = np.zeros((len(ys), 3, 3))
    for k, (mesh, matrix) in enumerate(source.placements):
        sel = place == k
        (_, vertices, indices), = source.meshes[mesh]
        p[sel] = vertices[indices[prim[sel]], :3].astype(float) @ matrix[:3, :3].astype(float).T \
            + matrix[:3, 3]
    point = np.einsum("ij,ijk->ik", b, p)
    n = np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0])
    n /= np.linalg.norm(n, axis=1, keepdims=True)
    n *= np.where(np.einsum("ij,ij->i", n, EYE - point) < 0, -1, 1)[:, None]
    mag = np.abs(p).max((1, 2))
    eps = np.maximum(1e-5, 16 * 2.0 ** (np.floor(np.log2(mag)) - 23))
    return ys, xs, point, point + eps[:, None] * n, eps, p, n


def inside(tri, q):
    """Barycentric point-in-triangle in light-space 2D, f64. tri (N,3,3), q (N,2).
    Returns mask and interpolated s; edge-on (zero projected area) triangles never hit."""
    t = tri.astype(float)
    a, b, c = t[:, 0, :2] - q, t[:, 1, :2] - q, t[:, 2, :2] - q
    l0 = b[:, 0] * c[:, 1] - b[:, 1] * c[:, 0]
    l1 = c[:, 0] * a[:, 1] - c[:, 1] * a[:, 0]
    l2 = a[:, 0] * b[:, 1] - a[:, 1] * b[:, 0]
    d = l0 + l1 + l2
    ok = (np.abs(d) > 1e-14) & (((l0 >= 0) & (l1 >= 0) & (l2 >= 0)) | ((l0 <= 0) & (l1 <= 0) & (l2 <= 0)))
    d = np.where(ok, d, 1.0)
    s = (l0 * t[:, 0, 2] + l1 * t[:, 1, 2] + l2 * t[:, 2, 2]) / d
    return ok, s


class Grid:
    """Triangles bucketed by light-space bbox centre into GRID cells."""

    BIG = 0.05  # triangles wider than this in light space are always candidates

    def __init__(self, tri):
        lo, hi = tri[..., :2].min(1), tri[..., :2].max(1)
        self.lo, self.hi, self.smax = lo, hi, tri[..., 2].max(1)
        extent = (hi - lo).max(1)
        big = extent > self.BIG
        self.big = np.nonzero(big)[0].astype(np.int32)
        self.margin = float(extent[~big].max()) / 2
        self.origin = lo.min(0) - 1e-3
        cx = ((lo[:, 0] + hi[:, 0]) / 2 - self.origin[0]) // GRID
        cy = ((lo[:, 1] + hi[:, 1]) / 2 - self.origin[1]) // GRID
        self.nx, self.ny = int(cx.max()) + 1, int(cy.max()) + 1
        key = cx.astype(np.int64) * self.ny + cy.astype(np.int64)
        del cx, cy
        key[big] = self.nx * self.ny  # parked past the last cell
        self.order = np.argsort(key, kind="stable").astype(np.int32)
        self.starts = np.searchsorted(key[self.order], np.arange(self.nx * self.ny + 1))

    def query(self, lo, hi):
        """Candidate ids: bbox centre within [lo - margin, hi + margin], plus big triangles."""
        c0 = np.floor((lo - self.margin - self.origin) / GRID).astype(int)
        c1 = np.floor((hi + self.margin - self.origin) / GRID).astype(int)
        c0 = np.maximum(c0, 0)
        c1 = np.minimum(c1, [self.nx - 1, self.ny - 1])
        ids = [self.order[self.starts[x * self.ny + c0[1]]:self.starts[x * self.ny + c1[1] + 1]]
               for x in range(c0[0], c1[0] + 1)] + [self.big]
        return np.concatenate(ids)


def bbox_dist(lo, hi, q):
    """2D distance from points q (N,2) to boxes [lo, hi] (N,2)."""
    d = np.maximum(np.maximum(lo - q, q - hi), 0)
    return np.hypot(d[:, 0], d[:, 1])


def covering(grid, tri, pts, ids):
    """Triangles of ids whose light-space projection contains each point (central
    direction). Returns point index, triangle id and interpolated s, chunked so that no
    pair array exceeds about 4M entries."""
    out_r, out_t, out_s, step = [], [], [], max(1, (1 << 22) // max(1, len(ids)))
    lo, hi = grid.lo[ids], grid.hi[ids]
    for k in range(0, len(pts), step):
        q = pts[k:k + step]
        inb = ((lo[None] <= q[:, None]) & (hi[None] >= q[:, None])).all(-1)
        ri, ci = np.nonzero(inb)
        ok, s = inside(tri[ids[ci]], q[ri])
        out_r.append(ri[ok] + k)
        out_t.append(ids[ci[ok]])
        out_s.append(s[ok])
    return np.concatenate(out_r), np.concatenate(out_t), np.concatenate(out_s)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--r1", type=Path,
                    default=Path("E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p0"))
    ap.add_argument("--asset", type=Path, default=Path(
        "D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla"))
    ap.add_argument("--tiles", type=int, default=300)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    hit = np.fromfile(args.r1 / "pose-0.hit.u32", "<u4").reshape(H, W, 4)
    exact = np.fromfile(args.r1 / "pose-0.exact.u32", "<u4").reshape(H, W)
    cover = hit[..., 0] > 0
    source = probe.Source(args.asset)
    ys, xs, point, origin, eps, own, normal = receivers(source, hit, cover)
    u, v, w = basis()
    m = np.stack([u, v, w], 1)
    centre = point.mean(0)
    lp, lo_ = (point - centre) @ m, (origin - centre) @ m
    own_l = (own - centre) @ m
    tri, facing = load_triangles(source, centre)
    grid = Grid(tri)
    t_load = time.time() - t0

    # Receiver-plane depth gradient of the own triangle in light space.
    e1, e2 = own_l[:, 1] - own_l[:, 0], own_l[:, 2] - own_l[:, 0]
    det = e1[:, 0] * e2[:, 1] - e1[:, 1] * e2[:, 0]
    det = np.where(np.abs(det) < 1e-18, 1e-18, det)
    grad = np.c_[e1[:, 2] * e2[:, 1] - e2[:, 2] * e1[:, 1], e2[:, 2] * e1[:, 0] - e1[:, 2] * e2[:, 0]] / det[:, None]

    tile_id = (ys // TILE) * (W // TILE) + xs // TILE
    tiles, counts = np.unique(tile_id, return_counts=True)
    rng = np.random.default_rng(20261106)
    chosen = rng.choice(tiles, size=min(args.tiles, len(tiles)), replace=False)
    smax_all = float(grid.smax.max())

    rec_idx, blocked, block_t, blocked_back, blocked_front, depth_cx = [], [], [], [], [], []
    vsm_plain = {h: [] for h in TEXELS_MM}
    vsm_plane = {h: [] for h in TEXELS_MM}
    tile_hard, tile_soft, tile_soft_back, pix_soft, pix_soft_back = [], [], [], [], []
    for tile in chosen:
        r = np.nonzero(tile_id == tile)[0]
        q, s_o = lo_[r, :2], lo_[r, 2]
        lo, hi = q.min(0), q.max(0)
        # Hard (central direction) list: bbox overlaps tile footprint, in front of some receiver.
        pad = max(TEXELS_MM) / 2000
        ids = grid.query(lo - pad, hi + pad)
        keep = (grid.lo[ids] <= hi + pad).all(1) & (grid.hi[ids] >= lo - pad).all(1)
        ids = ids[keep]
        tile_hard.append(len(ids))
        ri, ti, s = covering(grid, tri, q, ids)
        t = s - s_o[ri]
        depth_cx.append(np.bincount(ri, minlength=len(r)))
        blk = t > eps[r][ri]
        nearest = np.full(len(r), np.inf)
        np.minimum.at(nearest, ri[blk], t[blk])
        rec_idx.append(r)
        blocked.append(np.isfinite(nearest))
        block_t.append(nearest)
        blocked_back.append(np.bincount(ri[blk & (facing[ti] < 0)], minlength=len(r)) > 0)
        blocked_front.append(np.bincount(ri[blk & (facing[ti] > 0)], minlength=len(r)) > 0)
        # Depth map at texel centres: nearest-to-light surface depth.
        for h in TEXELS_MM:
            c = (np.floor(lp[r, :2] / (h / 1000)) + 0.5) * (h / 1000)
            ri2, _, s2 = covering(grid, tri, c, ids)
            depth = np.full(len(r), -np.inf)
            np.maximum.at(depth, ri2, s2)
            plane = lp[r, 2] + np.einsum("ij,ij->i", grad[r], c - lp[r, :2])
            vsm_plain[h].append(depth - lp[r, 2])
            vsm_plane[h].append(depth - plane)
        # Soft-shadow-volume candidates: cone of the sun disk from the tile and per pixel.
        reach = (smax_all - s_o.min()) * TAN
        ids = grid.query(lo - reach, hi + reach)
        height = grid.smax[ids] - s_o.min()
        tlo, thi = grid.lo[ids], grid.hi[ids]
        gap = np.maximum(np.maximum(lo - thi, tlo - hi), 0)
        keep = (height > 1e-5) & (np.hypot(gap[:, 0], gap[:, 1]) <= height * TAN)
        ids = ids[keep]
        tile_soft.append(len(ids))
        tile_soft_back.append(int((facing[ids] < 0).sum()))
        sample = r if len(r) <= 16 else rng.choice(r, 16, replace=False)
        for k in sample:
            hgt = grid.smax[ids] - lo_[k, 2]
            near = (hgt > eps[k]) & (bbox_dist(grid.lo[ids], grid.hi[ids], lo_[k, :2]) <= hgt * TAN)
            pix_soft.append(int(near.sum()))
            pix_soft_back.append(int((near & (facing[ids] < 0)).sum()))

    r = np.concatenate(rec_idx)
    blocked, block_t = np.concatenate(blocked), np.concatenate(block_t)
    back, front = np.concatenate(blocked_back), np.concatenate(blocked_front)
    ex = exact[ys[r], xs[r]]
    q = lambda a, qs=(.5, .9, .99, .999): [float(x) for x in np.quantile(a, qs)]
    lit_n, blk_n = int((~blocked).sum()), int(blocked.sum())
    report = dict(
        method=__doc__, seconds_load=round(t_load, 1), triangles=int(len(tri)),
        tiles=int(len(chosen)), receivers=int(len(r)), covered_pixels=int(cover.sum()),
        light=TO_LIGHT.tolist(), tan_disk=TAN,
        validation=dict(
            exact_umbra_but_central_lit=int(((ex == 0) & ~blocked).sum()),
            exact_umbra=int((ex == 0).sum()),
            exact_lit_but_central_blocked=int(((ex == 256) & blocked).sum()),
            exact_lit=int((ex == 256).sum())),
        central=dict(lit=lit_n, blocked=blk_n,
                     blocker_distance_mm=q(block_t[blocked] * 1000),
                     blocked_within_mm={str(d): int((block_t < d / 1000).sum()) for d in (1, 5, 10, 25, 50, 250)}),
        closed_mesh_cull=dict(
            keep_backfacing_lost=int((blocked & ~back).sum()),
            keep_frontfacing_lost=int((blocked & ~front).sum()),
            keep_backfacing_lost_within_10mm=int((blocked & ~back & (block_t < .01)).sum())),
        candidates=dict(
            hard_tile_list=q(tile_hard), depth_complexity=q(np.concatenate(depth_cx)),
            depth_complexity_mean=float(np.concatenate(depth_cx).mean()),
            big_triangles=int(len(grid.big)), grid_margin_mm=grid.margin * 1000,
            soft_tile_list=q(tile_soft), soft_tile_list_mean=float(np.mean(tile_soft)),
            soft_tile_list_backfacing_mean=float(np.mean(tile_soft_back)),
            soft_per_pixel=q(pix_soft), soft_per_pixel_mean=float(np.mean(pix_soft)),
            soft_per_pixel_backfacing_mean=float(np.mean(pix_soft_back))),
        depth_map={})
    for h in TEXELS_MM:
        plain, plane = np.concatenate(vsm_plain[h]), np.concatenate(vsm_plane[h])
        rows = {}
        for b in BIAS_MM:
            for name, margin in (("plain", plain), ("receiver_plane", plane)):
                mb = margin > b / 1000
                rows[f"{name}_{b}"] = dict(
                    false_shadow=int((mb & ~blocked).sum()), lost_shadow=int((~mb & blocked).sum()),
                    lost_within_10mm=int((~mb & blocked & (block_t < .01)).sum()))
        lit_margin = plane[~blocked]
        report["depth_map"][str(h)] = dict(
            rows=rows, receiver_plane_false_margin_mm=q(np.maximum(lit_margin, 0) * 1000, (.9, .99, .999, 1.0)),
            plain_false_margin_mm=q(np.maximum(plain[~blocked], 0) * 1000, (.9, .99, .999, 1.0)))
    report["seconds_total"] = round(time.time() - t0, 1)
    (args.out / "probe.json").write_text(json.dumps(report, indent=1))
    print(json.dumps({k: report[k] for k in ("receivers", "validation", "central", "closed_mesh_cull", "candidates", "seconds_total")}, indent=1))


if __name__ == "__main__":
    main()
