"""LC2's bounded functional batch, run only inside scripts/n8-run.py's GPU lease.

    N8_OWNER=m7-lc2 N8_OUT=D:/Coding/renderlab-work/m7-lc2
    py -3 -B scripts/n8-run.py --label lc2-a --minutes 15 --fast-check -- \
        py -3 -B scripts/m7-lc2-batch.py <binary> <output> [lib|fixture|quarry ...]

The build helper is m7-lc2-run.py. All captures and timings stay on D:.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]


def main():
    binary, root = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    owner = json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    if owner["owner"] != "m7-lc2":
        raise SystemExit("LC2 batch requires its own n8-run.py lease")
    spec = importlib.util.spec_from_file_location("lc2_runner", REPO / "scripts/m7-lc2-run.py")
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    u = helper.runner.ue()
    if helper.runner.quiet_active(u, any_lease=False) or helper.runner.hold_state(u) == "hold":
        raise SystemExit("LC2 batch deferred: QUIET or game hold")
    root.mkdir(parents=True, exist_ok=True)
    metadata = dict(binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    base=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(), lease=owner,
                    validation=False, quiet_timing=False, frames=8)
    (root / "build.json").write_text(json.dumps(metadata, indent=2))
    steps = sys.argv[3:] or ["lib"]
    log = []
    for step in steps:
        for path in ([None] if step == "lib" else ["traced", "nonrt", "raster"]):
            target = root / (step if path is None else f"{step}-{path}")
            target.mkdir(exist_ok=True)
            env = dict(os.environ, LC2="1", LC1_PATH=path or "traced", LC1_OUT=str(target),
                       LC1_QUARRY="E:/renderlab-work/m7-g1c/quarry/manifest.json", LC1_FRAMES="8",
                       VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS", None)
            command = [str(binary), "--test-threads=2"] if step == "lib" else [
                str(binary), "renderer::light_cluster_tests::" + (
                    "gpu_lc2_penumbra" if step == "fixture" else "gpu_lc1_quarry_night"),
                "--ignored", "--exact", "--nocapture", "--test-threads=1"]
            start = time.monotonic()
            with (target / "run.log").open("w") as output:
                result = subprocess.run(command, cwd=REPO, env=env, stdout=output, stderr=subprocess.STDOUT)
            log.append(dict(step=step, path=path, seconds=time.monotonic()-start, exit=result.returncode))
            (root / "batch.json").write_text(json.dumps(log, indent=2))
            print(log[-1], flush=True)
            if result.returncode:
                print((target / "run.log").read_text()[-9000:], flush=True)
                raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
