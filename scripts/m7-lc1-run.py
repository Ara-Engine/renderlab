"""LC1 (docs/m7-lc1.md): the G1 runner (scripts/m7-g1-run.py) under owner m7-lc1, with its
Cargo target, staged binaries and evidence on D:/Coding/renderlab-work/m7-lc1. Same
subcommands: build, gpu and plan. Plan steps name `test_name` (LC1's GPU test is
renderer::light_cluster_tests::gpu_lc1_quarry_night) and pass LC1_* variables through `env`;
`"lib": true` runs the non-ignored lib suite inside the lease."""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-lc1"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-lc1")
runner.TARGET = runner.OUT / "target"
runner.BINARY = runner.OUT / "bin/lc1-tests.exe"

if __name__ == "__main__":
    runner.main()
