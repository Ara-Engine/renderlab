"""AT1 (docs/m7-at1.md): the G1 runner (scripts/m7-g1-run.py) under owner m7-at1, with its Cargo
target, staged binaries and evidence on D:/Coding/renderlab-work/at1. Same subcommands: build, gpu
and plan. Plan steps name `test_name` (AT1's GPU test is atmosphere::tests::gpu_at1_quarry) and
pass AT1_* variables through `env`."""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-at1"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/at1")
runner.TARGET = runner.OUT / "target"
runner.BINARY = runner.OUT / "bin/at1-tests.exe"

if __name__ == "__main__":
    runner.main()
