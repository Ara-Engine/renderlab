"""M7-GC1: build, check and run the GI cost measurements (docs/m7-gc1.md). The M7-G2 runner
under GC1's unit name.

Every command, step syntax and guard is scripts/m7-g2-run.py's: Cargo with two jobs and its
own target on D: (stopped for another owner's QUIET window, never inside a lease), and GPU
steps inside one shared lease holding D:/Coding/renderlab-work/gpu-slot.lock, polled
in-process, with the game/RAM/foreign-renderer guards. Leases are labelled gc1-LABEL and
evidence goes to D:/Coding/renderlab-work/m7-gc1/runs/LABEL.

    py -3 -B scripts/m7-gc1-run.py build LABEL
    py -3 -B scripts/m7-gc1-run.py gpu RUN rt:quarry:cost nonrt:quarry:cost --binary LABEL
    py -3 -B scripts/m7-gc1-run.py gpu RUN rt:trace nonrt:trace --binary LABEL
    py -3 -B scripts/m7-gc1-run.py gpu RUN rt:lib --binary LABEL
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_g2_run", Path(__file__).with_name("m7-g2-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-gc1")
runner.TARGET = runner.OUT / "target"
runner.UNIT = "m7-gc1"
runner.PREFIX = "gc1-"

if __name__ == "__main__":
    runner.main()
