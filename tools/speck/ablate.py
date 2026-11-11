"""U-SPECK diagnosis flights (one lease, resumable): the game scene's flight with its cut at
frame 24, 180 frames at 1080p, on both paths, with one switch off at a time. A finished run
leaves `done.json` and is skipped on a rerun.

    py -3 -B tools/speck/ablate.py GAME_EXE OUT [--every N] [--only name,name]
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-speck")
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"

# name -> (extra arguments, extra environment), most telling first. `--every 7` writes frames
# 14, 21 and 98 (the frames Islam marked) among 27.
SWITCHES = {
    "base": ([], {}),
    "gi-off": ([], {"RENDERLAB_ABLATE": "gi"}),
    "flicker-off": (["--diag", "flicker=0"], {}),
    # Inside GI and lighting (support/estimator_diag.rs): no local lights, sky-only cache.
    "lights-off": (["--diag", "lights=0"], {}),
    "bounce-off": (["--diag", "gi-bounce=0"], {}),
    "tr1-off": ([], {"RENDERLAB_ABLATE": "tr1"}),
    "reflections-off": ([], {"RENDERLAB_ABLATE": "reflections"}),
    "particles-off": (["--no-particles"], {}),
    # Each local light alone (bit 0 arch point, 1 crate spot, 2 fire point), no sun, unrotated
    # GI direction sets.
    "arch-only": (["--diag", "lights=1"], {}),
    "spot-only": (["--diag", "lights=2"], {}),
    "fire-only": (["--diag", "lights=4"], {}),
    "sun-off": (["--diag", "sun=0"], {}),
    "rotate-off": (["--diag", "gi-rotate=0"], {}),
}


def main():
    game, out = sys.argv[1], Path(sys.argv[2])
    every = sys.argv[sys.argv.index("--every") + 1] if "--every" in sys.argv else "7"
    only = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else list(SWITCHES)
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
            "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    worst = 0
    for name in only:
        extra, extra_env = SWITCHES[name]
        for path in ("nonrt", "rt"):
            run = out / f"{path}-{name}"
            if (run / "done.json").exists():
                continue
            run.mkdir(parents=True, exist_ok=True)
            start = time.monotonic()
            r = subprocess.run([game, "--path", path, "--fox", FOX, "--size", "1920x1080", "--frames", "180", "--every", every,
                                "--diag", "cut=1", *extra, "--out", str(run), "--pipeline-cache", str(WORK / "cache")],
                               cwd=REPO, env=env | extra_env, capture_output=True, text=True, timeout=600)
            frames = len(list(run.glob("frame-*.png")))
            print(f"{run.name}: exit {r.returncode}, {frames} frames, {time.monotonic() - start:.0f} s", flush=True)
            if r.returncode == 0:
                (run / "done.json").write_text(json.dumps(dict(game=game, frames=frames, tail=(r.stdout + r.stderr)[-400:])))
            else:
                print((r.stdout + r.stderr)[-1500:], flush=True)
            worst = max(worst, r.returncode)
    return worst


if __name__ == "__main__":
    raise SystemExit(main())
