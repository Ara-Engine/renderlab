"""P-GI: build, check and run the world radiance cache cost measurements (docs/p-gi.md). The
M7-G2 runner under P-GI's unit name.

Every command, step syntax and guard is scripts/m7-g2-run.py's: Cargo with two jobs and its
own target on D: (stopped for another owner's QUIET window, never inside a lease), and GPU
steps inside one shared lease holding D:/Coding/renderlab-work/gpu-slot.lock (the lock
scripts/n8-run.py takes), polled in-process, with the game/RAM/foreign-renderer guards.
Leases are labelled p-gi-LABEL and evidence goes to D:/Coding/renderlab-work/p-gi/runs/LABEL;
the Cargo target is D:/Coding/renderlab-work/p-gi/target (deleted when the unit ends).

    py -3 -B scripts/p-gi-run.py build LABEL
    py -3 -B scripts/p-gi-run.py gpu RUN rt:quarry:cost nonrt:quarry:cost --binary LABEL --variants off,g2b
"""
import importlib.util
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_g2_run", Path(__file__).with_name("m7-g2-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/p-gi")
runner.TARGET = Path("D:/Coding/renderlab-work/p-gi/target")
runner.UNIT = "p-gi"
runner.PREFIX = "p-gi-"


class FastSleep:
    """m7-u0b's queued lease polls the lock every 0.5 s and lost every handoff to owners that
    poll faster (P-GI, 2026-11-09): its waits here are 50 ms. Every other check is unchanged,
    and each step's preflight still checks games, foreign renderers and RAM inside the lease."""

    def __getattr__(self, name):
        return getattr(time, name)

    @staticmethod
    def sleep(seconds):
        time.sleep(min(seconds, 0.05))


def guard():
    host = runner.load("m7-u0b")
    host.OUT = runner.OUT
    host.time = FastSleep()
    g = host.ue()
    g.OWNER = runner.UNIT
    g.OUT = runner.OUT
    return g


runner.guard = guard

if __name__ == "__main__":
    runner.main()
