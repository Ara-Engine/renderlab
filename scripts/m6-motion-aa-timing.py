"""Summarizes paired T1/T2 frame timing (docs/m6-motion-aa.md section 5, docs/m6-motion-aa-gpu.md).

py -3 -B scripts/m6-motion-aa-timing.py <run dir or batch.json>... [--out summary.json]
A run directory holds gpu_film_t1_timing's paired-timing.json; a batch.json (gpu_film_t2_batch)
holds several timing runs. Per run and Temporal variant (the gathers, or the scatter of
support/film_t2_scatter.wgsl): medians of the complete-frame span and its passes for Interactive
and for Temporal moving and static frames; the paired delta and ratio (Temporal minus Interactive
within each pair), p99 and maximum spans, and the frames above 1.5x the Temporal median (the
frozen spike check).
"""
import argparse
import json
from pathlib import Path
import statistics as st


def quantile(values, q):
    v = sorted(values)
    return v[min(len(v) - 1, int(q * (len(v) - 1) + 0.5))] if v else None


def stats(rows):
    spans = [s["span_ms"] for s in rows]
    median = st.median(spans)
    out = dict(n=len(rows), span_p50=median, span_p99=quantile(spans, .99), span_max=max(spans),
               over_1_5x=sum(v > 1.5 * median for v in spans),
               trace_p50=st.median([s["trace_ms"] or 0. for s in rows]),
               compute_p50=st.median([s.get("compute_ms") or 0. for s in rows]),
               shading_p50=st.median([s["shading_ms"] for s in rows]),
               tone_p50=st.median([s["tone_ms"] or 0. for s in rows]))
    out["other_p50"] = out["span_p50"] - out["trace_p50"] - out["compute_p50"] - out["shading_p50"] - out["tone_p50"]
    return out


def summarize(name, data):
    samples = data["samples"]
    pairs = {}
    for s in samples:
        pairs.setdefault(s["pair"], {})[s["mode"]] = s
    out = dict(run=name, records=data.get("records"), sequence=data.get("sequence"), pairs=len(pairs))
    interactive = [s for s in samples if s["mode"] == "Interactive"]
    if interactive:
        out["interactive"] = stats(interactive)
    variants = sorted({s.get("variant", "gather") for s in samples if s["mode"] == "Temporal"})
    for variant in variants:
        for kind, moving in (("moving", True), ("static", False)):
            rows = [s for s in samples if s["mode"] == "Temporal" and s.get("variant", "gather") == variant
                    and bool(s.get("moving")) == moving]
            if not rows:
                continue
            row = stats(rows)
            matched = [p for p in pairs.values() if "Temporal" in p and "Interactive" in p
                       and p["Temporal"].get("variant", "gather") == variant and bool(p["Temporal"].get("moving")) == moving]
            if matched:
                row["paired_delta_p50"] = st.median([p["Temporal"]["span_ms"] - p["Interactive"]["span_ms"] for p in matched])
                row["paired_ratio_p50"] = st.median([p["Temporal"]["span_ms"] / p["Interactive"]["span_ms"] for p in matched])
            out[f"{variant}_{kind}"] = row
    contention = [s["resource_context"]["rustc_count"] for s in samples if s.get("resource_context")]
    memory = [s["resource_context"]["available_mbytes"] for s in samples if s.get("resource_context")]
    out["contention"] = dict(rustc_max=max(contention, default=None), available_min_mbytes=min(memory, default=None))
    return out


def runs(path):
    if path.is_dir():
        yield path.name, json.loads((path / "paired-timing.json").read_text())
    else:
        for t in json.loads(path.read_text())["timing"]:
            yield t["label"], t


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("runs", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    rows = [summarize(name, data) for p in a.runs for name, data in runs(p)]
    text = json.dumps(rows, indent=1)
    if a.out:
        a.out.write_text(text)
    for r in rows:
        line = [f"{r['run']} K={r['records']}"]
        for name, v in r.items():
            if isinstance(v, dict) and "span_p50" in v:
                extra = f" paired delta {v['paired_delta_p50']:+.4f} ratio {v['paired_ratio_p50']:.3f}" if "paired_delta_p50" in v else ""
                line.append(f"{name} n {v['n']} p50 {v['span_p50']:.4f} (trace {v['trace_p50']:.4f}, validation {v['compute_p50']:.4f}, "
                            f"shading {v['shading_p50']:.4f}, other {v['other_p50']:.4f}) p99 {v['span_p99']:.3f} "
                            f"max {v['span_max']:.3f} >1.5x {v['over_1_5x']}{extra}")
        line.append(f"contention {r['contention']}")
        print("\n  ".join(line))


if __name__ == "__main__":
    main()
