"""P-YARD2 Renderlab runs of the shared manifest with the release adapter (host.py build).
Run inside this unit's n8-run lease. n8-run injects the Vulkan validation layer for
correctness batches; timing and capture here run without it, as a shipping build does.

    py -3 -B tools/p-yard/rl.py STEP...   steps: SCENE:MODE:PATH[:VISIT], e.g. quarry:timing:rt:1
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-yard2")
BINARY = WORK / "bin/p_yard_capture.exe"


def run(step):
    scene, mode, path, *visit = step.split(":")
    out = WORK / "rl" / scene / (mode if mode == "capture" else f"{mode}-{visit[0] if visit else 0}") / path
    if (out / "engine.json").exists():
        print("done", step, flush=True)
        return 0
    out.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH")}
    env["RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK"] = "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"
    start = time.monotonic()
    with (out / "stdout.log").open("w") as log:
        code = subprocess.run([str(BINARY), str(HERE / "manifest.json"), scene, path, str(out), mode],
                              stdout=log, stderr=subprocess.STDOUT, env=env, timeout=420,
                              creationflags=subprocess.CREATE_NO_WINDOW).returncode
    (out / "process.json").write_text(json.dumps(dict(step=step, exit_code=code, seconds=time.monotonic() - start,
                                                      binary=str(BINARY))))
    print(step, "exit", code, f"{time.monotonic() - start:.1f} s", flush=True)
    return code


if __name__ == "__main__":
    codes = [run(s) for s in sys.argv[1:]]
    raise SystemExit(max(codes) if codes else 0)
