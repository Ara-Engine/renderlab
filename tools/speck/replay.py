"""U-SPECK CPU replay of the solid-angle hypothesis (no GPU): do 64 fixed gather directions per
ground cell, against the arch lamp's lit bulb and lintel, make the blobs?

The game scene's arch in its own frame (examples/game_scene.rs): ground y = 0; the bulb, a
white sphere of radius 0.12 m at (0, 3.55, 0); its point light 0.25 m below it; the lintel's
underside at y = 4.0, 5.2 x 1.0 m. Each ground cell gathers as shaders/gi_cache.wgsl does: 64
equal-solid-angle hemisphere directions (gi_direction) turned about the normal by a per-cell
angle, each hit's Lambertian radiance under the point light times 2 pi / 64 into an L1 SH,
read back as irradiance (a delta of weight w reads w (0.25 + 0.5 cos)), then the 3x3 tent
(1, 0.5, 0.25) of the cell filter. The hit is shaded at the exact hit point, not at a cell's
query point, so amplitudes are indicative.

Prints, per source (bulb, lintel): the share of cells whose gather hits it, the mean bounce
against a 4096-direction estimate of the same integral, and the size of the outliers after
the filter, in lux and against the lamp's direct light on the same cell. `--out PNG` writes
the filtered bounce maps (64 directions | 4096 directions), white at `--white` lux.

    py -3 -B tools/speck/replay.py [--cell 0.06] [--half 6.0] [--out PNG] [--white 600]
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


def directions(count, rotation):
    """gi_direction about +y: (cells, count, 3)."""
    k = np.arange(count)
    z = 1.0 - (k + 0.5) / count
    r = np.sqrt(np.maximum(1.0 - z * z, 0.0))
    phi = k[None, :] * 2.399963229728653 + rotation[:, None]
    return np.stack([r * np.cos(phi), np.broadcast_to(z, phi.shape), r * np.sin(phi)], axis=-1)


def lit(points, normals):
    """Lambert irradiance (lux) of the point lamp at points with normals; the bulb shadows the lintel."""
    to = LIGHT - points
    dist2 = (to * to).sum(-1)
    cos = np.maximum((to * normals).sum(-1), 0.0) / np.sqrt(dist2)
    return CANDELA * cos / np.maximum(dist2, 1e-4)


def gather(origins, rotation, count):
    """Per-cell bounce irradiance (lux) from the bulb and from the lintel, and hit flags."""
    d = directions(count, rotation)
    o = origins[:, None, :]
    weight = 2.0 * np.pi / count
    # Bulb: nearest ray-sphere hit.
    oc = o - BULB
    b = (oc * d).sum(-1)
    c = (oc * oc).sum(-1) - BULB_R * BULB_R
    disc = b * b - c
    t = -b - np.sqrt(np.maximum(disc, 0.0))
    hit_bulb = (disc > 0.0) & (t > 0.0)
    p = o + d * t[..., None]
    n = (p - BULB) / BULB_R
    radiance = ALBEDO_BULB / np.pi * lit(p, n)
    # L1 SH read as irradiance about +y: a delta of weight w gives w (0.25 + 0.5 cos).
    lobe = weight * (0.25 + 0.5 * d[..., 1])
    bulb = np.where(hit_bulb, radiance * lobe, 0.0).sum(-1)
    # Lintel underside (facing down), behind the bulb where the bulb is hit first.
    tl = (LINTEL_Y - o[..., 1]) / np.maximum(d[..., 1], 1e-9)
    pl = o + d * tl[..., None]
    hit_lintel = (np.abs(pl[..., 0]) < LINTEL_X) & (np.abs(pl[..., 2]) < LINTEL_Z) & ~hit_bulb
    down = np.broadcast_to(np.array([0.0, -1.0, 0.0]), pl.shape)
    e = lit(pl, down)
    # The bulb's shadow on the lintel: the light's ray to the point passes the sphere.
    seg = pl - LIGHT
    seg_len = np.linalg.norm(seg, axis=-1)
    u = seg / seg_len[..., None]
    lc = LIGHT - BULB
    bb = (lc * u).sum(-1)
    dd = bb * bb - ((lc * lc).sum() - BULB_R * BULB_R)
    shadow = (dd > 0.0) & (-bb - np.sqrt(np.maximum(dd, 0.0)) < seg_len) & (-bb + np.sqrt(np.maximum(dd, 0.0)) > 0.0)
    lintel = np.where(hit_lintel & ~shadow, ALBEDO_LINTEL / np.pi * e * lobe, 0.0).sum(-1)
    return bulb, lintel, hit_bulb.any(-1), hit_lintel.any(-1)


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
    ap.add_argument("--out")
    ap.add_argument("--white", type=float, default=600.0)
    a = ap.parse_args()
    xs = np.arange(-a.half, a.half, a.cell) + 0.5 * a.cell
    gx, gz = np.meshgrid(xs, xs)
    origins = np.stack([gx.ravel(), np.zeros(gx.size), gz.ravel()], axis=-1)
    rng = np.random.default_rng(7)
    rotation = rng.uniform(0.0, 2.0 * np.pi, len(origins))
    shape = gx.shape
    direct = lit(origins, np.broadcast_to(np.array([0.0, 1.0, 0.0]), origins.shape)).reshape(shape)
    results = {}
    for count in (64, 4096):
        # About a million rays a chunk: well under 1 GiB of temporaries (the PC's RAM budget).
        step = max(1_000_000 // count, 1)
        parts = [gather(origins[i:i + step], rotation[i:i + step], count) for i in range(0, len(origins), step)]
        results[count] = [np.concatenate([p[j] for p in parts]).reshape(shape) for j in range(4)]
    print(f"cells {shape[0]}x{shape[1]} of {a.cell} m; lamp direct under the arch {direct.max():.0f} lux")
    for name, j in (("bulb", 0), ("lintel", 1)):
        raw, fine = results[64][j], results[4096][j]
        f = tent(raw)
        smooth = tent(fine)
        excess = f - smooth
        hot = excess > 0.02 * (direct + 1000.0)
        print(f"{name}: cells whose gather hits it {results[64][2 + j].mean() * 100:.2f}% | mean bounce 64 dirs {raw.mean():.1f} lux, "
              f"4096 dirs {fine.mean():.1f} | raw max {raw.max():.0f} lux | after the tent: max excess {excess.max():.0f} lux, "
              f"cells more than 2% above (direct + 1000 lux) {hot.mean() * 100:.2f}%, largest excess over the lamp's direct light "
              f"{(excess / np.maximum(direct, 1.0)).max():.2f}x")
    if a.out:
        from PIL import Image
        both = np.concatenate([tent(results[64][0] + results[64][1]), tent(results[4096][0] + results[4096][1])], axis=1)
        Image.fromarray((np.clip(both / a.white, 0.0, 1.0) ** (1 / 2.2) * 255).astype(np.uint8)).save(a.out)
        print(a.out)


if __name__ == "__main__":
    main()
