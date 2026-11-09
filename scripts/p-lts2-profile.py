"""Headless P-LTS2 baseline GPU Trace, run through p-lts2-run.py.

The submit-count trigger and long still replay keep startup work outside the trace.
Optional sampled shader attribution, no screenshots, UI, or clock changes. The
filtered Nsight-only Vulkan layer directory follows the local memory-note recipe.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path("D:/Coding/renderlab-work/p-lts2")
BASE = Path("D:/Coding/renderlab-work/ara1")
NGFX = "C:/Program Files/NVIDIA Corporation/Nsight Graphics 2026.4.0/host/windows-desktop-nomad-x64/ngfx.exe"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("label")
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--case", default="game-lights-unshadowed")
    p.add_argument("--sample", action="store_true")
    p.add_argument("--submits", type=int, default=1200)
    p.add_argument("--path", choices=["rt", "nonrt"], default="rt")
    a = p.parse_args()
    if not re.fullmatch(r"[a-zA-Z0-9_-]+", a.label):
        p.error("label must contain letters, digits, underscores or hyphens")
    owner = json.loads(Path("D:/Coding/renderlab-work/gpu-slot.lock/owner.json").read_text())
    if owner["owner"] != "p-lts2":
        raise RuntimeError("P-LTS2 lease required")
    folder = ROOT / f"profile-{a.label}-{a.path}"
    folder.mkdir(parents=True, exist_ok=False)
    spec = json.loads((BASE / f"phase2-scenes/{a.case}-timing.json").read_text())
    frame = spec["frames"][39].copy()
    frame.update(capture=False, cut=False)
    spec["frames"] = [frame] * 10_000
    scene = folder / "scene.json"
    scene.write_text(json.dumps(spec))
    env = os.environ.copy()
    env.pop("VK_INSTANCE_LAYERS", None)
    env.pop("WGPU_DEBUG", None)
    env.update(VK_LOADER_LAYERS_DISABLE="VK_LAYER_KHRONOS_validation",
               VK_IMPLICIT_LAYER_PATH="E:/renderlab-work/n8f/layers")
    command = [NGFX, "--activity", "GPU Trace Profiler", "--output-dir", str(folder),
        "--exe", str(a.binary), "--dir", str(ROOT),
        "--args", f'"{scene}" {a.path} "{folder}/capture"',
        "--env", "WGPU_DEBUG=1; RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK;",
        "--start-after-submits", str(a.submits),
        "--limit-to-submits", "4", "--max-duration-ms", "50", "--set-gpu-clocks", "unaltered",
        "--collect-screenshot", "0", "--architecture", "Ada", "--metric-set-name", "Top-Level Triage"]
    if a.sample:
        command += ["--real-time-shader-profiler"]
        env["WGPU_DEBUG"] = "1"
    with (folder / "ngfx.log").open("w") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
    # Only this uniquely labelled profiler target may be stopped after collection.
    script = ("Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'rl-capture.exe' "
              f"-and $_.CommandLine -like '*{folder.name}/capture*' "
              "} | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }")
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True,
                   creationflags=subprocess.CREATE_NO_WINDOW)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())

