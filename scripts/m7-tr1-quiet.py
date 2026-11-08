"""M7-TR1: one QUIET plan for a cost claim, after G1b's quiet poller (docs/m7-g1b.md 1.4).

  py -3 -B scripts/m7-tr1-quiet.py <prefix> <plan.json> [--minutes 8] [--attempts 4]

Posts a request line, then takes the shared lock at the first free moment that follows
120 s with no foreign `stillkit` compiler (sampled every 3 s), with no other owner's
reservation, request or QUIET window in force and no game hold. The plan announces its QUIET
window. Any stillkit sample between lock and release labels that attempt contended and the
next attempt runs as <prefix>-<n+1>. Never kills or pauses anything.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-tr1"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
RUNS = Path("E:/renderlab-work/m7-tr1/runs")
QUERY = ("@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(rustc|cargo|clippy-driver|rustdoc)\\.exe$' "
         "-and $_.CommandLine -match 'stillkit' }).Count")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("prefix")
    parser.add_argument("plan")
    parser.add_argument("--minutes", type=int, default=8)
    parser.add_argument("--attempts", type=int, default=4)
    parser.add_argument("--idle", type=float, default=120)
    parser.add_argument("--wait-minutes", type=float, default=150)
    args = parser.parse_args()
    busy, last_busy = [], [time.time()]

    def sampler():
        while True:
            out = subprocess.run(["powershell", "-NoProfile", "-Command", QUERY], capture_output=True, text=True,
                                 creationflags=subprocess.CREATE_NO_WINDOW).stdout
            n = int(out.strip() or "0")
            if n:
                last_busy[0] = time.time()
                busy.append((last_busy[0], n))
            time.sleep(3)

    threading.Thread(target=sampler, daemon=True).start()
    u = runner.ue()
    u.schedule_line(f"[m7-tr1 request] owner=m7-tr1 asks for one <={args.minutes} min QUIET lease \"tr1-{args.prefix}-N\" "
                    f"(final TR1 cost claim, timing only, headless; no Cargo inside), taken at the next free moment after "
                    f"{args.idle:.0f} s with no foreign stillkit compiler. {u.utc().isoformat()}")
    deadline = time.time() + args.wait_minutes * 60
    n = 1
    while n <= args.attempts and time.time() < deadline:
        label = f"{args.prefix}-{n}"
        free = (not u.LOCK.exists() and not runner.window_open(u) and not runner.quiet_active(u, any_lease=False)
                and runner.hold_state(u) != "hold")
        if not free or time.time() - last_busy[0] < args.idle:
            time.sleep(0.05)
            continue
        start = time.time()
        plan_args = argparse.Namespace(label=label, plan=args.plan, minutes=args.minutes, spp=2048, paths="rt,nonrt",
                                       after="", check_only=False, quiet=True, force_schedule=False)
        try:
            runner.plan(plan_args)
        except SystemExit as stop:
            if stop.code == 3:
                time.sleep(0.05)
                continue
            raise
        end = time.time()
        seen = [b for b in busy if start - 3 <= b[0] <= end + 3]
        record = dict(label=label, start=start, end=end, stillkit_idle_before_s=start - max(
            [t for t, _ in busy if t < start], default=start - args.idle), stillkit_samples_inside=seen,
                      timings="quiet" if not seen else "contended")
        (RUNS / label / "quiet-check.json").write_text(json.dumps(record, indent=1) + "\n")
        if not seen:
            print(f"clean quiet run: {label} ({end - start:.0f} s)", flush=True)
            return
        print(f"attempt {label} contended by stillkit ({len(seen)} samples)", flush=True)
        n += 1
    print("no clean quiet run", flush=True)
    sys.exit(1)


if __name__ == "__main__":
    main()
