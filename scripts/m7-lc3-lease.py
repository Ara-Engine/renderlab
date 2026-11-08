"""Block for LC3's short n8-run.py lease; stop if the reviewed root order changes.

Review the latest [root order] and its release lines first. Pass the exact reviewed
line in LC3_ROOT_ORDER. Leases are <=15 minutes. No test binary starts outside n8.
"""
import datetime
import os
from pathlib import Path
import subprocess
import sys
import time
import importlib.util

REPO = Path(__file__).resolve().parents[1]
SCHEDULE = Path("E:/renderlab-work/gpu-schedule.md")
OUT = Path("D:/Coding/renderlab-work/lc3")


def latest_order():
    return next((line for line in reversed(SCHEDULE.read_text(encoding="utf-8").splitlines())
                 if line.startswith("[root order]")), "")


def main():
    label, *command = sys.argv[1:]
    reviewed = os.environ["LC3_ROOT_ORDER"]
    env = dict(os.environ, N8_OWNER="m7-lc3", N8_OUT=str(OUT))
    with SCHEDULE.open("a", encoding="utf-8") as log:
        log.write(f"\n[m7-lc3 request] owner=m7-lc3 label={label} <=15 min headless, functional and diagnostic timing; yields to root orders. {datetime.datetime.now(datetime.timezone.utc).isoformat()}\n")
    deadline = time.monotonic() + 45 * 60
    spec = importlib.util.spec_from_file_location("lc3_resources", REPO / "scripts/m6-film-run.py")
    resources = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(resources)
    attempt = 0
    while time.monotonic() < deadline:
        # Re-read the actual directive before EVERY lease attempt. A new directive
        # requires the agent to inspect its dependencies, never automatic approval.
        if latest_order() != reviewed:
            raise SystemExit("DEFERRED: latest root order changed; review it before another lease")
        marker = os.environ.get("LC3_WAIT_MARKER")
        marker_lines = SCHEDULE.read_text(encoding="utf-8").splitlines()
        marker_lines = marker_lines[marker_lines.index(reviewed) + 1:]
        if marker and not any(line == marker or line.startswith(marker + " ") for line in marker_lines):
            time.sleep(1)
            continue
        # Optional dependencies from the reviewed directive, e.g. an owner's two
        # requested dry runs. Count only releases after that exact directive.
        owner = os.environ.get("LC3_WAIT_RELEASE_OWNER")
        if owner:
            lines = SCHEDULE.read_text(encoding="utf-8").splitlines()
            later = lines[lines.index(reviewed) + 1:]
            count = sum(line.startswith(owner + " ") and "window released" in line for line in later)
            if count < int(os.environ.get("LC3_WAIT_RELEASE_COUNT", "1")):
                time.sleep(1)
                continue
        floor = float(os.environ.get("LC3_PREFLIGHT_MB", "4096"))
        if resources.resource_sample()["available_mbytes"] < floor:
            time.sleep(1)
            continue
        attempt += 1
        result = subprocess.run([sys.executable, "-B", "scripts/n8-run.py", "--label",
            f"{label}-{attempt}", "--minutes", "15", "--fast-check", "--", *command],
            cwd=REPO, env=env, creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode != 3:
            raise SystemExit(result.returncode)
        time.sleep(1)
    raise SystemExit("DEFERRED: bounded lease queue expired")


if __name__ == "__main__":
    main()
