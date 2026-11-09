"""Block until each named owner writes its own release line after the latest root order.

    hold.py MAX_MINUTES OWNER [OWNER ...]   (exit 0 released, 3 timeout, 4 order changed)
"""
import re
import sys
import time
from pathlib import Path

SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def read():
    for _ in range(20):
        try:
            return SCHEDULE.read_text(encoding="utf-8").splitlines()
        except PermissionError:
            time.sleep(0.2)
    return []


minutes, owners = float(sys.argv[1]), sys.argv[2:]
lines = read()
order = max(i for i, l in enumerate(lines) if l.startswith("[root order]"))
first = lines[order]
deadline = time.time() + minutes * 60
while time.time() < deadline:
    lines = read()
    latest = max(i for i, l in enumerate(lines) if l.startswith("[root order]"))
    if lines[latest] != first:
        print("ORDER CHANGED:", lines[latest][:300], flush=True)
        raise SystemExit(4)
    after = lines[latest + 1:]
    done = {o for o in owners if any(re.match(rf"{re.escape(o)} \S+ window released ", l) for l in after)}
    if done == set(owners):
        print("released:", sorted(done), time.strftime("%H:%M:%S", time.gmtime()), flush=True)
        raise SystemExit(0)
    time.sleep(2)
print("TIMEOUT", flush=True)
raise SystemExit(3)
