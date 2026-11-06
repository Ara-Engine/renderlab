"""N7a scoring (docs/non-rt-n7a.md): the world-space visibility cache against film references.

Quarry: the probe's counts at poses 0 and 1 against the policy film reference R_F of
docs/m6-film-reference.md, beside Unreal VSM, Exact and Interactive scored on the same
populations in the same process (S1 defect counts: contact loss, leaks, flips, missed and
false film shadows).
S2/S3: each case's cache and N0 software Interactive crops against the regenerated S1 film
references, beside the archived per-frame Unreal VSM and Renderlab Interactive rows of
E:/renderlab-work/motion-benches/<bench>/per-frame.json. The regenerated references are checked
against the archived per-frame reference statistics, and the frozen N7 quality stop rule is
applied on every frame: any population RMS above VSM's, missed film shadows above VSM's, or
false shadows above max(VSM's, 2x Interactive's).

py -3 -B scripts/n7a-score.py --probe E:/renderlab-work/n7a/runs/probe-1 --bench S2=E:/renderlab-work/n7a/runs/s2-1 --out E:/renderlab-work/n7a/scores
CPU only; set OPENBLAS_NUM_THREADS=2.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import time

import numpy as np

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS.parent / "support"))
F0 = Path("D:/Coding/renderlab-work/m6-exact-quality/f0")
FROZEN = Path("D:/Coding/renderlab-work/m6-level-b-probe")
NATIVE = Path("D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow")
ARCHIVE = Path("E:/renderlab-work/motion-benches")
SHAPE = (1080, 1920)
POPULATIONS = ("partial", "lit", "occluded", "narrow", "faint", "isolated", "silhouette", "wide", "wide6", "contact", "solid", "contour")
DEFECTS = ("contact_lost", "contact_n", "leaks", "solid_flips", "solid_error_gt_002", "missed_shadows", "false_shadows", "missing_covered")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def counts(path):
    return np.fromfile(path, dtype="<u4").reshape(SHAPE) / 256.0


def compact(row):
    out = {p: row[p].get("rms") for p in POPULATIONS if isinstance(row.get(p), dict)}
    out.update({k: row.get(k) for k in DEFECTS})
    out["partial_n"] = row["partial"].get("n")
    return out


def rule(cache, vsm, interactive):
    """The frozen N7 quality stop rule on one frame: the populations and counts that break it."""
    broken = []
    for p in POPULATIONS:
        a, b = (cache.get(p) or {}).get("rms"), (vsm.get(p) or {}).get("rms")
        if a is not None and b is not None and a > b:
            broken.append(f"{p} rms {a:.4f} > VSM {b:.4f}")
    if cache["missed_shadows"] > vsm["missed_shadows"]:
        broken.append(f"missed {cache['missed_shadows']} > VSM {vsm['missed_shadows']}")
    bound = max(vsm["false_shadows"], 2 * interactive["false_shadows"])
    if cache["false_shadows"] > bound:
        broken.append(f"false {cache['false_shadows']} > {bound}")
    return broken


def quarry(probe, s1):
    film = load("m6-film-score")
    lb = film.lb
    lb.R = FROZEN / "runs/quarry-b6"
    lb.REFERENCE = FROZEN / "gate-final/reference"
    lb.NATIVE = NATIVE
    out = {}
    for pose, files in ((0, {"N7a cache, cold view": "pose0-cold.u32", "N7a cache, warm": "pose0-warm.u32",
                             "N7a cache, Q-walk end": "walk-160.u32", "N0 Interactive (this run)": "pose0-n0-interactive.u32"}),
                        (1, {"N7a cache, after pose 0": "pose1-after0.u32", "N7a cache, warm": "pose1-warm.u32",
                             "N0 Interactive (this run)": "pose1-n0-interactive.u32"})):
        d = film.decode(F0 / f"quarry-policy-p{pose}", pose)
        point = np.load(lb.REFERENCE / f"pose-{pose}-reference.npz")["ref"]
        pops = film.populations(d, point)
        hit = np.fromfile(lb.R / f"pose-{pose}-seed-0-hits.u32", dtype="<u4").reshape(*SHAPE, 8)
        prefix = hit[..., 0].copy().view("u1").reshape(*SHAPE, 4)
        blocked = np.unpackbits(prefix, axis=-1, bitorder="little")[..., :8].sum(-1)
        candidates = {"Exact": (lb.counts(pose, 0) / 256, None), "Interactive": ((8 - blocked.astype(float)) / 8, None)}
        native = np.load(lb.native_dir("vsm", pose) / "native.npz")
        candidates["Unreal-VSM"] = (native["visibility"].astype(float), native["valid"])
        for name, f in files.items():
            if (probe / f).exists():
                candidates[name] = (counts(probe / f), None)
        rows = {}
        for name, (v, valid) in candidates.items():
            valid = np.ones(SHAPE, bool) if valid is None else valid
            stat = film.statistics(v, d, pops, valid, domains=("all",))["all"]
            row = s1.scored(v, d, pops, valid)
            rows[name] = dict(compact(row), film_partial_rms=stat["partial"].get("rms"),
                              film_missed=stat["missed_shadows"], film_false=stat["false_shadows"])
        cache = [k for k in rows if k.startswith("N7a")]
        full = {k: s1.scored(candidates[k][0], d, pops, np.ones(SHAPE, bool)) for k in cache}
        vsm = s1.scored(*[candidates["Unreal-VSM"][0]], d, pops, candidates["Unreal-VSM"][1])
        inter = s1.scored(candidates["Interactive"][0], d, pops, np.ones(SHAPE, bool))
        out[f"pose{pose}"] = dict(rows=rows, rule={k: rule(full[k], vsm, inter) for k in cache})
        print("quarry pose", pose, json.dumps({k: [r["partial"], r["contact"], r["occluded"], r["missed_shadows"], r["false_shadows"], r["solid_flips"]] for k, r in rows.items()}), flush=True)
    return out


def bench(name, root, s1, motion):
    scene = json.loads((ARCHIVE / name / "fixture/scene.json").read_text())
    archived = {}
    for row in json.loads((ARCHIVE / name / "per-frame.json").read_text()):
        if row["mode"] in ("UE VSM", "RL Interactive", "RL Exact", "UE HRT1"):
            archived[(row["case"], row["mode"], row["frame"])] = row
    stats_root = root
    out = {}
    for directory in sorted(p for p in root.iterdir() if p.is_dir() and (p / "cache-stats.json").exists()):
        case = directory.name
        track = next(t for t in scene["tracks"] if case.startswith(t["name"] + "-r"))
        timing = json.loads((directory / "cache-stats.json").read_text())
        frames, broken, identity = [], [], []
        previous = {}
        for pose in track["frames"]:
            index = pose["frame"]
            d = motion.reference(directory / "reference", index, scene, pose)
            pops = s1.metrics.populations(d)
            shape = d["ref"].shape
            row = {"frame": index, "phase": pose["phase"]}
            vsm = archived.get((case, "UE VSM", index))
            inter = archived.get((case, "RL Interactive", index))
            sigma = float(np.sqrt(np.mean(d["sigma"][pops["partial"]] ** 2))) if pops["partial"].any() else None
            ref_row = vsm or inter
            if ref_row is not None:
                identity.append(dict(frame=index, partial_n=int(pops["partial"].sum()) == ref_row["partial_reference_n"],
                                     sigma=sigma is None and ref_row["reference_partial_sigma_rms"] is None or
                                     sigma is not None and ref_row["reference_partial_sigma_rms"] is not None and abs(sigma - ref_row["reference_partial_sigma_rms"]) < 1e-12))
            for label, sub in (("cache", "cache"), ("n0", "n0")):
                v = np.fromfile(directory / sub / f"{index:04}.f32", dtype="<f4").reshape(shape).astype(float)
                scored = s1.scored(v, d, pops, np.ones(shape, bool))
                if label in previous:
                    pv, pr = previous[label]
                    scored["residual_change_rms"] = float(np.sqrt(np.mean(((v - d["ref"]) - (pv - pr))[d["cover"]] ** 2)))
                previous[label] = (v, d["ref"])
                row[label] = scored
                if label == "cache" and vsm is not None and inter is not None:
                    b = rule(scored, vsm, inter)
                    if b:
                        broken.append(dict(frame=index, broken=b))
            row["vsm"] = vsm
            row["interactive_hw"] = inter
            frames.append(row)
            if index % 32 == 0:
                print(name, case, "frame", index, flush=True)
        out[case] = dict(summary=summarize(frames), rule_violations=broken, rule_frames_checked=sum(1 for f in frames if f["vsm"] and f["interactive_hw"]),
                         reference_identity=dict(frames=len(identity), partial_n_equal=sum(i["partial_n"] for i in identity), sigma_equal=sum(i["sigma"] for i in identity)),
                         timing=timing["track"], cold=timing["cold_first_warmup_frame"], validation=timing["validation"])
        (stats_root / f"{case}-frames.json").write_text(json.dumps([{k: (compact(v) if isinstance(v, dict) and "partial" in v else v) for k, v in f.items()} for f in frames], indent=1))
        print(name, case, json.dumps(out[case]["summary"]), "violations", len(broken), flush=True)
    return out


def summarize(frames):
    """Phase-pooled partial RMS (by pixel count), lost contact over pixel-frames, worst-frame
    leaks and flips, as docs/bench-motion.md tables them, per candidate."""
    out = {}
    for label in ("cache", "n0", "vsm", "interactive_hw"):
        rows = [f[label] for f in frames if f.get(label)]
        if not rows:
            continue
        n = sum(r["partial"].get("n", 0) for r in rows)
        sq = sum(r["partial"]["rms"] ** 2 * r["partial"]["n"] for r in rows if r["partial"].get("n"))
        out[label] = dict(frames=len(rows), partial_rms=float(np.sqrt(sq / n)) if n else None,
                          contact_lost=sum(r["contact_lost"] for r in rows), contact_n=sum(r["contact_n"] for r in rows),
                          max_leaks=max(r["leaks"] for r in rows), max_flips=max(r["solid_flips"] for r in rows),
                          missed=sum(r["missed_shadows"] for r in rows), false=sum(r["false_shadows"] for r in rows),
                          residual_change_rms=float(np.mean([r["residual_change_rms"] for r in rows if r.get("residual_change_rms") is not None])) if any(r.get("residual_change_rms") is not None for r in rows) else None)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--probe", type=Path)
    ap.add_argument("--bench", action="append", default=[], help="NAME=run directory, e.g. S2=E:/renderlab-work/n7a/runs/s2-1")
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    s1 = load("bench-contact-fold-score")
    result = {}
    if a.probe:
        result["quarry"] = quarry(a.probe, s1)
        (a.out / "quarry.json").write_text(json.dumps(result["quarry"], indent=1))
    motion = load("bench-motion-score")
    for spec in a.bench:
        name, root = spec.split("=", 1)
        result[name] = bench(name, Path(root), s1, motion)
        (a.out / f"{name}.json").write_text(json.dumps(result[name], indent=1))
    result["seconds"] = time.monotonic() - started
    (a.out / "n7a-scores.json").write_text(json.dumps(result, indent=1, default=float))
    print("done", round(result["seconds"], 1), "s", flush=True)


if __name__ == "__main__":
    main()
