"""T1 CPU phase: temporal jittered-primary accumulation, replayed from R1's saved samples.

Reads R1 (viewed policy), F0 helpers and accept-b7 Interactive captures in place; writes only
under --out. Four parts:
  1. Static accumulation curves N=1..16: sample orderings x {Exact-ray, modelled 8-ray} per
     population and channel, scored against an independent 240-sample complement reference.
  2. Exact (point) and Interactive (point, 8 rays) baselines against the same reference.
  3. Reprojection transfer at the registered 1 cm step: converged pose-0 film fetched through
     the pose-1 centre hit, under several validation rules, scored against the pose-1 film.
  4. Fourier steady-state model of exponential history under sub-pixel pans (bias + iid noise).

py -3 -B scripts/m6-temporal-replay.py --out D:/Coding/renderlab-work/m6-exact-quality/t1
Math-library threads should be set to two.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import time

import numpy as np
from PIL import Image, ImageDraw

spec = importlib.util.spec_from_file_location("film_score", Path(__file__).with_name("m6-film-score.py"))
f0 = importlib.util.module_from_spec(spec); spec.loader.exec_module(f0)

R1 = Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
B7 = Path("D:/Coding/renderlab-work/m6-level-b-probe/runs/accept-b7")
H, W = 1080, 1920
# Saved sample i is stratum row i, column 7i mod 16: a rank-1 lattice. Bit-reversed row order
# makes every 2^k prefix stratified; a common row shift is a toroidal lattice shift.
PROGRESSIVE = np.array([int(f"{k:04b}"[::-1], 2) for k in range(16)])
CHANNELS = ("L0", "L4", "D0", "D4", "F")
REPORT_POPS = ("partial", "faint", "narrow", "contact", "isolated", "lit", "occluded", "backlit",
               "multi-surface", "background-centre", "all-covered", "image")


def codes(hdr, stops):
    x = np.maximum(0, hdr * (2 ** stops)); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - .055)


def code_slope(hdr, stops):
    """d codes / d radiance, for propagating small linear noise into display codes."""
    s = 2 ** stops; x = np.maximum(0, hdr * s); y = x / (1 + x); dy = s / (1 + x) ** 2
    return 255 * np.where(y <= .0031308, 12.92, 1.055 / 2.4 * np.power(np.maximum(y, 1e-12), 1 / 2.4 - 1)) * dy


def f16(path, crop, n=4):
    a = np.fromfile(path, dtype="<f2").reshape(H, W, n)[crop][..., :3]
    return a.astype(np.float32)


def vbin(count):
    """Noise-model bin from a 0..256 visibility count: 0, 1..32 (width 8), 33 for fully lit."""
    return np.where(count == 0, 0, np.where(count >= 256, 33, 1 + (count - 1) // 8))


def noise_pools():
    """Empirical 8-ray residuals V8 - V256 at pixel centres, binned by V256 (accept-b7, M)."""
    pools = [[] for _ in range(34)]
    for pose in (0, 1):
        v = np.fromfile(R1 / f"quarry-viewed-p{pose}/pose-{pose}.exact.u32", dtype="<u4").reshape(H, W).astype(np.int64)
        cover = np.fromfile(R1 / f"quarry-viewed-p{pose}/pose-{pose}.hit.u32", dtype="<u4").reshape(H, W, 4)[..., 0] != 0
        for seed in range(4):
            v8 = np.fromfile(B7 / f"pose-{pose}-seed-{seed}-interactive.u32", dtype="<u4").reshape(H, W).astype(np.int64)
            b = vbin(v[cover]); r = (v8[cover] - v[cover]) / 256
            for k in range(34):
                pools[k].append(r[b == k])
    pools = [np.concatenate(p) for p in pools]
    partial = np.concatenate(pools[1:33])
    summary = dict(partial_rms=float(np.sqrt(np.mean(partial ** 2))), partial_n=int(partial.size),
                   lit_nonzero=float(np.mean(pools[33] != 0)), umbra_nonzero=float(np.mean(pools[0] != 0)),
                   bin_rms=[float(np.sqrt(np.mean(p ** 2))) if p.size else 0 for p in pools],
                   bin_n=[int(p.size) for p in pools])
    return pools, summary


def draw_v8(count, pools, rng):
    """Interactive-like visibility at a receiver with Exact count; NaN where not recorded."""
    v = count / 256.0; out = np.full(count.shape, np.nan)
    valid = count != 65535; b = vbin(count)
    out[valid] = v[valid]
    for k in range(34):
        m = valid & (b == k)
        if m.any() and pools[k].size:
            out[m] = np.clip(v[m] + pools[k][rng.integers(pools[k].size, size=int(m.sum()))], 0, 1)
    return out


def with_v8(hdr, sun, v, v8):
    """L = A + U V with U = D / V: replace V by V8 where the Exact receiver was partial or lit."""
    ok = np.isfinite(v8) & (v > 0)
    u = np.divide(sun, v[..., None], out=np.zeros_like(sun), where=ok[..., None])
    d = np.where(ok, v8 - v, 0)[..., None].astype(np.float32)
    return hdr + u * d, sun + u * d


def pose_data(pose, crop, pools, rng):
    d = R1 / f"quarry-viewed-p{pose}"
    ref = f0.decode(d, pose)
    point = np.load(f0.FROZEN / f"gate-final/reference/pose-{pose}-reference.npz")["ref"]
    pops = f0.populations(ref, point)
    pops["backlit"] = ref["cover"] & ~ref["front"]
    pops["all-covered"] = ref["cover"]
    pops["image"] = np.ones_like(ref["cover"])
    pops["multi-surface"] = ref["cover"] & (ref["n"] < 256)
    pops["background-centre"] = ~ref["cover"]
    raw = np.fromfile(d / f"pose-{pose}.film.bin", dtype=f0.DTYPE).reshape(H, W)
    direct = np.fromfile(d / f"pose-{pose}.direct.bin", dtype=f0.DTYPE).reshape(H, W)["radiance"][..., :3].astype(np.float64) / 256
    samples = dict(L=[], D=[], V=[], L8=[], D8=[], V8=[])
    full_sum = np.zeros((H, W, 3)); sun_sum = np.zeros((H, W, 3)); vis_sum = np.zeros((H, W)); acc_n = np.zeros((H, W))
    for i in range(16):
        L = np.fromfile(d / f"samples/{i:02}.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
        if i == 0:
            first = L
        D = np.fromfile(d / f"samples/{i:02}.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
        c = np.fromfile(d / f"samples/{i:02}.visibility.u16", dtype="<u2").reshape(H, W).astype(np.int64)
        full_sum += L; sun_sum += D; vis_sum += np.where(c != 65535, c, 0); acc_n += c != 65535
        v = np.where(c != 65535, c / 256.0, np.nan); v8 = draw_v8(c, pools, rng)
        L8, D8 = with_v8(L, D, np.nan_to_num(v), v8)
        for key, value in (("L", L), ("D", D), ("V", v), ("L8", L8), ("D8", D8), ("V8", v8)):
            samples[key].append(value[crop])
    # Independent references: remove the 16 replayed strata from the 256-stratum sums.
    comp_L = (ref["hdr"][..., :3] * 256 - full_sum) / 240
    comp_D = (direct * 256 - sun_sum) / 240
    counts = raw["counts"].astype(np.float64)
    rest_n = counts[..., 2] + counts[..., 3] - acc_n
    comp_F = np.divide(counts[..., 0] + counts[..., 1] - vis_sum, 256 * rest_n, out=np.full((H, W), np.nan), where=rest_n > 0)
    checks = dict(comp_L_min=float(comp_L.min()), comp_D_min=float(comp_D.min()),
                  saved_accepted_over_reference=int((acc_n > counts[..., 2] + counts[..., 3]).sum()),
                  saved_visible_over_reference=int((vis_sum > counts[..., 0] + counts[..., 1] + 1e-9).sum()),
                  rejected_covered_samples=int(sum(int((np.isnan(v) & ref["cover"][crop]).sum()) for v in samples["V"])),
                  covered_samples=int(16 * ref["cover"].sum()))
    outside = ~crop_mask(crop)
    assert not ref["cover"][outside].any(), "crop must contain every covered pixel"
    for n, est in ((1, first), (16, full_sum / 16)):
        e2 = np.mean((codes(est, 0) - codes(comp_L, 0)) ** 2, -1)[outside]
        checks[f"outside_crop_L0_rms_N{n}"] = float(np.sqrt(e2.mean()))
        checks[f"outside_crop_L0_share_of_image_sumsq_N{n}"] = float(e2.sum())
    exact_v = np.fromfile(d / f"pose-{pose}.exact.u32", dtype="<u4").reshape(H, W) / 256
    exact_L = np.fromfile(d / f"pose-{pose}.exact.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
    exact_D = np.fromfile(d / f"pose-{pose}.exact.sun.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
    cover = ref["cover"]
    centre_v = np.where(cover, exact_v, np.nan)
    centre_v8 = np.where(cover, draw_v8(np.where(cover, np.rint(exact_v * 256), 65535).astype(np.int64), pools, rng), np.nan)
    centre_L8, centre_D8 = with_v8(exact_L, exact_D, np.nan_to_num(centre_v), centre_v8)
    interactive = []
    for seed in range(4):
        iL = np.fromfile(B7 / f"pose-{pose}-seed-{seed}-interactive.rgba16f", dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)
        iv = np.fromfile(B7 / f"pose-{pose}-seed-{seed}-interactive.u32", dtype="<u4").reshape(H, W) / 256
        u = np.divide(exact_D, exact_v[..., None], out=np.zeros_like(exact_D), where=(cover & (exact_v > 0))[..., None])
        iD = np.where((cover & (exact_v > 0))[..., None], u * iv[..., None], exact_D)
        interactive.append(dict(L=iL[crop], D=iD[crop], V=np.where(cover, iv, np.nan)[crop]))
    return dict(ref=ref, pops={k: v[crop] for k, v in pops.items()}, pops_full=pops, comp=dict(L=comp_L[crop], D=comp_D[crop], F=comp_F[crop]),
                comp_full=dict(L=comp_L, D=comp_D), direct_full=direct, samples={k: np.stack(v) for k, v in samples.items()},
                exact=dict(L=exact_L[crop], D=exact_D[crop], V=centre_v[crop]),
                centre8=dict(L=centre_L8[crop], D=centre_D8[crop], V=centre_v8[crop]),
                interactive=interactive, checks=checks, cover=cover[crop])


def crop_mask(crop):
    m = np.zeros((H, W), bool); m[crop] = True; return m


class Scorer:
    """Per-population squared-error accumulation for one estimate against the complement reference."""

    def __init__(self, data):
        self.pops = {k: data["pops"][k] for k in data["pops"]}
        self.comp = data["comp"]; self.cover = data["cover"]
        self.ref_codes = {(c, s): codes(self.comp[c], s) for c in ("L", "D") for s in (0, 4)}
        self.counts = {k: int(v.sum()) for k, v in data["pops_full"].items()}

    def score(self, L, D, V):
        out = {}
        for c, est in (("L", L), ("D", D)):
            for s in (0, 4):
                e2 = np.mean((codes(est, s) - self.ref_codes[(c, s)]) ** 2, -1)
                row = {}
                for name in REPORT_POPS:
                    m = self.pops[name]
                    # Pixels outside the crop are background-only; their error is checked separately.
                    row[name] = (float(e2[m].sum()), self.counts[name])
                out[f"{c}{s}"] = row
        valid = self.cover & np.isfinite(V) & np.isfinite(self.comp["F"])
        e2 = np.where(valid, (np.nan_to_num(V) - np.nan_to_num(self.comp["F"])) ** 2, 0)
        row = {}
        for name in REPORT_POPS:
            if name in ("background-centre", "image"):
                continue
            m = self.pops[name] & self.cover
            row[name] = (float(e2[m & valid].sum()), int((m & valid).sum()), int((m & ~valid).sum()))
        out["F"] = row
        return out


def rms(table):
    return {ch: {p: (float(np.sqrt(v[0] / v[1])) if v[1] else None) for p, v in rows.items()} for ch, rows in table.items()}


def orderings(shape, rng):
    shift = rng.integers(16, size=shape)
    perm = np.argsort(rng.random((16,) + shape), axis=0)
    # Lattice symmetries that fix the centre point (row 8): identity, point reflection, transpose
    # (row r -> 7r, since 7 * 7 = 1 mod 16) and both. Each preserves every 2^k prefix set.
    sym = rng.integers(4, size=shape)
    mult = np.choose(sym, [1, 15, 7, 9])
    centred = [(PROGRESSIVE[k] + 8) % 16 for k in range(16)]
    return {
        "saved": [np.full(shape, k) for k in range(16)],
        "progressive": [np.full(shape, PROGRESSIVE[k]) for k in range(16)],
        "progressive-shifted": [(PROGRESSIVE[k] + shift) % 16 for k in range(16)],
        "random": [perm[k] for k in range(16)],
        "centre-first": [None] + [(PROGRESSIVE[k] + shift) % 16 for k in range(15)],
        "centre-progressive": [np.full(shape, c) for c in centred],
        "centre-progressive-scrambled": [(c * mult) % 16 for c in centred],
    }


def take(stack, idx):
    """Per-pixel gather of sample idx from a (16, h, w[, 3]) stack."""
    if stack.ndim == 4:
        return np.take_along_axis(stack, idx[None, ..., None], 0)[0]
    return np.take_along_axis(stack, idx[None], 0)[0]


def replay(data, rng):
    scorer = Scorer(data)
    s = data["samples"]; shape = data["cover"].shape
    result = dict(curves={}, single={}, baselines={})
    result["baselines"]["exact"] = rms(scorer.score(data["exact"]["L"], data["exact"]["D"], data["exact"]["V"]))
    result["baselines"]["interactive"] = [rms(scorer.score(i["L"], i["D"], i["V"])) for i in data["interactive"]]
    for i in range(16):
        result["single"][i] = rms(scorer.score(s["L"][i], s["D"][i], s["V"][i]))
    sigma = {}; orders = orderings(shape, rng)
    for noise in ("exact-rays", "8-rays"):
        L, D, V = (s["L"], s["D"], s["V"]) if noise == "exact-rays" else (s["L8"], s["D8"], s["V8"])
        centre = data["exact"] if noise == "exact-rays" else data["centre8"]
        # Per-sample error variance against the film, for the motion model (iid assumption).
        sigma[noise] = np.mean((L.astype(np.float64) - data["comp"]["L"][None]) ** 2, 0)
        sigma[noise + "-D"] = np.mean((D.astype(np.float64) - data["comp"]["D"][None]) ** 2, 0)
        for name, order in orders.items():
            sL = np.zeros(shape + (3,)); sD = np.zeros(shape + (3,)); sV = np.zeros(shape); nV = np.zeros(shape)
            curve = []
            for k, idx in enumerate(order):
                if idx is None:
                    l, dd, v = centre["L"], centre["D"], centre["V"]
                else:
                    l, dd, v = take(L, idx), take(D, idx), take(V, idx)
                sL += l; sD += dd; ok = np.isfinite(v); sV += np.where(ok, v, 0); nV += ok
                vm = np.divide(sV, nV, out=np.full(shape, np.nan), where=nV > 0)
                curve.append(rms(scorer.score(sL / (k + 1), sD / (k + 1), vm)))
            result["curves"][f"{name}/{noise}"] = curve
            print("  ", name, noise, "N=8 L0 image", round(curve[7]["L0"]["image"], 3), flush=True)
    return result, sigma


def reconcile_r1(data, crop):
    """R1's metric: linear-HDR RMS over (pixel, channel) against the full 256-sample film."""
    full = data["ref"]["hdr"][..., :3][crop]; s = data["samples"]["L"]; out = {}
    counts = {k: int(v.sum()) for k, v in data["pops_full"].items()}
    for name, rows in (("full-reference", full), ("complement", data["comp"]["L"])):
        acc = np.zeros(full.shape); curve = []
        for k in range(16):
            acc += s[k]
            e2 = np.sum((acc / (k + 1) - rows) ** 2, -1)
            curve.append({pop: float(np.sqrt(e2[data["pops"][pop]].sum() / (3 * counts[pop]))) for pop in ("partial", "all-covered", "image")})
        e2 = np.sum((data["exact"]["L"] - rows) ** 2, -1)
        exact = {pop: float(np.sqrt(e2[data["pops"][pop]].sum() / (3 * counts[pop]))) for pop in ("partial", "all-covered", "image")}
        out[name] = dict(curve=curve, exact=exact, first_below_exact={pop: next((k + 1 for k, c in enumerate(curve) if c[pop] <= exact[pop]), None) for pop in exact})
    return out


def fit(curve, lo=4):
    n = np.arange(1, len(curve) + 1); y = np.array(curve, float)
    keep = (n >= lo) & np.isfinite(y) & (y > 0)
    if keep.sum() < 3:
        return float("nan"), float("nan")
    slope, intercept = np.polyfit(np.log(n[keep]), np.log(y[keep]), 1)
    return float(-slope), float(np.exp(intercept))


def summarise(result):
    exact = result["baselines"]["exact"]; inter = result["baselines"]["interactive"]
    centre = result["single"][8]
    out = {}
    for key, curve in result["curves"].items():
        rows = {}
        for ch in CHANNELS:
            for pop in REPORT_POPS:
                e = exact[ch].get(pop)
                ys = [c[ch].get(pop) for c in curve]
                if e is None or any(y is None for y in ys):
                    continue
                i_rms = float(np.mean([i[ch][pop] for i in inter]))
                ev = centre[ch].get(pop); bar = min(e, ev)
                beta, amp = fit(ys)
                first = next((k + 1 for k, y in enumerate(ys) if y <= e), None)
                rows[f"{ch}/{pop}"] = dict(exact=e, exact_viewed_proxy=ev, bar=bar, interactive=i_rms,
                                           first_below_bar=next((k + 1 for k, y in enumerate(ys) if y <= bar), None),
                                           ratio8_bar=ys[7] / bar if bar > 0 else None, N1=ys[0], N2=ys[1], N4=ys[3], N8=ys[7], N16=ys[15],
                                           ratio8=ys[7] / e if e > 0 else None, first_below_exact=first, beta_4_16=beta,
                                           model32=amp * 32 ** -beta if np.isfinite(beta) else None,
                                           model_frames_to_exact=(amp / e) ** (1 / beta) if np.isfinite(beta) and beta > 0 and e > 0 else None,
                                           above_interactive=[k + 1 for k, y in enumerate(ys) if y > i_rms * 1.0])
        out[key] = rows
    return out


# ---------------------------------------------------------------- reprojection transfer (M)

def project(info, X):
    M = np.array(info["camera_inverse"]).reshape(4, 4).T
    P = np.linalg.inv(M)
    c = np.concatenate([X, np.ones(X.shape[:-1] + (1,))], -1) @ P.T
    ndc = c[..., :3] / c[..., 3:4]
    return (ndc[..., 0] + 1) / 2 * W - 0.5, (1 - ndc[..., 1]) / 2 * H - 0.5


def transfer(p0, p1, sigma0, sigma1, crop):
    """Fetch converged pose-0 film through each pose-1 centre hit; score against the pose-1 film."""
    r0, r1 = p0["ref"], p1["ref"]
    eye0 = np.asarray(r0["info"]["eye"][:3])
    hit0, hit1 = r0["hit"], r1["hit"]
    pl0, pl1 = hit0[..., 0].astype(np.int64), hit1[..., 0].astype(np.int64)
    d0 = np.linalg.norm(r0["world"][..., :3] - eye0, axis=-1)
    eye1 = np.asarray(r1["info"]["eye"][:3])
    d1 = np.linalg.norm(r1["world"][..., :3] - eye1, axis=-1)
    n0 = np.fromfile(R1 / "quarry-viewed-p0/pose-0.normal.f32", dtype="<f4").reshape(H, W, 4)[..., :3]
    n1 = np.fromfile(R1 / "quarry-viewed-p1/pose-1.normal.f32", dtype="<f4").reshape(H, W, 4)[..., :3]
    cover1 = r1["cover"]
    X = r1["world"][..., :3].astype(np.float64)
    qx, qy = project(r0["info"], X)
    qx = np.where(cover1, qx, np.arange(W)[None, :]); qy = np.where(cover1, qy, np.arange(H)[:, None])
    dX = np.where(cover1, np.linalg.norm(X - eye0, axis=-1), 0)
    fp = np.maximum(dX, 1e-6) * 1.028e-3
    x0 = np.floor(qx).astype(np.int64); y0 = np.floor(qy).astype(np.int64); fx = qx - x0; fy = qy - y0
    film0 = dict(L=r0["hdr"][..., :3], D=p0["direct_full"]); film1 = dict(L=r1["hdr"][..., :3], D=p1["direct_full"])
    taps = [(0, 0, (1 - fx) * (1 - fy)), (1, 0, fx * (1 - fy)), (0, 1, (1 - fx) * fy), (1, 1, fx * fy)]
    tapinfo = []
    for dx, dy, w in taps:
        tx = x0 + dx; ty = y0 + dy; inside = (tx >= 0) & (tx < W) & (ty >= 0) & (ty < H)
        tx = np.clip(tx, 0, W - 1); ty = np.clip(ty, 0, H - 1)
        same = pl0[ty, tx] == pl1
        rel = np.abs(d0[ty, tx] - dX) / np.maximum(dX, 1e-6)
        dfp = np.abs(d0[ty, tx] - dX) / fp
        ndot = np.einsum("ijk,ijk->ij", n0[ty, tx], n1)
        tri = same & (hit0[ty, tx, 1] == hit1[..., 1])
        # Reverse test: is the tap's own surface point still visible within 1.5 px of this pixel?
        Y = r0["world"][ty, tx, :3].astype(np.float64)
        ux, uy = project(r1["info"], Y)
        tap_bg = pl0[ty, tx] == 0
        ux = np.where(tap_bg, tx, ux); uy = np.where(tap_bg, ty, uy)
        uxi = np.clip(np.rint(ux).astype(np.int64), 0, W - 1); uyi = np.clip(np.rint(uy).astype(np.int64), 0, H - 1)
        near = (np.abs(ux - np.arange(W)[None, :]) <= 1.5) & (np.abs(uy - np.arange(H)[:, None]) <= 1.5)
        dY = np.linalg.norm(Y - eye1, axis=-1)
        seen = np.where(tap_bg, pl1[uyi, uxi] == 0, (pl1[uyi, uxi] == pl0[ty, tx]) & (np.abs(d1[uyi, uxi] - dY) <= .02 * np.maximum(dY, 1e-6)))
        tapinfo.append(dict(tx=tx, ty=ty, w=w, inside=inside, same=same, rel=rel, dfp=dfp, ndot=ndot, tri=tri, reverse=near & seen))
    bgcentre = ~cover1
    rules = {
        "none": lambda t: t["inside"],
        "instance": lambda t: t["inside"] & t["same"],
        "instance+depth1%": lambda t: t["inside"] & t["same"] & (t["rel"] <= .01),
        "instance+depth2%": lambda t: t["inside"] & t["same"] & (t["rel"] <= .02),
        "instance+depth4fp": lambda t: t["inside"] & t["same"] & (t["dfp"] <= 4),
        "instance+depth2%+normal0": lambda t: t["inside"] & t["same"] & (t["rel"] <= .02) & (t["ndot"] >= 0),
        "instance+depth2%+normal0.5": lambda t: t["inside"] & t["same"] & (t["rel"] <= .02) & (t["ndot"] >= .5),
        "instance+depth2%+normal0.9": lambda t: t["inside"] & t["same"] & (t["rel"] <= .02) & (t["ndot"] >= .9),
        "instance+depth2%+normal0.999": lambda t: t["inside"] & t["same"] & (t["rel"] <= .02) & (t["ndot"] >= .999),
        "triangle": lambda t: t["inside"] & t["tri"],
        "forward-or-reverse": lambda t: t["inside"] & ((t["same"] & (t["rel"] <= .02)) | t["reverse"]),
        "forward4fp-or-reverse": lambda t: t["inside"] & ((t["same"] & (t["dfp"] <= 4)) | t["reverse"]),
    }
    pops = p1["pops_full"]
    e_exact = {}; e_centre = {}
    for c in ("L", "D"):
        ex = p1["exact"][c]
        for s in (0, 4):
            # Exact on the full crop is enough: the reported populations lie inside it.
            full = film1[c].astype(np.float32).copy(); full[crop] = ex
            e_exact[f"{c}{s}"] = np.mean((codes(full, s) - codes(film1[c], s)) ** 2, -1)
            full = film1[c].astype(np.float32).copy(); full[crop] = p1["centre"][c]
            e_centre[f"{c}{s}"] = np.mean((codes(full, s) - codes(film1[c], s)) ** 2, -1)
    # Reference-noise floor of a film-to-film comparison: iid bound var/256 at both poses.
    noise = {}
    for c, key in (("L", "exact-rays"), ("D", "exact-rays-D")):
        for s in (0, 4):
            v0 = np.zeros((H, W, 3)); v1 = np.zeros((H, W, 3)); v0[crop] = sigma0[key]; v1[crop] = sigma1[key]
            noise[f"{c}{s}"] = np.mean(code_slope(film1[c], s) ** 2 * (v0 + v1) / 256, -1)
    out = dict(rules={}, motion=dict(), depth_quantiles={})
    sx = np.where(cover1, qx - np.arange(W)[None, :], np.nan); sy = np.where(cover1, qy - np.arange(H)[:, None], np.nan)
    out["motion"] = dict(dx_quantiles=np.nanquantile(sx, [.01, .5, .99]).tolist(), dy_quantiles=np.nanquantile(sy, [.01, .5, .99]).tolist(),
                         frac_x_median=float(np.nanmedian(np.abs(sx - np.round(sx)))), frac_y_median=float(np.nanmedian(np.abs(sy - np.round(sy)))))
    t0 = tapinfo[0]; m = cover1 & t0["inside"] & t0["same"]
    out["depth_quantiles"] = dict(rel=np.quantile(t0["rel"][m], [.5, .9, .99, .999]).tolist(), footprints=np.quantile(t0["dfp"][m], [.5, .9, .99, .999]).tolist(),
                                  ndot=np.quantile(t0["ndot"][m], [.01, .1, .28, .5]).tolist(), same_triangle=float(t0["tri"][m].mean()))
    report = ("partial", "lit", "occluded", "contact", "multi-surface", "all-covered", "background-centre", "image")

    film1_codes = {(c, s): codes(film1[c], s) for c in ("L", "D") for s in (0, 4)}

    def scored(values, accepted):
        rows = {}
        for ch in ("L0", "L4", "D0", "D4"):
            c, s = ch[0], int(ch[1])
            e2 = np.mean((codes(values[c], s) - film1_codes[(c, s)]) ** 2, -1)
            for pop in report:
                pm = pops[pop]; a = pm & accepted; r_ = pm & ~accepted
                rows[f"{ch}/{pop}"] = dict(accepted=float(a.sum() / max(pm.sum(), 1)),
                                           rms_accepted=float(np.sqrt(e2[a].mean())) if a.any() else None,
                                           noise_floor=float(np.sqrt(noise[ch][a].mean())) if a.any() else None,
                                           exact_on_accepted=float(np.sqrt(e_exact[ch][a].mean())) if a.any() else None,
                                           exact=float(np.sqrt(e_exact[ch][pm].mean())) if pm.any() else None,
                                           exact_viewed_proxy_on_accepted=float(np.sqrt(e_centre[ch][a].mean())) if a.any() else None,
                                           rejected_n=int(r_.sum()))
        return rows

    for name, rule in rules.items():
        num = {c: np.zeros((H, W, 3)) for c in ("L", "D")}; den = np.zeros((H, W))
        for t in tapinfo:
            ok = rule(t) & cover1
            # Background centres are static under a translation: reuse the same pixel if it was background.
            w = np.where(ok, t["w"], 0)
            den += w
            for c in num:
                num[c] += w[..., None] * film0[c][t["ty"], t["tx"]]
        bg_ok = bgcentre & (pl0 == 0)
        accepted = (cover1 & (den > 1e-9)) | bg_ok
        values = {}
        for c in num:
            v = np.divide(num[c], den[..., None], out=np.zeros_like(num[c]), where=(den > 1e-9)[..., None])
            v[bg_ok] = film0[c][bg_ok]
            values[c] = v
        out["rules"][name] = scored(values, accepted)
        # Error the rejected pixels would have had under the instance-only rule (ghost risk).
        if name == "instance":
            inst_values, inst_accepted = values, accepted
        print("   transfer", name, "accepted all-covered", round(out["rules"][name]["L0/all-covered"]["accepted"], 4), flush=True)
    # Ghost check: what rejected-by-rule pixels would have shown with instance-only history.
    ghost = {}
    for name, rule in rules.items():
        if name in ("none", "instance"):
            continue
        den = np.zeros((H, W))
        for t in tapinfo:
            den += np.where(rule(t) & cover1, t["w"], 0)
        rej = cover1 & (den <= 1e-9) & inst_accepted
        e2 = np.mean((codes(inst_values["L"], 0) - film1_codes[("L", 0)]) ** 2, -1)
        ghost[name] = dict(rejected=int(rej.sum()), instance_history_rms=float(np.sqrt(e2[rej].mean())) if rej.any() else None,
                           exact_rms=float(np.sqrt(e_exact["L0"][rej].mean())) if rej.any() else None)
    out["ghost"] = ghost
    # Nearest tap (no resampling blur, up to 0.5 px misregistration).
    nx = np.clip(np.rint(qx).astype(np.int64), 0, W - 1); ny = np.clip(np.rint(qy).astype(np.int64), 0, H - 1)
    rel = np.abs(d0[ny, nx] - dX) / np.maximum(dX, 1e-6)
    ok = cover1 & (pl0[ny, nx] == pl1) & (rel <= .02)
    values = {c: np.where(ok[..., None], film0[c][ny, nx], 0) for c in ("L", "D")}
    bg_ok = bgcentre & (pl0 == 0)
    for c in values:
        values[c][bg_ok] = film0[c][bg_ok]
    out["rules"]["nearest+instance+depth2%"] = scored(values, ok | bg_ok)
    # Catmull-Rom on pixels whose 16 taps all pass instance+depth2%; bilinear on the same subset.
    def cr(t):
        return [((-t + 2 * t ** 2 - t ** 3) / 2), ((2 - 5 * t ** 2 + 3 * t ** 3) / 2), ((t + 4 * t ** 2 - 3 * t ** 3) / 2), ((-t ** 2 + t ** 3) / 2)]
    wx = cr(fx); wy = cr(fy)
    num = {c: np.zeros((H, W, 3)) for c in ("L", "D")}; allok = cover1.copy()
    for j in range(4):
        for i in range(4):
            tx = x0 - 1 + i; ty = y0 - 1 + j; inside = (tx >= 0) & (tx < W) & (ty >= 0) & (ty < H)
            tx = np.clip(tx, 0, W - 1); ty = np.clip(ty, 0, H - 1)
            allok &= inside & (pl0[ty, tx] == pl1) & (np.abs(d0[ty, tx] - dX) <= .02 * np.maximum(dX, 1e-6))
            w = wx[i] * wy[j]
            for c in num:
                num[c] += w[..., None] * film0[c][ty, tx]
    for c in num:
        num[c] = np.maximum(num[c], 0)
    out["catmull_rom_subset"] = dict(pixels=int(allok.sum()), covered=int(cover1.sum()))
    out["rules"]["catmull-rom(subset)"] = scored(num, allok)
    bil = {c: np.zeros((H, W, 3)) for c in ("L", "D")}
    for t in tapinfo:
        for c in bil:
            bil[c] += t["w"][..., None] * film0[c][t["ty"], t["tx"]]
    out["rules"]["bilinear(subset)"] = scored(bil, allok)
    return out


# ---------------------------------------------------------------- motion model (I)

def motion_model(data, sigma, exact_rms):
    """Steady-state error of h_t = (1-a) bilinear_f(h_{t-1}) + a sample_t under a pan of f px/frame.

    Bias: the Fourier transfer a / (1 - (1-a) K(w) e^{-iwf}) applied to the film along x.
    Noise: iid per-sample error variance (no stratification) times the mean noise gain.
    """
    film = data["comp"]["L"]; filmD = data["comp"]["D"]; pops = data["pops"]
    n = film.shape[1]
    w_r = 2 * np.pi * np.fft.rfftfreq(n); w_f = 2 * np.pi * np.fft.fftfreq(n)
    FL = np.fft.rfft(film, axis=1); FD = np.fft.rfft(filmD, axis=1)
    slope = {(c, s): code_slope(data["comp"][c], s) for c in ("L", "D") for s in (0, 4)}
    report = ("partial", "lit", "contact", "multi-surface", "background-centre", "all-covered")
    counts = {k: int(v.sum()) for k, v in data["pops_full"].items()}
    out = {}
    for f in (0.0, 0.15, 0.25, 0.5):
        for a in (1.0, 1 / 2, 1 / 3, 1 / 4, 1 / 6, 1 / 8, 1 / 12, 1 / 16, 1 / 24, 1 / 32):
            K_r = (1 - f) + f * np.exp(1j * w_r); K_f = (1 - f) + f * np.exp(1j * w_f)
            h = a / (1 - (1 - a) * K_r * np.exp(-1j * w_r * f))
            gain = float(np.mean(a ** 2 / (1 - (1 - a) ** 2 * np.abs(K_f) ** 2))) if a < 1 else 1.0
            row = dict(noise_gain=gain, equivalent_iid_samples=1 / gain)
            for c, F, base, var in (("L", FL, film, sigma["8-rays"]), ("D", FD, filmD, sigma["8-rays-D"])):
                bias = np.fft.irfft((h - 1)[None, :, None] * F, n=n, axis=1)
                for s in (0, 4):
                    b2 = np.mean((codes(base + bias, s) - codes(base, s)) ** 2, -1)
                    n2 = np.mean(slope[(c, s)] ** 2 * var * gain, -1)
                    for pop in report:
                        m = pops[pop]
                        b, nn = b2[m].sum() / counts[pop], n2[m].sum() / counts[pop]
                        row[f"{c}{s}/{pop}"] = dict(bias=float(np.sqrt(b)), noise=float(np.sqrt(nn)), total=float(np.sqrt(b + nn)), exact=exact_rms[f"{c}{s}"][pop])
            out[f"f={f}/a={a:.4f}"] = row
    # Single-step transfer of a converged film (alpha -> 0 limit is not needed; one step of K e^{-iwf}).
    single = {}
    for f in (0.15, 0.25, 0.5):
        K_r = ((1 - f) + f * np.exp(1j * w_r)) * np.exp(-1j * w_r * f)
        e = np.fft.irfft((K_r - 1)[None, :, None] * FL, n=n, axis=1)
        e2 = np.mean((codes(film + e, 0) - codes(film, 0)) ** 2, -1)
        single[f] = {pop: float(np.sqrt(e2[pops[pop]].sum() / counts[pop])) for pop in report}
    return out, single


def cr_weights(t):
    return [(-t + 2 * t ** 2 - t ** 3) / 2, (2 - 5 * t ** 2 + 3 * t ** 3) / 2, (t + 4 * t ** 2 - 3 * t ** 3) / 2, (-t ** 2 + t ** 3) / 2]


def kernel(w, f, kind):
    """Shift-removed transfer of one history fetch at fractional offset f along one axis."""
    if kind == "bilinear":
        k = (1 - f) + f * np.exp(1j * w)
    else:
        k = sum(c * np.exp(1j * w * j) for c, j in zip(cr_weights(f), (-1, 0, 1, 2)))
    return k * np.exp(-1j * w * f)


def motion_model_2d(data, sigma, bars):
    """2D steady state of h_t = (1-a) fetch(h_{t-1}) + a sample_t under motion (fx, fy) px/frame."""
    pops = data["pops"]; hgt, wid = data["cover"].shape
    wy = 2 * np.pi * np.fft.fftfreq(hgt)[:, None]; wx = 2 * np.pi * np.fft.fftfreq(wid)[None, :]
    F = {c: np.fft.fft2(data["comp"][c], axes=(0, 1)) for c in ("L", "D")}
    var = {"L": sigma["8-rays"], "D": sigma["8-rays-D"]}
    slope = {(c, s): code_slope(data["comp"][c], s) for c in ("L", "D") for s in (0, 4)}
    report = ("partial", "lit", "contact", "multi-surface", "background-centre", "all-covered", "image")
    counts = {k: int(v.sum()) for k, v in data["pops_full"].items()}
    out = {}

    def evaluate(T, gain):
        row = dict(noise_gain=gain, equivalent_iid_samples=1 / gain)
        for c in ("L", "D"):
            bias = np.real(np.fft.ifft2((T - 1)[..., None] * F[c], axes=(0, 1)))
            for s in ((0, 4) if c == "L" else (4,)):
                b2 = np.mean((codes(data["comp"][c] + bias, s) - codes(data["comp"][c], s)) ** 2, -1)
                n2 = np.mean(slope[(c, s)] ** 2 * var[c] * gain, -1)
                for pop in report:
                    m = pops[pop]; b, nn = b2[m].sum() / counts[pop], n2[m].sum() / counts[pop]
                    row[f"{c}{s}/{pop}"] = dict(bias=float(np.sqrt(b)), noise=float(np.sqrt(nn)), total=float(np.sqrt(b + nn)), bar=bars[f"{c}{s}"][pop])
        return row

    for kind in ("bilinear", "catmull-rom"):
        for fx, fy in ((0.25, 0.32), (0.15, 0.0), (0.25, 0.25), (0.5, 0.5)):
            K = kernel(wx, fx, kind) * kernel(wy, fy, kind)
            out[f"{kind}/single/{fx},{fy}"] = evaluate(K, 0.0 + 1e-30)
            for a in (1 / 2, 1 / 3, 1 / 4, 1 / 6, 1 / 8, 1 / 16):
                h = a / (1 - (1 - a) * K)
                gain = float(np.mean(a ** 2 / (1 - (1 - a) ** 2 * np.abs(K) ** 2)))
                out[f"{kind}/{fx},{fy}/a={a:.4f}"] = evaluate(h, gain)
            print("   motion2d", kind, fx, fy, flush=True)
    return out


# ---------------------------------------------------------------- inspection

def tone_im(a, stops=0):
    return Image.fromarray(np.rint(np.clip(codes(a, stops), 0, 255)).astype("u1"))


def err_im(a, b, stops=0, scale=8):
    d = np.mean(codes(a, stops) - codes(b, stops), -1) * scale / 255
    rgb = np.stack([np.clip(d, 0, 1), np.zeros_like(d), np.clip(-d, 0, 1)], -1)
    return Image.fromarray(np.rint(rgb * 255).astype("u1"))


def inspection(data, rng, out, crop):
    s = data["samples"]; shape = data["cover"].shape
    order = orderings(shape, rng)["progressive-shifted"]
    acc = np.zeros(shape + (3,)); accD = np.zeros(shape + (3,)); frames = {}
    for k, idx in enumerate(order):
        acc += take(s["L8"], idx); accD += take(s["D8"], idx)
        if k + 1 in (1, 4, 8, 16):
            frames[k + 1] = (acc / (k + 1), accD / (k + 1))
    film, filmD = data["comp"]["L"], data["comp"]["D"]
    panels = [("film R_L (240)", tone_im(film)), ("Exact 1x", tone_im(data["exact"]["L"]))]
    for n, (l, _) in frames.items():
        panels.append((f"T1 N={n} (8 rays)", tone_im(l)))
    panels += [("Exact error x8", err_im(data["exact"]["L"], film))] + [(f"N={n} error x8", err_im(l, film)) for n, (l, _) in frames.items()]
    f0.sheet(panels, columns=3, width=1047).save(out / "pose-0-accumulation-sheet.png")
    panels = [("film R_D +4", tone_im(filmD, 4)), ("Exact R_D +4", tone_im(data["exact"]["D"], 4))]
    for n, (_, d) in frames.items():
        panels.append((f"T1 R_D N={n} +4", tone_im(d, 4)))
    panels += [("Exact R_D error x8 (+4)", err_im(data["exact"]["D"], filmD, 4))] + [(f"R_D N={n} error x8 (+4)", err_im(d, filmD, 4)) for n, (_, d) in frames.items()]
    f0.sheet(panels, columns=3, width=1047).save(out / "pose-0-sun-accumulation-sheet.png")
    # 3x crops at the frozen contact probes plus feature regions (crop-relative coordinates).
    oy, ox = crop[0].start, crop[1].start
    coords = [(x - ox, y - oy) for x, y in f0.COORDS + [(720, 600), (1400, 600), (1210, 330), (1040, 650), (1575, 500)] if 0 <= x - ox < shape[1] and 0 <= y - oy < shape[0]]
    rows = [("film", tone_im(film, 4)), ("Exact", tone_im(data["exact"]["L"], 4))] + [(f"N={n}", tone_im(l, 4)) for n, (l, _) in frames.items()]
    size = 33; cell = 99
    sheet = Image.new("RGB", (len(coords) * (cell + 6), len(rows) * (cell + 16)), (20, 27, 35)); draw = ImageDraw.Draw(sheet)
    for r, (name, im) in enumerate(rows):
        for c, (x, y) in enumerate(coords):
            draw.text((c * (cell + 6), r * (cell + 16)), name, fill="white")
            sheet.paste(im.crop((x - 16, y - 16, x + 17, y + 17)).resize((cell, cell), Image.Resampling.NEAREST), (c * (cell + 6), r * (cell + 16) + 14))
    sheet.save(out / "pose-0-accumulation-crops-plus4.png")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, default=Path("D:/Coding/renderlab-work/m6-exact-quality/t1"))
    a = p.parse_args(); start = time.perf_counter(); a.out.mkdir(parents=True, exist_ok=True)
    cover = np.zeros((H, W), bool)
    for pose in (0, 1):
        cover |= np.fromfile(R1 / f"quarry-viewed-p{pose}/pose-{pose}.hit.u32", dtype="<u4").reshape(H, W, 4)[..., 0] != 0
    ys, xs = np.nonzero(cover)
    crop = (slice(max(0, ys.min() - 8), min(H, ys.max() + 9)), slice(max(0, xs.min() - 8), min(W, xs.max() + 9)))
    pools, noise_summary = noise_pools()
    print("noise model", {k: noise_summary[k] for k in ("partial_rms", "partial_n", "lit_nonzero", "umbra_nonzero")}, flush=True)
    result = dict(crop=[int(crop[0].start), int(crop[0].stop), int(crop[1].start), int(crop[1].stop)], noise_model=noise_summary, poses={})
    datas = {}; sigmas = {}
    for pose in (0, 1):
        rng = np.random.default_rng(1000 + pose)
        print("pose", pose, "loading", flush=True)
        data = pose_data(pose, crop, pools, rng)
        print("pose", pose, "replaying", data["checks"], flush=True)
        r, sigma = replay(data, np.random.default_rng(2000 + pose))
        r["summary"] = summarise(r); r["checks"] = data["checks"]; r["r1_reconciliation"] = reconcile_r1(data, crop)
        if pose == 0:
            bars = {ch: {pop: min(r["baselines"]["exact"][ch][pop], r["single"][8][ch][pop]) for pop in r["baselines"]["exact"][ch]} for ch in ("L0", "L4", "D0", "D4")}
            r["motion"], r["motion_single_step"] = motion_model(data, sigma, bars)
            r["motion_2d"] = motion_model_2d(data, sigma, bars)
            inspection(data, np.random.default_rng(3000), a.out, crop)
        result["poses"][str(pose)] = r
        # Keep what the transfer test needs.
        datas[pose] = dict(ref=data["ref"], pops_full=data["pops_full"], direct_full=data["direct_full"], exact=data["exact"],
                           centre=dict(L=data["samples"]["L"][8], D=data["samples"]["D"][8]))
        print("pose", pose, "done at", round(time.perf_counter() - start, 1), "s", flush=True)
        sigmas[pose] = sigma
        del data
    result["transfer_1cm"] = transfer(datas[0], datas[1], sigmas[0], sigmas[1], crop)
    result["seconds"] = time.perf_counter() - start
    (a.out / "temporal-replay.json").write_bytes(json.dumps(result, indent=1, default=lambda o: o.item() if hasattr(o, "item") else str(o)).encode())
    print("done in", round(result["seconds"], 1), "s", flush=True)


if __name__ == "__main__":
    main()
