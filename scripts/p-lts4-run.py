"""P-LTS4 GPU lease: <=15 min, >=8 GiB admission, standard 2 GiB runtime guard.

This unit requires 8 GiB admission for every run. A 120-second compiler-free
admission interval avoids gaps between foreign Cargo commands.

Read `order` before each run. Commands execute through scripts/n8-run.py.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import re
import sys
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/p-lts4")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def order():
    latest = None
    with SCHEDULE.open(encoding="utf-8") as schedule:
        for line in schedule:
            if line.startswith("[root order]"):
                latest = line.rstrip("\r\n")
    if latest is None:
        raise RuntimeError("no root order in the schedule")
    return latest


def main():
    if sys.argv[1:] == ["order"]:
        print(order())
        return 0
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--label", required=True)
    p.add_argument("--minutes", type=float, default=10)
    p.add_argument("--handoff-seconds", type=float, default=600)
    p.add_argument("--after-owner", help="require this owner's release after the latest root order")
    p.add_argument("--quiet", action="store_true", help="reject compiler contention during the lease")
    p.add_argument("--admission-mib", type=int, default=8192, help="quarry requires 8192; short runs require at least 8192")
    p.add_argument("--runtime-mib", type=int, choices=[2048], default=2048)
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if not 0 < a.minutes <= 15:
        p.error("lease must be <=15 minutes")
    if a.admission_mib < 8192:
        p.error("admission floor must be >=8192 MiB")
    directive = order()
    import shutil
    assert shutil.disk_usage("D:/").free >= 5_000_000_000
    print(directive, flush=True)
    os.environ.update(N8_OWNER="p-lts4", N8_OUT=str(OUT / "lease"))
    spec = importlib.util.spec_from_file_location("p_lts_n8", REPO / "scripts/n8-run.py")
    n8 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(n8)
    runner = n8.s1.runner
    runner.PREFLIGHT_MEMORY_MBYTES = a.admission_mib
    runner.RUNTIME_MEMORY_MBYTES = a.runtime_mib
    with SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[p-lts4 request] owner=p-lts4 label={a.label} <= {a.minutes:g} min; "
                f"after-owner={a.after_owner or 'none'}, quiet={a.quiet}, "
                f"admission={a.admission_mib} MiB, runtime={a.runtime_mib} MiB. {runner.utc()}\n")
    original_save = runner.save

    def save(path, value):
        if path == runner.LOCK / "owner.json":
            if order() != directive:
                runner.LOCK.rmdir()
                raise RuntimeError("root order changed before admission")
            value["root_order"] = directive
            value["admission_mib"] = a.admission_mib
            if a.quiet:
                with SCHEDULE.open("a", encoding="utf-8") as f:
                    f.write(f"\n[p-lts4 QUIET active] owner=p-lts4 label={a.label} QUIET active; no foreign builds until owner release.\n")
        if (path.name.endswith("-preflight.json") and not value["games"]
                and not value["gpu_processes"] and value["available_mbytes"] >= a.admission_mib
                and not (a.quiet and value["builds"])):
            with SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\np-lts4 {a.label} admitted {runner.utc()} ({a.admission_mib} MiB admission passed).\n")
        original_save(path, value)

    runner.save = save
    deadline = time.monotonic() + a.handoff_seconds
    def released():
        if not a.after_owner:
            return True
        following = False
        with SCHEDULE.open(encoding="utf-8") as schedule:
            for line in schedule:
                if line.rstrip("\r\n") == directive:
                    following = True
                elif following and re.match(
                    rf"^{re.escape(a.after_owner)}(?: [A-Za-z0-9_-]+)? window released(?: |$)", line
                ):
                    return True
        return False

    compiler_quiet_since = [None]
    def builds_running():
        active = bool(runner.ps("ConvertTo-Json -InputObject @(Get-Process | Where-Object "
                              "{ $_.ProcessName -in @('cargo','rustc','clippy-driver','link','unrealbuildtool') } "
                              "| Select-Object Id) -Compress"))
        if active:
            compiler_quiet_since[0] = None
            return True
        if compiler_quiet_since[0] is None:
            compiler_quiet_since[0] = time.monotonic()
        return time.monotonic() - compiler_quiet_since[0] < 120

    while (runner.LOCK.exists() or Path("D:/Coding/renderlab-work/build-slot.lock").exists() or not released()
           or runner.resource_sample()["available_mbytes"] < a.admission_mib
           or builds_running()):
        if order() != directive:
            raise RuntimeError("root order changed during handoff")
        if time.monotonic() >= deadline:
            raise RuntimeError("bounded handoff expired")
        time.sleep(0.25)
    command = a.command[1:] if a.command[:1] == ["--"] else a.command
    sys.argv = [str(REPO / "scripts/n8-run.py"), "--label", a.label,
                "--minutes", str(a.minutes), "--wait-seconds", "45", "--record-resources", "--", *command]
    if a.quiet:
        sys.argv.insert(-len(command)-1, "--no-build-contention")
    try:
        return runner.main()
    finally:
        if n8.s1.claimed:
            with SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\np-lts4 {a.label} window released {runner.utc()} (no owned GPU/build job remains).\n")


if __name__ == "__main__":
    raise SystemExit(main())
