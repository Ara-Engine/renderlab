"""Analyze saved resize logs without creating a window or a GPU device."""
import argparse
import hashlib
import json
from pathlib import Path
import re


def summarize(lines, slow_ms=250.0):
    configures, active, phases, slow_calls, issues = [], {}, {}, [], []
    configure_begin = None
    probe_headers = 0
    saw_probe = False
    for line_number, line in enumerate(lines, 1):
        if line.startswith("configure_begin "):
            configure_begin = {"line": line_number, "text": line.strip()}
        if match := re.match(r"^configure_ms=([\d.]+) width=(\d+) height=(\d+)", line):
            configures.append({
                "line": line_number, "ms": float(match[1]),
                "width": int(match[2]), "height": int(match[3]),
                "begin": configure_begin,
            })
            configure_begin = None
        if not line.startswith("surface_probe "):
            continue
        saw_probe = True
        fields = dict(re.findall(r"(\w+)=(\S+)", line))
        try:
            if "version" in fields:
                probe_headers += 1
                if fields["version"] != "1" or probe_headers > 1:
                    raise ValueError("expected one version=1 probe session per log")
            elif "begin" in fields:
                identifier = int(fields["begin"])
                if identifier in active:
                    raise ValueError("duplicate active span")
                active[identifier] = {
                    "id": identifier, "phase": fields["phase"],
                    "at_us": int(fields["at_us"]), "line": line_number,
                }
            elif "end" in fields:
                identifier = int(fields["end"])
                begin = active[identifier]
                at_us, duration_us = int(fields["at_us"]), int(fields["duration_us"])
                if fields["phase"] != begin["phase"] or not 0 <= duration_us <= at_us - begin["at_us"]:
                    raise ValueError("span phase or clock mismatch")
                outcome = fields["outcome"]
                del active[identifier]
                ms = duration_us / 1000
                phase = phases.setdefault(fields["phase"], {"count": 0, "max_ms": 0, "outcomes": {}})
                phase["count"] += 1
                phase["max_ms"] = max(phase["max_ms"], ms)
                phase["outcomes"][outcome] = phase["outcomes"].get(outcome, 0) + 1
                if ms >= slow_ms:
                    slow_calls.append({**begin, "end_line": line_number, "ms": ms, "outcome": outcome})
            elif "event" in fields:
                int(fields["at_us"])
            else:
                raise ValueError("unrecognized probe record")
        except (KeyError, ValueError) as error:
            issues.append({"line": line_number, "error": str(error)})
    if saw_probe and not probe_headers:
        issues.append({"line": None, "error": "missing probe session header"})
    unfinished = list(active.values())
    return {
        "configure_count": len(configures),
        "configures": configures,
        "unfinished_configure_begin": configure_begin,
        "post_initial_slow_configures": [c for c in configures[1:] if c["ms"] >= slow_ms],
        "probe_status": "malformed" if issues else "incomplete" if unfinished else "balanced" if probe_headers else "absent",
        "phases": phases,
        "slow_calls": sorted(slow_calls, key=lambda call: call["line"]),
        "unfinished_calls": unfinished,
        "parse_errors": issues,
        "parse_error_count": len(issues),
        "call_error_count": sum(p["outcomes"].get(o, 0) for p in phases.values() for o in ("error", "lost", "validation")),
        "slow_threshold_ms": slow_ms,
        "causality": "INCONCLUSIVE. Call boundaries cannot identify an internal driver or desktop wait. An unmatched begin also permits process termination or a truncated log.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path)
    parser.add_argument("--slow-ms", type=float, default=250.0)
    args = parser.parse_args()
    if not 0 < args.slow_ms < float("inf"):
        parser.error("--slow-ms must be finite and positive")
    results = {}
    for path in args.logs:
        raw = path.read_bytes()
        results[str(path)] = {
            "sha256": hashlib.sha256(raw).hexdigest(),
            **summarize(raw.decode("utf-8", errors="replace").splitlines(), args.slow_ms),
        }
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
