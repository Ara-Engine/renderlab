"""LC2 build helper: own two-job Cargo target and staged test binary on D:.

    py -3 -B scripts/m7-lc2-run.py build --release --beside-leases

Run tests with scripts/n8-run.py and m7-lc2-batch.py. The quarry test is
renderer::light_cluster_tests::gpu_lc1_quarry_night with LC2=1; its LC1_* scene
controls remain compatible. The fixture is gpu_lc2_penumbra.
"""
import importlib.util
import os
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-lc2"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/m7-lc2")
runner.TARGET = runner.OUT / "target"
runner.BINARY = runner.OUT / "bin/lc2-tests.exe"

if __name__ == "__main__":
    runner.main()
