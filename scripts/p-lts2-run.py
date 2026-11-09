"""P-LTS2 GPU lease: <=15 min, >=7 GiB admission, standard 2 GiB runtime guard.

Root's 2026-11-09 amendment permits 7 GiB for short non-quarry runs only.
Pass --admission-mib 8192 for every full-quarry run.

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
OUT = Path("D:/Coding/renderlab-work/p-lts2")
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def order():
    return [line for line in SCHEDULE.read_text(encoding="utf-8").splitlines()
            if line.startswith("[root order]")][-1]


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
    p.add_argument("--admission-mib", type=int, default=7168, help="quarry requires 8192; short runs require at least 7168")
    p.add_argument("--runtime-mib", type=int, choices=[2048], default=2048)
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if not 0 < a.minutes <= 15:
        p.error("lease must be <=15 minutes")
    if a.admission_mib < 7168:
        p.error("admission floor must be >=7168 MiB")
    directive = order()
    print(directive, flush=True)
    os.environ.update(N8_OWNER="p-lts2", N8_OUT=str(OUT / "lease"))
    spec = importlib.util.spec_from_file_location("p_lts_n8", REPO / "scripts/n8-run.py")
    n8 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(n8)
    runner = n8.s1.runner
    runner.PREFLIGHT_MEMORY_MBYTES = a.admission_mib
    runner.RUNTIME_MEMORY_MBYTES = a.runtime_mib
    with SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[p-lts2 request] owner=p-lts2 label={a.label} <= {a.minutes:g} min; "
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
                    f.write(f"\n[p-lts2 QUIET active] owner=p-lts2 label={a.label} QUIET active; no foreign builds until owner release.\n")
        if path.name.endswith("-preflight.json") and not value["games"] and not value["gpu_processes"] and value["available_mbytes"] >= a.admission_mib:
            with SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\np-lts2 {a.label} admitted {runner.utc()} ({a.admission_mib} MiB admission passed).\n")
        original_save(path, value)

    runner.save = save
    deadline = time.monotonic() + a.handoff_seconds
    def released():
        following = SCHEDULE.read_text(encoding="utf-8").split(directive, 1)[1].splitlines()
        return not a.after_owner or any(
            re.match(rf"^{re.escape(a.after_owner)}(?: [A-Za-z0-9_-]+)? window released(?: |$)", line)
            for line in following
        )

    while runner.LOCK.exists() or not released() or (a.quiet and runner.resource_sample()["rustc_count"]) or runner.resource_sample()["available_mbytes"] < a.admission_mib:
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
                f.write(f"\np-lts2 {a.label} window released {runner.utc()} (no owned GPU/build job remains).\n")


if __name__ == "__main__":
    raise SystemExit(main())
