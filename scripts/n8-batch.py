"""N8's one batched GPU check: every ignored N8 test in one lease, one process per bench.

Run inside scripts/n8-run.py, which owns the GPU lock and the memory guard:

    py -3 -B scripts/n8-run.py --label n8-batch --minutes 40 --fast-check -- \
        py -3 -B scripts/n8-batch.py <test binary> <root> [step ...]

Steps: fixture, s1, s2, s3, quarry (default: all). Each step writes its manifest under
<root>; scoring runs afterwards on the CPU (n8-score.py).
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

QUARRY = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json")
MOTION = Path("E:/renderlab-work/motion-benches")
S1 = Path("E:/renderlab-work/contact-fold")
MODES = ["Interactive", "Virtual"]


def manifests(root):
    quarry = json.loads(QUARRY.read_text())
    return {
        "fixture": ("gpu_virtual_fixture", dict(output=str(root / "fixture-run"))),
        "s1": ("gpu_contact_fold_batch", dict(scene=str(S1 / "fixture/scene.json"), output=str(root / "S1/renderlab"), modes=MODES)),
        "s2": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S2/fixture/scene.json"), output=str(root / "S2/renderlab"), root=str(root / "S2"),
                                              score_script=None, modes=MODES, cases=["camera-0.25px-r0.04", "shadow-step-r0.04"])),
        "s3": ("gpu_motion_bench_batch", dict(scene=str(MOTION / "S3/fixture/scene.json"), output=str(root / "S3/renderlab"), root=str(root / "S3"),
                                              score_script=None, modes=MODES, cases=["sun-r0.04"])),
        "quarry": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-run"))),
        "quarry-timing": ("gpu_virtual_quarry", dict(quarry, output=str(root / "quarry-timing"), reference=False)),
    }


def main():
    binary, root = Path(sys.argv[1]), Path(sys.argv[2])
    steps = sys.argv[3:] or ["fixture", "s1", "s2", "s3", "quarry"]
    plan = manifests(root)
    record = []
    for step in steps:
        test, manifest = plan[step]
        path = root / f"manifest-{step}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(manifest, indent=2))
        env = dict(os.environ, RENDERLAB_PRIMARY_MANIFEST=str(path))
        started = time.monotonic()
        print(f"N8 batch step {step}: {test}", flush=True)
        result = subprocess.run([str(binary), test, "--ignored", "--nocapture", "--test-threads=1"], env=env)
        record.append(dict(step=step, test=test, exit=result.returncode, seconds=time.monotonic() - started))
        (root / "batch.json").write_text(json.dumps(record, indent=2))
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
