"""CPU-only: identity and timing of an A/B run directory (tools/p-nrt/abplan.py layout).

    py -3 -B tools/p-nrt/summarize.py RUNS_DIR [--base base] > summary.txt

Identity: every captured PNG of each binary's `*-quality` and `*-timing-*` runs against the
base binary's same case and frame, by SHA-256 (byte identity, the pure-optimisation bar).
Timing: ARA1's phases (still = frames 1-39, moving 40-71, first cut 72, recovered 73-87)
as medians of CPU service, GPU span, the named passes and render()'s CPU parts, pooled
over repetitions. Writes summary.json beside the runs.
"""
import argparse
import hashlib
import json
from pathlib import Path
import statistics as st

PHASES = {"still": range(1, 40), "moving": range(40, 72), "cut": range(72, 73), "recovered": range(73, 88)}
PASSES = ("span_ms", "shading_ms", "primary_ms", "visibility_ms", "reconstruction_ms", "post_ms")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("runs", type=Path)
    p.add_argument("--base", default="base")
    a = p.parse_args()
    bins = sorted(d.name for d in a.runs.iterdir() if d.is_dir() and (any(d.glob("*-quality")) or any(d.glob("*-timing-*"))))
    out = {"identity": {}, "timing": {}}
    base = a.runs / a.base
    for name in bins:
        if name == a.base:
            continue
        rows = {}
        for run in sorted((a.runs / name).glob("*-*")):
            if not run.is_dir() or run.name == "program-cache":
                continue
            ref = base / run.name
            if not ref.exists():
                continue
            for png in sorted(run.glob("frame-*.png")):
                other = ref / png.name
                if other.exists():
                    rows[f"{run.name}/{png.name}"] = sha(png) == sha(other)
        same = sum(rows.values())
        out["identity"][name] = dict(compared=len(rows), identical=same, differing=[k for k, v in rows.items() if not v])
        print(f"identity {name} vs {a.base}: {same}/{len(rows)} frames byte-identical", *(["differing:"] + out["identity"][name]["differing"][:12] if same != len(rows) else []))
    for name in bins:
        for engine in sorted((a.runs / name).glob("*-timing-*/engine.json")):
            case = engine.parent.name.rsplit("-timing-", 1)[0]
            e = json.loads(engine.read_text())
            gpu = {g["frame_id"]: g for g in e["gpu"]}
            slot = out["timing"].setdefault(case, {}).setdefault(name, {})
            for phase, frames in PHASES.items():
                rows = [f for f in e["frames"] if f["index"] in frames]
                bucket = slot.setdefault(phase, {})
                bucket.setdefault("service_ms", []).extend(f["cpu_service_ms"] for f in rows)
                bucket.setdefault("submit_ms", []).extend(f["cpu_submit_ms"] for f in rows)
                for f in rows:
                    for k, v in f.get("cpu_parts_ms", {}).items():
                        bucket.setdefault("cpu." + k, []).append(v)
                    g = gpu.get(f["frame_id"], {})
                    for k in PASSES:
                        if g.get(k) is not None:
                            bucket.setdefault(k, []).append(g[k])
    print()
    for case, by_bin in out["timing"].items():
        print(f"== {case}")
        for phase in PHASES:
            line = f"  {phase:9s}"
            for name, phases in by_bin.items():
                b = phases[phase]
                med = lambda k: st.median(b[k]) if b.get(k) else float("nan")
                line += f" | {name}: svc {med('service_ms'):6.2f} span {med('span_ms'):6.2f} sh {med('shading_ms'):5.2f} vb {med('primary_ms'):5.2f} vis {med('visibility_ms'):5.2f} tr1 {med('reconstruction_ms'):4.2f} sub {med('submit_ms'):4.2f}"
            print(line)
        for name, phases in by_bin.items():
            b = phases["still"]
            parts = {k[4:]: round(st.median(v), 3) for k, v in b.items() if k.startswith("cpu.") and v}
            print(f"  cpu parts still {name}: {parts}")
        for name, phases in by_bin.items():
            for phase, b in phases.items():
                phases[phase] = {k: dict(median=st.median(v), n=len(v), min=min(v), max=max(v)) for k, v in b.items() if v}
    (a.runs / "summary.json").write_text(json.dumps(out, indent=1) + "\n")


if __name__ == "__main__":
    main()
