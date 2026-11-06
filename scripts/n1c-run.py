"""N1c's guarded GPU lease: S1's hidden-process launcher, the shared atomic lock, the
gpu-schedule.md reservation lines and the live 4/2 GiB memory guard (--fast-check).
Processes the leased command starts (n1c-batch.py's test binaries) are owned, not foreign.

    py -3 -B scripts/n1c-run.py --label n1c-batch --minutes 8 --fast-check -- \
        py -3 -B scripts/n1c-batch.py <test binary> <root> quarry-no-rt
"""
import importlib.util
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("s1_runner", Path(__file__).with_name("bench-contact-fold-run.py"))
s1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(s1)
OWNER = "n1c-lod-visibility"
s1.runner.OUT = Path("E:/renderlab-work/n1c")
s1.runner.OWNER = OWNER
original_state = s1.runner.state


def state(owned=()):
    # As bench-motion-run.py: snapshot descendants after the original query, since the
    # driver can start a test binary between queries.
    value = original_state(owned)
    processes = s1.runner.ps("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Json -Compress")
    descendants = set(owned)
    while True:
        found = {p["ProcessId"] for p in processes if p["ParentProcessId"] in descendants}
        if found <= descendants:
            break
        descendants |= found
    live = {p["ProcessId"] for p in processes}
    for key in ("games", "gpu_processes", "builds"):
        value[key] = [p for p in value[key] if p["ProcessId"] not in descendants and p["ProcessId"] in live]
    return value


s1.runner.state = state

if __name__ == "__main__":
    try:
        raise SystemExit(s1.runner.main())
    finally:
        if s1.claimed:
            with s1.SCHEDULE.open("a", encoding="utf-8") as f:
                label = sys.argv[sys.argv.index("--label") + 1]
                f.write(f"\n{OWNER} {label} window released {s1.runner.utc()} (no owned GPU/build job remains).\n")

