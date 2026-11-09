"""Retest A's contended rows, then screen the fixed combined D prototype."""
import importlib.util
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
label = sys.argv[1]
spec = importlib.util.spec_from_file_location("plts2_resources", Path(__file__).with_name("m6-film-run.py"))
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)
deadline = time.monotonic() + 45
while resources.resource_sample()["rustc_count"] and time.monotonic() < deadline:
    time.sleep(0.25)
commands = [
    ["scripts/p-lts2-batch.py", label + "-a-unshadowed", "--binary", str(ROOT / "bin/a-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--cases", "game-lights-unshadowed", "--steady-warmup", "128"],
    ["scripts/p-lts2-batch.py", label + "-d", "--binary", str(ROOT / "bin/d-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/a-capture.exe"), "--steady-warmup", "128"],
    ["scripts/p-lts2-batch.py", label + "-boundary", "--binary", str(ROOT / "bin/d-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--scene-dir", str(ROOT / "boundary-scenes-v2"),
     "--cases", "lights-128,lights-129"],
    ["scripts/p-lts2-profile.py", label, "--binary", str(ROOT / "bin/d-capture.exe"), "--path", "nonrt", "--sample"],
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
    if index == 2:
        with Path("E:/renderlab-work/gpu-schedule.md").open("a", encoding="utf-8") as schedule:
            schedule.write("\np-lts2 " + label + " compiler-quiet prefix complete "
                           + datetime.now(timezone.utc).isoformat() + " (paired render timings complete).\n")
