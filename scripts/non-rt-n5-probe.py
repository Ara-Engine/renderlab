"""N5 CPU probe: split each sun shadow ray at r into an exact local near segment and a
conservative far map, on the saved quarry pose; no renderer.

Uses the same 300 random 16x16 pose-0 tiles (seed 20261106) and receivers as
non-rt-shadow-probe.py (R1 viewed, eps = max(10 um, 16 ulp)). Every receiver traces the
production's first 8 fresh_direction indices (R2 plus the per-pixel hash shift, slope
tan 0.04, tmin = eps) exactly in light space: two-sided f64 Moller-Trumbore over the f32
light-space vertices, against every triangle that can meet the ray. All hits are kept.

For r in R_MM it measures:
1. near sets: triangles meeting the truncated sun cone [eps, r] (bbox test), and the
   triangles actually hit within r;
2. capture of each near hit by the receiver triangle's vertex k-ring (same mesh, welded
   source positions) and by its 128-triangle Morton cluster; and the sizes of the
   sun-independent spatial lists (per triangle, per cluster: AABB within r) that capture
   every near blocker by construction;
3. a conservative far map: each texel holds the maximum light height of every candidate
   triangle over the texel (its plane at the texel corners, clamped to its top). The far
   segment (r, inf) walks every texel its light-space path crosses (exact DDA). It is
   "certainly lit" when each texel's top lies below the ray where the ray enters it, and
   "maybe" otherwise. Zero lost far shadows is checked, not assumed; maybe rays are the
   exact-refinement load;
4. per-ray errors of each hybrid against the exact test, by population.

Run: py -3 -B scripts/non-rt-n5-probe.py --out E:/renderlab-work/non-rt-n5/full
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import importlib.util
import json
import time
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("nrt", REPO / "scripts/non-rt-shadow-probe.py")
nrt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nrt)

H, W, TILE = nrt.H, nrt.W, nrt.TILE
R_MM = (2.0, 5.0, 10.0, 25.0)
TEXEL_MM = (0.5, 1.0, 2.0, 4.0, 8.0)
COARSE = 8  # coarse map level: COARSE x COARSE texels
K_MAX = 6
CLUSTER = 128
RAYS = 8
SLOPE = nrt.TAN
NEAR_KEEP = 0.026  # near hits kept for the capture analysis, metres
CROSS, BEYOND = 99, 98  # ring distance codes: other placement, beyond K_MAX
SAMPLE = 16  # receivers per tile for the spatial-list and ring-size counts
MAP_LIMIT = 8_000_000  # texels per local map; larger regions split their receivers


class Pmc(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("faults", wt.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "peak_ws", "ws", "qpp", "qp", "qpnp", "qnp", "private", "peak_private")]


class Status(ctypes.Structure):
    _fields_ = [("length", wt.DWORD), ("load", wt.DWORD)] + [
        (name, ctypes.c_ulonglong) for name in (
            "phys", "avail", "page", "avail_page", "virt", "avail_virt", "ext")]


def memory():
    """(private GiB, peak private GiB, peak working set GiB, system available GiB)."""
    c = Pmc()
    c.cb = ctypes.sizeof(c)
    info = ctypes.windll.psapi.GetProcessMemoryInfo
    info.argtypes = [wt.HANDLE, ctypes.POINTER(Pmc), wt.DWORD]
    info(wt.HANDLE(-1), ctypes.byref(c), c.cb)  # -1: the current-process pseudo handle
    s = Status()
    s.length = ctypes.sizeof(s)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(s))
    g = float(1 << 30)
    return c.private / g, c.peak_private / g, c.peak_ws / g, s.avail / g


def guard():
    private, _, _, avail = memory()
    if private > 1.5 or avail < 2.0:
        raise SystemExit(f"memory guard: private {private:.2f} GiB, available {avail:.2f} GiB")


def fresh_hash(x):
    x = x.astype(np.uint32)
    x = (x ^ (x >> np.uint32(16))) * np.uint32(0x7FEB352D)
    x = (x ^ (x >> np.uint32(15))) * np.uint32(0x846CA68B)
    return x ^ (x >> np.uint32(16))


def directions(ys, xs):
    """Light-space unit directions (n, RAYS, 3): fresh_direction indices 0..7."""
    seed = fresh_hash(xs.astype(np.uint32) + ys.astype(np.uint32) * np.uint32(W))
    shift = np.c_[(seed >> np.uint32(8)).astype(float),
                  (fresh_hash(seed ^ np.uint32(0xA511E9B3)) >> np.uint32(8)).astype(float)] / 16777216.0
    j = np.arange(RAYS, dtype=float)[None, :, None]
    r = np.mod(j * np.array([0.7548776662466927, 0.5698402909980532]) + shift[:, None, :], 1.0)
    a = 2 * np.pi * r[..., 1]
    rad = SLOPE * np.sqrt(r[..., 0])
    d = np.stack([rad * np.cos(a), rad * np.sin(a), np.ones_like(a)], -1)
    return d / np.linalg.norm(d, axis=-1, keepdims=True)


def intersect(o, d, tri, tmin):
    """Two-sided Moller-Trumbore in f64; o, d (N,3), tri (N,3,3) f32. t, or inf on miss."""
    t0 = tri[:, 0].astype(float)
    e1, e2 = tri[:, 1] - t0, tri[:, 2] - t0
    p = np.c_[d[:, 1] * e2[:, 2] - d[:, 2] * e2[:, 1], d[:, 2] * e2[:, 0] - d[:, 0] * e2[:, 2],
              d[:, 0] * e2[:, 1] - d[:, 1] * e2[:, 0]]
    det = (e1 * p).sum(1)
    ok = np.abs(det) > 1e-30
    inv = 1.0 / np.where(ok, det, 1.0)
    s = o - t0
    u = (s * p).sum(1) * inv
    q = np.c_[s[:, 1] * e1[:, 2] - s[:, 2] * e1[:, 1], s[:, 2] * e1[:, 0] - s[:, 0] * e1[:, 2],
              s[:, 0] * e1[:, 1] - s[:, 1] * e1[:, 0]]
    v = (d * q).sum(1) * inv
    t = (e2 * q).sum(1) * inv
    hit = ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t > tmin)
    return np.where(hit, t, np.inf)


def light_tris(source, offsets, centre, m, ids):
    """Light-space f32 vertices (len(ids), 3, 3) of global triangle ids, read from the mapped
    asset with elementwise f64 math, so any chunking gives bit-identical values."""
    out = np.empty((len(ids), 3, 3), np.float32)
    place = np.searchsorted(offsets, ids, "right") - 1
    for k in np.unique(place):
        mesh, matrix = source.placements[k]
        (_, vertices, indices), = source.meshes[mesh]
        rot, off = matrix[:3, :3].astype(float), matrix[:3, 3].astype(float)
        rows = np.nonzero(place == k)[0]
        for sel in np.array_split(rows, -(-len(rows) // (1 << 18))):
            p = vertices[indices[ids[sel] - offsets[k]], :3].astype(float)
            world = p[..., 0:1] * rot[:, 0] + p[..., 1:2] * rot[:, 1] + p[..., 2:3] * rot[:, 2] + off - centre
            out[sel] = (world[..., 0:1] * m[0] + world[..., 1:2] * m[1] + world[..., 2:3] * m[2]).astype(np.float32)
    return out


def bounds(source, offsets, centre, m):
    """Light-space boxes of every triangle: lo, hi (T,2), smin, smax (T,), f32."""
    t = int(offsets[-1])
    lo, hi = np.empty((t, 2), np.float32), np.empty((t, 2), np.float32)
    smin, smax = np.empty(t, np.float32), np.empty(t, np.float32)
    for c in range(0, t, 1 << 20):
        v = light_tris(source, offsets, centre, m, np.arange(c, min(t, c + (1 << 20))))
        s = slice(c, c + len(v))
        lo[s], hi[s] = v[..., :2].min(1), v[..., :2].max(1)
        smin[s], smax[s] = v[..., 2].min(1), v[..., 2].max(1)
    return lo, hi, smin, smax


class BoxGrid(nrt.Grid):
    """nrt.Grid built from precomputed light-space boxes instead of a triangle array."""

    def __init__(self, lo, hi, smax):
        self.lo, self.hi, self.smax = lo, hi, smax
        extent = (hi - lo).max(1)
        big = extent > self.BIG
        self.big = np.nonzero(big)[0].astype(np.int32)
        self.margin = float(extent[~big].max()) / 2
        self.origin = lo.min(0) - 1e-3
        cx = ((lo[:, 0] + hi[:, 0]) / 2 - self.origin[0]) // nrt.GRID
        cy = ((lo[:, 1] + hi[:, 1]) / 2 - self.origin[1]) // nrt.GRID
        self.nx, self.ny = int(cx.max()) + 1, int(cy.max()) + 1
        key = cx.astype(np.int64) * self.ny + cy.astype(np.int64)
        del cx, cy, extent
        key[big] = self.nx * self.ny
        self.order = np.argsort(key, kind="stable").astype(np.int32)
        # Sorted cell keys instead of nrt.Grid's dense starts: the quarry's light-space
        # extent (placement 11 lies hundreds of metres out) makes dense starts ~0.5 GB.
        self.keys = key[self.order].astype(np.int32)

    def query(self, lo, hi):
        """nrt.Grid.query over the sorted keys."""
        c0 = np.floor((lo - self.margin - self.origin) / nrt.GRID).astype(int)
        c1 = np.floor((hi + self.margin - self.origin) / nrt.GRID).astype(int)
        c0 = np.maximum(c0, 0)
        c1 = np.minimum(c1, [self.nx - 1, self.ny - 1])
        xs = np.arange(c0[0], c1[0] + 1)
        a = np.searchsorted(self.keys, xs * self.ny + c0[1], "left")
        b = np.searchsorted(self.keys, xs * self.ny + c1[1], "right")
        return np.concatenate([self.order[i:j] for i, j in zip(a, b)] + [self.big])


def part1by2(x):
    x = x.astype(np.uint64) & np.uint64(0x1FFFFF)
    for shift, mask in ((32, 0x1F00000000FFFF), (16, 0x1F0000FF0000FF), (8, 0x100F00F00F00F00F),
                        (4, 0x10C30C30C30C30C3), (2, 0x1249249249249249)):
        x = (x | (x << np.uint64(shift))) & np.uint64(mask)
    return x


def clusters(offsets, lo, hi, smin, smax):
    """CLUSTER-triangle clusters in Morton order of light-space box centres, per placement.
    Returns the cluster id per triangle and cluster boxes (lo2, hi2, smin, smax, count)."""
    cid = np.empty(int(offsets[-1]), np.int32)
    boxes, base, chunk = [], 0, 1 << 20
    for a, b in zip(offsets[:-1], offsets[1:]):
        n = int(b - a)
        bmin = np.r_[lo[a:b].min(0), smin[a:b].min()].astype(np.float64)
        span = max(float((np.r_[hi[a:b].max(0), smax[a:b].max()] - bmin).max()), 1e-9)
        code = np.empty(n, np.uint64)
        for c in range(0, n, chunk):
            e = min(b, a + c + chunk)
            centre = np.c_[(lo[a + c:e].astype(float) + hi[a + c:e]) / 2,
                           (smin[a + c:e].astype(float) + smax[a + c:e]) / 2]
            q = (centre - bmin) / span * ((1 << 21) - 1)
            q = np.clip(q, 0, (1 << 21) - 1).astype(np.uint64)
            code[c:c + chunk] = part1by2(q[:, 0]) | (part1by2(q[:, 1]) << np.uint64(1))                 | (part1by2(q[:, 2]) << np.uint64(2))
        order = np.argsort(code, kind="stable")
        del code
        parts = []
        for c in range(0, n, CLUSTER * 8192):
            g = a + order[c:c + CLUSTER * 8192]
            cid[g] = (base + (c + np.arange(len(g))) // CLUSTER).astype(np.int32)
            starts = np.arange(0, len(g), CLUSTER)
            parts.append((np.minimum.reduceat(lo[g], starts), np.maximum.reduceat(hi[g], starts),
                          np.minimum.reduceat(smin[g], starts), np.maximum.reduceat(smax[g], starts),
                          np.diff(np.r_[starts, len(g)])))
        boxes.extend(parts)
        base += -(-n // CLUSTER)
        del order
    return cid, [np.concatenate(x) for x in zip(*boxes)]


def box_gap(lo_a, hi_a, slo_a, shi_a, lo_b, hi_b, slo_b, shi_b):
    """Euclidean gap between light-space AABBs (broadcasting)."""
    g = np.maximum(np.maximum(lo_a - hi_b, lo_b - hi_a), 0)
    gs = np.maximum(np.maximum(slo_a - shi_b, slo_b - shi_a), 0)
    return np.sqrt((g ** 2).sum(-1) + gs ** 2)


def build_map(tv, lo, hi, smax, origin, nx, ny, tau):
    """Conservative maps from candidate triangles tv (their boxes lo, hi, smax).

    top: per texel, the max of each triangle's plane at the texel's corners, clamped to its
    top. No surface over the texel is higher, so a ray entering the column above it is lit
    there. cover_lo, cover_hi: the plane's height range over the texel for the topmost
    triangle whose projection contains the whole texel (the max cover_lo). A ray entering
    the column below cover_lo and leaving it above cover_hi crosses that triangle."""
    top = np.full(nx * ny, -np.inf)
    cover_lo = np.full(nx * ny, -np.inf)
    cover_hi = np.full(nx * ny, np.inf)
    ix0 = np.clip(np.floor((lo[:, 0] - origin[0]) / tau).astype(np.int64), 0, nx - 1)
    ix1 = np.clip(np.floor((hi[:, 0] - origin[0]) / tau).astype(np.int64), 0, nx - 1)
    iy0 = np.clip(np.floor((lo[:, 1] - origin[1]) / tau).astype(np.int64), 0, ny - 1)
    iy1 = np.clip(np.floor((hi[:, 1] - origin[1]) / tau).astype(np.int64), 0, ny - 1)
    t = tv.astype(float)
    e1, e2 = t[:, 1] - t[:, 0], t[:, 2] - t[:, 0]
    det = e1[:, 0] * e2[:, 1] - e1[:, 1] * e2[:, 0]
    flat = np.abs(det) < 1e-16
    sign = np.sign(det)
    det = np.where(flat, 1.0, det)
    gx = np.where(flat, 0.0, (e1[:, 2] * e2[:, 1] - e2[:, 2] * e1[:, 1]) / det)
    gy = np.where(flat, 0.0, (e2[:, 2] * e1[:, 0] - e1[:, 2] * e2[:, 0]) / det)
    smax_ = smax.astype(float) + 1e-9
    cx, cy = ix1 - ix0 + 1, iy1 - iy0 + 1
    cnt = cx * cy
    ends = np.cumsum(cnt)

    def chunks():
        begin = 0
        while begin < len(tv):
            base = ends[begin - 1] if begin else 0
            end = max(int(np.searchsorted(ends, base + (1 << 20), "right")), begin + 1)
            sl = slice(begin, end)
            rep = np.repeat(np.arange(begin, end), cnt[sl])
            local = np.arange(len(rep)) - np.repeat(ends[sl] - cnt[sl] - base, cnt[sl])
            tx = ix0[rep] + local % cx[rep]
            ty = iy0[rep] + local // cx[rep]
            x = origin[0] + (tx + 0.5) * tau
            y = origin[1] + (ty + 0.5) * tau
            mid = t[rep, 0, 2] + gx[rep] * (x - t[rep, 0, 0]) + gy[rep] * (y - t[rep, 0, 1])
            half = (np.abs(gx[rep]) + np.abs(gy[rep])) * tau / 2
            # The texel lies inside the triangle when every edge function, at its minimum over
            # the square (affine, so centre minus half-width times |gradient|), keeps the sign.
            inside = ~flat[rep]
            for i, j in ((0, 1), (1, 2), (2, 0)):
                ex, ey = t[rep, j, 0] - t[rep, i, 0], t[rep, j, 1] - t[rep, i, 1]
                edge = ex * (y - t[rep, i, 1]) - ey * (x - t[rep, i, 0])
                inside &= sign[rep] * edge - (np.abs(ex) + np.abs(ey)) * tau / 2 > 1e-14
            yield rep, tx * ny + ty, mid, half, inside
            begin = end

    for rep, cell, mid, half, inside in chunks():
        val = np.where(flat[rep], smax_[rep], np.minimum(smax_[rep], mid + half + 1e-9))
        np.maximum.at(top, cell, val)
        np.maximum.at(cover_lo, cell[inside], (mid - half)[inside])
    for rep, cell, mid, half, inside in chunks():
        best = inside & ((mid - half) == cover_lo[cell])
        np.minimum.at(cover_hi, cell[best], (mid + half)[best])
    return top, cover_lo, cover_hi


def walk(m, nx, ny, origin, tau, o, d, t0, cover=None):
    """Exact 2D DDA over the maps from P(t0) along the ray's light-space path.

    Returns (state, cells visited); state 0 is proved lit (the ray rises above every texel
    top it crosses), 1 proved blocked (a covered column, needs cover), 2 unsure (some top
    reaches the ray). Without cover the walk stops at the first unsure texel."""
    n = len(o)
    p = o + t0[:, None] * d
    cell = np.floor((p[:, :2] - origin) / tau).astype(np.int64)
    step = np.where(d[:, :2] > 0, 1, -1)
    with np.errstate(divide="ignore", invalid="ignore"):
        bound = origin + (cell + (d[:, :2] > 0)) * tau
        tnext = np.where(d[:, :2] != 0, (bound - p[:, :2]) / d[:, :2], np.inf) + t0[:, None]
        delta = np.where(d[:, :2] != 0, tau / np.abs(d[:, :2]), np.inf)
    tcur = t0.copy()
    mmax = float(m.max()) if len(m) else -np.inf
    state = np.zeros(n, np.int8)
    unsure = np.zeros(n, bool)
    steps = np.zeros(n, np.int32)
    active = np.arange(n)
    while len(active):
        a = active
        cx, cy = cell[a, 0], cell[a, 1]
        h = o[a, 2] + tcur[a] * d[a, 2]
        inside = (cx >= 0) & (cx < nx) & (cy >= 0) & (cy < ny)
        done = ~inside | (h > mmax)
        idx = np.where(inside, cx * ny + cy, 0)
        steps[a] += inside
        reach = ~done & (m[idx] >= h)
        if cover is None:
            proof = np.zeros(len(a), bool)
            unsure[a[reach]] = True
            stop = done | reach
        else:
            with np.errstate(invalid="ignore"):
                hout = o[a, 2] + np.minimum(tnext[a, 0], tnext[a, 1]) * d[a, 2]
            proof = ~done & (cover[0][idx] > h) & (cover[1][idx] < hout)
            unsure[a[reach & ~proof]] = True
            stop = done | proof
        state[a[proof]] = 1
        rest = a[~stop]
        if not len(rest):
            break
        axis = 1 - (tnext[rest, 0] <= tnext[rest, 1]).astype(np.int64)  # 0 when x comes first
        tcur[rest] = tnext[rest, axis]
        cell[rest, axis] += step[rest, axis]
        tnext[rest, axis] += delta[rest, axis]
        active = rest[np.isfinite(tcur[rest])]
    state[(state == 0) & unsure] = 2
    return state, steps


def far_states(tv, lo, hi, smax, q, d, reach, tau, rmax, texels, skipped):
    """Far-segment states (R, n, RAYS) and cells visited for receivers q (n,3), directions
    d (n,RAYS,3), at texel tau; then the coarse max-top walk's unsure flags and cells.

    The map region is the candidates' extent and the rays' starts, within the reach. A
    ray's lateral path is a line, so once it leaves that box it never re-enters it. A
    region over MAP_LIMIT splits the receivers in half; a lone receiver over it is scored
    unsure and listed in skipped."""
    n = len(q)
    shape = (len(R_MM), n, RAYS)
    tlo, thi = q[:, :2].min(0), q[:, :2].max(0)
    low = np.maximum(tlo - reach, np.minimum(tlo - 2 * rmax * SLOPE, lo.min(0))) - tau
    high = np.minimum(thi + reach, np.maximum(thi + 2 * rmax * SLOPE, hi.max(0))) + tau
    nx = int(np.ceil((high[0] - low[0]) / tau)) + 1
    ny = int(np.ceil((high[1] - low[1]) / tau)) + 1
    if nx * ny > MAP_LIMIT and n == 1:
        skipped.append(nx * ny)
        return (np.full(shape, 2, np.int8), np.zeros(shape, np.int32), np.ones(shape, bool),
                np.zeros(shape, np.int32))
    out = (np.empty(shape, np.int8), np.empty(shape, np.int32), np.empty(shape, bool),
           np.empty(shape, np.int32))
    if nx * ny > MAP_LIMIT:
        order = np.argsort(q[:, int(thi[0] - tlo[0] < thi[1] - tlo[1])])
        for half in (order[:n // 2], order[n // 2:]):
            parts = far_states(tv, lo, hi, smax, q[half], d[half], reach, tau, rmax, texels, skipped)
            for full, part in zip(out, parts):
                full[:, half] = part
        return out
    top, cov_lo, cov_hi = build_map(tv, lo, hi, smax, low, nx, ny, tau)
    texels.append(nx * ny)
    cm, cnx, cny = coarse(top, nx, ny)
    o3, d3 = np.repeat(q, RAYS, 0), d.reshape(-1, 3)
    for ri, rr in enumerate(R_MM):
        start = np.full(len(o3), rr / 1000)
        fs, st = walk(top, nx, ny, low, tau, o3, d3, start, (cov_lo, cov_hi))
        out[0][ri], out[1][ri] = fs.reshape(-1, RAYS), st.reshape(-1, RAYS)
        fs, st = walk(cm, cnx, cny, low, tau * COARSE, o3, d3, start)
        out[2][ri], out[3][ri] = (fs == 2).reshape(-1, RAYS), st.reshape(-1, RAYS)
    return out


def coarse(m, nx, ny):
    """COARSE x COARSE max-pool of a map."""
    cx, cy = -(-nx // COARSE), -(-ny // COARSE)
    pad = np.full((cx * COARSE, cy * COARSE), -np.inf)
    pad[:nx, :ny] = m.reshape(nx, ny)
    return pad.reshape(cx, COARSE, cy, COARSE).max((1, 3)).ravel(), cx, cy


def expand(mesh_rings, src, targets, keep_sizes):
    """Breadth-first vertex rings from triangle src of one mesh, given its welded corners,
    the triangles of each welded vertex and their starts. Returns each target's ring (or
    BEYOND) and, with keep_sizes, the |ring<=k| sizes for k = 0..K_MAX."""
    corners, tri_of, vstart = mesh_rings
    seen = np.array([src])
    frontier = seen
    out = np.full(len(targets), BEYOND, np.int16)
    out[targets == src] = 0
    size = [1]
    for level in range(1, K_MAX + 1):
        if (out != BEYOND).all() and not keep_sizes:
            break
        v = np.unique(corners[frontier].ravel())
        ln = vstart[v + 1] - vstart[v]
        idx = np.repeat(vstart[v], ln) + np.arange(ln.sum()) - np.repeat(np.cumsum(ln) - ln, ln)
        frontier = np.setdiff1d(tri_of[idx], seen)
        seen = np.union1d(seen, frontier)
        out[(out == BEYOND) & np.isin(targets, frontier)] = level
        size.append(len(seen))
    return out, size


def rings(source, offsets, own, own_place, nh_rec, nh_tri, sample):
    """Vertex k-ring distance (0..K_MAX, BEYOND, CROSS) from each near hit's receiver
    triangle to its hit triangle; and ring sizes |ring<=k| for the sampled receivers."""
    dist = np.full(len(nh_rec), BEYOND, np.int16)
    hit_place = np.searchsorted(offsets, nh_tri, "right") - 1
    dist[hit_place != own_place[nh_rec]] = CROSS
    sizes = np.zeros((len(sample), K_MAX + 1), np.int64)
    for k, (mesh, _) in enumerate(source.placements):
        need = np.nonzero((dist == BEYOND) & (own_place[nh_rec] == k))[0]
        smp = np.nonzero(own_place[sample] == k)[0]
        if not len(need) and not len(smp):
            continue
        (_, vertices, indices), = source.meshes[mesh]
        bits = np.ascontiguousarray(vertices[:, :3]).view(np.uint32)
        order = np.lexsort((bits[:, 2], bits[:, 1], bits[:, 0]))
        sb = bits[order]
        new = np.r_[True, (sb[1:] != sb[:-1]).any(1)]
        weld = np.empty(len(bits), np.int32)
        weld[order] = np.cumsum(new) - 1
        del bits, order, sb, new
        wt_ = weld[indices]
        del weld
        flat = wt_.ravel()
        vorder = np.argsort(flat, kind="stable")
        tri_of = (vorder // 3).astype(np.int32)
        vstart = np.r_[0, np.cumsum(np.bincount(flat, minlength=int(flat.max()) + 1))]
        del vorder, flat
        mesh_rings = (wt_, tri_of, vstart)
        rec = nh_rec[need]
        for r in np.unique(rec):
            sel = need[rec == r]
            out, _ = expand(mesh_rings, own[r] - offsets[k], nh_tri[sel] - offsets[k], False)
            dist[sel] = out
        for i in smp:
            _, size = expand(mesh_rings, own[sample[i]] - offsets[k], np.zeros(0, np.int64), True)
            sizes[i, :len(size)] = size
        del wt_, tri_of, vstart, mesh_rings
    return dist, sizes


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--r1", type=Path, default=Path("E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p0"))
    ap.add_argument("--asset", type=Path, default=Path(
        "D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla"))
    ap.add_argument("--tiles", type=int, default=300)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    hit = np.fromfile(args.r1 / "pose-0.hit.u32", "<u4").reshape(H, W, 4)
    cover = hit[..., 0] > 0
    source = nrt.probe.Source(args.asset)
    ys, xs, point, origin, eps, own_w, normal = nrt.receivers(source, hit, cover)
    sizes = [len(source.meshes[mesh][0][2]) for mesh, _ in source.placements]
    offsets = np.r_[0, np.cumsum(sizes)]
    own_place = (hit[ys, xs, 0].astype(np.int64) - 1)
    own = offsets[own_place] + hit[ys, xs, 1].astype(np.int64)
    del hit
    u, v, w = nrt.basis()
    m = np.stack([u, v, w], 1)
    centre = point.mean(0)
    lo_ = (origin - centre) @ m
    own_l = (own_w - centre) @ m
    nl = normal @ nrt.TO_LIGHT
    del point, origin, own_w, normal
    lo, hi, smin, smax = bounds(source, offsets, centre, m)
    grid = BoxGrid(lo, hi, smax)
    cid, (clo, chi, csmin, csmax, ccount) = clusters(offsets, lo, hi, smin, smax)
    T = int(offsets[-1])
    t_load = time.time() - t0
    print(f"loaded {T} triangles, {len(ccount)} clusters in {t_load:.0f} s; memory {memory()}", flush=True)
    guard()

    tile_id = (ys // TILE) * (W // TILE) + xs // TILE
    tiles = np.unique(tile_id)
    rng = np.random.default_rng(20261106)
    chosen = rng.choice(tiles, size=min(args.tiles, len(tiles)), replace=False)
    smax_all = float(smax.max())
    rmax = max(R_MM) / 1000
    pad = 1.5 * max(TEXEL_MM) / 1000

    rec_all = np.concatenate([np.nonzero(tile_id == t)[0] for t in chosen])
    n = len(rec_all)
    slot = np.full(len(ys), -1, np.int64)
    slot[rec_all] = np.arange(n)
    tmin_hit = np.full((n, RAYS), np.inf)
    tmax_hit = np.full((n, RAYS), -np.inf)
    cone_n = np.zeros((len(R_MM), n), np.int32)
    hitn = np.zeros((len(R_MM), n), np.int32)
    fstate = np.zeros((len(TEXEL_MM), len(R_MM), n, RAYS), np.int8)
    steps = np.zeros((len(TEXEL_MM), len(R_MM), n, RAYS), np.int32)
    cmaybe = np.zeros((len(TEXEL_MM), len(R_MM), n, RAYS), bool)
    csteps = np.zeros((len(TEXEL_MM), len(R_MM), n, RAYS), np.int32)
    map_texels = {tau: [] for tau in TEXEL_MM}
    skipped = {tau: [] for tau in TEXEL_MM}
    nh_rec, nh_ray, nh_tri, nh_t = [], [], [], []
    pairs_total, tests_total, cand_sizes = 0, 0, []
    sample, ball_n = [], []
    t_tiles = time.time()
    for ti, tile in enumerate(chosen):
        r = np.nonzero(tile_id == tile)[0]
        k0 = slot[r]
        q = lo_[r]
        e = eps[r]
        d = directions(ys[r], xs[r])
        tlo, thi = q[:, :2].min(0), q[:, :2].max(0)
        reach = (smax_all - q[:, 2].min()) * SLOPE * (1 + 1e-6) + pad
        ids = grid.query(tlo - reach, thi + reach)
        height = smax[ids] - q[:, 2].min()
        gap = np.maximum(np.maximum(tlo - hi[ids], lo[ids] - thi), 0)
        ids = ids[(height > 0) & (np.hypot(gap[:, 0], gap[:, 1]) <= height * SLOPE * (1 + 1e-6) + pad)]
        cand_sizes.append(len(ids))
        lo_i, hi_i, smax_i = lo[ids], hi[ids], smax[ids]
        tv = light_tris(source, offsets, centre, m, ids)
        # Per-receiver cone filter over the candidate list.
        pr, pb, pdist = [], [], []
        step_ = max(1, (1 << 22) // max(1, len(ids)))
        for c in range(0, len(r), step_):
            qq, so = q[c:c + step_], q[c:c + step_, 2]
            hgt = smax_i[None] - so[:, None]
            g = np.maximum(np.maximum(lo_i[None] - qq[:, None, :2], qq[:, None, :2] - hi_i[None]), 0)
            dist = np.hypot(g[..., 0], g[..., 1])
            ok = (hgt > 0) & (dist <= hgt * SLOPE * (1 + 1e-6) + 1e-7)
            a, b = np.nonzero(ok)
            pr.append(a + c)
            pb.append(b)
            pdist.append(dist[a, b])
        pr, pb, pdist = np.concatenate(pr), np.concatenate(pb), np.concatenate(pdist)
        pt = ids[pb]
        pairs_total += len(pr)
        for ri, rr in enumerate(R_MM):
            rm = rr / 1000
            near = (smin[pt] < q[pr, 2] + rm) & (pdist <= np.minimum(smax[pt] - q[pr, 2], rm) * SLOPE * (1 + 1e-6) + 1e-7)
            cone_n[ri, k0] = np.bincount(pr[near], minlength=len(r))
        # Exact rays: every (receiver, candidate) pair against the receiver's 8 rays.
        rr_ = np.repeat(pr, RAYS)
        jj = np.tile(np.arange(RAYS), len(pr))
        tb = np.repeat(pb, RAYS)
        th = np.empty(len(rr_))
        for c in range(0, len(rr_), 1 << 19):
            s = slice(c, c + (1 << 19))
            th[s] = intersect(q[rr_[s]], d[rr_[s], jj[s]], tv[tb[s]], e[rr_[s]])
        tests_total += len(rr_)
        h = np.isfinite(th)
        rh, jh, tth, thh = rr_[h], jj[h], ids[tb[h]], th[h]
        flat_ray = rh * RAYS + jh
        tmn = np.full(len(r) * RAYS, np.inf)
        tmx = np.full(len(r) * RAYS, -np.inf)
        np.minimum.at(tmn, flat_ray, thh)
        np.maximum.at(tmx, flat_ray, thh)
        tmin_hit[k0] = tmn.reshape(-1, RAYS)
        tmax_hit[k0] = tmx.reshape(-1, RAYS)
        nk = thh <= NEAR_KEEP
        nh_rec.append(k0[rh[nk]])
        nh_ray.append(jh[nk])
        nh_tri.append(tth[nk])
        nh_t.append(thh[nk])
        for ri, rr in enumerate(R_MM):
            sel = thh <= rr / 1000
            key = np.unique(rh[sel] * T + tth[sel])
            hitn[ri, k0] = np.bincount(key // T, minlength=len(r))
        # Far map: conservative max-top and cover maps, local to the reachable region.
        for xi, tau_mm in enumerate(TEXEL_MM):
            fstate[xi][:, k0], steps[xi][:, k0], cmaybe[xi][:, k0], csteps[xi][:, k0] = far_states(
                tv, lo_i, hi_i, smax_i, q, d, reach, tau_mm / 1000, rmax, map_texels[tau_mm], skipped[tau_mm])
        del tv, rr_, jj, tb, th
        # Sun-independent per-triangle ball lists for a sample of receivers.
        pick = r if len(r) <= SAMPLE else rng.choice(r, SAMPLE, replace=False)
        for g_ in pick:
            ol, oh = own_l[g_, :, :2].min(0), own_l[g_, :, :2].max(0)
            osl, osh = own_l[g_, :, 2].min(), own_l[g_, :, 2].max()
            cand = grid.query(ol - rmax, oh + rmax)
            gp = box_gap(ol, oh, osl, osh, lo[cand], hi[cand], smin[cand], smax[cand])
            ball_n.append([int((gp <= rr / 1000).sum()) for rr in R_MM])
            sample.append(slot[g_])
        if ti % 5 == 0:
            print(f"tile {ti}/{len(chosen)} cand {len(ids)} pairs {len(pr)} t={time.time() - t0:.0f}s mem {memory()}", flush=True)
            guard()
    t_rays = time.time() - t_tiles
    sample = np.array(sample)
    ball_n = np.array(ball_n)
    nh_rec, nh_ray, nh_tri, nh_t = (np.concatenate(x) for x in (nh_rec, nh_ray, nh_tri, nh_t))
    own_s, place_s, cid_own = own[rec_all], own_place[rec_all], cid[own[rec_all]]
    same_cluster = cid[nh_tri] == cid_own[nh_rec]
    # Cluster lists: whole clusters whose AABB lies within r of the receiver's cluster.
    ucl, inv = np.unique(cid_own[sample], return_inverse=True)
    clist = np.zeros((len(ucl), len(R_MM)), np.int64)
    for i, c in enumerate(ucl):
        gp = box_gap(clo[c], chi[c], csmin[c], csmax[c], clo, chi, csmin, csmax)
        clist[i] = [int(ccount[gp <= rr / 1000].sum()) for rr in R_MM]
    clist_n = clist[inv]
    cluster_size = ccount[cid_own[sample]]
    print(f"rays done in {t_rays:.0f} s; rings next; memory {memory()}", flush=True)
    del grid, lo, hi, smax, smin, cid, clo, chi, csmin, csmax
    t_r = time.time()
    ring, ring_sizes = rings(source, offsets, own_s, place_s, nh_rec, nh_tri, sample)
    t_rings = time.time() - t_r

    # Scoring.
    blocked = np.isfinite(tmin_hit)
    count = RAYS - blocked.sum(1)
    nl_s = nl[rec_all]
    pops = {
        "all": np.ones(n, bool),
        "lit": count == RAYS, "penumbra": (count > 0) & (count < RAYS), "umbra": count == 0,
        "facing": nl_s >= 0.26, "grazing_lit_side": (nl_s > 0) & (nl_s < 0.26), "back_to_sun": nl_s <= 0}
    flat_nh = nh_rec * RAYS + nh_ray

    def mark(sel):
        out = np.zeros(n * RAYS, bool)
        out[flat_nh[sel]] = True
        return out.reshape(n, RAYS)

    def rate(err, pop):
        rays = int(pop.sum()) * RAYS
        e = int(err[pop].sum())
        return dict(errors=e, rays=rays, rate=e / max(rays, 1))

    q = lambda a, qs=(.5, .9, .99, 1.0): [float(x) for x in np.quantile(a, qs)] if len(a) else []
    report = dict(
        method=__doc__, receivers=n, rays=n * RAYS, tiles=len(chosen), triangles=T,
        seconds=dict(load=round(t_load, 1), rays_and_maps=round(t_rays, 1), rings=round(t_rings, 1)),
        exact=dict(blocked_rays=int(blocked.sum()), pairs=int(pairs_total), ray_tests=int(tests_total),
                   candidates_per_tile=q(cand_sizes),
                   blocker_distance_mm=q(tmin_hit[blocked] * 1000, (.5, .9, .99)),
                   blocked_within_mm={str(r): int((tmin_hit <= r / 1000).sum()) for r in (1,) + R_MM},
                   receivers=dict((k, int(v.sum())) for k, v in pops.items())),
        near={}, far={})
    for ri, rr in enumerate(R_MM):
        rm = rr / 1000
        near_exact = tmin_hit <= rm
        far_exact = tmax_hit > rm
        within = nh_t <= rm
        cap = {}
        for k in range(K_MAX + 1):
            near_k = mark(within & (ring <= k))
            cap[f"ring{k}"] = dict(
                near_missed=rate(near_exact & ~near_k, pops["all"]),
                lost={p: rate(blocked & ~near_k & ~far_exact, pv) for p, pv in pops.items()})
        near_c = mark(within & same_cluster)
        cap["own_cluster"] = dict(near_missed=rate(near_exact & ~near_c, pops["all"]),
                                  lost={p: rate(blocked & ~near_c & ~far_exact, pv) for p, pv in pops.items()})
        rr_hits = within
        report["near"][str(rr)] = dict(
            cone_bbox_set=q(cone_n[ri]), cone_bbox_mean=float(cone_n[ri].mean()),
            hit_set=q(hitn[ri]), hit_set_mean=float(hitn[ri].mean()),
            near_blocked_rays=int(near_exact.sum()),
            near_hits_cross_placement=int((rr_hits & (ring == CROSS)).sum()),
            near_hits_beyond_ring=int((rr_hits & (ring == BEYOND)).sum()),
            near_hit_ring_hist={str(k): int((rr_hits & (ring == k)).sum()) for k in range(K_MAX + 1)},
            ball_list=q(ball_n[:, ri]), ball_list_mean=float(ball_n[:, ri].mean()),
            cluster_list=q(clist_n[:, ri]), cluster_list_mean=float(clist_n[:, ri].mean()),
            capture=cap)
        far = {}
        for xi, tau_mm in enumerate(TEXEL_MM):
            fs, cmb = fstate[xi, ri], cmaybe[xi, ri]
            need = ~near_exact
            unsure, proved = (fs == 2) & need, (fs == 1) & need
            far[str(tau_mm)] = dict(
                lost_far=int((far_exact & need & (fs == 0)).sum()),
                false_proof=int(((fs == 1) & ~far_exact).sum()),
                lost_far_coarse=int((far_exact & need & ~cmb).sum()),
                far_blocked_rays=int((far_exact & need).sum()),
                proved_blocked_rays=int(proved.sum()),
                unsure_as_blocked_false={p: rate(unsure & ~blocked, pv) for p, pv in pops.items()},
                unsure_as_lit_lost={p: rate(unsure & far_exact, pv) for p, pv in pops.items()},
                refine_rays=rate(unsure, pops["all"]),
                refine_receivers=int(unsure.any(1).sum()),
                maxtop_only_refine_rays=rate((proved | unsure), pops["all"]),
                coarse_refine_rays=rate(cmb & need, pops["all"]),
                steps_lit=q(steps[xi, ri][(fs == 0) & need]), steps_mean=float(steps[xi, ri][need].mean()),
                coarse_steps_mean=float(csteps[xi, ri][need].mean()),
                local_map_texels=q(map_texels[tau_mm]), receivers_over_map_limit=len(skipped[tau_mm]))
        report["far"][str(rr)] = far
    report["ring_sizes"] = {str(k): q(ring_sizes[:, k]) for k in range(K_MAX + 1)}
    report["ring_sizes_mean"] = {str(k): float(ring_sizes[:, k].mean()) for k in range(K_MAX + 1)}
    report["cluster_size"] = q(cluster_size)
    report["memory_gib"] = dict(zip(("private", "peak_private", "peak_working_set", "available"), memory()))
    report["seconds_total"] = round(time.time() - t0, 1)
    (args.out / "n5.json").write_text(json.dumps(report, indent=1))
    np.savez_compressed(args.out / "rays.npz", tmin_hit=tmin_hit.astype(np.float32),
                        tmax_hit=tmax_hit.astype(np.float32), nl=nl_s.astype(np.float32),
                        nh_rec=nh_rec, nh_ray=nh_ray.astype(np.int8), nh_t=nh_t.astype(np.float32),
                        ring=ring, same_cluster=same_cluster)
    print(json.dumps({k: report[k] for k in ("receivers", "seconds", "exact", "memory_gib", "seconds_total")}, indent=1))


if __name__ == "__main__":
    main()
