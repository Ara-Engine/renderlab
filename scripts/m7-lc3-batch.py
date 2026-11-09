"""LC3's bounded functional batch, run only inside scripts/n8-run.py's GPU lease.

    N8_OWNER=m7-lc3 N8_OUT=D:/Coding/renderlab-work/lc3
    py -3 -B scripts/n8-run.py --label lc3-a --minutes 15 --fast-check -- \
        py -3 -B scripts/m7-lc3-batch.py <binary> <output> [lib|fixture|quarry ...]

The build helper is m7-lc3-run.py. All captures and timings stay on D:.
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


def latest_order():
    return next((l for l in reversed(Path("E:/renderlab-work/gpu-schedule.md").read_text(encoding="utf-8").splitlines()) if l.startswith("[root order]")), "")


def main():
    binary, root = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    owner = json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    if owner["owner"] != "m7-lc3":
        raise SystemExit("LC3 batch requires its own n8-run.py lease")
    spec = importlib.util.spec_from_file_location("lc3_runner", REPO / "scripts/m7-lc3-run.py")
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    u = helper.runner.ue()
    if helper.runner.quiet_active(u, any_lease=False) or helper.runner.hold_state(u) == "hold":
        raise SystemExit("LC3 batch deferred: QUIET or game hold")
    # A coordinator may give a short gate priority before this batch. The optional
    # file is operational state outside the repo; no test starts before its release.
    after_file = Path(os.environ.get("N8_OUT", str(root.parent))) / "after-label.txt"
    if after_file.exists():
        after = after_file.read_text().strip()
        if not any(f" {after} window released " in line for line in u.SCHEDULE.read_text().splitlines()):
            raise SystemExit(f"DEFERRED: waiting for {after}'s release")
    root.mkdir(parents=True, exist_ok=True)
    metadata = dict(binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    base=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(), lease=owner,
                    validation=False, quiet_timing=False, frames=8)
    (root / "build.json").write_text(json.dumps(metadata, indent=2))
    steps = sys.argv[3:] or ["lib", "fixture", "quarry"]
    log = []
    order = os.environ["LC3_ROOT_ORDER"]
    for step in steps:
        if latest_order() != order:
            raise SystemExit("DEFERRED: root order changed between steps")
        for path in ([None] if step == "lib" else os.environ.get("LC3_PATHS", "traced,nonrt").split(",")):
            if step == "game-control" and path != "nonrt": continue
            if latest_order() != order or helper.runner.quiet_active(u, any_lease=False) or helper.runner.hold_state(u) == "hold":
                raise SystemExit("DEFERRED: root order, QUIET or game hold changed")
            target = root / (step if path is None else f"{step}-{path}")
            target.mkdir(exist_ok=True)
            env = dict(os.environ, LC2="1", LC3="1", LC1_PATH=path or "traced", LC1_OUT=str(target),
                       LC1_QUARRY="E:/renderlab-work/m7-g1c/quarry/manifest.json", LC1_FRAMES="8",
                       LC2_NEAR="E:/renderlab-work/m7-g1c/near-cache/near-field-v1/rla1-rvg1/packages/743a2fce42fdf444407e23c2b00c0586ff0438806eeb023bfc19d91979c502f3",
                       VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation")
            env.pop("VK_INSTANCE_LAYERS", None)
            command = [str(binary), "--test-threads=2"] if step == "lib" else [
                str(binary), "renderer::light_cluster_tests::" + (
                    "gpu_lc2_penumbra" if step == "fixture" else "gpu_lc1_quarry_night"),
                "--ignored", "--exact", "--nocapture", "--test-threads=1"]
            if step.startswith("game"):
                still = step in ("game-still", "game-control")
                command = [os.environ["LC3_GAME_BINARY"], "--path", "rt" if path == "traced" else "nonrt",
                    "--out", str(target), "--frames", "16" if still else "32",
                    "--every", "8", "--night-lanterns", "--timing-frames", "8",
                    "--pipeline-cache", str(root / f"pipeline-cache-{path}")]
                if os.environ.get("LC3_FOX"):
                    command += ["--fox", os.environ["LC3_FOX"]]
                if still: command.append("--still")
                if step == "game-control": command += ["--local-shadow-count", "4"]
            start = time.monotonic()
            with (target / "run.log").open("w") as output:
                result = subprocess.run(command, cwd=REPO, env=env, stdout=output, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
            log.append(dict(step=step, path=path, seconds=time.monotonic()-start, exit=result.returncode))
            (root / "batch.json").write_text(json.dumps(log, indent=2))
            print(log[-1], flush=True)
            if result.returncode:
                print((target / "run.log").read_text()[-9000:], flush=True)
                raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
