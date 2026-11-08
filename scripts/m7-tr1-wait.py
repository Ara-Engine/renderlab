"""M7-TR1: queue one plan on the shared GPU lease and start it in-process the moment the
schedule allows (shell retry loops lose the hand-off; docs/m7-g1b.md). Posts a request
line first, honours other owners' reservations, QUIET windows, requests and game holds
exactly as scripts/m7-g1-run.py's `plan` does, and gives up after --wait-minutes.

  py -3 -B scripts/m7-tr1-wait.py <label> <plan.json> [--minutes 14] [--wait-minutes 90]
"""
import argparse
import importlib.util
import os
import sys
import time
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-tr1"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("label")
    parser.add_argument("plan")
    parser.add_argument("--minutes", type=int, default=14)
    parser.add_argument("--wait-minutes", type=float, default=90)
    parser.add_argument("--quiet", action="store_true")
    parser.add_argument("--note", default="headless functional runs, no timing claim, no Cargo inside")
    args = parser.parse_args()
    u = runner.ue()
    kind = "QUIET lease" if args.quiet else "lease"
    u.schedule_line(f"[m7-tr1 request] owner=m7-tr1 asks for one <={args.minutes} min {kind} \"tr1-{args.label}\" "
                    f"({args.note}); takes the lock at the next free moment. {u.utc().isoformat()}")
    deadline = time.monotonic() + args.wait_minutes * 60
    plan_args = argparse.Namespace(label=args.label, plan=args.plan, minutes=args.minutes, spp=2048, paths="rt,nonrt",
                                   after="", check_only=False, quiet=args.quiet, force_schedule=False)
    while time.monotonic() < deadline:
        blocking = runner.window_open(u) or runner.quiet_active(u, any_lease=False)
        if not blocking and runner.hold_state(u) != "hold" and not u.LOCK.exists():
            try:
                runner.plan(plan_args)
                return
            except SystemExit as stop:
                if stop.code != 3:
                    raise
        time.sleep(0.5)
    print("DEFERRED: wait limit reached")
    sys.exit(3)


if __name__ == "__main__":
    main()
