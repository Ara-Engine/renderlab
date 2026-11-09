"""P-NRT's bounded GPU lease through scripts/n8-run.py (owner `m7-p-nrt`, root's name for this unit).

    py -3 -B tools/p-nrt/lease.py order
    py -3 -B tools/p-nrt/lease.py --expected-order HASH --label UNIQUE --minutes 12 \
        --wait-seconds 900 -- py -3 -B tools/p-nrt/plan.py ...

The latest anchored `[root order]` line must still hash to --expected-order when the
lock is taken. The lock is awaited in-process at 50 ms, bounded by --wait-seconds; a
changed order withdraws the waiter. 6 GiB available RAM at admission, 2 GiB runtime
guard. Other units' `build-script-build.exe` is not counted as a foreign renderer.
"""
import argparse
import importlib.util
import json
import os
import re
from pathlib import Path
import sys
import time

REPO = Path(__file__).resolve().parents[2]
ROOT = Path("D:/Coding/renderlab-work/p-nrt")
OWNER = "m7-p-nrt"
sys.path.insert(0, str(REPO / "tools/ara1"))
from lease import order_state  # noqa: E402  (ARA1's anchored-order reader)


def main():
    if sys.argv[1:] == ["order"]:
        state = order_state()
        print(json.dumps(dict(line=state["line"], sha256=state["sha256"]), indent=2))
        return 0
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--expected-order", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--minutes", type=float, default=12)
    p.add_argument("--wait-seconds", type=float, default=0)
    p.add_argument("--after", action="append", default=[],
                   help="regex anchored at line start, e.g. '(m7-)?p-shc .*released': also wait for such a"
                        " line after the latest root order (never a phrase quoted inside another line)")
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if not 0 < a.minutes <= 15:
        p.error("P-NRT leases are at most 15 minutes")
    spec = importlib.util.spec_from_file_location("pnrt_n8", REPO / "scripts/n8-run.py")
    n8 = importlib.util.module_from_spec(spec)
    os.environ.update(N8_OWNER=OWNER, N8_OUT=str(ROOT / "lease"))
    spec.loader.exec_module(n8)
    runner = n8.s1.runner
    runner.PREFLIGHT_MEMORY_MBYTES = 6144
    runner.RUNTIME_MEMORY_MBYTES = 2048
    wrapped = runner.state

    def state(owned=()):
        value = wrapped(owned)
        value["gpu_processes"] = [q for q in value["gpu_processes"] if q["Name"].lower() != "build-script-build.exe"]
        return value

    runner.state = state

    def current():
        s = order_state()
        if s["sha256"] != a.expected_order:
            raise RuntimeError("root order changed: " + s["line"])

    current()

    def released():
        try:
            lines = n8.s1.SCHEDULE.read_text(encoding="utf-8", errors="replace").splitlines()
        except PermissionError:  # another owner is appending
            return False
        orders = [i for i, line in enumerate(lines) if line.startswith("[root order]")]
        after = lines[orders[-1] + 1:] if orders else lines
        return all(any(re.match(gate, line) for line in after) for gate in a.after)

    # The lock is polled at 50 ms; the (large) schedule once a second.
    deadline = time.monotonic() + a.wait_seconds
    gates, checked = released(), time.monotonic()
    while runner.LOCK.exists() or not gates:
        if time.monotonic() >= deadline:
            print("DEFERRED: GPU lock occupied", flush=True)
            return 3
        time.sleep(.05)
        if time.monotonic() - checked >= 1:
            current()
            gates, checked = released(), time.monotonic()
    current()
    original_save = runner.save

    def save(path, value):
        if path == runner.LOCK / "owner.json":
            try:
                current()
            except RuntimeError:
                runner.LOCK.rmdir()
                raise
            value["root_order_sha256"] = a.expected_order
        original_save(path, value)

    runner.save = save
    command = a.command[1:] if a.command[:1] == ["--"] else a.command
    if not command:
        p.error("command required")
    sys.argv = [str(REPO / "scripts/n8-run.py"), "--label", a.label, "--minutes", str(a.minutes), "--record-resources", "--", *command]
    try:
        return runner.main()
    finally:
        if n8.s1.claimed:
            with n8.s1.SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\n{OWNER} {a.label} window released {runner.utc()} (no owned GPU/build job remains).\n")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as e:
        print("DEFERRED:", e, flush=True)
        raise SystemExit(3)
