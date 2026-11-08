"""M7-G2 quarry scorer (docs/m7-g2.md). CPU only.

  score RUN [--references DIR]  every capture under RUN/<path>[-tag]/ against U0b's full-radiance
                                references with ES1's metric (every registered population, RGB
                                RMS and bias in F0 codes at -14 and -10 stops, U0b's frozen
                                transform), plus two inspection populations on the static poses:
                                `crop` (Islam's inspection crop, pixels x 880-1120, y 480-640 of
                                pose 0, covered) and `hollow` (the hollow under the central slab
                                inside it, x 975-1030, y 555-595). Writes score.json per directory.
  gate RUN                      the default gate (milestone-7.md, 2026-11-08 ruling): per path, the
                                cache with bounce (g2) against the cache-off image (off), ES1's
                                unoccluded diagnostic (es1open, from PATH-open) and Lumen's Epic
                                floors, every population, both exposures, static (poses 0 and 1)
                                and in motion (the pan pooled over its witnesses). Prints the
                                tables and the populations each comparator wins; writes gate.json.
  settle RUN                    settle and sun-step curves: each captured frame against the
                                settled (or fresh) image and the reference, RMS in F0 codes.

Capture names: VARIANT-pP (quarry-pP.npz), VARIANT-pan-FFF (pan-FFF.npz), VARIANT-settle-NN,
VARIANT-sun-NN and VARIANT-sun-fresh.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re

import numpy as np

REPO = Path(__file__).resolve().parent.parent
REFERENCES = Path("E:/renderlab-work/m7-u0b/reference/quarry-final")
CROP = (880, 480, 1120, 640)
HOLLOW = (975, 555, 1030, 595)


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ES1 = load("m7-es1-score")


def reference_of(name):
    if (m := re.fullmatch(r".+-p(\d)", name)):
        return f"quarry-p{m.group(1)}"
    if (m := re.fullmatch(r".+-pan-(\d{3})", name)):
        return f"pan-{m.group(1)}"
    if re.fullmatch(r".+-(settle-\d\d|sun-\d\d|sun-fresh)", name):
        return "quarry-p0"
    return None


def rect(shape, box):
    m = np.zeros(shape[:2], bool)
    x0, y0, x1, y1 = box
    m[y0:y1, x0:x1] = True
    return m


def score(args):
    f0 = ES1.scorer()
    run = Path(args.run)
    for path_dir in sorted(p for p in run.iterdir() if p.is_dir() and re.fullmatch(r"(rt|nonrt)(-\w+)?", p.name)):
        groups = {}
        for c in sorted(path_dir.glob("*.f32")):
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
            if ref.startswith("quarry-"):
                covered = populations["all-covered"]
                populations["crop"] = covered & rect(covered.shape, CROP)
                populations["hollow"] = covered & rect(covered.shape, HOLLOW)
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
                ratio = (image @ ES1.LUM) / np.maximum(reference @ ES1.LUM, 1e-8)
                measures["ground"] = {str(k): dict(n=int((m := ground & (hit == k)).sum()), ratio=float(np.median(ratio[m])))
                                      for k in np.unique(hit[ground]) if (ground & (hit == k)).sum() > 1000}
                report[c.stem] = measures
                print(path_dir.name, c.stem, "all-covered", round(measures["-14"]["all-covered"]["rms"], 3),
                      "hollow" if "hollow" in measures["-14"] else "",
                      round(measures["-14"]["hollow"]["rms"], 3) if "hollow" in measures["-14"] else "", flush=True)
        (path_dir / "score.json").write_text(json.dumps(report, indent=2))


def gate(args):
    run = Path(args.run)
    floors = ES1.floors()
    motion = json.loads(ES1.FLOORS.read_text())["motion"] if ES1.FLOORS.exists() else {}
    cell = lambda a, b: f"{a:.2f}/{b:.2f}"
    result = {}
    for path in ("rt", "nonrt"):
        if not (run / path / "score.json").exists():
            continue
        report = json.loads((run / path / "score.json").read_text())
        open_report = {}
        if (run / f"{path}-open" / "score.json").exists():
            open_report = json.loads((run / f"{path}-open" / "score.json").read_text())
        merged = dict(report)
        merged.update({k.replace("es1open", "es1open"): v for k, v in open_report.items()})
        rows = {}
        for case in ("p0", "p1", "pan"):
            def value(variant, stops, pop):
                if case == "pan":
                    names = [n for n in merged if n.startswith(f"{variant}-pan-")]
                    if not names:
                        return None
                    v, n = ES1.pooled(merged, names, stops, pop)
                    return v if n else None
                m = merged.get(f"{variant}-{case}")
                return m[stops][pop]["rms"] if m and pop in m[stops] else None
            base = merged.get(f"g2-{case}") or merged.get(next((n for n in merged if n.startswith("g2-pan-")), ""), {})
            if not base:
                continue
            pops = sorted(base["-14"])
            table = []
            print(f"\n#### {path} {case}\n")
            head = ["Population", "Default (cache off)", "ES1 cache on", "ES1 open specular", "G2 (cache, bounce, ES1)",
                    "Lumen HW Epic", "Lumen SW Epic", "G2 loses to"]
            print("| " + " | ".join(head) + " |\n|" + "---|" * len(head))
            for pop in pops:
                vals = {v: [value(v, s, pop) for s in ("-14", "-10")] for v in ("off", "es1", "es1open", "g2")}
                lumen = {}
                for name in ("ue-hw-e", "ue-sw-e"):
                    if case == "p0":
                        m = floors.get(name, {})
                        lumen[name] = [m[k][pop]["rms"] if pop in m.get(k, {}) else None for k in ("0", "4")]
                    elif case == "pan":
                        m = motion.get(name, {}).get("error", {})
                        lumen[name] = [m[k][pop]["rms"] if pop in m.get(k, {}) else None for k in ("0", "4")]
                    else:
                        lumen[name] = [None, None]
                g = vals["g2"]
                losses = []
                for label, other in (("off", vals["off"]), ("es1open", vals["es1open"]),
                                     ("lumen-hw", lumen["ue-hw-e"]), ("lumen-sw", lumen["ue-sw-e"])):
                    if None in g or None in other:
                        continue
                    if g[0] > other[0] + 5e-4 or g[1] > other[1] + 5e-4:
                        losses.append(label)
                fmt = lambda v: cell(*v) if None not in v else ""
                print(f"| {pop} | {fmt(vals['off'])} | {fmt(vals['es1'])} | {fmt(vals['es1open'])} | "
                      f"**{fmt(g)}** | {fmt(lumen['ue-hw-e'])} | {fmt(lumen['ue-sw-e'])} | {', '.join(losses)} |")
                table.append(dict(population=pop, values=vals, lumen=lumen, losses=losses))
            rows[case] = table
        result[path] = rows
        for case, table in rows.items():
            for comparator in ("off", "es1open", "lumen-hw", "lumen-sw"):
                lost = [t["population"] for t in table if comparator in t["losses"]]
                print(f"{path} {case}: G2 loses to {comparator} on {len(lost)}: {', '.join(lost)}")
    (run / "gate.json").write_text(json.dumps(result, indent=2))


def settle(args):
    f0 = ES1.scorer()
    run = Path(args.run)
    archive = REFERENCES / "quarry-p0.npz"
    with np.load(archive) as a:
        reference = (a["L0"] + a["L1"]) / 2
        covered = a["pop_all-covered"] & ~a["pop_edge"]
    ref = f0.codes(reference, -14)
    for path_dir in sorted(p for p in run.iterdir() if p.is_dir()):
        rows = {}
        for kind, final in (("settle", "settle-12"), ("sun", "sun-fresh")):
            caps = sorted(path_dir.glob(f"*-{kind}-*.f32"))
            finals = [c for c in caps if c.stem.endswith(final)]
            if not finals:
                continue
            target = f0.codes(np.fromfile(finals[0], "<f4").reshape(reference.shape), -14)
            for c in caps:
                codes = f0.codes(np.fromfile(c, "<f4").reshape(reference.shape), -14)
                to_final = float(np.sqrt(((codes - target) ** 2).mean(-1)[covered].mean()))
                to_ref = float(np.sqrt(((codes - ref) ** 2).mean(-1)[covered].mean()))
                rows[c.stem] = dict(to_settled=to_final, to_reference=to_ref)
                print(path_dir.name, c.stem, f"to {final} {to_final:.3f}", f"to reference {to_ref:.3f}")
        if rows:
            (path_dir / "settle.json").write_text(json.dumps(rows, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("score")
    p.add_argument("run")
    p.add_argument("--references", type=Path, default=REFERENCES)
    p.set_defaults(run_command=score)
    p = sub.add_parser("gate")
    p.add_argument("run")
    p.set_defaults(run_command=gate)
    p = sub.add_parser("settle")
    p.add_argument("run")
    p.set_defaults(run_command=settle)
    args = parser.parse_args()
    args.run_command(args)


if __name__ == "__main__":
    main()
