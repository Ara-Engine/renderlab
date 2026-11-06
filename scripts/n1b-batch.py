"""Run N1b offscreen fixtures/benches under n1b-run.py's shared GPU/memory lease.

One process loads each asset once and checks all primary modes. The second device
request omits both experimental ray-query features and all AS limits.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("binary", type=Path)
    p.add_argument("root", type=Path)
    p.add_argument("steps", nargs="*", default=["fixture"])
    p.add_argument("--timing", action="store_true")
    a = p.parse_args()
    a.root.mkdir(parents=True, exist_ok=True)
    quarry = json.loads(Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json").read_text())
    record = []
    for step in a.steps:
        no_rt = step.endswith("-no-rt")
        name = step.removesuffix("-no-rt")
        m = dict(output=str(a.root / step), no_rt=no_rt, validate=not a.timing, timing=a.timing)
        if name == "fixture":
            test = "gpu_visibility_fixture"
        elif name == "quarry":
            test = "gpu_visibility_quarry"
            m.update(quarry, output=str(a.root / step), reference=not a.timing)
        else:
            test = "gpu_visibility_bench"
            scene = Path("E:/renderlab-work/contact-fold/fixture/scene.json") if name == "S1" else Path(f"E:/renderlab-work/motion-benches/{name}/fixture/scene.json")
            m["scene"] = str(scene)
        path = a.root / f"manifest-{step}.json"
        path.write_text(json.dumps(m, indent=2))
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path))
        if a.timing:
            env["VK_INSTANCE_LAYERS"] = ""
        start = time.monotonic()
        print(step, test, flush=True)
        result = subprocess.run([str(a.binary), test, "--ignored", "--nocapture", "--test-threads=1"], env=env)
        record.append(dict(step=step, exit=result.returncode, seconds=time.monotonic() - start))
        (a.root / "batch.json").write_text(json.dumps(record, indent=2))
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
