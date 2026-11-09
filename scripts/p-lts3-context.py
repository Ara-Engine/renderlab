"""Attribute recorded compiler/RAM samples to each paired renderer process."""
import argparse
import json
from pathlib import Path

ROOT = Path("D:/Coding/renderlab-work/p-lts3")
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("lease")
parser.add_argument("runs", nargs="+")
args = parser.parse_args()
lease = json.loads((ROOT / "lease/runs" / (args.lease + ".json")).read_text())
rows = []
for run in args.runs:
    report = json.loads((ROOT / "runs" / run / "summary.json").read_text())
    for pair in report.get("pairs", report.get("rows", [])):
        samples = [s for s in lease["resource_samples"]
                   if pair["start_unix"] <= s["host_unix_seconds"] <= pair["end_unix"]]
        row = {key: pair[key] for key in ("mode", "variant", "binary_sha256")}
        service = pair.get("service_ms", {}).get("steady_still")
        if pair.get("step") == "quarry":
            folder = ROOT / "runs" / run / f"quarry-{pair['mode']}-{pair['variant']}"
            service = json.loads((folder / f"lc2-{pair['mode']}.json").read_text())["time_default"]["median_ms"]
        row.update(run=run, case=pair.get("case", pair.get("step")), service_ms=service,
                   resource_samples=len(samples),
                   rustc_max=max((s["rustc_count"] for s in samples), default=None),
                   min_available_mib=min((s["available_mbytes"] for s in samples), default=None))
        rows.append(row)
        print(row)
(ROOT / (args.lease + "-context.json")).write_text(json.dumps(rows, indent=2))
