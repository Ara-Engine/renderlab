"""CPU-only: write an A/B plan for tools/p-nrt/plan.py.

    py -3 -B tools/p-nrt/abplan.py TAG --bin base=EXE --bin cand=EXE [--reps 2] [--build cand=TREE]
        [--tails] [--extra-json STEPS.json] > PLAN.json

Order: optional builds, optional extra steps, then per binary the three ARA1 quality
scenes (byte-identity witnesses; they also warm that binary's pipeline cache), then the
1080p ARA1 timing scenes interleaved A,B per case and repetition, then optional
ARA1-sequence-plus-1000-still-frame tails for the first binary. Each binary's outputs
share one parent, so its program cache warms once.
"""
import argparse
import json
from pathlib import Path

ROOT = Path("D:/Coding/renderlab-work/p-nrt")
ARA1 = Path("D:/Coding/renderlab-work/ara1/phase2-scenes")
CASES = ("game-base", "ara-pbr-oriented", "quarry-daylight")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("tag")
    p.add_argument("--bin", action="append", required=True)
    p.add_argument("--build", action="append", default=[])
    p.add_argument("--reps", type=int, default=2)
    p.add_argument("--cases", default=",".join(CASES))
    p.add_argument("--tails", action="store_true")
    p.add_argument("--no-quality", action="store_true")
    p.add_argument("--extra-json")
    a = p.parse_args()
    bins = dict(b.split("=", 1) for b in a.bin)
    cases = a.cases.split(",")
    runs = ROOT / "runs" / a.tag
    steps = []
    for item in a.build:
        name, tree = item.split("=", 1)
        steps.append(dict(kind="build", tree=tree, target=str(ROOT / f"target-{name}"), out=str(runs / f"build-{name}.log")))
    if a.extra_json:
        steps += json.loads(Path(a.extra_json).read_text())
    if not a.no_quality:
        for name, exe in bins.items():
            for case in cases:
                steps.append(dict(kind="capture", binary=exe, scene=str(ARA1 / f"{case}-quality.json"), out=str(runs / name / f"{case}-quality")))
    for rep in range(a.reps):
        for case in cases:
            for name, exe in (bins.items() if rep % 2 == 0 else reversed(bins.items())):
                steps.append(dict(kind="capture", binary=exe, scene=str(ARA1 / f"{case}-timing.json"), out=str(runs / name / f"{case}-timing-{rep}")))
    if a.tails:
        name, exe = next(iter(bins.items()))
        for case in cases:
            steps.append(dict(kind="capture", binary=exe, scene=str(ROOT / "scenes" / f"{case}-tail.json"), out=str(runs / name / f"{case}-tail")))
    print(json.dumps(steps, indent=1))


if __name__ == "__main__":
    main()
