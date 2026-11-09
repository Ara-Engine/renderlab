"""Clean prebuilt timing repeats, including the unchanged soft quarry default."""
from pathlib import Path
import subprocess
import sys

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
label = sys.argv[1]
commands = [
    ["scripts/p-lts2-batch.py", label + "-a-unshadowed", "--binary", str(ROOT / "bin/a-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--cases", "game-lights-unshadowed", "--steady-warmup", "128"],
    ["scripts/p-lts2-batch.py", label + "-d", "--binary", str(ROOT / "bin/d-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/a-capture.exe"), "--steady-warmup", "128"],
    ["scripts/p-lts2-fixtures.py", label + "-quarry", "--candidate", str(ROOT / "bin/a-tests.exe"),
     "--steps", "quarry", "--skip-quarry-controls"],
]
for index, command in enumerate(commands):
    path = ROOT / f"{label}-step-{index}.log"
    print("STEP", index, command, flush=True)
    with path.open("w") as log:
        result = subprocess.run([sys.executable, "-B", *command], stdout=log, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
    print(path.read_text()[-2500:], flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
