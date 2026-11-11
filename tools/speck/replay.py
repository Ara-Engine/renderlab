"""U-SPECK CPU replay of the solid-angle hypothesis (no GPU): do 64 fixed gather directions per
ground cell, against the arch lamp's lit bulb and lintel, make the blobs? And does aiming a
few rays around the lamp remove them without losing light?

The game scene's arch in its own frame (examples/game_scene.rs): ground y = 0; the bulb, a
white sphere of radius 0.12 m at (0, 3.55, 0); its point light 0.25 m below it; the lintel's
underside at y = 4.0, 5.2 x 1.0 m. Each ground cell gathers as shaders/gi_cache.wgsl does: 64
equal-solid-angle hemisphere directions (gi_direction) turned about the normal by a per-cell
angle, each hit's Lambertian radiance under the point light times 2 pi / 64 into an L1 SH,
read back as irradiance (a delta of weight w reads w (0.25 + 0.5 cos)), then the 3x3 tent
(1, 0.5, 0.25) of the cell filter. The hit is shaded at the exact hit point, not at a cell's
query point, so amplitudes are indicative.

Prints, per run (64 directions; `--near M`: the fix under test) and source (bulb, lintel):
the share of cells whose gather hits it, the mean bounce against a 4096-direction estimate of
the same integral, and the error after the filter. `--out PNG` writes the filtered bounce
maps (64 directions | with `--near M` | 4096 directions), white at `--white` lux.

    py -3 -B tools/speck/replay.py [--cell 0.06] [--half 6.0] [--near 16] [--out PNG] [--white 600]
"""
import argparse

import numpy as np

BULB = np.array([0.0, 3.55, 0.0])
BULB_R = 0.12
LIGHT = np.array([0.0, 3.30, 0.0])
# Luminance of the lamp's (30000, 21000, 11000) cd.
CANDELA = 0.2126 * 30000 + 0.7152 * 21000 + 0.0722 * 11000
LINTEL_Y, LINTEL_X, LINTEL_Z = 4.0, 2.6, 0.5
ALBEDO_BULB, ALBEDO_LINTEL = 0.9, 0.7
# The lamp's source radius, and the near region's radius over the cell's distance to the lamp.
SOURCE_R, KAPPA = 0.12, 1.0 / 3.0
GOLDEN = 2.399963229728653


def directions(count, rotation):
    """gi_direction about +y: (cells, count, 3)."""
    k = np.arange(count)
    z = 1.0 - (k + 0.5) / count
    r = np.sqrt(np.maximum(1.0 - z * z, 0.0))
    phi = k[None, :] * GOLDEN + rotation[:, None]
    return np.stack([r * np.cos(phi), np.broadcast_to(z, phi.shape), r * np.sin(phi)], axis=-1)


def lit(points, normals):
    """Lambert irradiance (lux) of the point lamp at points with normals."""
    to = LIGHT - points
    dist2 = (to * to).sum(-1)
    cos = np.maximum((to * normals).sum(-1), 0.0) / np.sqrt(dist2)
    return CANDELA * cos / np.maximum(dist2, 1e-4)


def trace(o, d):
    """Nearest hit of rays (cells, rays, 3) on the bulb, then the lintel's underside: the hit's
    Lambertian radiance under the lamp (cd/m2) per source, hit flags and the hit points."""
    oc = o - BULB
    b = (oc * d).sum(-1)
    disc = b * b - ((oc * oc).sum(-1) - BULB_R * BULB_R)
    t = -b - np.sqrt(np.maximum(disc, 0.0))
    hit_bulb = (disc > 0.0) & (t > 0.0)
    p = o + d * t[..., None]
    bulb = np.where(hit_bulb, ALBEDO_BULB / np.pi * lit(p, (p - BULB) / BULB_R), 0.0)
    # Lintel underside (facing down), behind the bulb where the bulb is hit first.
    tl = (LINTEL_Y - o[..., 1]) / np.maximum(d[..., 1], 1e-9)
    pl = o + d * tl[..., None]
    hit_lintel = (np.abs(pl[..., 0]) < LINTEL_X) & (np.abs(pl[..., 2]) < LINTEL_Z) & ~hit_bulb & (d[..., 1] > 0.0)
    e = lit(pl, np.broadcast_to(np.array([0.0, -1.0, 0.0]), pl.shape))
    # The bulb's shadow on the lintel: the light's ray to the point passes the sphere.
    seg = pl - LIGHT
    seg_len = np.linalg.norm(seg, axis=-1)
    u = seg / seg_len[..., None]
    lc = LIGHT - BULB
    bb = (lc * u).sum(-1)
    dd = bb * bb - ((lc * lc).sum() - BULB_R * BULB_R)
    root = np.sqrt(np.maximum(dd, 0.0))
    shadow = (dd > 0.0) & (-bb - root < seg_len) & (-bb + root > 0.0)
    lintel = np.where(hit_lintel & ~shadow, ALBEDO_LINTEL / np.pi * e, 0.0)
    return bulb, lintel, hit_bulb, hit_lintel, np.where(hit_bulb[..., None], p, pl)


def gather(origins, rotation, count, near=0):
    """Per-cell bounce irradiance (lux) from the bulb and from the lintel, and hit flags.

    near = M > 0 is the fix under test: the regular directions drop hits closer to the lamp
    than a third of the cell's distance to it, and M + 1 rays aimed around the lamp take them
    instead, their angle from the lamp log-uniform (density 1 / angle^2 per solid angle, which
    cancels the lit surfaces' 1 / r^2), each weighted by 1 / (M pdf)."""
    d = directions(count, rotation)
    o = origins[:, None, :]
    bulb, lintel, hit_bulb, hit_lintel, point = trace(o, d)
    # L1 SH read as irradiance about +y: a delta of weight w gives w (0.25 + 0.5 cos).
    lobe = 2.0 * np.pi / count * (0.25 + 0.5 * d[..., 1])
    to = LIGHT - origins
    dist = np.linalg.norm(to, axis=-1)
    if near:
        keep = np.linalg.norm(point - LIGHT, axis=-1) >= KAPPA * dist[:, None]
        bulb, lintel = bulb * keep, lintel * keep
    out_b = (bulb * lobe).sum(-1)
    out_l = (lintel * lobe).sum(-1)
    if near:
        c = to / dist[:, None]
        # A frame about the lamp's direction.
        up = np.where(np.abs(c[:, 1:2]) < 0.9, np.array([0.0, 1.0, 0.0]), np.array([1.0, 0.0, 0.0]))
        t1 = np.cross(up, c)
        t1 /= np.linalg.norm(t1, axis=-1, keepdims=True)
        t2 = np.cross(c, t1)
        psi_max = np.arcsin(KAPPA)
        psi_min = np.minimum(0.25 * SOURCE_R / dist, 0.5 * psi_max)
        log = np.log(psi_max / psi_min)
        s = np.arange(near)
        psi = psi_min[:, None] * np.exp(log[:, None] * (s[None, :] + 0.5) / near)
        phi = s[None, :] * GOLDEN + rotation[:, None]
        w = 2.0 * np.pi * np.sin(psi) * psi * log[:, None] / near
        # The cap inside psi_min: one ray at the lamp itself.
        zero = np.zeros((len(origins), 1))
        psi = np.concatenate([psi, zero], axis=1)
        phi = np.concatenate([phi, zero], axis=1)
        w = np.concatenate([w, (2.0 * np.pi * (1.0 - np.cos(psi_min)))[:, None]], axis=1)
        dn = (c[:, None, :] * np.cos(psi)[..., None]
              + (t1[:, None, :] * np.cos(phi)[..., None] + t2[:, None, :] * np.sin(phi)[..., None]) * np.sin(psi)[..., None])
        nb, nl, _, _, npoint = trace(o, dn)
        inside = (np.linalg.norm(npoint - LIGHT, axis=-1) < KAPPA * dist[:, None]) & (dn[..., 1] > 0.0)
        nlobe = w * (0.25 + 0.5 * dn[..., 1]) * inside
        out_b = out_b + (nb * nlobe).sum(-1)
        out_l = out_l + (nl * nlobe).sum(-1)
    return out_b, out_l, hit_bulb.any(-1), hit_lintel.any(-1)


def tent(a):
    """The cell filter's 3x3 tent: 1 centre, 0.5 edges, 0.25 corners."""
    p = np.pad(a, 1, mode="edge")
    k = np.array([[0.25, 0.5, 0.25], [0.5, 1.0, 0.5], [0.25, 0.5, 0.25]])
    out = sum(k[i, j] * p[i:i + a.shape[0], j:j + a.shape[1]] for i in range(3) for j in range(3))
    return out / k.sum()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cell", type=float, default=0.06)
    ap.add_argument("--half", type=float, default=6.0)
    ap.add_argument("--near", type=int, default=0, help="rays aimed around the lamp per cell (the fix under test)")
    ap.add_argument("--out")
    ap.add_argument("--white", type=float, default=600.0)
    a = ap.parse_args()
    xs = np.arange(-a.half, a.half, a.cell) + 0.5 * a.cell
    gx, gz = np.meshgrid(xs, xs)
    origins = np.stack([gx.ravel(), np.zeros(gx.size), gz.ravel()], axis=-1)
    rotation = np.random.default_rng(7).uniform(0.0, 2.0 * np.pi, len(origins))
    shape = gx.shape
    direct = lit(origins, np.broadcast_to(np.array([0.0, 1.0, 0.0]), origins.shape)).reshape(shape)
    results = {}
    for name, count, near in (("64", 64, 0), ("4096", 4096, 0), ("near", 64, a.near)):
        if name == "near" and not near:
            continue
        # About a million rays a chunk: well under 1 GiB of temporaries (the PC's RAM budget).
        step = max(1_000_000 // count, 1)
        parts = [gather(origins[i:i + step], rotation[i:i + step], count, near) for i in range(0, len(origins), step)]
        results[name] = [np.concatenate([p[j] for p in parts]).reshape(shape) for j in range(4)]
    print(f"cells {shape[0]}x{shape[1]} of {a.cell} m; lamp direct under the arch {direct.max():.0f} lux")
    for run in (r for r in ("64", "near") if r in results):
        for name, j in (("bulb", 0), ("lintel", 1)):
            raw, fine = results[run][j], results["4096"][j]
            error = tent(raw) - tent(fine)
            off = np.abs(error) > 0.02 * (direct + 1000.0)
            print(f"{run:>4} {name}: cells whose regular rays hit it {results[run][2 + j].mean() * 100:.2f}% | mean bounce {raw.mean():.1f} lux "
                  f"(4096 dirs {fine.mean():.1f}) | raw max {raw.max():.0f} | after the tent: error RMS {np.sqrt((error ** 2).mean()):.1f} lux, "
                  f"max {error.max():.0f}, min {error.min():.0f}; cells more than 2% of (direct + 1000 lux) off {off.mean() * 100:.2f}%")
    if a.out:
        from PIL import Image
        maps = [tent(results[r][0] + results[r][1]) for r in ("64", "near", "4096") if r in results]
        Image.fromarray((np.clip(np.concatenate(maps, axis=1) / a.white, 0.0, 1.0) ** (1 / 2.2) * 255).astype(np.uint8)).save(a.out)
        print(a.out)


if __name__ == "__main__":
    main()
