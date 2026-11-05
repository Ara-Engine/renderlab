"""Summarizes gpu_film_t1_timing runs (paired-timing.json) for T2a (docs/m6-motion-aa.md section 5).

py -3 -B scripts/m6-motion-aa-timing.py <run dir>... [--out summary.json]
Per run: medians of the complete-frame span and its passes for Interactive and for Temporal moving
and static frames; the paired delta and ratio (Temporal minus Interactive within each pair), p99
and maximum spans, and the frames above 1.5x the Temporal median (the frozen spike check).
"""
import argparse
import json
from pathlib import Path
import statistics as st


def quantile(values, q):
    v = sorted(values)
    return v[min(len(v) - 1, int(q * (len(v) - 1) + 0.5))] if v else None


def summarize(directory):
    data = json.loads((directory / "paired-timing.json").read_text())
    samples = data["samples"]
    pairs = {}
    for s in samples:
        pairs.setdefault(s["pair"], {})[s["mode"]] = s
    out = dict(run=str(directory), records=data.get("records"), sequence=data.get("sequence"), pairs=len(pairs))
    for name, keep in (("interactive", lambda s: s["mode"] == "Interactive"),
                       ("temporal_moving", lambda s: s["mode"] == "Temporal" and s.get("moving")),
                       ("temporal_static", lambda s: s["mode"] == "Temporal" and not s.get("moving"))):
        rows = [s for s in samples if keep(s)]
        if not rows:
            continue
        spans = [s["span_ms"] for s in rows]
        median = st.median(spans)
        out[name] = dict(n=len(rows), span_p50=median, span_p99=quantile(spans, .99), span_max=max(spans),
                         over_1_5x=sum(v > 1.5 * median for v in spans),
                         trace_p50=st.median([s["trace_ms"] for s in rows if s["trace_ms"] is not None]),
                         compute_p50=st.median([s.get("compute_ms") or 0. for s in rows]),
                         shading_p50=st.median([s["shading_ms"] for s in rows]),
                         tone_p50=st.median([s["tone_ms"] for s in rows if s["tone_ms"] is not None]))
        out[name]["other_p50"] = (out[name]["span_p50"] - out[name]["trace_p50"] - out[name]["compute_p50"]
                                  - out[name]["shading_p50"] - out[name]["tone_p50"])
    for name, moving in (("moving", True), ("static", False)):
        deltas = [p["Temporal"]["span_ms"] - p["Interactive"]["span_ms"] for p in pairs.values()
                  if "Temporal" in p and "Interactive" in p and bool(p["Temporal"].get("moving")) == moving]
        ratios = [p["Temporal"]["span_ms"] / p["Interactive"]["span_ms"] for p in pairs.values()
                  if "Temporal" in p and "Interactive" in p and bool(p["Temporal"].get("moving")) == moving]
        if deltas:
            out[f"paired_{name}"] = dict(n=len(deltas), delta_p50=st.median(deltas), ratio_p50=st.median(ratios))
    contention = [s.get("resource_context", {}).get("rustc_count") for s in samples if s.get("resource_context")]
    memory = [s.get("resource_context", {}).get("available_mbytes") for s in samples if s.get("resource_context")]
    out["contention"] = dict(rustc_max=max(contention, default=None), available_min_mbytes=min(memory, default=None))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("runs", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    rows = [summarize(d) for d in a.runs]
    text = json.dumps(rows, indent=1)
    if a.out:
        a.out.write_text(text)
    for r in rows:
        line = [r["run"].split("\\")[-1].split("/")[-1], f"K={r['records']}"]
        for name in ("interactive", "temporal_moving", "temporal_static"):
            if name in r:
                v = r[name]
                line.append(f"{name} p50 {v['span_p50']:.4f} (trace {v['trace_p50']:.4f}, validation {v['compute_p50']:.4f}, "
                            f"shading {v['shading_p50']:.4f}, other {v['other_p50']:.4f}) p99 {v['span_p99']:.3f} "
                            f"max {v['span_max']:.3f} >1.5x {v['over_1_5x']}")
        for name in ("paired_moving", "paired_static"):
            if name in r:
                line.append(f"{name} delta {r[name]['delta_p50']:+.4f} ratio {r[name]['ratio_p50']:.4f} n {r[name]['n']}")
        line.append(f"contention {r['contention']}")
        print("\n  ".join(line))


if __name__ == "__main__":
    main()
