"""P-FLICKER Renderlab runs of the P-YARD manifest with the release adapter, inside this unit's lease
(host.py lease). Validation layers are stripped unless a step asks for them.

    py -3 -B tools/p-flicker/run.py STEP...
      STEP: NAME:MODE:PATH[:OPTIONS], OPTIONS '+'-separated:
        KEY=VALUE   environment for the adapter (RENDERLAB_ABLATE=gi, RENDERLAB_P_FLICKER_GI_EVICT=512, ...)
        layer       keep the Khronos validation layer on (the pre-901c1d2 lease environment)
        passes      write RENDERLAB_PASS_LOG (per-pass GPU timeline) to passes.jsonl
        count       per-frame GI/shadow counters (P-STILL's count.rs)
        bin=NAME    run bin/NAME.exe instead of bin/p_yard_capture.exe
        scene=NAME  another manifest scene (s4-door, s5-cube); default quarry
        clip        the clip manifest (p-flicker/manifest-clip.json: every walk frame 160-223 captured)
      MODE: capture | timing
    Output: D:/Coding/renderlab-work/p-flicker/runs/NAME-PATH (finished steps are skipped).
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import host  # noqa: E402  (run_guarded: the 5 GiB stop)

HERE = Path(__file__).resolve().parent
UNIT = Path("D:/Coding/renderlab-work/p-flicker")  # Unreal captures and the clip manifest
WORK = Path(os.environ.get("P_FLICKER_WORK", UNIT))  # runs, sheets, builds (another unit sets its own)
MANIFEST = HERE.parent / "p-yard/manifest.json"
LAYER = dict(VK_LAYER_PATH="C:/VulkanSDK/1.4.363.0/Bin", VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation")


def run(step):
    name, mode, path, *options = step.split(":")
    options = options[0].split("+") if options else []
    out = WORK / "runs" / f"{name}-{path}"
    if (out / "engine.json").exists():
        print("done", step, flush=True)
        return 0
    out.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH")}
    env["TMP"] = env["TEMP"] = str(WORK / "tmp")
    env["RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK"] = "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"
    binary = WORK / "bin/p_yard_capture.exe"
    scene = "quarry"
    manifest = MANIFEST
    for o in options:
        if o == "layer":
            env.update(LAYER)
        elif o == "passes":
            env["RENDERLAB_PASS_LOG"] = str(out / "passes.jsonl")
        elif o == "count":
            env["RENDERLAB_P_STILL_COUNT"] = "1"
        elif o == "clip":
            manifest = UNIT / "manifest-clip.json"
        elif o.startswith("scene="):
            scene = o[6:]
        elif o.startswith("bin="):
            binary = WORK / "bin" / (o[4:] + ".exe")
        elif "=" in o:
            k, v = o.split("=", 1)
            env[k] = v
        else:
            raise SystemExit("unknown option " + o)
    start = time.monotonic()
    with (out / "stdout.log").open("w") as log:
        command = [str(binary), str(manifest), scene, path, str(out), mode]
        code, stopped = host.run_guarded(command, stdout=log, stderr=subprocess.STDOUT, env=env,
                                         creationflags=subprocess.CREATE_NO_WINDOW)
    (out / "process.json").write_text(json.dumps(dict(step=step, exit_code=code, seconds=time.monotonic() - start,
                                                      binary=str(binary), options=options)))
    print(step, "exit", code, "STOPPED below 5 GiB" if stopped else "", f"{time.monotonic() - start:.1f} s", flush=True)
    if stopped:
        import shutil
        shutil.rmtree(out)  # incomplete; rerun on requeue
        raise SystemExit(75)
    return code


if __name__ == "__main__":
    codes = [run(s) for s in sys.argv[1:]]
    raise SystemExit(max(codes) if codes else 0)
