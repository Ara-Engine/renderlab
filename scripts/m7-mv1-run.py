"""MV1 two-job compile or schedule/game-aware n8 lease. Bulk stays on D:."""
import importlib.util
import datetime as dt
import ctypes
import json
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

class MemoryStatus(ctypes.Structure):
    _fields_ = [("length", ctypes.c_uint32), ("load", ctypes.c_uint32),
                ("total", ctypes.c_uint64), ("available", ctypes.c_uint64),
                ("unused", ctypes.c_uint64 * 5)]

def available_mb():
    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise ctypes.WinError()
    return status.available // (1024 * 1024)

def turn_ready(u):
    """Acknowledge the exact latest order and count only subsequent releases."""
    lines = u.SCHEDULE.read_text(encoding="utf-8").splitlines()
    order_index = next((i for i in range(len(lines)-1, -1, -1)
                        if lines[i].startswith("[root order]")), None)
    if order_index is None:
        return True
    gate = OUT / "gpu-after.json"
    acknowledged = json.loads(gate.read_text()) if gate.exists() else {}
    if acknowledged.get("root_order") != lines[order_index]:
        (OUT / "latest-root-order.txt").write_text(lines[order_index])
        return False
    releases = [line for line in lines[order_index+1:]
                if "window released" in line and not line.startswith("[")]
    return all(sum(line.startswith(item["owner"] + " ") for line in releases) >= item["count"]
               for item in acknowledged.get("requires", []))

def main():
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
        if not turn_ready(u) or available_mb() < 8192 or Path("D:/Coding/renderlab-work/gpu-slot.lock").exists():
            time.sleep(2)
            continue
        reservation = g1.window_open(u)
        if (g1.quiet_active(u, any_lease=False) or g1.hold_state(u) == "hold"
                or (reservation and "quiet" in reservation.lower())):
            time.sleep(2)
            continue
        attempt += 1
        result = subprocess.call([sys.executable, "-B", str(ROOT / "scripts/n8-run.py"),
            "--label", f"{label}-{attempt}", "--minutes", "15", "--wait-seconds", "0", "--fast-check", "--", *command],
            cwd=ROOT, env=dict(os.environ, N8_OWNER="m7-mv1", N8_OUT=str(OUT)))
        if result != 3:
            raise SystemExit(result)
        time.sleep(2)
    raise SystemExit("MV1 lease timed out; no owned job remains")

if __name__ == "__main__":
    main()
