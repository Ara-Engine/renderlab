"""Summarize retained Unreal CSV profiler rows; never substitutes process time."""
import argparse
import csv
import hashlib
import json
from pathlib import Path

import numpy as np


def summarize(path):
    with path.open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.reader(stream))
    # UE may append columns during capture and repeat the complete header at
    # the end. Earlier short rows contain implicit zeroes for the new stats.
    all_header = next(row for row in reversed(rows) if row and row[0] == "EVENTS")
    indices = [i for i, name in enumerate(all_header) if name != "EVENTS"]
    header = [all_header[i] for i in indices]
    data = []
    for row in rows[1:]:
        if not row or row[0].startswith("[") or row[0] == "EVENTS" or len(row) > len(all_header):
            continue
        row = row + ["0"] * (len(all_header) - len(row))
        try:
            values = [float(row[i]) for i in indices]
        except ValueError:
            continue
        data.append(values)
    if len(data) < 160:
        raise ValueError(f"expected 160 rows including startup/warmup: {path}, got {len(data)}")
    # Registered before the run: 0..95 include warmup, ProfileGPU at 64 and
    # screenshot at 72. Keep every row in the fixed 96..159 measured block.
    block = np.asarray(data[96:160])
    statistics = {}
    for name, column in zip(header, block.T):
        if not np.isfinite(column).all():
            raise ValueError("nonfinite profiler column: " + name)
        statistics[name] = dict(n=len(column), median=float(np.median(column)), minimum=float(column.min()),
                                p95=float(np.quantile(column, .95)), maximum=float(column.max()),
                                standard_deviation=float(column.std(ddof=1)))
    return dict(csv=str(path), csv_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                captured_rows=len(data), measured_rows_inclusive=[96, 159], statistics=statistics,
                scope="CSV native fields and units; identify complete GPU frame and CPU thread fields using profiler log/schema before interpreting",
                cpu_critical_path="Thread/wall measurements alone do not establish a dependency-aware CPU critical path")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("refusing to overwrite a retained summary")
    result = summarize(args.csv)
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    print(json.dumps({"captured_rows": result["captured_rows"], "fields": list(result["statistics"])}, indent=2))
