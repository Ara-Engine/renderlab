"""T1b motion scoring (docs/m6-temporal-primaries.md sections 4 and 7) of a camera sequence.

Inputs: per-camera Level F references (gpu_film_t1_reference over the sequence's distinct
cameras, with one Exact and one Interactive centre frame each) and T1's run of the sequence
(gpu_film_t1_motion: film captures of the full, sun-only and visibility programs, per-pixel
ages, per-frame records). Frames with the same camera share a reference (holds, the return).

1. compact: decodes each completed reference once into what scoring needs (display codes,
   reference noise, visibility, populations as bit masks, centre placement and world point),
   evaluates the section 4 motion model where the manifest asks, and deletes the raw dumps.
2. score: per scored frame, T1 against the reference with bars from the same frame's Exact
   (times 1 + the static 2-s.d. seed spread), and the gates: F1 on pixels >= 32 frames past
   their last event, F2 from the start and from every disocclusion (pooled by age), F3, F4,
   F5 (changed pixels 8 frames after motion stops; ghost bias), F6 (motion residual and the
   static hold), F8 (from the run), the section 4 diagnostic and inspection images.

R_F is scored on single-surface pixels (every reference sample accepted the centre surface),
where T1's unconditioned visibility film has R_F's estimand.

py -3 -B scripts/m6-temporal-motion-score.py --ref <refs> --t1 <run> --out <scores> [--compact-only]
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

spec = importlib.util.spec_from_file_location("gpu_score", Path(__file__).with_name("m6-temporal-gpu-score.py"))
ts = importlib.util.module_from_spec(spec); spec.loader.exec_module(ts)
H, W = ts.H, ts.W
codes, code_slope, hdr = ts.codes, ts.code_slope, ts.hdr
POPS = ts.POPS + ("edge", "solid-lit", "solid-occluded")
CHANNELS = ("L0", "L4", "D0", "D4")
STATIC_SPREAD = Path("E:/renderlab-work/t1-gpu/scores-t1k/score.json")
RAW = (".film.bin", ".direct.bin", ".world.f32", ".normal.f32", ".hit.u32", ".exact.rgba16f", ".exact.u32")


def camera_key(frame):
    return json.dumps([frame[k] for k in ("eye", "target", "up", "fov_y_radians", "near", "far", "width", "height")])


def reference_of(sequence, poses):
    """Scored frame -> the reference pose with the same camera."""
    keys = {camera_key(sequence[p]): p for p in poses}
    return {i: keys.get(camera_key(f)) for i, f in enumerate(sequence)}


def project(matrix, X):
    """Continuous pixel positions of world points X (H, W, 3) under a column-major matrix."""
    m = np.asarray(matrix, dtype=np.float64).reshape(4, 4).T
    c = X @ m[:3, :3].T + m[:3, 3]
    wv = X @ m[3, :3] + m[3, 3]
    x = (c[..., 0] / wv * .5 + .5) * W; y = (-c[..., 1] / wv * .5 + .5) * H
    return x, y


def offsets(X, cover, previous, current):
    """Per-pixel motion of the centre surface point into the previous frame, its fractional
    fetch offset f, the blur s = sum f (1 - f) and window W = clamp(1 + 1/s, 1, 256)."""
    x0, y0 = project(previous, X); x1, y1 = project(current, X)
    dx, dy = x0 - x1, y0 - y1
    fx, fy = dx - np.floor(dx), dy - np.floor(dy)
    s = fx * (1 - fx) + fy * (1 - fy)
    window = np.where(s > 0, np.clip(1 + 1 / np.maximum(s, 1e-30), 1, 256), 256)
    q = lambda a: np.quantile(a[cover], [.01, .1, .5, .9, .99]).tolist()
    return dict(dx=q(dx), dy=q(dy), fx=q(fx), fy=q(fy), blur=q(s), window=q(window),
                fx_median=float(np.median(fx[cover])), fy_median=float(np.median(fy[cover])),
                window_median=float(np.median(window[cover])))


def cr_kernel(w, f):
    """Shift-removed Catmull-Rom fetch transfer at fractional offset f along one axis."""
    t = f
    weights = [(-t + 2 * t ** 2 - t ** 3) / 2, (2 - 5 * t ** 2 + 3 * t ** 3) / 2, (t + 4 * t ** 2 - 3 * t ** 3) / 2, (-t ** 2 + t ** 3) / 2]
    return sum(c * np.exp(1j * w * j) for c, j in zip(weights, (-1, 0, 1, 2))) * np.exp(-1j * w * f)


def motion_model(d, pops, inter_e2, fx, fy, window):
    """Section 4's steady state h = a / (1 - (1 - a) K) at the measured median offsets with
    a = 1 / W: bias applied to the reference film, plus Interactive's per-pixel squared error
    (one 8-ray sample against the film) times the noise gain."""
    a = 1 / window
    wy = 2 * np.pi * np.fft.fftfreq(H)[:, None]; wx = 2 * np.pi * np.fft.fftfreq(W)[None, :]
    K = cr_kernel(wx, fx) * cr_kernel(wy, fy)
    T = a / (1 - (1 - a) * K)
    gain = float(np.mean(a ** 2 / (1 - (1 - a) ** 2 * np.abs(K) ** 2)))
    out = dict(fx=fx, fy=fy, window=window, alpha=a, noise_gain=gain)
    for c in ("L", "D"):
        film = d[c]
        bias = np.real(np.fft.ifft2((T - 1)[..., None] * np.fft.fft2(film, axes=(0, 1)), axes=(0, 1)))
        for s in (0, 4):
            b2 = np.mean((codes(film + bias, s) - codes(film, s)) ** 2, -1)
            n2 = inter_e2[f"{c}{s}"] * gain
            for name in ("partial", "lit", "contact", "multi-surface", "background-centre", "all-covered", "image", "edge"):
                m = pops[name]
                if m.any():
                    out[f"{c}{s}/{name}"] = dict(bias=float(np.sqrt(b2[m].mean())), noise=float(np.sqrt(n2[m].mean())),
                                                 total=float(np.sqrt(b2[m].mean() + n2[m].mean())))
    return out


def edge_mask(placement):
    """Within 1 px of an instance or background boundary (8-neighbourhood, centre placements)."""
    p = np.pad(placement, 1, mode="edge"); e = np.zeros(placement.shape, bool)
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            e |= p[1 + dy:1 + dy + H, 1 + dx:1 + dx + W] != placement
    return e


def compact(directory, pose, motion, model_frames, sequence_frames):
    d = ts.decode(directory, pose)
    pops = ts.populations(d)
    placement = d["hit"][..., 0].copy()
    pops["edge"] = edge_mask(placement)
    bits = np.zeros((H, W), np.uint32)
    for i, name in enumerate(POPS):
        bits |= pops[name].astype(np.uint32) << i
    out = dict(masks=bits, placement=placement, V=d["ref"].astype(np.float32), sigma=d["sigma"].astype(np.float32),
               n=d["n"].astype(np.float32), full_n=np.int64(d["full_n"]), cover=d["cover"],
               world=np.fromfile(directory / f"pose-{pose}.world.f32", dtype="<f4").reshape(H, W, 4)[..., :3].copy())
    for c in ("L", "D"):
        h0, h1 = d[c + "_halves"]
        for s in (0, 4):
            out[f"code_{c}{s}"] = codes(d[c], s).astype(np.float16)
            out[f"noise_{c}{s}"] = np.mean(((h0 - h1) / 2 * code_slope(d[c], s)) ** 2, -1).astype(np.float32)
    extra = {}
    frames = [f for f in model_frames if sequence_frames[f] == pose]
    if frames:
        iL, iD, _ = ts.centre(directory, "interactive", 0)
        e2 = {f"{c}{s}": np.mean((codes(est, s) - codes(d[c], s)) ** 2, -1) for c, est in (("L", iL), ("D", iD)) for s in (0, 4)}
        for f in frames:
            o = offsets(out["world"].astype(np.float64), d["cover"], motion[f - 1]["matrix"], motion[f]["matrix"])
            extra[str(f)] = dict(offsets=o, model=motion_model(d, pops, e2, o["fx_median"], o["fy_median"], o["window_median"]))
    np.savez_compressed(directory / "compact.npz", **out)
    (directory / "compact.json").write_text(json.dumps(dict(pose=pose, info=d["info"], populations={k: int(v.sum()) for k, v in pops.items()}, model=extra), indent=1))
    for suffix in RAW:
        p = directory / f"pose-{pose}{suffix}"
        if p.exists():
            p.unlink()


def load(directory):
    z = np.load(directory / "compact.npz")
    c = {k: z[k] for k in z.files}
    c["pops"] = {name: (c["masks"] >> i & 1).astype(bool) for i, name in enumerate(POPS)}
    c["valid_f"] = c["cover"] & (c["n"] > 0)
    c["single"] = c["cover"] & (c["n"] == c["full_n"])
    return c


class Errors:
    """Per-pixel squared and signed display-code errors of one estimate, and R_F's error."""

    def __init__(self, c, L, D, V):
        self.e2, self.signed = {}, {}
        for ch, est in (("L0", L), ("L4", L), ("D0", D), ("D4", D)):
            e = codes(est, int(ch[1])) - c[f"code_{ch}"].astype(np.float64)
            self.e2[ch] = np.mean(e ** 2, -1); self.signed[ch] = np.mean(e, -1)
        self.f_valid = c["valid_f"] & c["single"] & np.isfinite(V)
        self.f_err = np.where(self.f_valid, np.nan_to_num(V) - c["V"], 0.)
        self.V = V

    def row(self, ch, m):
        if ch == "F":
            m = m & self.f_valid
            e = self.f_err[m]
            if not len(e):
                return None
            return dict(n=int(len(e)), rms=float(np.sqrt(np.mean(e ** 2))), mean=float(e.mean()),
                        se=float(e.std(ddof=1) / math.sqrt(len(e))) if len(e) > 1 else 0., max=float(np.abs(e).max()))
        k = int(m.sum())
        if not k:
            return None
        e2 = self.e2[ch][m]; s = self.signed[ch][m]
        return dict(n=k, rms=float(np.sqrt(e2.mean())), mean=float(s.mean()), se=float(s.std(ddof=1) / math.sqrt(k)) if k > 1 else 0.,
                    max=float(np.sqrt(e2.max())))


def table(errors, pops, mask):
    return {ch: {p: r for p in pops if (r := errors.row(ch, pops[p] & mask)) is not None} for ch in CHANNELS + ("F",)}


def defects(errors, c, mask):
    v = errors.f_valid & mask
    return dict(missed=int((v & (c["V"] <= .9) & (errors.V >= .99)).sum()), false=int((v & (c["V"] >= .99) & (errors.V <= .9)).sum()),
                D4_over16=int(((np.sqrt(errors.e2["D4"]) > 16) & mask).sum()))


def tone(a, stops):
    return Image.fromarray(np.rint(np.clip(codes(a, stops), 0, 255)).astype("u1"))


def error_image(a, ref_codes, stops, scale=8):
    v = np.clip(np.mean(codes(a, stops) - ref_codes.astype(np.float64), -1) * scale / 255, -1, 1)
    return Image.fromarray(np.rint(np.stack([np.maximum(v, 0), np.zeros_like(v), np.maximum(-v, 0)], -1) * 255).astype("u1"))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", type=Path, required=True)
    ap.add_argument("--t1", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--compact-only", action="store_true")
    ap.add_argument("--model-frames", default="48,52,56,60,64")
    ap.add_argument("--images", default="0,1,3,7,31,48,64,72,96,128,160")
    a = ap.parse_args(); start = time.perf_counter()
    run = json.loads((a.t1 / "motion.json").read_text())
    sequence = json.loads(Path(run["sequence"]).read_text())["frames"]
    manifest = json.loads((a.ref.parent / f"{a.ref.name}.json").read_text())
    poses = manifest["poses"]
    ref_of = reference_of(sequence, poses)
    motion = {f["frame"]: f for f in run["frames"]}
    model_frames = [int(f) for f in a.model_frames.split(",") if f]
    # 1. Compact every completed reference.
    for pose in poses:
        directory = a.ref / f"ref-p{pose}"
        if (directory / "complete").exists() and not (directory / "compact.npz").exists():
            t = time.perf_counter()
            compact(directory, pose, motion, model_frames, ref_of)
            print("compacted", pose, round(time.perf_counter() - t, 1), "s", flush=True)
    if a.compact_only:
        return
    a.out.mkdir(parents=True, exist_ok=True); images = a.out / "images"; images.mkdir(exist_ok=True)
    spread = json.loads(STATIC_SPREAD.read_text())["poses"]["0"]["bars"]
    bar = lambda ch, p, e: e * (1 + spread.get(ch, {}).get(p, {}).get("spread_2sd", 0.))
    scored = run["scored"]
    image_frames = [int(f) for f in a.images.split(",")]
    frames, f2 = {}, {}
    cache = {}

    def get(pose):
        # The last few compacts: a frame, its predecessor and the frame 8 before.
        if pose not in cache:
            if len(cache) >= 4:
                cache.pop(next(iter(cache)))
            cache[pose] = load(a.ref / f"ref-p{pose}")
        return cache[pose]

    # A static span compares with the reference 8 frames before the camera arrived.
    arrival = {}
    for i in range(len(sequence)):
        if ref_of.get(i) is not None and (i == 0 or ref_of.get(i - 1) != ref_of[i]):
            arrival.setdefault(ref_of[i], i)
    previous_capture = {}
    for f in scored:
        pose = ref_of[f]
        if pose is None:
            continue
        c = get(pose); pops = c["pops"]
        directory = a.ref / f"ref-p{pose}"
        L = hdr(a.t1 / f"full-f{f:03}.rgba16f"); D = hdr(a.t1 / f"sun-f{f:03}.rgba16f")
        V = hdr(a.t1 / f"visibility-f{f:03}.rgba16f")[..., 0].astype(np.float64)
        age = np.fromfile(a.t1 / f"age-f{f:03}.u8", dtype="u1").reshape(H, W)
        eL, eD, ev = ts.centre(directory, "exact", 0); iL, iD, iv = ts.centre(directory, "interactive", 0)
        t1, ex, it = Errors(c, L, D, V), Errors(c, eL, eD, ev), Errors(c, iL, iD, iv)
        everything = np.ones((H, W), bool)
        converged = age >= 32
        row = dict(reference=pose, phase=sequence[f]["phase"], moving=motion[f]["moving"], restarted=motion[f]["restarted_pixels"],
                   converged_pixels=int(converged.sum()), t1=table(t1, pops, everything), exact=table(ex, pops, everything),
                   interactive=table(it, pops, everything),
                   defects=dict(t1=defects(t1, c, everything), exact=defects(ex, c, everything)))
        if f >= 32:
            row["converged"] = dict(t1=table(t1, pops, converged), exact=table(ex, pops, converged))
            row["defects_converged"] = dict(t1=defects(t1, c, converged), exact=defects(ex, c, converged))
        # F2 pools: the start event (frames 0..7, every pixel of age f) and disocclusions.
        for kind, mask in (("start", (age == f) & (f < 8)), ("revealed", (age < 8) & (age < f))):
            for k in np.unique(age[mask]):
                m = mask & (age == k)
                for name in pops:
                    pm = pops[name] & m
                    if not pm.any():
                        continue
                    for ch in CHANNELS:
                        key = f"{kind}/{int(k)}/{name}/{ch}"
                        acc = f2.setdefault(key, [0, 0., 0., 0.])
                        acc[0] += int(pm.sum()); acc[1] += float(t1.e2[ch][pm].sum())
                        acc[2] += float(it.e2[ch][pm].sum()); acc[3] += float(ex.e2[ch][pm].sum())
        # F5 and the changed population: |R_t - R_{t-8}| > 0.05 on R_F or > 8 codes on R_D +4.
        back = ref_of.get(f - 8)
        if back == pose:
            back = ref_of.get(arrival[pose] - 8)
        if back is not None and back != pose:
            b = get(back)
            changed = (np.abs(c["V"] - b["V"]) > .05) | (np.mean(np.abs(c["code_D4"].astype(np.float64) - b["code_D4"].astype(np.float64)), -1) > 8)
            changed &= c["cover"]
            row["changed"] = dict(pixels=int(changed.sum()), t1={ch: t1.row(ch, changed) for ch in CHANNELS + ("F",)},
                                  exact={ch: ex.row(ch, changed) for ch in CHANNELS + ("F",)})
        # F6: consecutive scored frames.
        if f - 1 in previous_capture:
            p_pose, pL, peL = previous_capture[f - 1]
            pc = get(p_pose)
            same = c["cover"] & pc["cover"] & (c["placement"] == pc["placement"]) & (age >= 8)
            res = {}
            for s in (0, 4):
                dR = c[f"code_L{s}"].astype(np.float64) - pc[f"code_L{s}"].astype(np.float64)
                for name, cur, prev in (("t1", L, pL), ("exact", eL, peL)):
                    r = np.mean((codes(cur, s) - codes(prev, s) - dR) ** 2, -1)
                    res[f"{name}_L{s}"] = float(np.sqrt(r[same].mean())) if same.any() else None
                    res[f"{name}_L{s}_step"] = float(np.sqrt(np.mean((codes(cur, s) - codes(prev, s)) ** 2, -1)[same].mean())) if same.any() else None
            row["pair"] = dict(previous=f - 1, static=p_pose == pose, pixels=int(same.sum()), **res)
        previous_capture = {f: (pose, L, eL)}
        if f in image_frames:
            for s in (0, 4):
                tone(L, s).save(images / f"f{f:03}-t1-L{s}.png")
                error_image(L, c[f"code_L{s}"], s).save(images / f"f{f:03}-t1-L{s}-err-x8.png")
                error_image(eL, c[f"code_L{s}"], s).save(images / f"f{f:03}-exact-L{s}-err-x8.png")
                error_image(D, c[f"code_D{s}"], s).save(images / f"f{f:03}-t1-D{s}-err-x8.png")
            error_image(iL, c["code_L4"], 4).save(images / f"f{f:03}-interactive-L4-err-x8.png")
            Image.fromarray((np.clip(age, 0, 32) * 7).astype("u1")).save(images / f"f{f:03}-age.png")
        frames[str(f)] = row
        print("frame", f, "ref", pose, round(time.perf_counter() - start, 1), "s", flush=True)
    result = dict(run=dict(sequence=run["sequence"], replay_identical=run["replay_identical"], identity=run["identity"]),
                  frames=frames, f2_pools=f2, gates=gates(frames, f2, bar, run, a.ref, poses))
    result["seconds"] = time.perf_counter() - start
    (a.out / "score.json").write_text(json.dumps(result, indent=1, allow_nan=False))
    print(json.dumps(result["gates"], indent=1)[:6000])


def gates(frames, f2, bar, run, ref, poses):
    g = {}
    g["T-0"] = dict(passed=all(i["hdr_identical"] for i in run["identity"]), cases=run["identity"])
    g["F8_replay"] = dict(passed=bool(run["replay_identical"]))
    # F1: pixels >= 32 frames past their last event, per population, T1 <= bar on every channel.
    fails, solid = [], {}
    for f, row in frames.items():
        if "converged" not in row:
            continue
        t1, ex = row["converged"]["t1"], row["converged"]["exact"]
        for ch in CHANNELS + ("F",):
            for p, v in t1[ch].items():
                if p not in ex[ch]:
                    continue
                b = bar(ch, p, ex[ch][p]["rms"])
                if v["rms"] > b:
                    fails.append(dict(frame=int(f), channel=ch, population=p, rms=v["rms"], bar=b, ratio=v["rms"] / b if b else None, n=v["n"]))
        solid[f] = dict(F_max=max(t1["F"].get(k, {}).get("max", 0) for k in ("solid-lit", "solid-occluded")),
                        D_max={ch: {k: [t1[ch].get(k, {}).get("max"), ex[ch].get(k, {}).get("max")] for k in ("solid-lit", "solid-occluded")} for ch in ("D0", "D4")})
    solid_fail = {f: s for f, s in solid.items() if s["F_max"] > .002 or any(v[0] is not None and v[1] is not None and v[0] > v[1] for ch in s["D_max"].values() for v in ch.values())}
    fails.sort(key=lambda r: -(r["ratio"] or 0))
    g["F1_converged"] = dict(passed=not fails and not solid_fail, failures=fails[:40], failure_count=len(fails), solid_failures=solid_fail,
                             worst_ratio_by_frame={f: max([v["rms"] / bar(ch, p, row["converged"]["exact"][ch][p]["rms"]) for ch in CHANNELS + ("F",) for p, v in row["converged"]["t1"][ch].items()
                                                         if p in row["converged"]["exact"][ch] and row["converged"]["exact"][ch][p]["rms"] > 0], default=None)
                                                   for f, row in frames.items() if "converged" in row})
    # F2: pooled by event kind and age; T1 <= Interactive at every age, <= bar at age 7.
    f2f = []
    for key, (n, t, i, e) in f2.items():
        kind, age, p, ch = key.split("/")
        rt, ri, re = math.sqrt(t / n), math.sqrt(i / n), math.sqrt(e / n)
        if rt > ri * (1 + 1e-6) + 1e-9:
            f2f.append(dict(event=kind, age=int(age), population=p, channel=ch, n=n, rms=rt, interactive=ri, kind="above Interactive"))
        if int(age) == 7 and rt > bar(ch, p, re):
            f2f.append(dict(event=kind, age=7, population=p, channel=ch, n=n, rms=rt, bar=bar(ch, p, re), kind="above bar at age 7"))
    f2f.sort(key=lambda r: (-r["n"]))
    g["F2_transient"] = dict(passed=not f2f, failures=f2f[:40], failure_count=len(f2f))
    # F3: converged bias.
    f3 = []
    for f, row in frames.items():
        if "converged" not in row:
            continue
        t1, ex = row["converged"]["t1"], row["converged"]["exact"]
        for ch in CHANNELS + ("F",):
            for p, v in t1[ch].items():
                if p in ex[ch] and abs(v["mean"]) > abs(ex[ch][p]["mean"]) + 4 * v["se"]:
                    f3.append(dict(frame=int(f), channel=ch, population=p, mean=v["mean"], exact_mean=ex[ch][p]["mean"], se=v["se"], n=v["n"]))
    g["F3_bias"] = dict(passed=not f3, failures=f3[:30], failure_count=len(f3))
    # F4: per frame (single frames; persistence is not checked, which is conservative).
    f4 = {f: row["defects"] for f, row in frames.items()}
    bad = {f: d for f, d in f4.items() if d["t1"]["missed"] + d["t1"]["false"] > d["exact"]["missed"] + d["exact"]["false"] or d["t1"]["D4_over16"] > d["exact"]["D4_over16"]}
    g["F4_defects"] = dict(passed=not bad, failures=bad, D4_excess_frames=[f for f, d in f4.items() if d["t1"]["D4_over16"] > d["exact"]["D4_over16"]])
    # F5: changed pixels 8 frames after motion stops (hold frame 72 against the 56 -> 64 change), and ghost bias.
    f5 = dict(ghost=[])
    for f, row in frames.items():
        if "changed" in row:
            for ch in CHANNELS + ("F",):
                t, e = row["changed"]["t1"].get(ch), row["changed"]["exact"].get(ch)
                if t and e and abs(t["mean"]) > abs(e["mean"]) + 2 * t["se"]:
                    f5["ghost"].append(dict(frame=int(f), channel=ch, mean=t["mean"], exact_mean=e["mean"], se=t["se"], n=t["n"]))
    after = []
    for f, row in frames.items():
        if row["phase"] == "hold" and "changed" in row and int(f) - 64 == 8:
            for ch in CHANNELS + ("F",):
                t, e = row["changed"]["t1"].get(ch), row["changed"]["exact"].get(ch)
                if t and e:
                    after.append(dict(frame=int(f), channel=ch, rms=t["rms"], bar=bar(ch, "changed", e["rms"]), n=t["n"], passed=t["rms"] <= bar(ch, "changed", e["rms"])))
    g["F5_changed_after_stop"] = dict(passed=bool(after) and all(x["passed"] for x in after), rows=after)
    g["F5_ghost_bias"] = dict(passed=not f5["ghost"], failures=f5["ghost"][:30],
                              note="changed: |R_t - R_(t-8)| > 0.05 on R_F or > 8 codes on R_D +4; zero-weight-from-hidden-surface identity output not built")
    # F6: motion residual on unchanged surfaces <= Exact's; static hold pairs <= 0.5 / 1 code.
    f6 = []
    for f, row in frames.items():
        p = row.get("pair")
        if not p or p["t1_L0"] is None:
            continue
        if p["static"]:
            ok = p["t1_L0_step"] <= .5 and p["t1_L4_step"] <= 1.
        else:
            ok = p["t1_L0"] <= p["exact_L0"] and p["t1_L4"] <= p["exact_L4"]
        f6.append(dict(frames=[p["previous"], int(f)], passed=ok, **{k: v for k, v in p.items() if k != "previous"}))
    g["F6_flicker"] = dict(passed=bool(f6) and all(x["passed"] for x in f6), pairs=f6)
    # Section 4 diagnostic: steady walk error within 1.25x of the model at the measured offsets.
    diag = []
    for pose in poses:
        j = ref / f"ref-p{pose}" / "compact.json"
        if not j.exists():
            continue
        for f, m in json.loads(j.read_text())["model"].items():
            row = frames.get(f)
            if not row or "converged" not in row:
                continue
            for key, v in m["model"].items():
                if "/" not in key:
                    continue
                ch, p = key.split("/")
                got = row["converged"]["t1"][ch].get(p)
                if got:
                    diag.append(dict(frame=int(f), channel=ch, population=p, measured=got["rms"], model=v["total"], ratio=got["rms"] / v["total"] if v["total"] else None,
                                     model_bias=v["bias"], model_noise=v["noise"]))
    diag.sort(key=lambda r: -(r["ratio"] or 0))
    g["model_diagnostic"] = dict(passed=bool(diag) and all(r["ratio"] is None or r["ratio"] <= 1.25 for r in diag), worst=diag[:20])
    return g


if __name__ == "__main__":
    main()
