"""M7-G3: build, check and run the GI defect and lookup-cost measurements (docs/m7-g3.md). The
M7-G2 runner under G3's unit name.

Every command, step syntax and guard is scripts/m7-g2-run.py's: Cargo with two jobs and its
own target on D: (stopped for another owner's QUIET window, never inside a lease), and GPU
steps inside one shared lease holding D:/Coding/renderlab-work/gpu-slot.lock, polled
in-process, with the game/RAM/foreign-renderer guards. Leases are labelled g3-LABEL and
evidence goes to D:/Coding/renderlab-work/m7-g3/runs/LABEL; the Cargo target is
D:/Coding/renderlab-work/g3/target (deleted when the unit ends).

    py -3 -B scripts/m7-g3-run.py check
    py -3 -B scripts/m7-g3-run.py build LABEL
    py -3 -B scripts/m7-g3-run.py gpu RUN rt:quarry:static nonrt:quarry:static --binary LABEL --variants off,g2b,g2b+f3
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_g2_run", Path(__file__).with_name("m7-g2-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-g3")
runner.TARGET = Path("D:/Coding/renderlab-work/g3/target")
runner.UNIT = "m7-g3"
runner.PREFIX = "g3-"

if __name__ == "__main__":
    runner.main()
