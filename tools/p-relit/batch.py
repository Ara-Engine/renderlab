"""P-RELIT lease batch: scripts/p-flat-batch.py over a plan, skipping the runs an earlier lease
finished (an example run's run.json, a test run's result line), so a plan can span leases.

    py -3 -B tools/p-relit/host.py lease LABEL 15 -- py -3 -B tools/p-relit/batch.py PLAN.json OUT_DIR MINUTES
"""
import importlib.util
import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[2]


def finished(out, run):
    target = out / run["name"]
    if "path" in run:
        return (target / "run.json").exists()
    log = target / "stdout.log"
    return log.exists() and "test result:" in log.read_text(errors="replace")


if __name__ == "__main__":
    plan_path, out = Path(sys.argv[1]), Path(sys.argv[2])
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    todo = [r for r in plan["runs"] if not finished(out, r)]
    print(f"{len(plan['runs']) - len(todo)} runs already finished, {len(todo)} to go", flush=True)
    if not todo:
        raise SystemExit(0)
    rest = out / f"{plan_path.stem}.todo.json"
    out.mkdir(parents=True, exist_ok=True)
    rest.write_text(json.dumps(dict(plan, runs=todo)), encoding="utf-8")
    spec = importlib.util.spec_from_file_location("pflat_batch", REPO / "scripts/p-flat-batch.py")
    batch = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(batch)
    sys.argv = [sys.argv[0], str(rest), str(out), sys.argv[3]]
    raise SystemExit(batch.main())
