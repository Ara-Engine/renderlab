"""M7-MQ2 lease runner (docs/m7-mq2.md): MQ1's runner with its own owner and evidence root.
Polls in-process (50 ms) until the shared GPU lock is
free, no QUIET window or lease is in force and no game hold applies, then takes one lease with
R1's guard (scripts/m7-r1-run.py: the shared lock, schedule reservation/release lines, game,
RAM and foreign-renderer guards) under owner m7-mq2 and runs one guarded command, usually a
PowerShell batch of game_scene runs and test binaries. Evidence and lease records go under
D:/Coding/renderlab-work/mq2.

    py -3 -B scripts/m7-mq2-run.py LABEL MINUTES GIVE_UP_MINUTES -- powershell -NoProfile -File batch.ps1
"""
import importlib.util
import os
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = Path("D:/Coding/renderlab-work/mq2")


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), REPO / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    label, minutes, wait = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
    command = sys.argv[sys.argv.index("--") + 1:]
    r1 = load("m7-r1-run")
    r1.OUT, r1.OWNER = OUT, "m7-mq2"
    g = r1.guard()
    # Other units' Cargo build scripts (build-script-build.exe under renderlab-work) are CPU
    # builds, not renderers; the generic guard's path rule stops leases on them.
    def without_build_scripts(source):
        def state(*a, **k):
            value = source(*a, **k)
            value["foreign_renderers"] = [p for p in value.get("foreign_renderers", [])
                                          if p["Name"].lower() != "build-script-build.exe"]
            return value
        return state
    g.E2E.state = without_build_scripts(g.E2E.state)
    if getattr(g, "MONITOR_STATE", None):
        g.MONITOR_STATE = without_build_scripts(g.MONITOR_STATE)
    runner = load("m7-g1-run")
    env = dict(os.environ, RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
    g.schedule_line(f"[m7-mq2 request] owner=m7-mq2 asks for one <={minutes} min lease \"{label}\" "
                    f"(game_scene flights and TR1 tests, both paths; headless, functional, no timing claim); "
                    f"takes the lock at the next free moment. {g.utc().isoformat()}")
    deadline = time.time() + wait * 60
    while time.time() < deadline:
        if not r1.LOCK.exists() and not r1.schedule_read_retry(runner.quiet_active, g, any_lease=True) \
                and r1.schedule_read_retry(runner.hold_state, g) != "hold":
            try:
                with g.lease(label, minutes) as owner:
                    out = OUT / "lease" / label
                    g.guarded(command, out, timeout=max(60, int(g.remaining_seconds(owner)) - 20), cwd=REPO, env=env)
                    print(f"{label} done; log {out / 'stdout.log'}", flush=True)
                return 0
            except SystemExit as e:
                if e.code != 3:
                    raise
                print(f"{label} deferred; polling again", flush=True)
            except RuntimeError as e:
                print(f"{label} failed: {e}", flush=True)
                return 1
        time.sleep(0.05)
    print("lease wait timed out", flush=True)
    return 3


if __name__ == "__main__":
    sys.exit(main())
