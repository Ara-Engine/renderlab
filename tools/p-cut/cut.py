"""P-CUT post-cut sequence evidence (CPU only, docs/p-cut.md).

Reads P-YARD2's protocol through tools/p-yard/score.py: Unreal's per-frame errors and timings
from P-YARD2's evidence (the Unreal package and manifest are unchanged), Renderlab's from this
unit's runs under P_YARD_WORK (run `score.py quality` and `score.py timing` there first).

    py -3 -B tools/p-cut/cut.py curves            # per-frame error/ms after the quarry cut: CSV + PNG
    py -3 -B tools/p-cut/cut.py strips X0,Y0,X1,Y1  # side-by-side frames and 60 fps APNGs (crop in px)
    py -3 -B tools/p-cut/cut.py game REFDIR RUNS  # ARA1 game: per-frame error after the cut (P-NSQ3 scorer)

Frame 320 is the cut (frames after the cut = index - 320); 321-351 hold the cut pose.
"""
import csv
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
YARD = HERE.parent / "p-yard"
WORK = Path(os.environ.get("P_YARD_WORK", "D:/Coding/renderlab-work/p-cut"))
PYARD2 = Path("D:/Coding/renderlab-work/p-yard2")
CUT = 320
LAST = 351
# Renderlab rows of this unit and the Unreal rows they are compared with, per path.
PATHS = {
    "RT": dict(rl=["rl-rt", "rl-rt-base"], ue=["ue-hw-e"], others=["ue-hw-h", "ue-hw-c"]),
    "non-RT": dict(rl=["rl-nonrt", "rl-nonrt-base"], ue=["ue-sw-e"], others=["ue-sw-h", "ue-sw-c"]),
}
NAMES = {"rl-rt": "Renderlab RT, cut ramp", "rl-rt-base": "Renderlab RT, cut in one frame",
         "rl-nonrt": "Renderlab non-RT, cut ramp", "rl-nonrt-base": "Renderlab non-RT, cut in one frame",
         "ue-hw-e": "UE HW Lumen Epic", "ue-sw-e": "UE SW Lumen Epic", "ue-hw-h": "UE HW High",
         "ue-hw-c": "UE HW Cinematic", "ue-sw-h": "UE SW High", "ue-sw-c": "UE SW Cinematic"}


def score_module(work):
    os.environ["P_YARD_WORK"] = str(work)
    spec = importlib.util.spec_from_file_location(f"pyard_score_{work.name}", YARD / "score.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def per_frame(work):
    """{config: {index: (L0 rms, L4 rms, edge rms)}} and {config: {index: [ms per visit]}}."""
    q = json.loads((work / "quality.json").read_text())["frames"]
    t = json.loads((work / "timing.json").read_text())["visits"]
    err = {}
    for config, rows in q.items():
        err[config] = {r["index"]: tuple(r["measures"][o][p]["ms"] ** .5 for o, p in
                                         (("0", "all-covered"), ("4", "all-covered"), ("0", "edge")))
                       for r in rows if CUT - 1 <= r["index"] <= LAST}
    ms = {}
    for config, visits in t.items():
        ms[config] = {i: [v["per_frame"][str(i)] for v in visits if str(i) in v["per_frame"]]
                      for i in range(CUT - 4, LAST + 1)}
    return err, ms


def curves(_):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    ue_err, ue_ms = per_frame(PYARD2)
    rl_err, rl_ms = per_frame(WORK)
    err = {**{k: v for k, v in ue_err.items() if k.startswith("ue-")}, **rl_err}
    ms = {**{k: v for k, v in ue_ms.items() if k.startswith("ue-")}, **rl_ms}
    rows = []
    for config in sorted(set(err) | set(ms)):
        for i in range(CUT - 4, LAST + 1):
            e = err.get(config, {}).get(i)
            m = ms.get(config, {}).get(i, [])
            if e is None and not m:
                continue
            rows.append(dict(config=config, index=i, after_cut=i - CUT,
                             error_L0=round(e[0], 4) if e else "", error_L4=round(e[1], 4) if e else "",
                             edge_L0=round(e[2], 4) if e else "",
                             gpu_ms_median=round(float(np.median(m)), 4) if m else "",
                             gpu_ms_visits=" ".join(f"{x:.3f}" for x in m)))
    with (WORK / "post-cut.csv").open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    fig, axes = plt.subplots(2, 2, figsize=(13, 8), constrained_layout=True)
    for col, (path, sets) in enumerate(PATHS.items()):
        ax_e, ax_t = axes[0][col], axes[1][col]
        for config in sets["others"]:
            xs = sorted(i for i in err.get(config, {}) if i >= CUT)
            ax_e.plot([i - CUT for i in xs], [err[config][i][0] for i in xs], color="#bbbbbb", lw=1,
                      label=NAMES[config])
        for config, color, style in zip(sets["ue"] + sets["rl"], ["#d95f02", "#1b9e77", "#7570b3"], ["-", "-", "--"]):
            xs = sorted(i for i in err.get(config, {}) if i >= CUT)
            ax_e.plot([i - CUT for i in xs], [err[config][i][0] for i in xs], style, color=color, lw=2,
                      marker="o", ms=3, label=NAMES[config])
            ts = sorted(i for i in ms.get(config, {}) if ms[config][i])
            ax_t.plot([i - CUT for i in ts], [np.median(ms[config][i]) for i in ts], style, color=color, lw=2,
                      marker="o", ms=3, label=NAMES[config])
        ax_e.set(title=f"{path}: full-frame error after the cut (lower is closer to the reference)",
                 xlabel="frames after the cut (0 = cut frame)", ylabel="RMS, F0 codes at -14 EV")
        ax_t.set(title=f"{path}: GPU frame time", xlabel="frames after the cut (negative: before)",
                 ylabel="GPU ms (median of visits)")
        for ax in (ax_e, ax_t):
            ax.grid(alpha=.3)
            ax.legend(fontsize=8)
    fig.savefig(WORK / "post-cut.png", dpi=110)
    summary = {}
    for config in err:
        e = err[config]
        m = ms.get(config, {})
        med = lambda xs: float(np.median(xs)) if xs else None
        summary[config] = dict(
            cut_error=e.get(CUT, (None,))[0],
            error_1_7=med([e[i][0] for i in range(CUT + 1, CUT + 8) if i in e]),
            error_8_31=med([e[i][0] for i in range(CUT + 8, LAST + 1) if i in e]),
            cut_ms=med(m.get(CUT, [])), ms_1_7=med([x for i in range(CUT + 1, CUT + 8) for x in m.get(i, [])]),
            ms_8_31=med([x for i in range(CUT + 8, LAST + 1) for x in m.get(i, [])]),
            pre_cut_ms=med([x for i in range(CUT - 4, CUT) for x in m.get(i, [])]))
    (WORK / "post-cut-summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary, indent=1))


def strips(args):
    s = score_module(WORK)
    stops = s.MANIFEST["scenes"]["quarry"]["exposure_stops"]
    x0, y0, x1, y1 = [int(v) for v in args[0].split(",")]
    found = s.candidates()
    out = WORK / "strips"
    out.mkdir(exist_ok=True)
    picks = [CUT - 1, CUT, CUT + 1, CUT + 2, CUT + 3, CUT + 4, CUT + 6, CUT + 8, CUT + 12, CUT + 16, LAST]
    ref = s.reference(s.MANIFEST["scenes"]["quarry"]["frames"][CUT]["reference"])["display"][0][0]
    ref8 = np.clip(np.rint(ref), 0, 255).astype(np.uint8)
    for path, sets in PATHS.items():
        configs = [c for c in sets["rl"] + sets["ue"] if c in found]
        tile = []
        for config in configs:
            row = []
            for i in picks:
                img = found[config][i]() if i in found[config] else None
                crop = (np.clip(np.rint(s.codes(img, stops)), 0, 255).astype(np.uint8)[y0:y1, x0:x1]
                        if img is not None else np.zeros((y1 - y0, x1 - x0, 3), np.uint8))
                row.append(crop)
            row.append(ref8[y0:y1, x0:x1])
            tile.append(np.concatenate(row, 1))
        sheet = Image.fromarray(np.concatenate(tile, 0))
        d = ImageDraw.Draw(sheet)
        for r, config in enumerate(configs):
            d.text((4, r * (y1 - y0) + 4), NAMES[config], fill=(255, 255, 0))
        for c, i in enumerate(picks + ["ref"]):
            d.text((c * (x1 - x0) + 4, 18), f"+{i - CUT}" if i != "ref" else "reference", fill=(0, 255, 255))
        name = path.lower().replace("-", "")
        sheet.save(out / f"strip-{name}.png")
        # Playback speed: frames 316..351 at 60 fps, side by side (Renderlab ramp | one-frame | Unreal).
        frames = []
        for i in range(CUT - 4, LAST + 1):
            parts = []
            for config in configs:
                img = found[config].get(i)
                parts.append(np.clip(np.rint(s.codes(img(), stops)), 0, 255).astype(np.uint8)[y0:y1, x0:x1]
                             if img else np.zeros((y1 - y0, x1 - x0, 3), np.uint8))
            frames.append(Image.fromarray(np.concatenate(parts, 1)))
        frames[0].save(out / f"playback-{name}.png", save_all=True, append_images=frames[1:],
                       duration=1000 / 60, loop=0, format="PNG")
        print("wrote", out / f"strip-{name}.png", "rows", configs, flush=True)


def game(args):
    ref, runs = Path(args[0]), Path(args[1])
    scorer = WORK / "tools" / "score_nsq3.py"
    out = WORK / f"game-{runs.name}.json"
    rc = subprocess.run([sys.executable, "-I", str(scorer), str(ref), str(runs), str(out), "--rt", "none"]).returncode
    if rc:
        raise SystemExit(rc)
    print(out.read_text()[:4000])


if __name__ == "__main__":
    {"curves": curves, "strips": strips, "game": game}[sys.argv[1]](sys.argv[2:])
