"""U-SPECK flights (one lease, resumable): the game scene's flight with its cut at frame 24,
180 frames at 1080p, on both paths, with one switch changed at a time. A finished run leaves
`done.json` and is skipped on a rerun.

    py -3 -B tools/speck/ablate.py GAME_EXE OUT [--every N] [--only name,name] [--stills]
                                   [--lib-test LIB_EXE FILTER]

`--stills` also renders each switch from a frozen camera with a steady fire (`--still`, 96
frames, the last written) into OUT/still-PATH-NAME: the settled frame, for the energy check.
`--lib-test` first runs the lib test binary's test named FILTER and stops the lease if it
fails.
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
    # Which lamp-lit surface: no bulb spheres, or bulbs that glow but reflect nothing.
    "bulb-off": (["--diag", "bulb=off"], {}),
    "bulb-dark": (["--diag", "bulb=dark"], {}),
    # The fix (IndirectSettings::near_light_rays): rays aimed around each local light.
    "rays0": (["--diag", "gi-light-rays=0"], {}),
    "rays8": (["--diag", "gi-light-rays=8"], {}),
    "rays16": (["--diag", "gi-light-rays=16"], {}),
    "rays32": (["--diag", "gi-light-rays=32"], {}),
    "rays0-dark": (["--diag", "gi-light-rays=0", "--diag", "bulb=dark"], {}),
    "rays16-dark": (["--diag", "gi-light-rays=16", "--diag", "bulb=dark"], {}),
}
FLIGHT = ["--frames", "180", "--diag", "cut=1"]
STILL = ["--still", "--frames", "96", "--every", "95", "--diag", "flicker=0"]


def main():
    game, out = sys.argv[1], Path(sys.argv[2])
    every = sys.argv[sys.argv.index("--every") + 1] if "--every" in sys.argv else "7"
    only = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else list(SWITCHES)
    (WORK / "tmp").mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_ABLATE")}
    env |= {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
            "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp")}
    if "--lib-test" in sys.argv:
        lib, name = sys.argv[sys.argv.index("--lib-test") + 1:][:2]
        r = subprocess.run([lib, name, "--exact", "--test-threads", "1"], cwd=REPO, env=env, capture_output=True, text=True, timeout=600)
        print((r.stdout + r.stderr)[-1500:], flush=True)
        if r.returncode != 0:
            return r.returncode
    worst = 0
    modes = [("", FLIGHT + ["--every", every])] + ([("still-", STILL)] if "--stills" in sys.argv else [])
    for prefix, mode in modes:
        for name in only:
            extra, extra_env = SWITCHES[name]
            for path in ("nonrt", "rt"):
                run = out / f"{prefix}{path}-{name}"
                if (run / "done.json").exists():
                    continue
                run.mkdir(parents=True, exist_ok=True)
                start = time.monotonic()
                r = subprocess.run([game, "--path", path, "--fox", FOX, "--size", "1920x1080", *mode, *extra,
                                    "--out", str(run), "--pipeline-cache", str(WORK / "cache")],
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
