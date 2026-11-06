"""Adjacency preparation's offscreen/build lease: 4 GiB start, 2 GiB abort, two jobs."""
import datetime
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("film_run", Path(__file__).with_name("m6-film-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("E:/renderlab-work/cook-triangle-adjacency")
runner.OWNER = "cook-triangle-adjacency"
runner.PREFLIGHT_MEMORY_MBYTES = 4096
runner.RUNTIME_MEMORY_MBYTES = 2048
save = runner.save
reservation = None
base_state = runner.state
base_ps = runner.ps
processes = []


def ps(command):
    global processes
    if command.startswith("Get-CimInstance Win32_Process |"):
        command = command.replace("Name,ProcessId,ExecutablePath", "Name,ProcessId,ExecutablePath,ParentProcessId")
        processes = base_ps(command)
        return processes
    return base_ps(command)


def state(owned=()):
    result = base_state(owned)
    if owned:
        # Cargo's build scripts and test children belong to this lease too. The
        # common runner otherwise mistakes executables under target/release for rivals.
        target = (runner.OUT / "target").resolve()
        result["gpu_processes"] = [p for p in result["gpu_processes"]
            if not Path(p.get("ExecutablePath") or ".").resolve().is_relative_to(target)]
        descendants = set(owned)
        while True:
            children = {p["ProcessId"] for p in processes if p["ParentProcessId"] in descendants}
            if children <= descendants:
                break
            descendants.update(children)
        result["builds"] = [p for p in result["builds"] if p["ProcessId"] not in descendants]
    return result


def record(path, value):
    global reservation
    if path == runner.LOCK / "owner.json":
        reservation = value["utc"]
        end = datetime.datetime.fromisoformat(reservation) + datetime.timedelta(minutes=value["expected_minutes"])
        with Path("E:/renderlab-work/gpu-schedule.md").open("a", encoding="utf-8") as schedule:
            schedule.write(f"\n[cook-triangle-adjacency reservation] start={reservation} end={end.isoformat()} label={value['label']}\n")
    save(path, value)


runner.save = record
runner.ps = ps
runner.state = state
if __name__ == "__main__":
    try:
        raise SystemExit(runner.main())
    finally:
        if reservation:
            with Path("E:/renderlab-work/gpu-schedule.md").open("a", encoding="utf-8") as schedule:
                schedule.write(f"\ncook-triangle-adjacency window released {runner.utc()} (no owned GPU/build job remains).\n")
