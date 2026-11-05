"""Tables for docs/m6-motion-aa.md from the motion-AA simulation and real-step JSONs.

py -3 -B scripts/m6-motion-aa-report.py E:/renderlab-work/motion-aa
"""
import json
import sys
from pathlib import Path

POPS = ("all-covered", "partial", "lit", "occluded", "multi-surface", "edge", "background-centre")


def runs(root):
    out = {}
    for name in ("run-m13", "run-m2"):
        p = root / name / "motion-aa-sim.json"
        if p.exists():
            d = json.loads(p.read_text()); out.update(d["runs"]); bar = d["bar"]
    return out, bar


def fmt(v):
    return "—" if v is None else (f"{v:.2f}" if abs(v) >= 0.095 or v == 0 else f"{v:.3f}")


def main():
    root = Path(sys.argv[1])
    R, bar = runs(root)
    print("| Run | Frames | L0 all-covered | L0 partial | L0 edge | L4 multi-surface | D4 occluded | D4 lit | bg 1.5–3 px L0 (codes) | Verified solid R_F max | Verified solids > 0.002 | F4 missed + false | D4 > 16 | Aligned step L0 |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for key, r in R.items():
        fr = {int(k): v for k, v in r["frames"].items() if k != "0"}
        late = [t for t in fr if t >= 32] or list(fr)
        def worst(ch, pop):
            return max(fr[t]["ratio"][ch].get(pop, 0) for t in late)
        def mx(f):
            return max(f(fr[t]) for t in late)
        smax = mx(lambda x: max(x["F"]["solid-lit-v"]["max"], x["F"]["solid-occ-v"]["max"]))
        sover = mx(lambda x: x["F"]["solid-lit-v"]["over_002"] + x["F"]["solid-occ-v"]["over_002"])
        f4 = mx(lambda x: x["missed"] + x["false"]); d16 = mx(lambda x: x["D4_over16"])
        bg = mx(lambda x: x["L0"]["bg-d1.5-3"]["rms"])
        step = [fr[t].get("aligned_step_L0_covered") for t in late if fr[t].get("aligned_step_L0_covered") is not None]
        print(f"| {key} | {min(late)}–{max(late)} | {fmt(worst('L0', 'all-covered'))} | {fmt(worst('L0', 'partial'))} | {fmt(worst('L0', 'edge'))} | "
              f"{fmt(worst('L4', 'multi-surface'))} | {fmt(worst('D4', 'occluded'))} | {fmt(worst('D4', 'lit'))} | {fmt(bg)} | {fmt(smax)} | {sover} | {f4} | {d16} | {fmt(max(step) if step else None)} |")
    print(f"\nExact (bar): F4 {bar['missed']} + {bar['false']}, D4 > 16 {bar['D4_over16']}, bg 1.5–3 px L0 {bar['L0']['bg-d1.5-3']['rms']:.4f}")
    print("\nTransient from the start event (M1, aligned every 2 frames), worst population ratio over L0/L4/D0/D4:")
    for key, r in R.items():
        if not key.startswith("M1/"):
            continue
        cells = []
        for t in (2, 4, 6, 8):
            row = r["frames"].get(str(t))
            if row:
                ratio = row["ratio"]
                w = max(ratio[ch][p] for ch in ("L0", "L4", "D0", "D4") for p in ratio[ch] if p not in ("bg-d1.5-3", "bg-d3-6", "bg-d6+"))
                cells.append(f"age {t}: {w:.2f}")
        print(f"- {key}: " + ", ".join(cells))
    print("\nF3 bias (frames >= 32): populations where |mean| > |Exact mean| + 4 SE")
    for key, r in R.items():
        bad = set()
        for k, row in r["frames"].items():
            if k == "0" or int(k) < 32:
                continue
            for ch in ("L0", "L4", "D0", "D4"):
                for p in POPS:
                    if p in row[ch] and p in bar[ch]:
                        if abs(row[ch][p]["mean"]) > abs(bar[ch][p]["mean"]) + 4 * row[ch][p]["se"]:
                            bad.add(f"{ch}/{p}")
            for p in ("partial", "lit", "occluded", "solid-lit-v", "solid-occ-v"):
                if p in row["F"] and p in bar["F"] and abs(row["F"][p]["mean"]) > abs(bar["F"][p]["mean"]) + 4 * row["F"][p]["se"]:
                    bad.add(f"F/{p}")
        print(f"- {key}: {', '.join(sorted(bad)) if bad else 'none'}")
    step = root / "step" / "step.json"
    if step.exists():
        s = json.loads(step.read_text())
        print("\nReal 1 cm step (pose 0 samples to pose 1):")
        rows = [("pose-1 own 16 (static)", s["pose1_own16"], s["pose1_own16_ratio"])] + [(k, v, v["ratio"]) for k, v in s["rules"].items()] + \
               [("resampled bilinear", s["resampled_bilinear_step"], s["resampled_bilinear_step"]["ratio"])]
        print("| Estimate | L0 all-covered | L0 partial | L0 edge | L0 multi-surface | L0 background-centre | D4 occluded | Verified solid max | Verified solids > 0.002 | F4 | D4 > 16 |")
        print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for name, row, ratio in rows:
            F = row["F"]
            print(f"| {name} | {fmt(ratio['L0']['all-covered'])} | {fmt(ratio['L0']['partial'])} | {fmt(ratio['L0']['edge'])} | {fmt(ratio['L0']['multi-surface'])} | "
                  f"{fmt(ratio['L0'].get('background-centre'))} | {fmt(ratio['D4']['occluded'])} | {fmt(max(F['solid-lit-v']['max'], F['solid-occ-v']['max']))} | "
                  f"{F['solid-lit-v']['over_002'] + F['solid-occ-v']['over_002']} | {row['missed']} + {row['false']} | {row['D4_over16']} |")
        print(f"Exact pose 1: F4 {s['bar']['missed']} + {s['bar']['false']}, D4 > 16 {s['bar']['D4_over16']}; solid failures {s['rules']['plane4'].get('solid_failures')}")


if __name__ == "__main__":
    main()
