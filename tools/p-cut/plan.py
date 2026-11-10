"""P-CUT lease plans (each runs inside one n8-run lease of at most 15 minutes, via host.py lease).

    py -3 -B tools/p-cut/plan.py quarry    # quarry timing (2 visits) and post-cut captures, both paths, ramp and one-frame
    py -3 -B tools/p-cut/plan.py game      # ARA1 game-base, both paths, ramp and one-frame, 2 visits
    py -3 -B tools/p-cut/plan.py lib       # the dev lib-test binary built by host.py (P_YARD_BUILD=lib-tests)

Steps that already finished are skipped, so a stopped lease resumes. GPU steps come first.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
WORK = Path(os.environ.get("P_YARD_WORK", "D:/Coding/renderlab-work/p-cut"))
PY = [sys.executable, "-B"]


def quarry():
    os.environ["P_YARD_CAPTURE_FROM"] = "316"
    steps = [f"quarry:timing:{p}:{v}" for v in (0, 1) for p in ("rt", "rt-base", "nonrt", "nonrt-base")]
    steps += [f"quarry:capture:{p}" for p in ("rt", "rt-base", "nonrt", "nonrt-base")]
    return subprocess.run(PY + [str(REPO / "tools/p-yard/rl.py"), *steps]).returncode


def game():
    return max(subprocess.run(PY + [str(HERE / "game.py"), str(v)]).returncode for v in (0, 1))


def lib():
    exe = json.loads((WORK / "lib-tests.json").read_text())["executable"]
    with (WORK / "lib-tests.log").open("w") as log:
        code = subprocess.run([exe, "-q", "--test-threads", "2"], cwd=REPO, stdout=log, stderr=subprocess.STDOUT).returncode
    print("lib exit", code, (WORK / "lib-tests.log").read_text(errors="replace")[-1500:])
    return code


if __name__ == "__main__":
    raise SystemExit({"quarry": quarry, "game": game, "lib": lib}[sys.argv[1]]())
