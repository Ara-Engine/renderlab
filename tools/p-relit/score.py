"""P-RELIT scores (CPU; docs/p-relit.md) over runs of tools/p-relit/plan.py. Masks come from the
courtyard's analytic geometry (scripts/p-flat-score.py), never from colours.

    py -3 -B tools/p-relit/score.py flights OUT.json CONTROL RUN...
    py -3 -B tools/p-relit/score.py hold OUT.json CONTROL RUN...
    py -3 -B tools/p-relit/score.py case OUT.json CONTROL RUN...
    py -3 -B tools/p-relit/score.py same RUN_A RUN_B

flights  P-FLAT's flicker slope per surface against the no-history control (frames 8-47; 1
         follows the flicker), and its speckle per surface (frames 8, 16, 24, 32, 40, 47).
hold     camera held, scene animating: the frame-to-frame step of each static surface in display
         codes (RMS over pixels and frames 16-47), inside and outside the fire's restart sphere,
         and its slope against the control's step.
case     a scene event at frame 24: per frame 20-39, the run's signed mean and RMS difference from
         the control (codes) on the fire-lit surfaces, on the far ground the fire still reaches
         (stale light would sit there) and over the frame, and the pixels over 8 codes brighter.
same     whether two runs wrote the same image hashes, frame by frame.
"""
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("motion", REPO / "scripts/p-flat-motion.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
FIRE = np.array([-3.8, 0.95, 2.0])
# The restart sphere at the flicker's full swing (0.3 x 24,000 cd at 2^-11, budget 0.05).
ZONE = 6.7
EVENT = 24


def surfaces(run, entry):
    """Static-surface masks of one frame: P-FLAT's two fire-lit ones and the rest by distance."""
    inside, label, points = m.geometry(run, entry)
    near = np.linalg.norm((points - m.BRAZIER)[..., [0, 2]], axis=-1)
    reach = np.linalg.norm(points - FIRE, axis=-1)
    return dict(
        wall=inside & (label == 3) & (points[..., 2] > 0.6) & (points[..., 2] < 3.0),
        ground_near=inside & (label == 1) & (near < 2.5),
        ground_zone=inside & (label == 1) & (near >= 2.5) & (reach < ZONE),
        ground_far=inside & (label == 1) & (reach >= ZONE),
        ground_fringe=inside & (label == 1) & (reach >= ZONE) & (reach < 10.0),
        arch=inside & (label == 2),
        crate_top=inside & (label == 4),
        crate_side=inside & (label == 5),
    )


def codes(path):
    return m.score.luma(m.score.load(path))


def flights(control, runs):
    crun, cframes = m.frames(control)
    masks = {k: surfaces(crun, cframes[k]) for k in sorted(cframes)}

    def series(run):
        _, fr = m.frames(run)
        out = {c: [] for c in masks[0]}
        for k in sorted(fr):
            img = m.linear(m.score.load(fr[k]["png"]))
            for c in out:
                out[c].append(float(img[masks[k][c]].mean()) if masks[k][c].sum() >= 200 else np.nan)
        return {c: np.array(v) for c, v in out.items()}

    ref = series(control)
    result = {}
    for run in runs:
        m.score.FRAMES = None
        s = series(run)
        m.score.FRAMES = {8, 16, 24, 32, 40, 47}
        row = {}
        for c in s:
            dc, dv = np.diff(ref[c])[7:], np.diff(s[c])[7:]
            ok = np.isfinite(dc) & np.isfinite(dv)
            row[c] = dict(slope=float(np.dot(dv[ok], dc[ok]) / np.dot(dc[ok], dc[ok])) if ok.sum() > 8 else None,
                          bias=float(np.nanmean((s[c][8:] - ref[c][8:]) / ref[c][8:])))
        speckle = {}
        for r in m.score.score(Path(run)):
            if r["frame"] >= 8:
                speckle.setdefault(r["cls"], []).append(r["speckle"])
        result[Path(run).name] = dict(flicker=row, speckle={c: float(np.mean(v)) for c, v in speckle.items()})
        print(Path(run).name, "slope", " ".join(f"{c} {v['slope']:.3f}" for c, v in row.items() if v["slope"] is not None))
        print(" " * len(Path(run).name), "speckle", " ".join(f"{c} {v:.3f}" for c, v in result[Path(run).name]["speckle"].items()), flush=True)
    return result


def hold(control, runs):
    crun, cframes = m.frames(control)
    keys = [k for k in sorted(cframes) if k >= 16]
    # Static pixels: the same surface in every scored frame (the hero and the spinner move).
    masks = None
    for k in keys[::4] + keys[-1:]:
        s = surfaces(crun, cframes[k])
        masks = s if masks is None else {c: masks[c] & s[c] for c in s}
    groups = dict(inside=masks["wall"] | masks["ground_near"] | masks["ground_zone"],
                  outside=masks["ground_far"] | masks["arch"] | masks["crate_top"] | masks["crate_side"], **masks)

    def steps(run):
        _, fr = m.frames(run)
        images = np.stack([codes(fr[k]["png"]) for k in keys])
        return np.diff(images, axis=0)

    ref = steps(control)
    result = {}
    for run in [control, *runs]:
        d = steps(run)
        row = {}
        for c, mask in groups.items():
            if mask.sum() < 200:
                continue
            mean_step, ref_step = d[:, mask].mean(axis=1), ref[:, mask].mean(axis=1)
            row[c] = dict(step_rms=float(np.sqrt((d[:, mask] ** 2).mean())), pixels=int(mask.sum()),
                          # The surface's own brightness change against the control's: the flicker it shows.
                          slope=float(np.dot(mean_step, ref_step) / np.dot(ref_step, ref_step)),
                          mean_step_rms=float(np.sqrt((mean_step ** 2).mean())))
        result[Path(run).name] = row
        print(Path(run).name, " ".join(f"{c} {v['step_rms']:.3f}" for c, v in row.items()), flush=True)
    return result


def case(control, runs):
    crun, cframes = m.frames(control)
    keys = [k for k in sorted(cframes) if k >= EVENT - 4]
    result = {}
    for run in runs:
        _, fr = m.frames(run)
        rows = []
        for k in keys:
            s = surfaces(crun, cframes[k])
            lit = s["wall"] | s["ground_near"] | s["ground_zone"]
            d = codes(fr[k]["png"]) - codes(cframes[k]["png"])
            row = dict(frame=k, frame_rms=float(np.sqrt((d ** 2).mean())), frame_mean=float(d.mean()))
            for name, mask in (("lit", lit), ("fringe", s["ground_fringe"]), ("far", s["ground_far"] | s["arch"])):
                if mask.sum() >= 200:
                    row[name] = dict(mean=float(d[mask].mean()), rms=float(np.sqrt((d[mask] ** 2).mean())),
                                     brighter=float((d[mask] > 8).mean()), darker=float((d[mask] < -8).mean()))
            rows.append(row)
        result[Path(run).name] = rows
        after = [r for r in rows if EVENT <= r["frame"] < EVENT + 8]
        before = [r for r in rows if r["frame"] < EVENT]
        show = lambda rs, name: " ".join(f"{r[name]['mean']:+.2f}" for r in rs if name in r)
        print(Path(run).name)
        for name in ("lit", "fringe", "far"):
            print(f"  {name:7s} mean vs control, frames {before[0]['frame']}-{before[-1]['frame']}: {show(before, name)} | "
                  f"{EVENT}-{after[-1]['frame']}: {show(after, name)}")
        print("  frame rms:", " ".join(f"{r['frame_rms']:.2f}" for r in rows[:16]), flush=True)
    return result


def same(a, b):
    ha, hb = ({f["frame"]: f["rgba8_sha256"] for f in json.loads((Path(r) / "run.json").read_text(encoding="utf-8"))["frames"]
               if f["rgba8_sha256"]} for r in (a, b))
    equal = sum(ha[k] == hb.get(k) for k in ha)
    print(f"{Path(a).name} vs {Path(b).name}: {equal}/{len(ha)} frames identical")
    return equal == len(ha)


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "same":
        raise SystemExit(0 if same(*sys.argv[2:4]) else 1)
    out, control, runs = Path(sys.argv[2]), sys.argv[3], sys.argv[4:]
    result = dict(flights=flights, hold=hold, case=case)[mode](control, runs)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1), encoding="utf-8")
