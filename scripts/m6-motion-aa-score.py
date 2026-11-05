"""T2a motion scoring (docs/m6-motion-aa.md section 7) of a camera sequence with sample records.

Inputs: per-camera Level F references (gpu_film_t1_reference over the sequence's distinct cameras,
one Exact and one Interactive centre frame each, compacted on first use, which deletes their raw
dumps) and T2a's run (gpu_film_t2_motion: film captures of the full, sun-only and visibility
programs, per-pixel ages and record flags, per-frame record diagnostics, T-0 and replay results).
Frames with the same camera share a reference (holds, the return).

Gates, against the frozen T1 section 7 checks and the T2a stop rules:
- T-0: reset frames and every pixel without a kept record equal Interactive (HDR, hit, counts);
- F1: pixels >= 32 frames past their last event (a frame without any kept record), per population,
  T2 RMS <= bar on L0, L4, D0, D4 and F; solids' R_F maximum <= 0.002, split into receiver-band
  suspects (a kept or fresh record whose hit lay within 1e-4 barycentric of an edge with all rays
  blocked) and the rest, and, with --verify, restricted to replica-confirmed solids;
- F2 (pooled by event kind and age), F3, F4 (any R_D excess is the frozen stop), F5 (changed pixels
  8 frames after motion stops, ghost bias, and the exact count of kept records found hidden), F6;
- F8: every replay identical to the first run, records included;
- the CPU-model check: all-covered L0 within 1.25x of section 3 (0.58x the bar for K = 4, 0.44x
  for K = 8).
R_F is scored on single-surface pixels, where the visibility program's film has R_F's estimand.

py -3 -B scripts/m6-motion-aa-score.py --ref <refs> --run <run> --out <scores> [--verify <replica refs>]
Math-library threads should be set to two. Peak memory is about 1.3 GiB.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path
import time

import numpy as np
from PIL import Image, ImageDraw

here = Path(__file__).parent
spec = importlib.util.spec_from_file_location("gpu_score", here / "m6-temporal-gpu-score.py")
ts = importlib.util.module_from_spec(spec); spec.loader.exec_module(ts)
H, W = ts.H, ts.W
codes, code_slope, hdr = ts.codes, ts.code_slope, ts.hdr
POPS = ts.POPS + ("edge", "solid-lit", "solid-occluded")
CHANNELS = ("L0", "L4", "D0", "D4")
STATIC_SPREAD = Path("E:/renderlab-work/t1-gpu/scores-t1k/score.json")
RAW = (".film.bin", ".direct.bin", ".world.f32", ".normal.f32", ".hit.u32", ".exact.rgba16f", ".exact.u32")
# Section 3's CPU estimate of all-covered L0 under motion, times 1.25 (the frozen check).
CPU_MODEL = {4: 0.58, 8: 0.44}
# A silhouette of the cliff against the sky, as T1b's crops (x0, y0, width, height).
SILHOUETTE = (1290, 400, 320, 180)


def camera_key(frame):
    return json.dumps([frame[k] for k in ("eye", "target", "up", "fov_y_radians", "near", "far", "width", "height")])


def reference_of(sequence, cameras, poses):
    """Scored frame -> the reference pose (a frame of the references' camera file) with the same
    camera."""
    keys = {camera_key(cameras[p]): p for p in poses}
    return {i: keys.get(camera_key(f)) for i, f in enumerate(sequence)}


def edge_mask(placement):
    """Within 1 px of an instance or background boundary (8-neighbourhood, centre placements)."""
    p = np.pad(placement, 1, mode="edge"); e = np.zeros(placement.shape, bool)
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            e |= p[1 + dy:1 + dy + H, 1 + dx:1 + dx + W] != placement
    return e


def compact(directory, pose):
    d = ts.decode(directory, pose)
    pops = ts.populations(d)
    placement = d["hit"][..., 0].copy()
    pops["edge"] = edge_mask(placement)
    bits = np.zeros((H, W), np.uint32)
    for i, name in enumerate(POPS):
        bits |= pops[name].astype(np.uint32) << i
    out = dict(masks=bits, placement=placement, V=d["ref"].astype(np.float32), sigma=d["sigma"].astype(np.float32),
               n=d["n"].astype(np.float32), full_n=np.int64(d["full_n"]), cover=d["cover"])
    for c in ("L", "D"):
        h0, h1 = d[c + "_halves"]
        for s in (0, 4):
            out[f"code_{c}{s}"] = codes(d[c], s).astype(np.float16)
            out[f"noise_{c}{s}"] = np.mean(((h0 - h1) / 2 * code_slope(d[c], s)) ** 2, -1).astype(np.float32)
    np.savez_compressed(directory / "compact.npz", **out)
    (directory / "compact.json").write_text(json.dumps(dict(pose=pose, info=d["info"], populations={k: int(v.sum()) for k, v in pops.items()}), indent=1))
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


def replica_solids(verify, pose):
    """Replica-confirmed solids at `pose`: the 16-replica visibility estimate is exactly 0 or 1
    (gpu_film_t1_reference with replicas and visibility_only), or None without a verification."""
    if verify is None:
        return None
    p = verify / f"ref-p{pose}" / f"pose-{pose}.film.bin"
    if not p.exists():
        return None
    film = np.fromfile(p, dtype=ts.DTYPE).reshape(H, W)
    c = film["counts"].astype(np.float64); n = c[..., 2] + c[..., 3]
    v = np.divide(c[..., 0] + c[..., 1], 256 * n, out=np.full((H, W), np.nan), where=n > 0)
    return v


class Errors:
    """Per-pixel squared and signed display-code errors of one estimate, and R_F's error."""

    def __init__(self, c, L, D, V):
        self.e2, self.signed = {}, {}
        for ch, est in (("L0", L), ("L4", L), ("D0", D), ("D4", D)):
            e = (codes(est, int(ch[1])) - c[f"code_{ch}"].astype(np.float64)).astype(np.float32)
            self.e2[ch] = np.mean(e.astype(np.float64) ** 2, -1).astype(np.float32); self.signed[ch] = np.mean(e, -1)
        self.f_valid = c["valid_f"] & c["single"] & np.isfinite(V)
        self.f_err = np.where(self.f_valid, np.nan_to_num(V) - c["V"], 0.).astype(np.float32)
        self.V = V

    def row(self, ch, m):
        if ch == "F":
            m = m & self.f_valid
            e = self.f_err[m].astype(np.float64)
            if not len(e):
                return None
            return dict(n=int(len(e)), rms=float(np.sqrt(np.mean(e ** 2))), mean=float(e.mean()),
                        se=float(e.std(ddof=1) / math.sqrt(len(e))) if len(e) > 1 else 0., max=float(np.abs(e).max()))
        k = int(m.sum())
        if not k:
            return None
        e2 = self.e2[ch][m].astype(np.float64); s = self.signed[ch][m].astype(np.float64)
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


def visibility_image(V, ref, scale=8):
    v = np.clip(np.nan_to_num(V - ref) * scale, -1, 1)
    return Image.fromarray(np.rint(np.stack([np.maximum(v, 0), np.zeros_like(v), np.maximum(-v, 0)], -1) * 255).astype("u1"))


def crop_sheet(panels, path, box=SILHOUETTE, zoom=2):
    """Labelled crops of `box` from (label, image) panels, two per row, at `zoom`."""
    x0, y0, w, h = box
    cols = 2; rows = (len(panels) + 1) // cols
    sheet = Image.new("RGB", (cols * w * zoom, rows * (h * zoom + 18)), (20, 27, 35))
    d = ImageDraw.Draw(sheet)
    for i, (label, im) in enumerate(panels):
        x, y = (i % cols) * w * zoom, (i // cols) * (h * zoom + 18)
        d.text((x + 4, y + 3), label, fill="white")
        sheet.paste(im.convert("RGB").crop((x0, y0, x0 + w, y0 + h)).resize((w * zoom, h * zoom), Image.Resampling.NEAREST), (x, y + 18))
    sheet.save(path)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ref", type=Path, required=True)
    ap.add_argument("--run", type=Path)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--verify", type=Path)
    ap.add_argument("--compact-only", action="store_true")
    ap.add_argument("--images", default="0,1,3,7,31,48,64,72,96,99,104,128,160")
    a = ap.parse_args(); start = time.perf_counter()
    manifest = json.loads((a.ref.parent / f"{a.ref.name}.json").read_text())
    poses = manifest["poses"]
    for pose in poses:
        directory = a.ref / f"ref-p{pose}"
        if (directory / "complete").exists() and not (directory / "compact.npz").exists():
            t = time.perf_counter()
            compact(directory, pose)
            print("compacted", pose, round(time.perf_counter() - t, 1), "s", flush=True)
    if a.compact_only:
        return
    run = json.loads((a.run / "motion.json").read_text())
    sequence = json.loads(Path(run["sequence"]).read_text())["frames"]
    cameras = json.loads(Path(manifest["t1_cameras"]).read_text())["frames"]
    ref_of = reference_of(sequence, cameras, poses)
    motion = {f["frame"]: f for f in run["frames"]}
    a.out.mkdir(parents=True, exist_ok=True); images = a.out / "images"; images.mkdir(exist_ok=True)
    spread = json.loads(STATIC_SPREAD.read_text())["poses"]["0"]["bars"]
    bar = lambda ch, p, e: e * (1 + spread.get(ch, {}).get(p, {}).get("spread_2sd", 0.))
    scored = run["scored"]
    image_frames = [int(f) for f in a.images.split(",")]
    frames, f2 = {}, {}
    cache = {}

    def get(pose):
        if pose not in cache:
            if len(cache) >= 3:
                cache.pop(next(iter(cache)))
            cache[pose] = load(a.ref / f"ref-p{pose}")
        return cache[pose]

    arrival = {}
    for i in range(len(sequence)):
        if ref_of.get(i) is not None and (i == 0 or ref_of.get(i - 1) != ref_of[i]):
            arrival.setdefault(ref_of[i], i)
    previous_capture = {}
    solids = []
    for f in scored:
        pose = ref_of[f]
        if pose is None:
            continue
        c = get(pose); pops = c["pops"]
        directory = a.ref / f"ref-p{pose}"
        L = hdr(a.run / f"full-f{f:03}.rgba16f"); D = hdr(a.run / f"sun-f{f:03}.rgba16f")
        V = hdr(a.run / f"visibility-f{f:03}.rgba16f")[..., 0].astype(np.float64)
        age = np.fromfile(a.run / f"age-f{f:03}.u8", dtype="u1").reshape(H, W)
        flag_path = a.run / f"flags-f{f:03}.u16"
        flags = np.fromfile(flag_path, dtype="<u2").reshape(H, W) if flag_path.exists() else None
        eL, eD, ev = ts.centre(directory, "exact", 0); iL, iD, iv = ts.centre(directory, "interactive", 0)
        t2, ex, it = Errors(c, L, D, V), Errors(c, eL, eD, ev), Errors(c, iL, iD, iv)
        del eD, ev, iD, iv
        everything = np.ones((H, W), bool)
        converged = age >= 32
        m = motion.get(f, {})
        row = dict(reference=pose, phase=sequence[f]["phase"], moving=m.get("moving"),
                   empty=m.get("empty_pixels"), cold=m.get("cold_pixels"), hidden_records=m.get("hidden_records"),
                   converged_pixels=int(converged.sum()), t2=table(t2, pops, everything), exact=table(ex, pops, everything),
                   interactive=table(it, pops, everything),
                   defects=dict(t2=defects(t2, c, everything), exact=defects(ex, c, everything)))
        if f >= 32:
            row["converged"] = dict(t2=table(t2, pops, converged), exact=table(ex, pops, converged))
            row["defects_converged"] = dict(t2=defects(t2, c, converged), exact=defects(ex, c, converged))
            # F1's solid maximum: every converged single-surface solid over 0.002, with its flags.
            replicas = replica_solids(a.verify, pose)
            for name in ("solid-lit", "solid-occluded"):
                mm = pops[name] & converged & t2.f_valid & (np.abs(t2.f_err) > .002)
                for y, x in zip(*np.nonzero(mm)):
                    fl = int(flags[y, x]) if flags is not None else -1
                    confirmed = None if replicas is None else bool(replicas[y, x] == c["V"][y, x])
                    solids.append(dict(frame=f, pose=pose, population=name, pixel=[int(x), int(y)], error=float(t2.f_err[y, x]),
                                       age=int(age[y, x]), flags=fl, kept=fl & 0xf if fl >= 0 else None,
                                       band=bool(fl >= 0 and fl & 0x180), hidden=bool(fl >= 0 and fl & 0x200),
                                       discontinuity=bool(fl >= 0 and fl & 0x20), replica_confirmed=confirmed))
        for kind, mask in (("start", (age == f) & (f < 8)), ("revealed", (age < 8) & (age < f))):
            for k in np.unique(age[mask]):
                mk = mask & (age == k)
                for name in pops:
                    pm = pops[name] & mk
                    if not pm.any():
                        continue
                    for ch in CHANNELS:
                        key = f"{kind}/{int(k)}/{name}/{ch}"
                        acc = f2.setdefault(key, [0, 0., 0., 0.])
                        acc[0] += int(pm.sum()); acc[1] += float(t2.e2[ch][pm].sum())
                        acc[2] += float(it.e2[ch][pm].sum()); acc[3] += float(ex.e2[ch][pm].sum())
        back = ref_of.get(f - 8)
        if back == pose:
            back = ref_of.get(arrival[pose] - 8)
        if back is not None and back != pose:
            b = get(back)
            changed = (np.abs(c["V"] - b["V"]) > .05) | (np.mean(np.abs(c["code_D4"].astype(np.float64) - b["code_D4"].astype(np.float64)), -1) > 8)
            changed &= c["cover"]
            row["changed"] = dict(pixels=int(changed.sum()), t2={ch: t2.row(ch, changed) for ch in CHANNELS + ("F",)},
                                  exact={ch: ex.row(ch, changed) for ch in CHANNELS + ("F",)})
        if f - 1 in previous_capture:
            p_pose, pL, peL = previous_capture[f - 1]
            pc = get(p_pose)
            same = c["cover"] & pc["cover"] & (c["placement"] == pc["placement"]) & (age >= 8)
            res = {}
            for s in (0, 4):
                dR = c[f"code_L{s}"].astype(np.float64) - pc[f"code_L{s}"].astype(np.float64)
                for name, cur, prev in (("t2", L, pL), ("exact", eL, peL)):
                    r = np.mean((codes(cur, s) - codes(prev, s) - dR) ** 2, -1)
                    res[f"{name}_L{s}"] = float(np.sqrt(r[same].mean())) if same.any() else None
                    res[f"{name}_L{s}_step"] = float(np.sqrt(np.mean((codes(cur, s) - codes(prev, s)) ** 2, -1)[same].mean())) if same.any() else None
            row["pair"] = dict(previous=f - 1, static=p_pose == pose, pixels=int(same.sum()), **res)
        previous_capture = {f: (pose, L, eL)}
        if f in image_frames:
            panels = []
            for s in (0, 4):
                tone(L, s).save(images / f"f{f:03}-t2-L{s}.png")
                error_image(L, c[f"code_L{s}"], s).save(images / f"f{f:03}-t2-L{s}-err-x8.png")
                error_image(eL, c[f"code_L{s}"], s).save(images / f"f{f:03}-exact-L{s}-err-x8.png")
                error_image(D, c[f"code_D{s}"], s).save(images / f"f{f:03}-t2-D{s}-err-x8.png")
            error_image(iL, c["code_L4"], 4).save(images / f"f{f:03}-interactive-L4-err-x8.png")
            visibility_image(V, c["V"]).save(images / f"f{f:03}-t2-F-err-x8.png")
            Image.fromarray((np.clip(age, 0, 32) * 7).astype("u1")).save(images / f"f{f:03}-age.png")
            for label, name in (("T2 L0", "t2-L0"), ("T2 L4 err x8", "t2-L4-err-x8"), ("Exact L4 err x8", "exact-L4-err-x8"),
                                ("Interactive L4 err x8", "interactive-L4-err-x8"), ("T2 D4 err x8", "t2-D4-err-x8"),
                                ("T2 R_F err x8", "t2-F-err-x8")):
                panels.append((label, Image.open(images / f"f{f:03}-{name}.png")))
            crop_sheet(panels, images / f"f{f:03}-silhouette-crops.png")
            ov = [Image.open(images / f"f{f:03}-{n}.png").convert("RGB").resize((960, 540)) for n in ("t2-L0", "t2-L4-err-x8", "exact-L4-err-x8", "t2-F-err-x8")]
            o = Image.new("RGB", (1920, 1080)); [o.paste(im, ((i % 2) * 960, (i // 2) * 540)) for i, im in enumerate(ov)]
            o.save(images / f"f{f:03}-overview.png")
        frames[str(f)] = row
        del t2, ex, it
        print("frame", f, "ref", pose, round(time.perf_counter() - start, 1), "s", flush=True)
    result = dict(run=dict(sequence=run["sequence"], records=run["records"], replays=run["replays"],
                           replays_identical=run["replays_identical"], identity=run["identity"]),
                  frames=frames, f2_pools=f2, solids=solids, gates=gates(frames, f2, bar, run, solids))
    result["seconds"] = time.perf_counter() - start
    (a.out / "score.json").write_text(json.dumps(result, indent=1, allow_nan=False))
    print(json.dumps(result["gates"], indent=1)[:8000])


def replay_gate(run):
    """F8 over the full program's runs: every run's scored images equal the first run's, and the
    final records are identical within each setting. gpu_film_t2_motion turns test diagnostics on
    in runs 0-1 only, and they store a band marker in the records, so records compare within
    runs 0-1 and within runs 2 onward."""
    runs = run["digests"]["full"]
    images = all(r[:-1] == runs[0][:-1] for r in runs)
    groups = [runs[:2], runs[2:]]
    records = all(len({r[-1] for r in g}) <= 1 for g in groups)
    return dict(passed=images and records, replays=len(runs) - 1, images_identical=images,
                records_identical_within_setting=records, identical_to_first_including_records=run["replays_identical"])


def gates(frames, f2, bar, run, solids):
    g = {}
    empty = run.get("empty_identity", [])
    g["T-0"] = dict(passed=all(i["hdr_identical"] for i in run["identity"]) and bool(run.get("t0_empty_passed")),
                    reset=run["identity"], empty_pixels=empty)
    g["F8_replay"] = replay_gate(run)
    fails = []
    for f, row in frames.items():
        if "converged" not in row:
            continue
        t2, ex = row["converged"]["t2"], row["converged"]["exact"]
        for ch in CHANNELS + ("F",):
            for p, v in t2[ch].items():
                if p not in ex[ch]:
                    continue
                b = bar(ch, p, ex[ch][p]["rms"])
                if v["rms"] > b:
                    fails.append(dict(frame=int(f), channel=ch, population=p, rms=v["rms"], bar=b, ratio=v["rms"] / b if b else None, n=v["n"]))
    d_max = {}
    for f, row in frames.items():
        if "converged" not in row:
            continue
        t2, ex = row["converged"]["t2"], row["converged"]["exact"]
        for ch in ("D0", "D4"):
            for k in ("solid-lit", "solid-occluded"):
                tv, ev = t2[ch].get(k, {}).get("max"), ex[ch].get(k, {}).get("max")
                if tv is not None and ev is not None and tv > ev:
                    d_max.setdefault(f, []).append(dict(channel=ch, population=k, t2=tv, exact=ev))
    fails.sort(key=lambda r: -(r["ratio"] or 0))
    worst = {}
    for f, row in frames.items():
        if "converged" not in row:
            continue
        ratios = [v["rms"] / bar(ch, p, row["converged"]["exact"][ch][p]["rms"]) for ch in CHANNELS + ("F",)
                  for p, v in row["converged"]["t2"][ch].items()
                  if p in row["converged"]["exact"][ch] and row["converged"]["exact"][ch][p]["rms"] > 0]
        worst[f] = max(ratios, default=None)
    outside = [s for s in solids if not s["band"]]
    confirmed = [s for s in outside if s["replica_confirmed"] is not False]
    g["F1_converged"] = dict(passed=not fails and not d_max and not confirmed, failures=fails[:40], failure_count=len(fails),
                             worst_ratio_by_frame=worst, solid_D_max_above_exact=d_max,
                             solids_over_0002=dict(total=len(solids), band_suspects=len(solids) - len(outside), outside_band=len(outside),
                                                   outside_band_not_refuted_by_replicas=len(confirmed),
                                                   max_outside_band=max((abs(s["error"]) for s in outside), default=0.),
                                                   examples=confirmed[:40]))
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
    f3 = []
    for f, row in frames.items():
        if "converged" not in row:
            continue
        t2, ex = row["converged"]["t2"], row["converged"]["exact"]
        for ch in CHANNELS + ("F",):
            for p, v in t2[ch].items():
                if p in ex[ch] and abs(v["mean"]) > abs(ex[ch][p]["mean"]) + 4 * v["se"]:
                    f3.append(dict(frame=int(f), channel=ch, population=p, mean=v["mean"], exact_mean=ex[ch][p]["mean"], se=v["se"], n=v["n"]))
    g["F3_bias"] = dict(passed=not f3, failures=f3[:30], failure_count=len(f3))
    f4 = {f: row["defects"] for f, row in frames.items()}
    excess = [int(f) for f, d in f4.items() if d["t2"]["D4_over16"] > d["exact"]["D4_over16"] and int(f) > 0]
    shadows = {f: d for f, d in f4.items() if d["t2"]["missed"] + d["t2"]["false"] > d["exact"]["missed"] + d["exact"]["false"] and int(f) > 0}
    g["F4_defects"] = dict(passed=not excess and not shadows, D4_excess_frames=excess, shadow_excess=shadows,
                           counts={f: d for f, d in f4.items()})
    ghost = []
    for f, row in frames.items():
        if "changed" in row:
            for ch in CHANNELS + ("F",):
                t, e = row["changed"]["t2"].get(ch), row["changed"]["exact"].get(ch)
                if t and e and abs(t["mean"]) > abs(e["mean"]) + 2 * t["se"]:
                    ghost.append(dict(frame=int(f), channel=ch, mean=t["mean"], exact_mean=e["mean"], se=t["se"], n=t["n"]))
    after = []
    for f, row in frames.items():
        if row["phase"] == "hold" and "changed" in row and int(f) - 64 == 8:
            for ch in CHANNELS + ("F",):
                t, e = row["changed"]["t2"].get(ch), row["changed"]["exact"].get(ch)
                if t and e:
                    after.append(dict(frame=int(f), channel=ch, rms=t["rms"], bar=bar(ch, "changed", e["rms"]), n=t["n"], passed=t["rms"] <= bar(ch, "changed", e["rms"])))
    hidden = [dict(frame=r["frame"], hidden_records=r["hidden_records"], hidden_pixels=r["hidden_pixels"]) for r in run["frames"] if r.get("moving")]
    g["F5_changed_after_stop"] = dict(passed=bool(after) and all(x["passed"] for x in after), rows=after)
    g["F5_ghost_bias"] = dict(passed=not ghost, failures=ghost[:30])
    g["F5_hidden_weight"] = dict(passed=all(h["hidden_records"] == 0 for h in hidden),
                                 total_hidden_records=sum(h["hidden_records"] for h in hidden),
                                 frames_with_hidden=[h for h in hidden if h["hidden_records"]][:40],
                                 note="kept records that an exact camera ray from this frame finds hidden (test-only count; discontinuity pixels' records were already ray-tested)")
    f6 = []
    for f, row in frames.items():
        p = row.get("pair")
        if not p or p["t2_L0"] is None:
            continue
        if p["static"]:
            ok = p["t2_L0_step"] <= .5 and p["t2_L4_step"] <= 1.
        else:
            ok = p["t2_L0"] <= p["exact_L0"] and p["t2_L4"] <= p["exact_L4"]
        f6.append(dict(frames=[p["previous"], int(f)], passed=ok, **{k: v for k, v in p.items() if k != "previous"}))
    g["F6_flicker"] = dict(passed=bool(f6) and all(x["passed"] for x in f6), pairs=f6)
    limit = CPU_MODEL[run["records"]]
    cpu = []
    for f, row in frames.items():
        if "converged" in row and row["moving"] and "all-covered" in row["converged"]["t2"]["L0"]:
            ratio = row["converged"]["t2"]["L0"]["all-covered"]["rms"] / bar("L0", "all-covered", row["converged"]["exact"]["L0"]["all-covered"]["rms"])
            cpu.append(dict(frame=int(f), ratio=ratio))
    g["cpu_model_check"] = dict(passed=bool(cpu) and all(r["ratio"] <= limit for r in cpu), limit=limit, rows=cpu)
    return g


if __name__ == "__main__":
    main()
