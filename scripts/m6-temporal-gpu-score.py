"""T1 GPU scoring (docs/m6-temporal-primaries.md): Level F references, bars and T1a gates.

Reads the guarded runs of src/film_accumulation_tests.rs and writes only under --out:
  1. Decodes the 1,024 x 32 references (R_L, R_D, R_F with split-half noise) and checks them
     against R1's 256 x 256 viewed references at poses 0 and 1 (agreement within combined noise).
  2. Scores production Exact (seeds 0-7) and Interactive (seeds 0-3) per population and
     channel: the re-baselined bars E (1 + s), s = Exact's 2-s.d. relative seed spread.
  3. Scores T1 static checkpoints (--t1) and evaluates T-0, the static diagnostic against the
     CPU replay (1.25x), the frozen F1-AA checks, F1, F2, F3, F4, F6 and F8.
  4. Writes inspection sheets and crops at registered and +4 stops.

py -3 -B scripts/m6-temporal-gpu-score.py --ref E:/renderlab-work/t1-gpu/ref [--t1 E:/renderlab-work/t1-gpu/t1] --out E:/renderlab-work/t1-gpu/scores
Math-library threads should be set to two.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time

import numpy as np
from PIL import Image

spec = importlib.util.spec_from_file_location("film_score", Path(__file__).with_name("m6-film-score.py"))
f0 = importlib.util.module_from_spec(spec); spec.loader.exec_module(f0)

R1 = Path("E:/renderlab-work/m6-exact-quality/r1")
REPLAY = Path("D:/Coding/renderlab-work/m6-exact-quality/t1/temporal-replay.json")
H, W = 1080, 1920
DTYPE = np.dtype([("counts", "<u4", (4,)), ("moments", "<f4", (4,)), ("r0", "<f4", (4,)), ("r1", "<f4", (4,))])
POPS = ("partial", "faint", "narrow", "contact", "isolated", "lit", "occluded", "backlit",
        "multi-surface", "background-centre", "all-covered", "image")
CHANNELS = ("L0", "L4", "D0", "D4", "F")
EXACT_SEEDS, INTERACTIVE_SEEDS = 8, 4


def codes(hdr, stops):
    x = np.maximum(0, hdr * (2 ** stops)); x = x / (1 + x)
    return 255 * np.where(x <= .0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - .055)


def code_slope(hdr, stops):
    s = 2 ** stops; x = np.maximum(0, hdr * s); y = x / (1 + x); dy = s / (1 + x) ** 2
    return 255 * np.where(y <= .0031308, 12.92, 1.055 / 2.4 * np.power(np.maximum(y, 1e-12), 1 / 2.4 - 1)) * dy


def hdr(path):
    return np.fromfile(path, dtype="<f2").reshape(H, W, 4)[..., :3].astype(np.float32)


def decode(directory, pose):
    """The T1 reference: halves, means and per-pixel noise of R_L, R_D and R_F."""
    p = directory / f"pose-{pose}"
    info = json.loads(p.with_suffix(".json").read_text())
    half = info["samples"] // 2
    film = np.fromfile(p.with_suffix(".film.bin"), dtype=DTYPE).reshape(H, W)
    direct = np.fromfile(p.with_suffix(".direct.bin"), dtype=DTYPE).reshape(H, W)
    c = film["counts"].astype(np.float64)
    n = c[..., 2] + c[..., 3]
    ref = np.divide(c[..., 0] + c[..., 1], 256 * n, out=np.ones(n.shape), where=n > 0)
    halves = [np.divide(c[..., i], 256 * c[..., i + 2], out=np.ones(n.shape), where=c[..., i + 2] > 0) for i in range(2)]
    sigma = np.abs(halves[0] - halves[1]) * np.sqrt(c[..., 2] * c[..., 3]) / np.maximum(n, 1)
    moments = film["moments"][..., 0].astype(np.float64) + film["moments"][..., 1]
    variance = np.maximum(0, np.divide(moments, n, out=np.zeros(n.shape), where=n > 0) - ref ** 2)
    world = np.fromfile(p.with_suffix(".world.f32"), dtype="<f4").reshape(H, W, 4)
    normal = np.fromfile(p.with_suffix(".normal.f32"), dtype="<f4").reshape(H, W, 4)
    hit = np.fromfile(p.with_suffix(".hit.u32"), dtype="<u4").reshape(H, W, 4)
    cover = hit[..., 0] != 0
    eye = np.asarray(info["eye"][:3]); forward = np.asarray(info["forward"][:3])
    facing = normal[..., :3].astype(float)
    facing *= np.where(np.einsum("ijk,ijk->ij", facing, eye - world[..., :3]) < 0, -1, 1)[..., None]
    front = cover & (facing @ f0.lb.TO_LIGHT > 0)
    radiance = {}
    for key, a in (("L", film), ("D", direct)):
        h0 = a["r0"][..., :3].astype(np.float64) / half; h1 = a["r1"][..., :3].astype(np.float64) / half
        radiance[key] = (h0 + h1) / 2
        radiance[key + "_halves"] = (h0, h1)
    return dict(ref=ref, sigma=sigma, depth=world[..., 3], cover=cover, front=front, n=n, variance=variance,
                hit=hit, info=info, L=radiance["L"], D=radiance["D"], L_halves=radiance["L_halves"],
                D_halves=radiance["D_halves"], full_n=info["samples"])


def populations(d):
    p = f0.populations(d, np.zeros((H, W)))
    p["backlit"] = d["cover"] & ~d["front"]
    p["all-covered"] = d["cover"]
    p["image"] = np.ones_like(d["cover"])
    p["multi-surface"] = d["cover"] & (d["n"] < d["full_n"])
    p["background-centre"] = ~d["cover"]
    return {k: p[k] for k in POPS} | {"solid-lit": p["lit"], "solid-occluded": p["occluded"]}


class Scorer:
    """Per-population RMS of one estimate against the reference, with reference-noise
    subtraction from its two independent halves."""

    def __init__(self, d, pops):
        self.d, self.pops = d, pops
        self.ref = {(c, s): codes(d[c], s) for c in ("L", "D") for s in (0, 4)}
        # Var(mean of halves) = E[(h0 - h1)^2] / 4, propagated through the display slope.
        self.noise = {}
        for c in ("L", "D"):
            h0, h1 = d[c + "_halves"]
            for s in (0, 4):
                self.noise[(c, s)] = np.mean(((h0 - h1) / 2 * code_slope(d[c], s)) ** 2, -1)
        self.valid_f = d["cover"] & (d["n"] > 0)

    def score(self, L, D, V, maxima=False):
        out = {}
        for c, est in (("L", L), ("D", D)):
            for s in (0, 4):
                e = codes(est, s) - self.ref[(c, s)]
                e2 = np.mean(e ** 2, -1); signed = np.mean(e, -1)
                row = {}
                for name, m in self.pops.items():
                    if not m.any():
                        continue
                    k = int(m.sum())
                    r = dict(n=k, rms=float(np.sqrt(e2[m].mean())),
                             rms_noise_subtracted=float(np.sqrt(max(0, e2[m].mean() - self.noise[(c, s)][m].mean()))),
                             mean=float(signed[m].mean()), se=float(signed[m].std(ddof=1) / math.sqrt(k)) if k > 1 else 0.)
                    if maxima:
                        r["max"] = float(np.sqrt(e2[m]).max())
                    row[name] = r
                row["_over16"] = int((np.sqrt(e2) > 16).sum()) if s == 4 and c == "D" else None
                out[f"{c}{s}"] = row
        valid = self.valid_f & np.isfinite(V)
        e = np.where(valid, np.nan_to_num(V) - self.d["ref"], 0)
        row = {}
        for name, m in self.pops.items():
            if name in ("background-centre", "image"):
                continue
            mm = m & valid
            if not mm.any():
                continue
            k = int(mm.sum())
            row[name] = dict(n=k, rms=float(np.sqrt(np.mean(e[mm] ** 2))),
                             rms_noise_subtracted=float(np.sqrt(max(0, np.mean(e[mm] ** 2) - np.mean(self.d["sigma"][mm] ** 2)))),
                             mean=float(e[mm].mean()), se=float(e[mm].std(ddof=1) / math.sqrt(k)) if k > 1 else 0.,
                             max=float(np.abs(e[mm]).max()))
        row["_missed"] = int((valid & (self.d["ref"] <= .9) & (V >= .99)).sum())
        row["_false"] = int((valid & (self.d["ref"] >= .99) & (V <= .9)).sum())
        out["F"] = row
        return out


def r1_agreement(d, pose):
    """New reference minus R1's 256 x 256 viewed reference, against their combined noise."""
    r1dir = R1 / f"quarry-viewed-p{pose}"
    r1 = f0.decode(r1dir, pose)
    direct = np.fromfile(r1dir / f"pose-{pose}.direct.bin", dtype=f0.DTYPE).reshape(H, W)["radiance"][..., :3].astype(np.float64) / 256
    r1L = r1["hdr"][..., :3]
    # R1's per-sample variance from its 16 saved samples, over 256 samples: conservative.
    sL = np.zeros((H, W, 3)); qL = np.zeros((H, W, 3)); sD = np.zeros((H, W, 3)); qD = np.zeros((H, W, 3))
    for i in range(16):
        a = hdr(r1dir / f"samples/{i:02}.rgba16f").astype(np.float64); b = hdr(r1dir / f"samples/{i:02}.sun.rgba16f").astype(np.float64)
        sL += a; qL += a * a; sD += b; qD += b * b
    varL = np.maximum(0, qL / 16 - (sL / 16) ** 2) * 16 / 15 / 256
    varD = np.maximum(0, qD / 16 - (sD / 16) ** 2) * 16 / 15 / 256
    pops = populations(d)
    out = {}
    for c, new, old, var in (("L", d["L"], r1L, varL), ("D", d["D"], direct, varD)):
        h0, h1 = d[c + "_halves"]
        for s in (0, 4):
            slope_new = code_slope(new, s)
            noise = np.mean(((h0 - h1) / 2 * slope_new) ** 2 + var * code_slope(old, s) ** 2, -1)
            e = codes(new, s) - codes(old, s); e2 = np.mean(e ** 2, -1); signed = np.mean(e, -1)
            row = {}
            for name in ("partial", "lit", "occluded", "multi-surface", "background-centre", "all-covered", "image"):
                m = pops[name]
                if not m.any():
                    continue
                k = int(m.sum())
                row[name] = dict(n=k, rms=float(np.sqrt(e2[m].mean())), combined_noise_rms=float(np.sqrt(noise[m].mean())),
                                 ratio=float(np.sqrt(e2[m].mean() / max(noise[m].mean(), 1e-30))),
                                 mean=float(signed[m].mean()), se=float(signed[m].std(ddof=1) / math.sqrt(k)))
            out[f"{c}{s}"] = row
    both = d["cover"] & r1["cover"] & (d["n"] > 0) & (r1["n"] > 0)
    e = d["ref"] - r1["ref"]; noise = d["sigma"] ** 2 + r1["sigma"] ** 2
    row = {}
    for name in ("partial", "faint", "contact", "lit", "occluded", "all-covered"):
        m = pops[name] & both
        if m.any():
            row[name] = dict(n=int(m.sum()), rms=float(np.sqrt(np.mean(e[m] ** 2))), combined_noise_rms=float(np.sqrt(noise[m].mean())),
                             ratio=float(np.sqrt(np.mean(e[m] ** 2) / max(noise[m].mean(), 1e-30))), mean=float(e[m].mean()))
    out["F"] = row
    out["solid_visibility_flips"] = int((both & (((d["ref"] == 1) & (r1["ref"] <= .9)) | ((d["ref"] == 0) & (r1["ref"] >= .1)))).sum())
    return out


def centre(directory, mode, seed):
    p = directory / f"{mode}-seed-{seed}"
    v = np.fromfile(p.with_suffix(".u32"), dtype="<u4").reshape(H, W) / 256
    return hdr(p.with_suffix(".rgba16f")), hdr(p.with_suffix(".sun.rgba16f")), v


def rms_table(score):
    return {ch: {p: v["rms"] for p, v in rows.items() if isinstance(v, dict)} for ch, rows in score.items()}


def tone(a, stops=0):
    v = np.clip(codes(a, stops), 0, 255)
    return Image.fromarray(np.rint(v).astype("u1"))


def error_image(a, b, stops=0, scale=8):
    e = np.mean(codes(a, stops) - codes(b, stops), -1) * scale / 255
    v = np.clip(e, -1, 1)
    rgb = np.stack([np.maximum(v, 0), np.zeros_like(v), np.maximum(-v, 0)], -1)
    return Image.fromarray(np.rint(rgb * 255).astype("u1"))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", type=Path, required=True)
    ap.add_argument("--t1", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--poses", default="0,1,2")
    ap.add_argument("--verify", type=Path, help="16-replica visibility runs: verifies reference solids")
    a = ap.parse_args(); start = time.perf_counter()
    a.out.mkdir(parents=True, exist_ok=True); images = a.out / "images"; images.mkdir(exist_ok=True)
    replay = json.loads(REPLAY.read_text())["poses"]
    temporal = json.loads((a.t1 / "temporal.json").read_text()) if a.t1 else None
    result = dict(poses={}, gates={})
    for pose in [int(p) for p in a.poses.split(",")]:
        directory = a.ref / f"ref-p{pose}"
        d = decode(directory, pose); pops = populations(d); scorer = Scorer(d, pops)
        row = dict(reference=dict(info=d["info"], covered=int(d["cover"].sum()),
                   no_samples=int((d["cover"] & (d["n"] == 0)).sum()),
                   partial_sigma_rms=float(np.sqrt(np.mean(d["sigma"][pops["partial"]] ** 2))),
                   populations={k: int(v.sum()) for k, v in pops.items()}))
        if pose in (0, 1):
            row["r1_agreement"] = r1_agreement(d, pose)
        exact = [scorer.score(*centre(directory, "exact", s), maxima=True) for s in range(EXACT_SEEDS)]
        inter = [scorer.score(*centre(directory, "interactive", s)) for s in range(INTERACTIVE_SEEDS)]
        bars = {}
        for ch in CHANNELS:
            bars[ch] = {}
            for p in exact[0][ch]:
                if p.startswith("_"):
                    continue
                values = np.array([e[ch][p]["rms"] for e in exact])
                spread = float(2 * values.std(ddof=1) / values.mean()) if values.mean() > 0 else 0.
                bars[ch][p] = dict(exact=exact[0][ch][p]["rms"], spread_2sd=spread, bar=exact[0][ch][p]["rms"] * (1 + spread),
                                   interactive=inter[0][ch][p]["rms"] if p in inter[0][ch] else None,
                                   interactive_seed_mean=float(np.mean([i[ch][p]["rms"] for i in inter if p in i[ch]])),
                                   exact_mean=exact[0][ch][p]["mean"], exact_se=exact[0][ch][p]["se"],
                                   exact_max=exact[0][ch][p].get("max"))
        row["exact_seed0"] = exact[0]; row["exact_seeds_rms"] = [rms_table(e) for e in exact]
        row["interactive_rms"] = [rms_table(i) for i in inter]; row["bars"] = bars
        row["defects"] = dict(exact_missed=exact[0]["F"]["_missed"], exact_false=exact[0]["F"]["_false"],
                              exact_D4_over16=exact[0]["D4"]["_over16"],
                              interactive_missed=[i["F"]["_missed"] for i in inter], interactive_false=[i["F"]["_false"] for i in inter])
        # Inspection: film, Exact, Interactive at both exposures; Exact x8 error maps.
        eL, eD, _ = centre(directory, "exact", 0); iL, iD, _ = centre(directory, "interactive", 0)
        for stops in (0, 4):
            tone(d["L"], stops).save(images / f"p{pose}-film-L{stops}.png")
            tone(d["D"], stops).save(images / f"p{pose}-film-D{stops}.png")
            error_image(eL, d["L"], stops).save(images / f"p{pose}-exact-L{stops}-err-x8.png")
            error_image(iL, d["L"], stops).save(images / f"p{pose}-interactive-L{stops}-err-x8.png")
            error_image(eD, d["D"], stops).save(images / f"p{pose}-exact-D{stops}-err-x8.png")
        if a.t1:
            t1dir = a.t1 / f"t1-p{pose}"
            ns = sorted(int(p.stem.split("-n")[1]) for p in t1dir.glob("full-n*.rgba16f"))
            curve = {}
            for n in ns:
                L = hdr(t1dir / f"full-n{n:03}.rgba16f"); D = hdr(t1dir / f"sun-n{n:03}.rgba16f")
                # R_F's estimand: T1's own samples on the pixel's centre surface.
                c = np.fromfile(t1dir / f"conditioned-n{n:03}.u32", dtype="<u4").reshape(H, W, 2).astype(np.float64)
                V = np.divide(c[..., 0], 256 * c[..., 1], out=np.full((H, W), np.nan), where=c[..., 1] > 0)
                curve[n] = scorer.score(L, D, V, maxima=True)
                if n in (1, 2, 4, 8, 32, 128):
                    for stops in (0, 4):
                        tone(L, stops).save(images / f"p{pose}-t1-n{n:03}-L{stops}.png")
                        error_image(L, d["L"], stops).save(images / f"p{pose}-t1-n{n:03}-L{stops}-err-x8.png")
                        error_image(D, d["D"], stops).save(images / f"p{pose}-t1-n{n:03}-D{stops}-err-x8.png")
            row["t1_curve"] = {n: rms_table(s) for n, s in curve.items()}
            row["t1_defects"] = {n: dict(missed=s["F"]["_missed"], false=s["F"]["_false"], D4_over16=s["D4"]["_over16"]) for n, s in curve.items()}
            row["gates"] = gates(curve, bars, exact[0], inter, replay.get(str(pose)), t1dir, d, pops, ns)
            if a.verify:
                row["gates"]["F1_solid_verified"] = verified_solids(a.verify / f"ref-p{pose}", pose, d, pops, t1dir, ns)
        result["poses"][str(pose)] = row
        print("pose", pose, "done", round(time.perf_counter() - start, 1), "s", flush=True)
    if temporal:
        result["gates"]["T-0"] = dict(passed=all(i["hdr_identical"] and i["hit_and_counts_identical"] for i in temporal["identity"]), cases=temporal["identity"])
        rep = temporal["repeats"]
        result["gates"]["F8_static_repeats"] = dict(passed=rep["identical_to_first"] == rep["runs"] - 1, **{k: rep[k] for k in ("pose", "runs", "identical_to_first")})
        cold = {(int(k.split("/")[0][1:]), e["n"]): e["sha256"] for k, v in temporal["digests"].items() if k.endswith("/full") for e in v}
        cuts = [dict(c, cold_start_identical=cold.get((c["pose"], c["n"])) == c["sha256"]) for c in temporal["cuts"] if c["segment"] != "converged"]
        result["gates"]["Q-cut_cold_identity"] = dict(passed=all(c["cold_start_identical"] for c in cuts if (c["pose"], c["n"]) in cold), cases=cuts)
    result["seconds"] = time.perf_counter() - start
    (a.out / "score.json").write_text(json.dumps(result, indent=1, allow_nan=False))
    print(json.dumps(result["gates"], indent=1)[:4000])


def verified_solids(directory, pose, d, pops, t1dir, ns):
    """F1's solid bound on reference solids that independent replicas confirm. Selection uses
    only the references, never T1's error (as docs/m6-receiver-followup.md verified G2)."""
    film = np.fromfile(directory / f"pose-{pose}.film.bin", dtype=DTYPE).reshape(H, W)
    c = film["counts"].astype(np.float64); n = c[..., 2] + c[..., 3]
    v = np.divide(c[..., 0] + c[..., 1], 256 * n, out=np.full((H, W), np.nan), where=n > 0)
    out = dict(replica_samples=json.loads((directory / f"pose-{pose}.json").read_text()).get("replicas", 1) * 1024)
    for name in ("solid-lit", "solid-occluded"):
        m = pops[name] & (d["n"] > 0)
        confirmed = m & (v == d["ref"])
        out[name] = dict(reference=int(m.sum()), confirmed=int(confirmed.sum()), reclassified_partial=int((m & np.isfinite(v) & (v != d["ref"])).sum()))
        for n_ in [n_ for n_ in ns if n_ > 32]:
            cc = np.fromfile(t1dir / f"conditioned-n{n_:03}.u32", dtype="<u4").reshape(H, W, 2).astype(np.float64)
            t = np.divide(cc[..., 0], 256 * cc[..., 1], out=np.full((H, W), np.nan), where=cc[..., 1] > 0)
            e = np.abs(t - d["ref"]); k = confirmed & np.isfinite(t)
            out[name][f"max_n{n_}"] = float(e[k].max()) if k.any() else 0.
            out[name][f"over_0.002_n{n_}"] = int((k & (e > .002)).sum())
            # Pixels the replicas reclassified: T1's error against the replica estimate.
            r = m & np.isfinite(v) & (v != d["ref"]) & np.isfinite(t)
            out[name][f"reclassified_rms_vs_replicas_n{n_}"] = float(np.sqrt(np.mean((t[r] - v[r]) ** 2))) if r.any() else 0.
    out["passed"] = all(out[k][f"max_n{n_}"] <= .002 for k in ("solid-lit", "solid-occluded") for n_ in ns if n_ > 32)
    return out


def gates(curve, bars, exact, inter, replay, t1dir, d, pops, ns):
    """T1a gate checks at one pose (docs/m6-temporal-primaries.md sections 7 and 9)."""
    g = {}
    # Static diagnostic: within 1.25x of the CPU replay at N = 1, 2, 4, 8, 16.
    if replay:
        rc = replay["curves"]["centre-progressive-scrambled/8-rays"]
        worst = []
        for n in (1, 2, 4, 8, 16):
            if n not in curve:
                continue
            for ch in CHANNELS:
                for p, v in rc[n - 1][ch].items():
                    model = v if not isinstance(v, list) else v[0]
                    got = curve[n][ch].get(p, {}).get("rms")
                    if model and got is not None:
                        worst.append(dict(n=n, channel=ch, population=p, measured=got, replay=model, ratio=got / model))
        worst.sort(key=lambda r: -r["ratio"])
        g["static_vs_replay"] = dict(passed=all(w["ratio"] <= 1.25 for w in worst), worst=worst[:12])
    # Frozen F1-AA: visibility partial <= 0.12 at frame 8 and <= 0.058 at 32; R_L <= Exact by 2.
    f1aa = dict(partial_vis_n8=curve[8]["F"]["partial"]["rms"] if 8 in curve else None,
                partial_vis_n32=curve[32]["F"]["partial"]["rms"] if 32 in curve else None,
                L0_image_n2=curve[2]["L0"]["image"]["rms"], exact_L0_image=bars["L0"]["image"]["exact"])
    f1aa["passed"] = (f1aa["partial_vis_n8"] is None or f1aa["partial_vis_n8"] <= .12) and (f1aa["partial_vis_n32"] is None or f1aa["partial_vis_n32"] <= .058) and f1aa["L0_image_n2"] <= f1aa["exact_L0_image"]
    g["F1-AA_frozen"] = f1aa
    # F1 converged: >= 32 frames past the event, per population <= bar on every channel.
    fails = []
    for n in [n for n in ns if n > 32]:
        for ch in CHANNELS:
            for p, v in curve[n][ch].items():
                if p.startswith("_") or p not in bars[ch]:
                    continue
                if v["rms"] > bars[ch][p]["bar"]:
                    fails.append(dict(n=n, channel=ch, population=p, rms=v["rms"], bar=bars[ch][p]["bar"]))
    solid = {n: max(curve[n]["F"].get("solid-lit", {}).get("max", 0), curve[n]["F"].get("solid-occluded", {}).get("max", 0)) for n in ns if n > 32}
    g["F1_converged"] = dict(passed=not fails and all(v <= .002 for v in solid.values()), failures=fails[:20], solid_visibility_max=solid)
    # F2 transient: <= Interactive at every age, <= bar at age 7 (N = 8), R_L and R_D.
    f2 = []
    for n in [n for n in ns if n <= 8]:
        for ch in ("L0", "L4", "D0", "D4"):
            for p, v in curve[n][ch].items():
                if p.startswith("_") or p not in bars[ch]:
                    continue
                if v["rms"] > bars[ch][p]["interactive"] + 1e-9:
                    f2.append(dict(n=n, channel=ch, population=p, rms=v["rms"], interactive=bars[ch][p]["interactive"], kind="above Interactive"))
                if n == 8 and v["rms"] > bars[ch][p]["bar"]:
                    f2.append(dict(n=n, channel=ch, population=p, rms=v["rms"], bar=bars[ch][p]["bar"], kind="above bar at age 7"))
    g["F2_transient"] = dict(passed=not f2, failures=f2[:30])
    # F3 bias: |mean| <= |E mean| + 4 SE (converged checkpoints).
    f3 = []
    for n in [n for n in ns if n > 32]:
        for ch in CHANNELS:
            for p, v in curve[n][ch].items():
                if p.startswith("_") or p not in bars[ch]:
                    continue
                if abs(v["mean"]) > abs(bars[ch][p]["exact_mean"]) + 4 * v["se"]:
                    f3.append(dict(n=n, channel=ch, population=p, mean=v["mean"], exact_mean=bars[ch][p]["exact_mean"], se=v["se"]))
    g["F3_bias"] = dict(passed=not f3, failures=f3[:20])
    # F4 (static, one frame per checkpoint; persistence is implied by a static view).
    f4 = [dict(n=n, missed=curve[n]["F"]["_missed"], false=curve[n]["F"]["_false"], D4_over16=curve[n]["D4"]["_over16"]) for n in ns]
    ex = dict(missed=exact["F"]["_missed"], false=exact["F"]["_false"], D4_over16=exact["D4"]["_over16"])
    g["F4_defects"] = dict(passed=all(r["missed"] + r["false"] <= ex["missed"] + ex["false"] and r["D4_over16"] <= ex["D4_over16"] for r in f4 if r["n"] >= 8),
                           exact=ex, t1=f4, note="checked from frame 8; earlier frames reported")
    # F6 flicker on static converged pixels: consecutive frames.
    pairs = [(n - 1, n) for n in ns if n - 1 in ns and n > 32]
    flick = []
    for p0, p1 in pairs:
        a0 = hdr(t1dir / f"full-n{p0:03}.rgba16f"); a1 = hdr(t1dir / f"full-n{p1:03}.rgba16f")
        for stops, limit in ((0, .5), (4, 1.)):
            e = np.mean((codes(a1, stops) - codes(a0, stops)) ** 2, -1)
            flick.append(dict(frames=[p0, p1], stops=stops, rms=float(np.sqrt(e.mean())), covered_rms=float(np.sqrt(e[d["cover"]].mean())), limit=limit))
    g["F6_flicker"] = dict(passed=bool(flick) and all(f["rms"] <= f["limit"] and f["covered_rms"] <= f["limit"] for f in flick), pairs=flick)
    return g


if __name__ == "__main__":
    main()
