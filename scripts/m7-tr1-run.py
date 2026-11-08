"""M7-TR1: the G1 runner (scripts/m7-g1-run.py) under owner m7-tr1, with evidence in
E:/renderlab-work/m7-tr1. Same subcommands (build, gpu, plan) and the same schedule
poller: no build or lease inside another owner's QUIET window, game holds respected, the
shared lock for every GPU run. TR1's GPU steps name their test with `test_name` and pass
their manifest through `env` (RENDERLAB_TR1_MANIFEST)."""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-tr1"
# TR1b: new bulk output on D: (milestone-7.md, 2026-11-08); E:/renderlab-work/m7-tr1 keeps the earlier evidence.
os.environ.setdefault("M7_G1_ROOT", "D:/Coding/renderlab-work")
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

if __name__ == "__main__":
    runner.main()
