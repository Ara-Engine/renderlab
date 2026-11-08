"""M7-G1d: the G1 runner (scripts/m7-g1-run.py) under owner m7-g1d, with evidence in
E:/renderlab-work/m7-g1d. Same subcommands: build, gpu and plan."""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-g1d"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

if __name__ == "__main__":
    runner.main()
