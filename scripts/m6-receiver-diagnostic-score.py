"""Paired ratios and resource context for receiver-policy cost screens."""
import json
from pathlib import Path
import numpy as np

OUT = Path("E:/renderlab-work/receiver-adoption/followup")


def main():
    rows=[]
    for path in sorted(OUT.glob("*/paired-timing.json")):
        record=json.loads(path.read_text())
        for mode in ("hard","interactive","exact"):
            samples=[s for s in record["samples"] if s["mode"]==mode]
            row=dict(run=path.parent.name,mode=mode,stages={})
            contexts=[s["resource_context"] for s in samples if "resource_context" in s]
            if contexts:
                row.update(rustc_max=max(s["rustc_count"] for s in contexts),
                    available_min_mbytes=min(s["available_mbytes"] for s in contexts),
                    resource_max_age_ms=max(s["age_ms"] for s in contexts),
                    contended_screen=any(s.get("contended_screen",False) for s in samples))
            for stage in ("span_ms","receiver_ms","compute_ms","shading_ms"):
                pairs={}
                for s in samples:
                    if s[stage] is not None:pairs.setdefault(s["pair"],{})[s["baseline"]]=s[stage]
                if not pairs:continue
                if not all(True in p and False in p for p in pairs.values()):
                    row["stages"][stage]={
                        "baseline_median_ms":float(np.median([p[True] for p in pairs.values() if True in p])) if any(True in p for p in pairs.values()) else None,
                        "candidate_median_ms":float(np.median([p[False] for p in pairs.values() if False in p])) if any(False in p for p in pairs.values()) else None,
                    }
                    continue
                baseline=np.array([p[True] for p in pairs.values()]);candidate=np.array([p[False] for p in pairs.values()])
                ratio=candidate/baseline
                rng=np.random.default_rng(767)
                boot=np.median(rng.choice(ratio,size=(5000,len(ratio)),replace=True),axis=1)
                row["stages"][stage]=dict(baseline_median_ms=float(np.median(baseline)),
                    candidate_median_ms=float(np.median(candidate)),
                    paired_median_delta_ms=float(np.median(candidate-baseline)),
                    paired_ratio=float(np.median(ratio)),ratio_95pct=np.quantile(boot,[.025,.975]).tolist())
            rows.append(row)
    (OUT/"timing-summary.json").write_text(json.dumps(rows,indent=2))
    for row in rows:
        if "span_ms" in row["stages"]:print(row["run"],row["mode"],json.dumps(row["stages"]["span_ms"]))


if __name__ == "__main__":main()
