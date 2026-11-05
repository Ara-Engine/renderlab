"""Isolated back-face screen; shared GPU lease, 4 GiB start / 2 GiB abort."""
import datetime
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/backface-retry")
runner.OWNER = "backface-retry"
runner.PREFLIGHT_MEMORY_MBYTES = 4096
runner.RUNTIME_MEMORY_MBYTES = 2048
original_save = runner.save
original_state = runner.state
reserved = False


def save(path, value):
    global reserved
    if path == runner.LOCK / "owner.json" and value.get("owner") == runner.OWNER:
        end = datetime.datetime.fromisoformat(value["utc"]) + datetime.timedelta(minutes=value["expected_minutes"])
        with Path("E:/renderlab-work/gpu-schedule.md").open("a") as schedule:
            schedule.write(f"\n[backface-retry reservation] {value['utc']} end={end.isoformat()} label={value['label']}\n")
        reserved = True
    original_save(path, value)


runner.save = save


def state(owned=()):
    value = original_state(owned)
    # Cargo build scripts are CPU children under target/release/build, which the
    # generic runner's conservative renderer-path heuristic also matches.
    value["cpu_build_scripts"] = [p for p in value["gpu_processes"] if p["Name"] == "build-script-build.exe"]
    value["gpu_processes"] = [p for p in value["gpu_processes"] if p["Name"] != "build-script-build.exe"]
    return value


runner.state = state
if __name__ == "__main__":
    try:
        raise SystemExit(runner.main())
    finally:
        if reserved:
            with Path("E:/renderlab-work/gpu-schedule.md").open("a") as schedule:
                schedule.write(f"\nbackface-retry window released {runner.utc()} (no owned job remains).\n")
