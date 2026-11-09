"""CPU-only: shader-profiler hotspots of one pass in a GPU trace (N8f's analyze.py, trimmed).

    py -3 -I tools/p-nrt/hotspots.py TRACE_DIR MARKER_PREFIX [LIMIT]

Selects the longest marker range starting with MARKER_PREFIX, finds the shaders whose
pipeline ran in it, and writes perf/hotspots JSON next to the trace; prints top lines with
their sample share and stall reasons.
"""
import json
from pathlib import Path
import sys

sys.path.insert(0, "E:/renderlab-work/n8f/scripts")
from ngfx_client import Client  # noqa: E402


def main():
    directory, prefix = Path(sys.argv[1]), sys.argv[2]
    limit = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    c = Client()
    tid = c.call("gpu_trace.open_trace", path=str(next(directory.glob("*.ngfx-gputrace"))))["trace_id"]
    markers = c.call("gpu_trace.get_timeline_events", trace_id=tid, query="markers", device_index=0, queue_index=0, limit=5000)["records"]
    mine = [m for m in markers if m["name"].startswith(prefix)]
    longest = max(mine, key=lambda m: m["time_range"]["duration_ns"])
    t = longest["time_range"]
    sel = {"type": "time_range", "start_ns": t["start_ns"], "end_ns": t["end_ns"]}
    ranges = c.call("gpu_trace.get_timeline_events", trace_id=tid, query="queue_ranges", device_index=0, queue_index=0, kind="pipelines", limit=5000)["records"]
    inside = {r["name"].split(" | ")[0] for r in ranges if r["time_range"]["start_ns"] >= t["start_ns"] and r["time_range"]["end_ns"] <= t["end_ns"]}
    shaders = c.call("gpu_trace.get_shader_catalog", trace_id=tid, query="shaders", device_index=0, limit=500)["records"]
    pipelines = c.call("gpu_trace.get_shader_catalog", trace_id=tid, query="pipelines", device_index=0, limit=500)["records"]
    indices = sorted({i for p in pipelines if p["name"] in inside for i in p["shader_indices"]})
    print("marker", longest["name"], t["duration_ns"] / 1e3, "us; pipelines", inside, "shaders", [(i, shaders[i]["name"]) for i in indices])
    safe = "".join(ch if ch.isalnum() else "-" for ch in prefix)
    for si in indices:
        perf = c.call("gpu_trace.get_shader_profile", trace_id=tid, query="performance", device_index=0, kind="shader", shader_index=si, range_selector=sel)
        hot = c.call("gpu_trace.get_shader_profile", trace_id=tid, query="hotspots_shader", device_index=0, shader_index=si, range_selector=sel, include_source_line_text=True, limit=limit)
        (directory / f"hot-{safe}-{si}.json").write_text(json.dumps(dict(perf=perf, hot=hot), indent=1))
        print("== shader", si, shaders[si]["name"], json.dumps(perf)[:1500])
        for r in hot.get("records", [])[:limit]:
            line = {k: v for k, v in r.items() if k not in ("source_line_text",)}
            print(json.dumps(line)[:400])
            print("     ", (r.get("source_line_text") or "")[:220])
    c.close()


if __name__ == "__main__":
    main()
