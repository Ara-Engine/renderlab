"""Guarded runtime-adjacency build/check/measurement lease; all outputs on E:."""
import datetime
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/parallel-adjacency")
runner.OWNER = "parallel-adjacency"
runner.PREFLIGHT_MEMORY_MBYTES = 4096
runner.RUNTIME_MEMORY_MBYTES = 2048
schedule = Path("E:/renderlab-work/gpu-schedule.md")
original_save, original_state = runner.save, runner.state
reservation = None


def save(path, value):
    global reservation
    if path == runner.LOCK / "owner.json" and value.get("owner") == runner.OWNER:
        reservation = value["utc"]
        end = datetime.datetime.fromisoformat(reservation) + datetime.timedelta(minutes=value["expected_minutes"])
        with schedule.open("a", encoding="utf-8") as f:
            f.write(f"\n[parallel-adjacency reservation] start={reservation} end={end.isoformat()} label={value['label']}\n")
    original_save(path, value)


def state(owned=()):
    value = original_state(owned)
    # The lease launches one batch script, whose own test child is also ours.
    target = str(runner.OUT / "target").lower().replace("/", "\\") + "\\"
    value["gpu_processes"] = [p for p in value["gpu_processes"]
        if not (p.get("ExecutablePath") or "").lower().replace("/", "\\").startswith(target)]
    return value


runner.save, runner.state = save, state
if __name__ == "__main__":
    try:
        raise SystemExit(runner.main())
    finally:
        if reservation is not None:
            with schedule.open("a", encoding="utf-8") as f:
                f.write(f"\nparallel-adjacency window released {runner.utc()} (no owned GPU/build job remains).\n")
