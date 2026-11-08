"""AT1: start one plan on the shared GPU lease in-process the moment the schedule allows (as
scripts/m7-tr1-wait.py, owner m7-at1, evidence on D:/Coding/renderlab-work/at1). The request
line is posted separately; this honours others' reservations, QUIET windows, requests and game
holds exactly as scripts/m7-g1-run.py's `plan`, and gives up after --wait-minutes.

  py -3 -B scripts/at1-wait.py <label> <plan.json> [--minutes 25] [--wait-minutes 120]
"""
import argparse
import importlib.util
import sys
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location("at1_run", Path(__file__).with_name("at1-run.py"))
at1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(at1)
runner = at1.runner


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("label")
    parser.add_argument("plan")
    parser.add_argument("--minutes", type=int, default=25)
    parser.add_argument("--wait-minutes", type=float, default=120)
    args = parser.parse_args()
    u = runner.ue()
    deadline = time.monotonic() + args.wait_minutes * 60
    plan_args = argparse.Namespace(label=args.label, plan=args.plan, minutes=args.minutes, spp=2048, paths="rt,nonrt",
                                   after="", check_only=False, quiet=False, force_schedule=False)
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
