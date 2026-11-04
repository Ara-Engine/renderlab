"""Summarize retained Unreal CSV profiler rows; never substitutes process time."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import re

import numpy as np


def log_evidence(path):
    text = path.read_text(encoding="utf-8-sig")
    dumps = []
    for block in text.split("Tracked FD3D12RayTracingGeometry objects")[1:]:
        objects = [dict(name=name, mib=float(size), triangles=int(triangles), compaction=bool(int(compaction)))
            for name, size, triangles, compaction in re.findall(
                r"D3D12RayTracing: Name: (\S+) - Size: ([\d.]+) MB - Prims: (\d+) - Segments: \d+ -  Compaction: (\d+)", block)]
        quarry = [o for o in objects if re.fullmatch(r"Quarry_\d+", o["name"]) and not o["compaction"]]
        dumps.append(dict(objects=objects, native_nanite_quarry_triangles=sum(o["triangles"] for o in quarry),
                          native_nanite_quarry_blas_mib=sum(o["mib"] for o in quarry)))
    tree = []
    for line in text.splitlines():
        columns = line.split("┃")
        if len(columns) < 5:
            continue
        # UE 5.7 also emits wide trees with draw/dispatch/primitive/percentage
        # columns before each time. Select the two time cells, not fixed offsets.
        times = [(i, match) for i, column in enumerate(columns)
                 if (match := re.search(r"([\d.]+) ms", column))]
        if len(times) == 2:
            (i, exclusive), (j, inclusive) = times
            tree.append(dict(name=columns[j + 1].rstrip(), exclusive_ms=float(exclusive[1]), inclusive_ms=float(inclusive[1])))
    return dict(log_sha256=hashlib.sha256(path.read_bytes()).hexdigest(), ray_tracing_dumps=dumps, gpu_tree=tree,
                gpu_tree_scope="one instrumented ProfileGPU frame; inclusive parents overlap and must not be summed",
                native_geometry_scope="mode-1 noncompacted Quarry BLAS entries; verify exact per-mesh source counts")


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
    result = dict(csv=str(path), csv_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                captured_rows=len(data), measured_rows_inclusive=[96, 159], statistics=statistics,
                scope="CSV native fields and units; identify complete GPU frame and CPU thread fields using profiler log/schema before interpreting",
                cpu_critical_path="Thread/wall measurements alone do not establish a dependency-aware CPU critical path")
    log = path.parent / "ue.log"
    if log.is_file():
        result["log_evidence"] = log_evidence(log)
    return result


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
