"""P-FLICKER merge gate, inside this unit's lease: run.py STEPs first (GPU), then the lib tests
from the prebuilt test binary (host.py build lib), last, as the lease plans of other units do.

    py -3 -B tools/p-flicker/host.py lease LABEL 15 -- py -3 -B tools/p-flicker/gate.py STEP...
"""
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
UNIT = Path("D:/Coding/renderlab-work/p-flicker")  # Unreal captures and the clip manifest
WORK = Path(os.environ.get("P_FLICKER_WORK", UNIT))  # runs, sheets, builds (another unit sets its own)

if __name__ == "__main__":
    code = subprocess.call([sys.executable, "-B", str(HERE / "run.py"), *sys.argv[1:]]) if sys.argv[1:] else 0
    env = os.environ | {"CARGO_TARGET_DIR": str(WORK / "target"), "CARGO_BUILD_JOBS": "2",
                        "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    with (WORK / "lib-tests.log").open("w") as log:
        lib = subprocess.call(["cargo", "test", "--locked", "-j", "2", "--lib", "--", "--test-threads", "2"],
                              cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
    print("run.py exit", code, "lib tests exit", lib, flush=True)
    raise SystemExit(code or lib)
