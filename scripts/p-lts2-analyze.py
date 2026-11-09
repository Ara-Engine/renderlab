"""Summarise a headless GPU Trace without launching the Nsight UI (CPU only)."""
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, "E:/renderlab-work/n8f/scripts")
from ngfx_client import Client

folder = Path(sys.argv[1])
out = Path(sys.argv[2]) if len(sys.argv) > 2 else folder
out.mkdir(parents=True, exist_ok=True)
client = Client()
try:
    trace = next(folder.glob("*.ngfx-gputrace"))
    tid = client.call("gpu_trace.open_trace", path=str(trace))["trace_id"]
    def query(tool, **args):
        return client.call("gpu_trace." + tool, trace_id=tid, device_index=0, **args)
    ranges = query("get_timeline_events", query="queue_ranges", queue_index=0, kind="pipelines", limit=5000)
    shaders = query("get_shader_catalog", query="shaders", limit=500)
    pipelines = query("get_shader_catalog", query="pipelines", limit=500)
    (out / "catalog.json").write_text(json.dumps(dict(ranges=ranges, shaders=shaders, pipelines=pipelines), indent=2))
    totals = {}
    for row in ranges.get("records", []):
        totals.setdefault(row["name"], []).append(row["time_range"]["duration_ns"] / 1e6)
    (out / "timings.json").write_text(json.dumps(totals, indent=2))
    print("pipelines", sorted(((k, sum(v)/len(v), len(v)) for k,v in totals.items()), key=lambda v:-v[1])[:12], flush=True)
    for shader in shaders.get("records", []):
        if shader["name"] not in ("primary_shade_tiles", "primary_shade_list"):
            continue
        si = shader["shader_index"]
        performance = query("get_shader_profile", query="performance", kind="shader", shader_index=si)
        hotspots = query("get_shader_profile", query="hotspots_shader", shader_index=si, include_source_line_text=True, limit=2000)
        source = query("get_shader_code", query="source", shader_index=si, file_index=0, file_type="source", line_number=1, limit=10000)
        (out / f"shader-{si}.json").write_text(json.dumps(dict(shader=shader, performance=performance, hotspots=hotspots, source=source), indent=2))
        functions = []
        current = "<top>"
        for line in source.get("records", source.get("lines", [])):
            match = re.match(r"\s*fn\s+(\w+)", line.get("text", ""))
            if match:
                current = match[1]
            functions.append(current)
        counts = {}
        for row in hotspots.get("records", []):
            line = row["source_location"]["line_number"]
            fn = functions[line-1] if 0 < line <= len(functions) else "?"
            counts[fn] = counts.get(fn, 0) + row["performance"]["sample_count"]
        total = sum(counts.values())
        shares = {k:v/max(total, 1) for k,v in sorted(counts.items(), key=lambda v:-v[1])}
        (out / f"functions-{si}.json").write_text(json.dumps(shares, indent=2))
        print(si, shader["name"], shares, flush=True)
finally:
    client.close()
