"""M7-TR1: paired end-to-end cost of a reconstruction variant against the unreconstructed
baseline on the same registered track, from gpu_tr1_sequence's alternating visits, and the
check against TR0's half-TSR-Epic ceilings (E:/renderlab-work/m7-tr0/bar/tr1-bar.json).

  py -3 -B scripts/m7-tr1-cost.py <tr1.json> --candidate a-tr1 --baseline z-off [--out cost.json]

As TR0: each visit replays the whole track from a reset, the first 8 track frames of every
visit are excluded, and increments pair the same input frame of the same visit pair. The
span is Renderlab's GPU render span (first pass begin to present end), the whole-frame
measure; the reconstruction pass (sample copy and resolve) is reported separately.
"""
import argparse
import json
from pathlib import Path

import numpy as np

BAR = Path("E:/renderlab-work/m7-tr0/bar/tr1-bar.json")
COMPARATOR = {"rt": "hrt", "nonrt": "vsm-nort", "nonrt-on-rt": "vsm-nort"}
LEAD = 8


def percentiles(values):
    a = np.asarray(values, dtype="f8")
    return {"p50": float(np.percentile(a, 50)), "p95": float(np.percentile(a, 95)),
            "p99": float(np.percentile(a, 99)), "max": float(a.max()), "n": int(a.size)} if a.size else None


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("run")
    parser.add_argument("--candidate", default="a-tr1")
    parser.add_argument("--baseline", default="z-off")
    parser.add_argument("--out")
    parser.add_argument("--first-visit", type=int, default=1,
                        help="first visit index used (0 is the capture visit, which reads back frames)")
    args = parser.parse_args()
    run = json.loads(Path(args.run).read_text())
    cand = run["variants"][args.candidate]["visits"][args.first_visit:]
    base = run["variants"][args.baseline]["visits"][args.first_visit:]
    increments, totals, passes, base_totals = [], [], [], []
    stages = {k: [] for k in ("receiver_ms", "shading_ms", "present_ms")}
    for c, b in zip(cand, base):
        for f in range(LEAD, min(len(c), len(b))):
            if c[f] is None or b[f] is None:
                continue
            increments.append(c[f]["span_ms"] - b[f]["span_ms"])
            totals.append(c[f]["span_ms"])
            base_totals.append(b[f]["span_ms"])
            if c[f].get("reconstruction_ms") is not None:
                passes.append(c[f]["reconstruction_ms"])
            for k in stages:
                if c[f].get(k) is not None and b[f].get(k) is not None:
                    stages[k].append(c[f][k] - b[f][k])
    bar = json.loads(BAR.read_text())
    ceiling = next(c for c in bar["cost_ceilings"] if c["track"] == "q-walk" and c["path"] == COMPARATOR[run["path"]])
    inc, tot = percentiles(increments), percentiles(totals)
    verdict = {}
    for p in ("p50", "p95", "p99"):
        verdict[f"increment_{p}"] = bool(inc and inc[p] <= ceiling["increment_ceiling_ms"][p])
        verdict[f"total_{p}"] = bool(tot and tot[p] <= ceiling["whole_frame_ceiling_ms"][p])
    result = dict(run=args.run, path=run["path"], candidate=args.candidate, baseline=args.baseline,
                  pairs=len(list(zip(cand, base))), increment_ms=inc, candidate_span_ms=tot,
                  baseline_span_ms=percentiles(base_totals), reconstruction_pass_ms=percentiles(passes),
                  stage_increments_ms={k: percentiles(v) for k, v in stages.items()},
                  ceiling=ceiling, passes=verdict, all_pass=all(verdict.values()),
                  note="Contended unless the run's contention.json and schedule show a qualified QUIET window.")
    text = json.dumps(result, indent=1)
    if args.out:
        Path(args.out).write_text(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
