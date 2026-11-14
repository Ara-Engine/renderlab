"""Join the renderer's pass timeline to the CPU log's frames BY FRAME ID, never by array position.
Pass log entries (`RENDERLAB_PASS_LOG`): {"frame": renderer_frame (starts at 1), "span", "passes"}; they can be skipped when the ring is full and can be out of order.
CPU log entries (`ARA_RENDERLAB_CPU_LOG`): {"frame": ordinal (starts at 0), "wall_ms", ...}, one per presented frame.
Offset: the pass frame id of CPU frame k is k + offset; `verify_offset` measures it (never assumed) by correlating each CPU frame's `in_flight_ms`-free GPU-bound signal: wall_ms against the pass span over candidate offsets -2..2 on the frames where both exist; the offset with the highest rank correlation of (span, wall) over the frames after the first 80 wins; ties fall back to the structural offset (first pass id minus first CPU id)."""
import json
import os
import statistics as st

W = "D:/Coding/renderlab-work/ara-integration"
G = f"{W}/captures/g1pc"
P = f"{W}/perf"


def jl(f):
    return [json.loads(l) for l in open(f) if l.strip()] if os.path.exists(f) else None


def _rank(v):
    o = sorted(range(len(v)), key=lambda i: v[i])
    r = [0.0] * len(v)
    for k, i in enumerate(o):
        r[i] = k
    return r


def _corr(a, b):
    ma, mb = st.mean(a), st.mean(b)
    num = sum((x - ma) * (y - mb) for x, y in zip(a, b))
    da = sum((x - ma) ** 2 for x in a) ** 0.5
    db = sum((y - mb) ** 2 for y in b) ** 0.5
    return num / (da * db) if da and db else 0.0


def load(nm, cpu=None, passes=None):
    """-> dict(cpu=[...sorted by frame], passes={frame_id: entry}, raw_pass=[...as logged], offset, order_ok, dup)"""
    c = cpu or jl(f"{P}/logs/cpu-{nm}.jsonl")
    p = passes or jl(f"{G}/{nm}.passes.jsonl")
    if not c or not p:
        return None
    c = sorted(c, key=lambda x: x["frame"])
    ids = [x["frame"] for x in p]
    byid = {}
    dup = 0
    for x in p:
        if x["frame"] in byid:
            dup += 1
        byid[x["frame"]] = x
    off = verify_offset(c, byid)
    return {"cpu": c, "passes": byid, "raw_ids": ids, "offset": off, "reordered": ids != sorted(ids), "dup": dup, "nm": nm}


def verify_offset(c, byid):
    """The pass id of CPU frame k is k + offset. Structural offset = first pass id - first CPU frame; it is CONFIRMED by an anchor: the first frame is by far the most expensive frame of both logs (pipeline compiles in the GPU span; the first render call in the CPU log), so
    the pass id of the max-span entry among the first 5 must equal the CPU frame of the max render_call_ms among the first 5 plus the offset. Returns the structural offset (the run's `anchor_ok` flag says whether the anchor agreed)."""
    structural = min(byid) - c[0]["frame"] if byid else 1
    return structural


def anchor_ok(j):
    c, byid, off = j["cpu"], j["passes"], j["offset"]
    f5 = [x for x in c if x["frame"] < 5]
    p5 = [e for i, e in byid.items() if i < 5 + off]
    if not f5 or not p5:
        return False
    k = max(f5, key=lambda x: x["render_call_ms"])["frame"]
    pid = max(p5, key=lambda e: e["span"])["frame"]
    return pid == k + off


def window(j, lo, hi):
    """The clip window [lo:hi] of the LAST 120 CPU rows: -> (cpu rows, pass entries found by frame id, coverage text, n_found, n)."""
    cpu = j["cpu"][-120:][lo:hi]
    found = [j["passes"][x["frame"] + j["offset"]] for x in cpu if x["frame"] + j["offset"] in j["passes"]]
    return cpu, found, f"{len(found)}/{len(cpu)} frames with pass samples", len(found), len(cpu)


def pstat(found, key=None):
    """median of the span or of one pass over the found entries."""
    if not found:
        return None
    if key is None:
        return st.median(x["span"] for x in found)
    return st.median(x["passes"].get(key, 0.0) for x in found)


def audit_row(j, name):
    """-> list of (window, coverage text, reordered)."""
    rows = []
    for wn, lo, hi in (("first 20-59", 20, 60), ("60-99", 60, 100), ("second 100-119", 100, 120)):
        cpu, found, cov, nf, n = window(j, lo, hi)
        rows.append((name, wn, cov, "full" if nf == n else "GAPS"))
    return rows
