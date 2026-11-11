"""U-SPECK host discipline: tools/p-relit/host.py under this unit's owner, work folder and Cargo
target (schedule lines, root-order checks, the atomic build slot, GPU leases through
scripts/n8-run.py with its memory watchdog).

    py -3 -B tools/speck/host.py build NAME [TREE]   # release examples of TREE into bin/NAME
                                                     # (NAME lib-...: the lib test binary; clippy: lint)
    py -3 -B tools/speck/host.py lease LABEL MIN -- COMMAND...   # LABEL q-...: a quarry lease (14 GiB)
"""
import importlib.util
import inspect
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/unit-speck")


def load():
    spec = importlib.util.spec_from_file_location("speck_host", HERE.parent / "p-relit/host.py")
    host = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(host)
    host.OWNER, host.WORK, host.TARGET = "u-speck", WORK, WORK / "target"
    host.EXAMPLES = ("game_scene", "reflection_matched", "p_yard_capture")
    # Root's away-limits (2026-11-11): builds up to -j 8.
    source = inspect.getsource(host.build).replace('"-j", "2"', '"-j", "8"').replace('"CARGO_BUILD_JOBS": "2"', '"CARGO_BUILD_JOBS": "8"').replace("-j 2 {r.utc()}", "-j 8 {r.utc()}")
    exec(source, host.__dict__)
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
