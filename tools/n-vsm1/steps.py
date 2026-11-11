"""N-VSM1: several commands inside one lease, one after another, each with its own log.

    py -3 -B tools/n-vsm1/host.py lease LABEL MIN -- py -3 -B tools/n-vsm1/steps.py OUT_DIR NAME=COMMAND ARG... ";;" NAME=COMMAND ...

Each step's stdout and stderr go to OUT_DIR/NAME.log; the exit status is the worst step's.
`@KEY=VALUE` arguments before a step's NAME set its environment.
Validation layers are stripped from the environment (timings and images run as a shipping
build does); a step named `layer-...` keeps them.
"""
import os
from pathlib import Path
import subprocess
import sys
import time

WORK = Path("D:/Coding/renderlab-work/n-vsm1")

if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    steps, current = [], []
    for arg in sys.argv[2:]:
        if arg == ";;":
            steps.append(current)
            current = []
        else:
            current.append(arg)
    steps.append(current)
    worst = 0
    for step in filter(None, steps):
        env = dict(os.environ, TMP=str(WORK / "tmp"), TEMP=str(WORK / "tmp"),
                   RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK="I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK")
        while step[0].startswith("@"):
            key, value = step.pop(0)[1:].split("=", 1)
            env[key] = value
        name, first = step[0].split("=", 1)
        if not name.startswith("layer-"):
            env.pop("VK_INSTANCE_LAYERS", None)
            env.pop("VK_LAYER_PATH", None)
        start = time.monotonic()
        with (out / f"{name}.log").open("w") as log:
            code = subprocess.run([first, *step[1:]], stdout=log, stderr=subprocess.STDOUT, env=env,
                                  creationflags=subprocess.CREATE_NO_WINDOW).returncode
        print(name, "exit", code, f"{time.monotonic() - start:.1f} s", flush=True)
        worst = max(worst, abs(code))
    raise SystemExit(min(worst, 1))
