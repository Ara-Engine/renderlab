"""P-RTS guarded CPU builds: run cargo commands in order, and kill them if a newer
[root order] or a p-shc/compiler-quiet reservation appears after the order read at start.

    build.py LOG -- DIR TARGET "cargo args" ["cargo args" ...]
"""
import os
from pathlib import Path
import subprocess
import sys
import time

SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")


def lines():
    for _ in range(20):
        try:
            return SCHEDULE.read_text(encoding="utf-8").splitlines()
        except PermissionError:
            time.sleep(0.2)
    return []


def main():
    log, directory, target, *commands = [a for a in sys.argv[1:] if a != "--"]
    start = len(lines())
    env = os.environ | {"CARGO_TARGET_DIR": target, "CARGO_BUILD_JOBS": "2"}
    with open(log, "w", encoding="utf-8") as out:
        for command in commands:
            child = subprocess.Popen(["cargo", *command.split(), "-j", "2"], cwd=directory, env=env,
                                     stdout=out, stderr=subprocess.STDOUT)
            while child.poll() is None:
                new = lines()[start:]
                stop = [l for l in new if l.startswith("[root order]") or l.startswith("[p-shc reservation]")
                        or "COMPILER-QUIET" in l.upper() and l.startswith("[root order]")]
                if stop:
                    subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True)
                    print("STOPPED for schedule line:", stop[0][:200], flush=True)
                    return 3
                time.sleep(1)
            print(command, "exit", child.returncode, flush=True)
            if child.returncode:
                return child.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
