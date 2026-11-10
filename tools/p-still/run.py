"""P-STILL Renderlab runs with the release adapter (host.py build), inside this unit's lease.

    py -3 -B tools/p-still/run.py STEP...   STEP: NAME:MANIFEST:SCENE:MODE:PATH[:KNOBS]
      MANIFEST: count (D:/.../p-still/manifest-count.json) or yard (tools/p-yard/manifest.json)
      MODE: capture | timing | count (timing plus per-frame cache counts)
      KNOBS: '+'-separated RENDERLAB_P_STILL flags (nojitter, norotate, gi-stable, vsm-stable)
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
WORK = Path("D:/Coding/renderlab-work/p-still")
BINARY = WORK / "bin/p_yard_capture.exe"
MANIFESTS = {"count": WORK / "manifest-count.json", "yard": HERE.parent / "p-yard/manifest.json"}


def run(step):
    name, manifest, scene, mode, path, *knobs = step.split(":")
    out = WORK / "runs" / name / path
    if (out / "engine.json").exists():
        print("done", step, flush=True)
        return 0
    out.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH")}
    env["RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK"] = "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"
    env["RENDERLAB_P_STILL"] = ",".join(knobs[0].split("+")) if knobs else ""
    if mode == "count":
        env["RENDERLAB_P_STILL_COUNT"] = "1"
    start = time.monotonic()
    with (out / "stdout.log").open("w") as log:
        code = subprocess.run([str(BINARY), str(MANIFESTS[manifest]), scene, path, str(out),
                               "capture" if mode == "capture" else "timing"],
                              stdout=log, stderr=subprocess.STDOUT, env=env, timeout=600,
                              creationflags=subprocess.CREATE_NO_WINDOW).returncode
    (out / "process.json").write_text(json.dumps(dict(step=step, exit_code=code, seconds=time.monotonic() - start,
                                                      binary=str(BINARY))))
    print(step, "exit", code, f"{time.monotonic() - start:.1f} s", flush=True)
    return code


if __name__ == "__main__":
    codes = [run(s) for s in sys.argv[1:]]
    raise SystemExit(max(codes) if codes else 0)
