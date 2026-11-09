"""One final timing pair followed by the full-quarry image pairs (8 GiB admission)."""
from pathlib import Path
import importlib.util
import subprocess
import sys
import time

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
label, variant = sys.argv[1:3]
spec = importlib.util.spec_from_file_location("plts2_resources", Path(__file__).with_name("m6-film-run.py"))
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)
# The root's compiler hold begins at the actual admitted line. Allow active
# compilers to finish/stop before the final timing prefix, bounded within this lease.
deadline = time.monotonic() + 45
while resources.resource_sample()["rustc_count"] and time.monotonic() < deadline:
    time.sleep(0.25)
commands = [
    ["scripts/p-lts2-batch.py", label + "-timing", "--binary", str(ROOT / f"bin/{variant}-capture.exe"),
     "--baseline-binary", str(ROOT / "bin/base-capture.exe"), "--steady-warmup", "128"],
    ["scripts/p-lts2-fixtures.py", label + "-quarry", "--candidate", str(ROOT / f"bin/{variant}-tests.exe"),
     "--steps", "quarry"],
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
