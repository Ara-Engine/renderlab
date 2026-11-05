"""CPU-only follow-up to F0 (docs/m6-film-gap.md): where the film gap lives.

  py -3 -B scripts/m6-film-gap.py --f0 D:/Coding/renderlab-work/m6-exact-quality/f0 \
      --out D:/Coding/renderlab-work/m6-exact-quality/film-gap/out

Reads the saved F0 film buffers, centre captures, frozen point reference, frozen
Exact/Interactive HDR captures and decoded Unreal native masks. Writes one JSON.
No GPU, no renderer change. Pose 2 is byte-identical to pose 0 in F0 and is skipped.
"""
import argparse
import importlib.util
import json
import time
from pathlib import Path

import numpy as np
from scipy import ndimage

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("film_score", REPO / "scripts/m6-film-score.py")
fs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fs)
lb = fs.lb
B7 = fs.FROZEN / "runs/accept-b7"
H, W = lb.SHAPE
LUMA = np.array([0.2126, 0.7152, 0.0722])


def display(a, stops=0):
    """The F0 inspection transform: exposure, Reinhard, sRGB encode, in [0, 1]."""
    v = np.maximum(0, a * (2.0 ** stops))
    v = v / (1 + v)
    return np.where(v <= .0031308, 12.92 * v, 1.055 * np.power(v, 1 / 2.4) - .055)


def hdr(path):
    return np.fromfile(path, dtype="<f2").reshape(H, W, 4)[..., :3].astype(float)


def rms(x):
    return float(np.sqrt(np.mean(np.square(x)))) if x.size else None


def neighbour_spread(normal, hit):
    """Maximum angle (degrees) between a pixel's geometric normal and its covered
    same-placement 8-neighbours: a centre-sampled proxy for normal variation inside
    the footprint. Distinct primitives in the 3x3 block count sub-pixel triangles."""
    n = normal[..., :3].astype(float)
    n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-12)
    place = hit[..., 0]
    prim = hit[..., 1].astype(np.int64) + (place.astype(np.int64) << 32)
    worst = np.zeros((H, W))
    distinct = np.ones((H, W), np.int32)
    seen = [prim]
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            if dx == 0 and dy == 0:
                continue
            m = np.roll(np.roll(n, dy, 0), dx, 1)
            same = (np.roll(np.roll(place, dy, 0), dx, 1) == place) & (place != 0)
            # Geometric normals have arbitrary sign: compare as unoriented lines.
            c = np.abs(np.einsum("ijk,ijk->ij", n, m)).clip(0, 1)
            worst = np.where(same, np.maximum(worst, np.degrees(np.arccos(c))), worst)
            q = np.roll(np.roll(prim, dy, 0), dx, 1)
            new = same & np.all([q != s for s in seen], axis=0)
            distinct += new
            seen.append(np.where(same, q, -1))
    return worst, distinct


def triangle_sizes(world, hit, footprint):
    """World edge lengths of hit triangles from pixels whose right and lower neighbours
    share the primitive: solve dP = dB @ [E1, E2] for the two barycentric edges."""
    bary = hit[..., 2:].view("<f4").astype(float)
    p = world[..., :3].astype(float)
    key = (hit[..., 0].astype(np.int64) << 32) + hit[..., 1]
    ok = (hit[..., 0] != 0)
    r = ok & (np.roll(key, -1, 1) == key)
    d = ok & (np.roll(key, -1, 0) == key)
    both = r & d
    both[-1, :] = False
    both[:, -1] = False
    db = np.stack([np.roll(bary, -1, 1) - bary, np.roll(bary, -1, 0) - bary], -2)[both]
    dp = np.stack([np.roll(p, -1, 1) - p, np.roll(p, -1, 0) - p], -2)[both]
    det = db[:, 0, 0] * db[:, 1, 1] - db[:, 0, 1] * db[:, 1, 0]
    good = np.abs(det) > 1e-10
    inv = np.linalg.inv(db[good])
    e = inv @ dp[good]  # rows: E1, E2 (world)
    lengths = np.stack([np.linalg.norm(e[:, 0], axis=-1), np.linalg.norm(e[:, 1], axis=-1),
                        np.linalg.norm(e[:, 1] - e[:, 0], axis=-1)], -1)
    longest = lengths.max(-1)
    fp = footprint[both][good]
    share_r = float(r[ok].mean())
    share_d = float(d[ok].mean())
    return dict(measured_pixels=int(good.sum()), pixels_sharing_right=share_r, pixels_sharing_down=share_d,
                longest_edge_mm_quantiles=(np.quantile(longest, [.1, .5, .9]) * 1000).tolist(),
                longest_edge_over_footprint_quantiles=np.quantile(longest / fp, [.1, .5, .9]).tolist())


def fit_direct(pose, vc, lc, cover):
    """Per-pixel direct-sun radiance D in L = A + D*V, from the frozen Exact capture and
    the four Interactive seeds (same receivers and shading, different visibility).
    Defined only where the five visibilities differ."""
    vs = [vc] + [np.fromfile(B7 / f"pose-{pose}-seed-{k}-interactive.u32", dtype="<u4").reshape(H, W) / 256 for k in range(4)]
    ls = [lc] + [hdr(B7 / f"pose-{pose}-seed-{k}-interactive.rgba16f") for k in range(4)]
    v = np.stack(vs)
    l = np.stack(ls)
    vm = v.mean(0)
    lm = l.mean(0)
    var = ((v - vm) ** 2).mean(0)
    cov = ((v - vm)[..., None] * (l - lm)).mean(0)
    known = cover & (var > 1e-6)
    d = np.where(known[..., None], cov / np.maximum(var, 1e-12)[..., None], np.nan)
    a = lm - d * vm[..., None]
    resid = np.abs(l - (a + d * v[..., None])).max(0)
    rel = np.where(known, (resid @ LUMA) / np.maximum(lm @ LUMA, 1e-9), np.nan)
    return d, a, known, rel


def visibility_regression(lc, rl, vc, rf, full, part):
    """Signed display error (channel mean) regressed on the visibility gap V_c - R_F over
    single-surface footprints. The slope is the visible radiance change per unit of
    visibility gap, including any covariance of direct light and visibility inside the
    footprint; the residual bounds what a visibility-only correction could leave."""
    delta = (vc - rf)[full]
    rows = {}
    for stops in (0, 4):
        e = (display(lc, stops) - display(rl, stops)).mean(-1)[full] * 255
        x = np.c_[np.ones_like(delta), delta]
        coef, *_ = np.linalg.lstsq(x, e, rcond=None)
        resid = e - x @ coef
        p = part[full]
        bins = [-1.01, -.4, -.1, -.02, .02, .1, .4, 1.01]
        table = []
        for lo, hi in zip(bins[:-1], bins[1:]):
            m = (delta > lo) & (delta <= hi)
            table.append(dict(lo=lo, hi=hi, n=int(m.sum()), mean_codes=float(e[m].mean()) if m.any() else None,
                              rms_codes=rms(e[m])))
        # Upper bound for any correction that is a function of the gap alone: the share of
        # squared error carried by bin means, over pixels whose visibility gap exceeds 0.02.
        moving = np.abs(delta) > .02
        fine = np.linspace(-1, 1, 81)
        which = np.digitize(delta[moving], fine)
        means = np.bincount(which, e[moving], len(fine) + 1) / np.maximum(np.bincount(which, minlength=len(fine) + 1), 1)
        explained = float(np.sum(means[which] ** 2) / np.sum(e[moving] ** 2))
        rows[str(stops)] = dict(intercept=float(coef[0]), slope_codes_per_visibility=float(coef[1]),
                                gap_function_explained_share=explained, gap_pixels=int(moving.sum()),
                                partial_rms=rms(e[p]), partial_resid_rms=rms(resid[p]),
                                partial_r2=float(1 - np.mean(resid[p] ** 2) / np.mean((e[p] - e[p].mean()) ** 2)),
                                bins=table)
    return rows


def lit_umbra_contrast(rl, rf, full, hit, normal):
    """Median umbra/lit film radiance for umbra pixels with a lit pixel on the same
    primitive-neighbourhood (same placement, normals within 25 degrees, <= 3 px)."""
    y = rl @ LUMA
    n = normal[..., :3].astype(float)
    n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-12)
    lit = full & (rf >= 1 - 1e-9)
    umb = full & (rf <= 1e-9)
    best = np.full((H, W), np.nan)
    for r in (1, 2, 3):
        for dy in range(-r, r + 1):
            for dx in range(-r, r + 1):
                if max(abs(dx), abs(dy)) != r:
                    continue
                q = lambda a: np.roll(np.roll(a, dy, 0), dx, 1)
                ok = umb & q(lit) & (q(hit[..., 0]) == hit[..., 0]) & (np.abs(np.einsum("ijk,ijk->ij", n, q(n))) > .906)
                take = ok & np.isnan(best)
                best[take] = (y / np.maximum(q(y), 1e-12))[take]
    v = best[~np.isnan(best)]
    return dict(pairs=int(v.size), umbra_over_lit_quantiles=np.quantile(v, [.25, .5, .75]).tolist())


def source_triangles(source, hit, cover, eye):
    """Visible-triangle statistics from the read-only source asset: edge lengths, vertex
    normal deviation, and the smooth-surface (vertex-normal) shadow origin offset
    computed as in Cycles' shadow_ray_smooth_surface_offset."""
    ys, xs = np.nonzero(cover)
    place = hit[ys, xs, 0].astype(np.int64) - 1
    prim = hit[ys, xs, 1].astype(np.int64)
    bary = hit[ys, xs, 2:].view("<f4").astype(float)
    b = np.c_[1 - bary.sum(-1), bary]
    p = np.zeros((len(ys), 3, 3))
    nv = np.zeros((len(ys), 3, 3))
    for k, (mesh, matrix) in enumerate(source.placements):
        m = place == k
        if not m.any():
            continue
        (_, vertices, indices), = source.meshes[mesh]
        tri = indices[prim[m]]
        rot = matrix[:3, :3].astype(float)
        p[m] = vertices[tri, :3].astype(float) @ rot.T + matrix[:3, 3]
        nv[m] = vertices[tri, 3:6].astype(float) @ rot.T
    nv /= np.maximum(np.linalg.norm(nv, axis=-1, keepdims=True), 1e-12)
    e = np.stack([p[:, 1] - p[:, 0], p[:, 2] - p[:, 1], p[:, 0] - p[:, 2]], 1)
    lengths = np.linalg.norm(e, axis=-1)
    face = np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0])
    area = .5 * np.linalg.norm(face, axis=-1)
    face /= np.maximum(2 * area, 1e-30)[:, None]
    point = np.einsum("ij,ijk->ik", b, p)
    view = eye - point
    face *= np.where(np.einsum("ij,ij->i", face, view) < 0, -1, 1)[:, None]
    nv *= np.where(np.einsum("ijk,ik->ij", nv, face) < 0, -1, 1)[..., None]
    smooth = np.einsum("ij,ijk->ik", b, nv)
    smooth /= np.maximum(np.linalg.norm(smooth, axis=-1, keepdims=True), 1e-12)
    deviation = np.degrees(np.arccos(np.clip(np.einsum("ij,ij->i", smooth, face), -1, 1)))
    # Pseudo smooth surface: project the point onto each vertex tangent plane it lies below.
    offset = np.zeros_like(point)
    for i in range(3):
        d = np.minimum(0, np.einsum("ij,ij->i", point - p[:, i], nv[:, i]))
        offset -= b[:, i:i + 1] * d[:, None] * nv[:, i]
    lift = np.einsum("ij,ij->i", offset, face)
    ndl_face = face @ lb.TO_LIGHT
    ndl_smooth = smooth @ lb.TO_LIGHT
    out = np.full((H, W), np.nan)
    out[ys, xs] = lift
    q = lambda a, qs=(.1, .5, .9, .99): np.quantile(a, qs).tolist()
    return dict(
        longest_edge_mm=q(lengths.max(-1) * 1000), shortest_edge_mm=q(lengths.min(-1) * 1000),
        equal_area_edge_mm=q(np.sqrt(4 * area / np.sqrt(3)) * 1000),
        vertex_normal_deviation_deg=q(deviation),
        smooth_surface_offset_mm=q(lift * 1000),
        smooth_offset_over_1mm_fraction=float((lift > 1e-3).mean()),
        geometric_backlit_shading_lit_fraction=float(((ndl_face <= 0) & (ndl_smooth > 0)).mean()),
        geometric_backlit_fraction=float((ndl_face <= 0).mean()),
    ), out, ndl_face, ndl_smooth, (ys, xs)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--f0", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--asset", type=Path, default=Path("D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla"))
    args = ap.parse_args()
    probe_spec = importlib.util.spec_from_file_location("probe", REPO / "scripts/quarry-shadow-probe.py")
    probe = importlib.util.module_from_spec(probe_spec)
    probe_spec.loader.exec_module(probe)
    source = probe.Source(args.asset)
    t0 = time.perf_counter()
    lb.R = fs.FROZEN / "runs/quarry-b6"
    lb.REFERENCE = fs.FROZEN / "gate-final/reference"
    lb.NATIVE = fs.NATIVE
    out = {}
    for pose in (0, 1):
        point = np.load(lb.REFERENCE / f"pose-{pose}-reference.npz")["ref"]
        natives = {}
        for case, name in (("vsm", "Unreal-VSM"), ("hrt1", "Unreal-HRT")):
            z = np.load(lb.native_dir(case, pose) / "native.npz")
            natives[name] = (z["visibility"].astype(float), z["valid"])
        refs = {}
        for policy in ("policy", "minimal"):
            directory = args.f0 / f"quarry-{policy}-p{pose}"
            d = fs.decode(directory, pose)
            refs[policy] = d
            r = {}
            cover, n, rf = d["cover"], d["n"], d["ref"]
            vc = np.fromfile(directory / f"pose-{pose}.exact.u32", dtype="<u4").reshape(H, W) / 256
            lc = hdr(directory / f"pose-{pose}.exact.rgba16f")
            rl = d["hdr"][..., :3]
            full = cover & (n == 256)
            eps = 1e-9
            part = cover & (rf > eps) & (rf < 1 - eps)
            pops = {
                "film-partial": part,
                "film-partial-1surface": part & full,
                "film-lit-1surface": full & (rf >= 1 - eps),
                "film-umbra-1surface": full & (rf <= eps),
                "multi-surface": cover & (n < 256),
                "uncovered-centre": ~cover,
            }
            # 1. Estimator versus sampling position, on film partials.
            pp = part & (n > 0)
            e_exact = vc - rf
            e_point = point - rf
            r["visibility"] = dict(
                exact_rms=rms(e_exact[pp]), point_reference_rms=rms(e_point[pp]),
                exact_minus_point_rms=rms((vc - point)[pp]),
                point_share_of_exact_mse=float(np.mean(e_point[pp] ** 2) / np.mean(e_exact[pp] ** 2)),
                missed=int((pp & (rf <= .9) & (vc >= .99)).sum()),
                missed_also_by_point=int((pp & (rf <= .9) & (vc >= .99) & (point >= .99)).sum()),
                film_sigma_256_rms=rms(d["sigma"][pp]),
                footprint_sd_1_rms=rms(np.sqrt(d["variance"][pp])),
            )
            s1 = r["visibility"]["footprint_sd_1_rms"]
            s256 = r["visibility"]["film_sigma_256_rms"]
            beta = float(np.log(s1 / s256) / np.log(256))
            r["visibility"]["stratified_rate_beta"] = beta
            r["visibility"]["predicted_rms_by_samples"] = {str(k): s1 * k ** -beta for k in (1, 2, 4, 8, 16, 32, 64)}
            # 2. Radiance: centre Exact capture against the film radiance R_L.
            rad = {}
            total = {}
            for stops in (0, 4):
                err = display(lc, stops) - display(rl, stops)
                e2 = np.mean(err ** 2, -1)
                total[stops] = float(e2.sum())
                for name, m in pops.items():
                    row = rad.setdefault(name, dict(n=int(m.sum())))
                    row[f"codes_rms_{stops}"] = float(np.sqrt(e2[m].mean()) * 255) if m.any() else None
                    row[f"share_of_image_sq_error_{stops}"] = float(e2[m].sum() / total[stops])
                    row[f"frac_gt_4codes_{stops}"] = float((np.sqrt(e2[m]) * 255 > 4).mean()) if m.any() else None
            r["radiance"] = rad
            # 3. Visibility-only correction where D is measurable: L' = Lc + D (R_F - Vc).
            dsun, amb, known, rel = fit_direct(pose, vc, lc, cover)
            kk = known & part & full
            lv = lc + np.nan_to_num(dsun) * (rf - vc)[..., None]
            fix = {}
            for stops in (0, 4):
                e_pt = np.sqrt(np.mean((display(lc, stops) - display(rl, stops)) ** 2, -1))[kk] * 255
                e_fx = np.sqrt(np.mean((display(lv, stops) - display(rl, stops)) ** 2, -1))[kk] * 255
                fix[str(stops)] = dict(point_codes_rms=rms(e_pt), visibility_fixed_codes_rms=rms(e_fx))
            sun_share = (np.nan_to_num(dsun) @ LUMA) / np.maximum((np.nan_to_num(amb) + np.nan_to_num(dsun)) @ LUMA, 1e-12)
            fix["n"] = int(kk.sum())
            fix["linearity_rel_resid_p99"] = float(np.nanquantile(rel[kk], .99))
            fix["direct_share_of_lit_quantiles"] = np.quantile(sun_share[kk], [.1, .5, .9]).tolist()
            r["visibility_only_fix_measurable_subset"] = fix
            r["radiance_vs_visibility_gap"] = visibility_regression(lc, rl, vc, rf, full, part)
            r["lit_umbra_contrast"] = lit_umbra_contrast(rl, rf, full, d["hit"], np.fromfile(directory / f"pose-{pose}.normal.f32", dtype="<f4").reshape(H, W, 4))
            # 4. Where film partials sit: normal spread and sub-pixel triangle counts.
            spread, distinct = neighbour_spread(np.fromfile(directory / f"pose-{pose}.normal.f32", dtype="<f4").reshape(H, W, 4),
                                                d["hit"])
            bins = {"spread<5": spread < 5, "5-20": (spread >= 5) & (spread < 20), ">=20": spread >= 20}
            r["normal_spread_bins"] = {}
            for name, m in bins.items():
                mp = pp & m
                r["normal_spread_bins"][name] = dict(
                    covered_fraction=float((cover & m).sum() / cover.sum()),
                    film_partial_fraction=float(mp.sum() / pp.sum()),
                    exact_share_of_partial_mse=float((e_exact[mp] ** 2).sum() / (e_exact[pp] ** 2).sum()),
                    lit_codes_rms_4=rms(np.sqrt(np.mean((display(lc, 4) - display(rl, 4)) ** 2, -1))[pops["film-lit-1surface"] & m] * 255),
                )
            r["distinct_primitives_3x3_quantiles"] = np.quantile(distinct[cover], [.1, .5, .9]).tolist()
            dist = d["surface_depth"]
            footprint = dist * 0.001028
            if policy == "policy":
                r["triangles"] = triangle_sizes(d["world"], d["hit"], footprint)
                r["footprint_mm_quantiles"] = (np.quantile(footprint[cover], [.1, .5, .9]) * 1000).tolist()
            # 5. Unreal: native masks carry the same centre-point aliasing.
            un = {}
            for name, (v, valid) in natives.items():
                m = pp & valid
                eu = v - rf
                a = v - point
                b = point - rf
                un[name] = dict(n=int(m.sum()), film_rms=rms(eu[m]), point_rms=rms(a[m]), point_vs_film_rms=rms(b[m]),
                                cross_share=float(2 * np.mean(a[m] * b[m]) / np.mean(eu[m] ** 2)),
                                aliasing_share=float(np.mean(b[m] ** 2) / np.mean(eu[m] ** 2)),
                                corr_with_exact_error=float(np.corrcoef(eu[m], e_exact[m])[0, 1]))
            r["unreal"] = un
            out[f"{pose}/{policy}"] = r
            print(pose, policy, json.dumps(r["visibility"]), flush=True)
        # 6. Receiver-policy difference: where the 1 mm offset removes occlusion.
        a, b = refs["policy"], refs["minimal"]
        both = a["cover"] & b["cover"] & (a["n"] > 0) & (b["n"] > 0)
        diff = a["ref"] - b["ref"]
        normal = np.fromfile(args.f0 / f"quarry-policy-p{pose}/pose-{pose}.normal.f32", dtype="<f4").reshape(H, W, 4)[..., :3].astype(float)
        eye = np.asarray(a["info"]["eye"][:3])
        normal *= np.where(np.einsum("ijk,ijk->ij", normal, eye - a["world"][..., :3]) < 0, -1, 1)[..., None]
        ndl = normal @ lb.TO_LIGHT
        spread, _ = neighbour_spread(np.fromfile(args.f0 / f"quarry-policy-p{pose}/pose-{pose}.normal.f32", dtype="<f4").reshape(H, W, 4), a["hit"])
        big = both & (diff > .05)
        neg = both & (diff < -.05)
        lab, count = ndimage.label(big)
        sizes = np.bincount(lab.ravel())[1:] if count else np.array([0])
        flat_lit = big & (ndl > .5) & (spread < 5)
        tri, lift, ndl_face, ndl_smooth, (ys, xs) = source_triangles(source, a["hit"], a["cover"], eye)
        sel = big[ys, xs]
        backlit = a["cover"] & both & (ndl <= 0)
        out[f"{pose}/source-triangles"] = tri | dict(
            smooth_surface_offset_mm_on_brighter=np.quantile(lift[big] * 1000, [.1, .5, .9]).tolist(),
            vertex_normal_backlit_shading_lit_on_brighter=float(((ndl_face[sel] <= 0) & (ndl_smooth[sel] > 0)).mean()),
            backlit_mean_film_policy=float(a["ref"][backlit].mean()), backlit_mean_film_minimal=float(b["ref"][backlit].mean()),
            backlit_n=int(backlit.sum()))
        print(pose, "source triangles", json.dumps(out[f"{pose}/source-triangles"]), flush=True)
        out[f"{pose}/policy-difference"] = dict(
            n=int(both.sum()), brighter_with_1mm=int(big.sum()), darker_with_1mm=int(neg.sum()),
            mean=float(diff[both].mean()),
            camera_facing_ndl_quantiles_of_brighter=np.quantile(ndl[big], [.1, .5, .9]).tolist(),
            backlit_fraction_of_brighter=float((ndl[big] <= 0).mean()),
            grazing_fraction_of_brighter=float(((ndl[big] > 0) & (ndl[big] < .26)).mean()),
            flat_strongly_lit_fraction=float(flat_lit.sum() / max(big.sum(), 1)),
            normal_spread_quantiles_brighter=np.quantile(spread[big], [.1, .5, .9]).tolist(),
            normal_spread_quantiles_all=np.quantile(spread[both], [.1, .5, .9]).tolist(),
            component_count=int(count), component_size_quantiles=np.quantile(sizes, [.5, .9, .99]).tolist(),
            minimal_film_partial_fraction_of_covered=float((both & (b["ref"] > 1e-9) & (b["ref"] < 1 - 1e-9)).sum() / both.sum()),
        )
        print(pose, "policy difference", json.dumps(out[f"{pose}/policy-difference"]), flush=True)
    out["seconds"] = time.perf_counter() - t0
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "film-gap.json").write_text(json.dumps(out, indent=2, allow_nan=False))
    print("seconds", out["seconds"])


if __name__ == "__main__":
    main()
