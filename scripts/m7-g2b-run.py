"""M7-G2b: build, check and run (docs/m7-g2b.md). The M7-G2 runner under G2b's unit name.

Every command, step syntax and guard is scripts/m7-g2-run.py's: Cargo with two jobs and its
own target on D: (stopped for another owner's QUIET window, never inside a lease), and GPU
steps inside one shared lease holding D:/Coding/renderlab-work/gpu-slot.lock, polled
in-process, with the game/RAM/foreign-renderer guards. Leases are labelled g2b-LABEL and
evidence goes to D:/Coding/renderlab-work/m7-g2b/runs/LABEL.

    py -3 -B scripts/m7-g2b-run.py build LABEL
    py -3 -B scripts/m7-g2b-run.py gpu RUN rt:quarry:static nonrt:quarry:static --binary LABEL
    py -3 -B scripts/m7-g2b-run.py gpu RUN rt:lib --binary LABEL
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_g2_run", Path(__file__).with_name("m7-g2-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-g2b")
runner.TARGET = runner.OUT / "target"
runner.UNIT = "m7-g2b"
runner.PREFIX = "g2b-"

if __name__ == "__main__":
    runner.main()
