"""CPU-only: attribute every microsecond of traced frames to passes, GPU idle and gaps.

    py -3 -I tools/p-nrt/attribute.py TRACE_DIR [TRACE_DIR ...] > summary.txt

Opens each directory's .ngfx-gputrace through ngfx-mcp (N8f's stdio client), reads the
debug-label markers (one per wgpu pass) and splits the trace into frames at each
"LC1 light clusters" marker (the first pass of every frame). Per frame: each marker's
duration by category, GPU gaps inside the frame (barriers/idle between passes) and the
CPU-side gap to the next frame. Writes attribution.json next to the trace.
"""
import json
from pathlib import Path
import statistics as st
import sys

sys.path.insert(0, "E:/renderlab-work/n8f/scripts")
from ngfx_client import Client  # noqa: E402

CATEGORIES = [
    ("light clusters", ("LC1",)),
    ("visibility buffer", ("N1c",)),
    ("sun shadow pages", ("N8 page", "N8c", "N8k texel", "N8 ring horizons")),
    ("sun shadow visibility", ("N8n", "N8 ring and map", "N8k hybrid")),
    ("GI cache", ("indirect cache",)),
    ("atmosphere/fog", ("AT1", "VF1", "atmosphere")),
    ("transmission", ("TP2",)),
    ("deferred shading", ("traced deferred shading",)),
    ("reconstruction", ("temporal reconstruction",)),
    ("transparency", ("TP1", "transparen")),
    ("post", ("HDR present", "post", "bloom")),
    ("wgpu internal", ("(wgpu internal)",)),
]


def category(name):
    for label, prefixes in CATEGORIES:
        if any(name.startswith(p) or p in name for p in prefixes):
            return label
    return "other: " + name


def frames_of(markers):
    starts = [i for i, m in enumerate(markers) if m["name"].startswith("LC1 light clusters")]
    return [markers[a:b] for a, b in zip(starts, starts[1:] + [len(markers)])]


def main():
    c = Client()
    for directory in map(Path, sys.argv[1:]):
        trace = next(directory.glob("*.ngfx-gputrace"))
        tid = c.call("gpu_trace.open_trace", path=str(trace))["trace_id"]
        markers = c.call("gpu_trace.get_timeline_events", trace_id=tid, query="markers", device_index=0, queue_index=0, limit=5000)["records"]
        markers.sort(key=lambda m: m["time_range"]["start_ns"])
        rows = []
        frames = frames_of(markers)
        for i, frame in enumerate(frames):
            start = frame[0]["time_range"]["start_ns"]
            end = max(m["time_range"]["end_ns"] for m in frame)
            passes, gap, cursor = {}, 0, start
            for m in frame:
                t = m["time_range"]
                key = category(m["name"])
                passes[key] = passes.get(key, 0) + t["duration_ns"]
                gap += max(0, t["start_ns"] - cursor)
                cursor = max(cursor, t["end_ns"])
            following = frames[i + 1][0]["time_range"]["start_ns"] if i + 1 < len(frames) else None
            rows.append(dict(gpu_ns=end - start, passes=passes, gaps_ns=gap,
                             cpu_gap_ns=None if following is None else following - end,
                             names=[(m["name"], m["time_range"]["duration_ns"]) for m in frame]))
        complete = [r for r in rows if r["cpu_gap_ns"] is not None] or rows
        keys = sorted({k for r in complete for k in r["passes"]}, key=lambda k: -st.median(r["passes"].get(k, 0) for r in complete))
        print(f"== {directory.name}: {len(rows)} frames, median GPU frame {st.median(r['gpu_ns'] for r in complete)/1e6:.3f} ms,"
              f" in-frame gaps {st.median(r['gaps_ns'] for r in complete)/1e6:.3f} ms,"
              f" CPU gap {st.median((r['cpu_gap_ns'] or 0) for r in complete)/1e6:.3f} ms")
        for k in keys:
            values = [r["passes"].get(k, 0) / 1e6 for r in complete]
            print(f"   {k:28s} {st.median(values):7.3f} ms  ({min(values):.3f}-{max(values):.3f})")
        (directory / "attribution.json").write_text(json.dumps(rows, indent=1) + "\n")
    c.close()


if __name__ == "__main__":
    main()
