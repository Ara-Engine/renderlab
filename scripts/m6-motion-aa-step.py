"""Motion-AA single real step (docs/m6-motion-aa.md): pose-0 samples carried to pose 1 as points.

R1's registered 1 cm step has real parallax, folds, instance boundaries and silhouettes, which the
2D simulation (m6-motion-aa-sim.py) does not. Each of pose 0's 16 saved samples is placed on its
pixel's centre tangent plane (hit point and geometric normal), or a covered neighbour's for
silhouette samples, and projected into pose 1. It is accepted where it lands only by tests a frame
can make from its own one hit per pixel:
  geometry  - the landing pixel's centre hit has the same placement, and the sample lies within
              4 pixel footprints of that hit's tangent plane along the pose-1 ray (own plane), or of a
              same-placement 4-neighbour's plane ('plane4');
  background- the nearest coverage evidence (pose-1 centre hits and accepted geometry samples) is
              background.
Per pose-1 pixel the estimate is the mean of accepted samples and the frame's fresh sample (pose 1's
Exact centre); it is scored against pose 1's own 256-sample film, which is independent of pose 0's
samples. Compared with pose 1's own 16 samples (static, scored against their 240-stratum complement)
and with T1b's single step: pose 0's 16-sample mean fetched bilinearly and blended with the fresh
sample at T1b's window.

py -3 -B scripts/m6-motion-aa-step.py --out E:/renderlab-work/motion-aa/step
"""
import argparse
import importlib.util
import json
import time
from pathlib import Path

import numpy as np
from scipy import ndimage
from scipy.spatial import cKDTree

spec = importlib.util.spec_from_file_location("sim", Path(__file__).with_name("m6-motion-aa-sim.py"))
sim = importlib.util.module_from_spec(spec); spec.loader.exec_module(sim)
rp = sim.rp; f0 = rp.f0
H, W = rp.H, rp.W
PIX = 1.028e-3


def load(pose, crop):
    d = rp.R1 / f"quarry-viewed-p{pose}"
    ref = f0.decode(d, pose)
    point = np.load(f0.FROZEN / f"gate-final/reference/pose-{pose}-reference.npz")["ref"]
    pops = f0.populations(ref, point)
    pops["backlit"] = ref["cover"] & ~ref["front"]; pops["all-covered"] = ref["cover"]
    pops["image"] = np.ones_like(ref["cover"]); pops["multi-surface"] = ref["cover"] & (ref["n"] < 256)
    pops["background-centre"] = ~ref["cover"]
    raw = np.fromfile(d / f"pose-{pose}.film.bin", dtype=f0.DTYPE).reshape(H, W)
    direct = np.fromfile(d / f"pose-{pose}.direct.bin", dtype=f0.DTYPE).reshape(H, W)["radiance"][..., :3].astype(np.float64) / 256
    counts = raw["counts"].astype(np.float64)
    n = counts[..., 2] + counts[..., 3]
    film = dict(L=ref["hdr"][..., :3], D=direct,
                F=np.divide(counts[..., 0] + counts[..., 1], 256 * n, out=np.full((H, W), np.nan), where=n > 0))
    S = dict(L=[], D=[], V=[])
    for i in range(16):
        S["L"].append(np.fromfile(d / f"samples/{i:02}.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3][crop].astype(np.float32))
        S["D"].append(np.fromfile(d / f"samples/{i:02}.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3][crop].astype(np.float32))
        c = np.fromfile(d / f"samples/{i:02}.visibility.u16", dtype="<u2").reshape(H, W)[crop]
        S["V"].append(np.where(c != 65535, c / 256.0, np.nan).astype(np.float32))
    exact = dict(L=np.fromfile(d / f"pose-{pose}.exact.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3][crop].astype(np.float32),
                 D=np.fromfile(d / f"pose-{pose}.exact.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3][crop].astype(np.float32),
                 V=np.where(ref["cover"], np.fromfile(d / f"pose-{pose}.exact.u32", dtype="<u4").reshape(H, W) / 256, np.nan)[crop])
    normal = np.fromfile(d / f"pose-{pose}.normal.f32", dtype="<f4").reshape(H, W, 4)[..., :3].astype(np.float64)
    return dict(ref=ref, info=ref["info"], pops={k: v[crop] for k, v in pops.items()}, film={k: v[crop] for k, v in film.items()},
                samples={k: np.stack(v) for k, v in S.items()}, exact=exact, cover=ref["cover"][crop],
                world=ref["world"][..., :3][crop].astype(np.float64), normal=normal[crop], placement=ref["hit"][..., 0][crop].astype(np.int64),
                n=ref["n"][crop])


def rays(info, px, py):
    """World ray directions through continuous pixel positions (edge convention) of a pose."""
    M = np.array(info["camera_inverse"]).reshape(4, 4).T
    ndc = np.stack([px / W * 2 - 1, 1 - py / H * 2], -1)
    def at(z):
        c = np.concatenate([ndc, np.full(ndc.shape[:-1] + (1,), z), np.ones(ndc.shape[:-1] + (1,))], -1) @ M.T
        return c[..., :3] / c[..., 3:4]
    d = at(0.0) - at(1.0)
    return d / np.linalg.norm(d, axis=-1, keepdims=True)


def plane_t(eye, d, P, n):
    """Distance along unit rays d from eye to the plane through P with normal n (nan if parallel)."""
    den = np.einsum("...k,...k->...", d, n)
    t = np.einsum("...k,...k->...", P - eye, n) / np.where(np.abs(den) > 1e-6, den, np.nan)
    return t


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, default=Path("E:/renderlab-work/motion-aa/step"))
    a = p.parse_args(); a.out.mkdir(parents=True, exist_ok=True); t0 = time.perf_counter()
    cover = np.zeros((H, W), bool)
    for pose in (0, 1):
        cover |= np.fromfile(rp.R1 / f"quarry-viewed-p{pose}/pose-{pose}.hit.u32", dtype="<u4").reshape(H, W, 4)[..., 0] != 0
    ys, xs = np.nonzero(cover)
    crop = (slice(max(0, ys.min() - 8), min(H, ys.max() + 9)), slice(max(0, xs.min() - 8), min(W, xs.max() + 9)))
    oy, ox = crop[0].start, crop[1].start
    p0, p1 = load(0, crop), load(1, crop)
    h, w = p0["cover"].shape
    print(f"loaded in {time.perf_counter() - t0:.0f} s", flush=True)
    clear = np.median(p0["samples"]["L"][0][~p0["cover"]].reshape(-1, 3), 0)
    Dsky = np.median(p0["samples"]["D"][0][~p0["cover"]].reshape(-1, 3), 0)
    eye0 = np.asarray(p0["info"]["eye"][:3]); eye1 = np.asarray(p1["info"]["eye"][:3])
    # Plane for every pose-0 pixel: its own centre hit, else the nearest covered 8-neighbour's.
    c0 = p0["cover"]
    _, (iy, ix) = ndimage.distance_transform_edt(~c0, return_indices=True)
    near_ok = ndimage.distance_transform_edt(~c0) <= 1.5
    Pp = p0["world"][iy, ix]; Np = p0["normal"][iy, ix]; plc = p0["placement"][iy, ix]
    yy, xx = np.mgrid[0:h, 0:w]
    recs = dict(u=[], v=[], dist=[], placement=[], bg=[], L=[], D=[], V=[], src=[])
    for i in range(16):
        L = p0["samples"]["L"][i]
        bg = ~np.any(np.abs(L - clear) > 1e-7, -1)
        sx = xx + ox + sim.OFF[i, 0]; sy = yy + oy + sim.OFF[i, 1]
        d = rays(p0["info"], sx, sy)
        t = plane_t(eye0, d, Pp, Np)
        fallback = ~np.isfinite(t) | (t <= 0)
        t = np.where(fallback, np.linalg.norm(Pp - eye0, axis=-1), t)
        X = eye0 + d * t[..., None]
        geo = ~bg & near_ok
        qx, qy = rp.project(p1["info"], np.where(geo[..., None], X, eye1 + d * 1e4))
        u = qx + 0.5 - ox; v = qy + 0.5 - oy
        keep = geo | bg
        for k, val in (("u", u), ("v", v), ("dist", np.linalg.norm(X - eye1, axis=-1)), ("placement", plc), ("bg", bg),
                       ("L", L), ("D", p0["samples"]["D"][i]), ("V", p0["samples"]["V"][i]), ("src", np.full((h, w), i))):
            recs[k].append(val[keep])
    R = {k: np.concatenate(v) for k, v in recs.items()}
    inside = (R["u"] >= 0) & (R["v"] >= 0) & (R["u"] < w) & (R["v"] < h)
    R = {k: v[inside] for k, v in R.items()}
    qx = np.floor(R["u"]).astype(np.int64); qy = np.floor(R["v"]).astype(np.int64)
    # Geometry test against the pose-1 hit plane of the landing pixel (and optionally 4-neighbours).
    d1 = rays(p1["info"], R["u"] + ox, R["v"] + oy)
    c1 = p1["cover"]; pl1 = p1["placement"]
    def consistent(dy, dx):
        ty = np.clip(qy + dy, 0, h - 1); tx = np.clip(qx + dx, 0, w - 1)
        tp = plane_t(eye1, d1, p1["world"][ty, tx], p1["normal"][ty, tx])
        fp = 4 * PIX * R["dist"]
        same = c1[ty, tx] & (pl1[ty, tx] == R["placement"])
        return same & np.isfinite(tp) & (np.abs(R["dist"] - tp) <= fp)
    own = consistent(0, 0)
    four = own | consistent(-1, 0) | consistent(1, 0) | consistent(0, -1) | consistent(0, 1)
    geo = ~R["bg"]
    out = dict(crop=[crop[0].start, crop[0].stop, crop[1].start, crop[1].stop], records=int(R["u"].size), rules={})
    # Background test: nearest coverage evidence among pose-1 centre hits and accepted geometry samples.
    def background_ok(geo_accept):
        ex = np.r_[R["u"][geo & geo_accept], (xx + 0.5)[c1 | ~c1].ravel()]
        ey = np.r_[R["v"][geo & geo_accept], (yy + 0.5).ravel()]
        ecov = np.r_[np.ones(int((geo & geo_accept).sum()), bool), c1.ravel()]
        q = np.nonzero(R["bg"])[0]
        _, nb = cKDTree(np.c_[ex, ey]).query(np.c_[R["u"][q], R["v"][q]], k=1)
        ok = np.zeros(R["u"].size, bool); ok[q] = ~ecov[nb]
        return ok
    scorer_data = dict(pops=p1["pops"], cover=c1, ref=p1["ref"], comp=dict(L=p1["film"]["L"], D=p1["film"]["D"], F=p1["film"]["F"]),
                       cloudV=np.concatenate([p1["samples"]["V"], p1["exact"]["V"][None]]).astype(np.float32))
    scorer = sim.Scorer(scorer_data, crop)
    bar = scorer.score(p1["exact"]["L"], p1["exact"]["D"], p1["exact"]["V"])
    # Pose 1's own samples are scored against its 240-stratum complement (independent of them).
    s1 = p1["samples"]; nf = p1["n"]; Vs = np.where(np.isfinite(s1["V"]), s1["V"], 0).sum(0); acc_n = np.isfinite(s1["V"]).sum(0)
    rest = nf - acc_n
    comp = dict(L=(p1["film"]["L"] * 256 - s1["L"].sum(0)) / 240, D=(p1["film"]["D"] * 256 - s1["D"].sum(0)) / 240,
                F=np.divide(p1["film"]["F"] * nf - Vs, rest, out=np.full(nf.shape, np.nan), where=rest > 0))
    scorer_c = sim.Scorer(dict(scorer_data, comp=comp), crop)
    bar_c = scorer_c.score(p1["exact"]["L"], p1["exact"]["D"], p1["exact"]["V"])
    own16 = scorer_c.score(s1["L"].mean(0), s1["D"].mean(0), np.nanmean(s1["V"], 0))
    out["bar"] = bar; out["bar_complement"] = bar_c; out["pose1_own16"] = own16; out["pose1_own16_ratio"] = sim.ratios(own16, bar_c)
    # Direction check: pose-1 centre rays against the stored centre hits.
    cyx = np.nonzero(c1)
    dc = rays(p1["info"], cyx[1][::997] + ox + 0.5, cyx[0][::997] + oy + 0.5)
    hw = p1["world"][cyx[0][::997], cyx[1][::997]] - eye1
    out["ray_check_max_angle"] = float(np.max(np.arccos(np.clip(np.einsum("ij,ij->i", dc, hw / np.linalg.norm(hw, axis=-1, keepdims=True)), -1, 1))))
    print("ray check max angle (rad)", out["ray_check_max_angle"], flush=True)
    lab = qy * w + qx; N = h * w
    for name, gacc in (("own-plane", own), ("plane4", four), ("instance-only", c1[qy, qx] & (pl1[qy, qx] == R["placement"]))):
        acc = np.where(geo, gacc, background_ok(gacc))
        # Every pixel also has this frame's fresh sample: pose 1's centre (Exact capture).
        cnt = np.bincount(lab[acc], minlength=N).astype(np.float64) + 1
        fresh = dict(L=p1["exact"]["L"].reshape(-1, 3).astype(np.float64), D=p1["exact"]["D"].reshape(-1, 3).astype(np.float64))
        def mean_of(a, key):
            s = np.stack([np.bincount(lab[acc], weights=a[acc, c].astype(np.float64), minlength=N) for c in range(3)], -1)
            return (s + fresh[key]) / cnt[:, None]
        L = mean_of(R["L"], "L"); D = mean_of(R["D"], "D")
        # The 2% centre-surface estimand of R_F: geometry samples consistent with the landing hit.
        vok = acc & geo & np.isfinite(R["V"]) & own
        fvis = p1["exact"]["V"].ravel(); ff = np.isfinite(fvis)
        vc = np.bincount(lab[vok], minlength=N) + ff; vs = np.bincount(lab[vok], weights=R["V"][vok].astype(np.float64), minlength=N) + np.nan_to_num(fvis)
        V = np.divide(vs, vc, out=np.full(N, np.nan), where=vc > 0)
        row = scorer.score(L.reshape(h, w, 3), D.reshape(h, w, 3), V.reshape(h, w))
        row["ratio"] = sim.ratios(row, bar)
        cc = cnt.reshape(h, w)[c1] - 1
        row["accepted_per_covered_pixel"] = dict(mean=float(cc.mean()), p1=float(np.percentile(cc, 1)), p50=float(np.median(cc)), zero=int((cc == 0).sum()))
        row["rejected_geometry_share"] = float(1 - gacc[geo].mean())
        if name == "plane4":
            # Diagnose verified solids over 0.002: the opposite samples that landed in them.
            sv = scorer.pops["solid-lit-v"] | scorer.pops["solid-occ-v"]
            Vimg = V.reshape(h, w); F1 = p1["film"]["F"]
            bad = sv & np.isfinite(Vimg) & (np.abs(Vimg - F1) > 0.002)
            bl = np.zeros(N, bool); bl[np.nonzero(bad.ravel())[0]] = True
            m = vok & bl[lab] & (np.abs(R["V"] - F1.ravel()[lab]) > 0.5)
            fu = R["u"] - qx; fv = R["v"] - qy
            edge = np.minimum(np.minimum(fu, 1 - fu), np.minimum(fv, 1 - fv))
            # Source pixel of each record (pose 0), from the record order: per sample index, kept pixels in raster order.
            F0 = p0["film"]["F"]
            src_px = np.concatenate([np.nonzero((~np.any(np.abs(p0["samples"]["L"][i] - clear) > 1e-7, -1) | near_ok).ravel())[0] for i in range(16)])[inside]
            sF = F0.ravel()[src_px[m]]
            row["solid_failures"] = dict(pixels=int(bad.sum()), opposite_samples=int(m.sum()),
                                         landing_distance_to_footprint_edge=np.quantile(edge[m], [0, .5, 1]).tolist() if m.any() else None,
                                         source_pixel_film_F=np.quantile(sF, [0, .5, 1]).tolist() if m.any() else None,
                                         source_pixel_partial=int(((sF > 0) & (sF < 1)).sum()), sample_V=np.unique(np.round(R["V"][m], 3)).tolist()[:20],
                                         sample_index=np.bincount(R["src"][m].astype(np.int64), minlength=16).tolist(),
                                         landing_moved_px=np.quantile(np.hypot(R["u"][m] - (src_px[m] % w + 0.5), R["v"][m] - (src_px[m] // w + 0.5)), [0, .5, 1]).tolist() if m.any() else None)
            # Diagnostic only: drop records landing within 0.02 px of a footprint edge, where the
            # prototype's tangent-plane depth (not a stored hit distance) can misplace them.
            for guard in (0.005, 0.02):
                gok = vok & (edge > guard)
                gc = np.bincount(lab[gok], minlength=N) + ff; gs = np.bincount(lab[gok], weights=R["V"][gok].astype(np.float64), minlength=N) + np.nan_to_num(fvis)
                GV = np.divide(gs, gc, out=np.full(N, np.nan), where=gc > 0).reshape(h, w)
                gb = sv & np.isfinite(GV) & (np.abs(GV - F1) > 0.002)
                row["solid_failures"][f"guard_{guard}"] = dict(pixels=int(gb.sum()), max=float(np.abs(np.nan_to_num(GV) - F1)[gb].max()) if gb.any() else 0.0)
            print("solid failures:", row["solid_failures"], flush=True)
        out["rules"][name] = row
        r = row["ratio"]
        print(f"{name}: L0 all-covered {r['L0']['all-covered']:.3f}x edge {r['L0']['edge']:.3f}x multi {r['L0']['multi-surface']:.3f}x "
              f"bg-centre {r['L0']['background-centre']:.3f}x D4 occ {r['D4']['occluded']:.3f}x; verified solid max "
              f"{max(row['F']['solid-lit-v']['max'], row['F']['solid-occ-v']['max']):.4f} (over .002: {row['F']['solid-lit-v']['over_002']}+{row['F']['solid-occ-v']['over_002']}); "
              f"F4 {row['missed']}+{row['false']} D4>16 {row['D4_over16']}; accepted {cc.mean():.2f}/px, rejected geometry {row['rejected_geometry_share']:.4f}", flush=True)
    # T1b's single step: pose 0's 16-sample mean fetched bilinearly through each pose-1 centre hit,
    # forward test (placement, 4 footprints); background pixels keep their own pixel.
    m0 = dict(L=p0["samples"]["L"].mean(0), D=p0["samples"]["D"].mean(0))
    vfin = np.isfinite(p0["samples"]["V"]); m0["V"] = np.where(vfin.any(0), np.nansum(p0["samples"]["V"], 0) / np.maximum(vfin.sum(0), 1), np.nan)
    X1 = p1["world"]
    q0x, q0y = rp.project(p0["info"], X1)
    q0x = np.where(c1, q0x - ox, xx); q0y = np.where(c1, q0y - oy, yy)
    dX = np.linalg.norm(X1 - eye0, axis=-1); d0 = np.linalg.norm(p0["world"] - eye0, axis=-1)
    bx0 = np.floor(q0x).astype(np.int64); by0 = np.floor(q0y).astype(np.int64); fx = q0x - bx0; fy = q0y - by0
    acc = {k: 0 for k in ("L", "D", "V", "w", "wv")}
    for dx, dy, wgt in ((0, 0, (1 - fx) * (1 - fy)), (1, 0, fx * (1 - fy)), (0, 1, (1 - fx) * fy), (1, 1, fx * fy)):
        tx = np.clip(bx0 + dx, 0, w - 1); ty = np.clip(by0 + dy, 0, h - 1)
        ok = np.where(c1, (p0["placement"][ty, tx] == pl1) & (np.abs(d0[ty, tx] - dX) <= 4 * PIX * dX), ~p0["cover"][ty, tx])
        ww = wgt * ok
        acc["w"] = acc["w"] + ww; acc["L"] = acc["L"] + ww[..., None] * m0["L"][ty, tx]; acc["D"] = acc["D"] + ww[..., None] * m0["D"][ty, tx]
        vv = ww * np.isfinite(m0["V"][ty, tx]); acc["wv"] = acc["wv"] + vv; acc["V"] = acc["V"] + vv * np.nan_to_num(m0["V"][ty, tx])
    has = acc["w"] > 0
    # T1b's blend with the fresh sample (pose 1's centre): n' = min(16 * tap weight + 1, W), W the blur budget.
    blur = fx * (1 - fx) + fy * (1 - fy)
    win = np.clip(1 + 1 / np.maximum(blur, 1e-12), 1, 256)
    n1 = np.where(has, np.minimum(16 * acc["w"] + 1, win), 1.0)
    fL = acc["L"] / np.maximum(acc["w"], 1e-12)[..., None]; fD = acc["D"] / np.maximum(acc["w"], 1e-12)[..., None]
    rL = np.where(has[..., None], fL + (p1["exact"]["L"] - fL) / n1[..., None], p1["exact"]["L"])
    rD = np.where(has[..., None], fD + (p1["exact"]["D"] - fD) / n1[..., None], p1["exact"]["D"])
    fV = acc["V"] / np.maximum(acc["wv"], 1e-12)
    rV = np.where((acc["wv"] > 0) & np.isfinite(p1["exact"]["V"]), fV + (np.nan_to_num(p1["exact"]["V"]) - fV) / n1, p1["exact"]["V"])
    row = scorer.score(rL, rD, rV); row["ratio"] = sim.ratios(row, bar); row["no_valid_tap"] = int((c1 & ~has).sum())
    out["resampled_bilinear_step"] = row
    r = row["ratio"]
    print(f"resampled bilinear step (16-sample mean): L0 all-covered {r['L0']['all-covered']:.3f}x edge {r['L0']['edge']:.3f}x multi {r['L0']['multi-surface']:.3f}x "
          f"D4 occ {r['D4']['occluded']:.3f}x; verified solid max {max(row['F']['solid-lit-v']['max'], row['F']['solid-occ-v']['max']):.4f} "
          f"(over .002: {row['F']['solid-lit-v']['over_002']}+{row['F']['solid-occ-v']['over_002']}); F4 {row['missed']}+{row['false']} D4>16 {row['D4_over16']}", flush=True)
    r = out["pose1_own16_ratio"]
    print(f"pose-1 own 16: L0 all-covered {r['L0']['all-covered']:.3f}x edge {r['L0']['edge']:.3f}x multi {r['L0']['multi-surface']:.3f}x D4 occ {r['D4']['occluded']:.3f}x; "
          f"Exact F4 {bar['missed']}+{bar['false']} D4>16 {bar['D4_over16']}", flush=True)
    out["seconds"] = time.perf_counter() - t0
    (a.out / "step.json").write_bytes(json.dumps(out, indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o)).encode())
    print(f"done in {out['seconds']:.0f} s")


if __name__ == "__main__":
    main()
