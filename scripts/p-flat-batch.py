"""P-FLAT screen batch (docs/p-flat.md, docs/p-flat-tr1.md): runs prebuilt binaries over a plan
inside one lease (scripts/p-flat-run.py), one process per run, and records each run's exit
status and wall time. Stops starting new runs when the lease has under 40 s left.

    py -3 -B scripts/p-flat-batch.py PLAN.json OUT_DIR BUDGET_MINUTES

PLAN.json: {"binaries": {KEY: EXE}, "common": {KEY: [...]}, "env": {VAR: VALUE},
            "runs": [{"name": N, "binary": KEY, "path": "rt"|"nonrt", "args": [...],
                      "env": {VAR: VALUE}}, ...]}
A run with "path" gets `--path PATH --out OUT_DIR/NAME` and the binary's common arguments
(the examples); a run without it gets its args alone, with "{out}" replaced by OUT_DIR/NAME
(test binaries). Each run's stdout and stderr go to OUT_DIR/NAME/{stdout,stderr}.log.
"""
import json
import os
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
        exe = plan["binaries"][run["binary"]]
        if "path" in run:
            command = [exe, "--path", run["path"], "--out", str(target),
                       *plan.get("common", {}).get(run["binary"], []), *run["args"]]
        else:
            command = [exe, *(a.replace("{out}", str(target)) for a in run["args"])]
        env = dict(os.environ, **plan.get("env", {}),
                   **{k: v.replace("{out}", str(target)) for k, v in run.get("env", {}).items()})
        begin = time.time()
        with (target / "stdout.log").open("w") as so, (target / "stderr.log").open("w") as se:
            try:
                code = subprocess.run(command, stdout=so, stderr=se, env=env,
                                      timeout=max(10, deadline - 20 - time.time())).returncode
            except subprocess.TimeoutExpired:
                code = "timeout"
        summary.append(dict(name=run["name"], exit=code, seconds=round(time.time() - begin, 2), command=command))
        print(f"{run['name']}: exit {code} in {time.time() - begin:.1f} s", flush=True)
        (out / "batch.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")
    (out / "batch.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")
    return 0 if all(s.get("exit", 0) == 0 for s in summary) else 1


if __name__ == "__main__":
    sys.exit(main())
