"""N7 CPU probe: a world-space sun-visibility cache for non-RT shadows; no renderer.

Inputs are read in place: R1's saved quarry poses 0 and 1 (+1 cm x) with hit
records, world points, normals, Exact 256-ray counts and the 256-primary film counts; T1b's
Q-walk motion record; the S2 fixture and its camera tracks.

1. Keying: per-triangle lattice cells (placement, triangle, level, cell), micro-texels (same
   keys, cell-average value) and a world-position hash grid (placement, level, voxel). Cell
   size is the power of two at or below footprint / sqrt(N) for N samples per footprint.
   Counts are distinct keys from pixel-centre hits plus the area-based count sum(A_px / h^2).
2. Reuse across the registered 1 cm step: new keys, and newly seen surface by reprojection.
3. Quality at pose 1 from a cache filled at pose 0: each pose-1 pixel reads its key's value
   (8-ray outcomes, emulated as binomial(8, Exact/256), or the 256-ray Exact value once
   converged), or traces fresh when the key is cold. Scored against pose 1's film R_F on
   single-surface pixels, beside Exact and emulated Interactive at pose 1.
4. Q-walk: T1b's per-frame restarted pixels, an upper bound on newly seen surface.
5. Sun motion: a world-fixed direction lattice slid under the 0.04 rad disk; rays entering
   per frame at 0.25, 1 and 10 deg/s (Monte Carlo against 2 w dt / (pi alpha)).
6. S2: border entry and plate reveal per frame, and the shadow-step blocker's reach area.

Run: py -3 -B scripts/non-rt-n7-probe.py --out E:/renderlab-work/non-rt-n7
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

H, W = 1080, 1920
R1 = Path("E:/renderlab-work/m6-exact-quality/r1")
ASSET = Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla")
FILM = np.dtype([("counts", "<u4", (4,)), ("moments", "<f4", (4,)), ("radiance", "<f4", (4,))])
PIXEL_RAD = 2 * np.tan(0.85 / 2) / H
ALPHA = 0.04
NS_PER_RAY = 9.43e6 / 2_772_136  # N0 software 8-ray pass, ns per ray
RAYS = 8


def load(pose):
    d = R1 / f"quarry-viewed-p{pose}"
    info = json.loads((d / f"pose-{pose}.json").read_text())
    hit = np.fromfile(d / f"pose-{pose}.hit.u32", "<u4").reshape(H, W, 4)
    world = np.fromfile(d / f"pose-{pose}.world.f32", "<f4").reshape(H, W, 4)[..., :3].astype(float)
    normal = np.fromfile(d / f"pose-{pose}.normal.f32", "<f4").reshape(H, W, 4)[..., :3].astype(float)
    exact = np.fromfile(d / f"pose-{pose}.exact.u32", "<u4").reshape(H, W) / 256.0
    c = np.fromfile(d / f"pose-{pose}.film.bin", FILM).reshape(H, W)["counts"].astype(float)
    n = c[..., 2] + c[..., 3]
    rf = np.divide(c[..., 0] + c[..., 1], 256 * n, out=np.full((H, W), np.nan), where=n > 0)
    cover = hit[..., 0] > 0
    eye = np.array(info["eye"][:3])
    ys, xs = np.nonzero(cover)
    p = world[ys, xs]
    v = eye - p
    dist = np.linalg.norm(v, axis=1)
    cos = np.abs((normal[ys, xs] * v).sum(1)) / dist
    f = dist * PIXEL_RAD  # linear footprint, metres
    return dict(ys=ys, xs=xs, place=hit[ys, xs, 0].astype(np.int64), prim=hit[ys, xs, 1].astype(np.int64),
                p=p, f=f, area=f * f / np.maximum(cos, 0.05), exact=exact[ys, xs], rf=rf[ys, xs],
                single=(n[ys, xs] >= 256))


def level(f, n_fp):
    """Power-of-two cell edge at or below f / sqrt(n_fp), as an integer exponent."""
    return np.floor(np.log2(f / np.sqrt(n_fp))).astype(np.int64)


def keys(d, n_fp, kind):
    """One 64-bit-safe key tuple per receiver, hashed to a structured array for set ops."""
    lv = level(d["f"], n_fp)
    cell = np.floor(d["p"] / np.exp2(lv)[:, None]).astype(np.int64)
    if kind == "triangle":
        cols = [d["place"], d["prim"], lv, cell[:, 0], cell[:, 1], cell[:, 2]]
    else:  # world hash grid
        cols = [d["place"], lv, cell[:, 0], cell[:, 1], cell[:, 2]]
    return np.rec.fromarrays(cols).view(np.dtype((np.void, 8 * len(cols)))).ravel()


def tri_areas(source, place, prim):
    """World areas (m^2) of the given (placement, triangle) pairs."""
    out = np.empty(len(place))
    for k, (mesh, matrix) in enumerate(source.placements):
        sel = np.nonzero(place == k + 1)[0]
        if not len(sel):
            continue
        (_, vertices, indices), = source.meshes[mesh]
        q = vertices[indices[prim[sel]], :3].astype(float) @ matrix[:3, :3].astype(float).T
        out[sel] = 0.5 * np.linalg.norm(np.cross(q[:, 1] - q[:, 0], q[:, 2] - q[:, 0]), axis=1)
    return out


def counts(d, source, n_fp):
    """Entries touched by one 1080p view and their memory, per keying."""
    lv = level(d["f"], n_fp)
    h2 = np.exp2(2 * lv)
    area_samples = float((d["area"] / h2).sum())  # every cell under every footprint
    tri = np.rec.fromarrays([d["place"], d["prim"]]).view(np.dtype((np.void, 16))).ravel()
    utri, first, inv = np.unique(tri, return_index=True, return_inverse=True)
    hmin = np.full(len(utri), np.inf)
    np.minimum.at(hmin, inv, np.exp2(lv))
    a = tri_areas(source, d["place"][first], d["prim"][first])
    whole = float(np.maximum(1, np.ceil(a / hmin ** 2)).sum())  # whole-triangle allocation
    out = dict(triangles=int(len(utri)), cells_area_based=round(area_samples),
               triangle_whole_alloc_samples=round(whole))
    for kind in ("triangle", "grid"):
        out[f"{kind}_keys_centre_hits"] = int(len(np.unique(keys(d, n_fp, kind))))
    # Memory. Lattice sample: 1 B of 8 ray bits (+3 B for a 32-ray converged state). Triangle
    # header in an open-addressed table at load 0.5: 16 B entry -> 32 B. Grid voxel entry:
    # 8 B key + 4 B state at load 0.5 -> 24 B.
    out["mem_MB"] = dict(
        triangle_lattice_8ray=round((whole * 1 + len(utri) * 32) / 1e6, 1),
        triangle_lattice_32ray=round((whole * 4 + len(utri) * 32) / 1e6, 1),
        micro_texel_area=round((area_samples * 4 + len(utri) * 32) / 1e6, 1),
        grid_area=round(area_samples * 24 / 1e6, 1))
    return out


def reuse(d0, d1, n_fp, kind):
    """Keys at pose 1 absent from pose 0, as receivers and as area-weighted cells."""
    k0, k1 = keys(d0, n_fp, kind), keys(d1, n_fp, kind)
    cold = ~np.isin(k1, np.unique(k0))
    lv = level(d1["f"], n_fp)
    cells = d1["area"] / np.exp2(2 * lv)
    return dict(cold_receivers=int(cold.sum()), cold_fraction=round(float(cold.mean()), 5),
                new_cells_area=round(float(cells[cold].sum())),
                rays=round(float(cells[cold].sum()) * RAYS),
                ms=round(float(cells[cold].sum()) * RAYS * NS_PER_RAY * 1e-6, 4))


def quality(d0, d1, n_fp, kind, rng):
    """Pose 1 read from a pose-0-filled cache, scored against pose 1's film R_F."""
    k0, k1 = keys(d0, n_fp, kind), keys(d1, n_fp, kind)
    u, first = np.unique(k0, return_index=True)  # first writer (lowest pixel index) wins
    v8_0 = rng.binomial(RAYS, d0["exact"]) / RAYS
    v8_1 = rng.binomial(RAYS, d1["exact"]) / RAYS
    at = np.searchsorted(u, k1)
    at = np.minimum(at, len(u) - 1)
    warm = u[at] == k1
    src = first[at]
    est = {
        "Exact": d1["exact"],
        "Interactive (binomial 8)": v8_1,
        "cache 8-ray": np.where(warm, v8_0[src], v8_1),
        "cache converged (256-ray)": np.where(warm, d0["exact"][src], d1["exact"]),
    }
    rf, s = d1["rf"], d1["single"] & np.isfinite(d1["rf"])
    part = s & (rf > 0) & (rf < 1)
    lit, occ = s & (rf == 1), s & (rf == 0)
    rows = {}
    for name, e in est.items():
        err = e - rf
        rows[name] = dict(
            partial_rms=round(float(np.sqrt(np.mean(err[part] ** 2))), 4),
            partial_mean=round(float(err[part].mean()), 4),
            lit_rms=round(float(np.sqrt(np.mean(err[lit] ** 2))), 4),
            occluded_rms=round(float(np.sqrt(np.mean(err[occ] ** 2))), 4),
            solid_over_0p002=int((np.abs(err[lit | occ]) > 0.002).sum()),
            solid_flips_over_0p5=int((np.abs(err[lit | occ]) > 0.5).sum()),
            missed_film_shadows=int((s & (rf <= 0.9) & (e >= 0.99)).sum()),
            false_shadows=int((s & (rf >= 0.99) & (e <= 0.9)).sum()))
    return dict(warm_fraction=round(float(warm.mean()), 4), partial_n=int(part.sum()),
                solid_n=int((lit | occ).sum()), rows=rows)


def bit_identical(d0, d1):
    """Pose-1 receivers whose f32 world point and triangle equal a pose-0 receiver's."""
    def k(d):
        return np.rec.fromarrays([d["place"], d["prim"], *d["p"].astype(np.float32).view(np.int32).T]
                                 ).view(np.dtype((np.void, 28))).ravel()
    return int(np.isin(k(d1), np.unique(k(d0))).sum())


def seen_before(d0, d1, pose0_camera):
    """Pose-1 receivers whose point pose 0 also saw: projected into pose 0, the pose-0 hit there
    is the same placement within one footprint. The rest is newly seen surface."""
    eye, target, up = (np.array(pose0_camera[k], float) for k in ("eye", "target", "up"))
    fwd = (target - eye) / np.linalg.norm(target - eye)
    right = np.cross(fwd, up); right /= np.linalg.norm(right)
    upv = np.cross(right, fwd)
    rel = d1["p"] - eye
    z = rel @ fwd
    t = np.tan(pose0_camera["fov_y_radians"] / 2)
    sx = (rel @ right) / (z * t * W / H)
    sy = (rel @ upv) / (z * t)
    px = np.floor((sx + 1) / 2 * W).astype(np.int64)
    py = np.floor((1 - sy) / 2 * H).astype(np.int64)
    inside = (z > 0) & (px >= 0) & (px < W) & (py >= 0) & (py < H)
    grid = np.full((H, W), -1, np.int64)
    grid[d0["ys"], d0["xs"]] = np.arange(len(d0["ys"]))
    j = np.where(inside, grid[np.clip(py, 0, H - 1), np.clip(px, 0, W - 1)], -1)
    ok = j >= 0
    jj = np.maximum(j, 0)
    same = ok & (d0["place"][jj] == d1["place"]) & (
        np.linalg.norm(d0["p"][jj] - d1["p"], axis=1) <= 2 * d1["f"])
    return dict(outside_pose0_view=int((~inside).sum()), new_surface=int((~same).sum()),
                new_fraction=round(float((~same).mean()), 5))


def qwalk():
    m = json.loads(Path("E:/renderlab-work/t1-gpu/walk-b15/motion.json").read_text())
    fr = [f for f in m["frames"] if f.get("moving")]
    r = np.array([f["restarted_pixels"] for f in fr])
    return dict(moving_frames=len(r), restarted_p50=int(np.median(r)), restarted_p99=int(np.quantile(r, .99)),
                restarted_max=int(r.max()), restarted_mean=round(float(r.mean()), 1))


def sun_sweep(rng):
    """Rays entering the disk per frame for a lattice fixed in a world direction chart."""
    out = {}
    pts = rng.uniform(-3 * ALPHA, 3 * ALPHA, size=(400_000, 2))  # chart plane, density uniform
    in0 = np.hypot(*pts.T) < ALPHA
    for dps in (0.25, 1.0, 10.0):
        step = np.radians(dps) / 60
        in1 = np.hypot(pts[:, 0] - step, pts[:, 1]) < ALPHA
        frac = float((in1 & ~in0).sum() / in0.sum())
        out[str(dps)] = dict(entering_fraction=round(frac, 6),
                             formula=round(2 * step / (np.pi * ALPHA), 6))
    return out


def s2():
    s = json.loads(Path("E:/renderlab-work/motion-benches/S2/fixture/scene.json").read_text())
    mesh = {m["name"]: np.array(m["positions"]) for m in s["meshes"]}
    focal, depth = s["width_registration"]["focal_pixels"], s["width_registration"]["floor_depth_m"]
    tr = {t["name"]: t["frames"] for t in s["tracks"]}
    out = {}
    plate = mesh["reveal-plate"]
    px = np.array([f["plate_x"] for f in tr["camera-1px"]])
    dplate = np.abs(np.diff(px))
    plate_h_px = (plate[:, 1].max() - plate[:, 1].min()) * focal / depth
    out["plate_reveal_px_per_frame_max"] = round(float(dplate.max() * focal / depth * plate_h_px))
    out["plate_moving_frames"] = int((dplate > 0).sum())
    for name, frames in tr.items():
        eye = np.array([f["eye"] for f in frames])
        step_px = np.abs(np.diff(eye[:, 0])) * focal / depth
        out[name] = dict(border_px_per_frame_max=round(float(step_px.max() * H)))
    blk = mesh["shadow-blocker"]
    bx = np.array([f["blocker_x"] for f in tr["shadow-step"]])
    L = np.array(tr["shadow-step"][0]["to_light"])
    # Blocker shadow on the floor (y = 0): project its vertices along -L, widen by penumbra.
    def shadow_box(x):
        q = blk + [x, 0, 0]
        t = q[:, 1] / L[1]
        g = q - t[:, None] * L
        pen = t.max() * np.tan(ALPHA)
        return g[:, [0, 2]].min(0) - pen, g[:, [0, 2]].max(0) + pen
    moves = np.nonzero(np.diff(bx))[0]
    lo0, hi0 = shadow_box(bx[moves[0]]) if len(moves) else shadow_box(0)
    lo1, hi1 = shadow_box(bx[moves[0] + 1]) if len(moves) else shadow_box(0)
    union = np.prod(np.maximum(hi0, hi1) - np.minimum(lo0, lo1))
    mpp = s["width_registration"]["metres_per_pixel"]
    out["blocker"] = dict(moving_frames=int(len(moves)),
                          step_m=round(float(np.abs(np.diff(bx)).max()), 4),
                          reach_area_m2=round(float(union), 5),
                          reach_area_px=round(float(union / mpp ** 2)))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    rng = np.random.default_rng(20261106)
    source = probe.Source(ASSET)
    d = {k: load(k) for k in (0, 1)}
    rep = dict(method=__doc__, covered={k: int(len(v["ys"])) for k, v in d.items()},
               footprint_mm_median={k: round(float(np.median(v["f"])) * 1e3, 3) for k, v in d.items()},
               pixel_world_area_mm2_median={k: round(float(np.median(v["area"])) * 1e6, 2) for k, v in d.items()})
    rep["counts"] = {f"pose0_N{n}": counts(d[0], source, n) for n in (1, 4, 16)}
    rep["bit_identical_pose1_receivers"] = bit_identical(d[0], d[1])
    sizes = (1 / 16, 1 / 4, 1, 4)  # cell edge from 2-4 footprints down to 1/4-1/2
    rep["counts"].update({f"pose0_N{n}": counts(d[0], source, n) for n in sizes[:2]})
    rep["reuse"] = {f"0->1_N{n}_{kind}": reuse(d[0], d[1], n, kind)
                    for n in sizes for kind in ("triangle", "grid")}
    cams = json.loads(Path("E:/renderlab-work/t1-gpu/cameras.json").read_text())["frames"]
    rep["seen_before_pose1"] = seen_before(d[0], d[1], cams[0])
    rep["quality_pose1"] = {f"N{n}_{kind}": quality(d[0], d[1], n, kind, rng)
                            for n in sizes for kind in ("triangle", "grid")}
    rep["qwalk"] = qwalk()
    rep["sun"] = sun_sweep(rng)
    rep["s2"] = s2()
    rep["ns_per_software_ray"] = NS_PER_RAY
    rep["seconds"] = round(time.time() - t0, 1)
    (args.out / "n7.json").write_text(json.dumps(rep, indent=1))
    print(json.dumps({k: v for k, v in rep.items() if k != "method"}, indent=1))


if __name__ == "__main__":
    main()
