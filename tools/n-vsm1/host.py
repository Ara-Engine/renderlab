"""N-VSM1 host discipline: tools/p-relit/host.py under this unit's owner, work folder and Cargo
target (schedule lines, root-order checks, the atomic build slot, GPU leases through
scripts/n8-run.py with its memory watchdog).

    py -3 -B tools/n-vsm1/host.py build NAME [TREE]   # release examples of TREE into bin/NAME
                                                      # (NAME lib-...: the lib test binary; clippy: lint)
    py -3 -B tools/n-vsm1/host.py lease LABEL MIN -- COMMAND...   # LABEL q-...: a quarry lease (14 GiB)
"""
import importlib.util
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/n-vsm1")


def load():
    spec = importlib.util.spec_from_file_location("nvsm1_host", HERE.parent / "p-relit/host.py")
    host = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(host)
    host.OWNER, host.WORK, host.TARGET = "n-vsm1", WORK, WORK / "target"
    return host


if __name__ == "__main__":
    host = load()
    if sys.argv[1] == "build":
        raise SystemExit(host.build(*sys.argv[2:4]))
    if sys.argv[1] == "lease":
        i = sys.argv.index("--")
        raise SystemExit(host.lease(sys.argv[2], float(sys.argv[3]), sys.argv[i + 1:]))
    if sys.argv[1] == "ready":
        print(f"{host.free_gib():.1f} GiB free", host.ready(host.runner(), sys.argv[2] if len(sys.argv) > 2 else "check"))
