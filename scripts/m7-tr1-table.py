"""M7-TR1: headline per-measure table of one or more scored candidates against the bar's
TSR Epic and recorded T1b rows of the same case (TR0's own headline rows, docs/m7-tr0.md).

  py -3 -B scripts/m7-tr1-table.py --case w-e-r-c1 name=score.json [...] [--markdown]
"""
import argparse
import csv
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("score", Path(__file__).with_name("m7-tr1-score.py"))
score_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score_module)

HEADLINE = [
    ("All-covered, frame 64", "64", "{c}", "all-covered/age32", "rms_noise_subtracted"),
    ("Partial, frame 64", "64", "{c}", "partial/age32", "rms_noise_subtracted"),
    ("Occluded, frame 64", "64", "{c}", "occluded/age32", "rms_noise_subtracted"),
    ("Edge, frame 64", "64", "{c}", "edge/age32", "rms_noise_subtracted"),
    ("Background-centre, frame 64", "64", "{c}", "background-centre/age32", "rms_noise_subtracted"),
    ("Hold frame 96, all-covered", "96", "{c}", "all-covered/age32", "rms_noise_subtracted"),
    ("Return 120, all-covered", "120", "{c}", "all-covered/age32", "rms_noise_subtracted"),
    ("Return 128, all-covered", "128", "{c}", "all-covered/age32", "rms_noise_subtracted"),
    ("Frame 160, edge", "160", "{c}", "edge/age32", "rms_noise_subtracted"),
    ("Hold 80->81 step", "81", "{c}", "covered/same-instance/age8", "frame_step_rms"),
    ("Return 127->128 flicker", "128", "{c}", "covered/same-instance/age8", "reference_subtracted_flicker_rms"),
    ("Frame 64 background 1.5-3 px", "64", "{c}", "background-distance/1.5-3", "rms"),
    ("Frame 64 background 3-6 px", "64", "{c}", "background-distance/3-6", "rms"),
    ("Revealed age 3, all-covered", "pooled-age-3", "{c}", "all-covered/revealed", "rms_noise_subtracted"),
    ("Revealed age 7, all-covered", "pooled-age-7", "{c}", "all-covered/revealed", "rms_noise_subtracted"),
    ("Revealed age 7, background-centre", "pooled-age-7", "{c}", "background-centre/revealed", "rms_noise_subtracted"),
    ("Start age 0, all-covered", "pooled-age-0", "{c}", "all-covered/start", "rms_noise_subtracted"),
    ("Start age 3, all-covered", "pooled-age-3", "{c}", "all-covered/start", "rms_noise_subtracted"),
    ("Start age 7, all-covered", "pooled-age-7", "{c}", "all-covered/start", "rms_noise_subtracted"),
    ("+8 changed mask, frame 72", "72", "{c}", "changed/8-frame-input", "rms_noise_subtracted"),
    ("+8 changed |mean|, frame 72", "72", "{c}", "changed/8-frame-input", "absolute_signed_mean"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--case", required=True)
    parser.add_argument("--channels", nargs="+", default=["L0", "L4"])
    parser.add_argument("--markdown", action="store_true")
    parser.add_argument("candidates", nargs="+")
    args = parser.parse_args()
    bar = {"tsr": {}, "t1b-black": {}}
    with (score_module.TR0 / "bar/physical-measures.csv").open(newline="", encoding="utf-8") as stream:
        for r in csv.DictReader(stream):
            if r["label"] == args.case and r["method"] in bar:
                bar[r["method"]][(r["frame"], r["channel"], r["population"], r["measure"])] = float(r["value"])
    names, rows = [], []
    for entry in args.candidates:
        name, path = entry.split("=", 1)
        names.append(name)
        rows.append({k: v[0] for k, v in score_module.rows_of(json.loads(Path(path).read_text())).items()})
    header = ["Measure", "Ch", "TSR Epic", "T1b"] + names
    lines = []
    for label, frame, _, population, measure in HEADLINE:
        for c in args.channels:
            key = (frame, c, population, measure)
            cells = [bar["tsr"].get(key), bar["t1b-black"].get(key)] + [r.get(key) for r in rows]
            tsr = cells[0]
            text = []
            for i, v in enumerate(cells):
                if v is None:
                    text.append("—")
                else:
                    mark = "**" if i >= 2 and tsr is not None and v > tsr else ""
                    text.append(f"{mark}{v:.4g}{mark}")
            lines.append([label, c] + text)
    if args.markdown:
        print("| " + " | ".join(header) + " |")
        print("|" + "---|" * 2 + "---:|" * (len(header) - 2))
        for l in lines:
            print("| " + " | ".join(l) + " |")
    else:
        widths = [max(len(str(x)) for x in col) for col in zip(header, *lines)]
        for l in [header] + lines:
            print("  ".join(str(x).ljust(w) for x, w in zip(l, widths)))


if __name__ == "__main__":
    main()
