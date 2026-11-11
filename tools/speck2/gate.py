"""U-SPECK2 gate lease: the flights first (GPU steps first), then the lib suite, in one lease.
The exit status is the worse of the two; the lib log keeps libtest's `test result` line.

    py -3 -B tools/speck2/gate.py LIB_EXE LIB_LOG OUT name=EXE[,...] ...
"""
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck2")


def main():
    lib, log, flights = sys.argv[1], Path(sys.argv[2]), sys.argv[3:]
    worst = subprocess.run([sys.executable, "-B", str(REPO / "tools/speck2/flights.py"), *flights], cwd=REPO).returncode
    env = os.environ | {"TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    with log.open("w") as out:
        code = subprocess.run([lib, "--test-threads", "2"], cwd=REPO, env=env, stdout=out, stderr=subprocess.STDOUT).returncode
    lines = [l for l in log.read_text(errors="replace").splitlines() if l.startswith("test result") or "FAILED" in l or "panicked" in l]
    print(f"lib suite exit {code}:", *lines[-12:], sep="\n", flush=True)
    return max(worst, code)


if __name__ == "__main__":
    raise SystemExit(main())
