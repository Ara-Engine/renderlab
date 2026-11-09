"""P-FLAT screen batch (docs/p-flat.md): runs prebuilt example binaries over a plan of
variants inside one lease (scripts/p-flat-run.py), one process per variant, and records each
run's exit status and wall time. Stops starting new runs when the lease has under 40 s left.

    py -3 -B scripts/p-flat-batch.py PLAN.json OUT_DIR BUDGET_MINUTES

PLAN.json: {"binaries": {"game": EXE, "matched": EXE}, "common": {"game": [...], ...},
            "runs": [{"name": N, "binary": "game", "path": "rt"|"nonrt", "args": [...]}, ...]}
"""
import json
import subprocess
import sys
import time
from pathlib import Path


def main():
    plan = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    out, deadline = Path(sys.argv[2]), time.time() + 60 * float(sys.argv[3])
    out.mkdir(parents=True, exist_ok=True)
    summary = []
    for run in plan["runs"]:
        if time.time() > deadline - 40:
            summary.append(dict(name=run["name"], skipped="lease time"))
            continue
        target = out / run["name"]
        target.mkdir(parents=True, exist_ok=True)
        command = [plan["binaries"][run["binary"]], "--path", run["path"], "--out", str(target),
                   *plan["common"].get(run["binary"], []), *run["args"]]
        begin = time.time()
        with (target / "stdout.log").open("w") as so, (target / "stderr.log").open("w") as se:
            code = subprocess.run(command, stdout=so, stderr=se,
                                  timeout=max(10, deadline - 20 - time.time())).returncode
        summary.append(dict(name=run["name"], exit=code, seconds=round(time.time() - begin, 2), command=command))
        print(f"{run['name']}: exit {code} in {time.time() - begin:.1f} s", flush=True)
        (out / "batch.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")
    (out / "batch.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")
    return 0 if all(s.get("exit", 0) == 0 for s in summary) else 1


if __name__ == "__main__":
    sys.exit(main())
