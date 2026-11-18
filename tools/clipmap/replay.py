"""CPU replay of the sun's visibility over a dumped clipmap level (RENDERLAB_VD_DUMP).

For each receiver pixel in a crop: the exact visible fraction of the sun's disk over the dumped
heightfield of one level. N directions spread uniformly over the disk; each ray is marched with
an exact texel-boundary DDA through box columns (a texel holds its top height, solid below it);
a ray is blocked where it enters a column whose top is at or above the ray's entry height and
above the receiver's tangent plane by VD_BIAS texels (the shader's vd_above). The receiver's
origin is the shader's: the world point moved along the viewed side's normal by world.w.

usage: py -3 replay.py <dump raw dir> <frame> <level> <x0> <y0> <x1> <y1> <out.npy> [directions]
"""
import sys
import numpy as np

raw, frame, level = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
x0, y0, x1, y1 = map(int, sys.argv[4:8])
out = sys.argv[8]
N = int(sys.argv[9]) if len(sys.argv) > 9 else 1024
BIAS = 1.0
import os
# Column thickness in texels of the dumped level (the Disk lookup's least thickness, VD_THICK):
# a texel blocks a ray only where the ray's height inside it overlaps [top - THICK texels, top].
THICK = float(os.environ.get("THICK", "4"))

lines = [l.split() for l in open(f"{raw}/levels-{frame}.txt")]
def numeric(t):
    try:
        float(t)
        return True
    except ValueError:
        return False


levels = [tuple(map(float, x)) for x in lines if len(x) == 3 and numeric(x[0])]
kv = {x[0]: np.array([float(t) for t in x[1:]]) for x in lines if len(x) > 1 and not numeric(x[0])}
eye, u, v, l, sun = kv["eye"], kv["u"], kv["v"], kv["l"], kv["sun"]
disk = float(kv["disk"][0])
low, rng = kv["depth"]
side, count = int(kv["layout"][0]), int(kv["layout"][1])
ox, oy, texel = levels[level]
# The receiver bias is one texel of the receiver's level (the dumped level), as main's lookup.
bias_texel = texel

top = np.fromfile(f"{raw}/top-{frame}.bin", dtype=np.uint32).reshape(-1, 2)
words = top[level * side * side:(level + 1) * side * side].reshape(side, side, 2)
occupied = words[..., 0] != 0
height = np.where(occupied, words[..., 1].view(np.float32).astype(np.float64) * rng + low, -np.inf)

# Optional refinement: REFINE=k rebuilds the heightfield k times finer near the occluder's straight
# outline (fitted to the level's staircase), each fine texel taking the occluder's height or the
# floor's by which side of the line its centre falls (as the GPU raster samples texel centres).
import os
REFINE = int(os.environ.get("REFINE", "1"))
if os.environ.get("REBUILD") == "1":
    # The occluder's straight outline in light space (REF_LINE file: a point and the normal toward
    # the occluded side), fitted to the shadow's 50% contour over this map. Within three coarse
    # texels of it, texels k times finer take the occluder's or the floor's height by which side
    # their centre falls (as the GPU raster samples texel centres); elsewhere the coarse heights stand.
    lm0, lm1, ln0, ln1 = map(float, open(os.environ["REF_LINE"]).read().split())
    Wd = np.fromfile(f"{raw}/world-{frame}-1920x1080.bin", dtype=np.float32).reshape(1080, 1920, 4).astype(np.float64)
    q = Wd[y0:y1, x0:x1, :3].reshape(-1, 3)
    lx = (q - eye) @ u; ly = (q - eye) @ v
    a0 = int((lx.min() - ox) / texel) - 8; a1 = int((lx.max() - ox) / texel) + 8
    b0 = int((ly.min() - oy) / texel) - 8; b1 = int((ly.max() - oy) / texel) + 8
    blk = height[b0:b1, a0:a1]
    by, bx = np.mgrid[b0:b1, a0:a1]
    cc = np.stack([ox + (bx + 0.5) * texel, oy + (by + 0.5) * texel], -1)
    sd = (cc - [lm0, lm1]) @ [ln0, ln1]
    band = np.abs(sd) < 6 * texel
    # The occluder's top and the floor are both sloped in light space: a plane z = a + b x + c y
    # fitted to each side's texels near the line (those standing clear of the other side's height).
    mid = np.nanmedian(blk[band])
    def plane(sel):
        A = np.stack([np.ones(sel.sum()), cc[..., 0][sel], cc[..., 1][sel]], 1)
        return np.linalg.lstsq(A, blk[sel], rcond=None)[0]
    pt = plane(band & (sd > 0) & (blk > mid))
    pf = plane(band & (sd < 0) & (blk < mid))
    top_z = np.median(blk[band & (sd > 0) & (blk > mid)]); floor_z = np.median(blk[band & (sd < 0) & (blk < mid)])
    k = REFINE
    fh, fw = (b1 - b0) * k, (a1 - a0) * k
    fy, fx = np.mgrid[0:fh, 0:fw]
    t2 = texel / k
    fc = np.stack([ox + a0 * texel + (fx + 0.5) * t2, oy + b0 * texel + (fy + 0.5) * t2], -1)
    fsd = (fc - [lm0, lm1]) @ [ln0, ln1]
    coarse = np.repeat(np.repeat(blk, k, 0), k, 1)
    zt = pt[0] + pt[1] * fc[..., 0] + pt[2] * fc[..., 1]
    zf = pf[0] + pf[1] * fc[..., 0] + pf[2] * fc[..., 1]
    fine = np.where(np.abs(fsd) < 3 * texel, np.where(fsd > 0, zt, zf), coarse)
    print("rebuilt", k, "occluder %.3f floor %.3f" % (top_z, floor_z))
    ox, oy, texel = ox + a0 * texel, oy + b0 * texel, t2
    side = max(fh, fw)
    height = np.full((side, side), -np.inf); height[:fh, :fw] = fine

# Optional silhouette reconstructions on the dumped level (SIL=line|frac, REF_LINE as above):
# texels the occluder's straight outline crosses are silhouette texels. `line`: each stores the
# outline (a silhouette map) and a ray is tested against it, blocked where its path on the occluded
# side passes under the occluder's surface (the side planes fitted from the map's own texels).
# `frac`: each stores the share of its square on the occluded side, and a ray passing under its
# top is transmitted by one minus that share.
SIL = os.environ.get("SIL", "")
if SIL:
    lm0, lm1, ln0, ln1 = map(float, open(os.environ["REF_LINE"]).read().split())
    Wd = np.fromfile(f"{raw}/world-{frame}-1920x1080.bin", dtype=np.float32).reshape(1080, 1920, 4).astype(np.float64)
    q = Wd[y0:y1, x0:x1, :3].reshape(-1, 3)
    lx = (q - eye) @ u; ly = (q - eye) @ v
    a0 = int((lx.min() - ox) / texel) - 8; a1 = int((lx.max() - ox) / texel) + 8
    b0 = int((ly.min() - oy) / texel) - 8; b1 = int((ly.max() - oy) / texel) + 8
    blk = height[b0:b1, a0:a1]
    by, bx = np.mgrid[b0:b1, a0:a1]
    cc = np.stack([ox + (bx + 0.5) * texel, oy + (by + 0.5) * texel], -1)
    sd = (cc - [lm0, lm1]) @ [ln0, ln1]
    band = np.abs(sd) < 6 * texel
    mid = np.nanmedian(blk[band])
    def plane(sel):
        A = np.stack([np.ones(sel.sum()), cc[..., 0][sel], cc[..., 1][sel]], 1)
        return np.linalg.lstsq(A, blk[sel], rcond=None)[0]
    pt = plane(band & (sd > 0) & (blk > mid))
    pf = plane(band & (sd < 0) & (blk < mid))
    reach = 0.5 * texel * (abs(ln0) + abs(ln1))
    sil = np.zeros((side, side), bool)
    sil[b0:b1, a0:a1] = band & (np.abs(sd) < reach)
    # Covered share of each silhouette texel (16 x 16 subsamples).
    cover = np.zeros((side, side), np.float32)
    g = (np.arange(16) + 0.5) / 16 - 0.5
    gy_, gx_ = np.meshgrid(g, g, indexing="ij")
    for jy, jx in zip(*np.where(sil[b0:b1, a0:a1])):
        ccx, ccy = cc[jy, jx]
        sub = ((ccx + gx_ * texel - lm0) * ln0 + (ccy + gy_ * texel - lm1) * ln1) > 0
        cover[b0 + jy, a0 + jx] = sub.mean()
    print("silhouette texels", sil.sum(), "mode", SIL)

W = 1920
world = np.fromfile(f"{raw}/world-{frame}-1920x1080.bin", dtype=np.float32).reshape(1080, W, 4).astype(np.float64)
normal = np.fromfile(f"{raw}/normal-{frame}-1920x1080.bin", dtype=np.float32).reshape(1080, W, 4).astype(np.float64)


def light(p):
    q = p - eye
    return np.array([q @ u, q @ v, p @ l])


# Disk directions in light space, per unit height: uniform over the disk of angular radius `disk`.
s = sun / np.linalg.norm(sun)
e1 = np.cross(s, [0.0, 0.0, 1.0] if abs(s[2]) < 0.9 else [1.0, 0.0, 0.0])
e1 /= np.linalg.norm(e1)
e2 = np.cross(s, e1)
k = np.arange(N) + 0.5
r = np.sqrt(k / N) * np.tan(disk)
a = k * 2.399963229728653
dirs = s[None, :] + r[:, None] * (np.cos(a)[:, None] * e1 + np.sin(a)[:, None] * e2)
dl = np.stack([dirs @ u, dirs @ v, dirs @ l], 1)
D = dl[:, :2] / dl[:, 2:3]  # light xy per unit of height along each direction

result = np.full((y1 - y0, x1 - x0), np.nan)
for py in range(y0, y1):
    for px in range(x0, x1):
        w = world[py, px]
        n = normal[py, px, :3]
        if not np.isfinite(w).all() or np.linalg.norm(n) == 0:
            continue
        toward = eye - w[:3]
        n = n if n @ toward >= 0 else -n
        origin = w[:3] + n * w[3]
        p = light(origin)
        point = light(w[:3])
        nl = np.array([n @ u, n @ v, n @ l])
        margin = BIAS * bias_texel
        # Directions under the receiver's tangent plane are blocked.
        open_ = (D @ nl[:2] + nl[2]) > 0
        # DDA over the texel grid from p.xy along D, heights h from 0 to the ceiling.
        gx = (p[0] - ox) / texel
        gy = (p[1] - oy) / texel
        ix = np.full(N, int(np.floor(gx)))
        iy = np.full(N, int(np.floor(gy)))
        dx, dy = D[:, 0] / texel, D[:, 1] / texel  # texels per unit height
        stepx = np.where(dx > 0, 1, -1)
        stepy = np.where(dy > 0, 1, -1)
        with np.errstate(divide="ignore"):
            tx = np.where(dx != 0, ((ix + (stepx > 0)) - gx) / dx, np.inf)
            ty = np.where(dy != 0, ((iy + (stepy > 0)) - gy) / dy, np.inf)
            ddx = np.where(dx != 0, 1.0 / np.abs(dx), np.inf)
            ddy = np.where(dy != 0, 1.0 / np.abs(dy), np.inf)
        h_in = np.zeros(N)
        blocked = ~open_
        trans = np.where(open_, 1.0, 0.0)
        ceiling = np.nanmax(height[max(int(gy) - 200, 0):int(gy) + 200, max(int(gx) - 200, 0):int(gx) + 200]) - p[2]
        live = ~blocked
        while live.any():
            inside = (ix >= 0) & (ix < side) & (iy >= 0) & (iy < side)
            live &= inside & (h_in <= ceiling)
            if not live.any():
                break
            li = np.where(live)[0]
            H = height[iy[li], ix[li]]
            c = np.stack([ox + (ix[li] + 0.5) * texel, oy + (iy[li] + 0.5) * texel, H], 1)
            above = ((c - point) @ nl) > margin
            h_out = np.minimum(tx[li], ty[li])
            hit = (H - p[2] >= h_in[li]) & (H - p[2] - THICK * bias_texel <= h_out) & above & np.isfinite(H)
            if SIL:
                sm = sil[iy[li], ix[li]]
                hit &= ~sm
                if sm.any():
                    si = li[sm]
                    h0 = h_in[si]; h1 = np.minimum(tx[si], ty[si])
                    xy0 = p[:2] + D[si] * h0[:, None]; xy1 = p[:2] + D[si] * h1[:, None]
                    s0 = (xy0 - [lm0, lm1]) @ [ln0, ln1]; s1 = (xy1 - [lm0, lm1]) @ [ln0, ln1]
                    if SIL == "line":
                        # The lowest point of the path on the occluded side.
                        t = np.where(s0 > 0, 0.0, np.clip(s0 / np.where(s0 - s1 != 0, s0 - s1, 1), 0, 1))
                        hs = h0 + t * (h1 - h0); xs = xy0 + (xy1 - xy0) * t[:, None]
                        zt = pt[0] + pt[1] * xs[:, 0] + pt[2] * xs[:, 1]
                        cpt = np.stack([xs[:, 0], xs[:, 1], zt], 1)
                        ok = (np.maximum(s0, s1) > 0) & (zt - p[2] >= hs) & (zt - p[2] - THICK * bias_texel <= h1) & (((cpt - point) @ nl) > margin)
                        blocked[si[ok]] = True; live[si[ok]] = False; trans[si[ok]] = 0.0
                    else:
                        cx_ = ox + (ix[si] + 0.5) * texel; cy_ = oy + (iy[si] + 0.5) * texel
                        zt = pt[0] + pt[1] * cx_ + pt[2] * cy_
                        cpt = np.stack([cx_, cy_, zt], 1)
                        ok = (zt - p[2] >= h0) & (zt - p[2] - THICK * bias_texel <= h1) & (((cpt - point) @ nl) > margin)
                        trans[si[ok]] *= 1.0 - cover[iy[si[ok]], ix[si[ok]]]
            blocked[li[hit]] = True
            live[li[hit]] = False
            trans[li[hit]] = 0.0
            # Advance to the next texel boundary.
            gox = tx[li] <= ty[li]
            a_ = li[gox]; b_ = li[~gox]
            h_in[a_] = tx[a_]; ix[a_] += stepx[a_]; tx[a_] += ddx[a_]
            h_in[b_] = ty[b_]; iy[b_] += stepy[b_]; ty[b_] += ddy[b_]
        result[py - y0, px - x0] = trans.mean()
np.save(out, result)
print("saved", out, np.nanmin(result), np.nanmax(result))
