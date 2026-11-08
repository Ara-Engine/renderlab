"""IQ1 (docs/iq1-quarry-image.md): the G1 runner (scripts/m7-g1-run.py) under owner m7-iq1,
with its Cargo target, staged binaries and evidence on D:/Coding/renderlab-work/iq1. Same
subcommands: build, gpu and plan. Plan steps name `test_name` (iq1's GPU test is
renderer::iq1_tests::gpu_iq1_quarry) and pass IQ1_* variables through `env`."""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-iq1"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/iq1")
runner.TARGET = runner.OUT / "target"
runner.BINARY = runner.OUT / "bin/iq1-tests.exe"

if __name__ == "__main__":
    runner.main()
