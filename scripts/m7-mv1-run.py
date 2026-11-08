"""MV1 two-job compile or schedule/game-aware n8 lease. Bulk stays on D:."""
import importlib.util
import datetime as dt
import os
from pathlib import Path
import subprocess
import sys
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/m7-mv1")
os.environ["M7_G1_UNIT"] = "m7-mv1"
spec = importlib.util.spec_from_file_location("g1", ROOT / "scripts/m7-g1-run.py")
g1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g1)
g1.OUT = OUT
g1.TARGET = OUT / "target"
g1.BINARY = OUT / "bin/mv1-tests.exe"
OUT.mkdir(parents=True, exist_ok=True)

if sys.argv[1] == "build":
    g1.build(SimpleNamespace(release=False, beside_leases=True, name="mv1-tests"))
elif sys.argv[1] == "example":
    u = g1.ue()
    if g1.quiet_active(u, any_lease=False) or g1.hold_state(u) == "hold":
        raise SystemExit("MV1 example compile deferred for QUIET/game hold")
    with (OUT / "example-build.log").open("w") as log:
        proc = subprocess.Popen(["cargo", "build", "-p", "renderlab", "--example", "game_scene",
            "-j", "2", "--target-dir", str(g1.TARGET)], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        while proc.poll() is None:
            time.sleep(2)
            if g1.quiet_active(u, any_lease=False) or g1.hold_state(u) == "hold":
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
                proc.wait()
                raise SystemExit("MV1 example compile stopped for QUIET/game hold")
        raise SystemExit(proc.returncode)
else:
    label = sys.argv[2]
    command = sys.argv[3:]
    if command[:1] == ["--"]:
        command = command[1:]
    u = g1.ue()
    with u.SCHEDULE.open("a", encoding="utf-8") as f:
        f.write(f"\n[m7-mv1 request] owner=m7-mv1 asks for <=15 min lease {label} (lib + moving-object fixture/quarry on both paths; headless, diagnostic timing); {dt.datetime.now(dt.timezone.utc).isoformat()}\n")
    deadline = time.monotonic() + 75 * 60
    attempt = 0
    while time.monotonic() < deadline:
        reservation = g1.window_open(u)
        if (g1.quiet_active(u, any_lease=False) or g1.hold_state(u) == "hold"
                or (reservation and "quiet" in reservation.lower())):
            time.sleep(2)
            continue
        attempt += 1
        result = subprocess.call([sys.executable, "-B", str(ROOT / "scripts/n8-run.py"),
            "--label", f"{label}-{attempt}", "--minutes", "15", "--wait-seconds", "45", "--fast-check", "--", *command],
            cwd=ROOT, env=dict(os.environ, N8_OWNER="m7-mv1", N8_OUT=str(OUT)))
        if result != 3:
            raise SystemExit(result)
        time.sleep(2)
    raise SystemExit("MV1 lease timed out; no owned job remains")
