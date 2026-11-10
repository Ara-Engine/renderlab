"""P-CUT game-scene runs (inside an n8-run lease): ARA1 game-base through examples/p_nsq_screen,
one process per variant and visit (GI state must not carry between variants).

    py -3 -B tools/p-cut/game.py VISIT [VARIANT...]   variants: rt rt-base nonrt nonrt-base

`-base` traces the cut's whole shadow budget in the cut frame (pre-P-CUT); the others take the
engine defaults (the cut ramp). Frames 70-87 are captured (72 is the cut).
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

WORK = Path(os.environ.get("P_YARD_WORK", "D:/Coding/renderlab-work/p-cut"))
BINARY = WORK / "bin/p_nsq_screen.exe"
SCENE = Path("D:/Coding/renderlab-work/ara1/phase2-scenes/game-base-quality.json")
VARIANTS = {
    "rt": "rt:path=rt",
    "rt-base": "rt-base:path=rt,budget=256/64/32/8/256",
    "nonrt": "nonrt:path=nonrt",
    "nonrt-base": "nonrt-base:path=nonrt,budget=256/64/32/8/256",
}


def main():
    visit, *names = sys.argv[1:]
    out = WORK / "game" / f"visit-{visit}"
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH")}
    env["RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK"] = "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK"
    env["P_NSQ_FRAMES"] = ",".join(str(i) for i in range(70, 88))
    codes = []
    for name in names or list(VARIANTS):
        if (out / name / "run.json").exists():
            print("done", name, flush=True)
            continue
        out.mkdir(parents=True, exist_ok=True)
        start = time.monotonic()
        with (out / f"{name}.log").open("w") as log:
            code = subprocess.run([str(BINARY), str(SCENE), str(out), VARIANTS[name]], stdout=log,
                                  stderr=subprocess.STDOUT, env=env, timeout=300,
                                  creationflags=subprocess.CREATE_NO_WINDOW).returncode
        (out / f"{name}.process.json").write_text(json.dumps(dict(variant=VARIANTS[name], exit_code=code,
                                                                  seconds=time.monotonic() - start)))
        print(name, "exit", code, f"{time.monotonic() - start:.1f} s", flush=True)
        codes.append(code)
    raise SystemExit(max(codes, default=0))


if __name__ == "__main__":
    main()
