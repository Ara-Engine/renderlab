"""PP1: schedule-aware n8 lease, or two-job compile outside QUIET windows.

py -3 -B scripts/m7-pp1-run.py gpu label -- <test binary> <test args>
py -3 -B scripts/m7-pp1-run.py build
"""
import datetime as dt
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/m7-pp1")
os.environ["M7_G1_UNIT"] = "m7-pp1"
spec = importlib.util.spec_from_file_location("g1", ROOT / "scripts/m7-g1-run.py")
g1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g1)
g1.OUT = OUT
g1.TARGET = OUT / "target"
g1.BINARY = OUT / "bin/pp1-tests.exe"
OUT.mkdir(parents=True, exist_ok=True)

if sys.argv[1] == "build":
    g1.build(SimpleNamespace(release=False, beside_leases=True, name="pp1-tests"))
else:
    label = sys.argv[2]
    u = g1.ue()
    with u.SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[m7-pp1 request] owner=m7-pp1 asks for one <=15 min lease {label} (PP1 offscreen lib/effect verification; diagnostic timings); {dt.datetime.now(dt.timezone.utc).isoformat()}\n")
    deadline = time.monotonic() + 90 * 60
    while g1.window_open(u) or g1.quiet_active(u, any_lease=False) or g1.hold_state(u) == "hold":
        if time.monotonic() > deadline:
            raise SystemExit("PP1 timed out waiting for schedule/game hold")
        time.sleep(1)
    command = sys.argv[3:]
    if command[:1] == ["--"]:
        command = command[1:]
    env = dict(os.environ, N8_OWNER="m7-pp1", N8_OUT=str(OUT))
    raise SystemExit(subprocess.call([sys.executable, "-B", str(ROOT / "scripts/n8-run.py"),
        "--label", label, "--minutes", "15", "--wait-seconds", "45", "--fast-check", "--", *command], cwd=ROOT, env=env))
