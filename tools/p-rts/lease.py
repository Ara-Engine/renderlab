"""P-RTS's bounded GPU lease: scripts/n8-run.py under owner p-rts, after the latest
anchored root order (tools/ara1/lease.py's reader) is re-read and unchanged.

    py -3 -B tools/p-rts/lease.py order
    py -3 -B tools/p-rts/lease.py --expected-order HASH --label UNIQUE --minutes 14 -- CMD...

Admission needs 6 GiB available; the live guard stops the step below 2 GiB. The lock
wait polls in-process (0.25 s) for at most --wait-minutes; it never queues a launcher.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

REPO = Path(__file__).resolve().parents[2]
ROOT = Path("D:/Coding/renderlab-work/p-rts")
sys.path.insert(0, str(REPO / "tools/ara1"))
from lease import SCHEDULE, check_order, order_state, released_after  # noqa: E402


def released(after):
    """True once, after the latest root order, a release line *starts with* one of the
    owners in `after` (OWNER LABEL window released ...). Quoted text never matches."""
    return not after or any(released_after(order_state(), owner) for owner in after)


def main():
    if sys.argv[1:] == ["order"]:
        state = order_state()
        print(json.dumps(dict(line=state["line"], sha256=state["sha256"]), indent=2))
        return 0
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--expected-order", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--minutes", type=float, default=14)
    p.add_argument("--wait-minutes", type=float, default=0)
    p.add_argument("--after-owner", action="append", default=[],
                   help="hold until this owner's own release line follows the order (any of several spellings)")
    p.add_argument("command", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if not 0 < a.minutes <= 15:
        p.error("P-RTS leases are at most 15 minutes")
    check_order(a.expected_order)
    os.environ.update(N8_OWNER="p-rts", N8_OUT=str(ROOT / "lease"), P_RTS_EXPECTED_ORDER=a.expected_order)
    spec = importlib.util.spec_from_file_location("p_rts_n8", REPO / "scripts/n8-run.py")
    n8 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(n8)
    n8.s1.runner.PREFLIGHT_MEMORY_MBYTES = 6144
    n8.s1.runner.RUNTIME_MEMORY_MBYTES = 2048
    deadline = time.monotonic() + a.wait_minutes * 60
    while n8.s1.runner.LOCK.exists() or not released(a.after_owner):
        check_order(a.expected_order)
        if time.monotonic() >= deadline:
            print("DEFERRED: GPU lock occupied", flush=True)
            return 3
        time.sleep(0.25)
    original_save = n8.s1.runner.save

    def save(path, value):
        if path == n8.s1.runner.LOCK / "owner.json":
            try:
                check_order(a.expected_order)
                if not released(a.after_owner):
                    raise RuntimeError("hold owner has not released after the latest root order")
            except RuntimeError:
                n8.s1.runner.LOCK.rmdir()
                raise
        original_save(path, value)

    n8.s1.runner.save = save
    command = a.command[1:] if a.command[:1] == ["--"] else a.command
    sys.argv = [str(REPO / "scripts/n8-run.py"), "--label", a.label, "--minutes", str(a.minutes),
                "--record-resources", "--wait-seconds", "45", "--", *command]
    try:
        return n8.s1.runner.main()
    finally:
        if n8.s1.claimed:
            with SCHEDULE.open("a", encoding="utf-8") as f:
                f.write(f"\np-rts {a.label} window released {n8.s1.runner.utc()} (no owned GPU/build job remains).\n")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as e:
        print("DEFERRED:", e, flush=True)
        raise SystemExit(3)
