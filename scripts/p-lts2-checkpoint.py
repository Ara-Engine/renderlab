"""Retained candidate image gates and one 1080p timing pair, inside the lease."""
from pathlib import Path
import subprocess
import sys

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
label, variant = sys.argv[1:3]
fixture_steps = "fixture,mixed,game-still,game-move"
if "--quarry" in sys.argv[3:]:
    fixture_steps += ",quarry"
commands = [
    ["scripts/p-lts2-batch.py", label + "-quality", "--binary", str(ROOT / f"bin/{variant}-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--resolution", "quality"],
    ["scripts/p-lts2-batch.py", label + "-edge", "--binary", str(ROOT / f"bin/{variant}-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--scene-dir", str(ROOT / "edge-scenes"), "--cases", "odd-edge"],
    ["scripts/p-lts2-fixtures.py", label + "-fixtures", "--candidate", str(ROOT / f"bin/{variant}-tests.exe"),
     "--game", str(ROOT / f"bin/{variant}-game.exe"), "--steps", fixture_steps],
    ["scripts/p-lts2-batch.py", label + "-timing", "--binary", str(ROOT / f"bin/{variant}-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--steady-warmup", "128"],
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
