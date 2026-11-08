"""Run only as n8-run.py's leased command. One lease, sequential lib and SK1 suites."""
import os
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
out = Path(os.environ.get("N8_OUT", "D:/Coding/renderlab-work/m7-sk1"))
target = out / "target/debug/deps"
os.environ.update(RENDERLAB_SK1_SOURCE=str(out / "Fox.glb"),
                  RENDERLAB_SK1_OUT=str(out / "images"),
                  RENDERLAB_EVIDENCE_DIR=str(out / "lib-evidence"),
                  RENDERLAB_SK1_NEAR="E:/renderlab-work/m7-g1c/near-cache/near-field-v1/rla1-rvg1/packages/743a2fce42fdf444407e23c2b00c0586ff0438806eeb023bfc19d91979c502f3",
                  RENDERLAB_SK1_QUARRY="D:/Coding/projects/desktop/renderlab/measurements/quarry-quality/detail/full.rla")
for crate in ("renderlab_asset", "renderlab_cook", "renderlab"):
    matches = sorted(target.glob(f"{crate}-*.exe"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not matches:
        raise SystemExit(f"missing lib test binary for {crate}")
    command = [str(matches[0]), "--test-threads=1"]
    print("SK1 checking", command, flush=True)
    subprocess.run(command, cwd=root, check=True)
    if crate == "renderlab":
        subprocess.run([str(matches[0]), "skinning_tests::gpu_", "--ignored", "--nocapture", "--test-threads=1"], cwd=root, check=True)
print("SK1 lib and image checks complete", flush=True)
