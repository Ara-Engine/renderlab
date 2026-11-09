"""Short paired screens; keep failed candidates as excluded evidence."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
label = sys.argv[1]
base = ROOT / "bin/base-capture.exe"
steps = [
    ("profile", ["scripts/p-lts2-profile.py", label, "--binary", str(ROOT / "bin/rl-capture.exe"), "--path", "nonrt", "--sample"]),
    ("a", ["scripts/p-lts2-batch.py", label + "-a", "--binary", str(ROOT / "bin/a-capture.exe"), "--baseline-binary", str(base), "--steady-warmup", "128"]),
    ("b", ["scripts/p-lts2-batch.py", label + "-b", "--binary", str(ROOT / "bin/b-capture.exe"), "--baseline-binary", str(base), "--steady-warmup", "128"]),
    ("c", ["scripts/p-lts2-batch.py", label + "-c", "--binary", str(ROOT / "bin/c-capture.exe"), "--baseline-binary", str(base), "--steady-warmup", "128"]),
    ("brdf", ["scripts/p-lts2-batch.py", label + "-brdf", "--binary", str(base), "--baseline-binary", str(base), "--candidate-standard", str(ROOT / "brdf-prepared-standard.wgsl"), "--steady-warmup", "128"]),
    ("boundary", ["scripts/p-lts2-batch.py", label + "-boundary", "--binary", str(ROOT / "bin/b-capture.exe"), "--baseline-binary", str(base), "--scene-dir", str(ROOT / "boundary-scenes"), "--cases", "lights-128,lights-129"]),
]
if "--skip-profile" in sys.argv[2:]:
    steps = steps[1:]
results = []
for name, command in steps:
    print("STEP", name, flush=True)
    log_path = ROOT / f"{label}-{name}.log"
    with log_path.open("w") as log:
        result = subprocess.run([sys.executable, "-B", *command], stdout=log, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
    results.append(dict(step=name, exit=result.returncode))
    (ROOT / f"{label}-steps.json").write_text(json.dumps(results, indent=2))
    print("RESULT", results[-1], flush=True)
    if result.returncode:
        print(log_path.read_text()[-2000:], flush=True)
