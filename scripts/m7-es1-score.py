"""M7-ES1 quarry scorer (docs/m7-es1.md). CPU only.

  score RUN [--references DIR]   every capture under RUN/<path>/ against U0b's full-radiance
                                 references: every registered population, RGB RMS and bias in
                                 F0 codes at -14 and -10 stops (U0b's frozen transform, as
                                 MAT1 and G1c), and open-ground luminance ratios per placement.
                                 Writes RUN/<path>/score.json.
  table RUN                      markdown tables of a scored run, with U0b's Lumen Epic static
                                 floors beside pose 0.
  doc RUN                        the tables docs/m7-es1.md quotes (default, G1c, ES1, Lumen;
                                 the pooled pan; the open-specular diagnostic in RUN/PATH-open).

Capture names: PROGRAM-CACHE-pPOSE (quarry-pPOSE.npz) and PROGRAM-pan-FRAME (pan-FRAME.npz).
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re

import numpy as np

REPO = Path(__file__).resolve().parent.parent
REFERENCES = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
FLOORS = Path("E:/renderlab-work/m7-u0b/physical-floor-summary.json")
LUM = np.array([0.2126, 0.7152, 0.0722])


def scorer():
    spec = importlib.util.spec_from_file_location("es1_f0", REPO / "scripts/m7-u0b-score.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def reference_of(name):
    if (m := re.fullmatch(r".+-(?:on|off)-p(\d)", name)):
        return f"quarry-p{m.group(1)}"
    if (m := re.fullmatch(r".+-pan-(\d{3})", name)):
        return f"pan-{m.group(1)}"
    return None


def score(args):
    f0 = scorer()
    run = Path(args.run)
    for path_dir in sorted(p for p in run.iterdir() if p.is_dir() and p.name in ("rt", "nonrt", "rt-open", "nonrt-open")):
        captures = sorted(path_dir.glob("*.f32"))
        groups = {}
        for c in captures:
            if (ref := reference_of(c.stem)):
                groups.setdefault(ref, []).append(c)
        report = {}
        for ref, files in groups.items():
            archive = args.references / f"{ref}.npz"
            meta = json.loads(archive.with_suffix(".json").read_text())
            if meta["archive_sha256"] != f0.sha(archive) or meta["camera_background"] != [0, 0, 0]:
                raise SystemExit(f"reference identity/background mismatch: {archive}")
            with np.load(archive) as a:
                reference = (a["L0"] + a["L1"]) / 2
                populations = {k[4:]: a[k] for k in a.files if k.startswith("pop_") and a[k].any()}
                hit = a["hit"][..., 0]
                ground = (a["pop_all-covered"] & ~a["pop_edge"] & a["pop_lit"]
                          & (a["geometric_normal"][..., 1] > 0.97))
            ref_codes = {s: f0.codes(reference, s) for s in (-14, -10)}
            for c in files:
                image = np.fromfile(c, "<f4").reshape(reference.shape)
                measures = {"reference": ref, "reference_sha256": meta["archive_sha256"]}
                for stops in (-14, -10):
                    error = f0.codes(image, stops) - ref_codes[stops]
                    squared = (error * error).mean(-1)
                    bias = error.mean(-1)
                    measures[str(stops)] = {p: dict(n=int(m.sum()), rms=float(np.sqrt(squared[m].mean())),
                                                    bias=float(bias[m].mean())) for p, m in populations.items()}
                ratio = (image @ LUM) / np.maximum(reference @ LUM, 1e-8)
                measures["ground"] = {str(k): dict(n=int((m := ground & (hit == k)).sum()), ratio=float(np.median(ratio[m])))
                                      for k in np.unique(hit[ground]) if (ground & (hit == k)).sum() > 1000}
                report[c.stem] = measures
                print(path_dir.name, c.stem, "all-covered", round(measures["-14"]["all-covered"]["rms"], 3), flush=True)
        (path_dir / "score.json").write_text(json.dumps(report, indent=2))


def floors():
    if not FLOORS.exists():
        return {}
    d = json.loads(FLOORS.read_text())["static"]
    return {name: d[name]["measures"] for name in ("ue-hw-e", "ue-sw-e") if name in d}


def table(args):
    run = Path(args.run)
    lumen = floors()
    for path_dir in sorted(p for p in run.iterdir() if p.is_dir() and (p / "score.json").exists()):
        report = json.loads((path_dir / "score.json").read_text())
        names = sorted(report)
        for key in sorted({reference_of(n) for n in names}):
            cols = [n for n in names if reference_of(n) == key]
            pops = sorted(report[cols[0]]["-14"])
            head = ["Population", "Pixels"] + cols
            if key == "quarry-p0":
                head += ["Lumen HW Epic", "Lumen SW Epic"]
            print(f"\n### {path_dir.name} {key}\n")
            print("| " + " | ".join(head) + " |")
            print("|" + "---|" * len(head))
            for p in pops:
                cells = [p, f"{report[cols[0]]['-14'][p]['n']:,}"]
                for c in cols:
                    cells.append(f"{report[c]['-14'][p]['rms']:.3f}/{report[c]['-10'][p]['rms']:.3f}")
                if key == "quarry-p0":
                    for name in ("ue-hw-e", "ue-sw-e"):
                        m = lumen.get(name, {})
                        cells.append(f"{m['0'][p]['rms']:.3f}/{m['4'][p]['rms']:.3f}" if p in m.get("0", {}) else "")
                print("| " + " | ".join(cells) + " |")


def pooled(report, names, stops, pop):
    """RMS pooled over captures (pixel-weighted), and their pixel total."""
    rows = [report[n][stops][pop] for n in names if pop in report[n][stops]]
    n = sum(r["n"] for r in rows)
    return (np.sqrt(sum(r["n"] * r["rms"] ** 2 for r in rows) / n) if n else float("nan")), n


def doc(args):
    """The tables docs/m7-es1.md quotes: per path and pose, every population for the
    unchanged default (cache off), G1c's cache-on image, ES1's, and Lumen Epic; the moving
    pan pooled over its four witnesses; and the open-specular diagnostic when present."""
    run = Path(args.run)
    lumen = floors()
    motion = json.loads(FLOORS.read_text())["motion"] if FLOORS.exists() else {}
    cell = lambda a, b: f"{a:.2f}/{b:.2f}"
    for path in ("rt", "nonrt"):
        report = json.loads((run / path / "score.json").read_text())
        for pose in (0, 1):
            cols = [("Default (cache off)", f"es1-off-p{pose}"), ("G1c cache on", f"baseline-on-p{pose}"),
                    ("ES1 cache on", f"es1-on-p{pose}")]
            head = ["Population", "Pixels"] + [c for c, _ in cols] + (["Lumen HW Epic", "Lumen SW Epic"] if pose == 0 else [])
            print(f"\n#### {path} pose {pose}\n")
            print("| " + " | ".join(head) + " |\n|" + "---|" * len(head))
            for pop in sorted(report[cols[0][1]]["-14"]):
                row = [pop, f"{report[cols[0][1]]['-14'][pop]['n']:,}"]
                values = [report[k]["-14"][pop]["rms"] for _, k in cols]
                for (_, k), v in zip(cols, values):
                    text = cell(v, report[k]["-10"][pop]["rms"])
                    row.append(f"**{text}**" if k.startswith("es1-on") and (
                        v > values[1] + 5e-4 or report[k]["-10"][pop]["rms"] > report[cols[1][1]]["-10"][pop]["rms"] + 5e-4) else text)
                if pose == 0:
                    for name in ("ue-hw-e", "ue-sw-e"):
                        m = lumen.get(name, {})
                        row.append(cell(m["0"][pop]["rms"], m["4"][pop]["rms"]) if pop in m.get("0", {}) else "")
                print("| " + " | ".join(row) + " |")
        pan = sorted({n.rsplit("-", 1)[1] for n in report if "-pan-" in n})
        if pan:
            print(f"\n#### {path} moving pan, pooled over frames {', '.join(pan)}\n")
            head = ["Population", "Pixels", "G1c cache on", "ES1 cache on", "Lumen HW Epic Q-walk", "Lumen SW Epic Q-walk"]
            print("| " + " | ".join(head) + " |\n|" + "---|" * len(head))
            for pop in sorted(report[f"es1-pan-{pan[0]}"]["-14"]):
                b14, n = pooled(report, [f"baseline-pan-{f}" for f in pan], "-14", pop)
                b10, _ = pooled(report, [f"baseline-pan-{f}" for f in pan], "-10", pop)
                e14, _ = pooled(report, [f"es1-pan-{f}" for f in pan], "-14", pop)
                e10, _ = pooled(report, [f"es1-pan-{f}" for f in pan], "-10", pop)
                worse = e14 > b14 + 5e-4 or e10 > b10 + 5e-4
                row = [pop, f"{n:,}", cell(b14, b10), (f"**{cell(e14, e10)}**" if worse else cell(e14, e10))]
                for name in ("ue-hw-e", "ue-sw-e"):
                    m = motion.get(name, {}).get("error", {})
                    row.append(cell(m["0"][pop]["rms"], m["4"][pop]["rms"]) if pop in m.get("0", {}) else "")
                print("| " + " | ".join(row) + " |")
        open_dir = run / f"{path}-open"
        if (open_dir / "score.json").exists():
            other = json.loads((open_dir / "score.json").read_text())
            print(f"\n#### {path} diagnostic: open sky specular with the cache on (pose 0)\n")
            print("| Population | Default (cache off) | ES1 cache on | Open specular, cache on |\n|---|---|---|---|")
            for pop in sorted(report["es1-on-p0"]["-14"]):
                print(f"| {pop} | {cell(report['es1-off-p0']['-14'][pop]['rms'], report['es1-off-p0']['-10'][pop]['rms'])} | "
                      f"{cell(report['es1-on-p0']['-14'][pop]['rms'], report['es1-on-p0']['-10'][pop]['rms'])} | "
                      f"{cell(other['es1-on-p0']['-14'][pop]['rms'], other['es1-on-p0']['-10'][pop]['rms'])} |")
        for pose in (0, 1):
            print(f"\n{path} pose {pose} open-ground luminance ratio per placement, default / G1c on / ES1 on: " + ", ".join(
                f"{k} {report[f'es1-off-p{pose}']['ground'][k]['ratio']:.3f}/{report[f'baseline-on-p{pose}']['ground'][k]['ratio']:.3f}/"
                f"{report[f'es1-on-p{pose}']['ground'][k]['ratio']:.3f}" for k in report[f"es1-on-p{pose}"]["ground"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("score")
    p.add_argument("run")
    p.add_argument("--references", type=Path, default=REFERENCES)
    p.set_defaults(run_command=score)
    p = sub.add_parser("table")
    p.add_argument("run")
    p.set_defaults(run_command=table)
    p = sub.add_parser("doc")
    p.add_argument("run")
    p.set_defaults(run_command=doc)
    args = parser.parse_args()
    args.run_command(args)


if __name__ == "__main__":
    main()
