"""M7-G2c: build, check and run (docs/m7-g2c.md). The M7-G2 runner under G2c's unit name.

Every command, step syntax and guard is scripts/m7-g2-run.py's, including its `example`
command and `game` step for API1's example scene. Leases are labelled g2c-LABEL and evidence
goes to D:/Coding/renderlab-work/m7-g2c/runs/LABEL.

    py -3 -B scripts/m7-g2c-run.py example LABEL
    py -3 -B scripts/m7-g2c-run.py build LABEL
    py -3 -B scripts/m7-g2c-run.py gpu RUN rt:game:LABEL nonrt:game:LABEL rt:lib --binary LABEL
"""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("m7_g2_run", Path(__file__).with_name("m7-g2-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-g2c")
runner.TARGET = runner.OUT / "target"
runner.UNIT = "m7-g2c"
runner.PREFIX = "g2c-"

if __name__ == "__main__":
    runner.main()
