"""LC3 build helper: own two-job Cargo target and staged test binary on D:.

    py -3 -B scripts/m7-lc3-run.py build --release --beside-leases

Run tests with scripts/n8-run.py and m7-lc3-batch.py. The quarry test is
renderer::light_cluster_tests::gpu_lc1_quarry_night with LC3=1; its LC1_* scene
controls remain compatible. The fixture is gpu_lc2_penumbra.
"""
import importlib.util
import os
import sys
import subprocess
import shutil
import time
from pathlib import Path

os.environ["M7_G1_UNIT"] = "m7-lc3"
spec = importlib.util.spec_from_file_location("m7_g1_run", Path(__file__).with_name("m7-g1-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
runner.OUT = Path("D:/Coding/renderlab-work/lc3")
runner.TARGET = runner.OUT / "target"
runner.BINARY = runner.OUT / "bin/lc3-tests.exe"


def example():
    u = runner.ue()
    if runner.quiet_active(u, any_lease=False):
        raise SystemExit("DEFERRED: QUIET window")
    with (runner.OUT / "example-build.log").open("w") as log:
        child = subprocess.Popen(["cargo", "build", "--example", "game_scene", "--release",
            "-j", "2", "--target-dir", str(runner.TARGET)], cwd=runner.REPO,
            stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
        while child.poll() is None:
            time.sleep(5)
            if runner.quiet_active(u, any_lease=False):
                subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"], capture_output=True)
                child.wait()
                raise SystemExit("STOPPED for QUIET window")
        if child.returncode: raise SystemExit(child.returncode)
    target = runner.OUT / "bin/game_scene.exe"
    target.parent.mkdir(exist_ok=True)
    shutil.copy2(runner.TARGET / "release/examples/game_scene.exe", target)
    print("binary", target)

if __name__ == "__main__":
    if len(sys.argv)>1 and sys.argv[1]=="build":
        crlf=[str(p) for p in (runner.REPO/"shaders").rglob("*.wgsl") if b"\r\n" in p.read_bytes()]
        if crlf:
            raise SystemExit("WGSL includes require LF for engine seams; normalize before building: "+", ".join(crlf))
    runner.OUT.mkdir(parents=True, exist_ok=True)
    if len(sys.argv)>1 and sys.argv[1]=="example": example()
    else: runner.main()
